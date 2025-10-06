use http::{HeaderName, HeaderValue, Uri};

use crate::http::{Body, Request, Response};

pub fn request_for_uri(uri: &str) -> Request {
	request(uri, http::Method::GET, &[])
}

pub fn request(uri: &str, method: http::Method, headers: &[(&str, &str)]) -> Request {
	let mut rb = ::http::Request::builder()
		.uri(uri.parse::<Uri>().unwrap())
		.method(method);
	for (name, value) in headers {
		rb = rb.header(
			HeaderName::try_from(name.to_string()).unwrap(),
			HeaderValue::from_str(value).unwrap(),
		);
	}
	rb.body(Body::empty()).unwrap()
}

pub trait ResponseExt {
	fn hdr(&self, h: impl TryInto<HeaderName>) -> &str;
}

impl ResponseExt for Response {
	fn hdr(&self, h: impl TryInto<HeaderName>) -> &str {
		let Ok(h) = h.try_into() else {
			panic!("invalid header key")
		};
		self
			.headers()
			.get(h)
			.and_then(|s| s.to_str().ok())
			.unwrap_or_default()
	}
}
