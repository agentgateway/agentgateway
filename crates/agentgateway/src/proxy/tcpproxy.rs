use crate::cel::SourceContext;
use futures::pin_mut;
use rand::prelude::IndexedRandom;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::proxy::httpproxy::BackendCall;
use crate::proxy::{ProxyError, WaypointService, httpproxy};
use crate::store::{BackendPolicies, FrontendPolices, RoutePath};
use crate::telemetry::log;
use crate::telemetry::log::{DropOnLog, RequestLog};
use crate::telemetry::metrics::TCPLabels;
use crate::transport::stream::{Socket, TCPConnectionInfo, TLSConnectionInfo, WaypointTLSInfo};
use crate::types::agent::{
	BackendPolicy, BindKey, Listener, ListenerProtocol, SimpleBackend, SimpleBackendReference,
	SimpleBackendWithPolicies, TCPRoute, TCPRouteBackend, TCPRouteBackendReference, Target,
	TransportProtocol,
};
use crate::types::discovery::{NetworkAddress, WaypointIdentity, gatewayaddress::Destination};
use crate::types::{agent, frontend};
use crate::{ProxyInputs, Stores, *};

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
		let start = agent_core::Timestamp::now();

		let tcp = connection
			.ext::<TCPConnectionInfo>()
			.expect("tcp connection must be set");
		let tls = connection.ext::<TLSConnectionInfo>();
		let unverified_workload = crate::cel::WorkloadContext::from_stores(
			&self.inputs.stores,
			&self.inputs.cfg.network,
			tcp.peer_addr.ip(),
		);
		let src = SourceContext::from_tcp_connection(
			tcp,
			tls.and_then(|t| t.src_identity.clone()),
			unverified_workload,
		);
		let mut log: DropOnLog = RequestLog::new(
			log::CelLogging::new(
				self.inputs.cfg.logging.clone(),
				self.inputs.cfg.metrics.clone(),
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
		let frontend_policies = {
			let binds = self.inputs.stores.read_binds();
			let bind_port = binds.bind(&self.bind_name).map(|bind| bind.address.port());
			binds.listener_frontend_policies(
				&self.selected_listener.name,
				bind_port,
				connection
					.ext::<WaypointService>()
					.map(WaypointService::as_policy_ref),
			)
		};

		// Apply frontend policies for access logging (skip tracing for TCP)
		frontend_policies.register_cel_expressions(log.cel.ctx());
		if let Some(lp) = &frontend_policies.access_log {
			httpproxy::apply_logging_policy_to_log(log, lp);
		}

		// Only sniff TLS if: From waypoint AND service port's appProtocol is TLS
		let should_sniff = connection
			.ext::<WaypointTLSInfo>()
			.map(|info| info.should_sniff_tls)
			.unwrap_or(false);

		let connection = if should_sniff {
			Self::sniff_tls_sni(connection, &frontend_policies)
				.await
				.map_err(ProxyError::Processing)?
		} else {
			connection
		};
		if let Some(authz) = frontend_policies.network_authorization.as_ref() {
			authz.apply(
				log
					.source_context
					.as_ref()
					.expect("expected source context"),
			)?;
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
			.and_then(|tls| tls.server_name.as_deref())
			.map(|s| s.to_string());

		let selected_listener = self.selected_listener.clone();
		let inputs = self.inputs.clone();
		let bind_name = self.bind_name.clone();
		debug!(bind=%bind_name, "route for bind");
		log.bind_name = Some(bind_name.clone());
		log.listener_name = Some(selected_listener.name.clone());
		debug!(bind=%bind_name, listener=%selected_listener.key, "selected listener");

		let selected_route = select_best_route(
			sni.as_deref(),
			selected_listener.clone(),
			&self.inputs.stores,
			&self.inputs.cfg.network,
			self.target_address,
			self.inputs.cfg.self_addr.as_ref(),
		)
		.ok_or(ProxyError::RouteNotFound)?;
		log.route_name = Some(selected_route.name.clone());

		let route_path = RoutePath {
			routes: vec![&selected_route.name],
			service: selected_route.service_key.as_ref(),
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
			Some(route_path),
		);

		let backend_call = Self::build_backend_call(
			&mut Some(log),
			sni,
			&inputs,
			&selected_backend.backend.backend,
			backend_policies,
		)?;

		let bi = selected_backend.backend.backend.backend_info();
		log.endpoint = Some(backend_call.target.clone());
		log.backend_info = Some(bi);

		let transport = crate::proxy::httpproxy::build_transport(
			&inputs,
			&backend_call,
			backend_call.backend_policies.backend_tls.clone(),
			backend_call.backend_policies.tunnel.as_ref(),
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

	pub fn build_backend_call(
		log: &mut Option<&mut RequestLog>,
		sni: Option<String>,
		inputs: &ProxyInputs,
		selected_backend: &SimpleBackend,
		backend_policies: BackendPolicies,
	) -> Result<BackendCall, ProxyError> {
		let backend_call = match &selected_backend {
			SimpleBackend::Service(svc, port) => httpproxy::build_service_call(
				inputs,
				backend_policies,
				log,
				httpproxy::ServiceCallOverride::default(),
				svc,
				port,
				sni.as_deref(),
			)?,
			SimpleBackend::Opaque(_, target) => BackendCall {
				target: target.clone(),
				http_version_override: None,
				transport_override: None,
				network_gateway: None,
				backend_policies,
				ingress_waypoint: None,
			},
			SimpleBackend::Aws(_, config) => {
				let default_policies = BackendPolicies {
					backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
					backend_auth: Some(http::auth::BackendAuth::Aws(
						http::auth::AwsAuth::Implicit {},
					)),
					..Default::default()
				};
				BackendCall {
					target: Target::Hostname(config.get_host().into(), 443),
					http_version_override: None,
					transport_override: None,
					network_gateway: None,
					backend_policies: default_policies.merge(backend_policies),
				}
			},
			SimpleBackend::Invalid => return Err(ProxyError::BackendDoesNotExist),
		};
		Ok(backend_call)
	}

	async fn sniff_tls_sni(raw_stream: Socket, policies: &FrontendPolices) -> anyhow::Result<Socket> {
		let def = frontend::TLS::default();
		let tls_pol = policies.tls.as_ref();
		let to = tls_pol.unwrap_or(&def).handshake_timeout;
		let handshake = async move {
			let (mut ext, counter, inner) = raw_stream.into_parts();
			let inner = Socket::new_rewind(inner);
			let acceptor =
				tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), inner);
			pin_mut!(acceptor);
			let mut start = acceptor.as_mut().await?;
			let ch = start.client_hello();
			let sni = ch.server_name().unwrap_or_default().to_string();
			start.io.rewind();
			let existing = ext.remove().unwrap_or_default();
			ext.insert(TLSConnectionInfo {
				server_name: Some(sni),
				..existing
			});
			Ok(Socket::from_rewind(ext, counter, start.io))
		};
		tokio::time::timeout(to, handshake).await?
	}
}

fn select_best_route(
	host: Option<&str>,
	listener: Arc<Listener>,
	stores: &Stores,
	network: &Strng,
	dst: SocketAddr,
	self_addr: Option<&WaypointIdentity>,
) -> Option<Arc<TCPRoute>> {
	// TCP matching is much simpler than HTTP.
	// We pick the best matching hostname, else fallback to precedence:
	//
	//  * The oldest Route based on creation timestamp.
	//  * The Route appearing first in alphabetical order by "{namespace}/{name}".

	// Assume matches are ordered already (not true today)

	// Try explicit TCP routes first
	let listener_tcp_routes = {
		let binds = stores.read_binds();
		binds.get_listener_tcp_routes(&listener.key)
	};
	if let Some(listener_tcp_routes) = listener_tcp_routes {
		for hnm in agent::HostnameMatch::all_matches_or_none(host) {
			if let Some(r) = listener_tcp_routes.get_hostname(&hnm) {
				return Some(Arc::new(r.clone()));
			}
		}
	}

	// For HBONE waypoints, check service-keyed routes then fall back to default
	if matches!(listener.protocol, ListenerProtocol::HBONE) {
		let svc = resolve_waypoint_service(stores, network, dst, self_addr)?;

		// When routes are attached to a Service via parentRef, they take priority
		// over listener-attached routes. If service routes exist but none match,
		// the request is rejected (per GAMMA spec).
		let svc_nh = svc.namespaced_hostname();
		let svc_tcp_routes = {
			let binds = stores.read_binds();
			binds.get_service_tcp_routes(&svc_nh)
		};
		if let Some(svc_tcp_routes) = svc_tcp_routes {
			for hnm in agent::HostnameMatch::all_matches(&svc.hostname) {
				if let Some(r) = svc_tcp_routes.get_hostname(&hnm) {
					return Some(Arc::new(r.clone()));
				}
			}
			// GAMMA: service routes exist but none matched -> reject
			return None;
		}

		// No service-keyed routes: generate default passthrough
		return Some(Arc::new(TCPRoute {
			key: strng::literal!("_waypoint-default-tcp"),
			service_key: None,
			name: crate::types::agent::RouteName {
				name: strng::literal!("_waypoint-default-tcp"),
				namespace: svc.namespace.clone(),
				rule_name: None,
				kind: None,
			},
			hostnames: vec![],
			backends: vec![TCPRouteBackendReference {
				weight: 1,
				backend: SimpleBackendReference::Service {
					name: svc.namespaced_hostname(),
					port: dst.port(),
				},
				inline_policies: Vec::new(),
			}],
		}));
	}
	None
}

/// Resolve the waypoint service from a VIP and verify this proxy owns it.
fn resolve_waypoint_service(
	stores: &Stores,
	network: &Strng,
	dst: SocketAddr,
	self_addr: Option<&WaypointIdentity>,
) -> Option<Arc<crate::types::discovery::Service>> {
	let self_id = self_addr.or_else(|| {
		warn!("waypoint requires self address for TCP routing");
		None
	})?;

	let svc = stores
		.read_discovery()
		.services
		.get_by_vip(&NetworkAddress {
			network: network.clone(),
			address: dst.ip(),
		})?;

	let wp = svc.waypoint.as_ref()?;
	let is_ours = match &wp.destination {
		Destination::Address(addr) => {
			let stores_ref = stores.clone();
			self_id.matches_address(addr, |ns, hostname| {
				let discovery = stores_ref.read_discovery();
				let self_svc = discovery.services.get_by_namespaced_host(
					&crate::types::discovery::NamespacedHostname {
						namespace: ns.clone(),
						hostname: hostname.clone(),
					},
				)?;
				Some(self_svc.vips.clone())
			})
		},
		Destination::Hostname(n) => self_id.matches_hostname(n),
	};
	if !is_ours {
		warn!(
			"service {} is meant for waypoint {:?}, but we are {}.{}",
			svc.hostname, wp.destination, self_id.gateway, self_id.namespace
		);
		return None;
	}

	Some(svc)
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
	route_path: Option<RoutePath>,
) -> BackendPolicies {
	inputs.stores.read_binds().backend_policies(
		backend.backend.target(),
		&[&backend.inline_policies, inline_policies],
		route_path,
	)
}

#[cfg(test)]
mod tests {
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};
	use std::sync::Arc;
	use std::sync::RwLock;

	use agent_core::strng;

	use crate::store::{BackendPolicies, Stores};
	use crate::types::agent::{ListenerProtocol, SimpleBackendReference};
	use crate::types::discovery::{
		GatewayAddress, NamespacedHostname, NetworkAddress, Service, WaypointIdentity,
		gatewayaddress::Destination,
	};

	fn stores_with_services(services: Vec<Service>) -> Stores {
		let mut discovery_store = crate::store::DiscoveryStore::new();
		for svc in services {
			discovery_store.insert_service_internal(svc);
		}
		Stores {
			discovery: crate::store::DiscoveryStoreUpdater::new(Arc::new(RwLock::new(discovery_store))),
			binds: crate::store::BindStoreUpdater::new(Arc::new(RwLock::new(
				crate::store::BindStore::with_ipv6_enabled(true),
			))),
		}
	}

	fn make_service(
		name: &str,
		namespace: &str,
		hostname: &str,
		vip: &str,
		network: &str,
		waypoint: Option<GatewayAddress>,
	) -> Service {
		Service {
			name: strng::new(name),
			namespace: strng::new(namespace),
			hostname: strng::new(hostname),
			vips: vec![NetworkAddress {
				network: strng::new(network),
				address: vip.parse().unwrap(),
			}],
			ports: std::collections::HashMap::from([(3306, 3306)]),
			waypoint,
			..Default::default()
		}
	}

	fn make_self_addr(gateway: &str, namespace: &str) -> WaypointIdentity {
		WaypointIdentity {
			gateway: strng::new(gateway),
			namespace: strng::new(namespace),
		}
	}

	fn hbone_listener() -> Arc<crate::types::agent::Listener> {
		Arc::new(crate::types::agent::Listener {
			key: Default::default(),
			name: crate::types::agent::ListenerName {
				gateway_name: strng::EMPTY,
				gateway_namespace: strng::EMPTY,
				listener_name: strng::literal!("test"),
				listener_set: None,
			},
			hostname: Default::default(),
			protocol: ListenerProtocol::HBONE,
		})
	}

	#[tokio::test]
	async fn test_waypoint_default_tcp_route_for_known_service() {
		let svc = make_service(
			"mysql-db",
			"default",
			"mysql-db.default.svc.cluster.local",
			"10.0.0.50",
			"network",
			Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: strng::new("istio-system"),
					hostname: strng::new("my-waypoint.istio-system.svc.cluster.local"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::select_best_route(
			None,
			hbone_listener(),
			&stores,
			&network,
			dst,
			Some(&self_addr),
		);
		assert!(
			route.is_some(),
			"should generate default TCP route for known service"
		);
		let route = route.unwrap();
		assert_eq!(route.key.as_str(), "_waypoint-default-tcp");
		assert_eq!(route.backends.len(), 1);
		match &route.backends[0].backend {
			SimpleBackendReference::Service { name, port } => {
				assert_eq!(name.hostname.as_str(), "mysql-db.default.svc.cluster.local");
				assert_eq!(*port, 3306);
			},
			other => panic!("expected Service backend, got {:?}", other),
		}
	}

	#[tokio::test]
	async fn test_waypoint_default_tcp_route_unknown_vip() {
		let stores = stores_with_services(vec![]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 99)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::select_best_route(
			None,
			hbone_listener(),
			&stores,
			&network,
			dst,
			Some(&self_addr),
		);
		assert!(route.is_none(), "should return None for unknown VIP");
	}

	#[tokio::test]
	async fn test_waypoint_default_tcp_route_wrong_waypoint() {
		// Service is bound to a different waypoint
		let svc = make_service(
			"mysql-db",
			"default",
			"mysql-db.default.svc.cluster.local",
			"10.0.0.50",
			"network",
			Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: strng::new("istio-system"),
					hostname: strng::new("other-waypoint.istio-system.svc.cluster.local"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::select_best_route(
			None,
			hbone_listener(),
			&stores,
			&network,
			dst,
			Some(&self_addr),
		);
		assert!(
			route.is_none(),
			"should reject service bound to different waypoint"
		);
	}

	#[tokio::test]
	async fn test_waypoint_default_tcp_route_no_waypoint_config() {
		// Service has no waypoint configuration
		let svc = make_service(
			"mysql-db",
			"default",
			"mysql-db.default.svc.cluster.local",
			"10.0.0.50",
			"network",
			None, // No waypoint
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::select_best_route(
			None,
			hbone_listener(),
			&stores,
			&network,
			dst,
			Some(&self_addr),
		);
		assert!(
			route.is_none(),
			"should reject service without waypoint config"
		);
	}

	#[tokio::test]
	async fn test_waypoint_default_tcp_route_no_self_addr() {
		let svc = make_service(
			"mysql-db",
			"default",
			"mysql-db.default.svc.cluster.local",
			"10.0.0.50",
			"network",
			Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: strng::new("istio-system"),
					hostname: strng::new("my-waypoint.istio-system.svc.cluster.local"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);

		let route = super::select_best_route(None, hbone_listener(), &stores, &network, dst, None);
		assert!(
			route.is_none(),
			"should return None when self_addr not configured"
		);
	}

	#[tokio::test]
	async fn test_select_best_route_hbone_generates_default() {
		let svc = make_service(
			"redis",
			"default",
			"redis.default.svc.cluster.local",
			"10.0.0.60",
			"network",
			Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: strng::new("default"),
					hostname: strng::new("test-wp.default.svc.cluster.local"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 60)), 6379);
		let self_addr = make_self_addr("test-wp", "default");

		let route = super::select_best_route(
			None,
			hbone_listener(),
			&stores,
			&network,
			dst,
			Some(&self_addr),
		);
		assert!(
			route.is_some(),
			"HBONE listener should generate default TCP route"
		);
		assert_eq!(route.unwrap().key.as_str(), "_waypoint-default-tcp");
	}

	#[tokio::test]
	async fn test_select_best_route_non_hbone_no_default() {
		let stores = stores_with_services(vec![]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 60)), 6379);

		let listener = Arc::new(crate::types::agent::Listener {
			key: Default::default(),
			name: crate::types::agent::ListenerName {
				gateway_name: strng::EMPTY,
				gateway_namespace: strng::EMPTY,
				listener_name: strng::literal!("test"),
				listener_set: None,
			},
			hostname: Default::default(),
			protocol: ListenerProtocol::TLS(None), // Not HBONE
		});

		let route = super::select_best_route(None, listener, &stores, &network, dst, None);
		assert!(
			route.is_none(),
			"non-HBONE listener should not generate default route"
		);
	}

	#[tokio::test]
	async fn test_service_tcp_route_match() {
		let svc = make_service(
			"mysql-db",
			"default",
			"mysql-db.default.svc.cluster.local",
			"10.0.0.50",
			"network",
			Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: strng::new("istio-system"),
					hostname: strng::new("my-waypoint.istio-system.svc.cluster.local"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let svc_key = NamespacedHostname {
			namespace: strng::new("default"),
			hostname: strng::new("mysql-db.default.svc.cluster.local"),
		};
		{
			let mut binds = stores.binds.write();
			binds.insert_service_tcp_route(
				crate::types::agent::TCPRoute {
					key: strng::literal!("mysql-tcp-route"),
					service_key: Some(svc_key.clone()),
					name: Default::default(),
					hostnames: vec![],
					backends: vec![],
				},
				svc_key,
			);
		}
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::select_best_route(
			None,
			hbone_listener(),
			&stores,
			&network,
			dst,
			Some(&self_addr),
		);
		assert!(route.is_some(), "should match service TCP route");
		assert_eq!(route.unwrap().key.as_str(), "mysql-tcp-route");
	}

	#[tokio::test]
	async fn test_service_tcp_route_no_routes_falls_to_default() {
		// Service exists but no service-keyed TCP routes -> default passthrough
		let svc = make_service(
			"mysql-db",
			"default",
			"mysql-db.default.svc.cluster.local",
			"10.0.0.50",
			"network",
			Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: strng::new("istio-system"),
					hostname: strng::new("my-waypoint.istio-system.svc.cluster.local"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::select_best_route(
			None,
			hbone_listener(),
			&stores,
			&network,
			dst,
			Some(&self_addr),
		);
		assert!(route.is_some(), "should fall through to default");
		assert_eq!(route.unwrap().key.as_str(), "_waypoint-default-tcp");
	}

	fn make_proxy_inputs() -> Arc<crate::ProxyInputs> {
		use crate::client::Client;
		use crate::{BackendConfig, client};
		use agent_core::metrics;
		use hickory_resolver::config::{ResolverConfig, ResolverOpts};
		use prometheus_client::registry::Registry;

		let config = crate::config::parse_config("{}".to_string(), None).unwrap();
		let encoder = config.session_encoder.clone();
		let stores = Stores::with_ipv6_enabled(config.ipv6_enabled);
		let client = Client::new(
			&client::Config {
				resolver_cfg: ResolverConfig::default(),
				resolver_opts: ResolverOpts::default(),
			},
			None,
			BackendConfig::default(),
			None,
		);
		Arc::new(crate::ProxyInputs {
			cfg: Arc::new(config),
			stores: stores.clone(),
			metrics: Arc::new(crate::metrics::Metrics::new(
				metrics::sub_registry(&mut Registry::default()),
				Default::default(),
			)),
			upstream: client,
			ca: None,
			mcp_state: crate::mcp::App::new(stores, encoder),
		})
	}

	fn make_aws_simple_backend() -> super::SimpleBackend {
		use crate::aws::{AwsBackendConfig, AwsService};
		super::SimpleBackend::Aws(
			crate::types::agent::ResourceName::new(strng::new("test-aws"), strng::new("ns")),
			AwsBackendConfig {
				service: AwsService::AgentCore(
					crate::agentcore::AgentCoreConfig::new(
						"arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/abc123".to_string(),
						None,
					)
					.unwrap(),
				),
			},
		)
	}

	#[test]
	fn test_build_backend_call_aws_defaults() {
		let inputs = make_proxy_inputs();
		let backend = make_aws_simple_backend();
		let result = super::TCPProxy::build_backend_call(
			&mut None,
			None,
			&inputs,
			&backend,
			BackendPolicies::default(),
		)
		.unwrap();

		assert!(
			matches!(
				&result.target,
				super::Target::Hostname(h, 443)
					if h.as_str() == "bedrock-agentcore.us-east-1.amazonaws.com"
			),
			"target should be Hostname with port 443"
		);
		assert!(result.backend_policies.backend_tls.is_some());
		assert!(
			matches!(
				&result.backend_policies.backend_auth,
				Some(crate::http::auth::BackendAuth::Aws(
					crate::http::auth::AwsAuth::Implicit {}
				))
			),
			"should default to AWS implicit auth"
		);
		assert!(result.http_version_override.is_none());
		assert!(result.transport_override.is_none());
		assert!(result.network_gateway.is_none());
	}

	#[test]
	fn test_build_backend_call_aws_user_policies_override() {
		use crate::http::auth::{AwsAuth, BackendAuth};
		use secrecy::SecretString;

		let inputs = make_proxy_inputs();
		let backend = make_aws_simple_backend();

		let user_policies = BackendPolicies {
			backend_auth: Some(BackendAuth::Aws(AwsAuth::ExplicitConfig {
				access_key_id: SecretString::from("AKID"),
				secret_access_key: SecretString::from("SECRET"),
				region: Some("us-west-2".to_string()),
				session_token: None,
			})),
			..Default::default()
		};

		let result =
			super::TCPProxy::build_backend_call(&mut None, None, &inputs, &backend, user_policies)
				.unwrap();

		assert!(
			matches!(
				&result.backend_policies.backend_auth,
				Some(BackendAuth::Aws(AwsAuth::ExplicitConfig { .. }))
			),
			"user-provided auth should override default implicit auth"
		);
		assert!(
			result.backend_policies.backend_tls.is_some(),
			"TLS should still be present from defaults"
		);
	}
}
