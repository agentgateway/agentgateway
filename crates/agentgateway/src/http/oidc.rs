use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_singleflight::UnaryGroup;
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
use tokio::sync::RwLock;
use tracing::debug;
use url::Url;

use crate::client::Client;
use crate::http::jwt::{
	JWTValidationOptions, Jwt, Mode as JwtMode, Provider as JwtProvider, TokenError,
};
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::SimpleBackendReference;
use crate::types::discovery::NamespacedHostname;

type SharedError = Arc<Error>;
type SharedResult<T> = Result<T, SharedError>;
type MetadataSingleflight = Arc<UnaryGroup<MetadataCacheKey, SharedResult<Arc<OidcMetadata>>>>;
type ValidatorSingleflight = Arc<UnaryGroup<ValidatorCacheKey, SharedResult<Arc<Jwt>>>>;
type ExchangeSingleflight = Arc<UnaryGroup<ExchangeCacheKey, SharedResult<TokenResponse>>>;
type RefreshSingleflight = Arc<UnaryGroup<RefreshCacheKey, SharedResult<TokenResponse>>>;

#[derive(Clone, Debug, thiserror::Error)]
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
	// Metadata cache key is issuer + provider backend context.
	metadata_cache: RwLock<HashMap<MetadataCacheKey, CachedMetadata>>,
	// Per (issuer + provider backend) singleflight work gate for metadata discovery.
	metadata_singleflight: MetadataSingleflight,
	// Validators are specific to issuer + audiences + provider backend context.
	validator_cache: RwLock<HashMap<ValidatorCacheKey, CachedValidator>>,
	// Per (issuer + audiences + provider backend) singleflight work gate for JWKS/validator refresh.
	validator_singleflight: ValidatorSingleflight,
	// Per (token endpoint + client + code + code verifier + redirect URI + provider backend)
	// singleflight work gate for authorization-code exchange.
	exchange_singleflight: ExchangeSingleflight,
	// Short-lived cache for successful authorization-code exchanges.
	// This smooths over immediate duplicate callback requests that race after the first success.
	exchange_result_cache: RwLock<HashMap<ExchangeCacheKey, CachedExchangeResult>>,
	// Per (token endpoint + client + refresh token + provider backend) singleflight work gate
	// for refresh-token exchange.
	refresh_singleflight: RefreshSingleflight,
}

#[derive(Debug, Clone)]
struct CachedValidator {
	validator: Arc<Jwt>,
	last_refresh: Instant,
	last_refresh_forced: bool,
}

#[derive(Debug, Clone)]
struct CachedMetadata {
	metadata: Arc<OidcMetadata>,
	fetched_at: Instant,
}

#[derive(Debug, Clone)]
struct CachedExchangeResult {
	token: TokenResponse,
	exchanged_at: Instant,
}

mod cache_keys {
	use super::*;
	use aws_lc_rs::digest;

	#[derive(Debug, Clone, Eq, PartialEq, Hash)]
	pub(crate) enum ProviderBackendCacheKey {
		None,
		Service { name: NamespacedHostname, port: u16 },
		Backend(crate::types::agent::BackendKey),
		InlineBackend(crate::types::agent::Target),
		Invalid,
	}

	impl ProviderBackendCacheKey {
		pub(crate) fn from_ref(provider_backend: Option<&SimpleBackendReference>) -> Self {
			match provider_backend {
				None => Self::None,
				Some(SimpleBackendReference::Service { name, port }) => Self::Service {
					name: name.clone(),
					port: *port,
				},
				Some(SimpleBackendReference::Backend(name)) => Self::Backend(name.clone()),
				Some(SimpleBackendReference::InlineBackend(target)) => Self::InlineBackend(target.clone()),
				Some(SimpleBackendReference::Invalid) => Self::Invalid,
			}
		}
	}

	impl Display for ProviderBackendCacheKey {
		fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
			match self {
				Self::None => write!(f, "none"),
				Self::Service { name, port } => write!(f, "service:{name}:{port}"),
				Self::Backend(name) => write!(f, "backend:{name}"),
				Self::InlineBackend(target) => write!(f, "inline:{target}"),
				Self::Invalid => write!(f, "invalid"),
			}
		}
	}

	#[derive(Debug, Clone, Eq, PartialEq, Hash)]
	pub(crate) struct MetadataCacheKey {
		pub(crate) issuer: String,
		pub(crate) provider_backend: ProviderBackendCacheKey,
	}

	impl MetadataCacheKey {
		pub(crate) fn new(issuer: &str, provider_backend: Option<&SimpleBackendReference>) -> Self {
			Self {
				issuer: issuer.to_string(),
				provider_backend: ProviderBackendCacheKey::from_ref(provider_backend),
			}
		}
	}

	impl Display for MetadataCacheKey {
		fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
			write!(
				f,
				"issuer={},provider_backend={}",
				self.issuer, self.provider_backend
			)
		}
	}

	#[derive(Debug, Clone, Eq, PartialEq, Hash)]
	pub(crate) struct ValidatorCacheKey {
		pub(crate) metadata: MetadataCacheKey,
		pub(crate) audiences: Vec<String>,
	}

	impl ValidatorCacheKey {
		pub(crate) fn new(
			issuer: &str,
			provider_backend: Option<&SimpleBackendReference>,
			audiences: Option<&[String]>,
		) -> Self {
			Self {
				metadata: MetadataCacheKey::new(issuer, provider_backend),
				audiences: normalize_audiences(audiences),
			}
		}
	}

	impl Display for ValidatorCacheKey {
		fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
			write!(
				f,
				"{},audiences=[{}]",
				self.metadata,
				self.audiences.join(",")
			)
		}
	}

	pub(crate) fn normalize_audiences(audiences: Option<&[String]>) -> Vec<String> {
		let mut audiences = audiences.map_or_else(Vec::new, |values| values.to_vec());
		audiences.sort_unstable();
		audiences.dedup();
		audiences
	}

	#[derive(Debug, Clone, Eq, PartialEq, Hash)]
	pub(crate) struct ExchangeCacheKey {
		pub(crate) token_endpoint: String,
		pub(crate) client_id: String,
		pub(crate) code_hash: [u8; 32],
		pub(crate) code_verifier_hash: Option<[u8; 32]>,
		pub(crate) redirect_uri: String,
		pub(crate) provider_backend: ProviderBackendCacheKey,
	}

	fn hash_sensitive(value: &str) -> [u8; 32] {
		let digest = digest::digest(&digest::SHA256, value.as_bytes());
		let mut out = [0u8; 32];
		out.copy_from_slice(digest.as_ref());
		out
	}

	impl ExchangeCacheKey {
		pub(crate) fn new(
			token_endpoint: &str,
			client_id: &str,
			code: &str,
			code_verifier: Option<&str>,
			redirect_uri: &str,
			provider_backend: Option<&SimpleBackendReference>,
		) -> Self {
			Self {
				token_endpoint: token_endpoint.to_string(),
				client_id: client_id.to_string(),
				code_hash: hash_sensitive(code),
				code_verifier_hash: code_verifier.map(hash_sensitive),
				redirect_uri: redirect_uri.to_string(),
				provider_backend: ProviderBackendCacheKey::from_ref(provider_backend),
			}
		}
	}

	#[derive(Debug, Clone, Eq, PartialEq, Hash)]
	pub(crate) struct RefreshCacheKey {
		pub(crate) token_endpoint: String,
		pub(crate) client_id: String,
		pub(crate) refresh_token: String,
		pub(crate) provider_backend: ProviderBackendCacheKey,
	}

	impl RefreshCacheKey {
		pub(crate) fn new(
			token_endpoint: &str,
			client_id: &str,
			refresh_token: &str,
			provider_backend: Option<&SimpleBackendReference>,
		) -> Self {
			Self {
				token_endpoint: token_endpoint.to_string(),
				client_id: client_id.to_string(),
				refresh_token: refresh_token.to_string(),
				provider_backend: ProviderBackendCacheKey::from_ref(provider_backend),
			}
		}
	}
}

use cache_keys::{ExchangeCacheKey, MetadataCacheKey, RefreshCacheKey, ValidatorCacheKey};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OidcMetadata {
	pub authorization_endpoint: String,
	pub token_endpoint: String,
	pub jwks_uri: Option<String>,
	#[serde(default)]
	pub end_session_endpoint: Option<String>,
	#[serde(default)]
	pub token_endpoint_auth_methods_supported: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
	pub access_token: String,
	pub token_type: String,
	pub expires_in: Option<u64>,
	pub refresh_token: Option<String>,
	pub id_token: Option<String>,
}

#[derive(Clone, Copy)]
pub struct OidcCallContext<'a> {
	pub client: &'a Client,
	pub policy_client: Option<&'a PolicyClient>,
	pub provider_backend: Option<&'a SimpleBackendReference>,
}

impl<'a> OidcCallContext<'a> {
	pub fn new(
		client: &'a Client,
		policy_client: Option<&'a PolicyClient>,
		provider_backend: Option<&'a SimpleBackendReference>,
	) -> Self {
		Self {
			client,
			policy_client,
			provider_backend,
		}
	}
}

impl MetadataCacheKey {
	fn from_ctx(issuer: &str, ctx: OidcCallContext<'_>) -> Self {
		Self::new(issuer, ctx.provider_backend)
	}
}

impl ValidatorCacheKey {
	fn from_ctx(issuer: &str, ctx: OidcCallContext<'_>, audiences: Option<&[String]>) -> Self {
		Self::new(issuer, ctx.provider_backend, audiences)
	}
}

#[derive(Clone)]
struct OidcTransportContext {
	client: Client,
	policy_client: Option<PolicyClient>,
	provider_backend: Option<SimpleBackendReference>,
}

impl OidcTransportContext {
	fn from_call_context(ctx: OidcCallContext<'_>) -> Self {
		Self {
			client: ctx.client.clone(),
			policy_client: ctx.policy_client.cloned(),
			provider_backend: ctx.provider_backend.cloned(),
		}
	}
}

pub struct ExchangeCodeRequest<'a> {
	pub metadata: &'a OidcMetadata,
	pub code: &'a str,
	pub client_id: &'a str,
	pub client_secret: &'a str,
	pub redirect_uri: &'a str,
	pub code_verifier: Option<&'a str>,
}

pub struct RefreshTokenRequest<'a> {
	pub metadata: &'a OidcMetadata,
	pub refresh_token: &'a str,
	pub client_id: &'a str,
	pub client_secret: &'a str,
}

const FORCE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const METADATA_TTL: Duration = Duration::from_secs(300);
const VALIDATOR_TTL: Duration = Duration::from_secs(300);
const OIDC_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const OIDC_TOKEN_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const OIDC_HTTP_RESPONSE_LIMIT: usize = 2_097_152;
const EXCHANGE_RESULT_TTL: Duration = Duration::from_secs(10);
const EXCHANGE_RESULT_CACHE_MAX_ENTRIES: usize = 256;

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
			metadata_singleflight: Arc::new(UnaryGroup::new()),
			validator_cache: RwLock::new(HashMap::new()),
			validator_singleflight: Arc::new(UnaryGroup::new()),
			exchange_singleflight: Arc::new(UnaryGroup::new()),
			exchange_result_cache: RwLock::new(HashMap::new()),
			refresh_singleflight: Arc::new(UnaryGroup::new()),
		}
	}

	fn validate_issuer_url(issuer: &str) -> Result<(), Error> {
		let parsed =
			Url::parse(issuer).map_err(|e| Error::Discovery(format!("invalid issuer URL: {e}")))?;
		if parsed.fragment().is_some() || parsed.query().is_some() {
			return Err(Error::Discovery(
				"issuer must not contain query or fragment".to_string(),
			));
		}
		if !parsed.username().is_empty() || parsed.password().is_some() {
			return Err(Error::Discovery(
				"issuer must not include userinfo".to_string(),
			));
		}
		let host = parsed
			.host_str()
			.ok_or_else(|| Error::Discovery("issuer is missing host".to_string()))?;
		match parsed.scheme() {
			"https" => Ok(()),
			"http" if crate::http::is_loopback_host(host) => Ok(()),
			_ => Err(Error::Discovery(
				"issuer must use https (or http on loopback hosts)".to_string(),
			)),
		}
	}

	fn validate_endpoint_url(endpoint: &str, field: &str) -> Result<(), Error> {
		let parsed = Url::parse(endpoint)
			.map_err(|e| Error::Discovery(format!("invalid {field} URL in discovery metadata: {e}")))?;
		if parsed.fragment().is_some() || !parsed.username().is_empty() || parsed.password().is_some() {
			return Err(Error::Discovery(format!(
				"{field} must not contain fragment or userinfo"
			)));
		}
		let host = parsed
			.host_str()
			.ok_or_else(|| Error::Discovery(format!("{field} is missing host")))?;
		match parsed.scheme() {
			"https" => Ok(()),
			"http" if crate::http::is_loopback_host(host) => Ok(()),
			_ => Err(Error::Discovery(format!(
				"{field} must use https (or http on loopback hosts)"
			))),
		}
	}

	fn validate_metadata_endpoints(metadata: &OidcMetadata) -> Result<(), Error> {
		// Discovery metadata must stay HTTPS-only (aside from local loopback),
		// independent of provider backend routing.
		Self::validate_endpoint_url(&metadata.authorization_endpoint, "authorization_endpoint")?;
		Self::validate_endpoint_url(&metadata.token_endpoint, "token_endpoint")?;
		Self::validate_endpoint_url(
			metadata
				.jwks_uri
				.as_deref()
				.ok_or_else(|| Error::Discovery("jwks_uri is missing".to_string()))?,
			"jwks_uri",
		)?;
		if let Some(end_session_endpoint) = metadata.end_session_endpoint.as_deref() {
			Self::validate_endpoint_url(end_session_endpoint, "end_session_endpoint")?;
		}
		Ok(())
	}

	fn normalized_audiences(audiences: Option<Vec<String>>) -> Option<Vec<String>> {
		let audiences = cache_keys::normalize_audiences(audiences.as_deref());
		if audiences.is_empty() {
			None
		} else {
			Some(audiences)
		}
	}

	async fn call_oidc_endpoint(
		ctx: OidcCallContext<'_>,
		req: crate::http::Request,
	) -> Result<crate::http::Response, Error> {
		match ctx.provider_backend {
			Some(backend_ref) => {
				let policy_client = ctx.policy_client.ok_or_else(|| {
					Error::Internal("provider_backend requires policy client context".to_string())
				})?;
				policy_client
					.call_oidc_provider(req, backend_ref)
					.await
					.map_err(|e| Error::Discovery(e.to_string()))
			},
			None => ctx
				.client
				.simple_call(req)
				.await
				.map_err(|e| Error::Discovery(e.to_string())),
		}
	}

	pub async fn get_info(
		&self,
		ctx: OidcCallContext<'_>,
		issuer: &str,
		audiences: Option<Vec<String>>,
	) -> Result<(Arc<OidcMetadata>, Arc<Jwt>), Error> {
		let audiences = Self::normalized_audiences(audiences);
		let metadata = self.get_metadata(ctx, issuer).await?;
		let validator = self
			.get_validator(ctx, issuer, audiences, &metadata, false)
			.await?;
		Ok((metadata, validator))
	}

	/// Validates a token, attempting a JWKS refresh if the key is unknown.
	pub async fn validate_token(
		&self,
		ctx: OidcCallContext<'_>,
		issuer: &str,
		audiences: Option<Vec<String>>,
		token: &str,
	) -> Result<crate::http::jwt::Claims, Error> {
		let audiences = Self::normalized_audiences(audiences);
		let (metadata, validator) = self.get_info(ctx, issuer, audiences.clone()).await?;

		match validator.validate_claims(token) {
			Ok(claims) => Ok(claims),
			Err(TokenError::UnknownKeyId(_)) => {
				// Potential key rotation. Try refreshing JWKS.
				debug!(
					"Unknown key ID in token, attempting JWKS refresh for {}",
					issuer
				);
				let validator = self
					.get_validator(ctx, issuer, audiences, &metadata, true)
					.await?;
				Ok(validator.validate_claims(token)?)
			},
			Err(e) => Err(e.into()),
		}
	}

	pub async fn get_metadata(
		&self,
		ctx: OidcCallContext<'_>,
		issuer: &str,
	) -> Result<Arc<OidcMetadata>, Error> {
		Self::validate_issuer_url(issuer)?;
		let cache_key = MetadataCacheKey::from_ctx(issuer, ctx);
		// 1. Fast Path: Read Lock
		{
			if let Some(entry) = self.metadata_cache.read().await.get(&cache_key)
				&& entry.fetched_at.elapsed() < METADATA_TTL
			{
				return Ok(entry.metadata.clone());
			}
		} // Drop read lock

		// 2. Singleflight: ensure only one metadata fetch per cache key at a time.
		let shared_result = self
			.metadata_singleflight
			.work(&cache_key, async {
				let result: Result<Arc<OidcMetadata>, Error> = async {
					// 3. Re-check cache after waiting for another in-flight fetch.
					{
						if let Some(entry) = self.metadata_cache.read().await.get(&cache_key)
							&& entry.fetched_at.elapsed() < METADATA_TTL
						{
							return Ok(entry.metadata.clone());
						}
					}

					// 4. Slow Path: Network Call (singleflight work gate is held for this issuer)
					let url = format!(
						"{}/.well-known/openid-configuration",
						issuer.trim_end_matches('/')
					);
					let req = ::http::Request::builder()
						.uri(&url)
						.body(crate::http::Body::empty())
						.map_err(|e| Error::Internal(e.to_string()))?;
					let resp = tokio::time::timeout(OIDC_HTTP_TIMEOUT, Self::call_oidc_endpoint(ctx, req))
						.await
						.map_err(|_| {
							Error::Discovery(
								"oidc discovery request timed out while fetching metadata".to_string(),
							)
						})?
						.map_err(|e| Error::Discovery(e.to_string()))?;
					let metadata: OidcMetadata =
						tokio::time::timeout(OIDC_HTTP_TIMEOUT, crate::json::from_response_body(resp))
							.await
							.map_err(|_| {
								Error::Discovery("oidc metadata response body read timed out".to_string())
							})?
							.map_err(|e| Error::Discovery(e.to_string()))?;
					Self::validate_metadata_endpoints(&metadata)?;
					let metadata = Arc::new(metadata);

					// 5. Write Path: Update Cache
					let mut w = self.metadata_cache.write().await;
					// Optimization: In a thundering herd scenario, someone else might have updated it while we were fetching.
					if let Some(entry) = w.get(&cache_key)
						&& entry.fetched_at.elapsed() < METADATA_TTL
					{
						return Ok(entry.metadata.clone());
					}
					w.insert(
						cache_key.clone(),
						CachedMetadata {
							metadata: metadata.clone(),
							fetched_at: Instant::now(),
						},
					);
					Ok(metadata)
				}
				.await;

				result.map_err(Arc::new)
			})
			.await;

		shared_result.map_err(|err| err.as_ref().clone())
	}

	/// Returns cached metadata for an issuer, if present, without triggering network fetches.
	///
	/// This is useful on best-effort paths (for example logout) where we prefer low-latency
	/// behavior over blocking on discovery.
	pub async fn get_cached_metadata(
		&self,
		issuer: &str,
		provider_backend: Option<&SimpleBackendReference>,
	) -> Option<Arc<OidcMetadata>> {
		let cache_key = MetadataCacheKey::new(issuer, provider_backend);
		self
			.metadata_cache
			.read()
			.await
			.get(&cache_key)
			.map(|entry| entry.metadata.clone())
	}

	async fn get_validator(
		&self,
		ctx: OidcCallContext<'_>,
		issuer: &str,
		audiences: Option<Vec<String>>,
		metadata: &OidcMetadata,
		force_refresh: bool,
	) -> Result<Arc<Jwt>, Error> {
		let key = ValidatorCacheKey::from_ctx(issuer, ctx, audiences.as_deref());

		// 1. Fast Path: Read Lock
		{
			let cache = self.validator_cache.read().await;
			if let Some(entry) = cache.get(&key) {
				if !force_refresh && entry.last_refresh.elapsed() < VALIDATOR_TTL {
					return Ok(Arc::clone(&entry.validator));
				}
				// Throttle only repeated forced refreshes (e.g. kid-spray),
				// while allowing an immediate first forced refresh after a normal cache fill.
				if entry.last_refresh_forced && entry.last_refresh.elapsed() < FORCE_REFRESH_INTERVAL {
					debug!(
						"Skipping JWKS refresh for {}, already refreshed very recently",
						issuer
					);
					return Ok(Arc::clone(&entry.validator));
				}
			}
		} // Drop read lock

		// 2. Singleflight: ensure only one JWKS fetch per cache key at a time.
		let work_key = key.clone();
		let key_for_load = key.clone();
		let shared_result = self
			.validator_singleflight
			.work(&work_key, async move {
				let result: Result<Arc<Jwt>, Error> = async {
					// 3. Re-check cache after waiting for another in-flight fetch.
					{
						let cache = self.validator_cache.read().await;
						if let Some(entry) = cache.get(&key_for_load) {
							if !force_refresh && entry.last_refresh.elapsed() < VALIDATOR_TTL {
								return Ok(Arc::clone(&entry.validator));
							}
							if entry.last_refresh_forced && entry.last_refresh.elapsed() < FORCE_REFRESH_INTERVAL
							{
								debug!(
									"Skipping JWKS refresh for {}, already refreshed very recently",
									issuer
								);
								return Ok(Arc::clone(&entry.validator));
							}
						}
					}

					// 4. Slow Path: Network Call (singleflight work gate is held for this cache key)
					// Initialize Jwt validator using the discovered jwks_uri
					let jwks_req = ::http::Request::builder()
						.uri(
							metadata
								.jwks_uri
								.as_deref()
								.ok_or_else(|| Error::Discovery("jwks_uri is missing".to_string()))?,
						)
						.body(crate::http::Body::empty())
						.map_err(|e| Error::Internal(e.to_string()))?;
					let jwks_resp =
						tokio::time::timeout(OIDC_HTTP_TIMEOUT, Self::call_oidc_endpoint(ctx, jwks_req))
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

					let provider = JwtProvider::from_jwks(
						jwk_set,
						issuer.to_string(),
						audiences,
						JWTValidationOptions::default(),
					)
					.map_err(|e| Error::Internal(format!("failed to create JWT provider: {e}")))?;

					let jwt = Arc::new(Jwt::from_providers(vec![provider], JwtMode::Strict));

					// 5. Write Path: Update Cache
					let mut w = self.validator_cache.write().await;
					// Optimization: In a thundering herd scenario, someone else might have updated it while we were fetching.
					// Overwriting with a fresh validator is safe.
					w.insert(
						key_for_load,
						CachedValidator {
							validator: Arc::clone(&jwt),
							last_refresh: Instant::now(),
							last_refresh_forced: force_refresh,
						},
					);

					Ok(jwt)
				}
				.await;

				result.map_err(Arc::new)
			})
			.await;

		shared_result.map_err(|err| err.as_ref().clone())
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
		transport: OidcTransportContext,
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

		let response = if let Some(backend_ref) = transport.provider_backend.as_ref() {
			let policy_client = transport.policy_client.as_ref().ok_or_else(|| {
				OAuthHttpClientError::Call("provider_backend requires policy client context".to_string())
			})?;
			tokio::time::timeout(
				OIDC_HTTP_TIMEOUT,
				policy_client.call_oidc_provider(req, backend_ref),
			)
			.await
			.map_err(|_| OAuthHttpClientError::Call("request timed out".to_string()))?
			.map_err(|e| OAuthHttpClientError::Call(e.to_string()))?
		} else {
			tokio::time::timeout(OIDC_HTTP_TIMEOUT, transport.client.simple_call(req))
				.await
				.map_err(|_| OAuthHttpClientError::Call("request timed out".to_string()))?
				.map_err(|e| OAuthHttpClientError::Call(e.to_string()))?
		};
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

	async fn get_cached_exchange_result(&self, key: &ExchangeCacheKey) -> Option<TokenResponse> {
		let now = Instant::now();
		let cache = self.exchange_result_cache.read().await;
		cache.get(key).and_then(|entry| {
			(now.duration_since(entry.exchanged_at) <= EXCHANGE_RESULT_TTL).then(|| entry.token.clone())
		})
	}

	async fn put_cached_exchange_result(&self, key: ExchangeCacheKey, token: TokenResponse) {
		let now = Instant::now();
		let mut cache = self.exchange_result_cache.write().await;
		cache.retain(|_, entry| now.duration_since(entry.exchanged_at) <= EXCHANGE_RESULT_TTL);
		if cache.len() >= EXCHANGE_RESULT_CACHE_MAX_ENTRIES
			&& let Some(oldest_key) = cache
				.iter()
				.min_by_key(|(_, entry)| entry.exchanged_at)
				.map(|(existing_key, _)| existing_key.clone())
		{
			cache.remove(&oldest_key);
		}
		cache.insert(
			key,
			CachedExchangeResult {
				token,
				exchanged_at: now,
			},
		);
	}

	pub async fn exchange_code(
		&self,
		ctx: OidcCallContext<'_>,
		req: ExchangeCodeRequest<'_>,
	) -> Result<TokenResponse, Error> {
		let key = ExchangeCacheKey::new(
			&req.metadata.token_endpoint,
			req.client_id,
			req.code,
			req.code_verifier,
			req.redirect_uri,
			ctx.provider_backend,
		);
		if let Some(token) = self.get_cached_exchange_result(&key).await {
			return Ok(token);
		}
		let cache_key = key.clone();

		let shared_result = self
			.exchange_singleflight
			.work(&key, async {
				let result: Result<TokenResponse, Error> = async {
					let oauth_client = Self::oauth2_client(req.metadata, req.client_id, req.client_secret)?;
					let redirect_url = oauth2::RedirectUrl::new(req.redirect_uri.to_string())
						.map_err(|e| Error::Internal(format!("invalid redirect URI: {e}")))?;
					let oauth_client = oauth_client.set_redirect_uri(redirect_url);
					let mut token_req =
						oauth_client.exchange_code(AuthorizationCode::new(req.code.to_string()));

					if let Some(cv) = req.code_verifier {
						token_req = token_req.set_pkce_verifier(PkceCodeVerifier::new(cv.to_string()));
					}

					let transport = OidcTransportContext::from_call_context(ctx);
					let oauth_http_client = |request: OAuth2HttpRequest| {
						let transport = transport.clone();
						async move { Self::oauth_http_call(transport, request).await }
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
				.await;

				match result {
					Ok(token) => {
						self
							.put_cached_exchange_result(cache_key.clone(), token.clone())
							.await;
						Ok(token)
					},
					Err(err) => Err(Arc::new(err)),
				}
			})
			.await;

		shared_result.map_err(|err| err.as_ref().clone())
	}

	pub async fn refresh_token(
		&self,
		ctx: OidcCallContext<'_>,
		req: RefreshTokenRequest<'_>,
	) -> Result<TokenResponse, Error> {
		let key = RefreshCacheKey::new(
			&req.metadata.token_endpoint,
			req.client_id,
			req.refresh_token,
			ctx.provider_backend,
		);

		let shared_result = self
			.refresh_singleflight
			.work(&key, async {
				let result: Result<TokenResponse, Error> = async {
					let oauth_client = Self::oauth2_client(req.metadata, req.client_id, req.client_secret)?;
					let transport = OidcTransportContext::from_call_context(ctx);
					let oauth_http_client = |request: OAuth2HttpRequest| {
						let transport = transport.clone();
						async move { Self::oauth_http_call(transport, request).await }
					};
					let refresh_token = RefreshToken::new(req.refresh_token.to_string());
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
				.await;

				result.map_err(Arc::new)
			})
			.await;

		shared_result.map_err(|err| err.as_ref().clone())
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
	use std::time::Duration;

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
		crate::client::Client::new(&cfg, None, Default::default(), None)
	}

	fn test_ctx(client: &crate::client::Client) -> OidcCallContext<'_> {
		OidcCallContext::new(client, None, None)
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

	#[test]
	fn validator_cache_key_is_order_insensitive_for_audiences() {
		let issuer = "https://issuer.example.com";
		let canonical = ValidatorCacheKey::new(
			issuer,
			None,
			Some(&["aud-a".to_string(), "aud-b".to_string()]),
		);
		let permutations = [
			vec!["aud-b".to_string(), "aud-a".to_string()],
			vec!["aud-a".to_string(), "aud-b".to_string()],
			vec![
				"aud-b".to_string(),
				"aud-a".to_string(),
				"aud-b".to_string(),
			],
		];

		for auds in permutations {
			let key = ValidatorCacheKey::new(issuer, None, Some(&auds));
			assert_eq!(key, canonical);
		}
	}

	#[test]
	fn validator_cache_key_dedups_audiences() {
		let issuer = "https://issuer.example.com";
		let input = vec![
			"aud-z".to_string(),
			"aud-a".to_string(),
			"aud-z".to_string(),
			"aud-a".to_string(),
		];
		let key = ValidatorCacheKey::new(issuer, None, Some(&input));
		assert_eq!(
			key.audiences,
			vec!["aud-a".to_string(), "aud-z".to_string()]
		);
	}

	#[test]
	fn metadata_cache_key_separates_backend_variants() {
		let issuer = "https://issuer.example.com";
		let backend_ref = SimpleBackendReference::Backend("backend-a".into());
		let inline_ref = SimpleBackendReference::InlineBackend(crate::types::agent::Target::Hostname(
			"backend-a".into(),
			443,
		));

		let backend_key = MetadataCacheKey::new(issuer, Some(&backend_ref));
		let inline_key = MetadataCacheKey::new(issuer, Some(&inline_ref));
		assert_ne!(backend_key, inline_key);
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
			set.spawn(async move { provider.get_metadata(test_ctx(&client), &issuer).await });
		}

		while let Some(res) = set.join_next().await {
			let metadata = res.expect("task join").expect("metadata fetch");
			assert_eq!(
				metadata.jwks_uri.as_deref(),
				Some(format!("{issuer}/jwks").as_str())
			);
		}
	}

	#[tokio::test]
	async fn metadata_parses_end_session_endpoint_when_present() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": format!("{issuer}/jwks"),
			"end_session_endpoint": format!("{issuer}/logout")
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(metadata))
			.expect(1)
			.mount(&server)
			.await;

		let provider = OidcProvider::new();
		let client = make_test_client();
		let discovered = provider
			.get_metadata(test_ctx(&client), &issuer)
			.await
			.expect("metadata fetch");
		assert_eq!(
			discovered.end_session_endpoint.as_deref(),
			Some(format!("{issuer}/logout").as_str())
		);
	}

	#[tokio::test]
	async fn metadata_rejects_non_https_jwks_uri_for_non_loopback_hosts() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": "http://evil.example.com/jwks",
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(metadata))
			.expect(1)
			.mount(&server)
			.await;

		let provider = OidcProvider::new();
		let client = make_test_client();
		let err = provider
			.get_metadata(test_ctx(&client), &issuer)
			.await
			.expect_err("non-https jwks_uri should be rejected");
		assert!(
			err.to_string().contains("jwks_uri must use https"),
			"unexpected error: {err}"
		);
	}

	#[tokio::test]
	async fn metadata_rejects_non_https_token_endpoint_for_non_loopback_hosts() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": "http://evil.example.com/token",
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
		let err = provider
			.get_metadata(test_ctx(&client), &issuer)
			.await
			.expect_err("non-https token_endpoint should be rejected");
		assert!(
			err.to_string().contains("token_endpoint must use https"),
			"unexpected error: {err}"
		);
	}

	#[tokio::test]
	async fn metadata_rejects_non_https_issuer_for_non_loopback_hosts() {
		let provider = OidcProvider::new();
		let client = make_test_client();
		let err = provider
			.get_metadata(test_ctx(&client), "http://evil.example.com")
			.await
			.expect_err("non-https issuer should be rejected");
		assert!(
			err.to_string().contains("issuer must use https"),
			"unexpected error: {err}"
		);
	}

	#[tokio::test]
	async fn get_cached_metadata_returns_none_when_absent() {
		let provider = OidcProvider::new();
		let cached = provider
			.get_cached_metadata("https://issuer.example.com", None)
			.await;
		assert!(cached.is_none());
	}

	#[tokio::test]
	async fn get_cached_metadata_returns_value_after_fetch() {
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
		let fetched = provider
			.get_metadata(test_ctx(&client), &issuer)
			.await
			.expect("metadata fetch");
		let cached = provider
			.get_cached_metadata(&issuer, None)
			.await
			.expect("cached metadata should exist after fetch");
		assert_eq!(
			cached.authorization_endpoint,
			fetched.authorization_endpoint
		);
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
					.get_info(
						test_ctx(&client),
						&issuer,
						Some(vec!["test-aud".to_string()]),
					)
					.await
			});
		}

		while let Some(res) = set.join_next().await {
			let (_metadata, _validator) = res.expect("task join").expect("get_info");
		}
	}

	#[tokio::test]
	async fn exchange_code_is_singleflight_per_key() {
		let server = MockServer::start().await;
		let token_response = json!({
			"access_token": "access-token",
			"token_type": "Bearer",
			"expires_in": 3600
		});

		Mock::given(method("POST"))
			.and(path("/token"))
			.respond_with(
				ResponseTemplate::new(200)
					.set_delay(Duration::from_millis(150))
					.set_body_json(token_response),
			)
			.expect(1)
			.mount(&server)
			.await;

		let provider = Arc::new(OidcProvider::new());
		let client = make_test_client();
		let metadata = OidcMetadata {
			authorization_endpoint: format!("{}/authorize", server.uri()),
			token_endpoint: format!("{}/token", server.uri()),
			jwks_uri: None,
			end_session_endpoint: None,
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		};

		let mut set = JoinSet::new();
		for _ in 0..16 {
			let provider = provider.clone();
			let client = client.clone();
			let metadata = metadata.clone();
			set.spawn(async move {
				provider
					.exchange_code(
						test_ctx(&client),
						ExchangeCodeRequest {
							metadata: &metadata,
							code: "code-123",
							client_id: "client-123",
							client_secret: "secret-123",
							redirect_uri: "http://localhost:3000/oauth2/callback",
							code_verifier: Some("verifier-123"),
						},
					)
					.await
			});
		}

		while let Some(res) = set.join_next().await {
			let token = res.expect("task join").expect("token exchange");
			assert_eq!(token.access_token, "access-token");
		}
	}

	#[tokio::test]
	async fn exchange_code_replay_uses_recent_success_cache() {
		let server = MockServer::start().await;
		let token_response = json!({
			"access_token": "access-token",
			"token_type": "Bearer",
			"expires_in": 3600
		});

		Mock::given(method("POST"))
			.and(path("/token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(token_response))
			.expect(1)
			.mount(&server)
			.await;

		let provider = OidcProvider::new();
		let client = make_test_client();
		let metadata = OidcMetadata {
			authorization_endpoint: format!("{}/authorize", server.uri()),
			token_endpoint: format!("{}/token", server.uri()),
			jwks_uri: None,
			end_session_endpoint: None,
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		};

		let first = provider
			.exchange_code(
				test_ctx(&client),
				ExchangeCodeRequest {
					metadata: &metadata,
					code: "code-123",
					client_id: "client-123",
					client_secret: "secret-123",
					redirect_uri: "http://localhost:3000/oauth2/callback",
					code_verifier: Some("verifier-123"),
				},
			)
			.await
			.expect("first exchange should succeed");
		assert_eq!(first.access_token, "access-token");

		let replay = provider
			.exchange_code(
				test_ctx(&client),
				ExchangeCodeRequest {
					metadata: &metadata,
					code: "code-123",
					client_id: "client-123",
					client_secret: "secret-123",
					redirect_uri: "http://localhost:3000/oauth2/callback",
					code_verifier: Some("verifier-123"),
				},
			)
			.await
			.expect("duplicate exchange should use recent success cache");
		assert_eq!(replay.access_token, "access-token");
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
			.get_metadata(test_ctx(&client), &issuer)
			.await
			.expect("first metadata fetch");
		assert_eq!(
			first.jwks_uri.as_deref(),
			Some(format!("{issuer}/jwks").as_str())
		);

		{
			let key = MetadataCacheKey::new(&issuer, None);
			let mut cache = provider.metadata_cache.write().await;
			let entry = cache.get_mut(&key).expect("metadata cache entry");
			entry.fetched_at = Instant::now() - METADATA_TTL - Duration::from_secs(1);
		}

		let second = provider
			.get_metadata(test_ctx(&client), &issuer)
			.await
			.expect("metadata fetch after ttl");
		assert_eq!(
			second.jwks_uri.as_deref(),
			Some(format!("{issuer}/jwks").as_str())
		);
	}

	#[tokio::test]
	async fn validator_cache_refreshes_after_ttl_without_forced_refresh() {
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
			.expect(2)
			.mount(&server)
			.await;

		let provider = OidcProvider::new();
		let client = make_test_client();
		let _first = provider
			.get_info(test_ctx(&client), &issuer, Some(vec!["aud".to_string()]))
			.await
			.expect("first validator fetch");

		{
			let audiences = vec!["aud".to_string()];
			let key = ValidatorCacheKey::new(&issuer, None, Some(&audiences));
			let mut cache = provider.validator_cache.write().await;
			let entry = cache.get_mut(&key).expect("validator cache entry");
			entry.last_refresh = Instant::now() - VALIDATOR_TTL - Duration::from_secs(1);
			entry.last_refresh_forced = false;
		}

		let _second = provider
			.get_info(test_ctx(&client), &issuer, Some(vec!["aud".to_string()]))
			.await
			.expect("validator fetch after ttl");
	}

	#[tokio::test]
	async fn metadata_fetch_recovers_after_cancelled_request() {
		let server = MockServer::start().await;
		let issuer = server.uri();
		let metadata = json!({
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": format!("{issuer}/token"),
			"jwks_uri": format!("{issuer}/jwks"),
		});

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(
				ResponseTemplate::new(200)
					.set_delay(Duration::from_millis(250))
					.set_body_json(metadata),
			)
			.mount(&server)
			.await;

		let provider = Arc::new(OidcProvider::new());
		let client = make_test_client();
		let issuer_for_task = issuer.clone();
		let provider_for_task = provider.clone();
		let client_for_task = client.clone();

		let handle = tokio::spawn(async move {
			let _ = provider_for_task
				.get_metadata(test_ctx(&client_for_task), &issuer_for_task)
				.await;
		});

		tokio::time::sleep(Duration::from_millis(20)).await;
		handle.abort();
		let _ = handle.await;

		let recovered = tokio::time::timeout(
			Duration::from_secs(2),
			provider.get_metadata(test_ctx(&client), &issuer),
		)
		.await
		.expect("metadata fetch should not hang after cancellation")
		.expect("metadata fetch after cancellation should succeed");
		assert_eq!(
			recovered.jwks_uri.as_deref(),
			Some(format!("{issuer}/jwks").as_str())
		);
	}
}
