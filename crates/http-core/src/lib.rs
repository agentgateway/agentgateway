pub mod buflist;
mod peekbody;

pub type Error = axum_core::Error;
pub type Body = axum_core::body::Body;
pub type Request = http::Request<Body>;
pub type Response = http::Response<Body>;
pub use http::uri::{Authority, Scheme};
pub use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header, status, uri};

pub const DEFAULT_BUFFER_LIMIT: usize = 2_097_152;

#[derive(Debug, Clone)]
pub struct BufferLimit(pub usize);

/// HTTP extension containing a buffered copy of a request or response body for policy evaluation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BufferedBody(
	#[cfg_attr(feature = "schema", schemars(with = "String"))] BufferedBodyState,
);

#[derive(Debug, Clone)]
enum BufferedBodyState {
	Complete(bytes::Bytes),
	ExceededLimit(bytes::Bytes),
}

impl BufferedBody {
	pub fn complete(bytes: bytes::Bytes) -> Self {
		Self(BufferedBodyState::Complete(bytes))
	}

	pub fn exceeded_limit(bytes: bytes::Bytes) -> Self {
		Self(BufferedBodyState::ExceededLimit(bytes))
	}

	pub fn bytes(&self) -> Option<&bytes::Bytes> {
		match &self.0 {
			BufferedBodyState::Complete(bytes) => Some(bytes),
			BufferedBodyState::ExceededLimit(_) => None,
		}
	}

	pub fn prefix_bytes(&self) -> &bytes::Bytes {
		match &self.0 {
			BufferedBodyState::Complete(bytes) | BufferedBodyState::ExceededLimit(bytes) => bytes,
		}
	}

	pub fn is_too_large(&self) -> bool {
		matches!(&self.0, BufferedBodyState::ExceededLimit(_))
	}
}

impl serde::Serialize for BufferedBody {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use base64::Engine;
		match &self.0 {
			BufferedBodyState::Complete(bytes) => {
				let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
				serializer.serialize_str(&encoded)
			},
			BufferedBodyState::ExceededLimit(_) => serializer.serialize_none(),
		}
	}
}

impl<'de> serde::Deserialize<'de> for BufferedBody {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		use base64::Engine;
		let s = String::deserialize(deserializer)?;
		let bytes = base64::engine::general_purpose::STANDARD
			.decode(&s)
			.map_err(serde::de::Error::custom)?;
		Ok(BufferedBody::complete(bytes::Bytes::from(bytes)))
	}
}

impl cel::types::dynamic::DynamicType for BufferedBody {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> cel::Value<'_> {
		match &self.0 {
			BufferedBodyState::Complete(bytes) => {
				cel::Value::Bytes(cel::objects::BytesValue::Bytes(bytes.clone()))
			},
			BufferedBodyState::ExceededLimit(_) => cel::Value::Null,
		}
	}
}

impl BufferLimit {
	pub fn new(limit: usize) -> Self {
		BufferLimit(limit)
	}
}

pub fn buffer_limit(req: &Request) -> usize {
	req
		.extensions()
		.get::<BufferLimit>()
		.map(|b| b.0)
		.unwrap_or(DEFAULT_BUFFER_LIMIT)
}

pub fn response_buffer_limit(resp: &Response) -> usize {
	resp
		.extensions()
		.get::<BufferLimit>()
		.map(|b| b.0)
		.unwrap_or(DEFAULT_BUFFER_LIMIT)
}

pub async fn read_body_with_limit(body: Body, limit: usize) -> Result<bytes::Bytes, Error> {
	axum::body::to_bytes(body, limit).await
}

pub async fn read_req_body(req: Request) -> Result<bytes::Bytes, Error> {
	let lim = buffer_limit(&req);
	read_body_with_limit(req.into_body(), lim).await
}

pub async fn read_resp_body(resp: Response) -> Result<bytes::Bytes, Error> {
	let lim = response_buffer_limit(&resp);
	read_body_with_limit(resp.into_body(), lim).await
}

pub async fn read_response_body(
	resp: Response,
) -> Result<(http::response::Parts, bytes::Bytes), Error> {
	let lim = response_buffer_limit(&resp);
	let (h, b) = resp.into_parts();
	read_body_with_limit(b, lim).await.map(|b| (h, b))
}

/// Result of inspecting a body without consuming it from the caller's perspective.
#[derive(Debug)]
#[must_use]
pub enum BodyInspection {
	/// The complete body fit within the configured limit.
	Complete(bytes::Bytes),
	/// The body exceeded the limit. Contains the first `limit` bytes.
	Partial(bytes::Bytes),
}

impl From<BodyInspection> for BufferedBody {
	fn from(inspection: BodyInspection) -> Self {
		match inspection {
			BodyInspection::Complete(bytes) => Self::complete(bytes),
			BodyInspection::Partial(bytes) => Self::exceeded_limit(bytes),
		}
	}
}

pub async fn inspect_body(req: &mut Request) -> anyhow::Result<BodyInspection> {
	let lim = buffer_limit(req);
	inspect_body_with_limit(req.body_mut(), lim).await
}

pub async fn inspect_response_body(resp: &mut Response) -> anyhow::Result<BodyInspection> {
	let lim = response_buffer_limit(resp);
	inspect_body_with_limit(resp.body_mut(), lim).await
}

pub async fn inspect_body_with_limit(
	body: &mut Body,
	limit: usize,
) -> anyhow::Result<BodyInspection> {
	let mut bytes = peekbody::inspect_body(body, limit.saturating_add(1)).await?;
	if bytes.len() > limit {
		bytes.truncate(limit);
		Ok(BodyInspection::Partial(bytes))
	} else {
		Ok(BodyInspection::Complete(bytes))
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

pub fn get_path_and_query(req: &Uri) -> &str {
	req
		.path_and_query()
		.map(|pq| pq.as_str())
		.unwrap_or_else(|| req.path())
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
	let mut pairs = url::form_urlencoded::parse(query.as_bytes())
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

	let mut updated = url::form_urlencoded::Serializer::new(String::new());
	for (key, value) in pairs {
		updated.append_pair(&key, &value);
	}

	let updated = updated.finish();
	let new_path: Result<http::uri::PathAndQuery, _> = if updated.is_empty() {
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
			*uri = Uri::from_parts(parts)?;
			Err(e.into())
		},
	}
}

#[derive(Debug, Default)]
#[must_use]
pub struct PolicyResponse {
	pub direct_response: Option<Response>,
	pub response_headers: Option<HeaderMap>,
}

impl PolicyResponse {
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

/// A mutable handle that can represent either a request or a response.
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
				if k == http::header::HOST && matches!(self, RequestOrResponse::Request(_)) {
					let Some(value) = HeaderOrPseudoValue::from_raw(&HeaderOrPseudo::Authority, v.as_bytes())
					else {
						return;
					};
					self.headers().remove(http::header::HOST);
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
			(_, None) => {},
			(_, _) => {
				unreachable!("invalid k/v pair")
			},
		}
	}
}

/// Represents either an HTTP header or an HTTP/2 pseudo-header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HeaderOrPseudo {
	Header(HeaderName),
	Method,
	Scheme,
	Authority,
	Path,
	Status,
}

/// Represents a value for an HTTP header or an HTTP/2 pseudo-header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HeaderOrPseudoValue {
	Header(HeaderValue),
	Method(Method),
	Scheme(Scheme),
	Authority(Authority),
	Path(http::uri::PathAndQuery),
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
			HeaderOrPseudo::Method => Method::from_bytes(raw)
				.ok()
				.map(HeaderOrPseudoValue::Method),
			HeaderOrPseudo::Scheme => Scheme::try_from(raw).ok().map(HeaderOrPseudoValue::Scheme),
			HeaderOrPseudo::Authority => Authority::try_from(raw)
				.ok()
				.map(HeaderOrPseudoValue::Authority),
			HeaderOrPseudo::Path => http::uri::PathAndQuery::try_from(raw)
				.ok()
				.map(HeaderOrPseudoValue::Path),
		}
	}

	pub fn from_cel_result(
		k: &HeaderOrPseudo,
		res: Option<cel::Value>,
	) -> Option<HeaderOrPseudoValue> {
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
				.and_then(|b| Method::from_bytes(b).ok())
				.map(HeaderOrPseudoValue::Method),
			(v, HeaderOrPseudo::Scheme) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| Scheme::try_from(b).ok())
				.map(HeaderOrPseudoValue::Scheme),
			(v, HeaderOrPseudo::Authority) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| Authority::try_from(b).ok())
				.map(HeaderOrPseudoValue::Authority),
			(v, HeaderOrPseudo::Path) => v
				.as_bytes_pre_materialized()
				.ok()
				.and_then(|b| http::uri::PathAndQuery::try_from(b).ok())
				.map(HeaderOrPseudoValue::Path),
		}
	}
}

impl TryFrom<&str> for HeaderOrPseudo {
	type Error = http::header::InvalidHeaderName;

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

impl serde::Serialize for HeaderOrPseudo {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
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

impl<'de> serde::Deserialize<'de> for HeaderOrPseudo {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;

		match s.as_str() {
			":method" => Ok(HeaderOrPseudo::Method),
			":scheme" => Ok(HeaderOrPseudo::Scheme),
			":authority" => Ok(HeaderOrPseudo::Authority),
			":path" => Ok(HeaderOrPseudo::Path),
			":status" => Ok(HeaderOrPseudo::Status),
			_ => Ok(HeaderOrPseudo::Header(
				HeaderName::try_from(s.as_str()).map_err(serde::de::Error::custom)?,
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
		HeaderOrPseudo::Status => None,
		HeaderOrPseudo::Header(_) => None,
	}
}

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

pub fn modify_req(
	req: &mut Request,
	f: impl FnOnce(&mut http::request::Parts) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
	let nreq = std::mem::take(req);
	let (mut head, body) = nreq.into_parts();
	f(&mut head)?;
	*req = Request::from_parts(head, body);
	Ok(())
}

pub fn modify_req_uri(
	req: &mut Request,
	f: impl FnOnce(&mut http::uri::Parts) -> anyhow::Result<()>,
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
	f: impl FnOnce(&mut http::uri::Parts) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
	let nreq = std::mem::take(&mut head.uri);

	let mut parts = nreq.into_parts();
	f(&mut parts)?;
	head.uri = Uri::from_parts(parts)?;
	Ok(())
}

pub mod x_headers {
	use http::uri::Scheme;
	use http::{HeaderMap, HeaderName, HeaderValue, Uri};

	pub const TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");

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

	pub fn forwarded_scheme(headers: &HeaderMap<HeaderValue>) -> Option<Scheme> {
		forwarded_proto(headers).and_then(|proto| proto.parse().ok())
	}

	pub fn apply_forwarded_scheme(uri: Uri, headers: &HeaderMap<HeaderValue>) -> Uri {
		let Some(scheme) = forwarded_scheme(headers) else {
			return uri;
		};
		if uri.authority().is_none() {
			return uri;
		}

		let original = uri.clone();
		let mut parts = uri.into_parts();
		parts.scheme = Some(scheme);
		Uri::from_parts(parts).unwrap_or(original)
	}
}
