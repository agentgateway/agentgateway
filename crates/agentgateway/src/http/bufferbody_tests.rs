use std::convert::Infallible;

use bytes::Bytes;
use http_body::{Body as _, Frame};
use http_body_util::BodyExt;

use crate::http::bufferbody;
use crate::*;

#[tokio::test]
async fn buffered_body_emits_single_final_data_frame() {
	let frames = tokio_stream::iter(vec![
		Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"hello"))),
		Ok::<_, Infallible>(Frame::data(Bytes::from_static(b" "))),
		Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"world"))),
	]);
	let inner = http::Body::new(http_body_util::StreamBody::new(frames));
	let (mut body, handle) = bufferbody::BufferedBody::new_with_limit(inner, 1024);

	let frame = body.frame().await.unwrap().unwrap();
	assert_eq!(
		frame.into_data().unwrap(),
		Bytes::from_static(b"hello world")
	);
	assert!(body.is_end_stream());
	assert_eq!(handle.bytes().unwrap(), Bytes::from_static(b"hello world"));
	assert!(body.frame().await.is_none());
}

#[tokio::test]
async fn buffered_body_rejects_oversized_body() {
	let inner = http::Body::from("hello world");
	let (mut body, handle) = bufferbody::BufferedBody::new_with_limit(inner, 5);

	let err = body.frame().await.unwrap().unwrap_err();
	assert!(err.to_string().contains("body exceeded max buffer size"));
	assert!(handle.bytes().is_none());
}

#[tokio::test]
async fn buffered_body_preserves_trailers_after_buffered_data() {
	let mut trailers = ::http::HeaderMap::new();
	trailers.insert("x-test-trailer", "value".parse().unwrap());
	let frames = tokio_stream::iter(vec![
		Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"hello"))),
		Ok::<_, Infallible>(Frame::trailers(trailers.clone())),
	]);
	let inner = http::Body::new(http_body_util::StreamBody::new(frames));
	let (mut body, handle) = bufferbody::BufferedBody::new_with_limit(inner, 1024);

	let frame = body.frame().await.unwrap().unwrap();
	assert_eq!(frame.into_data().unwrap(), Bytes::from_static(b"hello"));
	assert!(!body.is_end_stream());
	assert_eq!(handle.bytes().unwrap(), Bytes::from_static(b"hello"));

	let frame = body.frame().await.unwrap().unwrap();
	assert_eq!(frame.into_trailers().unwrap(), trailers);
	assert!(body.is_end_stream());
	assert!(body.frame().await.is_none());
}
