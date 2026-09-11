//! RFC 7662 Token Introspection for opaque access tokens.
//!
//! When a bearer token cannot be parsed as a JWT (e.g., ZITADEL DCR-registered
//! clients receive opaque tokens), this module POSTs the token to the
//! authorization server's introspection endpoint and uses the returned claims
//! for downstream authorization decisions.
//!
//! The implementation follows RFC 7662:
//! - Section 2.1: Introspection request (client authentication)
//! - Section 2.2: Introspection response

use std::collections::HashMap;
use std::time::{Duration, Instant};

use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::sync::RwLock;

use crate::http::Body;
use crate::http::jwt::Claims;
use crate::json::from_body_with_limit;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::*;

#[cfg(test)]
#[path = "introspection_tests.rs"]
mod tests;

/// RFC 7662 Token Introspection configuration.
#[derive(Debug, Clone)]
pub struct IntrospectionConfig {
	/// Introspection endpoint URL. If None, derived from OIDC discovery at runtime.
	pub endpoint: Option<String>,
	/// OAuth 2.0 client ID for authenticating the introspection request.
	pub client_id: String,
	/// OAuth 2.0 client secret for confidential client authentication.
	pub client_secret: Option<SecretString>,
	/// Cache TTL for successful introspection results.
	pub cache_duration: Duration,
	/// HTTP request timeout.
	pub timeout: Duration,
	/// Behavior when the introspection endpoint is unreachable.
	pub failure_mode: FailureMode,
	/// Expected token issuer (for post-introspection validation).
	pub expected_issuer: String,
	/// Expected token audiences (for post-introspection validation).
	pub expected_audiences: Vec<String>,
}

/// Controls behavior when the introspection endpoint is unreachable.
#[apply(schema_enum!)]
#[derive(Default)]
pub enum FailureMode {
	/// Reject the request with 503 Service Unavailable.
	#[default]
	FailClosed,
	/// Allow the request through without validation.
	FailOpen,
}

/// RFC 7662 Section 2.2 introspection response.
#[derive(Debug, Deserialize)]
pub struct IntrospectionResponse {
	/// REQUIRED. Boolean indicating whether the token is currently active.
	pub active: bool,
	/// OPTIONAL. Space-separated list of OAuth 2.0 scope values.
	pub scope: Option<String>,
	/// OPTIONAL. Client identifier for the OAuth 2.0 client that requested this token.
	pub client_id: Option<String>,
	/// OPTIONAL. Human-readable identifier for the resource owner who authorized the token.
	pub username: Option<String>,
	/// OPTIONAL. Token type (e.g., "bearer").
	pub token_type: Option<String>,
	/// OPTIONAL. Timestamp of when the token expires (Unix seconds).
	pub exp: Option<i64>,
	/// OPTIONAL. Timestamp of when the token was issued (Unix seconds).
	pub iat: Option<i64>,
	/// OPTIONAL. Timestamp of before which the token must not be used (Unix seconds).
	pub nbf: Option<i64>,
	/// OPTIONAL. Subject identifier (the user principal).
	pub sub: Option<String>,
	/// OPTIONAL. Audience(s) the token is intended for.
	pub aud: Option<Value>,
	/// OPTIONAL. Issuer of the token.
	pub iss: Option<String>,
	/// OPTIONAL. Unique identifier for the token.
	pub jti: Option<String>,
	/// Extension claims (flattened into the map).
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}

/// Errors from the introspection flow.
#[derive(Debug, thiserror::Error)]
pub enum IntrospectionError {
	#[error("introspection endpoint unreachable: {0}")]
	Unreachable(String),

	#[error("introspection endpoint returned HTTP {0}")]
	HttpError(u16),

	#[error("introspection response invalid: {0}")]
	InvalidResponse(String),

	#[error("token is not active")]
	Inactive,

	#[error("token issuer mismatch: expected {expected}, got {actual:?}")]
	IssuerMismatch {
		expected: String,
		actual: Option<String>,
	},

	#[error("token audience mismatch")]
	AudienceMismatch,

	#[error("token has expired")]
	Expired,

	#[error("token is not yet valid (nbf)")]
	NotYetValid,
}

/// Thread-safe LRU-ish cache for introspection results.
///
/// Entries are keyed by SHA-256 hash of the token to avoid keeping raw tokens
/// in memory. Expired entries are evicted lazily on access.
pub struct IntrospectionCache {
	entries: RwLock<HashMap<String, CacheEntry>>,
	ttl: Duration,
}

struct CacheEntry {
	claims: Claims,
	expires_at: Instant,
}

impl IntrospectionCache {
	pub fn new(ttl: Duration) -> Self {
		Self {
			entries: RwLock::new(HashMap::new()),
			ttl,
		}
	}

	/// Look up a cached introspection result by token.
	pub async fn get(&self, token: &str) -> Option<Claims> {
		let key = hash_token(token);
		let entries = self.entries.read().await;
		entries.get(&key).and_then(|entry| {
			if entry.expires_at > Instant::now() {
				Some(entry.claims.clone())
			} else {
				None // Expired; will be refreshed on next call
			}
		})
	}

	/// Store a successful introspection result.
	pub async fn insert(&self, token: &str, claims: Claims) {
		let key = hash_token(token);
		let mut entries = self.entries.write().await;
		// Evict expired entries if the cache is getting large
		if entries.len() > 1000 {
			let now = Instant::now();
			entries.retain(|_, v| v.expires_at > now);
		}
		entries.insert(
			key,
			CacheEntry {
				claims,
				expires_at: Instant::now() + self.ttl,
			},
		);
	}
}

/// SHA-256 hash of a token for use as cache key.
fn hash_token(token: &str) -> String {
	use sha2::{Digest, Sha256};
	let mut hasher = Sha256::new();
	hasher.update(token.as_bytes());
	hex::encode(hasher.finalize())
}

/// Perform RFC 7662 token introspection.
///
/// The token is POSTed to the introspection endpoint with client authentication
/// (HTTP Basic or public client). The response is validated and converted to
/// JWT-compatible `Claims`.
pub async fn introspect(
	client: &PolicyClient,
	config: &IntrospectionConfig,
	token: &str,
	endpoint: &str,
) -> Result<Claims, IntrospectionError> {
	// Build form body per RFC 7662 Section 2.1.
	// The Serializer is !Send, so we scope it to drop before any await points.
	let (form_body, auth_header) = {
		let mut form_ser = url::form_urlencoded::Serializer::new(String::new());
		form_ser.append_pair("token", token);
		form_ser.append_pair("token_type_hint", "access_token");

		let auth_header: Option<String>;
		if let Some(secret) = &config.client_secret {
			// HTTP Basic authentication (confidential client)
			auth_header = Some(format!(
				"Basic {}",
				crate::http::oauth::encode_client_secret_basic(&config.client_id, secret)
			));
		} else {
			// Public client: include client_id in the form body
			form_ser.append_pair("client_id", &config.client_id);
			auth_header = None;
		}

		(form_ser.finish(), auth_header)
	};
	// form_ser is dropped here — no !Send values held across await

	let mut builder = ::http::Request::builder()
		.uri(endpoint)
		.method(::http::Method::POST)
		.header(
			::http::header::CONTENT_TYPE,
			"application/x-www-form-urlencoded",
		)
		.header(::http::header::ACCEPT, "application/json");

	if let Some(header) = auth_header {
		builder = builder.header(::http::header::AUTHORIZATION, header);
	}

	let req = builder
		.body(Body::from(form_body))
		.map_err(|e| IntrospectionError::InvalidResponse(format!("failed to build request: {e}")))?;

	let resp = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Oidc)
		.simple_call(req)
		.await
		.map_err(|e| IntrospectionError::Unreachable(e.to_string()))?;

	let status = resp.status();
	if !status.is_success() {
		return Err(IntrospectionError::HttpError(status.as_u16()));
	}

	let limit = crate::http::response_buffer_limit(&resp);
	let introspection_resp: IntrospectionResponse = from_body_with_limit(resp.into_body(), limit)
		.await
		.map_err(|e| IntrospectionError::InvalidResponse(format!("failed to parse response: {e}")))?;

	// Convert to Claims (validates active, iss, aud, exp, nbf)
	response_to_claims(introspection_resp, token, config)
}

/// Convert an introspection response into JWT-compatible Claims.
///
/// Validates the response per the configured expectations (issuer, audiences, expiry)
/// and constructs a `Claims` struct compatible with downstream consumers (CEL, RBAC, etc.).
fn response_to_claims(
	resp: IntrospectionResponse,
	raw_token: &str,
	config: &IntrospectionConfig,
) -> Result<Claims, IntrospectionError> {
	// RFC 7662 Section 2.2: inactive tokens must be rejected
	if !resp.active {
		return Err(IntrospectionError::Inactive);
	}

	// Validate issuer (always required, matching JWT path behavior)
	let iss = match &resp.iss {
		Some(iss) => {
			if *iss != config.expected_issuer {
				return Err(IntrospectionError::IssuerMismatch {
					expected: config.expected_issuer.clone(),
					actual: Some(iss.clone()),
				});
			}
			iss.clone()
		},
		// IdP didn't return iss; use configured issuer (some IdPs omit it)
		None => config.expected_issuer.clone(),
	};

	// Validate audiences (only if configured, matching JWT path behavior)
	if !config.expected_audiences.is_empty() {
		match &resp.aud {
			Some(Value::String(aud)) => {
				if !config.expected_audiences.contains(aud) {
					return Err(IntrospectionError::AudienceMismatch);
				}
			},
			Some(Value::Array(auds)) => {
				let aud_strs: Vec<&str> = auds.iter().filter_map(|v| v.as_str()).collect();
				if !config
					.expected_audiences
					.iter()
					.any(|expected| aud_strs.contains(&expected.as_str()))
				{
					return Err(IntrospectionError::AudienceMismatch);
				}
			},
			_ => return Err(IntrospectionError::AudienceMismatch),
		}
	}

	// Validate expiry (always checked when present, matching JWT path)
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs() as i64;

	if let Some(exp) = resp.exp
		&& exp < now
	{
		return Err(IntrospectionError::Expired);
	}

	// Validate not-before
	if let Some(nbf) = resp.nbf
		&& nbf > now
	{
		return Err(IntrospectionError::NotYetValid);
	}

	// Build claims map from introspection response
	let mut claims_map: Map<String, Value> = resp.extra;

	// Insert standard claims
	claims_map.insert("iss".into(), Value::String(iss));
	if let Some(sub) = resp.sub {
		claims_map.insert("sub".into(), Value::String(sub));
	}
	if let Some(aud) = resp.aud {
		claims_map.insert("aud".into(), aud);
	}
	if let Some(exp) = resp.exp {
		claims_map.insert("exp".into(), Value::Number(exp.into()));
	}
	if let Some(iat) = resp.iat {
		claims_map.insert("iat".into(), Value::Number(iat.into()));
	}
	if let Some(nbf) = resp.nbf {
		claims_map.insert("nbf".into(), Value::Number(nbf.into()));
	}
	if let Some(client_id) = resp.client_id {
		claims_map.insert("client_id".into(), Value::String(client_id));
	}
	if let Some(scope) = resp.scope {
		claims_map.insert("scope".into(), Value::String(scope));
	}
	if let Some(username) = resp.username {
		claims_map.insert("username".into(), Value::String(username));
	}
	if let Some(token_type) = resp.token_type {
		claims_map.insert("token_type".into(), Value::String(token_type));
	}
	if let Some(jti) = resp.jti {
		claims_map.insert("jti".into(), Value::String(jti));
	}

	Ok(Claims {
		inner: claims_map,
		jwt: SecretString::new(raw_token.to_string().into()),
	})
}

/// Discover the introspection endpoint from OIDC/OAuth metadata.
///
/// Resolution order:
/// 1. RFC 8414: `/.well-known/oauth-authorization-server`
/// 2. OIDC Discovery: `/.well-known/openid-configuration`
///
/// Returns `None` if neither document contains `introspection_endpoint`.
pub async fn discover_introspection_endpoint(
	client: &PolicyClient,
	issuer: &str,
) -> Result<Option<String>, String> {
	// Try RFC 8414 first
	let as_url = crate::http::oauth::authorization_server_metadata_url(issuer);
	if let Ok(Some(endpoint)) = fetch_introspection_endpoint(client, &as_url).await {
		return Ok(Some(endpoint));
	}

	// Fallback to OIDC Discovery
	let oidc_url = crate::http::oauth::openid_configuration_metadata_url(issuer);
	if let Ok(Some(endpoint)) = fetch_introspection_endpoint(client, &oidc_url).await {
		return Ok(Some(endpoint));
	}

	Ok(None)
}

async fn fetch_introspection_endpoint(
	client: &PolicyClient,
	metadata_url: &str,
) -> Result<Option<String>, String> {
	let req = ::http::Request::builder()
		.uri(metadata_url)
		.body(Body::empty())
		.map_err(|e| format!("invalid metadata URL {metadata_url}: {e}"))?;

	let resp = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Oidc)
		.simple_call(req)
		.await
		.map_err(|e| format!("metadata fetch failed for {metadata_url}: {e}"))?;

	if !resp.status().is_success() {
		return Err(format!(
			"metadata endpoint {metadata_url} returned {}",
			resp.status()
		));
	}

	let limit = crate::http::response_buffer_limit(&resp);
	let metadata: Value = from_body_with_limit(resp.into_body(), limit)
		.await
		.map_err(|e| format!("failed to parse metadata JSON: {e}"))?;

	Ok(
		metadata
			.get("introspection_endpoint")
			.and_then(|v| v.as_str())
			.map(|s| s.to_string()),
	)
}
