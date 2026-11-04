use headers::{CacheControl, ContentEncoding, ContentLength, ContentType, HeaderMapExt};
use http::header::{
	ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, VARY,
};
use http::{HeaderMap, HeaderValue, StatusCode};

use super::{BR, DEFLATE, EncodingDecision, GZIP, ZSTD, decompress_body, detect_encoding};
use crate::http::{Body, Request, Response};
use crate::proxy::dtrace;
use crate::{apply, schema};

const TRACE_POLICY_KIND: &str = "compression";

// Text formats compress well. Images, video, audio, fonts and archives are already compressed,
// so re-encoding them costs CPU for little or no gain. `text/event-stream` is omitted because
// streamed bodies are never compressed (see `skip_reason`).
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

// Below this, encoding overhead outweighs any savings.
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

fn de_algorithms<'de, D: serde::Deserializer<'de>>(
	deserializer: D,
) -> Result<Vec<CompressionAlgorithm>, D::Error> {
	let algorithms = <Vec<CompressionAlgorithm> as serde::Deserialize>::deserialize(deserializer)?;
	if algorithms.is_empty() {
		return Err(serde::de::Error::custom(
			"algorithms must not be empty; omit it to use the default (gzip)",
		));
	}
	Ok(algorithms)
}

#[apply(schema!)]
pub struct Compression {
	/// Compression applied to responses sent to the client.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response: Option<CompressionResponse>,
	/// Compression applied to requests received from the client.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request: Option<CompressionRequest>,
}

#[apply(schema!)]
pub struct CompressionResponse {
	/// Compress response bodies sent to the client. Only textual content types with a known
	/// length of at least 32 bytes are compressed, so streamed responses are sent as-is.
	/// Responses that are already encoded, partial (`Content-Range`), or marked `no-transform`
	/// are never compressed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub compress: Option<ResponseCompression>,
}

#[apply(schema!)]
pub struct CompressionRequest {
	/// Decompress request bodies after authorization, before route-level body-aware policies run
	/// and before forwarding. Gateway-phase policies observe the encoded body.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub decompress: Option<RequestDecompression>,
}

#[apply(schema!)]
pub struct ResponseCompression {
	/// Algorithms that may be negotiated with the client's Accept-Encoding header. When the client
	/// accepts several equally, the first listed wins. Defaults to gzip.
	#[serde(default = "default_algorithms", deserialize_with = "de_algorithms")]
	#[cfg_attr(feature = "schema", schemars(length(min = 1)))]
	pub algorithms: Vec<CompressionAlgorithm>,
}

#[apply(schema!)]
pub struct RequestDecompression {
	/// Algorithms decoded from request bodies. Bodies using any other (or more than one)
	/// encoding are forwarded untouched. Defaults to gzip.
	#[serde(default = "default_algorithms", deserialize_with = "de_algorithms")]
	#[cfg_attr(feature = "schema", schemars(length(min = 1)))]
	pub algorithms: Vec<CompressionAlgorithm>,
}

impl crate::store::RequestPolicyTrait for Compression {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyError> {
		if let Some(decompress) = self.request.as_ref().and_then(|r| r.decompress.as_ref()) {
			decompress.apply(req)?;
		}
		Ok(Default::default())
	}
}

impl RequestDecompression {
	/// Installs a streaming decoder without reading the body. This drops any inspection cached from
	/// the encoded bytes.
	pub fn apply(&self, req: &mut Request) -> Result<(), crate::proxy::ProxyError> {
		let Some(encoding) = req.headers().typed_get::<ContentEncoding>() else {
			dtrace::pol_result!(
				dtrace::Info,
				Skip,
				"request decompression skipped: request has no content-encoding"
			);
			return Ok(());
		};
		// Decoding only one layer of a layered encoding would misrepresent the body.
		let token = match detect_encoding(&encoding) {
			EncodingDecision::Single(token) => token,
			EncodingDecision::None => {
				dtrace::pol_result!(
					dtrace::Info,
					Skip,
					"request decompression skipped: content-encoding is identity"
				);
				return Ok(());
			},
			EncodingDecision::Multiple => {
				dtrace::pol_result!(
					dtrace::Info,
					Skip,
					"request decompression skipped: content-encoding has multiple layers"
				);
				return Ok(());
			},
			EncodingDecision::Unsupported => {
				dtrace::pol_result!(
					dtrace::Info,
					Skip,
					"request decompression skipped: content-encoding is not supported"
				);
				return Ok(());
			},
		};
		if !self.algorithms.iter().any(|a| a.token() == token) {
			dtrace::pol_result!(
				dtrace::Info,
				Skip,
				"request decompression skipped: content-encoding {token} is not a configured algorithm"
			);
			return Ok(());
		}
		let (body, _) = decompress_body(std::mem::take(req.body_mut()), Some(&encoding))
			.map_err(|e| crate::proxy::ProxyError::Processing(e.into()))?;
		*req.body_mut() = body;
		dtrace::pol_result!(dtrace::Info, Apply, "decoding {token} request body");
		req.headers_mut().remove(CONTENT_ENCODING);
		req.headers_mut().remove(CONTENT_LENGTH);
		Ok(())
	}
}

impl ResponseCompression {
	/// Negotiates with Accept-Encoding before request policies can strip it.
	pub fn for_request(&self, req: &Request) -> ResponseCompressor {
		let algorithm = if req.headers().contains_key(ACCEPT_ENCODING) {
			negotiate(&self.algorithms, req.headers())
				.ok_or("accept-encoding allows none of the configured algorithms")
		} else {
			Err("request has no accept-encoding")
		};
		ResponseCompressor { algorithm }
	}
}

/// Picks the configured algorithm the client accepts with the highest q-value. Ties go to the
/// configured order. Unlisted algorithms take the `*` q-value, if any. An explicit `identity`
/// with a strictly higher q-value wins over every algorithm.
fn negotiate(
	algorithms: &[CompressionAlgorithm],
	headers: &HeaderMap,
) -> Option<CompressionAlgorithm> {
	let mut wildcard = None;
	let mut identity = None;
	let mut listed = Vec::new();
	for value in headers.get_all(ACCEPT_ENCODING) {
		let Ok(value) = value.to_str() else {
			continue;
		};
		for entry in value.split(',') {
			let mut params = entry.split(';');
			let token = params.next().unwrap_or_default().trim();
			if token.is_empty() {
				continue;
			}
			let mut q = Some(1.0f32);
			for param in params {
				if let Some((k, v)) = param.split_once('=')
					&& k.trim().eq_ignore_ascii_case("q")
				{
					q = v.trim().parse().ok().filter(|q| (0.0..=1.0).contains(q));
				}
			}
			// Ignore entries with a malformed q-value.
			let Some(q) = q else {
				continue;
			};
			if token == "*" {
				wildcard = Some(q);
			} else if token.eq_ignore_ascii_case("identity") {
				identity = Some(q);
			} else {
				listed.push((token, q));
			}
		}
	}
	let mut best: Option<(CompressionAlgorithm, f32)> = None;
	for &algorithm in algorithms {
		let q = listed
			.iter()
			.find(|(token, _)| token.eq_ignore_ascii_case(algorithm.token()))
			.map(|(_, q)| *q)
			.or(wildcard)
			.unwrap_or(0.0);
		if q > 0.0 && best.is_none_or(|(_, best_q)| q > best_q) {
			best = Some((algorithm, q));
		}
	}
	best
		.filter(|(_, q)| identity.is_none_or(|identity| *q >= identity))
		.map(|(algorithm, _)| algorithm)
}

#[derive(Debug)]
pub struct ResponseCompressor {
	// The skip reason when negotiation found nothing acceptable.
	algorithm: Result<CompressionAlgorithm, &'static str>,
}

impl ResponseCompressor {
	/// Must run after every policy that can read or replace the response body, and after the
	/// request log snapshot so logs keep the plaintext body.
	pub fn apply(self, resp: &mut Response) {
		// Responses rebuilt by the proxy (LLM, transformations) drop Content-Length but keep a
		// known body size.
		let content_length = resp
			.headers()
			.typed_get::<ContentLength>()
			.map(|ContentLength(n)| n)
			.or_else(|| http_body::Body::size_hint(resp.body()).exact());
		if let Some(reason) = skip_reason(resp.status(), resp.headers(), content_length) {
			dtrace::pol_result!(dtrace::Info, Skip, "response compression skipped: {reason}");
			return;
		}
		// The representation depends on Accept-Encoding even when this client gets plaintext.
		if !resp.headers().get_all(VARY).iter().any(|v| {
			v.to_str().is_ok_and(|v| {
				v.split(',')
					.any(|t| t.trim() == "*" || t.trim().eq_ignore_ascii_case("accept-encoding"))
			})
		}) {
			resp
				.headers_mut()
				.append(VARY, HeaderValue::from_static("accept-encoding"));
		}
		let algorithm = match self.algorithm {
			Ok(algorithm) => algorithm,
			Err(reason) => {
				dtrace::pol_result!(dtrace::Info, Skip, "response compression skipped: {reason}");
				return;
			},
		};
		let token = algorithm.token();
		// Wrap rather than replace the plaintext body, so its recorder keeps observing plaintext.
		let plaintext = axum_core::body::Body::new(std::mem::take(resp.body_mut()));
		*resp.body_mut() = Body::new(
			super::encode_body_stream(plaintext, token).expect("configured algorithms are supported"),
		);
		let headers = resp.headers_mut();
		headers.insert(CONTENT_ENCODING, HeaderValue::from_static(token));
		headers.remove(CONTENT_LENGTH);
		// Byte ranges of the encoded representation can't be served.
		headers.remove(ACCEPT_RANGES);
		weaken_etag(headers);
		dtrace::pol_result!(dtrace::Info, Apply, "compressing response with {token}");
	}
}

fn skip_reason(
	status: StatusCode,
	headers: &HeaderMap,
	content_length: Option<u64>,
) -> Option<String> {
	if status.is_informational()
		|| status == StatusCode::NO_CONTENT
		|| status == StatusCode::NOT_MODIFIED
	{
		return Some(format!("status {status} has no body"));
	}
	if headers.contains_key(CONTENT_ENCODING) {
		return Some("response is already encoded".to_string());
	}
	if headers.contains_key(CONTENT_RANGE) {
		return Some("response is a partial range".to_string());
	}
	if headers
		.typed_get::<CacheControl>()
		.is_some_and(|c| c.no_transform())
	{
		return Some("cache-control is no-transform".to_string());
	}
	// The encoder does not flush while the body is pending, so streamed bodies would stall.
	// The header takes precedence over the body size so HEAD gets the same headers as GET.
	match content_length {
		None => return Some("response has no known length".to_string()),
		Some(n) if n < MIN_COMPRESS_SIZE => {
			return Some(format!(
				"content-length {n} is below {MIN_COMPRESS_SIZE} bytes"
			));
		},
		Some(_) => {},
	}
	let Some(mime) = headers
		.typed_get::<ContentType>()
		.map(crate::http::mime::Mime::from)
	else {
		return Some("response has no content-type".to_string());
	};
	let essence = mime.essence_str();
	if COMPRESSIBLE_CONTENT_TYPES.contains(&essence)
		|| mime
			.suffix()
			.is_some_and(|s| s == crate::http::mime::JSON || s == crate::http::mime::XML)
	{
		return None;
	}
	Some(format!("content-type {essence} is not compressible"))
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
