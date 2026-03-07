use super::*;

#[tokio::test]
async fn test_multiplex_transport_matrix_end_to_end() -> anyhow::Result<()> {
	let fixture = TransportMatrixFixture::setup().await?;
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
		.call_tool(call_tool_params(
			"stream__echo",
			Some(json!({"val": "hello-stream"}).as_object().unwrap().clone()),
			None,
		))
		.await?;
	let stream_echo_val = serde_json::to_value(&stream_echo.content[0])?;
	assert_eq!(
		stream_echo_val.get("text").and_then(|v| v.as_str()),
		Some("stream: hello-stream")
	);

	let sse_echo = client
		.call_tool(call_tool_params(
			"sse__echo",
			Some(json!({"val": "hello-sse"}).as_object().unwrap().clone()),
			None,
		))
		.await?;
	let sse_echo_val = serde_json::to_value(&sse_echo.content[0])?;
	assert_eq!(
		sse_echo_val.get("text").and_then(|v| v.as_str()),
		Some("sse: hello-sse")
	);

	let stream_elicitation = tokio::time::timeout(
		Duration::from_secs(10),
		client.call_tool(call_tool_params("stream__elicitation", None, None)),
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
		.get_prompt(
			GetPromptRequestParams::new("stream__test_prompt")
				.with_arguments(json!({"val": "world"}).as_object().unwrap().clone()),
		)
		.await?;
	let stream_prompt_val = serde_json::to_value(&stream_prompt.messages[0].content)?;
	assert_eq!(
		stream_prompt_val.get("text").and_then(|v| v.as_str()),
		Some("val: world")
	);

	let sse_prompt = client
		.get_prompt(
			GetPromptRequestParams::new("sse__test_prompt")
				.with_arguments(json!({"val": "world"}).as_object().unwrap().clone()),
		)
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
		.read_resource(ReadResourceRequestParams::new(stream_res.uri.clone()))
		.await?;
	let stream_read_val = serde_json::to_value(&stream_read.contents[0])?;
	assert_eq!(
		stream_read_val.get("text").and_then(|v| v.as_str()),
		Some("server-data")
	);

	let sse_read = client
		.read_resource(ReadResourceRequestParams::new(sse_res.uri.clone()))
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
		.complete(CompleteRequestParams::new(
			Reference::for_prompt("stream__test_prompt"),
			ArgumentInfo {
				name: "val".to_string(),
				value: "dial".to_string(),
			},
		))
		.await?;
	assert_eq!(
		stream_prompt_completion.completion.values,
		vec!["stream:prompt:dial".to_string()]
	);

	let stream_resource_completion = client
		.complete(CompleteRequestParams::new(
			Reference::for_resource(stream_res.uri.clone()),
			ArgumentInfo {
				name: "id".to_string(),
				value: "abc".to_string(),
			},
		))
		.await?;
	assert_eq!(
		stream_resource_completion.completion.values,
		vec!["stream:resource:abc".to_string()]
	);

	let task_call = client
		.send_request(ClientRequest::CallToolRequest(CallToolRequest::new(
			call_tool_params(
				"stream__echo",
				Some(json!({"val": "task-hello"}).as_object().unwrap().clone()),
				Some(JsonObject::default()),
			),
		)))
		.await?;
	let task_id = match task_call {
		ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult for task call, got: {:?}", other),
	};
	assert_prefixed_by(&task_id, "stream");

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

	client
		.subscribe(SubscribeRequestParams::new(stream_res.uri.clone()))
		.await?;
	client
		.call_tool(call_tool_params("stream__trigger_update", None, None))
		.await?;
	wait_for_resource_updates(&fixture.update_count).await?;
	client
		.unsubscribe(unsubscribe_request_params(stream_res.uri.clone()))
		.await?;

	Ok(())
}
