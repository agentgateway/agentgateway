use itertools::Itertools;
use rmcp::model::*;
use serde_json::json;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::Duration;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::gateway::AgentGateway;
use crate::common::mcp::{
	ComprehensiveClient, MockMcpServer, multiplex_config, multiplex_transport_matrix_config,
	setup_comprehensive_client, start_mock_legacy_sse_server, start_mock_mcp_server,
};

struct MultiplexTestFixture {
	client: ComprehensiveClient,
	update_count: Arc<AtomicUsize>,
	_gw: AgentGateway,
	_mcp1: MockMcpServer,
	_mcp2: MockMcpServer,
	_s3_mock: MockServer,
}

impl MultiplexTestFixture {
	async fn setup() -> anyhow::Result<Self> {
		agent_core::telemetry::testing::setup_test_logging();

		let mcp1 = start_mock_mcp_server("s1", true).await;
		let mcp2 = start_mock_mcp_server("s2", false).await;

		let s3_mock = MockServer::start().await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(500).set_body_string("CRASH"))
			.mount(&s3_mock)
			.await;

		let config = multiplex_config(&mcp1, &mcp2, *s3_mock.address());
		let gw = AgentGateway::new(config).await?;
		let mcp_url = format!("http://localhost:{}/mcp", gw.port());
		let update_count = Arc::new(AtomicUsize::new(0));
		let client = setup_comprehensive_client(&mcp_url, update_count.clone()).await?;

		Ok(Self {
			client,
			update_count,
			_gw: gw,
			_mcp1: mcp1,
			_mcp2: mcp2,
			_s3_mock: s3_mock,
		})
	}
}

struct MultiplexTransportMatrixFixture {
	client: ComprehensiveClient,
	update_count: Arc<AtomicUsize>,
	_gw: AgentGateway,
	_streamable: MockMcpServer,
	_sse: MockMcpServer,
	_s3_mock: MockServer,
}

impl MultiplexTransportMatrixFixture {
	async fn setup() -> anyhow::Result<Self> {
		agent_core::telemetry::testing::setup_test_logging();

		let streamable = start_mock_mcp_server("stream", true).await;
		let sse = start_mock_legacy_sse_server().await;

		let s3_mock = MockServer::start().await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(500).set_body_string("CRASH"))
			.mount(&s3_mock)
			.await;

		let config = multiplex_transport_matrix_config(&streamable, &sse, *s3_mock.address());
		let gw = AgentGateway::new(config).await?;
		let mcp_url = format!("http://localhost:{}/mcp", gw.port());
		let update_count = Arc::new(AtomicUsize::new(0));
		let client = setup_comprehensive_client(&mcp_url, update_count.clone()).await?;

		Ok(Self {
			client,
			update_count,
			_gw: gw,
			_streamable: streamable,
			_sse: sse,
			_s3_mock: s3_mock,
		})
	}
}

struct EnterpriseMultiplexFixture {
	mcp_url: String,
	labels: Vec<String>,
	stateful_labels: HashSet<String>,
	_gw: AgentGateway,
	_servers: Vec<MockMcpServer>,
	_broken: MockServer,
}

impl EnterpriseMultiplexFixture {
	async fn setup(target_count: usize) -> anyhow::Result<Self> {
		agent_core::telemetry::testing::setup_test_logging();
		assert!(
			target_count >= 20,
			"enterprise tests should use at least 20 targets"
		);

		let mut labels = Vec::with_capacity(target_count);
		let mut stateful_labels = HashSet::with_capacity(target_count / 2 + 1);
		let mut servers = Vec::with_capacity(target_count);
		for i in 0..target_count {
			let label = format!("s{:02}", i + 1);
			let stateful = i % 2 == 0;
			if stateful {
				stateful_labels.insert(label.clone());
			}
			let server = start_mock_mcp_server(label.clone(), stateful).await;
			labels.push(label);
			servers.push(server);
		}

		let broken = MockServer::start().await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(500).set_body_string("CRASH"))
			.mount(&broken)
			.await;

		let targets = labels
			.iter()
			.zip(servers.iter())
			.map(|(label, server)| (label.clone(), server.addr))
			.collect_vec();
		let config = enterprise_multiplex_config(&targets, *broken.address());
		let gw = AgentGateway::new(config).await?;
		let mcp_url = format!("http://localhost:{}/mcp", gw.port());

		Ok(Self {
			mcp_url,
			labels,
			stateful_labels,
			_gw: gw,
			_servers: servers,
			_broken: broken,
		})
	}
}

fn enterprise_multiplex_config(
	targets: &[(String, SocketAddr)],
	broken_addr: SocketAddr,
) -> String {
	let mut config = String::from(
		r#"config: {}
binds:
- port: $PORT
  listeners:
  - name: enterprise-multiplex-gateway
    routes:
    - matches:
      - path:
          pathPrefix: /mcp
      backends:
      - mcp:
          targets:
"#,
	);

	for (name, addr) in targets {
		config.push_str(&format!(
			"          - name: {name}\n            mcp:\n              host: http://{addr}/mcp\n"
		));
	}

	config.push_str(&format!(
		r#"          - name: dead
            mcp:
              host: http://{}/mcp
      policies:
        mcpAuthorization:
          rules:
          - 'true'
"#,
		broken_addr
	));

	config
}

fn assert_prefixed_by(name: &str, target: &str) {
	assert!(
		name.starts_with(&format!("{target}__")),
		"expected '{name}' to be prefixed by '{target}__'"
	);
}

fn assert_wrapped_uri_for_target(uri: &str, target: &str) {
	assert!(
		uri.starts_with(&format!("agw://{target}/")),
		"expected URI '{uri}' to be wrapped for target '{target}'"
	);
}

#[tokio::test]
async fn test_tools_aggregation_and_rbac_filtering() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let tools = client.list_tools(None).await?;
	let names = tools.tools.iter().map(|t| t.name.to_string()).collect_vec();
	let s1_echo = names
		.iter()
		.find(|name| name.as_str() == "s1__echo")
		.expect("s1__echo missing");
	assert_prefixed_by(s1_echo, "s1");
	assert!(
		!names.contains(&"s2__echo".into()),
		"RBAC failed to filter s2__echo"
	);
	assert!(
		!names.iter().any(|n| n.starts_with("s3__")),
		"Broken backend tools leaked into list"
	);

	let tool_resp = client
		.call_tool(CallToolRequestParams {
			name: "s1__echo".into(),
			arguments: Some(json!({"val": "hello"}).as_object().unwrap().clone()),
			meta: None,
			task: None,
		})
		.await?;

	let tool_val = serde_json::to_value(&tool_resp.content[0])?;
	assert_eq!(
		tool_val.get("text").and_then(|v| v.as_str()),
		Some("s1: hello")
	);
	Ok(())
}

#[tokio::test]
async fn test_resources_list_partial_success_with_one_failing_upstream() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let resources = client.list_resources(None).await?;
	let names = resources
		.resources
		.iter()
		.map(|r| r.name.to_string())
		.collect_vec();
	assert!(names.contains(&"s1__data".to_string()));
	assert!(names.contains(&"s2__data".to_string()));
	assert!(
		!names.iter().any(|name| name.starts_with("s3__")),
		"partial-success fanout failed; expected failing upstream s3 to be excluded"
	);
	Ok(())
}

#[tokio::test]
async fn test_tasks_lifecycle_multiplex() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let task_call = client
		.send_request(ClientRequest::CallToolRequest(CallToolRequest {
			method: Default::default(),
			params: CallToolRequestParams {
				meta: None,
				task: json!({}).as_object().cloned(),
				name: "s1__echo".into(),
				arguments: Some(json!({"val": "task-hello"}).as_object().unwrap().clone()),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_id = match task_call {
		ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult for task call, got: {:?}", other),
	};
	assert_prefixed_by(&task_id, "s1");

	let listed_tasks = client
		.send_request(ClientRequest::ListTasksRequest(ListTasksRequest {
			method: Default::default(),
			params: Some(PaginatedRequestParams {
				meta: None,
				cursor: None,
			}),
			extensions: Default::default(),
		}))
		.await?;
	let listed_tasks = match listed_tasks {
		ServerResult::ListTasksResult(result) => result,
		other => panic!("Expected ListTasksResult, got: {:?}", other),
	};
	assert!(
		listed_tasks
			.tasks
			.iter()
			.any(|task| task.task_id == task_id),
		"Task not returned in list/tasks response"
	);

	let task_info = client
		.send_request(ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
			method: Default::default(),
			params: GetTaskInfoParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_info = match task_info {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/info, got: {:?}", other),
	};
	assert_eq!(task_info.task_id, task_id);
	assert_eq!(task_info.status, TaskStatus::Completed);

	let task_payload = client
		.send_request(ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
			method: Default::default(),
			params: GetTaskResultParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_payload = match task_payload {
		ServerResult::CustomResult(result) => result.0,
		other => panic!("Expected CustomResult for tasks/result, got: {:?}", other),
	};
	assert_eq!(
		task_payload.get("tool").and_then(|v| v.as_str()),
		Some("echo")
	);
	assert_eq!(
		task_payload
			.get("arguments")
			.and_then(|v| v.get("val"))
			.and_then(|v| v.as_str()),
		Some("task-hello")
	);

	let task_cancel = client
		.send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest {
			method: Default::default(),
			params: CancelTaskParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_cancel = match task_cancel {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/cancel, got: {:?}", other),
	};
	assert_eq!(task_cancel.task_id, task_id);
	assert_eq!(task_cancel.status, TaskStatus::Cancelled);
	Ok(())
}

#[tokio::test]
async fn test_prompts_multiplex_roundtrip() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let prompts = client.list_prompts(None).await?;
	assert!(prompts.prompts.iter().any(|p| p.name == "s1__test_prompt"));

	let prompt_resp = client
		.get_prompt(GetPromptRequestParams {
			name: "s1__test_prompt".into(),
			arguments: Some(json!({"val": "world"}).as_object().unwrap().clone()),
			meta: None,
		})
		.await?;

	let prompt_val = serde_json::to_value(&prompt_resp.messages[0].content)?;
	assert_eq!(
		prompt_val.get("text").and_then(|v| v.as_str()),
		Some("val: world")
	);
	Ok(())
}

#[tokio::test]
async fn test_resources_multiplex_uri_wrapping_and_read_roundtrip() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let resources = client.list_resources(None).await?;
	let s1_res = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__data")
		.expect("s1__res missing");
	assert_wrapped_uri_for_target(&s1_res.uri, "s1");

	let s1_template = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__template")
		.expect("s1__template missing");
	assert!(
		s1_template.uri.contains("{id}"),
		"Braces were incorrectly encoded in template: {}",
		s1_template.uri
	);

	// Verify Reading through Wrapped URI
	let r_resp = client
		.read_resource(ReadResourceRequestParams {
			uri: s1_res.uri.clone(),
			meta: None,
		})
		.await?;

	let resource_val = serde_json::to_value(&r_resp.contents[0])?;
	assert_eq!(
		resource_val.get("text").and_then(|v| v.as_str()),
		Some("server-data")
	);
	Ok(())
}

#[tokio::test]
async fn test_elicitation_roundtrip_multiplex() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let e_resp = client
		.call_tool(CallToolRequestParams {
			name: "s1__elicitation".into(),
			arguments: None,
			meta: None,
			task: None,
		})
		.await?;
	assert_eq!(
		e_resp.structured_content.unwrap().get("color").unwrap(),
		"diamond"
	);
	Ok(())
}

#[tokio::test]
async fn test_resource_update_notification_after_subscribe() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let resources = client.list_resources(None).await?;
	let s1_res = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__data")
		.expect("s1__res missing");

	client
		.subscribe(SubscribeRequestParams {
			uri: s1_res.uri.clone(),
			meta: None,
		})
		.await?;

	client
		.call_tool(CallToolRequestParams {
			name: "s1__trigger_update".into(),
			arguments: None,
			meta: None,
			task: None,
		})
		.await?;

	let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
	while fixture.update_count.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < deadline {
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
	assert!(
		fixture.update_count.load(Ordering::SeqCst) > 0,
		"expected at least one resources/updated notification after subscribe"
	);

	Ok(())
}

#[tokio::test]
async fn test_multiplex_full_surface_all_supported_operations() -> anyhow::Result<()> {
	let fixture = MultiplexTestFixture::setup().await?;
	let client = &fixture.client;

	let tools = client.list_tools(None).await?;
	let tool_names = tools.tools.iter().map(|t| t.name.to_string()).collect_vec();
	assert!(tool_names.contains(&"s1__echo".to_string()));
	assert!(tool_names.contains(&"s1__elicitation".to_string()));
	assert!(tool_names.contains(&"s1__trigger_update".to_string()));
	assert!(tool_names.contains(&"s2__trigger_update".to_string()));
	assert!(
		!tool_names.contains(&"s2__echo".to_string()),
		"RBAC failed to filter s2__echo"
	);
	assert!(
		!tool_names.iter().any(|name| name.starts_with("s3__")),
		"Broken backend tools leaked into list"
	);

	let tool_resp = client
		.call_tool(CallToolRequestParams {
			name: "s1__echo".into(),
			arguments: Some(json!({"val": "full-flow"}).as_object().unwrap().clone()),
			meta: None,
			task: None,
		})
		.await?;
	let tool_val = serde_json::to_value(&tool_resp.content[0])?;
	assert_eq!(
		tool_val.get("text").and_then(|v| v.as_str()),
		Some("s1: full-flow")
	);

	let prompts = client.list_prompts(None).await?;
	assert!(prompts.prompts.iter().any(|p| p.name == "s1__test_prompt"));
	let prompt_resp = client
		.get_prompt(GetPromptRequestParams {
			name: "s1__test_prompt".into(),
			arguments: Some(json!({"val": "world"}).as_object().unwrap().clone()),
			meta: None,
		})
		.await?;
	let prompt_val = serde_json::to_value(&prompt_resp.messages[0].content)?;
	assert_eq!(
		prompt_val.get("text").and_then(|v| v.as_str()),
		Some("val: world")
	);

	let resources = client.list_resources(None).await?;
	let resource_names = resources
		.resources
		.iter()
		.map(|r| r.name.to_string())
		.collect_vec();
	assert!(resource_names.contains(&"s1__data".to_string()));
	assert!(resource_names.contains(&"s2__data".to_string()));
	assert!(
		!resource_names.iter().any(|name| name.starts_with("s3__")),
		"partial-success fanout failed; expected failing upstream s3 to be excluded"
	);
	let s1_data = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__data")
		.expect("s1__data missing");
	assert_wrapped_uri_for_target(&s1_data.uri, "s1");
	let s1_resource_template = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__template")
		.expect("s1__template missing");
	assert!(
		s1_resource_template.uri.contains("{id}"),
		"Braces were incorrectly encoded in template: {}",
		s1_resource_template.uri
	);

	let resource_read = client
		.read_resource(ReadResourceRequestParams {
			uri: s1_data.uri.clone(),
			meta: None,
		})
		.await?;
	let resource_val = serde_json::to_value(&resource_read.contents[0])?;
	assert_eq!(
		resource_val.get("text").and_then(|v| v.as_str()),
		Some("server-data")
	);

	let templates = client.list_resource_templates(None).await?;
	let template_names = templates
		.resource_templates
		.iter()
		.map(|t| t.name.to_string())
		.collect_vec();
	assert!(template_names.contains(&"s1__template".to_string()));
	assert!(template_names.contains(&"s2__template".to_string()));
	assert!(
		!template_names.iter().any(|name| name.starts_with("s3__")),
		"failing upstream resource templates leaked into list"
	);
	let s1_template = templates
		.resource_templates
		.iter()
		.find(|t| t.name == "s1__template")
		.expect("s1__template resource template missing");
	assert_wrapped_uri_for_target(&s1_template.uri_template, "s1");
	assert!(
		s1_template.uri_template.contains("{id}"),
		"Resource template braces were incorrectly encoded: {}",
		s1_template.uri_template
	);

	let prompt_completion = client
		.complete(CompleteRequestParams {
			meta: None,
			r#ref: Reference::for_prompt("s1__test_prompt"),
			argument: ArgumentInfo {
				name: "val".to_string(),
				value: "dial".to_string(),
			},
			context: None,
		})
		.await?;
	assert_eq!(
		prompt_completion.completion.values,
		vec!["s1:prompt:dial".to_string()]
	);

	let resource_completion = client
		.complete(CompleteRequestParams {
			meta: None,
			r#ref: Reference::for_resource(s1_data.uri.clone()),
			argument: ArgumentInfo {
				name: "id".to_string(),
				value: "abc".to_string(),
			},
			context: None,
		})
		.await?;
	assert_eq!(
		resource_completion.completion.values,
		vec!["s1:resource:abc".to_string()]
	);

	let task_call = client
		.send_request(ClientRequest::CallToolRequest(CallToolRequest {
			method: Default::default(),
			params: CallToolRequestParams {
				meta: None,
				task: json!({}).as_object().cloned(),
				name: "s1__echo".into(),
				arguments: Some(json!({"val": "task-hello"}).as_object().unwrap().clone()),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_id = match task_call {
		ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult for task call, got: {:?}", other),
	};
	assert_prefixed_by(&task_id, "s1");
	let listed_tasks = client
		.send_request(ClientRequest::ListTasksRequest(ListTasksRequest {
			method: Default::default(),
			params: Some(PaginatedRequestParams {
				meta: None,
				cursor: None,
			}),
			extensions: Default::default(),
		}))
		.await?;
	let listed_tasks = match listed_tasks {
		ServerResult::ListTasksResult(result) => result,
		other => panic!("Expected ListTasksResult, got: {:?}", other),
	};
	assert!(
		listed_tasks
			.tasks
			.iter()
			.any(|task| task.task_id == task_id),
		"Task not returned in list/tasks response"
	);
	let task_info = client
		.send_request(ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
			method: Default::default(),
			params: GetTaskInfoParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_info = match task_info {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/info, got: {:?}", other),
	};
	assert_eq!(task_info.task_id, task_id);
	assert_eq!(task_info.status, TaskStatus::Completed);
	let task_payload = client
		.send_request(ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
			method: Default::default(),
			params: GetTaskResultParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_payload = match task_payload {
		ServerResult::CustomResult(result) => result.0,
		other => panic!("Expected CustomResult for tasks/result, got: {:?}", other),
	};
	assert_eq!(
		task_payload.get("tool").and_then(|v| v.as_str()),
		Some("echo")
	);
	assert_eq!(
		task_payload
			.get("arguments")
			.and_then(|v| v.get("val"))
			.and_then(|v| v.as_str()),
		Some("task-hello")
	);
	let task_cancel = client
		.send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest {
			method: Default::default(),
			params: CancelTaskParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_cancel = match task_cancel {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/cancel, got: {:?}", other),
	};
	assert_eq!(task_cancel.task_id, task_id);
	assert_eq!(task_cancel.status, TaskStatus::Cancelled);

	let e_resp = client
		.call_tool(CallToolRequestParams {
			name: "s1__elicitation".into(),
			arguments: None,
			meta: None,
			task: None,
		})
		.await?;
	assert_eq!(
		e_resp.structured_content.unwrap().get("color").unwrap(),
		"diamond"
	);

	client
		.subscribe(SubscribeRequestParams {
			uri: s1_data.uri.clone(),
			meta: None,
		})
		.await?;
	client
		.call_tool(CallToolRequestParams {
			name: "s1__trigger_update".into(),
			arguments: None,
			meta: None,
			task: None,
		})
		.await?;
	let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
	while fixture.update_count.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < deadline {
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
	assert!(
		fixture.update_count.load(Ordering::SeqCst) > 0,
		"expected at least one resources/updated notification after subscribe"
	);
	client
		.unsubscribe(UnsubscribeRequestParams {
			uri: s1_data.uri.clone(),
			meta: None,
		})
		.await?;

	Ok(())
}

#[tokio::test]
async fn test_multiplex_transport_matrix_end_to_end() -> anyhow::Result<()> {
	let fixture = MultiplexTransportMatrixFixture::setup().await?;
	let client = &fixture.client;

	let tools = client.list_tools(None).await?;
	let tool_names = tools.tools.iter().map(|t| t.name.to_string()).collect_vec();
	assert!(tool_names.contains(&"stream__echo".to_string()));
	assert!(tool_names.contains(&"sse__echo".to_string()));
	assert!(tool_names.contains(&"stream__elicitation".to_string()));
	assert!(
		!tool_names.iter().any(|name| name.starts_with("s3__")),
		"Broken backend tools leaked into list"
	);

	let stream_echo = client
		.call_tool(CallToolRequestParams {
			name: "stream__echo".into(),
			arguments: Some(json!({"val": "hello-stream"}).as_object().unwrap().clone()),
			meta: None,
			task: None,
		})
		.await?;
	let stream_echo_val = serde_json::to_value(&stream_echo.content[0])?;
	assert_eq!(
		stream_echo_val.get("text").and_then(|v| v.as_str()),
		Some("stream: hello-stream")
	);

	let sse_echo = client
		.call_tool(CallToolRequestParams {
			name: "sse__echo".into(),
			arguments: Some(json!({"val": "hello-sse"}).as_object().unwrap().clone()),
			meta: None,
			task: None,
		})
		.await?;
	let sse_echo_val = serde_json::to_value(&sse_echo.content[0])?;
	assert_eq!(
		sse_echo_val.get("text").and_then(|v| v.as_str()),
		Some("sse: hello-sse")
	);

	let stream_elicitation = tokio::time::timeout(
		Duration::from_secs(10),
		client.call_tool(CallToolRequestParams {
			name: "stream__elicitation".into(),
			arguments: None,
			meta: None,
			task: None,
		}),
	)
	.await
	.map_err(|_| anyhow::anyhow!("timed out waiting for stream elicitation roundtrip"))??;
	assert_eq!(
		stream_elicitation
			.structured_content
			.unwrap()
			.get("color")
			.unwrap(),
		"diamond"
	);

	let prompts = client.list_prompts(None).await?;
	assert!(
		prompts
			.prompts
			.iter()
			.any(|p| p.name == "stream__test_prompt")
	);
	assert!(prompts.prompts.iter().any(|p| p.name == "sse__test_prompt"));

	let stream_prompt = client
		.get_prompt(GetPromptRequestParams {
			name: "stream__test_prompt".into(),
			arguments: Some(json!({"val": "world"}).as_object().unwrap().clone()),
			meta: None,
		})
		.await?;
	let stream_prompt_val = serde_json::to_value(&stream_prompt.messages[0].content)?;
	assert_eq!(
		stream_prompt_val.get("text").and_then(|v| v.as_str()),
		Some("val: world")
	);

	let sse_prompt = client
		.get_prompt(GetPromptRequestParams {
			name: "sse__test_prompt".into(),
			arguments: Some(json!({"val": "world"}).as_object().unwrap().clone()),
			meta: None,
		})
		.await?;
	let sse_prompt_val = serde_json::to_value(&sse_prompt.messages[0].content)?;
	assert_eq!(
		sse_prompt_val.get("text").and_then(|v| v.as_str()),
		Some("sse val: world")
	);

	let resources = client.list_resources(None).await?;
	let stream_res = resources
		.resources
		.iter()
		.find(|r| r.name == "stream__data")
		.expect("stream__data missing");
	let sse_res = resources
		.resources
		.iter()
		.find(|r| r.name == "sse__data")
		.expect("sse__data missing");
	assert_wrapped_uri_for_target(&stream_res.uri, "stream");
	assert_wrapped_uri_for_target(&sse_res.uri, "sse");

	let stream_read = client
		.read_resource(ReadResourceRequestParams {
			uri: stream_res.uri.clone(),
			meta: None,
		})
		.await?;
	let stream_read_val = serde_json::to_value(&stream_read.contents[0])?;
	assert_eq!(
		stream_read_val.get("text").and_then(|v| v.as_str()),
		Some("server-data")
	);

	let sse_read = client
		.read_resource(ReadResourceRequestParams {
			uri: sse_res.uri.clone(),
			meta: None,
		})
		.await?;
	let sse_read_val = serde_json::to_value(&sse_read.contents[0])?;
	assert_eq!(
		sse_read_val.get("text").and_then(|v| v.as_str()),
		Some("sse-server-data")
	);

	let templates = client.list_resource_templates(None).await?;
	let stream_template = templates
		.resource_templates
		.iter()
		.find(|t| t.name == "stream__template")
		.expect("stream__template missing");
	let sse_template = templates
		.resource_templates
		.iter()
		.find(|t| t.name == "sse__template")
		.expect("sse__template missing");
	assert_wrapped_uri_for_target(&stream_template.uri_template, "stream");
	assert_wrapped_uri_for_target(&sse_template.uri_template, "sse");
	assert!(stream_template.uri_template.contains("{id}"));
	assert!(sse_template.uri_template.contains("{id}"));

	let stream_prompt_completion = client
		.complete(CompleteRequestParams {
			meta: None,
			r#ref: Reference::for_prompt("stream__test_prompt"),
			argument: ArgumentInfo {
				name: "val".to_string(),
				value: "dial".to_string(),
			},
			context: None,
		})
		.await?;
	assert_eq!(
		stream_prompt_completion.completion.values,
		vec!["stream:prompt:dial".to_string()]
	);

	let stream_resource_completion = client
		.complete(CompleteRequestParams {
			meta: None,
			r#ref: Reference::for_resource(stream_res.uri.clone()),
			argument: ArgumentInfo {
				name: "id".to_string(),
				value: "abc".to_string(),
			},
			context: None,
		})
		.await?;
	assert_eq!(
		stream_resource_completion.completion.values,
		vec!["stream:resource:abc".to_string()]
	);

	let task_call = client
		.send_request(ClientRequest::CallToolRequest(CallToolRequest {
			method: Default::default(),
			params: CallToolRequestParams {
				meta: None,
				task: json!({}).as_object().cloned(),
				name: "stream__echo".into(),
				arguments: Some(json!({"val": "task-hello"}).as_object().unwrap().clone()),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_id = match task_call {
		ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult for task call, got: {:?}", other),
	};
	assert_prefixed_by(&task_id, "stream");

	let task_info = client
		.send_request(ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
			method: Default::default(),
			params: GetTaskInfoParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_info = match task_info {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/info, got: {:?}", other),
	};
	assert_eq!(task_info.task_id, task_id);
	assert_eq!(task_info.status, TaskStatus::Completed);

	let task_payload = client
		.send_request(ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
			method: Default::default(),
			params: GetTaskResultParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_payload = match task_payload {
		ServerResult::CustomResult(result) => result.0,
		other => panic!("Expected CustomResult for tasks/result, got: {:?}", other),
	};
	assert_eq!(
		task_payload.get("tool").and_then(|v| v.as_str()),
		Some("echo")
	);

	let task_cancel = client
		.send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest {
			method: Default::default(),
			params: CancelTaskParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_cancel = match task_cancel {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/cancel, got: {:?}", other),
	};
	assert_eq!(task_cancel.task_id, task_id);
	assert_eq!(task_cancel.status, TaskStatus::Cancelled);

	client
		.subscribe(SubscribeRequestParams {
			uri: stream_res.uri.clone(),
			meta: None,
		})
		.await?;
	client
		.call_tool(CallToolRequestParams {
			name: "stream__trigger_update".into(),
			arguments: None,
			meta: None,
			task: None,
		})
		.await?;
	let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
	while fixture.update_count.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < deadline {
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
	assert!(
		fixture.update_count.load(Ordering::SeqCst) > 0,
		"expected at least one resources/updated notification after subscribe"
	);
	client
		.unsubscribe(UnsubscribeRequestParams {
			uri: stream_res.uri.clone(),
			meta: None,
		})
		.await?;

	Ok(())
}

#[tokio::test]
#[ignore = "enterprise load test; run manually with RUST_MIN_STACK=8388608"]
async fn test_enterprise_multiplex_concurrent_load_20_upstreams() -> anyhow::Result<()> {
	const TARGETS: usize = 20;
	const WORKERS: usize = 40;
	const ITERATIONS_PER_WORKER: usize = 14;

	let fixture = EnterpriseMultiplexFixture::setup(TARGETS).await?;
	let labels = Arc::new(fixture.labels.clone());
	let stateful_labels = Arc::new(fixture.stateful_labels.iter().cloned().collect_vec());
	let url = fixture.mcp_url.clone();

	let mut handles = Vec::with_capacity(WORKERS);
	for worker_idx in 0..WORKERS {
		let labels = labels.clone();
		let stateful_labels = stateful_labels.clone();
		let url = url.clone();
		handles.push(tokio::spawn(async move {
			let update_count = Arc::new(AtomicUsize::new(0));
			let client = setup_comprehensive_client(&url, update_count).await?;
			for iter_idx in 0..ITERATIONS_PER_WORKER {
				let target_idx = (worker_idx + iter_idx) % labels.len();
				let mut target = labels[target_idx].clone();
				let input = format!("w{worker_idx}-i{iter_idx}");
				let op_case = (worker_idx + iter_idx) % 7;
				if op_case == 6 {
					let stateful_idx = (worker_idx + iter_idx) % stateful_labels.len();
					target = stateful_labels[stateful_idx].clone();
				}
				let op = async {
					match op_case {
						// tools/call
						0 => {
							let resp = client
								.call_tool(CallToolRequestParams {
									name: format!("{target}__echo").into(),
									arguments: Some(json!({"val": input}).as_object().unwrap().clone()),
									meta: None,
									task: None,
								})
								.await?;
							let value = serde_json::to_value(&resp.content[0])?;
							assert_eq!(
								value.get("text").and_then(|v| v.as_str()),
								Some(format!("{target}: w{worker_idx}-i{iter_idx}").as_str())
							);
						},
						// prompts/list + prompts/get
						1 => {
							let prompts = client.list_prompts(None).await?;
							assert!(
								prompts
									.prompts
									.iter()
									.any(|p| p.name == format!("{target}__test_prompt")),
								"missing prompt for target {target}"
							);
							let prompt = client
								.get_prompt(GetPromptRequestParams {
									name: format!("{target}__test_prompt"),
									arguments: Some(json!({"val": input}).as_object().unwrap().clone()),
									meta: None,
								})
								.await?;
							let value = serde_json::to_value(&prompt.messages[0].content)?;
							assert_eq!(
								value.get("text").and_then(|v| v.as_str()),
								Some(format!("val: {input}").as_str())
							);
						},
						// resources/list + resources/read
						2 => {
							let resources = client.list_resources(None).await?;
							let resource = resources
								.resources
								.iter()
								.find(|r| r.name == format!("{target}__data"))
								.expect("target resource should exist");
							let read = client
								.read_resource(ReadResourceRequestParams {
									uri: resource.uri.clone(),
									meta: None,
								})
								.await?;
							let value = serde_json::to_value(&read.contents[0])?;
							assert_eq!(
								value.get("text").and_then(|v| v.as_str()),
								Some("server-data")
							);
						},
						// resources/templates list
						3 => {
							let templates = client.list_resource_templates(None).await?;
							assert!(
								templates
									.resource_templates
									.iter()
									.any(|t| t.name == format!("{target}__template")),
								"missing resource template for target {target}"
							);
						},
						// completion on prompt
						4 => {
							let completion = client
								.complete(CompleteRequestParams {
									meta: None,
									r#ref: Reference::for_prompt(format!("{target}__test_prompt")),
									argument: ArgumentInfo {
										name: "val".to_string(),
										value: input.clone(),
									},
									context: None,
								})
								.await?;
							assert_eq!(
								completion.completion.values,
								vec![format!("{target}:prompt:{input}")]
							);
						},
						// completion on resource
						5 => {
							let resources = client.list_resources(None).await?;
							let resource = resources
								.resources
								.iter()
								.find(|r| r.name == format!("{target}__data"))
								.expect("target resource should exist");
							let completion = client
								.complete(CompleteRequestParams {
									meta: None,
									r#ref: Reference::for_resource(resource.uri.clone()),
									argument: ArgumentInfo {
										name: "id".to_string(),
										value: input.clone(),
									},
									context: None,
								})
								.await?;
							assert_eq!(
								completion.completion.values,
								vec![format!("{target}:resource:{input}")]
							);
						},
						// task lifecycle
						6 => {
							let task_call = client
								.send_request(ClientRequest::CallToolRequest(CallToolRequest {
									method: Default::default(),
									params: CallToolRequestParams {
										meta: None,
										task: json!({}).as_object().cloned(),
										name: format!("{target}__echo").into(),
										arguments: Some(json!({"val": input}).as_object().unwrap().clone()),
									},
									extensions: Default::default(),
								}))
								.await?;
							let task_id = match task_call {
								ServerResult::CreateTaskResult(result) => result.task.task_id,
								other => panic!("expected CreateTaskResult, got {other:?}"),
							};
							assert_prefixed_by(&task_id, &target);

							let task_info = client
								.send_request(ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
									method: Default::default(),
									params: GetTaskInfoParams {
										meta: None,
										task_id: task_id.clone(),
									},
									extensions: Default::default(),
								}))
								.await?;
							let task_info = match task_info {
								ServerResult::GetTaskResult(result) => result.task,
								other => panic!("expected GetTaskResult, got {other:?}"),
							};
							assert_eq!(task_info.task_id, task_id);

							let task_result = client
								.send_request(ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
									method: Default::default(),
									params: GetTaskResultParams {
										meta: None,
										task_id: task_id.clone(),
									},
									extensions: Default::default(),
								}))
								.await?;
							let payload = match task_result {
								ServerResult::CustomResult(result) => result.0,
								other => panic!("expected CustomResult, got {other:?}"),
							};
							assert_eq!(payload.get("tool").and_then(|v| v.as_str()), Some("echo"));

							let cancel = client
								.send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest {
									method: Default::default(),
									params: CancelTaskParams {
										meta: None,
										task_id: task_id.clone(),
									},
									extensions: Default::default(),
								}))
								.await?;
							let cancelled = match cancel {
								ServerResult::GetTaskResult(result) => result.task,
								other => panic!("expected GetTaskResult from cancel, got {other:?}"),
							};
							assert_eq!(cancelled.task_id, task_id);
							assert_eq!(cancelled.status, TaskStatus::Cancelled);
						},
						_ => unreachable!("op_case is computed as modulo 7"),
					}
					anyhow::Ok(())
				};

				tokio::time::timeout(Duration::from_secs(15), op)
					.await
					.map_err(|_| {
						anyhow::anyhow!("timed out worker={worker_idx} iter={iter_idx} op_case={op_case}")
					})??;
			}
			anyhow::Ok(())
		}));
	}

	for handle in handles {
		handle.await??;
	}

	Ok(())
}

#[tokio::test]
#[ignore = "enterprise interactive load test; run manually with RUST_MIN_STACK=8388608"]
async fn test_enterprise_multiplex_interactive_load_20_upstreams() -> anyhow::Result<()> {
	const TARGETS: usize = 20;
	const WORKERS: usize = 12;
	const ITERATIONS_PER_WORKER: usize = 6;

	let fixture = EnterpriseMultiplexFixture::setup(TARGETS).await?;
	let stateful_labels = Arc::new(fixture.stateful_labels.iter().cloned().collect_vec());
	let url = fixture.mcp_url.clone();

	let mut handles = Vec::with_capacity(WORKERS);
	for worker_idx in 0..WORKERS {
		let stateful_labels = stateful_labels.clone();
		let url = url.clone();
		handles.push(tokio::spawn(async move {
			let update_count = Arc::new(AtomicUsize::new(0));
			let client = setup_comprehensive_client(&url, update_count.clone()).await?;
			for iter_idx in 0..ITERATIONS_PER_WORKER {
				let target_idx = (worker_idx + iter_idx) % stateful_labels.len();
				let target = stateful_labels[target_idx].clone();
				let op_case = (worker_idx + iter_idx) % 2;
				let op = async {
					match op_case {
						// elicitation
						0 => {
							let resp = client
								.call_tool(CallToolRequestParams {
									name: format!("{target}__elicitation").into(),
									arguments: None,
									meta: None,
									task: None,
								})
								.await?;
							assert_eq!(
								resp
									.structured_content
									.as_ref()
									.and_then(|c| c.get("color"))
									.and_then(|v| v.as_str()),
								Some("diamond")
							);
						},
						// subscribe + notify + unsubscribe
						_ => {
							let resources = client.list_resources(None).await?;
							let resource = resources
								.resources
								.iter()
								.find(|r| r.name == format!("{target}__data"))
								.expect("target resource should exist");
							let before_updates = update_count.load(Ordering::SeqCst);
							client
								.subscribe(SubscribeRequestParams {
									uri: resource.uri.clone(),
									meta: None,
								})
								.await?;
							client
								.call_tool(CallToolRequestParams {
									name: format!("{target}__trigger_update").into(),
									arguments: None,
									meta: None,
									task: None,
								})
								.await?;
							let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
							while update_count.load(Ordering::SeqCst) == before_updates
								&& tokio::time::Instant::now() < deadline
							{
								tokio::time::sleep(Duration::from_millis(25)).await;
							}
							assert!(
								update_count.load(Ordering::SeqCst) > before_updates,
								"expected at least one resources/updated notification for {target}"
							);
							client
								.unsubscribe(UnsubscribeRequestParams {
									uri: resource.uri.clone(),
									meta: None,
								})
								.await?;
						},
					}
					anyhow::Ok(())
				};

				tokio::time::timeout(Duration::from_secs(20), op)
					.await
					.map_err(|_| {
						anyhow::anyhow!(
							"timed out interactive worker={worker_idx} iter={iter_idx} op_case={op_case}"
						)
					})??;
			}
			anyhow::Ok(())
		}));
	}

	for handle in handles {
		handle.await??;
	}

	Ok(())
}
