use bytes::Bytes;
use serde_json::{Value, json};

use super::to_responses;

#[test]
fn responses_assistant_history_translates_to_completions() {
	let req: crate::types::responses::Request = serde_json::from_slice(include_bytes!(
		"../tests/requests/responses/assistant-history.json"
	))
	.unwrap();
	let translated = super::from_responses::translate_request(&req).unwrap();
	let assistant = translated
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
			{"type": "custom_tool_call", "status": "completed", "call_id": "call_1", "name": "exec", "input": "ls"},
			{"type": "message", "id": "msg_1", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "Checking it."}]},
			{"role": "assistant", "content": [{"type": "output_text", "text": "One moment."}]},
			{"type": "custom_tool_call_output", "call_id": "call_1", "output": [{"type": "input_text", "text": "file.txt"}]}
		]
	}))
	.unwrap();
	let translated =
		serde_json::to_value(super::from_responses::translate_request(&req).unwrap()).unwrap();

	assert_eq!(translated["tools"][0]["type"], "function");
	assert_eq!(
		translated["tools"][0]["function"]["name"],
		"__agw_custom_exec"
	);
	assert!(
		translated["tools"][0]["function"]["description"]
			.as_str()
			.unwrap()
			.contains("start: WORD")
	);
	assert_eq!(
		translated["tool_choice"]["function"]["name"],
		"__agw_custom_exec"
	);
	assert_eq!(translated["messages"][1]["tool_calls"][0]["id"], "call_1");
	assert_eq!(
		translated["messages"][1]["tool_calls"][0]["function"]["name"],
		"__agw_custom_exec"
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
	let translated =
		serde_json::to_value(super::from_responses::translate_request(&req).unwrap()).unwrap();

	assert_eq!(translated["messages"].as_array().unwrap().len(), 1);
	assert_eq!(translated["messages"][0]["content"][0]["text"], "before");
	assert_eq!(
		translated["messages"][0]["content"][1]["prompt_cache_breakpoint"]["mode"],
		"explicit"
	);
	assert_eq!(translated["messages"][0]["content"][2]["text"], "after");
}

fn translate(input: &[u8]) -> Value {
	let response = to_responses::translate_response(&Bytes::copy_from_slice(input), "input-model")
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
		"function": {"name": "__agw_custom_exec", "arguments": "{\"input\":\"ls\"}"}
	}]);
	input["choices"][0]["finish_reason"] = json!("tool_calls");

	let response = translate(&serde_json::to_vec(&input).unwrap());
	let call = &response["output"][0];
	assert_eq!(call["type"], "custom_tool_call");
	assert_eq!(call["call_id"], "call_1");
	assert_eq!(call["name"], "exec");
	assert_eq!(call["input"], "ls");
	assert!(call["id"].as_str().unwrap().starts_with("ctc_"));
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
			"function": {"name": "__agw_custom_exec", "arguments": "raw input"}
		},
		{
			"type": "custom",
			"id": "call_2",
			"custom_tool": {"name": "apply_patch", "input": "*** Begin Patch"}
		}
	]);
	input["choices"][0]["finish_reason"] = json!("tool_calls");

	let response = translate(&serde_json::to_vec(&input).unwrap());
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

	let response = translate(&input);

	assert_eq!(response["status"], json!("failed"));
	assert_eq!(
		response["error"],
		json!({"code": "content_filter", "message": "Content filtered"})
	);
}
