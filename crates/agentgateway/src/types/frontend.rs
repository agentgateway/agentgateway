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

#[apply(schema!)]
pub struct HTTP {
	#[serde(default = "defaults::max_buffer_size")]
	pub max_buffer_size: usize,

	/// The maximum number of headers allowed in a request. Changing this value results in a performance
	/// degradation, even if set to a lower value than the default (100)
	#[serde(default)]
	pub http1_max_headers: Option<usize>,
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(default = "defaults::http1_idle_timeout")]
	pub http1_idle_timeout: Duration,

	#[serde(default)]
	pub http2_window_size: Option<u32>,
	#[serde(default)]
	pub http2_connection_window_size: Option<u32>,
	#[serde(default)]
	pub http2_frame_size: Option<u32>,
	#[serde(default)]
	pub http2_max_header_size: Option<u32>,
	#[serde(with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	#[serde(default)]
	pub http2_keepalive_interval: Option<Duration>,
	#[serde(with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	#[serde(default)]
	pub http2_keepalive_timeout: Option<Duration>,

	/// Maximum duration a connection is allowed to remain open. After this duration,
	/// the connection is gracefully closed after the current in-flight request completes.
	/// Useful for ensuring even traffic distribution behind load balancers during scaling events.
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
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(default = "defaults::tls_handshake_timeout")]
	pub handshake_timeout: Duration,
	#[serde(default)]
	pub alpn: Option<Vec<Vec<u8>>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub min_version: Option<TLSVersion>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_version: Option<TLSVersion>,
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
	pub keepalives: super::agent::KeepaliveConfig,
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum ProxyVersion {
	V1,
	#[default]
	V2,
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
	#[default]
	Strict,
	Optional,
}

#[apply(schema!)]
#[derive(Default, PartialEq, Eq)]
pub struct Proxy {
	#[serde(default)]
	pub version: ProxyVersion,
	#[serde(default)]
	pub mode: ProxyMode,
}

#[apply(schema!)]
pub struct NetworkAuthorization(pub crate::http::authorization::RuleSet);

#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsFieldsPolicy {
	#[serde(default, skip_serializing_if = "OrderedStringMap::is_empty")]
	pub add: Arc<OrderedStringMap<Arc<cel::Expression>>>,
}

#[apply(schema!)]
pub struct LoggingPolicy {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub filter: Option<Arc<cel::Expression>>,
	#[serde(default, skip_serializing_if = "OrderedStringMap::is_empty")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	pub add: Arc<OrderedStringMap<Arc<cel::Expression>>>,
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashSet<String>")
	)]
	#[serde(default, skip_serializing_if = "empty_string_set")]
	pub remove: Arc<FzHashSet<String>>,
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
	#[serde(flatten)]
	pub provider_backend: super::agent::SimpleBackendReference,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	pub policies: Vec<super::agent::BackendTrafficPolicy>,
	#[serde(default)]
	pub protocol: super::agent::TracingProtocol,
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
