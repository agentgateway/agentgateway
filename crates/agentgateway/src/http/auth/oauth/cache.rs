use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use quick_cache::sync::Cache;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use super::ExchangeRequest;

pub(super) const CACHE_SAFETY_MARGIN: Duration = Duration::from_secs(30);
const DEFAULT_CACHE_CAPACITY: usize = 8192;
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

/// SHA-256 digest of the per-request exchange inputs. Keyed by digest so the raw
/// bearer credential is never retained as a cache key.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) struct TokenCacheKey([u8; 32]);

impl TokenCacheKey {
	pub(super) fn new(request: &ExchangeRequest) -> Self {
		let mut h = Sha256::new();
		feed(&mut h, request.subject_token.expose_secret().as_bytes());
		// Tag actor presence so "no actor" and an empty actor token cannot collide.
		match &request.actor {
			Some((token, _)) => {
				feed(&mut h, &[1]);
				feed(&mut h, token.expose_secret().as_bytes());
			},
			None => feed(&mut h, &[0]),
		}
		// `extra_params` arrives sorted by key (built from a BTreeMap) for a stable digest.
		for (k, v) in &request.extra_params {
			feed(&mut h, k.as_bytes());
			feed(&mut h, v.as_bytes());
		}
		let mut key = [0u8; 32];
		key.copy_from_slice(&h.finalize());
		Self(key)
	}
}

/// Length-prefix each field so distinct inputs cannot collide through concatenation.
fn feed(h: &mut Sha256, bytes: &[u8]) {
	h.update((bytes.len() as u64).to_le_bytes());
	h.update(bytes);
}

#[derive(Clone)]
pub(super) struct CachedToken {
	pub(super) access_token: SecretString,
	pub(super) expires_at: SystemTime,
}

#[derive(Clone, Debug)]
pub(super) struct TokenCacheConfig {
	pub(super) enabled: bool,
	pub(super) max_entries: usize,
	pub(super) default_ttl: Duration,
}

impl Default for TokenCacheConfig {
	fn default() -> Self {
		Self {
			enabled: true,
			max_entries: DEFAULT_CACHE_CAPACITY,
			default_ttl: DEFAULT_CACHE_TTL,
		}
	}
}

#[derive(Clone)]
pub(super) struct TokenExchangeCache {
	pub(super) entries: Option<Arc<Cache<TokenCacheKey, CachedToken>>>,
	pub(super) default_ttl: Duration,
}

impl TokenExchangeCache {
	pub(super) fn new(cfg: &TokenCacheConfig) -> Self {
		Self {
			entries: cfg.enabled.then(|| Arc::new(Cache::new(cfg.max_entries))),
			default_ttl: cfg.default_ttl,
		}
	}
}

impl Default for TokenExchangeCache {
	fn default() -> Self {
		Self::new(&TokenCacheConfig::default())
	}
}

impl std::fmt::Debug for TokenExchangeCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("TokenExchangeCache")
	}
}

pub(super) fn cache_expiry(expires_in: Option<u64>, subject_token: &str) -> Option<SystemTime> {
	let ttl = Duration::from_secs(expires_in?);
	let now = SystemTime::now();
	let mut expires_at = now.checked_add(ttl)?;
	if let Some(subject_exp) = subject_token_exp(subject_token) {
		expires_at = expires_at.min(subject_exp);
	}
	is_fresh(expires_at, now).then_some(expires_at)
}

pub(super) fn cached_token_valid(token: &CachedToken, now: SystemTime) -> bool {
	is_fresh(token.expires_at, now)
}

fn is_fresh(expires_at: SystemTime, now: SystemTime) -> bool {
	expires_at
		.duration_since(now)
		.is_ok_and(|remaining| remaining > CACHE_SAFETY_MARGIN)
}

fn subject_token_exp(token: &str) -> Option<SystemTime> {
	#[derive(serde::Deserialize)]
	struct ExpClaim {
		exp: Option<u64>,
	}

	let payload = token.split('.').nth(1)?;
	let decoded = BASE64_URL_SAFE_NO_PAD.decode(payload).ok()?;
	let exp = serde_json::from_slice::<ExpClaim>(&decoded).ok()?.exp?;
	UNIX_EPOCH.checked_add(Duration::from_secs(exp))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn unsigned_jwt(exp: u64) -> String {
		let header = BASE64_URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
		let body = BASE64_URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
		format!("{header}.{body}.")
	}

	#[test]
	fn cache_expiry_is_capped_by_subject_exp() {
		let subject_exp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
			+ 90;
		let token = unsigned_jwt(subject_exp);

		let expires_at = cache_expiry(Some(3600), &token).unwrap();
		assert_eq!(expires_at, UNIX_EPOCH + Duration::from_secs(subject_exp));
	}

	#[test]
	fn cache_expiry_stores_endpoint_expiry_without_safety_margin() {
		let now = SystemTime::now();
		let expires_at = cache_expiry(Some(300), "not-a-jwt").unwrap();

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
		let token = unsigned_jwt(subject_exp);

		assert!(cache_expiry(Some(3600), &token).is_none());
	}

	#[test]
	fn cache_expiry_skips_subject_tokens_near_expiry() {
		let subject_exp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
			+ CACHE_SAFETY_MARGIN.as_secs();
		let token = unsigned_jwt(subject_exp);

		assert!(cache_expiry(Some(3600), &token).is_none());
	}

	#[test]
	fn cache_expiry_skips_responses_without_expires_in() {
		assert!(cache_expiry(None, "not-a-jwt").is_none());
	}

	#[test]
	fn cached_token_valid_requires_safety_margin() {
		let now = SystemTime::now();
		let near_expiry = CachedToken {
			access_token: "near".into(),
			expires_at: now + CACHE_SAFETY_MARGIN,
		};
		let fresh = CachedToken {
			access_token: "fresh".into(),
			expires_at: now + CACHE_SAFETY_MARGIN + Duration::from_secs(1),
		};

		assert!(!cached_token_valid(&near_expiry, now));
		assert!(cached_token_valid(&fresh, now));
	}
}
