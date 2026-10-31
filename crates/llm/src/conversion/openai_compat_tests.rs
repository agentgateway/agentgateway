use bytes::Bytes;
use serde_json::{Value, json};

use super::{from_responses, to_responses};

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

	translated.request.strip_tool_call_extra_content();
	let other_provider: Value = serde_json::to_value(translated.request).unwrap();
	let calls = other_provider["messages"][0]["tool_calls"]
		.as_array()
		.unwrap();
	assert!(calls[0].get("extra_content").is_none());
}

#[test]
fn gemini_thought_signature_pairs_with_the_next_call_when_call_ids_repeat() {
	let carrier = |sig: &str| crate::conversion::GeminiThoughtSignature::encode("call_dup", sig);
	let turn = |sig: &str| {
		vec![
			json!({"type":"reasoning","id":"rs","summary":[],"encrypted_content": carrier(sig)}),
			json!({"type":"function_call","call_id":"call_dup","name":"f","arguments":"{}"}),
			json!({"type":"function_call_output","call_id":"call_dup","output":"ok"}),
		]
	};
	let mut input = turn("sig1");
	input.extend(turn("sig2"));
	let request: crate::types::responses::Request = serde_json::from_value(
		json!({"model":"gemini-3.8-flash","input":input,"tools":[],"store":false}),
	)
	.unwrap();
	let translated = super::from_responses::translate_request(&request).unwrap();
	let v: Value = serde_json::to_value(&translated.request).unwrap();
	let sigs: Vec<_> = v["messages"]
		.as_array()
		.unwrap()
		.iter()
		.filter_map(|m| {
			m["tool_calls"][0]["extra_content"]["google"]["thought_signature"]
				.as_str()
				.map(str::to_owned)
		})
		.collect();
	assert_eq!(sigs, ["sig1", "sig2"]);
}

#[test]
fn buffered_function_call_survives_discarded_custom_call() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["finish_reason"] = Value::Null;
	input["choices"][0]["message"] = json!({
		"content": null,
		"role": "assistant",
		"tool_calls": [
			{"custom_tool": {"input": "input", "name": "custom"}, "id": "custom", "type": "custom"},
			{"function": {"arguments": "{}", "name": "get_weather"}, "id": "call_1", "type": "function"},
		],
	});
	let response = translate(&serde_json::to_vec(&input).unwrap());
	assert_eq!(response["status"], "completed");
	assert_eq!(response["output"].as_array().unwrap().len(), 1);
	assert_eq!(response["output"][0]["type"], "function_call");
	assert_eq!(response["output"][0]["call_id"], "call_1");
}

#[test]
fn tool_choice_without_tools_is_dropped_unless_forced() {
	let function = json!([{"type": "function", "name": "f", "parameters": {"type": "object"}}]);
	let unsupported = json!([{"type": "web_search"}]);
	// (tools, tool_choice) in, (tools, tool_choice) out.
	let cases = [
		(None, json!("auto"), None, None),
		(Some(json!([])), json!("auto"), None, None),
		(None, json!("none"), None, None),
		(Some(unsupported), json!("auto"), None, None),
		(None, json!("required"), None, Some(json!("required"))),
		(
			None,
			json!({"type": "function", "name": "f"}),
			None,
			Some(json!({"type": "function", "function": {"name": "f"}})),
		),
		(
			Some(function),
			json!("auto"),
			Some(
				json!([{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}]),
			),
			Some(json!("auto")),
		),
	];
	for (tools, tool_choice, want_tools, want_choice) in cases {
		let mut req = json!({
			"model": "gemini-4-argon",
			"input": "synthetic",
			"stream": true,
			"tool_choice": tool_choice,
			"parallel_tool_calls": false,
		});
		if let Some(tools) = &tools {
			req["tools"] = tools.clone();
		}
		let req = serde_json::from_value(req).unwrap();
		let out: Value = serde_json::from_slice(&from_responses::translate(&req).unwrap()).unwrap();
		assert_eq!(
			out.get("tools"),
			want_tools.as_ref(),
			"{tools:?} {tool_choice}"
		);
		assert_eq!(
			out.get("tool_choice"),
			want_choice.as_ref(),
			"{tools:?} {tool_choice}"
		);
		assert_eq!(out["parallel_tool_calls"], false, "{tools:?} {tool_choice}");
	}
}

#[test]
fn buffered_signature_only_reply_without_finish_reason_fails() {
	// Responses drops a bare signature, so the client would get an empty completed reply.
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["finish_reason"] = Value::Null;
	input["choices"][0]["message"] =
		json!({"content": null, "role": "assistant", "reasoning_signature": "sig"});
	let err =
		to_responses::translate_response(&Bytes::from(serde_json::to_vec(&input).unwrap()), "m", None)
			.err()
			.expect("an untranslated signature is not output");
	assert!(
		matches!(err, crate::AIError::EmptyResponse(Some(_))),
		"{err}"
	);
}
