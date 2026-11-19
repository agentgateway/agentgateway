use std::fmt;
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::{ExposeSecret, SecretString};

use super::transport::TokenEndpointResponse;
use super::{ExchangeRequest, decode_unverified_jwt_claims};
use crate::crypto::digest::Sha256;
use crate::http::expiring_cache::ExpiringCache;

pub(super) const DEFAULT_CACHE_CAPACITY: usize = 8192;
pub(super) const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

// Avoid caching tokens near expiry
const CACHE_SAFETY_MARGIN: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub(super) struct InMemoryTokenCache {
	entries: ExpiringCache<TokenCacheKey, SecretString>,
	default_ttl: Duration,
}

impl InMemoryTokenCache {
	pub(super) fn new(max_entries: usize, default_ttl: Duration) -> Self {
		Self {
			entries: ExpiringCache::new(max_entries),
			default_ttl,
		}
	}

	pub(super) async fn get_or_insert_with<F, Fut, E>(
		&self,
		req: &ExchangeRequest,
		fetch: F,
	) -> Result<SecretString, E>
	where
		F: FnOnce() -> Fut,
		Fut: Future<Output = Result<TokenEndpointResponse, E>>,
		E: fmt::Display,
	{
		self
			.entries
			.get_or_fetch(TokenCacheKey::from(req), || async {
				let TokenEndpointResponse {
					access_token,
					expires_in,
				} = fetch().await?;
				let ttl = cache_expiry(
					expires_in,
					req.subject_token.expose_secret(),
					self.default_ttl,
				)
				.and_then(|expires_at| expires_at.duration_since(SystemTime::now()).ok())
				.and_then(|ttl| ttl.checked_sub(CACHE_SAFETY_MARGIN));
				Ok::<_, E>((access_token, ttl))
			})
			.await
	}
}

impl Default for InMemoryTokenCache {
	fn default() -> Self {
		Self::new(DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_TTL)
	}
}

impl fmt::Debug for InMemoryTokenCache {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("InMemoryTokenCache")
	}
}

/// SHA-256 digest of the per-request exchange inputs. Keyed by digest so the raw
/// bearer credential is never retained as a cache key.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct TokenCacheKey([u8; 32]);

impl From<&ExchangeRequest> for TokenCacheKey {
	#[deny(unused_variables)]
	fn from(req: &ExchangeRequest) -> Self {
		let mut digest = CacheKeyDigest::new();

		let ExchangeRequest {
			subject_token,
			subject_token_type,
			actor,
			extra_params,
			chained_extra_params,
		} = req;

		digest.field(subject_token.expose_secret());
		digest.field(subject_token_type.as_str());
		digest.optional(actor.as_ref(), |digest, (token, token_type)| {
			digest.field(token.expose_secret());
			digest.field(token_type.as_str());
		});
		digest.params(extra_params);
		digest.params(chained_extra_params);

		digest.finish()
	}
}

struct CacheKeyDigest(Sha256);

impl CacheKeyDigest {
	fn new() -> Self {
		Self(Sha256::new())
	}

	fn field(&mut self, bytes: impl AsRef<[u8]>) {
		let bytes = bytes.as_ref();
		self.0.update((bytes.len() as u64).to_le_bytes());
		self.0.update(bytes);
	}

	fn params(&mut self, params: &[(String, String)]) {
		self.0.update((params.len() as u64).to_le_bytes());
		for (key, value) in params {
			self.field(key);
			self.field(value);
		}
	}

	fn optional<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
		match value {
			None => self.0.update([0]),
			Some(value) => {
				self.0.update([1]);
				write(self, value);
			},
		}
	}

	fn finish(self) -> TokenCacheKey {
		TokenCacheKey(self.0.finalize())
	}
}

// Best-effort `exp` from a JWT-shaped subject token; opaque tokens yield None.
// Decoded without signature or expiry validation — we only need the raw `exp`.
fn subject_token_exp(token: &str) -> Option<SystemTime> {
	#[derive(serde::Deserialize)]
	struct ExpClaim {
		exp: Option<u64>,
	}
	let claims = decode_unverified_jwt_claims::<ExpClaim>(token)?;
	UNIX_EPOCH.checked_add(Duration::from_secs(claims.exp?))
}

fn cache_expiry(
	expires_in: Option<u64>,
	subject_token: &str,
	default_ttl: Duration,
) -> Option<SystemTime> {
	let now = SystemTime::now();
	let mut expires_at = now.checked_add(expires_in.map_or(default_ttl, Duration::from_secs))?;
	if let Some(subject_exp) = subject_token_exp(subject_token) {
		expires_at = expires_at.min(subject_exp);
	}
	is_fresh(expires_at, now).then_some(expires_at)
}

fn is_fresh(expires_at: SystemTime, now: SystemTime) -> bool {
	expires_at
		.duration_since(now)
		.is_ok_and(|remaining| remaining > CACHE_SAFETY_MARGIN)
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
