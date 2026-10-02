use agentgateway::test_helpers::{ateapimock, credprovidermock, oteltracemock};
use agentgateway::transport::stream::TLSConnectionInfo;
use agentgateway::transport::tls::TlsInfo;
use agentgateway::types::agent::{Backend, BackendWithPolicies, BindMode, TunnelProtocol};
use protos::ateapi::{
	Actor, ActorState, ActorStatus, EgressPolicy, ResourceMetadata, ResumeActorResponse,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;

use crate::common::prelude::*;

const ACTOR_UID: &str = "6f1c2d3e-4a5b-6c7d-8e9f-0a1b2c3d4e5f";
// Port 1 is privileged and outside the ephemeral range: nothing in the test process listens there.
const UNREACHABLE_ADDR: &str = "127.0.0.1:1";

async fn send_request(io: MemoryClient, method: Method, url: &str) -> Response {
	let authority = url
		.strip_prefix("http://")
		.and_then(|url| url.split('/').next())
		.expect("Substrate ingress test URL has an HTTP authority");
	let mut labels = authority.split('.');
	let actor = labels
		.next()
		.expect("Substrate ingress test URL has an actor");
	let atespace = labels
		.next()
		.expect("Substrate ingress test URL has an atespace");
	let target_actor = format!("{atespace}/{actor}");
	send_request_headers(
		io,
		method,
		url,
		&[("ate-target-actor", target_actor.as_str())],
	)
	.await
}

#[derive(Clone)]
struct IngressHandler {
	pod_ip: String,
	calls: Arc<AtomicUsize>,
	resumed: bool,
	uid: &'static str,
}

#[derive(Clone)]
struct EgressHandler {
	uid: &'static str,
	state: ActorState,
	error: Option<tonic::Code>,
}

#[derive(Clone)]
struct CredentialEgressHandler {
	policy: Result<EgressPolicy, tonic::Status>,
}

#[async_trait::async_trait]
impl ateapimock::Handler for CredentialEgressHandler {
	async fn get_actor(
		&mut self,
		request: &protos::ateapi::GetActorRequest,
	) -> Result<Actor, tonic::Status> {
		let actor = request.actor.as_ref().unwrap();
		assert_eq!(
			(actor.atespace.as_str(), actor.name.as_str()),
			("demo", "my-actor")
		);
		Ok(Actor {
			metadata: Some(ResourceMetadata {
				uid: "uid-1".to_owned(),
				..Default::default()
			}),
			status: Some(ActorStatus {
				state: ActorState::Running as i32,
				worker_assignment: None,
			}),
		})
	}

	async fn get_actor_egress_policy(
		&mut self,
		request: &protos::ateapi::GetActorEgressPolicyRequest,
	) -> Result<EgressPolicy, tonic::Status> {
		let actor = request.actor.as_ref().unwrap();
		assert_eq!(
			(actor.atespace.as_str(), actor.name.as_str()),
			("demo", "my-actor")
		);
		self.policy.clone()
	}
}

#[derive(Clone)]
struct CredentialHandler {
	calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl credprovidermock::Handler for CredentialHandler {
	async fn fetch_secret(
		&mut self,
		request: &protos::credprovider::FetchSecretRequest,
	) -> Result<protos::credprovider::FetchSecretResponse, tonic::Status> {
		assert_eq!(
			request.uri,
			"ate-secret://kubernetes.io/default/upstream-token"
		);
		assert_eq!(
			request.actor_spiffe_id,
			"spiffe://substrate-actor.local/actor/demo/my-actor"
		);
		self.calls.fetch_add(1, Ordering::Relaxed);
		Ok(protos::credprovider::FetchSecretResponse {
			opaque_bytes: b"injected-token".to_vec(),
		})
	}
}

#[async_trait::async_trait]
impl ateapimock::Handler for EgressHandler {
	async fn get_actor(
		&mut self,
		request: &protos::ateapi::GetActorRequest,
	) -> Result<Actor, tonic::Status> {
		let actor = request.actor.as_ref().unwrap();
		assert_eq!(
			(actor.atespace.as_str(), actor.name.as_str()),
			("demo", "my-actor")
		);
		if let Some(code) = self.error {
			return Err(tonic::Status::new(code, "GetActor failed"));
		}
		Ok(Actor {
			metadata: Some(ResourceMetadata {
				uid: self.uid.to_owned(),
				..Default::default()
			}),
			status: Some(ActorStatus {
				state: self.state as i32,
				worker_assignment: None,
			}),
		})
	}
}

#[async_trait::async_trait]
impl ateapimock::Handler for IngressHandler {
	async fn resume_actor(
		&mut self,
		request: &protos::ateapi::ResumeActorRequest,
	) -> Result<ResumeActorResponse, tonic::Status> {
		let actor = request.actor.as_ref().unwrap();
		assert_eq!(actor.atespace, "demo");
		assert_eq!(actor.name, "my-actor");
		self.calls.fetch_add(1, Ordering::Relaxed);
		Ok(ResumeActorResponse {
			actor: Some(Actor {
				metadata: Some(ResourceMetadata {
					uid: self.uid.to_owned(),
					..Default::default()
				}),
				status: Some(ActorStatus {
					state: 0,
					worker_assignment: Some(protos::ateapi::WorkerAssignment {
						worker_pod_ip: self.pod_ip.clone(),
					}),
				}),
			}),
			resumed: self.resumed,
		})
	}
}

#[derive(Clone)]
struct ParkingHandler {
	pod_ip: String,
	calls: Arc<AtomicUsize>,
	failures_before_success: usize,
	failure_code: tonic::Code,
	entered: Option<Arc<Notify>>,
	resumed: bool,
}

#[derive(Clone)]
struct SelectiveParkingHandler {
	pod_ip: String,
	parked_actor: String,
	entered: Arc<Notify>,
	release: Arc<Notify>,
	calls: Arc<AtomicUsize>,
	resumed: bool,
	uid: &'static str,
}

#[async_trait::async_trait]
impl ateapimock::Handler for SelectiveParkingHandler {
	async fn resume_actor(
		&mut self,
		request: &protos::ateapi::ResumeActorRequest,
	) -> Result<ResumeActorResponse, tonic::Status> {
		let actor = request.actor.as_ref().unwrap();
		self.calls.fetch_add(1, Ordering::Relaxed);
		if actor.name == self.parked_actor {
			self.entered.notify_one();
			self.release.notified().await;
		}
		Ok(ResumeActorResponse {
			actor: Some(Actor {
				metadata: Some(ResourceMetadata {
					uid: self.uid.to_owned(),
					..Default::default()
				}),
				status: Some(ActorStatus {
					state: 0,
					worker_assignment: Some(protos::ateapi::WorkerAssignment {
						worker_pod_ip: self.pod_ip.clone(),
					}),
				}),
			}),
			resumed: self.resumed,
		})
	}
}

#[async_trait::async_trait]
impl ateapimock::Handler for ParkingHandler {
	async fn resume_actor(
		&mut self,
		_request: &protos::ateapi::ResumeActorRequest,
	) -> Result<ResumeActorResponse, tonic::Status> {
		let call = self.calls.fetch_add(1, Ordering::Relaxed);
		if call == 0 {
			self
				.entered
				.as_ref()
				.inspect(|entered| entered.notify_one());
		}
		if call < self.failures_before_success {
			return Err(tonic::Status::new(
				self.failure_code,
				"no free workers available",
			));
		}
		Ok(ResumeActorResponse {
			actor: Some(Actor {
				status: Some(ActorStatus {
					state: 0,
					worker_assignment: Some(protos::ateapi::WorkerAssignment {
						worker_pod_ip: self.pod_ip.clone(),
					}),
				}),
				..Default::default()
			}),
			resumed: self.resumed,
		})
	}
}

#[tokio::test]
async fn actor_ingress_resolves_the_dynamic_backend() {
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;

	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api.address.to_string(),
				"connectTargetPort": actor.address().port(),
			}
		}))
		.await;

	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		"http://my-actor.demo.actors.resources.substrate.ate.dev/",
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(calls.load(Ordering::Relaxed), 1);
	let actor_requests = actor.received_requests().await.unwrap();
	assert_eq!(
		actor_requests[0].headers.get("x-ate-target-port").unwrap(),
		"80"
	);
}

#[tokio::test]
async fn actor_ingress_parks_while_worker_capacity_recovers() {
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || ParkingHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			failures_before_success: 2,
			failure_code: tonic::Code::ResourceExhausted,
			entered: None,
			resumed: true,
		}
	})
	.spawn()
	.await;

	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api.address.to_string(),
				"connectTargetPort": actor.address().port(),
				"requestParking": {
					"budget": "1s",
					"max": 1,
					"retryInterval": "1ms",
					"retryFactor": 1.0,
				}
			}
		}))
		.await;

	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		"http://my-actor.demo.actors.resources.substrate.ate.dev/",
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn actor_ingress_sheds_when_request_parking_is_full() {
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let entered = Arc::new(Notify::new());
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let entered = entered.clone();
		let pod_ip = actor.address().ip().to_string();
		move || ParkingHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			failures_before_success: 2,
			failure_code: tonic::Code::FailedPrecondition,
			entered: Some(entered.clone()),
			resumed: true,
		}
	})
	.spawn()
	.await;

	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api.address.to_string(),
				"connectTargetPort": actor.address().port(),
				"requestParking": {
					"budget": "1s",
					"max": 1,
					"retryInterval": "100ms",
					"retryFactor": 1.0,
				}
			}
		}))
		.await;

	let first = tokio::spawn(send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		"http://my-actor.demo.actors.resources.substrate.ate.dev/",
	));
	entered.notified().await;
	let second = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		"http://another-actor.demo.actors.resources.substrate.ate.dev/",
	)
	.await;
	assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
	assert_eq!(first.await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn actor_ingress_keeps_cached_actor_available_when_parking_is_full() {
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Notify::new());
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let entered = entered.clone();
		let release = release.clone();
		let pod_ip = actor.address().ip().to_string();
		move || SelectiveParkingHandler {
			pod_ip: pod_ip.clone(),
			parked_actor: "cold-actor".to_string(),
			entered: entered.clone(),
			release: release.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;

	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api.address.to_string(),
				"connectTargetPort": actor.address().port(),
				"requestParking": {
					"budget": "1s",
					"max": 1,
				}
			}
		}))
		.await;

	let running_actor = "http://running-actor.demo.actors.resources.substrate.ate.dev/";
	assert_eq!(
		send_request(gateway.serve_http(BIND_KEY), Method::GET, running_actor)
			.await
			.status(),
		StatusCode::OK
	);

	let cold = tokio::spawn(send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		"http://cold-actor.demo.actors.resources.substrate.ate.dev/",
	));
	entered.notified().await;

	assert_eq!(
		send_request(gateway.serve_http(BIND_KEY), Method::GET, running_actor)
			.await
			.status(),
		StatusCode::OK
	);
	assert_eq!(calls.load(Ordering::Relaxed), 2);

	release.notify_one();
	assert_eq!(cold.await.unwrap().status(), StatusCode::OK);
}

/// Asserts the access log emitted `ate.router.resume` for the request that used `path`. Every
/// caller needs its own path: the capture buffer is process-global.
async fn assert_logged_resume(path: &str, want: &str) {
	agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", path),
		("ate.router.resume", want),
	])
	.await
	.unwrap();
}

fn logged_route_duration(log: &Value) -> f64 {
	let duration = &log["ate.router.route.duration"];
	assert!(
		duration.as_str().is_none(),
		"the route duration must be a number, not a formatted string: {log:#?}"
	);
	duration
		.as_f64()
		.unwrap_or_else(|| panic!("no numeric ate.router.route.duration: {log:#?}"))
}

async fn find_request_log(path: &str) -> Value {
	agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", path)])
		.await
		.unwrap()
}

fn actor_url(actor: &str, path: &str) -> String {
	format!("http://{actor}.demo.actors.resources.substrate.ate.dev{path}")
}

async fn resume_disposition_gateway(
	api_address: std::net::SocketAddr,
	actor_port: u16,
) -> agentgateway::test_helpers::proxymock::TestBind {
	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api_address.to_string(),
				"connectTargetPort": actor_port,
			}
		}))
		.await;
	gateway
}

async fn unreachable_actor_service_ingress_status(request_parking: Value) -> StatusCode {
	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": UNREACHABLE_ADDR,
				"connectTargetPort": 80,
				"requestParking": request_parking,
			}
		}))
		.await;
	send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", "/"),
	)
	.await
	.status()
}

#[tokio::test]
async fn actor_ingress_reports_an_unreachable_actor_service_as_unavailable() {
	assert_eq!(
		unreachable_actor_service_ingress_status(json!({ "max": 0 })).await,
		StatusCode::SERVICE_UNAVAILABLE,
	);
	// While parking, an unavailable service is retried until the budget runs out.
	assert_eq!(
		unreachable_actor_service_ingress_status(json!({
			"budget": "100ms",
			"max": 1,
			"retryInterval": "10ms",
			"retryFactor": 1.0,
		}))
		.await,
		StatusCode::GATEWAY_TIMEOUT,
	);
}

#[tokio::test]
async fn actor_ingress_reports_a_triggered_resume_as_a_cold_start() {
	const PATH: &str = "/resume-triggered";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let mut trace_rx = agentgateway::proxy::dtrace::track_expression(Some(
		agentgateway::cel::Expression::new_strict(format!("request.path == '{PATH}'")).unwrap(),
	));
	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", PATH),
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(calls.load(Ordering::Relaxed), 1);

	assert_logged_resume(PATH, "triggered").await;

	let mut events = Vec::new();
	while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(50), trace_rx.recv()).await {
		events.push(serde_json::to_value(msg).unwrap())
	}
	let resumes: Vec<&serde_json::Value> = events
		.iter()
		.filter(|event| event["message"]["type"] == "policyEvent")
		.filter(|event| event["message"]["kind"] == "substrate")
		.map(|event| &event["message"]["details"]["resume"])
		.collect();
	assert_eq!(resumes, vec!["triggered"], "{events:#?}");
}

#[tokio::test]
async fn actor_ingress_reports_no_resume_when_the_actor_is_already_running() {
	const PATH: &str = "/resume-already-running";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: false,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", PATH),
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(calls.load(Ordering::Relaxed), 1);
	assert_logged_resume(PATH, "none").await;
}

#[tokio::test]
async fn actor_ingress_reports_no_resume_for_a_cache_hit_after_a_cold_start() {
	const COLD_PATH: &str = "/resume-cache-cold";
	const WARM_PATH: &str = "/resume-cache-warm";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	for path in [COLD_PATH, WARM_PATH] {
		let response = send_request(
			gateway.serve_http(BIND_KEY),
			Method::GET,
			&actor_url("my-actor", path),
		)
		.await;
		assert_eq!(response.status(), StatusCode::OK);
	}

	assert_eq!(
		calls.load(Ordering::Relaxed),
		1,
		"the second request must be served from the assignment cache"
	);
	assert_logged_resume(COLD_PATH, "triggered").await;
	assert_logged_resume(WARM_PATH, "none").await;
}

#[tokio::test]
async fn actor_ingress_logs_the_actor_uid_on_a_cold_start() {
	const PATH: &str = "/actor-uid-cold";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", PATH),
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(calls.load(Ordering::Relaxed), 1);

	let log = find_request_log(PATH).await;
	assert_eq!(log["ate.actor.uid"].as_str(), Some(ACTOR_UID), "{log:#?}");
}

#[tokio::test]
async fn actor_ingress_logs_the_actor_uid_from_the_assignment_cache() {
	const COLD_PATH: &str = "/actor-uid-cache-cold";
	const WARM_PATH: &str = "/actor-uid-cache-warm";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	for path in [COLD_PATH, WARM_PATH] {
		let response = send_request(
			gateway.serve_http(BIND_KEY),
			Method::GET,
			&actor_url("my-actor", path),
		)
		.await;
		assert_eq!(response.status(), StatusCode::OK);
	}

	assert_eq!(
		calls.load(Ordering::Relaxed),
		1,
		"the second request must be served from the assignment cache"
	);
	for path in [COLD_PATH, WARM_PATH] {
		let log = find_request_log(path).await;
		assert_eq!(
			log["ate.actor.uid"].as_str(),
			Some(ACTOR_UID),
			"{path}: {log:#?}"
		);
	}
}

#[tokio::test]
async fn actor_ingress_logs_actor_identity_under_the_upstream_spellings() {
	const PATH: &str = "/actor-identity-spelling";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", PATH),
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);

	let log = find_request_log(PATH).await;
	assert_eq!(log["ate.actor.name"].as_str(), Some("my-actor"), "{log:#?}");
	assert_eq!(log["ate.actor.uid"].as_str(), Some(ACTOR_UID), "{log:#?}");
	assert_eq!(log["ate.atespace"].as_str(), Some("demo"), "{log:#?}");
	assert!(
		log.get("ate.actor.id").is_none(),
		"ate.actor.id was renamed to ate.actor.name: {log:#?}"
	);
}

#[tokio::test]
async fn actor_ingress_reports_a_joined_resume_for_a_follower_on_an_in_flight_resume() {
	const LEADER_PATH: &str = "/resume-join-leader";
	const FOLLOWER_PATH: &str = "/resume-join-follower";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Notify::new());
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let entered = entered.clone();
		let release = release.clone();
		let pod_ip = actor.address().ip().to_string();
		move || SelectiveParkingHandler {
			pod_ip: pod_ip.clone(),
			parked_actor: "my-actor".to_string(),
			entered: entered.clone(),
			release: release.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let leader = tokio::spawn({
		let io = gateway.serve_http(BIND_KEY);
		let url = actor_url("my-actor", LEADER_PATH);
		async move { send_request(io, Method::GET, &url).await }
	});
	// The leader is inside ResumeActor, so the singleflight placeholder exists and the follower
	// cannot become a second leader or read a warm cache entry.
	entered.notified().await;
	let follower = tokio::spawn({
		let io = gateway.serve_http(BIND_KEY);
		let url = actor_url("my-actor", FOLLOWER_PATH);
		async move { send_request(io, Method::GET, &url).await }
	});
	tokio::time::sleep(Duration::from_millis(200)).await;
	release.notify_one();

	assert_eq!(leader.await.unwrap().status(), StatusCode::OK);
	assert_eq!(follower.await.unwrap().status(), StatusCode::OK);
	assert_eq!(
		calls.load(Ordering::Relaxed),
		1,
		"the follower must have joined the leader's resume, not started its own"
	);

	assert_logged_resume(LEADER_PATH, "triggered").await;
	assert_logged_resume(FOLLOWER_PATH, "joined").await;
}

#[tokio::test]
async fn actor_ingress_reports_the_activation_time_for_a_triggered_resume() {
	const PATH: &str = "/route-duration-triggered";
	const GATE: Duration = Duration::from_millis(300);
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Notify::new());
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let entered = entered.clone();
		let release = release.clone();
		let pod_ip = actor.address().ip().to_string();
		move || SelectiveParkingHandler {
			pod_ip: pod_ip.clone(),
			parked_actor: "my-actor".to_string(),
			entered: entered.clone(),
			release: release.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let request = tokio::spawn({
		let io = gateway.serve_http(BIND_KEY);
		let url = actor_url("my-actor", PATH);
		async move { send_request(io, Method::GET, &url).await }
	});
	entered.notified().await;
	tokio::time::sleep(GATE).await;
	release.notify_one();
	assert_eq!(request.await.unwrap().status(), StatusCode::OK);

	assert_logged_resume(PATH, "triggered").await;
	let log = find_request_log(PATH).await;
	let duration = logged_route_duration(&log);
	assert!(
		duration >= 0.25,
		"a resume gated for {GATE:?} must report at least that long, got {duration}: {log:#?}"
	);
}

#[tokio::test]
async fn actor_ingress_reports_a_followers_own_wait_rather_than_the_leaders() {
	const LEADER_PATH: &str = "/route-duration-leader";
	const FOLLOWER_PATH: &str = "/route-duration-follower";
	const LEAD: Duration = Duration::from_millis(300);
	const FOLLOWER_WAIT: Duration = Duration::from_millis(200);
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Notify::new());
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let entered = entered.clone();
		let release = release.clone();
		let pod_ip = actor.address().ip().to_string();
		move || SelectiveParkingHandler {
			pod_ip: pod_ip.clone(),
			parked_actor: "my-actor".to_string(),
			entered: entered.clone(),
			release: release.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let leader = tokio::spawn({
		let io = gateway.serve_http(BIND_KEY);
		let url = actor_url("my-actor", LEADER_PATH);
		async move { send_request(io, Method::GET, &url).await }
	});
	// The leader is inside ResumeActor. It stays there for LEAD before the follower is even sent,
	// so the two requests cannot have waited the same amount of time.
	entered.notified().await;
	tokio::time::sleep(LEAD).await;
	let follower = tokio::spawn({
		let io = gateway.serve_http(BIND_KEY);
		let url = actor_url("my-actor", FOLLOWER_PATH);
		async move { send_request(io, Method::GET, &url).await }
	});
	tokio::time::sleep(FOLLOWER_WAIT).await;
	release.notify_one();

	assert_eq!(leader.await.unwrap().status(), StatusCode::OK);
	assert_eq!(follower.await.unwrap().status(), StatusCode::OK);
	assert_eq!(
		calls.load(Ordering::Relaxed),
		1,
		"the follower must have joined the leader's resume, not started its own"
	);
	assert_logged_resume(LEADER_PATH, "triggered").await;
	assert_logged_resume(FOLLOWER_PATH, "joined").await;

	let leader_log = find_request_log(LEADER_PATH).await;
	let follower_log = find_request_log(FOLLOWER_PATH).await;
	let leader_duration = logged_route_duration(&leader_log);
	let follower_duration = logged_route_duration(&follower_log);
	assert!(
		leader_duration >= 0.45,
		"the leader waited {LEAD:?} + {FOLLOWER_WAIT:?}, got {leader_duration}: {leader_log:#?}"
	);
	assert!(
		follower_duration >= 0.15,
		"the follower parked on the guard for {FOLLOWER_WAIT:?}, got {follower_duration}: {follower_log:#?}"
	);
	assert!(
		leader_duration - follower_duration > 0.15,
		"a follower must report its own wait, not the leader's cached number: \
		 leader={leader_duration} follower={follower_duration}"
	);
}

#[tokio::test]
async fn actor_ingress_reports_a_near_zero_duration_for_a_cache_hit() {
	const COLD_PATH: &str = "/route-duration-cache-cold";
	const WARM_PATH: &str = "/route-duration-cache-warm";
	const GATE: Duration = Duration::from_millis(300);
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Notify::new());
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let entered = entered.clone();
		let release = release.clone();
		let pod_ip = actor.address().ip().to_string();
		move || SelectiveParkingHandler {
			pod_ip: pod_ip.clone(),
			parked_actor: "my-actor".to_string(),
			entered: entered.clone(),
			release: release.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let cold = tokio::spawn({
		let io = gateway.serve_http(BIND_KEY);
		let url = actor_url("my-actor", COLD_PATH);
		async move { send_request(io, Method::GET, &url).await }
	});
	entered.notified().await;
	tokio::time::sleep(GATE).await;
	release.notify_one();
	assert_eq!(cold.await.unwrap().status(), StatusCode::OK);

	let warm = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", WARM_PATH),
	)
	.await;
	assert_eq!(warm.status(), StatusCode::OK);
	assert_eq!(
		calls.load(Ordering::Relaxed),
		1,
		"the second request must be served from the assignment cache"
	);

	let cold_log = find_request_log(COLD_PATH).await;
	let warm_log = find_request_log(WARM_PATH).await;
	let cold_duration = logged_route_duration(&cold_log);
	let warm_duration = logged_route_duration(&warm_log);
	assert!(
		warm_duration < 0.05,
		"a cache hit resolves without ateapi, got {warm_duration}: {warm_log:#?}"
	);
	assert!(
		cold_duration - warm_duration > 0.15,
		"a cache hit must not inherit the cold start's duration: \
		 cold={cold_duration} warm={warm_duration}"
	);
}

#[tokio::test]
async fn actor_ingress_emits_the_route_duration_as_a_number_of_seconds() {
	const PATH: &str = "/route-duration-number";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", PATH),
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);

	let log = find_request_log(PATH).await;
	assert!(
		log["ate.router.route.duration"].is_number(),
		"a latency panel queries this arithmetically: {log:#?}"
	);
	assert!(
		log["duration"].as_str().is_some(),
		"the sibling `duration` is the formatted style this key must not copy: {log:#?}"
	);
}

#[tokio::test]
async fn actor_ingress_reports_no_resume_when_the_resume_fails() {
	const PATH: &str = "/resume-failed";
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || ParkingHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			failures_before_success: 1,
			failure_code: tonic::Code::NotFound,
			entered: None,
			resumed: true,
		}
	})
	.spawn()
	.await;
	let gateway = resume_disposition_gateway(api.address, actor.address().port()).await;

	let mut trace_rx = agentgateway::proxy::dtrace::track_expression(Some(
		agentgateway::cel::Expression::new_strict(format!("request.path == '{PATH}'")).unwrap(),
	));
	let response = send_request(
		gateway.serve_http(BIND_KEY),
		Method::GET,
		&actor_url("my-actor", PATH),
	)
	.await;
	assert_eq!(response.status(), StatusCode::NOT_FOUND);
	assert_eq!(calls.load(Ordering::Relaxed), 1);

	assert_logged_resume(PATH, "none").await;

	let mut events = Vec::new();
	while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(50), trace_rx.recv()).await {
		events.push(serde_json::to_value(msg).unwrap())
	}
	let resumes: Vec<&serde_json::Value> = events
		.iter()
		.filter(|event| event["message"]["type"] == "policyEvent")
		.filter(|event| event["message"]["kind"] == "substrate")
		.map(|event| &event["message"]["details"]["resume"])
		.collect();
	assert_eq!(resumes, vec!["none"], "{events:#?}");
}

#[tokio::test]
async fn actor_ingress_uses_the_original_connect_target_actor() {
	let actor = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = actor.address().ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;

	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15012".parse().unwrap();
	outer.tunnel_protocol = TunnelProtocol::Connect;
	let mut inner = simple_bind();
	inner.key = strng::literal!("bind/wildcard");
	inner.mode = BindMode::Internal;
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api.address.to_string(),
				"connectTargetPort": actor.address().port(),
			}
		}))
		.await;

	let mut io = gateway.serve_tunnel(strng::literal!("outer"));
	let connect_target = "application.example:9090";
	io.write_all(
	format!(
		"CONNECT {connect_target} HTTP/1.1\r\nHost: {connect_target}\r\nate-target-actor: demo/my-actor\r\n\r\n"
	)
	.as_bytes(),
	)
	.await
	.unwrap();
	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|window| window == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);

	// The re-entered request's Host is unrelated to the actor. Native ingress
	// must use the original CONNECT routing header retained in SourceContext.
	io.write_all(b"GET / HTTP/1.1\r\nHost: irrelevant.example\r\nConnection: close\r\n\r\n")
		.await
		.unwrap();
	let mut tunneled = Vec::new();
	tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut tunneled))
		.await
		.expect("timed out waiting for tunneled response")
		.unwrap();
	assert!(
		String::from_utf8_lossy(&tunneled).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected tunneled response: {}",
		String::from_utf8_lossy(&tunneled),
	);
	assert_eq!(calls.load(Ordering::Relaxed), 1);
	let actor_requests = actor.received_requests().await.unwrap();
	assert_eq!(
		actor_requests[0].headers.get("x-ate-target-port").unwrap(),
		"9090"
	);
}

#[tokio::test]
async fn actor_ingress_uses_backend_tunnel_for_connect() {
	let actor = simple_mock().await;
	let actor_address = *actor.address();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let atunnel_address = listener.local_addr().unwrap();
	let atunnel = tokio::spawn(async move {
		let (mut downstream, _) = listener.accept().await.unwrap();
		let mut request = Vec::new();
		loop {
			let mut chunk = [0; 1024];
			let n = downstream.read(&mut chunk).await.unwrap();
			assert!(n > 0, "CONNECT request unexpectedly closed");
			request.extend_from_slice(&chunk[..n]);
			if request.windows(4).any(|window| window == b"\r\n\r\n") {
				break;
			}
		}
		let request = String::from_utf8(request).unwrap();
		assert!(
			request.starts_with("CONNECT application.example:9090 HTTP/1.1\r\n"),
			"unexpected tunnel request: {request:?}"
		);
		assert!(
			request.contains("ate-target-actor: demo/my-actor\r\n"),
			"tunnel request is missing the actor header: {request:?}"
		);
		downstream
			.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
			.await
			.unwrap();
		let mut upstream = TcpStream::connect(actor_address).await.unwrap();
		let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
	});

	let calls = Arc::new(AtomicUsize::new(0));
	let api = ateapimock::AteApiMock::new({
		let calls = calls.clone();
		let pod_ip = atunnel_address.ip().to_string();
		move || IngressHandler {
			pod_ip: pod_ip.clone(),
			calls: calls.clone(),
			resumed: true,
			uid: ACTOR_UID,
		}
	})
	.spawn()
	.await;

	let dynamic_backend = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
	let dynamic_name = dynamic_backend.name();
	let dynamic = BackendWithPolicies {
		backend: dynamic_backend,
		inline_policies: vec![BackendTrafficPolicy::Tunnel(backend::Tunnel {
			proxy: Arc::new(SimpleBackendReference::Backend(dynamic_name.clone())),
			mode: backend::TunnelMode::Connect,
			policies: vec![],
		})],
	};
	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.tunnel_protocol = TunnelProtocol::Connect;
	let mut inner = simple_bind();
	inner.key = strng::literal!("bind/wildcard");
	inner.mode = BindMode::Internal;
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic)
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api.address.to_string(),
				"connectTargetPort": atunnel_address.port(),
			}
		}))
		.await;
	let mut io = gateway.serve_tunnel(strng::literal!("outer"));
	let authority = "application.example:9090";
	io.write_all(
		format!(
			"CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nate-target-actor: demo/my-actor\r\n\r\n"
		)
		.as_bytes(),
	)
	.await
	.unwrap();
	let mut response = [0; 128];
	let response_len = io.read(&mut response).await.unwrap();
	assert!(String::from_utf8_lossy(&response[..response_len]).starts_with("HTTP/1.1 200 OK\r\n"));

	io.write_all(b"GET / HTTP/1.1\r\nHost: irrelevant.example\r\nConnection: close\r\n\r\n")
		.await
		.unwrap();
	let mut response = Vec::new();
	tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut response))
		.await
		.expect("timed out waiting for tunneled response")
		.unwrap();
	assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"));
	assert_eq!(calls.load(Ordering::Relaxed), 1);
	let actor_requests = actor.received_requests().await.unwrap();
	assert_eq!(actor_requests.len(), 1);
	assert_eq!(
		actor_requests[0].headers.get("x-ate-target-port").unwrap(),
		"9090"
	);
	drop(io);
	atunnel.abort();
}

fn actor_certificate(uri: &str) -> String {
	let mut params = rcgen::CertificateParams::default();
	params
		.subject_alt_names
		.push(rcgen::SanType::URI(uri.try_into().unwrap()));
	params
		.self_signed(&rcgen::KeyPair::generate().unwrap())
		.unwrap()
		.pem()
}

async fn substrate_egress_connect_status(
	handler: EgressHandler,
	certificate_uri: &str,
	payload: &[u8],
) -> StatusCode {
	let api = ateapimock::AteApiMock::new(move || handler.clone())
		.spawn()
		.await;
	substrate_egress_connect_status_at(api.address, certificate_uri, payload).await
}

async fn substrate_egress_connect_status_at(
	api_address: std::net::SocketAddr,
	certificate_uri: &str,
	payload: &[u8],
) -> StatusCode {
	let upstream = simple_mock().await;

	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15012".parse().unwrap();
	let mut inner = simple_bind();
	inner.address = "0.0.0.0:18080".parse().unwrap();
	inner.mode = BindMode::Internal;
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*upstream.address())
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_route(*upstream.address()))
		.with_connect_mode_on_port(agentgateway::types::frontend::ConnectMode::Tunnel, 15012);
	gateway
		.attach_frontend_policy(json!({
			"substrateEgressActorResolution": {
				"host": api_address.to_string(),
			}
		}))
		.await;

	let mut io = gateway.serve_tunnel_with_tls_info(
		strng::literal!("outer"),
		Some(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				certificate: Some(actor_certificate(certificate_uri).into()),
				..Default::default()
			}),
			..Default::default()
		}),
	);
	io.write_all(b"CONNECT allowed.example:18080 HTTP/1.1\r\nHost: allowed.example:18080\r\n\r\n")
		.await
		.unwrap();
	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|window| window == b"\r\n\r\n") {
			break;
		}
	}
	let response = String::from_utf8(response).unwrap();
	if response.starts_with("HTTP/1.1 200 OK\r\n") {
		io.write_all(payload).await.unwrap();
		StatusCode::OK
	} else if response.starts_with("HTTP/1.1 403 Forbidden\r\n") {
		StatusCode::FORBIDDEN
	} else if response.starts_with("HTTP/1.1 503 Service Unavailable\r\n") {
		StatusCode::SERVICE_UNAVAILABLE
	} else {
		panic!("unexpected CONNECT response: {response}")
	}
}

#[tokio::test]
async fn substrate_egress_actor_resolution_refreshes_on_long_lived_tunnel() {
	#[derive(Clone)]
	struct CountingHandler {
		calls: Arc<AtomicUsize>,
	}

	#[async_trait::async_trait]
	impl ateapimock::Handler for CountingHandler {
		async fn get_actor(
			&mut self,
			_request: &protos::ateapi::GetActorRequest,
		) -> Result<Actor, tonic::Status> {
			self.calls.fetch_add(1, Ordering::Relaxed);
			Ok(Actor {
				metadata: Some(ResourceMetadata {
					uid: "uid-1".to_owned(),
					..Default::default()
				}),
				status: Some(ActorStatus {
					state: ActorState::Running as i32,
					worker_assignment: None,
				}),
			})
		}
	}

	let first_calls = Arc::new(AtomicUsize::new(0));
	let first_api = ateapimock::AteApiMock::new({
		let calls = first_calls.clone();
		move || CountingHandler {
			calls: calls.clone(),
		}
	})
	.spawn()
	.await;
	let second_calls = Arc::new(AtomicUsize::new(0));
	let second_api = ateapimock::AteApiMock::new({
		let calls = second_calls.clone();
		move || CountingHandler {
			calls: calls.clone(),
		}
	})
	.spawn()
	.await;

	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15012".parse().unwrap();
	let mut inner = simple_bind();
	inner.address = "0.0.0.0:18080".parse().unwrap();
	inner.mode = BindMode::Internal;
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_bind(outer)
		.with_bind(inner)
		.with_connect_mode_on_port(agentgateway::types::frontend::ConnectMode::Tunnel, 15012);
	gateway
		.attach_frontend_policy(json!({
			"substrateEgressActorResolution": {
				"host": first_api.address.to_string(),
			}
		}))
		.await;

	let io = gateway.serve_tunnel_with_tls_info(
		strng::literal!("outer"),
		Some(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				certificate: Some(
					actor_certificate("spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor").into(),
				),
				..Default::default()
			}),
			..Default::default()
		}),
	);
	let (mut sender, connection) =
		hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
			.handshake(hyper_util::rt::TokioIo::new(io))
			.await
			.unwrap();
	let connection = tokio::spawn(connection);

	let connect = || {
		::http::Request::builder()
			.method(Method::CONNECT)
			.uri("allowed.example:18080")
			.body(Body::empty())
			.unwrap()
	};
	assert_eq!(
		sender.send_request(connect()).await.unwrap().status(),
		StatusCode::OK
	);
	assert_eq!(first_calls.load(Ordering::Relaxed), 1);

	gateway
		.pi
		.stores
		.binds
		.write()
		.remove_policy(strng::literal!("pol/1"));
	gateway
		.attach_frontend_policy(json!({
			"substrateEgressActorResolution": {
				"host": second_api.address.to_string(),
			}
		}))
		.await;

	assert_eq!(
		sender.send_request(connect()).await.unwrap().status(),
		StatusCode::OK
	);
	assert_eq!(first_calls.load(Ordering::Relaxed), 1);
	assert_eq!(second_calls.load(Ordering::Relaxed), 1);
	connection.abort();
}

async fn open_actor_egress_tunnel(gateway: &TestBind, port: u16) -> tokio::io::DuplexStream {
	let mut io = gateway.serve_tunnel_with_tls_info(
		strng::literal!("outer"),
		Some(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				certificate: Some(
					actor_certificate("spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor").into(),
				),
				..Default::default()
			}),
			..Default::default()
		}),
	);
	io.write_all(
		format!("CONNECT 127.0.0.2:{port} HTTP/1.1\r\nHost: 127.0.0.2:{port}\r\n\r\n").as_bytes(),
	)
	.await
	.unwrap();
	let mut response = Vec::new();
	while !response.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
		let mut buf = [0; 128];
		let n = io.read(&mut buf).await.unwrap();
		assert_ne!(n, 0, "CONNECT closed before responding");
		response.extend_from_slice(&buf[..n]);
	}
	assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"));
	io
}

#[cfg(feature = "crypto-aws-lc")]
#[rstest::rstest]
#[case(false, false)]
#[case(false, true)]
#[case(true, false)]
#[case(true, true)]
#[tokio::test]
async fn substrate_egress_replaces_only_requested_credentials(
	#[case] https: bool,
	#[case] placeholder: bool,
) {
	let (upstream, backend_tls) = if https {
		let (upstream, certs) = tls_mock().await;
		let tls: agentgateway::http::backendtls::BackendTLS =
			agentgateway::http::backendtls::ResolvedBackendTLS {
				root: Some(certs.root_cert.pem().into_bytes()),
				hostname: Some("localhost".to_owned()),
				..Default::default()
			}
			.try_into()
			.unwrap();
		(upstream, vec![BackendTrafficPolicy::BackendTLS(tls)])
	} else {
		(simple_mock().await, vec![])
	};
	let port = upstream.address().port();
	let effects = Some(protos::ateapi::HttpRuleEffects {
		replace_headers: vec![protos::ateapi::CredentialHeader {
			header: "authorization".to_owned(),
			prefix: "Bearer ".to_owned(),
			credential_uri: "ate-secret://kubernetes.io/default/upstream-token".to_owned(),
		}],
	});
	let ports = Some(protos::ateapi::Ports {
		all: None,
		numbers: vec![i32::from(port)],
	});
	let rule = if https {
		protos::ateapi::EgressRule {
			https: Some(protos::ateapi::HttpsRule {
				hostnames: vec!["localhost".to_owned()],
				ports,
				effects,
			}),
			..Default::default()
		}
	} else {
		protos::ateapi::EgressRule {
			http: Some(protos::ateapi::HttpRule {
				hostnames: vec!["localhost".to_owned()],
				ports,
				effects,
			}),
			..Default::default()
		}
	};
	// A less specific rule has an unavailable provider. Only the winner may apply effects.
	let mut fallback = rule.clone();
	let fallback_effects = Some(protos::ateapi::HttpRuleEffects {
		replace_headers: vec![protos::ateapi::CredentialHeader {
			header: "authorization".to_owned(),
			credential_uri: "ate-secret://unconfigured/default/token".to_owned(),
			..Default::default()
		}],
	});
	let any_port = Some(protos::ateapi::Ports {
		all: Some(protos::ateapi::AllPorts {}),
		numbers: vec![],
	});
	if let Some(http) = fallback.http.as_mut() {
		http.ports = any_port.clone();
		http.effects = fallback_effects.clone();
	}
	if let Some(https) = fallback.https.as_mut() {
		https.ports = any_port;
		https.effects = fallback_effects;
	}
	let policy = EgressPolicy {
		rules: vec![fallback, rule],
		..Default::default()
	};
	let api = ateapimock::AteApiMock::new(move || CredentialEgressHandler {
		policy: Ok(policy.clone()),
	})
	.spawn()
	.await;
	let credential_calls = Arc::new(AtomicUsize::new(0));
	let credential_provider = credprovidermock::CredentialProviderMock::new({
		let credential_calls = credential_calls.clone();
		move || CredentialHandler {
			calls: credential_calls.clone(),
		}
	})
	.spawn()
	.await;

	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15013".parse().unwrap();
	let inner = BindSnapshot::new(
		Bind {
			key: BIND_KEY,
			address: std::net::SocketAddr::from(([0, 0, 0, 0], port)),
			protocol: if https {
				BindProtocol::tls
			} else {
				BindProtocol::http
			},
			tunnel_protocol: Default::default(),
			mode: BindMode::Internal,
		},
		ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: Default::default(),
			protocol: if https {
				ListenerProtocol::HTTPS(crate::tests::tls::test_server_tls_config())
			} else {
				ListenerProtocol::HTTP
			},
		}]),
	);
	let mut gateway = crate::tests::dfp::setup_dfp_bind()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Dynamic(
				ResourceName::new("dynamic".into(), "".into()),
				Some(Arc::new(
					agentgateway::cel::Expression::new_strict(
						r#"destination.address + ":" + string(destination.port)"#,
					)
					.unwrap(),
				)),
			),
			inline_policies: backend_tls,
		})
		.with_bind(outer)
		.with_bind(inner)
		.with_connect_mode_on_port(agentgateway::types::frontend::ConnectMode::Tunnel, 15013);
	gateway
		.attach_frontend_policy(json!({
			"substrateEgressActorResolution": { "host": api.address.to_string() }
		}))
		.await;
	gateway
		.attach_route_policy(json!({
			"substrateEgress": {
				"host": api.address.to_string(),
				"credentialProviders": [{
					"uriAuthority": "kubernetes.io",
					"target": { "host": credential_provider.address.to_string() }
				}]
			}
		}))
		.await;

	let io = open_actor_egress_tunnel(&gateway, port).await;

	let path = format!("/substrate-egress-credentials-{https}-{placeholder}");
	let authorization = if placeholder {
		"AuThOrIzAtIoN: Bearer actor-supplied\r\n"
	} else {
		""
	};
	// The request's port must neither select a rule nor change the dynamic dial target.
	// Deliberately use the opposite URI scheme to exercise transport-based matching.
	let scheme = if https { "http" } else { "https" };
	let request = format!(
		"GET {scheme}://localhost:1{path} HTTP/1.1\r\nHost: localhost:1\r\n{authorization}Connection: close\r\n\r\n"
	);
	async fn exchange(mut io: impl AsyncRead + AsyncWrite + Unpin, request: &str) {
		io.write_all(request.as_bytes()).await.unwrap();
		let mut response = Vec::new();
		tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut response))
			.await
			.expect("timed out waiting for tunneled response")
			.unwrap();
		assert!(
			String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
			"{}",
			String::from_utf8_lossy(&response)
		);
	}
	if https {
		let tls: agentgateway::http::backendtls::BackendTLS =
			agentgateway::http::backendtls::ResolvedBackendTLS {
				root: Some(include_bytes!("../../../../examples/mcp-tls/certs/ca-cert.pem").to_vec()),
				insecure_host: true,
				alpn: Some(vec!["http/1.1".to_owned()]),
				..Default::default()
			}
			.try_into()
			.unwrap();
		let io = tokio_rustls::TlsConnector::from(tls.base_config().config)
			.connect(
				rustls_pki_types::ServerName::try_from("localhost").unwrap(),
				io,
			)
			.await
			.unwrap();
		exchange(io, &request).await;
	} else {
		exchange(io, &request).await;
	}

	assert_eq!(
		credential_calls.load(Ordering::Relaxed),
		usize::from(placeholder)
	);
	let upstream_requests = upstream.received_requests().await.unwrap();
	assert_eq!(upstream_requests.len(), 1);
	assert_eq!(
		upstream_requests[0]
			.headers
			.get("authorization")
			.map(|v| v.to_str().unwrap()),
		placeholder.then_some("Bearer injected-token")
	);
	let log = find_request_log(&path).await;
	assert_eq!(log["ate.actor.uid"].as_str(), Some("uid-1"), "{log:#?}");
	assert_eq!(log["ate.actor.name"].as_str(), Some("my-actor"), "{log:#?}");
	assert_eq!(log["ate.atespace"].as_str(), Some("demo"), "{log:#?}");
}

// Provide both TLS paths for the same name. The policy must choose between them,
// even when the HTTPS listener is a more specific static listener match.
async fn substrate_tls_gateway(
	policy: Result<EgressPolicy, tonic::Status>,
	upstream: std::net::SocketAddr,
	root: Option<Vec<u8>>,
) -> (TestBind, agentgateway::test_helpers::MockInstance) {
	let api = ateapimock::AteApiMock::new(move || CredentialEgressHandler {
		policy: policy.clone(),
	})
	.spawn()
	.await;
	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15013".parse().unwrap();
	outer.mode = BindMode::Internal;
	let tls_listener = strng::literal!("passthrough");
	let tcp_listener = strng::literal!("tcp");
	let inner = BindSnapshot::new(
		Bind {
			key: BIND_KEY,
			address: std::net::SocketAddr::from(([0, 0, 0, 0], upstream.port())),
			protocol: BindProtocol::auto,
			tunnel_protocol: Default::default(),
			mode: BindMode::Internal,
		},
		ListenerSet::from_list([
			Listener {
				key: LISTENER_KEY,
				name: Default::default(),
				hostname: "localhost".into(),
				protocol: ListenerProtocol::HTTPS(crate::tests::tls::test_server_tls_config()),
			},
			Listener {
				key: tls_listener.clone(),
				name: Default::default(),
				hostname: Default::default(),
				protocol: ListenerProtocol::TLS(None),
			},
			Listener {
				key: tcp_listener.clone(),
				name: Default::default(),
				hostname: Default::default(),
				protocol: ListenerProtocol::TCP,
			},
		]),
	);
	let backend_tls = root
		.map(|root| {
			let tls = agentgateway::http::backendtls::ResolvedBackendTLS {
				root: Some(root),
				hostname: Some("localhost".to_owned()),
				..Default::default()
			}
			.try_into()
			.unwrap();
			BackendTrafficPolicy::BackendTLS(tls)
		})
		.into_iter()
		.collect();
	let mut tls_route = basic_named_tcp_route("/passthrough".into());
	tls_route.key = "tls-route".into();
	let mut tcp_route = basic_named_tcp_route(strng::format!("/{upstream}"));
	tcp_route.key = "tcp-route".into();
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None),
			inline_policies: backend_tls,
		})
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Dynamic(ResourceName::new("passthrough".into(), "".into()), None),
			inline_policies: vec![],
		})
		.with_backend(upstream)
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_named_route("/dynamic".into()))
		.with_tcp_route_for_listener(tls_listener, tls_route)
		.with_tcp_route_for_listener(tcp_listener, tcp_route)
		.with_connect_mode_on_port(agentgateway::types::frontend::ConnectMode::Tunnel, 15013);
	gateway
		.attach_frontend_policy(json!({
				"substrateEgressActorResolution": { "host": api.address.to_string() },
				"tls": { "handshakeTimeout": "1s" }
		}))
		.await;
	gateway
		.attach_route_policy(json!({ "substrateEgress": { "host": api.address.to_string() } }))
		.await;
	(gateway, api)
}

#[cfg(feature = "crypto-aws-lc")]
#[rstest::rstest]
#[case(true, true, false)]
#[case(false, true, false)]
#[case(false, true, true)]
#[case(true, false, false)]
#[tokio::test]
async fn substrate_egress_selects_tls_from_policy_and_rechecks_http(
	#[case] intercept: bool,
	#[case] allowed_request: bool,
	#[case] use_connect_ip: bool,
) {
	let (upstream, certs) = tls_mock().await;
	let port = upstream.address().port();
	let ports = Some(protos::ateapi::Ports {
		numbers: vec![i32::from(port)],
		..Default::default()
	});
	let https_name = if intercept { "localhost" } else { "*" };
	let tls_name = if intercept { "*" } else { "localhost" };
	let mut hostnames = vec![https_name.to_owned()];
	if allowed_request {
		hostnames.push("*".to_owned());
	}
	let policy = EgressPolicy {
		rules: vec![
			protos::ateapi::EgressRule {
				tls_passthrough: Some(protos::ateapi::TlsPassthroughRule {
					hostnames: vec![tls_name.to_owned()],
					ports: ports.clone(),
				}),
				..Default::default()
			},
			protos::ateapi::EgressRule {
				https: Some(protos::ateapi::HttpsRule {
					hostnames,
					ports,
					effects: None,
				}),
				..Default::default()
			},
		],
		..Default::default()
	};
	let (gateway, _api) = substrate_tls_gateway(
		Ok(policy),
		*upstream.address(),
		Some(certs.root_cert.pem().into_bytes()),
	)
	.await;
	let gateway = if use_connect_ip {
		gateway.with_raw_backend(BackendWithPolicies {
			backend: Backend::Dynamic(
				ResourceName::new("passthrough".into(), "".into()),
				Some(Arc::new(
					agentgateway::cel::Expression::new_strict(
						r#"destination.address + ":" + string(destination.port)"#,
					)
					.unwrap(),
				)),
			),
			inline_policies: vec![],
		})
	} else {
		gateway
	};
	let io = open_actor_egress_tunnel(&gateway, port).await;
	let mut roots = certs.root_cert.pem().into_bytes();
	roots.extend_from_slice(include_bytes!(
		"../../../../examples/mcp-tls/certs/ca-cert.pem"
	));
	let tls: agentgateway::http::backendtls::BackendTLS =
		agentgateway::http::backendtls::ResolvedBackendTLS {
			root: Some(roots),
			insecure_host: true,
			alpn: Some(vec!["http/1.1".to_owned()]),
			..Default::default()
		}
		.try_into()
		.unwrap();
	let mut io = tokio::time::timeout(
		Duration::from_secs(3),
		tokio_rustls::TlsConnector::from(tls.base_config().config).connect(
			rustls_pki_types::ServerName::try_from("localhost").unwrap(),
			io,
		),
	)
	.await
	.expect("TLS handshake timed out")
	.unwrap();
	let gateway_cert = pem::parse(include_bytes!(
		"../../../../examples/mcp-tls/certs/cert.pem"
	))
	.unwrap();
	assert_eq!(
		io.get_ref().1.peer_certificates().unwrap()[0].as_ref() == gateway_cert.contents(),
		intercept
	);
	// An HTTPS SNI match does not authorize a different request authority.
	// Passthrough forwards this request untouched; the gateway cannot inspect it.
	io.write_all(b"GET /tls-selection HTTP/1.1\r\nHost: 127.0.0.1:1\r\nConnection: close\r\n\r\n")
		.await
		.unwrap();
	let mut response = Vec::new();
	tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut response))
		.await
		.unwrap()
		.unwrap();
	let expected = if allowed_request {
		"HTTP/1.1 200 OK"
	} else {
		"HTTP/1.1 403 Forbidden"
	};
	assert!(
		String::from_utf8_lossy(&response).starts_with(expected),
		"{}",
		String::from_utf8_lossy(&response)
	);
	assert_eq!(
		upstream.received_requests().await.unwrap().len(),
		usize::from(allowed_request)
	);
}

#[cfg(feature = "crypto-aws-lc")]
#[rstest::rstest]
#[case(false, "websocket")]
#[case(true, "websocket")]
#[case(false, "custom-protocol")]
#[case(true, "custom-protocol")]
#[tokio::test]
async fn substrate_egress_denies_http_upgrades_without_dialing(
	#[case] https: bool,
	#[case] upgrade: &str,
) {
	let (upstream, root) = if https {
		let (upstream, certs) = tls_mock().await;
		(upstream, Some(certs.root_cert.pem().into_bytes()))
	} else {
		(simple_mock().await, None)
	};
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let port = address.port();
	let upstream_address = *upstream.address();
	let (connected, mut connections) = tokio::sync::mpsc::unbounded_channel();
	let forwarder = tokio::spawn(async move {
		loop {
			let (mut downstream, _) = listener.accept().await.unwrap();
			connected.send(()).unwrap();
			tokio::spawn(async move {
				let mut upstream = TcpStream::connect(upstream_address).await.unwrap();
				let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
			});
		}
	});
	let ports = Some(protos::ateapi::Ports {
		numbers: vec![i32::from(port)],
		..Default::default()
	});
	let rule = if https {
		protos::ateapi::EgressRule {
			https: Some(protos::ateapi::HttpsRule {
				hostnames: vec!["localhost".to_owned()],
				ports,
				effects: None,
			}),
			..Default::default()
		}
	} else {
		protos::ateapi::EgressRule {
			http: Some(protos::ateapi::HttpRule {
				hostnames: vec!["localhost".to_owned()],
				ports,
				effects: None,
			}),
			..Default::default()
		}
	};
	let (gateway, _api) = substrate_tls_gateway(
		Ok(EgressPolicy {
			rules: vec![rule],
			..Default::default()
		}),
		address,
		root,
	)
	.await;
	let gateway = if https {
		gateway
	} else {
		let mut inner = simple_bind();
		inner.address = std::net::SocketAddr::from(([0, 0, 0, 0], port));
		inner.mode = BindMode::Internal;
		inner.protocol = BindProtocol::auto;
		gateway.with_bind(inner)
	};

	async fn exchange(mut io: impl AsyncRead + AsyncWrite + Unpin, request: &str) -> Vec<u8> {
		io.write_all(request.as_bytes()).await.unwrap();
		let mut response = Vec::new();
		while !response.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
			let mut buf = [0; 256];
			let n = io.read(&mut buf).await.unwrap();
			assert_ne!(n, 0, "connection closed before HTTP response headers");
			response.extend_from_slice(&buf[..n]);
		}
		response
	}

	for upgrading in [false, true] {
		let io = open_actor_egress_tunnel(&gateway, port).await;
		let headers = if upgrading {
			format!(
				"Connection: keep-alive, UpGrAdE\r\nUpGrAdE: {upgrade}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
			)
		} else {
			"Connection: close\r\n".to_owned()
		};
		let request = format!("GET /upgrade HTTP/1.1\r\nHost: localhost\r\n{headers}\r\n");
		let response = tokio::time::timeout(Duration::from_secs(3), async {
			if https {
				let tls: agentgateway::http::backendtls::BackendTLS =
					agentgateway::http::backendtls::ResolvedBackendTLS {
						root: Some(include_bytes!("../../../../examples/mcp-tls/certs/ca-cert.pem").to_vec()),
						alpn: Some(vec!["http/1.1".to_owned()]),
						..Default::default()
					}
					.try_into()
					.unwrap();
				let io = tokio_rustls::TlsConnector::from(tls.base_config().config)
					.connect(
						rustls_pki_types::ServerName::try_from("localhost").unwrap(),
						io,
					)
					.await
					.unwrap();
				exchange(io, &request).await
			} else {
				exchange(io, &request).await
			}
		})
		.await
		.expect("timed out waiting for HTTP response");
		let expected = if upgrading {
			"HTTP/1.1 403 Forbidden\r\n"
		} else {
			"HTTP/1.1 200 OK\r\n"
		};
		assert!(
			response.starts_with(expected.as_bytes()),
			"{}",
			String::from_utf8_lossy(&response)
		);
		if upgrading {
			assert!(
				tokio::time::timeout(Duration::from_millis(50), connections.recv())
					.await
					.is_err(),
				"denied upgrade opened an upstream connection"
			);
		} else {
			connections.recv().await.unwrap();
		}
		assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
	}
	forwarder.abort();
}

#[cfg(feature = "crypto-aws-lc")]
#[rstest::rstest]
#[case("unmatched-sni")]
#[case("unmatched-sni-allowed-authority")]
#[case("unmatched-port")]
#[case("empty-policy")]
#[case("http-rule")]
#[case("before-network-authorization")]
#[case("before-network-ext-authz")]
#[case("before-gateway-ext-authz")]
#[case("before-route-ext-authz")]
#[case("before-cors")]
#[case("before-route-match")]
#[tokio::test]
async fn substrate_egress_https_denial_returns_403_without_dialing(
	#[case] kind: &str,
	#[values(false, true)] http2: bool,
) {
	let (upstream, certs) = tls_mock().await;
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let port = address.port();
	let upstream_address = *upstream.address();
	let (connected, mut connections) = tokio::sync::mpsc::unbounded_channel();
	let forwarder = tokio::spawn(async move {
		loop {
			let (mut downstream, _) = listener.accept().await.unwrap();
			connected.send(()).unwrap();
			tokio::spawn(async move {
				let mut upstream = TcpStream::connect(upstream_address).await.unwrap();
				let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
			});
		}
	});

	let authz = simple_mock().await;

	// The same destination must work when allowed, then return an HTTP denial
	// without even opening a TCP connection when its policy disallows it.
	for allowed in [true, false] {
		let hostname = if !allowed && kind.starts_with("unmatched-sni") {
			"allowed.example"
		} else {
			"localhost"
		};
		let rule_port = if !allowed && kind == "unmatched-port" {
			if port == 443 { 444 } else { 443 }
		} else {
			port
		};
		let ports = Some(protos::ateapi::Ports {
			numbers: vec![i32::from(rule_port)],
			..Default::default()
		});
		let rule = if !allowed && kind == "http-rule" {
			protos::ateapi::EgressRule {
				http: Some(protos::ateapi::HttpRule {
					hostnames: vec![hostname.to_owned()],
					ports,
					effects: None,
				}),
				..Default::default()
			}
		} else {
			protos::ateapi::EgressRule {
				https: Some(protos::ateapi::HttpsRule {
					hostnames: vec![hostname.to_owned()],
					ports,
					effects: None,
				}),
				..Default::default()
			}
		};
		let policy = EgressPolicy {
			rules: if !allowed && (kind == "empty-policy" || kind.starts_with("before-")) {
				vec![]
			} else {
				vec![rule]
			},
			..Default::default()
		};
		let (mut gateway, _api) = substrate_tls_gateway(
			Ok(policy),
			address,
			Some(certs.root_cert.pem().into_bytes()),
		)
		.await;
		let authz_policy = json!({
			"host": authz.address().to_string(),
			"protocol": { "http": {} }
		});
		match kind {
			"before-network-authorization" => {
				gateway
					.attach_frontend_policy(json!({
						"networkAuthorization": { "rules": [if allowed { "true" } else { "false" }] }
					}))
					.await;
			},
			"before-network-ext-authz" => {
				gateway
					.attach_frontend_policy(json!({ "networkExtAuthz": authz_policy }))
					.await;
			},
			"before-gateway-ext-authz" => {
				gateway
					.attach_gateway_policy(json!({ "extAuthz": authz_policy }))
					.await;
			},
			"before-route-ext-authz" => {
				gateway
					.attach_route_policy(json!({ "extAuthz": authz_policy }))
					.await;
			},
			"before-cors" => {
				gateway
					.attach_route_policy(json!({
						"cors": { "allowOrigins": ["http://example.com"], "allowMethods": ["GET"] }
					}))
					.await;
			},
			"before-route-match" if !allowed => {
				gateway.pi.stores.binds.write().remove_route("route".into());
			},
			_ => {},
		}
		let io = open_actor_egress_tunnel(&gateway, port).await;
		let tls: agentgateway::http::backendtls::BackendTLS =
			agentgateway::http::backendtls::ResolvedBackendTLS {
				root: Some(include_bytes!("../../../../examples/mcp-tls/certs/ca-cert.pem").to_vec()),
				alpn: Some(vec![if http2 { "h2" } else { "http/1.1" }.to_owned()]),
				..Default::default()
			}
			.try_into()
			.unwrap();
		let io = tokio::time::timeout(
			Duration::from_secs(3),
			tokio_rustls::TlsConnector::from(tls.base_config().config).connect(
				rustls_pki_types::ServerName::try_from("localhost").unwrap(),
				io,
			),
		)
		.await
		.expect("TLS handshake timed out")
		.expect("gateway must complete TLS for HTTPS denial");
		let authority = if !allowed && kind == "unmatched-sni-allowed-authority" {
			hostname
		} else {
			"localhost"
		};
		let preflight = !allowed && kind == "before-cors";
		let request = ::http::Request::builder()
			.method(if preflight {
				Method::OPTIONS
			} else {
				Method::GET
			})
			.uri(format!("https://{authority}/https-denial"))
			.header(header::HOST, authority)
			.header(header::ORIGIN, "http://example.com")
			.header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
			.body(Body::empty())
			.unwrap();
		let (response, connection) = tokio::time::timeout(Duration::from_secs(3), async {
			if http2 {
				let (mut sender, connection) =
					hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
						.handshake(TokioIo::new(io))
						.await
						.unwrap();
				let connection = tokio::spawn(connection);
				(sender.send_request(request).await.unwrap(), connection)
			} else {
				let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
					.await
					.unwrap();
				let connection = tokio::spawn(connection);
				(sender.send_request(request).await.unwrap(), connection)
			}
		})
		.await
		.unwrap();
		assert_eq!(
			response.status(),
			if allowed {
				StatusCode::OK
			} else {
				StatusCode::FORBIDDEN
			}
		);
		if kind == "before-cors" {
			assert_eq!(
				response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
				allowed.then_some(&::http::HeaderValue::from_static("http://example.com"))
			);
		}
		tokio::time::timeout(Duration::from_secs(3), response.into_body().collect())
			.await
			.unwrap()
			.unwrap();
		if allowed {
			connections.recv().await.unwrap();
		} else {
			assert!(
				tokio::time::timeout(Duration::from_millis(50), connections.recv())
					.await
					.is_err(),
				"denied HTTPS opened an upstream connection"
			);
		}
		assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
		assert_eq!(
			authz.received_requests().await.unwrap().len(),
			usize::from(kind.ends_with("ext-authz")),
			"denied HTTPS must not call auth services"
		);
		connection.abort();
	}
	forwarder.abort();
}

#[cfg(feature = "crypto-aws-lc")]
#[rstest::rstest]
#[case("opaque")]
#[case("server-first")]
#[case("missing-sni")]
#[case("missing-https-listener")]
#[case("api-unavailable")]
#[case("static-backend")]
#[tokio::test]
async fn substrate_egress_rejects_unrecognized_or_unauthorized_connections(#[case] kind: &str) {
	let (upstream, _certs) = tls_mock().await;
	let raw_upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = if matches!(kind, "opaque" | "server-first" | "static-backend") {
		raw_upstream.local_addr().unwrap()
	} else {
		*upstream.address()
	};
	let port = address.port();
	let ports = Some(protos::ateapi::Ports {
		numbers: vec![i32::from(port)],
		..Default::default()
	});
	let rule = protos::ateapi::EgressRule {
		tls_passthrough: Some(protos::ateapi::TlsPassthroughRule {
			hostnames: vec!["localhost".to_owned()],
			ports,
		}),
		..Default::default()
	};
	let policy = if kind == "api-unavailable" {
		Err(tonic::Status::unavailable("control API unavailable"))
	} else {
		Ok(EgressPolicy {
			rules: vec![rule],
			..Default::default()
		})
	};
	let (gateway, _api) = substrate_tls_gateway(policy, address, None).await;
	let gateway = if kind == "static-backend" {
		gateway.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new("passthrough".into(), "".into()),
				Target::Address(address),
			),
			inline_policies: vec![],
		})
	} else {
		gateway
	};

	let mut io = open_actor_egress_tunnel(&gateway, port).await;
	if kind == "opaque" || kind == "server-first" {
		if kind == "opaque" {
			io.write_all(b"opaque tcp").await.unwrap();
		}
		let mut buf = [0; 1];
		assert_eq!(
			tokio::time::timeout(Duration::from_secs(3), io.read(&mut buf))
				.await
				.unwrap()
				.unwrap(),
			0
		);
	} else {
		let tls: agentgateway::http::backendtls::BackendTLS =
			agentgateway::http::backendtls::ResolvedBackendTLS {
				insecure: true,
				..Default::default()
			}
			.try_into()
			.unwrap();
		let name = if kind == "missing-sni" {
			"127.0.0.1"
		} else if kind == "missing-https-listener" {
			"denied.example"
		} else {
			"localhost"
		};
		let result = tokio::time::timeout(
			Duration::from_secs(3),
			tokio_rustls::TlsConnector::from(tls.base_config().config).connect(
				rustls_pki_types::ServerName::try_from(name.to_owned()).unwrap(),
				io,
			),
		)
		.await
		.unwrap();
		assert!(result.is_err(), "unauthorized TLS handshake succeeded");
	}
	assert!(upstream.received_requests().await.unwrap().is_empty());
	assert!(
		tokio::time::timeout(Duration::from_millis(50), raw_upstream.accept())
			.await
			.is_err(),
		"denied traffic dialed the raw TCP backend"
	);
}

/// Records the `traceparent` each Substrate gRPC call receives, tagged by method.
#[derive(Clone)]
struct Traceparents<H> {
	inner: H,
	pending: Option<String>,
	seen: Arc<StdMutex<Vec<(&'static str, String)>>>,
}

impl<H> Traceparents<H> {
	fn new(inner: H, seen: Arc<StdMutex<Vec<(&'static str, String)>>>) -> Self {
		Self {
			inner,
			pending: None,
			seen,
		}
	}

	fn stash(&mut self, metadata: &tonic::metadata::MetadataMap) {
		self.pending = metadata
			.get("traceparent")
			.and_then(|v| v.to_str().ok())
			.map(str::to_owned);
	}

	fn record(&mut self, method: &'static str) {
		if let Some(tp) = self.pending.take() {
			self.seen.lock().unwrap().push((method, tp));
		}
	}
}

#[async_trait::async_trait]
impl ateapimock::Handler for Traceparents<CredentialEgressHandler> {
	fn metadata(&mut self, metadata: &tonic::metadata::MetadataMap) {
		self.stash(metadata);
	}

	async fn get_actor(
		&mut self,
		request: &protos::ateapi::GetActorRequest,
	) -> Result<Actor, tonic::Status> {
		self.record("GetActor");
		self.inner.get_actor(request).await
	}

	async fn get_actor_egress_policy(
		&mut self,
		request: &protos::ateapi::GetActorEgressPolicyRequest,
	) -> Result<EgressPolicy, tonic::Status> {
		self.record("GetActorEgressPolicy");
		self.inner.get_actor_egress_policy(request).await
	}
}

#[async_trait::async_trait]
impl credprovidermock::Handler for Traceparents<CredentialHandler> {
	fn metadata(&mut self, metadata: &tonic::metadata::MetadataMap) {
		self.stash(metadata);
	}

	async fn fetch_secret(
		&mut self,
		request: &protos::credprovider::FetchSecretRequest,
	) -> Result<protos::credprovider::FetchSecretResponse, tonic::Status> {
		self.record("FetchSecret");
		self.inner.fetch_secret(request).await
	}
}

struct CollectTraces(Arc<StdMutex<Vec<opentelemetry_proto::tonic::trace::v1::Span>>>);

#[async_trait::async_trait]
impl oteltracemock::Handler for CollectTraces {
	async fn export(
		&mut self,
		request: &opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest,
	) -> Result<
		opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceResponse,
		tonic::Status,
	> {
		self.0.lock().unwrap().extend(
			request
				.resource_spans
				.iter()
				.flat_map(|resource| &resource.scope_spans)
				.flat_map(|scope| &scope.spans)
				.cloned(),
		);
		oteltracemock::ok_response()
	}
}

fn hex_id(id: &[u8]) -> String {
	id.iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test]
async fn substrate_egress_propagates_trace_context_to_policy_and_credential_calls() {
	unsafe {
		// Drop export time to make tests fast
		std::env::set_var("OTEL_BSP_SCHEDULE_DELAY", "20");
	}
	let spans = Arc::new(StdMutex::new(Vec::new()));
	let otel = oteltracemock::OtelTraceMock::new({
		let spans = spans.clone();
		move || CollectTraces(spans.clone())
	})
	.spawn()
	.await;
	let upstream = simple_mock().await;
	let port = upstream.address().port();
	let policy = EgressPolicy {
		rules: vec![protos::ateapi::EgressRule {
			http: Some(protos::ateapi::HttpRule {
				hostnames: vec!["localhost".to_owned()],
				ports: Some(protos::ateapi::Ports {
					numbers: vec![i32::from(port)],
					..Default::default()
				}),
				effects: Some(protos::ateapi::HttpRuleEffects {
					replace_headers: vec![protos::ateapi::CredentialHeader {
						header: "authorization".to_owned(),
						prefix: "Bearer ".to_owned(),
						credential_uri: "ate-secret://kubernetes.io/default/upstream-token".to_owned(),
					}],
				}),
			}),
			..Default::default()
		}],
		..Default::default()
	};
	let seen = Arc::new(StdMutex::new(Vec::new()));
	let api = ateapimock::AteApiMock::new({
		let seen = seen.clone();
		move || {
			Traceparents::new(
				CredentialEgressHandler {
					policy: Ok(policy.clone()),
				},
				seen.clone(),
			)
		}
	})
	.spawn()
	.await;
	let credential_provider = credprovidermock::CredentialProviderMock::new({
		let seen = seen.clone();
		move || {
			Traceparents::new(
				CredentialHandler {
					calls: Default::default(),
				},
				seen.clone(),
			)
		}
	})
	.spawn()
	.await;

	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15014".parse().unwrap();
	let mut inner = simple_bind();
	inner.address = std::net::SocketAddr::from(([0, 0, 0, 0], port));
	inner.mode = BindMode::Internal;
	let mut gateway = crate::tests::dfp::setup_dfp_bind()
		.with_bind(outer)
		.with_bind(inner)
		.with_connect_mode_on_port(agentgateway::types::frontend::ConnectMode::Tunnel, 15014);
	gateway
		.attach_frontend_policy(json!({
			"tracing": { "host": otel.address.to_string() },
			"substrateEgressActorResolution": {
				"host": api.address.to_string(),
			}
		}))
		.await;
	gateway
		.attach_route_policy(json!({
			"substrateEgress": {
				"host": api.address.to_string(),
				"credentialProviders": [{
					"uriAuthority": "kubernetes.io",
					"target": { "host": credential_provider.address.to_string() }
				}]
			}
		}))
		.await;

	let mut io = gateway.serve_tunnel_with_tls_info(
		strng::literal!("outer"),
		Some(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				certificate: Some(
					actor_certificate("spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor").into(),
				),
				..Default::default()
			}),
			..Default::default()
		}),
	);
	io.write_all(
		format!("CONNECT 127.0.0.2:{port} HTTP/1.1\r\nHost: 127.0.0.2:{port}\r\n\r\n").as_bytes(),
	)
	.await
	.unwrap();
	let mut connect_response = [0; 128];
	let response_len = io.read(&mut connect_response).await.unwrap();
	assert!(
		String::from_utf8_lossy(&connect_response[..response_len]).starts_with("HTTP/1.1 200 OK\r\n")
	);

	let client_tp = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
	io.write_all(
		format!(
			"GET / HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer actor-supplied\r\ntraceparent: {client_tp}\r\nConnection: close\r\n\r\n"
		)
		.as_bytes(),
	)
	.await
	.unwrap();
	let mut response = Vec::new();
	tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut response))
		.await
		.expect("timed out waiting for tunneled response")
		.unwrap();
	assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"));

	// CONNECT-time GetActor precedes any request trace, so only the two
	// request-path calls carry one. Each continues the client's sampled trace
	// under a gateway span.
	let seen = seen.lock().unwrap().clone();
	let methods: Vec<_> = seen.iter().map(|(method, _)| *method).collect();
	assert_eq!(methods, ["GetActorEgressPolicy", "FetchSecret"], "{seen:?}");
	for (method, tp) in &seen {
		assert_eq!(tp[..36], client_tp[..36], "{method}: {tp}");
		assert_ne!(tp[36..52], client_tp[36..52], "{method}: {tp}");
		assert!(tp.ends_with("-01"), "{method}: {tp}");
	}

	// The traceparent each server received names that call's exported client
	// span, which is a child of the gateway's request span.
	tokio::time::timeout(Duration::from_secs(2), async {
		while spans.lock().unwrap().len() < 3 {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.expect("timed out waiting for exported spans");
	let spans = spans.lock().unwrap();
	let request = spans
		.iter()
		.find(|span| hex_id(&span.parent_span_id) == client_tp[36..52])
		.expect("request span should be exported");
	for (method, tp) in &seen {
		let path = match *method {
			"GetActorEgressPolicy" => "/ateapi.Control/GetActorEgressPolicy",
			"FetchSecret" => "/credprovider.CredentialProvider/FetchSecret",
			other => panic!("unexpected method {other}"),
		};
		let span = spans
			.iter()
			.find(|span| {
				span.name == "Substrate"
					&& span.attributes.iter().any(|attribute| {
						attribute.key == "http.path"
							&& matches!(
								attribute.value.as_ref().and_then(|value| value.value.as_ref()),
								Some(
									opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
										value
									)
								) if value == path
							)
					})
			})
			.unwrap_or_else(|| panic!("{path} span should be exported"));
		assert_eq!(hex_id(&span.span_id), tp[36..52], "{path}");
		assert_eq!(span.parent_span_id, request.span_id, "{path}");
	}
}

#[tokio::test]
async fn substrate_egress_rejects_invalid_or_unavailable_actors_at_connect_time() {
	let running = ActorState::Running;
	assert_eq!(
		substrate_egress_connect_status(
			EgressHandler {
				uid: "uid-1",
				state: running,
				error: Some(tonic::Code::NotFound)
			},
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor",
			b"",
		)
		.await,
		StatusCode::FORBIDDEN,
	);
	assert_eq!(
		substrate_egress_connect_status(
			EgressHandler {
				uid: "uid-1",
				state: running,
				error: None
			},
			"spiffe://substrate-actor.local/actor/demo/my-actor",
			b"",
		)
		.await,
		StatusCode::FORBIDDEN,
	);
	assert_eq!(
		substrate_egress_connect_status(
			EgressHandler {
				uid: "uid-1",
				state: ActorState::Suspended,
				error: None
			},
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor",
			b"",
		)
		.await,
		StatusCode::FORBIDDEN,
	);
	assert_eq!(
		substrate_egress_connect_status(
			EgressHandler {
				uid: "uid-1",
				state: running,
				error: Some(tonic::Code::Unavailable)
			},
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor",
			b"",
		)
		.await,
		StatusCode::SERVICE_UNAVAILABLE,
	);
}

#[tokio::test]
async fn substrate_egress_reports_an_unreachable_actor_service_as_unavailable() {
	let closed = UNREACHABLE_ADDR.parse().unwrap();
	assert_eq!(
		substrate_egress_connect_status_at(
			closed,
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor",
			b"",
		)
		.await,
		StatusCode::SERVICE_UNAVAILABLE,
	);
}

#[tokio::test]
async fn substrate_egress_accepts_valid_actor_connect_before_protocol_detection() {
	for payload in [
		b"GET / HTTP/1.1\r\nHost: allowed.example\r\n\r\n".as_slice(),
		b"\x16\x03\x03\x00\x01\x00".as_slice(),
		b"opaque tcp".as_slice(),
	] {
		assert_eq!(
			substrate_egress_connect_status(
				EgressHandler {
					uid: "uid-1",
					state: ActorState::Running,
					error: None
				},
				"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor",
				payload,
			)
			.await,
			StatusCode::OK,
		);
	}
}
