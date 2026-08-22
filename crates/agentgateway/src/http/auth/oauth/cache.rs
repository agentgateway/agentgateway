use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quick_cache::sync::{Cache, EntryAction, EntryResult};
use secrecy::{ExposeSecret, SecretString};

use super::transport::TokenEndpointResponse;
use super::{ExchangeRequest, decode_unverified_jwt_claims};
use crate::crypto::digest::Sha256;

pub(super) const DEFAULT_CACHE_CAPACITY: usize = 8192;
pub(super) const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

// Avoid caching tokens near expiry
const CACHE_SAFETY_MARGIN: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub(super) struct InMemoryTokenCache {
	entries: Arc<Cache<TokenCacheKey, CachedToken>>,
	default_ttl: Duration,
}

impl InMemoryTokenCache {
	pub(super) fn new(max_entries: usize, default_ttl: Duration) -> Self {
		Self {
			entries: Arc::new(Cache::new(max_entries)),
			default_ttl,
		}
	}

	pub(super) async fn get_or_insert_with<F, Fut, E>(
		&self,
		req: &ExchangeRequest,
		fetch: F,
	) -> Result<TokenCacheResult, E>
	where
		F: FnOnce() -> Fut,
		Fut: Future<Output = Result<TokenEndpointResponse, E>>,
	{
		let now = SystemTime::now();
		let cache_key = TokenCacheKey::from(req);
		let guard = match self
			.entries
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

		let subject_token = req.subject_token.expose_secret();

		let TokenEndpointResponse {
			access_token,
			expires_in,
		} = fetch().await?;

		if let Some(expires_at) = cache_expiry(expires_in, subject_token, self.default_ttl) {
			let _ = guard.insert(CachedToken {
				access_token: access_token.clone(),
				expires_at,
			});
		}
		Ok(TokenCacheResult::Miss(access_token))
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
	fn from(req: &ExchangeRequest) -> Self {
		let mut digest = CacheKeyDigest::new();
		digest.field(req.subject_token.expose_secret().as_bytes());
		digest.field(req.subject_token_type.as_str().as_bytes());

		match &req.actor {
			Some((token, token_type)) => {
				digest.field([1]);
				digest.field(token.expose_secret().as_bytes());
				digest.field(token_type.as_str().as_bytes());
			},
			None => digest.field([0]),
		}

		// Expects sorted keys to ensure a stable digest.
		for (key, value) in &req.extra_params {
			digest.field(key.as_bytes());
			digest.field(value.as_bytes());
		}
		for (key, value) in &req.chained_extra_params {
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
		TokenCacheKey(self.0.finalize())
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
mod tests {
	use std::convert::Infallible;
	use std::sync::atomic::{AtomicUsize, Ordering};

	use base64::Engine;
	use base64::prelude::BASE64_URL_SAFE_NO_PAD;
	use rstest::rstest;

	use super::super::OAuthTokenType;
	use super::*;

	struct CacheFetch {
		req: ExchangeRequest,
		access_token: &'static str,
		expires_in: Option<u64>,
	}

	struct CacheScenario {
		cache: InMemoryTokenCache,
		fetches: Vec<CacheFetch>,
		expected_tokens: Vec<&'static str>,
		expected_calls: usize,
	}

	fn exchange_req(subject: &str, token_type: &str) -> ExchangeRequest {
		ExchangeRequest {
			subject_token: subject.to_string().into(),
			subject_token_type: OAuthTokenType::from_urn(token_type).unwrap(),
			..Default::default()
		}
	}

	fn jwt_with_exp(exp: u64) -> String {
		let header = BASE64_URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
		let body = BASE64_URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
		format!("{header}.{body}.sig")
	}

	async fn fetch_cached(
		cache: &InMemoryTokenCache,
		req: &ExchangeRequest,
		access_token: &str,
		expires_in: Option<u64>,
		calls: &Arc<AtomicUsize>,
	) -> SecretString {
		let access_token = access_token.to_string();
		let calls = Arc::clone(calls);

		cache
			.get_or_insert_with(req, move || {
				calls.fetch_add(1, Ordering::Relaxed);
				std::future::ready(Ok::<_, Infallible>(TokenEndpointResponse {
					access_token: access_token.into(),
					expires_in,
				}))
			})
			.await
			.unwrap()
			.into_token()
	}

	fn same_request_cached_case() -> CacheScenario {
		let req = exchange_req("subj", "urn:ietf:params:oauth:token-type:access_token");
		CacheScenario {
			cache: InMemoryTokenCache::default(),
			fetches: vec![
				CacheFetch {
					req: req.clone(),
					access_token: "upstream-token",
					expires_in: Some(3600),
				},
				CacheFetch {
					req,
					access_token: "other-token",
					expires_in: Some(3600),
				},
			],
			expected_tokens: vec!["upstream-token", "upstream-token"],
			expected_calls: 1,
		}
	}

	fn caches_per_subject_token_type_case() -> CacheScenario {
		CacheScenario {
			cache: InMemoryTokenCache::default(),
			fetches: vec![
				CacheFetch {
					req: exchange_req("subj", "urn:ietf:params:oauth:token-type:access_token"),
					access_token: "access-token",
					expires_in: Some(3600),
				},
				CacheFetch {
					req: exchange_req("subj", "urn:ietf:params:oauth:token-type:jwt"),
					access_token: "jwt-token",
					expires_in: Some(3600),
				},
				CacheFetch {
					req: exchange_req("subj", "urn:ietf:params:oauth:token-type:access_token"),
					access_token: "other-token",
					expires_in: Some(3600),
				},
			],
			expected_tokens: vec!["access-token", "jwt-token", "access-token"],
			expected_calls: 2,
		}
	}

	fn missing_expires_in_falls_back_to_default_ttl_case() -> CacheScenario {
		let req = exchange_req("subj", "urn:ietf:params:oauth:token-type:access_token");
		CacheScenario {
			cache: InMemoryTokenCache::new(DEFAULT_CACHE_CAPACITY, Duration::from_secs(120)),
			fetches: vec![
				CacheFetch {
					req: req.clone(),
					access_token: "upstream-token",
					expires_in: None,
				},
				CacheFetch {
					req,
					access_token: "other-token",
					expires_in: None,
				},
			],
			expected_tokens: vec!["upstream-token", "upstream-token"],
			expected_calls: 1,
		}
	}

	fn expired_subject_not_cached_case() -> CacheScenario {
		let subject = jwt_with_exp(
			SystemTime::now()
				.duration_since(UNIX_EPOCH)
				.unwrap()
				.as_secs()
				.saturating_sub(10),
		);
		let req = exchange_req(&subject, "urn:ietf:params:oauth:token-type:access_token");
		CacheScenario {
			cache: InMemoryTokenCache::default(),
			fetches: vec![
				CacheFetch {
					req: req.clone(),
					access_token: "first-token",
					expires_in: Some(3600),
				},
				CacheFetch {
					req,
					access_token: "second-token",
					expires_in: Some(3600),
				},
			],
			expected_tokens: vec!["first-token", "second-token"],
			expected_calls: 2,
		}
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

	#[rstest]
	#[case::same_request_cached(same_request_cached_case())]
	#[case::per_subject_token_type(caches_per_subject_token_type_case())]
	#[case::missing_expires_in(missing_expires_in_falls_back_to_default_ttl_case())]
	#[case::expired_subject(expired_subject_not_cached_case())]
	#[tokio::test]
	async fn cache_fetch_cases(#[case] scenario: CacheScenario) {
		let calls = Arc::new(AtomicUsize::new(0));
		let cache = scenario.cache;
		let mut tokens = Vec::with_capacity(scenario.fetches.len());

		for fetch in scenario.fetches {
			tokens.push(
				fetch_cached(
					&cache,
					&fetch.req,
					fetch.access_token,
					fetch.expires_in,
					&calls,
				)
				.await
				.expose_secret()
				.to_string(),
			);
		}

		assert_eq!(tokens, scenario.expected_tokens);
		assert_eq!(calls.load(Ordering::Relaxed), scenario.expected_calls);
	}
}
