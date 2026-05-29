use std::convert::Infallible;

use bytes::Bytes;
use http_body::Frame;
use http_body_util::BodyExt;

use super::*;
use crate::http::Request;
use crate::proxy::ProxyResponse;
use crate::transport::BufferLimit;

fn request_with_body(body: crate::http::Body) -> Request {
	::http::Request::builder()
		.uri("http://example.com/")
		.body(body)
		.expect("request builds")
}

async fn read_body_bytes(req: &mut Request) -> Bytes {
	let body = std::mem::replace(req.body_mut(), crate::http::Body::empty());
	body.collect().await.expect("collect succeeds").to_bytes()
}

#[test]
fn try_from_serde_copies_field() {
	let buffering = Buffering::try_from(BufferingSerde {
		buffer_request_body: true,
	})
	.expect("valid buffering policy");
	assert!(buffering.buffer_request_body);

	let buffering = Buffering::try_from(BufferingSerde {
		buffer_request_body: false,
	})
	.expect("valid buffering policy");
	assert!(!buffering.buffer_request_body);
}

#[test]
fn deserialize_reads_camel_case_field() {
	let buffering: Buffering =
		serde_json::from_str(r#"{"bufferRequestBody": true}"#).expect("valid json");
	assert!(buffering.buffer_request_body);
}

#[test]
fn deserialize_defaults_missing_field_to_false() {
	let buffering: Buffering = serde_json::from_str("{}").expect("valid json");
	assert!(!buffering.buffer_request_body);
}

#[test]
fn deserialize_rejects_unknown_fields() {
	let err = serde_json::from_str::<Buffering>(r#"{"unknown": true}"#)
		.expect_err("unknown fields must be rejected");
	assert!(err.to_string().contains("unknown"));
}

#[test]
fn serialize_emits_camel_case_field() {
	let buffering = Buffering {
		buffer_request_body: true,
	};
	let json = serde_json::to_string(&buffering).expect("serializable");
	assert_eq!(json, r#"{"bufferRequestBody":true}"#);
}

#[test]
fn roundtrip_preserves_value() {
	let original = Buffering {
		buffer_request_body: true,
	};
	let json = serde_json::to_string(&original).expect("serializable");
	let parsed: Buffering = serde_json::from_str(&json).expect("deserializable");
	assert_eq!(original.buffer_request_body, parsed.buffer_request_body);
}

#[tokio::test]
async fn apply_to_request_is_noop_when_disabled() {
	let policy = Buffering {
		buffer_request_body: false,
	};
	let mut req = request_with_body(crate::http::Body::from("payload"));

	policy
		.apply_to_request(&mut req)
		.await
		.expect("disabled buffering should succeed");

	assert!(req.extensions().get::<BufferedRequestBody>().is_none());
	assert_eq!(
		read_body_bytes(&mut req).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_request_drains_streaming_body_into_extension() {
	let policy = Buffering {
		buffer_request_body: true,
	};
	// A multi-frame streaming body to verify the policy collects across frames, not just
	// the first chunk.
	let frames = tokio_stream::iter(vec![
		Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"hello"))),
		Ok::<_, Infallible>(Frame::data(Bytes::from_static(b" "))),
		Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"world"))),
	]);
	let body = crate::http::Body::new(http_body_util::StreamBody::new(frames));
	let mut req = request_with_body(body);

	policy
		.apply_to_request(&mut req)
		.await
		.expect("buffering should succeed");

	let buffered = req
		.extensions()
		.get::<BufferedRequestBody>()
		.expect("extension inserted")
		.0
		.clone();
	assert_eq!(buffered, Bytes::from_static(b"hello world"));
	// The replaced body must still hand the same bytes back to downstream readers.
	assert_eq!(
		read_body_bytes(&mut req).await,
		Bytes::from_static(b"hello world")
	);
}

#[tokio::test]
async fn apply_to_request_is_noop_when_already_buffered() {
	let policy = Buffering {
		buffer_request_body: true,
	};
	// Pre-populate the extension to mimic a previous scope (gateway) having buffered already.
	let prebuffered = Bytes::from_static(b"already-here");
	let mut req = request_with_body(crate::http::Body::from("ignored-payload"));
	req
		.extensions_mut()
		.insert(BufferedRequestBody(prebuffered.clone()));

	policy
		.apply_to_request(&mut req)
		.await
		.expect("second pass should succeed");

	// Extension is preserved as-is; the body is not re-read into it.
	assert_eq!(
		req.extensions().get::<BufferedRequestBody>().unwrap().0,
		prebuffered
	);
	// The original body is left untouched on this no-op path.
	assert_eq!(
		read_body_bytes(&mut req).await,
		Bytes::from_static(b"ignored-payload")
	);
}

#[tokio::test]
async fn apply_to_request_skips_upgrade_requests() {
	let policy = Buffering {
		buffer_request_body: true,
	};
	let mut req = ::http::Request::builder()
		.uri("http://example.com/")
		.header(::http::header::UPGRADE, "websocket")
		.body(crate::http::Body::from("payload"))
		.expect("request builds");

	policy
		.apply_to_request(&mut req)
		.await
		.expect("upgrade requests skip buffering");

	assert!(req.extensions().get::<BufferedRequestBody>().is_none());
	assert_eq!(
		read_body_bytes(&mut req).await,
		Bytes::from_static(b"payload")
	);
}

#[tokio::test]
async fn apply_to_request_fails_when_body_exceeds_buffer_limit() {
	let policy = Buffering {
		buffer_request_body: true,
	};
	let mut req = request_with_body(crate::http::Body::from("a body that is way too large"));
	// Force a small limit so the body cannot fit.
	req.extensions_mut().insert(BufferLimit(4));

	let err = policy
		.apply_to_request(&mut req)
		.await
		.expect_err("oversize body must surface as an error");

	assert!(
		matches!(err, ProxyResponse::Error(_)),
		"expected ProxyResponse::Error, got {err:?}"
	);
}
