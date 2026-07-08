use agent_core::strng;
use assert_matches::assert_matches;

use super::{HboneAddress, find_service_by_hostname};
use crate::proxy::httpproxy;
use crate::store::LocalWorkload;
use crate::test_helpers::proxymock::{setup_proxy_test, simple_mock};
use crate::types::agent::Target;
use crate::types::discovery::gatewayaddress::Destination;
use crate::types::discovery::{
	GatewayAddress, Identity, InboundProtocol, NamespacedHostname, NetworkAddress, Service, Workload,
};

#[test]
fn hbone_address_parsing() {
	let uri = "127.0.0.1:8080".parse::<http::Uri>().unwrap();
	let addr = HboneAddress::try_from(&uri).unwrap();
	assert_matches!(addr, HboneAddress::SocketAddr(_));

	// Test parsing hostname:port
	let uri = "example.com:443".parse::<http::Uri>().unwrap();
	let addr = HboneAddress::try_from(&uri).unwrap();
	assert_matches!(addr, HboneAddress::SvcHostname(host, port) => {
		assert_eq!(host.as_ref(), "example.com");
		assert_eq!(port, 443);
	});

	// Test URI with no host
	let uri_no_host = "/path".parse::<http::Uri>().unwrap();
	assert!(HboneAddress::try_from(&uri_no_host).is_err());

	// Test URI with host but no port (should fail for CONNECT)
	let uri_no_port = "http://example.com".parse::<http::Uri>().unwrap();
	assert!(HboneAddress::try_from(&uri_no_port).is_err());
}

#[test]
fn hostname_resolution_logic() {
	// Create a mock service store with a service that has a hostname
	let mut stores = crate::store::DiscoveryStore::new();

	let service = Service {
		name: strng::new("waypoint-service"),
		namespace: strng::new("default"),
		hostname: strng::new("my-app.example.com"),
		vips: vec![NetworkAddress {
			network: strng::new("default"),
			address: "10.0.0.100".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, 8080)]),
		waypoint: Some(GatewayAddress {
			destination: Destination::Hostname(NamespacedHostname {
				namespace: strng::new("istio-system"),
				hostname: strng::new("waypoint.istio-system.svc.cluster.local"),
			}),
			hbone_mtls_port: 15008,
		}),
		..Default::default()
	};

	stores.insert_service_internal(service);

	// Test URI parsing for hostname:port
	let uri = "my-app.example.com:80".parse::<http::Uri>().unwrap();
	let parsed_addr = super::HboneAddress::try_from(&uri).unwrap();

	// Should parse as SvcHostname
	assert_matches!(parsed_addr, super::HboneAddress::SvcHostname(host, port) => {
		assert_eq!(host.as_ref(), "my-app.example.com");
		assert_eq!(port, 80);
	});

	let svc = find_service_by_hostname(&stores, "my-app.example.com").unwrap();
	assert_eq!(svc.hostname.as_str(), "my-app.example.com");
	assert_eq!(svc.namespace.as_str(), "default");
	assert!(!svc.vips.is_empty());

	// Verify we can get the VIP
	let network = strng::new("default");
	let vip = svc.vips.iter().find(|v| v.network == network).unwrap();
	assert_eq!(vip.address.to_string(), "10.0.0.100");

	// Test hostname that doesn't exist as a service
	assert!(find_service_by_hostname(&stores, "nonexistent.example.com").is_none());

	// Test service exists but has no VIPs
	let service_no_vips = Service {
		name: strng::new("service-no-vips"),
		namespace: strng::new("default"),
		hostname: strng::new("no-vips.example.com"),
		vips: vec![],
		..Default::default()
	};
	stores.insert_service_internal(service_no_vips);

	assert!(find_service_by_hostname(&stores, "no-vips.example.com").is_none());
}

#[test]
fn accept_error_classification() {
	use std::io::{Error, ErrorKind};

	use super::{is_accept_error_per_connection, is_accept_error_permanent};

	// Fatal errors: socket is permanently broken
	assert!(is_accept_error_permanent(&Error::from_raw_os_error(
		libc::EBADF
	)));
	assert!(is_accept_error_permanent(&Error::from_raw_os_error(
		libc::ENOTSOCK
	)));

	#[cfg(target_os = "linux")]
	assert!(is_accept_error_permanent(&Error::from_raw_os_error(
		libc::EINVAL
	)));
	#[cfg(not(target_os = "linux"))]
	assert!(!is_accept_error_permanent(&Error::from_raw_os_error(
		libc::EINVAL
	)));

	// Per-connection errors: harmless, no backoff needed
	assert!(is_accept_error_per_connection(&Error::from_raw_os_error(
		libc::ECONNABORTED
	)));
	assert!(is_accept_error_per_connection(&Error::from_raw_os_error(
		libc::ECONNRESET
	)));
	assert!(is_accept_error_per_connection(&Error::from_raw_os_error(
		libc::EPERM
	)));

	// Resource pressure errors: need backoff
	let pressure = Error::from_raw_os_error(libc::EMFILE);
	assert!(!is_accept_error_permanent(&pressure));
	assert!(!is_accept_error_per_connection(&pressure));

	let pressure = Error::from_raw_os_error(libc::ENOMEM);
	assert!(!is_accept_error_permanent(&pressure));
	assert!(!is_accept_error_per_connection(&pressure));

	// Generic errors: not permanent, not per-connection
	assert!(!is_accept_error_permanent(&Error::new(
		ErrorKind::WouldBlock,
		"again"
	)));
	assert!(!is_accept_error_per_connection(&Error::new(
		ErrorKind::WouldBlock,
		"again"
	)));
}

#[tokio::test]
async fn ingress_use_waypoint_sets_waypoint_target() {
	use crate::proxy::httpproxy;
	use crate::types::discovery::NamespacedHostname;

	let mock = simple_mock().await;
	let waypoint_addr: std::net::SocketAddr = "10.0.0.50:15008".parse().unwrap();
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_ingress_use_waypoint_service(*mock.address(), waypoint_addr);

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	assert!(svc.ingress_use_waypoint, "ingress_use_waypoint must be set");
	assert!(svc.waypoint.is_some(), "waypoint must be configured");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	// Waypoint target should be populated
	let wp = backend_call
		.waypoint()
		.expect("waypoint target must be set when ingress_use_waypoint is true");
	assert_eq!(
		wp.address.ip(),
		waypoint_addr.ip(),
		"waypoint address should be the waypoint VIP"
	);
	assert_eq!(
		wp.address.port(),
		waypoint_addr.port(),
		"waypoint port should be the hbone_mtls_port"
	);

	// Target must be the service hostname (used as the HBONE CONNECT authority for the waypoint)
	assert_matches!(backend_call.target, Target::Hostname(host, port) => {
		assert_eq!(host.as_str(), "my-svc.default.svc.cluster.local");
		assert_eq!(port, 80);
	});
}

#[tokio::test]
async fn ingress_use_waypoint_false_no_waypoint() {
	let mock = simple_mock().await;
	// Use the standard waypoint service helper which has ingress_use_waypoint: false
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_waypoint_service(*mock.address());

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	assert!(!svc.ingress_use_waypoint);

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	// Waypoint should NOT be set
	assert!(
		backend_call.waypoint().is_none(),
		"waypoint should not be set when ingress_use_waypoint is false"
	);

	// Target should be a direct workload address, not hostname
	assert_matches!(backend_call.target, Target::Address(_));
}

#[tokio::test]
async fn ingress_use_waypoint_remote_waypoint_uses_network_gateway() {
	let mock = simple_mock().await;
	let waypoint_vip: std::net::IpAddr = "240.240.0.5".parse().unwrap();
	let waypoint_ip: std::net::IpAddr = "10.20.0.12".parse().unwrap();
	let gateway_ip: std::net::IpAddr = "172.18.7.110".parse().unwrap();
	let remote_network = strng::literal!("network-2");
	let t = setup_proxy_test("{}").unwrap();

	let svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		waypoint: Some(GatewayAddress {
			destination: Destination::Hostname(NamespacedHostname {
				namespace: strng::literal!("default"),
				hostname: strng::literal!("waypoint.default.svc.cluster.local"),
			}),
			hbone_mtls_port: 15008,
		}),
		ingress_use_waypoint: true,
		..Default::default()
	};
	let wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-wl-uid"),
			name: strng::literal!("test-wl"),
			namespace: strng::literal!("default"),
			workload_ips: vec![mock.address().ip()],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};
	let wp_svc = Service {
		name: strng::literal!("waypoint"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("waypoint.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: waypoint_vip,
		}],
		ports: std::collections::HashMap::from([(15008, 15008)]),
		subject_alt_names: vec![Identity::Spiffe {
			trust_domain: strng::literal!("td2"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("waypoint-san"),
		}],
		..Default::default()
	};
	let wp_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-waypoint-wl-uid"),
			name: strng::literal!("test-waypoint-wl"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("waypoint"),
			network: remote_network.clone(),
			workload_ips: vec![waypoint_ip],
			network_gateway: Some(GatewayAddress {
				destination: Destination::Address(NetworkAddress {
					network: remote_network.clone(),
					address: gateway_ip,
				}),
				hbone_mtls_port: 15008,
			}),
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/waypoint.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(15008, 15008)]),
		)]),
	};
	let gw_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-gateway-wl-uid"),
			name: strng::literal!("test-gateway-wl"),
			namespace: strng::literal!("istio-gateways"),
			service_account: strng::literal!("istio-eastwest"),
			network: remote_network.clone(),
			workload_ips: vec![gateway_ip],
			..Default::default()
		},
		services: Default::default(),
	};

	t.pi
		.stores
		.discovery
		.sync_local(
			vec![svc, wp_svc],
			vec![wl, wp_wl, gw_wl],
			Default::default(),
		)
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	assert!(
		backend_call.waypoint().is_none(),
		"remote waypoint should be reached through double HBONE, not direct waypoint transport"
	);
	let (resolved_gw, gw_identities) = backend_call
		.network_gateway()
		.expect("remote waypoint should resolve a network gateway");
	assert_matches!(&resolved_gw.destination, Destination::Address(addr) => {
		assert_eq!(addr.address, gateway_ip);
		assert_eq!(addr.network, remote_network);
	});
	assert_eq!(resolved_gw.hbone_mtls_port, 15008);
	// Outer tunnel: gateway workload id (the gateway is referenced by address, so no SANs).
	assert_eq!(
		gw_identities,
		&vec![Identity::Spiffe {
			trust_domain: strng::EMPTY,
			namespace: strng::literal!("istio-gateways"),
			service_account: strng::literal!("istio-eastwest"),
		}]
	);
	// Inner tunnel: waypoint workload id + waypoint service SANs.
	assert_matches!(backend_call.transport_override, Some((InboundProtocol::HBONE, identities)) => {
		assert_eq!(identities, vec![
			Identity::Spiffe {
				trust_domain: strng::EMPTY,
				namespace: strng::literal!("default"),
				service_account: strng::literal!("waypoint"),
			},
			Identity::Spiffe {
				trust_domain: strng::literal!("td2"),
				namespace: strng::literal!("default"),
				service_account: strng::literal!("waypoint-san"),
			},
		]);
	});
	assert_matches!(backend_call.target, Target::Hostname(host, port) => {
		assert_eq!(host.as_str(), "my-svc.default.svc.cluster.local");
		assert_eq!(port, 80);
	});
}

#[tokio::test]
async fn ingress_use_waypoint_ip_based_waypoint() {
	let mock = simple_mock().await;
	let waypoint_ip: std::net::IpAddr = "10.0.0.99".parse().unwrap();
	let t = setup_proxy_test("{}").unwrap();

	// Create a service with an IP-based waypoint (not hostname)
	let svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		waypoint: Some(GatewayAddress {
			destination: Destination::Address(NetworkAddress {
				network: strng::EMPTY,
				address: waypoint_ip,
			}),
			hbone_mtls_port: 15008,
		}),
		ingress_use_waypoint: true,
		..Default::default()
	};
	let wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-wl-uid"),
			name: strng::literal!("test-wl"),
			namespace: strng::literal!("default"),
			workload_ips: vec![mock.address().ip()],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};
	// Waypoint workload at the IP-based waypoint address, so its SPIFFE identity
	// can be resolved for mTLS verification.
	let wp_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-waypoint-wl-uid"),
			name: strng::literal!("test-waypoint-wl"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("waypoint"),
			workload_ips: vec![waypoint_ip],
			..Default::default()
		},
		services: Default::default(),
	};
	t.pi
		.stores
		.discovery
		.sync_local(vec![svc], vec![wl, wp_wl], Default::default())
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	let wp = backend_call
		.waypoint()
		.expect("waypoint target must be set for IP-based waypoint");
	assert_eq!(wp.address.ip(), waypoint_ip);
	assert_eq!(wp.address.port(), 15008);
}

#[tokio::test]
async fn ingress_use_waypoint_no_waypoint_field_no_routing() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}").unwrap();

	// Service with ingress_use_waypoint=true but NO waypoint configured
	let svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		waypoint: None, // No waypoint
		ingress_use_waypoint: true,
		..Default::default()
	};
	let wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-wl-uid"),
			name: strng::literal!("test-wl"),
			namespace: strng::literal!("default"),
			workload_ips: vec![mock.address().ip()],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};
	t.pi
		.stores
		.discovery
		.sync_local(vec![svc], vec![wl], Default::default())
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	// No waypoint configured, so it should fall back to direct routing
	assert!(
		backend_call.waypoint().is_none(),
		"waypoint should not be set when no waypoint is configured on the service"
	);
	assert_matches!(backend_call.target, Target::Address(_));
}

#[tokio::test]
async fn ingress_use_waypoint_build_transport_falls_back_without_ca() {
	let mock = simple_mock().await;
	let waypoint_addr: std::net::SocketAddr = "10.0.0.50:15008".parse().unwrap();
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_ingress_use_waypoint_service(*mock.address(), waypoint_addr);

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	assert!(backend_call.waypoint().is_some());

	// build_transport with no CA should fall back to plain transport
	let transport = httpproxy::build_transport(&t.pi, &backend_call, None, None, None, None)
		.await
		.expect("build_transport should succeed");
	// Without CA, it falls back to Plain
	assert_eq!(transport.name(), "plaintext");
}

#[tokio::test]
async fn network_gateway_hostname_resolves_via_service_endpoint() {
	let mock = simple_mock().await;
	let gw_ip: std::net::IpAddr = "192.168.1.10".parse().unwrap();
	let remote_network = strng::literal!("network-3");
	let gateway_namespace = strng::literal!("gateway-ns");
	let gateway_hostname = strng::literal!("gateway.example.internal");
	let svc_port: u16 = 15008;
	let gw_target_port: u16 = 31234;

	let t = setup_proxy_test("{}").unwrap();

	let app_svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		..Default::default()
	};
	let remote_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("remote-wl-uid"),
			name: strng::literal!("remote-wl"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("remote-sa"),
			network: remote_network.clone(),
			workload_ips: vec!["10.244.0.5".parse().unwrap()],
			network_gateway: Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: gateway_namespace.clone(),
					hostname: gateway_hostname.clone(),
				}),
				hbone_mtls_port: svc_port,
			}),
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};

	// gateway service with port mapping
	let gw_svc = Service {
		name: strng::literal!("gateway-svc"),
		namespace: gateway_namespace.clone(),
		hostname: gateway_hostname.clone(),
		vips: vec![],
		ports: std::collections::HashMap::from([(svc_port, gw_target_port)]),
		subject_alt_names: vec![Identity::Spiffe {
			trust_domain: strng::literal!("td-gw"),
			namespace: gateway_namespace.clone(),
			service_account: strng::literal!("gateway-san"),
		}],
		..Default::default()
	};
	let gw_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("gw-wl-uid"),
			name: strng::literal!("gw-1"),
			namespace: gateway_namespace.clone(),
			service_account: strng::literal!("gateway-sa"),
			network: remote_network.clone(),
			workload_ips: vec![gw_ip],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			format!("{}/{}", gateway_namespace, gateway_hostname),
			std::collections::HashMap::from([(svc_port, gw_target_port)]),
		)]),
	};

	t.pi
		.stores
		.discovery
		.sync_local(
			vec![app_svc, gw_svc],
			vec![remote_wl, gw_wl],
			Default::default(),
		)
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("app service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	let (resolved_gw, gw_identities) = backend_call
		.network_gateway()
		.expect("network_gateway must be resolved for hostname-form destination");

	assert_matches!(&resolved_gw.destination, Destination::Address(addr) => {
		assert_eq!(addr.address, gw_ip, "should resolve to the gateway endpoint IP");
		assert_eq!(addr.network, remote_network, "network should be the gateway workload's network");
	});
	assert_eq!(
		resolved_gw.hbone_mtls_port, gw_target_port,
		"port should be the endpoint target port, not the service port"
	);
	// Outer-tunnel identities match ztunnel: gateway workload id + gateway service SANs.
	assert_eq!(
		gw_identities,
		&vec![
			Identity::Spiffe {
				trust_domain: strng::EMPTY,
				namespace: gateway_namespace.clone(),
				service_account: strng::literal!("gateway-sa"),
			},
			Identity::Spiffe {
				trust_domain: strng::literal!("td-gw"),
				namespace: gateway_namespace.clone(),
				service_account: strng::literal!("gateway-san"),
			},
		]
	);
}
