use std::net::SocketAddr;

use agent_core::strng;
use insta::assert_json_snapshot;
use itertools::Itertools;
use rmcp::RoleClient;
use rmcp::model::{ErrorCode, InitializeRequestParams};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpServerConfig;
use secrecy::SecretString;
use serde_json::json;

use crate::http::auth::BackendAuth;
use crate::http::authorization::{PolicySet, RuleSet};
use crate::mcp::McpAuthorization;
use crate::test_helpers::proxymock::{
	BIND_KEY, TestBind, basic_named_route, basic_route, setup_proxy_test, simple_bind,
};
use crate::types::agent::BackendPolicy;
use crate::*;

#[tokio::test]
async fn streamable_client_to_streamable_backend_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn legacy_sse_client_to_streamable_backend_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_sse_client(io).await;
	standard_sse_assertions(client).await;
}

#[tokio::test]
async fn streamable_client_to_legacy_sse_backend_single() {
	let mock = mock_sse_server().await;
	let (_bind, io) = setup_proxy(&mock, true, true).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn legacy_sse_client_to_legacy_sse_backend_single() {
	let mock = mock_sse_server().await;
	let (_bind, io) = setup_proxy(&mock, true, true).await;
	let client = mcp_sse_client(io).await;
	standard_sse_assertions(client).await;
}

#[tokio::test]
async fn snapshot_tools_list_response() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let mut tools = client.list_tools(None).await.unwrap();
	tools
		.tools
		.sort_by(|a, b| a.name.as_ref().cmp(b.name.as_ref()));

	let value = serde_json::to_value(&tools).expect("tools json");
	assert_json_snapshot!(value);
}

#[tokio::test]
async fn snapshot_unknown_tool_error_response() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let err = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "unknown_tool".into(),
			arguments: None,
		})
		.await
		.expect_err("expected unknown tool error");

	let value = match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			serde_json::to_value(&mcp_error).expect("error json")
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	};
	assert_json_snapshot!(value);
}

#[tokio::test]
async fn snapshot_initialize_response() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let info = client.peer_info().expect("initialize response");
	let mut value = serde_json::to_value(info).expect("initialize json");
	if let Some(version) = value
		.get_mut("serverInfo")
		.and_then(|info| info.get_mut("version"))
	{
		*version = json!("<redacted>");
	}
	assert_json_snapshot!(value);
}

#[tokio::test]
async fn snapshot_url_elicitation_error_response() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let err = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "test_url_elicitation_required".into(),
			arguments: None,
		})
		.await
		.expect_err("expected url elicitation error");

	let value = match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			serde_json::to_value(&mcp_error).expect("error json")
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	};
	assert_json_snapshot!(value);
}

#[tokio::test]
async fn multiplex_tools_are_prefixed_and_routed() {
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
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
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
			"mcp__decrement".to_string(),
			"mcp__echo".to_string(),
			"mcp__echo_http".to_string(),
			"sse__decrement".to_string(),
			"sse__echo".to_string(),
			"sse__echo_http".to_string()
		]
	);

	let ctr = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "mcp__echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);

	let ctr = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "sse__echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);

	// No target set...
	assert!(
		client
			.call_tool(rmcp::model::CallToolRequestParams {
				meta: None,
				task: None,
				name: "echo".into(),
				arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
			})
			.await
			.is_err()
	);
}

#[tokio::test]
async fn multiplex_merged_tools_preserve_upstream_meta() {
	let mock_stream = mock_streamable_http_meta_server(true, "mcp").await;
	let mock_sse = mock_streamable_http_meta_server(true, "sse").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("sse", mock_sse.addr, false),
				("mcp", mock_stream.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let tools = client.list_tools(None).await.unwrap();
	let meta = tools.meta.expect("merged meta should be present");
	let upstreams = meta
		.0
		.get("upstreams")
		.and_then(|v| v.as_object())
		.expect("meta.upstreams");
	let mcp_meta = upstreams
		.get("mcp")
		.and_then(|v| v.get("label"))
		.and_then(|v| v.as_str());
	let sse_meta = upstreams
		.get("sse")
		.and_then(|v| v.get("label"))
		.and_then(|v| v.as_str());
	assert_eq!(mcp_meta, Some("mcp"));
	assert_eq!(sse_meta, Some("sse"));
}

#[tokio::test]
async fn stateless_client_to_stateful_upstream_roundtrip() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, false, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn stateless_client_to_stateless_upstream_roundtrip() {
	let mock = mock_streamable_http_server(false).await;
	let (_bind, io) = setup_proxy(&mock, false, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn streamable_client_to_streamable_backend_single_tls() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::BackendAuth(BackendAuth::Key(
			SecretString::new("my-key".into()),
		))],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let ctr = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "echo_http".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
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
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to call a tool - should fail with "Unknown tool" error
	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected tool call to fail due to authorization denial"
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
				"Unknown tool: echo",
				"Expected error message 'Unknown tool: echo', got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

#[tokio::test]
async fn url_elicitation_error_passthrough_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "test_url_elicitation_required".into(),
			arguments: None,
		})
		.await;

	assert!(result.is_err(), "Expected URL elicitation error");
	let err = result.unwrap_err();
	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code,
				ErrorCode::URL_ELICITATION_REQUIRED,
				"Expected URL elicitation required error code"
			);
			// SEP-1036 specifies the official error message
			assert_eq!(
				mcp_error.message.as_ref(),
				"This request requires more information.",
				"Expected spec-compliant SEP-1036 error message, got: {}",
				mcp_error.message
			);
			let data = mcp_error.data.expect("error data should be present");
			let elicitations = data
				.get("elicitations")
				.and_then(|v| v.as_array())
				.expect("elicitations array should be present");
			assert_eq!(elicitations.len(), 1);
			let first = elicitations[0].as_object().expect("elicitation object");
			assert_eq!(first.get("mode").and_then(|v| v.as_str()), Some("url"));
			assert_eq!(
				first.get("elicitationId").and_then(|v| v.as_str()),
				Some("elicit-1")
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

#[tokio::test]
async fn tasks_lifecycle_roundtrip_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let task_id = call_tool_task(&client, "echo", json!({"hi": "world"}).as_object().cloned()).await;
	let task = get_task_info(&client, &task_id).await;
	assert_eq!(task.status, rmcp::model::TaskStatus::Completed);

	let result = get_task_result(&client, &task_id).await;
	assert_eq!(result.get("tool").and_then(|v| v.as_str()), Some("echo"));
}

#[tokio::test]
async fn tasks_cancel_roundtrip_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let task_id = call_tool_task(&client, "echo", None).await;
	cancel_task(&client, &task_id).await;
	let task = get_task_info(&client, &task_id).await;
	assert_eq!(task.status, rmcp::model::TaskStatus::Cancelled);
}

#[tokio::test]
async fn task_support_required_rejects_non_task_call() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "test_task_required".into(),
			arguments: None,
		})
		.await;

	let err = result.expect_err("expected required task tool to reject non-task invocation");
	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32601,
				"Expected METHOD_NOT_FOUND error code (-32601), got: {}",
				mcp_error.code.0
			);
			assert!(
				mcp_error
					.message
					.as_ref()
					.contains("requires task-based invocation"),
				"Expected task-based invocation error message, got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

#[tokio::test]
async fn task_support_forbidden_rejects_task_call() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let params = rmcp::model::CallToolRequestParams {
		meta: None,
		task: empty_task_meta(),
		name: "test_task_forbidden".into(),
		arguments: None,
	};
	let request = rmcp::model::CallToolRequest {
		method: Default::default(),
		params,
		extensions: Default::default(),
	};
	let result = client
		.send_request(rmcp::model::ClientRequest::CallToolRequest(request))
		.await;

	let err = result.expect_err("expected forbidden task tool to reject task invocation");
	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert!(
				mcp_error
					.message
					.as_ref()
					.contains("does not support task-based invocation"),
				"Expected task-based invocation error message, got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

#[tokio::test]
async fn session_id_propagates_to_stateful_upstream() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let first = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "test_session_id".into(),
			arguments: None,
		})
		.await
		.unwrap();
	let first_id = first.content[0]
		.raw
		.as_text()
		.expect("session id should be text")
		.text
		.clone();
	assert!(
		!first_id.is_empty(),
		"expected upstream session id to be set"
	);

	let second = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "test_session_id".into(),
			arguments: None,
		})
		.await
		.unwrap();
	let second_id = second.content[0]
		.raw
		.as_text()
		.expect("session id should be text")
		.text
		.clone();
	assert_eq!(first_id, second_id, "expected stable upstream session id");
}

#[tokio::test]
async fn elicitation_form_roundtrip_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;

	let response_content = json!({"color": "blue"});
	let client =
		mcp_streamable_client_with_handler(io, ElicitationClient::new(response_content.clone())).await;

	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "test_elicitation_roundtrip".into(),
			arguments: None,
		})
		.await
		.unwrap();

	assert_eq!(result.structured_content, Some(response_content));
}

#[tokio::test]
async fn roots_list_roundtrip_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;

	let expected_roots = vec![rmcp::model::Root {
		uri: "file:///Users/test/workspace".to_string(),
		name: Some("workspace".to_string()),
	}];
	let client =
		mcp_streamable_client_with_handler(io, RootsClient::new(expected_roots.clone())).await;

	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "test_roots_roundtrip".into(),
			arguments: None,
		})
		.await
		.unwrap();

	assert_eq!(
		result.structured_content,
		Some(json!({
			"rootsAdvertised": true,
			"roots": expected_roots,
		}))
	);
}

#[tokio::test]
async fn prompts_roundtrip_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;

	let names = list_prompt_names(&client).await;
	assert!(names.contains(&"example_prompt".to_string()));
	assert!(names.contains(&"counter_analysis".to_string()));

	let text = prompt_text(
		&client,
		"example_prompt",
		json!({"message": "hi"}).as_object().cloned(),
	)
	.await;
	assert!(text.contains("hi"));
}

#[tokio::test]
async fn prompts_roundtrip_multiplex() {
	let mock_counter = mock_streamable_http_server(true).await;
	let mock_meta = mock_streamable_http_meta_server(true, "meta").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter.addr, false),
				("meta", mock_meta.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let names = list_prompt_names(&client).await;
	assert_eq!(
		names,
		vec![
			"counter__counter_analysis".to_string(),
			"counter__example_prompt".to_string()
		]
	);

	let text = prompt_text(
		&client,
		"counter__example_prompt",
		json!({"message": "world"}).as_object().cloned(),
	)
	.await;
	assert!(text.contains("world"));
}

#[tokio::test]
async fn resources_roundtrip_multiplex() {
	let mock_counter = mock_streamable_http_server(true).await;
	let mock_meta = mock_streamable_http_meta_server(true, "meta").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter.addr, false),
				("meta", mock_meta.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let resources = client.list_resources(None).await.unwrap();
	let uris = resources
		.resources
		.iter()
		.map(|r| r.uri.to_string())
		.sorted()
		.collect_vec();

	// Verify that URIs are wrapped in agw:// and have the correct target
	assert!(uris.iter().all(|u| u.starts_with("agw://counter/")));

	let target_uri = uris.first().unwrap();
	let content = client
		.read_resource(rmcp::model::ReadResourceRequestParams {
			meta: None,
			uri: target_uri.clone(),
		})
		.await
		.unwrap();

	// Verify content was actually retrieved (mock server returns specific paths)
	assert!(!content.contents.is_empty());
}

#[tokio::test]
async fn resource_templates_roundtrip_multiplex() {
	let mock_counter_a = mock_streamable_http_server(true).await;
	let mock_counter_b = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter_a.addr, false),
				("counter2", mock_counter_b.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let templates = list_resource_templates(&client).await;
	let names = templates
		.resource_templates
		.iter()
		.map(|t| t.name.clone())
		.sorted()
		.collect_vec();
	assert_eq!(
		names,
		vec![
			"counter2__counter-template".to_string(),
			"counter__counter-template".to_string()
		]
	);
	let uris = templates
		.resource_templates
		.iter()
		.map(|t| t.uri_template.clone())
		.sorted()
		.collect_vec();
	assert_eq!(uris.len(), 2);
	assert!(
		uris.iter().all(|u| u.contains("{id}")),
		"wrapped URI templates must preserve template braces: {uris:?}"
	);
	assert!(
		uris.iter().any(|u| u.starts_with("agw://counter/"))
			&& uris.iter().any(|u| u.starts_with("agw://counter2/")),
		"unexpected wrapped URI templates: {uris:?}"
	);
}

#[tokio::test]
async fn prompts_list_errors_when_all_targeted_upstreams_fail() {
	let healthy_meta = mock_streamable_http_meta_server(true, "meta").await;
	let dead_addr = {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
			.await
			.expect("bind dead listener");
		let addr = listener.local_addr().expect("dead listener addr");
		drop(listener);
		addr
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("meta", healthy_meta.addr, false),
				("dead", dead_addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let err = client
		.list_prompts(None)
		.await
		.expect_err("expected all targeted upstreams to fail");
	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(mcp_error.code.0, -32600);
			assert!(
				mcp_error
					.message
					.contains("all eligible backends failed for method prompts/list"),
				"unexpected error message: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

#[tokio::test]
async fn tasks_roundtrip_multiplex() {
	let mock_counter = mock_streamable_http_server(true).await;
	let mock_meta = mock_streamable_http_meta_server(true, "meta").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter.addr, false),
				("meta", mock_meta.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let task_id = assert_task_roundtrip(
		&client,
		"counter__echo",
		json!({"hi": "world"}).as_object().cloned(),
		"echo",
	)
	.await;
	assert!(task_id.starts_with("counter__"));
}

#[tokio::test]
async fn tasks_roundtrip_single_always_prefix() {
	let mock = mock_streamable_http_server(true).await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend_policies_with_prefix(mock.addr, true, false, vec![], true)
		.with_bind(simple_bind(basic_route(mock.addr)));
	let io = t.serve_real_listener(BIND_KEY).await;
	let client = mcp_streamable_client(io).await;

	let task_id = assert_task_roundtrip(
		&client,
		"mcp__echo",
		json!({"hi": "world"}).as_object().cloned(),
		"echo",
	)
	.await;
	assert!(task_id.starts_with("mcp__"));
}

#[tokio::test]
async fn elicitation_form_roundtrip_multiplex() {
	let mock_counter = mock_streamable_http_server(true).await;
	let mock_meta = mock_streamable_http_meta_server(true, "meta").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter.addr, false),
				("meta", mock_meta.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;

	let response_content = json!({"color": "blue"});
	let client =
		mcp_streamable_client_with_handler(io, ElicitationClient::new(response_content.clone())).await;

	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "counter__test_elicitation_roundtrip".into(),
			arguments: None,
		})
		.await
		.unwrap();

	assert_eq!(result.structured_content, Some(response_content));
}

#[tokio::test]
async fn completions_prompt_roundtrip_multiplex() {
	let mock_counter = mock_streamable_http_server(true).await;
	let mock_meta = mock_streamable_http_meta_server(true, "meta").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter.addr, false),
				("meta", mock_meta.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let values = client
		.complete_prompt_simple("counter__example_prompt", "message", "wo")
		.await
		.unwrap();
	assert_eq!(values, vec!["prompt-wo".to_string()]);
}

#[tokio::test]
async fn completions_resource_roundtrip_multiplex() {
	let mock_counter = mock_streamable_http_server(true).await;
	let mock_meta = mock_streamable_http_meta_server(true, "meta").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter.addr, false),
				("meta", mock_meta.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let resources = client.list_resources(None).await.unwrap();
	let uri = resources
		.resources
		.first()
		.expect("resource")
		.uri
		.to_string();
	let values = client
		.complete_resource_simple(uri, "path", "me")
		.await
		.unwrap();
	assert_eq!(values, vec!["resource-me".to_string()]);
}

#[tokio::test]
async fn url_elicitation_error_passthrough_in_multiplex() {
	let mock_counter = mock_streamable_http_server(true).await;
	let mock_meta = mock_streamable_http_meta_server(true, "meta").await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("counter", mock_counter.addr, false),
				("meta", mock_meta.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;

	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "counter__test_url_elicitation_required".into(),
			arguments: None,
		})
		.await;

	assert!(result.is_err(), "Expected URL elicitation error");
	let err = result.unwrap_err();
	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code,
				ErrorCode::URL_ELICITATION_REQUIRED,
				"Expected URL elicitation required error code"
			);
			// SEP-1036 specifies the official error message
			assert_eq!(
				mcp_error.message.as_ref(),
				"This request requires more information.",
				"Expected spec-compliant SEP-1036 error message, got: {}",
				mcp_error.message
			);
			let data = mcp_error.data.expect("error data should be present");
			let elicitations = data
				.get("elicitations")
				.and_then(|v| v.as_array())
				.expect("elicitations array should be present");
			assert_eq!(elicitations.len(), 1);
			let first = elicitations[0].as_object().expect("elicitation object");
			assert_eq!(first.get("mode").and_then(|v| v.as_str()), Some("url"));
			assert_eq!(
				first.get("elicitationId").and_then(|v| v.as_str()),
				Some("elicit-1")
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
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
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to get a prompt - should fail with "Unknown prompt" error
	let result = client
		.get_prompt(rmcp::model::GetPromptRequestParams {
			meta: None,
			name: "example_prompt".into(),
			arguments: None,
		})
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
	let deny_all_policy = deny_all_mcp_authorization_policy();

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to read a resource - should fail with "Unknown resource" error
	let result = client
		.read_resource(rmcp::model::ReadResourceRequestParams {
			meta: None,
			uri: "memo://insights".into(),
		})
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected read_resource call to fail due to authorization denial"
	);

	assert_unknown_resource_error(result.unwrap_err(), "memo://insights");
}

#[tokio::test]
async fn authorization_denied_wrapped_resource_uri_returns_unknown_resource_error() {
	let mock = mock_streamable_http_server(true).await;

	let deny_all_policy = deny_all_mcp_authorization_policy();

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend_policies_with_prefix(
			mock.addr,
			true,
			false,
			vec![BackendPolicy::McpAuthorization(deny_all_policy)],
			true,
		)
		.with_bind(simple_bind(basic_route(mock.addr)));
	let io = t.serve_real_listener(BIND_KEY).await;
	let client = mcp_streamable_client(io).await;

	let wrapped_uri = "agw://mcp/?u=memo%3A%2F%2Finsights".to_string();
	let err = client
		.read_resource(rmcp::model::ReadResourceRequestParams {
			meta: None,
			uri: wrapped_uri,
		})
		.await
		.expect_err("expected wrapped resource read to fail due to authorization denial");

	assert_unknown_resource_error(err, "memo://insights");
}

#[tokio::test]
async fn authorization_denied_returns_unknown_task_error() {
	let mock = mock_streamable_http_server(true).await;

	let deny_all_policy = deny_all_mcp_authorization_policy();

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;
	let err = client
		.send_request(rmcp::model::ClientRequest::GetTaskInfoRequest(
			rmcp::model::GetTaskInfoRequest {
				method: Default::default(),
				params: rmcp::model::GetTaskInfoParams {
					meta: None,
					task_id: "task-0".to_string(),
				},
				extensions: Default::default(),
			},
		))
		.await
		.expect_err("expected task info call to fail due to authorization denial");

	assert_unknown_task_error(err, "task-0");
}

fn empty_task_meta() -> Option<rmcp::model::JsonObject> {
	json!({}).as_object().cloned()
}

fn task_from_result(result: rmcp::model::ServerResult) -> rmcp::model::Task {
	match result {
		rmcp::model::ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult, got: {:?}", other),
	}
}

async fn call_tool_task(
	client: &rmcp::service::Peer<rmcp::RoleClient>,
	name: &str,
	arguments: Option<rmcp::model::JsonObject>,
) -> String {
	let params = rmcp::model::CallToolRequestParams {
		meta: None,
		task: empty_task_meta(),
		name: name.to_string().into(),
		arguments,
	};
	let request = rmcp::model::CallToolRequest {
		method: Default::default(),
		params,
		extensions: Default::default(),
	};
	let response = client
		.send_request(rmcp::model::ClientRequest::CallToolRequest(request))
		.await
		.unwrap();
	match response {
		rmcp::model::ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult, got: {:?}", other),
	}
}

async fn get_task_info(
	client: &rmcp::service::Peer<rmcp::RoleClient>,
	task_id: &str,
) -> rmcp::model::Task {
	let info_request = rmcp::model::GetTaskInfoRequest {
		method: Default::default(),
		params: rmcp::model::GetTaskInfoParams {
			meta: None,
			task_id: task_id.to_string(),
		},
		extensions: Default::default(),
	};
	let info_response = client
		.send_request(rmcp::model::ClientRequest::GetTaskInfoRequest(info_request))
		.await
		.unwrap();
	task_from_result(info_response)
}

async fn get_task_result(
	client: &rmcp::service::Peer<rmcp::RoleClient>,
	task_id: &str,
) -> serde_json::Value {
	let result_request = rmcp::model::GetTaskResultRequest {
		method: Default::default(),
		params: rmcp::model::GetTaskResultParams {
			meta: None,
			task_id: task_id.to_string(),
		},
		extensions: Default::default(),
	};
	let result_response = client
		.send_request(rmcp::model::ClientRequest::GetTaskResultRequest(
			result_request,
		))
		.await
		.unwrap();
	match result_response {
		rmcp::model::ServerResult::CustomResult(result) => result.0,
		other => panic!("Expected CustomResult for tasks/result, got: {:?}", other),
	}
}

async fn list_prompt_names(client: &rmcp::service::Peer<rmcp::RoleClient>) -> Vec<String> {
	let prompts = client.list_prompts(None).await.unwrap();
	prompts
		.prompts
		.iter()
		.map(|p| p.name.to_string())
		.sorted()
		.collect_vec()
}

async fn prompt_text(
	client: &rmcp::service::Peer<rmcp::RoleClient>,
	name: &str,
	arguments: Option<serde_json::Map<String, serde_json::Value>>,
) -> String {
	let prompt = client
		.get_prompt(rmcp::model::GetPromptRequestParams {
			meta: None,
			name: name.into(),
			arguments,
		})
		.await
		.unwrap();
	match &prompt.messages[0].content {
		rmcp::model::PromptMessageContent::Text { text } => text.clone(),
		other => panic!("Expected text prompt content, got: {:?}", other),
	}
}

async fn list_tasks(
	client: &rmcp::service::Peer<rmcp::RoleClient>,
) -> rmcp::model::ListTasksResult {
	let list_request = rmcp::model::ListTasksRequest {
		method: Default::default(),
		params: Some(rmcp::model::PaginatedRequestParams {
			meta: None,
			cursor: None,
		}),
		extensions: Default::default(),
	};
	let list_response = client
		.send_request(rmcp::model::ClientRequest::ListTasksRequest(list_request))
		.await
		.unwrap();
	match list_response {
		rmcp::model::ServerResult::ListTasksResult(result) => result,
		other => panic!("Expected ListTasksResult, got: {:?}", other),
	}
}

async fn list_resource_templates(
	client: &rmcp::service::Peer<rmcp::RoleClient>,
) -> rmcp::model::ListResourceTemplatesResult {
	let request = rmcp::model::ListResourceTemplatesRequest {
		method: Default::default(),
		params: Some(rmcp::model::PaginatedRequestParams {
			meta: None,
			cursor: None,
		}),
		extensions: Default::default(),
	};
	let response = client
		.send_request(rmcp::model::ClientRequest::ListResourceTemplatesRequest(
			request,
		))
		.await
		.unwrap();
	match response {
		rmcp::model::ServerResult::ListResourceTemplatesResult(result) => result,
		other => panic!("Expected ListResourceTemplatesResult, got: {:?}", other),
	}
}

fn deny_all_mcp_authorization_policy() -> McpAuthorization {
	McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],                                                       // no allow rules
		vec![Arc::new(cel::Expression::new_strict("true").unwrap())], // deny all
	)))
}

fn assert_unknown_resource_error(err: rmcp::ServiceError, expected_uri: &str) {
	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert_eq!(
				mcp_error.message.as_ref(),
				format!("Unknown resource: {expected_uri}"),
				"Expected Unknown resource error for {}, got: {}",
				expected_uri,
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
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
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_echo_policy)],
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
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_policy)],
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
		let client_info = ClientInfo {
			meta: None,
			protocol_version: Default::default(),
			capabilities: ClientCapabilities::default(),
			client_info: Implementation {
				name: format!("test-{agent_name}"),
				version: "0.0.1".to_string(),
				title: None,
				website_url: None,
				icons: None,
				description: None,
			},
		};
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

fn assert_unknown_task_error(err: rmcp::ServiceError, expected_task_id: &str) {
	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert_eq!(
				mcp_error.message.as_ref(),
				format!("Unknown task: {expected_task_id}"),
				"Expected Unknown task error for {}, got: {}",
				expected_task_id,
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

async fn assert_task_roundtrip(
	client: &rmcp::service::Peer<rmcp::RoleClient>,
	tool_name: &str,
	arguments: Option<serde_json::Map<String, serde_json::Value>>,
	expected_tool: &str,
) -> String {
	let task_id = call_tool_task(client, tool_name, arguments).await;
	assert!(task_id.contains("__"));

	let tasks = list_tasks(client).await;
	assert!(tasks.tasks.iter().any(|t| t.task_id == task_id));

	let task = get_task_info(client, &task_id).await;
	assert_eq!(task.task_id, task_id);

	let result = get_task_result(client, &task_id).await;
	assert_eq!(
		result.get("tool").and_then(|v| v.as_str()),
		Some(expected_tool)
	);

	task_id
}

async fn cancel_task(client: &rmcp::service::Peer<rmcp::RoleClient>, task_id: &str) {
	let cancel_request = rmcp::model::CancelTaskRequest {
		method: Default::default(),
		params: rmcp::model::CancelTaskParams {
			meta: None,
			task_id: task_id.to_string(),
		},
		extensions: Default::default(),
	};
	let cancel_response = client
		.send_request(rmcp::model::ClientRequest::CancelTaskRequest(
			cancel_request,
		))
		.await
		.unwrap();
	match cancel_response {
		rmcp::model::ServerResult::GetTaskResult(_) => {},
		other => panic!("Expected GetTaskResult for tasks/cancel, got: {:?}", other),
	}
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
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
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
	policies: Vec<BackendPolicy>,
) -> (TestBind, SocketAddr) {
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend_policies(mock.addr, stateful, legacy_sse, policies)
		.with_bind(simple_bind(basic_route(mock.addr)));
	let io = t.serve_real_listener(BIND_KEY).await;
	(t, io)
}

pub async fn mcp_streamable_client(
	s: SocketAddr,
) -> RunningService<RoleClient, InitializeRequestParams> {
	use rmcp::ServiceExt;
	use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use rmcp::transport::StreamableHttpClientTransport;
	let transport = StreamableHttpClientTransport::with_client(
		reqwest::Client::new(),
		rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
			format!("http://{s}/mcp"),
		),
	);
	let client_info = ClientInfo {
		meta: None,
		protocol_version: Default::default(),
		capabilities: ClientCapabilities::default(),
		client_info: Implementation {
			name: "test client".to_string(),
			version: "0.0.1".to_string(),
			title: None,
			description: None,
			website_url: None,
			icons: None,
		},
	};

	client_info
		.serve(transport)
		.await
		.inspect_err(|e| {
			tracing::error!("client error: {:?}", e);
		})
		.unwrap()
}

pub async fn mcp_streamable_client_with_handler<H: rmcp::ClientHandler>(
	s: SocketAddr,
	handler: H,
) -> RunningService<RoleClient, H> {
	use rmcp::ServiceExt;
	use rmcp::transport::StreamableHttpClientTransport;
	let transport = StreamableHttpClientTransport::with_client(
		reqwest::Client::new(),
		rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
			format!("http://{s}/mcp"),
		),
	);
	handler
		.serve(transport)
		.await
		.inspect_err(|e| {
			tracing::error!("client error: {:?}", e);
		})
		.unwrap()
}

fn client_info_with_tasks_and_elicitation() -> rmcp::model::ClientInfo {
	use rmcp::model::{ClientCapabilities, ElicitationCapability, Implementation, TasksCapability};
	rmcp::model::ClientInfo {
		meta: None,
		protocol_version: Default::default(),
		capabilities: ClientCapabilities::builder()
			.enable_elicitation_with(ElicitationCapability {
				form: Some(rmcp::model::FormElicitationCapability {
					schema_validation: Some(true),
				}),
				url: Some(rmcp::model::UrlElicitationCapability::default()),
			})
			.enable_tasks_with(TasksCapability::client_default())
			.build(),
		client_info: Implementation {
			name: "test client".to_string(),
			version: "0.0.1".to_string(),
			title: None,
			description: None,
			website_url: None,
			icons: None,
		},
	}
}

fn client_info_with_roots() -> rmcp::model::ClientInfo {
	use rmcp::model::{ClientCapabilities, Implementation};
	rmcp::model::ClientInfo {
		meta: None,
		protocol_version: Default::default(),
		capabilities: ClientCapabilities::builder()
			.enable_roots()
			.enable_roots_list_changed()
			.build(),
		client_info: Implementation {
			name: "test client".to_string(),
			version: "0.0.1".to_string(),
			title: None,
			description: None,
			website_url: None,
			icons: None,
		},
	}
}

struct ElicitationClient {
	info: rmcp::model::ClientInfo,
	response: serde_json::Value,
}

impl ElicitationClient {
	fn new(response: serde_json::Value) -> Self {
		Self {
			info: client_info_with_tasks_and_elicitation(),
			response,
		}
	}
}

impl rmcp::ClientHandler for ElicitationClient {
	fn create_elicitation(
		&self,
		_request: rmcp::model::CreateElicitationRequestParams,
		_context: rmcp::service::RequestContext<rmcp::RoleClient>,
	) -> impl std::future::Future<Output = Result<rmcp::model::CreateElicitationResult, rmcp::ErrorData>>
	+ Send
	+ '_ {
		let response = self.response.clone();
		std::future::ready(Ok(rmcp::model::CreateElicitationResult {
			action: rmcp::model::ElicitationAction::Accept,
			content: Some(response),
		}))
	}

	fn get_info(&self) -> rmcp::model::ClientInfo {
		self.info.clone()
	}
}

struct RootsClient {
	info: rmcp::model::ClientInfo,
	roots: Vec<rmcp::model::Root>,
}

impl RootsClient {
	fn new(roots: Vec<rmcp::model::Root>) -> Self {
		Self {
			info: client_info_with_roots(),
			roots,
		}
	}
}

impl rmcp::ClientHandler for RootsClient {
	fn list_roots(
		&self,
		_context: rmcp::service::RequestContext<rmcp::RoleClient>,
	) -> impl std::future::Future<Output = Result<rmcp::model::ListRootsResult, rmcp::ErrorData>> + Send + '_
	{
		std::future::ready(Ok(rmcp::model::ListRootsResult {
			roots: self.roots.clone(),
		}))
	}

	fn get_info(&self) -> rmcp::model::ClientInfo {
		self.info.clone()
	}
}

struct MockServer {
	addr: SocketAddr,
	_cancel: tokio::sync::oneshot::Sender<()>,
}

async fn mock_streamable_http_server(stateful: bool) -> MockServer {
	use mockserver::Counter;
	use rmcp::transport::streamable_http_server::StreamableHttpService;
	use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
	agent_core::telemetry::testing::setup_test_logging();

	let service = StreamableHttpService::new(
		|| Ok(Counter::new()),
		LocalSessionManager::default().into(),
		StreamableHttpServerConfig {
			sse_retry: None,
			sse_keep_alive: None,
			stateful_mode: stateful,
			json_response: false,
			cancellation_token: Default::default(),
		},
	);

	let (tx, rx) = tokio::sync::oneshot::channel();
	let router = axum::Router::new().nest_service("/mcp", service);
	let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = tcp_listener.local_addr().unwrap();
	tokio::spawn(async move {
		let _ = axum::serve(tcp_listener, router)
			.with_graceful_shutdown(async { rx.await.unwrap() })
			.await;
		info!("server stopped");
	});
	MockServer { addr, _cancel: tx }
}

async fn mock_streamable_http_meta_server(stateful: bool, label: &str) -> MockServer {
	use mockserver::MetaServer;
	use rmcp::transport::streamable_http_server::StreamableHttpService;
	use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
	agent_core::telemetry::testing::setup_test_logging();

	let label = label.to_string();
	let service = StreamableHttpService::new(
		move || Ok(MetaServer::new(label.clone())),
		LocalSessionManager::default().into(),
		StreamableHttpServerConfig {
			sse_keep_alive: None,
			sse_retry: None,
			stateful_mode: stateful,
			json_response: false,
			cancellation_token: Default::default(),
		},
	);

	let (tx, rx) = tokio::sync::oneshot::channel();
	let router = axum::Router::new().nest_service("/mcp", service);
	let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = tcp_listener.local_addr().unwrap();
	tokio::spawn(async move {
		let _ = axum::serve(tcp_listener, router)
			.with_graceful_shutdown(async { rx.await.unwrap() })
			.await;
		info!("server stopped");
	});
	MockServer { addr, _cancel: tx }
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
	MockServer { addr, _cancel: tx }
}

type LegacyService = legacy_rmcp::service::RunningService<
	legacy_rmcp::RoleClient,
	legacy_rmcp::model::InitializeRequestParam,
>;

pub async fn mcp_sse_client(s: SocketAddr) -> LegacyService {
	use legacy_rmcp::ServiceExt;
	use legacy_rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use legacy_rmcp::transport::SseClientTransport;
	let transport = SseClientTransport::start(format!("http://{s}/sse"))
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

	client_info.serve(transport).await.unwrap()
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

mod mockserver {
	use std::collections::HashMap;
	use std::sync::Arc;

	use http::request::Parts;
	use rmcp::handler::server::router::prompt::PromptRouter;
	use rmcp::handler::server::router::tool::ToolRouter;
	use rmcp::handler::server::wrapper::Parameters;
	use rmcp::model::*;
	use rmcp::service::RequestContext;
	use rmcp::transport::common::http_header::HEADER_SESSION_ID;
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
		tasks: Arc<Mutex<TaskStore>>,
		tool_router: ToolRouter<Counter>,
		prompt_router: PromptRouter<Counter>,
	}

	#[tool_router]
	impl Counter {
		#[allow(dead_code)]
		pub fn new() -> Self {
			Self {
				counter: Arc::new(Mutex::new(0)),
				tasks: Arc::new(Mutex::new(TaskStore::default())),
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

		#[tool(
			description = "Repeat what you say",
			execution(task_support = "optional")
		)]
		fn echo(&self, Parameters(object): Parameters<JsonObject>) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				serde_json::Value::Object(object).to_string(),
			)]))
		}

		#[tool(
			description = "Return task-required tool for validation",
			execution(task_support = "required")
		)]
		fn test_task_required(&self) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text("required")]))
		}

		#[tool(
			description = "Return task-forbidden tool for validation",
			execution(task_support = "forbidden")
		)]
		fn test_task_forbidden(&self) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text("forbidden")]))
		}

		#[tool(description = "Return upstream MCP session id header")]
		fn test_session_id(&self, rq: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
			let ext = rq.extensions.get::<Parts>();
			let value = ext
				.and_then(|parts| parts.headers.get(HEADER_SESSION_ID))
				.and_then(|v| v.to_str().ok())
				.unwrap_or_default()
				.to_string();
			Ok(CallToolResult::success(vec![Content::text(value)]))
		}

		#[tool(description = "Return URL elicitation required error for testing")]
		fn test_url_elicitation_required(&self) -> Result<CallToolResult, McpError> {
			Err(McpError::new(
				ErrorCode::URL_ELICITATION_REQUIRED,
				"This request requires more information.",
				Some(json!({
					"elicitations": [
						{
							"mode": "url",
							"message": "Authenticate to continue",
							"elicitationId": "elicit-1",
							"url": "https://example.com/auth"
						}
					]
				})),
			))
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

		#[tool(description = "Round-trip form elicitation for testing")]
		async fn test_elicitation_roundtrip(
			&self,
			ctx: RequestContext<RoleServer>,
		) -> Result<CallToolResult, McpError> {
			let params = CreateElicitationRequestParams::FormElicitationParams {
				meta: None,
				message: "Provide your favorite color".to_string(),
				requested_schema: ElicitationSchema::builder()
					.required_string("color")
					.build()
					.map_err(|e| McpError::invalid_params(format!("schema error: {e}"), None))?,
			};

			let request = CreateElicitationRequest {
				method: Default::default(),
				params,
				extensions: Default::default(),
			};

			let response = ctx
				.peer
				.send_request(ServerRequest::CreateElicitationRequest(request))
				.await
				.map_err(|e| McpError::internal_error(format!("elicitation error: {e}"), None))?;

			let result = match response {
				ClientResult::CreateElicitationResult(result) => result,
				other => {
					return Err(McpError::internal_error(
						format!("unexpected response: {other:?}"),
						None,
					));
				},
			};

			if result.action != ElicitationAction::Accept {
				return Err(McpError::invalid_request(
					"elicitation not accepted".to_string(),
					None,
				));
			}

			let content = result.content.ok_or_else(|| {
				McpError::invalid_request("elicitation response missing content".to_string(), None)
			})?;

			Ok(CallToolResult {
				content: vec![Content::text("elicitation accepted")],
				structured_content: Some(content),
				is_error: Some(false),
				meta: None,
			})
		}

		#[tool(description = "Round-trip roots/list for testing")]
		async fn test_roots_roundtrip(
			&self,
			ctx: RequestContext<RoleServer>,
		) -> Result<CallToolResult, McpError> {
			let request = ListRootsRequest {
				method: Default::default(),
				extensions: Default::default(),
			};

			let response = ctx
				.peer
				.send_request(ServerRequest::ListRootsRequest(request))
				.await
				.map_err(|e| McpError::internal_error(format!("roots/list error: {e}"), None))?;

			let result = match response {
				ClientResult::ListRootsResult(result) => result,
				other => {
					return Err(McpError::internal_error(
						format!("unexpected response: {other:?}"),
						None,
					));
				},
			};

			let roots_advertised = ctx
				.peer
				.peer_info()
				.and_then(|info| info.capabilities.roots.as_ref())
				.is_some();
			let payload = json!({
				"rootsAdvertised": roots_advertised,
				"roots": result.roots,
			});

			Ok(CallToolResult {
				content: vec![Content::text(payload.to_string())],
				structured_content: Some(payload),
				is_error: Some(false),
				meta: None,
			})
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
					.enable_completions()
					.enable_prompts()
					.enable_resources()
					.enable_tools()
					.enable_tasks_with(TasksCapability::server_default())
					.build(),
				server_info: Implementation::from_build_env(),
				instructions: Some("This server provides counter tools and prompts.".to_string()),
			}
		}

		async fn complete(
			&self,
			request: CompleteRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<CompleteResult, McpError> {
			let kind = match request.r#ref {
				Reference::Prompt(_) => "prompt",
				Reference::Resource(_) => "resource",
			};
			let value = format!("{kind}-{}", request.argument.value);
			let completion =
				CompletionInfo::with_all_values(vec![value]).expect("completion values should be valid");
			Ok(CompleteResult { completion })
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
			_request: Option<PaginatedRequestParams>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourceTemplatesResult, McpError> {
			Ok(ListResourceTemplatesResult {
				next_cursor: None,
				resource_templates: vec![
					RawResourceTemplate {
						uri_template: "memo://{id}".to_string(),
						name: "counter-template".to_string(),
						title: None,
						description: None,
						mime_type: None,
						icons: None,
					}
					.no_annotation(),
				],
				meta: None,
			})
		}

		async fn initialize(
			&self,
			_request: InitializeRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<InitializeResult, McpError> {
			Ok(self.get_info())
		}

		async fn enqueue_task(
			&self,
			request: CallToolRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<CreateTaskResult, McpError> {
			let mut tasks = self.tasks.lock().await;
			let result = json!({
				"tool": request.name,
				"arguments": request.arguments,
			});
			let task = tasks.create_task(result);
			Ok(CreateTaskResult { task })
		}

		async fn list_tasks(
			&self,
			_request: Option<PaginatedRequestParams>,
			_: RequestContext<RoleServer>,
		) -> Result<ListTasksResult, McpError> {
			let tasks = self.tasks.lock().await;
			Ok(ListTasksResult {
				tasks: tasks
					.tasks
					.values()
					.map(|entry| entry.task.clone())
					.collect(),
				next_cursor: None,
				total: None,
			})
		}

		async fn get_task_info(
			&self,
			request: GetTaskInfoParams,
			_: RequestContext<RoleServer>,
		) -> Result<GetTaskResult, McpError> {
			let mut tasks = self.tasks.lock().await;
			if let Some(entry) = tasks.tasks.get_mut(&request.task_id) {
				if entry.task.status == TaskStatus::Working && entry.result.is_some() {
					entry.task.status = TaskStatus::Completed;
					entry.task.status_message = Some("completed".to_string());
					entry.task.last_updated_at = entry.task.created_at.clone();
				}
				return Ok(GetTaskResult {
					meta: None,
					task: entry.task.clone(),
				});
			}
			Err(McpError::resource_not_found(
				"task not found".to_string(),
				None,
			))
		}

		async fn get_task_result(
			&self,
			request: GetTaskResultParams,
			_: RequestContext<RoleServer>,
		) -> Result<GetTaskPayloadResult, McpError> {
			let mut tasks = self.tasks.lock().await;
			let entry = tasks.tasks.get_mut(&request.task_id);
			let Some(entry) = entry else {
				return Err(McpError::invalid_request(
					"task not found".to_string(),
					None,
				));
			};
			if let Some(result) = entry.result.clone() {
				entry.task.status = TaskStatus::Completed;
				entry.task.status_message = Some("completed".to_string());
				entry.task.last_updated_at = entry.task.created_at.clone();
				return Ok(GetTaskPayloadResult(result));
			}
			Err(McpError::invalid_request(
				"task not ready".to_string(),
				None,
			))
		}

		async fn cancel_task(
			&self,
			request: CancelTaskParams,
			_: RequestContext<RoleServer>,
		) -> Result<CancelTaskResult, McpError> {
			let mut tasks = self.tasks.lock().await;
			if let Some(entry) = tasks.tasks.get_mut(&request.task_id) {
				entry.task.status = TaskStatus::Cancelled;
				entry.task.status_message = Some("cancelled".to_string());
				entry.task.last_updated_at = entry.task.created_at.clone();
				entry.result = None;
				return Ok(CancelTaskResult {
					meta: None,
					task: entry.task.clone(),
				});
			}
			Err(McpError::resource_not_found(
				"task not found".to_string(),
				None,
			))
		}
	}

	#[derive(Clone)]
	pub struct MetaServer {
		label: String,
	}

	impl MetaServer {
		pub fn new(label: impl Into<String>) -> Self {
			Self {
				label: label.into(),
			}
		}
	}

	impl ServerHandler for MetaServer {
		fn get_info(&self) -> ServerInfo {
			ServerInfo {
				protocol_version: ProtocolVersion::V_2025_06_18,
				capabilities: ServerCapabilities::builder()
					.enable_completions()
					.enable_tools()
					.build(),
				server_info: Implementation::from_build_env(),
				instructions: Some("Meta-only test server.".to_string()),
			}
		}

		async fn complete(
			&self,
			request: CompleteRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<CompleteResult, McpError> {
			let kind = match request.r#ref {
				Reference::Prompt(_) => "prompt",
				Reference::Resource(_) => "resource",
			};
			let value = format!("meta-{kind}-{}", request.argument.value);
			let completion =
				CompletionInfo::with_all_values(vec![value]).expect("completion values should be valid");
			Ok(CompleteResult { completion })
		}

		async fn list_tools(
			&self,
			_request: Option<PaginatedRequestParams>,
			_: RequestContext<RoleServer>,
		) -> Result<ListToolsResult, McpError> {
			let mut meta = Meta::new();
			meta.0.insert("label".to_string(), json!(self.label));
			Ok(ListToolsResult {
				tools: Vec::new(),
				next_cursor: None,
				meta: Some(meta),
			})
		}

		async fn initialize(
			&self,
			_request: InitializeRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<InitializeResult, McpError> {
			Ok(self.get_info())
		}
	}

	#[derive(Debug, Default)]
	struct TaskStore {
		next_id: u64,
		tasks: HashMap<String, TaskEntry>,
	}

	#[derive(Debug)]
	struct TaskEntry {
		task: Task,
		result: Option<serde_json::Value>,
	}

	impl TaskStore {
		fn create_task(&mut self, result: serde_json::Value) -> Task {
			let task_id = format!("task-{}", self.next_id);
			self.next_id += 1;
			let created_at = "2026-01-01T00:00:00Z".to_string();
			let task = Task {
				task_id: task_id.clone(),
				status: TaskStatus::Working,
				status_message: Some("queued".to_string()),
				created_at: created_at.clone(),
				last_updated_at: created_at,
				ttl: None,
				poll_interval: Some(10),
			};
			self.tasks.insert(
				task_id,
				TaskEntry {
					task: task.clone(),
					result: Some(result),
				},
			);
			task
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
