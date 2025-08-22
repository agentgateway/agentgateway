use ::http::header::InvalidHeaderName;
use ::http::response;
use ::http::uri::InvalidUri;

use crate::http::uri::Scheme;
use crate::http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri};
use crate::types::agent::{HostRedirect, PathMatch, PathRedirect, SimpleBackendReference};
use crate::*;

#[cfg(test)]
#[path = "filters_test.rs"]
mod tests;

#[apply(schema!)]
pub struct HeaderModifier {
	#[serde(default, skip_serializing_if = "is_default")]
	#[serde_as(as = "serde_with::Map<_, _>")]
	pub add: Vec<(Strng, Strng)>,
	#[serde(default, skip_serializing_if = "is_default")]
	#[serde_as(as = "serde_with::Map<_, _>")]
	pub set: Vec<(Strng, Strng)>,
	#[serde(default, skip_serializing_if = "is_default")]
	pub remove: Vec<Strng>,
}

impl HeaderModifier {
	pub fn apply(&self, headers: &mut HeaderMap<HeaderValue>) -> Result<(), Error> {
		for (k, v) in &self.add {
			headers.append(HeaderName::from_bytes(k.as_bytes())?, v.parse()?);
		}
		for (k, v) in &self.set {
			headers.insert(HeaderName::from_bytes(k.as_bytes())?, v.parse()?);
		}
		for k in &self.remove {
			headers.remove(HeaderName::from_bytes(k.as_bytes())?);
		}
		Ok(())
	}
}

#[apply(schema!)]
pub struct RequestRedirect {
	#[serde(
		default,
		skip_serializing_if = "is_default",
		serialize_with = "ser_display_option",
		deserialize_with = "de_parse_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub scheme: Option<http::uri::Scheme>,
	#[serde(skip_serializing_if = "is_default")]
	pub authority: Option<HostRedirect>,
	#[serde(skip_serializing_if = "is_default")]
	pub path: Option<PathRedirect>,
	#[serde(
		default,
		skip_serializing_if = "is_default",
		with = "http_serde::option::status_code"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<std::num::NonZeroU16>"))]
	pub status: Option<http::StatusCode>,
}

impl RequestRedirect {
	pub fn apply(&self, req: &mut Request, path_match: &PathMatch) -> Result<Response, Error> {
		let RequestRedirect {
			scheme,
			authority,
			path,
			status,
		} = self;
		let new_scheme = scheme
			.as_ref()
			.or_else(|| req.uri().scheme())
			.cloned()
			.unwrap_or(Scheme::HTTP);
		let authority = rewrite_host(authority, req.uri(), scheme.as_ref(), &new_scheme)?;
		let path_and_query = rewrite_path(path, path_match, req.uri())?;
		let new = Uri::builder()
			.scheme(new_scheme)
			.authority(authority)
			.path_and_query(path_and_query)
			.build()?;
		Ok(
			::http::Response::builder()
				.status(status.unwrap_or(StatusCode::FOUND))
				.header(http::header::LOCATION, new.to_string())
				.body(http::Body::empty())?,
		)
	}
}

#[apply(schema!)]
pub struct UrlRewrite {
	#[serde(skip_serializing_if = "is_default")]
	pub authority: Option<HostRedirect>,
	#[serde(skip_serializing_if = "is_default")]
	pub path: Option<PathRedirect>,
}

#[derive(Debug, Clone)]
pub struct OriginalUrl(pub Uri);
#[derive(Debug, Clone)]
pub struct AutoHostname();

impl UrlRewrite {
	pub fn apply(&self, req: &mut Request, path_match: &PathMatch) -> Result<(), Error> {
		let UrlRewrite { authority, path } = self;
		let orig = req.uri().clone();
		req.extensions_mut().insert(OriginalUrl(orig));
		let scheme = req.uri().scheme().cloned().unwrap_or(Scheme::HTTP);

		let new_authority = rewrite_host(authority, req.uri(), Some(&scheme), &scheme)?;
		if matches!(authority, Some(HostRedirect::Auto)) {
			req.extensions_mut().insert(AutoHostname());
		}
		let path_and_query = rewrite_path(path, path_match, req.uri())?;
		let new = Uri::builder()
			.scheme(scheme)
			.authority(new_authority)
			.path_and_query(path_and_query)
			.build()
			.map_err(|e| Error::InvalidFilterConfiguration(e.to_string()))?;
		*req.uri_mut() = new;
		Ok(())
	}
}

#[apply(schema!)]
pub struct DirectResponse {
	pub body: Bytes,
	#[serde(with = "http_serde::status_code")]
	#[cfg_attr(feature = "schema", schemars(with = "std::num::NonZeroU16"))]
	pub status: StatusCode,
}

impl DirectResponse {
	pub fn apply(&self) -> Result<Response, Error> {
		response::Builder::new()
			.status(self.status)
			.body(http::Body::from(self.body.clone()))
			.map_err(Into::into)
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestMirror {
	pub backend: SimpleBackendReference,
	// 0.0-1.0
	pub percentage: f64,
}

fn rewrite_host(
	rewrite: &Option<HostRedirect>,
	orig: &Uri,
	original_scheme: Option<&Scheme>,
	new_scheme: &Scheme,
) -> Result<http::uri::Authority, Error> {
	match &rewrite {
		// For Auto, we need to handle it later after we pick the backend!
		None | Some(HostRedirect::Auto) => orig.authority().cloned().ok_or(Error::InvalidURI),
		Some(HostRedirect::Full(hp)) => Ok(hp.as_str().try_into()?),
		Some(HostRedirect::Host(h)) => {
			if original_scheme == Some(&Scheme::HTTP) || original_scheme == Some(&Scheme::HTTPS) {
				Ok(h.as_str().try_into()?)
			} else {
				let new_port = orig
					.port_u16()
					.and_then(|p| port_respecting_default(new_scheme, p));
				match new_port {
					Some(p) => Ok(format!("{h}:{p}").try_into()?),
					None => Ok(h.as_str().try_into()?),
				}
			}
		},
		Some(HostRedirect::Port(p)) => {
			match port_respecting_default(new_scheme, p.get()) {
				// We need to set port here
				Some(p) if Some(p) != orig.port_u16() => {
					Ok(format!("{}:{}", orig.host().ok_or(Error::InvalidURI)?, p).try_into()?)
				},

				// Strip the port
				None if orig.port().is_some() => Ok(orig.host().ok_or(Error::InvalidURI)?.parse()?),

				// Keep it as-is
				_ => Ok(orig.authority().ok_or(Error::InvalidURI)?.clone()),
			}
		},
	}
}

fn port_respecting_default(scheme: &http::uri::Scheme, port: u16) -> Option<u16> {
	if *scheme == http::uri::Scheme::HTTP && port == 80 {
		return None;
	}
	if *scheme == http::uri::Scheme::HTTPS && port == 443 {
		return None;
	}
	Some(port)
}

fn rewrite_path(
	rewrite: &Option<PathRedirect>,
	path_match: &PathMatch,
	orig: &http::Uri,
) -> Result<http::uri::PathAndQuery, Error> {
	match rewrite {
		None => Ok(orig.path_and_query().ok_or(Error::InvalidURI).cloned()?),
		Some(PathRedirect::Full(r)) => {
			let mut new_path = r.to_string();
			// Preserve query parameters from the original URI
			if let Some(q) = orig.query() {
				new_path.push('?');
				new_path.push_str(q);
			}
			Ok(new_path.try_into()?)
		},
		Some(PathRedirect::Prefix(r)) => {
			let PathMatch::PathPrefix(match_pfx) = path_match else {
				return Err(Error::InvalidFilterConfiguration(
					"prefix redirect requires prefix match".to_string(),
				));
			};
			let mut new_path = r.to_string();
			let (_, rest) = orig.path().split_at(match_pfx.len());
			if !rest.is_empty() && !rest.starts_with('/') {
				new_path.push('/');
			}
			new_path.push_str(rest);
			if let Some(q) = orig.query() {
				new_path.push('?');
				new_path.push_str(q);
			}
			Ok(new_path.try_into()?)
		},
	}
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("invalid URI")]
	InvalidURI,
	#[error("invalid URI: {0}")]
	InvalidHTTPURI(#[from] InvalidUri),
	#[error("invalid header name: {0}")]
	InvalidHeaderName(#[from] InvalidHeaderName),
	#[error("invalid header value: {0}")]
	InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),
	#[error("invalid filter configuration: {0}")]
	InvalidFilterConfiguration(String),
	#[error("http error: {0}")]
	Http(#[from] ::http::Error),
}
