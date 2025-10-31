use std::collections::HashMap;
use std::path::PathBuf;

use axum_core::RequestExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Basic;
use htpasswd_verify::Htpasswd;
use macro_rules_attribute::apply;

use crate::http::{Request, PolicyResponse};
use crate::proxy::ProxyError;
use crate::telemetry::log::RequestLog;
use crate::*;

#[cfg(test)]
#[path = "basicauth_tests.rs"]
mod tests;

#[derive(thiserror::Error, Debug)]
pub enum BasicAuthError {
	#[error("no basic authentication credentials found")]
	Missing,
	
	#[error("invalid credentials")]
	InvalidCredentials,
	
	#[error("failed to load htpasswd file: {0}")]
	FileLoadError(String),
	
	#[error("failed to parse htpasswd file: {0}")]
	ParseError(String),
}

/// Validation mode for basic authentication
#[apply(schema_ser!)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
	/// A valid username/password must be present.
	Strict,
	/// If credentials exist, validate them.
	/// This is the default option.
	/// Warning: this allows requests without credentials!
	#[default]
	Optional,
	/// Requests are never rejected. This is useful for usage in later steps (authorization, logging, etc).
	/// Warning: this allows requests without credentials and accepts invalid credentials!
	Permissive,
}

#[apply(schema_ser!)]
pub struct BasicAuthentication {
	/// Path to .htpasswd file containing user credentials
	pub htpasswd_file: PathBuf,
	
	/// Realm name for the WWW-Authenticate header
	#[serde(default = "default_realm")]
	pub realm: String,
	
	/// Validation mode for basic authentication
	#[serde(default)]
	pub mode: Mode,
	
	/// Cached htpasswd data
	#[serde(skip)]
	pub htpasswd: Option<Htpasswd>,
}

fn default_realm() -> String {
	"Restricted".to_string()
}

impl BasicAuthentication {
	/// Create a new BasicAuthentication from a file path
	pub fn new(htpasswd_file: PathBuf, realm: Option<String>, mode: Mode) -> Result<Self, BasicAuthError> {
		let content = std::fs::read_to_string(&htpasswd_file)
			.map_err(|e| BasicAuthError::FileLoadError(e.to_string()))?;
		
		let htpasswd = Htpasswd::new(&content);
		
		Ok(Self {
			htpasswd_file,
			realm: realm.unwrap_or_else(default_realm),
			mode,
			htpasswd: Some(htpasswd),
		})
	}
	
	/// Load htpasswd from file if not already loaded
	fn ensure_loaded(&mut self) -> Result<(), BasicAuthError> {
		if self.htpasswd.is_none() {
			let content = std::fs::read_to_string(&self.htpasswd_file)
				.map_err(|e| BasicAuthError::FileLoadError(e.to_string()))?;
			self.htpasswd = Some(Htpasswd::new(&content));
		}
		Ok(())
	}
	
	/// Apply basic authentication to a request
	pub async fn apply(&mut self, _log: &mut RequestLog, req: &mut Request) -> Result<PolicyResponse, ProxyError> {
		// Ensure htpasswd is loaded
		self.ensure_loaded()
			.map_err(|e| ProxyError::BasicAuthenticationFailure(e))?;
		
		// Extract Basic authorization header
		let Ok(TypedHeader(Authorization(basic))) = req
			.extract_parts::<TypedHeader<Authorization<Basic>>>()
			.await
		else {
			// In strict mode, we require credentials
			if self.mode == Mode::Strict {
				return Err(ProxyError::BasicAuthenticationFailure(BasicAuthError::Missing));
			}
			// Otherwise without credentials, don't attempt to authenticate
			return Ok(PolicyResponse::default());
		};
		
		let username = basic.username();
		let password = basic.password();
		
		// Verify credentials
		let htpasswd = self.htpasswd.as_ref().unwrap();
		let valid = htpasswd.check(username, password);
		
		if valid {
			// Authentication successful
			Ok(PolicyResponse::default())
		} else {
			// Invalid credentials
			if self.mode == Mode::Permissive {
				debug!("basic auth verification failed, continue due to permissive mode");
				return Ok(PolicyResponse::default());
			}
			Err(ProxyError::BasicAuthenticationFailure(BasicAuthError::InvalidCredentials))
		}
	}
	
	/// Get the realm for the WWW-Authenticate header
	pub fn realm(&self) -> &str {
		&self.realm
	}
}

impl Clone for BasicAuthentication {
	fn clone(&self) -> Self {
		Self {
			htpasswd_file: self.htpasswd_file.clone(),
			realm: self.realm.clone(),
			mode: self.mode,
			htpasswd: None, // Don't clone the parsed htpasswd, will be loaded on demand
		}
	}
}

impl std::fmt::Debug for BasicAuthentication {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BasicAuthentication")
			.field("htpasswd_file", &self.htpasswd_file)
			.field("realm", &self.realm)
			.field("mode", &self.mode)
			.finish()
	}
}

#[apply(schema_de!)]
pub struct LocalBasicAuth {
	/// Path to .htpasswd file
	pub htpasswd_file: PathBuf,
	
	/// Realm name for the WWW-Authenticate header
	#[serde(default)]
	pub realm: Option<String>,
	
	/// Validation mode for basic authentication
	#[serde(default)]
	pub mode: Mode,
}

impl LocalBasicAuth {
	pub fn try_into(self) -> Result<BasicAuthentication, BasicAuthError> {
		BasicAuthentication::new(self.htpasswd_file, self.realm, self.mode)
	}
}
