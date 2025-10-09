pub mod filters;
pub mod timeout;

mod buflist;
pub mod cors;
pub mod jwt;
pub mod localratelimit;
pub mod retry;
pub mod route;

pub mod auth;
pub mod authorization;
pub mod backendtls;
pub mod compression;
pub mod csrf;
pub mod ext_authz;
pub mod ext_proc;
pub mod outlierdetection;
mod peekbody;
pub mod remoteratelimit;
#[cfg(any(test, feature = "internal_benches"))]
pub mod tests_common;
pub mod transformation_cel;

pub type Error = axum_core::Error;
pub type Body = axum_core::body::Body;
pub type Request = ::http::Request<Body>;
pub type Response = ::http::Response<Body>;

/// A mutable handle that can represent either a request or a response
#[derive(Debug)]
pub enum RequestOrResponse<'a> {
	Request(&'a mut Request),
	Response(&'a mut Response),
}

use std::fmt::Debug;
use std::pin::Pin;
use std::task::{Context, Poll};

pub use ::http::uri::{Authority, Scheme};
pub use ::http::{
	HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header, status, uri,
};
use bytes::Bytes;
use http_body::{Frame, SizeHint};
use tower_serve_static::private::mime;
use url::Url;

use crate::proxy::{ProxyError, ProxyResponse};
use crate::transport::BufferLimit;

use serde::{Serialize, Serializer};

/// Represents either an HTTP header or an HTTP/2 pseudo-header
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderOrPseudo {
	Header(HeaderName),
	Method,
	Scheme,
	Authority,
	Path,
	Status,
}

impl TryFrom<&str> for HeaderOrPseudo {
	type Error = ::http::header::InvalidHeaderName;

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		match value {
			":method" => Ok(HeaderOrPseudo::Method),
			":scheme" => Ok(HeaderOrPseudo::Scheme),
			":authority" => Ok(HeaderOrPseudo::Authority),
			":path" => Ok(HeaderOrPseudo::Path),
			":status" => Ok(HeaderOrPseudo::Status),
			_ => HeaderName::try_from(value).map(HeaderOrPseudo::Header),
		}
	}
}

impl Serialize for HeaderOrPseudo {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self {
			HeaderOrPseudo::Header(h) => h.as_str().serialize(serializer),
			HeaderOrPseudo::Method => ":method".serialize(serializer),
			HeaderOrPseudo::Scheme => ":scheme".serialize(serializer),
			HeaderOrPseudo::Authority => ":authority".serialize(serializer),
			HeaderOrPseudo::Path => ":path".serialize(serializer),
			HeaderOrPseudo::Status => ":status".serialize(serializer),
		}
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for HeaderOrPseudo {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"HeaderOrPseudo".into()
	}

	fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({ "type": "string" })
	}
}

impl std::fmt::Display for HeaderOrPseudo {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			HeaderOrPseudo::Header(h) => write!(f, "{}", h.as_str()),
			HeaderOrPseudo::Method => write!(f, ":method"),
			HeaderOrPseudo::Scheme => write!(f, ":scheme"),
			HeaderOrPseudo::Authority => write!(f, ":authority"),
			HeaderOrPseudo::Path => write!(f, ":path"),
			HeaderOrPseudo::Status => write!(f, ":status"),
		}
	}
}

/// Extract the value for a pseudo header from the request
pub fn get_pseudo_header_value(pseudo: &HeaderOrPseudo, req: &Request) -> Option<String> {
	match pseudo {
		HeaderOrPseudo::Method => Some(req.method().to_string()),
		HeaderOrPseudo::Scheme => req.uri().scheme().map(|s| s.to_string()),
		HeaderOrPseudo::Authority => req.uri().authority().map(|a| a.to_string()).or_else(|| {
			req
				.headers()
				.get("host")
				.and_then(|h| h.to_str().ok().map(|s| s.to_string()))
		}),
		HeaderOrPseudo::Path => req
			.uri()
			.path_and_query()
			.map(|pq| pq.to_string())
			.or_else(|| Some(req.uri().path().to_string())),
		HeaderOrPseudo::Status => None,    // no status for requests
		HeaderOrPseudo::Header(_) => None, // skip regular headers
	}
}

/// Return all present request pseudo headers without introducing defaults
pub fn get_request_pseudo_headers(req: &Request) -> Vec<(HeaderOrPseudo, String)> {
	let mut out = Vec::with_capacity(4);
	if let Some(v) = get_pseudo_header_value(&HeaderOrPseudo::Method, req) {
		out.push((HeaderOrPseudo::Method, v));
	}
	if let Some(v) = get_pseudo_header_value(&HeaderOrPseudo::Scheme, req) {
		out.push((HeaderOrPseudo::Scheme, v));
	}
	if let Some(v) = get_pseudo_header_value(&HeaderOrPseudo::Authority, req) {
		out.push((HeaderOrPseudo::Authority, v));
	}
	if let Some(v) = get_pseudo_header_value(&HeaderOrPseudo::Path, req) {
		out.push((HeaderOrPseudo::Path, v));
	}
	out
}

/// Apply a pseudo header mutation to either a request or a response. Returns true if applied.
pub fn apply_pseudo(rr: &mut RequestOrResponse, pseudo: &HeaderOrPseudo, raw: &[u8]) -> bool {
	match (rr, pseudo) {
		(RequestOrResponse::Request(req), HeaderOrPseudo::Method) => {
			if let Ok(m) = ::http::Method::from_bytes(raw) {
				*req.method_mut() = m;
				return true;
			}
		},
		(RequestOrResponse::Request(req), HeaderOrPseudo::Scheme) => {
			if let Ok(s) = ::http::uri::Scheme::try_from(raw) {
				let _ = modify_req_uri(req, |uri| {
					uri.scheme = Some(s);
					Ok(())
				});
				return true;
			}
		},
		(RequestOrResponse::Request(req), HeaderOrPseudo::Authority) => {
			if let Ok(a) = ::http::uri::Authority::try_from(raw) {
				let _ = modify_req_uri(req, |uri| {
					uri.authority = Some(a);
					Ok(())
				});
				return true;
			}
		},
		(RequestOrResponse::Request(req), HeaderOrPseudo::Path) => {
			if let Ok(pq) = ::http::uri::PathAndQuery::try_from(raw) {
				let _ = modify_req_uri(req, |uri| {
					uri.path_and_query = Some(pq);
					Ok(())
				});
				return true;
			}
		},
		(RequestOrResponse::Response(resp), HeaderOrPseudo::Status) => {
			if let Some(code) = std::str::from_utf8(raw)
				.ok()
				.and_then(|s| s.parse::<u16>().ok())
				.and_then(|c| ::http::StatusCode::from_u16(c).ok())
			{
				*resp.status_mut() = code;
				return true;
			}
		},
		// Non-applicable combinations or regular headers
		_ => {},
	}
	false
}

pub mod x_headers {
	use http::HeaderName;

	pub const X_RATELIMIT_LIMIT: HeaderName = HeaderName::from_static("x-ratelimit-limit");
	pub const X_RATELIMIT_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");
	pub const X_RATELIMIT_RESET: HeaderName = HeaderName::from_static("x-ratelimit-reset");
	pub const X_AMZN_REQUESTID: HeaderName = HeaderName::from_static("x-amzn-requestid");

	pub const RETRY_AFTER_MS: HeaderName = HeaderName::from_static("retry-after-ms");

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

pub fn modify_url(
	uri: &mut Uri,
	f: impl FnOnce(&mut Url) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
	fn url_to_uri(url: &Url) -> anyhow::Result<Uri> {
		if !url.has_authority() {
			anyhow::bail!("no authority");
		}
		if !url.has_host() {
			anyhow::bail!("no host");
		}

		let scheme = url.scheme();
		let authority = url.authority();

		let authority_end = scheme.len() + "://".len() + authority.len();
		let path_and_query = &url.as_str()[authority_end..];

		Ok(
			Uri::builder()
				.scheme(scheme)
				.authority(authority)
				.path_and_query(path_and_query)
				.build()?,
		)
	}
	fn uri_to_url(uri: &Uri) -> anyhow::Result<Url> {
		Ok(Url::parse(&uri.to_string())?)
	}
	let mut url = uri_to_url(uri)?;
	f(&mut url)?;
	*uri = url_to_uri(&url)?;
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

pub fn buffer_limit(req: &Request) -> usize {
	req
		.extensions()
		.get::<BufferLimit>()
		.map(|b| b.0)
		.unwrap_or(2_097_152)
}

pub fn response_buffer_limit(resp: &Response) -> usize {
	resp
		.extensions()
		.get::<BufferLimit>()
		.map(|b| b.0)
		.unwrap_or(2_097_152)
}

pub async fn read_body(req: Request) -> Result<Bytes, axum_core::Error> {
	let lim = buffer_limit(&req);
	read_body_with_limit(req.into_body(), lim).await
}

pub async fn read_body_with_limit(body: Body, limit: usize) -> Result<Bytes, axum_core::Error> {
	axum::body::to_bytes(body, limit).await
}

pub async fn inspect_body(req: &mut Request) -> anyhow::Result<Bytes> {
	let lim = buffer_limit(req);
	inspect_body_with_limit(req.body_mut(), lim).await
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

pin_project_lite::pin_project! {
	/// DropBody is simply a Body wrapper that holds onto another item such that it is dropped when the body
	/// is complete.
	#[derive(Debug)]
	pub struct DropBody<B, D> {
		#[pin]
		body: B,
		dropper: D,
	}
}

impl<B, D> DropBody<B, D> {
	pub fn new(body: B, dropper: D) -> Self {
		Self { body, dropper }
	}
}

impl<B: http_body::Body + Debug + Unpin, D> http_body::Body for DropBody<B, D>
where
	B::Data: Debug,
{
	type Data = B::Data;
	type Error = B::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let this = self.project();
		this.body.poll_frame(cx)
	}

	fn is_end_stream(&self) -> bool {
		self.body.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		self.body.size_hint()
	}
}
