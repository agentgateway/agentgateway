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

mod missing_finish_reason {
	use std::sync::{Arc, Mutex};

	use agent_core::strng;
	use agent_http::Body;
	use bytes::Bytes;
	use http_body_util::BodyExt;
	use serde_json::{Value, json};

	use super::to_responses;
	use crate::{
		AIError, CacheTokenConvention, InputFormat, LLMInfo, LLMRequest, LLMResponse, LogContentFields,
		MISSING_FINISH_REASON_MESSAGE, ReportedUsage, StreamingUsageGuard, StreamingUsageReporter,
	};

	fn completion(finish_reason: Option<&str>, content: Option<&str>) -> Vec<u8> {
		serde_json::to_vec(&json!({
			"id": "chatcmpl-1",
			"object": "chat.completion",
			"created": 1,
			"model": "gemini",
			"choices": [{
				"index": 0,
				"finish_reason": finish_reason,
				"message": {"role": "assistant", "content": content}
			}],
			"usage": {
				"prompt_tokens": 10,
				"completion_tokens": 4,
				"total_tokens": 14,
				"completion_tokens_details": {"reasoning_tokens": 6}
			}
		}))
		.unwrap()
	}

	fn reported(err: AIError) -> ReportedUsage {
		match err {
			AIError::MissingFinishReason(Some(usage)) => usage,
			other => panic!("expected missing finish_reason, got {other}"),
		}
	}

	#[test]
	fn buffered_null_finish_reason_is_not_completed() {
		for content in [None, Some("partial")] {
			let err = match to_responses::translate_response(
				&Bytes::from(completion(None, content)),
				"gemini",
				None,
			) {
				Err(err) => err,
				Ok(_) => panic!("a missing finish_reason is not a completed response"),
			};
			let usage = reported(err);
			assert_eq!(usage.input_tokens, 10);
			assert_eq!(usage.output_tokens, 4);
			assert_eq!(usage.reasoning_tokens, Some(6));
		}
	}

	#[test]
	fn buffered_explicit_stop_stays_completed_even_when_empty() {
		let response = to_responses::translate_response(
			&Bytes::from(completion(Some("stop"), None)),
			"gemini",
			None,
		)
		.unwrap();
		let response: Value = serde_json::from_slice(&response.serialize().unwrap()).unwrap();
		assert_eq!(response["status"], "completed");
		assert!(response.get("error").is_none() || response["error"].is_null());
	}

	struct Capture {
		info: Arc<Mutex<LLMInfo>>,
	}

	impl StreamingUsageReporter for Capture {
		fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
			f(&mut self.info.lock().unwrap());
		}

		fn report_usage(&mut self) {}
	}

	async fn sse(body: String) -> (String, LLMResponse) {
		let info = Arc::new(Mutex::new(LLMInfo::new(
			LLMRequest {
				input_tokens: None,
				input_format: InputFormat::Responses,
				cache_convention: CacheTokenConvention::pending(),
				request_model: strng::literal!("gemini"),
				original_model: None,
				provider: Default::default(),
				streaming: true,
				params: Default::default(),
				prompt: None,
				provider_state: None,
			},
			LLMResponse::default(),
		)));
		let guard = StreamingUsageGuard::new(Box::new(Capture { info: info.clone() }));
		let bytes = to_responses::translate_stream(
			Body::from(body),
			1024 * 1024,
			guard,
			LogContentFields::default(),
			None,
		)
		.collect()
		.await
		.unwrap()
		.to_bytes();
		(
			String::from_utf8(bytes.to_vec()).unwrap(),
			info.lock().unwrap().response.clone(),
		)
	}

	const USAGE: &str = r#""usage":{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14,"completion_tokens_details":{"reasoning_tokens":6}}"#;

	#[tokio::test]
	async fn stream_ending_without_finish_reason_fails_the_response() {
		for delta in [
			r#""role":"assistant","content":null"#,
			r#""content":"partial""#,
		] {
			let input = format!(
				"data: {{\"id\":\"chunk-empty\",\"choices\":[{{\"index\":0,\"delta\":{{{delta}}}}}],\"model\":\"gemini\",{USAGE}}}\n\ndata: [DONE]\n\n"
			);
			let (events, logged) = sse(input).await;
			assert!(events.contains("response.failed"), "{events}");
			assert!(events.contains("server_error"), "{events}");
			assert!(events.contains(MISSING_FINISH_REASON_MESSAGE), "{events}");
			assert!(!events.contains("response.completed"), "{events}");
			assert_eq!(logged.input_tokens, Some(10));
			assert_eq!(logged.output_tokens, Some(4));
			assert_eq!(logged.reasoning_tokens, Some(6));
		}
	}

	#[tokio::test]
	async fn null_chunks_before_stop_still_complete() {
		let input = format!(
			"data: {{\"id\":\"c\",\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"Hi\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"c\",\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],{USAGE}}}\n\ndata: [DONE]\n\n"
		);
		let (events, _) = sse(input).await;
		assert!(events.contains("response.completed"), "{events}");
		assert!(!events.contains("response.failed"), "{events}");
	}
}
