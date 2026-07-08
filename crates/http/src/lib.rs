pub type Error = axum_core::Error;
pub type Body = axum_core::body::Body;
pub type Request = http::Request<Body>;
pub type Response = http::Response<Body>;

pub const DEFAULT_BUFFER_LIMIT: usize = 2_097_152;

#[derive(Debug, Clone)]
pub struct BufferLimit(pub usize);

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
