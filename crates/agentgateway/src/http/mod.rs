pub mod filters;
pub mod timeout;

mod buflist;
pub mod cors;
pub mod jwt;
pub mod localratelimit;
pub mod retry;
pub mod route;

pub mod auth;
#[cfg(any(test, feature = "internal_benches"))]
mod tests_common;
#[allow(dead_code)]
mod transformation;
// Do not warn is it is WIP
pub mod authorization;
pub mod backendtls;
pub mod compression;
pub mod ext_authz;
pub mod ext_proc;
pub mod outlierdetection;
mod peekbody;
pub mod remoteratelimit;
pub mod transformation_cel;

pub type Error = axum_core::Error;
pub type Body = axum_core::body::Body;
pub type Request = ::http::Request<Body>;
pub type Response = ::http::Response<Body>;
pub use ::http::uri::{Authority, Scheme};
pub use ::http::{
	HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header, status, uri,
};
use axum::body::to_bytes;
use bytes::Bytes;
use tower_serve_static::private::mime;

use crate::proxy::{ProxyError, ProxyResponse};

pub mod x_headers {
	use http::HeaderName;

	pub const X_RATELIMIT_LIMIT: HeaderName = HeaderName::from_static("x-ratelimit-limit");
	pub const X_RATELIMIT_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");
	pub const X_RATELIMIT_RESET: HeaderName = HeaderName::from_static("x-ratelimit-reset");
	pub const X_AMZN_REQUESTID: HeaderName = HeaderName::from_static("x-amzn-requestid");

	pub const X_RATELIMIT_RESET_REQUESTS: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-requests");
	pub const X_RATELIMIT_RESET_TOKENS: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-tokens");
	pub const X_RATELIMIT_RESET_REQUESTS_DAY: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-requests-day");
	pub const X_RATELIMIT_RESET_TOKENS_MINUTE: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-tokens-minute");
}

pub fn modify_req(
	req: &mut Request,
	f: impl FnOnce(&mut ::http::request::Parts) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
	let nreq = std::mem::take(req);
	let (mut head, body) = nreq.into_parts();
	f(&mut head)?;
	*req = Request::from_parts(head, body);
	Ok(())
}

pub fn modify_req_uri(
	req: &mut Request,
	f: impl FnOnce(&mut uri::Parts) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
	let nreq = std::mem::take(req);
	let (mut head, body) = nreq.into_parts();
	let mut parts = head.uri.into_parts();
	f(&mut parts)?;
	head.uri = Uri::from_parts(parts)?;
	*req = Request::from_parts(head, body);
	Ok(())
}

pub fn modify_uri(
	head: &mut http::request::Parts,
	f: impl FnOnce(&mut uri::Parts) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
	let nreq = std::mem::take(&mut head.uri);

	let mut parts = nreq.into_parts();
	f(&mut parts)?;
	head.uri = Uri::from_parts(parts)?;
	Ok(())
}

#[derive(Debug)]
pub enum WellKnownContentTypes {
	Json,
	Sse,
	Unknown,
}

pub fn classify_content_type(h: &HeaderMap) -> WellKnownContentTypes {
	if let Some(content_type) = h.get(header::CONTENT_TYPE)
		&& let Ok(content_type_str) = content_type.to_str()
		&& let Ok(mime) = content_type_str.parse::<mime::Mime>()
	{
		match (mime.type_(), mime.subtype()) {
			(mime::APPLICATION, mime::JSON) => return WellKnownContentTypes::Json,
			(mime::TEXT, mime::EVENT_STREAM) => {
				return WellKnownContentTypes::Sse;
			},
			_ => {},
		}
	}
	WellKnownContentTypes::Unknown
}

pub fn get_host(req: &Request) -> Result<&str, ProxyError> {
	// We expect a normalized request, so this will always be in the URI
	// TODO: handle absolute HTTP/1.1 form
	let host = req.uri().host().ok_or(ProxyError::InvalidRequest)?;
	let host = strip_port(host);
	Ok(host)
}

pub async fn inspect_body(body: &mut Body) -> anyhow::Result<Bytes> {
	inspect_body_with_limit(body, 2_097_152).await
}

pub async fn inspect_body_with_limit(body: &mut Body, limit: usize) -> anyhow::Result<Bytes> {
	peekbody::inspect_body(body, limit).await
}

// copied from private `http` method
fn strip_port(auth: &str) -> &str {
	let host_port = auth
		.rsplit('@')
		.next()
		.expect("split always has at least 1 item");

	if host_port.as_bytes()[0] == b'[' {
		let i = host_port
			.find(']')
			.expect("parsing should validate brackets");
		// ..= ranges aren't available in 1.20, our minimum Rust version...
		&host_port[0..i + 1]
	} else {
		host_port
			.split(':')
			.next()
			.expect("split always has at least 1 item")
	}
}

#[derive(Debug, Default)]
#[must_use]
pub struct PolicyResponse {
	pub direct_response: Option<Response>,
	pub response_headers: Option<crate::http::HeaderMap>,
}

impl PolicyResponse {
	pub fn apply(self, hm: &mut HeaderMap) -> Result<(), ProxyResponse> {
		if let Some(mut dr) = self.direct_response {
			merge_in_headers(self.response_headers, dr.headers_mut());
			Err(ProxyResponse::DirectResponse(Box::new(dr)))
		} else {
			merge_in_headers(self.response_headers, hm);
			Ok(())
		}
	}
	pub fn should_short_circuit(&self) -> bool {
		self.direct_response.is_some()
	}
	pub fn with_response(self, other: Response) -> Self {
		PolicyResponse {
			direct_response: Some(other),
			response_headers: self.response_headers,
		}
	}
	pub fn merge(self, other: Self) -> Self {
		if other.direct_response.is_some() {
			other
		} else {
			match (self.response_headers, other.response_headers) {
				(None, None) => PolicyResponse::default(),
				(a, b) => PolicyResponse {
					direct_response: None,
					response_headers: Some({
						let mut hm = HeaderMap::new();
						merge_in_headers(a, &mut hm);
						merge_in_headers(b, &mut hm);
						hm
					}),
				},
			}
		}
	}
}

pub fn merge_in_headers(additional_headers: Option<HeaderMap>, dest: &mut HeaderMap) {
	if let Some(rh) = additional_headers {
		for (k, v) in rh.into_iter() {
			let Some(k) = k else { continue };
			dest.insert(k, v);
		}
	}
}
