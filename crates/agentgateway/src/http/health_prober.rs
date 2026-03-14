use crate::cel;
use crate::client::{ApplicationTransport, Call, Client, Transport};
use crate::http;
use crate::store::Stores;
use crate::types::agent::Target;
use crate::types::discovery::NamespacedHostname;
use agent_core::prelude::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::Instant;

pub async fn run(stores: Stores, client: Client) {
	let mut last_probe: HashMap<(NamespacedHostname, Strng), Instant> = HashMap::new();
	let mut interval = tokio::time::interval(Duration::from_millis(1000));

	loop {
		interval.tick().await;

		let probes_to_run = {
			let discovery = stores.discovery.read();
			let mut to_run = Vec::new();
			for svc in discovery.services.services() {
				let Some(health_policy) = &svc.health else {
					continue;
				};
				let Some(probe_cfg) = &health_policy.probe else {
					continue;
				};

				let nh = svc.namespaced_hostname();
				for (ep_uid, ep) in svc.endpoints.all_endpoints() {
					let key = (nh.clone(), ep_uid.clone());
					let last = last_probe.get(&key).copied();

					if last.is_none() || last.unwrap().elapsed() >= probe_cfg.interval {
						if let Some(w) = discovery.workloads.find_uid(&ep_uid) {
							if let Some(ip) = w.workload_ips.first().copied() {
								to_run.push((
									key.clone(),
									client.clone(),
									probe_cfg.clone(),
									svc.hostname.clone(),
									svc.endpoints.clone(),
									ep_uid.clone(),
									ip,
									ep.port.clone(),
									svc.ports.clone(),
									health_policy.eviction_duration(),
								));
							}
						}
					}
				}
			}
			to_run
		};

		for (
			key,
			client,
			probe_cfg,
			svc_hostname,
			endpoint_set,
			ep_uid,
			ip,
			ep_ports,
			svc_ports,
			eviction_duration,
		) in probes_to_run
		{
			last_probe.insert(key, Instant::now());
			tokio::spawn(async move {
				if let Err(e) = probe_endpoint(
					&client,
					&probe_cfg,
					&svc_hostname,
					&endpoint_set,
					ep_uid,
					ip,
					ep_ports,
					svc_ports,
					eviction_duration,
				)
				.await
				{
					debug!("probe failed for {}: {}", svc_hostname, e);
				}
			});
		}
	}
}

async fn probe_endpoint(
	client: &Client,
	probe: &crate::http::health::Probe,
	_svc_hostname: &str,
	endpoint_set: &crate::types::loadbalancer::EndpointSet<crate::types::discovery::Endpoint>,
	ep_uid: Strng,
	ip: std::net::IpAddr,
	ep_ports: HashMap<u16, u16>,
	svc_ports: HashMap<u16, u16>,
	eviction_duration: Option<Duration>,
) -> anyhow::Result<()> {
	// Pick a port.
	let svc_port = svc_ports.keys().next().copied().unwrap_or(80);
	let target_port = ep_ports.get(&svc_port).copied().unwrap_or(svc_port);

	let host = probe.host.as_deref().filter(|s| !s.is_empty());
	let authority = match host {
		Some(h) => format!("{}:{}", h, target_port),
		None => format!("{}:{}", ip, target_port),
	};

	let url = format!("http://{}{}", authority, probe.path);

	let mut req_builder = ::http::Request::builder()
		.method(::http::Method::GET)
		.uri(&url);

	if let Some(h) = host {
		req_builder = req_builder.header(::http::header::HOST, h);
	}

	let req = req_builder.body(http::Body::default())?;

	let target = Target::Address(std::net::SocketAddr::new(ip, target_port));
	let transport = Transport::Plain(ApplicationTransport::Plaintext);

	let call = Call {
		req,
		target,
		transport,
	};

	let resp = tokio::time::timeout(probe.timeout, client.call(call)).await;

	let success = match resp {
		Ok(Ok(resp)) => {
			let executor = cel::Executor::new_response(None, &resp);
			executor.eval_bool(&probe.expected_condition)
		},
		_ => false,
	};

	if !success {
		let evict_until = Instant::now() + eviction_duration.unwrap_or(Duration::from_secs(30));
		endpoint_set.evict(ep_uid, evict_until.into()).await;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::cel::Expression;
	use crate::http::health::Probe;

	#[tokio::test]
	async fn test_probe_logic() {
		let _probe = Probe {
			interval: Duration::from_secs(1),
			timeout: Duration::from_secs(1),
			expected_condition: Arc::new(Expression::new_strict("response.code == 200").unwrap()),
			host: None,
			path: "/health".into(),
		};

		// This is just a compilation and structural test for now.
		// Integration testing would require a full MockConnector.
	}
}
