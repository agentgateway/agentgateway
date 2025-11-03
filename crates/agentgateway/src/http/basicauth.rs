use axum_core::RequestExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Basic;
use htpasswd_verify::Htpasswd;
use macro_rules_attribute::apply;

use crate::http::Request;
use crate::proxy::ProxyError;
use crate::telemetry::log::RequestLog;
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

#[apply(schema_ser!)]
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
}

fn default_realm() -> String {
	"Restricted".to_string()
}

impl BasicAuthentication {
	/// Create a new BasicAuthentication from a file path
	pub fn new(htpasswd: &str, realm: Option<String>, mode: Mode) -> Self {
		let htpasswd = Htpasswd::new_owned(htpasswd);

		Self {
			htpasswd: Arc::new(htpasswd),
			realm,
			mode,
		}
	}

	/// Apply basic authentication to a request
	pub async fn apply(&self, log: &mut RequestLog, req: &mut Request) -> Result<(), ProxyError> {
		let res = self.verify(req).await?;
		if let Some(claims) = res {
			log.cel.ctx().with_basic_auth(&claims);
			req.headers_mut().remove(http::header::AUTHORIZATION);
			// Insert the claims into extensions so we can reference it later
			req.extensions_mut().insert(claims);
		}
		Ok(())
	}

	async fn verify(&self, req: &mut Request) -> Result<Option<Claims>, ProxyError> {
		// Extract Basic authorization header
		let Ok(TypedHeader(Authorization(basic))) = req
			.extract_parts::<TypedHeader<Authorization<Basic>>>()
			.await
		else {
			// In strict mode, we require credentials
			if self.mode == Mode::Strict {
				return Err(ProxyError::BasicAuthenticationFailure(Error::Missing {
					realm: self.realm.clone().unwrap_or_else(default_realm),
				}));
			}
			// Otherwise without credentials, don't attempt to authenticate
			return Ok(None);
		};

		let username = basic.username();
		let password = basic.password();

		// Verify credentials
		let valid = self.htpasswd.check(username, password);

		if valid {
			// Authentication successful
			Ok(Some(Claims {
				username: username.into(),
			}))
		} else {
			Err(ProxyError::BasicAuthenticationFailure(
				Error::InvalidCredentials {
					realm: self.realm.clone().unwrap_or_else(default_realm),
				},
			))
		}
	}
}

impl std::fmt::Debug for BasicAuthentication {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BasicAuthentication")
			.field("htpasswd", &"<redacted>")
			.field("realm", &self.realm)
			.field("mode", &self.mode)
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
}

impl LocalBasicAuth {
	pub fn try_into(self) -> anyhow::Result<BasicAuthentication> {
		Ok(BasicAuthentication::new(
			&self.htpasswd.load()?,
			self.realm,
			self.mode,
		))
	}
}
