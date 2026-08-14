use bytes::Bytes;
use futures_util::stream::iter;
use http_body_util::{BodyExt, Full, StreamBody};
use itertools::Itertools;

use super::*;
use crate::*;

// Helper function to collect all data from a ReplayBody
async fn collect_replay_body<B: Body + Unpin>(
	mut body: ReplayBody<B>,
) -> Result<Vec<Bytes>, axum_core::Error>
where
	B::Error: Into<axum_core::Error>,
{
	let mut chunks = Vec::new();
	while let Some(frame) = body.frame().await {
		match frame {
			Ok(frame) => {
				if let Ok(data) = frame.into_data() {
					chunks.push(data);
				}
			},
			Err(e) => return Err(e),
		}
	}
	Ok(chunks)
}

fn mock_body(data: Vec<&str>) -> crate::http::Body {
	let vec = data.into_iter().map(|s| s.to_owned()).collect_vec();
	let iter = vec
		.into_iter()
		.map(|d| Ok::<_, crate::http::Error>(Frame::data(Bytes::copy_from_slice(d.as_bytes()))));
	crate::http::Body::new(StreamBody::new(futures_util::stream::iter(iter)))
}

#[tokio::test]
async fn test_basic_replay_body() {
	let mock_body = mock_body(vec!["hello", " world"]);
	let replay_body = ReplayBody::try_new(mock_body, 1024).unwrap();

	let chunks = collect_replay_body(replay_body).await.unwrap();
	assert_eq!(chunks.len(), 2);
	assert_eq!(chunks[0], Bytes::from_static(b"hello"));
	assert_eq!(chunks[1], Bytes::from_static(b" world"));
}

#[tokio::test]
async fn test_replay_body_with_clone() {
	let mock_body = mock_body(vec!["hello", " world"]);
	let replay_body = ReplayBody::try_new(mock_body, 1024).unwrap();
	let clone = replay_body.clone();

	// Read from the original
	let got = replay_body.collect().await.unwrap().to_bytes();
	assert_eq!(got, Bytes::from_static(b"hello world"));

	// Read from the clone - should replay the same data
	let got = clone.collect().await.unwrap().to_bytes();
	assert_eq!(got, Bytes::from_static(b"hello world"));
}

fn with_trailers(b: http::Body, t: HeaderMap) -> http::Body {
	http::Body::new(b.with_trailers(async move { Some(Ok(t)) }))
}

#[tokio::test]
async fn test_replay_body_with_trailers() {
	let mut trailers = HeaderMap::new();
	trailers.insert("x-test", "value".parse().unwrap());

	let mock_body = with_trailers(mock_body(vec!["hello", "world"]), trailers.clone());
	let replay_body = ReplayBody::try_new(mock_body, 1024).unwrap();
	let clone = replay_body.clone();

	// Read from the original
	{
		let read = replay_body.collect().await.unwrap();
		assert_eq!(read.trailers(), Some(&trailers));
		assert_eq!(read.to_bytes(), Bytes::from_static(b"helloworld"));
	}

	// Read from the clone
	{
		let read = clone.collect().await.unwrap();
		assert_eq!(read.trailers(), Some(&trailers));
		assert_eq!(read.to_bytes(), Bytes::from_static(b"helloworld"));
	}
}

#[tokio::test]
async fn test_replay_body_size_limit() {
	let mock_body = mock_body(vec!["hello", " world", " extra"]);
	let replay_body = ReplayBody::try_new(mock_body, 5).unwrap(); // Only buffer 5 bytes
	let clone = replay_body.clone();

	// Read from the original - should work fine
	let chunks1 = collect_replay_body(replay_body).await.unwrap();
	assert_eq!(
		chunks1,
		vec![
			Bytes::from_static(b"hello"),
			Bytes::from_static(b" world"),
			Bytes::from_static(b" extra")
		]
	);

	// Read from the clone - should fail with Capped error
	let result = collect_replay_body(clone).await;
	assert!(result.is_err());
	assert!(
		result
			.unwrap_err()
			.to_string()
			.contains("replay body discarded")
	);
}

#[tokio::test]
async fn test_replay_body_size_hint_too_large() {
	// Create a body with a known large size that exceeds our buffer limit
	let large_data = "x".repeat(1024); // 1KB of data
	let mock_body = http::Body::new(Full::new(Bytes::copy_from_slice(large_data.as_bytes())));

	// This should fail because the body size (1024 bytes) is larger than our buffer limit (5 bytes)
	let result = ReplayBody::try_new(mock_body, 5);
	assert!(result.is_err());
}

#[tokio::test]
async fn test_replay_body_empty() {
	let mock_body = http::Body::new(Full::new(Bytes::new()));

	let replay_body = ReplayBody::try_new(mock_body, 1024).unwrap();
	let clone = replay_body.clone();

	// Both original and clone should be empty
	assert!(replay_body.is_end_stream());
	assert!(clone.is_end_stream());

	let chunks1 = collect_replay_body(replay_body).await.unwrap();
	let chunks2 = collect_replay_body(clone).await.unwrap();
	assert!(chunks1.is_empty());
	assert!(chunks2.is_empty());
}

#[tokio::test]
async fn test_replay_body_reports_end_stream_after_inner_eof() {
	let inner = crate::http::Body::new(StreamBody::new(futures_util::stream::unfold(
		false,
		|emitted| async move {
			if emitted {
				None
			} else {
				Some((
					Ok::<_, crate::http::Error>(Frame::data(Bytes::from_static(b"body"))),
					true,
				))
			}
		},
	)));
	let mut body = ReplayBody::try_new(inner, 1024).unwrap();

	assert!(body.frame().await.is_some(), "data frame should be present");
	assert!(body.frame().await.is_none(), "stream should report EOF");
	assert!(
		body.is_end_stream(),
		"completed body should report end-of-stream"
	);
}

#[tokio::test]
async fn test_replay_body_error_propagation() {
	// Create a body that will error by using a stream that fails
	let stream = iter(vec![
		Ok::<_, crate::http::Error>(Frame::data(Bytes::from_static(b"hello"))),
		Err::<Frame<Bytes>, _>(crate::http::Error::new("mock error")),
	]);
	let error_body = crate::http::Body::new(StreamBody::new(stream));

	let replay_body = ReplayBody::try_new(error_body, 1024).unwrap();

	let result = collect_replay_body(replay_body).await;
	assert!(result.is_err());
}

#[tokio::test]
async fn test_replay_body_multiple_clones() {
	let mock_body = mock_body(vec!["hello", " world"]);
	let replay_body = ReplayBody::try_new(mock_body, 1024).unwrap();
	let clone1 = replay_body.clone();
	let clone2 = replay_body.clone();

	// Read from the original
	let chunks1 = collect_replay_body(replay_body).await.unwrap();
	assert_eq!(
		chunks1,
		vec![Bytes::from_static(b"hello"), Bytes::from_static(b" world")]
	);

	// Read from first clone
	let chunks2 = collect_replay_body(clone1).await.unwrap();
	assert_eq!(
		chunks2,
		vec![Bytes::from_static(b"hello"), Bytes::from_static(b" world")]
	);

	// Read from second clone
	let chunks3 = collect_replay_body(clone2).await.unwrap();
	assert_eq!(
		chunks3,
		vec![Bytes::from_static(b"hello"), Bytes::from_static(b" world")]
	);

	// All should have the same data
	assert_eq!(chunks1, chunks2);
	assert_eq!(chunks2, chunks3);
}

#[tokio::test]
async fn test_replay_body_size_hint() {
	let mock_body = http::Body::new(Full::new(Bytes::from_static(b"hello world")));
	let replay_body = ReplayBody::try_new(mock_body, 1024).unwrap();

	let size_hint = replay_body.size_hint();
	assert_eq!(size_hint.lower(), 11); // "hello world".len()
	assert_eq!(size_hint.upper(), Some(11));
}

#[tokio::test]
async fn test_replay_body_is_capped() {
	let mock_body = mock_body(vec!["hello", " world", " extra"]);
	let replay_body = ReplayBody::try_new(mock_body, 5).unwrap(); // Only buffer 5 bytes
	let clone = replay_body.clone();

	// Original should not be capped initially
	assert_eq!(replay_body.is_capped(), Some(false));

	// Read from original
	let _chunks = collect_replay_body(replay_body).await.unwrap();

	// Clone should be capped after original is consumed
	assert_eq!(clone.is_capped(), Some(true));
}

#[tokio::test]
async fn test_replay_body_large_data() {
	let large_data = ["a".repeat(1000), "b".repeat(1000)];
	let mock_body = mock_body(large_data.iter().map(|s| s.as_str()).collect());
	let replay_body = ReplayBody::try_new(mock_body, 1500).unwrap(); // Buffer limit between chunks
	let clone = replay_body.clone();

	// Read from original
	let chunks1 = collect_replay_body(replay_body).await.unwrap();
	assert_eq!(chunks1.len(), 2);
	assert_eq!(chunks1[0].len(), 1000);
	assert_eq!(chunks1[1].len(), 1000);

	// Clone should be capped and fail
	let result = collect_replay_body(clone).await;
	assert!(result.is_err());
}
