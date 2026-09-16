use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::{Value, json};

use super::to_responses;

#[tokio::test]
async fn completions_stream_defaults_missing_cache_write_tokens_to_zero() {
	let input = r#"data: {"id":"chunk","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}],"model":"claude-fable-5.1","usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11,"prompt_tokens_details":{"cached_tokens":4}}}

data: [DONE]

"#;
	let output = to_responses::translate_stream(
		agent_http::Body::from(input),
		1024 * 1024,
		crate::StreamingUsageGuard::default(),
		crate::LogContentFields::default(),
		None,
		None,
	)
	.collect()
	.await
	.unwrap()
	.to_bytes();
	let completed = String::from_utf8(output.to_vec())
		.unwrap()
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
		.filter_map(|data| serde_json::from_str::<Value>(data).ok())
		.find(|event| event["type"] == "response.completed")
		.unwrap();

	assert_eq!(
		completed["response"]["usage"]["input_tokens_details"]["cache_write_tokens"],
		0
	);
}

#[test]
fn responses_assistant_history_translates_to_completions() {
	let req: crate::types::responses::Request = serde_json::from_slice(include_bytes!(
		"../tests/requests/responses/assistant-history.json"
	))
	.unwrap();
	let translated = super::from_responses::translate_request(&req).unwrap();
	let assistant = translated
		.request
		.messages
		.iter()
		.find_map(|message| match message {
			crate::types::completions::typed::RequestMessage::Assistant(message) => Some(message),
			_ => None,
		})
		.unwrap();
	assert_eq!(
		assistant.content.as_ref().unwrap(),
		&crate::types::completions::typed::RequestAssistantMessageContent::Text(
			"Here is the answer.\nI cannot help with that part.".into()
		)
	);
}

#[test]
fn responses_additional_custom_tool_loop_translates_to_completions() {
	let req = serde_json::from_value(json!({
		"model": "gpt-5",
		"tool_choice": {"type": "custom", "name": "exec"},
		"input": [
			{"type": "additional_tools", "role": "developer", "tools": [
				{"type": "namespace", "name": "functions", "description": "Local tools", "tools": [
					{"type": "custom", "name": "exec", "description": "Run a command", "format": {
						"type": "grammar", "syntax": "lark", "definition": "start: WORD"
					}}
				]}
			]},
			{"role": "user", "content": "inspect it"},
			{"type": "custom_tool_call", "status": "completed", "call_id": "call_1", "namespace": "functions", "name": "exec", "input": "ls"},
			{"type": "message", "id": "msg_1", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "Checking it."}]},
			{"role": "assistant", "content": [{"type": "output_text", "text": "One moment."}]},
			{"type": "custom_tool_call_output", "call_id": "call_1", "output": [{"type": "input_text", "text": "file.txt"}]}
		]
	}))
	.unwrap();
	let translated = super::from_responses::translate_request(&req).unwrap();
	assert!(translated.custom_tools.contains("functions__exec"));
	let namespaces = translated.namespaces;
	let custom_tools = translated.custom_tools;
	let translated = serde_json::to_value(translated.request).unwrap();

	assert_eq!(translated["tools"][0]["type"], "function");
	assert_eq!(
		translated["tools"][0]["function"]["name"],
		"functions__exec"
	);
	assert!(
		translated["tools"][0]["function"]["description"]
			.as_str()
			.unwrap()
			.contains("Local tools")
	);
	assert!(
		translated["tools"][0]["function"]["description"]
			.as_str()
			.unwrap()
			.contains("start: WORD")
	);
	assert_eq!(
		translated["tool_choice"]["function"]["name"],
		"functions__exec"
	);
	assert_eq!(translated["messages"][1]["tool_calls"][0]["id"], "call_1");
	assert_eq!(
		translated["messages"][1]["tool_calls"][0]["function"]["name"],
		"functions__exec"
	);
	assert_eq!(
		translated["messages"][1]["tool_calls"][0]["function"]["arguments"],
		serde_json::json!({"input": "ls"}).to_string()
	);
	assert_eq!(
		translated["messages"][1]["content"],
		"Checking it.\nOne moment."
	);
	assert_eq!(translated["messages"][2]["tool_call_id"], "call_1");
	assert_eq!(translated["messages"][2]["content"], "file.txt");

	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["message"]["content"] = Value::Null;
	input["choices"][0]["message"]["tool_calls"] = json!([{
		"type": "function",
		"id": "call_2",
		"function": {"name": "functions__exec", "arguments": "{\"input\":\"pwd\"}"}
	}]);
	let response = to_responses::translate_response(
		&Bytes::from(serde_json::to_vec(&input).unwrap()),
		"input-model",
		Some(&namespaces),
		Some(&custom_tools),
	)
	.unwrap();
	let response: Value = serde_json::from_slice(&response.serialize().unwrap()).unwrap();
	assert_eq!(response["output"][0]["type"], "custom_tool_call");
	assert_eq!(response["output"][0]["namespace"], "functions");
	assert_eq!(response["output"][0]["name"], "exec");
}

#[test]
fn assistant_content_shapes_merge_without_losing_cache_breakpoints() {
	let req = serde_json::from_value(json!({
		"model": "gpt-5",
		"input": [
			{"type": "message", "id": "msg_1", "status": "completed", "role": "assistant",
				"content": [{"type": "output_text", "text": "before"}]},
			{"role": "assistant", "content": [{"type": "input_text", "text": "cached",
				"prompt_cache_breakpoint": {"mode": "explicit"}}]},
			{"type": "message", "id": "msg_2", "status": "completed", "role": "assistant",
				"content": [{"type": "output_text", "text": "after"}]}
		]
	}))
	.unwrap();
	let translated = serde_json::to_value(
		super::from_responses::translate_request(&req)
			.unwrap()
			.request,
	)
	.unwrap();

	assert_eq!(translated["messages"].as_array().unwrap().len(), 1);
	assert_eq!(translated["messages"][0]["content"][0]["text"], "before");
	assert_eq!(
		translated["messages"][0]["content"][1]["prompt_cache_breakpoint"]["mode"],
		"explicit"
	);
	assert_eq!(translated["messages"][0]["content"][2]["text"], "after");
}

fn translate_with_custom(input: &[u8], custom_names: &[&str]) -> Value {
	let custom_tools = custom_names.iter().map(|name| name.to_string()).collect();
	let response = to_responses::translate_response(
		&Bytes::copy_from_slice(input),
		"input-model",
		None,
		Some(&custom_tools),
	)
	.expect("Chat Completions response should translate");
	serde_json::from_slice(&response.serialize().expect("response should serialize"))
		.expect("translated response should be JSON")
}

#[test]
fn completion_compat_custom_call_translates_back_to_responses() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["message"]["content"] = Value::Null;
	input["choices"][0]["message"]["tool_calls"] = json!([{
		"type": "function",
		"id": "call_1",
		"function": {"name": "exec", "arguments": "{\"input\":\"ls\"}"}
	}]);
	input["choices"][0]["finish_reason"] = json!("tool_calls");

	let response = translate_with_custom(&serde_json::to_vec(&input).unwrap(), &["exec"]);
	let call = &response["output"][0];
	assert_eq!(call["type"], "custom_tool_call");
	assert_eq!(call["call_id"], "call_1");
	assert_eq!(call["name"], "exec");
	assert_eq!(call["input"], "ls");
	assert!(call["id"].as_str().unwrap().starts_with("ctc_"));
}

#[test]
fn custom_tool_state_does_not_capture_prefixed_function() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["message"]["content"] = Value::Null;
	input["choices"][0]["message"]["tool_calls"] = json!([{
		"type": "function",
		"id": "call_1",
		"function": {"name": "__agw_custom_exec", "arguments": "{}"}
	}]);
	input["choices"][0]["finish_reason"] = json!("tool_calls");

	let response = translate_with_custom(&serde_json::to_vec(&input).unwrap(), &["exec"]);
	assert_eq!(response["output"][0]["type"], "function_call");
	assert_eq!(response["output"][0]["name"], "__agw_custom_exec");
}

#[test]
fn duplicate_function_and_custom_names_are_rejected() {
	let req = serde_json::from_value(json!({
		"input": "hello",
		"tools": [
			{"type": "function", "name": "exec", "parameters": {}},
			{"type": "custom", "name": "exec", "format": {"type": "text"}}
		]
	}))
	.unwrap();

	let Err(error) = super::from_responses::translate_request(&req) else {
		panic!("duplicate tool names should be rejected")
	};
	assert_eq!(
		error.to_string(),
		"unsupported conversion: duplicate upstream tool name: exec"
	);
}

#[test]
fn historical_function_shape_does_not_conflict_with_current_custom_tool() {
	let req = serde_json::from_value(json!({
		"input": [{
			"type": "function_call",
			"call_id": "call_1",
			"name": "exec",
			"arguments": "{\"input\":\"ls\"}"
		}],
		"tools": [{"type": "custom", "name": "exec", "format": {"type": "text"}}]
	}))
	.unwrap();

	let translated = super::from_responses::translate_request(&req).unwrap();
	assert!(translated.custom_tools.contains("exec"));
}

#[test]
fn malformed_and_native_custom_calls_stay_custom() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["message"]["content"] = Value::Null;
	input["choices"][0]["message"]["tool_calls"] = json!([
		{
			"type": "function",
			"id": "call_1",
			"function": {"name": "exec", "arguments": "raw input"}
		},
		{
			"type": "custom",
			"id": "call_2",
			"custom_tool": {"name": "apply_patch", "input": "*** Begin Patch"}
		}
	]);
	input["choices"][0]["finish_reason"] = json!("tool_calls");

	let response = translate_with_custom(&serde_json::to_vec(&input).unwrap(), &["exec"]);
	assert_eq!(response["output"][0]["type"], "custom_tool_call");
	assert_eq!(response["output"][0]["name"], "exec");
	assert_eq!(response["output"][0]["input"], "raw input");
	assert_eq!(response["output"][1]["type"], "custom_tool_call");
	assert_eq!(response["output"][1]["name"], "apply_patch");
}

#[test]
fn buffered_failed_response_preserves_error() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["finish_reason"] = json!("content_filter");
	let input = serde_json::to_vec(&input).unwrap();

	let response = translate_with_custom(&input, &[]);

	assert_eq!(response["status"], json!("failed"));
	assert_eq!(
		response["error"],
		json!({"code": "content_filter", "message": "Content filtered"})
	);
}
