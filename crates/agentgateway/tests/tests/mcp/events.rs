use super::*;

#[tokio::test]
async fn test_resource_update_notification_after_subscribe() -> anyhow::Result<()> {
	let fixture = MultiplexFixture::setup().await?;
	let client = &fixture.client;

	let resources = client.list_resources(None).await?;
	let s1_res = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__data")
		.expect("s1__res missing");

	client
		.subscribe(SubscribeRequestParams::new(s1_res.uri.clone()))
		.await?;

	client
		.call_tool(call_tool_params("s1__trigger_update", None, None))
		.await?;
	wait_for_resource_updates(&fixture.update_count).await?;

	Ok(())
}

#[tokio::test]
async fn test_streamable_http_last_event_id_replays_buffered_notification() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();

	let s1 = start_mock_mcp_server("s1", true).await;
	let s2 = start_mock_mcp_server("s2", false).await;
	let s3_mock = MockServer::start().await;
	Mock::given(method("POST"))
		.respond_with(ResponseTemplate::new(500).set_body_string("CRASH"))
		.mount(&s3_mock)
		.await;

	let gw = AgentGateway::new(multiplex_config(&s1, &s2, *s3_mock.address())).await?;

	let initialize_response = gw
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
	let _initialize_message = read_server_message(initialize_response).await?;

	let initialized_response = gw
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

	let resources_response = gw
		.send_request_json_with_headers(
			"http://localhost/mcp",
			list_resources_request_message(2),
			&mcp_post_headers(Some(&session_id)),
		)
		.await;
	let resources_message = read_server_message(resources_response).await?;
	let resources_result = match resources_message {
		ServerJsonRpcMessage::Response(resp) => match resp.result {
			ServerResult::ListResourcesResult(result) => result,
			other => panic!("expected list resources result, got: {other:?}"),
		},
		other => panic!("expected list resources response, got: {other:?}"),
	};
	let s1_resource = resources_result
		.resources
		.iter()
		.find(|resource| resource.name == "s1__data")
		.cloned()
		.expect("s1__data resource missing");

	let subscribe_response = gw
		.send_request_json_with_headers(
			"http://localhost/mcp",
			subscribe_request_message(3, s1_resource.uri.clone()),
			&mcp_post_headers(Some(&session_id)),
		)
		.await;
	let _subscribe_message = read_server_message(subscribe_response).await?;

	let first_stream = open_event_stream(gw.port(), &session_id, None).await?;
	let first_event_task = tokio::spawn(async move { read_first_sse_message(first_stream).await });

	let first_update_response = gw
		.send_request_json_with_headers(
			"http://localhost/mcp",
			call_tool_request_message(4, "s1__trigger_update", None),
			&mcp_post_headers(Some(&session_id)),
		)
		.await;
	let _first_update_message = read_server_message(first_update_response).await?;

	let first_event = first_event_task.await??;
	assert_eq!(
		resource_updated_uri(&first_event.message)?,
		s1_resource.uri.as_str(),
		"first event should target the subscribed wrapped URI"
	);

	let second_update_response = gw
		.send_request_json_with_headers(
			"http://localhost/mcp",
			call_tool_request_message(5, "s1__trigger_update", None),
			&mcp_post_headers(Some(&session_id)),
		)
		.await;
	let _second_update_message = read_server_message(second_update_response).await?;
	tokio::time::sleep(Duration::from_millis(100)).await;

	let replay_stream =
		open_event_stream(gw.port(), &session_id, Some(&first_event.event_id)).await?;
	let replayed_event = read_first_sse_message(replay_stream).await?;

	assert_ne!(
		replayed_event.event_id, first_event.event_id,
		"replayed event should advance beyond the cursor"
	);
	assert_eq!(
		resource_updated_uri(&replayed_event.message)?,
		s1_resource.uri.as_str(),
		"replayed event should target the subscribed wrapped URI"
	);

	let report = json!({
		"scenario": "events/replay_buffered_notification",
		"steps": [
			{
				"action": "initialize",
				"session_id": session_id,
			},
			{
				"action": "list_resources",
				"selected_resource": {
					"name": s1_resource.name,
					"uri": s1_resource.uri,
				},
			},
			{
				"action": "subscribe",
				"uri": s1_resource.uri,
			},
			{
				"action": "first_event",
				"event_id": first_event.event_id,
				"uri": resource_updated_uri(&first_event.message)?,
			},
			{
				"action": "replay_from_last_event_id",
				"last_event_id": first_event.event_id,
				"replayed_event_id": replayed_event.event_id,
				"uri": resource_updated_uri(&replayed_event.message)?,
			},
		],
	});
	assert_mcp_json_snapshot("events/replay_buffered_notification", report);

	Ok(())
}
