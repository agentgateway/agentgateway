pub mod aws;
pub mod azure;
mod copilot;
pub mod gcp;
pub mod oauth;

use std::borrow::Cow;
use std::time::Duration;

use ::http::HeaderValue;
pub use aws::{AwsAssumeRole, AwsAuth};
pub use azure::AzureAuth;
use cookie::Cookie;
pub use gcp::GcpAuth;
pub use oauth::{
	CrossAppAccessAuth, OAuthClientAuth, OAuthClientAuthMethod, OAuthGrantType,
	OAuthTokenExchangeAuth, PrivateKeyJwt,
};
use secrecy::{ExposeSecret, SecretString};
use url::form_urlencoded;

use crate::http::Request;
use crate::http::jwt::Claims;
use crate::proxy::ProxyError;
use crate::proxy::ProxyError::ProcessingString;
use crate::serdes::deser_key_from_file;
use crate::types::agent::{BackendTarget, Target};
use crate::*;

const CLOUD_AUTH_TIMEOUT: Duration = Duration::from_secs(5);

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "BackendAuth"))]
pub enum BackendAuthKind {
	/// Forward the validated incoming JWT to the backend.
	Passthrough {
		/// Where to place the forwarded credential in the backend request.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		location: Option<AuthorizationLocation>,
	},
	/// Send a configured secret value to the backend.
	Key {
		/// Secret value to send to the backend.
		#[cfg_attr(feature = "schema", schemars(with = "FileOrInline"))]
		#[serde(
			serialize_with = "ser_redact",
			deserialize_with = "deser_key_from_file"
		)]
		value: SecretString,
		/// Where to place the secret in the backend request.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		location: Option<AuthorizationLocation>,
	},
	/// Authenticate to Google Cloud services.
	#[serde(rename = "gcp")]
	Gcp(gcp::GcpAuth),
	/// Sign backend requests with AWS credentials.
	#[serde(rename = "aws")]
	Aws(aws::AwsAuth),
	/// Authenticate to Azure services.
	#[serde(rename = "azure")]
	Azure(azure::AzureAuth),
	/// Authenticate to GitHub Copilot.
	#[serde(rename = "copilot")]
	Copilot,
	/// Use OAuth token exchange flows to obtain a backend access token.
	#[serde(rename = "oauthTokenExchange")]
	OAuthTokenExchange(Box<OAuthTokenExchangeAuth>),
	/// Use Cross App Access (Identity Assertion / ID-JAG) to obtain a backend access token.
	#[serde(rename = "crossAppAccess")]
	CrossAppAccess(Box<CrossAppAccessAuth>),
}

/// Backend authentication configuration.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BackendAuth {
	#[serde(flatten, skip_serializing_if = "Option::is_none")]
	pub kind: Option<BackendAuthKind>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub credentials: Vec<BackendAuthCredential>,
}

impl BackendAuth {
	pub fn new(kind: BackendAuthKind) -> Self {
		Self {
			kind: Some(kind),
			credentials: Vec::new(),
		}
	}
}

/// An additional credential to inject on the backend request.
#[apply(schema!)]
pub struct BackendAuthCredential {
	/// Where the credential is inserted on the backend request.
	pub location: AuthorizationLocation,
	/// Credential value.
	#[serde(
		serialize_with = "ser_redact",
		deserialize_with = "deser_key_from_file"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "FileOrInline"))]
	pub key: SecretString,
}

/// Records whether the backend auth location was explicitly configured by the user
/// (vs. defaulted).
///
/// Downstream providers (e.g. Anthropic) inspect this to decide whether to rewrite
/// auth headers.
#[derive(Clone, Debug)]
pub struct AppliedBackendAuthLocation {
	pub explicit: bool,
}

#[derive(Clone)]
pub struct BackendInfo {
	pub target: BackendTarget,
	pub call_target: Target,
	pub inputs: Arc<ProxyInputs>,
}

pub fn apply_tunnel_auth(auth: &BackendAuth) -> Result<HeaderValue, ProxyError> {
	if !auth.credentials.is_empty() {
		return Err(ProcessingString(
			"backendAuth.credentials is not supported on tunnel-bound backends".to_string(),
		));
	}
	match auth.kind.as_ref() {
		Some(BackendAuthKind::Key {
			value: key,
			location,
		}) => {
			let resolved = location.as_ref().unwrap_or(&DEFAULT_AUTHORIZATION_LOCATION);
			match resolved {
				AuthorizationLocation::Header { name: _, prefix } => {
					let value = key.expose_secret();
					let value = match prefix {
						Some(prefix) => Cow::Owned(format!("{prefix}{value}")),
						None => Cow::Borrowed(value),
					};
					let mut header_value =
						HeaderValue::from_str(&value).map_err(|e| ProxyError::Processing(e.into()))?;
					header_value.set_sensitive(true);
					Ok(header_value)
				},
				_ => Err(ProcessingString(
					"only header auth is supported in tunnel".to_string(),
				)),
			}
		},
		_ => Err(ProcessingString(
			"only key auth is supported in tunnel".to_string(),
		)),
	}
}
pub async fn apply_backend_auth(
	backend_info: &BackendInfo,
	auth: &BackendAuth,
	req: &mut Request,
) -> Result<(), ProxyError> {
	if let Some(kind) = auth.kind.as_ref() {
		apply_backend_auth_kind(backend_info, kind, req).await?;
	}
	for credential in &auth.credentials {
		credential
			.location
			.insert(req, credential.key.expose_secret())?;
		// Credential locations are always explicitly configured. Mark Authorization writes
		// so providers (e.g. Anthropic) do not rewrite or relocate the header. Other
		// locations must not touch the marker set by the primary auth kind.
		if matches!(&credential.location, AuthorizationLocation::Header { name, .. } if *name == http::header::AUTHORIZATION)
		{
			req
				.extensions_mut()
				.insert(AppliedBackendAuthLocation { explicit: true });
		}
	}
	Ok(())
}

async fn apply_backend_auth_kind(
	backend_info: &BackendInfo,
	auth: &BackendAuthKind,
	req: &mut Request,
) -> Result<(), ProxyError> {
	match auth {
		BackendAuthKind::Passthrough { location } => {
			let explicit = location.is_some();
			let resolved = location.as_ref().unwrap_or(&DEFAULT_AUTHORIZATION_LOCATION);
			// They should have a JWT policy defined. That will strip the token. Here we add it back
			// TODO: should we also support API key, etc?
			if let Some(token) = req
				.extensions()
				.get::<Claims>()
				.map(|claim| claim.jwt.expose_secret().to_string())
			{
				resolved.insert(req, &token)?;
			}
			req
				.extensions_mut()
				.insert(AppliedBackendAuthLocation { explicit });
		},
		BackendAuthKind::Key {
			value: key,
			location,
		} => {
			let explicit = location.is_some();
			let resolved = location.as_ref().unwrap_or(&DEFAULT_AUTHORIZATION_LOCATION);
			resolved.insert(req, key.expose_secret())?;
			req
				.extensions_mut()
				.insert(AppliedBackendAuthLocation { explicit });
		},
		BackendAuthKind::Gcp(g) => {
			gcp::insert_token(g, &backend_info.call_target, req.headers_mut())
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
		},
		BackendAuthKind::Aws(_) => {
			// We handle this in 'apply_late_backend_auth' since it must come at the end (due to request signing)!
		},
		BackendAuthKind::Azure(azure_auth) => {
			let token = azure::get_token(
				&backend_info.inputs.upstream,
				azure_auth,
				&backend_info.call_target,
			)
			.await
			.map_err(ProxyError::BackendAuthenticationFailed)?;
			req.headers_mut().insert(http::header::AUTHORIZATION, token);
		},
		BackendAuthKind::Copilot => {
			copilot::insert_headers(req)
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
		},
		BackendAuthKind::OAuthTokenExchange(te_auth) => {
			let explicit = oauth::apply_token_exchange(&backend_info.inputs, te_auth, req).await?;
			req
				.extensions_mut()
				.insert(AppliedBackendAuthLocation { explicit });
		},
		BackendAuthKind::CrossAppAccess(auth) => {
			let explicit = oauth::apply_identity_assertion(&backend_info.inputs, auth, req).await?;
			req
				.extensions_mut()
				.insert(AppliedBackendAuthLocation { explicit });
		},
	}
	Ok(())
}

pub async fn apply_late_backend_auth(
	auth: Option<&BackendAuth>,
	req: &mut Request,
) -> Result<(), ProxyError> {
	let Some(BackendAuth {
		kind: Some(BackendAuthKind::Aws(aws_auth)),
		..
	}) = auth
	else {
		return Ok(());
	};

	aws::sign_request(req, aws_auth)
		.await
		.map_err(ProxyError::BackendAuthenticationFailed)
}

#[apply(schema!)]
pub enum AuthorizationLocation {
	/// Read the credential from an HTTP header.
	Header {
		/// Header name containing the credential.
		#[serde(with = "http_serde::header_name")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		name: http::HeaderName,
		/// Prefix to remove from the header value before validation, such as `Bearer ` or `Basic `.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		prefix: Option<Strng>,
	},
	/// Read the credential from a URL query parameter.
	QueryParameter {
		/// Query parameter name containing the credential.
		name: Strng,
	},
	/// Read the credential from a request cookie.
	Cookie {
		/// Cookie name containing the credential.
		name: Strng,
	},
	/// Read the credential from a CEL expression evaluated against the incoming request.
	/// CEL expression that returns the credential string. This location can extract credentials but cannot insert them.
	Expression(Arc<crate::cel::Expression>),
}

impl Default for AuthorizationLocation {
	fn default() -> Self {
		Self::bearer_header()
	}
}

pub static DEFAULT_AUTHORIZATION_LOCATION: AuthorizationLocation =
	AuthorizationLocation::bearer_header();

impl AuthorizationLocation {
	pub const fn bearer_header() -> Self {
		Self::Header {
			name: http::header::AUTHORIZATION,
			prefix: Some(strng::literal!("Bearer ")),
		}
	}

	pub fn basic_header() -> Self {
		Self::Header {
			name: http::header::AUTHORIZATION,
			prefix: Some(strng::literal!("Basic ")),
		}
	}

	pub fn extract<'a>(&self, req: &'a Request) -> Option<Cow<'a, str>> {
		match self {
			AuthorizationLocation::Header { name, prefix } => {
				let value = req.headers().get(name)?.to_str().ok()?;
				match prefix.as_deref() {
					Some(prefix) => strip_prefix_ascii_case_insensitive(value, prefix).map(Cow::Borrowed),
					None => Some(Cow::Borrowed(value)),
				}
			},
			AuthorizationLocation::QueryParameter { name } => query_parameter(req, name),
			AuthorizationLocation::Cookie { name } => crate::http::read_request_cookie(req, name),
			AuthorizationLocation::Expression(expression) => crate::cel::Executor::new_request(req)
				.eval(expression)
				.ok()
				.and_then(|v| v.as_str().ok().map(Cow::into_owned))
				.map(Cow::Owned),
		}
	}

	pub fn remove(&self, req: &mut Request) -> Result<(), ProxyError> {
		match self {
			AuthorizationLocation::Header { name, .. } => {
				req.headers_mut().remove(name);
			},
			AuthorizationLocation::QueryParameter { name } => {
				crate::http::modify_query_parameters(
					req.uri_mut(),
					std::iter::empty::<(&str, &str)>(),
					[name.as_str()],
				)
				.map_err(ProxyError::Processing)?;
			},
			AuthorizationLocation::Cookie { name } => {
				set_request_cookie(req, name, None)?;
			},
			AuthorizationLocation::Expression(_) => {},
		}
		Ok(())
	}

	pub fn insert(&self, req: &mut Request, value: &str) -> Result<(), ProxyError> {
		match self {
			AuthorizationLocation::Header { name, prefix } => {
				let value = match prefix {
					Some(prefix) => Cow::Owned(format!("{prefix}{value}")),
					None => Cow::Borrowed(value),
				};
				let mut header_value =
					HeaderValue::from_str(&value).map_err(|e| ProxyError::Processing(e.into()))?;
				header_value.set_sensitive(true);
				req.headers_mut().insert(name, header_value);
			},
			AuthorizationLocation::QueryParameter { name } => {
				crate::http::modify_query_parameters(
					req.uri_mut(),
					[(name.as_str(), value)],
					std::iter::empty::<&str>(),
				)
				.map_err(ProxyError::Processing)?;
			},
			AuthorizationLocation::Cookie { name } => {
				set_request_cookie(req, name, Some(value))?;
			},
			AuthorizationLocation::Expression(_) => {
				return Err(ProcessingString(
					"expression auth location is only supported for credential extraction".to_string(),
				));
			},
		}
		Ok(())
	}

	pub fn expression(&self) -> Option<&crate::cel::Expression> {
		match self {
			AuthorizationLocation::Expression(expression) => Some(expression),
			_ => None,
		}
	}
}

fn strip_prefix_ascii_case_insensitive<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
	if value.len() < prefix.len() {
		return None;
	}
	let (candidate, remainder) = value.split_at(prefix.len());
	if candidate.eq_ignore_ascii_case(prefix) {
		Some(remainder)
	} else {
		None
	}
}

fn query_parameter<'a>(req: &'a Request, name: &str) -> Option<Cow<'a, str>> {
	for (key, value) in form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes()) {
		if key == name {
			return Some(value);
		}
	}
	None
}

fn set_request_cookie(
	req: &mut Request,
	name: &str,
	value: Option<&str>,
) -> Result<(), ProxyError> {
	let mut preserved: Vec<String> = crate::http::iter_request_cookies(req)
		.filter(|cookie| cookie.name() != name)
		.map(|cookie| cookie.to_string())
		.collect();
	if let Some(value) = value {
		preserved.push(Cookie::new(name.to_string(), value.to_string()).to_string());
	}
	req.headers_mut().remove(http::header::COOKIE);
	if !preserved.is_empty() {
		let mut header_value =
			HeaderValue::from_str(&preserved.join("; ")).map_err(|e| ProxyError::Processing(e.into()))?;
		header_value.set_sensitive(true);
		req.headers_mut().insert(http::header::COOKIE, header_value);
	}
	Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
