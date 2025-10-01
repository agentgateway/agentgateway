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
mod tests_common;
pub mod transformation_cel;

pub type Error = axum_core::Error;
pub type Body = axum_core::body::Body;
pub type Request = ::http::Request<Body>;
pub type Response = ::http::Response<Body>;

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
