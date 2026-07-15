use agentgateway::http::ext_proc;

use crate::common::prelude::*;
use crate::tests::tls::test_server_tls_config;
// --- Dynamic Forward Proxy (DFP) tests ---

/// Helper to set up a DFP test: creates a Dynamic backend and a route pointing to it.
pub(in crate::tests) fn setup_dfp_bind() -> TestBind {
	setup_dfp_bind_with_config("{}")
}

pub(in crate::tests) fn setup_dfp_bind_with_config(config: &str) -> TestBind {
	let backend_name = ResourceName::new("dynamic".into(), "".into());
	let dynamic_backend = Backend::Dynamic(backend_name, ());

	let route = basic_named_route("/dynamic".into());

	let t = setup_proxy_test(config).unwrap();
	let pi = t.inputs();
	pi.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.name(), dynamic_backend.into());
	t.with_bind(simple_bind()).with_route(route)
}

fn setup_dfp() -> (TestBind, MemoryClient) {
	let t = setup_dfp_bind();
	let io = t.serve_http(BIND_KEY);
	(t, io)
}

/// Helper to set up a DFP test behind an HTTPS listener.
fn setup_dfp_https() -> (TestBind, MemoryClient) {
	let backend_name = ResourceName::new("dynamic".into(), "".into());
	let dynamic_backend = Backend::Dynamic(backend_name, ());

	let route = basic_named_route("/dynamic".into());

	let bind = Bind {
		key: BIND_KEY,
		// not really used
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: Default::default(),
			protocol: ListenerProtocol::HTTPS(test_server_tls_config()),
		}]),
		protocol: BindProtocol::tls,
		tunnel_protocol: Default::default(),
		mode: Default::default(),
	};

	let t = setup_proxy_test("{}").unwrap();
	let pi = t.inputs();
	pi.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.name(), dynamic_backend.into());
	let t = t.with_bind(bind).with_route(route);
	let io = t.serve_https(BIND_KEY, None);
	(t, io)
}

/// DFP and inference routing are orthogonal: DFP chooses the upstream from the request authority,
/// while inference routing expects an endpoint picker to choose the upstream endpoint.
#[tokio::test]
async fn dfp_rejects_inference_routing() {
	let backend_name = ResourceName::new("dynamic".into(), "".into());
	let dynamic_backend = BackendWithPolicies {
		backend: Backend::Dynamic(backend_name, ()),
		inline_policies: vec![BackendTrafficPolicy::InferenceRouting(
			ext_proc::InferenceRouting {
				target: Arc::new(
					agentgateway::types::agent::SimpleBackendReference::InlineBackend(Target::from((
						"127.0.0.1",
						9002,
					))),
				),
				destination_mode: ext_proc::InferenceRoutingDestinationMode::Passthrough,
				failure_mode: ext_proc::FailureMode::FailClosed,
			},
		)],
	};

	let route = basic_named_route("/dynamic".into());
	let t = setup_proxy_test("{}").unwrap();
	let pi = t.inputs();
	pi.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.backend.name(), dynamic_backend);
	let t = t.with_bind(simple_bind()).with_route(route);
	let io = t.serve_http(BIND_KEY);

	let res = send_request(io, Method::GET, "http://example.com/dynamic").await;
	assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
	let body = res.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(
		String::from_utf8_lossy(&body),
		"processing failed: inferenceRouting is not supported with dynamic backends"
	);
}

/// DFP resolves the destination from the request's Host/URI authority, including the port.
#[tokio::test]
async fn dfp_uses_host_port() {
	let mock = simple_mock().await;
	let mock_addr = *mock.address();
	let (_bind, io) = setup_dfp();

	let r = rand::rng().random::<u128>();
	let path = format!("/dfp-explicit-port-{r}");
	let url = format!("http://{mock_addr}{path}");
	let res = send_request(io, Method::GET, &url).await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.uri.path(), path);

	// Also verify telemetry recorded the expected upstream endpoint with the explicit authority port.
	let log =
		agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", &path)])
			.await
			.unwrap();
	let expected_endpoint = mock_addr.to_string();
	assert_eq!(log["endpoint"].as_str(), Some(expected_endpoint.as_str()));
}

/// DFP defaults to port 80 when the URI has no explicit port and scheme is HTTP.
#[tokio::test]
async fn dfp_defaults_to_port_80_for_http() {
	let (_bind, io) = setup_dfp();
	let r = rand::rng().random::<u128>();
	let path = format!("/dfp-http-default-{r}");

	// No port in URI — should default to 80 per HTTP scheme
	let _res = send_request(io, Method::GET, &format!("http://127.0.0.1{path}")).await;

	let log =
		agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", &path)])
			.await
			.unwrap();
	assert_eq!(log["endpoint"].as_str(), Some("127.0.0.1:80"));
}

/// DFP defaults to port 443 when the URI has no explicit port and scheme is HTTPS.
#[tokio::test]
async fn dfp_defaults_to_port_443_for_https() {
	let (_bind, io) = setup_dfp_https();
	let r = rand::rng().random::<u128>();
	let path = format!("/dfp-https-default-{r}");

	// No port in URI over HTTPS listener — should default to 443 per HTTPS scheme
	let _res = send_request(io, Method::GET, &format!("http://127.0.0.1{path}")).await;

	let log =
		agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", &path)])
			.await
			.unwrap();
	assert_eq!(log["endpoint"].as_str(), Some("127.0.0.1:443"));
}
