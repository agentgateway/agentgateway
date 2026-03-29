use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ::http::{HeaderValue, StatusCode, header};
use secrecy::ExposeSecret;
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::http::jwt;
use crate::http::{Body, PolicyResponse, Request, Response, Uri};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;

mod callback;
mod local;
mod provider;
mod redirect;
mod session;

#[cfg(test)]
mod tests;

pub use crate::http::oauth::TokenEndpointAuth;
pub use local::LocalOidcConfig;
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

/// Validated absolute HTTP(S) endpoint used by an OIDC provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEndpoint {
	uri: Uri,
	url: url::Url,
}

impl ProviderEndpoint {
	pub fn as_uri(&self) -> &Uri {
		&self.uri
	}

	pub fn as_url(&self) -> &url::Url {
		&self.url
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

		let uri = value
			.parse::<Uri>()
			.map_err(|e| format!("must be a valid HTTP URI: {e}"))?;
		if uri.scheme().is_none() || uri.authority().is_none() {
			return Err("must be an absolute http(s) URL".into());
		}

		Ok(Self { uri, url })
	}
}

impl std::str::FromStr for ProviderEndpoint {
	type Err = String;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Self::try_from(value)
	}
}

impl TryFrom<Uri> for ProviderEndpoint {
	type Error = String;

	fn try_from(value: Uri) -> Result<Self, Self::Error> {
		Self::try_from(value.to_string().as_str())
	}
}

impl fmt::Display for ProviderEndpoint {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.uri.fmt(f)
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

struct CallbackQuery {
	state: String,
	code: Option<String>,
	error: Option<String>,
}

impl OidcPolicy {
	pub async fn apply(
		&self,
		log: Option<&mut RequestLog>,
		req: &mut Request,
		client: PolicyClient,
	) -> Result<PolicyResponse, Error> {
		if let Some(response) = self.maybe_handle_callback(req, client.clone()).await? {
			return Ok(response);
		}

		if let Some(cookie) = read_cookie(req, &self.session.cookie_name)
			&& let Ok(browser_session) = self.session.decode_browser_session(&cookie)
			&& browser_session.policy_id == self.policy_id
			&& let Ok(claims) = self
				.provider
				.id_token_validator
				.validate_claims(browser_session.raw_id_token.expose_secret())
		{
			if let Some(Value::String(sub)) = claims.inner.get("sub")
				&& let Some(log) = log
			{
				log.jwt_sub = Some(sub.clone());
			}
			req.extensions_mut().insert(claims);
			strip_reserved_cookies(req);
			return Ok(PolicyResponse::default());
		}

		// OIDC is an interactive browser policy: unauthenticated non-callback requests enter login.
		callback::start_login(self, req)
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

		if let Some(error) = query.error {
			return Err(Error::ProviderCallback(error));
		}
		let code = query.code.ok_or(Error::InvalidCallback)?;
		let transaction_cookie =
			read_cookie(req, &self.session.transaction_cookie_name).ok_or(Error::MissingTransaction)?;
		let response = callback::handle_callback(
			self,
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

fn iter_request_cookies(req: &Request) -> impl Iterator<Item = (String, String)> + '_ {
	req
		.headers()
		.get_all(header::COOKIE)
		.into_iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|header_value| {
			cookie::Cookie::split_parse(header_value.to_owned())
				.filter_map(|c: Result<cookie::Cookie<'_>, _>| c.ok())
				.map(|c: cookie::Cookie<'_>| (c.name().to_owned(), c.value().to_owned()))
				.collect::<Vec<_>>()
		})
}

fn read_cookie(req: &Request, name: &str) -> Option<String> {
	let mut matched = None;
	for (cookie_name, cookie_value) in iter_request_cookies(req) {
		if cookie_name == name {
			matched = Some(cookie_value);
		}
	}
	matched
}

fn strip_reserved_cookies(req: &mut Request) {
	let preserved: Vec<String> = iter_request_cookies(req)
		.filter(|(name, _)| !name.starts_with(RESERVED_COOKIE_PREFIX))
		.map(|(name, value)| format!("{name}={value}"))
		.collect();

	req.headers_mut().remove(header::COOKIE);
	if !preserved.is_empty() {
		let hv = HeaderValue::from_str(&preserved.join("; "))
			.expect("re-joined cookie header from valid source data");
		req.headers_mut().insert(header::COOKIE, hv);
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
