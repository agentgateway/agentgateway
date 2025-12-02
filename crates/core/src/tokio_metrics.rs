use std::fmt::Error;

use prometheus_client::collector::Collector;
use prometheus_client::encoding::{DescriptorEncoder, EncodeMetric};
use prometheus_client::metrics::gauge::ConstGauge;
use prometheus_client::registry::Registry;

#[derive(Debug)]
pub struct TokioCollector {
	metrics: tokio::runtime::RuntimeMetrics,
}

impl TokioCollector {
	pub fn register(registry: &mut Registry, handle: &tokio::runtime::Handle) {
		let me = TokioCollector {
			metrics: handle.metrics(),
		};
		registry.register_collector(Box::new(me));
	}
}

macro_rules! encode {
	($self:expr, $encoder:expr, $metric_type:ident, $name:tt, $help:expr) => {{
		let metric = $metric_type::new($self.metrics.$name() as u64);
		let metric_encoder = $encoder.encode_descriptor(
			concat!("tokio_", stringify!($name)),
			$help,
			None,
			metric.metric_type(),
		)?;
		metric.encode(metric_encoder)?;
	}};
}

impl Collector for TokioCollector {
	fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), Error> {
		encode!(
			self,
			&mut encoder,
			ConstGauge,
			global_queue_depth,
			"number of tasks currently scheduled in the runtime’s global queue"
		);
		encode!(
			self,
			&mut encoder,
			ConstGauge,
			num_alive_tasks,
			"number of currently alive tasks in the runtime"
		);
		encode!(
			self,
			&mut encoder,
			ConstGauge,
			num_workers,
			"number of worker threads used by the runtime"
		);
		Ok(())
	}
}
