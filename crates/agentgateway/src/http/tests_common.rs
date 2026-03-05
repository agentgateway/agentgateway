use bytes::Bytes;
use http::{HeaderName, HeaderValue, Uri};

use crate::http::{Body, Request, Response};
use crate::test_helpers::proxymock::read_body_raw;

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

pub fn assert_status(res: &Response, want: u16) {
	assert_eq!(res.status().as_u16(), want);
}

pub fn assert_header(res: &Response, h: impl TryInto<HeaderName>, want: &str) {
	let Ok(h) = h.try_into() else {
		panic!("invalid header key")
	};
	assert_eq!(res.hdr(h), want);
}

pub async fn assert_body(res: Response, want: impl AsRef<[u8]>) {
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(body.as_ref(), want.as_ref());
}

pub trait IntoTestBody {
	fn into_test_body(self) -> Body;
}

impl IntoTestBody for Body {
	fn into_test_body(self) -> Body {
		self
	}
}

impl IntoTestBody for Response {
	fn into_test_body(self) -> Body {
		self.into_body()
	}
}

pub async fn read_body_for_test(body: impl IntoTestBody) -> Bytes {
	crate::http::read_body_with_limit(body.into_test_body(), 100)
		.await
		.unwrap()
}

#[macro_export]
macro_rules! read_body {
	($body:expr) => {
		$crate::http::tests_common::read_body_for_test($body).await
	};
}
