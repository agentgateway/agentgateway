use crate::http::filters::BackendRequestTimeout;
use crate::transport::stream::TLSConnectionInfo;
use crate::{apply, *};

#[apply(schema!)]
#[derive(Default)]
pub struct HTTP {
	#[serde(default, with = "http_serde::option::version")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub version: Option<::http::Version>,
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub request_timeout: Option<Duration>,
}

impl HTTP {
	pub fn apply(&self, req: &mut http::Request, version_override: Option<::http::Version>) {
		if let Some(timeout) = self.request_timeout {
			req.extensions_mut().insert(BackendRequestTimeout(timeout));
		};
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
				// If they are plaintext, however, that means the client very intentionally sent HTTP/2, and we respect that.
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
					None
				}
			},
		};
		match set_version {
			Some(::http::Version::HTTP_2) => {
				req.headers_mut().remove(http::header::TRANSFER_ENCODING);
				*req.version_mut() = ::http::Version::HTTP_2;
			},
			Some(::http::Version::HTTP_11) => {
				*req.version_mut() = ::http::Version::HTTP_11;
			},
			_ => {},
		};
	}
}

fn is_grpc(req: &http::Request) -> bool {
	req
		.headers()
		.get(http::header::CONTENT_TYPE)
		.is_some_and(|value| value.as_bytes().starts_with("application/grpc".as_bytes()))
}

#[apply(schema!)]
pub struct TCP {
	pub keepalives: super::agent::KeepaliveConfig,
	pub connect_timeout: Duration,
}

impl Default for TCP {
	fn default() -> Self {
		Self {
			keepalives: Default::default(),
			connect_timeout: defaults::connect_timeout(),
		}
	}
}
pub mod defaults {
	use std::time::Duration;

	pub fn connect_timeout() -> Duration {
		// We would pick 10, but everyone picks 10! If we pick 11, and we see timeouts at exactly
		// 11s, we can have more confidence this is caused by this default, and not someone else's 10s timer
		Duration::from_secs(11)
	}
}
