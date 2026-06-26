use std::sync::Arc;
use std::time::{Duration, Instant};

use ::http::header::{ACCEPT, CONTENT_TYPE};
use quick_cache::sync::Cache;
use secrecy::SecretString;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::trace;
use url::form_urlencoded;

use crate::http::Body;
use crate::proxy::httpproxy::PolicyClient;
use crate::serdes::schema;
use crate::types::agent::SimpleBackendReference;
use crate::{apply, http, json};

#[apply(schema!)]
pub struct OAuthTokenExchangeAuth {
	/// Backend serving the RFC 8693 token endpoint.
	pub(super) token_endpoint: Arc<SimpleBackendReference>,
	/// Token endpoint path on the backend; defaults to "/".
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub(super) token_endpoint_path: String,
	/// `audience` parameters naming the target services at the authorization server.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub(super) audiences: Vec<String>,
	/// `scope` values for the requested token, sent space-delimited.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub(super) scopes: Vec<String>,
	/// `resource` parameters with the target service URIs.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub(super) resources: Vec<String>,
	/// `requested_token_type` parameter; the server picks when unset.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) requested_token_type: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) client_auth: Option<OAuthClientAuth>,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache: TokenExchangeCache,
}

#[apply(schema!)]
pub struct OAuthClientAuth {
	/// `client_id` parameter identifying the gateway at the authorization server.
	pub(super) client_id: String,
}

impl OAuthClientAuth {
	pub fn new(client_id: String) -> Self {
		Self { client_id }
	}
}

impl OAuthTokenExchangeAuth {
	pub fn new(
		token_endpoint: Arc<SimpleBackendReference>,
		token_endpoint_path: String,
		audiences: Vec<String>,
		scopes: Vec<String>,
		resources: Vec<String>,
		requested_token_type: Option<String>,
		client_auth: Option<OAuthClientAuth>,
	) -> Self {
		Self {
			token_endpoint,
			token_endpoint_path,
			audiences,
			scopes,
			resources,
			requested_token_type,
			client_auth,
			cache: TokenExchangeCache::default(),
		}
	}
}

const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub(super) const TOKEN_TYPE_ACCESS: &str = "urn:ietf:params:oauth:token-type:access_token";
const TOKEN_TYPE_JWT: &str = "urn:ietf:params:oauth:token-type:jwt";

const CACHE_SAFETY_MARGIN: Duration = Duration::from_secs(30);
const CACHE_CAPACITY: usize = 1024;

#[derive(Debug, Deserialize)]
struct TokenResponse {
	access_token: SecretString,
	issued_token_type: String,
	token_type: String,
	#[serde(default)]
	expires_in: Option<u64>,
}

#[derive(Clone)]
struct CachedToken {
	access_token: SecretString,
	expires_at: Instant,
}

#[derive(Clone)]
struct TokenExchangeCache(Arc<Cache<String, CachedToken>>);

impl Default for TokenExchangeCache {
	fn default() -> Self {
		Self(Arc::new(Cache::new(CACHE_CAPACITY)))
	}
}

impl std::fmt::Debug for TokenExchangeCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("TokenExchangeCache")
	}
}

pub(super) async fn fetch_token(
	client: &PolicyClient,
	auth: &OAuthTokenExchangeAuth,
	subject_token: &str,
	subject_token_type: &str,
) -> anyhow::Result<SecretString> {
	let cache_key = {
		let mut h = Sha256::new();
		h.update(subject_token.as_bytes());
		hex::encode(h.finalize())
	};
	let cache = &auth.cache.0;
	if let Some(cached) = cache.get(&cache_key) {
		if cached.expires_at > Instant::now() {
			trace!("token exchange cache hit");
			return Ok(cached.access_token);
		}
		cache.remove(&cache_key);
	}

	let guard = match cache.get_value_or_guard_async(&cache_key).await {
		Ok(cached) => return Ok(cached.access_token),
		Err(guard) => guard,
	};

	let body = {
		let mut ser = form_urlencoded::Serializer::new(String::new());
		ser
			.append_pair("grant_type", GRANT_TYPE)
			.append_pair("subject_token", subject_token)
			.append_pair("subject_token_type", subject_token_type);
		for audience in &auth.audiences {
			ser.append_pair("audience", audience);
		}
		if !auth.scopes.is_empty() {
			ser.append_pair("scope", &auth.scopes.join(" "));
		}
		for resource in &auth.resources {
			ser.append_pair("resource", resource);
		}
		if let Some(rtt) = &auth.requested_token_type {
			ser.append_pair("requested_token_type", rtt);
		}
		if let Some(client_auth) = &auth.client_auth {
			ser.append_pair("client_id", &client_auth.client_id);
		}
		ser.finish()
	};

	let path = if auth.token_endpoint_path.is_empty() {
		"/"
	} else {
		auth.token_endpoint_path.as_str()
	};
	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri(path)
		.header(CONTENT_TYPE, "application/x-www-form-urlencoded")
		.header(ACCEPT, "application/json")
		.body(Body::from(body.into_bytes()))?;

	let resp = client
		.call_reference(req, &auth.token_endpoint)
		.await
		.map_err(|e| anyhow::anyhow!("token exchange request failed: {e}"))?;

	let status = resp.status();
	let limit = http::response_buffer_limit(&resp);
	if !status.is_success() {
		let body = http::read_body_with_limit(resp.into_body(), limit)
			.await
			.unwrap_or_default();
		let body: String = String::from_utf8_lossy(&body).chars().take(256).collect();
		anyhow::bail!("token exchange returned status {status}: {body}");
	}

	let parsed: TokenResponse = json::from_body_with_limit(resp.into_body(), limit)
		.await
		.map_err(|e| anyhow::anyhow!("token exchange response decode failed: {e}"))?;

	if !parsed.token_type.eq_ignore_ascii_case("Bearer") {
		anyhow::bail!(
			"token exchange returned unsupported token_type: {}",
			parsed.token_type
		);
	}

	if parsed.issued_token_type != TOKEN_TYPE_ACCESS && parsed.issued_token_type != TOKEN_TYPE_JWT {
		anyhow::bail!(
			"token exchange returned unusable issued_token_type: {}",
			parsed.issued_token_type
		);
	}

	let access_token = parsed.access_token;

	if let Some(secs) = parsed.expires_in
		&& secs > CACHE_SAFETY_MARGIN.as_secs()
	{
		let ttl = Duration::from_secs(secs) - CACHE_SAFETY_MARGIN;
		let _ = guard.insert(CachedToken {
			access_token: access_token.clone(),
			expires_at: Instant::now() + ttl,
		});
	}

	trace!("token exchange succeeded");
	Ok(access_token)
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use secrecy::ExposeSecret;
	use serde_json::json;
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::types::agent::Target;

	fn policy_client() -> PolicyClient {
		PolicyClient::new(
			crate::test_helpers::proxymock::setup_proxy_test("{}")
				.unwrap()
				.inputs(),
		)
	}

	fn token_body() -> serde_json::Value {
		json!({
			"access_token": "upstream-token",
			"token_type": "Bearer",
			"issued_token_type": TOKEN_TYPE_ACCESS,
			"expires_in": 3600,
		})
	}

	async fn mock_token_endpoint(body: ResponseTemplate) -> MockServer {
		let mock = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/token"))
			.respond_with(body)
			.mount(&mock)
			.await;
		mock
	}

	fn endpoint(mock: &MockServer) -> Arc<SimpleBackendReference> {
		Arc::new(SimpleBackendReference::InlineBackend(Target::Address(
			*mock.address(),
		)))
	}

	fn auth(endpoint: Arc<SimpleBackendReference>) -> OAuthTokenExchangeAuth {
		OAuthTokenExchangeAuth::new(
			endpoint,
			"/token".into(),
			vec!["https://upstream.example".into()],
			vec![],
			vec![],
			None,
			None,
		)
	}

	async fn sent_form_params(mock: &MockServer) -> HashMap<String, String> {
		let req = &mock.received_requests().await.unwrap()[0];
		form_urlencoded::parse(&req.body).into_owned().collect()
	}

	#[test]
	fn deserializes_minimal_config() {
		let a: OAuthTokenExchangeAuth =
			serde_json::from_str(r#"{"tokenEndpoint": {"host": "localhost:8089"}}"#).unwrap();
		assert!(matches!(
			a.token_endpoint.as_ref(),
			SimpleBackendReference::InlineBackend(_)
		));
		assert!(a.token_endpoint_path.is_empty());
	}

	#[tokio::test]
	async fn sends_form_params() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));

		let tok = fetch_token(&policy_client(), &a, "subj-jwt", TOKEN_TYPE_ACCESS)
			.await
			.expect("exchange succeeds");
		assert_eq!(tok.expose_secret(), "upstream-token");

		let pairs = sent_form_params(&mock).await;
		assert_eq!(pairs["grant_type"], GRANT_TYPE);
		assert_eq!(pairs["subject_token"], "subj-jwt");
		assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_ACCESS);
		assert_eq!(pairs["audience"], "https://upstream.example");
		for k in ["scope", "resource", "requested_token_type", "client_id"] {
			assert!(!pairs.contains_key(k), "unset param {k} must not be sent");
		}
	}

	#[tokio::test]
	async fn sends_optional_params() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = OAuthTokenExchangeAuth::new(
			endpoint(&mock),
			"/token".into(),
			vec![],
			vec!["read".into(), "write".into()],
			vec!["https://upstream.example/api".into()],
			Some(TOKEN_TYPE_ACCESS.into()),
			Some(OAuthClientAuth {
				client_id: "gateway-client".into(),
			}),
		);

		fetch_token(&policy_client(), &a, "subj", TOKEN_TYPE_JWT)
			.await
			.unwrap();
		let pairs = sent_form_params(&mock).await;
		assert!(!pairs.contains_key("audience"));
		assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_JWT);
		assert_eq!(pairs["scope"], "read write");
		assert_eq!(pairs["resource"], "https://upstream.example/api");
		assert_eq!(pairs["requested_token_type"], TOKEN_TYPE_ACCESS);
		assert_eq!(pairs["client_id"], "gateway-client");
	}

	#[tokio::test]
	async fn fails_closed_on_client_error() {
		let mock = mock_token_endpoint(ResponseTemplate::new(401)).await;
		let a = auth(endpoint(&mock));

		assert!(
			fetch_token(&policy_client(), &a, "subj", TOKEN_TYPE_ACCESS)
				.await
				.is_err()
		);
	}

	#[tokio::test]
	async fn rejects_unusable_issued_token_type() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "t",
			"token_type": "Bearer",
			"issued_token_type": "urn:ietf:params:oauth:token-type:saml2",
		})))
		.await;
		let a = auth(endpoint(&mock));

		assert!(
			fetch_token(&policy_client(), &a, "subj", TOKEN_TYPE_ACCESS)
				.await
				.is_err()
		);
	}

	#[tokio::test]
	async fn caches_per_subject() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));
		let client = policy_client();

		let t1 = fetch_token(&client, &a, "subj", TOKEN_TYPE_ACCESS)
			.await
			.unwrap();
		let t2 = fetch_token(&client, &a, "subj", TOKEN_TYPE_ACCESS)
			.await
			.unwrap();
		assert_eq!(t1.expose_secret(), t2.expose_secret());
		assert_eq!(mock.received_requests().await.unwrap().len(), 1);
	}
}
