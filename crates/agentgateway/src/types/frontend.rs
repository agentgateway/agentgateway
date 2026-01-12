use std::time::Duration;

use frozen_collections::FzHashSet;
use serde::{Deserialize, Serialize};

use crate::telemetry::log::OrderedStringMap;
use crate::{apply, defaults, *};

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
	#[serde(with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	#[serde(default)]
	pub http2_keepalive_interval: Option<Duration>,
	#[serde(with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	#[serde(default)]
	pub http2_keepalive_timeout: Option<Duration>,
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

			http2_keepalive_interval: None,
			http2_keepalive_timeout: None,
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
}

impl Default for TLS {
	fn default() -> Self {
		Self {
			handshake_timeout: defaults::tls_handshake_timeout(),
			alpn: None,
			min_version: None,
			max_version: None,
			cipher_suites: None,
		}
	}
}

#[apply(schema!)]
pub struct TCP {
	pub keepalives: super::agent::KeepaliveConfig,
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
	#[serde(default, skip_serializing_if = "FzHashSet::is_empty")]
	pub remove: Arc<FzHashSet<String>>,
}
