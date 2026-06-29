use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quick_cache::sync::{Cache, EntryAction, EntryResult};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use super::ExchangeRequest;
use super::transport::TokenEndpointResponse;

const DEFAULT_CACHE_CAPACITY: usize = 8192;
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

// Avoid caching tokens near expiry
const CACHE_SAFETY_MARGIN: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct TokenCacheConfig {
	pub max_entries: usize,
	pub default_ttl: Duration,
}

impl Default for TokenCacheConfig {
	fn default() -> Self {
		Self {
			max_entries: DEFAULT_CACHE_CAPACITY,
			default_ttl: DEFAULT_CACHE_TTL,
		}
	}
}

#[derive(Clone)]
pub(super) struct TokenExchangeCache {
	entries: Option<Arc<Cache<TokenCacheKey, CachedToken>>>,
	default_ttl: Duration,
}

impl TokenExchangeCache {
	pub(super) fn new(cfg: &TokenCacheConfig) -> Self {
		Self {
			entries: (cfg.max_entries > 0).then(|| Arc::new(Cache::new(cfg.max_entries))),
			default_ttl: cfg.default_ttl,
		}
	}

	pub(super) async fn get_or_insert_with<F, E>(
		&self,
		req: &ExchangeRequest,
		fetch: F,
	) -> Result<TokenCacheResult, E>
	where
		F: AsyncFnOnce(&ExchangeRequest) -> Result<TokenEndpointResponse, E>,
	{
		let Some(entries) = self.entries.as_ref() else {
			let TokenEndpointResponse { access_token, .. } = fetch(req).await?;
			return Ok(TokenCacheResult::Miss(access_token));
		};

		let now = SystemTime::now();
		let subject_token = req.subject_token.expose_secret();
		let cache_key = TokenCacheKey::from(req);
		let guard = match entries
			.entry_async(&cache_key, |_key, cached| {
				if is_fresh(cached.expires_at, now) {
					EntryAction::Retain(cached.access_token.clone())
				} else {
					EntryAction::ReplaceWithGuard
				}
			})
			.await
		{
			EntryResult::Retained(access_token) => return Ok(TokenCacheResult::Hit(access_token)),
			EntryResult::Vacant(guard) | EntryResult::Replaced(guard, _) => guard,
			EntryResult::Removed(_, _) | EntryResult::Timeout => unreachable!(),
		};

		let TokenEndpointResponse {
			access_token,
			expires_in,
		} = fetch(req).await?;
		if let Some(expires_at) = cache_expiry(expires_in, subject_token, self.default_ttl) {
			let _ = guard.insert(CachedToken {
				access_token: access_token.clone(),
				expires_at,
			});
		}
		Ok(TokenCacheResult::Miss(access_token))
	}

	#[cfg(test)]
	pub(super) fn enabled(&self) -> bool {
		self.entries.is_some()
	}

	#[cfg(test)]
	pub(super) fn default_ttl(&self) -> Duration {
		self.default_ttl
	}
}

impl Default for TokenExchangeCache {
	fn default() -> Self {
		Self::new(&TokenCacheConfig::default())
	}
}

impl fmt::Debug for TokenExchangeCache {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("TokenExchangeCache")
	}
}

/// SHA-256 digest of the per-request exchange inputs. Keyed by digest so the raw
/// bearer credential is never retained as a cache key.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct TokenCacheKey([u8; 32]);

impl From<&ExchangeRequest> for TokenCacheKey {
	fn from(req: &ExchangeRequest) -> Self {
		let mut digest = CacheKeyDigest::new();
		digest.field(req.subject_token.expose_secret().as_bytes());
		digest.field(req.subject_token_type.as_bytes());

		match &req.actor {
			Some((token, token_type)) => {
				digest.field([1]);
				digest.field(token.expose_secret().as_bytes());
				digest.field(token_type.as_bytes());
			},
			None => digest.field([0]),
		}

		// Expects sorted keys to ensure a stable digest.
		for (key, value) in &req.extra_params {
			digest.field(key.as_bytes());
			digest.field(value.as_bytes());
		}

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

	fn finish(self) -> TokenCacheKey {
		TokenCacheKey(self.0.finalize().into())
	}
}

#[derive(Clone)]
struct CachedToken {
	access_token: SecretString,
	expires_at: SystemTime,
}

pub enum TokenCacheResult {
	Hit(SecretString),
	Miss(SecretString),
}

impl TokenCacheResult {
	pub fn into_token(self) -> SecretString {
		match self {
			Self::Hit(token) | Self::Miss(token) => token,
		}
	}
}

// Best-effort `exp` from a JWT-shaped subject token; opaque tokens yield None.
// Decoded without signature or expiry validation — we only need the raw `exp`.
fn subject_token_exp(token: &str) -> Option<SystemTime> {
	#[derive(serde::Deserialize)]
	struct ExpClaim {
		exp: Option<u64>,
	}
	let decoded = jsonwebtoken::dangerous::insecure_decode::<ExpClaim>(token).ok()?;
	UNIX_EPOCH.checked_add(Duration::from_secs(decoded.claims.exp?))
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
mod tests {
	use base64::Engine;
	use base64::prelude::BASE64_URL_SAFE_NO_PAD;

	use super::*;

	fn jwt_with_exp(exp: u64) -> String {
		let header = BASE64_URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
		let body = BASE64_URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
		format!("{header}.{body}.sig")
	}

	#[test]
	fn cache_expiry_is_capped_by_subject_exp() {
		let subject_exp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
			+ 90;
		let token = jwt_with_exp(subject_exp);

		let expires_at = cache_expiry(Some(3600), &token, DEFAULT_CACHE_TTL).unwrap();
		assert_eq!(expires_at, UNIX_EPOCH + Duration::from_secs(subject_exp));
	}

	#[test]
	fn cache_expiry_stores_endpoint_expiry_without_safety_margin() {
		let now = SystemTime::now();
		let expires_at = cache_expiry(Some(300), "not-a-jwt", DEFAULT_CACHE_TTL).unwrap();

		assert!(
			expires_at.duration_since(now).unwrap() > Duration::from_secs(290),
			"expires_at should not subtract the safety margin at insert time"
		);
		assert!(
			expires_at.duration_since(now).unwrap() <= Duration::from_secs(301),
			"expires_at should still reflect the endpoint ttl"
		);
	}

	#[test]
	fn cache_expiry_skips_expired_subject_tokens() {
		let subject_exp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
			.saturating_sub(10);
		let token = jwt_with_exp(subject_exp);

		assert!(cache_expiry(Some(3600), &token, DEFAULT_CACHE_TTL).is_none());
	}

	#[test]
	fn cache_expiry_skips_subject_tokens_near_expiry() {
		let subject_exp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
			+ CACHE_SAFETY_MARGIN.as_secs();
		let token = jwt_with_exp(subject_exp);

		assert!(cache_expiry(Some(3600), &token, DEFAULT_CACHE_TTL).is_none());
	}

	#[test]
	fn cache_expiry_falls_back_to_default_ttl_without_expires_in() {
		let now = SystemTime::now();
		let expires_at = cache_expiry(None, "not-a-jwt", Duration::from_secs(300)).unwrap();

		let remaining = expires_at.duration_since(now).unwrap();
		assert!(
			remaining > Duration::from_secs(290) && remaining <= Duration::from_secs(301),
			"expiry should reflect the default ttl, got {remaining:?}"
		);
	}

	#[test]
	fn is_fresh_requires_safety_margin() {
		let now = SystemTime::now();

		assert!(!is_fresh(now + CACHE_SAFETY_MARGIN, now));
		assert!(is_fresh(
			now + CACHE_SAFETY_MARGIN + Duration::from_secs(1),
			now
		));
	}
}
