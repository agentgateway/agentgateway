//! Locality-aware load balancing end-to-end tests.
//!
//! Tests exercise the Service → endpoint selection path through the proxy against wiremock
//! servers, one per endpoint. Each test is a sequence of declarative steps (Sync / SetSelf / Hit)
//! so we can cover both static bucketing and lifecycle scenarios — services arriving after
//! workloads, LB config changes, late self-identity resolution, etc.

use std::collections::HashMap;
use std::net::SocketAddr;

use agent_core::strng;
use http::{Method, StatusCode};
use wiremock::MockServer;

use crate::store::{DiscoveryPreviousState, LocalWorkload};
use crate::test_helpers::proxymock::*;
use crate::types::agent::{
	BackendReference, PathMatch, Route, RouteBackendReference, RouteMatch, RouteName,
};
use crate::types::discovery::{
	HealthStatus, LoadBalancer, LoadBalancerHealthPolicy, LoadBalancerMode, LoadBalancerScopes,
	Locality, NamespacedHostname, Service, Workload,
};

use LoadBalancerMode::{Failover, Standard, Strict};
use LoadBalancerScopes::{Node, Region, Zone};

// ---------- tests ----------

/// PreferSameZone (Region + Zone): all traffic lands on the same-zone endpoint even when
/// other zones are healthy.
#[tokio::test]
async fn prefer_same_zone_pins_to_local_zone() {
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Region, Zone],
				}],
				endpoints: vec![
					Ep {
						label: "local",
						svc: "app",
						loc: ("r1", "z1", "node-a"),
						healthy: true,
					},
					Ep {
						label: "other-zone",
						svc: "app",
						loc: ("r1", "z2", "node-b"),
						healthy: true,
					},
					Ep {
						label: "other-region",
						svc: "app",
						loc: ("r2", "z1", "node-c"),
						healthy: true,
					},
				],
			},
			Step::Hit {
				hits: 10,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![("local", 10), ("other-zone", 0), ("other-region", 0)]),
			},
		],
	})
	.await;
}

/// Failover: when the local-zone endpoint is absent, traffic spills to the next-best bucket
/// (same region, different zone) before the worst tier.
#[tokio::test]
async fn failover_spills_to_next_tier_when_local_missing() {
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Region, Zone],
				}],
				endpoints: vec![
					Ep {
						label: "same-region",
						svc: "app",
						loc: ("r1", "z2", "node-b"),
						healthy: true,
					},
					Ep {
						label: "other-region",
						svc: "app",
						loc: ("r2", "z1", "node-c"),
						healthy: true,
					},
				],
			},
			Step::Hit {
				hits: 10,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![("same-region", 10), ("other-region", 0)]),
			},
		],
	})
	.await;
}

/// PreferSameNode beats zone-only matches.
#[tokio::test]
async fn prefer_same_node_pins_to_local_node() {
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Node],
				}],
				endpoints: vec![
					Ep {
						label: "same-node",
						svc: "app",
						loc: ("r1", "z1", "node-a"),
						healthy: true,
					},
					Ep {
						label: "same-zone",
						svc: "app",
						loc: ("r1", "z1", "node-b"),
						healthy: true,
					},
				],
			},
			Step::Hit {
				hits: 6,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![("same-node", 6), ("same-zone", 0)]),
			},
		],
	})
	.await;
}

/// Strict mode drops endpoints that don't fully match; with no survivors the gateway returns
/// 503 (NoHealthyEndpoints) rather than spilling to a worse locality.
#[tokio::test]
async fn strict_mode_drops_non_matching_endpoints() {
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Strict,
					scopes: vec![Region, Zone],
				}],
				endpoints: vec![
					Ep {
						label: "other-zone",
						svc: "app",
						loc: ("r1", "z2", "node-b"),
						healthy: true,
					},
					Ep {
						label: "other-region",
						svc: "app",
						loc: ("r2", "z1", "node-c"),
						healthy: true,
					},
				],
			},
			Step::Hit {
				hits: 3,
				want_status: StatusCode::SERVICE_UNAVAILABLE,
				want: Expect::Exact(vec![("other-zone", 0), ("other-region", 0)]),
			},
		],
	})
	.await;
}

/// Strict mode keeps a fully-matching endpoint and ignores the rest.
#[tokio::test]
async fn strict_mode_keeps_full_matches() {
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Strict,
					scopes: vec![Region, Zone],
				}],
				endpoints: vec![
					Ep {
						label: "match",
						svc: "app",
						loc: ("r1", "z1", "node-a"),
						healthy: true,
					},
					Ep {
						label: "drop",
						svc: "app",
						loc: ("r1", "z2", "node-b"),
						healthy: true,
					},
				],
			},
			Step::Hit {
				hits: 5,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![("match", 5), ("drop", 0)]),
			},
		],
	})
	.await;
}

/// Standard mode: preferences are configured but mode disables bucketing — every endpoint is
/// eligible regardless of locality.
#[tokio::test]
async fn standard_mode_ignores_locality() {
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Standard,
					scopes: vec![Zone],
				}],
				endpoints: vec![
					Ep {
						label: "z1",
						svc: "app",
						loc: ("r1", "z1", "node-a"),
						healthy: true,
					},
					Ep {
						label: "z2",
						svc: "app",
						loc: ("r1", "z2", "node-b"),
						healthy: true,
					},
					Ep {
						label: "z3",
						svc: "app",
						loc: ("r1", "z3", "node-c"),
						healthy: true,
					},
				],
			},
			Step::Hit {
				hits: 30,
				want_status: StatusCode::OK,
				want: Expect::Spread(vec!["z1", "z2", "z3"]),
			},
		],
	})
	.await;
}

/// No self-identity configured: ranker collapses every endpoint into bucket 0, so all endpoints
/// stay reachable and requests spread across them.
#[tokio::test]
async fn missing_self_identity_keeps_all_endpoints_reachable() {
	for mode in [Failover, Standard, Strict] {
		run(Case {
			self_loc: None,
			steps: vec![
				Step::Sync {
					services: vec![Svc {
						name: "app",
						mode,
						scopes: vec![Zone],
					}],
					endpoints: vec![
						Ep {
							label: "a",
							svc: "app",
							loc: ("r1", "z1", "node-a"),
							healthy: true,
						},
						Ep {
							label: "b",
							svc: "app",
							loc: ("r1", "z2", "node-b"),
							healthy: true,
						},
					],
				},
				Step::Hit {
					hits: 20,
					want_status: StatusCode::OK,
					want: Expect::Spread(vec!["a", "b"]),
				},
			],
		})
		.await;
	}
}

/// Default health_policy = OnlyHealthy, so an unhealthy endpoint is excluded from the service
/// entirely; its bucket is empty and traffic spills to the next tier.
#[tokio::test]
async fn unhealthy_local_zone_falls_back_to_next_tier() {
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Region, Zone],
				}],
				endpoints: vec![
					Ep {
						label: "local-bad",
						svc: "app",
						loc: ("r1", "z1", "node-a"),
						healthy: false,
					},
					Ep {
						label: "same-region",
						svc: "app",
						loc: ("r1", "z2", "node-b"),
						healthy: true,
					},
				],
			},
			Step::Hit {
				hits: 6,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![("local-bad", 0), ("same-region", 6)]),
			},
		],
	})
	.await;
}

// ---------- lifecycle ----------

/// Workloads arrive before their service. Before the service exists, requests 503 and endpoints
/// sit in `staged_services`. When the service lands, staged endpoints are bucketed and served.
#[tokio::test]
async fn service_arrives_after_workloads() {
	let endpoints = vec![
		Ep {
			label: "local",
			svc: "app",
			loc: ("r1", "z1", "node-a"),
			healthy: true,
		},
		Ep {
			label: "other-zone",
			svc: "app",
			loc: ("r1", "z2", "node-b"),
			healthy: true,
		},
	];
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			// Workloads only — service does not exist yet.
			Step::Sync {
				services: vec![],
				endpoints: endpoints.clone(),
			},
			// Backend ref can't resolve to a service yet — 500, not 503 (which would mean
			// "service exists but has no healthy endpoints").
			Step::Hit {
				hits: 3,
				want_status: StatusCode::INTERNAL_SERVER_ERROR,
				want: Expect::Exact(vec![("local", 0), ("other-zone", 0)]),
			},
			// Service arrives; previously-staged endpoints should be bucketed against its LB.
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Region, Zone],
				}],
				endpoints,
			},
			Step::Hit {
				hits: 10,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![("local", 10), ("other-zone", 0)]),
			},
		],
	})
	.await;
}

/// Changing a service's LB preferences rebuilds its EndpointSet with the new ranker.
/// Flip preferences between Hits to verify the same endpoint pool gets re-bucketed in place.
#[tokio::test]
async fn service_lb_preference_change_rebuckets() {
	let endpoints = vec![
		Ep {
			label: "same-node",
			svc: "app",
			loc: ("r1", "z1", "node-a"),
			healthy: true,
		},
		Ep {
			label: "same-zone",
			svc: "app",
			loc: ("r1", "z1", "node-b"),
			healthy: true,
		},
		Ep {
			label: "same-region",
			svc: "app",
			loc: ("r1", "z2", "node-c"),
			healthy: true,
		},
	];
	run(Case {
		self_loc: Some(("r1", "z1", "node-a")),
		steps: vec![
			// Start with Zone-only: same-node and same-zone share bucket 0.
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Zone],
				}],
				endpoints: endpoints.clone(),
			},
			Step::Hit {
				hits: 20,
				want_status: StatusCode::OK,
				want: Expect::Spread(vec!["same-node", "same-zone"]),
			},
			// Tighten to Node: only same-node is in bucket 0.
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Node],
				}],
				endpoints,
			},
			Step::Hit {
				hits: 10,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![
					("same-node", 10),
					("same-zone", 0),
					("same-region", 0),
				]),
			},
		],
	})
	.await;
}

/// Late self-identity: gateway starts without self-identity (all endpoints collapse to bucket 0),
/// then WDS delivers its workload and rebucket_all re-ranks existing endpoints by locality.
#[tokio::test]
async fn late_self_identity_rebuckets() {
	let endpoints = vec![
		Ep {
			label: "local",
			svc: "app",
			loc: ("r1", "z1", "node-a"),
			healthy: true,
		},
		Ep {
			label: "other-zone",
			svc: "app",
			loc: ("r1", "z2", "node-b"),
			healthy: true,
		},
	];
	run(Case {
		self_loc: None,
		steps: vec![
			Step::Sync {
				services: vec![Svc {
					name: "app",
					mode: Failover,
					scopes: vec![Region, Zone],
				}],
				endpoints,
			},
			// Without self-identity, ranker puts everything in bucket 0 — requests spread.
			Step::Hit {
				hits: 20,
				want_status: StatusCode::OK,
				want: Expect::Spread(vec!["local", "other-zone"]),
			},
			// Self-identity resolves; rebucket_all pins traffic to the local zone.
			Step::SetSelf(("r1", "z1", "node-a")),
			Step::Hit {
				hits: 10,
				want_status: StatusCode::OK,
				want: Expect::Exact(vec![("local", 10), ("other-zone", 0)]),
			},
		],
	})
	.await;
}

//  --- helpers and harness ---

const SVC_NAMESPACE: &str = "default";
/// Hostname the test route targets. A service named "app" fills this backend.
const ROUTE_TARGET: &str = "app";
const SVC_PORT: u16 = 80;

fn svc_hostname(name: &str) -> String {
	format!("{name}.{SVC_NAMESPACE}.svc.cluster.local")
}

fn svc_key(name: &str) -> String {
	format!("{SVC_NAMESPACE}/{}", svc_hostname(name))
}

fn locality(region: &str, zone: &str) -> Locality {
	Locality {
		region: region.into(),
		zone: zone.into(),
		subzone: strng::EMPTY,
	}
}

fn self_workload(loc: Locality, node: &str) -> Workload {
	Workload {
		uid: "self-uid".into(),
		name: "self".into(),
		namespace: SVC_NAMESPACE.into(),
		locality: loc,
		node: node.into(),
		..Default::default()
	}
}

/// Route that dispatches every request to whatever service is named `ROUTE_TARGET`.
fn service_route() -> Route {
	Route {
		key: "r".into(),
		service_key: None,
		name: RouteName {
			name: "r".into(),
			namespace: SVC_NAMESPACE.into(),
			rule_name: None,
			kind: None,
		},
		hostnames: Default::default(),
		matches: vec![RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/".into()),
			method: None,
			query: vec![],
		}],
		inline_policies: Default::default(),
		backends: vec![RouteBackendReference {
			weight: 1,
			backend: BackendReference::Service {
				name: NamespacedHostname {
					namespace: SVC_NAMESPACE.into(),
					hostname: svc_hostname(ROUTE_TARGET).into(),
				},
				port: SVC_PORT,
			},
			inline_policies: Default::default(),
		}],
	}
}

/// (region, zone, node)
type Loc = (&'static str, &'static str, &'static str);

struct Svc {
	name: &'static str,
	mode: LoadBalancerMode,
	scopes: Vec<LoadBalancerScopes>,
}

#[derive(Clone)]
struct Ep {
	label: &'static str,
	svc: &'static str,
	loc: Loc,
	healthy: bool,
}

enum Expect {
	/// Per-label request-count deltas since the previous Hit. Labels omitted are not checked.
	Exact(Vec<(&'static str, usize)>),
	/// Every listed label must receive ≥1 request this step; counts must sum to `hits`.
	Spread(Vec<&'static str>),
}

enum Step {
	/// Replace the full service+workload set. Empty `services` means no service exists yet.
	Sync {
		services: Vec<Svc>,
		endpoints: Vec<Ep>,
	},
	/// Set self-identity and trigger rebucket_all. Can only be called once per test
	/// (SelfWorkload is a OnceLock).
	SetSelf(Loc),
	/// Fire traffic; assert response status; assert per-label deltas since the previous Hit.
	Hit {
		hits: usize,
		want_status: StatusCode,
		want: Expect,
	},
}

struct Case {
	/// Initial self-identity, set before the first Sync so initial bucketing is correct.
	/// Use None to test late resolution with Step::SetSelf.
	self_loc: Option<Loc>,
	steps: Vec<Step>,
}

struct Harness {
	_bind: TestBind,
	client: hyper_util::client::legacy::Client<MemoryConnector, crate::http::Body>,
	mocks: HashMap<&'static str, MockServer>,
	/// Cumulative received counts observed at the previous Hit, per label.
	baseline: HashMap<&'static str, usize>,
	prev: DiscoveryPreviousState,
}

impl Harness {
	async fn mock_addr(&mut self, label: &'static str) -> SocketAddr {
		if !self.mocks.contains_key(label) {
			self.mocks.insert(label, simple_mock().await);
		}
		*self.mocks[label].address()
	}

	async fn mock_count(&self, label: &str) -> usize {
		self.mocks[label].received_requests().await.unwrap().len()
	}
}

fn build_service(s: &Svc) -> Service {
	Service {
		name: s.name.into(),
		namespace: SVC_NAMESPACE.into(),
		hostname: svc_hostname(s.name).into(),
		ports: HashMap::from([(SVC_PORT, SVC_PORT)]),
		load_balancer: Some(LoadBalancer {
			routing_preferences: s.scopes.clone(),
			mode: s.mode.clone(),
			health_policy: LoadBalancerHealthPolicy::default(),
		}),
		..Default::default()
	}
}

fn build_workload(
	label: &str,
	addr: SocketAddr,
	loc: Locality,
	node: &str,
	svc: &str,
	status: HealthStatus,
) -> LocalWorkload {
	LocalWorkload {
		workload: Workload {
			uid: format!("wl-{label}").into(),
			name: format!("pod-{label}").into(),
			namespace: SVC_NAMESPACE.into(),
			workload_ips: vec![addr.ip()],
			locality: loc,
			node: node.into(),
			status,
			..Default::default()
		},
		services: HashMap::from([(svc_key(svc), HashMap::from([(SVC_PORT, addr.port())]))]),
	}
}

async fn run(c: Case) {
	let t = setup_proxy_test("{}").unwrap();
	if let Some((r, z, n)) = c.self_loc {
		t.inputs()
			.stores
			.discovery
			.read()
			.self_workload
			.set(self_workload(locality(r, z), n));
	}
	let t = t.with_bind(simple_bind(service_route()));
	let client = t.serve_http(BIND_KEY);
	let mut h = Harness {
		_bind: t,
		client,
		mocks: HashMap::new(),
		baseline: HashMap::new(),
		prev: Default::default(),
	};

	for step in c.steps {
		match step {
			Step::Sync {
				services,
				endpoints,
			} => apply_sync(&mut h, services, endpoints).await,
			Step::SetSelf(loc) => apply_set_self(&mut h, loc),
			Step::Hit {
				hits,
				want_status,
				want,
			} => apply_hit(&mut h, hits, want_status, want).await,
		}
	}
}

async fn apply_sync(h: &mut Harness, services: Vec<Svc>, endpoints: Vec<Ep>) {
	let mut workloads = Vec::with_capacity(endpoints.len());
	for ep in &endpoints {
		let addr = h.mock_addr(ep.label).await;
		let (r, z, n) = ep.loc;
		let status = if ep.healthy {
			HealthStatus::Healthy
		} else {
			HealthStatus::Unhealthy
		};
		workloads.push(build_workload(
			ep.label,
			addr,
			locality(r, z),
			n,
			ep.svc,
			status,
		));
	}
	let svcs: Vec<Service> = services.iter().map(build_service).collect();
	let prev = std::mem::take(&mut h.prev);
	h.prev = h
		._bind
		.inputs()
		.stores
		.discovery
		.sync_local(svcs, workloads, prev)
		.unwrap();
}

fn apply_set_self(h: &mut Harness, loc: Loc) {
	let pi = h._bind.inputs();
	let store = pi.stores.discovery.read();
	let (r, z, n) = loc;
	store.self_workload.set(self_workload(locality(r, z), n));
	store.rebucket_all();
}

async fn apply_hit(h: &mut Harness, hits: usize, want_status: StatusCode, want: Expect) {
	for _ in 0..hits {
		let res = send_request(h.client.clone(), Method::GET, "http://app/").await;
		assert_eq!(res.status(), want_status);
	}
	let labels: Vec<&'static str> = h.mocks.keys().copied().collect();
	let mut snapshot = HashMap::with_capacity(labels.len());
	for label in labels {
		snapshot.insert(label, h.mock_count(label).await);
	}
	let delta = |label: &'static str| -> usize {
		snapshot.get(label).copied().unwrap_or(0) - h.baseline.get(label).copied().unwrap_or(0)
	};

	match want {
		Expect::Exact(wants) => {
			for (label, want) in &wants {
				let got = delta(label);
				assert_eq!(got, *want, "label={label}");
			}
		},
		Expect::Spread(labels) => {
			let mut total = 0;
			for label in &labels {
				let got = delta(label);
				assert!(got > 0, "label={label} got 0, want >0");
				total += got;
			}
			assert_eq!(total, hits, "total hits");
		},
	}
	h.baseline = snapshot;
}
