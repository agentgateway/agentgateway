use agentgateway::test_helpers::ateapimock;
use agentgateway::transport::stream::TLSConnectionInfo;
use agentgateway::transport::tls::TlsInfo;
use agentgateway::types::agent::{Backend, BackendWithPolicies, BindMode, TunnelProtocol};
use protos::ateapi::{Actor, ActorStatus, ResumeActorResponse};
use tokio::sync::Notify;

use crate::common::prelude::*;

#[derive(Clone)]
struct IngressHandler {
	pod_ip: String,
	calls: Arc<AtomicUsize>,
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
				status: Some(ActorStatus {
					worker_assignment: Some(protos::ateapi::WorkerAssignment {
						worker_pod_ip: self.pod_ip.clone(),
					}),
				}),
				..Default::default()
			}),
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
}

#[derive(Clone)]
struct SelectiveParkingHandler {
	pod_ip: String,
	parked_actor: String,
	entered: Arc<Notify>,
	release: Arc<Notify>,
	calls: Arc<AtomicUsize>,
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
				status: Some(ActorStatus {
					worker_assignment: Some(protos::ateapi::WorkerAssignment {
						worker_pod_ip: self.pod_ip.clone(),
					}),
				}),
				..Default::default()
			}),
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
					worker_assignment: Some(protos::ateapi::WorkerAssignment {
						worker_pod_ip: self.pod_ip.clone(),
					}),
				}),
				..Default::default()
			}),
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

#[tokio::test]
async fn substrate_egress_authorizes_a_reentered_connect_request() {
	let upstream = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));

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
		.attach_route_policy(json!({
			"substrateEgress": {
				"host": "http://dummy", // Egress is not yet implemented
			}
		}))
		.await;

	let mut io = gateway.serve_tunnel_with_tls_info(
		strng::literal!("outer"),
		Some(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				spiffe_id: Some(strng::literal!(
					"spiffe://substrate-actor.local/atespace/demo/actor/my-actor"
				)),
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
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);

	io.write_all(b"GET / HTTP/1.1\r\nHost: allowed.example\r\nConnection: close\r\n\r\n")
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
	assert_eq!(calls.load(Ordering::Relaxed), 0);
}
