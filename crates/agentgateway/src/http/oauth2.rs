use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::hkdf;
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cookie::{Cookie, SameSite};
use http::{HeaderValue, StatusCode};
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope, TokenUrl};
use rand::Rng;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use url::Url;

use crate::client::Client;
use crate::http::auth::UpstreamAccessToken;
use crate::http::jwt::{
	Claims, JWTValidationOptions, Jwt, Mode as JwtMode, Provider as JwtProvider,
};
use crate::http::oidc::{
	Error as OidcError, ExchangeCodeRequest, OidcCallContext, OidcMetadata, OidcTokenClient,
	RefreshTokenRequest,
};
use crate::http::{PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::OAuth2Policy;

const DEFAULT_COOKIE_NAME: &str = "__Host-ag-session";
const INSECURE_DEFAULT_COOKIE_NAME: &str = "ag-session";
const DEFAULT_HANDSHAKE_COOKIE_NAME: &str = "__Host-ag-nonce";
const INSECURE_DEFAULT_HANDSHAKE_COOKIE_NAME: &str = "ag-nonce";
const STATE_TTL: Duration = Duration::from_secs(300); // 5 minutes for login handshake
const MAX_COOKIE_SIZE: usize = 3800; // Leave room for browser limits and cookie attributes
const COOKIE_CLEAR_SLOTS: usize = 5;
// Bound parsed chunk indices from request cookies to avoid unbounded cleanup loops on crafted inputs.
const MAX_SESSION_COOKIE_CHUNK_INDEX: usize = 63;
// Keep refresh-capable sessions alive long enough to perform token refreshes.
const DEFAULT_REFRESHABLE_COOKIE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_REFRESHABLE_COOKIE_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
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

#[derive(Debug, thiserror::Error)]
enum SessionCookieError {
	#[error("encoded session exceeds cookie chunk budget")]
	TooLarge,
}

#[derive(Debug, Clone)]
struct ValidatedRedirectUrl(Url);

impl ValidatedRedirectUrl {
	fn parse(raw: &str, allow_insecure_redirect_uri: bool, field_name: &str) -> anyhow::Result<Self> {
		let parsed =
			Url::parse(raw).map_err(|e| anyhow::anyhow!("invalid {field_name} config: {e}"))?;
		if !OAuth2::is_allowed_redirect_url(&parsed, allow_insecure_redirect_uri) {
			anyhow::bail!(
				"{field_name} must use https (or http on loopback hosts unless allow_insecure_redirect_uri is true), include a host, must not contain a fragment, and must not include userinfo"
			);
		}
		Ok(Self(parsed))
	}

	fn as_url(&self) -> &Url {
		&self.0
	}
}

#[derive(Debug, Clone)]
struct ValidatedProviderEndpointUrl(Url);

impl ValidatedProviderEndpointUrl {
	fn parse(raw: &str, field_name: &str) -> anyhow::Result<Self> {
		let parsed =
			Url::parse(raw).map_err(|e| anyhow::anyhow!("invalid {field_name} config: {e}"))?;
		if !OAuth2::is_allowed_provider_endpoint_url(&parsed) {
			anyhow::bail!(
				"{field_name} must use https (or http on loopback hosts), include a host, must not contain a fragment, and must not include userinfo"
			);
		}
		Ok(Self(parsed))
	}

	fn into_url(self) -> Url {
		self.0
	}
}

/// OAuth2 implements modernized, stateless, and secure OAuth2/OIDC policy handling.
#[derive(Debug, Clone)]
pub struct OAuth2 {
	config: OAuth2Policy,
	session_codec: Arc<SessionCodec>,
	handshake_codec: Arc<SessionCodec>,
	static_redirect_uri: Option<ValidatedRedirectUrl>,
	resolved_metadata: Option<Arc<OidcMetadata>>,
	resolved_jwt_validator: Option<Arc<Jwt>>,
}

#[derive(Clone, Copy)]
struct OAuth2CallContext<'a> {
	client: &'a Client,
	policy_client: &'a PolicyClient,
	oidc: OidcTokenClient<'a>,
}

impl serde::Serialize for OAuth2 {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.config.serialize(serializer)
	}
}

impl OAuth2 {
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

	fn session_cookie_name(&self) -> &str {
		if self.cookie_secure() {
			self
				.config
				.cookie_name
				.as_deref()
				.unwrap_or(DEFAULT_COOKIE_NAME)
		} else {
			self
				.config
				.cookie_name
				.as_deref()
				.unwrap_or(INSECURE_DEFAULT_COOKIE_NAME)
		}
	}

	fn handshake_cookie_base_name(&self) -> &'static str {
		if self.cookie_secure() {
			DEFAULT_HANDSHAKE_COOKIE_NAME
		} else {
			INSECURE_DEFAULT_HANDSHAKE_COOKIE_NAME
		}
	}

	pub fn validate_policy(config: &OAuth2Policy) -> anyhow::Result<()> {
		let redirect_uri = config
			.redirect_uri
			.as_deref()
			.ok_or_else(|| anyhow::anyhow!("oauth2 policy requires redirect_uri"))?;
		let parsed_redirect_uri = ValidatedRedirectUrl::parse(
			redirect_uri,
			config.allow_insecure_redirect_uri,
			"redirect_uri",
		)?;
		if parsed_redirect_uri.as_url().scheme() == "http"
			&& config
				.cookie_name
				.as_deref()
				.is_some_and(|name| name.starts_with("__Host-"))
		{
			anyhow::bail!("__Host- cookie names require https redirect_uri");
		}
		if parsed_redirect_uri.as_url().scheme() == "http"
			&& config
				.cookie_name
				.as_deref()
				.is_some_and(|name| name.starts_with("__Secure-"))
		{
			anyhow::bail!("__Secure- cookie names require https redirect_uri");
		}
		if let Some(uri) = &config.post_logout_redirect_uri {
			ValidatedRedirectUrl::parse(
				uri,
				config.allow_insecure_redirect_uri,
				"post_logout_redirect_uri",
			)?;
		}
		if let Some(max_age) = config.refreshable_cookie_max_age_seconds
			&& max_age == 0
		{
			anyhow::bail!("oauth2 policy refreshable_cookie_max_age_seconds must be > 0");
		}
		if let Some(max_age) = config.refreshable_cookie_max_age_seconds
			&& max_age > MAX_REFRESHABLE_COOKIE_MAX_AGE.as_secs()
		{
			anyhow::bail!(
				"oauth2 policy refreshable_cookie_max_age_seconds must be <= {}",
				MAX_REFRESHABLE_COOKIE_MAX_AGE.as_secs()
			);
		}
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

	fn new_with_overrides(
		config: OAuth2Policy,
		override_metadata: Option<Arc<OidcMetadata>>,
		override_jwt_validator: Option<Arc<Jwt>>,
	) -> anyhow::Result<Self> {
		Self::validate_policy(&config)?;
		let resolved_metadata = match override_metadata {
			Some(metadata) => Some(metadata),
			None => Self::build_resolved_metadata(&config)?,
		};
		let resolved_jwt_validator = match override_jwt_validator {
			Some(validator) => Some(validator),
			None => Self::build_resolved_jwt_validator(&config)?,
		};
		if resolved_metadata.is_none() {
			anyhow::bail!("oauth2 policy requires resolved provider metadata");
		}
		let static_redirect_uri = config
			.redirect_uri
			.as_deref()
			.map(|raw| {
				ValidatedRedirectUrl::parse(raw, config.allow_insecure_redirect_uri, "redirect_uri")
			})
			.transpose()?;
		// Derive distinct keys for session and handshake encryption using HKDF to ensure key separation.
		let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]);
		let prk = salt.extract(config.client_secret.expose_secret().as_bytes());
		let cookie_scope = config.cookie_name.as_deref().unwrap_or_else(|| {
			if static_redirect_uri
				.as_ref()
				.is_some_and(|uri| uri.as_url().scheme() == "http")
			{
				INSECURE_DEFAULT_COOKIE_NAME
			} else {
				DEFAULT_COOKIE_NAME
			}
		});
		let legacy_provider_identity = config.oidc_issuer.as_deref().unwrap_or(&config.provider_id);
		let session_info = format!(
			"agentgateway_session|issuer={}|client_id={}|cookie={cookie_scope}",
			legacy_provider_identity, config.client_id
		);
		let handshake_info = format!(
			"agentgateway_handshake|issuer={}|client_id={}|cookie={cookie_scope}",
			legacy_provider_identity, config.client_id
		);

		let derive_codec = |info: &[u8], aad: &'static [u8]| -> anyhow::Result<SessionCodec> {
			let info_binding = [info];
			let okm = prk
				.expand(&info_binding, hkdf::HKDF_SHA256)
				.map_err(|_| anyhow::anyhow!("HKDF expansion failed"))?;
			let mut key_bytes = [0u8; 32];
			okm
				.fill(&mut key_bytes)
				.map_err(|_| anyhow::anyhow!("HKDF fill failed"))?;
			SessionCodec::new(&key_bytes, aad)
		};

		let session_codec = Arc::new(derive_codec(session_info.as_bytes(), SESSION_COOKIE_AAD)?);
		let handshake_codec = Arc::new(derive_codec(
			handshake_info.as_bytes(),
			HANDSHAKE_STATE_AAD,
		)?);

		Ok(Self {
			config,
			session_codec,
			handshake_codec,
			static_redirect_uri,
			resolved_metadata,
			resolved_jwt_validator,
		})
	}

	pub fn new(config: OAuth2Policy) -> anyhow::Result<Self> {
		Self::new_with_overrides(config, None, None)
	}

	pub fn new_with_resolved_provider(
		config: OAuth2Policy,
		resolved_metadata: Arc<OidcMetadata>,
		resolved_jwt_validator: Option<Arc<Jwt>>,
	) -> anyhow::Result<Self> {
		Self::new_with_overrides(config, Some(resolved_metadata), resolved_jwt_validator)
	}

	#[cfg(test)]
	pub(crate) fn config(&self) -> &OAuth2Policy {
		&self.config
	}

	#[tracing::instrument(
		skip_all,
		fields(provider_id = %self.config.provider_id, client_id = %self.config.client_id)
	)]
	pub async fn apply(
		&self,
		client: &Client,
		policy_client: &PolicyClient,
		oidc: OidcTokenClient<'_>,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		let runtime = OAuth2CallContext {
			client,
			policy_client,
			oidc,
		};
		debug!(path = req.uri().path(), "applying oauth2 policy");
		// Handle logout endpoint.
		if let Some(path) = &self.config.sign_out_path
			&& req.uri().path() == path
		{
			let end_session_endpoint = self.resolve_end_session_endpoint_for_logout();
			return self.handle_logout(req.headers(), end_session_endpoint.as_deref());
		}

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
							Ok(encoded) => {
								let previous_max_chunk_index = self.session_cookie_max_chunk_index(req.headers());
								match self.set_session_cookies(
									encoded,
									Some(previous_max_chunk_index),
									session.cookie_max_age(self.refreshable_cookie_max_age()),
								) {
									Ok(headers) => {
										updated_cookie_headers = Some(headers);
									},
									Err(err) => {
										warn!(error = %err, "failed to persist refreshed oauth2 session; forcing re-authentication");
										updated_cookie_headers =
											Some(self.clear_session_cookies(Some(previous_max_chunk_index)));
										session.expires_at = SystemTime::UNIX_EPOCH;
									},
								}
							},
							Err(err) => {
								debug!("failed to encode refreshed session: {err}");
							},
						},
						_ => {
							debug!("Refresh failed, requiring re-auth");
							let previous_max_chunk_index = self.session_cookie_max_chunk_index(req.headers());
							updated_cookie_headers =
								Some(self.clear_session_cookies(Some(previous_max_chunk_index)));
						},
					}
				} else {
					debug!("Session expired with no refresh token, requiring re-auth");
					let previous_max_chunk_index = self.session_cookie_max_chunk_index(req.headers());
					updated_cookie_headers = Some(self.clear_session_cookies(Some(previous_max_chunk_index)));
				}
			}

			if !session.is_expired() {
				// If a logged-in user revisits callback, send them back to the original target.
				if req.uri().path() == redirect_uri.path() {
					let target = self
						.extract_original_url(req.uri())
						.unwrap_or_else(|| "/".into());
					let target = if Self::is_safe_redirect_target(&target) {
						target
					} else {
						"/".to_string()
					};
					let resp = Response::builder()
						.status(StatusCode::FOUND)
						.header(http::header::LOCATION, target)
						.body(Default::default())
						.map_err(|e| {
							ProxyError::from(Error::Internal(format!(
								"failed to build redirect response: {e}"
							)))
						})?;
					return Ok(PolicyResponse {
						direct_response: Some(resp),
						response_headers: updated_cookie_headers,
					});
				}

				self.inject_auth(req, &session.access_token, session.claims.take());
				return Ok(PolicyResponse {
					direct_response: None,
					response_headers: updated_cookie_headers,
				});
			}
		}

		// Handle callback and authorization-code exchange.
		if req.uri().path() == redirect_uri.path() {
			let (metadata, jwt_validator) = self.resolved_oidc_info()?;
			let callback_response = self
				.handle_callback(
					runtime,
					req.headers(),
					req.uri(),
					CallbackValidation {
						metadata: &metadata,
						jwt_validator: jwt_validator.as_deref(),
					},
					&redirect_uri,
				)
				.await?;
			return Ok(Self::merge_response_headers(
				callback_response,
				updated_cookie_headers,
			));
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

	fn resolve_end_session_endpoint_for_logout(&self) -> Option<String> {
		self
			.resolved_metadata
			.as_ref()
			.and_then(|metadata| metadata.end_session_endpoint.clone())
	}

	fn refreshable_cookie_max_age(&self) -> Duration {
		self
			.config
			.refreshable_cookie_max_age_seconds
			.map(Duration::from_secs)
			.unwrap_or(DEFAULT_REFRESHABLE_COOKIE_MAX_AGE)
	}

	fn clear_session_cookies(
		&self,
		observed_max_chunk_index: Option<usize>,
	) -> crate::http::HeaderMap {
		let cookie_name = self.session_cookie_name();
		let observed_max_chunk = observed_max_chunk_index
			.map(|idx| idx.min(MAX_SESSION_COOKIE_CHUNK_INDEX))
			.unwrap_or(0);
		let clear_end = std::cmp::max(COOKIE_CLEAR_SLOTS, observed_max_chunk.saturating_add(1));

		let mut response_headers = crate::http::HeaderMap::new();
		for i in 0..=clear_end {
			let name = Self::session_cookie_slot_name(cookie_name, i);
			let cookie = self.build_clear_cookie(name);
			Self::append_set_cookie_header(&mut response_headers, &cookie);
		}
		response_headers
	}

	fn session_cookie_slot_name(cookie_name: &str, idx: usize) -> String {
		if idx == 0 {
			cookie_name.to_string()
		} else {
			format!("{}.{}", cookie_name, idx)
		}
	}

	fn build_clear_cookie(&self, name: String) -> Cookie<'static> {
		Cookie::build((name, ""))
			.path("/")
			.secure(self.cookie_secure())
			.http_only(true)
			.max_age(cookie::time::Duration::seconds(0))
			.build()
	}

	fn encode_set_cookie_header(
		cookie: &Cookie<'_>,
	) -> Result<HeaderValue, http::header::InvalidHeaderValue> {
		HeaderValue::from_str(&cookie.to_string())
	}

	fn append_set_cookie_header(headers: &mut crate::http::HeaderMap, cookie: &Cookie<'_>) {
		if let Ok(value) = Self::encode_set_cookie_header(cookie) {
			headers.append(http::header::SET_COOKIE, value);
		}
	}

	fn for_each_request_cookie(headers: &http::HeaderMap, mut f: impl FnMut(Cookie<'_>)) {
		for cookies in headers.get_all(http::header::COOKIE) {
			let cookies = match cookies.to_str() {
				Ok(value) => value,
				Err(err) => {
					debug!("ignoring non-utf8 cookie header: {err}");
					continue;
				},
			};
			for cookie in Cookie::split_parse(cookies) {
				match cookie {
					Ok(cookie) => f(cookie),
					Err(err) => debug!("ignoring malformed cookie: {err}"),
				}
			}
		}
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

	fn extract_original_url(&self, uri: &http::Uri) -> Option<String> {
		let state_str = Self::query_param(uri, "state")?;
		let state = self
			.handshake_codec
			.decrypt_handshake_state(&state_str)
			.ok()?;
		Some(state.original_url)
	}

	fn handle_logout(
		&self,
		req_headers: &http::HeaderMap,
		end_session_endpoint: Option<&str>,
	) -> Result<PolicyResponse, ProxyError> {
		let observed_max_chunk = self.session_cookie_max_chunk_index(req_headers);
		let response_headers = self.clear_session_cookies(Some(observed_max_chunk));

		let end_session_redirect = self
			.get_session(req_headers)
			.and_then(|session| self.build_end_session_redirect(&session, end_session_endpoint));

		let mut resp_builder = Response::builder();
		if let Some(location) = end_session_redirect {
			resp_builder = resp_builder
				.status(StatusCode::FOUND)
				.header(http::header::LOCATION, location.as_str());
		} else {
			resp_builder = resp_builder.status(StatusCode::OK);
		}
		let resp = resp_builder.body(Default::default()).map_err(|e| {
			ProxyError::from(Error::Internal(format!(
				"failed to build logout response: {e}"
			)))
		})?;

		Ok(PolicyResponse {
			direct_response: Some(resp),
			response_headers: Some(response_headers),
		})
	}

	fn build_end_session_redirect(
		&self,
		session: &SessionState,
		end_session_endpoint: Option<&str>,
	) -> Option<Url> {
		let endpoint = end_session_endpoint?;
		let mut redirect = match ValidatedProviderEndpointUrl::parse(endpoint, "end_session_endpoint") {
			Ok(url) => url.into_url(),
			Err(err) => {
				warn!(endpoint, error = %err, "invalid end_session_endpoint from metadata");
				return None;
			},
		};
		let mut query = redirect
			.query_pairs()
			.into_owned()
			.filter(|(k, _)| k != "client_id" && k != "id_token_hint" && k != "post_logout_redirect_uri")
			.collect::<Vec<_>>();
		query.push(("client_id".to_string(), self.config.client_id.clone()));
		if let Some(id_token) = session.id_token.as_deref() {
			query.push(("id_token_hint".to_string(), id_token.to_string()));
		}
		if let Some(post_logout_redirect_uri) = self.config.post_logout_redirect_uri.as_deref() {
			query.push((
				"post_logout_redirect_uri".to_string(),
				post_logout_redirect_uri.to_string(),
			));
		}
		{
			let mut pairs = redirect.query_pairs_mut();
			pairs.clear();
			for (k, v) in &query {
				pairs.append_pair(k, v);
			}
		}
		Some(redirect)
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

	async fn handle_callback(
		&self,
		runtime: OAuth2CallContext<'_>,
		headers: &http::HeaderMap,
		uri: &http::Uri,
		callback: CallbackValidation<'_>,
		redirect_uri: &Url,
	) -> Result<PolicyResponse, ProxyError> {
		let mut code = None;
		let mut state_str = None;
		if let Some(query) = uri.query() {
			for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
				match k.as_ref() {
					"code" if code.is_none() => code = Some(v.into_owned()),
					"state" if state_str.is_none() => state_str = Some(v.into_owned()),
					_ => {},
				}
				if code.is_some() && state_str.is_some() {
					break;
				}
			}
		}
		let code = code.ok_or_else(|| {
			ProxyError::OAuth2AuthenticationFailure(Error::Handshake("missing code".into()))
		})?;
		let state_str = state_str.ok_or_else(|| {
			ProxyError::OAuth2AuthenticationFailure(Error::Handshake("missing state".into()))
		})?;

		// Decrypt Handshake State
		let state = self
			.handshake_codec
			.decrypt_handshake_state(&state_str)
			.map_err(|e| {
				ProxyError::OAuth2AuthenticationFailure(Error::Handshake(format!("invalid state: {e}")))
			})?;

		// Verify Expiry
		if SystemTime::now() > state.expires_at {
			return Err(ProxyError::OAuth2AuthenticationFailure(Error::Handshake(
				"login state expired".into(),
			)));
		}

		// Verify Handshake Isolation (Double Submit Cookie)
		let handshake_cookie_name = format!(
			"{}.{}",
			self.handshake_cookie_base_name(),
			state.handshake_id
		);
		let mut found_binding = false;
		Self::for_each_request_cookie(headers, |cookie| {
			if !found_binding && cookie.name() == handshake_cookie_name {
				found_binding = true;
			}
		});

		if !found_binding {
			return Err(ProxyError::OAuth2AuthenticationFailure(Error::Handshake(
				"handshake browser binding failed (missing or mismatched attempt ID)".into(),
			)));
		}

		// Exchange Code (Manual)
		let token_resp = runtime
			.oidc
			.exchange_code(
				self.oidc_context(runtime),
				ExchangeCodeRequest {
					metadata: callback.metadata,
					code: &code,
					client_id: &self.config.client_id,
					client_secret: self.config.client_secret.expose_secret(),
					redirect_uri: redirect_uri.as_str(),
					code_verifier: state.pkce_verifier.as_deref(),
				},
			)
			.await
			.map_err(|e| ProxyError::OAuth2AuthenticationFailure(Error::Handshake(e.to_string())))?;

		// Validate ID token only when a JWKS validator is available.
		// In explicit-endpoint OAuth2 mode, providers may still return id_token even when no
		// validator is configured; treat it as optional and ignore it in that case.
		let (claims, validated_id_token) = if let Some(id_token) = &token_resp.id_token {
			match callback.jwt_validator {
				Some(jwt_validator) => {
					let claims = jwt_validator.validate_claims(id_token).map_err(|e| {
						ProxyError::OAuth2AuthenticationFailure(Error::InvalidToken(e.to_string()))
					})?;

					// Additional OIDC specific verification: check nonce
					let token_nonce = claims
						.inner
						.get("nonce")
						.and_then(|v| v.as_str())
						.ok_or_else(|| {
							ProxyError::OAuth2AuthenticationFailure(Error::InvalidToken(
								"id_token missing nonce".into(),
							))
						})?;

					if token_nonce != state.nonce {
						return Err(ProxyError::OAuth2AuthenticationFailure(
							Error::InvalidToken("id_token nonce mismatch".into()),
						));
					}

					(Some(claims), Some(id_token.clone()))
				},
				None => {
					warn!(
						"id_token returned but no JWKS validator is configured; ignoring callback id_token"
					);
					(None, None)
				},
			}
		} else {
			(None, None)
		};

		// Create Session
		let expires_in = token_resp.expires_in.unwrap_or(3600);
		let session = SessionState {
			access_token: token_resp.access_token,
			refresh_token: token_resp.refresh_token,
			claims,
			expires_at: SystemTime::now() + Duration::from_secs(expires_in),
			nonce: Some(state.nonce.clone()),
			id_token: validated_id_token,
		};

		// Set Cookies & Redirect
		let cookie_value = self
			.session_codec
			.encode_session(&session)
			.map_err(|e| ProxyError::from(Error::Internal(format!("failed to encode session: {e}"))))?;

		let previous_max_chunk_index = self.session_cookie_max_chunk_index(headers);
		let mut response_headers = self
			.set_session_cookies(
				cookie_value,
				Some(previous_max_chunk_index),
				session.cookie_max_age(self.refreshable_cookie_max_age()),
			)
			.map_err(|err| {
				ProxyError::OAuth2AuthenticationFailure(Error::Handshake(format!(
					"unable to persist oauth2 session: {err}"
				)))
			})?;

		// Cleanup: Clear the specific namespaced handshake cookie
		let clear_handshake = self.build_clear_cookie(handshake_cookie_name);
		response_headers.append(
			http::header::SET_COOKIE,
			Self::encode_set_cookie_header(&clear_handshake).map_err(|e| {
				ProxyError::from(Error::Internal(format!(
					"invalid handshake clear cookie header: {e}"
				)))
			})?,
		);

		let target = if Self::is_safe_redirect_target(&state.original_url) {
			state.original_url.as_str()
		} else {
			"/"
		};
		let resp = Response::builder()
			.status(StatusCode::FOUND)
			.header(http::header::LOCATION, target)
			.body(Default::default())
			.map_err(|e| {
				ProxyError::from(Error::Internal(format!(
					"failed to build callback redirect: {e}"
				)))
			})?;

		Ok(PolicyResponse {
			direct_response: Some(resp),
			response_headers: Some(response_headers),
		})
	}

	fn get_session(&self, headers: &http::HeaderMap) -> Option<SessionState> {
		let cookie_name = self.session_cookie_name();

		let mut chunks = std::collections::HashMap::with_capacity(4);
		Self::for_each_request_cookie(headers, |cookie| {
			if cookie.name() == cookie_name {
				chunks.insert(0, cookie.value().to_string());
			} else if let Some(idx_str) = cookie
				.name()
				.strip_prefix(cookie_name)
				.and_then(|v| v.strip_prefix('.'))
				&& let Ok(idx) = idx_str.parse::<usize>()
				&& idx <= MAX_SESSION_COOKIE_CHUNK_INDEX
			{
				chunks.insert(idx, cookie.value().to_string());
			}
		});

		if chunks.is_empty() {
			return None;
		}

		let (full_value, has_gap) = Self::reassemble_cookie_chunks(chunks);
		if has_gap {
			warn!(
				cookie = cookie_name,
				"session cookie reassembly stopped due to chunk gap"
			);
		}

		self.session_codec.decode_session(&full_value).ok()
	}

	fn reassemble_cookie_chunks(
		mut chunks: std::collections::HashMap<usize, String>,
	) -> (String, bool) {
		let mut full_value = String::new();
		let mut i = 0;
		while let Some(chunk) = chunks.remove(&i) {
			full_value.push_str(&chunk);
			i += 1;
		}
		(full_value, !chunks.is_empty())
	}

	async fn refresh_session(
		&self,
		runtime: OAuth2CallContext<'_>,
		session: &mut SessionState,
		metadata: &OidcMetadata,
		jwt_validator: Option<&Jwt>,
	) -> Result<bool, OidcError> {
		let Some(rt) = &session.refresh_token else {
			return Ok(false);
		};

		let token_resp = runtime
			.oidc
			.refresh_token(
				self.oidc_context(runtime),
				RefreshTokenRequest {
					metadata,
					refresh_token: rt,
					client_id: &self.config.client_id,
					client_secret: self.config.client_secret.expose_secret(),
				},
			)
			.await?;

		session.access_token = token_resp.access_token;
		if let Some(new_rt) = token_resp.refresh_token {
			session.refresh_token = Some(new_rt);
		}
		let expires_in = token_resp.expires_in.unwrap_or(3600);
		session.expires_at = SystemTime::now() + Duration::from_secs(expires_in);
		self.update_session_claims(session, token_resp.id_token.as_deref(), jwt_validator)?;

		Ok(true)
	}

	fn update_session_claims(
		&self,
		session: &mut SessionState,
		id_token: Option<&str>,
		jwt_validator: Option<&Jwt>,
	) -> Result<(), crate::http::jwt::TokenError> {
		match id_token {
			Some(id_token) => {
				let Some(jwt_validator) = jwt_validator else {
					warn!(
						"refresh returned id_token but no JWKS validator is configured; ignoring refreshed id_token"
					);
					return Ok(());
				};
				let claims = jwt_validator.validate_claims(id_token)?;
				// If the refreshed id_token contains a nonce, verify it matches the original.
				// A mismatch means we can't trust this id_token's claims, so clear them
				// rather than preserving stale data from the original login.
				if let Some(token_nonce) = claims.inner.get("nonce").and_then(|v| v.as_str())
					&& let Some(expected) = &session.nonce
					&& token_nonce != expected
				{
					warn!("refreshed id_token nonce mismatch, clearing claims");
					session.claims = None;
					session.id_token = None;
					return Ok(());
				}
				session.claims = Some(claims);
				session.id_token = Some(id_token.to_string());
			},
			None => {
				// Many providers omit id_token on refresh; preserve existing claims for CEL continuity.
			},
		}
		Ok(())
	}

	fn inject_auth(&self, req: &mut Request, access_token: &str, claims: Option<Claims>) {
		req
			.extensions_mut()
			.insert(UpstreamAccessToken(access_token.to_string().into()));

		// Inject claims into extensions for RBAC/logging
		if let Some(claims) = claims {
			req.extensions_mut().insert(claims);
		}
	}

	fn set_session_cookies(
		&self,
		value: String,
		previous_max_chunk_index: Option<usize>,
		cookie_max_age: cookie::time::Duration,
	) -> Result<crate::http::HeaderMap, SessionCookieError> {
		let cookie_name = self.session_cookie_name();
		let mut headers = crate::http::HeaderMap::new();
		let required_chunks = value.len().div_ceil(MAX_COOKIE_SIZE);
		if required_chunks > MAX_SESSION_COOKIE_CHUNK_INDEX.saturating_add(1) {
			return Err(SessionCookieError::TooLarge);
		}

		let mut i = 0;
		if value.len() <= MAX_COOKIE_SIZE {
			let cookie = self.build_session_cookie(
				Self::session_cookie_slot_name(cookie_name, 0),
				value,
				cookie_max_age,
			);
			Self::append_set_cookie_header(&mut headers, &cookie);
			i = 1;
		} else {
			// Chunking
			let mut remaining = &value[..];
			while !remaining.is_empty() {
				let chunk_size = std::cmp::min(remaining.len(), MAX_COOKIE_SIZE);
				let chunk = &remaining[..chunk_size];
				remaining = &remaining[chunk_size..];

				let name = Self::session_cookie_slot_name(cookie_name, i);
				let cookie = self.build_session_cookie(name, chunk.to_string(), cookie_max_age);
				Self::append_set_cookie_header(&mut headers, &cookie);
				i += 1;
			}
		}

		// Cleanup potential stale chunks from previous sessions.
		let observed_max_chunk = previous_max_chunk_index
			.map(|idx| idx.min(MAX_SESSION_COOKIE_CHUNK_INDEX))
			.unwrap_or(0);
		let clear_end = std::cmp::max(i + COOKIE_CLEAR_SLOTS, observed_max_chunk.saturating_add(1));
		for j in i..clear_end {
			let name = Self::session_cookie_slot_name(cookie_name, j);
			let cookie = self.build_clear_cookie(name);
			Self::append_set_cookie_header(&mut headers, &cookie);
		}

		Ok(headers)
	}

	fn session_cookie_max_chunk_index(&self, headers: &http::HeaderMap) -> usize {
		let cookie_name = self.session_cookie_name();

		let mut max_idx = 0usize;
		Self::for_each_request_cookie(headers, |cookie| {
			if cookie.name() == cookie_name {
				return;
			}
			if let Some(idx_str) = cookie
				.name()
				.strip_prefix(cookie_name)
				.and_then(|v| v.strip_prefix('.'))
				&& let Ok(idx) = idx_str.parse::<usize>()
				&& idx <= MAX_SESSION_COOKIE_CHUNK_INDEX
			{
				max_idx = max_idx.max(idx);
			}
		});
		max_idx
	}

	fn query_param(uri: &http::Uri, name: &str) -> Option<String> {
		let query = uri.query()?;
		url::form_urlencoded::parse(query.as_bytes())
			.find_map(|(k, v)| (k == name).then(|| v.into_owned()))
	}

	fn build_session_cookie(
		&self,
		name: String,
		value: String,
		cookie_max_age: cookie::time::Duration,
	) -> Cookie<'static> {
		Cookie::build((name, value))
			.path("/")
			.secure(self.cookie_secure())
			.http_only(true)
			.same_site(SameSite::Lax)
			.max_age(cookie_max_age)
			.build()
	}

	async fn trigger_auth(
		&self,
		headers: &http::HeaderMap,
		uri: &http::Uri,
		metadata: &OidcMetadata,
		redirect_uri: &Url,
	) -> Result<PolicyResponse, ProxyError> {
		let requested_scopes: Vec<String> = if self.config.scopes.is_empty() {
			DEFAULT_SCOPE_PARAM
				.split_whitespace()
				.map(ToOwned::to_owned)
				.collect()
		} else {
			self
				.config
				.scopes
				.iter()
				.flat_map(|scope| scope.split_whitespace())
				.map(ToOwned::to_owned)
				.collect()
		};
		let scope_string = if requested_scopes.is_empty() {
			Cow::Borrowed(DEFAULT_SCOPE_PARAM)
		} else {
			Cow::Owned(requested_scopes.join(" "))
		};

		if self.should_return_unauthorized(headers) {
			// API Client -> 401
			let resp = Response::builder()
				.status(StatusCode::UNAUTHORIZED)
				.header(
					http::header::WWW_AUTHENTICATE,
					format!(
						"Bearer realm=\"{}\", scope=\"{}\"",
						self.auth_realm(),
						scope_string.as_ref()
					),
				)
				.body(Default::default())
				.map_err(|e| {
					ProxyError::from(Error::Internal(format!(
						"failed to build unauthorized response: {e}"
					)))
				})?;
			return Ok(PolicyResponse::default().with_response(resp));
		}

		// Browser -> 302 Redirect
		let nonce = Self::random_token();
		let handshake_id = Self::random_token();
		let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

		let state = HandshakeState {
			original_url: Self::original_target_from_uri(uri),
			nonce,
			pkce_verifier: Some(pkce_verifier.secret().to_string()),
			expires_at: SystemTime::now() + STATE_TTL,
			handshake_id: handshake_id.clone(),
		};

		let encrypted_state = self
			.handshake_codec
			.encrypt_handshake_state(&state)
			.map_err(|e| ProxyError::from(Error::Internal(format!("failed to encrypt state: {e}"))))?;

		let auth_url = BasicClient::new(ClientId::new(self.config.client_id.clone()))
			.set_auth_uri(
				AuthUrl::new(metadata.authorization_endpoint.clone())
					.map_err(|e| ProxyError::from(Error::Internal(format!("invalid auth endpoint: {e}"))))?,
			)
			.set_token_uri(
				TokenUrl::new(metadata.token_endpoint.clone())
					.map_err(|e| ProxyError::from(Error::Internal(format!("invalid token endpoint: {e}"))))?,
			)
			.set_redirect_uri(
				RedirectUrl::new(redirect_uri.as_str().to_string()).map_err(|e| {
					ProxyError::from(Error::Internal(format!(
						"invalid redirect uri for oauth2 client: {e}"
					)))
				})?,
			);
		let mut auth_request = auth_url
			.authorize_url(|| CsrfToken::new(encrypted_state.clone()))
			.add_extra_param("nonce", state.nonce.clone())
			.set_pkce_challenge(pkce_challenge);
		for scope in requested_scopes {
			auth_request = auth_request.add_scope(Scope::new(scope));
		}
		let (auth_url, _csrf) = auth_request.url();

		// Set namespaced handshake cookie for Browser Binding (Handshake Isolation)
		let handshake_cookie_name = format!("{}.{}", self.handshake_cookie_base_name(), handshake_id);
		let handshake_cookie = Cookie::build((handshake_cookie_name, "1"))
			.path("/")
			.secure(self.cookie_secure())
			.http_only(true)
			.same_site(SameSite::Lax)
			.max_age(cookie::time::Duration::seconds(STATE_TTL.as_secs() as i64))
			.build();

		let mut response_headers = crate::http::HeaderMap::new();
		response_headers.insert(
			http::header::SET_COOKIE,
			HeaderValue::from_str(&handshake_cookie.to_string()).map_err(|e| {
				ProxyError::from(Error::Internal(format!(
					"invalid handshake cookie header: {e}"
				)))
			})?,
		);

		let resp = Response::builder()
			.status(StatusCode::FOUND)
			.header(http::header::LOCATION, auth_url.as_str())
			.body(Default::default())
			.map_err(|e| {
				ProxyError::from(Error::Internal(format!(
					"failed to build auth redirect response: {e}"
				)))
			})?;

		Ok(PolicyResponse {
			direct_response: Some(resp),
			response_headers: Some(response_headers),
		})
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

	fn random_token() -> String {
		let mut bytes = [0u8; 32];
		let mut rng = rand::rng();
		rng.fill_bytes(&mut bytes);
		URL_SAFE_NO_PAD.encode(bytes)
	}

	fn original_target_from_uri(uri: &http::Uri) -> String {
		let path = uri.path();
		if !path.starts_with('/') {
			return "/".to_string();
		}
		match uri.query() {
			Some(query) => format!("{path}?{query}"),
			None => path.to_string(),
		}
	}

	fn is_safe_redirect_target(target: &str) -> bool {
		target.starts_with('/')
			&& !target.starts_with("//")
			&& !target.contains('\\')
			&& !target.chars().any(char::is_control)
	}

	fn is_allowed_redirect_url(url: &Url, allow_insecure_redirect_uri: bool) -> bool {
		if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
			return false;
		}
		match url.scheme() {
			"https" => url.host_str().is_some(),
			"http" => url
				.host_str()
				.is_some_and(|host| allow_insecure_redirect_uri || crate::http::is_loopback_host(host)),
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

// --- Helper Structs ---

#[derive(Debug)]
struct SessionCodec {
	key: LessSafeKey,
	aad: &'static [u8],
}

impl SessionCodec {
	fn new(key_bytes: &[u8], aad: &'static [u8]) -> anyhow::Result<Self> {
		let unbound =
			UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| anyhow::anyhow!("invalid key"))?;
		Ok(Self {
			key: LessSafeKey::new(unbound),
			aad,
		})
	}

	fn encrypt_handshake_state(&self, state: &HandshakeState) -> anyhow::Result<String> {
		let json = serde_json::to_vec(state)?;
		self.encrypt(&json)
	}

	fn decrypt_handshake_state(&self, encoded: &str) -> anyhow::Result<HandshakeState> {
		let json = self.decrypt(encoded)?;
		Ok(serde_json::from_slice(&json)?)
	}

	fn encode_session(&self, session: &SessionState) -> anyhow::Result<String> {
		let json = serde_json::to_vec(session)?;
		self.encrypt(&json)
	}

	fn decode_session(&self, encoded: &str) -> anyhow::Result<SessionState> {
		let json = self.decrypt(encoded)?;
		Ok(serde_json::from_slice(&json)?)
	}

	fn encrypt(&self, plaintext: &[u8]) -> anyhow::Result<String> {
		let mut nonce_bytes = [0u8; 12];
		let mut rng = rand::rng();
		rng.fill_bytes(&mut nonce_bytes);
		let nonce = Nonce::assume_unique_for_key(nonce_bytes);

		// Pre-allocate plaintext + tag (16 bytes), then encrypt in place.
		let mut in_out = Vec::with_capacity(plaintext.len() + AES_256_GCM.tag_len());
		in_out.extend_from_slice(plaintext);

		self
			.key
			.seal_in_place_append_tag(nonce, Aad::from(self.aad), &mut in_out)
			.map_err(|_| anyhow::anyhow!("encryption failed"))?;

		// Prefix nonce so decoding can reconstruct the AEAD input.
		let mut result = Vec::with_capacity(12 + in_out.len());
		result.extend_from_slice(&nonce_bytes);
		result.extend_from_slice(&in_out);

		Ok(URL_SAFE_NO_PAD.encode(result))
	}

	fn decrypt(&self, encoded: &str) -> anyhow::Result<Vec<u8>> {
		let mut data = URL_SAFE_NO_PAD
			.decode(encoded)
			.map_err(|e| anyhow::anyhow!("base64 decode failed: {e}"))?;

		let tag_len = AES_256_GCM.tag_len();
		if data.len() < 12 + tag_len {
			anyhow::bail!("data too short");
		}

		let nonce = Nonce::try_assume_unique_for_key(&data[..12])
			.map_err(|_| anyhow::anyhow!("invalid nonce"))?;

		let plaintext_len = {
			let in_out = &mut data[12..];
			let plaintext = self
				.key
				.open_in_place(nonce, Aad::from(self.aad), in_out)
				.map_err(|_| anyhow::anyhow!("decryption failed"))?;
			plaintext.len()
		};

		// Shift data left to remove nonce
		data.copy_within(12..12 + plaintext_len, 0);
		data.truncate(plaintext_len);
		Ok(data)
	}
}

#[derive(Serialize, Deserialize)]
struct HandshakeState {
	original_url: String,
	nonce: String,
	pkce_verifier: Option<String>,
	expires_at: SystemTime,
	#[serde(default)]
	handshake_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct SessionState {
	access_token: String,
	refresh_token: Option<String>,
	claims: Option<Claims>,
	expires_at: SystemTime,
	#[serde(default)]
	nonce: Option<String>,
	#[serde(default)]
	id_token: Option<String>,
}

struct CallbackValidation<'a> {
	metadata: &'a OidcMetadata,
	jwt_validator: Option<&'a Jwt>,
}

impl SessionState {
	fn is_expired(&self) -> bool {
		SystemTime::now() > self.expires_at
	}

	fn cookie_max_age(&self, refreshable_cookie_max_age: Duration) -> cookie::time::Duration {
		if self.refresh_token.is_some() {
			let seconds = i64::try_from(refreshable_cookie_max_age.as_secs()).unwrap_or(i64::MAX);
			return cookie::time::Duration::seconds(seconds);
		}
		let remaining = self
			.expires_at
			.duration_since(SystemTime::now())
			.unwrap_or_default();
		let seconds = remaining.as_secs().max(1);
		let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
		cookie::time::Duration::seconds(seconds)
	}
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
			allow_insecure_redirect_uri: false,
			scopes: vec![],
			cookie_name: None,
			refreshable_cookie_max_age_seconds: None,
			sign_out_path: None,
			post_logout_redirect_uri: None,
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
		let inputs = Arc::new(crate::ProxyInputs {
			cfg: Arc::new(config),
			stores: stores.clone(),
			metrics: Arc::new(crate::metrics::Metrics::new(
				agent_core::metrics::sub_registry(&mut Registry::default()),
				Default::default(),
			)),
			upstream,
			ca: None,
			mcp_state: crate::mcp::App::new(stores, encoder),
		});
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

	fn test_oauth2() -> OAuth2 {
		OAuth2::new(test_config()).unwrap()
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
	fn oauth2_new_validates_redirect_uri_rules() {
		struct Case {
			name: &'static str,
			redirect_uri: Option<&'static str>,
			allow_insecure_redirect_uri: bool,
			cookie_name: Option<&'static str>,
			want_err: Option<&'static str>,
		}

		let cases = [
			Case {
				name: "requires redirect uri",
				redirect_uri: None,
				allow_insecure_redirect_uri: false,
				cookie_name: None,
				want_err: Some("requires redirect_uri"),
			},
			Case {
				name: "rejects invalid uri",
				redirect_uri: Some("not-a-valid-uri"),
				allow_insecure_redirect_uri: false,
				cookie_name: None,
				want_err: Some("invalid redirect_uri config"),
			},
			Case {
				name: "rejects non loopback http by default",
				redirect_uri: Some("http://app.example.com/callback"),
				allow_insecure_redirect_uri: false,
				cookie_name: None,
				want_err: Some("redirect_uri must use https (or http on loopback hosts"),
			},
			Case {
				name: "accepts non loopback http when explicitly allowed",
				redirect_uri: Some("http://app.example.com/callback"),
				allow_insecure_redirect_uri: true,
				cookie_name: None,
				want_err: None,
			},
			Case {
				name: "accepts loopback http",
				redirect_uri: Some("http://127.0.0.1:3000/callback"),
				allow_insecure_redirect_uri: false,
				cookie_name: None,
				want_err: None,
			},
			Case {
				name: "rejects non http https scheme",
				redirect_uri: Some("ftp://example.com/callback"),
				allow_insecure_redirect_uri: false,
				cookie_name: None,
				want_err: Some("redirect_uri must use https (or http on loopback hosts"),
			},
			Case {
				name: "rejects host cookie on insecure redirect",
				redirect_uri: Some("http://127.0.0.1:3000/callback"),
				allow_insecure_redirect_uri: false,
				cookie_name: Some("__Host-custom"),
				want_err: Some("__Host- cookie names require https redirect_uri"),
			},
			Case {
				name: "rejects secure cookie on insecure redirect",
				redirect_uri: Some("http://127.0.0.1:3000/callback"),
				allow_insecure_redirect_uri: false,
				cookie_name: Some("__Secure-custom"),
				want_err: Some("__Secure- cookie names require https redirect_uri"),
			},
		];

		for case in cases {
			let mut config = test_config();
			config.redirect_uri = case.redirect_uri.map(ToOwned::to_owned);
			config.allow_insecure_redirect_uri = case.allow_insecure_redirect_uri;
			config.cookie_name = case.cookie_name.map(ToOwned::to_owned);

			match case.want_err {
				Some(want_err) => {
					let err = OAuth2::new(config).unwrap_err();
					assert!(
						err.to_string().contains(want_err),
						"case {:?}: unexpected error: {err}",
						case.name
					);
				},
				None => {
					assert!(
						OAuth2::new(config).is_ok(),
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
		let err = OAuth2::new(config).unwrap_err();
		assert!(
			err
				.to_string()
				.contains("oauth2 policy requires resolved provider metadata")
		);
	}

	#[test]
	fn oauth2_new_validates_post_logout_redirect_uri_rules() {
		struct Case {
			name: &'static str,
			post_logout_redirect_uri: &'static str,
			allow_insecure_redirect_uri: bool,
			want_err: Option<&'static str>,
		}

		let cases = [
			Case {
				name: "rejects invalid uri",
				post_logout_redirect_uri: "not-a-url",
				allow_insecure_redirect_uri: false,
				want_err: Some("invalid post_logout_redirect_uri config"),
			},
			Case {
				name: "rejects non loopback http by default",
				post_logout_redirect_uri: "http://app.example.com/signed-out",
				allow_insecure_redirect_uri: false,
				want_err: Some("post_logout_redirect_uri must use https (or http on loopback hosts"),
			},
			Case {
				name: "accepts non loopback http when explicitly allowed",
				post_logout_redirect_uri: "http://app.example.com/signed-out",
				allow_insecure_redirect_uri: true,
				want_err: None,
			},
			Case {
				name: "rejects fragment",
				post_logout_redirect_uri: "https://app.example.com/signed-out#fragment",
				allow_insecure_redirect_uri: false,
				want_err: Some("must not contain a fragment"),
			},
			Case {
				name: "rejects userinfo",
				post_logout_redirect_uri: "https://user:pass@app.example.com/signed-out",
				allow_insecure_redirect_uri: false,
				want_err: Some("must not include userinfo"),
			},
			Case {
				name: "accepts loopback http",
				post_logout_redirect_uri: "http://127.0.0.1:3000/signed-out",
				allow_insecure_redirect_uri: false,
				want_err: None,
			},
			Case {
				name: "rejects non http https scheme",
				post_logout_redirect_uri: "ftp://app.example.com/signed-out",
				allow_insecure_redirect_uri: false,
				want_err: Some("post_logout_redirect_uri must use https (or http on loopback hosts"),
			},
		];

		for case in cases {
			let mut config = test_config();
			config.post_logout_redirect_uri = Some(case.post_logout_redirect_uri.to_string());
			config.allow_insecure_redirect_uri = case.allow_insecure_redirect_uri;

			match case.want_err {
				Some(want_err) => {
					let err = OAuth2::new(config).unwrap_err();
					assert!(
						err.to_string().contains(want_err),
						"case {:?}: unexpected error: {err}",
						case.name
					);
				},
				None => {
					assert!(
						OAuth2::new(config).is_ok(),
						"case {:?} should succeed",
						case.name
					);
				},
			}
		}
	}

	#[test]
	fn oauth2_new_validates_refreshable_cookie_max_age() {
		let cases = [
			("zero", 0, "refreshable_cookie_max_age_seconds must be > 0"),
			(
				"too large",
				MAX_REFRESHABLE_COOKIE_MAX_AGE.as_secs() + 1,
				"refreshable_cookie_max_age_seconds must be <=",
			),
		];

		for (name, max_age, want_err) in cases {
			let mut config = test_config();
			config.refreshable_cookie_max_age_seconds = Some(max_age);

			let err = OAuth2::new(config).unwrap_err();
			assert!(
				err.to_string().contains(want_err),
				"case {:?}: unexpected error: {err}",
				name
			);
		}
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
					let err = OAuth2::new(config).unwrap_err();
					assert!(
						err.to_string().contains(want_err),
						"case {:?}: unexpected error: {err}",
						case.name
					);
				},
				None => {
					assert!(
						OAuth2::new(config).is_ok(),
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
		let legacy_oauth2 = OAuth2::new(legacy).unwrap();

		let mut updated = test_config();
		updated.provider_id = "provider-b".to_string();
		let updated_oauth2 = OAuth2::new(updated).unwrap();

		let session = SessionState {
			access_token: "access-token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
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
	async fn oauth2_apply_uses_resolved_metadata_without_discovery() {
		let mut config = test_config();
		config.resolved_provider = Some(Box::new(crate::types::agent::ResolvedOAuth2Provider {
			authorization_endpoint: "https://issuer.example.com/authorize".to_string(),
			token_endpoint: "https://issuer.example.com/token".to_string(),
			jwks_inline: Some(resolved_test_jwks_inline()),
			end_session_endpoint: Some("https://issuer.example.com/logout".to_string()),
			token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
		}));
		let oauth2 = OAuth2::new(config).unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let mut req = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "/private/data".parse().unwrap();
		req
			.headers_mut()
			.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));

		let response = oauth2
			.apply(&client, &policy_client, oidc.tokens(), &mut req)
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
		let oauth2 = OAuth2::new(config).unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());

		let mut initial_req = Request::new(crate::http::Body::empty());
		*initial_req.uri_mut() = "/private/data".parse().unwrap();
		initial_req
			.headers_mut()
			.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));

		let initial = oauth2
			.apply(&client, &policy_client, oidc.tokens(), &mut initial_req)
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
			.apply(&client, &policy_client, oidc.tokens(), &mut callback_req)
			.await
			.expect("callback should succeed when id_token is returned without validator");
		let callback_redirect = callback
			.direct_response
			.expect("callback should redirect to original target");
		assert_eq!(callback_redirect.status(), StatusCode::FOUND);

		let callback_set_cookie_values: Vec<String> = callback
			.response_headers
			.expect("callback should set session cookies")
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
		assert!(session.claims.is_none());
	}

	#[test]
	fn refreshable_cookie_max_age_uses_policy_override() {
		let mut oauth2 = test_oauth2();
		oauth2.config.refreshable_cookie_max_age_seconds = Some(1800);
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(30),
			nonce: None,
			id_token: None,
		};
		assert_eq!(
			oauth2.refreshable_cookie_max_age(),
			Duration::from_secs(1800)
		);
		assert_eq!(
			session.cookie_max_age(oauth2.refreshable_cookie_max_age()),
			cookie::time::Duration::seconds(1800),
		);
	}

	#[test]
	fn refreshable_cookie_max_age_accepts_upper_bound() {
		let mut oauth2 = test_oauth2();
		oauth2.config.refreshable_cookie_max_age_seconds =
			Some(MAX_REFRESHABLE_COOKIE_MAX_AGE.as_secs());
		assert_eq!(
			oauth2.refreshable_cookie_max_age(),
			MAX_REFRESHABLE_COOKIE_MAX_AGE
		);
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
	fn update_session_claims_preserves_claims_without_id_token() {
		let oauth2 = test_oauth2();
		let mut session = SessionState {
			access_token: "a".to_string(),
			refresh_token: None,
			claims: Some(Claims {
				inner: Map::from_iter([("sub".to_string(), Value::String("user".to_string()))]),
				jwt: SecretString::new("header.payload.sig".into()),
			}),
			expires_at: SystemTime::now(),
			nonce: None,
			id_token: Some("existing-id-token".to_string()),
		};
		let jwt = Jwt::from_providers(vec![], crate::http::jwt::Mode::Strict);

		oauth2
			.update_session_claims(&mut session, None, Some(&jwt))
			.unwrap();
		assert_eq!(
			session
				.claims
				.as_ref()
				.and_then(|claims| claims.inner.get("sub"))
				.and_then(|v| v.as_str()),
			Some("user")
		);
		assert_eq!(session.id_token.as_deref(), Some("existing-id-token"));
	}

	#[test]
	fn update_session_claims_invalid_id_token_keeps_existing_claims() {
		let oauth2 = test_oauth2();
		let mut session = SessionState {
			access_token: "a".to_string(),
			refresh_token: None,
			claims: Some(Claims {
				inner: Map::from_iter([("sub".to_string(), Value::String("user".to_string()))]),
				jwt: SecretString::new("header.payload.sig".into()),
			}),
			expires_at: SystemTime::now(),
			nonce: None,
			id_token: Some("existing-id-token".to_string()),
		};
		let jwt = Jwt::from_providers(vec![], crate::http::jwt::Mode::Strict);

		assert!(
			oauth2
				.update_session_claims(&mut session, Some("not-a-jwt"), Some(&jwt))
				.is_err()
		);
		assert!(session.claims.is_some());
		assert_eq!(session.id_token.as_deref(), Some("existing-id-token"));
	}

	#[test]
	fn update_session_claims_without_validator_ignores_refreshed_id_token() {
		let oauth2 = test_oauth2();
		let mut session = SessionState {
			access_token: "access".to_string(),
			refresh_token: Some("refresh".to_string()),
			claims: Some(Claims {
				inner: serde_json::json!({
					"sub": "existing-user",
				})
				.as_object()
				.cloned()
				.unwrap(),
				jwt: SecretString::new("header.payload.sig".into()),
			}),
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: Some("existing-id-token".to_string()),
		};

		oauth2
			.update_session_claims(&mut session, Some("new-id-token"), None)
			.expect("missing validator should not fail refresh handling");

		assert_eq!(
			session
				.claims
				.as_ref()
				.and_then(|claims| claims.inner.get("sub"))
				.and_then(|v| v.as_str()),
			Some("existing-user")
		);
		assert_eq!(session.id_token.as_deref(), Some("existing-id-token"));
	}

	#[test]
	fn update_session_claims_nonce_mismatch_clears_claims() {
		let oauth2 = test_oauth2();

		let ec_key = jsonwebtoken::EncodingKey::from_ec_pem(
			concat!(
				"-----BEGIN PRIVATE KEY-----\n",
				"MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgXwpfmh19sVcCixou\n",
				"FK98emEN4f5pOK8BVMlL29Gh13ChRANCAARZ9RKwGWYq1NfxF+aj0r7o+wobVizD\n",
				"WPdK35lRlKrgdbzv0dJI193daM/tmlLaaFnwafsLu2MTv14xkh7+NLYD\n",
				"-----END PRIVATE KEY-----\n",
			)
			.as_bytes(),
		)
		.unwrap();
		let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
		header.kid = Some("test-nonce-kid".to_string());
		let claims_map = json!({
			"sub": "user",
			"iss": "https://issuer.example.com",
			"aud": "client-id",
			"nonce": "wrong-nonce",
			"exp": (SystemTime::now() + Duration::from_secs(3600))
				.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
			"iat": SystemTime::now()
				.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
		});
		let token = jsonwebtoken::encode(&header, &claims_map, &ec_key).unwrap();

		let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_value(json!({
			"keys": [{
				"kty": "EC",
				"crv": "P-256",
				"kid": "test-nonce-kid",
				"alg": "ES256",
				"x": "WfUSsBlmKtTX8Rfmo9K-6PsKG1Ysw1j3St-ZUZSq4HU",
				"y": "vO_R0kjX3d1oz-2aUtpoWfBp-wu7YxO_XjGSHv40tgM",
				"use": "sig"
			}]
		}))
		.unwrap();
		let provider = crate::http::jwt::Provider::from_jwks(
			jwks,
			"https://issuer.example.com".to_string(),
			Some(vec!["client-id".to_string()]),
			crate::http::jwt::JWTValidationOptions::default(),
		)
		.unwrap();
		let jwt = Jwt::from_providers(vec![provider], crate::http::jwt::Mode::Strict);

		let mut session = SessionState {
			access_token: "a".to_string(),
			refresh_token: None,
			claims: Some(Claims {
				inner: Map::from_iter([(
					"sub".to_string(),
					Value::String("original-user".to_string()),
				)]),
				jwt: SecretString::new("old.token.sig".into()),
			}),
			expires_at: SystemTime::now(),
			nonce: Some("expected-nonce".to_string()),
			id_token: Some("old-id-token".to_string()),
		};

		oauth2
			.update_session_claims(&mut session, Some(&token), Some(&jwt))
			.unwrap();
		assert!(
			session.claims.is_none(),
			"nonce mismatch on refresh must clear claims"
		);
		assert!(
			session.id_token.is_none(),
			"nonce mismatch on refresh must clear id_token hint"
		);
	}

	#[test]
	fn update_session_claims_nonce_match_updates_claims() {
		let oauth2 = test_oauth2();

		let ec_key = jsonwebtoken::EncodingKey::from_ec_pem(
			concat!(
				"-----BEGIN PRIVATE KEY-----\n",
				"MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgXwpfmh19sVcCixou\n",
				"FK98emEN4f5pOK8BVMlL29Gh13ChRANCAARZ9RKwGWYq1NfxF+aj0r7o+wobVizD\n",
				"WPdK35lRlKrgdbzv0dJI193daM/tmlLaaFnwafsLu2MTv14xkh7+NLYD\n",
				"-----END PRIVATE KEY-----\n",
			)
			.as_bytes(),
		)
		.unwrap();
		let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
		header.kid = Some("test-nonce-kid".to_string());
		let claims_map = json!({
			"sub": "refreshed-user",
			"iss": "https://issuer.example.com",
			"aud": "client-id",
			"nonce": "expected-nonce",
			"exp": (SystemTime::now() + Duration::from_secs(3600))
				.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
			"iat": SystemTime::now()
				.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
		});
		let token = jsonwebtoken::encode(&header, &claims_map, &ec_key).unwrap();

		let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_value(json!({
			"keys": [{
				"kty": "EC",
				"crv": "P-256",
				"kid": "test-nonce-kid",
				"alg": "ES256",
				"x": "WfUSsBlmKtTX8Rfmo9K-6PsKG1Ysw1j3St-ZUZSq4HU",
				"y": "vO_R0kjX3d1oz-2aUtpoWfBp-wu7YxO_XjGSHv40tgM",
				"use": "sig"
			}]
		}))
		.unwrap();
		let provider = crate::http::jwt::Provider::from_jwks(
			jwks,
			"https://issuer.example.com".to_string(),
			Some(vec!["client-id".to_string()]),
			crate::http::jwt::JWTValidationOptions::default(),
		)
		.unwrap();
		let jwt = Jwt::from_providers(vec![provider], crate::http::jwt::Mode::Strict);

		let mut session = SessionState {
			access_token: "a".to_string(),
			refresh_token: None,
			claims: None,
			expires_at: SystemTime::now(),
			nonce: Some("expected-nonce".to_string()),
			id_token: None,
		};

		oauth2
			.update_session_claims(&mut session, Some(&token), Some(&jwt))
			.unwrap();
		let claims = session
			.claims
			.as_ref()
			.expect("matching nonce must update claims");
		assert_eq!(
			claims.inner.get("sub").and_then(|v| v.as_str()),
			Some("refreshed-user"),
		);
		assert_eq!(session.id_token.as_deref(), Some(token.as_str()));
	}

	#[test]
	fn inject_auth_sets_upstream_access_token_extension() {
		let oauth2 = OAuth2::new(test_config()).unwrap();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			claims: None,
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
		let oauth2 = OAuth2::new(test_config()).unwrap();
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
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let cookies = format!("{DEFAULT_COOKIE_NAME}={encoded}; {DEFAULT_COOKIE_NAME}evil.1=malicious");

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
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();

		let mut headers = http::HeaderMap::new();
		headers.append(http::header::COOKIE, HeaderValue::from_static("other=1"));
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let decoded = oauth2
			.get_session(&headers)
			.expect("session should decode when cookies are split across headers");
		assert_eq!(decoded.access_token, "token");
	}

	#[test]
	fn get_session_prefers_last_cookie_value_across_split_headers() {
		let oauth2 = test_oauth2();
		let first = SessionState {
			access_token: "first-token".to_string(),
			refresh_token: None,
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let second = SessionState {
			access_token: "second-token".to_string(),
			refresh_token: None,
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let first_encoded = oauth2.session_codec.encode_session(&first).unwrap();
		let second_encoded = oauth2.session_codec.encode_session(&second).unwrap();

		let mut headers = http::HeaderMap::new();
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={first_encoded}")).unwrap(),
		);
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={second_encoded}")).unwrap(),
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
		let oauth2 = OAuth2::new(config).unwrap();
		assert_eq!(oauth2.session_cookie_name(), INSECURE_DEFAULT_COOKIE_NAME);
		assert!(!oauth2.cookie_secure());

		let cookie = oauth2.build_session_cookie(
			oauth2.session_cookie_name().to_string(),
			"value".to_string(),
			cookie::time::Duration::seconds(60),
		);
		assert_eq!(cookie.secure(), Some(false));
	}

	#[test]
	fn large_session_is_chunked_and_round_trips() {
		let oauth2 = test_oauth2();
		let large_token = "a".repeat(MAX_COOKIE_SIZE * 2 + 512);
		let session = SessionState {
			access_token: large_token.clone(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: None,
			id_token: None,
		};

		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		assert!(encoded.len() > MAX_COOKIE_SIZE);

		let response_headers = oauth2
			.set_session_cookies(
				encoded,
				None,
				session.cookie_max_age(oauth2.refreshable_cookie_max_age()),
			)
			.expect("session should fit within configured cookie chunk budget");
		let set_cookie_values: Vec<String> = response_headers
			.get_all(http::header::SET_COOKIE)
			.iter()
			.filter_map(|value| value.to_str().ok().map(ToOwned::to_owned))
			.collect();
		assert!(
			set_cookie_values
				.iter()
				.any(|v| v.starts_with(&format!("{DEFAULT_COOKIE_NAME}.1="))),
			"session cookie should be chunked into multiple cookies"
		);

		let cookie_header =
			request_cookie_header_from_set_cookie_values(&set_cookie_values, DEFAULT_COOKIE_NAME);
		let mut request_headers = http::HeaderMap::new();
		request_headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&cookie_header).unwrap(),
		);

		let decoded = oauth2
			.get_session(&request_headers)
			.expect("chunked session should decode");
		assert_eq!(decoded.access_token, large_token);
		assert_eq!(decoded.refresh_token.as_deref(), Some("refresh-token"));
	}

	#[test]
	fn set_session_cookies_rejects_values_above_chunk_budget() {
		let oauth2 = test_oauth2();
		let too_large = "a".repeat(MAX_COOKIE_SIZE * (MAX_SESSION_COOKIE_CHUNK_INDEX + 2));
		let result = oauth2.set_session_cookies(too_large, None, cookie::time::Duration::seconds(60));
		assert!(matches!(result, Err(SessionCookieError::TooLarge)));
	}

	#[test]
	fn session_cookie_gap_returns_none() {
		let oauth2 = test_oauth2();
		let session = SessionState {
			access_token: "b".repeat(MAX_COOKIE_SIZE * 2 + 512),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let encoded_bytes = encoded.as_bytes();
		let chunks = encoded_bytes.chunks(MAX_COOKIE_SIZE).collect::<Vec<_>>();
		assert!(chunks.len() >= 3, "test requires at least three chunks");

		let cookie_header = chunks
			.iter()
			.enumerate()
			.filter_map(|(idx, chunk)| {
				if idx == 1 {
					return None;
				}
				let value = std::str::from_utf8(chunk).expect("base64 should be utf8");
				let name = if idx == 0 {
					DEFAULT_COOKIE_NAME.to_string()
				} else {
					format!("{DEFAULT_COOKIE_NAME}.{idx}")
				};
				Some(format!("{name}={value}"))
			})
			.collect::<Vec<_>>()
			.join("; ");

		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&cookie_header).unwrap(),
		);
		assert!(oauth2.get_session(&headers).is_none());
	}

	#[test]
	fn reassemble_cookie_chunks_reports_gap() {
		let chunks =
			std::collections::HashMap::from_iter([(0usize, "a".to_string()), (2usize, "c".to_string())]);
		let (assembled, has_gap) = OAuth2::reassemble_cookie_chunks(chunks);
		assert_eq!(assembled, "a");
		assert!(has_gap);
	}

	#[test]
	fn reassemble_cookie_chunks_without_gap() {
		let chunks =
			std::collections::HashMap::from_iter([(0usize, "a".to_string()), (1usize, "b".to_string())]);
		let (assembled, has_gap) = OAuth2::reassemble_cookie_chunks(chunks);
		assert_eq!(assembled, "ab");
		assert!(!has_gap);
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
			// Refresh exchange should be deduped across concurrent requests for the same session.
			.expect(1)
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
		let oauth2 = OAuth2::new(config).unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let session = SessionState {
			access_token: "access-expired".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() - Duration::from_secs(60),
			nonce: None,
			id_token: None,
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let cookie_name = oauth2.session_cookie_name();
		let cookie_header = format!("{cookie_name}={encoded}");

		let mut set = JoinSet::new();
		for _ in 0..8 {
			let oauth2 = oauth2.clone();
			let client = client.clone();
			let policy_client = policy_client.clone();
			let oidc = oidc.clone();
			let cookie_header = cookie_header.clone();
			set.spawn(async move {
				let mut req = Request::new(crate::http::Body::empty());
				*req.uri_mut() = "/private/data".parse().unwrap();
				req.headers_mut().insert(
					http::header::COOKIE,
					HeaderValue::from_str(&cookie_header).unwrap(),
				);

				let response = oauth2
					.apply(&client, &policy_client, oidc.tokens(), &mut req)
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
		let oauth2 = OAuth2::new(config).unwrap();
		let client = make_test_client();
		let policy_client = make_test_policy_client();
		let oidc = Arc::new(OidcClient::new());
		let session = SessionState {
			access_token: "access-expired".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
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
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let response = oauth2
			.apply(&client, &policy_client, oidc.tokens(), &mut req)
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
				.any(|v| v.starts_with(&format!("{DEFAULT_COOKIE_NAME}=")) && v.contains("Max-Age=0")),
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

	#[test]
	fn logout_clears_all_cookie_clear_slots() {
		let oauth2 = test_oauth2();
		let headers = http::HeaderMap::new();
		let policy = oauth2
			.handle_logout(&headers, None)
			.expect("logout should succeed");
		let headers = policy
			.response_headers
			.expect("logout response should include cookie clear headers");
		let set_cookie_values: Vec<String> = headers
			.get_all(http::header::SET_COOKIE)
			.iter()
			.filter_map(|v| v.to_str().ok().map(ToOwned::to_owned))
			.collect();

		assert!(
			set_cookie_values
				.iter()
				.any(|v| v.starts_with(&format!("{DEFAULT_COOKIE_NAME}="))),
			"logout must clear base session cookie"
		);
		assert!(
			set_cookie_values
				.iter()
				.any(|v| v.starts_with(&format!("{DEFAULT_COOKIE_NAME}.{}=", COOKIE_CLEAR_SLOTS))),
			"logout must clear highest configured chunk slot"
		);
	}

	#[test]
	fn logout_redirects_to_end_session_endpoint_with_id_token_hint() {
		let oauth2 = test_oauth2();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: Some("nonce".to_string()),
			id_token: Some("id-token-value".to_string()),
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let policy = oauth2
			.handle_logout(&headers, Some("https://issuer.example.com/logout?foo=bar"))
			.expect("logout should succeed");
		let response = policy
			.direct_response
			.expect("logout response should be present");
		assert_eq!(response.status(), StatusCode::FOUND);
		let location = response
			.headers()
			.get(http::header::LOCATION)
			.and_then(|v| v.to_str().ok())
			.expect("logout redirect location should be set");
		let parsed = Url::parse(location).expect("redirect location should be a valid URL");
		assert_eq!(parsed.scheme(), "https");
		assert_eq!(parsed.host_str(), Some("issuer.example.com"));
		assert_eq!(parsed.path(), "/logout");
		let query = parsed.query_pairs().into_owned().collect::<Vec<_>>();
		assert!(query.contains(&("foo".to_string(), "bar".to_string())));
		assert!(query.contains(&("client_id".to_string(), "client-id".to_string())));
		assert!(query.contains(&("id_token_hint".to_string(), "id-token-value".to_string())));
	}

	#[test]
	fn logout_redirect_includes_post_logout_redirect_uri_when_configured() {
		let mut config = test_config();
		config.post_logout_redirect_uri = Some("https://app.example.com/signed-out".to_string());
		let oauth2 = OAuth2::new(config).unwrap();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: Some("nonce".to_string()),
			id_token: Some("id-token-value".to_string()),
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let policy = oauth2
			.handle_logout(&headers, Some("https://issuer.example.com/logout"))
			.expect("logout should succeed");
		let response = policy
			.direct_response
			.expect("logout response should be present");
		let location = response
			.headers()
			.get(http::header::LOCATION)
			.and_then(|v| v.to_str().ok())
			.expect("logout redirect location should be set");
		let parsed = Url::parse(location).expect("redirect location should be a valid URL");
		let query = parsed.query_pairs().into_owned().collect::<Vec<_>>();
		assert!(query.contains(&(
			"post_logout_redirect_uri".to_string(),
			"https://app.example.com/signed-out".to_string()
		)));
	}

	#[test]
	fn logout_with_invalid_end_session_endpoint_falls_back_to_local_only() {
		let oauth2 = test_oauth2();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: Some("nonce".to_string()),
			id_token: Some("id-token-value".to_string()),
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let policy = oauth2
			.handle_logout(&headers, Some("::invalid-url::"))
			.expect("logout should succeed");
		let response = policy
			.direct_response
			.expect("logout response should be present");
		assert_eq!(response.status(), StatusCode::OK);
	}

	#[test]
	fn logout_with_non_https_end_session_endpoint_falls_back_to_local_only() {
		let oauth2 = test_oauth2();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: Some("nonce".to_string()),
			id_token: Some("id-token-value".to_string()),
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let policy = oauth2
			.handle_logout(&headers, Some("http://issuer.example.com/logout"))
			.expect("logout should succeed");
		let response = policy
			.direct_response
			.expect("logout response should be present");
		assert_eq!(response.status(), StatusCode::OK);
	}

	#[test]
	fn logout_with_userinfo_end_session_endpoint_falls_back_to_local_only() {
		let oauth2 = test_oauth2();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: Some("nonce".to_string()),
			id_token: Some("id-token-value".to_string()),
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let policy = oauth2
			.handle_logout(
				&headers,
				Some("https://user:pass@issuer.example.com/logout"),
			)
			.expect("logout should succeed");
		let response = policy
			.direct_response
			.expect("logout response should be present");
		assert_eq!(response.status(), StatusCode::OK);
	}

	#[test]
	fn logout_redirect_replaces_reserved_query_params() {
		let mut config = test_config();
		config.post_logout_redirect_uri = Some("https://app.example.com/signed-out".to_string());
		let oauth2 = OAuth2::new(config).unwrap();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
			nonce: Some("nonce".to_string()),
			id_token: Some("id-token-value".to_string()),
		};
		let encoded = oauth2.session_codec.encode_session(&session).unwrap();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&format!("{DEFAULT_COOKIE_NAME}={encoded}")).unwrap(),
		);

		let policy = oauth2
			.handle_logout(
				&headers,
				Some(
					"https://issuer.example.com/logout?foo=bar&client_id=old&id_token_hint=old&post_logout_redirect_uri=https://old.example.com",
				),
			)
			.expect("logout should succeed");
		let response = policy
			.direct_response
			.expect("logout response should be present");
		assert_eq!(response.status(), StatusCode::FOUND);
		let location = response
			.headers()
			.get(http::header::LOCATION)
			.and_then(|v| v.to_str().ok())
			.expect("logout redirect location should be set");
		let parsed = Url::parse(location).expect("redirect location should be a valid URL");
		let query = parsed.query_pairs().into_owned().collect::<Vec<_>>();
		assert_eq!(
			query.iter().filter(|(k, _)| k == "client_id").count(),
			1,
			"client_id should be replaced, not duplicated"
		);
		assert_eq!(
			query.iter().filter(|(k, _)| k == "id_token_hint").count(),
			1,
			"id_token_hint should be replaced, not duplicated"
		);
		assert_eq!(
			query
				.iter()
				.filter(|(k, _)| k == "post_logout_redirect_uri")
				.count(),
			1,
			"post_logout_redirect_uri should be replaced, not duplicated"
		);
		assert!(query.contains(&("foo".to_string(), "bar".to_string())));
		assert!(query.contains(&("client_id".to_string(), "client-id".to_string())));
		assert!(query.contains(&("id_token_hint".to_string(), "id-token-value".to_string())));
		assert!(query.contains(&(
			"post_logout_redirect_uri".to_string(),
			"https://app.example.com/signed-out".to_string()
		)));
	}

	#[test]
	fn session_cookie_max_chunk_index_ignores_out_of_range_chunks() {
		let oauth2 = test_oauth2();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_static("__Host-ag-session=base; __Host-ag-session.999=evil"),
		);
		assert_eq!(oauth2.session_cookie_max_chunk_index(&headers), 0);
	}

	#[test]
	fn session_cookie_max_chunk_index_reads_split_cookie_headers() {
		let oauth2 = test_oauth2();
		let mut headers = http::HeaderMap::new();
		headers.append(http::header::COOKIE, HeaderValue::from_static("other=1"));
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_static("__Host-ag-session.8=chunk"),
		);
		assert_eq!(oauth2.session_cookie_max_chunk_index(&headers), 8);
	}

	#[test]
	fn session_cookie_max_chunk_index_prefers_largest_duplicate_across_split_headers() {
		let oauth2 = test_oauth2();
		let mut headers = http::HeaderMap::new();
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_static("__Host-ag-session.2=chunk"),
		);
		headers.append(
			http::header::COOKIE,
			HeaderValue::from_static("__Host-ag-session.8=chunk"),
		);
		assert_eq!(oauth2.session_cookie_max_chunk_index(&headers), 8);
	}

	#[test]
	fn logout_clears_observed_chunk_slots() {
		let oauth2 = test_oauth2();
		let mut req_headers = http::HeaderMap::new();
		req_headers.insert(
			http::header::COOKIE,
			HeaderValue::from_static("__Host-ag-session=base; __Host-ag-session.8=chunk"),
		);
		let policy = oauth2
			.handle_logout(&req_headers, None)
			.expect("logout should succeed");
		let headers = policy
			.response_headers
			.expect("logout response should include cookie clear headers");
		let set_cookie_values: Vec<String> = headers
			.get_all(http::header::SET_COOKIE)
			.iter()
			.filter_map(|v| v.to_str().ok().map(ToOwned::to_owned))
			.collect();
		assert!(
			set_cookie_values
				.iter()
				.any(|v| v.starts_with(&format!("{DEFAULT_COOKIE_NAME}.8="))),
			"logout must clear observed chunk slots from request cookies"
		);
	}
}
