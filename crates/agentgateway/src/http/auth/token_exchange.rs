use std::sync::Arc;
use std::time::{Duration, Instant};

use ::http::header::{ACCEPT, CONTENT_TYPE};
use quick_cache::sync::Cache;
use secrecy::SecretString;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::trace;
use url::form_urlencoded;

use crate::client::Client;
use crate::http::Body;
use crate::http::oidc::ProviderEndpoint;
use crate::serdes::schema;
use crate::{apply, http, json};

#[apply(schema!)]
pub struct TokenExchangeAuth {
	/// RFC 8693 token endpoint URL.
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub token_endpoint: ProviderEndpoint,
	/// `audience` parameter naming the target service at the authorization server.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub audience: Option<String>,
	/// Space-delimited `scope` parameter for the requested token.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	/// `resource` parameter with the target service URI.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub resource: Option<String>,
	/// `requested_token_type` parameter; the server picks when unset.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub requested_token_type: Option<String>,
	/// `client_id` parameter identifying the gateway at the authorization server.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_id: Option<String>,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache: TokenExchangeCache,
}

impl TokenExchangeAuth {
	pub fn new(
		token_endpoint: ProviderEndpoint,
		audience: Option<String>,
		scope: Option<String>,
		resource: Option<String>,
		requested_token_type: Option<String>,
		client_id: Option<String>,
	) -> Self {
		Self {
			token_endpoint,
			audience,
			scope,
			resource,
			requested_token_type,
			client_id,
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
	access_token: String,
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
	client: &Client,
	auth: &TokenExchangeAuth,
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
			trace!("token exchange cache hit for {}", auth.token_endpoint);
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
		for (k, v) in [
			("audience", &auth.audience),
			("scope", &auth.scope),
			("resource", &auth.resource),
			("requested_token_type", &auth.requested_token_type),
			("client_id", &auth.client_id),
		] {
			if let Some(v) = v {
				ser.append_pair(k, v);
			}
		}
		ser.finish()
	};

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri(auth.token_endpoint.as_str())
		.header(CONTENT_TYPE, "application/x-www-form-urlencoded")
		.header(ACCEPT, "application/json")
		.body(Body::from(body.into_bytes()))?;

	let resp = client
		.simple_call(req)
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

	let access_token = SecretString::from(parsed.access_token);

	if let Some(secs) = parsed.expires_in
		&& secs > CACHE_SAFETY_MARGIN.as_secs()
	{
		let ttl = Duration::from_secs(secs) - CACHE_SAFETY_MARGIN;
		let _ = guard.insert(CachedToken {
			access_token: access_token.clone(),
			expires_at: Instant::now() + ttl,
		});
	}

	trace!("token exchange succeeded for {}", auth.token_endpoint);
	Ok(access_token)
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use hickory_resolver::config::{ResolverConfig, ResolverOpts};
	use secrecy::ExposeSecret;
	use serde_json::json;
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::client;

	fn test_client() -> client::Client {
		client::Client::new(
			&client::Config {
				resolver_cfg: ResolverConfig::default(),
				resolver_opts: ResolverOpts::default(),
			},
			None,
			Default::default(),
			None,
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

	fn endpoint(mock: &MockServer) -> ProviderEndpoint {
		format!("{}/token", mock.uri()).as_str().try_into().unwrap()
	}

	fn auth(endpoint: ProviderEndpoint) -> TokenExchangeAuth {
		TokenExchangeAuth::new(
			endpoint,
			Some("https://upstream.example".into()),
			None,
			None,
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
		let a: TokenExchangeAuth =
			serde_json::from_str(r#"{"tokenEndpoint": "http://localhost:8089/oauth/v2/token"}"#).unwrap();
		assert_eq!(
			a.token_endpoint.as_str(),
			"http://localhost:8089/oauth/v2/token"
		);
	}

	#[tokio::test]
	async fn sends_form_params() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));

		let tok = fetch_token(&test_client(), &a, "subj-jwt", TOKEN_TYPE_ACCESS)
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
		let a = TokenExchangeAuth::new(
			endpoint(&mock),
			None,
			Some("read write".into()),
			Some("https://upstream.example/api".into()),
			Some(TOKEN_TYPE_ACCESS.into()),
			Some("gateway-client".into()),
		);

		fetch_token(&test_client(), &a, "subj", TOKEN_TYPE_JWT)
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
			fetch_token(&test_client(), &a, "subj", TOKEN_TYPE_ACCESS)
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
			fetch_token(&test_client(), &a, "subj", TOKEN_TYPE_ACCESS)
				.await
				.is_err()
		);
	}

	#[tokio::test]
	async fn caches_per_subject() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));
		let client = test_client();

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
