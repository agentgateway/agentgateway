use std::convert::Infallible;

use futures_util::FutureExt;
use headers::{ContentEncoding, HeaderMapExt};
use http::header::{
	ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, ETAG,
};
use http::{HeaderMap, HeaderValue, StatusCode};
use tower::ServiceExt;
use tower_http::compression::Compression as CompressionService;
use tower_http::compression::predicate::{Predicate, SizeAbove};

use super::{BR, DEFLATE, EncodingDecision, GZIP, ZSTD, decompress_body, detect_encoding};
use crate::http::{Body, Request, Response};
use crate::{apply, schema};

// Compression only pays off for textual payloads. Binary formats (images, video, archives,
// protobuf) are usually already compressed, and streaming types such as SSE would be delayed by
// encoder buffering, so anything not listed here is sent as-is.
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
	/// Compress response bodies sent to the client.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response: Option<ResponseCompression>,
	/// Decompress request bodies before body-aware policies run and before forwarding.
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
	pub accepted_algorithms: Vec<CompressionAlgorithm>,
}

impl crate::store::RequestPolicyTrait for Compression {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		if let Some(request) = &self.request {
			request.apply(req)?;
		}
		Ok(Default::default())
	}
}

impl RequestDecompression {
	/// Install a streaming decoder; nothing is read here. Replacing the body content also drops
	/// any inspection cached from the encoded bytes, so later body-aware CEL must re-inspect.
	pub fn apply(&self, req: &mut Request) -> Result<(), crate::proxy::ProxyError> {
		let Some(encoding) = req.headers().typed_get::<ContentEncoding>() else {
			return Ok(());
		};
		// Layered or unknown encodings are left alone: decoding one layer would misrepresent the body.
		let EncodingDecision::Single(token) = detect_encoding(&encoding) else {
			return Ok(());
		};
		if !self.accepted_algorithms.iter().any(|a| a.token() == token) {
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
	/// Capture the client's Accept-Encoding now, since request policies may strip it before the
	/// request is forwarded.
	pub fn for_request(&self, req: &Request) -> Option<ResponseCompressor> {
		if req.method() == http::Method::HEAD || !req.headers().contains_key(ACCEPT_ENCODING) {
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
	/// Negotiate and apply compression using tower-http. This must run after every policy that can
	/// read or replace the response body, since the body is only encoded as it is sent.
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
		.compress_when(SizeAbove::default().and(is_compressible));

		let mut request = ::http::Request::new(());
		*request.headers_mut() = self.accept_encoding;
		let compressed = service
			.oneshot(request)
			.now_or_never()
			.expect("inner service is always ready")
			.expect("inner service is infallible");

		let (mut parts, body) = compressed.into_parts();
		// tower-http answers 406 when no offered coding is acceptable; a proxy must not rewrite the
		// upstream status, so keep it and send the body unencoded.
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
	// RFC 9111 §5.2.2.6: intermediaries must not transform a no-transform payload.
	let no_transform = headers
		.get_all(CACHE_CONTROL)
		.iter()
		.filter_map(|v| v.to_str().ok())
		.flat_map(|v| v.split(','))
		.any(|d| d.trim().eq_ignore_ascii_case("no-transform"));
	if no_transform {
		return false;
	}
	let Some(essence) = headers
		.get(CONTENT_TYPE)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| v.split(';').next())
		.map(|v| v.trim().to_ascii_lowercase())
	else {
		return false;
	};
	COMPRESSIBLE_CONTENT_TYPES.contains(&essence.as_str())
		|| essence.ends_with("+json")
		|| essence.ends_with("+xml")
}

// The encoded representation differs byte-wise, so a strong validator no longer holds
// (RFC 9110 §8.8.1); weakening keeps conditional requests working.
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
