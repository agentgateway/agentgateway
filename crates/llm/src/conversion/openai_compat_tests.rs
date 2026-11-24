use bytes::Bytes;
use serde_json::{Value, json};

use super::{from_responses, to_responses};

#[rstest::rstest]
fn responses_to_completions_rejects_images_without_a_url(
	#[values(None, Some("message"))] message_type: Option<&str>,
	#[values(None, Some(""))] image_url: Option<&str>,
) {
	let mut message = json!({
		"role": "user",
		"content": [
			{"type": "input_text", "text": "Describe this image."},
			{"type": "input_image", "file_id": "file-image"}
		]
	});
	if let Some(message_type) = message_type {
		message["type"] = json!(message_type);
	}
	if let Some(image_url) = image_url {
		message["content"][1]["image_url"] = json!(image_url);
	}
	let request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"input": [message]
	}))
	.unwrap();

	let result = from_responses::translate(&request);
	assert!(
		matches!(&result, Err(crate::AIError::UnsupportedConversion(reason)) if reason.contains("image_url")),
		"{result:?}"
	);
}

#[rstest::rstest]
#[case::remote_file_url(json!({"type": "input_file", "file_url": "https://example.com/report.pdf"}))]
#[case::empty_data_and_id(json!({"type": "input_file", "file_data": "", "file_id": ""}))]
fn responses_to_completions_rejects_files_without_data_or_id(#[case] file: Value) {
	let request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"input": [{"role": "user", "content": [file]}]
	}))
	.unwrap();

	let result = from_responses::translate(&request);
	assert!(
		matches!(&result, Err(crate::AIError::UnsupportedConversion(reason)) if reason.contains("input_file")),
		"{result:?}"
	);
}

// The typed `message` item form has no assistant input role.
#[rstest::rstest]
#[case::easy_system(None, "system")]
#[case::easy_developer(None, "developer")]
#[case::easy_assistant(None, "assistant")]
#[case::typed_system(Some("message"), "system")]
#[case::typed_developer(Some("message"), "developer")]
fn responses_to_completions_rejects_media_outside_user_messages(
	#[case] message_type: Option<&str>,
	#[case] role: &str,
	#[values("input_image", "input_file")] kind: &str,
) {
	let part = match kind {
		"input_image" => json!({"type": kind, "image_url": "data:image/png;base64,AAAA"}),
		"input_file" => json!({"type": kind, "file_data": "data:application/pdf;base64,JVBERi0="}),
		_ => unreachable!("unknown media kind {kind}"),
	};
	let mut message = json!({
		"role": role,
		"content": [{"type": "input_text", "text": "Look at this."}, part]
	});
	if let Some(message_type) = message_type {
		message["type"] = json!(message_type);
	}
	let request = serde_json::from_value(json!({"model": "gpt-4o", "input": [message]})).unwrap();

	let result = from_responses::translate(&request);
	assert!(
		matches!(&result, Err(crate::AIError::UnsupportedConversion(reason)) if reason.contains("user messages")),
		"{result:?}"
	);
}

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

struct NoopReporter;

impl crate::StreamingUsageReporter for NoopReporter {
	fn update(&self, _: &mut dyn FnMut(&mut crate::LLMInfo)) {}
	fn report_usage(&mut self) {}
}

fn chunk(delta: Value, finish_reason: Option<&str>) -> String {
	let chunk = json!({
		"id": "chatcmpl-text-then-tool",
		"object": "chat.completion.chunk",
		"created": 1772656915,
		"model": "gpt-5.1",
		"choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
	});
	format!("data: {chunk}\n\n")
}

/// The `response.output_item.done` items in the order a streaming client receives them.
async fn streamed_done_items(chunks: &[String]) -> Vec<Value> {
	use http_body_util::BodyExt;

	let body = agent_http::Body::from(format!("{}data: [DONE]\n\n", chunks.concat()));
	let translated = to_responses::translate_stream(
		body,
		1024 * 1024,
		crate::StreamingUsageGuard::new(Box::new(NoopReporter)),
		crate::LogContentFields {
			completion: true,
			tool_calls: true,
		},
		None,
	);
	let bytes = translated.collect().await.unwrap().to_bytes();
	String::from_utf8(bytes.to_vec())
		.unwrap()
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
		.filter_map(|data| serde_json::from_str::<Value>(data).ok())
		.filter(|event| event["type"] == "response.output_item.done")
		.collect()
}

// Responses clients such as Codex append each item to the conversation as its
// `response.output_item.done` arrives, then send that history back. When a model
// writes text before calling a tool, the message must finish before the call,
// or the replayed history puts the text between the call and its result, which
// Chat Completions rejects.
#[tokio::test]
async fn streamed_text_and_tool_call_finish_in_output_order() {
	let chunks = [
		chunk(json!({"role": "assistant", "content": "Running it."}), None),
		chunk(
			json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
				"function": {"name": "shell", "arguments": ""}}]}),
			None,
		),
		chunk(
			json!({"tool_calls": [{"index": 0, "function": {"arguments": "{\"cmd\":\"echo hi\"}"}}]}),
			None,
		),
		chunk(json!({}), Some("tool_calls")),
	];

	let done = streamed_done_items(&chunks).await;
	let order: Vec<_> = done
		.iter()
		.map(|event| (event["output_index"].clone(), event["item"]["type"].clone()))
		.collect();
	assert_eq!(
		order,
		[
			(json!(0), json!("message")),
			(json!(1), json!("function_call")),
		]
	);

	let mut input: Vec<Value> = done
		.into_iter()
		.map(|event| event["item"].clone())
		.collect();
	input.push(json!({"type": "function_call_output", "call_id": "call_1", "output": "hi"}));
	let request: crate::types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-5.1",
		"input": input,
		"tools": [],
		"store": false
	}))
	.unwrap();
	let translated = from_responses::translate_request(&request).unwrap();
	let next_request = serde_json::to_value(&translated.request).unwrap();
	let messages = next_request["messages"].as_array().unwrap();
	let roles: Vec<_> = messages.iter().map(|message| &message["role"]).collect();
	let call = messages
		.iter()
		.position(|message| message["tool_calls"].is_array())
		.expect("history should carry the tool call");
	assert_eq!(
		roles[call + 1],
		"tool",
		"tool result must follow its call: {roles:?}"
	);
}
