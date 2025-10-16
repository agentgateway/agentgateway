use crate::mcp::MCPOperation;
use crate::proxy::ProxyResponseReason;
use crate::types::agent::BindProtocol;
use agent_core::metrics::{CustomField, DefaultedUnknown, EncodeArc, EncodeDebug, EncodeDisplay};
use agent_core::strng::RichStrng;
use agent_core::version;
use frozen_collections::FzHashSet;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::Histogram as PromHistogram;
use prometheus_client::metrics::info::Info;
use prometheus_client::registry::{Metric, Registry, Unit};
use std::fmt::Debug;
use tracing::{debug, trace};

#[derive(Clone, Hash, Default, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct RouteIdentifier {
	pub bind: DefaultedUnknown<RichStrng>,
	pub gateway: DefaultedUnknown<RichStrng>,
	pub listener: DefaultedUnknown<RichStrng>,
	pub route: DefaultedUnknown<RichStrng>,
	pub route_rule: DefaultedUnknown<RichStrng>,
}

#[derive(Clone, Hash, Default, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct HTTPLabels {
	pub backend: DefaultedUnknown<RichStrng>,
	pub protocol: DefaultedUnknown<EncodeDebug<crate::cel::BackendProtocol>>,

	pub method: DefaultedUnknown<EncodeDisplay<http::Method>>,
	pub status: DefaultedUnknown<EncodeDisplay<u16>>,
	pub reason: DefaultedUnknown<EncodeDisplay<ProxyResponseReason>>,

	#[prometheus(flatten)]
	pub route: RouteIdentifier,

	#[prometheus(flatten)]
	pub custom: CustomField,
}

#[derive(Clone, Hash, Default, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct GenAILabels {
	pub gen_ai_operation_name: DefaultedUnknown<RichStrng>,
	pub gen_ai_system: DefaultedUnknown<RichStrng>,
	pub gen_ai_request_model: DefaultedUnknown<RichStrng>,
	pub gen_ai_response_model: DefaultedUnknown<RichStrng>,

	#[prometheus(flatten)]
	pub route: RouteIdentifier,

	#[prometheus(flatten)]
	pub custom: CustomField,
}

#[derive(Clone, Hash, Default, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct GenAILabelsTokenUsage {
	pub gen_ai_token_type: DefaultedUnknown<RichStrng>,

	#[prometheus(flatten)]
	pub common: EncodeArc<GenAILabels>,
}

#[derive(Clone, Hash, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct MCPCall {
	pub method: DefaultedUnknown<RichStrng>,

	pub resource_type: DefaultedUnknown<MCPOperation>,
	pub server: DefaultedUnknown<RichStrng>,
	pub resource: DefaultedUnknown<RichStrng>,

	#[prometheus(flatten)]
	pub route: RouteIdentifier,

	#[prometheus(flatten)]
	pub custom: CustomField,
}

#[derive(Clone, Hash, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct TCPLabels {
	pub bind: DefaultedUnknown<RichStrng>,
	pub gateway: DefaultedUnknown<RichStrng>,
	pub listener: DefaultedUnknown<RichStrng>,
	pub protocol: BindProtocol,
}

#[derive(Clone, Hash, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct ConnectLabels {
	pub transport: DefaultedUnknown<RichStrng>,
}

type Counter = Family<HTTPLabels, counter::Counter>;
type Histogram<T> = Family<T, prometheus_client::metrics::histogram::Histogram>;
type TCPCounter = Family<TCPLabels, counter::Counter>;

#[derive(Clone, Hash, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct BuildLabel {
	tag: &'static str,
}

#[derive(Debug)]
pub struct Metrics {
	pub requests: Counter,
	pub downstream_connection: TCPCounter,

	pub mcp_requests: Family<MCPCall, counter::Counter>,

	pub gen_ai_token_usage: Histogram<GenAILabelsTokenUsage>,
	pub gen_ai_request_duration: Histogram<GenAILabels>,
	pub gen_ai_time_per_output_token: Histogram<GenAILabels>,
	pub gen_ai_time_to_first_token: Histogram<GenAILabels>,

	pub response_bytes: Family<HTTPLabels, counter::Counter>,

	pub tcp_downstream_rx_bytes: Family<TCPLabels, counter::Counter>,
	pub tcp_downstream_tx_bytes: Family<TCPLabels, counter::Counter>,

	pub upstream_connect_duration: Histogram<ConnectLabels>,
	pub tls_handshake_duration: Histogram<TCPLabels>,
}

// FilteredRegistry is a wrapper around Registry that allows to filter out certain metrics.
// Note: this currently only excludes them from the registry, but the underlying metrics are still
// stored. This can result in memory cost, etc to store the labels.
// A more robust future solution would be to have a sort of `Disabled` metric that does not store;
// note that even still, we would be computing the labels (and then dropping them), but in many cases
// the same labels are shared by many metrics, and are cheap to construct, so likely not a major concern.
struct FilteredRegistry<'a> {
	registry: &'a mut Registry,
	removes: FzHashSet<String>,
}

impl<'a> FilteredRegistry<'a> {
	fn should_skip(&self, name: &str, unit: Option<&Unit>) -> bool {
		let mut names = vec![
			name.to_string(),
			format!("{}_total", name),
			format!("{}_{}_total", agent_core::metrics::PREFIX, name),
			format!("{}_{}", agent_core::metrics::PREFIX, name),
		];
		if let Some(unit) = unit {
			names.extend_from_slice(&[
				format!("{}_{}", name, unit.as_str()),
				format!("{}_{}_total", name, unit.as_str()),
				format!(
					"{}_{}_{}_total",
					agent_core::metrics::PREFIX,
					name,
					unit.as_str()
				),
				format!("{}_{}_{}", agent_core::metrics::PREFIX, name, unit.as_str()),
			])
		}

		for n in names.into_iter() {
			let exclude = self.removes.contains(&n);
			trace!(name = n, exclude, "check metric for exclusion");
			if exclude {
				return true;
			}
		}
		false
	}
	fn register(&mut self, name: impl Into<String>, help: impl Into<String>, metric: impl Metric) {
		let name = name.into();
		if self.should_skip(&name, None) {
			debug!("skip register metric: {}", name);
			return;
		}
		self.registry.register(name, help, metric);
	}

	fn register_with_unit(
		&mut self,
		name: impl Into<String>,
		help: impl Into<String>,
		unit: Unit,
		metric: impl Metric,
	) {
		let name = name.into();
		if self.should_skip(&name, Some(&unit)) {
			debug!("skip register metric: {}_{}", name, unit.as_str());
			return;
		}
		self.registry.register_with_unit(name, help, unit, metric);
	}
}

impl Metrics {
	pub fn new(registry: &mut Registry, removes: FzHashSet<String>) -> Self {
		let mut registry = FilteredRegistry { registry, removes };
		registry.register(
			"build",
			"Agentgateway build information",
			Info::new(BuildLabel {
				tag: version::BuildInfo::new().version,
			}),
		);

		let gen_ai_token_usage = Family::<GenAILabelsTokenUsage, _>::new_with_constructor(move || {
			PromHistogram::new(TOKEN_USAGE_BUCKET)
		});
		registry.register(
			"gen_ai_client_token_usage",
			"Number of tokens used per request",
			gen_ai_token_usage.clone(),
		);

		// TODO: add error attribute if it ends with an error
		let gen_ai_request_duration = Family::<GenAILabels, _>::new_with_constructor(move || {
			PromHistogram::new(REQUEST_DURATION_BUCKET)
		});
		registry.register(
			"gen_ai_server_request_duration",
			"Duration of generative AI request",
			gen_ai_request_duration.clone(),
		);

		let gen_ai_time_per_output_token = Family::<GenAILabels, _>::new_with_constructor(move || {
			PromHistogram::new(OUTPUT_TOKEN_BUCKET)
		});
		registry.register(
			"gen_ai_server_time_per_output_token",
			"Time to generate each output token for a given request",
			gen_ai_time_per_output_token.clone(),
		);

		let gen_ai_time_to_first_token = Family::<GenAILabels, _>::new_with_constructor(move || {
			PromHistogram::new(FIRST_TOKEN_BUCKET)
		});
		registry.register(
			"gen_ai_server_time_to_first_token",
			"Time to generate the first token for a given request",
			gen_ai_time_to_first_token.clone(),
		);

		Metrics {
			requests: build(
				&mut registry,
				"requests",
				"The total number of HTTP requests sent",
			),
			downstream_connection: build(
				&mut registry,
				"downstream_connections",
				"The total number of downstream connections established",
			),

			mcp_requests: build(
				&mut registry,
				"mcp_requests",
				"Total number of MCP tool calls",
			),

			gen_ai_token_usage,
			gen_ai_request_duration,
			gen_ai_time_per_output_token,
			gen_ai_time_to_first_token,

			response_bytes: {
				let m = Family::<HTTPLabels, _>::default();
				registry.register_with_unit(
					"response",
					"Total HTTP response bytes received",
					Unit::Bytes,
					m.clone(),
				);
				m
			},
			tcp_downstream_rx_bytes: {
				let m = Family::<TCPLabels, _>::default();
				registry.register_with_unit(
					"downstream_received",
					"Total TCP bytes received per connection labels",
					Unit::Bytes,
					m.clone(),
				);
				m
			},
			tcp_downstream_tx_bytes: {
				let m = Family::<TCPLabels, _>::default();
				registry.register_with_unit(
					"downstream_sent",
					"Total TCP bytes transmitted per connection labels",
					Unit::Bytes,
					m.clone(),
				);
				m
			},
			upstream_connect_duration: {
				let m = Family::<ConnectLabels, _>::new_with_constructor(move || {
					PromHistogram::new(CONNECT_DURATION_BUCKET)
				});
				registry.register_with_unit(
					"upstream_connect_duration",
					"Duration to establish upstream connection (seconds)",
					Unit::Seconds,
					m.clone(),
				);
				m
			},
			tls_handshake_duration: {
				let m = Family::<TCPLabels, _>::new_with_constructor(move || {
					PromHistogram::new(CONNECT_DURATION_BUCKET)
				});
				registry.register_with_unit(
					"tls_handshake_duration",
					"Duration to complete inbound TLS/HTTPS handshake (seconds)",
					Unit::Seconds,
					m.clone(),
				);
				m
			},
		}
	}
}

fn build<'a, T: Clone + std::hash::Hash + Eq + Send + Sync + Debug + EncodeLabelSet + 'static>(
	registry: &mut FilteredRegistry<'a>,
	name: &str,
	help: &str,
) -> Family<T, counter::Counter> {
	let m = Family::<T, _>::default();
	registry.register(name, help, m.clone());
	m
}

// https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-metrics/#metric-gen_aiclienttokenusage
const TOKEN_USAGE_BUCKET: [f64; 14] = [
	1., 4., 16., 64., 256., 1024., 4096., 16384., 65536., 262144., 1048576., 4194304., 16777216.,
	67108864.,
];
// https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-metrics/#metric-gen_aiserverrequestduration
const REQUEST_DURATION_BUCKET: [f64; 14] = [
	0.01, 0.02, 0.04, 0.08, 0.16, 0.32, 0.64, 1.28, 2.56, 5.12, 10.24, 20.48, 40.96, 81.92,
];
// Finer-grained, exponentially growing buckets for TCP/TLS connect.
// Keep in seconds (Prometheus convention). Prioritize sub-second resolution, with a few larger outlier buckets.
const CONNECT_DURATION_BUCKET: [f64; 10] = [
	0.0005, // 0.5 ms
	0.0015, // 1.5 ms
	0.0043, // 4.3 ms
	0.0126, // 12.6 ms
	0.0368, // 36.8 ms
	0.108,  // 108 ms
	0.316,  // 316 ms
	0.924,  // 924 ms
	2.71,   // 2.71 s
	8.0,    // 8 s
];
// https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-metrics/#metric-gen_aiservertime_per_output_token
// NOTE: the spec has SHOULD, but is not smart enough to handle the faster LLMs.
// We have added 0.001 (1000 TPS)
const OUTPUT_TOKEN_BUCKET: [f64; 14] = [
	0.001, 0.01, 0.025, 0.05, 0.075, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.75, 1.0, 2.5,
];
// https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-metrics/#metric-gen_aiservertime_to_first_token
const FIRST_TOKEN_BUCKET: [f64; 16] = [
	0.001, 0.005, 0.01, 0.02, 0.04, 0.06, 0.08, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];
