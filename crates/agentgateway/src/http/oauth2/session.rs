use std::time::{Duration, SystemTime};

use secrecy::ExposeSecret;
use tracing::warn;

use super::cookies::{SessionCookieError, for_each_request_cookie, set_session_cookies};
use super::{OAuth2, OAuth2CallContext};
use crate::http::Request;
use crate::http::auth::UpstreamAccessToken;
use crate::http::jwt::{Claims, Jwt};
use crate::http::oidc::{Error as OidcError, OidcMetadata, RefreshTokenRequest};

impl OAuth2 {
	pub(super) fn get_session(
		&self,
		headers: &http::HeaderMap,
	) -> Option<super::state::SessionState> {
		let cookie_name = self.session_cookie_name();
		let mut value = None;
		for_each_request_cookie(headers, |cookie| {
			if cookie.name() == cookie_name {
				value = Some(cookie.value().to_string());
			}
		});
		self.decode_session(value.as_deref()?).ok()
	}

	pub(super) async fn refresh_session(
		&self,
		runtime: OAuth2CallContext<'_>,
		session: &mut super::state::SessionState,
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
		self.update_session_identity(session, token_resp.id_token.as_deref(), jwt_validator)?;

		Ok(true)
	}

	fn update_session_identity(
		&self,
		session: &mut super::state::SessionState,
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
				if let Some(token_nonce) = claims.inner.get("nonce").and_then(|v| v.as_str())
					&& let Some(expected) = &session.nonce
					&& token_nonce != expected
				{
					warn!("refreshed id_token nonce mismatch, clearing id_token");
					session.id_token = None;
					return Ok(());
				}
				session.id_token = Some(id_token.to_string());
			},
			None => {
				// Many providers omit id_token on refresh; preserve the last validated id_token.
			},
		}
		Ok(())
	}

	pub(super) fn session_claims(&self, session: &super::state::SessionState) -> Option<Claims> {
		let id_token = session.id_token.as_deref()?;
		let jwt_validator = self.resolved_jwt_validator.as_deref()?;
		match jwt_validator.validate_claims(id_token) {
			Ok(claims) => Some(claims),
			Err(err) => {
				warn!(error = %err, "stored oauth2 id_token failed validation");
				None
			},
		}
	}

	pub(super) fn inject_auth(&self, req: &mut Request, access_token: &str, claims: Option<Claims>) {
		req
			.extensions_mut()
			.insert(UpstreamAccessToken(access_token.to_string().into()));

		if let Some(claims) = claims {
			req.extensions_mut().insert(claims);
		}
	}

	pub(super) fn set_session_cookies(
		&self,
		value: String,
		cookie_max_age: cookie::time::Duration,
	) -> Result<crate::http::HeaderMap, SessionCookieError> {
		set_session_cookies(
			self.session_cookie_name(),
			self.cookie_secure(),
			value,
			cookie_max_age,
		)
	}
}
