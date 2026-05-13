use base64::Engine;
use htpasswd_verify_fork::Htpasswd;
use macro_rules_attribute::apply;

use crate::http::Request;
use crate::http::auth::AuthorizationLocation;
use crate::proxy::dtrace::{self};
use crate::proxy::{ProxyError, ProxyResponse};
use crate::*;

#[cfg(test)]
#[path = "basicauth_tests.rs"]
mod tests;

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("no basic authentication credentials found")]
	Missing { realm: String },

	#[error("invalid credentials")]
	InvalidCredentials { realm: String },
}

/// Validation mode for basic authentication
#[apply(schema!)]
#[derive(Copy, PartialEq, Eq, Default)]
pub enum Mode {
	/// A valid username/password must be present.
	Strict,
	/// If credentials exist, validate them.
	/// This is the default option.
	/// Warning: this allows requests without credentials!
	#[default]
	Optional,
}

#[apply(schema!)]
#[derive(::cel::DynamicType)]
pub struct Claims {
	pub username: Strng,
}

#[serde_with::serde_as]
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "LocalBasicAuth"))]
pub struct BasicAuthentication {
	/// Path to .htpasswd file containing user credentials
	#[serde(serialize_with = "ser_redact")]
	pub htpasswd: Arc<Htpasswd<'static>>,

	/// Realm name for the WWW-Authenticate header
	pub realm: Option<String>,

	/// Validation mode for basic authentication
	pub mode: Mode,

	#[serde(default = "default_authorization_location")]
	pub authorization_location: AuthorizationLocation,
}

fn default_realm() -> String {
	"Restricted".to_string()
}

const TRACE_POLICY_KIND: &str = "basic_auth";

impl BasicAuthentication {
	/// Create a new BasicAuthentication from a file path
	pub fn new(
		htpasswd: &str,
		realm: Option<String>,
		mode: Mode,
		authorization_location: AuthorizationLocation,
	) -> Self {
		let htpasswd = Htpasswd::new(htpasswd);

		Self {
			htpasswd: Arc::new(htpasswd),
			realm,
			mode,
			authorization_location,
		}
	}

	async fn verify(&self, req: &mut Request) -> Result<Option<Claims>, ProxyError> {
		let Some(encoded_credentials) = self.authorization_location.extract(req) else {
			// In strict mode, we require credentials
			if self.mode == Mode::Strict {
				dtrace::pol_result!(
					dtrace::Error,
					Apply,
					"rejected request because basic auth credentials are required but missing"
				);
				return Err(ProxyError::BasicAuthenticationFailure(Error::Missing {
					realm: self.realm.clone().unwrap_or_else(default_realm),
				}));
			}
			// Otherwise without credentials, don't attempt to authenticate
			dtrace::pol_result!(
				dtrace::Info,
				Skip,
				"request has no basic auth credentials and auth mode is optional"
			);
			return Ok(None);
		};

		let invalid_credentials = || {
			ProxyError::BasicAuthenticationFailure(Error::InvalidCredentials {
				realm: self.realm.clone().unwrap_or_else(default_realm),
			})
		};
		let (username, password) = base64::engine::general_purpose::STANDARD
			.decode(encoded_credentials.as_ref())
			.ok()
			.and_then(|decoded| String::from_utf8(decoded).ok())
			.and_then(|decoded| {
				decoded
					.split_once(':')
					.map(|(username, password)| (username.to_owned(), password.to_owned()))
			})
			.ok_or_else(invalid_credentials)?;

		// Verify credentials
		let valid = self.htpasswd.check(&username, &password);

		if valid {
			// Authentication successful
			dtrace::pol_result!(
				dtrace::Info,
				Apply,
				"authenticated request as basic auth user {username}"
			);
			Ok(Some(Claims {
				username: username.into(),
			}))
		} else {
			dtrace::pol_result!(
				dtrace::Error,
				Apply,
				"rejected request because basic auth credentials are invalid"
			);
			Err(invalid_credentials())
		}
	}
}

impl crate::store::RequestPolicyTrait for BasicAuthentication {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<crate::http::PolicyResponse, ProxyResponse> {
		let res = self.verify(req).await.map_err(ProxyResponse::from)?;
		if let Some(claims) = res {
			self
				.authorization_location
				.remove(req)
				.map_err(ProxyResponse::from)?;
			// Insert the claims into extensions so we can reference it later
			req.extensions_mut().insert(claims);
		}
		Ok(crate::http::PolicyResponse::default())
	}
}

impl std::fmt::Debug for BasicAuthentication {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BasicAuthentication")
			.field("htpasswd", &"<redacted>")
			.field("realm", &self.realm)
			.field("mode", &self.mode)
			.field("authorization_location", &self.authorization_location)
			.finish()
	}
}
#[apply(schema_de!)]
pub struct LocalBasicAuth {
	/// .htpasswd file contents/reference
	pub htpasswd: FileOrInline,

	/// Realm name for the WWW-Authenticate header
	#[serde(default)]
	pub realm: Option<String>,

	/// Validation mode for basic authentication
	#[serde(default)]
	pub mode: Mode,

	#[serde(default = "default_authorization_location")]
	pub authorization_location: AuthorizationLocation,
}

impl LocalBasicAuth {
	pub fn try_into(self) -> anyhow::Result<BasicAuthentication> {
		Ok(BasicAuthentication::new(
			&self.htpasswd.load()?,
			self.realm,
			self.mode,
			self.authorization_location,
		))
	}
}

fn default_authorization_location() -> AuthorizationLocation {
	AuthorizationLocation::basic_header()
}
