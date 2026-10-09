use bytes::Bytes;
use serde_json::{Value, json};

use super::to_responses;

fn translate(input: &[u8]) -> Value {
	let response =
		to_responses::translate_response(&Bytes::copy_from_slice(input), "input-model", None)
			.expect("Chat Completions response should translate");
	serde_json::from_slice(&response.serialize().expect("response should serialize"))
		.expect("translated response should be JSON")
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

#[test]
fn gemini_thought_signature_round_trips_through_responses_reasoning_item() {
	let signature = "CqUBAbc123def456GHI789jklMNOpqrSTUvwxYZ0123456789+/aBcDeFgHiJkLmNoPqRsTuVwXyZ==";
	let mut upstream: Value = serde_json::from_slice(include_bytes!(
		"../tests/response/completions/tool_call.json"
	))
	.unwrap();
	upstream["choices"][0]["message"]["tool_calls"][0]["extra_content"] = json!({
		"google": {"thought_signature": signature}
	});

	let first_response = translate(&serde_json::to_vec(&upstream).unwrap());
	let output = first_response["output"].as_array().unwrap();
	assert_eq!(output[0]["type"], "reasoning");
	assert_eq!(output[1]["type"], "function_call");
	assert_eq!(output[1]["call_id"], "call_abc123");
	assert_eq!(output[2]["type"], "function_call");
	assert_eq!(output[2]["call_id"], "call_xyz789");

	let mut input = output.clone();
	input.push(json!({
		"type": "function_call_output",
		"call_id": "call_abc123",
		"output": "sunny"
	}));
	input.push(json!({
		"type": "function_call_output",
		"call_id": "call_xyz789",
		"output": "done"
	}));
	let request: crate::types::responses::Request = serde_json::from_value(json!({
		"model": "gemini-3.8-flash",
		"input": input,
		"tools": [],
		"store": false
	}))
	.unwrap();
	let mut translated = super::from_responses::translate_request(&request).unwrap();
	let next_request: Value = serde_json::to_value(&translated.request).unwrap();
	let calls = next_request["messages"][0]["tool_calls"]
		.as_array()
		.unwrap();

	assert_eq!(calls[0]["id"], "call_abc123");
	assert_eq!(
		calls[0]["extra_content"]["google"]["thought_signature"],
		signature
	);
	assert_eq!(calls[1]["id"], "call_xyz789");
	assert!(calls[1].get("extra_content").is_none());

	translated.request.strip_thought_signatures();
	let other_provider: Value = serde_json::to_value(translated.request).unwrap();
	let calls = other_provider["messages"][0]["tool_calls"]
		.as_array()
		.unwrap();
	assert!(calls[0].get("extra_content").is_none());
}
