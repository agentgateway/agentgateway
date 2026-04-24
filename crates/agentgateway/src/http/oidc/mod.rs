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
pub(crate) use local::compile_oidc_policy_from_xds;
pub use redirect::RedirectUri;
pub use session::{
	BrowserSession, CookieSecureMode, RESERVED_COOKIE_PREFIX, SameSiteMode, SessionConfig,
	TransactionState,
};

pub use crate::http::oauth::TokenEndpointAuth;

/// Stable identity used to prefix session cookies and to correlate an OIDC
/// policy between the controller, xDS, and the dataplane runtime. The
/// canonical wire form is `policy/<key>` or `route/<key>`; see
/// [`PolicyId::policy`] / [`PolicyId::route`] for the constructors and
/// [`<PolicyId as std::str::FromStr>::from_str`] for the parser.
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

impl std::str::FromStr for PolicyId {
	type Err = String;

	/// Parse a PolicyId from its canonical string form, used to transport the
	/// identity through xDS. The encoding is produced by [`PolicyId::route`]
	/// and [`PolicyId::policy`]; any other shape is rejected so a malformed
	/// identity never becomes a stable cookie prefix.
	fn from_str(value: &str) -> Result<Self, Self::Err> {
		let (prefix, rest) = value
			.split_once('/')
			.ok_or_else(|| format!("policy id '{value}' is missing a '<prefix>/' segment"))?;
		if rest.is_empty() {
			return Err(format!("policy id '{value}' has an empty identifier"));
		}
		match prefix {
			"route" | "policy" => Ok(Self(value.to_string())),
			other => Err(format!(
				"policy id prefix '{other}' is not supported (expected 'route' or 'policy')"
			)),
		}
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
	pub credentials: ClientCredentials,
}

/// Runtime-strict pairing of a token-endpoint auth method with the secret it
/// requires. Confidential variants carry the `client_secret`; `None` is the
/// public-client mode where PKCE alone protects the authorization code
/// exchange. By tying the method to the secret at the type level, public and
/// confidential configurations cannot be mixed by accident at a call site.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "method")]
pub enum ClientCredentials {
	ClientSecretBasic {
		#[serde(serialize_with = "crate::serdes::ser_redact")]
		client_secret: SecretString,
	},
	ClientSecretPost {
		#[serde(serialize_with = "crate::serdes::ser_redact")]
		client_secret: SecretString,
	},
	None,
}

impl ClientCredentials {
	#[cfg(test)]
	pub(super) fn method(&self) -> TokenEndpointAuth {
		match self {
			Self::ClientSecretBasic { .. } => TokenEndpointAuth::ClientSecretBasic,
			Self::ClientSecretPost { .. } => TokenEndpointAuth::ClientSecretPost,
			Self::None => TokenEndpointAuth::None,
		}
	}

	/// Pair a method with its (optional) secret, rejecting mismatches. Returns
	/// an [`Error::Config`] with a user-comprehensible message that names both
	/// the method and whether a secret was expected.
	pub fn from_parts(
		method: TokenEndpointAuth,
		client_secret: Option<SecretString>,
	) -> Result<Self, Error> {
		match (method, client_secret) {
			(TokenEndpointAuth::ClientSecretBasic, Some(client_secret)) => {
				Ok(Self::ClientSecretBasic { client_secret })
			},
			(TokenEndpointAuth::ClientSecretPost, Some(client_secret)) => {
				Ok(Self::ClientSecretPost { client_secret })
			},
			(TokenEndpointAuth::None, None) => Ok(Self::None),
			(method, None) if method.requires_client_secret() => Err(Error::Config(format!(
				"tokenEndpointAuthMethod {} requires a clientSecret",
				method.as_str()
			))),
			(TokenEndpointAuth::None, Some(_)) => Err(Error::Config(
				"tokenEndpointAuthMethod None must not be paired with a clientSecret".into(),
			)),
			// Unreachable today (requires_client_secret covers the confidential methods)
			// but kept exhaustive so a future method addition is a compile error.
			(method, _) => Err(Error::Config(format!(
				"tokenEndpointAuthMethod {} is not supported in this build",
				method.as_str()
			))),
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
			match self.session.decode_browser_session(&cookie) {
				Ok(browser_session) => {
					if browser_session.policy_id == self.policy_id
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
					}
				},
				Err(err) => {
					debug!(error=%err, "failed to decode oidc browser session cookie");
				},
			}
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

pub(crate) fn now_unix() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or(Duration::ZERO)
		.as_secs()
}

pub(crate) fn dedupe_scopes(scopes: Vec<String>) -> Vec<String> {
	let mut seen = HashSet::new();
	seen.insert("openid".to_string());

	let mut deduped = Vec::with_capacity(scopes.len() + 1);
	deduped.push("openid".to_string());
	for scope in scopes {
		if seen.insert(scope.clone()) {
			deduped.push(scope);
		}
	}
	deduped
}

pub(crate) fn cap_session_expiry(now: u64, ttl: Duration, claims: &Map<String, Value>) -> u64 {
	let ttl_exp = now.saturating_add(ttl.as_secs());
	match claims.get("exp").and_then(Value::as_u64) {
		Some(exp) => exp.min(ttl_exp),
		None => ttl_exp,
	}
}
