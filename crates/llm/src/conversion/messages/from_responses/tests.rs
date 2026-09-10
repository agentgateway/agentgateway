use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::strng;
use bytes::Bytes;
use futures_util::stream;
use http_body_util::BodyExt;
use serde_json::json;

use super::{State, translate, translate_error, translate_response, translate_stream};
use crate::{
	CacheTokenConvention, InputFormat, LLMInfo, LLMRequest, LLMResponse, LogContentFields,
	StreamingUsageGuard, StreamingUsageReporter, types,
};

fn request(value: serde_json::Value) -> types::responses::Request {
	serde_json::from_value(value).expect("valid local Responses request")
}

fn response_state() -> State {
	let (_, state) = translate(&request(json!({
		"model": "request-model",
		"input": "work",
		"tools": [
			{
				"type": "function",
				"name": "get_weather",
				"parameters": {"type": "object"}
			},
			{
				"type": "custom",
				"name": "python",
				"format": {"type": "text"}
			}
		]
	})))
	.expect("state request should translate");
	state
}

#[test]
fn explicit_thinking_budget_is_preserved_and_capped() {
	let (body, _) = translate(&request(json!({
		"model": "claude",
		"input": "work",
		"max_output_tokens": 2048,
		"vendor_extensions": {"thinking_budget_tokens": 3072}
	})))
	.expect("request should translate");
	let translated: types::messages::typed::Request =
		serde_json::from_slice(&body).expect("valid Messages request");

	assert!(matches!(
		translated.thinking,
		Some(types::messages::typed::ThinkingInput::Enabled {
			budget_tokens: 2047
		})
	));
}

#[test]
fn explicit_none_reasoning_disables_thinking() {
	let (body, _) = translate(&request(json!({
		"model": "claude",
		"input": "work",
		"reasoning": {"effort": "none"}
	})))
	.expect("request should translate");
	let translated: types::messages::typed::Request =
		serde_json::from_slice(&body).expect("valid Messages request");

	assert!(matches!(
		translated.thinking,
		Some(types::messages::typed::ThinkingInput::Disabled {})
	));
}

#[test]
fn explicit_thinking_budget_takes_precedence_over_disabled_reasoning() {
	let (body, _) = translate(&request(json!({
		"model": "claude",
		"input": "work",
		"reasoning": {"effort": "none"},
		"vendor_extensions": {"thinking_budget_tokens": 1024}
	})))
	.expect("request should translate");
	let translated: types::messages::typed::Request =
		serde_json::from_slice(&body).expect("valid Messages request");

	assert!(matches!(
		translated.thinking,
		Some(types::messages::typed::ThinkingInput::Enabled {
			budget_tokens: 1024
		})
	));
}

fn sse_event(name: &str, data: serde_json::Value) -> String {
	format!("event: {name}\ndata: {data}\n\n")
}

fn message_start(input_tokens: u64) -> String {
	sse_event(
		"message_start",
		json!({
			"type": "message_start",
			"message": {
				"id": "msg_upstream",
				"type": "message",
				"role": "assistant",
				"content": [],
				"model": "upstream-model",
				"stop_reason": null,
				"stop_sequence": null,
				"usage": {"input_tokens": input_tokens, "output_tokens": 0}
			}
		}),
	)
}

fn terminal(stop_reason: &str, output_tokens: u64) -> Vec<String> {
	vec![
		sse_event(
			"message_delta",
			json!({
				"type": "message_delta",
				"delta": {"stop_reason": stop_reason, "stop_sequence": null},
				"usage": {"output_tokens": output_tokens}
			}),
		),
		sse_event("message_stop", json!({"type": "message_stop"})),
	]
}

async fn collect_stream(
	frames: Vec<String>,
	buffer_limit: usize,
	state: State,
) -> Vec<serde_json::Value> {
	collect_stream_with_guard(frames, buffer_limit, state, StreamingUsageGuard::default()).await
}

async fn collect_stream_with_guard(
	frames: Vec<String>,
	buffer_limit: usize,
	state: State,
	guard: StreamingUsageGuard,
) -> Vec<serde_json::Value> {
	let chunks = frames
		.into_iter()
		.map(|frame| Ok::<_, Infallible>(Bytes::from(frame)));
	let body = axum_core::body::Body::from_stream(stream::iter(chunks));
	collect_stream_body(body, buffer_limit, state, guard).await
}

async fn collect_stream_body(
	body: axum_core::body::Body,
	buffer_limit: usize,
	state: State,
	guard: StreamingUsageGuard,
) -> Vec<serde_json::Value> {
	let output = translate_stream(
		body,
		buffer_limit,
		guard,
		"request-model",
		LogContentFields::default(),
		state,
	)
	.collect()
	.await
	.expect("translated stream should collect")
	.to_bytes();
	String::from_utf8(output.to_vec())
		.expect("translated stream should be UTF-8")
		.split("\n\n")
		.filter(|frame| !frame.is_empty())
		.map(|frame| {
			let data = frame
				.lines()
				.find_map(|line| line.strip_prefix("data: "))
				.expect("translated SSE data");
			serde_json::from_str(data).expect("translated SSE JSON")
		})
		.collect()
}

#[derive(Clone)]
struct TestStreamingReporter {
	info: Arc<Mutex<LLMInfo>>,
}

impl StreamingUsageReporter for TestStreamingReporter {
	fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
		f(&mut self.info.lock().expect("reporter lock"));
	}

	fn report_usage(&mut self) {}
}

fn tracking_stream() -> (StreamingUsageGuard, Arc<Mutex<LLMInfo>>) {
	let info = Arc::new(Mutex::new(LLMInfo::new(
		LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Responses,
			cache_convention: CacheTokenConvention::InputExcludesCache,
			request_model: strng::literal!("request-model"),
			provider: strng::literal!("anthropic"),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		LLMResponse::default(),
	)));
	let guard = StreamingUsageGuard::new(Box::new(TestStreamingReporter { info: info.clone() }));
	(guard, info)
}

fn assert_one_safe_error(events: &[serde_json::Value]) {
	let errors = events
		.iter()
		.filter(|event| event["type"] == "error")
		.collect::<Vec<_>>();
	assert_eq!(errors.len(), 1);
	assert_eq!(errors[0]["code"], "server_error");
	assert_eq!(
		errors[0]["message"],
		"Upstream Anthropic stream was invalid"
	);
	assert!(!events.iter().any(|event| matches!(
		event["type"].as_str(),
		Some("response.completed" | "response.incomplete")
	)));
}

#[rstest::rstest]
#[case::bad_request(400, "invalid_request_error")]
#[case::unauthorized(401, "authentication_error")]
#[case::forbidden(403, "permission_error")]
#[case::not_found(404, "not_found_error")]
#[case::conflict(409, "conflict_error")]
#[case::too_large(413, "request_too_large")]
#[case::rate_limited(429, "rate_limit_error")]
#[case::internal_server_error(500, "server_error")]
fn error_status_map_redacts_provider_data(#[case] status: u16, #[case] expected_type: &str) {
	let marker = "SENSITIVE_PROVIDER_ERROR";
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"type": "error",
			"error": {"type": "invalid_request_error", "message": marker}
		}))
		.expect("valid Anthropic error"),
	);
	let status = ::http::StatusCode::from_u16(status).expect("valid status");
	let translated = translate_error(&body, status).expect("error should translate");
	let value: serde_json::Value =
		serde_json::from_slice(&translated).expect("valid Responses error");

	assert_eq!(value["error"]["type"], expected_type);
	assert_eq!(
		value["error"]["message"],
		format!(
			"Upstream Anthropic request failed with HTTP {}",
			status.as_u16()
		)
	);
	assert!(!String::from_utf8_lossy(&translated).contains(marker));
}

#[rstest::rstest]
#[case::stored(json!({"store": true}))]
#[case::background(json!({"background": true}))]
#[case::previous_response(json!({"previous_response_id": "resp_previous"}))]
#[case::conversation(json!({"conversation": "conv_previous"}))]
#[case::prompt(json!({"prompt": {"id": "prompt_1"}}))]
#[case::tool_limit(json!({"max_tool_calls": 1}))]
#[case::automatic_truncation(json!({"truncation": "auto"}))]
#[case::priority_tier(json!({"service_tier": "priority"}))]
#[case::reasoning_context(json!({"reasoning": {"context": "all_turns"}}))]
#[case::extended_prompt_cache(json!({"prompt_cache_retention": "24h"}))]
#[case::top_logprobs(json!({"top_logprobs": 5}))]
#[case::stream_obfuscation(json!({"stream_options": {"include_obfuscation": true}}))]
#[case::verbosity_low(json!({"text": {"verbosity": "low"}}))]
#[case::verbosity_medium(json!({"text": {"verbosity": "medium"}}))]
#[case::verbosity_high(json!({"text": {"verbosity": "high"}}))]
#[case::image_detail_low(json!({
	"input": [{
		"role": "user",
		"content": [{
			"type": "input_image",
			"image_url": "data:image/png;base64,aQ==",
			"detail": "low"
		}]
	}]
}))]
#[case::image_detail_high(json!({
	"input": [{
		"role": "user",
		"content": [{
			"type": "input_image",
			"image_url": "data:image/png;base64,aQ==",
			"detail": "high"
		}]
	}]
}))]
#[case::image_detail_original(json!({
	"input": [{
		"role": "user",
		"content": [{
			"type": "input_image",
			"image_url": "data:image/png;base64,aQ==",
			"detail": "original"
		}]
	}]
}))]
#[case::file_detail_high(json!({
	"input": [{
		"role": "user",
		"content": [{
			"type": "input_file",
			"file_data": "data:application/pdf;base64,JVBERi0=",
			"detail": "high"
		}]
	}]
}))]
fn stateful_or_execution_changing_requests_are_rejected(#[case] extra: serde_json::Value) {
	let mut value = json!({"model": "claude", "input": "hello"});
	value
		.as_object_mut()
		.expect("request object")
		.extend(extra.as_object().expect("extra object").clone());

	assert!(translate(&request(value)).is_err());
}

#[rstest::rstest]
#[case::namespace(json!({
	"type": "namespace",
	"name": "tools",
	"description": "Grouped tools",
	"tools": []
}))]
#[case::local_shell(json!({"type": "local_shell"}))]
#[case::shell(json!({"type": "shell", "environment": {"type": "local"}}))]
#[case::apply_patch(json!({"type": "apply_patch"}))]
fn wrapped_tools_are_explicitly_unsupported(#[case] tool: serde_json::Value) {
	let error = translate(&request(json!({
		"model": "claude",
		"input": "hello",
		"tools": [tool]
	})))
	.expect_err("wrapped tool should be rejected");
	assert!(
		error
			.to_string()
			.contains("require a separate Anthropic Messages tool mapping")
	);
}

#[tokio::test]
async fn invalid_stream_state_transitions_emit_one_safe_error() {
	let cases = [
		vec![sse_event(
			"message_delta",
			json!({
				"type": "message_delta",
				"delta": {"stop_reason": "end_turn", "stop_sequence": null},
				"usage": {"output_tokens": 1}
			}),
		)],
		vec![
			message_start(1),
			sse_event("message_stop", json!({"type": "message_stop"})),
		],
		vec![message_start(1), message_start(1)],
	];

	for frames in cases {
		let events = collect_stream(frames, 1024 * 1024, State::default()).await;
		assert_one_safe_error(&events);
	}
}

#[tokio::test]
async fn premature_stream_eof_emits_one_safe_error() {
	let events = collect_stream(vec![message_start(1)], 1024 * 1024, State::default()).await;

	assert_one_safe_error(&events);
}

#[tokio::test]
async fn upstream_body_error_emits_one_safe_error() {
	let body = axum_core::body::Body::from_stream(stream::iter(vec![Err::<Bytes, std::io::Error>(
		std::io::Error::other("SENSITIVE_UPSTREAM_BODY_ERROR"),
	)]));
	let events = collect_stream_body(
		body,
		1024 * 1024,
		State::default(),
		StreamingUsageGuard::default(),
	)
	.await;

	assert_one_safe_error(&events);
}

#[tokio::test]
async fn upstream_error_event_emits_one_safe_error() {
	let events = collect_stream(
		vec![sse_event(
			"error",
			json!({
				"type": "error",
				"error": {
					"type": "invalid_request_error",
					"message": "SENSITIVE_UPSTREAM_ERROR"
				}
			}),
		)],
		1024 * 1024,
		State::default(),
	)
	.await;

	assert_one_safe_error(&events);
}

#[tokio::test]
async fn sse_decoder_error_emits_one_safe_error() {
	let body = axum_core::body::Body::from("data: {\"type\":\"message_start\"}\n\n");
	let events = collect_stream_body(body, 8, State::default(), StreamingUsageGuard::default()).await;

	assert_one_safe_error(&events);
}

#[tokio::test]
async fn stream_usage_overflow_emits_one_safe_error() {
	let mut frames = vec![message_start(u64::from(u32::MAX) + 1)];
	frames.extend(terminal("end_turn", 1));
	let events = collect_stream(frames, 1024 * 1024, State::default()).await;
	assert_one_safe_error(&events);
}

#[tokio::test]
async fn stream_retained_output_limit_emits_one_safe_error() {
	let mut frames = vec![message_start(1)];
	for index in 0..10 {
		frames.extend([
			sse_event(
				"content_block_start",
				json!({
					"type": "content_block_start",
					"index": index,
					"content_block": {"type": "text", "text": ""}
				}),
			),
			sse_event(
				"content_block_delta",
				json!({
					"type": "content_block_delta",
					"index": index,
					"delta": {"type": "text_delta", "text": "x".repeat(100)}
				}),
			),
			sse_event(
				"content_block_stop",
				json!({"type": "content_block_stop", "index": index}),
			),
		]);
	}
	frames.extend(terminal("end_turn", 1));

	let events = collect_stream(frames, 700, State::default()).await;
	assert_one_safe_error(&events);
}

#[tokio::test]
async fn empty_custom_tool_input_does_not_record_a_visible_token() {
	let (guard, info) = tracking_stream();
	let mut frames = vec![
		message_start(1),
		sse_event(
			"content_block_start",
			json!({
				"type": "content_block_start",
				"index": 0,
				"content_block": {
					"type": "tool_use",
					"id": "toolu_python",
					"name": "python",
					"input": {}
				}
			}),
		),
		sse_event(
			"content_block_delta",
			json!({
				"type": "content_block_delta",
				"index": 0,
				"delta": {"type": "input_json_delta", "partial_json": "{\"content\":\"\"}"}
			}),
		),
		sse_event(
			"content_block_stop",
			json!({"type": "content_block_stop", "index": 0}),
		),
	];
	frames.extend(terminal("tool_use", 1));

	let events = collect_stream_with_guard(frames, 1024 * 1024, response_state(), guard).await;

	assert!(
		events
			.iter()
			.any(|event| event["type"] == "response.completed")
	);
	assert_eq!(
		info.lock().expect("reporter lock").response.first_token,
		None
	);
}

#[test]
fn buffered_response_output_limit_is_enforced() {
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"id": "msg_1",
			"type": "message",
			"role": "assistant",
			"model": "upstream-model",
			"content": [{"type": "text", "text": "x".repeat(1024)}],
			"stop_reason": "end_turn",
			"stop_sequence": null,
			"usage": {"input_tokens": 1, "output_tokens": 1}
		}))
		.expect("valid response"),
	);

	assert!(translate_response(&body, &State::default(), 256).is_err());
}

#[test]
fn buffered_pause_turn_is_rejected() {
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"id": "msg_1",
			"type": "message",
			"role": "assistant",
			"model": "upstream-model",
			"content": [],
			"stop_reason": "pause_turn",
			"stop_sequence": null,
			"usage": {"input_tokens": 1, "output_tokens": 1}
		}))
		.expect("valid response"),
	);

	assert!(translate_response(&body, &State::default(), 1024 * 1024).is_err());
}

#[test]
fn buffered_refusal_is_a_completed_refusal() {
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"id": "msg_1",
			"type": "message",
			"role": "assistant",
			"model": "upstream-model",
			"content": [{"type": "text", "text": "I cannot help with that."}],
			"stop_reason": "refusal",
			"stop_sequence": null,
			"usage": {"input_tokens": 1, "output_tokens": 6}
		}))
		.expect("valid response"),
	);

	let response =
		translate_response(&body, &State::default(), 1024 * 1024).expect("refusal should translate");
	let value = serde_json::to_value(response).expect("serializable response");

	assert_eq!(value["status"], "completed");
	assert_eq!(value["output"][0]["status"], "completed");
	assert_eq!(value["output"][0]["content"][0]["type"], "refusal");
	assert_eq!(
		value["output"][0]["content"][0]["refusal"],
		"I cannot help with that."
	);
}

#[tokio::test]
async fn streaming_text_is_emitted_before_the_terminal_event() {
	let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
	let upstream =
		axum_core::body::Body::from_stream(stream::poll_fn(move |cx| receiver.poll_recv(cx)));
	let mut downstream = translate_stream(
		upstream,
		1024 * 1024,
		StreamingUsageGuard::default(),
		"request-model",
		LogContentFields::default(),
		State::default(),
	);
	sender
		.send(Ok::<_, Infallible>(Bytes::from(format!(
			"{}{}{}",
			message_start(1),
			sse_event(
				"content_block_start",
				json!({
					"type": "content_block_start",
					"index": 0,
					"content_block": {"type": "text", "text": ""}
				})
			),
			sse_event(
				"content_block_delta",
				json!({
					"type": "content_block_delta",
					"index": 0,
					"delta": {"type": "text_delta", "text": "hello"}
				})
			)
		))))
		.expect("upstream receiver should remain open");

	tokio::time::timeout(Duration::from_secs(1), async {
		loop {
			let frame = downstream
				.frame()
				.await
				.expect("downstream should remain open")
				.expect("downstream frame should be valid");
			let data = frame.into_data().expect("SSE frame should contain data");
			if String::from_utf8_lossy(&data).contains("response.output_text.delta") {
				break;
			}
		}
	})
	.await
	.expect("text delta should be emitted before the terminal event");
}

#[tokio::test]
async fn streaming_refusal_completes_as_output_text() {
	let mut frames = vec![
		message_start(1),
		sse_event(
			"content_block_start",
			json!({
				"type": "content_block_start",
				"index": 0,
				"content_block": {"type": "text", "text": ""}
			}),
		),
		sse_event(
			"content_block_delta",
			json!({
				"type": "content_block_delta",
				"index": 0,
				"delta": {"type": "text_delta", "text": "I cannot help with that."}
			}),
		),
		sse_event(
			"content_block_stop",
			json!({"type": "content_block_stop", "index": 0}),
		),
	];
	frames.extend(terminal("refusal", 6));

	let events = collect_stream(frames, 1024 * 1024, State::default()).await;

	assert!(
		events
			.iter()
			.any(|event| event["type"] == "response.output_text.delta")
	);
	assert!(!events.iter().any(|event| event["type"] == "error"));
	let completed = events
		.iter()
		.find(|event| event["type"] == "response.completed")
		.expect("completed response");
	assert_eq!(
		completed["response"]["output"][0]["content"][0],
		json!({
			"type": "output_text",
			"annotations": [],
			"logprobs": null,
			"text": "I cannot help with that."
		})
	);
}

#[test]
fn buffered_programmatic_tool_caller_is_rejected() {
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"id": "msg_1",
			"type": "message",
			"role": "assistant",
			"model": "upstream-model",
			"content": [{
				"type": "tool_use",
				"id": "toolu_1",
				"name": "get_weather",
				"input": {},
				"caller": {"type": "code_execution_20250825"}
			}],
			"stop_reason": "tool_use",
			"stop_sequence": null,
			"usage": {"input_tokens": 1, "output_tokens": 1}
		}))
		.expect("valid response"),
	);

	assert!(translate_response(&body, &response_state(), 1024 * 1024).is_err());
}

#[test]
fn malformed_buffered_response_is_rejected_without_reflection() {
	let marker = "SENSITIVE_MALFORMED_RESPONSE";
	let body = Bytes::from(format!(
		r#"{{"id":"msg_1","type":"message","role":"assistant","model":"{marker}","content":[]}}"#
	));
	let error = match translate_response(&body, &response_state(), 1024 * 1024) {
		Ok(_) => panic!("malformed response should fail"),
		Err(error) => error,
	};

	assert!(!error.to_string().contains(marker));
}
