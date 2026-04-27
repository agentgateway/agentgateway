pub mod aws;
pub mod azure;
pub mod gcp;

use std::borrow::Cow;

use ::http::HeaderValue;
pub use aws::AwsAuth;
pub use azure::AzureAuth;
use cookie::Cookie;
pub use gcp::GcpAuth;
use secrecy::{ExposeSecret, SecretString};
use url::form_urlencoded;

use crate::http::Request;
use crate::http::jwt::Claims;
use crate::proxy::ProxyError;
use crate::proxy::ProxyError::ProcessingString;
use crate::serdes::deser_key_from_file;
use crate::types::agent::{BackendTarget, Target};
use crate::*;

#[apply(schema!)]
pub enum BackendAuth {
	Passthrough {
		#[serde(default)]
		location: AuthorizationLocation,
	},
	Key {
		#[cfg_attr(feature = "schema", schemars(with = "FileOrInline"))]
		#[serde(
			serialize_with = "ser_redact",
			deserialize_with = "deser_key_from_file"
		)]
		value: SecretString,
		#[serde(default)]
		location: AuthorizationLocation,
	},
	#[serde(rename = "gcp")]
	Gcp(gcp::GcpAuth),
	#[serde(rename = "aws")]
	Aws(aws::AwsAuth),
	#[serde(rename = "azure")]
	Azure(azure::AzureAuth),
}

#[derive(Clone)]
pub struct BackendInfo {
	pub target: BackendTarget,
	pub call_target: Target,
	pub inputs: Arc<ProxyInputs>,
}

pub fn apply_tunnel_auth(auth: &BackendAuth) -> Result<HeaderValue, ProxyError> {
	match auth {
		BackendAuth::Key {
			value: key,
			location,
		} => match location {
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
	match auth {
		BackendAuth::Passthrough { location } => {
			// They should have a JWT policy defined. That will strip the token. Here we add it back
			// TODO: should we also support API key, etc?
			if let Some(token) = req
				.extensions()
				.get::<Claims>()
				.map(|claim| claim.jwt.expose_secret().to_string())
			{
				location.insert(req, &token)?;
			}
		},
		BackendAuth::Key {
			value: key,
			location,
		} => location.insert(req, key.expose_secret())?,
		BackendAuth::Gcp(g) => {
			gcp::insert_token(g, &backend_info.call_target, req.headers_mut())
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
		},
		BackendAuth::Aws(_) => {
			// We handle this in 'apply_late_backend_auth' since it must come at the end (due to request signing)!
		},
		BackendAuth::Azure(azure_auth) => {
			let token = azure::get_token(
				&backend_info.inputs.upstream,
				azure_auth,
				&backend_info.call_target,
			)
			.await
			.map_err(ProxyError::BackendAuthenticationFailed)?;
			req.headers_mut().insert(http::header::AUTHORIZATION, token);
		},
	}
	Ok(())
}

pub async fn apply_late_backend_auth(
	auth: Option<&BackendAuth>,
	req: &mut Request,
) -> Result<(), ProxyError> {
	let Some(auth) = auth else {
		return Ok(());
	};
	match auth {
		BackendAuth::Passthrough { .. } => {},
		BackendAuth::Key { .. } => {},
		BackendAuth::Gcp(_) => {},
		BackendAuth::Aws(aws_auth) => {
			aws::sign_request(req, aws_auth)
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
		},
		BackendAuth::Azure(_) => {},
	};
	Ok(())
}

#[apply(schema!)]
pub enum AuthorizationLocation {
	Header {
		#[serde(with = "http_serde::header_name")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		name: http::HeaderName,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		prefix: Option<Strng>,
	},
	QueryParameter {
		name: Strng,
	},
	Cookie {
		name: Strng,
	},
}

impl Default for AuthorizationLocation {
	fn default() -> Self {
		Self::bearer_header()
	}
}

impl AuthorizationLocation {
	pub fn bearer_header() -> Self {
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
		}
		Ok(())
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
