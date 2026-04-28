use crate::telemetry::log::MetricsConfig;

pub fn policy_client() -> crate::proxy::httpproxy::PolicyClient {
	let proxy = super::proxymock::setup_proxy_test("{}").expect("proxy test harness");
	crate::proxy::httpproxy::PolicyClient {
		inputs: proxy.inputs(),
	}
}

pub async fn test_policy<P>(
	policy: &P,
	req: &mut crate::http::Request,
) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse>
where
	P: crate::store::RequestPolicyTrait + ?Sized,
{
	let client = policy_client();
	let mut log = make_min_req_log();
	policy.apply(&client, &mut log, req).await
}

fn make_min_req_log() -> crate::telemetry::log::RequestLog {
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};
	use std::sync::Arc;

	use frozen_collections::FzHashSet;
	use prometheus_client::registry::Registry;

	use crate::telemetry::log;
	use crate::telemetry::log::{LoggingFields, RequestLog};
	use crate::telemetry::metrics::Metrics;
	use crate::transport::stream::TCPConnectionInfo;

	let log_cfg = log::Config {
		filter: None,
		fields: LoggingFields::default(),
		level: "info".to_string(),
		format: crate::LoggingFormat::Text,
	};
	let cel = log::CelLogging::new(log_cfg, MetricsConfig::default());
	let mut prom = Registry::default();
	let metrics = Arc::new(Metrics::new(&mut prom, FzHashSet::default()));
	let start = agent_core::Timestamp::now();
	let tcp_info = TCPConnectionInfo {
		peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345),
		local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080),
		start: start.as_instant(),
		raw_peer_addr: None,
	};
	RequestLog::new(cel, metrics, start, tcp_info)
}
