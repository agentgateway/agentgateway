use std::convert::Infallible;
use std::future::Ready;
use std::ops::Add;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use ::http::{Method, Uri, Version};
use agent_core::drain::{DrainTrigger, DrainWatcher};
use agent_core::{drain, metrics, strng};
use axum::body::to_bytes;
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use prometheus_client::registry::Registry;
use rand::Rng;
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::http::{Body, Response};
use crate::llm::{AIBackend, AIProvider, openai};
use crate::proxy::Gateway;
use crate::proxy::request_builder::RequestBuilder;
use crate::store::Stores;
use crate::transport::stream::{Socket, TCPConnectionInfo};
use crate::types::agent::{
	Backend, BackendReference, Bind, BindName, Listener, ListenerProtocol, ListenerSet, PathMatch,
	Policy, PolicyTarget, Route, RouteBackendReference, RouteMatch, RouteSet, Target, TargetedPolicy,
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
	let t = setup("{}")
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
	let (_mock, bind, io) = basic_setup().await;
	let _bind = bind.with_policy(TargetedPolicy {
		name: strng::new("rl"),
		target: PolicyTarget::Route("route".into()),
		policy: Policy::LocalRateLimit(vec![
			http::localratelimit::RateLimitSpec {
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

#[tokio::test]
async fn llm_openai() {
	let mock = body_mock(include_bytes!("../llm/tests/response_basic.json")).await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		false,
		"{}",
	);

	let want = json!({
		"llm.provider": "openai",
		"llm.request.model": "replaceme",
		"llm.response.model": "gpt-3.5-turbo-0125",
		"llm.request.tokens": 17,
		"llm.response.tokens": 23
	});
	assert_llm(io, include_bytes!("../llm/tests/request_basic.json"), want).await;
}

#[tokio::test]
async fn llm_openai_tokenize() {
	let mock = body_mock(include_bytes!("../llm/tests/response_basic.json")).await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		true,
		"{}",
	);

	let want = json!({
		"llm.provider": "openai",
		"llm.request.model": "replaceme",
		"llm.response.model": "gpt-3.5-turbo-0125",
		"llm.request.tokens": 17,
		"llm.response.tokens": 23
	});
	assert_llm(io, include_bytes!("../llm/tests/request_basic.json"), want).await;
}

#[tokio::test]
async fn llm_log_body() {
	let mock = body_mock(include_bytes!("../llm/tests/response_basic.json")).await;
	let x = serde_json::to_string(&json!({
		"config": {
			"logging": {
				"fields": {
					"add": {
						"prompt": "llm.prompt",
						"completion": "llm.completion"
					}
				}
			}
		}
	}))
	.unwrap();
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		true,
		x.as_str(),
	);

	let want = json!({
		"llm.provider": "openai",
		"llm.request.model": "replaceme",
		"llm.response.model": "gpt-3.5-turbo-0125",
		"llm.request.tokens": 17,
		"llm.response.tokens": 23,
		"completion": ["Sorry, I couldn't find the name of the LLM provider. Could you please provide more information or context?"],
		"prompt": [
			{"role":"system","content":"You are a helpful assistant."},
			{"role":"user","content":"What is the name of the LLM provider?"},
		]
	});
	assert_llm(io, include_bytes!("../llm/tests/request_basic.json"), want).await;
}

async fn assert_llm(io: Client<MemoryConnector, Body>, body: &[u8], want: Value) {
	let r = rand::rng().random::<u128>();
	let res = send_request_body(io.clone(), Method::POST, &format!("http://lo/{r}"), body).await;

	// Ensure body finishes
	let _ = res.into_body().collect().await.unwrap();
	let logs = check_eventually(
		Duration::from_secs(1),
		|| async {
			agent_core::telemetry::testing::find(&[("scope", "request"), ("http.path", &format!("/{r}"))])
				.to_vec()
		},
		|log| log.len() == 1,
	)
	.await
	.unwrap();
	let log = logs.first().unwrap();
	let valid = is_json_subset(&want, log);
	assert!(valid, "want={want:#?} got={log:#?}");
}

async fn send_request(io: Client<MemoryConnector, Body>, method: Method, url: &str) -> Response {
	RequestBuilder::new(method, url).send(io).await.unwrap()
}

async fn send_request_body(
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
	setup_mock(mock)
}

fn setup_mock(mock: MockServer) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let t = setup("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind(basic_route(*mock.address())));
	let io = t.serve_http(strng::new("bind"));
	(mock, t, io)
}

fn setup_llm_mock(
	mock: MockServer,
	provider: AIProvider,
	tokenize: bool,
	config: &str,
) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let t = setup(config).unwrap();
	let b = Backend::AI(
		strng::format!("{}", mock.address()),
		AIBackend {
			provider,
			host_override: Some(Target::Address(*mock.address())),
			tokenize,
		},
	);
	t.pi.stores.binds.write().insert_backend(b);
	let t = t.with_bind(simple_bind(basic_route(*mock.address())));
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
		inline_policies: Default::default(),
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

async fn body_mock(body: &[u8]) -> MockServer {
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
	_drain_tx: DrainTrigger,
}

#[derive(Debug, Clone)]
struct MemoryConnector {
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
}

fn setup(cfg: &str) -> anyhow::Result<TestBind> {
	agent_core::telemetry::testing::setup_test_logging();
	let config = crate::config::parse_config(cfg.to_string(), None)?;
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
		_drain_tx: drain_tx,
	})
}

async fn read_body_raw(body: axum_core::body::Body) -> Bytes {
	to_bytes(body, 2_097_152).await.unwrap()
}

async fn read_body(body: axum_core::body::Body) -> RequestDump {
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
