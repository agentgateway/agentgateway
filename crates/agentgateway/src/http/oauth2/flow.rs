use std::borrow::Cow;
use std::time::{Duration, SystemTime};

use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cookie::Cookie;
use http::{HeaderValue, StatusCode};
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope, TokenUrl};
use rand::Rng;
use secrecy::ExposeSecret;
use tracing::warn;
use url::Url;

use super::cookies::{build_clear_cookie, encode_set_cookie_header, for_each_request_cookie};
use super::state::HandshakeState;
use super::{CallbackValidation, Error, OAuth2, OAuth2CallContext, RuntimeCookieSecret, STATE_TTL};
use crate::http::PolicyResponse;
use crate::proxy::ProxyError;

impl OAuth2 {
	pub(super) async fn handle_callback(
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

		let state = self.decrypt_handshake_state(&state_str).map_err(|e| {
			ProxyError::OAuth2AuthenticationFailure(Error::Handshake(format!("invalid state: {e}")))
		})?;
		if state.attachment_key != self.attachment_key {
			return Err(ProxyError::OAuth2AuthenticationFailure(Error::Handshake(
				"login state was issued for a different oauth2 policy".into(),
			)));
		}

		if SystemTime::now() > state.expires_at {
			return Err(ProxyError::OAuth2AuthenticationFailure(Error::Handshake(
				"login state expired".into(),
			)));
		}

		let handshake_cookie_name = format!(
			"{}.{}",
			self.handshake_cookie_base_name(),
			state.handshake_id
		);
		let mut found_binding = false;
		for_each_request_cookie(headers, |cookie| {
			if !found_binding && cookie.name() == handshake_cookie_name {
				found_binding = true;
			}
		});

		if !found_binding {
			return Err(ProxyError::OAuth2AuthenticationFailure(Error::Handshake(
				"handshake browser binding failed (missing or mismatched attempt ID)".into(),
			)));
		}

		let token_resp = runtime
			.oidc
			.exchange_code(
				self.oidc_context(runtime),
				crate::http::oidc::ExchangeCodeRequest {
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

		let validated_id_token = if let Some(id_token) = &token_resp.id_token {
			match callback.jwt_validator {
				Some(jwt_validator) => {
					let claims = jwt_validator.validate_claims(id_token).map_err(|e| {
						ProxyError::OAuth2AuthenticationFailure(Error::InvalidToken(e.to_string()))
					})?;

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

					Some(id_token.clone())
				},
				None => {
					warn!(
						"id_token returned but no JWKS validator is configured; ignoring callback id_token"
					);
					None
				},
			}
		} else {
			None
		};

		let expires_in = token_resp.expires_in.unwrap_or(3600);
		let session = super::state::SessionState {
			access_token: token_resp.access_token,
			refresh_token: token_resp.refresh_token,
			expires_at: SystemTime::now() + Duration::from_secs(expires_in),
			nonce: Some(state.nonce.clone()),
			id_token: validated_id_token,
		};

		let cookie_value = self
			.session_codec
			.encode_session(&session)
			.map_err(|e| ProxyError::from(Error::Internal(format!("failed to encode session: {e}"))))?;

		let mut response_headers = self
			.set_session_cookies(cookie_value, session.cookie_max_age())
			.map_err(|err| {
				ProxyError::OAuth2AuthenticationFailure(Error::Handshake(format!(
					"unable to persist oauth2 session: {err}"
				)))
			})?;

		let clear_handshake = build_clear_cookie(handshake_cookie_name, self.cookie_secure());
		response_headers.append(
			http::header::SET_COOKIE,
			encode_set_cookie_header(&clear_handshake).map_err(|e| {
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

	pub(super) async fn trigger_auth(
		&self,
		headers: &http::HeaderMap,
		uri: &http::Uri,
		metadata: &crate::http::oidc::OidcMetadata,
		redirect_uri: &Url,
	) -> Result<PolicyResponse, ProxyError> {
		let requested_scopes: Vec<String> = if self.config.scopes.is_empty() {
			super::DEFAULT_SCOPE_PARAM
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
			Cow::Borrowed(super::DEFAULT_SCOPE_PARAM)
		} else {
			Cow::Owned(requested_scopes.join(" "))
		};

		if self.should_return_unauthorized(headers) {
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

		let nonce = Self::random_token();
		let handshake_id = Self::random_token();
		let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

		let state = HandshakeState {
			attachment_key: self.attachment_key.clone(),
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

		let handshake_cookie_name = format!("{}.{}", self.handshake_cookie_base_name(), handshake_id);
		let handshake_cookie = Cookie::build((handshake_cookie_name, "1"))
			.path("/")
			.secure(self.cookie_secure())
			.http_only(true)
			.same_site(cookie::SameSite::Lax)
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

	pub(super) fn query_param(uri: &http::Uri, name: &str) -> Option<String> {
		let query = uri.query()?;
		url::form_urlencoded::parse(query.as_bytes())
			.find_map(|(k, v)| (k == name).then(|| v.into_owned()))
	}

	fn random_token() -> String {
		let mut bytes = [0u8; 32];
		let mut rng = rand::rng();
		rng.fill_bytes(&mut bytes);
		URL_SAFE_NO_PAD.encode(bytes)
	}

	pub(super) fn original_target_from_uri(uri: &http::Uri) -> String {
		let path = uri.path();
		if !path.starts_with('/') {
			return "/".to_string();
		}
		match uri.query() {
			Some(query) => format!("{path}?{query}"),
			None => path.to_string(),
		}
	}

	pub(super) fn is_safe_redirect_target(target: &str) -> bool {
		target.starts_with('/')
			&& !target.starts_with("//")
			&& !target.contains('\\')
			&& !target.chars().any(char::is_control)
	}

	pub(crate) fn callback_attachment_key_from_uri(
		uri: &http::Uri,
		runtime_secret: &RuntimeCookieSecret,
	) -> Option<String> {
		let state = Self::query_param(uri, "state")?;
		let codec = Self::derive_runtime_handshake_codec(runtime_secret.as_bytes()).ok()?;
		let state = codec.decrypt_handshake_state(&state).ok()?;
		Some(state.attachment_key.to_string())
	}
}
