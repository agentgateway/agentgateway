use crate::http::filters::BackendRequestTimeout;
use crate::transport::stream::TLSConnectionInfo;
use crate::types::agent::SimpleBackendReference;
use crate::{apply, *};

#[apply(schema!)]
#[derive(Default)]
pub struct HTTP {
	/// HTTP version to use when connecting to the backend.
	#[serde(default, with = "http_serde::option::version")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub version: Option<::http::Version>,
	/// Maximum time allowed for a backend HTTP request.
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
					if http::is_grpc_content_type(req.headers()) {
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

#[apply(schema_enum!)]
#[derive(Default)]
pub enum TunnelMode {
	/// Use CONNECT for TLS and non-HTTP transports, and absolute-form requests for plaintext HTTP.
	#[default]
	Auto,
	/// Use CONNECT for all transports, including plaintext HTTP.
	Connect,
}

#[apply(schema!)]
pub struct Tunnel {
	/// Proxy backend used to tunnel the connection.
	pub proxy: Arc<SimpleBackendReference>,
	/// How requests are sent through the proxy.
	#[serde(default)]
	pub mode: TunnelMode,
	/// Policies to connect to the proxy backend
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	pub policies: Vec<super::agent::BackendTrafficPolicy>,
}

#[apply(schema!)]
#[derive(Default, Hash, PartialEq, Eq)]
pub struct TCP {
	/// TCP keepalive settings for backend connections.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub keepalives: Option<super::agent::KeepaliveConfig>,
	/// Maximum time allowed to establish a backend TCP connection.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "crate::serdes::serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub connect_timeout: Option<Duration>,
}
