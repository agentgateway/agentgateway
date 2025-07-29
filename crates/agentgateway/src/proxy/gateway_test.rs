use std::convert::Infallible;
use std::future::Ready;
use std::sync::Arc;
use std::time::Instant;

use ::http::{Method, Request, Uri, Version};
use agent_core::drain::{DrainTrigger, DrainWatcher};
use agent_core::{drain, metrics, strng};
use axum::body::to_bytes;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::Connected;
use hyper_util::rt::tokio::WithHyperIo;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use prometheus_client::registry::Registry;
use tokio::io::DuplexStream;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::http::{Body, Response};
use crate::proxy::Gateway;
use crate::proxy::request_builder::RequestBuilder;
use crate::store::Stores;
use crate::transport::stream::{Socket, TCPConnectionInfo};
use crate::types::agent::{
	Backend, BackendReference, Bind, BindName, Listener, ListenerProtocol, ListenerSet, PathMatch,
	Policy, PolicyTarget, Route, RouteBackend, RouteBackendReference, RouteMatch, RouteSet, Target,
	TargetedPolicy,
};
use crate::{ProxyInputs, client, mcp, *};

#[tokio::test]
async fn basic_handling() {
	let (_mock, _bind, io) = basic_setup().await;
	let res = send_request(io, Method::POST, "http://lo").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::POST);
}

#[tokio::test]
async fn multiple_requests() {
	let (_mock, _bind, io) = basic_setup().await;
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn basic_http2() {
	let mock = simple_mock().await;
	let t = setup()
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind(basic_route(*mock.address())));
	let io = t.serve_http2(strng::new("bind"));
	let res = RequestBuilder::new(Method::GET, "http://lo")
		.version(Version::HTTP_2)
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn local_ratelimit() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind = bind.with_policy(TargetedPolicy {
		name: strng::new("rl"),
		target: PolicyTarget::Route("route".into()),
		policy: Policy::LocalRateLimit(vec![
			http::localratelimit::RateLimitSerde {
				max_tokens: 1,
				tokens_per_fill: 1,
				fill_interval: Duration::from_secs(1),
				limit_type: Default::default(),
			}
			.try_into()
			.unwrap(),
		]),
	});

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 429);
}

async fn send_request(io: Client<MemoryConnector, Body>, method: Method, url: &str) -> Response {
	RequestBuilder::new(method, url).send(io).await.unwrap()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RequestDump {
	#[serde(with = "http_serde::method")]
	method: ::http::Method,

	#[serde(with = "http_serde::uri")]
	uri: ::http::Uri,

	#[serde(with = "http_serde::header_map")]
	headers: ::http::HeaderMap,

	body: Bytes,
}

async fn basic_setup() -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let mock = simple_mock().await;
	let t = setup()
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind(basic_route(*mock.address())));
	let io = t.serve_http(strng::new("bind"));
	(mock, t, io)
}

fn basic_route(target: SocketAddr) -> Route {
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
		rule_name: None,
		backends: vec![RouteBackendReference {
			weight: 1,
			backend: BackendReference::Backend(target.to_string().into()),
			filters: Default::default(),
		}],
		policies: None,
	}
}

fn simple_bind(route: Route) -> Bind {
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

const VERSION: &str = "version";

async fn simple_mock() -> MockServer {
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

struct TestBind {
	pi: Arc<ProxyInputs>,
	drain_rx: DrainWatcher,
	drain_tx: DrainTrigger,
}

#[derive(Debug, Clone)]
struct MemoryConnector {
	io: Arc<Mutex<Option<DuplexStream>>>,
}

impl tower::Service<Uri> for MemoryConnector {
	type Response = TokioIo<Socket>;
	type Error = Infallible;
	type Future = Ready<Result<Self::Response, Self::Error>>;

	fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
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
		let (mut client, mut server) = tokio::io::duplex(8192);
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
}

fn setup() -> anyhow::Result<TestBind> {
	agent_core::telemetry::testing::setup_test_logging();
	let config = crate::config::parse_config("{}".to_string(), None)?;
	let stores = Stores::new();
	let client = client::Client::new(&config.dns, None);
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

		mcp_state: mcp::sse::App::new(
			stores.clone(),
			Arc::new(crate::mcp::relay::metrics::Metrics::new(
				&mut Registry::default(),
				None, // TODO custom tags
			)),
			drain_rx.clone(),
		),
	});
	Ok(TestBind {
		pi,
		drain_rx,
		drain_tx,
	})
}

async fn read_body_raw(body: axum_core::body::Body) -> Bytes {
	to_bytes(body, 2_097_152).await.unwrap()
}

async fn read_body(body: axum_core::body::Body) -> RequestDump {
	let b = read_body_raw(body).await;
	serde_json::from_slice(&b).unwrap()
}
