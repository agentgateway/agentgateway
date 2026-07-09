use crate::common::prelude::*;

#[tokio::test]
async fn waypoint_http_basic() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::GET);
}

#[tokio::test]
async fn waypoint_http_port_selects_distinct_backend() {
	// Two service routes on the same (service, path "/") differ only by their referenced
	// service_port. A request to :443 must reach the :443 backend, :80 the :80 backend.
	async fn id_mock(id: &'static str) -> MockServer {
		let mock = MockServer::start().await;
		Mock::given(wiremock::matchers::path_regex("/.*"))
			.respond_with(ResponseTemplate::new(200).insert_header("x-backend", id))
			.mount(&mock)
			.await;
		mock
	}
	let b80 = id_mock("p80").await;
	let b443 = id_mock("p443").await;

	let svc_nh = agentgateway::types::discovery::NamespacedHostname {
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
	};
	let route = |key: &'static str, port: u16| Route {
		key: strng::new(key),
		service_key: Some(svc_nh.clone()),
		service_port: port,
		name: agentgateway::types::agent::RouteName {
			name: strng::new(key),
			namespace: strng::literal!("default"),
			rule_name: None,
			kind: None,
		},
		hostnames: vec![],
		matches: vec![RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/".into()),
			method: None,
			query: vec![],
		}],
		llm_router: None,
		inline_policies: vec![],
		backends: vec![agentgateway::types::agent::RouteBackendReference {
			weight: 1,
			target: agentgateway::types::agent::BackendReference::Service {
				name: svc_nh.clone(),
				port,
			}
			.into(),
			inline_policies: vec![],
		}],
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service_with_ports(&[(80, *b80.address()), (443, *b443.address())])
		.with_service_route(route("r80", 80))
		.with_service_route(route("r443", 443));

	// Destination :443 -> the 443-scoped route -> the :443 backend.
	let io = t.serve_waypoint_http_port(BIND_KEY, 443);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-backend"), "p443");

	// Destination :80 -> the 80-scoped route -> the :80 backend.
	let io = t.serve_waypoint_http_port(BIND_KEY, 80);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-backend"), "p80");
}

#[tokio::test]
async fn waypoint_http_fallback() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HTTP))
		.with_waypoint_service(*mock.address());
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::POST, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::POST);
}

#[tokio::test]
async fn waypoint_tcp_basic() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	let io = t.serve_waypoint_tcp(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn waypoint_service_policy_header_modifier() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_service_policy(json!({
		"requestHeaderModifier": {
			"add": { "x-svc-req": "from-service" },
		},
		"responseHeaderModifier": {
			"add": { "x-svc-resp": "from-service" },
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-svc-resp"), "from-service");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-svc-req").unwrap().as_bytes(),
		b"from-service"
	);
}

#[tokio::test]
async fn waypoint_service_policy_direct_response() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_service_policy(json!({
		"directResponse": {
			"status": 418,
			"body": "teapot",
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 418);
	assert_eq!(read_body!(res).as_ref(), b"teapot");
}

#[tokio::test]
async fn waypoint_gateway_policy_authz_allow() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": ["source.port == 12345"],
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn waypoint_gateway_policy_authz_deny() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": ["source.port == 54321"],
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	RequestBuilder::new(Method::GET, "http://my-svc.default.svc.cluster.local")
		.send(io)
		.await
		.expect_err("should be denied by network authorization");
}

/// Gateway-targeted network authorization applies to TCP waypoint path.
#[tokio::test]
async fn waypoint_tcp_gateway_policy_authz_deny() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": ["source.port == 54321"],
		},
	}))
	.await;
	let io = t.serve_waypoint_tcp(BIND_KEY);
	RequestBuilder::new(Method::GET, "http://my-svc.default.svc.cluster.local")
		.send(io)
		.await
		.expect_err("should be denied by network authorization");
}
