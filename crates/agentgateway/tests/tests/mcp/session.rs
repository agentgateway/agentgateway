use super::*;

#[tokio::test]
async fn test_multiplex_session_restart_resume() -> anyhow::Result<()> {
	let listener_name = "restart-handoff";
	let s1 = start_mock_mcp_tools_only_prompt_leak_server("s1", true).await;
	let s2 = start_mock_mcp_server("s2", true).await;
	let dead_s3_addr = unused_loopback_addr().await?;
	let gw_a = AgentGateway::new(simple_multiplex_config(
		listener_name,
		&[
			("s1".to_string(), s1.addr),
			("s2".to_string(), s2.addr),
			("s3".to_string(), dead_s3_addr),
		],
	))
	.await?;

	let initialize_response = gw_a
		.send_request_json_with_headers(
			"http://localhost/mcp",
			initialize_request_message(1),
			&mcp_post_headers(None),
		)
		.await;
	let session_id = initialize_response
		.headers()
		.get(HEADER_SESSION_ID)
		.and_then(|v| v.to_str().ok())
		.map(ToOwned::to_owned)
		.expect("initialize should mint an MCP session id");
	let initialize_message = read_server_message(initialize_response).await?;
	let initialize_result = match initialize_message {
		ServerJsonRpcMessage::Response(resp) => match resp.result {
			ServerResult::InitializeResult(result) => result,
			other => panic!("expected initialize result, got: {other:?}"),
		},
		other => panic!("expected initialize response, got: {other:?}"),
	};
	assert!(
		initialize_result.capabilities.tools.is_some(),
		"merged initialize should retain tool capability"
	);
	assert!(
		initialize_result.capabilities.prompts.is_some(),
		"merged initialize should retain prompt capability"
	);

	let initialized_response = gw_a
		.send_request_json_with_headers(
			"http://localhost/mcp",
			initialized_notification_message(),
			&mcp_post_headers(Some(&session_id)),
		)
		.await;
	assert!(
		initialized_response.status().is_success(),
		"initialized notification should be accepted"
	);
	gw_a.shutdown().await;

	let s3_recovered = start_mock_mcp_tools_only_prompt_leak_server("s3", true).await;
	let gw_b = AgentGateway::new(simple_multiplex_config(
		listener_name,
		&[
			("s1".to_string(), s1.addr),
			("s2".to_string(), s2.addr),
			("s3".to_string(), s3_recovered.addr),
		],
	))
	.await?;

	let prompts_response = gw_b
		.send_request_json_with_headers(
			"http://localhost/mcp",
			list_prompts_request_message(2),
			&mcp_post_headers(Some(&session_id)),
		)
		.await;
	let prompts_message = read_server_message(prompts_response).await?;
	let prompts_result = match prompts_message {
		ServerJsonRpcMessage::Response(resp) => match resp.result {
			ServerResult::ListPromptsResult(result) => result,
			other => panic!("expected list prompts result, got: {other:?}"),
		},
		other => panic!("expected list prompts response, got: {other:?}"),
	};
	let prompt_names = prompts_result
		.prompts
		.iter()
		.map(|prompt| prompt.name.to_string())
		.collect_vec();

	let report = json!({
		"scenario": "session/restart_resume",
		"steps": [
			{
				"gateway": "a",
				"action": "initialize",
				"session_id": session_id,
				"capabilities": {
					"prompts": initialize_result.capabilities.prompts.is_some(),
					"resources": initialize_result.capabilities.resources.is_some(),
					"tools": initialize_result.capabilities.tools.is_some(),
					"tasks": initialize_result.capabilities.tasks.is_some(),
				},
			},
			{
				"gateway": "a",
				"action": "initialized_notification",
				"accepted": true,
			},
			{
				"gateway": "b",
				"action": "list_prompts_after_resume",
				"session_id": session_id,
				"prompt_names": prompt_names,
			},
		],
	});
	assert_mcp_json_snapshot("session/restart_resume", report);

	Ok(())
}
