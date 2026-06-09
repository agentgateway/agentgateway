use agent_core::metrics::Recorder;
use prometheus_client::encoding::{EncodeLabelSet, EncodeLabelValue};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::{Registry, Unit};

use super::service::discovery::v3::DeltaDiscoveryResponse;

#[derive(Debug)]
pub struct Metrics {
	pub connection_terminations: Family<ConnectionTermination, Counter>,
	pub message_types: Family<TypeUrl, Counter>,
	pub total_messages_size: Family<TypeUrl, Counter>,
	pub config_synchronized: Gauge,
}

#[derive(Clone, Hash, Debug, PartialEq, Eq, EncodeLabelSet)]
pub struct ConnectionTermination {
	pub reason: ConnectionTerminationReason,
}

#[derive(Copy, Clone, Hash, Debug, PartialEq, Eq, EncodeLabelValue)]
pub enum ConnectionTerminationReason {
	ConnectionError,
	Error,
	Reconnect,
	Complete,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TypeUrl {
	pub url: String,
}

impl Metrics {
	pub fn new(registry: &mut Registry) -> Self {
		let connection_terminations = Family::default();
		registry.register(
			"xds_connection_terminations",
			"The total number of completed connections to xds server (unstable)",
			connection_terminations.clone(),
		);

		let message_count = Family::default();

		registry.register(
			"xds_message",
			"Total number of messages received (unstable)",
			message_count.clone(),
		);

		let total_messages_size = Family::default();

		registry.register_with_unit(
			"xds_message",
			"Total number of bytes received (unstable)",
			Unit::Bytes,
			total_messages_size.clone(),
		);

		let config_synchronized = Gauge::default();
		config_synchronized.set(1);
		registry.register(
            "config_synchronized",
            "Whether the last configuration load/reload was successful or not, being synchronized with the on-disk configuration",
            config_synchronized.clone()
        );

		Self {
			connection_terminations,
			message_types: message_count,
			total_messages_size,
			config_synchronized,
		}
	}
}

impl Recorder<&ConnectionTerminationReason, u64> for Metrics {
	fn record(&self, reason: &ConnectionTerminationReason, count: u64) {
		self
			.connection_terminations
			.get_or_create(&ConnectionTermination { reason: *reason })
			.inc_by(count);
	}
}

impl Recorder<&DeltaDiscoveryResponse, ()> for Metrics {
	fn record(&self, response: &DeltaDiscoveryResponse, _: ()) {
		let type_url = TypeUrl {
			url: response.type_url.clone(),
		};
		self.message_types.get_or_create(&type_url).inc();

		let mut total_message_size: u64 = 0;
		for resource in &response.resources {
			total_message_size += resource
				.resource
				.as_ref()
				.map(|v| v.value.len())
				.unwrap_or_default() as u64;
		}
		self
			.total_messages_size
			.get_or_create(&type_url)
			.inc_by(total_message_size);
	}
}
