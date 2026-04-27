pub mod filters;
pub mod health;
pub mod timeout;

mod buflist;
pub mod cors;
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
mod peekbody;
mod recordbody;
pub mod remoteratelimit;
pub mod sessionpersistence;
#[cfg(any(test, feature = "internal_benches"))]
pub mod tests_common;
pub mod transformation_cel;

pub type Error = axum_core::Error;
pub type Body = axum_core::body::Body;
pub type Request = ::http::Request<Body>;
pub type Response = ::http::Response<Body>;

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

pub fn version_str(v: &http::Version) -> &'static str {
	match *v {
		http::Version::HTTP_09 => "HTTP/0.9",
		http::Version::HTTP_10 => "HTTP/1.0",
		http::Version::HTTP_11 => "HTTP/1.1",
		http::Version::HTTP_2 => "HTTP/2",
		http::Version::HTTP_3 => "HTTP/3",
		_ => "unknown",
	}
}

/// A mutable handle that can represent either a request or a response
#[derive(Debug)]
pub enum RequestOrResponse<'a> {
	Request(&'a mut Request),
	Response(&'a mut Response),
}

impl<'a> From<&'a mut Request> for RequestOrResponse<'a> {
	fn from(req: &'a mut Request) -> Self {
		RequestOrResponse::Request(req)
	}
}

impl<'a> From<&'a mut Response> for RequestOrResponse<'a> {
	fn from(req: &'a mut Response) -> RequestOrResponse<'a> {
		RequestOrResponse::Response(req)
	}
}

impl RequestOrResponse<'_> {
	pub fn headers(&mut self) -> &mut http::HeaderMap {
		match self {
			RequestOrResponse::Request(r) => r.headers_mut(),
			RequestOrResponse::Response(r) => r.headers_mut(),
		}
	}
	pub fn body(&mut self) -> &mut Body {
		match self {
			RequestOrResponse::Request(r) => r.body_mut(),
			RequestOrResponse::Response(r) => r.body_mut(),
		}
	}
	pub fn apply_header(
		&mut self,
		k: &HeaderOrPseudo,
		v: Option<HeaderOrPseudoValue>,
		action: HeaderMutationAction,
	) {
		match (k, v) {
			(HeaderOrPseudo::Header(k), Some(HeaderOrPseudoValue::Header(v))) => {
				// Normalize modification of host header to authority header.
				if k == header::HOST && matches!(self, RequestOrResponse::Request(_)) {
					let Some(value) = HeaderOrPseudoValue::from_raw(&HeaderOrPseudo::Authority, v.as_bytes())
					else {
						return;
					};
					self.headers().remove(header::HOST);
					self.apply_header(&HeaderOrPseudo::Authority, Some(value), action);
					return;
				}

				let exists = self.headers().contains_key(k);
				if !action.should_apply(exists) {
					return;
				}
				if action.should_append() {
					self.headers().append(k.clone(), v);
				} else {
					self.headers().insert(k.clone(), v);
				}
			},
			(HeaderOrPseudo::Header(k), None) => {
				// Need to sanitize it, so a failed execution cannot mean the user can set arbitrary headers.
				self.headers().remove(k);
			},
			(_, Some(HeaderOrPseudoValue::Method(v))) => {
				if let RequestOrResponse::Request(r) = self {
					*r.method_mut() = v;
				}
			},
			(_, Some(HeaderOrPseudoValue::Scheme(v))) => {
				if let RequestOrResponse::Request(r) = self {
					let _ = modify_req_uri(r, |uri| {
						uri.scheme = Some(v);
						Ok(())
					});
				}
			},
			(_, Some(HeaderOrPseudoValue::Authority(v))) => {
				if let RequestOrResponse::Request(r) = self {
					let _ = modify_req_uri(r, |uri| {
						uri.authority = Some(v);
						if uri.scheme.is_none() {
							// When authority is set, scheme must also be set
							// TODO: do the same for HeaderOrPseudo::Scheme
							uri.scheme = Some(Scheme::HTTP);
						}
						Ok(())
					});
				}
			},
			(_, Some(HeaderOrPseudoValue::Path(v))) => {
				if let RequestOrResponse::Request(r) = self {
					let _ = modify_req_uri(r, |uri| {
						uri.path_and_query = Some(v);
						Ok(())
					});
				}
			},
			(_, Some(HeaderOrPseudoValue::Status(v))) => {
				if let RequestOrResponse::Response(r) = self {
					*r.status_mut() = v;
				}
			},
			(_, None) => {
				// Invalid, do nothing
			},
			(_, _) => {
				unreachable!("invalid k/v pair")
			},
		}
	}
}

use std::borrow::Cow;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll};

pub use ::http::uri::{Authority, Scheme};
pub use ::http::{
	HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header, status, uri,
};
use axum_core::BoxError;
use bytes::Bytes;
use cel::Value;
use http::uri::PathAndQuery;
use http_body::{Frame, SizeHint};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tower_serve_static::private::mime;
use url::{Url, form_urlencoded};

use crate::cel::{BackendContext, LLMContext, RequestTime, SourceContext};
use crate::client::PoolKey;
use crate::proxy::{ProxyError, ProxyResponse};
use crate::transport::BufferLimit;
use crate::transport::stream::TCPConnectionInfo;
use crate::types::agent::PathMatch;

/// Represents either an HTTP header or an HTTP/2 pseudo-header
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HeaderOrPseudo {
	Header(HeaderName),
	Method,
	Scheme,
	Authority,
	Path,
	Status,
}

/// Represents a value for an HTTP header or an HTTP/2 pseudo-header
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HeaderOrPseudoValue {
	Header(HeaderValue),
	Method(Method),
	Scheme(Scheme),
	Authority(Authority),
	Path(PathAndQuery),
	Status(StatusCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeaderMutationAction {
	AppendIfExistsOrAdd,
	AddIfAbsent,
	OverwriteIfExistsOrAdd,
	OverwriteIfExists,
}

impl HeaderMutationAction {
	pub fn should_apply(self, exists: bool) -> bool {
		match self {
			HeaderMutationAction::AppendIfExistsOrAdd | HeaderMutationAction::OverwriteIfExistsOrAdd => {
				true
			},
			HeaderMutationAction::AddIfAbsent => !exists,
			HeaderMutationAction::OverwriteIfExists => exists,
		}
	}

	pub fn should_append(self) -> bool {
		matches!(self, HeaderMutationAction::AppendIfExistsOrAdd)
	}
}

impl HeaderOrPseudoValue {
	pub fn from_raw(k: &HeaderOrPseudo, raw: &[u8]) -> Option<HeaderOrPseudoValue> {
		match k {
			HeaderOrPseudo::Header(_) => HeaderValue::from_bytes(raw)
				.ok()
				.map(HeaderOrPseudoValue::Header),
			HeaderOrPseudo::Status => std::str::from_utf8(raw)
				.ok()
				.and_then(|s| s.parse::<u16>().ok())
				.and_then(|s| StatusCode::from_u16(s).ok())
				.map(HeaderOrPseudoValue::Status),
			HeaderOrPseudo::Method => ::http::Method::from_bytes(raw)
				.ok()
				.map(HeaderOrPseudoValue::Method),
			HeaderOrPseudo::Scheme => ::http::uri::Scheme::try_from(raw)
				.ok()
				.map(HeaderOrPseudoValue::Scheme),
			HeaderOrPseudo::Authority => ::http::uri::Authority::try_from(raw)
				.ok()
				.map(HeaderOrPseudoValue::Authority),
			HeaderOrPseudo::Path => ::http::uri::PathAndQuery::try_from(raw)
				.ok()
				.map(HeaderOrPseudoValue::Path),
		}
	}

	pub fn from_cel_result(k: &HeaderOrPseudo, res: Option<Value>) -> Option<HeaderOrPseudoValue> {
		match (res?.always_materialize_owned(), k) {
			(v, HeaderOrPseudo::Header(_)) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| HeaderValue::from_bytes(b).ok())
				.map(HeaderOrPseudoValue::Header),
			(v, HeaderOrPseudo::Status) => v
				.as_unsigned()
				.ok()
				.and_then(|v| u16::try_from(v).ok())
				.and_then(|v| StatusCode::from_u16(v).ok())
				.map(HeaderOrPseudoValue::Status),
			(v, HeaderOrPseudo::Method) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| ::http::Method::from_bytes(b).ok())
				.map(HeaderOrPseudoValue::Method),
			(v, HeaderOrPseudo::Scheme) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| ::http::uri::Scheme::try_from(b).ok())
				.map(HeaderOrPseudoValue::Scheme),
			(v, HeaderOrPseudo::Authority) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| ::http::uri::Authority::try_from(b).ok())
				.map(HeaderOrPseudoValue::Authority),
			(v, HeaderOrPseudo::Path) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| ::http::uri::PathAndQuery::try_from(b).ok())
				.map(HeaderOrPseudoValue::Path),
		}
	}
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

impl<'de> Deserialize<'de> for HeaderOrPseudo {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;

		match s.as_str() {
			":method" => Ok(HeaderOrPseudo::Method),
			":scheme" => Ok(HeaderOrPseudo::Scheme),
			":authority" => Ok(HeaderOrPseudo::Authority),
			":path" => Ok(HeaderOrPseudo::Path),
			":status" => Ok(HeaderOrPseudo::Status),
			_ => Ok(HeaderOrPseudo::Header(
				HeaderName::from_str(&s).map_err(serde::de::Error::custom)?,
			)),
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

/// Extract the value for a pseudo header or header from the request
pub fn get_pseudo_or_header_value<'a>(
	pseudo: &HeaderOrPseudo,
	req: &'a Request,
) -> Option<std::borrow::Cow<'a, HeaderValue>> {
	match pseudo {
		HeaderOrPseudo::Header(v) => req.headers().get(v).map(std::borrow::Cow::Borrowed),
		_ => get_pseudo_header_value(pseudo, req)
			.and_then(|v| HeaderValue::try_from(&v).ok().map(std::borrow::Cow::Owned)),
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

pub mod x_headers {
	use http::{HeaderMap, HeaderName, HeaderValue};

	pub const X_RATELIMIT_LIMIT: HeaderName = HeaderName::from_static("x-ratelimit-limit");
	pub const X_RATELIMIT_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");
	pub const X_RATELIMIT_RESET: HeaderName = HeaderName::from_static("x-ratelimit-reset");
	pub const X_AMZN_REQUESTID: HeaderName = HeaderName::from_static("x-amzn-requestid");
	pub const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

	pub const RETRY_AFTER_MS: HeaderName = HeaderName::from_static("retry-after-ms");

	pub const X_RATELIMIT_RESET_REQUESTS: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-requests");
	pub const X_RATELIMIT_RESET_TOKENS: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-tokens");
	pub const X_RATELIMIT_RESET_REQUESTS_DAY: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-requests-day");
	pub const X_RATELIMIT_RESET_TOKENS_MINUTE: HeaderName =
		HeaderName::from_static("x-ratelimit-reset-tokens-minute");

	pub fn forwarded_proto(headers: &HeaderMap<HeaderValue>) -> Option<String> {
		headers
			.get_all(&X_FORWARDED_PROTO)
			.iter()
			.filter_map(|value| value.to_str().ok())
			.flat_map(|value| value.split(','))
			.map(str::trim)
			.find(|value| !value.is_empty())
			.map(|value| value.to_ascii_lowercase())
	}
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

pub fn modify_query_parameters<S, R, KSet, VSet, KRemove>(
	uri: &mut Uri,
	query_parameters_to_set: S,
	query_parameters_to_remove: R,
) -> anyhow::Result<()>
where
	S: IntoIterator<Item = (KSet, VSet)>,
	R: IntoIterator<Item = KRemove>,
	KSet: AsRef<str>,
	VSet: AsRef<str>,
	KRemove: AsRef<str>,
{
	let query_parameters_to_set = query_parameters_to_set
		.into_iter()
		.map(|(key, value)| (key.as_ref().to_owned(), value.as_ref().to_owned()))
		.collect::<Vec<_>>();
	let query_parameters_to_remove = query_parameters_to_remove
		.into_iter()
		.map(|key| key.as_ref().to_owned())
		.collect::<Vec<_>>();

	if query_parameters_to_set.is_empty() && query_parameters_to_remove.is_empty() {
		return Ok(());
	}

	let mut parts = std::mem::take(uri).into_parts();
	let path = parts
		.path_and_query
		.as_ref()
		.map(|pq| pq.path())
		.filter(|path| !path.is_empty())
		.unwrap_or("/");
	let query = parts
		.path_and_query
		.as_ref()
		.and_then(|pq| pq.query())
		.unwrap_or_default();
	let mut pairs = form_urlencoded::parse(query.as_bytes())
		.map(|(key, value)| (key.into_owned(), value.into_owned()))
		.collect::<Vec<_>>();

	for (key, value) in query_parameters_to_set {
		pairs.retain(|(current_key, _)| current_key != &key);
		pairs.push((key, value));
	}

	if !query_parameters_to_remove.is_empty() {
		pairs.retain(|(key, _)| {
			!query_parameters_to_remove
				.iter()
				.any(|remove| remove == key)
		});
	}

	let mut updated = form_urlencoded::Serializer::new(String::new());
	for (key, value) in pairs {
		updated.append_pair(&key, &value);
	}

	let updated = updated.finish();
	let new_path: Result<PathAndQuery, _> = if updated.is_empty() {
		path.to_string()
	} else {
		format!("{path}?{updated}")
	}
	.parse();
	match new_path {
		Ok(p) => {
			parts.path_and_query = Some(p);
			*uri = Uri::from_parts(parts)?;
			Ok(())
		},
		Err(e) => {
			// Just a backup, in the event that somehow our new param was invalid we still set the URI
			// so its not wiped out
			*uri = Uri::from_parts(parts)?;
			Err(e.into())
		},
	}
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

pub fn get_path_and_query(req: &Uri) -> &str {
	req
		.path_and_query()
		.map(|pq| pq.as_str())
		.unwrap_or_else(|| req.path())
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

pub async fn read_req_body(req: Request) -> Result<Bytes, axum_core::Error> {
	let lim = buffer_limit(&req);
	read_body_with_limit(req.into_body(), lim).await
}

pub async fn read_resp_body(resp: Response) -> Result<Bytes, axum_core::Error> {
	let lim = response_buffer_limit(&resp);
	read_body_with_limit(resp.into_body(), lim).await
}

pub async fn read_response_body(
	resp: Response,
) -> Result<(::http::response::Parts, Bytes), axum_core::Error> {
	let lim = response_buffer_limit(&resp);
	let (h, b) = resp.into_parts();
	read_body_with_limit(b, lim).await.map(|b| (h, b))
}

pub async fn read_body_with_limit(body: Body, limit: usize) -> Result<Bytes, axum_core::Error> {
	axum::body::to_bytes(body, limit).await
}

pub async fn inspect_body(req: &mut Request) -> anyhow::Result<Bytes> {
	let lim = buffer_limit(req);
	inspect_body_with_limit(req.body_mut(), lim).await
}

pub async fn inspect_response_body(resp: &mut Response) -> anyhow::Result<Bytes> {
	let lim = response_buffer_limit(resp);
	inspect_body_with_limit(resp.body_mut(), lim).await
}

pub async fn inspect_body_with_limit(body: &mut Body, limit: usize) -> anyhow::Result<Bytes> {
	peekbody::inspect_body(body, limit).await
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
		if let Some(e) = ext.get::<crate::llm::bedrock::AwsServiceName>() {
			d.field("AwsServiceName", e);
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
}
