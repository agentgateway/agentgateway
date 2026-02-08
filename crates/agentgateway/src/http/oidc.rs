use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oauth2::basic::{
	BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse, BasicTokenType,
};
use oauth2::{
	AuthType, AuthorizationCode, Client as OAuth2Client, ClientId, ClientSecret, ExtraTokenFields,
	HttpRequest as OAuth2HttpRequest, HttpResponse as OAuth2HttpResponse, PkceCodeVerifier,
	RefreshToken, RequestTokenError, StandardRevocableToken,
	TokenResponse as OAuth2TokenResponseTrait, TokenUrl,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tracing::debug;

use crate::client::Client;
use crate::http::jwt::{Jwt, Mode as JwtMode, Provider as JwtProvider, TokenError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("discovery failed: {0}")]
	Discovery(String),
	#[error("token exchange failed: {0}")]
	Exchange(String),
	#[error("invalid state")]
	InvalidState,
	#[error("state expired")]
	StateExpired,
	#[error("invalid token: {0}")]
	InvalidToken(#[from] TokenError),
	#[error("internal error: {0}")]
	Internal(String),
}

#[derive(Debug)]
pub struct OidcProvider {
	// Metadata is global per issuer. Key is issuer URL.
	metadata_cache: RwLock<HashMap<String, CachedMetadata>>,
	// Per-issuer singleflight lock for metadata discovery.
	metadata_inflight: Mutex<HashMap<String, Arc<Mutex<()>>>>,
	// Validators are specific to issuer + audiences.
	validator_cache: RwLock<HashMap<String, CachedValidator>>,
	// Per (issuer + audiences) singleflight lock for JWKS/validator refresh.
	validator_inflight: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

#[derive(Debug, Clone)]
struct CachedValidator {
	validator: Jwt,
	last_refresh: Instant,
	last_refresh_forced: bool,
}

#[derive(Debug, Clone)]
struct CachedMetadata {
	metadata: Arc<OidcMetadata>,
	fetched_at: Instant,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OidcMetadata {
	pub authorization_endpoint: String,
	pub token_endpoint: String,
	pub jwks_uri: String,
	#[serde(default)]
	pub token_endpoint_auth_methods_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
	pub access_token: String,
	pub token_type: String,
	pub expires_in: Option<u64>,
	pub refresh_token: Option<String>,
	pub id_token: Option<String>,
}

const FORCE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const METADATA_TTL: Duration = Duration::from_secs(300);
const OIDC_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const OIDC_TOKEN_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const OIDC_HTTP_RESPONSE_LIMIT: usize = 2_097_152;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct OidcTokenExtraFields {
	#[serde(default)]
	id_token: Option<String>,
}
impl ExtraTokenFields for OidcTokenExtraFields {}

type OidcOAuth2TokenResponse = oauth2::StandardTokenResponse<OidcTokenExtraFields, BasicTokenType>;
type OidcOAuth2BaseClient = OAuth2Client<
	BasicErrorResponse,
	OidcOAuth2TokenResponse,
	BasicTokenIntrospectionResponse,
	StandardRevocableToken,
	BasicRevocationErrorResponse,
>;
type OidcOAuth2Client = OAuth2Client<
	BasicErrorResponse,
	OidcOAuth2TokenResponse,
	BasicTokenIntrospectionResponse,
	StandardRevocableToken,
	BasicRevocationErrorResponse,
	oauth2::EndpointNotSet,
	oauth2::EndpointNotSet,
	oauth2::EndpointNotSet,
	oauth2::EndpointNotSet,
	oauth2::EndpointSet,
>;

#[derive(Debug, thiserror::Error)]
enum OAuthHttpClientError {
	#[error("failed to build oauth2 request: {0}")]
	BuildRequest(String),
	#[error("token endpoint call failed: {0}")]
	Call(String),
	#[error("failed to read token endpoint response body: {0}")]
	ReadBody(String),
	#[error("failed to build oauth2 response: {0}")]
	BuildResponse(String),
}

impl Default for OidcProvider {
	fn default() -> Self {
		Self::new()
	}
}

impl OidcProvider {
	pub fn new() -> Self {
		Self {
			metadata_cache: RwLock::new(HashMap::new()),
			metadata_inflight: Mutex::new(HashMap::new()),
			validator_cache: RwLock::new(HashMap::new()),
			validator_inflight: Mutex::new(HashMap::new()),
		}
	}

	async fn inflight_lock_for(
		inflight: &Mutex<HashMap<String, Arc<Mutex<()>>>>,
		key: &str,
	) -> Arc<Mutex<()>> {
		let mut map = inflight.lock().await;
		map
			.entry(key.to_string())
			.or_insert_with(|| Arc::new(Mutex::new(())))
			.clone()
	}

	async fn maybe_cleanup_inflight(
		inflight: &Mutex<HashMap<String, Arc<Mutex<()>>>>,
		key: &str,
		flight: &Arc<Mutex<()>>,
	) {
		let mut map = inflight.lock().await;
		let should_remove = map
			.get(key)
			.is_some_and(|entry| Arc::ptr_eq(entry, flight) && Arc::strong_count(entry) == 2);
		if should_remove {
			map.remove(key);
		}
	}

	fn normalized_audiences(audiences: Option<Vec<String>>) -> Option<Vec<String>> {
		let mut audiences = audiences?;
		audiences.sort_unstable();
		audiences.dedup();
		if audiences.is_empty() {
			None
		} else {
			Some(audiences)
		}
	}

	fn validator_cache_key(issuer: &str, audiences: Option<&[String]>) -> String {
		let Some(audiences) = audiences else {
			return issuer.to_string();
		};
		if audiences.is_empty() {
			return issuer.to_string();
		}

		// Length-prefix each audience for an unambiguous key, independent of separator chars.
		let mut key =
			String::with_capacity(issuer.len() + audiences.iter().map(String::len).sum::<usize>() + 16);
		key.push_str(issuer);
		for audience in audiences {
			key.push('|');
			key.push_str(&audience.len().to_string());
			key.push(':');
			key.push_str(audience);
		}
		key
	}

	pub async fn get_info(
		&self,
		client: &Client,
		issuer: &str,
		audiences: Option<Vec<String>>,
	) -> Result<(Arc<OidcMetadata>, Jwt), Error> {
		let audiences = Self::normalized_audiences(audiences);
		let metadata = self.get_metadata(client, issuer).await?;
		let validator = self
			.get_validator(client, issuer, audiences, &metadata, false)
			.await?;
		Ok((metadata, validator))
	}

	/// Validates a token, attempting a JWKS refresh if the key is unknown.
	pub async fn validate_token(
		&self,
		client: &Client,
		issuer: &str,
		audiences: Option<Vec<String>>,
		token: &str,
	) -> Result<crate::http::jwt::Claims, Error> {
		let audiences = Self::normalized_audiences(audiences);
		let (metadata, validator) = self.get_info(client, issuer, audiences.clone()).await?;

		match validator.validate_claims(token) {
			Ok(claims) => Ok(claims),
			Err(TokenError::UnknownKeyId(_)) => {
				// Potential key rotation. Try refreshing JWKS.
				debug!(
					"Unknown key ID in token, attempting JWKS refresh for {}",
					issuer
				);
				let validator = self
					.get_validator(client, issuer, audiences, &metadata, true)
					.await?;
				Ok(validator.validate_claims(token)?)
			},
			Err(e) => Err(e.into()),
		}
	}

	pub async fn get_metadata(
		&self,
		client: &Client,
		issuer: &str,
	) -> Result<Arc<OidcMetadata>, Error> {
		// 1. Fast Path: Read Lock
		{
			if let Some(entry) = self.metadata_cache.read().await.get(issuer)
				&& entry.fetched_at.elapsed() < METADATA_TTL
			{
				return Ok(entry.metadata.clone());
			}
		} // Drop read lock

		// 2. Singleflight: ensure only one metadata fetch per issuer at a time.
		let flight = Self::inflight_lock_for(&self.metadata_inflight, issuer).await;
		let result = async {
			let _flight_guard = flight.lock().await;

			// 3. Re-check cache after waiting for another in-flight fetch.
			{
				if let Some(entry) = self.metadata_cache.read().await.get(issuer)
					&& entry.fetched_at.elapsed() < METADATA_TTL
				{
					return Ok(entry.metadata.clone());
				}
			}

			// 4. Slow Path: Network Call (singleflight lock is held for this issuer)
			let url = format!(
				"{}/.well-known/openid-configuration",
				issuer.trim_end_matches('/')
			);
			let req = ::http::Request::builder()
				.uri(&url)
				.body(crate::http::Body::empty())
				.map_err(|e| Error::Internal(e.to_string()))?;
			let resp = tokio::time::timeout(OIDC_HTTP_TIMEOUT, client.simple_call(req))
				.await
				.map_err(|_| {
					Error::Discovery("oidc discovery request timed out while fetching metadata".to_string())
				})?
				.map_err(|e| Error::Discovery(e.to_string()))?;
			let metadata: OidcMetadata =
				tokio::time::timeout(OIDC_HTTP_TIMEOUT, crate::json::from_response_body(resp))
					.await
					.map_err(|_| Error::Discovery("oidc metadata response body read timed out".to_string()))?
					.map_err(|e| Error::Discovery(e.to_string()))?;
			let metadata = Arc::new(metadata);

			// 5. Write Path: Update Cache
			let mut w = self.metadata_cache.write().await;
			// Optimization: In a thundering herd scenario, someone else might have updated it while we were fetching.
			if let Some(entry) = w.get(issuer)
				&& entry.fetched_at.elapsed() < METADATA_TTL
			{
				return Ok(entry.metadata.clone());
			}
			w.insert(
				issuer.to_string(),
				CachedMetadata {
					metadata: metadata.clone(),
					fetched_at: Instant::now(),
				},
			);
			Ok(metadata)
		}
		.await;
		Self::maybe_cleanup_inflight(&self.metadata_inflight, issuer, &flight).await;
		result
	}

	async fn get_validator(
		&self,
		client: &Client,
		issuer: &str,
		audiences: Option<Vec<String>>,
		metadata: &OidcMetadata,
		force_refresh: bool,
	) -> Result<Jwt, Error> {
		let key = Self::validator_cache_key(issuer, audiences.as_deref());
		let cleanup_key = key.clone();

		// 1. Fast Path: Read Lock
		{
			let cache = self.validator_cache.read().await;
			if let Some(entry) = cache.get(&key) {
				if !force_refresh {
					return Ok(entry.validator.clone());
				}
				// Throttle only repeated forced refreshes (e.g. kid-spray),
				// while allowing an immediate first forced refresh after a normal cache fill.
				if entry.last_refresh_forced && entry.last_refresh.elapsed() < FORCE_REFRESH_INTERVAL {
					debug!(
						"Skipping JWKS refresh for {}, already refreshed very recently",
						issuer
					);
					return Ok(entry.validator.clone());
				}
			}
		} // Drop read lock

		// 2. Singleflight: ensure only one JWKS fetch per cache key at a time.
		let flight = Self::inflight_lock_for(&self.validator_inflight, &key).await;
		let result = async {
			let _flight_guard = flight.lock().await;

			// 3. Re-check cache after waiting for another in-flight fetch.
			{
				let cache = self.validator_cache.read().await;
				if let Some(entry) = cache.get(&key) {
					if !force_refresh {
						return Ok(entry.validator.clone());
					}
					if entry.last_refresh_forced && entry.last_refresh.elapsed() < FORCE_REFRESH_INTERVAL {
						debug!(
							"Skipping JWKS refresh for {}, already refreshed very recently",
							issuer
						);
						return Ok(entry.validator.clone());
					}
				}
			}

			// 4. Slow Path: Network Call (singleflight lock is held for this cache key)
			// Initialize Jwt validator using the discovered jwks_uri
			let jwks_req = ::http::Request::builder()
				.uri(&metadata.jwks_uri)
				.body(crate::http::Body::empty())
				.map_err(|e| Error::Internal(e.to_string()))?;
			let jwks_resp = tokio::time::timeout(OIDC_HTTP_TIMEOUT, client.simple_call(jwks_req))
				.await
				.map_err(|_| Error::Discovery("jwks fetch request timed out".to_string()))?
				.map_err(|e| Error::Discovery(format!("JWKS fetch failed: {e}")))?;
			let jwk_set: jsonwebtoken::jwk::JwkSet = tokio::time::timeout(
				OIDC_HTTP_TIMEOUT,
				crate::json::from_response_body(jwks_resp),
			)
			.await
			.map_err(|_| Error::Discovery("jwks response body read timed out".to_string()))?
			.map_err(|e| Error::Discovery(format!("JWKS parse failed: {e}")))?;

			let provider = JwtProvider::from_jwks(jwk_set, issuer.to_string(), audiences)
				.map_err(|e| Error::Internal(format!("failed to create JWT provider: {e}")))?;

			let jwt = Jwt::from_providers(vec![provider], JwtMode::Strict);

			// 5. Write Path: Update Cache
			let mut w = self.validator_cache.write().await;
			// Optimization: In a thundering herd scenario, someone else might have updated it while we were fetching.
			// Overwriting with a fresh validator is safe.
			w.insert(
				key,
				CachedValidator {
					validator: jwt.clone(),
					last_refresh: Instant::now(),
					last_refresh_forced: force_refresh,
				},
			);

			Ok(jwt)
		}
		.await;
		Self::maybe_cleanup_inflight(&self.validator_inflight, &cleanup_key, &flight).await;
		result
	}

	fn preferred_token_auth_type(metadata: &OidcMetadata) -> Result<AuthType, Error> {
		let supports = &metadata.token_endpoint_auth_methods_supported;
		if supports.is_empty() {
			// Per OIDC discovery defaults, use basic auth when methods are not advertised.
			return Ok(AuthType::BasicAuth);
		}
		if supports
			.iter()
			.any(|m| m.eq_ignore_ascii_case("client_secret_basic"))
		{
			return Ok(AuthType::BasicAuth);
		}
		if supports
			.iter()
			.any(|m| m.eq_ignore_ascii_case("client_secret_post"))
		{
			return Ok(AuthType::RequestBody);
		}
		Err(Error::Discovery(
			"token endpoint auth methods do not include client_secret_basic or client_secret_post"
				.to_string(),
		))
	}

	fn oauth2_client(
		metadata: &OidcMetadata,
		client_id: &str,
		client_secret: &str,
	) -> Result<OidcOAuth2Client, Error> {
		let token_url = TokenUrl::new(metadata.token_endpoint.clone())
			.map_err(|e| Error::Internal(format!("invalid token endpoint URL: {e}")))?;
		let auth_type = Self::preferred_token_auth_type(metadata)?;
		Ok(
			OidcOAuth2BaseClient::new(ClientId::new(client_id.to_string()))
				.set_client_secret(ClientSecret::new(client_secret.to_string()))
				.set_auth_type(auth_type)
				.set_token_uri(token_url),
		)
	}

	async fn oauth_http_call(
		client: Client,
		request: OAuth2HttpRequest,
	) -> Result<OAuth2HttpResponse, OAuthHttpClientError> {
		let (parts, body) = request.into_parts();
		let mut req_builder = ::http::Request::builder()
			.method(parts.method)
			.uri(parts.uri)
			.version(parts.version);
		for (name, value) in &parts.headers {
			req_builder = req_builder.header(name, value);
		}
		let req = req_builder
			.body(crate::http::Body::from(body))
			.map_err(|e| OAuthHttpClientError::BuildRequest(e.to_string()))?;

		let response = client.simple_call(req);
		let response = tokio::time::timeout(OIDC_HTTP_TIMEOUT, response)
			.await
			.map_err(|_| OAuthHttpClientError::Call("request timed out".to_string()))?
			.map_err(|e| OAuthHttpClientError::Call(e.to_string()))?;
		let (parts, body) = response.into_parts();
		let bytes = tokio::time::timeout(
			OIDC_HTTP_TIMEOUT,
			crate::http::read_body_with_limit(body, OIDC_HTTP_RESPONSE_LIMIT),
		)
		.await
		.map_err(|_| OAuthHttpClientError::ReadBody("response body read timed out".to_string()))?
		.map_err(|e| OAuthHttpClientError::ReadBody(e.to_string()))?;

		let mut response_builder = ::http::Response::builder()
			.status(parts.status)
			.version(parts.version);
		for (name, value) in &parts.headers {
			response_builder = response_builder.header(name, value);
		}
		response_builder
			.body(bytes.to_vec())
			.map_err(|e| OAuthHttpClientError::BuildResponse(e.to_string()))
	}

	fn convert_token_response(token_response: OidcOAuth2TokenResponse) -> TokenResponse {
		TokenResponse {
			access_token: token_response.access_token().secret().to_string(),
			token_type: token_response.token_type().as_ref().to_string(),
			expires_in: token_response.expires_in().map(|d| d.as_secs()),
			refresh_token: token_response
				.refresh_token()
				.map(|v| v.secret().to_string()),
			id_token: token_response.extra_fields().id_token.clone(),
		}
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn exchange_code(
		&self,
		client: &Client,
		metadata: &OidcMetadata,
		code: &str,
		client_id: &str,
		client_secret: &str,
		redirect_uri: &str,
		code_verifier: Option<&str>,
	) -> Result<TokenResponse, Error> {
		let oauth_client = Self::oauth2_client(metadata, client_id, client_secret)?;
		let redirect_url = oauth2::RedirectUrl::new(redirect_uri.to_string())
			.map_err(|e| Error::Internal(format!("invalid redirect URI: {e}")))?;
		let oauth_client = oauth_client.set_redirect_uri(redirect_url);
		let mut token_req = oauth_client.exchange_code(AuthorizationCode::new(code.to_string()));

		if let Some(cv) = code_verifier {
			token_req = token_req.set_pkce_verifier(PkceCodeVerifier::new(cv.to_string()));
		}

		let oauth_http_client = |request: OAuth2HttpRequest| {
			let upstream = client.clone();
			async move { Self::oauth_http_call(upstream, request).await }
		};
		let token_response = tokio::time::timeout(
			OIDC_TOKEN_OPERATION_TIMEOUT,
			token_req.request_async(&oauth_http_client),
		)
		.await
		.map_err(|_| Error::Exchange("token exchange timed out".to_string()))?
		.map_err(
			|e: RequestTokenError<OAuthHttpClientError, BasicErrorResponse>| {
				Error::Exchange(e.to_string())
			},
		)?;
		let token_response = Self::convert_token_response(token_response);
		Self::validate_token_type(&token_response)?;
		Ok(token_response)
	}

	pub async fn refresh_token(
		&self,
		client: &Client,
		metadata: &OidcMetadata,
		refresh_token: &str,
		client_id: &str,
		client_secret: &str,
	) -> Result<TokenResponse, Error> {
		let oauth_client = Self::oauth2_client(metadata, client_id, client_secret)?;
		let oauth_http_client = |request: OAuth2HttpRequest| {
			let upstream = client.clone();
			async move { Self::oauth_http_call(upstream, request).await }
		};
		let refresh_token = RefreshToken::new(refresh_token.to_string());
		let refresh_req = oauth_client.exchange_refresh_token(&refresh_token);
		let token_response = tokio::time::timeout(
			OIDC_TOKEN_OPERATION_TIMEOUT,
			refresh_req.request_async(&oauth_http_client),
		)
		.await
		.map_err(|_| Error::Exchange("token refresh timed out".to_string()))?
		.map_err(
			|e: RequestTokenError<OAuthHttpClientError, BasicErrorResponse>| {
				Error::Exchange(e.to_string())
			},
		)?;
		let token_response = Self::convert_token_response(token_response);
		Self::validate_token_type(&token_response)?;
		Ok(token_response)
	}

	fn validate_token_type(token_response: &TokenResponse) -> Result<(), Error> {
		let token_type = token_response.token_type.trim();
		if token_type.eq_ignore_ascii_case("bearer") {
			return Ok(());
		}
		Err(Error::Exchange(format!(
			"unsupported token_type '{token_type}', expected Bearer"
		)))
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use serde_json::json;
	use tokio::task::JoinSet;
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::client;

	fn make_test_client() -> crate::client::Client {
		let cfg = client::Config {
			resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
			resolver_opts: hickory_resolver::config::ResolverOpts::default(),
		};
		crate::client::Client::new(
			&cfg,
			None,
			Default::default(),
			None,
			Arc::new(OidcProvider::new()),
		)
	}

	fn jwks_fixture() -> serde_json::Value {
		json!({
			"keys": [
				{
					"use": "sig",
					"kty": "EC",
					"kid": "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI",
					"crv": "P-256",
					"alg": "ES256",
					"x": "XZHF8Em5LbpqfgewAalpSEH4Ka2I2xjcxxUt2j6-lCo",
					"y": "g3DFz45A7EOUMgmsNXatrXw1t-PG5xsbkxUs851RxSE"
				}
			]
		})
	}

	#[tokio::test]
	async fn metadata_fetch_is_singleflight_per_issuer() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": format!("{issuer}/jwks"),
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(metadata))
			.expect(1)
			.mount(&server)
			.await;

		let provider = Arc::new(OidcProvider::new());
		let client = make_test_client();

		let mut set = JoinSet::new();
		for _ in 0..16 {
			let provider = provider.clone();
			let client = client.clone();
			let issuer = issuer.clone();
			set.spawn(async move { provider.get_metadata(&client, &issuer).await });
		}

		while let Some(res) = set.join_next().await {
			let metadata = res.expect("task join").expect("metadata fetch");
			assert_eq!(metadata.jwks_uri, format!("{issuer}/jwks"));
		}
	}

	#[tokio::test]
	async fn validator_fetch_is_singleflight_per_key() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": format!("{issuer}/jwks"),
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(metadata))
			.expect(1)
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.and(path("/jwks"))
			.respond_with(ResponseTemplate::new(200).set_body_json(jwks_fixture()))
			.expect(1)
			.mount(&server)
			.await;

		let provider = Arc::new(OidcProvider::new());
		let client = make_test_client();

		let mut set = JoinSet::new();
		for _ in 0..16 {
			let provider = provider.clone();
			let client = client.clone();
			let issuer = issuer.clone();
			set.spawn(async move {
				provider
					.get_info(&client, &issuer, Some(vec!["test-aud".to_string()]))
					.await
			});
		}

		while let Some(res) = set.join_next().await {
			let (_metadata, _validator) = res.expect("task join").expect("get_info");
		}
	}

	#[tokio::test]
	async fn metadata_cache_refreshes_after_ttl() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": format!("{issuer}/jwks"),
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(metadata))
			.expect(2)
			.mount(&server)
			.await;

		let provider = OidcProvider::new();
		let client = make_test_client();

		let first = provider
			.get_metadata(&client, &issuer)
			.await
			.expect("first metadata fetch");
		assert_eq!(first.jwks_uri, format!("{issuer}/jwks"));

		{
			let mut cache = provider.metadata_cache.write().await;
			let entry = cache.get_mut(&issuer).expect("metadata cache entry");
			entry.fetched_at = Instant::now() - METADATA_TTL - Duration::from_secs(1);
		}

		let second = provider
			.get_metadata(&client, &issuer)
			.await
			.expect("metadata fetch after ttl");
		assert_eq!(second.jwks_uri, format!("{issuer}/jwks"));
	}

	#[tokio::test]
	async fn inflight_metadata_keys_are_cleaned_after_request() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": format!("{issuer}/jwks"),
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(metadata))
			.expect(1)
			.mount(&server)
			.await;

		let provider = OidcProvider::new();
		let client = make_test_client();
		provider
			.get_metadata(&client, &issuer)
			.await
			.expect("metadata fetch");

		let inflight = provider.metadata_inflight.lock().await;
		assert!(
			inflight.is_empty(),
			"metadata inflight map should be cleaned"
		);
	}

	#[tokio::test]
	async fn inflight_validator_keys_are_cleaned_after_request() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": format!("{issuer}/jwks"),
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(metadata))
			.expect(1)
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.and(path("/jwks"))
			.respond_with(ResponseTemplate::new(200).set_body_json(jwks_fixture()))
			.expect(1)
			.mount(&server)
			.await;

		let provider = OidcProvider::new();
		let client = make_test_client();
		provider
			.get_info(&client, &issuer, Some(vec!["aud".to_string()]))
			.await
			.expect("oidc info fetch");

		let inflight = provider.validator_inflight.lock().await;
		assert!(
			inflight.is_empty(),
			"validator inflight map should be cleaned"
		);
	}
}
