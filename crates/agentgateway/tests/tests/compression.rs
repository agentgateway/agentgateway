use agentgateway::http::compression::{decompress_body, encode_body};
use headers::{ContentEncoding, HeaderMapExt};

use crate::common::prelude::*;

#[tokio::test]
async fn response_is_compressed_for_accepting_client() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"compression": {"response": {"compress": {"algorithms": ["gzip", "brotli"]}}},
		}))
		.await;

	let res = RequestBuilder::new(Method::GET, "http://lo/compressed")
		.header(header::ACCEPT_ENCODING, "br;q=0.5, gzip")
		.send(io)
		.await
		.unwrap();

	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr(header::CONTENT_ENCODING), "gzip");
	let ce = res.headers().typed_get::<ContentEncoding>();
	let (body, _) = decompress_body(res.into_body(), ce.as_ref()).unwrap();
	let dump = read_body(body).await;
	assert_eq!(dump.uri.path(), "/compressed");
	// The client's Accept-Encoding is still forwarded; the backend chose not to encode.
	assert_eq!(dump.headers[header::ACCEPT_ENCODING], "br;q=0.5, gzip");
}

#[tokio::test]
async fn head_gets_same_encoding_headers_as_get() {
	let (_mock, mut bind, _) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"compression": {"response": {"compress": {"algorithms": ["gzip"]}}},
		}))
		.await;

	let mut headers = Vec::new();
	for method in [Method::GET, Method::HEAD] {
		let res = RequestBuilder::new(method, "http://lo/compressed")
			.header(header::ACCEPT_ENCODING, "gzip")
			.send(bind.serve_http(BIND_KEY))
			.await
			.unwrap();
		assert_eq!(res.status(), 200);
		headers.push((
			res.headers().get(header::CONTENT_ENCODING).cloned(),
			res.headers().get(header::VARY).cloned(),
		));
	}
	assert_eq!(headers[0].0.as_ref().unwrap(), "gzip");
	assert_eq!(headers[0], headers[1]);
}

#[tokio::test]
async fn request_body_is_decoded_for_policies_and_backend() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"compression": {"request": {"decompress": {"algorithms": ["gzip"]}}},
			"transformations": {"request": {"set": {"x-body": "string(request.body)"}}},
		}))
		.await;

	let encoded = encode_body(b"plain body", "gzip").await.unwrap();
	let res = RequestBuilder::new(Method::POST, "http://lo/upload")
		.header(header::CONTENT_ENCODING, "gzip")
		.body(encoded)
		.send(io)
		.await
		.unwrap();

	assert_eq!(res.status(), 200);
	let dump = read_body(res.into_body()).await;
	assert_eq!(dump.body, "plain body");
	assert_eq!(dump.headers["x-body"], "plain body");
	assert!(!dump.headers.contains_key(header::CONTENT_ENCODING));
}

#[tokio::test]
async fn denied_request_is_not_decoded() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"compression": {"request": {"decompress": {}}},
			"authorization": {"rules": [{"allow": "has(jwt.sub)"}]},
			// Body-aware CEL forces the body to be buffered before authorization runs.
			"transformations": {"request": {"set": {"x-body": "string(request.body)"}}},
		}))
		.await;

	// Not valid gzip: decoding before authorization would surface as an error, not a 403.
	let res = RequestBuilder::new(Method::POST, "http://lo/upload")
		.header(header::CONTENT_ENCODING, "gzip")
		.body("not gzip")
		.send(io)
		.await
		.unwrap();

	assert_eq!(res.status(), 403);
}
