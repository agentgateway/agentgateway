use async_compression::tokio::bufread::{
	BrotliDecoder, BrotliEncoder, GzipDecoder, GzipEncoder, ZlibDecoder, ZlibEncoder, ZstdDecoder,
	ZstdEncoder,
};
use bytes::Bytes;
use futures_util::TryStreamExt;
use headers::{ContentEncoding, Header};
use http::header::{
	ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE,
};
use http_body::Body;
use http_body_util::BodyExt;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio_util::io::{ReaderStream, StreamReader};
use tower::ServiceExt;
use tower_http::compression::Compression as CompressionService;
use tower_http::compression::predicate::SizeAbove;
use tower_http::decompression::RequestDecompression as RequestDecompressionService;

use crate::{apply, schema};

const GZIP: &str = "gzip";
const DEFLATE: &str = "deflate";
const BR: &str = "br";
const ZSTD: &str = "zstd";
// Compression framing can make very small payloads larger; 30 bytes avoids that common case.
const MIN_CONTENT_LENGTH: u64 = 30;

// Restrict transparent compression to textual formats that generally benefit from it. Binary
// media is commonly already compressed, while latency-sensitive SSE should avoid encoder buffering.
const COMPRESSIBLE_CONTENT_TYPES: &[&str] = &[
	"application/javascript",
	"application/json",
	"application/xhtml+xml",
	"image/svg+xml",
	"text/css",
	"text/html",
	"text/plain",
	"text/xml",
];

/// An HTTP compression algorithm, named after the token used in the `Content-Encoding` and
/// `Accept-Encoding` headers.
#[apply(schema!)]
#[derive(Copy, Eq, PartialEq)]
pub enum CompressionAlgorithm {
	Gzip,
	Brotli,
	Deflate,
	Zstd,
}

impl CompressionAlgorithm {
	fn as_str(self) -> &'static str {
		match self {
			Self::Gzip => GZIP,
			Self::Brotli => BR,
			Self::Deflate => DEFLATE,
			Self::Zstd => ZSTD,
		}
	}
}

#[apply(schema!)]
#[derive(Default)]
pub struct Compression {
	/// Compress response bodies sent to the client.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_compression: Option<ResponseCompression>,
	/// Decompress request bodies before other policies inspect them and before forwarding.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request_decompression: Option<RequestDecompression>,
}

#[apply(schema!)]
pub struct ResponseCompression {
	/// Algorithms offered when negotiating against the client's Accept-Encoding header.
	/// Defaults to gzip.
	#[serde(default = "default_algorithms")]
	pub preferred_algorithms: Vec<CompressionAlgorithm>,
}

impl Default for ResponseCompression {
	fn default() -> Self {
		Self {
			preferred_algorithms: default_algorithms(),
		}
	}
}

impl ResponseCompression {
	pub fn for_request(&self, req: &crate::http::Request) -> Option<ResponseCompressor> {
		if req.method() == http::Method::HEAD || self.preferred_algorithms.is_empty() {
			return None;
		}
		let mut accept_encoding = http::HeaderMap::new();
		for value in req.headers().get_all(ACCEPT_ENCODING) {
			accept_encoding.append(ACCEPT_ENCODING, value.clone());
		}
		Some(ResponseCompressor {
			preferred_algorithms: self.preferred_algorithms.clone(),
			accept_encoding,
		})
	}
}

#[apply(schema!)]
pub struct RequestDecompression {
	/// Algorithms decoded from request bodies. Bodies encoded with any other algorithm are
	/// forwarded untouched. Defaults to gzip.
	#[serde(default = "default_algorithms")]
	pub accepted_algorithms: Vec<CompressionAlgorithm>,
}

impl Default for RequestDecompression {
	fn default() -> Self {
		Self {
			accepted_algorithms: default_algorithms(),
		}
	}
}

impl RequestDecompression {
	pub async fn apply_to_request(&self, req: &mut crate::http::Request) -> Result<(), Error> {
		let Some(raw) = sole_content_encoding(req.headers()) else {
			return Ok(());
		};
		let Some(algorithm) = compression_algorithm(raw) else {
			return Ok(());
		};
		if !self.accepted_algorithms.contains(&algorithm) {
			return Ok(());
		};
		req.headers_mut().insert(
			CONTENT_ENCODING,
			http::HeaderValue::from_static(algorithm.as_str()),
		);

		let request = std::mem::replace(req, crate::http::Request::new(crate::http::Body::empty()));
		let (sender, receiver) = tokio::sync::oneshot::channel();
		let mut sender = Some(sender);
		let service = RequestDecompressionService::new(tower::service_fn(
			move |request: ::http::Request<
				tower_http::decompression::DecompressionBody<crate::http::Body>,
			>| {
				let sender = sender.take().expect("decompression service called once");
				async move {
					let (parts, body) = request.into_parts();
					sender
						.send(crate::http::Request::from_parts(
							parts,
							crate::http::Body::new(body),
						))
						.expect("decompressed request receiver is open");
					Ok::<_, std::convert::Infallible>(crate::http::Response::new(crate::http::Body::empty()))
				}
			},
		))
		.pass_through_unaccepted(true)
		.gzip(
			self
				.accepted_algorithms
				.contains(&CompressionAlgorithm::Gzip),
		)
		.br(
			self
				.accepted_algorithms
				.contains(&CompressionAlgorithm::Brotli),
		)
		.deflate(
			self
				.accepted_algorithms
				.contains(&CompressionAlgorithm::Deflate),
		)
		.zstd(
			self
				.accepted_algorithms
				.contains(&CompressionAlgorithm::Zstd),
		);
		service
			.oneshot(request)
			.await
			.expect("decompression service is infallible");
		*req = receiver
			.await
			.expect("decompression service forwards the request");
		Ok(())
	}
}

impl crate::store::RequestPolicyTrait for Compression {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		if let Some(decompression) = &self.request_decompression {
			decompression
				.apply_to_request(req)
				.await
				.map_err(|error| crate::proxy::ProxyError::Processing(error.into()))?;
		}
		Ok(Default::default())
	}
}

#[derive(Debug, Clone)]
pub struct ResponseCompressor {
	preferred_algorithms: Vec<CompressionAlgorithm>,
	accept_encoding: http::HeaderMap,
}

impl ResponseCompressor {
	pub async fn apply(self, resp: &mut crate::http::Response) -> Result<(), Error> {
		if !should_compress_response(resp) {
			return Ok(());
		}
		// The encoder sets its own Content-Encoding, so drop a no-op identity value first.
		if matches!(
			content_encoding_state(resp.headers()),
			ContentEncodingState::Identity
		) {
			resp.headers_mut().remove(CONTENT_ENCODING);
		}

		let response = std::mem::replace(resp, crate::http::Response::new(crate::http::Body::empty()));
		let mut response = Some(response);
		let service = CompressionService::new(tower::service_fn(move |_| {
			std::future::ready(Ok::<_, std::convert::Infallible>(
				response.take().expect("compression service called once"),
			))
		}))
		.gzip(
			self
				.preferred_algorithms
				.contains(&CompressionAlgorithm::Gzip),
		)
		.br(
			self
				.preferred_algorithms
				.contains(&CompressionAlgorithm::Brotli),
		)
		.deflate(
			self
				.preferred_algorithms
				.contains(&CompressionAlgorithm::Deflate),
		)
		.zstd(
			self
				.preferred_algorithms
				.contains(&CompressionAlgorithm::Zstd),
		)
		.compress_when(SizeAbove::new(0));
		let mut request = crate::http::Request::new(crate::http::Body::empty());
		*request.headers_mut() = self.accept_encoding;
		let response = service
			.oneshot(request)
			.await
			.expect("compression service is infallible");
		let (parts, body) = response.into_parts();
		*resp = crate::http::Response::from_parts(parts, crate::http::Body::new(body));
		Ok(())
	}
}

fn default_algorithms() -> Vec<CompressionAlgorithm> {
	vec![CompressionAlgorithm::Gzip]
}

/// Returns the sole `Content-Encoding` value, if the message carries exactly one.
///
/// Repeated headers describe layered encodings, which this policy leaves untouched: decoding or
/// replacing a single layer would misrepresent the body.
fn sole_content_encoding(headers: &http::HeaderMap) -> Option<&str> {
	let mut values = headers.get_all(CONTENT_ENCODING).into_iter();
	let value = values.next()?;
	if values.next().is_some() {
		return None;
	}
	value.to_str().ok()
}

enum ContentEncodingState {
	Absent,
	Identity,
	Encoded,
}

fn content_encoding_state(headers: &http::HeaderMap) -> ContentEncodingState {
	if !headers.contains_key(CONTENT_ENCODING) {
		return ContentEncodingState::Absent;
	}
	match sole_content_encoding(headers) {
		Some(value) if value.trim().eq_ignore_ascii_case("identity") => ContentEncodingState::Identity,
		// A comma-separated list, repeated headers, or a non-UTF-8 value all mean the body is
		// already encoded in some way we should not layer on top of.
		_ => ContentEncodingState::Encoded,
	}
}

fn compression_algorithm(raw: &str) -> Option<CompressionAlgorithm> {
	let raw = raw.trim();
	if raw.eq_ignore_ascii_case(GZIP) {
		Some(CompressionAlgorithm::Gzip)
	} else if raw.eq_ignore_ascii_case(BR) {
		Some(CompressionAlgorithm::Brotli)
	} else if raw.eq_ignore_ascii_case(DEFLATE) {
		Some(CompressionAlgorithm::Deflate)
	} else if raw.eq_ignore_ascii_case(ZSTD) {
		Some(CompressionAlgorithm::Zstd)
	} else {
		None
	}
}

fn should_compress_response(resp: &crate::http::Response) -> bool {
	let status = resp.status();
	if status.is_informational()
		|| status == http::StatusCode::NO_CONTENT
		|| status == http::StatusCode::NOT_MODIFIED
	{
		return false;
	}
	if matches!(
		content_encoding_state(resp.headers()),
		ContentEncodingState::Encoded
	) {
		return false;
	}
	if resp
		.headers()
		.get_all(CACHE_CONTROL)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(','))
		.any(|directive| directive.trim().eq_ignore_ascii_case("no-transform"))
	{
		return false;
	}
	if resp
		.headers()
		.get(CONTENT_LENGTH)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<u64>().ok())
		.is_some_and(|length| length < MIN_CONTENT_LENGTH)
	{
		return false;
	}

	resp
		.headers()
		.get(CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.split(';').next())
		.map(str::trim)
		.is_some_and(|content_type| {
			COMPRESSIBLE_CONTENT_TYPES
				.iter()
				.any(|allowed| content_type.eq_ignore_ascii_case(allowed))
		})
}

/// Errors that can occur during compression/decompression operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("unsupported content encoding")]
	UnsupportedEncoding,
	#[error("body exceeded buffer limit")]
	LimitExceeded,
	#[error("decompression failed: {0}")]
	Io(#[from] std::io::Error),
	#[error("body read error: {0}")]
	Body(#[from] axum_core::Error),
}

impl From<Error> for axum_core::Error {
	fn from(e: Error) -> Self {
		axum_core::Error::new(e)
	}
}

enum EncodingDecision {
	None,
	Single(&'static str),
	Multiple,
	Unsupported,
}

/// Detects which single supported encoding is present in the Content-Encoding header.
///
/// Returns `Single(encoding)` if exactly one supported encoding is present.
/// Returns `None` if no encoding (or only `identity`) is present.
/// Returns `Multiple` if multiple encodings are present (chain decoding unsupported).
/// Returns `Unsupported` if an unknown encoding is present.
fn detect_encoding(ce: &ContentEncoding) -> EncodingDecision {
	let mut values = Vec::new();
	ce.encode(&mut values);
	let Some(value) = values.first() else {
		return EncodingDecision::None;
	};
	let Ok(raw) = value.to_str() else {
		return EncodingDecision::Unsupported;
	};

	let mut supported_count = 0;
	let mut single_supported = None;
	let mut has_unknown = false;

	for token in raw.split(',') {
		let token = token.trim();
		if token.is_empty() {
			continue;
		}
		if token.eq_ignore_ascii_case("identity") {
			// identity is a no-op encoding (RFC 9110 §8.4.1), skip it so
			// "identity, gzip" is treated the same as "gzip".
			continue;
		}

		if token.eq_ignore_ascii_case(GZIP) {
			supported_count += 1;
			single_supported = Some(GZIP);
		} else if token.eq_ignore_ascii_case(DEFLATE) {
			supported_count += 1;
			single_supported = Some(DEFLATE);
		} else if token.eq_ignore_ascii_case(BR) {
			supported_count += 1;
			single_supported = Some(BR);
		} else if token.eq_ignore_ascii_case(ZSTD) {
			supported_count += 1;
			single_supported = Some(ZSTD);
		} else {
			has_unknown = true;
		}
	}

	if has_unknown {
		return EncodingDecision::Unsupported;
	}

	// Strict policy: identity-only => None; >1 supported => Multiple.
	if supported_count == 0 {
		return EncodingDecision::None;
	}

	if supported_count > 1 {
		return EncodingDecision::Multiple;
	}

	match single_supported {
		Some(enc) => EncodingDecision::Single(enc),
		None => EncodingDecision::Unsupported,
	}
}

/// Decompresses an HTTP body stream, returning a new body that yields decompressed chunks.
///
/// Use this for streaming responses (SSE, large files) where you can't buffer the entire body.
/// If encoding is None or identity, returns the body unchanged.
/// If encoding is unsupported or multi-encoded, returns an error.
pub fn decompress_body<B>(
	body: B,
	encoding: Option<&ContentEncoding>,
) -> Result<(axum_core::body::Body, Option<&'static str>), Error>
where
	B: Body<Data = Bytes> + Send + Unpin + 'static,
	B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
	match encoding {
		None => Ok((axum_core::body::Body::new(body), None)),
		Some(ce) => match detect_encoding(ce) {
			EncodingDecision::Single(enc) => {
				decompress_body_with_encoding(body, enc).map(|b| (b, Some(enc)))
			},
			EncodingDecision::None => Ok((axum_core::body::Body::new(body), None)),
			EncodingDecision::Multiple | EncodingDecision::Unsupported => Err(Error::UnsupportedEncoding),
		},
	}
}

fn decompress_body_with_encoding<B>(body: B, encoding: &str) -> Result<axum_core::body::Body, Error>
where
	B: Body + Send + Unpin + 'static,
	B::Data: Send,
	B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
	let byte_stream = body.into_data_stream().map_err(std::io::Error::other);
	let stream_reader = BufReader::new(StreamReader::new(byte_stream));

	let decoder: Box<dyn AsyncRead + Unpin + Send> = match encoding {
		GZIP => Box::new(GzipDecoder::new(stream_reader)),
		DEFLATE => Box::new(ZlibDecoder::new(stream_reader)),
		BR => Box::new(BrotliDecoder::new(stream_reader)),
		ZSTD => Box::new(ZstdDecoder::new(stream_reader)),
		_ => return Err(Error::UnsupportedEncoding),
	};

	Ok(axum_core::body::Body::from_stream(ReaderStream::new(
		decoder,
	)))
}

pub async fn to_bytes_with_decompression(
	body: axum_core::body::Body,
	encoding: Option<&ContentEncoding>,
	limit: usize,
) -> Result<(Option<&'static str>, Bytes), Error> {
	match encoding {
		None => {
			// No encoding - use optimized direct body read
			Ok((None, read_body_with_limit(body, limit).await?))
		},
		Some(ce) => match detect_encoding(ce) {
			EncodingDecision::Single(enc) => Ok((Some(enc), decode_body(body, enc, limit).await?)),
			EncodingDecision::None => Ok((None, read_body_with_limit(body, limit).await?)),
			EncodingDecision::Multiple | EncodingDecision::Unsupported => Err(Error::UnsupportedEncoding),
		},
	}
}

pub async fn encode_body(body: &[u8], encoding: &str) -> Result<Bytes, axum_core::Error> {
	let reader = BufReader::new(body);

	let encoder: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match encoding {
		GZIP => Box::new(GzipEncoder::new(reader)),
		DEFLATE => Box::new(ZlibEncoder::new(reader)),
		BR => Box::new(BrotliEncoder::new(reader)),
		ZSTD => Box::new(ZstdEncoder::new(reader)),
		_ => return Err(Error::UnsupportedEncoding.into()),
	};

	// Preallocate assuming ~50% compression (it can grow if we are wrong)
	read_to_bytes(encoder, body.len() / 2)
		.await
		.map_err(Into::into)
}

async fn decode_body<B>(body: B, encoding: &str, limit: usize) -> Result<Bytes, Error>
where
	B: Body<Data = Bytes> + Send + Unpin + 'static,
	B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
	// Compose streaming decompression with optimized body reading
	let decompressed = decompress_body_with_encoding(body, encoding)?;
	read_body_with_limit(decompressed, limit).await
}

async fn read_to_bytes<R>(mut reader: R, initial_capacity: usize) -> Result<Bytes, Error>
where
	R: AsyncRead + Unpin,
{
	let mut buffer = bytes::BytesMut::with_capacity(initial_capacity);
	loop {
		let n = reader.read_buf(&mut buffer).await?;
		if n == 0 {
			break;
		}
	}
	Ok(buffer.freeze())
}

async fn read_body_with_limit(body: axum_core::body::Body, limit: usize) -> Result<Bytes, Error> {
	crate::http::read_body_with_limit(body, limit)
		.await
		.map_err(map_body_error)
}

fn map_body_error(err: axum_core::Error) -> Error {
	if is_length_limit_error(&err) {
		Error::LimitExceeded
	} else {
		Error::Body(err)
	}
}

fn is_length_limit_error(err: &axum_core::Error) -> bool {
	use std::error::Error as _;

	err
		.source()
		.is_some_and(|source| source.is::<http_body_util::LengthLimitError>())
}

#[cfg(test)]
mod tests {
	use headers::HeaderMapExt;
	use http::header::VARY;
	use http_body_util::BodyExt;

	use super::*;
	use crate::http::Body;

	#[tokio::test]
	async fn test_decompress_unsupported() {
		let body = Body::from("hello");
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_static("unsupported"),
		);
		let ce = headers.typed_get::<ContentEncoding>().unwrap();
		let result = decompress_body(body, Some(&ce));
		assert!(matches!(result, Err(Error::UnsupportedEncoding)));
	}

	#[tokio::test]
	async fn test_to_bytes_limit_exceeded() {
		let body = Body::from("this is too long");
		let result = to_bytes_with_decompression(body, None, 5).await;
		assert!(matches!(result, Err(Error::LimitExceeded)));
	}

	#[tokio::test]
	async fn test_to_bytes_unsupported() {
		let body = Body::from("hello");
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_static("unsupported"),
		);
		let ce = headers.typed_get::<ContentEncoding>().unwrap();
		let result = to_bytes_with_decompression(body, Some(&ce), 100).await;
		assert!(matches!(result, Err(Error::UnsupportedEncoding)));
	}

	#[tokio::test]
	async fn test_identity_passthrough() {
		let body = Body::from("hello");
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_static("identity"),
		);
		let ce = headers.typed_get::<ContentEncoding>().unwrap();
		let (encoding, bytes) = to_bytes_with_decompression(body, Some(&ce), 100)
			.await
			.unwrap();
		assert!(encoding.is_none());
		assert_eq!(bytes, Bytes::from_static(b"hello"));
	}

	#[tokio::test]
	async fn test_multi_encoding_rejected() {
		// Multiple encodings (e.g., "gzip, br") should be rejected since we don't
		// support chain decoding
		let body = Body::from("hello");
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_static("gzip, br"),
		);
		let ce = headers.typed_get::<ContentEncoding>().unwrap();
		let result = to_bytes_with_decompression(body, Some(&ce), 100).await;
		assert!(matches!(result, Err(Error::UnsupportedEncoding)));
	}

	#[tokio::test]
	async fn test_identity_gzip_allowed() {
		// identity, gzip should be treated as gzip (identity is a no-op per RFC 9110)
		let original = b"hello world";
		let compressed = encode_body(original, GZIP).await.unwrap();
		let body = Body::from(compressed);
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_static("identity, gzip"),
		);
		let ce = headers.typed_get::<ContentEncoding>().unwrap();
		let (decompressed_body, encoding) = decompress_body(body, Some(&ce)).unwrap();
		let bytes = decompressed_body.collect().await.unwrap().to_bytes();
		assert_eq!(bytes, original.as_slice());
		assert_eq!(encoding, Some(GZIP));
	}

	fn make_content_encoding(enc: &str) -> ContentEncoding {
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_str(enc).unwrap(),
		);
		headers.typed_get::<ContentEncoding>().unwrap()
	}

	#[tokio::test]
	async fn test_streaming_decompression_round_trip() {
		// Test decompress_body (streaming path used for SSE/MCP)
		let original = b"hello world from a streaming decompressor test";
		let compressed = encode_body(original, GZIP).await.unwrap();
		let body = Body::from(compressed);
		let ce = make_content_encoding(GZIP);
		let (decompressed_body, enc) = decompress_body(body, Some(&ce)).unwrap();
		let bytes = decompressed_body.collect().await.unwrap().to_bytes();
		assert_eq!(bytes, original.as_slice());
		assert_eq!(enc, Some(GZIP));
	}

	#[tokio::test]
	async fn test_streaming_decompression_none_passthrough() {
		// decompress_body with no encoding returns the body unchanged
		let body = Body::from("hello");
		let (body, enc) = decompress_body(body, None).unwrap();
		let bytes = body.collect().await.unwrap().to_bytes();
		assert_eq!(bytes.as_ref(), b"hello");
		assert!(enc.is_none());
	}

	#[tokio::test]
	async fn test_buffered_decompression_round_trip() {
		// Test to_bytes_with_decompression (buffered path used for non-streaming LLM responses)
		let original = b"buffered decompression test payload";
		let compressed = encode_body(original, GZIP).await.unwrap();
		let body = Body::from(compressed);
		let ce = make_content_encoding(GZIP);
		let (enc, bytes) = to_bytes_with_decompression(body, Some(&ce), 1024)
			.await
			.unwrap();
		assert_eq!(bytes, original.as_slice());
		assert_eq!(enc, Some(GZIP));
	}

	#[tokio::test]
	async fn test_buffered_decompression_limit_exceeded() {
		// Decompressed output exceeds the limit
		let original = b"this payload will exceed the tiny limit after decompression";
		let compressed = encode_body(original, GZIP).await.unwrap();
		let body = Body::from(compressed);
		let ce = make_content_encoding(GZIP);
		let result = to_bytes_with_decompression(body, Some(&ce), 10).await;
		assert!(matches!(result, Err(Error::LimitExceeded)));
	}

	fn response_compressor(
		accept_encoding: &'static str,
		preferred_algorithms: Vec<CompressionAlgorithm>,
	) -> ResponseCompressor {
		let request = ::http::Request::builder()
			.header(ACCEPT_ENCODING, accept_encoding)
			.body(crate::http::Body::empty())
			.unwrap();
		ResponseCompression {
			preferred_algorithms,
		}
		.for_request(&request)
		.unwrap()
	}

	fn compressible_response() -> crate::http::Response {
		::http::Response::builder()
			.header(CONTENT_TYPE, "application/json")
			.header(CONTENT_LENGTH, "64")
			.body(crate::http::Body::from(
				r#"{"message":"a sufficiently long response body for compression"}"#,
			))
			.unwrap()
	}

	#[tokio::test]
	async fn negotiates_highest_quality_and_server_preference() {
		let preferred_algorithms = vec![
			CompressionAlgorithm::Gzip,
			CompressionAlgorithm::Brotli,
			CompressionAlgorithm::Deflate,
			CompressionAlgorithm::Zstd,
		];
		let mut response = compressible_response();
		response_compressor(
			"zstd;q=0.5, br, gzip, deflate",
			preferred_algorithms.clone(),
		)
		.apply(&mut response)
		.await
		.unwrap();
		assert_eq!(response.headers()[CONTENT_ENCODING], BR);

		let mut response = compressible_response();
		response_compressor("zstd, gzip, br", preferred_algorithms)
			.apply(&mut response)
			.await
			.unwrap();
		assert_eq!(response.headers()[CONTENT_ENCODING], ZSTD);
	}

	#[tokio::test]
	async fn explicit_rejection_overrides_wildcard() {
		let mut response = compressible_response();
		response_compressor("gzip;q=0, *;q=0.5", vec![CompressionAlgorithm::Gzip])
			.apply(&mut response)
			.await
			.unwrap();
		assert!(!response.headers().contains_key(CONTENT_ENCODING));

		let mut response = compressible_response();
		response_compressor("gzip;q=0, *;q=0.5", vec![CompressionAlgorithm::Brotli])
			.apply(&mut response)
			.await
			.unwrap();
		assert_eq!(response.headers()[CONTENT_ENCODING], BR);
	}

	#[tokio::test]
	async fn response_compression_supports_all_encodings() {
		for (algorithm, name) in [
			(CompressionAlgorithm::Gzip, GZIP),
			(CompressionAlgorithm::Brotli, BR),
			(CompressionAlgorithm::Deflate, DEFLATE),
			(CompressionAlgorithm::Zstd, ZSTD),
		] {
			let mut response = compressible_response();
			response_compressor(name, vec![algorithm])
				.apply(&mut response)
				.await
				.unwrap();

			assert_eq!(response.headers()[CONTENT_ENCODING], name);
			assert!(
				response.headers()[VARY]
					.to_str()
					.unwrap()
					.eq_ignore_ascii_case("Accept-Encoding")
			);
			assert!(!response.headers().contains_key(CONTENT_LENGTH));
			let content_encoding = make_content_encoding(name);
			let (body, _) = decompress_body(response.into_body(), Some(&content_encoding)).unwrap();
			let decoded = body.collect().await.unwrap().to_bytes();
			assert_eq!(
				decoded,
				r#"{"message":"a sufficiently long response body for compression"}"#
			);
		}
	}

	#[tokio::test]
	async fn response_compression_skips_small_or_ineligible_responses() {
		for (content_type, content_length) in [
			("application/json", "20"),
			("application/octet-stream", "100"),
			("text/event-stream", "100"),
		] {
			let mut response = ::http::Response::builder()
				.header(CONTENT_TYPE, content_type)
				.header(CONTENT_LENGTH, content_length)
				.body(crate::http::Body::from("not compressed"))
				.unwrap();
			response_compressor("gzip", vec![CompressionAlgorithm::Gzip])
				.apply(&mut response)
				.await
				.unwrap();
			assert!(!response.headers().contains_key(CONTENT_ENCODING));
		}
	}

	#[tokio::test]
	async fn response_compression_replaces_identity_encoding() {
		let mut response = ::http::Response::builder()
			.header(CONTENT_TYPE, "application/json")
			.header(CONTENT_ENCODING, "identity")
			.body(crate::http::Body::from(
				r#"{"message":"a sufficiently long response body for compression"}"#,
			))
			.unwrap();
		response_compressor("gzip", vec![CompressionAlgorithm::Gzip])
			.apply(&mut response)
			.await
			.unwrap();

		assert_eq!(response.headers()[CONTENT_ENCODING], GZIP);
		assert_eq!(
			response.headers().get_all(CONTENT_ENCODING).iter().count(),
			1
		);
	}

	#[tokio::test]
	async fn response_compression_skips_layered_content_encoding() {
		let mut response = ::http::Response::builder()
			.header(CONTENT_TYPE, "application/json")
			.header(CONTENT_ENCODING, "identity")
			.header(CONTENT_ENCODING, GZIP)
			.body(crate::http::Body::from("already encoded"))
			.unwrap();
		response_compressor("br", vec![CompressionAlgorithm::Brotli])
			.apply(&mut response)
			.await
			.unwrap();

		let encodings = response
			.headers()
			.get_all(CONTENT_ENCODING)
			.iter()
			.collect::<Vec<_>>();
		assert_eq!(encodings, vec!["identity", GZIP]);
	}

	#[tokio::test]
	async fn request_decompression_skips_layered_content_encoding() {
		let encoded = encode_body(b"layered request body", GZIP).await.unwrap();
		let mut request = ::http::Request::builder()
			.header(CONTENT_ENCODING, GZIP)
			.header(CONTENT_ENCODING, GZIP)
			.body(crate::http::Body::from(encoded.clone()))
			.unwrap();
		RequestDecompression::default()
			.apply_to_request(&mut request)
			.await
			.unwrap();

		assert_eq!(
			request.headers().get_all(CONTENT_ENCODING).iter().count(),
			2
		);
		assert_eq!(
			request.into_body().collect().await.unwrap().to_bytes(),
			encoded
		);
	}

	#[tokio::test]
	async fn request_decompression_supports_all_encodings() {
		let original = b"request body decompression";
		for (algorithm, name) in [
			(CompressionAlgorithm::Gzip, GZIP),
			(CompressionAlgorithm::Brotli, BR),
			(CompressionAlgorithm::Deflate, DEFLATE),
			(CompressionAlgorithm::Zstd, ZSTD),
		] {
			let encoded = encode_body(original, name).await.unwrap();
			let mut request = ::http::Request::builder()
				.header(CONTENT_ENCODING, name)
				.header(CONTENT_LENGTH, encoded.len())
				.body(crate::http::Body::from(encoded))
				.unwrap();
			RequestDecompression {
				accepted_algorithms: vec![algorithm],
			}
			.apply_to_request(&mut request)
			.await
			.unwrap();

			assert!(!request.headers().contains_key(CONTENT_ENCODING));
			assert!(!request.headers().contains_key(CONTENT_LENGTH));
			let decoded = request.into_body().collect().await.unwrap().to_bytes();
			assert_eq!(decoded, original.as_slice());
		}
	}

	#[tokio::test]
	async fn request_decompression_passes_unconfigured_encoding_through() {
		let encoded = encode_body(b"leave compressed", BR).await.unwrap();
		let mut request = ::http::Request::builder()
			.header(CONTENT_ENCODING, BR)
			.body(crate::http::Body::from(encoded.clone()))
			.unwrap();
		RequestDecompression::default()
			.apply_to_request(&mut request)
			.await
			.unwrap();

		assert_eq!(request.headers()[CONTENT_ENCODING], BR);
		assert_eq!(
			request.into_body().collect().await.unwrap().to_bytes(),
			encoded
		);
	}
}
