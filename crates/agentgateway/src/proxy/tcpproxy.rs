use std::net::SocketAddr;
use std::sync::Arc;

use rand::prelude::IndexedRandom;

use crate::proxy::ProxyError;
use crate::telemetry::log;
use crate::telemetry::log::{DropOnLog, RequestLog};
use crate::telemetry::metrics::TCPLabels;
use crate::transport::stream::{Socket, TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent;
use crate::types::agent::{
	BindName, BindProtocol, Listener, ListenerProtocol, PolicyTarget, SimpleBackend, TCPRoute,
	TCPRouteBackend, TCPRouteBackendReference, Target,
};
use crate::{ProxyInputs, *};

#[derive(Clone)]
pub struct TCPProxy {
	pub(super) bind_name: BindName,
	pub(super) inputs: Arc<ProxyInputs>,
	pub(super) selected_listener: Arc<Listener>,
	#[allow(unused)]
	pub(super) target_address: SocketAddr,
}

impl TCPProxy {
	pub async fn proxy(&self, connection: Socket) {
		let start = Instant::now();
		let start_time = agent_core::telemetry::render_current_time();

		let tcp = connection
			.ext::<TCPConnectionInfo>()
			.expect("tcp connection must be set");
		let mut log: DropOnLog = RequestLog::new(
			log::CelLogging::new(
				self.inputs.cfg.logging.clone(),
				self.inputs.cfg.tracing.clone(),
			),
			self.inputs.metrics.clone(),
			start,
			start_time,
			tcp.clone(),
		)
		.into();
		let ret = self.proxy_internal(connection, log.as_mut().unwrap()).await;
		if let Err(e) = ret {
			log.with(|l| l.error = Some(e.to_string()));
		}
	}

	async fn proxy_internal(
		&self,
		connection: Socket,
		log: &mut RequestLog,
	) -> Result<(), ProxyError> {
		log.tls_info = connection.ext::<TLSConnectionInfo>().cloned();
		self
			.inputs
			.metrics
			.downstream_connection
			.get_or_create(&TCPLabels {
				bind: Some(&self.bind_name).into(),
				gateway: Some(&self.selected_listener.gateway_name).into(),
				listener: Some(&self.selected_listener.name).into(),
				protocol: if log.tls_info.is_some() {
					BindProtocol::tls
				} else {
					BindProtocol::tcp
				},
			})
			.inc();
		let sni = log
			.tls_info
			.as_ref()
			.and_then(|tls| tls.server_name.as_deref());

		let selected_listener = self.selected_listener.clone();
		let _upstream = self.inputs.upstream.clone();
		let inputs = self.inputs.clone();
		let bind_name = self.bind_name.clone();
		debug!(bind=%bind_name, "route for bind");
		log.bind_name = Some(bind_name.clone());
		log.gateway_name = Some(selected_listener.gateway_name.clone());
		log.listener_name = Some(selected_listener.name.clone());
		debug!(bind=%bind_name, listener=%selected_listener.key, "selected listener");

		let selected_route =
			select_best_route(sni, selected_listener.clone()).ok_or(ProxyError::RouteNotFound)?;
		log.route_rule_name = selected_route.rule_name.clone();
		log.route_name = Some(selected_route.route_name.clone());

		debug!(bind=%bind_name, listener=%selected_listener.key, route=%selected_route.key, "selected route");
		let selected_backend =
			select_tcp_backend(selected_route.as_ref()).ok_or(ProxyError::NoValidBackends)?;
		let selected_backend = resolve_backend(selected_backend, self.inputs.as_ref())?;

		let (_target, _policy_key) = match &selected_backend.backend {
			SimpleBackend::Service(svc, port) => {
				let port = *port;
				let workloads = &inputs.stores.read_discovery().workloads;
				let (ep, _, wl) = svc
					.endpoints
					.select_endpoint(workloads, svc.as_ref(), port, None)
					.ok_or(ProxyError::NoHealthyEndpoints)?;
				let svc_target_port = svc.ports.get(&port).copied().unwrap_or_default();
				let target_port = if let Some(&ep_target_port) = ep.port.get(&port) {
					// use endpoint port mapping
					ep_target_port
				} else if svc_target_port > 0 {
					// otherwise, check if the service has the port
					svc_target_port
				} else {
					return Err(ProxyError::NoHealthyEndpoints);
				};
				let Some(ip) = wl.workload_ips.first() else {
					return Err(ProxyError::NoHealthyEndpoints);
				};
				let dest = std::net::SocketAddr::from((*ip, target_port));
				(
					Target::Address(dest),
					PolicyTarget::Backend(selected_backend.backend.name()),
				)
			},
			SimpleBackend::Opaque(name, target) => (target.clone(), PolicyTarget::Backend(name.clone())),
			SimpleBackend::Invalid => return Err(ProxyError::BackendDoesNotExist),
		};

		// let _policies = inputs
		// .stores
		// .read_binds()
		// .backend_policies(todo!(), None);
		// let transport = policies.i // TODO
		// let Target::Address(addr) = _target else {
		// panic!("TODO")
		// };
		// let upstream = stream::Socket::dial(addr)
		// .await
		// .map_err(ProxyError::Processing)?;
		// agent_core::copy::copy_bidirectional(
		// connection,
		// upstream,
		// &agent_core::copy::ConnectionResult {},
		// )
		// .await
		// .map_err(|e| ProxyError::Processing(e.into()))?;
		//
		Ok(())
	}
}

fn select_best_route(host: Option<&str>, listener: Arc<Listener>) -> Option<Arc<TCPRoute>> {
	// TCP matching is much simpler than HTTP.
	// We pick the best matching hostname, else fallback to precedence:
	//
	//  * The oldest Route based on creation timestamp.
	//  * The Route appearing first in alphabetical order by "{namespace}/{name}".

	// Assume matches are ordered already (not true today)
	if matches!(listener.protocol, ListenerProtocol::HBONE) && listener.routes.is_empty() {
		// TODO: TCP for waypoint
		return None;
	}
	for hnm in agent::HostnameMatch::all_matches_or_none(host) {
		if let Some(r) = listener.tcp_routes.get_hostname(&hnm) {
			return Some(Arc::new(r.clone()));
		}
	}
	None
}

fn select_tcp_backend(route: &TCPRoute) -> Option<TCPRouteBackendReference> {
	route
		.backends
		.choose_weighted(&mut rand::rng(), |b| b.weight)
		.ok()
		.cloned()
}

fn resolve_backend(
	b: TCPRouteBackendReference,
	pi: &ProxyInputs,
) -> Result<TCPRouteBackend, ProxyError> {
	let backend = super::resolve_simple_backend(&b.backend, pi)?;
	Ok(TCPRouteBackend {
		weight: b.weight,
		backend,
	})
}
