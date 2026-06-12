use std::time::Duration;

use frozen_collections::{FzHashSet, Len};
use serde::{Deserialize, Serialize};

use crate::telemetry::log::OrderedStringMap;
use crate::{apply, defaults, *};

fn empty_string_set(set: &Arc<FzHashSet<String>>) -> bool {
	set.is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[allow(non_camel_case_types)]
pub enum TLSVersion {
	TLS_V1_0,
	TLS_V1_1,
	TLS_V1_2,
	TLS_V1_3,
}

impl From<TLSVersion> for super::agent::TLSVersion {
	fn from(value: TLSVersion) -> Self {
		match value {
			TLSVersion::TLS_V1_0 => super::agent::TLSVersion::TLS_V1_0,
			TLSVersion::TLS_V1_1 => super::agent::TLSVersion::TLS_V1_1,
			TLSVersion::TLS_V1_2 => super::agent::TLSVersion::TLS_V1_2,
			TLSVersion::TLS_V1_3 => super::agent::TLSVersion::TLS_V1_3,
		}
	}
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum HTTPHeaderCase {
	/// Encode HTTP/1 header names in lowercase.
	#[default]
	Lowercase,
	/// Preserve original HTTP/1 request header casing when encoding responses on the same connection.
	Preserve,
}

#[apply(schema!)]
pub struct HTTP {
	/// Maximum request or response body size buffered by the frontend.
	#[serde(default = "defaults::max_buffer_size")]
	pub max_buffer_size: usize,

	/// Maximum number of headers allowed in an HTTP/1 request. Changing this value causes a
	/// performance degradation, even when set lower than the default of 100.
	#[serde(default)]
	pub http1_max_headers: Option<usize>,
	/// How long an idle HTTP/1 connection may stay open.
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(default = "defaults::http1_idle_timeout")]
	pub http1_idle_timeout: Duration,
	/// Header casing behavior for HTTP/1 responses.
	#[serde(default)]
	pub http1_header_case: HTTPHeaderCase,

	/// HTTP/2 stream flow-control window size.
	#[serde(default)]
	pub http2_window_size: Option<u32>,
	/// HTTP/2 connection flow-control window size.
	#[serde(default)]
	pub http2_connection_window_size: Option<u32>,
	/// Maximum HTTP/2 frame size.
	#[serde(default)]
	pub http2_frame_size: Option<u32>,
	/// Maximum size of HTTP/2 request headers.
	#[serde(default)]
	pub http2_max_header_size: Option<u32>,
	/// Interval between HTTP/2 keepalive pings.
	#[serde(with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	#[serde(default)]
	pub http2_keepalive_interval: Option<Duration>,
	/// Time to wait for an HTTP/2 keepalive ping response.
	#[serde(with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	#[serde(default)]
	pub http2_keepalive_timeout: Option<Duration>,

	/// Maximum time a connection may stay open. After this duration, the connection is gracefully
	/// closed after the current in-flight request completes. Useful for even traffic distribution
	/// behind load balancers during scaling events.
	#[serde(with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	#[serde(default)]
	pub max_connection_duration: Option<Duration>,
}

impl Default for HTTP {
	fn default() -> Self {
		Self {
			max_buffer_size: defaults::max_buffer_size(),

			http1_max_headers: None,
			http1_idle_timeout: defaults::http1_idle_timeout(),
			http1_header_case: HTTPHeaderCase::Lowercase,

			http2_window_size: None,
			http2_connection_window_size: None,
			http2_frame_size: None,
			http2_max_header_size: None,

			http2_keepalive_interval: None,
			http2_keepalive_timeout: None,

			max_connection_duration: None,
		}
	}
}

#[apply(schema!)]
pub struct TLS {
	/// Maximum time allowed to complete the downstream TLS handshake.
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(default = "defaults::tls_handshake_timeout")]
	pub handshake_timeout: Duration,
	/// ALPN protocols advertised to downstream clients.
	#[serde(default)]
	pub alpn: Option<Vec<Vec<u8>>>,
	/// Minimum TLS version accepted from downstream clients.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub min_version: Option<TLSVersion>,
	/// Maximum TLS version accepted from downstream clients.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_version: Option<TLSVersion>,
	/// Cipher suites allowed for downstream TLS.
	#[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cipher_suites: Option<Vec<crate::transport::tls::CipherSuite>>,
	/// Key exchange groups allowed for negotiating TLS.
	#[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub key_exchange_groups: Option<Vec<crate::transport::tls::KeyExchangeGroup>>,
}

impl Default for TLS {
	fn default() -> Self {
		Self {
			handshake_timeout: defaults::tls_handshake_timeout(),
			alpn: None,
			min_version: None,
			max_version: None,
			cipher_suites: None,
			key_exchange_groups: None,
		}
	}
}

#[apply(schema!)]
pub struct TCP {
	/// TCP keepalive settings for downstream connections.
	pub keepalives: super::agent::KeepaliveConfig,
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum ProxyVersion {
	/// Accept PROXY protocol v1.
	V1,
	/// Accept PROXY protocol v2.
	#[default]
	V2,
	/// Accept PROXY protocol v1 or v2.
	All,
}

impl ProxyVersion {
	pub fn allows_v1(self) -> bool {
		matches!(self, Self::V1 | Self::All)
	}

	pub fn allows_v2(self) -> bool {
		matches!(self, Self::V2 | Self::All)
	}
}

impl std::fmt::Display for ProxyVersion {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::V1 => f.write_str("v1"),
			Self::V2 => f.write_str("v2"),
			Self::All => f.write_str("all"),
		}
	}
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum ProxyMode {
	/// Require a PROXY protocol header on each connection.
	#[default]
	Strict,
	/// Accept connections with or without a PROXY protocol header.
	Optional,
}

#[apply(schema!)]
#[derive(Default, PartialEq, Eq)]
pub struct Proxy {
	/// PROXY protocol versions accepted from downstream clients.
	#[serde(default)]
	pub version: ProxyVersion,
	/// Whether downstream connections must include a PROXY protocol header.
	#[serde(default)]
	pub mode: ProxyMode,
}

#[apply(schema_enum!)]
pub enum ConnectMode {
	/// Reject HTTP CONNECT requests.
	Deny,
	/// Route HTTP CONNECT requests through normal route matching.
	Route,
	/// Treat HTTP CONNECT requests as tunnels.
	Tunnel,
}

#[apply(schema!)]
pub struct Connect {
	/// How downstream HTTP CONNECT requests are handled.
	pub mode: ConnectMode,
}

#[apply(schema!)]
pub struct NetworkAuthorization(
	/// CEL authorization rules for downstream network connections.
	pub crate::http::authorization::RuleSet,
);

#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsFieldsPolicy {
	/// Metric fields to add, computed from CEL expressions.
	#[serde(default, skip_serializing_if = "OrderedStringMap::is_empty")]
	pub add: Arc<OrderedStringMap<Arc<cel::Expression>>>,
}

#[apply(schema!)]
pub struct LoggingPolicy {
	/// CEL expression that decides whether a request is logged.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub filter: Option<Arc<cel::Expression>>,
	/// Access log fields to add, computed from CEL expressions.
	#[serde(default, skip_serializing_if = "OrderedStringMap::is_empty")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	pub add: Arc<OrderedStringMap<Arc<cel::Expression>>>,
	/// Access log field names to remove.
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashSet<String>")
	)]
	#[serde(default, skip_serializing_if = "empty_string_set")]
	pub remove: Arc<FzHashSet<String>>,
	/// OTLP log export settings.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub otlp: Option<OtlpLoggingConfig>,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub access_log_policy: Option<Arc<super::agent::AccessLogPolicy>>,
}

impl LoggingPolicy {
	/// Initializes the shared `AccessLogPolicy` from the OTLP config, if present.
	/// Must be called after deserialization so the `OnceCell`-backed logger is
	/// shared across requests instead of being recreated each time.
	pub fn init_access_log_policy(&mut self) {
		if let Some(otlp_cfg) = &self.otlp {
			self.access_log_policy = Some(Arc::new(super::agent::AccessLogPolicy {
				config: otlp_cfg.clone(),
				logger: once_cell::sync::OnceCell::new(),
			}));
		}
	}
}

#[apply(schema!)]
pub struct OtlpLoggingConfig {
	/// Backend that receives OTLP logs.
	#[serde(flatten)]
	pub provider_backend: super::agent::SimpleBackendReference,
	/// Backend policies used when exporting OTLP logs.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	pub policies: Vec<super::agent::BackendTrafficPolicy>,
	/// OTLP protocol used to export logs.
	#[serde(default)]
	pub protocol: super::agent::TracingProtocol,
	/// OTLP HTTP path used to export logs.
	#[serde(
		default = "default_logs_path",
		skip_serializing_if = "is_default_logs_path"
	)]
	pub path: String,
}

fn default_logs_path() -> String {
	"/v1/logs".to_string()
}

fn is_default_logs_path(path: &str) -> bool {
	path == "/v1/logs"
}
