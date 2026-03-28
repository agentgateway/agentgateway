use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ::http::{HeaderValue, StatusCode, header};
use secrecy::ExposeSecret;
use secrecy::SecretString;
use serde_json::{Map, Value};

use crate::http::jwt;
use crate::http::{Body, PolicyResponse, Request, Response, Uri};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;

mod callback;
pub mod config;
mod provider;
mod redirect;
mod session;

#[cfg(test)]
mod tests;

pub use config::LocalOidcConfig;
pub use redirect::RedirectUri;
pub use session::{
	BrowserSession, CookieSecureMode, RESERVED_COOKIE_PREFIX, SameSiteMode, SessionConfig,
	TransactionState,
};

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

impl From<String> for PolicyId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for PolicyId {
	fn from(value: &str) -> Self {
		Self(value.to_string())
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcPolicy {
	pub policy_id: PolicyId,
	pub provider: Arc<Provider>,
	pub client: ClientConfig,
	pub redirect_uri: RedirectUri,
	pub unauthenticated_action: UnauthenticatedAction,
	pub session: SessionConfig,
	pub scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
	pub issuer: String,
	#[serde(serialize_with = "crate::serdes::ser_display")]
	pub authorization_endpoint: Uri,
	#[serde(serialize_with = "crate::serdes::ser_display")]
	pub token_endpoint: Uri,
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

#[derive(
	Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TokenEndpointAuth {
	#[default]
	ClientSecretBasic,
	ClientSecretPost,
}

impl TokenEndpointAuth {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::ClientSecretBasic => "clientSecretBasic",
			Self::ClientSecretPost => "clientSecretPost",
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("missing session")]
	MissingSession,
	#[error("invalid session")]
	InvalidSession,
	#[error("authentication required")]
	AuthenticationRequired,
	#[error("encoded browser session exceeds cookie size budget")]
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

impl Error {
	pub fn status_code(&self) -> StatusCode {
		match self {
			Self::AuthenticationRequired => StatusCode::UNAUTHORIZED,
			Self::MissingSession
			| Self::InvalidSession
			| Self::MissingTransaction
			| Self::InvalidTransaction
			| Self::PolicyMismatch
			| Self::CsrfMismatch
			| Self::NonceMismatch
			| Self::InvalidCallback
			| Self::ProviderCallback(_) => StatusCode::BAD_REQUEST,
			Self::SessionCookieTooLarge
			| Self::TokenExchangeFailed(_)
			| Self::MissingIdToken
			| Self::InvalidIdToken(_)
			| Self::Config(_)
			| Self::Http(_) => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}
}

struct CallbackQuery {
	state: String,
	code: Option<String>,
	error: Option<String>,
}

#[derive(
	Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum UnauthenticatedAction {
	#[default]
	Auto,
	Redirect,
	Deny,
}

impl OidcPolicy {
	pub async fn apply(
		&self,
		mut log: Option<&mut RequestLog>,
		req: &mut Request,
		client: PolicyClient,
	) -> Result<PolicyResponse, Error> {
		if let Some(response) = self
			.maybe_handle_callback(log.as_deref_mut(), req, client.clone())
			.await?
		{
			return Ok(response);
		}

		if let Some(cookie) = crate::http::request_cookies::read_cookie(req, &self.session.cookie_name)
			&& let Ok(browser_session) = self.session.decode_browser_session(&cookie)
			&& browser_session.policy_id == self.policy_id
			&& !browser_session.is_expired()
			&& let Ok(claims) = self
				.provider
				.id_token_validator
				.validate_claims(browser_session.raw_id_token.expose_secret())
		{
			if let Some(Value::String(sub)) = claims.inner.get("sub")
				&& let Some(log) = log.as_deref_mut()
			{
				log.jwt_sub = Some(sub.clone());
			}
			req.extensions_mut().insert(claims);
			crate::http::request_cookies::strip_cookies_by_prefix(req, RESERVED_COOKIE_PREFIX);
			return Ok(PolicyResponse::default());
		}

		match self.unauthenticated_action {
			UnauthenticatedAction::Auto if should_start_login_redirect(req) => {
				callback::start_login(self, log, req)
			},
			UnauthenticatedAction::Auto | UnauthenticatedAction::Deny => {
				Err(Error::AuthenticationRequired)
			},
			UnauthenticatedAction::Redirect => callback::start_login(self, log, req),
		}
	}

	async fn maybe_handle_callback(
		&self,
		log: Option<&mut RequestLog>,
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

		if let Some(error) = query.error {
			return Err(Error::ProviderCallback(error));
		}
		let code = query.code.ok_or(Error::InvalidCallback)?;
		let transaction_cookie =
			crate::http::request_cookies::read_cookie(req, &self.session.transaction_cookie_name)
				.ok_or(Error::MissingTransaction)?;
		let response = callback::handle_callback(
			self,
			log,
			callback::CallbackRequestContext {
				is_https: req.uri().scheme_str() == Some("https"),
				code,
				state: query.state,
				transaction_cookie,
			},
			client,
		)
		.await?;
		Ok(Some(response))
	}
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

fn should_start_login_redirect(req: &Request) -> bool {
	// Keep the default narrow: only GET requests that explicitly prefer an HTML document enter the
	// interactive browser login flow. Everything else gets a normal 401.
	if req.method() != ::http::Method::GET {
		return false;
	}

	accepts_html_document(req)
}

fn accepts_html_document(req: &Request) -> bool {
	let accept = req
		.headers()
		.get(header::ACCEPT)
		.and_then(|v| v.to_str().ok())
		.unwrap_or_default();
	let html = accept.find("text/html").unwrap_or(usize::MAX);
	let json = accept.find("application/json").unwrap_or(usize::MAX);
	let sse = accept.find("text/event-stream").unwrap_or(usize::MAX);
	html != usize::MAX && html < json && html < sse
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

pub(crate) fn now_unix() -> u64 {
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
