use crate::apply;
use crate::telemetry::log::RequestLog;
use crate::transport::stream::TLSConnectionInfo;
use crate::*;

#[apply(schema!)]
#[derive(Default)]
pub struct HTTP {
	#[serde(with = "http_serde::option::version")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub version: Option<::http::Version>,
}

impl HTTP {
	pub fn apply(
		&self,
		req: &mut http::Request,
		version_override: Option<::http::Version>,
		log: &mut Option<&mut RequestLog>,
	) {
		// Version override comes from a Service having a version specified. A policy is more specific
		// so we use the policy first.
		let set_version = match self.version.or(version_override) {
			Some(v) => Some(v),
			None => {
				// There are a few cases here...
				// In general, we cannot be assured that the downstream and the upstream protocol have anything
				// to do with each other. Typically, the downstream will ALPN negotiate up to HTTP/2, even
				// if the backend shouldn't do HTTP/2. So, if TLS is used, we never want to trust the downstream
				// protocol.
				// If they are plaintext, however, that means the client very intentionally sent HTTP/2, and we
				// respect that.
				// Additionally, since gRPC is known to only work over HTTP/2, we special case that.
				let tls = req.extensions().get::<TLSConnectionInfo>();
				if tls.is_some() {
					// Do not trust the downstream, use HTTP/1.1
					if is_grpc(req) {
						Some(::http::Version::HTTP_2)
					} else {
						Some(::http::Version::HTTP_11)
					}
				} else {
					// Plaintext: mirror downstream version
					// NOTE: If client uses h2c prior-knowledge, this mirrors HTTP/2.
					// For HTTP/1.1-only backends (local AI), set policy version: "1.1" explicitly.
					Some(req.version())
				}
			},
		};

		match set_version {
			Some(::http::Version::HTTP_2) => {
				req.headers_mut().remove(http::header::TRANSFER_ENCODING);
				req.headers_mut().remove(http::header::CONNECTION);
				*req.version_mut() = ::http::Version::HTTP_2;
			},
			Some(::http::Version::HTTP_11) => {
				*req.version_mut() = ::http::Version::HTTP_11;
			},
			_ => {},
		};

		// Observability: record selected upstream HTTP version
		if let Some(ver) = set_version {
			if let Some(log) = log {
				log.upstream_http_version = Some(ver);
			}
		}
	}

	/// Returns true if this policy enforces HTTP/1.1
	pub fn is_http11(&self) -> bool {
		matches!(self.version, Some(::http::Version::HTTP_11))
	}
}

fn is_grpc(req: &http::Request) -> bool {
	req
		.headers()
		.get(http::header::CONTENT_TYPE)
		.is_some_and(|value| value.as_bytes().starts_with("application/grpc".as_bytes()))
}

#[apply(schema!)]
#[derive(Default)]
pub struct TCP {}
