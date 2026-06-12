use std::net::SocketAddr;
use std::sync::Arc;

use agent_core::strng;
use itertools::Itertools;
use openapiv3::OpenAPI;
use rmcp::RoleClient;
use rmcp::model::{ClientJsonRpcMessage, InitializeRequestParams, RequestId};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpServerConfig;
use secrecy::SecretString;

use crate::http::auth::BackendAuth;
use crate::http::authorization::{PolicySet, RuleSet};
use crate::http::sessionpersistence::MCPSession;
use crate::mcp::handler::Relay;
use crate::mcp::router::{McpBackendGroup, McpTarget};
use crate::mcp::{FailureMode, McpAuthorization, guardrails};
use crate::proxy::httpproxy::PolicyClient;
use crate::test_helpers::extauthmock::{ExtAuthMock, deny_response};
use crate::test_helpers::proxymock::{
	BIND_KEY, TestBind, basic_named_route, basic_route, setup_proxy_test, simple_bind,
};
use crate::test_helpers::ratelimitmock::{RateLimitMock, over_limit_response};
use crate::types::agent::{BackendTrafficPolicy, FrontendPolicy, PolicyTarget, TargetedPolicy};
use crate::*;

#[tokio::test]
async fn stream_to_stream_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn sse_to_stream_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_sse_client(io).await;
	standard_sse_assertions(client).await;
}

#[tokio::test]
async fn stream_to_sse_single() {
	let mock = mock_sse_server().await;
	let (_bind, io) = setup_proxy(&mock, true, true).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn sse_to_sse_single() {
	let mock = mock_sse_server().await;
	let (_bind, io) = setup_proxy(&mock, true, true).await;
	let client = mcp_sse_client(io).await;
	standard_sse_assertions(client).await;
}

#[tokio::test]
async fn stream_to_multiplex() {
	let mock_stream = mock_streamable_http_server(true).await;
	let mock_sse = mock_sse_server().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("sse", mock_sse.addr, true),
				("mcp", mock_stream.addr, false),
			],
			true,
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;
	let tools = client.list_tools(None).await.unwrap();
	let t = tools
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.filter(|n| n.contains("decrement") || n.contains("echo"))
		.collect_vec();
	assert_eq!(
		t,
		vec![
			"mcp_decrement".to_string(),
			"mcp_echo".to_string(),
			"mcp_echo_http".to_string(),
			"sse_decrement".to_string(),
			"sse_echo".to_string(),
			"sse_echo_http".to_string()
		]
	);

	let ctr = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("mcp_echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);

	let ctr = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("sse_echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);

	// No target set...
	assert!(
		client
			.call_tool(
				rmcp::model::CallToolRequestParams::new("echo").with_arguments(
					serde_json::json!({"hi": "world"})
						.as_object()
						.cloned()
						.unwrap(),
				),
			)
			.await
			.is_err()
	);
}

#[tokio::test]
async fn stream_to_multiplex_resources() {
	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
			true,
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	// 1. list_resources should return resources from both backends with prefixed URIs
	let resources = client.list_resources(None).await.unwrap();
	let uris: Vec<String> = resources
		.resources
		.iter()
		.map(|r| r.uri.clone())
		.sorted()
		.collect();
	// Each mock provides "str:////Users/to/some/path/" and "memo://insights"
	// With multiplexing these become "a+str:////Users/to/some/path/", "a+memo://insights", etc.
	assert!(
		uris.iter().any(|u| u == "a+memo://insights"),
		"Expected 'a+memo://insights' in resources, got: {:?}",
		uris
	);
	assert!(
		uris.iter().any(|u| u == "b+memo://insights"),
		"Expected 'b+memo://insights' in resources, got: {:?}",
		uris
	);

	// 2. read_resource with prefixed URI should route to the correct backend
	let result = client
		.read_resource(rmcp::model::ReadResourceRequestParams::new(
			"a+memo://insights",
		))
		.await
		.unwrap();
	assert!(
		!result.contents.is_empty(),
		"Expected non-empty resource contents"
	);
	let text = match &result.contents[0] {
		rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
		other => panic!("Expected text resource content, got: {:?}", other),
	};
	assert!(
		text.contains("Business Intelligence Memo"),
		"Expected memo content, got: {}",
		text
	);

	// Also read from backend "b"
	let result_b = client
		.read_resource(rmcp::model::ReadResourceRequestParams::new(
			"b+memo://insights",
		))
		.await
		.unwrap();
	assert!(
		!result_b.contents.is_empty(),
		"Expected non-empty resource contents from backend b"
	);

	// 3. list_resource_templates should not error (mock returns empty)
	let templates = client.list_resource_templates(None).await.unwrap();
	// Templates may be empty since mock server returns empty vec, but should not error
	assert!(
		templates.resource_templates.is_empty(),
		"Expected empty resource templates from mock, got: {:?}",
		templates.resource_templates
	);

	// 4. read_resource with unprefixed URI should fail
	assert!(
		client
			.read_resource(rmcp::model::ReadResourceRequestParams::new(
				"memo://insights",
			))
			.await
			.is_err(),
		"Expected error when reading resource without service prefix"
	);
}

#[tokio::test]
async fn multiplex_advertises_tool_and_resource_subscribe_capabilities() {
	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
			true,
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;
	let caps = &client.peer_info().unwrap().capabilities;
	assert_eq!(caps.tools.as_ref().unwrap().list_changed, Some(true));
	assert_eq!(caps.prompts.as_ref().unwrap().list_changed, Some(true));
	assert_eq!(caps.resources.as_ref().unwrap().list_changed, Some(true));
	assert_eq!(caps.resources.as_ref().unwrap().subscribe, Some(true));
}

#[tokio::test]
async fn stateless_multiplex_does_not_advertise_resource_subscribe() {
	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
			false,
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;
	let caps = &client.peer_info().unwrap().capabilities;
	assert_ne!(caps.resources.as_ref().unwrap().subscribe, Some(true));
}

#[tokio::test]
async fn stateless_multiplex_tool_call_initializes_only_target() {
	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
			false,
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;
	let a_init_before = mock_a.init_count().await;
	let b_init_before = mock_b.init_count().await;

	// A direct tool call to one target should initialize only that target.
	let _ = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("a_echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.unwrap();
	let a_init_after = mock_a.init_count().await;
	let b_init_after = mock_b.init_count().await;
	assert_eq!(a_init_after, a_init_before + 1);
	assert_eq!(b_init_after, b_init_before);
}

#[test]
fn stateless_multiplex_get_prompt_initializes_only_target() {
	// This test stacks the rmcp client, proxy, stateless initialize wrapper,
	// upstream streamable HTTP client, and rmcp mock server initialize path in
	// one integration flow. On small CI test stacks (reproduced with
	// RUST_MIN_STACK=1048576) that combined async polling stack overflows before
	// the initialize response completes. Use an explicit worker stack here so the
	// test continues to exercise the real path instead of depending on libtest's
	// default thread stack.
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.thread_stack_size(4 * 1024 * 1024)
		.build()
		.unwrap();
	runtime.block_on(async {
		let mock_a = mock_streamable_http_server(true).await;
		let mock_b = mock_streamable_http_server(true).await;
		let t = setup_proxy_test("{}")
			.unwrap()
			.with_multiplex_mcp_backend(
				"mcp",
				vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
				false,
			)
			.with_bind(simple_bind())
			.with_route(basic_named_route(strng::new("/mcp")));
		let io = t.serve_real_listener(strng::new("bind")).await;
		let client = mcp_streamable_client(io).await;
		let a_init_before = mock_a.init_count().await;
		let b_init_before = mock_b.init_count().await;

		let _ = client
			.get_prompt(
				rmcp::model::GetPromptRequestParams::new("a_example_prompt").with_arguments(
					serde_json::json!({"message": "hello"})
						.as_object()
						.cloned()
						.unwrap(),
				),
			)
			.await
			.unwrap();

		let a_init_after = mock_a.init_count().await;
		let b_init_after = mock_b.init_count().await;
		assert_eq!(a_init_after, a_init_before + 1);
		assert_eq!(b_init_after, b_init_before);
	});
	runtime.shutdown_timeout(std::time::Duration::from_secs(1));
}

#[tokio::test]
async fn stateless_multiplex_delete_session_skips_uninitialized_targets() {
	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				fake_streamable_target("a", mock_a.addr),
				fake_streamable_target("b", mock_b.addr),
			],
			stateful: false,
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();
	let session_manager =
		super::session::SessionManager::new(http::sessionpersistence::Encoder::base64());
	let mut session = session_manager.create_stateless_session(relay);
	let parts = ::http::Request::<()>::builder()
		.method(http::Method::POST)
		.uri("http://localhost/mcp")
		.body(())
		.unwrap()
		.into_parts()
		.0;

	session
		.stateless_send_and_initialize(
			parts.clone(),
			ClientJsonRpcMessage::request(
				rmcp::model::CallToolRequest::new(
					rmcp::model::CallToolRequestParams::new("a_echo").with_arguments(
						serde_json::json!({"hi": "world"})
							.as_object()
							.cloned()
							.unwrap(),
					),
				)
				.into(),
				RequestId::Number(1),
			),
		)
		.await
		.unwrap();

	let sessions = match http::sessionpersistence::SessionState::decode(
		session.id.as_ref(),
		&http::sessionpersistence::Encoder::base64(),
	)
	.unwrap()
	{
		http::sessionpersistence::SessionState::MCP(state) => state.sessions,
		_ => panic!("expected MCP session state"),
	};
	assert_eq!(sessions.len(), 2);
	assert_eq!(sessions[0].target_name.as_deref(), Some("a"));
	assert!(sessions[0].session.is_some());
	assert_eq!(sessions[1].target_name.as_deref(), Some("b"));
	assert!(sessions[1].session.is_none());

	let response = session.delete_session(parts).await.unwrap();
	assert_eq!(response.status(), http::StatusCode::ACCEPTED);
	assert_eq!(mock_b.init_count().await, 0);
}

#[tokio::test]
async fn stateful_streamable_http_rejects_no_session_non_initialize_messages() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = reqwest::Client::new();
	let url = format!("http://{io}/mcp");

	for body in [
		serde_json::json!({
			"jsonrpc": "2.0",
			"method": "notifications/initialized",
			"params": {}
		}),
		serde_json::json!({
			"jsonrpc": "2.0",
			"id": 1,
			"result": {}
		}),
		serde_json::json!({
			"jsonrpc": "2.0",
			"id": 1,
			"error": {
				"code": -32603,
				"message": "client response error"
			}
		}),
	] {
		let response = mcp_json_post(&client, &url, &body).send().await.unwrap();
		assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
		assert!(
			response.headers().get("mcp-session-id").is_none(),
			"rejected no-session message must not create a session"
		);
	}
}

#[tokio::test]
async fn streamable_http_validates_protocol_version_header() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = reqwest::Client::new();
	let url = format!("http://{io}/mcp");
	let init_body = serde_json::json!({
		"jsonrpc": "2.0",
		"id": 1,
		"method": "initialize",
		"params": {
			"protocolVersion": "2025-06-18",
			"capabilities": {},
			"clientInfo": {
				"name": "test client",
				"version": "0.0.1"
			}
		}
	});

	let unsupported = mcp_json_post(&client, &url, &init_body)
		.header("mcp-protocol-version", "1900-01-01")
		.send()
		.await
		.unwrap();
	assert_eq!(unsupported.status(), reqwest::StatusCode::BAD_REQUEST);

	let mismatch = mcp_json_post(&client, &url, &init_body)
		.header("mcp-protocol-version", "2025-11-25")
		.send()
		.await
		.unwrap();
	assert_eq!(mismatch.status(), reqwest::StatusCode::BAD_REQUEST);

	let init = mcp_json_post(&client, &url, &init_body)
		.header("mcp-protocol-version", "2025-06-18")
		.send()
		.await
		.unwrap();
	assert_eq!(init.status(), reqwest::StatusCode::OK);
	let session_id = init
		.headers()
		.get("mcp-session-id")
		.expect("initialize response should include a session id")
		.to_str()
		.unwrap()
		.to_string();

	let list_body = serde_json::json!({
		"jsonrpc": "2.0",
		"id": 2,
		"method": "tools/list",
		"params": {}
	});
	let subsequent_unsupported = mcp_json_post(&client, &url, &list_body)
		.header("mcp-session-id", session_id)
		.header("mcp-protocol-version", "1900-01-01")
		.send()
		.await
		.unwrap();
	assert_eq!(
		subsequent_unsupported.status(),
		reqwest::StatusCode::BAD_REQUEST
	);
}

fn mcp_json_post<'a>(
	client: &'a reqwest::Client,
	url: &'a str,
	body: &'a serde_json::Value,
) -> reqwest::RequestBuilder {
	client
		.post(url)
		.header(
			http::header::ACCEPT.as_str(),
			"application/json, text/event-stream",
		)
		.header(http::header::CONTENT_TYPE.as_str(), "application/json")
		.json(body)
}

#[tokio::test]
async fn stateless_to_stateful() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, false, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn stateless_to_stateless() {
	let mock = mock_streamable_http_server(false).await;
	let (_bind, io) = setup_proxy(&mock, false, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn stream_to_stream_single_tls() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendTrafficPolicy::BackendAuth(BackendAuth::Key {
			value: SecretString::new("my-key".into()),
			location: None,
		})],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let ctr = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo_http").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"Bearer my-key"#
	);
}

/// Test that calling a tool denied by MCP authorization policy returns proper JSON-RPC error
/// with INVALID_PARAMS error code (-32602) and message "Unknown tool: {tool_name}"
#[tokio::test]
async fn authorization_denied_returns_unknown_tool_error() {
	let mock = mock_streamable_http_server(true).await;

	// Create an MCP authorization policy that denies all tools
	// The deny rule matches all tools; no allow rules means everything is denied
	let deny_all_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],                                                       // no allow rules
		vec![Arc::new(cel::Expression::new_strict("true").unwrap())], // deny all
		vec![],
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendTrafficPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to call a tool - should fail with "Unknown tool" error
	let result = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected tool call to fail due to authorization denial"
	);

	let err = result.unwrap_err();

	// Verify error code is INVALID_PARAMS (-32602) and message format
	let mcp_error = match &err {
		rmcp::ServiceError::McpError(mcp_error) => mcp_error,
		// rmcp::ServiceError::TransportSend(d) => d.downcast::(),
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	};

	assert_eq!(
		mcp_error.code.0, -32602,
		"Expected INVALID_PARAMS error code (-32602), got: {}",
		mcp_error.code.0
	);
	assert_eq!(
		mcp_error.message.as_ref(),
		"Unknown tool: echo",
		"Expected error message 'Unknown tool: echo', got: {}",
		mcp_error.message
	);
}

/// Test that getting a prompt denied by MCP authorization policy returns proper JSON-RPC error
/// with INVALID_PARAMS error code (-32602) and message "Unknown prompt: {prompt_name}"
#[tokio::test]
async fn authorization_denied_returns_unknown_prompt_error() {
	let mock = mock_streamable_http_server(true).await;

	// Create an MCP authorization policy that denies all prompts
	let deny_all_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],                                                       // no allow rules
		vec![Arc::new(cel::Expression::new_strict("true").unwrap())], // deny all
		vec![],
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendTrafficPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to get a prompt - should fail with "Unknown prompt" error
	let result = client
		.get_prompt(rmcp::model::GetPromptRequestParams::new("example_prompt"))
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected get_prompt call to fail due to authorization denial"
	);

	let err = result.unwrap_err();

	// Verify error code is INVALID_PARAMS (-32602) and message format
	match &err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert_eq!(
				mcp_error.message.as_ref(),
				"Unknown prompt: example_prompt",
				"Expected error message 'Unknown prompt: example_prompt', got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

/// Test that reading a resource denied by MCP authorization policy returns proper JSON-RPC error
/// with INVALID_PARAMS error code (-32602) and message "Unknown resource: {resource_uri}"
#[tokio::test]
async fn authorization_denied_returns_unknown_resource_error() {
	let mock = mock_streamable_http_server(true).await;

	// Create an MCP authorization policy that denies all resources
	let deny_all_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],                                                       // no allow rules
		vec![Arc::new(cel::Expression::new_strict("true").unwrap())], // deny all
		vec![],
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendTrafficPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to read a resource - should fail with "Unknown resource" error
	let result = client
		.read_resource(rmcp::model::ReadResourceRequestParams::new(
			"memo://insights",
		))
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected read_resource call to fail due to authorization denial"
	);

	let err = result.unwrap_err();

	// Verify error code is INVALID_PARAMS (-32602) and message format
	match &err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert_eq!(
				mcp_error.message.as_ref(),
				"Unknown resource: memo://insights",
				"Expected error message 'Unknown resource: memo://insights', got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

#[tokio::test]
async fn resource_subscribe_and_unsubscribe_forward_to_single_backend() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	client
		.subscribe(rmcp::model::SubscribeRequestParams::new("memo://insights"))
		.await
		.unwrap();
	client
		.unsubscribe(rmcp::model::UnsubscribeRequestParams::new(
			"memo://insights",
		))
		.await
		.unwrap();
}

#[tokio::test]
async fn multiplex_resource_subscribe_and_unsubscribe_route_to_target() {
	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
			true,
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	client
		.subscribe(rmcp::model::SubscribeRequestParams::new(
			"a+memo://insights",
		))
		.await
		.unwrap();
	client
		.unsubscribe(rmcp::model::UnsubscribeRequestParams::new(
			"a+memo://insights",
		))
		.await
		.unwrap();

	assert!(
		client
			.subscribe(rmcp::model::SubscribeRequestParams::new("memo://insights"))
			.await
			.is_err(),
		"expected unprefixed multiplex resource subscribe to fail"
	);
}

#[tokio::test]
async fn multiplex_resource_updated_notification_is_prefixed() {
	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
			true,
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let (client, updated_uri, notify) = mcp_streamable_client_capture_resource_updates(io).await;

	client
		.subscribe(rmcp::model::SubscribeRequestParams::new(
			"a+memo://insights",
		))
		.await
		.unwrap();
	tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
		.await
		.unwrap();

	assert_eq!(
		updated_uri.lock().await.as_deref(),
		Some("a+memo://insights")
	);
}

#[tokio::test]
async fn single_resource_updated_notification_is_not_prefixed() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let (client, updated_uri, notify) = mcp_streamable_client_capture_resource_updates(io).await;

	client
		.subscribe(rmcp::model::SubscribeRequestParams::new("memo://insights"))
		.await
		.unwrap();
	tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
		.await
		.unwrap();

	assert_eq!(updated_uri.lock().await.as_deref(), Some("memo://insights"));
}

/// Test that a deny policy targeting a specific tool filters only that tool from list_tools,
/// while leaving all other tools accessible.
#[tokio::test]
async fn authorization_deny_specific_tool_filters_only_that_tool() {
	let mock = mock_streamable_http_server(true).await;

	// Create a deny policy that only denies the "echo" tool
	let deny_echo_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],
		vec![Arc::new(
			cel::Expression::new_strict(r#"mcp.tool.name == "echo""#).unwrap(),
		)],
		vec![],
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendTrafficPolicy::McpAuthorization(deny_echo_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// List tools - "echo" should be filtered out, all others should remain
	let tools = client.list_tools(None).await.unwrap();
	let tool_names: Vec<String> = tools
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.collect();

	// The mock server has: increment, decrement, get_value, say_hello, echo, sum, echo_http
	// After denying "echo", we should have all except "echo"
	assert!(
		!tool_names.contains(&"echo".to_string()),
		"echo should be denied but was found in tools: {:?}",
		tool_names
	);
	assert!(
		tool_names.contains(&"increment".to_string()),
		"increment should be allowed but was not found in tools: {:?}",
		tool_names
	);
	assert!(
		tool_names.contains(&"decrement".to_string()),
		"decrement should be allowed but was not found in tools: {:?}",
		tool_names
	);
	assert!(
		tool_names.len() >= 5,
		"Expected at least 5 tools after denying 1, got {}: {:?}",
		tool_names.len(),
		tool_names
	);
}

/// Test that a deny policy using request.headers correctly filters tools per-agent.
/// This exercises the router.rs fix that registers authorization policies on the log's
/// CEL context so the request snapshot includes headers needed by CEL expressions.
#[tokio::test]
async fn authorization_deny_with_request_header_filters_per_agent() {
	use std::collections::HashMap;

	use ::http::{HeaderName, HeaderValue};
	use rmcp::ServiceExt;
	use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use rmcp::transport::StreamableHttpClientTransport;
	use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

	let mock = mock_streamable_http_server(true).await;

	// Deny "echo" only when request header x-agent-name == "agent-one"
	let deny_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],
		vec![Arc::new(
			cel::Expression::new_strict(
				r#"mcp.tool.name == "echo" && request.headers["x-agent-name"] == "agent-one""#,
			)
			.unwrap(),
		)],
		vec![],
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendTrafficPolicy::McpAuthorization(deny_policy)],
	)
	.await;

	// Helper to create a client with custom headers
	let make_client = |addr: SocketAddr, agent_name: &'static str| async move {
		let mut headers = HashMap::new();
		headers.insert(
			HeaderName::from_static("x-agent-name"),
			HeaderValue::from_static(agent_name),
		);
		let config = StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"))
			.custom_headers(headers);
		let transport = StreamableHttpClientTransport::from_config(config);
		let client_info = ClientInfo::new(
			ClientCapabilities::default(),
			Implementation::new(format!("test-{agent_name}"), "0.0.1"),
		);
		client_info
			.serve(transport)
			.await
			.expect("client should connect")
	};

	// Agent-one: "echo" should be denied
	let client1 = make_client(io, "agent-one").await;
	let tools1: Vec<String> = client1
		.list_tools(None)
		.await
		.unwrap()
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.collect();

	assert!(
		!tools1.contains(&"echo".to_string()),
		"agent-one should NOT see 'echo' but tools were: {:?}",
		tools1
	);
	assert!(
		tools1.contains(&"increment".to_string()),
		"agent-one should still see 'increment' but tools were: {:?}",
		tools1
	);

	// Agent-two: "echo" should be allowed (header doesn't match deny rule)
	let client2 = make_client(io, "agent-two").await;
	let tools2: Vec<String> = client2
		.list_tools(None)
		.await
		.unwrap()
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.collect();

	assert!(
		tools2.contains(&"echo".to_string()),
		"agent-two SHOULD see 'echo' but tools were: {:?}",
		tools2
	);
	assert!(
		tools2.contains(&"increment".to_string()),
		"agent-two should still see 'increment' but tools were: {:?}",
		tools2
	);
}

#[tokio::test]
async fn mcp_authentication_early_response_transformation_has_request_context() {
	let mock = mock_streamable_http_server(true).await;
	let authn = crate::types::agent::McpAuthentication {
		issuer: "https://issuer.example.com".to_string(),
		audiences: vec!["mcp".to_string()],
		provider: None,
		resource_metadata: crate::types::agent::ResourceMetadata {
			extra: Default::default(),
		},
		jwt_validator: Arc::new(crate::http::jwt::Jwt::from_providers(
			vec![],
			crate::http::jwt::Mode::Strict,
			crate::http::auth::AuthorizationLocation::bearer_header(),
		)),
		mode: crate::types::agent::McpAuthenticationMode::Strict,
		client_id: None,
	};

	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend_policies(
			mock.addr,
			true,
			false,
			vec![BackendTrafficPolicy::McpAuthentication(authn)],
		)
		.with_bind(simple_bind())
		.with_route(basic_route(mock.addr));

	t.attach_route_policy(serde_json::json!({
		"transformations": {
			"response": {
				"set": {
					"x-request-id-from-cel": "request.headers[\"x-regression-id\"]",
					"x-request-path-from-cel": "request.path"
				}
			}
		}
	}))
	.await;

	let io = t.serve_real_listener(BIND_KEY).await;
	let resp = reqwest::Client::new()
		.get(format!(
			"http://{io}/.well-known/oauth-protected-resource/mcp"
		))
		.header("x-regression-id", "mcp-authn-snapshot")
		.send()
		.await
		.expect("metadata request should complete");

	assert_eq!(resp.status(), reqwest::StatusCode::OK);
	assert_eq!(
		resp
			.headers()
			.get("x-request-id-from-cel")
			.and_then(|v| v.to_str().ok()),
		Some("mcp-authn-snapshot")
	);
	assert_eq!(
		resp
			.headers()
			.get("x-request-path-from-cel")
			.and_then(|v| v.to_str().ok()),
		Some("/.well-known/oauth-protected-resource/mcp")
	);
}

async fn standard_assertions(client: RunningService<RoleClient, InitializeRequestParams>) {
	let tools = client.list_tools(None).await.unwrap();
	let t = tools
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.take(2)
		.collect_vec();
	assert_eq!(t, vec!["decrement".to_string(), "echo".to_string()]);
	let ctr = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);
}

async fn standard_sse_assertions(client: LegacyService) {
	let tools = client.list_tools(None).await.unwrap();
	let t = tools
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.take(2)
		.collect_vec();
	assert_eq!(t, vec!["decrement".to_string(), "echo".to_string()]);
	let ctr = client
		.call_tool(legacy_rmcp::model::CallToolRequestParam {
			name: "echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);
}

fn access_log_payload_policy() -> crate::types::frontend::LoggingPolicy {
	let mut policy: crate::types::frontend::LoggingPolicy =
		serde_json::from_value(serde_json::json!({
			"add": {
				"mcp_trace": "mcp.tool.arguments.traceId",
				"mcp_method_cel": "mcp.methodName",
				"mcp_session_cel": "mcp.sessionId",
				"mcp_tool_name_cel": "mcp.tool.name",
				"mcp_tool_target_cel": "mcp.tool.target",
				"mcp_prompt_name_cel": "mcp.prompt.name",
				"mcp_prompt_target_cel": "mcp.prompt.target",
				"mcp_args_cel": "mcp.tool.arguments",
				"mcp_result_cel": "mcp.tool.result",
				"mcp_error_cel": "mcp.tool.error",
				"proxy_request_processing_duration_cel": "proxy.requestProcessingDuration",
				"proxy_upstream_duration_cel": "proxy.upstreamDuration",
				"proxy_response_processing_duration_cel": "proxy.responseProcessingDuration"
			}
		}))
		.unwrap();
	policy.init_access_log_policy();
	policy
}

async fn setup_access_log_mcp_proxy(mock: &MockServer) -> (TestBind, SocketAddr) {
	let (mut t, io) = setup_proxy(mock, true, false).await;
	let listener_name = t
		.pi
		.stores
		.read_binds()
		.bind(&BIND_KEY)
		.unwrap()
		.listeners
		.iter()
		.next()
		.unwrap()
		.name
		.clone();
	t.with_policy(TargetedPolicy {
		key: "frontend/accessLog".into(),
		name: None,
		target: PolicyTarget::Gateway(listener_name.clone().into()),
		inheritance: Default::default(),
		policy: FrontendPolicy::AccessLog(access_log_payload_policy()).into(),
	});
	assert!(
		t.pi
			.stores
			.read_binds()
			.listener_frontend_policies(&listener_name, None, None)
			.access_log
			.is_some()
	);
	(t, io)
}

#[tokio::test]
async fn tool_call_exposes_payload_fields_to_access_log_cel() {
	let mock = mock_streamable_http_server(true).await;
	let trace_id = format!("mcp-e2e-{}", uuid::Uuid::new_v4());
	let (_t, io) = setup_access_log_mcp_proxy(&mock).await;
	let client = mcp_streamable_client(io).await;

	let result = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({
					"traceId": trace_id,
					"hi": "world",
				})
				.as_object()
				.cloned()
				.expect("tool arguments should serialize to an object"),
			),
		)
		.await
		.unwrap();
	let direct_result_text = &result.content[0].raw.as_text().unwrap().text;
	let direct_result_json: serde_json::Value =
		serde_json::from_str(direct_result_text).expect("tool result should be valid JSON text");
	assert_eq!(direct_result_json["traceId"], trace_id);
	assert_eq!(direct_result_json["hi"], "world");

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("mcp_trace", &trace_id),
	])
	.await
	.unwrap();

	assert_eq!(
		log.get("mcp_method_cel"),
		Some(&serde_json::json!("tools/call"))
	);
	assert_eq!(
		log.get("mcp_tool_name_cel"),
		Some(&serde_json::json!("echo"))
	);
	assert_eq!(
		log.get("mcp_tool_target_cel"),
		Some(&serde_json::json!("mcp"))
	);
	assert_eq!(log["mcp_args_cel"]["traceId"], trace_id);
	assert_eq!(log["mcp_args_cel"]["hi"], "world");
	assert!(
		log["mcp_session_cel"]
			.as_str()
			.is_some_and(|session_id| !session_id.is_empty())
	);
	assert_eq!(log["mcp_result_cel"]["isError"], false);

	let result_text = log["mcp_result_cel"]["content"][0]["text"]
		.as_str()
		.expect("tool result text should be logged");
	let result_json: serde_json::Value =
		serde_json::from_str(result_text).expect("tool result should be valid JSON text");
	assert_eq!(result_json["traceId"], trace_id);
	assert_eq!(result_json["hi"], "world");
	assert!(log.get("mcp_error_cel").is_none());
	assert_duration_log_field(&log, "proxy_request_processing_duration_cel");
	assert_duration_log_field(&log, "proxy_upstream_duration_cel");
	assert_duration_log_field(&log, "proxy_response_processing_duration_cel");

	assert_eq!(
		log.get("gen_ai.tool.name"),
		Some(&serde_json::json!("echo"))
	);
	assert!(log.get("gen_ai.tool.call.arguments").is_none());
	assert!(log.get("gen_ai.tool.call.result").is_none());
}

fn assert_duration_log_field(log: &serde_json::Value, field: &str) {
	assert!(
		log
			.get(field)
			.and_then(|value| value.as_str())
			.is_some_and(|value| !value.is_empty()),
		"{field} should be present and non-empty"
	);
}

#[tokio::test]
async fn tool_call_error_exposes_error_payload_to_access_log_cel() {
	let mock = mock_streamable_http_server(true).await;
	let trace_id = format!("mcp-e2e-error-{}", uuid::Uuid::new_v4());
	let (_t, io) = setup_access_log_mcp_proxy(&mock).await;
	let client = mcp_streamable_client(io).await;

	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("does_not_exist").with_arguments(
				serde_json::json!({
					"traceId": trace_id,
				})
				.as_object()
				.cloned()
				.expect("tool arguments should serialize to an object"),
			),
		)
		.await
		.unwrap_err();
	match &err {
		rmcp::ServiceError::McpError(mcp_error) => assert_eq!(mcp_error.code.0, -32602),
		other => panic!("Expected ServiceError::McpError, got: {other:?}"),
	}

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("mcp_trace", &trace_id),
	])
	.await
	.unwrap();

	assert_eq!(
		log.get("mcp_method_cel"),
		Some(&serde_json::json!("tools/call"))
	);
	assert_eq!(
		log.get("mcp_tool_name_cel"),
		Some(&serde_json::json!("does_not_exist"))
	);
	assert_eq!(log["mcp_args_cel"]["traceId"], trace_id);
	assert_eq!(log["mcp_error_cel"]["code"], -32602);
	assert!(
		log["mcp_error_cel"]["message"]
			.as_str()
			.is_some_and(|message| message.contains("tool"))
	);
	assert!(log.get("mcp_result_cel").is_none());
	assert_eq!(
		log.get("gen_ai.tool.name"),
		Some(&serde_json::json!("does_not_exist"))
	);
	assert!(log.get("gen_ai.tool.call.arguments").is_none());
	assert!(log.get("gen_ai.tool.call.result").is_none());
}

#[tokio::test]
async fn legacy_sse_tool_call_exposes_arguments_without_terminal_payloads() {
	let mock = mock_streamable_http_server(true).await;
	let trace_id = format!("mcp-e2e-sse-{}", uuid::Uuid::new_v4());
	let (_t, io) = setup_access_log_mcp_proxy(&mock).await;
	let client = mcp_sse_client(io).await;

	let result = client
		.call_tool(legacy_rmcp::model::CallToolRequestParam {
			name: "echo".into(),
			arguments: serde_json::json!({
				"traceId": trace_id,
				"hi": "world",
			})
			.as_object()
			.cloned(),
		})
		.await
		.unwrap();
	let direct_result_text = &result.content[0].raw.as_text().unwrap().text;
	let direct_result_json: serde_json::Value =
		serde_json::from_str(direct_result_text).expect("tool result should be valid JSON text");
	assert_eq!(direct_result_json["traceId"], trace_id);
	assert_eq!(direct_result_json["hi"], "world");

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("mcp_trace", &trace_id),
	])
	.await
	.unwrap();

	assert_eq!(
		log.get("mcp_method_cel"),
		Some(&serde_json::json!("tools/call"))
	);
	assert_eq!(
		log.get("mcp_tool_name_cel"),
		Some(&serde_json::json!("echo"))
	);
	assert_eq!(log["mcp_args_cel"]["traceId"], trace_id);
	assert_eq!(log["mcp_args_cel"]["hi"], "world");
	assert!(log.get("mcp_result_cel").is_none());
	assert!(log.get("mcp_error_cel").is_none());

	assert_eq!(
		log.get("gen_ai.tool.name"),
		Some(&serde_json::json!("echo"))
	);
	assert!(log.get("gen_ai.tool.call.arguments").is_none());
	assert!(log.get("gen_ai.tool.call.result").is_none());
}

#[tokio::test]
async fn prompt_request_emits_gen_ai_prompt_name() {
	let mock = mock_streamable_http_server(true).await;
	let (_t, io) = setup_access_log_mcp_proxy(&mock).await;
	let client = mcp_streamable_client(io).await;

	let _result = client
		.get_prompt(
			rmcp::model::GetPromptRequestParams::new("example_prompt").with_arguments(
				serde_json::json!({ "message": "hello" })
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.unwrap();

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("mcp_prompt_name_cel", "example_prompt"),
	])
	.await
	.unwrap();

	assert_eq!(
		log.get("mcp_method_cel"),
		Some(&serde_json::json!("prompts/get"))
	);
	assert_eq!(
		log.get("gen_ai.prompt.name"),
		Some(&serde_json::json!("example_prompt"))
	);
	assert!(log.get("gen_ai.tool.name").is_none());
	assert!(log.get("mcp_tool_name_cel").is_none());
}

async fn setup_proxy(
	mock: &MockServer,
	stateful: bool,
	legacy_sse: bool,
) -> (TestBind, SocketAddr) {
	setup_proxy_policies(mock, stateful, legacy_sse, vec![]).await
}

async fn setup_proxy_policies(
	mock: &MockServer,
	stateful: bool,
	legacy_sse: bool,
	policies: Vec<BackendTrafficPolicy>,
) -> (TestBind, SocketAddr) {
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend_policies(mock.addr, stateful, legacy_sse, policies)
		.with_bind(simple_bind())
		.with_route(basic_route(mock.addr));
	let io = t.serve_real_listener(BIND_KEY).await;
	(t, io)
}

// Like `setup_proxy_policies`, but also attaches `target_policies` to the opaque
// backend behind the MCP target so they run on the upstream leg.
async fn setup_proxy_policies_with_target(
	mock: &MockServer,
	stateful: bool,
	legacy_sse: bool,
	policies: Vec<BackendTrafficPolicy>,
	target_policies: Vec<BackendTrafficPolicy>,
) -> (TestBind, SocketAddr) {
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend_and_target_policies(
			mock.addr,
			stateful,
			legacy_sse,
			policies,
			target_policies,
		)
		.with_bind(simple_bind())
		.with_route(basic_route(mock.addr));
	let io = t.serve_real_listener(BIND_KEY).await;
	(t, io)
}

pub async fn mcp_streamable_client(
	s: SocketAddr,
) -> RunningService<RoleClient, InitializeRequestParams> {
	use rmcp::ServiceExt;
	use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use rmcp::transport::StreamableHttpClientTransport;
	let transport =
		StreamableHttpClientTransport::<reqwest::Client>::from_uri(format!("http://{s}/mcp"));
	let client_info = ClientInfo::new(
		ClientCapabilities::default(),
		Implementation::new("test client".to_string(), "0.0.1".to_string()),
	);

	Box::pin(client_info.serve(transport))
		.await
		.inspect_err(|e| {
			tracing::error!("client error: {:?}", e);
		})
		.unwrap()
}

#[derive(Clone)]
struct ResourceUpdateClient {
	updated_uri: Arc<tokio::sync::Mutex<Option<String>>>,
	notify: Arc<tokio::sync::Notify>,
}

impl rmcp::ClientHandler for ResourceUpdateClient {
	async fn on_resource_updated(
		&self,
		params: rmcp::model::ResourceUpdatedNotificationParam,
		_: rmcp::service::NotificationContext<RoleClient>,
	) {
		*self.updated_uri.lock().await = Some(params.uri);
		self.notify.notify_one();
	}
}

async fn mcp_streamable_client_capture_resource_updates(
	s: SocketAddr,
) -> (
	RunningService<RoleClient, ResourceUpdateClient>,
	Arc<tokio::sync::Mutex<Option<String>>>,
	Arc<tokio::sync::Notify>,
) {
	use rmcp::ServiceExt;
	use rmcp::transport::StreamableHttpClientTransport;
	let transport =
		StreamableHttpClientTransport::<reqwest::Client>::from_uri(format!("http://{s}/mcp"));
	let updated_uri = Arc::new(tokio::sync::Mutex::new(None));
	let notify = Arc::new(tokio::sync::Notify::new());
	let client = ResourceUpdateClient {
		updated_uri: updated_uri.clone(),
		notify: notify.clone(),
	};

	(
		Box::pin(client.serve(transport)).await.unwrap(),
		updated_uri,
		notify,
	)
}

type LegacyService = legacy_rmcp::service::RunningService<
	legacy_rmcp::RoleClient,
	legacy_rmcp::model::InitializeRequestParam,
>;

pub async fn mcp_sse_client(s: SocketAddr) -> LegacyService {
	use legacy_rmcp::ServiceExt;
	use legacy_rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use legacy_rmcp::transport::SseClientTransport;
	let transport = SseClientTransport::<legacyreqwest::Client>::start(format!("http://{s}/sse"))
		.await
		.unwrap();
	let client_info = ClientInfo {
		protocol_version: Default::default(),
		capabilities: ClientCapabilities::default(),
		client_info: Implementation {
			name: "test client".to_string(),
			version: "0.0.1".to_string(),
			title: None,
			website_url: None,
			icons: None,
		},
	};

	Box::pin(client_info.serve(transport)).await.unwrap()
}

struct MockServer {
	addr: SocketAddr,
	init_counter: std::sync::Arc<tokio::sync::Mutex<i32>>,
	_cancel: tokio::sync::oneshot::Sender<()>,
}

impl MockServer {
	async fn init_count(&self) -> i32 {
		*self.init_counter.lock().await
	}
}

async fn mock_streamable_http_server(stateful: bool) -> MockServer {
	mock_streamable_http_server_inner(stateful, None).await
}

type HeaderCapture = std::sync::Arc<std::sync::Mutex<Vec<http::HeaderMap>>>;

async fn mock_streamable_http_server_with_capture(stateful: bool) -> (MockServer, HeaderCapture) {
	let capture: HeaderCapture = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
	let server = mock_streamable_http_server_inner(stateful, Some(capture.clone())).await;
	(server, capture)
}

async fn mock_streamable_http_server_inner(
	stateful: bool,
	capture: Option<HeaderCapture>,
) -> MockServer {
	use mockserver::Counter;
	use rmcp::transport::streamable_http_server::StreamableHttpService;
	use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
	agent_core::telemetry::testing::setup_test_logging();
	let init_counter = std::sync::Arc::new(tokio::sync::Mutex::new(0_i32));

	let service = StreamableHttpService::new(
		{
			let init_counter = init_counter.clone();
			move || Ok(Counter::new(init_counter.clone()))
		},
		LocalSessionManager::default().into(),
		StreamableHttpServerConfig::default()
			.with_sse_retry(None)
			.with_sse_keep_alive(None)
			.with_stateful_mode(stateful)
			.with_json_response(false),
	);

	let (tx, rx) = tokio::sync::oneshot::channel();
	let mut router = axum::Router::new().nest_service("/mcp", service);
	if let Some(cap) = capture {
		router = router.layer(axum::middleware::from_fn(
			move |req: axum::extract::Request, next: axum::middleware::Next| {
				let cap = cap.clone();
				async move {
					cap.lock().unwrap().push(req.headers().clone());
					next.run(req).await
				}
			},
		));
	}
	let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = tcp_listener.local_addr().unwrap();
	tokio::spawn(async move {
		let _ = axum::serve(tcp_listener, router)
			.with_graceful_shutdown(async {
				let _ = rx.await;
			})
			.await;
		info!("server stopped");
	});
	MockServer {
		addr,
		init_counter,
		_cancel: tx,
	}
}

async fn mock_sse_server() -> MockServer {
	use legacy_rmcp::transport::sse_server::{SseServer, SseServerConfig};
	use tokio_util::sync::CancellationToken;

	agent_core::telemetry::testing::setup_test_logging();
	let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = tcp_listener.local_addr().unwrap();
	let ct = CancellationToken::new();
	let (sse_server, service) = SseServer::new(SseServerConfig {
		bind: addr,
		sse_path: "/sse".to_string(),
		post_path: "/message".to_string(),
		ct: ct.child_token(),
		sse_keep_alive: None,
	});

	let (tx, rx) = tokio::sync::oneshot::channel();
	let ct2 = sse_server.with_service_directly(legacymockserver::Counter::new);
	tokio::spawn(async move {
		let _ = axum::serve(tcp_listener, service)
			.with_graceful_shutdown(async move {
				rx.await.unwrap();
				ct.cancel();
				ct2.cancel();
				tracing::info!("sse server cancelled");
			})
			.await;
	});
	MockServer {
		addr,
		init_counter: std::sync::Arc::new(tokio::sync::Mutex::new(0)),
		_cancel: tx,
	}
}
mod mockserver {
	use std::sync::Arc;

	use http::request::Parts;
	use rmcp::handler::server::wrapper::Parameters;
	use rmcp::model::*;
	use rmcp::service::RequestContext;
	use rmcp::{
		ErrorData as McpError, RoleServer, ServerHandler, prompt, prompt_handler, prompt_router,
		schemars, tool, tool_handler, tool_router,
	};
	use serde_json::json;
	use tokio::sync::Mutex;

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct ExamplePromptArgs {
		/// A message to put in the prompt
		pub message: String,
	}

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct CounterAnalysisArgs {
		/// The target value you're trying to reach
		pub goal: i32,
		/// Preferred strategy: 'fast' or 'careful'
		#[serde(skip_serializing_if = "Option::is_none")]
		pub strategy: Option<String>,
	}

	#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
	pub struct StructRequest {
		pub a: i32,
		pub b: i32,
	}

	#[derive(Clone)]
	pub struct Counter {
		counter: Arc<Mutex<i32>>,
		init_counter: Arc<Mutex<i32>>,
	}

	#[tool_router]
	impl Counter {
		pub fn new(init_counter: Arc<Mutex<i32>>) -> Self {
			Self {
				counter: Arc::new(Mutex::new(0)),
				init_counter,
			}
		}

		fn _create_resource_text(&self, uri: &str, name: &str) -> Resource {
			RawResource::new(uri, name.to_string()).no_annotation()
		}

		#[tool(description = "Increment the counter by 1")]
		async fn increment(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter += 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Decrement the counter by 1")]
		async fn decrement(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter -= 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Get the current counter value")]
		async fn get_value(&self) -> Result<CallToolResult, McpError> {
			let counter = self.counter.lock().await;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Say hello to the client")]
		fn say_hello(&self) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text("hello")]))
		}

		#[tool(description = "Repeat what you say")]
		fn echo(&self, Parameters(object): Parameters<JsonObject>) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				serde_json::Value::Object(object).to_string(),
			)]))
		}

		#[tool(description = "Calculate the sum of two numbers")]
		fn sum(
			&self,
			Parameters(StructRequest { a, b }): Parameters<StructRequest>,
		) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				(a + b).to_string(),
			)]))
		}

		#[tool(description = "Echo HTTP attributes")]
		fn echo_http(&self, rq: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
			let ext = rq.extensions.get::<Parts>();
			Ok(CallToolResult::success(vec![Content::text(
				ext
					.unwrap()
					.headers
					.get("authorization")
					.map(|s| String::from_utf8_lossy(s.as_bytes()))
					.unwrap_or_default(),
			)]))
		}

		#[tool(description = "Get initialize call count")]
		async fn get_init_count(&self) -> Result<CallToolResult, McpError> {
			let init_counter = self.init_counter.lock().await;
			Ok(CallToolResult::success(vec![Content::text(
				init_counter.to_string(),
			)]))
		}
	}

	#[prompt_router]
	impl Counter {
		/// This is an example prompt that takes one required argument, message
		#[prompt(name = "example_prompt")]
		async fn example_prompt(
			&self,
			Parameters(args): Parameters<ExamplePromptArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<Vec<PromptMessage>, McpError> {
			let prompt = format!(
				"This is an example prompt with your message here: '{}'",
				args.message
			);
			Ok(vec![PromptMessage::new(
				PromptMessageRole::User,
				PromptMessageContent::text(prompt),
			)])
		}

		/// Analyze the current counter value and suggest next steps
		#[prompt(name = "counter_analysis")]
		async fn counter_analysis(
			&self,
			Parameters(args): Parameters<CounterAnalysisArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<GetPromptResult, McpError> {
			let strategy = args.strategy.unwrap_or_else(|| "careful".to_string());
			let current_value = *self.counter.lock().await;
			let difference = args.goal - current_value;

			let messages = vec![
				PromptMessage::new_text(
					PromptMessageRole::Assistant,
					"I'll analyze the counter situation and suggest the best approach.",
				),
				PromptMessage::new_text(
					PromptMessageRole::User,
					format!(
						"Current counter value: {}\nGoal value: {}\nDifference: {}\nStrategy preference: {}\n\nPlease analyze the situation and suggest the best approach to reach the goal.",
						current_value, args.goal, difference, strategy
					),
				),
			];

			Ok(GetPromptResult::new(messages).with_description(format!(
				"Counter analysis for reaching {} from {}",
				args.goal, current_value
			)))
		}
	}

	#[tool_handler]
	#[prompt_handler]
	impl ServerHandler for Counter {
		fn get_info(&self) -> ServerInfo {
			ServerInfo::new(
				ServerCapabilities::builder()
					.enable_prompts()
					.enable_resources()
					.enable_resources_subscribe()
					.enable_tools()
					.build(),
			)
			.with_protocol_version(ProtocolVersion::V_2025_06_18)
			.with_instructions("This server provides counter tools and prompts.")
		}

		async fn list_resources(
			&self,
			_request: Option<PaginatedRequestParams>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourcesResult, McpError> {
			Ok(ListResourcesResult {
				resources: vec![
					self._create_resource_text("str:////Users/to/some/path/", "cwd"),
					self._create_resource_text("memo://insights", "memo-name"),
				],
				next_cursor: None,
				meta: None,
			})
		}

		async fn read_resource(
			&self,
			ReadResourceRequestParams { uri, .. }: ReadResourceRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<ReadResourceResult, McpError> {
			match uri.as_str() {
				"str:////Users/to/some/path/" => {
					let cwd = "/Users/to/some/path/";
					Ok(ReadResourceResult::new(vec![ResourceContents::text(
						cwd, uri,
					)]))
				},
				"memo://insights" => {
					let memo = "Business Intelligence Memo\n\nAnalysis has revealed 5 key insights ...";
					Ok(ReadResourceResult::new(vec![ResourceContents::text(
						memo, uri,
					)]))
				},
				_ => Err(McpError::resource_not_found(
					"resource_not_found",
					Some(json!({
							"uri": uri
					})),
				)),
			}
		}

		async fn subscribe(
			&self,
			SubscribeRequestParams { uri, .. }: SubscribeRequestParams,
			ctx: RequestContext<RoleServer>,
		) -> Result<(), McpError> {
			match uri.as_str() {
				"str:////Users/to/some/path/" | "memo://insights" => {
					let peer = ctx.peer;
					let notify_uri = uri.clone();
					tokio::spawn(async move {
						let _ = peer
							.notify_resource_updated(ResourceUpdatedNotificationParam::new(notify_uri))
							.await;
					});
					Ok(())
				},
				_ => Err(McpError::resource_not_found(
					"resource_not_found",
					Some(json!({
							"uri": uri
					})),
				)),
			}
		}

		async fn unsubscribe(
			&self,
			UnsubscribeRequestParams { uri, .. }: UnsubscribeRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<(), McpError> {
			match uri.as_str() {
				"str:////Users/to/some/path/" | "memo://insights" => Ok(()),
				_ => Err(McpError::resource_not_found(
					"resource_not_found",
					Some(json!({
							"uri": uri
					})),
				)),
			}
		}

		async fn list_resource_templates(
			&self,
			_request: Option<PaginatedRequestParams>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourceTemplatesResult, McpError> {
			Ok(ListResourceTemplatesResult {
				next_cursor: None,
				resource_templates: Vec::new(),
				meta: None,
			})
		}

		async fn initialize(
			&self,
			_request: InitializeRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<InitializeResult, McpError> {
			let mut init_counter = self.init_counter.lock().await;
			*init_counter += 1;
			Ok(self.get_info())
		}
	}
}

mod legacymockserver {
	use std::sync::Arc;

	use http::request::Parts;
	use legacy_rmcp as rmcp;
	use rmcp::handler::server::router::prompt::PromptRouter;
	use rmcp::handler::server::router::tool::ToolRouter;
	use rmcp::handler::server::wrapper::Parameters;
	use rmcp::model::*;
	use rmcp::service::RequestContext;
	use rmcp::{
		ErrorData as McpError, RoleServer, ServerHandler, prompt, prompt_handler, prompt_router,
		schemars, tool, tool_handler, tool_router,
	};
	use serde_json::json;
	use tokio::sync::Mutex;

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct ExamplePromptArgs {
		/// A message to put in the prompt
		pub message: String,
	}

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct CounterAnalysisArgs {
		/// The target value you're trying to reach
		pub goal: i32,
		/// Preferred strategy: 'fast' or 'careful'
		#[serde(skip_serializing_if = "Option::is_none")]
		pub strategy: Option<String>,
	}

	#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
	pub struct StructRequest {
		pub a: i32,
		pub b: i32,
	}

	#[derive(Clone)]
	pub struct Counter {
		counter: Arc<Mutex<i32>>,
		tool_router: ToolRouter<Counter>,
		prompt_router: PromptRouter<Counter>,
	}

	#[tool_router]
	impl Counter {
		#[allow(dead_code)]
		pub fn new() -> Self {
			Self {
				counter: Arc::new(Mutex::new(0)),
				tool_router: Self::tool_router(),
				prompt_router: Self::prompt_router(),
			}
		}

		fn _create_resource_text(&self, uri: &str, name: &str) -> Resource {
			RawResource::new(uri, name.to_string()).no_annotation()
		}

		#[tool(description = "Increment the counter by 1")]
		async fn increment(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter += 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Decrement the counter by 1")]
		async fn decrement(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter -= 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Get the current counter value")]
		async fn get_value(&self) -> Result<CallToolResult, McpError> {
			let counter = self.counter.lock().await;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Say hello to the client")]
		fn say_hello(&self) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text("hello")]))
		}

		#[tool(description = "Repeat what you say")]
		fn echo(&self, Parameters(object): Parameters<JsonObject>) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				serde_json::Value::Object(object).to_string(),
			)]))
		}

		#[tool(description = "Calculate the sum of two numbers")]
		fn sum(
			&self,
			Parameters(StructRequest { a, b }): Parameters<StructRequest>,
		) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				(a + b).to_string(),
			)]))
		}

		#[tool(description = "Echo HTTP attributes")]
		fn echo_http(&self, rq: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
			let ext = rq.extensions.get::<Parts>();
			Ok(CallToolResult::success(vec![Content::text(
				ext
					.unwrap()
					.headers
					.get("authorization")
					.map(|s| String::from_utf8_lossy(s.as_bytes()))
					.unwrap_or_default(),
			)]))
		}
	}

	#[prompt_router]
	impl Counter {
		/// This is an example prompt that takes one required argument, message
		#[prompt(name = "example_prompt")]
		async fn example_prompt(
			&self,
			Parameters(args): Parameters<ExamplePromptArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<Vec<PromptMessage>, McpError> {
			let prompt = format!(
				"This is an example prompt with your message here: '{}'",
				args.message
			);
			Ok(vec![PromptMessage {
				role: PromptMessageRole::User,
				content: PromptMessageContent::text(prompt),
			}])
		}

		/// Analyze the current counter value and suggest next steps
		#[prompt(name = "counter_analysis")]
		async fn counter_analysis(
			&self,
			Parameters(args): Parameters<CounterAnalysisArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<GetPromptResult, McpError> {
			let strategy = args.strategy.unwrap_or_else(|| "careful".to_string());
			let current_value = *self.counter.lock().await;
			let difference = args.goal - current_value;

			let messages = vec![
				PromptMessage::new_text(
					PromptMessageRole::Assistant,
					"I'll analyze the counter situation and suggest the best approach.",
				),
				PromptMessage::new_text(
					PromptMessageRole::User,
					format!(
						"Current counter value: {}\nGoal value: {}\nDifference: {}\nStrategy preference: {}\n\nPlease analyze the situation and suggest the best approach to reach the goal.",
						current_value, args.goal, difference, strategy
					),
				),
			];

			Ok(GetPromptResult {
				description: Some(format!(
					"Counter analysis for reaching {} from {}",
					args.goal, current_value
				)),
				messages,
			})
		}
	}

	#[tool_handler]
	#[prompt_handler]
	impl ServerHandler for Counter {
		fn get_info(&self) -> ServerInfo {
			ServerInfo {
				protocol_version: ProtocolVersion::V_2025_06_18,
				capabilities: ServerCapabilities::builder()
					.enable_prompts()
					.enable_resources()
					.enable_tools()
					.build(),
				server_info: Implementation::from_build_env(),
				instructions: Some("This server provides counter tools and prompts.".to_string()),
			}
		}

		async fn list_resources(
			&self,
			_request: Option<PaginatedRequestParam>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourcesResult, McpError> {
			Ok(ListResourcesResult {
				resources: vec![
					self._create_resource_text("str:////Users/to/some/path/", "cwd"),
					self._create_resource_text("memo://insights", "memo-name"),
				],
				next_cursor: None,
			})
		}

		async fn read_resource(
			&self,
			ReadResourceRequestParam { uri }: ReadResourceRequestParam,
			_: RequestContext<RoleServer>,
		) -> Result<ReadResourceResult, McpError> {
			match uri.as_str() {
				"str:////Users/to/some/path/" => {
					let cwd = "/Users/to/some/path/";
					Ok(ReadResourceResult {
						contents: vec![ResourceContents::text(cwd, uri)],
					})
				},
				"memo://insights" => {
					let memo = "Business Intelligence Memo\n\nAnalysis has revealed 5 key insights ...";
					Ok(ReadResourceResult {
						contents: vec![ResourceContents::text(memo, uri)],
					})
				},
				_ => Err(McpError::resource_not_found(
					"resource_not_found",
					Some(json!({
							"uri": uri
					})),
				)),
			}
		}

		async fn list_resource_templates(
			&self,
			_request: Option<PaginatedRequestParam>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourceTemplatesResult, McpError> {
			Ok(ListResourceTemplatesResult {
				next_cursor: None,
				resource_templates: Vec::new(),
			})
		}

		async fn initialize(
			&self,
			_request: InitializeRequestParam,
			_: RequestContext<RoleServer>,
		) -> Result<InitializeResult, McpError> {
			Ok(self.get_info())
		}
	}
}

#[tokio::test]
async fn test_zero_targets_fail_closed() {
	let backend = McpBackendGroup {
		targets: vec![],
		..Default::default()
	};
	let client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi);
	let err = crate::mcp::upstream::UpstreamGroup::new(client, backend).unwrap_err();
	assert!(matches!(err, crate::mcp::Error::NoBackends));
}

#[tokio::test]
async fn test_zero_targets_fail_open() {
	let backend = McpBackendGroup {
		targets: vec![],
		failure_mode: FailureMode::FailOpen,
		..Default::default()
	};
	let client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi);
	crate::mcp::upstream::UpstreamGroup::new(client, backend).unwrap();
}

#[tokio::test]
async fn test_setup_partial_success_fail_open() {
	// Test skipping failed stdio targets
	let backend = McpBackendGroup {
		targets: vec![
			Arc::new(McpTarget {
				name: "bad".into(),
				spec: crate::types::agent::McpTargetSpec::Stdio {
					cmd: "this-binary-does-not-exist-agentgateway-test".into(),
					args: vec![],
					env: Default::default(),
					clear_env: false,
				},
				backend_policies: Default::default(),
				backend: None,
				always_use_prefix: false,
			}),
			Arc::new(McpTarget {
				name: "ok".into(),
				spec: crate::types::agent::McpTargetSpec::Stdio {
					cmd: "cat".into(),
					args: vec![],
					env: Default::default(),
					clear_env: false,
				},
				backend_policies: Default::default(),
				backend: None,
				always_use_prefix: false,
			}),
		],
		stateful: false,
		failure_mode: FailureMode::FailOpen,
		..Default::default()
	};
	let client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi);
	let group = crate::mcp::upstream::UpstreamGroup::new(client, backend).unwrap();
	assert_eq!(group.size(), 1);
}

#[tokio::test]
async fn test_all_targets_fail_open_still_errors() {
	let backend = McpBackendGroup {
		targets: vec![
			Arc::new(McpTarget {
				name: "bad-1".into(),
				spec: crate::types::agent::McpTargetSpec::Stdio {
					cmd: "this-binary-does-not-exist-agentgateway-test-1".into(),
					args: vec![],
					env: Default::default(),
					clear_env: false,
				},
				backend_policies: Default::default(),
				backend: None,
				always_use_prefix: false,
			}),
			Arc::new(McpTarget {
				name: "bad-2".into(),
				spec: crate::types::agent::McpTargetSpec::Stdio {
					cmd: "this-binary-does-not-exist-agentgateway-test-2".into(),
					args: vec![],
					env: Default::default(),
					clear_env: false,
				},
				backend_policies: Default::default(),
				backend: None,
				always_use_prefix: false,
			}),
		],
		stateful: false,
		failure_mode: FailureMode::FailOpen,
		..Default::default()
	};
	let client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi);
	let err = crate::mcp::upstream::UpstreamGroup::new(client, backend).unwrap_err();
	assert!(matches!(err, crate::mcp::Error::NoBackends));
}

fn fake_streamable_target(name: &str, addr: SocketAddr) -> Arc<McpTarget> {
	Arc::new(McpTarget {
		name: name.into(),
		spec: crate::types::agent::McpTargetSpec::Mcp(crate::types::agent::StreamableHTTPTargetSpec {
			backend: crate::types::agent::SimpleBackendReference::Backend(strng::format!(
				"/unused-{name}"
			)),
			path: "/mcp".to_string(),
		}),
		backend_policies: Default::default(),
		backend: Some(crate::types::agent::SimpleBackend::Opaque(
			crate::types::agent::ResourceName::new(strng::format!("backend-{name}"), "".into()),
			crate::types::agent::Target::Address(addr),
		)),
		always_use_prefix: false,
	})
}

fn fake_sse_target(name: &str, addr: SocketAddr) -> Arc<McpTarget> {
	Arc::new(McpTarget {
		name: name.into(),
		spec: crate::types::agent::McpTargetSpec::Sse(crate::types::agent::SseTargetSpec {
			backend: crate::types::agent::SimpleBackendReference::Backend(strng::format!(
				"/unused-{name}"
			)),
			path: "/sse".to_string(),
		}),
		backend_policies: Default::default(),
		backend: Some(crate::types::agent::SimpleBackend::Opaque(
			crate::types::agent::ResourceName::new(strng::format!("backend-{name}"), "".into()),
			crate::types::agent::Target::Address(addr),
		)),
		always_use_prefix: false,
	})
}

fn fake_openapi_target(name: &str, addr: SocketAddr) -> Arc<McpTarget> {
	let schema: OpenAPI = serde_json::from_value(serde_json::json!({
		"openapi": "3.0.0",
		"info": {
			"title": "Test API",
			"version": "1.0.0"
		},
		"paths": {}
	}))
	.expect("valid OpenAPI schema");

	Arc::new(McpTarget {
		name: name.into(),
		spec: crate::types::agent::McpTargetSpec::OpenAPI(crate::types::agent::OpenAPITarget {
			backend: crate::types::agent::SimpleBackendReference::Backend(strng::format!(
				"/unused-{name}"
			)),
			schema: Arc::new(schema),
		}),
		backend_policies: Default::default(),
		backend: Some(crate::types::agent::SimpleBackend::Opaque(
			crate::types::agent::ResourceName::new(strng::format!("backend-{name}"), "".into()),
			crate::types::agent::Target::Address(addr),
		)),
		always_use_prefix: false,
	})
}

fn fake_stdio_target(name: &str) -> Arc<McpTarget> {
	Arc::new(McpTarget {
		name: name.into(),
		spec: crate::types::agent::McpTargetSpec::Stdio {
			cmd: "cat".into(),
			args: vec![],
			env: Default::default(),
			clear_env: false,
		},
		backend_policies: Default::default(),
		backend: None,
		always_use_prefix: false,
	})
}

fn empty_mcp_policies() -> crate::mcp::McpAuthorizationSet {
	crate::mcp::McpAuthorizationSet::new(crate::http::authorization::RuleSets::from(Vec::new()))
}

fn empty_cel() -> crate::mcp::rbac::CelExecWrapper {
	crate::mcp::rbac::CelExecWrapper::new(::http::Request::new(()))
}

fn persisted_session(
	target_name: &str,
	session: &str,
	backend: SocketAddr,
) -> http::sessionpersistence::MCPSession {
	http::sessionpersistence::MCPSession {
		target_name: Some(target_name.to_string()),
		session: Some(session.to_string()),
		backend: Some(backend),
	}
}

fn persisted_stateless_session(
	target_name: &str,
	backend: SocketAddr,
) -> http::sessionpersistence::MCPSession {
	http::sessionpersistence::MCPSession {
		target_name: Some(target_name.to_string()),
		session: None,
		backend: Some(backend),
	}
}

#[test]
fn test_openapi_targets_emit_stateless_session_state() {
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![fake_openapi_target(
				"openapi",
				SocketAddr::from(([127, 0, 0, 1], 30031)),
			)],
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	let sessions = relay
		.get_sessions()
		.expect("OpenAPI should support stateless sessions");
	assert_eq!(
		sessions,
		vec![MCPSession {
			target_name: Some("openapi".to_string()),
			session: None,
			backend: None,
		}]
	);

	let pinned = SocketAddr::from(([127, 0, 0, 1], 31031));
	relay
		.set_sessions(vec![persisted_stateless_session("openapi", pinned)])
		.unwrap();

	let sessions = relay
		.get_sessions()
		.expect("OpenAPI session state should still be available");
	assert_eq!(
		sessions,
		vec![MCPSession {
			target_name: Some("openapi".to_string()),
			session: None,
			backend: Some(pinned),
		}]
	);
}

#[test]
fn test_sse_targets_emit_stateless_session_state() {
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![fake_sse_target(
				"sse",
				SocketAddr::from(([127, 0, 0, 1], 30032)),
			)],
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	let sessions = relay
		.get_sessions()
		.expect("SSE should support stateless sessions");
	assert_eq!(
		sessions,
		vec![MCPSession {
			target_name: Some("sse".to_string()),
			session: None,
			backend: None,
		}]
	);

	let pinned = SocketAddr::from(([127, 0, 0, 1], 31032));
	relay
		.set_sessions(vec![persisted_stateless_session("sse", pinned)])
		.unwrap();

	let sessions = relay
		.get_sessions()
		.expect("SSE session state should still be available");
	assert_eq!(
		sessions,
		vec![MCPSession {
			target_name: Some("sse".to_string()),
			session: None,
			backend: Some(pinned),
		}]
	);
}

#[tokio::test]
async fn test_stdio_targets_remain_non_stateless() {
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![fake_stdio_target("stdio")],
			stateful: false,
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	assert!(relay.get_sessions().is_none());
}

#[tokio::test]
async fn test_fanout_deletion_fail_open_skips_failed_upstreams() {
	let good = mock_streamable_http_server(true).await;
	let bad_addr = SocketAddr::from(([127, 0, 0, 1], 31999));
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				fake_streamable_target("good", good.addr),
				fake_streamable_target("bad", bad_addr),
			],
			stateful: true,
			failure_mode: FailureMode::FailOpen,
			session_idle_ttl: crate::mcp::DEFAULT_SESSION_IDLE_TTL,
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	relay
		.set_sessions(vec![
			persisted_session("good", "session-good", good.addr),
			persisted_session("bad", "session-bad", bad_addr),
		])
		.unwrap();

	let response = relay
		.send_fanout_deletion(crate::mcp::upstream::IncomingRequestContext::empty())
		.await
		.unwrap();

	assert_eq!(response.status(), http::StatusCode::ACCEPTED);
}

#[test]
fn test_set_sessions_matches_by_target_name() {
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				fake_streamable_target("alpha", SocketAddr::from(([127, 0, 0, 1], 30001))),
				fake_streamable_target("beta", SocketAddr::from(([127, 0, 0, 1], 30002))),
			],
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	relay
		.set_sessions(vec![
			persisted_session(
				"beta",
				"session-beta",
				SocketAddr::from(([127, 0, 0, 1], 31002)),
			),
			persisted_session(
				"alpha",
				"session-alpha",
				SocketAddr::from(([127, 0, 0, 1], 31001)),
			),
		])
		.unwrap();

	let sessions = relay.get_sessions().unwrap();
	assert_eq!(sessions.len(), 2);
	assert_eq!(sessions[0].target_name.as_deref(), Some("alpha"));
	assert_eq!(sessions[0].session.as_deref(), Some("session-alpha"));
	assert_eq!(
		sessions[0].backend,
		Some(SocketAddr::from(([127, 0, 0, 1], 31001)))
	);
	assert_eq!(sessions[1].target_name.as_deref(), Some("beta"));
	assert_eq!(sessions[1].session.as_deref(), Some("session-beta"));
	assert_eq!(
		sessions[1].backend,
		Some(SocketAddr::from(([127, 0, 0, 1], 31002)))
	);
}

#[test]
fn test_set_sessions_rejects_mismatched_target_set() {
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				fake_streamable_target("alpha", SocketAddr::from(([127, 0, 0, 1], 30011))),
				fake_streamable_target("beta", SocketAddr::from(([127, 0, 0, 1], 30012))),
			],
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	let err = relay
		.set_sessions(vec![
			persisted_session(
				"beta",
				"session-beta",
				SocketAddr::from(([127, 0, 0, 1], 32012)),
			),
			persisted_session(
				"gamma",
				"session-gamma",
				SocketAddr::from(([127, 0, 0, 1], 32013)),
			),
		])
		.unwrap_err();

	assert!(
		err
			.to_string()
			.contains("missing persisted session for target alpha")
	);
}

#[test]
fn test_merge_initialize_merges_upstream_instructions_when_multiplexing() {
	use rmcp::model::{
		Implementation, InitializeResult, ProtocolVersion, ServerCapabilities, ServerResult,
	};

	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				fake_streamable_target("alpha", SocketAddr::from(([127, 0, 0, 1], 30101))),
				fake_streamable_target("beta", SocketAddr::from(([127, 0, 0, 1], 30102))),
			],
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	let merge_fn = relay.merge_initialize(ProtocolVersion::V_2025_06_18, true);

	let results: Vec<(Strng, ServerResult)> = vec![
		(
			"alpha".into(),
			ServerResult::InitializeResult(
				InitializeResult::new(ServerCapabilities::default())
					.with_protocol_version(ProtocolVersion::V_2025_06_18)
					.with_server_info(Implementation::new("alpha-server", "1.0"))
					.with_instructions("Alpha server: handles data processing."),
			),
		),
		(
			"beta".into(),
			ServerResult::InitializeResult(
				InitializeResult::new(ServerCapabilities::default())
					.with_protocol_version(ProtocolVersion::V_2025_06_18)
					.with_server_info(Implementation::new("beta-server", "1.0"))
					.with_instructions("Beta server: handles notifications."),
			),
		),
	];

	let result = merge_fn(results, &empty_cel()).unwrap();
	let info = match result {
		ServerResult::InitializeResult(ir) => ir,
		other => panic!("expected InitializeResult, got: {:?}", other),
	};

	let instructions = info.instructions.expect("instructions should be present");
	assert!(
		instructions.contains("Alpha server: handles data processing."),
		"merged instructions should contain alpha's instructions, got: {instructions}"
	);
	assert!(
		instructions.contains("Beta server: handles notifications."),
		"merged instructions should contain beta's instructions, got: {instructions}"
	);
	assert!(
		instructions.contains("[alpha]"),
		"merged instructions should label alpha's section, got: {instructions}"
	);
	assert!(
		instructions.contains("[beta]"),
		"merged instructions should label beta's section, got: {instructions}"
	);
	assert!(
		instructions.contains("gateway"),
		"merged instructions should contain gateway preamble, got: {instructions}"
	);
}

#[test]
fn test_merge_initialize_no_instructions_when_multiplexing() {
	use rmcp::model::{
		Implementation, InitializeResult, ProtocolVersion, ServerCapabilities, ServerResult,
	};

	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![fake_streamable_target(
				"alpha",
				SocketAddr::from(([127, 0, 0, 1], 30103)),
			)],
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	let merge_fn = relay.merge_initialize(ProtocolVersion::V_2025_06_18, true);

	let results: Vec<(Strng, ServerResult)> = vec![(
		"alpha".into(),
		ServerResult::InitializeResult(
			InitializeResult::new(ServerCapabilities::default())
				.with_protocol_version(ProtocolVersion::V_2025_06_18)
				.with_server_info(Implementation::new("alpha-server", "1.0")),
		),
	)];

	let result = merge_fn(results, &empty_cel()).unwrap();
	let info = match result {
		ServerResult::InitializeResult(ir) => ir,
		other => panic!("expected InitializeResult, got: {:?}", other),
	};

	let instructions = info.instructions.expect("instructions should be present");
	// When no upstream provides instructions, only the gateway preamble should be present
	assert!(
		instructions.contains("gateway"),
		"should contain gateway preamble, got: {instructions}"
	);
	assert!(
		!instructions.contains("[alpha]"),
		"should not contain server sections when no instructions provided, got: {instructions}"
	);
}

#[test]
fn test_merge_initialize_forwards_single_backend_without_multiplexing() {
	use rmcp::model::{
		Implementation, InitializeResult, ProtocolVersion, ServerCapabilities, ServerResult,
	};

	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![fake_streamable_target(
				"solo",
				SocketAddr::from(([127, 0, 0, 1], 30104)),
			)],
			..Default::default()
		},
		empty_mcp_policies(),
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
	)
	.unwrap();

	let merge_fn = relay.merge_initialize(ProtocolVersion::V_2025_06_18, false);

	let results: Vec<(Strng, ServerResult)> = vec![(
		"solo".into(),
		ServerResult::InitializeResult(
			InitializeResult::new(ServerCapabilities::default())
				.with_protocol_version(ProtocolVersion::V_2025_06_18)
				.with_server_info(Implementation::new("solo-server", "1.0"))
				.with_instructions("Solo server instructions."),
		),
	)];

	let result = merge_fn(results, &empty_cel()).unwrap();
	let info = match result {
		ServerResult::InitializeResult(ir) => ir,
		other => panic!("expected InitializeResult, got: {:?}", other),
	};

	// Non-multiplexing should forward the upstream's instructions directly
	assert_eq!(
		info.instructions.as_deref(),
		Some("Solo server instructions."),
		"non-multiplexing should forward upstream instructions unchanged"
	);
	assert_eq!(info.server_info.name, "solo-server");
}

#[tokio::test]
async fn test_runtime_fanout_fail_open() {
	use futures_util::StreamExt;
	use rmcp::model::{ListToolsResult, RequestId, ServerJsonRpcMessage};

	use crate::mcp::mergestream::{MergeStream, Messages};

	let ok_msg = ServerJsonRpcMessage::response(
		rmcp::model::ServerResult::ListToolsResult(ListToolsResult {
			tools: vec![],
			next_cursor: None,
			meta: None,
		}),
		RequestId::Number(1),
	);
	let ok_stream = Messages::from(ok_msg);
	let err_stream = Messages::from(Err(crate::mcp::ClientError::new(anyhow::anyhow!(
		"bad upstream"
	))));

	let streams = vec![("ok".into(), ok_stream), ("bad".into(), err_stream)];

	let merge = Box::new(
		|results: Vec<(Strng, rmcp::model::ServerResult)>, _cel: &_| {
			// Just return the first one for simplicity in this test
			Ok(results.into_iter().next().unwrap().1)
		},
	);

	let mut ms = MergeStream::new(
		streams,
		RequestId::Number(1),
		merge,
		empty_cel(),
		FailureMode::FailOpen,
	);

	let res = ms.next().await;
	assert!(res.is_some());
	let res = res.unwrap();
	assert!(
		res.is_ok(),
		"expected success with FailOpen even if one upstream errors: {:?}",
		res.err()
	);
}

#[tokio::test]
async fn test_runtime_fanout_fail_open_all_fail() {
	use futures_util::StreamExt;
	use rmcp::model::{ListToolsResult, RequestId};

	use crate::mcp::mergestream::{MergeStream, Messages};

	let err_stream1 = Messages::from(Err(crate::mcp::ClientError::new(anyhow::anyhow!("bad 1"))));
	let err_stream2 = Messages::from(Err(crate::mcp::ClientError::new(anyhow::anyhow!("bad 2"))));

	let streams = vec![("bad1".into(), err_stream1), ("bad2".into(), err_stream2)];

	let merge = Box::new(
		|results: Vec<(Strng, rmcp::model::ServerResult)>, _cel: &_| {
			// All failed, so results should be empty.
			// Return an empty success result (idiomatic for FailOpen).
			assert!(results.is_empty());
			Ok(rmcp::model::ServerResult::ListToolsResult(
				ListToolsResult {
					tools: vec![],
					next_cursor: None,
					meta: None,
				},
			))
		},
	);

	let mut ms = MergeStream::new(
		streams,
		RequestId::Number(1),
		merge,
		empty_cel(),
		FailureMode::FailOpen,
	);

	let res = ms.next().await;
	assert!(res.is_some());
	let res = res.unwrap();
	assert!(
		res.is_ok(),
		"expected success with FailOpen even if ALL upstreams error mid-request: {:?}",
		res.err()
	);
}

#[tokio::test]
async fn mcp_local_ratelimit() {
	let mock = mock_streamable_http_server(true).await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend(mock.addr, true, false)
		.with_bind(simple_bind())
		.with_route(basic_route(mock.addr));

	// Attach local rate limit policy
	// MCP protocol overhead: initialize + notification + SSE GET = 3 requests
	// Allow 5 total: overhead (3) + tool calls (2), then rate limit the 6th
	t.attach_route_policy(serde_json::json!({
		"localRateLimit": [{
			"maxTokens": 5,
			"tokensPerFill": 1,
			"fillInterval": "10s",
			"type": "requests"
		}]
	}))
	.await;

	let io = t.serve_real_listener(BIND_KEY).await;
	let client = mcp_streamable_client(io).await;

	// First two calls should succeed
	let result1 = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo")
				.with_arguments(serde_json::json!({"n": 1}).as_object().cloned().unwrap()),
		)
		.await;
	assert!(result1.is_ok(), "First request should succeed");

	let result2 = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo")
				.with_arguments(serde_json::json!({"n": 2}).as_object().cloned().unwrap()),
		)
		.await;
	assert!(result2.is_ok(), "Second request should succeed");

	// Third call should be rate limited
	let result3 = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo")
				.with_arguments(serde_json::json!({"n": 3}).as_object().cloned().unwrap()),
		)
		.await;
	assert!(result3.is_err(), "Third request should be rate limited");
}

#[tokio::test]
async fn mcp_extauth_deny() {
	struct DenyAllAuthz;

	#[async_trait::async_trait]
	impl crate::test_helpers::extauthmock::Handler for DenyAllAuthz {
		async fn check(
			&mut self,
			_request: &crate::http::ext_authz::proto::CheckRequest,
		) -> Result<crate::http::ext_authz::proto::CheckResponse, tonic::Status> {
			deny_response(
				crate::http::ext_authz::proto::StatusCode::Forbidden,
				"denied by mock ext_authz",
			)
		}
	}

	let authz = ExtAuthMock::new(|| DenyAllAuthz).spawn().await;

	let mock = mock_streamable_http_server(true).await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend(mock.addr, true, false)
		.with_bind(simple_bind())
		.with_route(basic_route(mock.addr));

	// Attach extAuthz policy pointing to our mock server
	t.attach_route_policy(serde_json::json!({
		"extAuthz": {
			"host": authz.address.to_string(),
			"protocol": {
				"grpc": {}
			}
		}
	}))
	.await;

	let io = t.serve_real_listener(BIND_KEY).await;

	// Client should fail to initialize due to ext_authz denial
	let result = try_mcp_streamable_client(io).await;
	let err = result.expect_err("Client initialization should be denied by ext_authz");
	let err_msg = err.to_string();
	assert!(
		err_msg.contains("403") && err_msg.contains("denied by mock ext_authz"),
		"Expected 403 denial from ext_authz, got: {err_msg}"
	);
}

async fn try_mcp_streamable_client(
	s: SocketAddr,
) -> Result<RunningService<RoleClient, InitializeRequestParams>, rmcp::service::ClientInitializeError>
{
	use rmcp::ServiceExt;
	use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use rmcp::transport::StreamableHttpClientTransport;
	let transport =
		StreamableHttpClientTransport::<reqwest::Client>::from_uri(format!("http://{s}/mcp"));
	let client_info = ClientInfo::new(
		ClientCapabilities::default(),
		Implementation::new("test client".to_string(), "0.0.1".to_string()),
	);

	Box::pin(client_info.serve(transport)).await
}

#[tokio::test]
async fn mcp_remote_ratelimit_deny() {
	struct DenyAllRateLimit;

	#[async_trait::async_trait]
	impl crate::test_helpers::ratelimitmock::Handler for DenyAllRateLimit {
		async fn should_rate_limit(
			&mut self,
			_request: &crate::http::remoteratelimit::proto::RateLimitRequest,
		) -> Result<crate::http::remoteratelimit::proto::RateLimitResponse, tonic::Status> {
			over_limit_response(b"rate limit exceeded by mock".to_vec())
		}
	}

	let ratelimit = RateLimitMock::new(|| DenyAllRateLimit).spawn().await;

	let mock = mock_streamable_http_server(true).await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend(mock.addr, true, false)
		.with_bind(simple_bind())
		.with_route(basic_route(mock.addr));

	// Attach remoteRateLimit policy pointing to our mock server
	t.attach_route_policy(serde_json::json!({
		"remoteRateLimit": {
			"host": ratelimit.address.to_string(),
			"domain": "test",
			"descriptors": [{
				"entries": [
					{"key": "generic_key", "value": "\"test\""}
				],
				"type": "requests"
			}]
		}
	}))
	.await;

	let io = t.serve_real_listener(BIND_KEY).await;

	// Client should fail to initialize due to rate limit denial
	let result = try_mcp_streamable_client(io).await;
	let err = result.expect_err("Client initialization should be rate limited");
	let err_msg = err.to_string();
	assert!(
		err_msg.contains("429") && err_msg.contains("rate limit exceeded by mock"),
		"Expected 429 rate limit from remote service, got: {err_msg}"
	);
}

// =========================== mcpGuardrails test helpers ============================

mod guardrails_test_support {
	use std::collections::HashMap;
	use std::net::SocketAddr;
	use std::sync::Arc;

	use rmcp::model::{CallToolResult, RawContent};

	use crate::mcp::guardrails;
	use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference, Target};

	// Default test allowlist: every method exercised in this module's tests,
	// all at Phase::Full. Tests that need narrower coverage build their own.
	pub fn default_methods() -> HashMap<String, guardrails::Phase> {
		["tools/call", "tools/list", "prompts/get", "resources/read"]
			.into_iter()
			.map(|m| (m.to_string(), guardrails::Phase::Full))
			.collect()
	}

	pub fn policy(addr: SocketAddr) -> BackendTrafficPolicy {
		policy_with(
			addr,
			guardrails::FailureMode::FailClosed,
			default_methods(),
			HashMap::new(),
		)
	}

	pub fn policy_with(
		addr: SocketAddr,
		failure_mode: guardrails::FailureMode,
		methods: HashMap<String, guardrails::Phase>,
		metadata: HashMap<String, Arc<crate::cel::Expression>>,
	) -> BackendTrafficPolicy {
		let remote = guardrails::Remote {
			target: Arc::new(SimpleBackendReference::InlineBackend(Target::Address(addr))),
			policies: Vec::new(),
			failure_mode,
			metadata,
			request_headers: Default::default(),
		};
		BackendTrafficPolicy::McpGuardrails(Arc::new(guardrails::McpGuardrails {
			processors: vec![guardrails::Processor {
				methods,
				kind: guardrails::ProcessorKind::Remote(remote),
			}],
		}))
	}

	pub fn echo_text(r: &CallToolResult) -> String {
		r.content
			.iter()
			.find_map(|c| match c.raw {
				RawContent::Text(ref t) => Some(t.text.clone()),
				_ => None,
			})
			.expect("echo returned text")
	}
}

// ============================== mcpGuardrails tests ===============================

#[tokio::test]
async fn mcp_guardrails_pass_through() {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use crate::test_helpers::extmcpmock::{closure_mock, pass_request, pass_response};

	let (req_n, resp_n) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
	let extmcp_mock = {
		let (r, p) = (req_n.clone(), resp_n.clone());
		closure_mock(
			move |_| {
				r.fetch_add(1, Ordering::SeqCst);
				pass_request()
			},
			move |_| {
				p.fetch_add(1, Ordering::SeqCst);
				pass_response()
			},
		)
		.spawn()
		.await
	};

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let result = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("tool call should succeed when mcpGuardrails returns Pass");

	assert!(!result.content.is_empty());
	assert!(req_n.load(Ordering::SeqCst) >= 1);
	assert!(resp_n.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn mcp_guardrails_reject_surfaces_jsonrpc_error() {
	use protos::ext_mcp::authorization_error::Code;

	use crate::test_helpers::extmcpmock::{closure_mock, pass_response, reject_request};

	let extmcp_mock = closure_mock(
		|_| reject_request(Code::PermissionDenied, "denied by mock mcpGuardrails"),
		|_| pass_response(),
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect_err("tool call should fail when mcpGuardrails rejects");

	let rmcp::ServiceError::McpError(e) = &err else {
		panic!("expected McpError, got {err:?}");
	};
	assert_eq!(e.code.0, -32001, "PermissionDenied should map to -32001");
	assert_eq!(e.message.as_ref(), "denied by mock mcpGuardrails");
}

#[tokio::test]
async fn mcp_guardrails_denies_tool_by_name() {
	use protos::ext_mcp::authorization_error::Code;

	use crate::test_helpers::extmcpmock::{
		closure_mock, pass_request, pass_response, reject_request,
	};

	let extmcp_mock = closure_mock(
		|req| {
			let name = req
				.mcp_request
				.as_deref()
				.and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
				.and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_owned))
				.unwrap_or_default();
			if name.contains("forbidden") {
				reject_request(Code::PermissionDenied, format!("tool {name} is forbidden"))
			} else {
				pass_request()
			}
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;

	// Forbidden tool is rejected at the request phase, before reaching upstream.
	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("forbidden-tool")
				.with_arguments(serde_json::Map::new()),
		)
		.await
		.expect_err("forbidden tool call should be denied by mcpGuardrails");
	let rmcp::ServiceError::McpError(e) = &err else {
		panic!("expected McpError, got {err:?}");
	};
	assert_eq!(e.code.0, -32001, "PermissionDenied should map to -32001");
	assert!(
		e.message.contains("forbidden-tool"),
		"deny message should name the tool: {}",
		e.message
	);

	// An allowed tool passes the request phase through to the upstream.
	let result = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("allowed tool call should pass through mcpGuardrails");
	assert!(!result.content.is_empty(), "echo should return content");
}

#[tokio::test]
async fn mcp_guardrails_mutated_request_reaches_upstream() {
	use crate::test_helpers::extmcpmock::{
		closure_mock, mutated_request_json, pass_request, pass_response,
	};

	let extmcp_mock = closure_mock(
		|req| {
			if req.method != "tools/call" {
				return pass_request();
			}
			mutated_request_json(serde_json::json!({
				"name": "echo",
				"arguments": { "rewritten": true, "limit": 10, "ratio": 2.5 },
			}))
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let result = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("tool call should succeed");

	// echo serializes its arguments verbatim; rewritten args ⇒ rewrite reached upstream.
	let text = guardrails_test_support::echo_text(&result);
	assert!(text.contains("rewritten") && text.contains("true"));
	assert!(!text.contains("\"hi\""));
	// Mutated numbers keep their JSON form: integers don't become 10.0.
	assert!(text.contains("\"limit\":10,"), "got: {text}");
	assert!(text.contains("\"ratio\":2.5"), "got: {text}");
}

#[tokio::test]
async fn mcp_guardrails_metadata_cel_evaluated_per_request() {
	use std::collections::HashMap;
	use std::sync::Mutex as StdMutex;

	use crate::cel::Expression;
	use crate::test_helpers::extmcpmock::{closure_mock, pass_request, pass_response};

	let captured: Arc<StdMutex<Option<prost_wkt_types::Struct>>> = Arc::new(StdMutex::new(None));
	let extmcp_mock = {
		let store = captured.clone();
		closure_mock(
			move |req| {
				if req.method == "tools/call"
					&& let Some(md) = req.metadata_context.as_ref()
				{
					*store.lock().unwrap() = Some(md.clone());
				}
				pass_request()
			},
			|_| pass_response(),
		)
		.spawn()
		.await
	};

	let mut metadata = HashMap::new();
	metadata.insert(
		"tenant.io".to_string(),
		Arc::new(Expression::new_strict(r#"{"path": request.path}"#).unwrap()),
	);
	let policy = guardrails_test_support::policy_with(
		extmcp_mock.address,
		guardrails::FailureMode::FailClosed,
		guardrails_test_support::default_methods(),
		metadata,
	);

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(&mock, true, false, vec![policy]).await;
	let client = mcp_streamable_client(io).await;
	let _ = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("call should succeed");

	let md = captured.lock().unwrap().clone().expect("metadata captured");
	let entry = md.fields.get("tenant.io").expect("tenant.io key present");
	assert_eq!(
		serde_json::to_value(entry).unwrap(),
		serde_json::json!({"path": "/mcp"}),
	);
}

// mcpGuardrails returns metadata in its request result; an MCP authorization rule then
// denies a tool based on that metadata (the inbound metadata -> CEL authz path).
#[tokio::test]
async fn mcp_guardrails_metadata_consumed_by_authz() {
	use crate::test_helpers::extmcpmock::{closure_mock, pass_request_with, pass_response};

	let extmcp_mock = closure_mock(
		|_| {
			pass_request_with(
				Vec::<(String, String)>::new(),
				Vec::<String>::new(),
				Some(serde_json::from_value(serde_json::json!({"tier": "free"})).unwrap()),
			)
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let deny_free_tier = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],
		vec![Arc::new(
			cel::Expression::new_strict(r#"mcp.tool.name == "echo" && mcpGuardrails.tier == "free""#)
				.unwrap(),
		)],
		vec![],
	)));

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![
			BackendTrafficPolicy::McpAuthorization(deny_free_tier),
			guardrails_test_support::policy(extmcp_mock.address),
		],
	)
	.await;
	let client = mcp_streamable_client(io).await;

	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(serde_json::Map::new()),
		)
		.await
		.expect_err("echo should be denied when mcpGuardrails marks the caller free-tier");
	let rmcp::ServiceError::McpError(e) = &err else {
		panic!("expected McpError, got {err:?}");
	};
	assert_eq!(e.code.0, -32602, "authz denial maps to INVALID_PARAMS");
	assert_eq!(e.message.as_ref(), "Unknown tool: echo");
}

// Simiilar to mcp_guardrails_metadata_consumed_by_authz but for the fanout path.
#[tokio::test]
async fn mcp_guardrails_metadata_consumed_by_list_authz() {
	use crate::test_helpers::extmcpmock::{closure_mock, pass_request_with, pass_response};

	let extmcp_mock = closure_mock(
		|_| {
			pass_request_with(
				Vec::<(String, String)>::new(),
				Vec::<String>::new(),
				Some(serde_json::from_value(serde_json::json!({"tier": "free"})).unwrap()),
			)
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let deny_free_tier = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],
		vec![Arc::new(
			cel::Expression::new_strict(r#"mcp.tool.name == "echo" && mcpGuardrails.tier == "free""#)
				.unwrap(),
		)],
		vec![],
	)));

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![
			BackendTrafficPolicy::McpAuthorization(deny_free_tier),
			guardrails_test_support::policy(extmcp_mock.address),
		],
	)
	.await;
	let client = mcp_streamable_client(io).await;

	let tool_names: Vec<String> = client
		.list_tools(None)
		.await
		.expect("list_tools should succeed")
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.collect();

	// `echo` is filtered only because list authz saw mcpGuardrails's `tier=free` metadata;
	// without it the deny rule would not match and `echo` would remain.
	assert!(
		!tool_names.contains(&"echo".to_string()),
		"echo should be filtered when mcpGuardrails marks the caller free-tier: {tool_names:?}"
	);
	assert!(
		tool_names.contains(&"increment".to_string()),
		"non-denied tools should remain: {tool_names:?}"
	);
}

#[tokio::test]
async fn mcp_guardrails_filtered_list_via_response_mutation() {
	use crate::test_helpers::extmcpmock::{
		closure_mock, mutated_response_json, pass_request, pass_response,
	};

	let extmcp_mock = closure_mock(
		|_| pass_request(),
		|req| {
			if req.method != "tools/list" {
				return pass_response();
			}
			mutated_response_json(serde_json::json!({
				"tools": [{
					"name": "echo",
					"description": "Repeat what you say",
					"inputSchema": { "type": "object" },
				}],
			}))
		},
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let tools = client
		.list_tools(None)
		.await
		.expect("list_tools should succeed");
	let names: Vec<String> = tools.tools.iter().map(|t| t.name.to_string()).collect();
	assert_eq!(names, vec!["echo".to_string()]);
}

// Fanout (multi-backend) runs the response hook ONCE on the merged, muxed result
// rather than once per upstream: the processor sees a single checkResponse carrying
// the prefixed (`a_echo`, `b_echo`) tools and every backend in service_names.
#[tokio::test]
async fn mcp_guardrails_fanout_runs_once_on_merged_muxed_result() {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::{Arc, Mutex};

	use protos::ext_mcp::McpResponse;

	use crate::test_helpers::extmcpmock::{closure_mock, mutated_response_json, pass_request};

	type Captured = Arc<Mutex<Option<(Vec<String>, Vec<String>)>>>;

	let resp_count = Arc::new(AtomicUsize::new(0));
	let captured: Captured = Arc::new(Mutex::new(None));
	let rc = resp_count.clone();
	let cap = captured.clone();
	let extmcp_mock = closure_mock(
		move |_| pass_request(),
		move |req: &McpResponse| {
			if req.method != "tools/list" {
				return crate::test_helpers::extmcpmock::pass_response();
			}
			rc.fetch_add(1, Ordering::SeqCst);
			let names: Vec<String> = serde_json::from_slice::<serde_json::Value>(&req.mcp_response)
				.ok()
				.and_then(|v| {
					v.get("tools").and_then(|t| t.as_array()).map(|arr| {
						arr
							.iter()
							.filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
							.collect()
					})
				})
				.unwrap_or_default();
			*cap.lock().unwrap() = Some((req.service_names.clone(), names));
			// Mutating the merged list proves the hook operates on the aggregate.
			mutated_response_json(serde_json::json!({
				"tools": [{
					"name": "a_echo",
					"description": "Repeat what you say",
					"inputSchema": { "type": "object" },
				}],
			}))
		},
	)
	.spawn()
	.await;

	let mock_a = mock_streamable_http_server(true).await;
	let mock_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend_policies(
			"mcp",
			vec![("a", mock_a.addr, false), ("b", mock_b.addr, false)],
			true,
			vec![guardrails_test_support::policy(extmcp_mock.address)],
		)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::new("/mcp")));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let tools = client
		.list_tools(None)
		.await
		.expect("list_tools should succeed");
	let names: Vec<String> = tools.tools.iter().map(|t| t.name.to_string()).collect();

	// One RPC for the whole fanout, not one per backend.
	assert_eq!(
		resp_count.load(Ordering::SeqCst),
		1,
		"response hook should run exactly once for the merged fanout result"
	);

	let (service_names, seen) = captured
		.lock()
		.unwrap()
		.clone()
		.expect("processor saw tools/list");
	// Aggregate identity = every fanned-out backend.
	assert_eq!(service_names, vec!["a".to_string(), "b".to_string()]);
	// The processor saw the merged, muxed list (prefixed names from both backends).
	assert!(
		seen.iter().any(|n| n == "a_echo") && seen.iter().any(|n| n == "b_echo"),
		"processor should see muxed names from both backends, got: {seen:?}"
	);

	// The mutation on the merged result is what reaches the client.
	assert_eq!(names, vec!["a_echo".to_string()]);
}

// A mutated `tools/call` result must round-trip back through `ServerResult` and
// reach the client (the `*/list` case above exercises a different variant).
#[tokio::test]
async fn mcp_guardrails_mutated_tool_call_response_reaches_client() {
	use crate::test_helpers::extmcpmock::{
		closure_mock, mutated_response_json, pass_request, pass_response,
	};

	let extmcp_mock = closure_mock(
		|_| pass_request(),
		|req| {
			if req.method != "tools/call" {
				return pass_response();
			}
			mutated_response_json(serde_json::json!({
				"content": [{ "type": "text", "text": "scrubbed-by-guardrails" }],
			}))
		},
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let result = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("tool call should succeed");

	let text = guardrails_test_support::echo_text(&result);
	assert_eq!(text, "scrubbed-by-guardrails");
	assert!(!text.contains("world"));
}

#[tokio::test]
async fn mcp_guardrails_fail_open_on_grpc_error() {
	use std::collections::HashMap;

	use crate::test_helpers::extmcpmock::{closure_mock, pass_response};

	let extmcp_mock = closure_mock(
		|_| Err(tonic::Status::internal("simulated mcpGuardrails failure")),
		|_| pass_response(),
	)
	.spawn()
	.await;

	let policy = guardrails_test_support::policy_with(
		extmcp_mock.address,
		guardrails::FailureMode::FailOpen,
		guardrails_test_support::default_methods(),
		HashMap::new(),
	);
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(&mock, true, false, vec![policy]).await;
	let client = mcp_streamable_client(io).await;
	let result = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("tool call should succeed under failure_mode=Allow");

	let text = guardrails_test_support::echo_text(&result);
	assert!(text.contains("\"hi\"") && text.contains("\"world\""));
}

#[tokio::test]
async fn mcp_guardrails_fail_closed_on_grpc_error() {
	use std::collections::HashMap;

	use crate::test_helpers::extmcpmock::{closure_mock, pass_response};

	let extmcp_mock = closure_mock(
		|_| Err(tonic::Status::internal("simulated mcpGuardrails failure")),
		|_| pass_response(),
	)
	.spawn()
	.await;

	let policy = guardrails_test_support::policy_with(
		extmcp_mock.address,
		guardrails::FailureMode::FailClosed,
		guardrails_test_support::default_methods(),
		HashMap::new(),
	);
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(&mock, true, false, vec![policy]).await;
	let client = mcp_streamable_client(io).await;
	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect_err("tool call should fail under failure_mode=Deny when mcpGuardrails errors");

	let rmcp::ServiceError::McpError(e) = &err else {
		panic!("expected McpError, got {err:?}");
	};
	assert_eq!(
		e.code,
		rmcp::model::ErrorCode::INTERNAL_ERROR,
		"gRPC failure should map to internal error"
	);
	assert!(
		e.message.contains("mcpGuardrails checkRequest failed"),
		"unexpected message: {}",
		e.message
	);
}

#[tokio::test]
async fn mcp_guardrails_response_reject_surfaces_jsonrpc_error() {
	use protos::ext_mcp::authorization_error::Code;

	use crate::test_helpers::extmcpmock::{
		closure_mock, pass_request, pass_response, reject_response,
	};

	let extmcp_mock = closure_mock(
		|_| pass_request(),
		|req| {
			if req.method != "tools/call" {
				return pass_response();
			}
			reject_response(Code::PermissionDenied, "blocked on response")
		},
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect_err("tool call should fail when mcpGuardrails rejects the response");

	let rmcp::ServiceError::McpError(e) = &err else {
		panic!("expected McpError, got {err:?}");
	};
	assert_eq!(e.code.0, -32001, "PermissionDenied should map to -32001");
	assert_eq!(e.message.as_ref(), "blocked on response");
}

#[tokio::test]
async fn mcp_guardrails_protocol_violation_fails_closed() {
	use crate::mcp::guardrails::wire;
	use crate::test_helpers::extmcpmock::{closure_mock, pass_response};

	// A response with no `result` oneof set is a contract violation; under
	// failure_mode=Deny it must reject rather than pass through.
	let extmcp_mock = closure_mock(
		|_| {
			Ok(wire::McpRequestResult {
				result: None,
				header_mutation: None,
				metadata: None,
			})
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect_err(
			"tool call should fail on mcpGuardrails protocol violation under failure_mode=Deny",
		);

	let rmcp::ServiceError::McpError(e) = &err else {
		panic!("expected McpError, got {err:?}");
	};
	assert_eq!(
		e.code,
		rmcp::model::ErrorCode::INTERNAL_ERROR,
		"protocol violation should map to internal error"
	);
	assert!(
		e.message.contains("protocol violation"),
		"unexpected message: {}",
		e.message
	);
}

#[tokio::test]
async fn mcp_guardrails_non_object_mutation_is_protocol_violation() {
	use crate::test_helpers::extmcpmock::{closure_mock, mutated_request_json, pass_response};

	// Mutated payloads must parse as the method's params; valid-but-wrong-shape
	// JSON must hit the protocol-violation path, not surface later as a
	// malformed MCP request.
	let extmcp_mock = closure_mock(
		|_| mutated_request_json(serde_json::json!([1, 2])),
		|_| pass_response(),
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let err = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect_err("non-object mutation should reject under failure_mode=Deny");

	let rmcp::ServiceError::McpError(e) = &err else {
		panic!("expected McpError, got {err:?}");
	};
	assert!(
		e.message.contains("protocol violation"),
		"unexpected message: {}",
		e.message
	);
}

#[tokio::test]
async fn mcp_guardrails_header_mutation_reaches_upstream() {
	use crate::test_helpers::extmcpmock::{closure_mock, pass_request_with, pass_response};

	let extmcp_mock = closure_mock(
		|_| {
			pass_request_with(
				vec![("x-guardrails-test", "from-policy"), ("x-tenant", "acme")],
				vec!["user-agent"],
				None,
			)
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let (mock, captured) = mock_streamable_http_server_with_capture(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let _ = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("tool call should succeed");

	let headers = captured.lock().unwrap().clone();
	let saw_injected = headers.iter().any(|h| {
		h.get("x-guardrails-test").map(|v| v.as_bytes()) == Some(b"from-policy")
			&& h.get("x-tenant").map(|v| v.as_bytes()) == Some(b"acme")
	});
	assert!(
		saw_injected,
		"expected x-guardrails-test+x-tenant headers on upstream request; saw {headers:?}"
	);
	let tools_call_req = headers
		.iter()
		.rev()
		.find(|h| h.contains_key("x-guardrails-test"))
		.expect("found upstream request with injected header");
	assert!(
		!tools_call_req.contains_key("user-agent"),
		"expected user-agent to be removed by header_mutation.remove",
	);
}

#[tokio::test]
async fn mcp_guardrails_request_headers_visible_to_policy_server() {
	use std::sync::Mutex as StdMutex;

	use crate::test_helpers::extmcpmock::{closure_mock, pass_request, pass_response};

	let captured: Arc<StdMutex<Option<Vec<crate::mcp::guardrails::wire::McpHeader>>>> =
		Arc::new(StdMutex::new(None));
	let extmcp_mock = {
		let store = captured.clone();
		closure_mock(
			move |req| {
				if req.method == "tools/call" {
					*store.lock().unwrap() = Some(req.headers.clone());
				}
				pass_request()
			},
			|_| pass_response(),
		)
		.spawn()
		.await
	};

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let _ = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("tool call should succeed");

	let headers = captured.lock().unwrap().clone().expect("headers captured");
	// The inbound POST's headers reach the policy server without per-header CEL config.
	assert!(
		headers
			.iter()
			.any(|h| h.key.eq_ignore_ascii_case("content-type")),
		"expected incoming request headers forwarded to policy server; saw {headers:?}"
	);
}

// mcpGuardrails processor metadata is readable as `guardrails.*` in an upstream-leg transformation.
#[tokio::test]
async fn mcp_guardrails_request_metadata_usable_in_backend_transformation() {
	use crate::http::transformation_cel::{
		LocalTransform, LocalTransformationConfig, Transformation,
	};
	use crate::test_helpers::extmcpmock::{closure_mock, pass_request_with, pass_response};

	let extmcp_mock = closure_mock(
		|_| {
			let md = serde_json::from_value(serde_json::json!({ "tenant": "acme" })).unwrap();
			pass_request_with(Vec::<(&str, &str)>::new(), Vec::<&str>::new(), Some(md))
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let xfm = Transformation::try_from_local_config(
		LocalTransformationConfig {
			request: Some(LocalTransform {
				set: vec![(
					strng::new("x-from-guardrails"),
					strng::new("mcpGuardrails.tenant"),
				)],
				..Default::default()
			}),
			response: None,
		},
		true,
	)
	.unwrap();
	let target_policy = BackendTrafficPolicy::Transformation(Arc::new(xfm));

	let (mock, captured) = mock_streamable_http_server_with_capture(true).await;
	let (_bind, io) = setup_proxy_policies_with_target(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
		vec![target_policy],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let _ = client
		.call_tool(
			rmcp::model::CallToolRequestParams::new("echo").with_arguments(
				serde_json::json!({"hi": "world"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("tool call should succeed");

	let headers = captured.lock().unwrap().clone();
	let saw_metadata = headers
		.iter()
		.any(|h| h.get("x-from-guardrails").map(|v| v.as_bytes()) == Some(b"acme"));
	assert!(
		saw_metadata,
		"expected x-from-guardrails:acme set by a backend transformation reading guardrails.tenant; saw {headers:?}"
	);
}

#[tokio::test]
async fn mcp_guardrails_mutated_prompt_request_reaches_upstream() {
	use crate::test_helpers::extmcpmock::{
		closure_mock, mutated_request_json, pass_request, pass_response,
	};

	let extmcp_mock = closure_mock(
		|req| {
			if req.method != "prompts/get" {
				return pass_request();
			}
			mutated_request_json(serde_json::json!({
				"name": "example_prompt",
				"arguments": { "message": "rewritten-by-guardrails" },
			}))
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let result = client
		.get_prompt(
			rmcp::model::GetPromptRequestParams::new("example_prompt").with_arguments(
				serde_json::json!({"message": "original-message"})
					.as_object()
					.cloned()
					.unwrap(),
			),
		)
		.await
		.expect("get_prompt should succeed");

	// example_prompt echoes `message` into its text body.
	let text = result
		.messages
		.iter()
		.find_map(|m| match &m.content {
			rmcp::model::PromptMessageContent::Text { text } => Some(text.clone()),
			_ => None,
		})
		.expect("prompt should have text content");
	assert!(text.contains("rewritten-by-guardrails"));
	assert!(!text.contains("original-message"));
}

#[tokio::test]
async fn mcp_guardrails_mutated_resource_read_reaches_upstream() {
	use crate::test_helpers::extmcpmock::{
		closure_mock, mutated_request_json, pass_request, pass_response,
	};

	let extmcp_mock = closure_mock(
		|req| {
			if req.method != "resources/read" {
				return pass_request();
			}
			// Redirect the client's "cwd" request to the "memo" resource.
			mutated_request_json(serde_json::json!({ "uri": "memo://insights" }))
		},
		|_| pass_response(),
	)
	.spawn()
	.await;

	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![guardrails_test_support::policy(extmcp_mock.address)],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let result = client
		.read_resource(rmcp::model::ReadResourceRequestParams::new(
			"str:////Users/to/some/path/",
		))
		.await
		.expect("read_resource should succeed after URI rewrite");

	let text = result
		.contents
		.iter()
		.find_map(|c| match c {
			rmcp::model::ResourceContents::TextResourceContents { text, .. } => Some(text.clone()),
			_ => None,
		})
		.expect("resource should return text");
	assert!(text.contains("Business Intelligence Memo"));
}
