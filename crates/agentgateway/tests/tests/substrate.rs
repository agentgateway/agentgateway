use agentgateway::test_helpers::ateapimock;
use agentgateway::transport::stream::TLSConnectionInfo;
use agentgateway::transport::tls::TlsInfo;
use agentgateway::types::agent::{Backend, BackendWithPolicies, BindMode, TunnelProtocol};
use protos::ateapi::{Actor, ActorState, ActorStatus, ResourceMetadata, ResumeActorResponse};
use tokio::sync::Notify;

use crate::common::prelude::*;

const ACTOR_UID: &str = "6f1c2d3e-4a5b-6c7d-8e9f-0a1b2c3d4e5f";

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
async fn actor_ingress_uses_the_original_connect_authority() {
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
	let connect_target = "my-actor.demo.actors.resources.substrate.ate.dev:9090";
	io.write_all(
		format!("CONNECT {connect_target} HTTP/1.1\r\nHost: {connect_target}\r\n\r\n").as_bytes(),
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
	// must use the original CONNECT authority retained in SourceContext.
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
			request
				.starts_with("CONNECT my-actor.demo.actors.resources.substrate.ate.dev:9090 HTTP/1.1\r\n"),
			"unexpected tunnel request: {request:?}"
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
	let authority = "my-actor.demo.actors.resources.substrate.ate.dev:9090";
	io.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
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
	assert_eq!(actor.received_requests().await.unwrap().len(), 1);
	drop(io);
	atunnel.abort();
}

fn actor_certificate(uid: &str) -> String {
	let mut params = rcgen::CertificateParams::default();
	params
		.custom_extensions
		.push(rcgen::CustomExtension::from_oid_content(
			&[1, 3, 6, 1, 4, 1, 11129, 2, 12, 2],
			serde_json::to_vec(&json!({
				"Atespace": "demo",
				"ActorName": "my-actor",
				"ActorUid": uid,
				"Purpose": "atunnel",
			}))
			.unwrap(),
		));
	params
		.self_signed(&rcgen::KeyPair::generate().unwrap())
		.unwrap()
		.pem()
}

async fn substrate_egress_connect_status(
	handler: EgressHandler,
	certificate_uid: &str,
	payload: &[u8],
) -> StatusCode {
	let upstream = simple_mock().await;
	let api = ateapimock::AteApiMock::new(move || handler.clone())
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
		.with_backend(*upstream.address())
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_route(*upstream.address()))
		.with_connect_mode_on_port(agentgateway::types::frontend::ConnectMode::Tunnel, 15012);
	gateway
		.attach_frontend_policy(json!({
			"substrateEgress": {
				"host": api.address.to_string(),
			}
		}))
		.await;

	let mut io = gateway.serve_tunnel_with_tls_info(
		strng::literal!("outer"),
		Some(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				certificate: Some(actor_certificate(certificate_uid).into()),
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
async fn substrate_egress_rejects_invalid_or_unavailable_actors_at_connect_time() {
	let running = ActorState::Running;
	assert_eq!(
		substrate_egress_connect_status(
			EgressHandler {
				uid: "uid-1",
				state: running,
				error: Some(tonic::Code::NotFound)
			},
			"uid-1",
			b"",
		)
		.await,
		StatusCode::FORBIDDEN,
	);
	assert_eq!(
		substrate_egress_connect_status(
			EgressHandler {
				uid: "uid-2",
				state: running,
				error: None
			},
			"uid-1",
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
			"uid-1",
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
			"uid-1",
			b"",
		)
		.await,
		StatusCode::SERVICE_UNAVAILABLE,
	);
}

#[tokio::test]
async fn substrate_egress_authorizes_http_tls_and_opaque_tcp_connect_tunnels() {
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
				"uid-1",
				payload,
			)
			.await,
			StatusCode::OK,
		);
	}
}
