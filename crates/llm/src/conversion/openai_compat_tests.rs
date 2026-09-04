use bytes::Bytes;
use serde_json::{Value, json};

use super::to_responses;

fn translate(input: &[u8]) -> Value {
	let response = to_responses::translate_response(&Bytes::copy_from_slice(input), "input-model")
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

/// Drive the streaming translator over an OpenAI-compatible SSE body and return
/// the emitted Responses events, parsed.
async fn translate_stream(chunks: &[Value]) -> Vec<Value> {
	use http_body_util::BodyExt;
	let mut sse = String::new();
	for c in chunks {
		sse.push_str("data: ");
		sse.push_str(&c.to_string());
		sse.push_str("\n\n");
	}
	sse.push_str("data: [DONE]\n\n");
	let body = to_responses::translate_stream(
		axum_core::body::Body::from(sse),
		1 << 20,
		crate::StreamingUsageGuard::default(),
		crate::LogContentFields::default(),
	);
	let bytes = body.collect().await.expect("stream body").to_bytes();
	String::from_utf8(bytes.to_vec())
		.expect("utf8")
		.lines()
		.filter_map(|l| l.strip_prefix("data: "))
		.map(|d| serde_json::from_str::<Value>(d).expect("event json"))
		.collect()
}

fn chunk(delta: Value, finish_reason: Option<&str>, usage: Option<Value>) -> Value {
	let mut c = json!({
		"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "m",
		"choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]
	});
	if let Some(u) = usage {
		c["usage"] = u;
	}
	c
}

#[tokio::test]
async fn stream_terminal_events_carry_the_finished_text() {
	let events = translate_stream(&[
		chunk(json!({"role": "assistant", "content": ""}), None, None),
		chunk(json!({"content": "o"}), None, None),
		chunk(json!({"content": "k"}), None, None),
		chunk(
			json!({}),
			Some("stop"),
			Some(json!({"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5})),
		),
	])
	.await;
	let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
	assert_eq!(&types[..2], &["response.created", "response.in_progress"]);
	assert!(types.contains(&"response.output_text.done"));

	let text_done = events
		.iter()
		.find(|e| e["type"] == "response.output_text.done")
		.unwrap();
	assert_eq!(text_done["text"], json!("ok"));
	let part_done = events
		.iter()
		.find(|e| e["type"] == "response.content_part.done")
		.unwrap();
	assert_eq!(part_done["part"]["text"], json!("ok"));
	let item_done = events
		.iter()
		.find(|e| e["type"] == "response.output_item.done")
		.unwrap();
	assert_eq!(item_done["item"]["content"][0]["text"], json!("ok"));

	// What an SDK's get_final_response() reads.
	let completed = events.last().unwrap();
	assert_eq!(completed["type"], json!("response.completed"));
	let output = completed["response"]["output"].as_array().unwrap();
	assert_eq!(output.len(), 1);
	assert_eq!(output[0]["type"], json!("message"));
	assert_eq!(output[0]["content"][0]["text"], json!("ok"));
	assert_eq!(completed["response"]["usage"]["output_tokens"], json!(2));

	let seqs: Vec<u64> = events
		.iter()
		.map(|e| e["sequence_number"].as_u64().unwrap())
		.collect();
	assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
}

#[tokio::test]
async fn stream_incomplete_and_tool_calls_land_in_the_terminal_output() {
	let events = translate_stream(&[
		chunk(
			json!({"role": "assistant", "content": "partial"}),
			None,
			None,
		),
		chunk(
			json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
				"function": {"name": "lookup", "arguments": "{\"q\":"}}]}),
			None,
			None,
		),
		chunk(
			json!({"tool_calls": [{"index": 0, "function": {"arguments": "1}"}}]}),
			None,
			None,
		),
		chunk(
			json!({}),
			Some("length"),
			Some(json!({"prompt_tokens": 3, "completion_tokens": 9, "total_tokens": 12})),
		),
	])
	.await;
	let last = events.last().unwrap();
	assert_eq!(last["type"], json!("response.incomplete"));
	assert_eq!(
		last["response"]["incomplete_details"]["reason"],
		json!("max_tokens")
	);
	let output = last["response"]["output"].as_array().unwrap();
	assert_eq!(output.len(), 2);
	assert_eq!(output[0]["type"], json!("message"));
	assert_eq!(output[0]["content"][0]["text"], json!("partial"));
	assert_eq!(output[1]["type"], json!("function_call"));
	assert_eq!(output[1]["name"], json!("lookup"));
	assert_eq!(output[1]["arguments"], json!("{\"q\":1}"));
}
