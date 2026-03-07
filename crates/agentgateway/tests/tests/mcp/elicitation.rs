use super::*;

#[tokio::test]
async fn test_elicitation_roundtrip_multiplex() -> anyhow::Result<()> {
	let fixture = MultiplexFixture::setup().await?;
	let client = &fixture.client;

	let e_resp = client
		.call_tool(call_tool_params("s1__elicitation", None, None))
		.await?;
	assert_eq!(
		e_resp.structured_content.unwrap().get("color").unwrap(),
		"diamond"
	);

	let report = json!({
		"scenario": "elicitation/roundtrip",
		"steps": [
			{
				"action": "call_tool",
				"tool": "s1__elicitation",
			},
			{
				"action": "elicitation_response",
				"structured_content": {
					"color": "diamond",
				},
			},
		],
	});
	assert_mcp_json_snapshot("elicitation/roundtrip", report);
	Ok(())
}

#[tokio::test]
async fn test_url_elicitation_error_passthrough_in_multiplex() -> anyhow::Result<()> {
	let fixture = MultiplexFixture::setup().await?;
	let client = &fixture.client;

	let err = client
		.call_tool(call_tool_params(
			"s1__test_url_elicitation_required",
			None,
			None,
		))
		.await
		.expect_err("Expected URL elicitation error");

	match err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(mcp_error.code, ErrorCode::URL_ELICITATION_REQUIRED);
			assert_eq!(
				mcp_error.message.as_ref(),
				"This request requires more information."
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

			let report = json!({
				"scenario": "elicitation/url_error_passthrough",
				"steps": [
					{
						"action": "call_tool",
						"tool": "s1__test_url_elicitation_required",
						"outcome": "mcp_error",
					},
					{
						"action": "error",
						"code": mcp_error.code,
						"message": mcp_error.message,
						"elicitations": elicitations,
					},
				],
			});
			assert_mcp_json_snapshot("elicitation/url_error_passthrough", report);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}

	Ok(())
}
