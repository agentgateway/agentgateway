use agentgateway::test_helpers::ateapimock;
use agentgateway::types::agent::{Backend, BindMode};
use protos::ateapi::{Actor, ResumeActorResponse};

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
				ateom_pod_ip: self.pod_ip.clone(),
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

	let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), ());
	let mut gateway = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(dynamic.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/dynamic")));
	gateway
		.attach_route_policy(json!({
			"substrateIngress": {
				"host": api.address.to_string(),
				"targetPort": actor.address().port(),
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
}

#[tokio::test]
async fn substrate_egress_authorizes_a_reentered_connect_request() {
	let upstream = simple_mock().await;
	let calls = Arc::new(AtomicUsize::new(0));

	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15012".parse().unwrap();
	let mut inner = simple_bind();
	inner.address = "127.0.0.1:18080".parse().unwrap();
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

	let mut io = gateway.serve_tunnel(strng::literal!("outer"));
	io.write_all(
		b"CONNECT allowed.example:18080 HTTP/1.1\r\nHost: allowed.example:18080\r\nX-Ate-Atespace: demo\r\nX-Ate-Actor: my-actor\r\nX-Ate-Actor-Version: 1\r\n\r\n",
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
