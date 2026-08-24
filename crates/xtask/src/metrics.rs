use std::fmt::Write;

use agent_core::metrics::{MetricRegistry, PREFIX};
use agentgateway::HistogramMode;
use anyhow::Result;
use prometheus_client::metrics::MetricType;
use prometheus_client::registry::{Metric, Unit};

struct MetricDescription {
	name: String,
	kind: MetricType,
	unit: Option<String>,
	help: String,
}

#[derive(Default)]
struct DocumentationRegistry {
	metrics: Vec<MetricDescription>,
}

impl DocumentationRegistry {
	fn push(
		&mut self,
		name: impl Into<String>,
		help: impl Into<String>,
		unit: Option<Unit>,
		metric: impl Metric,
	) {
		let mut name = format!("{PREFIX}_{}", name.into());
		let unit = unit.map(|unit| unit.as_str().to_string());
		if let Some(unit) = &unit {
			name.push('_');
			name.push_str(unit);
		}
		let kind = metric.metric_type();
		match kind {
			MetricType::Counter => name.push_str("_total"),
			MetricType::Info => name.push_str("_info"),
			MetricType::Gauge | MetricType::Histogram | MetricType::Unknown => {},
		}
		self.metrics.push(MetricDescription {
			name,
			kind,
			unit,
			help: help.into() + ".",
		});
	}
}

impl MetricRegistry for DocumentationRegistry {
	fn register(&mut self, name: impl Into<String>, help: impl Into<String>, metric: impl Metric) {
		self.push(name, help, None, metric);
	}

	fn register_with_unit(
		&mut self,
		name: impl Into<String>,
		help: impl Into<String>,
		unit: Unit,
		metric: impl Metric,
	) {
		self.push(name, help, Some(unit), metric);
	}
}

pub fn generate_metrics() -> Result<()> {
	let mut registry = DocumentationRegistry::default();
	agent_xds::Metrics::new(&mut registry);
	agentgateway::telemetry::metrics::Metrics::new(
		&mut registry,
		Default::default(),
		HistogramMode::Classic,
	);
	registry.metrics.sort_by(|a, b| a.name.cmp(&b.name));

	let mut groups: [Vec<MetricDescription>; 6] = Default::default();
	for metric in registry.metrics {
		let name = metric.name.strip_prefix("agentgateway_").unwrap();
		let group = if name.starts_with("xds_") {
			0
		} else if name.starts_with("request_")
			|| name.starts_with("requests_")
			|| name.starts_with("response_")
			|| name.starts_with("retries_")
		{
			1
		} else if name.starts_with("downstream_")
			|| name.starts_with("tls_")
			|| name.starts_with("upstream_connect_")
		{
			2
		} else if name.starts_with("mcp_") {
			3
		} else if name.starts_with("gen_ai_")
			|| name.starts_with("guardrail_")
			|| name.starts_with("cost_catalog_")
		{
			4
		} else {
			5
		};
		groups[group].push(metric);
	}

	let mut markdown = String::from("# Metrics\n");
	for (name, metrics) in ["XDS", "HTTP", "TCP", "MCP", "LLM", "Misc"]
		.into_iter()
		.zip(groups)
	{
		writeln!(
			markdown,
			"\n## {name}\n\n| Metric | Type | Unit | Description |\n| --- | --- | --- | --- |"
		)?;
		for metric in metrics {
			writeln!(
				markdown,
				"| `{}` | {} | {} | {} |",
				metric.name,
				match metric.kind {
					MetricType::Counter => "Counter",
					MetricType::Gauge => "Gauge",
					MetricType::Histogram => "Histogram",
					MetricType::Info => "Info",
					MetricType::Unknown => "Unknown",
				},
				metric.unit.as_deref().unwrap_or("—"),
				metric.help.replace('|', "\\|").replace('\n', "<br>")
			)?;
		}
	}

	let xtask_path = std::env::var("CARGO_MANIFEST_DIR")?;
	fs_err::write(format!("{xtask_path}/../../schema/metrics.md"), markdown)?;
	Ok(())
}
