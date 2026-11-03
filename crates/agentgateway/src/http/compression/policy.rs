use std::convert::Infallible;

use futures_util::FutureExt;
use headers::{CacheControl, ContentEncoding, ContentLength, ContentType, HeaderMapExt};
use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, ETAG};
use http::{HeaderMap, HeaderValue, StatusCode};
use tower::ServiceExt;
use tower_http::compression::Compression as CompressionService;

use super::{BR, DEFLATE, EncodingDecision, GZIP, ZSTD, decompress_body, detect_encoding};
use crate::http::{Body, Request, Response};
use crate::{apply, schema};

// Binary formats are usually already compressed.
const COMPRESSIBLE_CONTENT_TYPES: &[&str] = &[
	"application/javascript",
	"application/json",
	"application/xml",
	"application/xhtml+xml",
	"image/svg+xml",
	"text/css",
	"text/html",
	"text/javascript",
	"text/plain",
	"text/xml",
];

// Matches tower-http's SizeAbove default.
const MIN_COMPRESS_SIZE: u64 = 32;

/// An HTTP content coding supported by the compression policy.
#[apply(schema!)]
#[derive(Copy, Eq, PartialEq)]
pub enum CompressionAlgorithm {
	Gzip,
	Brotli,
	Deflate,
	Zstd,
}

impl CompressionAlgorithm {
	fn token(self) -> &'static str {
		match self {
			Self::Gzip => GZIP,
			Self::Brotli => BR,
			Self::Deflate => DEFLATE,
			Self::Zstd => ZSTD,
		}
	}
}

fn default_algorithms() -> Vec<CompressionAlgorithm> {
	vec![CompressionAlgorithm::Gzip]
}

#[apply(schema!)]
pub struct Compression {
	/// Compress response bodies sent to the client. Only textual content types with a known
	/// `Content-Length` of at least 32 bytes are compressed, so streamed responses are sent as-is.
	/// Responses that are already encoded, partial (`Content-Range`), or marked `no-transform`
	/// are never compressed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response: Option<ResponseCompression>,
	/// Decompress request bodies after authorization, before body-aware policies run and before
	/// forwarding.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request: Option<RequestDecompression>,
}

#[apply(schema!)]
pub struct ResponseCompression {
	/// Algorithms that may be negotiated with the client's Accept-Encoding header.
	/// Defaults to gzip.
	#[serde(default = "default_algorithms")]
	pub algorithms: Vec<CompressionAlgorithm>,
}

#[apply(schema!)]
pub struct RequestDecompression {
	/// Algorithms decoded from request bodies. Bodies using any other (or more than one)
	/// encoding are forwarded untouched. Defaults to gzip.
	#[serde(default = "default_algorithms")]
	pub algorithms: Vec<CompressionAlgorithm>,
}

impl crate::store::RequestPolicyTrait for Compression {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyError> {
		if let Some(request) = &self.request {
			request.apply(req)?;
		}
		Ok(Default::default())
	}
}

impl RequestDecompression {
	/// Installs a streaming decoder without reading the body. This drops any inspection cached from
	/// the encoded bytes.
	pub fn apply(&self, req: &mut Request) -> Result<(), crate::proxy::ProxyError> {
		let Some(encoding) = req.headers().typed_get::<ContentEncoding>() else {
			return Ok(());
		};
		// Decoding only one layer of a layered encoding would misrepresent the body.
		let EncodingDecision::Single(token) = detect_encoding(&encoding) else {
			return Ok(());
		};
		if !self.algorithms.iter().any(|a| a.token() == token) {
			return Ok(());
		}
		let (body, _) = decompress_body(std::mem::take(req.body_mut()), Some(&encoding))
			.map_err(|e| crate::proxy::ProxyError::Processing(e.into()))?;
		*req.body_mut() = body;
		req.headers_mut().remove(CONTENT_ENCODING);
		req.headers_mut().remove(CONTENT_LENGTH);
		Ok(())
	}
}

impl ResponseCompression {
	/// Captures Accept-Encoding before request policies can strip it.
	pub fn for_request(&self, req: &Request) -> Option<ResponseCompressor> {
		if !req.headers().contains_key(ACCEPT_ENCODING) {
			return None;
		}
		let mut accept_encoding = HeaderMap::new();
		for value in req.headers().get_all(ACCEPT_ENCODING) {
			accept_encoding.append(ACCEPT_ENCODING, value.clone());
		}
		Some(ResponseCompressor {
			algorithms: self.algorithms.clone(),
			accept_encoding,
		})
	}
}

#[derive(Debug)]
pub struct ResponseCompressor {
	algorithms: Vec<CompressionAlgorithm>,
	accept_encoding: HeaderMap,
}

impl ResponseCompressor {
	/// Must run after every policy that can read or replace the response body.
	pub fn apply(self, resp: &mut Response) {
		let status = resp.status();
		if status.is_informational()
			|| status == StatusCode::NO_CONTENT
			|| status == StatusCode::NOT_MODIFIED
		{
			return;
		}
		let already_encoded = resp.headers().contains_key(CONTENT_ENCODING);

		let mut upstream = Some(std::mem::take(resp));
		let enabled = |algorithm| self.algorithms.contains(&algorithm);
		let service = CompressionService::new(tower::service_fn(move |_: ::http::Request<()>| {
			std::future::ready(Ok::<_, Infallible>(
				upstream.take().expect("compression service is called once"),
			))
		}))
		.gzip(enabled(CompressionAlgorithm::Gzip))
		.br(enabled(CompressionAlgorithm::Brotli))
		.deflate(enabled(CompressionAlgorithm::Deflate))
		.zstd(enabled(CompressionAlgorithm::Zstd))
		.compress_when(is_compressible);

		let mut request = ::http::Request::new(());
		*request.headers_mut() = self.accept_encoding;
		// The inner service is always ready and `Compression` only maps its future.
		let compressed = service
			.oneshot(request)
			.now_or_never()
			.expect("inner service is always ready")
			.expect("inner service is infallible");

		let (mut parts, body) = compressed.into_parts();
		// tower-http answers 406 when nothing is acceptable; keep the upstream status instead.
		parts.status = status;
		if !already_encoded && parts.headers.contains_key(CONTENT_ENCODING) {
			weaken_etag(&mut parts.headers);
		}
		*resp = Response::from_parts(parts, Body::new(body));
	}
}

fn is_compressible(
	_: StatusCode,
	_: http::Version,
	headers: &HeaderMap,
	_: &http::Extensions,
) -> bool {
	if headers
		.typed_get::<CacheControl>()
		.is_some_and(|c| c.no_transform())
	{
		return false;
	}
	// The encoder does not flush while the body is pending, so streamed bodies would stall.
	// Using the header rather than the body size also gives HEAD the same headers as GET.
	if !headers
		.typed_get::<ContentLength>()
		.is_some_and(|ContentLength(n)| n >= MIN_COMPRESS_SIZE)
	{
		return false;
	}
	let Some(mime) = headers
		.typed_get::<ContentType>()
		.map(crate::http::mime::Mime::from)
	else {
		return false;
	};
	let essence = mime.essence_str();
	COMPRESSIBLE_CONTENT_TYPES.contains(&essence)
		|| mime
			.suffix()
			.is_some_and(|s| s == crate::http::mime::JSON || s == crate::http::mime::XML)
}

// A strong validator no longer holds once the bytes change (RFC 9110 section 8.8.1).
fn weaken_etag(headers: &mut HeaderMap) {
	if let Some(etag) = headers.get(ETAG)
		&& !etag.as_bytes().starts_with(b"W/")
		&& let Ok(weak) = HeaderValue::from_bytes(&[b"W/", etag.as_bytes()].concat())
	{
		headers.insert(ETAG, weak);
	}
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
