pub mod filters;
pub mod health;
pub mod timeout;

pub mod buffer;
pub mod bufferbody;
pub use agent_http::buflist;
pub mod cors;
pub mod delay;
pub mod jwt;
pub mod localratelimit;
pub mod retry;
pub mod route;

pub mod apikey;
pub mod auth;
pub mod authorization;
pub mod backendtls;
pub mod basicauth;
pub mod compression;
pub mod csrf;
pub mod envoy_proto_common;
pub mod ext_authz;
pub mod ext_proc;
pub(crate) mod oauth;
pub mod oidc;
pub mod outlierdetection;
mod recordbody;
pub mod remoteratelimit;
pub mod sessionpersistence;
pub mod tests_common;
pub mod transformation_cel;

pub use agent_http::{
	Authority, Body, BodyInspection, BufferLimit, BufferedBody, Error, HeaderMap,
	HeaderMutationAction, HeaderName, HeaderOrPseudo, HeaderOrPseudoValue, HeaderValue, Method,
	PolicyResponse, Request, RequestOrResponse, Response, Scheme, StatusCode, Uri, buffer_limit,
	get_path_and_query, get_pseudo_header_value, get_pseudo_or_header_value,
	get_request_pseudo_headers, inspect_body, inspect_body_with_limit, inspect_response_body,
	merge_in_headers, modify_query_parameters, modify_req, modify_req_uri, modify_uri,
	read_body_with_limit, read_req_body, read_resp_body, read_response_body, response_buffer_limit,
	version_str, x_headers,
};
pub use recordbody::{RecordedBody, RecordedBodyHandle};

pub(crate) fn iter_request_cookies<'a>(
	req: &'a Request,
) -> impl Iterator<Item = cookie::Cookie<'a>> + 'a {
	req
		.headers()
		.get_all(header::COOKIE)
		.into_iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(move |header_value| {
			cookie::Cookie::split_parse(Cow::Borrowed(header_value)).filter_map(Result::ok)
		})
}

pub(crate) fn read_request_cookie<'a>(req: &'a Request, name: &str) -> Option<Cow<'a, str>> {
	for cookie in iter_request_cookies(req) {
		if cookie.name() == name {
			return Some(Cow::Owned(cookie.value().to_owned()));
		}
	}
	None
}

pub(crate) fn strip_request_cookies_by_prefix(req: &mut Request, prefix: &str) {
	let preserved: Vec<String> = iter_request_cookies(req)
		.filter(|cookie| !cookie.name().starts_with(prefix))
		.map(|cookie| cookie.to_string())
		.collect();

	req.headers_mut().remove(header::COOKIE);
	if !preserved.is_empty() {
		let hv =
			HeaderValue::from_str(&preserved.join("; ")).expect("re-joined cookie header is valid");
		req.headers_mut().insert(header::COOKIE, hv);
	}
}

// SendDirectResponse is a Response that has been buffered so that it is Send.
pub struct SendDirectResponse(pub ::http::Response<Bytes>);

impl Debug for SendDirectResponse {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SendDirectResponse")
			.field("status", &self.0.status())
			.finish()
	}
}

impl Deref for SendDirectResponse {
	type Target = ::http::Response<Bytes>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl SendDirectResponse {
	pub async fn new(response: Response) -> Result<Self, Error> {
		let (head, bytes) = read_response_body(response).await?;
		Ok(SendDirectResponse(::http::Response::from_parts(
			head, bytes,
		)))
	}
}

use std::borrow::Cow;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::pin::Pin;
use std::task::{Context, Poll};

pub use ::http::{header, status, uri};
use axum_core::BoxError;
use bytes::Bytes;
use http_body::{Frame, SizeHint};
use tower_serve_static::private::mime;
use url::Url;

use crate::cel::{BackendContext, DestinationContext, LLMContext, RequestTime, SourceContext};
use crate::client::PoolKey;
use crate::proxy::{ProxyError, ProxyResponse};
use crate::transport::stream::TCPConnectionInfo;
use crate::types::agent::{HeaderValueMatch, PathMatch};

/// Match repeated header fields independently without splitting commas within a field value.
pub(crate) fn request_header_matches(
	name: &HeaderOrPseudo,
	value: &HeaderValueMatch,
	req: &Request,
) -> bool {
	match name {
		HeaderOrPseudo::Header(name) => req
			.headers()
			.get_all(name)
			.iter()
			.any(|have| value.matches(have)),
		_ => get_pseudo_or_header_value(name, req).is_some_and(|have| value.matches(have.as_ref())),
	}
}

pub fn as_url(uri: &Uri) -> anyhow::Result<Url> {
	Ok(Url::parse(&uri.to_string())?)
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

pub fn is_grpc_request<B>(req: &::http::Request<B>) -> bool {
	!req.uri().path().is_empty() && is_grpc_content_type(req.headers())
}

pub fn is_grpc_content_type(headers: &HeaderMap) -> bool {
	let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
		return false;
	};
	let Ok(content_type) = content_type.to_str() else {
		return false;
	};
	let content_type = content_type.split(';').next().unwrap_or_default().trim();
	content_type.eq_ignore_ascii_case("application/grpc")
		|| content_type
			.get(..17)
			.is_some_and(|prefix| prefix.eq_ignore_ascii_case("application/grpc+"))
}

pub fn get_host(req: &Request) -> Result<&str, ProxyError> {
	// We expect a normalized request, so this will always be in the URI
	// TODO: handle absolute HTTP/1.1 form
	let host = req.uri().host().ok_or(ProxyError::InvalidRequest)?;
	Ok(host)
}

pub fn get_host_with_port(req: &Request) -> Result<&str, ProxyError> {
	// We expect a normalized request, so this will always be in the URI
	// TODO: handle absolute HTTP/1.1 form
	let host = req
		.uri()
		.authority()
		.map(|a| a.as_str())
		.ok_or(ProxyError::InvalidRequest)?;
	Ok(host)
}

pub trait PolicyResponseExt {
	fn apply(self, hm: &mut HeaderMap) -> Result<(), ProxyResponse>;
}

impl PolicyResponseExt for PolicyResponse {
	fn apply(self, hm: &mut HeaderMap) -> Result<(), ProxyResponse> {
		if let Some(mut dr) = self.direct_response {
			merge_in_headers(self.response_headers, dr.headers_mut());
			Err(ProxyResponse::DirectResponse(Box::new(dr)))
		} else {
			merge_in_headers(self.response_headers, hm);
			Ok(())
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

impl<B, D> DropBody<B, D>
where
	D: Send + 'static,
	B: http_body::Body<Data = Bytes> + Send + Unpin + 'static,
	B::Error: Into<BoxError>,
{
	#[allow(clippy::new_ret_no_self)]
	pub fn new(body: B, dropper: D) -> Body {
		Body::new(Self { body, dropper })
	}
}

impl<B: http_body::Body + Unpin, D> http_body::Body for DropBody<B, D> {
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

// DebugExtensions is a wrapper that logs a requests known-extensions in the Debug implementation.
// Note: there is no compile time guarantees we did not miss a given extension.
pub struct DebugExtensions<'a>(pub &'a Request);

impl Debug for DebugExtensions<'_> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let mut d = f.debug_struct("Extensions");
		let ext = self.0.extensions();
		if let Some(e) = ext.get::<jwt::Claims>() {
			d.field("jwt::Claims", e);
		}
		if let Some(e) = ext.get::<apikey::Claims>() {
			d.field("apikey::Claims", e);
		}
		if let Some(e) = ext.get::<basicauth::Claims>() {
			d.field("basicauth::Claims", e);
		}
		if let Some(e) = ext.get::<crate::http::filters::BackendRequestTimeout>() {
			d.field("BackendRequestTimeout", e);
		}
		if let Some(e) = ext.get::<crate::http::filters::OriginalUrl>() {
			d.field("OriginalUrl", e);
		}
		if let Some(e) = ext.get::<crate::http::filters::AutoHostname>() {
			d.field("AutoHostname", e);
		}
		if let Some(e) = ext.get::<crate::llm::bedrock::AwsRegion>() {
			d.field("AwsRegion", e);
		}
		if let Some(e) = ext.get::<crate::client::ResolvedDestination>() {
			d.field("ResolvedDestination", e);
		}
		if let Some(e) = ext.get::<crate::http::ext_authz::ExtAuthzDynamicMetadata>() {
			d.field("ExtAuthzDynamicMetadata", e);
		}
		if let Some(e) = ext.get::<PathMatch>() {
			d.field("PathMatch", e);
		}
		if let Some(e) = ext.get::<crate::telemetry::trc::TraceParent>() {
			d.field("TraceParent", e);
		}
		if let Some(e) = ext.get::<crate::transport::stream::TLSConnectionInfo>() {
			d.field("TLSConnectionInfo", e);
		}
		if let Some(e) = ext.get::<TCPConnectionInfo>() {
			d.field("TCPConnectionInfo", e);
		}
		if let Some(e) = ext.get::<crate::transport::stream::HBONEConnectionInfo>() {
			d.field("HBONEConnectionInfo", e);
		}
		if let Some(e) = ext.get::<BufferLimit>() {
			d.field("BufferLimit", e);
		}
		if let Some(e) = ext.get::<PoolKey>() {
			d.field("PoolKey", e);
		}
		if let Some(e) = ext.get::<LLMContext>() {
			d.field("LLMContext", e);
		}
		if let Some(e) = ext.get::<BackendContext>() {
			d.field("BackendContext", e);
		}
		if let Some(e) = ext.get::<SourceContext>() {
			d.field("SourceContext", e);
		}
		if let Some(e) = ext.get::<DestinationContext>() {
			d.field("DestinationContext", e);
		}
		if let Some(e) = ext.get::<RequestTime>() {
			d.field("RequestTime", e);
		}
		if let Some(e) = ext.get::<transformation_cel::TransformationMetadata>() {
			d.field("TransformationMetadata", e);
		}
		d.finish()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_modify_query_parameters_for_relative_uri() {
		let mut uri = "/resource?keep=1&set=old&set=older&remove=gone"
			.parse()
			.unwrap();

		modify_query_parameters(
			&mut uri,
			[("set", "updated"), ("new", "added value")],
			["remove"],
		)
		.unwrap();

		assert_eq!(
			uri.to_string(),
			"/resource?keep=1&set=updated&new=added+value"
		);
	}

	#[test]
	fn test_modify_query_parameters_for_absolute_uri() {
		let mut uri = "https://example.com/resource?remove=1".parse().unwrap();

		modify_query_parameters(&mut uri, std::iter::empty::<(&str, &str)>(), ["remove"]).unwrap();

		assert_eq!(uri.to_string(), "https://example.com/resource");
	}

	#[test]
	fn detects_grpc_request_content_types() {
		for content_type in [
			"application/grpc",
			"application/grpc+proto",
			"application/grpc; charset=utf-8",
		] {
			let req = ::http::Request::builder()
				.uri("/svc.Method/Call")
				.header(header::CONTENT_TYPE, content_type)
				.body(Body::empty())
				.unwrap();

			assert!(is_grpc_request(&req), "{content_type}");
		}
	}

	#[test]
	fn rejects_non_grpc_request_content_types() {
		for content_type in ["application/json", "application/grpc-web"] {
			let req = ::http::Request::builder()
				.uri("/svc.Method/Call")
				.header(header::CONTENT_TYPE, content_type)
				.body(Body::empty())
				.unwrap();

			assert!(!is_grpc_request(&req), "{content_type}");
		}
	}
}
