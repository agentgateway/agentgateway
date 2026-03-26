use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ::http::{HeaderValue, StatusCode, header};
use agent_core::prelude::Strng;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use serde_json::{Map, Value};

use crate::http::jwt;
use crate::http::{Body, PolicyResponse, Request, Response, Uri};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;
use crate::types::agent::{HostnameMatch, Listener, ListenerOidc, ListenerProtocol};

mod callback;
pub mod config;
pub mod provider;
mod redirect;
pub mod session;

#[cfg(test)]
mod tests;

pub use config::{LocalOidcConfig, LocalOidcListenerConfig, LocalOidcProvider, OidcProviderRef};
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
	pub session: SessionConfig,
	pub scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedOidcProvider {
	pub name: Strng,
	pub policy: OidcPolicy,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
	pub issuer: String,
	#[serde(serialize_with = "crate::serdes::ser_display")]
	pub authorization_endpoint: Uri,
	#[serde(serialize_with = "crate::serdes::ser_display")]
	pub token_endpoint: Uri,
	pub token_endpoint_auth_methods_supported: Vec<TokenEndpointAuth>,
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

impl OidcPolicy {
	pub async fn apply(
		&self,
		log: Option<&mut RequestLog>,
		req: &mut Request,
		_client: PolicyClient,
	) -> Result<PolicyResponse, Error> {
		if is_cors_preflight(req) {
			return Ok(PolicyResponse::default());
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
				&& let Some(log) = log
			{
				log.jwt_sub = Some(sub.clone());
			}
			req.extensions_mut().insert(claims);
			return Ok(PolicyResponse::default());
		}

		if self.should_redirect(req) {
			callback::start_login(self, log, req)
		} else {
			Ok(PolicyResponse::default().with_response(unauthorized_response()))
		}
	}

	fn should_redirect(&self, req: &Request) -> bool {
		// Only redirect real browser document navigations into the login flow. API/XHR/fetch-style
		// requests should get a 401 so callers do not see an unexpected HTML redirect.
		if req.method() != ::http::Method::GET {
			return false;
		}
		if req
			.headers()
			.get("x-requested-with")
			.and_then(|v| v.to_str().ok())
			.is_some_and(|v| v.eq_ignore_ascii_case("xmlhttprequest"))
		{
			return false;
		}
		if req
			.headers()
			.get("sec-fetch-mode")
			.and_then(|v| v.to_str().ok())
			.is_some_and(|v| !v.eq_ignore_ascii_case("navigate"))
		{
			return false;
		}

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
}

fn is_cors_preflight(req: &Request) -> bool {
	req.method() == ::http::Method::OPTIONS
		&& req
			.headers()
			.contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
}

impl Listener {
	pub(crate) async fn maybe_handle_oidc_callback(
		&self,
		log: Option<&mut RequestLog>,
		req: &mut Request,
		client: PolicyClient,
	) -> Result<PolicyResponse, Error> {
		let Some(oidc) = &self.oidc else {
			return Ok(PolicyResponse::default());
		};
		oidc.maybe_handle_callback(log, req, client).await
	}

	pub(crate) fn validate_oidc(&self, listener_port: u16) -> anyhow::Result<()> {
		let Some(oidc) = &self.oidc else {
			return Ok(());
		};
		oidc.validate_for_listener(self, listener_port)
	}
}

impl ListenerOidc {
	async fn maybe_handle_callback(
		&self,
		log: Option<&mut RequestLog>,
		req: &mut Request,
		client: PolicyClient,
	) -> Result<PolicyResponse, Error> {
		if req.method() != ::http::Method::GET {
			return Ok(PolicyResponse::default());
		}

		let Some(query) = CallbackQuery::parse(req) else {
			return Ok(PolicyResponse::default());
		};

		let Some(policy) = self.find_callback_policy(req)? else {
			return Ok(PolicyResponse::default());
		};

		if let Some(error) = query.error {
			return Err(Error::ProviderCallback(error));
		}
		let code = query.code.ok_or(Error::InvalidCallback)?;
		let transaction_cookie =
			crate::http::request_cookies::read_cookie(req, &policy.session.transaction_cookie_name)
				.ok_or(Error::MissingTransaction)?;
		callback::handle_callback(
			policy,
			log,
			callback::CallbackRequestContext {
				is_https: req.uri().scheme_str() == Some("https"),
				code,
				state: query.state,
				transaction_cookie,
			},
			client,
		)
		.await
	}

	fn find_callback_policy<'a>(&'a self, req: &Request) -> Result<Option<&'a OidcPolicy>, Error> {
		let host = req
			.uri()
			.host()
			.ok_or(Error::InvalidCallback)?
			.to_ascii_lowercase();
		let scheme = req.uri().scheme_str().ok_or(Error::InvalidCallback)?;
		let port = req
			.uri()
			.port_u16()
			.or_else(|| default_port_for_scheme(scheme))
			.ok_or(Error::InvalidCallback)?;

		let Some(matches) = self.callback_matches(&host, port, req.uri().path()) else {
			return Ok(None);
		};
		match matches {
			[idx] => Ok(self.providers().get(*idx).map(|provider| &provider.policy)),
			_ => Err(Error::Config(
				"multiple oidc providers matched the same callback request".into(),
			)),
		}
	}

	fn validate_for_listener(&self, listener: &Listener, listener_port: u16) -> anyhow::Result<()> {
		let host_match = if listener.hostname.is_empty() {
			HostnameMatch::None
		} else {
			HostnameMatch::from(listener.hostname.clone())
		};

		let mut callback_owners = HashSet::with_capacity(self.providers().len());
		let mut host_cookie_names: std::collections::HashMap<&str, HashSet<&str>> =
			std::collections::HashMap::with_capacity(self.providers().len());

		for provider in self.providers() {
			let policy = &provider.policy;
			match &listener.protocol {
				ListenerProtocol::HTTP => {
					anyhow::ensure!(
						!policy.redirect_uri.https,
						"oidc redirectURI '{}' must use http on HTTP listener '{}'",
						policy.redirect_uri.redirect_uri,
						listener.name.listener_name
					);
				},
				ListenerProtocol::HTTPS(_) => {
					anyhow::ensure!(
						policy.redirect_uri.https,
						"oidc redirectURI '{}' must use https on HTTPS listener '{}'",
						policy.redirect_uri.redirect_uri,
						listener.name.listener_name
					);
				},
				_ => {
					anyhow::bail!(
						"oidc requires an HTTP or HTTPS listener, got incompatible listener '{}'",
						listener.name.listener_name
					);
				},
			}
			anyhow::ensure!(
				policy.redirect_uri.port == listener_port,
				"oidc redirectURI '{}' must use listener port '{}' for listener '{}'",
				policy.redirect_uri.redirect_uri,
				listener_port,
				listener.name.listener_name
			);
			anyhow::ensure!(
				host_match.matches_host(&policy.redirect_uri.host),
				"oidc redirectURI host '{}' is not covered by listener hostname '{}' on listener '{}'",
				policy.redirect_uri.host,
				if listener.hostname.is_empty() {
					"*"
				} else {
					listener.hostname.as_str()
				},
				listener.name.listener_name
			);

			let callback_key = (
				policy.redirect_uri.host.as_str(),
				policy.redirect_uri.port,
				policy.redirect_uri.callback_path.path(),
			);
			anyhow::ensure!(
				callback_owners.insert(callback_key),
				"duplicate oidc callback ownership for '{}:{}{}' on listener '{}'",
				policy.redirect_uri.host,
				policy.redirect_uri.port,
				policy.redirect_uri.callback_path,
				listener.name.listener_name
			);

			let reserved_cookie_names = host_cookie_names
				.entry(policy.redirect_uri.host.as_str())
				.or_default();
			for cookie_name in [
				policy.session.cookie_name.as_str(),
				policy.session.transaction_cookie_name.as_str(),
			] {
				anyhow::ensure!(
					reserved_cookie_names.insert(cookie_name),
					"duplicate oidc cookie name '{}' for redirect host '{}' on listener '{}'",
					cookie_name,
					policy.redirect_uri.host,
					listener.name.listener_name
				);
			}
		}
		Ok(())
	}
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
	match scheme {
		"http" => Some(80),
		"https" => Some(443),
		_ => None,
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

fn unauthorized_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::UNAUTHORIZED)
		.header(header::CONTENT_TYPE, "text/plain")
		.body(Body::from("unauthorized"))
		.expect("static unauthorized response")
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
