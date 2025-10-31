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

#[apply(schema_ser!)]
pub struct BasicAuthentication {
	/// Path to .htpasswd file containing user credentials
	pub htpasswd_file: PathBuf,
	
	/// Realm name for the WWW-Authenticate header
	#[serde(default = "default_realm")]
	pub realm: String,
	
	/// Cached htpasswd data
	#[serde(skip)]
	pub htpasswd: Option<Htpasswd>,
}

fn default_realm() -> String {
	"Restricted".to_string()
}

impl BasicAuthentication {
	/// Create a new BasicAuthentication from a file path
	pub fn new(htpasswd_file: PathBuf, realm: Option<String>) -> Result<Self, BasicAuthError> {
		let content = std::fs::read_to_string(&htpasswd_file)
			.map_err(|e| BasicAuthError::FileLoadError(e.to_string()))?;
		
		let htpasswd = Htpasswd::new(&content);
		
		Ok(Self {
			htpasswd_file,
			realm: realm.unwrap_or_else(default_realm),
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
			// No credentials provided, return 401 with WWW-Authenticate header
			return Err(ProxyError::BasicAuthenticationFailure(BasicAuthError::Missing));
		};
		
		let username = basic.username();
		let password = basic.password();
		
		// Verify credentials
		let htpasswd = self.htpasswd.as_ref().unwrap();
		if htpasswd.check(username, password) {
			// Authentication successful
			Ok(PolicyResponse::default())
		} else {
			// Invalid credentials
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
			htpasswd: None, // Don't clone the parsed htpasswd, will be loaded on demand
		}
	}
}

impl std::fmt::Debug for BasicAuthentication {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BasicAuthentication")
			.field("htpasswd_file", &self.htpasswd_file)
			.field("realm", &self.realm)
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
}

impl LocalBasicAuth {
	pub fn try_into(self) -> Result<BasicAuthentication, BasicAuthError> {
		BasicAuthentication::new(self.htpasswd_file, self.realm)
	}
}
