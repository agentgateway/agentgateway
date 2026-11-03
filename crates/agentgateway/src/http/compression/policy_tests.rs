use http::header::{CACHE_CONTROL, CONTENT_TYPE, VARY};
use http_body_util::BodyExt;

use super::*;
use crate::http::compression::encode_body;

const JSON: &str = r#"{"message":"a sufficiently long response body for compression"}"#;

fn compressor(
	accept_encoding: &'static str,
	algorithms: Vec<CompressionAlgorithm>,
) -> ResponseCompressor {
	let req = ::http::Request::builder()
		.header(ACCEPT_ENCODING, accept_encoding)
		.body(Body::empty())
		.unwrap();
	ResponseCompression { algorithms }
		.for_request(&req)
		.unwrap()
}

fn response(content_type: &str, body: &'static str) -> Response {
	::http::Response::builder()
		.header(CONTENT_TYPE, content_type)
		.header(CONTENT_LENGTH, body.len())
		.body(Body::from(body))
		.unwrap()
}

fn request(encoding: &str, body: bytes::Bytes) -> Request {
	::http::Request::builder()
		.header(CONTENT_ENCODING, encoding)
		.header(CONTENT_LENGTH, body.len())
		.body(Body::from(body))
		.unwrap()
}

fn all() -> [(CompressionAlgorithm, &'static str); 4] {
	[
		(CompressionAlgorithm::Gzip, GZIP),
		(CompressionAlgorithm::Brotli, BR),
		(CompressionAlgorithm::Deflate, DEFLATE),
		(CompressionAlgorithm::Zstd, ZSTD),
	]
}

#[tokio::test]
async fn response_compression_round_trips_every_algorithm() {
	for (algorithm, name) in all() {
		let mut resp = response("application/json; charset=utf-8", JSON);
		compressor(name, vec![algorithm]).apply(&mut resp);

		assert_eq!(resp.headers()[CONTENT_ENCODING], name);
		assert!(
			resp.headers()[VARY]
				.to_str()
				.unwrap()
				.eq_ignore_ascii_case("accept-encoding")
		);
		assert!(!resp.headers().contains_key(CONTENT_LENGTH));
		let ce = resp.headers().typed_get::<ContentEncoding>();
		let (body, _) = decompress_body(resp.into_body(), ce.as_ref()).unwrap();
		assert_eq!(body.collect().await.unwrap().to_bytes(), JSON);
	}
}

#[test]
fn response_compression_only_offers_configured_algorithms() {
	let mut resp = response("application/json", JSON);
	compressor("br", vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
	assert!(!resp.headers().contains_key(CONTENT_ENCODING));
	assert_eq!(resp.status(), StatusCode::OK);
}

#[test]
fn response_compression_keeps_upstream_status_when_nothing_acceptable() {
	let mut resp = response("application/json", JSON);
	compressor("identity;q=0", vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
	assert_eq!(resp.status(), StatusCode::OK);
	assert!(!resp.headers().contains_key(CONTENT_ENCODING));
}

#[test]
fn response_compression_skips_ineligible_responses() {
	let skipped = [
		response("application/json", "tiny"),
		response("application/octet-stream", JSON),
		response("text/event-stream", JSON),
		{
			let mut r = response("application/json", JSON);
			r.headers_mut().insert(
				CACHE_CONTROL,
				HeaderValue::from_static("public, no-transform"),
			);
			r
		},
		{
			let mut r = response("application/json", JSON);
			r.headers_mut()
				.insert(CONTENT_ENCODING, HeaderValue::from_static(BR));
			r
		},
		{
			let mut r = response("application/json", JSON);
			*r.status_mut() = StatusCode::NOT_MODIFIED;
			r
		},
	];
	for mut resp in skipped {
		let before = resp.headers().get(CONTENT_ENCODING).cloned();
		compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
		assert_eq!(resp.headers().get(CONTENT_ENCODING).cloned(), before);
	}
}

#[test]
fn response_compression_weakens_strong_etag() {
	let mut resp = response("application/problem+json", JSON);
	resp
		.headers_mut()
		.insert(ETAG, HeaderValue::from_static("\"v1\""));
	compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
	assert_eq!(resp.headers()[ETAG], "W/\"v1\"");
}

#[test]
fn response_compression_keeps_weak_etag_and_merges_vary() {
	let mut resp = response("application/json", JSON);
	resp
		.headers_mut()
		.insert(ETAG, HeaderValue::from_static("W/\"v1\""));
	resp
		.headers_mut()
		.insert(VARY, HeaderValue::from_static("Origin"));
	compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
	assert_eq!(resp.headers()[ETAG], "W/\"v1\"");
	let vary: Vec<_> = resp.headers().get_all(VARY).iter().collect();
	assert!(vary.iter().any(|v| *v == "Origin"));
	assert!(
		vary
			.iter()
			.any(|v| v.to_str().unwrap().eq_ignore_ascii_case("accept-encoding"))
	);
}

#[test]
fn response_compression_negotiates_wildcard_and_repeated_headers() {
	let mut resp = response("application/json", JSON);
	compressor("*", vec![CompressionAlgorithm::Zstd]).apply(&mut resp);
	assert_eq!(resp.headers()[CONTENT_ENCODING], ZSTD);

	let req = ::http::Request::builder()
		.header(ACCEPT_ENCODING, "gzip;q=0.1")
		.header(ACCEPT_ENCODING, "br")
		.body(Body::empty())
		.unwrap();
	let mut resp = response("application/json", JSON);
	ResponseCompression {
		algorithms: vec![CompressionAlgorithm::Gzip, CompressionAlgorithm::Brotli],
	}
	.for_request(&req)
	.unwrap()
	.apply(&mut resp);
	assert_eq!(resp.headers()[CONTENT_ENCODING], BR);
}

#[test]
fn response_compression_skips_streaming_bodies() {
	let mut resp = response("text/plain", JSON);
	resp.headers_mut().remove(CONTENT_LENGTH);
	compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
	assert!(!resp.headers().contains_key(CONTENT_ENCODING));
}

#[test]
fn response_compression_head_matches_get_headers() {
	let policy = ResponseCompression {
		algorithms: vec![CompressionAlgorithm::Gzip],
	};
	let head = ::http::Request::builder()
		.method(http::Method::HEAD)
		.header(ACCEPT_ENCODING, GZIP)
		.body(Body::empty())
		.unwrap();
	let mut resp = ::http::Response::builder()
		.header(CONTENT_TYPE, "application/json")
		.header(CONTENT_LENGTH, JSON.len())
		.body(Body::empty())
		.unwrap();
	policy.for_request(&head).unwrap().apply(&mut resp);
	assert_eq!(resp.headers()[CONTENT_ENCODING], GZIP);
	assert!(resp.headers().contains_key(VARY));
	assert!(policy.for_request(&Request::new(Body::empty())).is_none());
}

#[tokio::test]
async fn request_decompression_decodes_every_algorithm() {
	let original = b"request body decompression";
	for (algorithm, name) in all() {
		let mut req = request(name, encode_body(original, name).await.unwrap());
		RequestDecompression {
			algorithms: vec![algorithm],
		}
		.apply(&mut req)
		.unwrap();

		assert!(!req.headers().contains_key(CONTENT_ENCODING));
		assert!(!req.headers().contains_key(CONTENT_LENGTH));
		assert_eq!(
			req.into_body().collect().await.unwrap().to_bytes(),
			&original[..]
		);
	}
}

#[tokio::test]
async fn request_decompression_passes_through_unaccepted_or_layered() {
	let encoded = encode_body(b"leave compressed", BR).await.unwrap();
	let gzip_only = RequestDecompression {
		algorithms: vec![CompressionAlgorithm::Gzip],
	};

	let mut req = request(BR, encoded.clone());
	gzip_only.apply(&mut req).unwrap();
	assert_eq!(req.headers()[CONTENT_ENCODING], BR);
	assert_eq!(req.into_body().collect().await.unwrap().to_bytes(), encoded);

	let mut req = request(GZIP, encoded.clone());
	req
		.headers_mut()
		.append(CONTENT_ENCODING, HeaderValue::from_static(GZIP));
	gzip_only.apply(&mut req).unwrap();
	assert_eq!(req.headers().get_all(CONTENT_ENCODING).iter().count(), 2);
	assert_eq!(req.into_body().collect().await.unwrap().to_bytes(), encoded);
}

#[tokio::test]
async fn request_decompression_invalidates_cached_inspection() {
	let original = b"decoded body seen by later policies";
	let encoded = encode_body(original, GZIP).await.unwrap();
	let mut req = request(GZIP, encoded.clone());
	// An earlier policy (e.g. gateway-level CEL) already buffered the encoded body.
	let _ = crate::http::inspect_body(&mut req).await.unwrap();
	assert_eq!(req.body().known_bytes(), Some(&encoded));

	RequestDecompression {
		algorithms: vec![CompressionAlgorithm::Gzip],
	}
	.apply(&mut req)
	.unwrap();
	assert!(req.body().inspection().is_none());
	let inspection = crate::http::inspect_body(&mut req).await.unwrap();
	assert!(
		matches!(inspection, crate::http::BodyInspection::Complete(b) if b == original.as_slice())
	);
}

#[test]
fn response_compression_skips_ranges_and_drops_accept_ranges() {
	let mut partial = response("application/json", JSON);
	*partial.status_mut() = StatusCode::PARTIAL_CONTENT;
	partial.headers_mut().insert(
		http::header::CONTENT_RANGE,
		HeaderValue::from_static("bytes 0-63/1000"),
	);
	compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut partial);
	assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
	assert!(!partial.headers().contains_key(CONTENT_ENCODING));

	// Byte ranges of the encoded representation can't be served, so stop advertising them.
	let mut full = response("application/json", JSON);
	full.headers_mut().insert(
		http::header::ACCEPT_RANGES,
		HeaderValue::from_static("bytes"),
	);
	compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut full);
	assert_eq!(full.headers()[CONTENT_ENCODING], GZIP);
	assert!(!full.headers().contains_key(http::header::ACCEPT_RANGES));
}

fn with_trailers(content_length: Option<usize>) -> Response {
	let mut trailers = HeaderMap::new();
	trailers.insert("x-checksum", HeaderValue::from_static("abc"));
	let frames = vec![
		Ok::<_, Infallible>(http_body::Frame::data(bytes::Bytes::from_static(
			JSON.as_bytes(),
		))),
		Ok(http_body::Frame::trailers(trailers)),
	];
	let mut builder = ::http::Response::builder().header(CONTENT_TYPE, "application/json");
	if let Some(len) = content_length {
		builder = builder.header(CONTENT_LENGTH, len);
	}
	builder
		.body(Body::new(http_body_util::StreamBody::new(
			futures_util::stream::iter(frames),
		)))
		.unwrap()
}

#[tokio::test]
async fn response_compression_preserves_trailers() {
	// Known length: compressed, trailers still delivered after the encoded data.
	let mut resp = with_trailers(Some(JSON.len()));
	compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
	assert_eq!(resp.headers()[CONTENT_ENCODING], GZIP);
	let collected = resp.into_body().collect().await.unwrap();
	assert_eq!(collected.trailers().unwrap()["x-checksum"], "abc");
	let ce = headers::ContentEncoding::gzip();
	let (body, _) = decompress_body(Body::from(collected.to_bytes()), Some(&ce)).unwrap();
	assert_eq!(body.collect().await.unwrap().to_bytes(), JSON);

	// Chunked stream: passed through untouched, trailers included.
	let mut resp = with_trailers(None);
	compressor(GZIP, vec![CompressionAlgorithm::Gzip]).apply(&mut resp);
	assert!(!resp.headers().contains_key(CONTENT_ENCODING));
	let collected = resp.into_body().collect().await.unwrap();
	assert_eq!(collected.trailers().unwrap()["x-checksum"], "abc");
	assert_eq!(collected.to_bytes(), JSON);
}
