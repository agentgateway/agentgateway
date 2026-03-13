use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_lc_rs::hkdf;
use axum::response::Response;
#[cfg(test)]
use cookie::Cookie;
#[cfg(test)]
use http::StatusCode;
#[cfg(test)]
use secrecy::ExposeSecret;
use tracing::{debug, warn};
use url::Url;

use crate::client::Client;
#[cfg(test)]
use crate::http::auth::UpstreamAccessToken;
#[cfg(test)]
use crate::http::jwt::Claims;
use crate::http::jwt::{JWTValidationOptions, Jwt, Mode as JwtMode, Provider as JwtProvider};
use crate::http::oidc::{
	Error as OidcError, ExchangeCodeRequest, OidcCallContext, OidcMetadata, OidcTokenService,
	RefreshTokenRequest, TokenResponse,
};
use crate::http::{PolicyResponse, Request, merge_in_headers};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{OAuth2AttachmentKey, OAuth2Policy};
#[cfg(test)]
use http::HeaderValue;

mod cookies;
mod crypto;
mod flow;
mod protocol;
mod session;
mod state;

use self::cookies::clear_session_cookies;
use self::crypto::SessionCodec;
pub(crate) use self::crypto::{RuntimeCookieSecret, parse_runtime_cookie_secret};
use self::state::{HandshakeState, SessionState};

const DEFAULT_COOKIE_NAME_PREFIX: &str = "__Host-ag-session";
const INSECURE_DEFAULT_COOKIE_NAME_PREFIX: &str = "ag-session";
const DEFAULT_HANDSHAKE_COOKIE_NAME_PREFIX: &str = "__Host-ag-nonce";
const INSECURE_DEFAULT_HANDSHAKE_COOKIE_NAME_PREFIX: &str = "ag-nonce";
const STATE_TTL: Duration = Duration::from_secs(300); // 5 minutes for login handshake
// Keep refresh-capable sessions alive long enough to perform token refreshes.
const DEFAULT_REFRESHABLE_COOKIE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const DEFAULT_SCOPE_PARAM: &str = "openid profile email";
const SESSION_COOKIE_AAD: &[u8] = b"agentgateway_session_cookie";
const HANDSHAKE_STATE_AAD: &[u8] = b"agentgateway_handshake_state";

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("oidc discovery failed: {0}")]
	OidcDiscovery(#[from] OidcError),
	#[error("oauth2 handshake failed: {0}")]
	Handshake(String),
	#[error("invalid token: {0}")]
	InvalidToken(String),
	#[error("internal error: {0}")]
	Internal(String),
}

#[derive(Debug, Clone)]
struct ValidatedRedirectUrl(Url);

impl ValidatedRedirectUrl {
	fn parse(raw: &str, field_name: &str) -> anyhow::Result<Self> {
		let parsed =
			Url::parse(raw).map_err(|e| anyhow::anyhow!("invalid {field_name} config: {e}"))?;
		if !OAuth2::is_allowed_redirect_url(&parsed) {
			anyhow::bail!(
				"{field_name} must use https (or http on loopback hosts), include a host, must not contain a fragment, and must not include userinfo"
			);
		}
		Ok(Self(parsed))
	}

	fn as_url(&self) -> &Url {
		&self.0
	}
}

#[derive(Debug, Clone)]
struct ValidatedProviderEndpointUrl;

impl ValidatedProviderEndpointUrl {
	fn parse(raw: &str, field_name: &str) -> anyhow::Result<Self> {
		let parsed =
			Url::parse(raw).map_err(|e| anyhow::anyhow!("invalid {field_name} config: {e}"))?;
		if !OAuth2::is_allowed_provider_endpoint_url(&parsed) {
			anyhow::bail!(
				"{field_name} must use https (or http on loopback hosts), include a host, must not contain a fragment, and must not include userinfo"
			);
		}
		Ok(Self)
	}
}

/// OAuth2 implements modernized, stateless, and secure OAuth2/OIDC policy handling.
#[derive(Debug, Clone)]
pub struct OAuth2 {
	config: OAuth2Policy,
	attachment_key: OAuth2AttachmentKey,
	session_codec: Arc<SessionCodec>,
	handshake_codec: Arc<SessionCodec>,
	session_cookie_name: String,
	handshake_cookie_base_name: String,
	static_redirect_uri: Option<ValidatedRedirectUrl>,
	resolved_metadata: Option<Arc<OidcMetadata>>,
	resolved_jwt_validator: Option<Arc<Jwt>>,
}

#[derive(Debug, Clone)]
pub struct StoredOAuth2Policy {
	config: OAuth2Policy,
	attachment_key: OAuth2AttachmentKey,
	callback_path: String,
}

#[derive(Clone, Copy)]
struct OAuth2CallContext<'a> {
	client: &'a Client,
	policy_client: &'a PolicyClient,
	oidc: &'a OAuth2TokenService,
}

#[derive(Debug, Clone)]
pub struct OAuth2TokenService {
	inner: OidcTokenService,
}

impl OAuth2TokenService {
	pub fn new_runtime() -> Self {
		Self {
			inner: OidcTokenService::new_runtime(),
		}
	}

	#[cfg(any(test, feature = "internal_benches"))]
	pub(crate) fn from_oidc(inner: OidcTokenService) -> Self {
		Self { inner }
	}

	pub async fn exchange_code(
		&self,
		ctx: OidcCallContext<'_>,
		req: ExchangeCodeRequest<'_>,
	) -> Result<TokenResponse, OidcError> {
		self.inner.exchange_code(ctx, req).await
	}

	pub async fn refresh_token(
		&self,
		ctx: OidcCallContext<'_>,
		req: RefreshTokenRequest<'_>,
	) -> Result<TokenResponse, OidcError> {
		self.inner.refresh_token(ctx, req).await
	}
}

impl serde::Serialize for OAuth2 {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.config.serialize(serializer)
	}
}

impl serde::Serialize for StoredOAuth2Policy {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.config.serialize(serializer)
	}
}

impl OAuth2 {
	fn derive_codec(
		root_secret: &[u8],
		info: &[u8],
		aad: &'static [u8],
	) -> anyhow::Result<SessionCodec> {
		let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]);
		let prk = salt.extract(root_secret);
		let info_binding = [info];
		let okm = prk
			.expand(&info_binding, hkdf::HKDF_SHA256)
			.map_err(|_| anyhow::anyhow!("HKDF expansion failed"))?;
		let mut key_bytes = [0u8; 32];
		okm
			.fill(&mut key_bytes)
			.map_err(|_| anyhow::anyhow!("HKDF fill failed"))?;
		SessionCodec::new(&key_bytes, aad)
	}

	fn derive_runtime_session_codec(
		root_secret: &[u8],
		attachment_key: &OAuth2AttachmentKey,
		cookie_scope: &str,
	) -> anyhow::Result<SessionCodec> {
		let session_info = format!(
			"agentgateway_oauth2_session|attachment={}|cookie={cookie_scope}",
			attachment_key
		);
		Self::derive_codec(root_secret, session_info.as_bytes(), SESSION_COOKIE_AAD)
	}

	fn derive_runtime_handshake_codec(root_secret: &[u8]) -> anyhow::Result<SessionCodec> {
		Self::derive_codec(
			root_secret,
			b"agentgateway_oauth2_handshake_state_v1",
			HANDSHAKE_STATE_AAD,
		)
	}

	fn oidc_context<'a>(&'a self, runtime: OAuth2CallContext<'a>) -> OidcCallContext<'a> {
		OidcCallContext::new(
			runtime.client,
			Some(runtime.policy_client),
			self.config.provider_backend.as_ref(),
		)
	}

	fn auth_realm(&self) -> &str {
		self
			.config
			.oidc_issuer
			.as_deref()
			.unwrap_or(&self.config.provider_id)
	}

	fn cookie_secure(&self) -> bool {
		self
			.static_redirect_uri
			.as_ref()
			.is_none_or(|uri| uri.as_url().scheme() == "https")
	}

	pub(crate) fn session_cookie_name(&self) -> &str {
		&self.session_cookie_name
	}

	pub(crate) fn attachment_key(&self) -> &OAuth2AttachmentKey {
		&self.attachment_key
	}

	fn handshake_cookie_base_name(&self) -> &str {
		&self.handshake_cookie_base_name
	}

	pub fn validate_policy(config: &OAuth2Policy) -> anyhow::Result<()> {
		let redirect_uri = config
			.redirect_uri
			.as_deref()
			.ok_or_else(|| anyhow::anyhow!("oauth2 policy requires redirect_uri"))?;
		ValidatedRedirectUrl::parse(redirect_uri, "redirect_uri")?;
		if let Some(provider) = config.resolved_provider.as_deref() {
			ValidatedProviderEndpointUrl::parse(
				provider.authorization_endpoint.as_str(),
				"authorization_endpoint",
			)?;
			ValidatedProviderEndpointUrl::parse(provider.token_endpoint.as_str(), "token_endpoint")?;
			if let Some(endpoint) = provider.end_session_endpoint.as_deref() {
				ValidatedProviderEndpointUrl::parse(endpoint, "end_session_endpoint")?;
			}
		}
		Ok(())
	}

	fn build_resolved_metadata(config: &OAuth2Policy) -> anyhow::Result<Option<Arc<OidcMetadata>>> {
		let Some(provider) = config.resolved_provider.as_deref() else {
			return Ok(None);
		};
		Ok(Some(Arc::new(OidcMetadata {
			authorization_endpoint: provider.authorization_endpoint.clone(),
			token_endpoint: provider.token_endpoint.clone(),
			jwks_uri: None,
			end_session_endpoint: provider.end_session_endpoint.clone(),
			token_endpoint_auth_methods_supported: provider.token_endpoint_auth_methods_supported.clone(),
		})))
	}

	fn build_resolved_jwt_validator(config: &OAuth2Policy) -> anyhow::Result<Option<Arc<Jwt>>> {
		let Some(provider) = config.resolved_provider.as_deref() else {
			return Ok(None);
		};
		let Some(jwks_inline) = provider.jwks_inline.as_deref() else {
			return Ok(None);
		};
		let provider = JwtProvider::from_inline_jwks(
			jwks_inline,
			config
				.oidc_issuer
				.clone()
				.ok_or_else(|| anyhow::anyhow!("jwks_inline requires oidc_issuer in oauth2 config"))?,
			Some(vec![config.client_id.clone()]),
			JWTValidationOptions::default(),
		)
		.map_err(|e| anyhow::anyhow!("invalid jwks_inline in oauth2 config: {e}"))?;
		Ok(Some(Arc::new(Jwt::from_providers(
			vec![provider],
			JwtMode::Strict,
		))))
	}

	pub fn new(
		config: OAuth2Policy,
		attachment_key: OAuth2AttachmentKey,
		root_secret: Arc<RuntimeCookieSecret>,
	) -> anyhow::Result<Self> {
		Self::validate_policy(&config)?;
		let resolved_metadata = Self::build_resolved_metadata(&config)?;
		if resolved_metadata.is_none() {
			anyhow::bail!("oauth2 policy requires resolved provider metadata");
		}
		Self::new_materialized(config, attachment_key, root_secret, resolved_metadata)
	}

	fn new_materialized(
		config: OAuth2Policy,
		attachment_key: OAuth2AttachmentKey,
		root_secret: Arc<RuntimeCookieSecret>,
		resolved_metadata: Option<Arc<OidcMetadata>>,
	) -> anyhow::Result<Self> {
		let resolved_jwt_validator = Self::build_resolved_jwt_validator(&config)?;
		let static_redirect_uri = config
			.redirect_uri
			.as_deref()
			.map(|raw| ValidatedRedirectUrl::parse(raw, "redirect_uri"))
			.transpose()?;
		let cookie_secure = static_redirect_uri
			.as_ref()
			.is_none_or(|uri| uri.as_url().scheme() == "https");
		let cookie_namespace = attachment_key.cookie_namespace();
		let session_cookie_name = Self::default_session_cookie_name(&cookie_namespace, cookie_secure);
		let handshake_cookie_base_name =
			Self::default_handshake_cookie_base_name(&cookie_namespace, cookie_secure);
		let cookie_scope = session_cookie_name.as_str();
		let session_codec = Arc::new(Self::derive_runtime_session_codec(
			root_secret.as_bytes(),
			&attachment_key,
			cookie_scope,
		)?);
		let handshake_codec = Arc::new(Self::derive_runtime_handshake_codec(
			root_secret.as_bytes(),
		)?);

		Ok(Self {
			config,
			attachment_key,
			session_codec,
			handshake_codec,
			session_cookie_name,
			handshake_cookie_base_name,
			static_redirect_uri,
			resolved_metadata,
			resolved_jwt_validator,
		})
	}

	#[cfg(test)]
	pub(crate) fn config(&self) -> &OAuth2Policy {
		&self.config
	}

	fn decrypt_handshake_state(&self, encoded: &str) -> anyhow::Result<HandshakeState> {
		self.handshake_codec.decrypt_handshake_state(encoded)
	}

	fn decode_session(&self, encoded: &str) -> anyhow::Result<SessionState> {
		self.session_codec.decode_session(encoded)
	}

	fn default_session_cookie_name(cookie_namespace: &str, cookie_secure: bool) -> String {
		let prefix = if cookie_secure {
			DEFAULT_COOKIE_NAME_PREFIX
		} else {
			INSECURE_DEFAULT_COOKIE_NAME_PREFIX
		};
		format!("{prefix}-{cookie_namespace}")
	}

	fn default_handshake_cookie_base_name(cookie_namespace: &str, cookie_secure: bool) -> String {
		let prefix = if cookie_secure {
			DEFAULT_HANDSHAKE_COOKIE_NAME_PREFIX
		} else {
			INSECURE_DEFAULT_HANDSHAKE_COOKIE_NAME_PREFIX
		};
		format!("{prefix}-{cookie_namespace}")
	}

	#[tracing::instrument(
		skip_all,
		fields(provider_id = %self.config.provider_id, client_id = %self.config.client_id)
	)]
	pub async fn enforce_request(
		&self,
		client: &Client,
		policy_client: &PolicyClient,
		oidc: &OAuth2TokenService,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		debug!(path = req.uri().path(), "enforcing oauth2 policy");
		let runtime = OAuth2CallContext {
			client,
			policy_client,
			oidc,
		};
		let redirect_uri = self.resolve_redirect_uri()?;
		let mut updated_cookie_headers = None;

		// Reuse an existing session when possible.
		if let Some(mut session) = self.get_session(req.headers()) {
			// Refresh expired sessions when a refresh token is available.
			if session.is_expired() {
				debug!("Session expired, attempting refresh");
				if session.refresh_token.is_some() {
					let (metadata, jwt_validator) = self.resolved_oidc_info()?;
					match self
						.refresh_session(runtime, &mut session, &metadata, jwt_validator.as_deref())
						.await
					{
						Ok(true) => match self.session_codec.encode_session(&session) {
							Ok(encoded) => match self.set_session_cookies(encoded, session.cookie_max_age()) {
								Ok(headers) => {
									updated_cookie_headers = Some(headers);
								},
								Err(err) => {
									warn!(error = %err, "failed to persist refreshed oauth2 session; forcing re-authentication");
									updated_cookie_headers = Some(self.clear_session_cookies());
									session.expires_at = SystemTime::UNIX_EPOCH;
								},
							},
							Err(err) => {
								debug!("failed to encode refreshed session: {err}");
							},
						},
						_ => {
							debug!("Refresh failed, requiring re-auth");
							updated_cookie_headers = Some(self.clear_session_cookies());
						},
					}
				} else {
					debug!("Session expired with no refresh token, requiring re-auth");
					updated_cookie_headers = Some(self.clear_session_cookies());
				}
			}

			if !session.is_expired() {
				let claims = self.session_claims(&session);
				self.inject_auth(req, &session.access_token, claims);
				return Ok(PolicyResponse {
					direct_response: None,
					response_headers: updated_cookie_headers,
				});
			}
		}

		// No valid session: start authorization flow.
		let metadata = self.resolved_oidc_metadata()?;
		let auth_response = self
			.trigger_auth(req.headers(), req.uri(), &metadata, &redirect_uri)
			.await?;
		Ok(Self::merge_response_headers(
			auth_response,
			updated_cookie_headers,
		))
	}

	fn resolved_oidc_metadata(&self) -> Result<Arc<OidcMetadata>, ProxyError> {
		self.resolved_metadata.clone().ok_or_else(|| {
			ProxyError::from(Error::Internal(
				"oauth2 policy requires resolved provider metadata".into(),
			))
		})
	}

	fn resolved_oidc_info(&self) -> Result<(Arc<OidcMetadata>, Option<Arc<Jwt>>), ProxyError> {
		Ok((
			self.resolved_oidc_metadata()?,
			self.resolved_jwt_validator.clone(),
		))
	}

	fn clear_session_cookies(&self) -> crate::http::HeaderMap {
		clear_session_cookies(self.session_cookie_name(), self.cookie_secure())
	}

	fn merge_response_headers(
		mut response: PolicyResponse,
		extra_headers: Option<crate::http::HeaderMap>,
	) -> PolicyResponse {
		let Some(extra_headers) = extra_headers else {
			return response;
		};
		let mut merged = crate::http::HeaderMap::new();
		crate::http::merge_in_headers(Some(extra_headers), &mut merged);
		crate::http::merge_in_headers(response.response_headers.take(), &mut merged);
		response.response_headers = Some(merged);
		response
	}

	fn into_direct_response(response: PolicyResponse) -> Result<Response, ProxyError> {
		let Some(mut direct_response) = response.direct_response else {
			return Err(ProxyError::AuthPolicyConflict(
				"oauth2 protocol endpoint handling must return a direct response",
			));
		};
		merge_in_headers(response.response_headers, direct_response.headers_mut());
		Ok(direct_response)
	}

	fn resolve_redirect_uri(&self) -> Result<Url, ProxyError> {
		self
			.static_redirect_uri
			.as_ref()
			.map(|uri| uri.as_url().clone())
			.ok_or_else(|| {
				ProxyError::from(Error::Internal(
					"oauth2 policy requires redirect_uri".into(),
				))
			})
	}

	#[cfg(test)]
	fn build_session_cookie(
		&self,
		name: String,
		value: String,
		cookie_max_age: cookie::time::Duration,
	) -> Cookie<'static> {
		self::cookies::build_session_cookie(name, value, self.cookie_secure(), cookie_max_age)
	}

	fn should_return_unauthorized(&self, headers: &http::HeaderMap) -> bool {
		let accept = headers
			.get(http::header::ACCEPT)
			.and_then(|v| v.to_str().ok())
			.unwrap_or("");
		!Self::accepts_html_media_type(accept)
	}

	fn accepts_html_media_type(accept: &str) -> bool {
		if accept.trim().is_empty() {
			return false;
		}
		accept
			.split(',')
			.filter_map(Self::parse_accept_media_range)
			.any(|(media_range, quality)| {
				quality > 0.0
					&& (media_range == "text/html"
						|| media_range == "application/xhtml+xml"
						|| media_range == "text/*")
			})
	}

	fn parse_accept_media_range(raw: &str) -> Option<(String, f32)> {
		let mut parts = raw.split(';');
		let media_range = parts.next()?.trim().to_ascii_lowercase();
		if media_range.is_empty() {
			return None;
		}
		let mut quality = 1.0f32;
		for parameter in parts {
			let parameter = parameter.trim();
			let Some((name, value)) = parameter.split_once('=') else {
				continue;
			};
			if name.trim().eq_ignore_ascii_case("q")
				&& let Ok(q) = value.trim().parse::<f32>()
			{
				quality = q.clamp(0.0, 1.0);
			}
		}
		Some((media_range, quality))
	}

	fn is_allowed_redirect_url(url: &Url) -> bool {
		if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
			return false;
		}
		match url.scheme() {
			"https" => url.host_str().is_some(),
			"http" => url.host_str().is_some_and(crate::http::is_loopback_host),
			_ => false,
		}
	}

	fn is_allowed_provider_endpoint_url(url: &Url) -> bool {
		if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
			return false;
		}
		match url.scheme() {
			"https" => url.host_str().is_some(),
			"http" => url.host_str().is_some_and(crate::http::is_loopback_host),
			_ => false,
		}
	}
}

impl StoredOAuth2Policy {
	pub fn new(config: OAuth2Policy, attachment_key: OAuth2AttachmentKey) -> anyhow::Result<Self> {
		OAuth2::validate_policy(&config)?;
		let callback_path = ValidatedRedirectUrl::parse(
			config
				.redirect_uri
				.as_deref()
				.expect("validated oauth2 policy must include redirect_uri"),
			"redirect_uri",
		)?
		.as_url()
		.path()
		.to_string();
		if OAuth2::build_resolved_metadata(&config)?.is_none() {
			anyhow::bail!("oauth2 policy requires resolved provider metadata");
		}
		Ok(Self {
			config,
			attachment_key,
			callback_path,
		})
	}

	pub(crate) fn attachment_key(&self) -> &OAuth2AttachmentKey {
		&self.attachment_key
	}

	pub(crate) fn matches_protocol_endpoint(&self, path: &str) -> bool {
		self.callback_path == path
	}

	pub(crate) fn materialize(&self, runtime: Arc<RuntimeCookieSecret>) -> anyhow::Result<OAuth2> {
		OAuth2::new_materialized(
			self.config.clone(),
			self.attachment_key.clone(),
			runtime,
			Some(
				OAuth2::build_resolved_metadata(&self.config)?
					.expect("stored oauth2 policy must have resolved provider metadata"),
			),
		)
	}
}

struct CallbackValidation<'a> {
	metadata: &'a OidcMetadata,
	jwt_validator: Option<&'a Jwt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtocolEndpointKind {
	Callback,
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use prometheus_client::registry::Registry;
	use secrecy::SecretString;
	use serde_json::json;
	use serde_json::{Map, Value};
	use tokio::task::JoinSet;
	use wiremock::matchers::{body_string_contains, method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::http::oidc::OidcClient;

	const TEST_POLICY_KEY: &str = "test-policy";

	fn test_attachment_key() -> crate::types::agent::OAuth2AttachmentKey {
		crate::types::agent::OAuth2AttachmentKey::targeted_policy(TEST_POLICY_KEY)
	}

	fn expected_default_cookie_name(secure: bool) -> String {
		let namespace = test_attachment_key().cookie_namespace();
		let prefix = if secure {
			DEFAULT_COOKIE_NAME_PREFIX
		} else {
			INSECURE_DEFAULT_COOKIE_NAME_PREFIX
		};
		format!("{prefix}-{namespace}")
	}

	fn expected_default_handshake_cookie_name(secure: bool) -> String {
		let namespace = test_attachment_key().cookie_namespace();
		let prefix = if secure {
			DEFAULT_HANDSHAKE_COOKIE_NAME_PREFIX
		} else {
			INSECURE_DEFAULT_HANDSHAKE_COOKIE_NAME_PREFIX
		};
		format!("{prefix}-{namespace}")
	}

	fn test_config() -> OAuth2Policy {
		OAuth2Policy {
			provider_id: "https://issuer.example.com".to_string(),
			oidc_issuer: Some("https://issuer.example.com".to_string()),
			provider_backend: None,
			client_id: "client-id".to_string(),
			client_secret: SecretString::new("secret".into()),
			resolved_provider: Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
				authorization_endpoint: "https://issuer.example.com/authorize".to_string(),
				token_endpoint: "https://issuer.example.com/token".to_string(),
				jwks_inline: None,
				end_session_endpoint: Some("https://issuer.example.com/logout".to_string()),
				token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
			})),
			redirect_uri: Some("https://fixed.example.com/callback".to_string()),
			scopes: vec![],
		}
	}

	fn resolved_test_jwks_inline() -> String {
		json!({
			"keys": [{
				"kty": "EC",
				"crv": "P-256",
				"kid": "test-nonce-kid",
				"alg": "ES256",
				"x": "WfUSsBlmKtTX8Rfmo9K-6PsKG1Ysw1j3St-ZUZSq4HU",
				"y": "vO_R0kjX3d1oz-2aUtpoWfBp-wu7YxO_XjGSHv40tgM",
				"use": "sig"
			}]
		})
		.to_string()
	}

	fn make_test_client() -> crate::client::Client {
		let cfg = crate::client::Config {
			resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
			resolver_opts: hickory_resolver::config::ResolverOpts::default(),
		};
		crate::client::Client::new(&cfg, None, Default::default(), None)
	}

	fn make_test_policy_client() -> PolicyClient {
		let config = crate::config::parse_config("{}".to_string(), None).unwrap();
		let encoder = config.session_encoder.clone();
		let stores = crate::store::Stores::from_init(crate::store::StoresInit {
			ipv6_enabled: config.ipv6_enabled,
		});
		let upstream = make_test_client();
		let inputs = Arc::new(crate::ProxyInputs::new(
			Arc::new(config),
			stores.clone(),
			Arc::new(crate::metrics::Metrics::new(
				agent_core::metrics::sub_registry(&mut Registry::default()),
				Default::default(),
			)),
			upstream,
			None,
			crate::mcp::App::new(stores, encoder),
		));
		PolicyClient { inputs }
	}

	fn request_cookie_header_from_set_cookie_values(
		set_cookie_values: &[String],
		cookie_name: &str,
	) -> String {
		set_cookie_values
			.iter()
			.filter_map(|set_cookie| {
				let pair = set_cookie.split(';').next()?.trim();
				let (name, value) = pair.split_once('=')?;
				let is_chunk = name == cookie_name
					|| name
						.strip_prefix(cookie_name)
						.is_some_and(|suffix| suffix.starts_with('.'));
				(is_chunk && !value.is_empty()).then(|| format!("{name}={value}"))
			})
			.collect::<Vec<_>>()
			.join("; ")
	}

	fn test_oauth_cookie_secret() -> Arc<RuntimeCookieSecret> {
		Arc::new(
			parse_runtime_cookie_secret(
				"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
			)
			.unwrap(),
		)
	}

	fn test_oauth2() -> OAuth2 {
		OAuth2::new(
			test_config(),
			test_attachment_key(),
			test_oauth_cookie_secret(),
		)
		.unwrap()
	}

	#[test]
	fn original_target_only_keeps_path_and_query() {
		let uri: http::Uri = "https://evil.example.com/path?q=1".parse().unwrap();
		assert_eq!(OAuth2::original_target_from_uri(&uri), "/path?q=1");
	}

	#[test]
	fn safe_redirect_target_allows_local_path_only() {
		assert!(OAuth2::is_safe_redirect_target("/ok"));
		assert!(OAuth2::is_safe_redirect_target("/ok?q=1"));
		assert!(!OAuth2::is_safe_redirect_target("//evil.example.com"));
		assert!(!OAuth2::is_safe_redirect_target("https://evil.example.com"));
		assert!(!OAuth2::is_safe_redirect_target("/\\evil.example.com"));
		assert!(!OAuth2::is_safe_redirect_target("/ok\nbad"));
	}

	#[test]
	fn resolve_redirect_uri_prefers_config() {
		let oauth2 = test_oauth2();
		let resolved = oauth2.resolve_redirect_uri().unwrap();
		assert_eq!(resolved.as_str(), "https://fixed.example.com/callback");
	}

	#[test]
	fn default_cookie_names_are_namespaced_by_attachment_key() {
		let oauth2 = test_oauth2();
		assert_eq!(
			oauth2.session_cookie_name(),
			expected_default_cookie_name(true)
		);
		assert_eq!(
			oauth2.handshake_cookie_base_name(),
			expected_default_handshake_cookie_name(true)
		);
	}

	#[test]
	fn oauth2_new_validates_redirect_uri_rules() {
		struct Case {
			name: &'static str,
			redirect_uri: Option<&'static str>,
			want_err: Option<&'static str>,
		}

		let cases = [
			Case {
				name: "requires redirect uri",
				redirect_uri: None,
				want_err: Some("requires redirect_uri"),
			},
			Case {
				name: "rejects invalid uri",
				redirect_uri: Some("not-a-valid-uri"),
				want_err: Some("invalid redirect_uri config"),
			},
			Case {
				name: "rejects non loopback http by default",
				redirect_uri: Some("http://app.example.com/callback"),
				want_err: Some("redirect_uri must use https (or http on loopback hosts"),
			},
			Case {
				name: "accepts loopback http",
				redirect_uri: Some("http://127.0.0.1:3000/callback"),
				want_err: None,
			},
			Case {
				name: "rejects non http https scheme",
				redirect_uri: Some("ftp://example.com/callback"),
				want_err: Some("redirect_uri must use https (or http on loopback hosts"),
			},
		];

		for case in cases {
			let mut config = test_config();
			config.redirect_uri = case.redirect_uri.map(ToOwned::to_owned);

			match case.want_err {
				Some(want_err) => {
					let err =
						OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap_err();
					assert!(
						err.to_string().contains(want_err),
						"case {:?}: unexpected error: {err}",
						case.name
					);
				},
				None => {
					assert!(
						OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).is_ok(),
						"case {:?} should succeed",
						case.name
					);
				},
			}
		}
	}

	#[test]
	fn oauth2_new_requires_resolved_provider_metadata() {
		let mut config = test_config();
		config.resolved_provider = None;
		let err = OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap_err();
		assert!(
			err
				.to_string()
				.contains("oauth2 policy requires resolved provider metadata")
		);
	}

	#[test]
	fn oauth2_new_validates_resolved_provider_endpoints() {
		struct Case {
			name: &'static str,
			authorization_endpoint: &'static str,
			token_endpoint: &'static str,
			end_session_endpoint: Option<&'static str>,
			jwks_inline: Option<String>,
			want_err: Option<&'static str>,
		}

		let cases = [
			Case {
				name: "accepts resolved provider with jwks",
				authorization_endpoint: "https://issuer.example.com/authorize",
				token_endpoint: "https://issuer.example.com/token",
				end_session_endpoint: Some("https://issuer.example.com/logout"),
				jwks_inline: Some(resolved_test_jwks_inline()),
				want_err: None,
			},
			Case {
				name: "accepts resolved provider without jwks",
				authorization_endpoint: "https://issuer.example.com/authorize",
				token_endpoint: "https://issuer.example.com/token",
				end_session_endpoint: None,
				jwks_inline: None,
				want_err: None,
			},
			Case {
				name: "rejects non loopback http authorization endpoint",
				authorization_endpoint: "http://idp.example.com/authorize",
				token_endpoint: "https://issuer.example.com/token",
				end_session_endpoint: None,
				jwks_inline: None,
				want_err: Some("authorization_endpoint must use https (or http on loopback hosts)"),
			},
			Case {
				name: "accepts loopback http provider endpoints",
				authorization_endpoint: "http://127.0.0.1:3000/authorize",
				token_endpoint: "http://127.0.0.1:3000/token",
				end_session_endpoint: Some("http://127.0.0.1:3000/logout"),
				jwks_inline: None,
				want_err: None,
			},
		];

		for case in cases {
			let mut config = test_config();
			config.resolved_provider = Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
				authorization_endpoint: case.authorization_endpoint.to_string(),
				token_endpoint: case.token_endpoint.to_string(),
				jwks_inline: case.jwks_inline.clone(),
				end_session_endpoint: case.end_session_endpoint.map(ToOwned::to_owned),
				token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
			}));

			match case.want_err {
				Some(want_err) => {
					let err =
						OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap_err();
					assert!(
						err.to_string().contains(want_err),
						"case {:?}: unexpected error: {err}",
						case.name
					);
				},
				None => {
					assert!(
						OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).is_ok(),
						"case {:?} should succeed",
						case.name
					);
				},
			}
		}
	}

	#[test]
	fn oidc_session_cookie_key_stays_compatible_when_provider_id_changes() {
		let mut legacy = test_config();
		legacy.provider_id = "provider-a".to_string();
		let legacy_oauth2 =
			OAuth2::new(legacy, test_attachment_key(), test_oauth_cookie_secret()).unwrap();

		let mut updated = test_config();
		updated.provider_id = "provider-b".to_string();
		let updated_oauth2 =
			OAuth2::new(updated, test_attachment_key(), test_oauth_cookie_secret()).unwrap();

		let session = SessionState {
			access_token: "access-token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: Some("nonce".to_string()),
			id_token: Some("id-token".to_string()),
		};

		let encoded = legacy_oauth2
			.session_codec
			.encode_session(&session)
			.expect("legacy oauth2 should encode session");
		let decoded = updated_oauth2
			.session_codec
			.decode_session(&encoded)
			.expect("updated oauth2 should decode legacy session");

		assert_eq!(decoded.access_token, session.access_token);
		assert_eq!(decoded.refresh_token, session.refresh_token);
		assert_eq!(decoded.id_token, session.id_token);
	}

	#[tokio::test]
	async fn oauth2_enforce_request_uses_resolved_metadata_without_discovery() {
		let mut config = test_config();
		config.resolved_provider = Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
			authorization_endpoint: "https://issuer.example.com/authorize".to_string(),
			token_endpoint: "https://issuer.example.com/token".to_string(),
			jwks_inline: Some(resolved_test_jwks_inline()),
			end_session_endpoint: Some("https://issuer.example.com/logout".to_string()),
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		}));
		let oauth2 = OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let oidc_tokens = OAuth2TokenService::from_oidc(oidc.token_service());
		let mut req = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "/private/data".parse().unwrap();
		req
			.headers_mut()
			.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));

		let response = oauth2
			.enforce_request(&client, &policy_client, &oidc_tokens, &mut req)
			.await
			.unwrap();
		let redirect = response
			.direct_response
			.expect("oauth2 should redirect to authorization endpoint");
		assert_eq!(redirect.status(), StatusCode::FOUND);
		let location = redirect
			.headers()
			.get(http::header::LOCATION)
			.and_then(|v| v.to_str().ok())
			.expect("redirect location must be present");
		assert!(
			location.starts_with("https://issuer.example.com/authorize?"),
			"unexpected redirect location: {location}"
		);
	}

	#[tokio::test]
	async fn oauth2_callback_ignores_id_token_without_validator_in_explicit_mode() {
		let server = MockServer::start().await;

		Mock::given(method("POST"))
			.and(path("/token"))
			.and(body_string_contains("grant_type=authorization_code"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"access_token": "access-from-code",
				"token_type": "Bearer",
				"expires_in": 3600,
				"refresh_token": "refresh-from-code",
				"id_token": "not-a-jwt"
			})))
			.expect(1)
			.mount(&server)
			.await;

		let mut config = test_config();
		config.oidc_issuer = None;
		config.provider_id = format!("{}/authorize", server.uri());
		config.resolved_provider = Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
			authorization_endpoint: format!("{}/authorize", server.uri()),
			token_endpoint: format!("{}/token", server.uri()),
			jwks_inline: None,
			end_session_endpoint: None,
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		}));
		let oauth2 = OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let oidc_tokens = OAuth2TokenService::from_oidc(oidc.token_service());

		let mut initial_req = Request::new(crate::http::Body::empty());
		*initial_req.uri_mut() = "/private/data".parse().unwrap();
		initial_req
			.headers_mut()
			.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));

		let initial = oauth2
			.enforce_request(&client, &policy_client, &oidc_tokens, &mut initial_req)
			.await
			.unwrap();
		let redirect = initial
			.direct_response
			.expect("oauth2 should redirect to provider");
		assert_eq!(redirect.status(), StatusCode::FOUND);
		let location = redirect
			.headers()
			.get(http::header::LOCATION)
			.and_then(|v| v.to_str().ok())
			.expect("redirect location must be present");
		let location_uri: http::Uri = location.parse().expect("redirect location should be a URI");
		let state =
			OAuth2::query_param(&location_uri, "state").expect("state query param must be present");

		let handshake_set_cookie_values: Vec<String> = initial
			.response_headers
			.expect("initial oauth2 redirect should set handshake cookie")
			.get_all(http::header::SET_COOKIE)
			.iter()
			.filter_map(|v| v.to_str().ok().map(ToOwned::to_owned))
			.collect();
		let handshake_cookie_header = request_cookie_header_from_set_cookie_values(
			&handshake_set_cookie_values,
			oauth2.handshake_cookie_base_name(),
		);
		assert!(
			!handshake_cookie_header.is_empty(),
			"handshake cookie should be present"
		);

		let mut callback_req = Request::new(crate::http::Body::empty());
		*callback_req.uri_mut() = format!("/callback?code=auth-code-1&state={state}")
			.parse()
			.unwrap();
		callback_req.headers_mut().append(
			http::header::COOKIE,
			HeaderValue::from_static("other-cookie=1"),
		);
		callback_req.headers_mut().append(
			http::header::COOKIE,
			HeaderValue::from_str(&handshake_cookie_header).unwrap(),
		);

		let callback = oauth2
			.handle_protocol_endpoint(&client, &policy_client, &oidc_tokens, &mut callback_req)
			.await
			.expect("callback should succeed when id_token is returned without validator");
		assert_eq!(callback.status(), StatusCode::FOUND);

		let callback_set_cookie_values: Vec<String> = callback
			.headers()
			.get_all(http::header::SET_COOKIE)
			.iter()
			.filter_map(|v| v.to_str().ok().map(ToOwned::to_owned))
			.collect();
		let session_cookie_header = request_cookie_header_from_set_cookie_values(
			&callback_set_cookie_values,
			oauth2.session_cookie_name(),
		);
		assert!(
			!session_cookie_header.is_empty(),
			"session cookie should be present"
		);

		let mut session_headers = http::HeaderMap::new();
		session_headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&session_cookie_header).unwrap(),
		);
		let session = oauth2
			.get_session(&session_headers)
			.expect("session should decode after callback");
		assert_eq!(session.access_token, "access-from-code");
		assert_eq!(session.id_token, None);
	}

	#[tokio::test]
	async fn oauth2_callback_rejects_state_from_different_attachment() {
		let server = MockServer::start().await;

		Mock::given(method("POST"))
			.and(path("/token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"access_token": "access-from-code",
				"token_type": "Bearer",
				"expires_in": 3600,
			})))
			.mount(&server)
			.await;

		let mut config = test_config();
		config.oidc_issuer = None;
		config.provider_id = format!("{}/authorize", server.uri());
		config.resolved_provider = Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
			authorization_endpoint: format!("{}/authorize", server.uri()),
			token_endpoint: format!("{}/token", server.uri()),
			jwks_inline: None,
			end_session_endpoint: None,
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		}));
		let initiating = OAuth2::new(
			config.clone(),
			crate::types::agent::OAuth2AttachmentKey::targeted_policy("first-policy"),
			test_oauth_cookie_secret(),
		)
		.unwrap();
		let dispatched = OAuth2::new(
			config,
			crate::types::agent::OAuth2AttachmentKey::targeted_policy("second-policy"),
			test_oauth_cookie_secret(),
		)
		.unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let oidc_tokens = OAuth2TokenService::from_oidc(oidc.token_service());

		let mut initial_req = Request::new(crate::http::Body::empty());
		*initial_req.uri_mut() = "/private/data".parse().unwrap();
		initial_req
			.headers_mut()
			.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));

		let initial = initiating
			.enforce_request(&client, &policy_client, &oidc_tokens, &mut initial_req)
			.await
			.expect("initial auth redirect should succeed");
		let redirect = initial.direct_response.expect("oauth2 should redirect");
		let location = redirect
			.headers()
			.get(http::header::LOCATION)
			.and_then(|v| v.to_str().ok())
			.expect("redirect location should be set");
		let location_uri: http::Uri = location.parse().expect("redirect location should be a URI");
		let state =
			OAuth2::query_param(&location_uri, "state").expect("state query param must be present");
		let handshake_state = initiating
			.handshake_codec
			.decrypt_handshake_state(&state)
			.expect("state should decrypt with initiating runtime");
		let handshake_cookie_header = format!(
			"{}.{}=1",
			dispatched.handshake_cookie_base_name(),
			handshake_state.handshake_id
		);

		let mut callback_req = Request::new(crate::http::Body::empty());
		*callback_req.uri_mut() = format!("/callback?code=auth-code-1&state={state}")
			.parse()
			.unwrap();
		callback_req.headers_mut().append(
			http::header::COOKIE,
			HeaderValue::from_str(&handshake_cookie_header).unwrap(),
		);

		let err = dispatched
			.handle_protocol_endpoint(&client, &policy_client, &oidc_tokens, &mut callback_req)
			.await
			.expect_err("callback should reject mismatched oauth2 attachment");
		assert!(matches!(
			err,
			ProxyError::OAuth2AuthenticationFailure(Error::Handshake(_))
		));
		assert!(err.to_string().contains("different oauth2 policy"));
	}

	#[test]
	fn non_html_clients_get_unauthorized_instead_of_redirect() {
		let oauth2 = test_oauth2();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::ACCEPT,
			HeaderValue::from_static("application/json"),
		);
		assert!(oauth2.should_return_unauthorized(&headers));
		headers.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));
		assert!(!oauth2.should_return_unauthorized(&headers));
		headers.insert(http::header::ACCEPT, HeaderValue::from_static("*/*"));
		assert!(oauth2.should_return_unauthorized(&headers));
		headers.insert(
			http::header::ACCEPT,
			HeaderValue::from_static("application/json, text/html;q=0"),
		);
		assert!(oauth2.should_return_unauthorized(&headers));
		headers.insert(
			http::header::ACCEPT,
			HeaderValue::from_static("application/json, text/html;q=0.2"),
		);
		assert!(!oauth2.should_return_unauthorized(&headers));
	}

	#[test]
	fn inject_auth_sets_upstream_access_token_extension() {
		let oauth2 = OAuth2::new(
			test_config(),
			test_attachment_key(),
			test_oauth_cookie_secret(),
		)
		.unwrap();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let mut req = Request::new(crate::http::Body::empty());
		oauth2.inject_auth(&mut req, &session.access_token, None);
		let token = req
			.extensions()
			.get::<UpstreamAccessToken>()
			.expect("upstream access token extension should be set");
		assert_eq!(token.0.expose_secret(), "token");
		assert!(req.headers().get(http::header::AUTHORIZATION).is_none());
	}

	#[test]
	fn oauth2_injected_claims_are_visible_to_cel() {
		let oauth2 = OAuth2::new(
			test_config(),
			test_attachment_key(),
			test_oauth_cookie_secret(),
		)
		.unwrap();
		let mut req = http::Request::builder()
			.method(http::Method::GET)
			.uri("https://example.com/private")
			.body(crate::http::Body::empty())
			.unwrap();
		oauth2.inject_auth(
			&mut req,
			"access-token",
			Some(Claims {
				inner: Map::from_iter([("sub".to_string(), Value::String("oauth-user".to_string()))]),
				jwt: SecretString::new("id.token.value".into()),
			}),
		);

		let expr = crate::cel::Expression::new_strict(r#"jwt.sub == "oauth-user""#)
			.expect("expression should compile");
		let exec = crate::cel::Executor::new_request(&req);
		let value = exec
			.eval(&expr)
			.expect("oauth2-injected claims should evaluate in CEL")
			.json()
			.expect("CEL result should serialize");

		assert_eq!(value, serde_json::json!(true));
	}

	#[test]
	fn get_session_ignores_similar_cookie_prefixes() {
		let oauth2 = test_oauth2();
		let cookie_name = oauth2.session_cookie_name();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let cookies = format!("{cookie_name}={encoded}; {cookie_name}evil.1=malicious");

		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&cookies).unwrap(),
		);

		let decoded = oauth2.get_session(&headers).expect("session should decode");
		assert_eq!(decoded.access_token, "token");
	}

	#[test]
	fn get_session_reads_split_cookie_headers() {
		let oauth2 = test_oauth2();
		let cookie_name = oauth2.session_cookie_name();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();

		let mut headers = http::HeaderMap::new();
		headers.append(http::header::COOKIE, HeaderValue::from_static("other=1"));
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{cookie_name}={encoded}")).unwrap(),
		);

		let decoded = oauth2
			.get_session(&headers)
			.expect("session should decode when cookies are split across headers");
		assert_eq!(decoded.access_token, "token");
	}

	#[test]
	fn get_session_prefers_last_cookie_value_across_split_headers() {
		let oauth2 = test_oauth2();
		let cookie_name = oauth2.session_cookie_name();
		let first = SessionState {
			access_token: "first-token".to_string(),
			refresh_token: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let second = SessionState {
			access_token: "second-token".to_string(),
			refresh_token: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let first_encoded = oauth2.session_codec.encode_session(&first).unwrap();
		let second_encoded = oauth2.session_codec.encode_session(&second).unwrap();

		let mut headers = http::HeaderMap::new();
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{cookie_name}={first_encoded}")).unwrap(),
		);
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{cookie_name}={second_encoded}")).unwrap(),
		);

		let decoded = oauth2
			.get_session(&headers)
			.expect("session should decode when duplicate cookies are present");
		assert_eq!(decoded.access_token, "second-token");
	}

	#[test]
	fn loopback_http_redirect_uses_insecure_default_cookie_name_and_attributes() {
		let mut config = test_config();
		config.redirect_uri = Some("http://127.0.0.1:3000/callback".to_string());
		let oauth2 = OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap();
		assert_eq!(
			oauth2.session_cookie_name(),
			expected_default_cookie_name(false)
		);
		assert_eq!(
			oauth2.handshake_cookie_base_name(),
			expected_default_handshake_cookie_name(false)
		);
		assert!(!oauth2.cookie_secure());

		let cookie = oauth2.build_session_cookie(
			oauth2.session_cookie_name().to_string(),
			"value".to_string(),
			cookie::time::Duration::seconds(60),
		);
		assert_eq!(cookie.secure(), Some(false));
	}

	#[tokio::test]
	async fn concurrent_apply_requests_with_expired_session_refresh_successfully() {
		let server = MockServer::start().await;
		let issuer = server.uri();

		Mock::given(method("POST"))
			.and(path("/token"))
			.and(body_string_contains("grant_type=refresh_token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"access_token": "access-refreshed",
				"token_type": "Bearer",
				"expires_in": 3600,
				"refresh_token": "refresh-next"
			})))
			// Concurrent requests may each perform their own refresh exchange.
			.expect(8)
			.mount(&server)
			.await;

		let mut config = test_config();
		config.provider_id = issuer.clone();
		config.oidc_issuer = Some(issuer);
		config.resolved_provider = Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
			authorization_endpoint: format!("{}/authorize", config.provider_id),
			token_endpoint: format!("{}/token", config.provider_id),
			jwks_inline: None,
			end_session_endpoint: None,
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		}));
		let oauth2 = OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let oidc_tokens = OAuth2TokenService::from_oidc(oidc.token_service());
		let session = SessionState {
			access_token: "access-expired".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			expires_at: SystemTime::now() - Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let cookie_name = oauth2.session_cookie_name();
		let cookie_header = format!("{cookie_name}={encoded}");

		let mut set: JoinSet<(PolicyResponse, Option<String>)> = JoinSet::new();
		for _ in 0..8 {
			let oauth2 = oauth2.clone();
			let client = client.clone();
			let policy_client = policy_client.clone();
			let oidc_tokens = oidc_tokens.clone();
			let cookie_header = cookie_header.clone();
			set.spawn(async move {
				let mut req = Request::new(crate::http::Body::empty());
				*req.uri_mut() = "/private/data".parse().unwrap();
				req.headers_mut().insert(
					http::header::COOKIE,
					HeaderValue::from_str(&cookie_header).unwrap(),
				);

				let response = oauth2
					.enforce_request(&client, &policy_client, &oidc_tokens, &mut req)
					.await
					.expect("apply should succeed");
				let passthrough = req
					.extensions()
					.get::<UpstreamAccessToken>()
					.map(|token| token.0.expose_secret().to_string());
				(response, passthrough)
			});
		}

		while let Some(joined) = set.join_next().await {
			let (response, passthrough) = joined.expect("task should join");
			assert!(response.direct_response.is_none());
			assert!(
				response.response_headers.is_some(),
				"refresh should return updated session cookies"
			);
			assert_eq!(passthrough.as_deref(), Some("access-refreshed"));
		}
	}

	#[tokio::test]
	async fn refresh_failure_clears_stale_session_cookie_before_reauth() {
		let mut config = test_config();
		config.resolved_provider = Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
			authorization_endpoint: "https://issuer.example.com/authorize".to_string(),
			token_endpoint: "https://issuer.example.com/token".to_string(),
			jwks_inline: None,
			end_session_endpoint: None,
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		}));
		let oauth2 = OAuth2::new(config, test_attachment_key(), test_oauth_cookie_secret()).unwrap();
		let cookie_name = oauth2.session_cookie_name();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let oidc_tokens = OAuth2TokenService::from_oidc(oidc.token_service());
		let session = SessionState {
			access_token: "access-expired".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			expires_at: SystemTime::now() - Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();

		let mut req = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "/private/data".parse().unwrap();
		req
			.headers_mut()
			.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));
		req.headers_mut().insert(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{cookie_name}={encoded}")).unwrap(),
		);

		let response = oauth2
			.enforce_request(&client, &policy_client, &oidc_tokens, &mut req)
			.await
			.unwrap();
		let redirect = response
			.direct_response
			.expect("refresh failure should trigger re-auth redirect");
		assert_eq!(redirect.status(), StatusCode::FOUND);

		let set_cookie_values: Vec<String> = response
			.response_headers
			.expect("reauth response should include cookie updates")
			.get_all(http::header::SET_COOKIE)
			.iter()
			.filter_map(|value| value.to_str().ok().map(ToOwned::to_owned))
			.collect();
		assert!(
			set_cookie_values
				.iter()
				.any(|v| v.starts_with(&format!("{cookie_name}=")) && v.contains("Max-Age=0")),
			"stale session cookie should be cleared after refresh failure"
		);
		assert!(
			!request_cookie_header_from_set_cookie_values(
				&set_cookie_values,
				oauth2.handshake_cookie_base_name()
			)
			.is_empty(),
			"reauth response should also set a new handshake cookie"
		);
	}
}
