use crate::*;
use crate::{apply, defaults};
use std::time::Duration;

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
	pub tls_handshake_timeout: Duration,
}

impl Default for TLS {
	fn default() -> Self {
		Self {
			tls_handshake_timeout: defaults::tls_handshake_timeout(),
		}
	}
}

#[apply(schema!)]
pub struct TCP {
	pub keepalives: super::agent::KeepaliveConfig,
}
