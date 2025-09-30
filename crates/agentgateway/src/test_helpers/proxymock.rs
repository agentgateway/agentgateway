use std::convert::Infallible;
use std::fmt::Debug;
use std::future::Ready;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use agent_core::drain::{DrainTrigger, DrainWatcher};
use agent_core::strng::Strng;
use agent_core::{drain, metrics, strng};
use axum::body::to_bytes;
use bytes::Bytes;
use http::{Method, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use itertools::Itertools;
use prometheus_client::registry::Registry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::DuplexStream;
use tracing::{info, trace};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::http::{Body, Response};
use crate::llm::AIProvider;
use crate::proxy::Gateway;
use crate::proxy::request_builder::RequestBuilder;
use crate::store::Stores;
use crate::transport::stream::{Socket, TCPConnectionInfo};
use crate::types::agent::{
	Backend, BackendReference, Bind, BindName, Listener, ListenerProtocol, ListenerSet, McpBackend,
	McpTarget, McpTargetSpec, PathMatch, Route, RouteBackendReference, RouteMatch, RouteSet,
	SimpleBackendReference, SseTargetSpec, StreamableHTTPTargetSpec, Target, TargetedPolicy,
};
use crate::types::local::LocalNamedAIProvider;
use crate::{ProxyInputs, client, mcp};

pub async fn send_request(
	io: Client<MemoryConnector, Body>,
	method: Method,
	url: &str,
) -> Response {
	RequestBuilder::new(method, url).send(io).await.unwrap()
}

pub async fn send_request_body(
	io: Client<MemoryConnector, Body>,
	method: Method,
	url: &str,
	body: &[u8],
) -> Response {
	RequestBuilder::new(method, url)
		.body(Body::from(body.to_vec()))
		.send(io)
		.await
		.unwrap()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestDump {
	#[serde(with = "http_serde::method")]
	pub method: ::http::Method,

	#[serde(with = "http_serde::uri")]
	pub uri: ::http::Uri,

	#[serde(with = "http_serde::header_map")]
	pub headers: ::http::HeaderMap,

	pub body: Bytes,
}

pub async fn basic_setup() -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let mock = simple_mock().await;
	setup_mock(mock)
}

pub fn setup_mock(mock: MockServer) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind(basic_route(*mock.address())));
	let io = t.serve_http(strng::new("bind"));
	(mock, t, io)
}

pub fn setup_llm_mock(
	mock: MockServer,
	provider: AIProvider,
	tokenize: bool,
	config: &str,
) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let t = setup_proxy_test(config).unwrap();
	let (be, _) = crate::types::local::LocalAIBackend::Provider(LocalNamedAIProvider {
		name: "default".into(),
		provider,
		host_override: Some(Target::Address(*mock.address())),
		path_override: None,
		tokenize,
		backend_tls: None,
		backend_auth: None,
		policies: None,
		routes: Default::default(),
	})
	.translate(strng::format!("{}", mock.address()))
	.unwrap();
	let b = Backend::AI(strng::format!("{}", mock.address()), be);
	t.pi.stores.binds.write().insert_backend(b);
	let t = t.with_bind(simple_bind(basic_route(*mock.address())));
	let io = t.serve_http(strng::new("bind"));
	(mock, t, io)
}

pub fn basic_route(target: SocketAddr) -> Route {
	basic_named_route(target.to_string().into())
}

pub fn basic_named_route(target: Strng) -> Route {
	Route {
		key: "route".into(),
		route_name: "route".into(),
		hostnames: Default::default(),
		matches: vec![RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/".into()),
			method: None,
			query: vec![],
		}],
		filters: Default::default(),
		inline_policies: Default::default(),
		rule_name: None,
		backends: vec![RouteBackendReference {
			weight: 1,
			backend: BackendReference::Backend(target),
			filters: Default::default(),
		}],
		policies: None,
	}
}

pub fn simple_bind(route: Route) -> Bind {
	Bind {
		key: strng::new("bind"),
		// not really used
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: Default::default(),
			name: Default::default(),
			gateway_name: Default::default(),
			hostname: Default::default(),
			protocol: ListenerProtocol::HTTP,
			tcp_routes: Default::default(),
			routes: RouteSet::from_list(vec![route]),
		}]),
	}
}

pub async fn body_mock(body: &[u8]) -> MockServer {
	let body = Arc::new(body.to_vec());
	let mock = wiremock::MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |_: &wiremock::Request| {
			ResponseTemplate::new(200).set_body_raw(body.clone().to_vec(), "application/json")
		})
		.mount(&mock)
		.await;
	mock
}

pub async fn simple_mock() -> MockServer {
	let mock = wiremock::MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(|req: &wiremock::Request| {
			let r = RequestDump {
				method: req.method.clone(),
				uri: req.url.to_string().parse().unwrap(),
				headers: req.headers.clone(),
				body: Bytes::copy_from_slice(&req.body),
			};
			ResponseTemplate::new(200).set_body_json(r)
		})
		.mount(&mock)
		.await;
	mock
}

pub struct TestBind {
	pi: Arc<ProxyInputs>,
	drain_rx: DrainWatcher,
	_drain_tx: DrainTrigger,
}

#[derive(Debug, Clone)]
pub struct MemoryConnector {
	io: Arc<Mutex<Option<DuplexStream>>>,
}

impl tower::Service<Uri> for MemoryConnector {
	type Response = TokioIo<Socket>;
	type Error = Infallible;
	type Future = Ready<Result<Self::Response, Self::Error>>;

	fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, dst: Uri) -> Self::Future {
		trace!("establish connection for {dst}");
		let mut io = self.io.lock().unwrap();
		let io = io.take().expect("MemoryConnector can only be called once");
		let io = Socket::from_memory(
			io,
			TCPConnectionInfo {
				peer_addr: "127.0.0.1:12345".parse().unwrap(),
				local_addr: "127.0.0.1:80".parse().unwrap(),
				start: Instant::now(),
			},
		);
		std::future::ready(Ok(TokioIo::new(io)))
	}
}

impl TestBind {
	pub fn with_bind(self, bind: Bind) -> Self {
		self.pi.stores.binds.write().insert_bind(bind);
		self
	}

	pub fn with_backend(self, b: SocketAddr) -> Self {
		let b = Backend::Opaque(strng::format!("{}", b), Target::Address(b));
		self.pi.stores.binds.write().insert_backend(b);
		self
	}

	pub fn with_mcp_backend(self, b: SocketAddr, stateful: bool, legacy_sse: bool) -> Self {
		let opb = Backend::Opaque(strng::format!("basic-{}", b), Target::Address(b));
		let sb = SimpleBackendReference::Backend(strng::format!("basic-{}", b));
		let b = Backend::MCP(
			strng::format!("{}", b),
			McpBackend {
				targets: vec![Arc::new(McpTarget {
					name: "mcp".into(),
					spec: if !legacy_sse {
						McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
							backend: sb,
							path: "/mcp".to_string(),
						})
					} else {
						McpTargetSpec::Sse(SseTargetSpec {
							backend: sb,
							path: "/sse".to_string(),
						})
					},
				})],
				stateful,
			},
		);
		{
			let mut bw = self.pi.stores.binds.write();
			bw.insert_backend(opb);
			bw.insert_backend(b);
		}
		self
	}

	pub fn with_multiplex_mcp_backend(
		self,
		name: &str,
		servers: Vec<(&str, SocketAddr, bool)>,
		stateful: bool,
	) -> Self {
		let b = Backend::MCP(
			name.into(),
			McpBackend {
				targets: servers
					.iter()
					.map(|(name, addr, legacy_sse)| {
						let sb = SimpleBackendReference::Backend(strng::format!("basic-{}", addr));
						Arc::new(McpTarget {
							name: strng::new(name),
							spec: if !legacy_sse {
								McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
									backend: sb,
									path: "/mcp".to_string(),
								})
							} else {
								McpTargetSpec::Sse(SseTargetSpec {
									backend: sb,
									path: "/sse".to_string(),
								})
							},
						})
					})
					.collect_vec(),
				stateful,
			},
		);
		{
			let mut bw = self.pi.stores.binds.write();
			for (_, b, _) in servers {
				bw.insert_backend(Backend::Opaque(
					strng::format!("basic-{}", b),
					Target::Address(b),
				))
			}
			bw.insert_backend(b);
		}
		self
	}

	pub fn with_policy(self, p: TargetedPolicy) -> TestBind {
		self.pi.stores.binds.write().insert_policy(p);
		self
	}
	pub fn serve_http(&self, bind_name: BindName) -> Client<MemoryConnector, Body> {
		let io = self.serve(bind_name);
		::hyper_util::client::legacy::Client::builder(TokioExecutor::new())
			.timer(TokioTimer::new())
			.build(MemoryConnector {
				io: Arc::new(Mutex::new(Some(io))),
			})
	}
	// The need to split http/http2 is a hyper limit, not our proxy
	pub fn serve_http2(&self, bind_name: BindName) -> Client<MemoryConnector, Body> {
		let io = self.serve(bind_name);
		::hyper_util::client::legacy::Client::builder(TokioExecutor::new())
			.timer(TokioTimer::new())
			.http2_only(true)
			.build(MemoryConnector {
				io: Arc::new(Mutex::new(Some(io))),
			})
	}
	pub fn serve(&self, bind_name: BindName) -> DuplexStream {
		let (client, server) = tokio::io::duplex(8192);
		let server = Socket::from_memory(
			server,
			TCPConnectionInfo {
				peer_addr: "127.0.0.1:12345".parse().unwrap(),
				local_addr: "127.0.0.1:80".parse().unwrap(),
				start: Instant::now(),
			},
		);
		let bind = Gateway::proxy_bind(bind_name, server, self.pi.clone(), self.drain_rx.clone());
		tokio::spawn(async move {
			info!("starting bind...");
			bind.await;
			info!("finished bind...");
		});
		client
	}
	pub async fn serve_real_listener(&self, bind_name: BindName) -> SocketAddr {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();

		let pi = self.pi.clone();
		let drain_rx = self.drain_rx.clone();

		tokio::spawn(async move {
			info!("starting real listener on {}...", addr);
			loop {
				let (tcp_stream, peer_addr) = match listener.accept().await {
					Ok(conn) => conn,
					Err(e) => {
						info!("listener error: {}", e);
						break;
					},
				};
				info!("accepted connection from {}", peer_addr);

				let socket = Socket::from_tcp(tcp_stream).unwrap();

				let bind = Gateway::proxy_bind(bind_name.clone(), socket, pi.clone(), drain_rx.clone());
				tokio::spawn(bind);
			}
			info!("finished real listener...");
		});

		addr
	}
}

pub fn setup_proxy_test(cfg: &str) -> anyhow::Result<TestBind> {
	agent_core::telemetry::testing::setup_test_logging();
	let config = crate::config::parse_config(cfg.to_string(), None)?;
	let stores = Stores::new();
	let client = client::Client::new(&config.dns, None, Default::default());
	let (drain_tx, drain_rx) = drain::new();
	let pi = Arc::new(ProxyInputs {
		cfg: Arc::new(config),
		stores: stores.clone(),
		tracer: None,
		metrics: Arc::new(crate::metrics::Metrics::new(metrics::sub_registry(
			&mut Registry::default(),
		))),
		upstream: client.clone(),
		ca: None,

		mcp_state: mcp::App::new(stores.clone(), &mut Registry::default()),
	});
	Ok(TestBind {
		pi,
		drain_rx,
		_drain_tx: drain_tx,
	})
}

pub async fn read_body_raw(body: axum_core::body::Body) -> Bytes {
	to_bytes(body, 2_097_152).await.unwrap()
}

pub async fn read_body(body: axum_core::body::Body) -> RequestDump {
	let b = read_body_raw(body).await;
	serde_json::from_slice(&b).unwrap()
}

/// Check if `subset` is a subset of `superset`
/// Returns true if all keys/values in `subset` exist in `superset` with matching values
/// `superset` can have additional keys not present in `subset`
pub fn is_json_subset(subset: &Value, superset: &Value) -> bool {
	match (subset, superset) {
		// If both are objects, check that all keys in subset exist in superset with matching values
		(Value::Object(subset_map), Value::Object(superset_map)) => {
			subset_map.iter().all(|(key, subset_value)| {
				superset_map
					.get(key)
					.is_some_and(|superset_value| is_json_subset(subset_value, superset_value))
			})
		},

		// If both are arrays, check that subset array is a prefix or exact match of superset array
		(Value::Array(subset_arr), Value::Array(superset_arr)) => {
			subset_arr.len() <= superset_arr.len()
				&& subset_arr
					.iter()
					.zip(superset_arr.iter())
					.all(|(a, b)| is_json_subset(a, b))
		},

		// For primitive values, they must be exactly equal
		_ => subset == superset,
	}
}

/// check_eventually runs a function many times until it reaches the expected result.
/// If it doesn't the last result is returned
pub async fn check_eventually<F, CF, T, Fut>(dur: Duration, f: F, expected: CF) -> Result<T, T>
where
	F: Fn() -> Fut,
	Fut: Future<Output = T>,
	T: Eq + Debug,
	CF: Fn(&T) -> bool,
{
	use std::ops::Add;
	let mut delay = Duration::from_millis(10);
	let end = SystemTime::now().add(dur);
	let mut last: T;
	let mut attempts = 0;
	loop {
		attempts += 1;
		last = f().await;
		if expected(&last) {
			return Ok(last);
		}
		trace!("attempt {attempts} with delay {delay:?}");
		if SystemTime::now().add(delay) > end {
			return Err(last);
		}
		tokio::time::sleep(delay).await;
		delay *= 2;
	}
}
