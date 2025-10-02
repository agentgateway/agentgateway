use std::net::SocketAddr;
use std::sync::Arc;

use rand::prelude::IndexedRandom;

use crate::proxy::httpproxy::BackendCall;
use crate::proxy::{ProxyError, httpproxy};
use crate::telemetry::log;
use crate::telemetry::log::{DropOnLog, RequestLog};
use crate::telemetry::metrics::TCPLabels;
use crate::transport::stream::{Socket, TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent;
use crate::types::agent::{
	BindName, BindProtocol, Listener, ListenerProtocol, SimpleBackend, TCPRoute, TCPRouteBackend,
	TCPRouteBackendReference,
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

		let service = match &selected_backend.backend {
			SimpleBackend::Service(svc, _) => Some(strng::format!("{}/{}", svc.namespace, svc.hostname)),
			_ => None,
		};
		let backend_call = match &selected_backend.backend {
			SimpleBackend::Service(svc, port) => {
				httpproxy::build_service_call(inputs.as_ref(), None, &mut Some(log), None, svc, port)?
			},
			SimpleBackend::Opaque(_, target) => BackendCall {
				target: target.clone(),
				http_version_override: None,
				transport_override: None,
				default_policies: None,
			},
			SimpleBackend::Invalid => return Err(ProxyError::BackendDoesNotExist),
		};
		log.endpoint = Some(backend_call.target.clone());
		let policies =
			inputs
				.stores
				.read_binds()
				.backend_policies(selected_backend.backend.name(), service, None);
		let transport = crate::proxy::httpproxy::build_transport(
			&inputs,
			&backend_call,
			policies.backend_tls.clone(),
		)
		.await?;

		inputs
			.upstream
			.call_tcp(client::TCPCall {
				source: connection,
				target: backend_call.target,
				transport,
			})
			.await?;
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
