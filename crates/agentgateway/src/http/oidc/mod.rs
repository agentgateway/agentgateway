use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ::http::{HeaderValue, StatusCode, header};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use tracing::debug;

use crate::http::{Body, PolicyResponse, Request, Response, jwt};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;

mod callback;
mod local;
mod provider;
mod redirect;
mod session;

#[cfg(test)]
mod tests;

pub use local::LocalOidcConfig;
pub use redirect::RedirectUri;
pub use session::{
	BrowserSession, CookieSecureMode, RESERVED_COOKIE_PREFIX, RefreshSession, SameSiteMode,
	SessionConfig, TransactionState,
};

pub use crate::http::oauth::TokenEndpointAuth;

#[derive(
	Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct PolicyId(String);

impl PolicyId {
	pub fn as_str(&self) -> &str {
		&self.0
	}

	pub fn route(route_key: impl std::fmt::Display) -> Self {
		Self(format!("route/{route_key}"))
	}

	pub fn policy(policy_key: impl std::fmt::Display) -> Self {
		Self(format!("policy/{policy_key}"))
	}
}

/// Validated absolute HTTP(S) endpoint used by an OIDC provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEndpoint(url::Url);

impl ProviderEndpoint {
	pub fn as_str(&self) -> &str {
		self.0.as_ref()
	}

	pub fn with_query(&self, params: &[(&str, String)]) -> String {
		let mut url = self.0.clone();
		{
			let mut query = url.query_pairs_mut();
			for (key, value) in params {
				query.append_pair(key, value);
			}
		}
		url.to_string()
	}
}

impl TryFrom<&str> for ProviderEndpoint {
	type Error = String;

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		let url =
			url::Url::parse(value).map_err(|e| format!("must be an absolute http(s) URL: {e}"))?;
		if !matches!(url.scheme(), "http" | "https") {
			return Err(format!(
				"must use an http or https scheme, got '{}'",
				url.scheme()
			));
		}

		Ok(Self(url))
	}
}

impl std::str::FromStr for ProviderEndpoint {
	type Err = String;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Self::try_from(value)
	}
}

impl fmt::Display for ProviderEndpoint {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(f)
	}
}

impl Serialize for ProviderEndpoint {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.to_string().serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for ProviderEndpoint {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let value = String::deserialize(deserializer)?;
		Self::try_from(value.as_str()).map_err(serde::de::Error::custom)
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcPolicy {
	pub policy_id: PolicyId,
	pub provider: Arc<Provider>,
	pub client: ClientConfig,
	pub redirect_uri: RedirectUri,
	pub session: SessionConfig,
	pub scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
	pub issuer: String,
	pub authorization_endpoint: ProviderEndpoint,
	pub token_endpoint: ProviderEndpoint,
	pub id_token_validator: jwt::Jwt,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConfig {
	pub client_id: String,
	#[serde(serialize_with = "crate::serdes::ser_redact")]
	pub client_secret: SecretString,
	pub token_endpoint_auth: TokenEndpointAuth,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("missing session")]
	MissingSession,
	#[error("invalid session")]
	InvalidSession,
	#[error("authentication required")]
	AuthenticationRequired,
	#[error("encoded oidc session exceeds cookie size budget")]
	SessionCookieTooLarge,
	#[error("missing transaction")]
	MissingTransaction,
	#[error("invalid transaction")]
	InvalidTransaction,
	#[error("policy mismatch")]
	PolicyMismatch,
	#[error("csrf mismatch")]
	CsrfMismatch,
	#[error("token exchange failed")]
	TokenExchangeFailed(#[source] anyhow::Error),
	#[error("missing id token")]
	MissingIdToken,
	#[error("invalid id token: {0}")]
	InvalidIdToken(jwt::TokenError),
	#[error("nonce mismatch")]
	NonceMismatch,
	#[error("invalid callback")]
	InvalidCallback,
	#[error("oidc provider returned callback error '{0}'")]
	ProviderCallback(String),
	#[error("{0}")]
	Config(String),
	#[error("{0}")]
	Http(#[from] anyhow::Error),
}

struct CallbackQuery {
	state: String,
	code: Option<String>,
	error: Option<String>,
}

impl OidcPolicy {
	pub async fn apply(
		&self,
		log: &mut RequestLog,
		req: &mut Request,
		client: PolicyClient,
	) -> Result<PolicyResponse, Error> {
		if let Some(response) = self.maybe_handle_callback(req, client.clone()).await? {
			return Ok(response);
		}

		if is_cors_preflight(req) {
			return Ok(PolicyResponse::default());
		}

		if let Some(cookie) = crate::http::read_request_cookie(req, &self.session.cookie_name) {
			match self.session.decode_browser_session_for_refresh(&cookie) {
				Ok(browser_session) => {
					if browser_session.policy_id != self.policy_id {
						debug!("oidc browser session rejected due to policy mismatch");
					} else if !browser_session.is_expired()
						&& let Ok(claims) = self
							.provider
							.id_token_validator
							.validate_claims(browser_session.raw_id_token.expose_secret())
					{
						if let Some(Value::String(sub)) = claims.inner.get("sub") {
							log.jwt_sub = Some(sub.clone());
						}
						req.extensions_mut().insert(claims);
						return Ok(PolicyResponse::default());
					} else if let Some(cookie) =
						crate::http::read_request_cookie(req, &self.session.refresh_cookie_name)
					{
						match self.session.decode_refresh_session(&cookie) {
							Ok(refresh_session)
								if refresh_session.policy_id == browser_session.policy_id
									&& refresh_session.subject == browser_session.subject =>
							{
								match self
									.refresh_browser_session(browser_session, refresh_session, client)
									.await
								{
									Ok((claims, response)) => {
										if let Some(Value::String(sub)) = claims.inner.get("sub") {
											log.jwt_sub = Some(sub.clone());
										}
										req.extensions_mut().insert(claims);
										return Ok(response);
									},
									Err(err) => {
										debug!(error=%err, "failed to refresh oidc browser session");
									},
								}
							},
							// The two encrypted cookies are independently valid, so bind them to the same
							// policy and original subject before using the refresh credential.
							Ok(_) => debug!("oidc refresh session rejected due to session mismatch"),
							Err(err) => {
								debug!(error=%err, "failed to decode oidc refresh session cookie");
							},
						}
					}
				},
				Err(err) => {
					debug!(error=%err, "failed to decode oidc browser session cookie");
				},
			}
		}

		// Fetch Metadata is controlled by the browser. Known non-navigation modes identify requests
		// that cannot complete a cross-origin OIDC redirect; return 401 so the caller can initiate a
		// document navigation instead. Missing, navigation, and unknown modes retain the redirect.
		if req.headers().get("sec-fetch-mode").is_some_and(|mode| {
			matches!(
				mode.to_str(),
				Ok("cors" | "no-cors" | "same-origin" | "websocket")
			)
		}) {
			return Err(Error::AuthenticationRequired);
		}

		// OIDC is an interactive browser policy: unauthenticated non-callback requests enter login.
		callback::start_login(self, req)
	}

	async fn refresh_browser_session(
		&self,
		mut browser_session: BrowserSession,
		mut refresh_session: RefreshSession,
		client: PolicyClient,
	) -> Result<(jwt::Claims, PolicyResponse), Error> {
		let token = provider::refresh_token(
			client,
			&self.provider,
			&self.client,
			&refresh_session.refresh_token,
		)
		.await?;
		let id_token = token.id_token.ok_or(Error::MissingIdToken)?;
		let claims = self
			.provider
			.id_token_validator
			.validate_claims(&id_token)
			.map_err(Error::InvalidIdToken)?;
		// OIDC Core 1.0 section 12.2 requires a refreshed ID token to have the same subject as the
		// original ID token. Normal signature, issuer, and audience validation alone would still
		// accept a valid token issued for a different user.
		if claims.inner.get("sub").and_then(Value::as_str) != browser_session.subject.as_deref() {
			return Err(Error::InvalidSession);
		}

		browser_session.raw_id_token = SecretString::new(id_token.into_boxed_str());
		if let Some(refresh_token) = token.refresh_token {
			refresh_session.refresh_token = SecretString::new(refresh_token.into_boxed_str());
		}
		refresh_session.expires_at_unix = now_unix().saturating_add(self.session.ttl.as_secs());
		browser_session.expires_at_unix = Some(cap_session_expiry(
			now_unix(),
			self.session.ttl,
			&claims.inner,
		));

		let encoded_session = self.session.encode_browser_session(&browser_session)?;
		let session_cookie = self.session.set_cookie(
			&self.session.cookie_name,
			&encoded_session,
			self.redirect_uri.https,
			self.session.ttl,
		);
		let refresh_cookie = match self.session.encode_refresh_session(&refresh_session) {
			Ok(encoded) => self.session.set_cookie(
				&self.session.refresh_cookie_name,
				&encoded,
				self.redirect_uri.https,
				self.session.ttl,
			),
			// A rotated token can be larger than the token it replaces. Keep the newly refreshed
			// browser session usable, but discard refresh capability rather than the whole request.
			Err(Error::SessionCookieTooLarge) => {
				debug!(
					"rotated oidc refresh token exceeds cookie size budget; refresh disabled for session"
				);
				self
					.session
					.clear_cookie(&self.session.refresh_cookie_name, self.redirect_uri.https)
			},
			Err(err) => return Err(err),
		};
		let mut response_headers = ::http::HeaderMap::new();
		response_headers.append(
			header::SET_COOKIE,
			HeaderValue::from_str(&session_cookie)
				.map_err(|e| Error::Config(format!("invalid set-cookie header: {e}")))?,
		);
		response_headers.append(
			header::SET_COOKIE,
			HeaderValue::from_str(&refresh_cookie)
				.map_err(|e| Error::Config(format!("invalid set-cookie header: {e}")))?,
		);
		Ok((
			claims,
			PolicyResponse {
				direct_response: None,
				response_headers: Some(response_headers),
			},
		))
	}

	async fn maybe_handle_callback(
		&self,
		req: &mut Request,
		client: PolicyClient,
	) -> Result<Option<PolicyResponse>, Error> {
		if req.method() != ::http::Method::GET
			|| req.uri().path() != self.redirect_uri.callback_path.path()
		{
			return Ok(None);
		}

		let Some(query) = CallbackQuery::parse(req) else {
			return Ok(None);
		};

		let callback_state = callback::CallbackTransactionState::decode(&query.state)?;
		let transaction_cookie_name = self
			.session
			.transaction_cookie_name(&callback_state.transaction_id);
		let transaction_cookie = crate::http::read_request_cookie(req, &transaction_cookie_name)
			.ok_or(Error::MissingTransaction)?
			.to_string();
		if let Some(error) = query.error {
			return Err(Error::ProviderCallback(error));
		}
		let code = query.code.ok_or(Error::InvalidCallback)?;
		let response = callback::handle_callback(
			self,
			callback::CallbackRequestContext {
				code,
				callback_state,
				transaction_cookie_name,
				transaction_cookie,
			},
			client,
		)
		.await?;
		Ok(Some(response))
	}
}

impl crate::store::RequestPolicyTrait for OidcPolicy {
	async fn apply(
		&self,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, crate::proxy::ProxyResponse> {
		self
			.apply(log, req, client.clone())
			.await
			.map_err(|e| crate::proxy::ProxyResponse::from(crate::proxy::ProxyError::OidcFailure(e)))
	}
}

fn is_cors_preflight(req: &Request) -> bool {
	req.method() == ::http::Method::OPTIONS
		&& req.headers().contains_key(header::ORIGIN)
		&& req
			.headers()
			.get(header::ACCESS_CONTROL_REQUEST_METHOD)
			.map(|value| !value.as_bytes().is_empty())
			.unwrap_or(false)
}

impl CallbackQuery {
	/// Parse callback query parameters from the request in a single pass.
	/// Returns `None` if the query does not contain `state` + (`code` | `error`),
	/// meaning this request is not an OAuth2 callback.
	fn parse(req: &Request) -> Option<Self> {
		let mut state = None;
		let mut code = None;
		let mut error = None;
		for (key, value) in
			url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes())
		{
			match key.as_ref() {
				"state" => state = Some(value.into_owned()),
				"code" => code = Some(value.into_owned()),
				"error" => error = Some(value.into_owned()),
				_ => {},
			}
		}
		let state = state?;
		if code.is_none() && error.is_none() {
			return None;
		}
		Some(CallbackQuery { state, code, error })
	}
}

pub(crate) fn build_redirect_response(
	location: &str,
	set_cookies: &[String],
) -> Result<Response, Error> {
	let mut response = ::http::Response::builder()
		.status(StatusCode::FOUND)
		.header(header::LOCATION, location);
	let headers = response
		.headers_mut()
		.ok_or_else(|| Error::Config("failed to build redirect response".into()))?;
	for cookie in set_cookies {
		headers.append(
			header::SET_COOKIE,
			HeaderValue::from_str(cookie)
				.map_err(|e| Error::Config(format!("invalid set-cookie header: {e}")))?,
		);
	}
	response
		.body(Body::empty())
		.map_err(|e| Error::Config(format!("failed to finalize redirect response: {e}")))
}

pub fn now_unix() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or(Duration::ZERO)
		.as_secs()
}

pub(crate) fn dedupe_scopes(mut scopes: Vec<String>) -> Vec<String> {
	scopes.insert(0, "openid".into());
	let mut seen = HashSet::new();
	scopes.retain(|scope| seen.insert(scope.clone()));
	scopes
}

pub(crate) fn cap_session_expiry(now: u64, ttl: Duration, claims: &Map<String, Value>) -> u64 {
	let ttl_exp = now.saturating_add(ttl.as_secs());
	match claims.get("exp").and_then(Value::as_u64) {
		Some(exp) => exp.min(ttl_exp),
		None => ttl_exp,
	}
}
