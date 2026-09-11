mod body;
mod buflist;
mod idle_timeout;
mod peekbody;
mod recordbody;

pub use body::{Body, BodyContent, BodyObserver, BodyTimeoutError, ReplayBodyState};
pub use buflist::BufList;
pub use recordbody::{RecordedBody, RecordedBodyHandle};

pub type Error = axum_core::Error;
pub type RawBody = axum_core::body::Body;
pub type Request = http::Request<Body>;
pub type Response = http::Response<Body>;

pub trait ResponseBodyExt {
	/// Replace content and discard the old length.
	fn replace_body_bytes(&mut self, bytes: bytes::Bytes);

	fn try_modify_body<F, Fut, E>(&mut self, f: F) -> impl Future<Output = Result<(), E>>
	where
		F: FnOnce(Body) -> Fut,
		Fut: Future<Output = Result<BodyContent, E>>;
}

impl ResponseBodyExt for Response {
	fn replace_body_bytes(&mut self, bytes: bytes::Bytes) {
		self.body_mut().replace_bytes(bytes);
		self.headers_mut().remove(http::header::CONTENT_LENGTH);
	}

	async fn try_modify_body<F, Fut, E>(&mut self, f: F) -> Result<(), E>
	where
		F: FnOnce(Body) -> Fut,
		Fut: Future<Output = Result<BodyContent, E>>,
	{
		self.body_mut().try_modify(f).await?;
		self.headers_mut().remove(http::header::CONTENT_LENGTH);
		Ok(())
	}
}

pub trait RequestBodyExt {
	/// Replace content and discard the old length.
	fn replace_body_bytes(&mut self, bytes: bytes::Bytes);
}

impl RequestBodyExt for Request {
	fn replace_body_bytes(&mut self, bytes: bytes::Bytes) {
		self.body_mut().replace_bytes(bytes);
		self.headers_mut().remove(http::header::CONTENT_LENGTH);
	}
}

pub const DEFAULT_BUFFER_LIMIT: usize = 2_097_152;

#[derive(Debug, Clone)]
pub struct BufferLimit(pub usize);

/// A bounded snapshot made available without consuming the body from the
/// downstream caller's perspective.
///
/// Inspection may poll and buffer the body so a policy can examine it before
/// forwarding. It belongs to the specific body content and is invalidated when
/// that content changes. In contrast, [`RecordedBodyHandle`] passively observes
/// bytes only as downstream consumes them, primarily for logging, and can remain
/// attached across a content replacement.
#[derive(Clone, Debug)]
#[must_use]
pub enum BodyInspection {
	/// The complete body fit within the configured limit.
	Complete(bytes::Bytes),
	/// The body exceeded the limit. Contains the first `limit` bytes.
	Partial(bytes::Bytes),
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
	body.into_bytes(limit).await
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

	/// Sets `x-ratelimit-limit`/`-remaining`/`-reset` for the most-constrained limit. No-op if any
	/// is already present (e.g. from an upstream rate limit service) to avoid mixing header sets.
	pub fn set_ratelimit_headers(
		hm: &mut HeaderMap<HeaderValue>,
		limit: u64,
		remaining: u64,
		reset_seconds: u64,
	) {
		if hm.contains_key(&X_RATELIMIT_LIMIT)
			|| hm.contains_key(&X_RATELIMIT_REMAINING)
			|| hm.contains_key(&X_RATELIMIT_RESET)
		{
			return;
		}
		insert_header(hm, X_RATELIMIT_LIMIT, limit);
		insert_header(hm, X_RATELIMIT_REMAINING, remaining);
		insert_header(hm, X_RATELIMIT_RESET, reset_seconds);
	}

	fn insert_header(hm: &mut HeaderMap<HeaderValue>, name: HeaderName, value: u64) {
		if let Ok(hv) = HeaderValue::try_from(value.to_string()) {
			hm.insert(name, hv);
		}
	}
}
