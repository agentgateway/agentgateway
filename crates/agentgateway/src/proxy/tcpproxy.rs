use std::net::SocketAddr;
use std::sync::Arc;

use crate::cel::SourceContext;
use rand::prelude::IndexedRandom;

use crate::proxy::httpproxy::BackendCall;
use crate::proxy::{ProxyError, httpproxy};
use crate::store::{BackendPolicies, RoutePath};
use crate::telemetry::log;
use crate::telemetry::log::{DropOnLog, RequestLog};
use crate::telemetry::metrics::TCPLabels;
use crate::transport::stream::{Socket, TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent;
use crate::types::agent::{
	BackendPolicy, BindKey, Listener, ListenerProtocol, SimpleBackend, SimpleBackendWithPolicies,
	TCPRoute, TCPRouteBackend, TCPRouteBackendReference, TransportProtocol,
};
use crate::{ProxyInputs, *};

#[derive(Clone)]
pub struct TCPProxy {
	pub(super) bind_name: BindKey,
	pub(super) inputs: Arc<ProxyInputs>,
	pub(super) selected_listener: Arc<Listener>,
	#[allow(unused)]
	pub(super) target_address: SocketAddr,
}

impl TCPProxy {
	pub async fn proxy(&self, connection: Socket) {
		let start = Instant::now();

		let tcp = connection
			.ext::<TCPConnectionInfo>()
			.expect("tcp connection must be set");
		let tls = connection.ext::<TLSConnectionInfo>();
		let src = SourceContext {
			address: tcp.peer_addr.ip(),
			port: tcp.peer_addr.port(),
			tls: tls.and_then(|t| t.src_identity.clone()),
		};
		let mut log: DropOnLog = RequestLog::new(
			log::CelLogging::new(
				self.inputs.cfg.logging.clone(),
				self.inputs.cfg.tracing.clone(),
			),
			self.inputs.metrics.clone(),
			start,
			tcp.clone(),
		)
		.into();
		// Set source context for TCP logging
		log.with(|l| l.source_context = Some(src));
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
		let frontend_policies = self
			.inputs
			.stores
			.read_binds()
			.frontend_policies(self.inputs.cfg.gateway_ref());

		// Apply frontend policies for access logging (skip tracing for TCP)
		frontend_policies.register_cel_expressions(log.cel.ctx());
		if let Some(lp) = &frontend_policies.access_log {
			httpproxy::apply_logging_policy_to_log(log, lp);
		}

		log.tls_info = connection.ext::<TLSConnectionInfo>().cloned();
		log.backend_protocol = Some(cel::BackendProtocol::tcp);
		let tcp_labels = TCPLabels {
			bind: Some(&self.bind_name).into(),
			gateway: Some(&self.selected_listener.name.as_gateway_name()).into(),
			listener: self.selected_listener.name.listener_name.clone().into(),
			protocol: if log.tls_info.is_some() {
				TransportProtocol::tls
			} else {
				TransportProtocol::tcp
			},
		};
		self
			.inputs
			.metrics
			.downstream_connection
			.get_or_create(&tcp_labels)
			.inc();
		let sni = log
			.tls_info
			.as_ref()
			.and_then(|tls| tls.server_name.as_deref());

		let selected_listener = self.selected_listener.clone();
		let inputs = self.inputs.clone();
		let bind_name = self.bind_name.clone();
		debug!(bind=%bind_name, "route for bind");
		log.bind_name = Some(bind_name.clone());
		log.listener_name = Some(selected_listener.name.clone());
		debug!(bind=%bind_name, listener=%selected_listener.key, "selected listener");

		let selected_route =
			select_best_route(sni, selected_listener.clone()).ok_or(ProxyError::RouteNotFound)?;
		log.route_name = Some(selected_route.name.clone());

		let route_path = RoutePath {
			route: &selected_route.name,
			listener: &selected_listener.name,
		};

		debug!(bind=%bind_name, listener=%selected_listener.key, route=%selected_route.key, "selected route");
		let selected_backend =
			select_tcp_backend(selected_route.as_ref()).ok_or(ProxyError::NoValidBackends)?;
		let selected_backend = resolve_backend(selected_backend, self.inputs.as_ref())?;
		let backend_policies = get_backend_policies(
			&self.inputs,
			&selected_backend.backend,
			&selected_backend.inline_policies,
			route_path,
		);

		let backend_call = match &selected_backend.backend.backend {
			SimpleBackend::Service(svc, port) => httpproxy::build_service_call(
				inputs.as_ref(),
				backend_policies,
				&mut Some(log),
				None,
				svc,
				port,
			)?,
			SimpleBackend::Opaque(_, target) => BackendCall {
				target: target.clone(),
				http_version_override: None,
				transport_override: None,
				network_gateway: None,
				backend_policies,
			},
			SimpleBackend::Invalid => return Err(ProxyError::BackendDoesNotExist),
		};

		let bi = selected_backend.backend.backend.backend_info();
		log.endpoint = Some(backend_call.target.clone());
		log.backend_info = Some(bi);

		let transport = crate::proxy::httpproxy::build_transport(
			&inputs,
			&backend_call,
			backend_call.backend_policies.backend_tls.clone(),
			// TODO: for TCP we should actually probably do something here: telling it to not use ALPN at all?
			None,
		)
		.await?;

		// export rx/tx bytes on drop
		let mut connection = connection;
		connection.set_transport_metrics(self.inputs.metrics.clone(), tcp_labels);

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
		inline_policies: b.inline_policies,
	})
}

pub fn get_backend_policies(
	inputs: &ProxyInputs,
	backend: &SimpleBackendWithPolicies,
	inline_policies: &[BackendPolicy],
	route_path: RoutePath,
) -> BackendPolicies {
	inputs.stores.read_binds().backend_policies(
		backend.backend.target(),
		&[&backend.inline_policies, inline_policies],
		Some(route_path),
	)
}
