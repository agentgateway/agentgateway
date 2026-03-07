use super::*;

#[tokio::test]
async fn test_tools_aggregation_and_rbac_filtering() -> anyhow::Result<()> {
	let fixture = MultiplexFixture::setup().await?;
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
		.call_tool(call_tool_params(
			"s1__echo",
			Some(json!({"val": "hello"}).as_object().unwrap().clone()),
			None,
		))
		.await?;

	let tool_val = serde_json::to_value(&tool_resp.content[0])?;
	assert_eq!(
		tool_val.get("text").and_then(|v| v.as_str()),
		Some("s1: hello")
	);
	Ok(())
}

#[tokio::test]
async fn test_tools_multiplex_preserves_upstream_meta_labels() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();
	let left = start_mock_mcp_meta_server("mcp", true).await;
	let right = start_mock_mcp_meta_server("sse", true).await;
	let gw = AgentGateway::new(simple_multiplex_config(
		"meta-gateway",
		&[
			("mcp".to_string(), left.addr),
			("sse".to_string(), right.addr),
		],
	))
	.await?;
	let client = setup_default_client_for_gateway(&gw).await?;

	let tools = client.list_tools(None).await?;
	let meta = tools.meta.expect("merged meta should be present");
	let upstreams = meta
		.0
		.get("upstreams")
		.and_then(|v| v.as_object())
		.expect("meta.upstreams");
	let mcp_label = upstreams
		.get("mcp")
		.and_then(|v| v.get("label"))
		.and_then(|v| v.as_str());
	let sse_label = upstreams
		.get("sse")
		.and_then(|v| v.get("label"))
		.and_then(|v| v.as_str());
	assert_eq!(mcp_label, Some("mcp"));
	assert_eq!(sse_label, Some("sse"));
	Ok(())
}

#[tokio::test]
async fn test_resources_list_partial_success_with_one_failing_upstream() -> anyhow::Result<()> {
	let fixture = MultiplexFixture::setup().await?;
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
	let fixture = MultiplexFixture::setup().await?;
	let client = &fixture.client;

	let task_call = client
		.send_request(ClientRequest::CallToolRequest(CallToolRequest::new(
			call_tool_params(
				"s1__echo",
				Some(json!({"val": "task-hello"}).as_object().unwrap().clone()),
				Some(JsonObject::default()),
			),
		)))
		.await?;
	let task_id = match task_call {
		ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult for task call, got: {:?}", other),
	};
	assert_prefixed_by(&task_id, "s1");

	let listed_tasks = client
		.send_request(ClientRequest::ListTasksRequest(ListTasksRequest {
			method: Default::default(),
			params: Some(PaginatedRequestParams::default()),
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
		.send_request(ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest::new(
			GetTaskInfoParams {
				meta: None,
				task_id: task_id.clone(),
			},
		)))
		.await?;
	let task_info = match task_info {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/info, got: {:?}", other),
	};
	assert_eq!(task_info.task_id, task_id);
	assert_eq!(task_info.status, TaskStatus::Completed);

	let task_payload = client
		.send_request(ClientRequest::GetTaskResultRequest(
			GetTaskResultRequest::new(GetTaskResultParams {
				meta: None,
				task_id: task_id.clone(),
			}),
		))
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
		.send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest::new(
			CancelTaskParams {
				meta: None,
				task_id: task_id.clone(),
			},
		)))
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
async fn test_prompts_list_errors_when_all_targeted_upstreams_fail() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();
	let meta = start_mock_mcp_meta_server("meta", true).await;
	let dead_addr = {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
		let addr = listener.local_addr()?;
		drop(listener);
		addr
	};

	let gw = AgentGateway::new(simple_multiplex_config(
		"prompt-fail-gateway",
		&[
			("meta".to_string(), meta.addr),
			("dead".to_string(), dead_addr),
		],
	))
	.await?;
	let client = setup_default_client_for_gateway(&gw).await?;

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
			let report = json!({
				"scenario": "multiplex/prompts_list_all_targets_fail",
				"steps": [
					{
						"action": "list_prompts",
						"targets": ["meta", "dead"],
						"outcome": "mcp_error",
					},
					{
						"action": "error",
						"code": mcp_error.code.0,
						"message": mcp_error.message,
					},
				],
			});
			assert_mcp_json_snapshot("multiplex/prompts_list_all_targets_fail", report);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}

	Ok(())
}

#[tokio::test]
async fn test_prompts_multiplex_roundtrip() -> anyhow::Result<()> {
	let fixture = MultiplexFixture::setup().await?;
	let client = &fixture.client;

	let prompts = client.list_prompts(None).await?;
	assert!(prompts.prompts.iter().any(|p| p.name == "s1__test_prompt"));

	let prompt_resp = client
		.get_prompt(
			GetPromptRequestParams::new("s1__test_prompt")
				.with_arguments(json!({"val": "world"}).as_object().unwrap().clone()),
		)
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
	let fixture = MultiplexFixture::setup().await?;
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
		.read_resource(ReadResourceRequestParams::new(s1_res.uri.clone()))
		.await?;

	let resource_val = serde_json::to_value(&r_resp.contents[0])?;
	assert_eq!(
		resource_val.get("text").and_then(|v| v.as_str()),
		Some("server-data")
	);
	Ok(())
}

#[tokio::test]
async fn test_multiplex_full_surface_all_supported_operations() -> anyhow::Result<()> {
	let fixture = MultiplexFixture::setup().await?;
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
		.call_tool(call_tool_params(
			"s1__echo",
			Some(json!({"val": "full-flow"}).as_object().unwrap().clone()),
			None,
		))
		.await?;
	let tool_val = serde_json::to_value(&tool_resp.content[0])?;
	assert_eq!(
		tool_val.get("text").and_then(|v| v.as_str()),
		Some("s1: full-flow")
	);

	let prompts = client.list_prompts(None).await?;
	assert!(prompts.prompts.iter().any(|p| p.name == "s1__test_prompt"));
	let prompt_resp = client
		.get_prompt(
			GetPromptRequestParams::new("s1__test_prompt")
				.with_arguments(json!({"val": "world"}).as_object().unwrap().clone()),
		)
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
		.read_resource(ReadResourceRequestParams::new(s1_data.uri.clone()))
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
		.complete(CompleteRequestParams::new(
			Reference::for_prompt("s1__test_prompt"),
			ArgumentInfo {
				name: "val".to_string(),
				value: "dial".to_string(),
			},
		))
		.await?;
	assert_eq!(
		prompt_completion.completion.values,
		vec!["s1:prompt:dial".to_string()]
	);

	let resource_completion = client
		.complete(CompleteRequestParams::new(
			Reference::for_resource(s1_data.uri.clone()),
			ArgumentInfo {
				name: "id".to_string(),
				value: "abc".to_string(),
			},
		))
		.await?;
	assert_eq!(
		resource_completion.completion.values,
		vec!["s1:resource:abc".to_string()]
	);

	let task_call = client
		.send_request(ClientRequest::CallToolRequest(CallToolRequest::new(
			call_tool_params(
				"s1__echo",
				Some(json!({"val": "task-hello"}).as_object().unwrap().clone()),
				Some(JsonObject::default()),
			),
		)))
		.await?;
	let task_id = match task_call {
		ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult for task call, got: {:?}", other),
	};
	assert_prefixed_by(&task_id, "s1");
	let listed_tasks = client
		.send_request(ClientRequest::ListTasksRequest(ListTasksRequest {
			method: Default::default(),
			params: Some(PaginatedRequestParams::default()),
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
		.send_request(ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest::new(
			GetTaskInfoParams {
				meta: None,
				task_id: task_id.clone(),
			},
		)))
		.await?;
	let task_info = match task_info {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/info, got: {:?}", other),
	};
	assert_eq!(task_info.task_id, task_id);
	assert_eq!(task_info.status, TaskStatus::Completed);
	let task_payload = client
		.send_request(ClientRequest::GetTaskResultRequest(
			GetTaskResultRequest::new(GetTaskResultParams {
				meta: None,
				task_id: task_id.clone(),
			}),
		))
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
		.send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest::new(
			CancelTaskParams {
				meta: None,
				task_id: task_id.clone(),
			},
		)))
		.await?;
	let task_cancel = match task_cancel {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/cancel, got: {:?}", other),
	};
	assert_eq!(task_cancel.task_id, task_id);
	assert_eq!(task_cancel.status, TaskStatus::Cancelled);

	let e_resp = client
		.call_tool(call_tool_params("s1__elicitation", None, None))
		.await?;
	assert_eq!(
		e_resp.structured_content.unwrap().get("color").unwrap(),
		"diamond"
	);

	client
		.subscribe(SubscribeRequestParams::new(s1_data.uri.clone()))
		.await?;
	client
		.call_tool(call_tool_params("s1__trigger_update", None, None))
		.await?;
	wait_for_resource_updates(&fixture.update_count).await?;
	client
		.unsubscribe(unsubscribe_request_params(s1_data.uri.clone()))
		.await?;

	Ok(())
}
