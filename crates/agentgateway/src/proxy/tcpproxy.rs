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
	BackendPolicy, BindKey, Listener, ListenerProtocol, SimpleBackend, SimpleBackendReference,
	SimpleBackendWithPolicies, TCPRoute, TCPRouteBackend, TCPRouteBackendReference,
	TransportProtocol,
};
use crate::types::discovery::{NetworkAddress, WaypointIdentity, gatewayaddress::Destination};
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

		let selected_route = select_best_route(
			sni,
			selected_listener.clone(),
			&self.inputs.stores,
			&self.inputs.cfg.network,
			self.target_address,
			self.inputs.cfg.self_addr.as_ref(),
		)
		.ok_or(ProxyError::RouteNotFound)?;
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
	for hnm in agent::HostnameMatch::all_matches_or_none(host) {
		if let Some(r) = listener.tcp_routes.get_hostname(&hnm) {
			return Some(Arc::new(r.clone()));
		}
	}

	// For HBONE waypoints, generate a default passthrough route to the service
	if matches!(listener.protocol, ListenerProtocol::HBONE) {
		return waypoint_default_tcp_route(stores, network, dst, self_addr);
	}
	None
}

/// Generate a default TCP route for waypoint proxies, routing to the service
/// identified by the destination VIP. Mirrors the HTTP default route in route.rs.
/// Includes waypoint self-verification: the service must have a waypoint config
/// that matches this proxy instance.
fn waypoint_default_tcp_route(
	stores: &Stores,
	network: &Strng,
	dst: SocketAddr,
	self_addr: Option<&WaypointIdentity>,
) -> Option<Arc<TCPRoute>> {
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

	// Verify the service is bound to this waypoint
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

	Some(Arc::new(TCPRoute {
		key: strng::literal!("_waypoint-default-tcp"),
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
	}))
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

#[cfg(test)]
mod tests {
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};
	use std::sync::Arc;
	use std::sync::RwLock;

	use agent_core::strng;

	use crate::store::Stores;
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
			tcp_routes: Default::default(),
			routes: Default::default(),
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
					hostname: strng::new("my-waypoint"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::waypoint_default_tcp_route(&stores, &network, dst, Some(&self_addr));
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

		let route = super::waypoint_default_tcp_route(&stores, &network, dst, Some(&self_addr));
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
					hostname: strng::new("other-waypoint"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);
		let self_addr = make_self_addr("my-waypoint", "istio-system");

		let route = super::waypoint_default_tcp_route(&stores, &network, dst, Some(&self_addr));
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

		let route = super::waypoint_default_tcp_route(&stores, &network, dst, Some(&self_addr));
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
					hostname: strng::new("my-waypoint"),
				}),
				hbone_mtls_port: 15008,
			}),
		);
		let stores = stores_with_services(vec![svc]);
		let network = strng::literal!("network");
		let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 3306);

		let route = super::waypoint_default_tcp_route(&stores, &network, dst, None);
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
					hostname: strng::new("test-wp"),
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
			tcp_routes: Default::default(),
			routes: Default::default(),
		});

		let route = super::select_best_route(None, listener, &stores, &network, dst, None);
		assert!(
			route.is_none(),
			"non-HBONE listener should not generate default route"
		);
	}
}
