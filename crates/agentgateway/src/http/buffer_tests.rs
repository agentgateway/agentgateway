use std::convert::Infallible;

use bytes::Bytes;
use http_body::Frame;
use http_body_util::BodyExt;

use super::*;
use crate::http::{Request, Response};
use crate::proxy::ProxyResponse;
use crate::transport::BufferLimit;

fn request_with_body(body: crate::http::Body) -> Request {
	::http::Request::builder()
		.uri("http://example.com/")
		.body(body)
		.expect("request builds")
}

fn response_with_body(body: crate::http::Body) -> Response {
	::http::Response::builder()
		.status(::http::StatusCode::OK)
		.body(body)
		.expect("response builds")
}

fn streaming_body(chunks: &[&'static [u8]]) -> crate::http::Body {
	let frames: Vec<_> = chunks
		.iter()
		.map(|chunk| Ok::<_, Infallible>(Frame::data(Bytes::from_static(chunk))))
		.collect();
	crate::http::Body::new(http_body_util::StreamBody::new(tokio_stream::iter(frames)))
}

async fn read_request_body_bytes(req: &mut Request) -> Bytes {
	let body = std::mem::replace(req.body_mut(), crate::http::Body::empty());
	body.collect().await.expect("collect succeeds").to_bytes()
}

async fn read_response_body_bytes(resp: &mut Response) -> Bytes {
	let body = std::mem::replace(resp.body_mut(), crate::http::Body::empty());
	body.collect().await.expect("collect succeeds").to_bytes()
}

fn enabled_request(max_bytes: usize) -> Buffer {
	Buffer {
		request: Some(BufferBody {
			max_bytes: Some(max_bytes),
		}),
		response: None,
	}
}

fn enabled_response(max_bytes: usize) -> Buffer {
	Buffer {
		request: None,
		response: Some(BufferBody {
			max_bytes: Some(max_bytes),
		}),
	}
}

fn disabled_request() -> Buffer {
	Buffer {
		request: None,
		response: None,
	}
}

fn disabled_response() -> Buffer {
	Buffer {
		request: None,
		response: None,
	}
}

#[test]
fn deserialize_reads_camel_case_fields() {
	let buffer: Buffer =
		serde_json::from_str(r#"{"request":{"maxBytes":10},"response":{"maxBytes":20}}"#)
			.expect("valid json");
	assert_eq!(buffer.request.unwrap().max_bytes, Some(10));
	assert_eq!(buffer.response.unwrap().max_bytes, Some(20));
}

#[test]
fn deserialize_defaults_missing_fields_to_none() {
	let buffer: Buffer = serde_json::from_str("{}").expect("valid json");
	assert!(buffer.request.is_none());
	assert!(buffer.response.is_none());
}

#[test]
fn deserialize_defaults_missing_max_bytes_to_none() {
	let buffer: Buffer = serde_json::from_str(r#"{"request":{},"response":{}}"#).expect("valid json");
	assert_eq!(buffer.request.unwrap().max_bytes, None);
	assert_eq!(buffer.response.unwrap().max_bytes, None);
}

#[test]
fn deserialize_rejects_unknown_fields() {
	let err = serde_json::from_str::<Buffer>(r#"{"bogus": true}"#)
		.expect_err("unknown fields must be rejected");
	assert!(err.to_string().contains("bogus"), "got: {err}");
}

#[test]
fn serialize_emits_camel_case() {
	let buffer = Buffer {
		request: Some(BufferBody { max_bytes: Some(7) }),
		response: Some(BufferBody { max_bytes: Some(9) }),
	};
	let json = serde_json::to_string(&buffer).expect("serializable");
	assert!(json.contains("\"maxBytes\":7"), "got: {json}");
	assert!(json.contains("\"maxBytes\":9"), "got: {json}");
}

#[test]
fn roundtrip_preserves_values() {
	let original = Buffer {
		request: Some(BufferBody {
			max_bytes: Some(100),
		}),
		response: Some(BufferBody {
			max_bytes: Some(200),
		}),
	};
	let json = serde_json::to_string(&original).expect("serializable");
	let parsed: Buffer = serde_json::from_str(&json).expect("deserializable");
	assert_eq!(
		original.request.unwrap().max_bytes,
		parsed.request.unwrap().max_bytes
	);
	assert_eq!(
		original.response.unwrap().max_bytes,
		parsed.response.unwrap().max_bytes
	);
}

// --- apply_to_request -------------------------------------------------------

#[tokio::test]
async fn apply_to_request_is_noop_when_disabled() {
	let policy = disabled_request();
	let mut req = request_with_body(crate::http::Body::from("payload"));

	policy
		.apply_to_request(&mut req)
		.await
		.expect("disabled buffer should succeed");

	assert_eq!(
		read_request_body_bytes(&mut req).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_request_drains_streaming_body() {
	let policy = enabled_request(64);
	let mut req = request_with_body(streaming_body(&[b"hello", b" ", b"world"]));

	policy
		.apply_to_request(&mut req)
		.await
		.expect("buffer should succeed");

	assert_eq!(
		read_request_body_bytes(&mut req).await,
		Bytes::from_static(b"hello world")
	);
}

#[tokio::test]
async fn apply_to_request_handles_empty_body() {
	let policy = enabled_request(64);
	let mut req = request_with_body(crate::http::Body::empty());

	policy
		.apply_to_request(&mut req)
		.await
		.expect("empty body should buffer");

	assert_eq!(read_request_body_bytes(&mut req).await, Bytes::new());
}

#[tokio::test]
async fn apply_to_request_replaces_body_with_in_memory_copy() {
	let policy = enabled_request(64);
	let mut req = request_with_body(streaming_body(&[b"abc", b"def"]));

	policy
		.apply_to_request(&mut req)
		.await
		.expect("buffer succeeds");

	// First downstream consumer drains the body...
	let first = read_request_body_bytes(&mut req).await;
	assert_eq!(first, Bytes::from_static(b"abcdef"));
	// ...the body is now empty for the next consumer (consume-once semantics).
	let second = read_request_body_bytes(&mut req).await;
	assert_eq!(second, Bytes::new());
}

#[tokio::test]
async fn apply_to_request_skips_upgrade_requests() {
	let policy = enabled_request(64);
	let mut req = ::http::Request::builder()
		.uri("http://example.com/")
		.header(::http::header::UPGRADE, "websocket")
		.body(crate::http::Body::from("payload"))
		.expect("request builds");

	policy
		.apply_to_request(&mut req)
		.await
		.expect("upgrade requests skip buffer");

	assert_eq!(
		read_request_body_bytes(&mut req).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_request_fails_when_body_exceeds_limit() {
	let policy = enabled_request(4);
	let mut req = request_with_body(crate::http::Body::from("a body that is way too large"));

	let err = policy
		.apply_to_request(&mut req)
		.await
		.expect_err("oversize body must surface as an error");

	match err {
		ProxyResponse::DirectResponse(resp) => {
			assert_eq!(resp.status(), ::http::StatusCode::PAYLOAD_TOO_LARGE);
		},
		other => panic!("expected 413 DirectResponse, got {other:?}"),
	}
}

#[tokio::test]
async fn apply_to_request_uses_explicit_max_bytes_over_extension_limit() {
	let policy = enabled_request(64);
	let mut req = request_with_body(crate::http::Body::from("payload"));
	req.extensions_mut().insert(BufferLimit(4));

	policy
		.apply_to_request(&mut req)
		.await
		.expect("explicit maxBytes should allow payload");

	assert_eq!(
		read_request_body_bytes(&mut req).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_request_falls_back_to_extension_limit_when_max_bytes_missing() {
	let policy = Buffer {
		request: Some(BufferBody { max_bytes: None }),
		response: None,
	};
	let mut req = request_with_body(crate::http::Body::from("payload"));
	req.extensions_mut().insert(BufferLimit(4));

	let err = policy
		.apply_to_request(&mut req)
		.await
		.expect_err("fallback limit should reject payload");

	match err {
		ProxyResponse::DirectResponse(resp) => {
			assert_eq!(resp.status(), ::http::StatusCode::PAYLOAD_TOO_LARGE);
		},
		other => panic!("expected 413 DirectResponse, got {other:?}"),
	}
}

#[tokio::test]
async fn apply_to_request_ignores_response_max_bytes() {
	// response.max_bytes != 0 must not turn request buffering on.
	let policy = Buffer {
		request: None,
		response: Some(BufferBody {
			max_bytes: Some(1024),
		}),
	};
	let mut req = request_with_body(streaming_body(&[b"hello"]));

	policy
		.apply_to_request(&mut req)
		.await
		.expect("disabled request buffer is a no-op");

	assert_eq!(
		read_request_body_bytes(&mut req).await,
		Bytes::from_static(b"hello"),
	);
}

// --- apply_to_response ------------------------------------------------------

#[tokio::test]
async fn apply_to_response_is_noop_when_disabled() {
	let policy = disabled_response();
	let mut resp = response_with_body(crate::http::Body::from("payload"));

	policy
		.apply_to_response(&mut resp)
		.await
		.expect("disabled buffer should succeed");

	assert_eq!(
		read_response_body_bytes(&mut resp).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_response_drains_streaming_body() {
	let policy = enabled_response(64);
	let mut resp = response_with_body(streaming_body(&[b"hello", b" ", b"world"]));

	policy
		.apply_to_response(&mut resp)
		.await
		.expect("buffer should succeed");

	assert_eq!(
		read_response_body_bytes(&mut resp).await,
		Bytes::from_static(b"hello world")
	);
}

#[tokio::test]
async fn apply_to_response_handles_empty_body() {
	let policy = enabled_response(64);
	let mut resp = response_with_body(crate::http::Body::empty());

	policy
		.apply_to_response(&mut resp)
		.await
		.expect("empty body should buffer");

	assert_eq!(read_response_body_bytes(&mut resp).await, Bytes::new());
}

#[tokio::test]
async fn apply_to_response_skips_switching_protocols() {
	let policy = enabled_response(64);
	let mut resp = ::http::Response::builder()
		.status(::http::StatusCode::SWITCHING_PROTOCOLS)
		.body(crate::http::Body::from("payload"))
		.expect("response builds");

	policy
		.apply_to_response(&mut resp)
		.await
		.expect("101 responses skip buffer");

	assert_eq!(
		read_response_body_bytes(&mut resp).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_response_fails_when_body_exceeds_limit() {
	let policy = enabled_response(4);
	let mut resp = response_with_body(crate::http::Body::from("a body that is way too large"));

	let err = policy
		.apply_to_response(&mut resp)
		.await
		.expect_err("oversize body must surface as an error");

	match err {
		ProxyResponse::DirectResponse(resp) => {
			assert_eq!(resp.status(), ::http::StatusCode::BAD_GATEWAY);
		},
		other => panic!("expected 502 DirectResponse, got {other:?}"),
	}
}

#[tokio::test]
async fn apply_to_response_uses_explicit_max_bytes_over_extension_limit() {
	let policy = enabled_response(64);
	let mut resp = response_with_body(crate::http::Body::from("payload"));
	resp.extensions_mut().insert(BufferLimit(4));

	policy
		.apply_to_response(&mut resp)
		.await
		.expect("explicit maxBytes should allow payload");

	assert_eq!(
		read_response_body_bytes(&mut resp).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_response_falls_back_to_extension_limit_when_max_bytes_missing() {
	let policy = Buffer {
		request: None,
		response: Some(BufferBody { max_bytes: None }),
	};
	let mut resp = response_with_body(crate::http::Body::from("payload"));
	resp.extensions_mut().insert(BufferLimit(4));

	let err = policy
		.apply_to_response(&mut resp)
		.await
		.expect_err("fallback limit should reject payload");

	match err {
		ProxyResponse::DirectResponse(resp) => {
			assert_eq!(resp.status(), ::http::StatusCode::BAD_GATEWAY);
		},
		other => panic!("expected 502 DirectResponse, got {other:?}"),
	}
}

#[tokio::test]
async fn apply_to_response_ignores_request_max_bytes() {
	let policy = Buffer {
		request: Some(BufferBody {
			max_bytes: Some(1024),
		}),
		response: None,
	};
	let mut resp = response_with_body(streaming_body(&[b"hello"]));

	policy
		.apply_to_response(&mut resp)
		.await
		.expect("disabled response buffer is a no-op");

	assert_eq!(
		read_response_body_bytes(&mut resp).await,
		Bytes::from_static(b"hello"),
	);
}

// --- Trait integration ------------------------------------------------------

#[tokio::test]
async fn request_policy_trait_drains_body() {
	let policy = enabled_request(64);
	let mut req = request_with_body(streaming_body(&[b"hi ", b"there"]));

	let resp = crate::test_helpers::test_policy(&policy, &mut req)
		.await
		.expect("trait apply should succeed");

	assert!(
		!resp.should_short_circuit(),
		"buffer must not short-circuit the policy chain"
	);
	assert_eq!(
		read_request_body_bytes(&mut req).await,
		Bytes::from_static(b"hi there")
	);
}

#[tokio::test]
async fn request_policy_trait_is_noop_when_disabled() {
	let policy = disabled_request();
	let mut req = request_with_body(crate::http::Body::from("payload"));

	let resp = crate::test_helpers::test_policy(&policy, &mut req)
		.await
		.expect("trait apply should succeed");

	assert!(!resp.should_short_circuit());
	assert_eq!(
		read_request_body_bytes(&mut req).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn request_policy_trait_propagates_oversize_error() {
	let policy = enabled_request(4);
	let mut req = request_with_body(crate::http::Body::from(
		"way too large for the configured limit",
	));

	let err = crate::test_helpers::test_policy(&policy, &mut req)
		.await
		.expect_err("trait apply must surface oversize errors");

	match err {
		ProxyResponse::DirectResponse(resp) => {
			assert_eq!(resp.status(), ::http::StatusCode::PAYLOAD_TOO_LARGE);
		},
		other => panic!("expected 413 DirectResponse, got {other:?}"),
	}
}
