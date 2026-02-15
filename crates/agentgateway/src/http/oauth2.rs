use std::borrow::Cow;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::constant_time;
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cookie::{Cookie, SameSite};
use http::{HeaderValue, StatusCode};
use ipnet::IpNet;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope, TokenUrl};
use rand::Rng;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use url::Url;

use crate::cel::SourceContext;
use crate::client::Client;
use crate::http::jwt::{Claims, Jwt};
use crate::http::oidc::{Error as OidcError, OidcMetadata, OidcProvider};
use crate::http::{PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::types::agent::OAuth2Policy;

const DEFAULT_COOKIE_NAME: &str = "__Host-ag-session";
const DEFAULT_HANDSHAKE_COOKIE_NAME: &str = "__Host-ag-nonce";
const STATE_TTL: Duration = Duration::from_secs(300); // 5 minutes for login handshake
const MAX_COOKIE_SIZE: usize = 3800; // Leave room for browser limits and cookie attributes
const COOKIE_CLEAR_SLOTS: usize = 5;
// Bound parsed chunk indices from request cookies to avoid unbounded cleanup loops on crafted inputs.
const MAX_SESSION_COOKIE_CHUNK_INDEX: usize = 63;
// Keep refresh-capable sessions alive long enough to perform token refreshes.
const DEFAULT_REFRESHABLE_COOKIE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_REFRESHABLE_COOKIE_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const DEFAULT_CALLBACK_PATH: &str = "/_gateway/callback";
const DEFAULT_SCOPE_PARAM: &str = "openid profile email";
const SESSION_COOKIE_AAD: &[u8] = b"agentgateway_session_cookie_v1";
const HANDSHAKE_STATE_AAD: &[u8] = b"agentgateway_handshake_state_v1";

#[derive(Debug, thiserror::Error)]
pub enum OAuth2Error {
	#[error("oidc discovery failed: {0}")]
	OidcDiscovery(#[from] OidcError),
	#[error("oauth2 handshake failed: {0}")]
	Handshake(String),
	#[error("invalid token: {0}")]
	InvalidToken(String),
}

/// OAuth2Filter implements a modernized, stateless, and secure OAuth2/OIDC filter.
#[derive(Debug, Clone)]
pub struct OAuth2Filter {
	config: OAuth2Policy,
	oidc_provider: Arc<OidcProvider>,
	session_codec: Arc<SessionCodec>,
	handshake_codec: Arc<SessionCodec>,
	trusted_proxy_nets: Arc<[IpNet]>,
}

use aws_lc_rs::hkdf;

impl OAuth2Filter {
	fn trusted_proxy_nets(config: &OAuth2Policy) -> anyhow::Result<Arc<[IpNet]>> {
		config
			.trusted_proxy_cidrs
			.iter()
			.map(|cidr| {
				cidr
					.parse::<IpNet>()
					.map_err(|e| anyhow::anyhow!("invalid trusted_proxy_cidrs entry `{cidr}`: {e}"))
			})
			.collect::<Result<Vec<_>, _>>()
			.map(Into::into)
	}

	pub fn validate_policy(config: &OAuth2Policy) -> anyhow::Result<()> {
		let auto_detect_redirect_uri = config.auto_detect_redirect_uri.unwrap_or(false);
		if config.redirect_uri.is_none() && !auto_detect_redirect_uri {
			anyhow::bail!("oauth2 policy requires redirect_uri or auto_detect_redirect_uri=true");
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
		let _ = Self::trusted_proxy_nets(config)?;
		Ok(())
	}

	pub fn new(config: OAuth2Policy, oidc_provider: Arc<OidcProvider>) -> anyhow::Result<Self> {
		Self::validate_policy(&config)?;
		let trusted_proxy_nets = Self::trusted_proxy_nets(&config)?;
		// Derive distinct keys for session and handshake encryption using HKDF to ensure key separation.
		let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]);
		let prk = salt.extract(config.client_secret.expose_secret().as_bytes());
		let cookie_scope = config.cookie_name.as_deref().unwrap_or(DEFAULT_COOKIE_NAME);
		let session_info = format!(
			"agentgateway_session_v1|issuer={}|client_id={}|cookie={cookie_scope}",
			config.issuer, config.client_id
		);
		let handshake_info = format!(
			"agentgateway_handshake_v1|issuer={}|client_id={}|cookie={cookie_scope}",
			config.issuer, config.client_id
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
			oidc_provider,
			session_codec,
			handshake_codec,
			trusted_proxy_nets,
		})
	}

	pub(crate) fn matches_policy(&self, policy: &OAuth2Policy) -> bool {
		let secrets_match = constant_time::verify_slices_are_equal(
			self.config.client_secret.expose_secret().as_bytes(),
			policy.client_secret.expose_secret().as_bytes(),
		)
		.is_ok();
		self.config.issuer == policy.issuer
			&& self.config.client_id == policy.client_id
			&& secrets_match
			&& self.config.redirect_uri == policy.redirect_uri
			&& self.config.auto_detect_redirect_uri == policy.auto_detect_redirect_uri
			&& self.config.scopes == policy.scopes
			&& self.config.cookie_name == policy.cookie_name
			&& self.config.refreshable_cookie_max_age_seconds == policy.refreshable_cookie_max_age_seconds
			&& self.config.pass_access_token == policy.pass_access_token
			&& self.config.sign_out_path == policy.sign_out_path
			&& self.config.pass_through_matchers == policy.pass_through_matchers
			&& self.config.deny_redirect_matchers == policy.deny_redirect_matchers
			&& self.config.trusted_proxy_cidrs == policy.trusted_proxy_cidrs
	}

	#[tracing::instrument(
		skip_all,
		fields(issuer = %self.config.issuer, client_id = %self.config.client_id)
	)]
	pub async fn apply(
		&self,
		client: &Client,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		debug!(path = req.uri().path(), "applying oauth2 filter");
		// 0. Check Bypass
		if self.should_bypass(req) {
			return Ok(PolicyResponse::default());
		}

		// 1. Check Logout
		if let Some(path) = &self.config.sign_out_path
			&& req.uri().path() == path
		{
			return self.handle_logout(req.headers());
		}

		let redirect_uri = self.resolve_redirect_uri(req)?;

		// 2. Check for Existing Session
		if let Some(mut session) = self.get_session(req.headers()) {
			let previous_max_chunk_index = self.session_cookie_max_chunk_index(req.headers());
			let mut updated_cookie_headers = None;

			// Check expiry & Refresh if needed
			if session.is_expired() {
				debug!("Session expired, attempting refresh");
				if session.refresh_token.is_some() {
					let (metadata, jwt_validator) = self.load_oidc_info(client).await?;
					match self
						.refresh_session(client, &mut session, &metadata, &jwt_validator)
						.await
					{
						Ok(true) => match self.session_codec.encode_session(&session) {
							Ok(encoded) => {
								updated_cookie_headers = Some(self.set_session_cookies(
									encoded,
									Some(previous_max_chunk_index),
									session.cookie_max_age(self.refreshable_cookie_max_age()),
								));
							},
							Err(err) => {
								debug!("failed to encode refreshed session: {err}");
							},
						},
						_ => {
							debug!("Refresh failed, requiring re-auth");
						},
					}
				} else {
					debug!("Session expired with no refresh token, requiring re-auth");
				}
			}

			if !session.is_expired() {
				// Handle Accidental Callback Revisit (Race Redirect):
				// If validly logged in but on the callback path, redirect away.
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
						.map_err(|e| ProxyError::Generic(format!("failed to build redirect response: {e}")))?;
					return Ok(PolicyResponse {
						direct_response: Some(resp),
						response_headers: updated_cookie_headers,
					});
				}

				// Session valid
				self.inject_auth(req, &session.access_token, session.claims.take());
				return Ok(PolicyResponse {
					direct_response: None,
					response_headers: updated_cookie_headers,
				});
			}
		}

		// 3. Check for Callback (Code Exchange)
		if req.uri().path() == redirect_uri.path() {
			let (metadata, jwt_validator) = self.load_oidc_info(client).await?;
			return self
				.handle_callback(
					client,
					req.headers(),
					req.uri(),
					&metadata,
					&jwt_validator,
					&redirect_uri,
				)
				.await;
		}

		// 4. No Valid Session -> Trigger Auth
		let metadata = self.load_oidc_metadata(client).await?;
		self
			.trigger_auth(req.headers(), req.uri(), &metadata, &redirect_uri)
			.await
	}

	async fn load_oidc_metadata(&self, client: &Client) -> Result<Arc<OidcMetadata>, ProxyError> {
		self
			.oidc_provider
			.get_metadata(client, &self.config.issuer)
			.await
			.map_err(OAuth2Error::from)
			.map_err(ProxyError::from)
	}

	async fn load_oidc_info(&self, client: &Client) -> Result<(Arc<OidcMetadata>, Jwt), ProxyError> {
		self
			.oidc_provider
			.get_info(
				client,
				&self.config.issuer,
				Some(vec![self.config.client_id.clone()]),
			)
			.await
			.map_err(OAuth2Error::from)
			.map_err(ProxyError::from)
	}

	fn refreshable_cookie_max_age(&self) -> Duration {
		self
			.config
			.refreshable_cookie_max_age_seconds
			.map(Duration::from_secs)
			.unwrap_or(DEFAULT_REFRESHABLE_COOKIE_MAX_AGE)
	}

	fn extract_original_url(&self, uri: &http::Uri) -> Option<String> {
		let state_str = Self::query_param(uri, "state")?;
		let state = self
			.handshake_codec
			.decrypt_handshake_state(&state_str)
			.ok()?;
		Some(state.original_url)
	}

	fn should_bypass(&self, req: &Request) -> bool {
		let path = req.uri().path();
		for prefix in &self.config.pass_through_matchers {
			if Self::matches_path_prefix(path, prefix) {
				return true;
			}
		}
		false
	}

	fn handle_logout(&self, headers: &http::HeaderMap) -> Result<PolicyResponse, ProxyError> {
		let cookie_name = self
			.config
			.cookie_name
			.as_deref()
			.unwrap_or(DEFAULT_COOKIE_NAME);
		let observed_max_chunk = self.session_cookie_max_chunk_index(headers);
		let clear_end = std::cmp::max(COOKIE_CLEAR_SLOTS, observed_max_chunk.saturating_add(1));

		let mut headers = crate::http::HeaderMap::new();
		for i in 0..=clear_end {
			let name = if i == 0 {
				cookie_name.to_string()
			} else {
				format!("{}.{}", cookie_name, i)
			};
			let cookie = Cookie::build((name, ""))
				.path("/")
				.secure(true)
				.http_only(true)
				.max_age(cookie::time::Duration::seconds(0))
				.build();
			if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
				headers.append(http::header::SET_COOKIE, value);
			}
		}

		let resp = Response::builder()
			.status(StatusCode::OK)
			.body(Default::default())
			.map_err(|e| ProxyError::Generic(format!("failed to build logout response: {e}")))?;

		Ok(PolicyResponse {
			direct_response: Some(resp),
			response_headers: Some(headers),
		})
	}

	fn resolve_redirect_uri(&self, req: &Request) -> Result<Url, ProxyError> {
		if let Some(uri) = &self.config.redirect_uri {
			return Url::parse(uri)
				.map_err(|e| ProxyError::Generic(format!("invalid redirect_uri config: {e}")));
		}
		if !self.config.auto_detect_redirect_uri.unwrap_or(false) {
			return Err(ProxyError::Generic(
				"redirect uri inference disabled: set oauth2.redirect_uri or enable oauth2.auto_detect_redirect_uri"
					.into(),
			));
		}
		// Auto-detect callback URL from request authority/forwarded headers.
		// In multi-proxy host/scheme rewrite topologies this may not match the public callback URL;
		// prefer explicit `redirect_uri` in production for deterministic behavior.
		let host = self.resolve_external_host(req)?;
		let scheme = self.resolve_external_scheme(req);
		let url_str = format!("{scheme}://{host}{DEFAULT_CALLBACK_PATH}");
		Url::parse(&url_str).map_err(|e| {
			ProxyError::Generic(format!(
				"failed to construct redirect uri from {url_str}: {e}"
			))
		})
	}

	async fn handle_callback(
		&self,
		client: &Client,
		headers: &http::HeaderMap,
		uri: &http::Uri,
		metadata: &OidcMetadata,
		jwt_validator: &Jwt,
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
			ProxyError::OAuth2AuthenticationFailure(OAuth2Error::Handshake("missing code".into()))
		})?;
		let state_str = state_str.ok_or_else(|| {
			ProxyError::OAuth2AuthenticationFailure(OAuth2Error::Handshake("missing state".into()))
		})?;

		// Decrypt Handshake State
		let state = self
			.handshake_codec
			.decrypt_handshake_state(&state_str)
			.map_err(|e| {
				ProxyError::OAuth2AuthenticationFailure(OAuth2Error::Handshake(format!(
					"invalid state: {e}"
				)))
			})?;

		// Verify Expiry
		if SystemTime::now() > state.expires_at {
			return Err(ProxyError::OAuth2AuthenticationFailure(
				OAuth2Error::Handshake("login state expired".into()),
			));
		}

		// Verify Handshake Isolation (Double Submit Cookie)
		let handshake_cookie_name = format!("{}.{}", DEFAULT_HANDSHAKE_COOKIE_NAME, state.handshake_id);
		let cookies = headers
			.get(http::header::COOKIE)
			.and_then(|v| v.to_str().ok())
			.unwrap_or_default();
		let mut found_binding = false;
		for cookie in Cookie::split_parse(cookies) {
			let cookie = match cookie {
				Ok(c) => c,
				Err(e) => {
					debug!("ignoring malformed cookie during callback: {e}");
					continue;
				},
			};
			if cookie.name() == handshake_cookie_name {
				found_binding = true;
				break;
			}
		}

		if !found_binding {
			return Err(ProxyError::OAuth2AuthenticationFailure(
				OAuth2Error::Handshake(
					"handshake browser binding failed (missing or mismatched attempt ID)".into(),
				),
			));
		}

		// Exchange Code (Manual)
		let token_resp = self
			.oidc_provider
			.exchange_code(
				client,
				metadata,
				&code,
				&self.config.client_id,
				self.config.client_secret.expose_secret(),
				redirect_uri.as_str(),
				state.pkce_verifier.as_deref(),
			)
			.await
			.map_err(|e| {
				ProxyError::OAuth2AuthenticationFailure(OAuth2Error::Handshake(e.to_string()))
			})?;

		// Verify ID Token using existing JWT module
		let claims = if let Some(id_token) = &token_resp.id_token {
			let claims = jwt_validator.validate_claims(id_token).map_err(|e| {
				ProxyError::OAuth2AuthenticationFailure(OAuth2Error::InvalidToken(e.to_string()))
			})?;

			// Additional OIDC specific verification: check nonce
			let token_nonce = claims
				.inner
				.get("nonce")
				.and_then(|v| v.as_str())
				.ok_or_else(|| {
					ProxyError::OAuth2AuthenticationFailure(OAuth2Error::InvalidToken(
						"id_token missing nonce".into(),
					))
				})?;

			if token_nonce != state.nonce {
				return Err(ProxyError::OAuth2AuthenticationFailure(
					OAuth2Error::InvalidToken("id_token nonce mismatch".into()),
				));
			}

			Some(claims)
		} else {
			None
		};

		// Create Session
		let expires_in = token_resp.expires_in.unwrap_or(3600);
		let session = SessionState {
			access_token: token_resp.access_token,
			refresh_token: token_resp.refresh_token,
			claims,
			expires_at: SystemTime::now() + Duration::from_secs(expires_in),
		};

		// Set Cookies & Redirect
		let cookie_value = self
			.session_codec
			.encode_session(&session)
			.map_err(|e| ProxyError::Generic(format!("failed to encode session: {e}")))?;

		let previous_max_chunk_index = self.session_cookie_max_chunk_index(headers);
		let mut response_headers = self.set_session_cookies(
			cookie_value,
			Some(previous_max_chunk_index),
			session.cookie_max_age(self.refreshable_cookie_max_age()),
		);

		// Cleanup: Clear the specific namespaced handshake cookie
		let clear_handshake = Cookie::build((handshake_cookie_name, ""))
			.path("/")
			.secure(true)
			.http_only(true)
			.max_age(cookie::time::Duration::seconds(0))
			.build();
		response_headers.append(
			http::header::SET_COOKIE,
			HeaderValue::from_str(&clear_handshake.to_string())
				.map_err(|e| ProxyError::Generic(format!("invalid handshake clear cookie header: {e}")))?,
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
			.map_err(|e| ProxyError::Generic(format!("failed to build callback redirect: {e}")))?;

		Ok(PolicyResponse {
			direct_response: Some(resp),
			response_headers: Some(response_headers),
		})
	}

	fn get_session(&self, headers: &http::HeaderMap) -> Option<SessionState> {
		let cookie_name = self
			.config
			.cookie_name
			.as_deref()
			.unwrap_or(DEFAULT_COOKIE_NAME);
		let cookies_header = headers.get(http::header::COOKIE)?.to_str().ok()?;

		let mut chunks = std::collections::HashMap::with_capacity(4);
		for cookie in Cookie::split_parse(cookies_header) {
			let cookie = match cookie {
				Ok(c) => c,
				Err(e) => {
					debug!("ignoring malformed cookie: {e}");
					continue;
				},
			};
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
		}

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
		client: &Client,
		session: &mut SessionState,
		metadata: &OidcMetadata,
		jwt_validator: &Jwt,
	) -> Result<bool, OidcError> {
		let Some(rt) = &session.refresh_token else {
			return Ok(false);
		};

		let token_resp = self
			.oidc_provider
			.refresh_token(
				client,
				metadata,
				rt,
				&self.config.client_id,
				self.config.client_secret.expose_secret(),
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
		jwt_validator: &Jwt,
	) -> Result<(), crate::http::jwt::TokenError> {
		match id_token {
			Some(id_token) => {
				let claims = jwt_validator.validate_claims(id_token)?;
				session.claims = Some(claims);
			},
			None => {
				// Many providers omit id_token on refresh; preserve existing claims for CEL continuity.
			},
		}
		Ok(())
	}

	fn inject_auth(&self, req: &mut Request, access_token: &str, claims: Option<Claims>) {
		if self.config.pass_access_token.unwrap_or(false) {
			let val = format!("Bearer {}", access_token);
			if let Ok(hv) = HeaderValue::from_str(&val) {
				req.headers_mut().insert(http::header::AUTHORIZATION, hv);
			}
		}

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
	) -> crate::http::HeaderMap {
		let cookie_name = self
			.config
			.cookie_name
			.as_deref()
			.unwrap_or(DEFAULT_COOKIE_NAME);
		let mut headers = crate::http::HeaderMap::new();

		let mut i = 0;
		if value.len() <= MAX_COOKIE_SIZE {
			let cookie = self.build_session_cookie(cookie_name.to_string(), value, cookie_max_age);
			if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
				headers.insert(http::header::SET_COOKIE, value);
			}
			i = 1;
		} else {
			// Chunking
			let mut remaining = &value[..];
			while !remaining.is_empty() {
				let chunk_size = std::cmp::min(remaining.len(), MAX_COOKIE_SIZE);
				let chunk = &remaining[..chunk_size];
				remaining = &remaining[chunk_size..];

				let name = if i == 0 {
					cookie_name.to_string()
				} else {
					format!("{}.{}", cookie_name, i)
				};
				let cookie = self.build_session_cookie(name, chunk.to_string(), cookie_max_age);
				if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
					headers.append(http::header::SET_COOKIE, value);
				}
				i += 1;
			}
		}

		// Cleanup potential stale chunks from previous sessions.
		let observed_max_chunk = previous_max_chunk_index
			.map(|idx| idx.min(MAX_SESSION_COOKIE_CHUNK_INDEX))
			.unwrap_or(0);
		let clear_end = std::cmp::max(i + COOKIE_CLEAR_SLOTS, observed_max_chunk.saturating_add(1));
		for j in i..clear_end {
			let name = format!("{}.{}", cookie_name, j);
			let cookie = Cookie::build((name, ""))
				.path("/")
				.secure(true)
				.http_only(true)
				.max_age(cookie::time::Duration::seconds(0))
				.build();
			if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
				headers.append(http::header::SET_COOKIE, value);
			}
		}

		headers
	}

	fn session_cookie_max_chunk_index(&self, headers: &http::HeaderMap) -> usize {
		let cookie_name = self
			.config
			.cookie_name
			.as_deref()
			.unwrap_or(DEFAULT_COOKIE_NAME);
		let Some(cookies_header) = headers
			.get(http::header::COOKIE)
			.and_then(|v| v.to_str().ok())
		else {
			return 0;
		};

		let mut max_idx = 0usize;
		for cookie in Cookie::split_parse(cookies_header) {
			let cookie = match cookie {
				Ok(c) => c,
				Err(_) => continue,
			};
			if cookie.name() == cookie_name {
				continue;
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
		}
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
			.secure(true)
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

		if self.should_return_unauthorized(headers, uri.path()) {
			// API Client -> 401
			let resp = Response::builder()
				.status(StatusCode::UNAUTHORIZED)
				.header(
					http::header::WWW_AUTHENTICATE,
					format!(
						"Bearer realm=\"{}\", scope=\"{}\"",
						self.config.issuer,
						scope_string.as_ref()
					),
				)
				.body(Default::default())
				.map_err(|e| ProxyError::Generic(format!("failed to build unauthorized response: {e}")))?;
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
			.map_err(|e| ProxyError::Generic(format!("failed to encrypt state: {e}")))?;

		let auth_url = BasicClient::new(ClientId::new(self.config.client_id.clone()))
			.set_auth_uri(
				AuthUrl::new(metadata.authorization_endpoint.clone())
					.map_err(|e| ProxyError::Generic(format!("invalid auth endpoint: {e}")))?,
			)
			.set_token_uri(
				TokenUrl::new(metadata.token_endpoint.clone())
					.map_err(|e| ProxyError::Generic(format!("invalid token endpoint: {e}")))?,
			)
			.set_redirect_uri(
				RedirectUrl::new(redirect_uri.as_str().to_string()).map_err(|e| {
					ProxyError::Generic(format!("invalid redirect uri for oauth2 client: {e}"))
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
		let handshake_cookie_name = format!("{}.{}", DEFAULT_HANDSHAKE_COOKIE_NAME, handshake_id);
		let handshake_cookie = Cookie::build((handshake_cookie_name, "1"))
			.path("/")
			.secure(true)
			.http_only(true)
			.same_site(SameSite::Lax)
			.max_age(cookie::time::Duration::seconds(STATE_TTL.as_secs() as i64))
			.build();

		let mut response_headers = crate::http::HeaderMap::new();
		response_headers.insert(
			http::header::SET_COOKIE,
			HeaderValue::from_str(&handshake_cookie.to_string())
				.map_err(|e| ProxyError::Generic(format!("invalid handshake cookie header: {e}")))?,
		);

		let resp = Response::builder()
			.status(StatusCode::FOUND)
			.header(http::header::LOCATION, auth_url.as_str())
			.body(Default::default())
			.map_err(|e| ProxyError::Generic(format!("failed to build auth redirect response: {e}")))?;

		Ok(PolicyResponse {
			direct_response: Some(resp),
			response_headers: Some(response_headers),
		})
	}

	fn should_return_unauthorized(&self, headers: &http::HeaderMap, path: &str) -> bool {
		if self
			.config
			.deny_redirect_matchers
			.iter()
			.any(|matcher| Self::matches_path_prefix(path, matcher))
		{
			return true;
		}
		let accept = headers
			.get(http::header::ACCEPT)
			.and_then(|v| v.to_str().ok())
			.unwrap_or("");
		!accept.contains("text/html")
	}

	fn random_token() -> String {
		let mut bytes = [0u8; 16];
		let mut rng = rand::rng();
		rng.fill_bytes(&mut bytes);
		URL_SAFE_NO_PAD.encode(bytes)
	}

	fn forwarded_header_first_value(
		headers: &http::HeaderMap,
		name: http::header::HeaderName,
	) -> Option<String> {
		headers
			.get(name)
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.split(',').next())
			.map(str::trim)
			.filter(|v| !v.is_empty())
			.map(ToOwned::to_owned)
	}

	fn is_trusted_proxy_source(&self, req: &Request) -> bool {
		if self.trusted_proxy_nets.is_empty() {
			return false;
		}
		let Some(source) = req.extensions().get::<SourceContext>() else {
			return false;
		};
		self
			.trusted_proxy_nets
			.iter()
			.any(|cidr| cidr.contains(&source.address))
	}

	fn resolve_external_host(&self, req: &Request) -> Result<String, ProxyError> {
		let trusted_proxy = self.is_trusted_proxy_source(req);
		let headers = req.headers();
		let host = if trusted_proxy {
			Self::forwarded_header_first_value(
				headers,
				http::header::HeaderName::from_static("x-forwarded-host"),
			)
			.or_else(|| Self::forwarded_header_first_value(headers, http::header::HOST))
		} else {
			Self::forwarded_header_first_value(headers, http::header::HOST)
		}
		.ok_or_else(|| {
			ProxyError::Generic("unable to resolve redirect host from request headers".into())
		})?;

		http::uri::Authority::from_str(&host)
			.map(|_| host)
			.map_err(|_| ProxyError::Generic("invalid host header for redirect uri".into()))
	}

	fn resolve_external_scheme(&self, req: &Request) -> &'static str {
		if self.is_trusted_proxy_source(req)
			&& let Some(proto) = Self::forwarded_header_first_value(
				req.headers(),
				http::header::HeaderName::from_static("x-forwarded-proto"),
			) {
			if proto.eq_ignore_ascii_case("http") {
				return "http";
			}
			if proto.eq_ignore_ascii_case("https") {
				return "https";
			}
		}
		match req.uri().scheme_str() {
			Some("http") => "http",
			_ => "https",
		}
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

	fn matches_path_prefix(path: &str, matcher: &str) -> bool {
		if path == matcher {
			return true;
		}
		let matcher = matcher.trim_end_matches('/');
		if matcher == "/" {
			return path.starts_with('/');
		}
		if matcher.is_empty() {
			return false;
		}
		let Some(rest) = path.trim_end_matches('/').strip_prefix(matcher) else {
			return false;
		};
		rest.is_empty() || rest.starts_with('/')
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

		// 1. Encrypt plaintext into a Vec.
		// Pre-allocate space for plaintext + tag (16 bytes)
		let mut in_out = Vec::with_capacity(plaintext.len() + AES_256_GCM.tag_len());
		in_out.extend_from_slice(plaintext);

		self
			.key
			.seal_in_place_append_tag(nonce, Aad::from(self.aad), &mut in_out)
			.map_err(|_| anyhow::anyhow!("encryption failed"))?;

		// 2. Combine Nonce + (Ciphertext + Tag) into final buffer
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

	use secrecy::SecretString;
	use serde_json::json;
	use serde_json::{Map, Value};
	use tokio::task::JoinSet;
	use wiremock::matchers::{body_string_contains, method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;

	fn test_config() -> OAuth2Policy {
		OAuth2Policy {
			issuer: "https://issuer.example.com".to_string(),
			client_id: "client-id".to_string(),
			client_secret: SecretString::new("secret".into()),
			redirect_uri: Some("https://fixed.example.com/callback".to_string()),
			auto_detect_redirect_uri: None,
			scopes: vec![],
			cookie_name: None,
			refreshable_cookie_max_age_seconds: None,
			pass_access_token: None,
			sign_out_path: None,
			pass_through_matchers: vec![],
			deny_redirect_matchers: vec![],
			trusted_proxy_cidrs: vec![],
		}
	}

	fn make_test_client() -> crate::client::Client {
		let cfg = crate::client::Config {
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

	fn test_filter() -> OAuth2Filter {
		OAuth2Filter::new(test_config(), Arc::new(OidcProvider::new())).unwrap()
	}

	fn test_auto_detect_filter() -> OAuth2Filter {
		let mut config = test_config();
		config.redirect_uri = None;
		config.auto_detect_redirect_uri = Some(true);
		OAuth2Filter::new(config, Arc::new(OidcProvider::new())).unwrap()
	}

	#[test]
	fn original_target_only_keeps_path_and_query() {
		let uri: http::Uri = "https://evil.example.com/path?q=1".parse().unwrap();
		assert_eq!(OAuth2Filter::original_target_from_uri(&uri), "/path?q=1");
	}

	#[test]
	fn safe_redirect_target_allows_local_path_only() {
		assert!(OAuth2Filter::is_safe_redirect_target("/ok"));
		assert!(OAuth2Filter::is_safe_redirect_target("/ok?q=1"));
		assert!(!OAuth2Filter::is_safe_redirect_target("//evil.example.com"));
		assert!(!OAuth2Filter::is_safe_redirect_target(
			"https://evil.example.com"
		));
		assert!(!OAuth2Filter::is_safe_redirect_target(
			"/\\evil.example.com"
		));
		assert!(!OAuth2Filter::is_safe_redirect_target("/ok\nbad"));
	}

	#[test]
	fn resolve_redirect_uri_prefers_config() {
		let filter = test_filter();

		let mut req = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "http://ignored/".parse().unwrap();
		req.headers_mut().insert(
			http::header::HOST,
			HeaderValue::from_static("ignored.example.com"),
		);
		let resolved = filter.resolve_redirect_uri(&req).unwrap();
		assert_eq!(resolved.as_str(), "https://fixed.example.com/callback");
	}

	#[test]
	fn oauth2_new_requires_redirect_or_auto_detect() {
		let mut config = test_config();
		config.redirect_uri = None;
		config.auto_detect_redirect_uri = Some(false);
		let err = OAuth2Filter::new(config, Arc::new(OidcProvider::new())).unwrap_err();
		assert!(
			err
				.to_string()
				.contains("requires redirect_uri or auto_detect_redirect_uri=true")
		);
	}

	#[test]
	fn oauth2_new_rejects_zero_refreshable_cookie_max_age() {
		let mut config = test_config();
		config.refreshable_cookie_max_age_seconds = Some(0);
		let err = OAuth2Filter::new(config, Arc::new(OidcProvider::new())).unwrap_err();
		assert!(
			err
				.to_string()
				.contains("refreshable_cookie_max_age_seconds must be > 0")
		);
	}

	#[test]
	fn oauth2_new_rejects_excessive_refreshable_cookie_max_age() {
		let mut config = test_config();
		config.refreshable_cookie_max_age_seconds = Some(MAX_REFRESHABLE_COOKIE_MAX_AGE.as_secs() + 1);
		let err = OAuth2Filter::new(config, Arc::new(OidcProvider::new())).unwrap_err();
		assert!(
			err
				.to_string()
				.contains("refreshable_cookie_max_age_seconds must be <=")
		);
	}

	#[test]
	fn refreshable_cookie_max_age_uses_policy_override() {
		let mut filter = test_filter();
		filter.config.refreshable_cookie_max_age_seconds = Some(1800);
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: Some("refresh".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(30),
		};
		assert_eq!(
			filter.refreshable_cookie_max_age(),
			Duration::from_secs(1800)
		);
		assert_eq!(
			session.cookie_max_age(filter.refreshable_cookie_max_age()),
			cookie::time::Duration::seconds(1800),
		);
	}

	#[test]
	fn refreshable_cookie_max_age_accepts_upper_bound() {
		let mut filter = test_filter();
		filter.config.refreshable_cookie_max_age_seconds =
			Some(MAX_REFRESHABLE_COOKIE_MAX_AGE.as_secs());
		assert_eq!(
			filter.refreshable_cookie_max_age(),
			MAX_REFRESHABLE_COOKIE_MAX_AGE
		);
	}

	#[test]
	fn resolve_redirect_uri_uses_forwarded_host_and_proto_for_trusted_proxy() {
		let mut filter = test_auto_detect_filter();
		filter.config.trusted_proxy_cidrs = vec!["10.0.0.0/8".to_string()];
		filter.trusted_proxy_nets = vec!["10.0.0.0/8".parse().unwrap()].into();
		let mut req = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "http://internal/".parse().unwrap();
		req.extensions_mut().insert(SourceContext {
			address: "10.1.2.3".parse().unwrap(),
			port: 12345,
			tls: None,
		});
		req.headers_mut().insert(
			http::header::HeaderName::from_static("x-forwarded-host"),
			HeaderValue::from_static("public.example.com"),
		);
		req.headers_mut().insert(
			http::header::HeaderName::from_static("x-forwarded-proto"),
			HeaderValue::from_static("https"),
		);
		let resolved = filter.resolve_redirect_uri(&req).unwrap();
		assert_eq!(
			resolved.as_str(),
			"https://public.example.com/_gateway/callback"
		);
	}

	#[test]
	fn resolve_redirect_uri_ignores_forwarded_host_for_untrusted_source() {
		let filter = test_auto_detect_filter();
		let mut req = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "http://internal/".parse().unwrap();
		req.headers_mut().insert(
			http::header::HOST,
			HeaderValue::from_static("actual.example.com"),
		);
		req.headers_mut().insert(
			http::header::HeaderName::from_static("x-forwarded-host"),
			HeaderValue::from_static("ignored-forwarded.example.com"),
		);
		let resolved = filter.resolve_redirect_uri(&req).unwrap();
		assert_eq!(
			resolved.as_str(),
			"http://actual.example.com/_gateway/callback"
		);
	}

	#[test]
	fn resolve_redirect_uri_rejects_invalid_host_header() {
		let filter = test_auto_detect_filter();
		let mut req = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "http://internal/".parse().unwrap();
		req
			.headers_mut()
			.insert(http::header::HOST, HeaderValue::from_static("bad host"));
		assert!(filter.resolve_redirect_uri(&req).is_err());
	}

	#[test]
	fn resolve_redirect_uri_rejects_when_auto_detect_disabled() {
		let mut filter = test_filter();
		filter.config.redirect_uri = None;
		filter.config.auto_detect_redirect_uri = Some(false);
		let req = Request::new(crate::http::Body::empty());
		assert!(filter.resolve_redirect_uri(&req).is_err());
	}

	#[test]
	fn pass_through_matchers_require_segment_boundary() {
		let mut filter = test_filter();
		filter.config.pass_through_matchers = vec!["/public".to_string()];

		let mut public_req = Request::new(crate::http::Body::empty());
		*public_req.uri_mut() = "/public/health".parse().unwrap();
		assert!(filter.should_bypass(&public_req));

		let mut false_positive_req = Request::new(crate::http::Body::empty());
		*false_positive_req.uri_mut() = "/publicity".parse().unwrap();
		assert!(!filter.should_bypass(&false_positive_req));
	}

	#[test]
	fn deny_redirect_matchers_force_unauthorized_for_html_clients() {
		let mut filter = test_filter();
		filter.config.deny_redirect_matchers = vec!["/api".to_string()];
		let mut headers = http::HeaderMap::new();
		headers.insert(http::header::ACCEPT, HeaderValue::from_static("text/html"));
		assert!(filter.should_return_unauthorized(&headers, "/api/v1/resource"));
		assert!(!filter.should_return_unauthorized(&headers, "/ui/home"));
	}

	#[test]
	fn update_session_claims_preserves_claims_without_id_token() {
		let filter = test_filter();
		let mut session = SessionState {
			access_token: "a".to_string(),
			refresh_token: None,
			claims: Some(Claims {
				inner: Map::from_iter([("sub".to_string(), Value::String("user".to_string()))]),
				jwt: SecretString::new("header.payload.sig".into()),
			}),
			expires_at: SystemTime::now(),
		};
		let jwt = Jwt::from_providers(vec![], crate::http::jwt::Mode::Strict);

		filter
			.update_session_claims(&mut session, None, &jwt)
			.unwrap();
		assert_eq!(
			session
				.claims
				.as_ref()
				.and_then(|claims| claims.inner.get("sub"))
				.and_then(|v| v.as_str()),
			Some("user")
		);
	}

	#[test]
	fn update_session_claims_invalid_id_token_keeps_existing_claims() {
		let filter = test_filter();
		let mut session = SessionState {
			access_token: "a".to_string(),
			refresh_token: None,
			claims: Some(Claims {
				inner: Map::from_iter([("sub".to_string(), Value::String("user".to_string()))]),
				jwt: SecretString::new("header.payload.sig".into()),
			}),
			expires_at: SystemTime::now(),
		};
		let jwt = Jwt::from_providers(vec![], crate::http::jwt::Mode::Strict);

		assert!(
			filter
				.update_session_claims(&mut session, Some("not-a-jwt"), &jwt)
				.is_err()
		);
		assert!(session.claims.is_some());
	}

	#[test]
	fn pass_access_token_defaults_to_false() {
		let mut config = test_config();
		config.pass_access_token = None;
		let filter = OAuth2Filter::new(config, Arc::new(OidcProvider::new())).unwrap();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
		};
		let mut req = Request::new(crate::http::Body::empty());
		filter.inject_auth(&mut req, &session.access_token, None);
		assert!(req.headers().get(http::header::AUTHORIZATION).is_none());
	}

	#[test]
	fn get_session_ignores_similar_cookie_prefixes() {
		let filter = test_filter();
		let session = SessionState {
			access_token: "token".to_string(),
			refresh_token: None,
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(60),
		};
		let encoded = filter.session_codec.encode_session(&session).unwrap();
		let cookies = format!("{DEFAULT_COOKIE_NAME}={encoded}; {DEFAULT_COOKIE_NAME}evil.1=malicious");

		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_str(&cookies).unwrap(),
		);

		let decoded = filter.get_session(&headers).expect("session should decode");
		assert_eq!(decoded.access_token, "token");
	}

	#[test]
	fn large_session_is_chunked_and_round_trips() {
		let filter = test_filter();
		let large_token = "a".repeat(MAX_COOKIE_SIZE * 2 + 512);
		let session = SessionState {
			access_token: large_token.clone(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
		};

		let encoded = filter.session_codec.encode_session(&session).unwrap();
		assert!(encoded.len() > MAX_COOKIE_SIZE);

		let response_headers = filter.set_session_cookies(
			encoded,
			None,
			session.cookie_max_age(filter.refreshable_cookie_max_age()),
		);
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

		let decoded = filter
			.get_session(&request_headers)
			.expect("chunked session should decode");
		assert_eq!(decoded.access_token, large_token);
		assert_eq!(decoded.refresh_token.as_deref(), Some("refresh-token"));
	}

	#[test]
	fn session_cookie_gap_returns_none() {
		let filter = test_filter();
		let session = SessionState {
			access_token: "b".repeat(MAX_COOKIE_SIZE * 2 + 512),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() + Duration::from_secs(3600),
		};
		let encoded = filter.session_codec.encode_session(&session).unwrap();
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
		assert!(filter.get_session(&headers).is_none());
	}

	#[test]
	fn reassemble_cookie_chunks_reports_gap() {
		let chunks =
			std::collections::HashMap::from_iter([(0usize, "a".to_string()), (2usize, "c".to_string())]);
		let (assembled, has_gap) = OAuth2Filter::reassemble_cookie_chunks(chunks);
		assert_eq!(assembled, "a");
		assert!(has_gap);
	}

	#[test]
	fn reassemble_cookie_chunks_without_gap() {
		let chunks =
			std::collections::HashMap::from_iter([(0usize, "a".to_string()), (1usize, "b".to_string())]);
		let (assembled, has_gap) = OAuth2Filter::reassemble_cookie_chunks(chunks);
		assert_eq!(assembled, "ab");
		assert!(!has_gap);
	}

	#[tokio::test]
	async fn concurrent_apply_requests_with_expired_session_refresh_successfully() {
		let server = MockServer::start().await;
		let issuer = server.uri();

		Mock::given(method("GET"))
			.and(path("/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"authorization_endpoint": format!("{issuer}/authorize"),
				"token_endpoint": format!("{issuer}/token"),
				"jwks_uri": format!("{issuer}/jwks"),
				"token_endpoint_auth_methods_supported": ["client_secret_post"]
			})))
			.expect(1)
			.mount(&server)
			.await;

		Mock::given(method("GET"))
			.and(path("/jwks"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
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
			})))
			.expect(1)
			.mount(&server)
			.await;

		Mock::given(method("POST"))
			.and(path("/token"))
			.and(body_string_contains("grant_type=refresh_token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"access_token": "access-refreshed",
				"token_type": "Bearer",
				"expires_in": 3600,
				"refresh_token": "refresh-next"
			})))
			.expect(8)
			.mount(&server)
			.await;

		let mut config = test_config();
		config.issuer = issuer;
		config.pass_access_token = Some(true);
		let filter = OAuth2Filter::new(config, Arc::new(OidcProvider::new())).unwrap();
		let client = make_test_client();
		let session = SessionState {
			access_token: "access-expired".to_string(),
			refresh_token: Some("refresh-token".to_string()),
			claims: None,
			expires_at: SystemTime::now() - Duration::from_secs(60),
		};
		let encoded = filter.session_codec.encode_session(&session).unwrap();
		let cookie_name = filter
			.config
			.cookie_name
			.as_deref()
			.unwrap_or(DEFAULT_COOKIE_NAME);
		let cookie_header = format!("{cookie_name}={encoded}");

		let mut set = JoinSet::new();
		for _ in 0..8 {
			let filter = filter.clone();
			let client = client.clone();
			let cookie_header = cookie_header.clone();
			set.spawn(async move {
				let mut req = Request::new(crate::http::Body::empty());
				*req.uri_mut() = "/private/data".parse().unwrap();
				req.headers_mut().insert(
					http::header::COOKIE,
					HeaderValue::from_str(&cookie_header).unwrap(),
				);

				let response = filter
					.apply(&client, &mut req)
					.await
					.expect("apply should succeed");
				let auth = req
					.headers()
					.get(http::header::AUTHORIZATION)
					.and_then(|value| value.to_str().ok())
					.map(ToOwned::to_owned);
				(response, auth)
			});
		}

		while let Some(joined) = set.join_next().await {
			let (response, auth) = joined.expect("task should join");
			assert!(response.direct_response.is_none());
			assert!(
				response.response_headers.is_some(),
				"refresh should return updated session cookies"
			);
			assert_eq!(auth.as_deref(), Some("Bearer access-refreshed"));
		}
	}

	#[test]
	fn logout_clears_all_cookie_clear_slots() {
		let filter = test_filter();
		let headers = http::HeaderMap::new();
		let policy = filter
			.handle_logout(&headers)
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
	fn session_cookie_max_chunk_index_ignores_out_of_range_chunks() {
		let filter = test_filter();
		let mut headers = http::HeaderMap::new();
		headers.insert(
			http::header::COOKIE,
			HeaderValue::from_static("__Host-ag-session=base; __Host-ag-session.999=evil"),
		);
		assert_eq!(filter.session_cookie_max_chunk_index(&headers), 0);
	}

	#[test]
	fn logout_clears_observed_chunk_slots() {
		let filter = test_filter();
		let mut req_headers = http::HeaderMap::new();
		req_headers.insert(
			http::header::COOKIE,
			HeaderValue::from_static("__Host-ag-session=base; __Host-ag-session.8=chunk"),
		);
		let policy = filter
			.handle_logout(&req_headers)
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
