use std::convert::Infallible;
use std::fs;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::{StreamExt, stream};
use http_body_util::BodyExt;
use serde_json::json;

use super::{
	State, retain_response_item, translate, translate_error, translate_response, translate_stream,
};
use crate::{
	CacheTokenConvention, InputFormat, LLMInfo, LLMRequest, LLMRequestParams, LLMResponse,
	StreamingUsageGuard, StreamingUsageReporter, types,
};

fn raw_request(value: serde_json::Value) -> types::responses::Request {
	serde_json::from_value(value).expect("valid local Responses request")
}

fn request(mut value: serde_json::Value) -> types::responses::Request {
	let object = value.as_object_mut().expect("request object");
	object.entry("store").or_insert_with(|| json!(false));
	if object.get("stream").and_then(serde_json::Value::as_bool) == Some(true) {
		object
			.entry("stream_options")
			.or_insert_with(|| json!({"include_obfuscation": false}));
	}
	raw_request(value)
}

fn translated(value: serde_json::Value) -> serde_json::Value {
	let (body, _) = translate(&request(value)).expect("request should translate");
	serde_json::from_slice(&body).expect("valid Messages request")
}

fn translation_fails(value: serde_json::Value) -> bool {
	translate(&request(value)).is_err()
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
#[case::bad_gateway(502, "server_error")]
fn error_status_map_is_closed_and_provider_data_is_redacted(
	#[case] status: u16,
	#[case] expected_type: &str,
) {
	let markers = [
		"SENSITIVE_REPLAY_CARRIER",
		"SENSITIVE_THINKING_SIGNATURE",
		"SENSITIVE_REDACTED_DATA",
		"SENSITIVE_TOOL_ARGUMENTS",
	];
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"type": "error",
			"error": {
				"type": "invalid_request_error",
				"message": markers.join(" ")
			}
		}))
		.expect("valid Anthropic error envelope"),
	);
	let status = ::http::StatusCode::from_u16(status).expect("valid test status");

	let translated = translate_error(&body, status).expect("error should translate");
	let value: serde_json::Value =
		serde_json::from_slice(&translated).expect("valid Responses error");

	assert_eq!(
		value,
		json!({
			"error": {
				"message": format!(
					"Upstream Anthropic request failed with HTTP {}",
					status.as_u16()
				),
				"type": expected_type,
				"param": null,
				"code": null
			}
		})
	);
	let translated = String::from_utf8_lossy(&translated);
	for marker in markers {
		assert!(
			!translated.contains(marker),
			"translated error leaked {marker}"
		);
	}
}

#[test]
fn error_malformed_json_is_not_reflected() {
	let marker = "SENSITIVE_MALFORMED_JSON";
	let translated = translate_error(
		&Bytes::from(format!(r#"{{"message":"{marker}""#)),
		::http::StatusCode::BAD_GATEWAY,
	)
	.expect("malformed provider error should translate");
	let value: serde_json::Value =
		serde_json::from_slice(&translated).expect("valid Responses error");

	assert_eq!(value["error"]["type"], "server_error");
	assert_eq!(
		value["error"]["message"],
		"Upstream Anthropic request failed with HTTP 502"
	);
	assert!(!String::from_utf8_lossy(&translated).contains(marker));
}

fn tool_declarations() -> serde_json::Value {
	json!([
		{
			"type": "function",
			"name": "weather",
			"description": "Get weather",
			"parameters": {
				"type": "object",
				"properties": {"city": {"type": "string"}},
				"required": ["city"]
			},
			"strict": false
		},
		{
			"type": "namespace",
			"name": "crm",
			"description": "CRM tools",
			"tools": [
				{
					"type": "function",
					"name": "lookup",
					"parameters": {"type": "object"}
				},
				{
					"type": "custom",
					"name": "query",
					"format": {"type": "text"}
				}
			]
		},
		{
			"type": "custom",
			"name": "python",
			"description": "Run Python",
			"format": {"type": "text"}
		},
		{"type": "local_shell"},
		{"type": "shell", "environment": {"type": "local"}},
		{"type": "apply_patch"}
	])
}

fn buffered_state() -> State {
	let (body, state) = translate(&request(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations()
	})))
	.expect("state request should translate");
	assert!(!body.is_empty());
	state
}

fn sse_event(name: &str, data: serde_json::Value) -> String {
	format!("event: {name}\ndata: {data}\n\n")
}

async fn translated_stream(frames: Vec<String>, state: State) -> Vec<serde_json::Value> {
	translated_stream_with(frames, state, StreamingUsageGuard::default(), false).await
}

async fn translated_stream_with(
	frames: Vec<String>,
	state: State,
	logger: StreamingUsageGuard,
	include_completion_in_log: bool,
) -> Vec<serde_json::Value> {
	let chunks = frames
		.into_iter()
		.flat_map(|frame| {
			let split = frame.len().min(7);
			let (first, second) = frame.split_at(split);
			[first.to_string(), second.to_string()]
		})
		.map(|chunk| Ok::<_, Infallible>(Bytes::from(chunk)));
	let body = axum_core::body::Body::from_stream(stream::iter(chunks));
	let output = translate_stream(
		body,
		1024 * 1024,
		logger,
		"claude-sonnet-4-5",
		include_completion_in_log,
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

async fn translated_body(
	body: axum_core::body::Body,
	buffer_limit: usize,
) -> Vec<serde_json::Value> {
	let output = translate_stream(
		body,
		buffer_limit,
		StreamingUsageGuard::default(),
		"claude-sonnet-4-5",
		false,
		State::default(),
	)
	.collect()
	.await
	.expect("strict stream errors should be translated")
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

fn stream_message_start() -> String {
	stream_message_start_with_usage(json!({"input_tokens":2,"output_tokens":0}))
}

fn stream_message_start_with_usage(usage: serde_json::Value) -> String {
	sse_event(
		"message_start",
		json!({
			"type":"message_start",
			"message": {
				"id":"msg_upstream_123", "type":"message", "role":"assistant",
				"content":[], "model":"claude-upstream", "stop_reason":null,
				"stop_sequence":null,
				"usage":usage
			}
		}),
	)
}

fn stream_terminal(stop_reason: &str, output_tokens: usize) -> Vec<String> {
	stream_terminal_with(
		stop_reason,
		serde_json::Value::Null,
		json!({"output_tokens":output_tokens}),
	)
}

fn stream_terminal_with(
	stop_reason: &str,
	stop_sequence: serde_json::Value,
	usage: serde_json::Value,
) -> Vec<String> {
	vec![
		sse_event(
			"message_delta",
			json!({
				"type":"message_delta",
				"delta":{"stop_reason":stop_reason,"stop_sequence":stop_sequence},
				"usage":usage
			}),
		),
		sse_event("message_stop", json!({"type":"message_stop"})),
	]
}

#[tokio::test]
async fn stream_complete_fixture_has_exact_public_event_order() {
	let fixture = fs::read_to_string("src/tests/response/anthropic/responses-complete-stream.json")
		.expect("stream fixture should be readable");
	let frames = fixture
		.split("\n\n")
		.filter(|frame| !frame.trim().is_empty())
		.map(|frame| format!("{frame}\n\n"))
		.collect();
	let events = translated_stream(frames, buffered_state()).await;
	let event_types = events
		.iter()
		.map(|event| event["type"].as_str().expect("event type"))
		.collect::<Vec<_>>();

	assert_eq!(
		event_types,
		vec![
			"response.created",
			"response.in_progress",
			"response.output_item.added",
			"response.content_part.added",
			"response.output_text.delta",
			"response.output_text.done",
			"response.content_part.done",
			"response.output_item.added",
			"response.function_call_arguments.delta",
			"response.function_call_arguments.delta",
			"response.function_call_arguments.done",
			"response.output_item.done",
			"response.output_item.done",
			"response.completed",
		]
	);
	assert_eq!(
		events
			.iter()
			.map(|event| event["sequence_number"].as_u64().expect("sequence number"))
			.collect::<Vec<_>>(),
		(0_u64..events.len() as u64).collect::<Vec<_>>()
	);
	assert_eq!(
		events
			.iter()
			.find(|event| event["type"] == "response.function_call_arguments.done")
			.expect("function call arguments done")["arguments"],
		serde_json::Value::String(r#"{"city":"Seattle"}"#.to_string())
	);
}

#[tokio::test]
async fn stream_text_fragmented_lifecycle_is_standard_and_contiguous() {
	let frames = vec![
		sse_event(
			"message_start",
			json!({
				"type":"message_start",
				"message": {
					"id":"msg_upstream_123", "type":"message", "role":"assistant",
					"content":[], "model":"claude-upstream", "stop_reason":null,
					"stop_sequence":null,
					"usage":{"input_tokens":2,"output_tokens":0}
				}
			}),
		),
		sse_event(
			"content_block_start",
			json!({
				"type":"content_block_start", "index":0,
				"content_block":{"type":"text","text":""}
			}),
		),
		sse_event(
			"content_block_delta",
			json!({
				"type":"content_block_delta", "index":0,
				"delta":{"type":"text_delta","text":"hel"}
			}),
		),
		sse_event(
			"content_block_delta",
			json!({
				"type":"content_block_delta", "index":0,
				"delta":{"type":"text_delta","text":"lo"}
			}),
		),
		sse_event(
			"content_block_stop",
			json!({"type":"content_block_stop","index":0}),
		),
		sse_event(
			"message_delta",
			json!({
				"type":"message_delta",
				"delta":{"stop_reason":"end_turn","stop_sequence":null},
				"usage":{"output_tokens":1}
			}),
		),
		sse_event("message_stop", json!({"type":"message_stop"})),
	];

	let events = translated_stream(frames, State::default()).await;
	assert_eq!(
		events
			.iter()
			.map(|event| event["type"].as_str().expect("event type"))
			.collect::<Vec<_>>(),
		vec![
			"response.created",
			"response.in_progress",
			"response.output_item.added",
			"response.content_part.added",
			"response.output_text.delta",
			"response.output_text.delta",
			"response.output_text.done",
			"response.content_part.done",
			"response.output_item.done",
			"response.completed",
		]
	);
	assert_eq!(
		events
			.iter()
			.map(|event| event["sequence_number"].as_u64().expect("sequence number"))
			.collect::<Vec<_>>(),
		(0_u64..10).collect::<Vec<_>>()
	);
	assert_eq!(events[8]["item"], events[9]["response"]["output"][0]);
}

#[tokio::test]
async fn stream_late_refusal_after_streamed_text_fails_instead_of_rewriting() {
	// Anthropic only reveals a refusal via the terminal stop_reason, but Responses commits to
	// output_text vs refusal typing as soon as content_part.added is sent. Once text has actually
	// streamed to the client, it cannot be retyped, so a late refusal must fail the stream instead
	// of silently relabeling already-emitted content as a refusal.
	let mut frames = vec![
		stream_message_start(),
		block_start(0, json!({"type":"text","text":""})),
		block_delta(0, json!({"type":"text_delta","text":"cannot"})),
		block_delta(0, json!({"type":"text_delta","text":" comply"})),
		block_stop(0),
	];
	frames.extend(stream_terminal("refusal", 2));
	let events = translated_stream(frames, State::default()).await;
	assert_eq!(
		events
			.iter()
			.map(|event| event["type"].as_str().expect("event type"))
			.collect::<Vec<_>>(),
		vec![
			"response.created",
			"response.in_progress",
			"response.output_item.added",
			"response.content_part.added",
			"response.output_text.delta",
			"response.output_text.delta",
			"response.output_text.done",
			"response.content_part.done",
			"error",
		]
	);
	assert_eq!(
		events
			.iter()
			.map(|event| event["sequence_number"].as_u64().expect("sequence"))
			.collect::<Vec<_>>(),
		(0..9).collect::<Vec<_>>()
	);
	assert_eq!(events[4]["delta"], "cannot");
	assert_eq!(events[5]["delta"], " comply");
	assert_eq!(events[6]["text"], "cannot comply");
	let error = events.last().expect("error event");
	assert_eq!(error["code"], "refusal_after_streaming");
}

#[tokio::test]
async fn stream_late_refusal_with_only_empty_text_block_still_fails() {
	// Even zero characters of text still commits content_part.added to type "output_text" on
	// the wire, so a late refusal must still fail rather than pretend nothing was typed.
	let mut frames = vec![
		stream_message_start(),
		block_start(0, json!({"type":"text","text":""})),
		block_stop(0),
	];
	frames.extend(stream_terminal("refusal", 1));
	let events = translated_stream(frames, State::default()).await;
	assert_eq!(events.last().expect("terminal event")["type"], "error");
	assert_eq!(
		events.last().expect("terminal event")["code"],
		"refusal_after_streaming"
	);
}

#[tokio::test]
async fn stream_rejects_thinking_blocks_after_optional_completed_text() {
	let cases = [
		(
			"thinking",
			json!({"type":"thinking","thinking":"","signature":""}),
		),
		(
			"redacted",
			json!({"type":"redacted_thinking","data":"opaque"}),
		),
	];
	for (case, block) in cases {
		for after_text in [false, true] {
			let mut frames = vec![stream_message_start()];
			if after_text {
				frames.extend([
					block_start(0, json!({"type":"text","text":""})),
					block_delta(0, json!({"type":"text_delta","text":"secret"})),
					block_stop(0),
				]);
			}
			frames.push(block_start(usize::from(after_text), block.clone()));
			let events = translated_stream(frames, State::default()).await;
			// Completed content that streamed before the thinking block is real and already
			// delivered; only content after the rejection point must never leak.
			let expected_types: Vec<&str> = if after_text {
				vec![
					"response.created",
					"response.in_progress",
					"response.output_item.added",
					"response.content_part.added",
					"response.output_text.delta",
					"response.output_text.done",
					"response.content_part.done",
					"error",
				]
			} else {
				vec!["response.created", "response.in_progress", "error"]
			};
			assert_eq!(
				events
					.iter()
					.map(|event| event["type"].as_str().expect("type"))
					.collect::<Vec<_>>(),
				expected_types,
				"{case} after_text={after_text}: {events:?}"
			);
			assert_eq!(
				events
					.iter()
					.map(|event| event["sequence_number"].as_u64().expect("sequence"))
					.collect::<Vec<_>>(),
				(0..events.len() as u64).collect::<Vec<_>>(),
				"{case} after_text={after_text}"
			);
			let serialized = serde_json::to_string(&events).unwrap();
			if after_text {
				assert!(
					serialized.contains("secret"),
					"{case}: already-completed text must stream even though a later block is rejected"
				);
			} else {
				assert!(!serialized.contains("secret"), "{case}: {events:?}");
			}
		}
	}
}

#[tokio::test]
async fn stream_empty_text_keeps_completed_output_part_snapshot() {
	let mut frames = vec![
		stream_message_start(),
		block_start(0, json!({"type":"text","text":""})),
		block_stop(0),
	];
	frames.extend(stream_terminal("end_turn", 0));
	let events = translated_stream(frames, State::default()).await;
	let added = events
		.iter()
		.find(|event| event["type"] == "response.output_item.added")
		.expect("added item");
	assert_eq!(added["item"]["content"], json!([]));
	let done = events
		.iter()
		.find(|event| event["type"] == "response.output_item.done")
		.expect("done item");
	assert_eq!(
		done["item"]["content"],
		json!([{"type":"output_text","annotations":[],"logprobs":null,"text":""}])
	);
	assert_eq!(
		done["item"],
		events.last().unwrap()["response"]["output"][0]
	);
}

#[tokio::test]
async fn stream_adjacent_text_blocks_share_one_message_item() {
	let mut frames = vec![
		stream_message_start(),
		block_start(0, json!({"type":"text","text":""})),
		block_delta(0, json!({"type":"text_delta","text":"one"})),
		block_stop(0),
		block_start(1, json!({"type":"text","text":""})),
		block_delta(1, json!({"type":"text_delta","text":"two"})),
		block_stop(1),
	];
	frames.extend(stream_terminal("end_turn", 2));

	let events = translated_stream(frames, State::default()).await;
	let terminal = events.last().expect("terminal event");
	let output = terminal["response"]["output"].as_array().expect("output");
	assert_eq!(output.len(), 1);
	assert_eq!(
		output[0]["content"],
		json!([
			{"type":"output_text","annotations":[],"logprobs":null,"text":"one"},
			{"type":"output_text","annotations":[],"logprobs":null,"text":"two"}
		])
	);
	assert_eq!(
		events
			.iter()
			.filter(|event| event["type"] == "response.output_item.added")
			.count(),
		1
	);
	assert_eq!(
		events
			.iter()
			.filter(|event| event["type"] == "response.output_item.done")
			.count(),
		1
	);
	assert_eq!(
		events
			.iter()
			.filter(|event| event["type"] == "response.content_part.added")
			.map(|event| event["content_index"].as_u64().expect("content index"))
			.collect::<Vec<_>>(),
		vec![0, 1]
	);
}

#[tokio::test]
async fn stream_terminal_lifecycles_preserve_closed_item_statuses() {
	let cases = [
		(
			"end_turn",
			serde_json::Value::Null,
			"response.completed",
			None,
			true,
			false,
		),
		(
			"tool_use",
			serde_json::Value::Null,
			"response.completed",
			None,
			false,
			true,
		),
		(
			"stop_sequence",
			json!("STOP"),
			"response.completed",
			None,
			true,
			false,
		),
		// A refusal without any streamed text (nothing to retype) still completes normally,
		// matching the non-streaming translate_response semantics. Refusal after text has
		// already streamed is covered separately by
		// stream_late_refusal_after_streamed_text_fails_instead_of_rewriting.
		(
			"refusal",
			serde_json::Value::Null,
			"response.completed",
			None,
			false,
			false,
		),
		(
			"max_tokens",
			serde_json::Value::Null,
			"response.incomplete",
			Some("max_output_tokens"),
			true,
			false,
		),
		(
			"model_context_window_exceeded",
			serde_json::Value::Null,
			"response.incomplete",
			Some("max_output_tokens"),
			true,
			false,
		),
	];
	for (reason, sequence, terminal_type, incomplete, with_text, with_tool) in cases {
		let state = if with_tool {
			buffered_state()
		} else {
			State::default()
		};
		let mut frames = vec![stream_message_start()];
		if with_text {
			frames.extend([
				block_start(0, json!({"type":"text","text":""})),
				block_delta(0, json!({"type":"text_delta","text":"answer"})),
				block_stop(0),
			]);
		} else if with_tool {
			frames.extend([
				block_start(
					0,
					json!({"type":"tool_use","id":"call","name":"weather","input":{}}),
				),
				block_stop(0),
			]);
		}
		frames.extend(stream_terminal_with(
			reason,
			sequence,
			json!({"output_tokens":1}),
		));
		let events = translated_stream(frames, state).await;
		let terminal = events.last().expect("terminal event");
		assert_eq!(terminal["type"], terminal_type, "{reason}");
		assert_eq!(
			terminal["response"]["incomplete_details"]["reason"].as_str(),
			incomplete,
			"{reason}"
		);
		if with_text || with_tool {
			let expected_item_status = if terminal_type == "response.incomplete" {
				"incomplete"
			} else {
				"completed"
			};
			assert_eq!(
				terminal["response"]["output"][0]["status"], expected_item_status,
				"{reason}"
			);
		} else {
			assert_eq!(
				terminal["response"]["output"][0]["content"][0],
				json!({"type": "refusal", "refusal": ""}),
				"empty refusal"
			);
		}
	}
}

#[tokio::test]
async fn stream_empty_refusal_emits_added_before_done() {
	let frames = [
		vec![stream_message_start()],
		stream_terminal_with(
			"refusal",
			serde_json::Value::Null,
			json!({"output_tokens":1}),
		),
	]
	.concat();
	let events = translated_stream(frames, State::default()).await;

	let event_types = events
		.iter()
		.map(|event| event["type"].as_str().expect("event type"))
		.collect::<Vec<_>>();
	assert_eq!(
		event_types,
		vec![
			"response.created",
			"response.in_progress",
			"response.output_item.added",
			"response.content_part.added",
			"response.refusal.done",
			"response.content_part.done",
			"response.output_item.done",
			"response.completed",
		]
	);
	assert_eq!(events[2]["item"]["content"], json!([]));
	assert_eq!(events[3]["part"], json!({"type": "refusal", "refusal": ""}));
	assert_eq!(events[4]["refusal"], "");
	assert_eq!(events[5]["part"], json!({"type": "refusal", "refusal": ""}));
}

#[tokio::test]
async fn stream_pause_turn_emits_one_safe_error() {
	let mut frames = text_stream_prefix();
	frames.push(block_stop(0));
	frames.extend(stream_terminal("pause_turn", 1));
	let events = translated_stream(frames, State::default()).await;
	assert_one_safe_stream_error(&events, "pause turn");
}

fn block_start(index: usize, content_block: serde_json::Value) -> String {
	sse_event(
		"content_block_start",
		json!({
			"type":"content_block_start", "index":index, "content_block":content_block
		}),
	)
}

fn block_delta(index: usize, delta: serde_json::Value) -> String {
	sse_event(
		"content_block_delta",
		json!({
			"type":"content_block_delta", "index":index, "delta":delta
		}),
	)
}

fn block_stop(index: usize) -> String {
	sse_event(
		"content_block_stop",
		json!({"type":"content_block_stop","index":index}),
	)
}

#[tokio::test]
async fn stream_tools_ping_and_sequential_blocks_use_standard_lifecycles() {
	let state = buffered_state();
	let mut frames = vec![
		stream_message_start(),
		sse_event("ping", json!({"type":"ping"})),
		block_start(
			0,
			json!({"type":"tool_use","id":"call_0","name":"weather","input":{}}),
		),
		block_delta(
			0,
			json!({"type":"input_json_delta","partial_json":"{\"city\":\"Pa"}),
		),
		block_delta(
			0,
			json!({"type":"input_json_delta","partial_json":"ris\"}"}),
		),
		block_stop(0),
		block_start(
			1,
			json!({"type":"tool_use","id":"call_1","name":"weather","input":{}}),
		),
		block_stop(1),
		block_start(
			2,
			json!({"type":"tool_use","id":"call_2","name":"agentgateway__responses__namespace_function_1","input":{}}),
		),
		block_delta(
			2,
			json!({"type":"input_json_delta","partial_json":"{\"arguments\":{\"id\":1}}"}),
		),
		block_stop(2),
		block_start(
			3,
			json!({"type":"tool_use","id":"call_3","name":"agentgateway__responses__custom_3","input":{}}),
		),
		block_delta(
			3,
			json!({"type":"input_json_delta","partial_json":"{\"input\":\"print(1)\"}"}),
		),
		block_stop(3),
		block_start(
			4,
			json!({"type":"tool_use","id":"call_4","name":"agentgateway__responses__local_shell_4","input":{}}),
		),
		block_delta(
			4,
			json!({"type":"input_json_delta","partial_json":"{\"action\":{\"command\":[\"pwd\"],\"env\":{},\"timeout_ms\":1000,\"user\":null,\"working_directory\":\"/tmp\"}}"}),
		),
		block_stop(4),
		block_start(
			5,
			json!({"type":"tool_use","id":"call_5","name":"agentgateway__responses__shell_5","input":{}}),
		),
		block_delta(
			5,
			json!({"type":"input_json_delta","partial_json":"{\"action\":{\"commands\":[\"true\"],\"timeout_ms\":1000,\"max_output_length\":1024}}"}),
		),
		block_stop(5),
		block_start(
			6,
			json!({"type":"tool_use","id":"call_6","name":"agentgateway__responses__apply_patch_6","input":{}}),
		),
		block_delta(
			6,
			json!({"type":"input_json_delta","partial_json":"{\"operation\":{\"type\":\"update_file\",\"path\":\"src/lib.rs\",\"diff\":\"@@\"}}"}),
		),
		block_stop(6),
	];
	frames.extend(stream_terminal("tool_use", 7));

	let events = translated_stream(frames, state).await;
	assert_eq!(
		events.last().and_then(|event| event["type"].as_str()),
		Some("response.completed")
	);
	assert_eq!(
		events.last().expect("terminal")["response"]["output"]
			.as_array()
			.expect("output")
			.len(),
		7
	);

	let per_output = |index: u64| {
		events
			.iter()
			.filter(|event| event["output_index"].as_u64() == Some(index))
			.map(|event| event["type"].as_str().expect("event type"))
			.collect::<Vec<_>>()
	};
	assert_eq!(
		per_output(0),
		vec![
			"response.output_item.added",
			"response.function_call_arguments.delta",
			"response.function_call_arguments.delta",
			"response.function_call_arguments.done",
			"response.output_item.done"
		]
	);
	assert_eq!(
		per_output(1),
		vec![
			"response.output_item.added",
			"response.function_call_arguments.done",
			"response.output_item.done"
		]
	);
	assert_eq!(
		per_output(2),
		vec![
			"response.output_item.added",
			"response.function_call_arguments.delta",
			"response.function_call_arguments.done",
			"response.output_item.done"
		]
	);
	assert_eq!(
		per_output(3),
		vec![
			"response.output_item.added",
			"response.custom_tool_call_input.delta",
			"response.custom_tool_call_input.done",
			"response.output_item.done"
		]
	);
	for index in 4..=6 {
		assert_eq!(
			per_output(index),
			vec!["response.output_item.added", "response.output_item.done"]
		);
	}
	assert_eq!(
		events.last().expect("terminal")["response"]["output"][0]["arguments"],
		r#"{"city":"Paris"}"#
	);
	assert_eq!(
		events.last().expect("terminal")["response"]["output"][1]["arguments"],
		"{}"
	);
	assert_eq!(
		events.last().expect("terminal")["response"]["output"][2]["namespace"],
		"crm"
	);
	assert_eq!(
		events.last().expect("terminal")["response"]["output"][3]["input"],
		"print(1)"
	);
	let done_items = events
		.iter()
		.filter(|event| event["type"] == "response.output_item.done")
		.map(|event| event["item"].clone())
		.collect::<Vec<_>>();
	assert_eq!(
		events.last().expect("terminal")["response"]["output"],
		json!(done_items)
	);
}

#[tokio::test]
async fn stream_ping_after_message_delta_before_stop_is_accepted() {
	let frames = vec![
		stream_message_start(),
		stream_terminal("end_turn", 1)[0].clone(),
		sse_event("ping", json!({"type":"ping"})),
		sse_event("message_stop", json!({"type":"message_stop"})),
	];
	let events = translated_stream(frames, State::default()).await;
	assert_eq!(
		events.last().and_then(|event| event["type"].as_str()),
		Some("response.completed")
	);
	assert!(events.iter().all(|event| event["type"] != "error"));
}

#[tokio::test]
async fn stream_ping_before_message_start_is_accepted() {
	// Anthropic may send a content-free ping keepalive at any point, including while a request
	// is queued before message_start -- unlike every other event type, it references no
	// established message state, so it must not be treated as an out-of-order error.
	let mut frames = vec![
		sse_event("ping", json!({"type":"ping"})),
		stream_message_start(),
	];
	frames.extend(stream_terminal("end_turn", 1));
	let events = translated_stream(frames, State::default()).await;
	assert!(events.iter().all(|event| event["type"] != "error"));
	assert_eq!(
		events.last().and_then(|event| event["type"].as_str()),
		Some("response.completed")
	);
}

#[tokio::test]
async fn stream_namespace_custom_uses_custom_lifecycle_and_restores_identity() {
	let state = buffered_state();
	let mut frames = vec![
		stream_message_start(),
		block_start(
			0,
			json!({"type":"tool_use","id":"call","name":"agentgateway__responses__namespace_custom_2","input":{}}),
		),
		block_delta(
			0,
			json!({"type":"input_json_delta","partial_json":"{\"input\":\"select 1\"}"}),
		),
		block_stop(0),
	];
	frames.extend(stream_terminal("tool_use", 1));
	let events = translated_stream(frames, state).await;
	assert_eq!(
		events
			.iter()
			.filter(|event| event["output_index"] == 0)
			.map(|event| event["type"].as_str().unwrap())
			.collect::<Vec<_>>(),
		vec![
			"response.output_item.added",
			"response.custom_tool_call_input.delta",
			"response.custom_tool_call_input.done",
			"response.output_item.done"
		]
	);
	let item = &events.last().unwrap()["response"]["output"][0];
	assert_eq!(item["namespace"], "crm");
	assert_eq!(item["name"], "query");
	assert_eq!(item["input"], "select 1");
	let done = events
		.iter()
		.find(|event| event["type"] == "response.output_item.done")
		.expect("done item");
	assert_eq!(done["item"], *item);
}

#[tokio::test]
async fn stream_wrapped_tool_restored_delta_is_emitted_once_before_done() {
	for (name, partial, delta_type) in [
		(
			"agentgateway__responses__namespace_function_1",
			"{\"arguments\":{\"id\":1}}",
			"response.function_call_arguments.delta",
		),
		(
			"agentgateway__responses__custom_3",
			"{\"input\":\"print(1)\"}",
			"response.custom_tool_call_input.delta",
		),
	] {
		let mut frames = vec![
			stream_message_start(),
			block_start(
				0,
				json!({"type":"tool_use","id":"call","name":name,"input":{}}),
			),
			block_delta(0, json!({"type":"input_json_delta","partial_json":partial})),
			block_delta(0, json!({"type":"input_json_delta","partial_json":" \n"})),
			block_stop(0),
		];
		frames.extend(stream_terminal("tool_use", 1));
		let events = translated_stream(frames, buffered_state()).await;
		assert_eq!(
			events
				.iter()
				.filter(|event| event["type"] == delta_type)
				.count(),
			1,
			"{name}: {events:?}"
		);
	}
}

struct TestReporter {
	info: Arc<Mutex<LLMInfo>>,
	first_token_updates: Arc<Mutex<usize>>,
	updates: Arc<Mutex<usize>>,
}

impl StreamingUsageReporter for TestReporter {
	fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
		*self.updates.lock().expect("updates lock") += 1;
		let mut info = self.info.lock().expect("test reporter lock");
		let before = info.response.first_token;
		f(&mut info);
		if before.is_none() && info.response.first_token.is_some() {
			*self.first_token_updates.lock().expect("counter lock") += 1;
		}
	}

	fn report_usage(&mut self) {}
}

fn test_info() -> LLMInfo {
	LLMInfo::new(
		LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Responses,
			cache_convention: CacheTokenConvention::InputIncludesCache,
			request_model: agent_core::strng::literal!("claude-sonnet-4-5"),
			provider: agent_core::strng::literal!("test"),
			streaming: true,
			params: LLMRequestParams::default(),
			prompt: None,
			provider_state: None,
		},
		LLMResponse::default(),
	)
}

#[allow(clippy::type_complexity)]
fn test_logger() -> (
	StreamingUsageGuard,
	Arc<Mutex<LLMInfo>>,
	Arc<Mutex<usize>>,
	Arc<Mutex<usize>>,
) {
	let info = Arc::new(Mutex::new(test_info()));
	let first = Arc::new(Mutex::new(0));
	let updates = Arc::new(Mutex::new(0));
	(
		StreamingUsageGuard::new(Box::new(TestReporter {
			info: info.clone(),
			first_token_updates: first.clone(),
			updates: updates.clone(),
		})),
		info,
		first,
		updates,
	)
}

#[tokio::test]
async fn stream_first_visible_token_and_completion_telemetry_update_once() {
	let (logger, info, updates, calls) = test_logger();
	let mut frames = vec![
		stream_message_start(),
		block_start(0, json!({"type":"text","text":""})),
		block_delta(0, json!({"type":"text_delta","text":"a"})),
		block_delta(0, json!({"type":"text_delta","text":"b"})),
		block_stop(0),
	];
	frames.extend(stream_terminal("end_turn", 2));
	let _ = translated_stream_with(frames, State::default(), logger, true).await;

	assert_eq!(*updates.lock().expect("counter lock"), 1);
	assert_eq!(*calls.lock().expect("updates lock"), 1);
	assert_eq!(
		info.lock().expect("test reporter lock").response.completion,
		Some(vec!["ab".to_string()])
	);
}

#[tokio::test]
async fn stream_streams_text_immediately_without_waiting_for_terminal_classification() {
	let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(4);
	let upstream = stream::unfold(receiver, |mut receiver| async move {
		receiver.recv().await.map(|item| (item, receiver))
	});
	let (logger, info, first_updates, updates) = test_logger();
	let body = translate_stream(
		axum_core::body::Body::from_stream(upstream),
		1024 * 1024,
		logger,
		"claude-sonnet-4-5",
		false,
		State::default(),
	);
	let mut output = body.into_data_stream();
	let sent_at = std::time::Instant::now();
	sender
		.send(Ok(Bytes::from(
			[
				stream_message_start(),
				block_start(0, json!({"type":"text","text":""})),
				block_delta(0, json!({"type":"text_delta","text":"held"})),
			]
			.concat(),
		)))
		.await
		.expect("send upstream prefix");
	// The delta must arrive without waiting for block_stop or the terminal message_delta/
	// message_stop frames, which we deliberately have not sent yet.
	let mut buffer = String::new();
	loop {
		let chunk = tokio::time::timeout(std::time::Duration::from_millis(500), output.next())
			.await
			.expect("output_text.delta should arrive without waiting for terminal classification")
			.expect("delta lifecycle frame")
			.expect("delta lifecycle body");
		buffer.push_str(core::str::from_utf8(&chunk).expect("UTF-8 delta lifecycle"));
		if buffer.contains("response.output_text.delta") {
			break;
		}
	}
	assert!(buffer.contains("response.created"));
	assert!(buffer.contains("response.in_progress"));
	assert!(buffer.contains("\"delta\":\"held\""));
	sender
		.send(Ok(Bytes::from(
			[block_stop(0), stream_terminal("end_turn", 1).concat()].concat(),
		)))
		.await
		.expect("send upstream terminal");
	drop(sender);
	let remaining = futures_util::StreamExt::collect::<Vec<_>>(output)
		.await
		.into_iter()
		.collect::<Result<Vec<_>, _>>()
		.expect("translated terminal body");
	let remaining = remaining
		.into_iter()
		.flat_map(|bytes| bytes.to_vec())
		.collect::<Vec<_>>();
	let remaining = String::from_utf8(remaining).expect("UTF-8 terminal body");
	assert!(remaining.contains("response.output_text.done"));
	assert!(remaining.contains("response.completed"));
	let first_token = info
		.lock()
		.expect("info lock")
		.response
		.first_token
		.expect("first token timestamp");
	assert!(first_token <= sent_at + std::time::Duration::from_millis(500));
	assert_eq!(*first_updates.lock().unwrap(), 1);
	assert_eq!(*updates.lock().unwrap(), 1);
}

#[tokio::test]
async fn stream_usage_cache_arithmetic_tiers_and_private_telemetry_are_exact() {
	for (tier, public) in [
		(None, None),
		(Some("standard"), Some("default")),
		(Some("priority"), Some("priority")),
	] {
		let mut usage = json!({
			"input_tokens":8,"output_tokens":0,
			"cache_read_input_tokens":2,"cache_creation_input_tokens":1
		});
		if let Some(tier) = tier {
			usage["service_tier"] = json!(tier);
		}
		let (logger, info, _, updates) = test_logger();
		let mut frames = vec![stream_message_start_with_usage(usage)];
		frames.extend(stream_terminal_with(
			"end_turn",
			serde_json::Value::Null,
			json!({
				"input_tokens":10,"output_tokens":5,
				"cache_read_input_tokens":4,"cache_creation_input_tokens":1
			}),
		));
		let events = translated_stream_with(frames, State::default(), logger, false).await;
		for snapshot in [
			&events[0]["response"],
			&events[1]["response"],
			&events.last().unwrap()["response"],
		] {
			assert_eq!(snapshot["service_tier"].as_str(), public, "tier {tier:?}");
		}
		let terminal = &events.last().unwrap()["response"];
		assert_eq!(terminal["usage"]["input_tokens"], 15);
		assert_eq!(
			terminal["usage"]["input_tokens_details"]["cached_tokens"],
			4
		);
		assert_eq!(
			terminal["usage"]["output_tokens_details"]["reasoning_tokens"],
			0
		);
		assert_eq!(terminal["usage"]["total_tokens"], 20);
		assert!(
			!terminal["usage"]
				.as_object()
				.unwrap()
				.contains_key("cache_creation_input_tokens")
		);
		assert!(
			!terminal
				.as_object()
				.unwrap()
				.contains_key("cache_creation_input_tokens")
		);
		assert_eq!(*updates.lock().unwrap(), 1);
		let info = info.lock().unwrap();
		assert_eq!(info.response.input_tokens, Some(10));
		assert_eq!(info.response.output_tokens, Some(5));
		assert_eq!(info.response.total_tokens, Some(15));
		assert_eq!(info.response.cached_input_tokens, Some(4));
		assert_eq!(info.response.cache_creation_input_tokens, Some(1));
		assert_eq!(
			info.response.provider_model.as_deref(),
			Some("claude-upstream")
		);
		assert_eq!(info.response.service_tier.as_deref(), tier);
	}

	let (logger, info, _, updates) = test_logger();
	let events = translated_stream_with(
		vec![stream_message_start_with_usage(
			json!({"input_tokens":1,"output_tokens":0,"service_tier":"secret-unknown"}),
		)],
		State::default(),
		logger,
		false,
	)
	.await;
	assert_eq!(
		events
			.iter()
			.map(|event| event["type"].as_str().unwrap())
			.collect::<Vec<_>>(),
		vec!["error"]
	);
	assert!(
		!serde_json::to_string(&events)
			.unwrap()
			.contains("secret-unknown")
	);
	assert_eq!(*updates.lock().unwrap(), 0);
	assert!(info.lock().unwrap().response.service_tier.is_none());
}

#[tokio::test]
async fn stream_thinking_tokens_use_terminal_value_and_initial_fallback() {
	for (terminal_details, expected) in [(json!({"thinking_tokens":5}), 5), (json!({}), 2)] {
		let (logger, info, _, updates) = test_logger();
		let mut frames = vec![stream_message_start_with_usage(json!({
			"input_tokens":2,
			"output_tokens":2,
			"output_tokens_details":{"thinking_tokens":2}
		}))];
		frames.extend(stream_terminal_with(
			"end_turn",
			serde_json::Value::Null,
			json!({
				"output_tokens":5,
				"output_tokens_details":terminal_details
			}),
		));
		let events = translated_stream_with(frames, State::default(), logger, false).await;
		assert_eq!(
			events.last().unwrap()["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
			expected
		);
		assert_eq!(
			info.lock().unwrap().response.reasoning_tokens,
			Some(expected)
		);
		assert_eq!(*updates.lock().unwrap(), 1);
	}
}

#[tokio::test]
async fn stream_rejects_invalid_thinking_token_details() {
	let cases = [
		(
			"regression",
			json!({"thinking_tokens":2}),
			json!({"thinking_tokens":1}),
			2,
		),
		(
			"greater_than_output",
			json!({}),
			json!({"thinking_tokens":6}),
			5,
		),
		(
			"unknown",
			json!({}),
			json!({"thinking_tokens":1,"future":1}),
			5,
		),
	];
	for (case, initial_details, terminal_details, output_tokens) in cases {
		let (logger, info, _, updates) = test_logger();
		let mut frames = vec![stream_message_start_with_usage(json!({
			"input_tokens":2,
			"output_tokens":2,
			"output_tokens_details":initial_details
		}))];
		frames.extend(stream_terminal_with(
			"end_turn",
			serde_json::Value::Null,
			json!({
				"output_tokens":output_tokens,
				"output_tokens_details":terminal_details
			}),
		));
		let events = translated_stream_with(frames, State::default(), logger, false).await;
		assert_one_safe_stream_error(&events, case);
		assert_eq!(*updates.lock().unwrap(), 0);
		assert!(info.lock().unwrap().response.reasoning_tokens.is_none());
	}
}

#[tokio::test]
async fn stream_usage_overflows_emit_one_contiguous_safe_error() {
	let cases = [
		json!({"input_tokens":usize::MAX,"output_tokens":0,"cache_read_input_tokens":1}),
		json!({"input_tokens":usize::MAX,"output_tokens":0,"cache_creation_input_tokens":1}),
		json!({"input_tokens":u64::from(u32::MAX)+1,"output_tokens":0}),
		json!({"input_tokens":0,"output_tokens":u64::from(u32::MAX)+1}),
		json!({"input_tokens":u32::MAX,"output_tokens":1}),
		json!({"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":u64::from(u32::MAX)+1}),
		json!({"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":u64::from(u32::MAX)+1}),
	];
	for usage in cases {
		let mut frames = vec![stream_message_start()];
		frames.extend(stream_terminal_with(
			"end_turn",
			serde_json::Value::Null,
			usage,
		));
		let events = translated_stream(frames, State::default()).await;
		assert_one_safe_stream_error(&events, "usage overflow");
		assert_eq!(
			events
				.iter()
				.map(|event| event["sequence_number"].as_u64().unwrap())
				.collect::<Vec<_>>(),
			(0..events.len() as u64).collect::<Vec<_>>()
		);
	}
}

#[tokio::test]
async fn stream_first_token_covers_text_and_tool_arguments_but_never_failures() {
	for kind in ["text", "tool"] {
		let (logger, info, first, updates) = test_logger();
		let state = if kind == "tool" {
			buffered_state()
		} else {
			State::default()
		};
		let mut frames = vec![stream_message_start()];
		match kind {
			"text" => frames.extend([
				block_start(0, json!({"type":"text","text":""})),
				block_delta(0, json!({"type":"text_delta","text":"x"})),
				block_stop(0),
			]),
			"tool" => frames.extend([
				block_start(
					0,
					json!({"type":"tool_use","id":"call","name":"weather","input":{}}),
				),
				block_delta(0, json!({"type":"input_json_delta","partial_json":"{}"})),
				block_stop(0),
			]),
			_ => unreachable!(),
		}
		frames.extend(stream_terminal(
			if kind == "tool" {
				"tool_use"
			} else {
				"end_turn"
			},
			1,
		));
		let _ = translated_stream_with(frames, state, logger, false).await;
		assert_eq!(*first.lock().unwrap(), 1, "{kind}");
		assert_eq!(*updates.lock().unwrap(), 1, "{kind}");
		assert!(
			info.lock().unwrap().response.completion.is_none(),
			"disabled completion"
		);
	}

	let (logger, info, first, updates) = test_logger();
	let _ = translated_stream_with(
		vec![
			stream_message_start(),
			block_start(0, json!({"type":"text","text":""})),
			block_delta(0, json!({"type":"text_delta","text":"x"})),
		],
		State::default(),
		logger,
		true,
	)
	.await;
	assert_eq!(*first.lock().unwrap(), 0);
	assert_eq!(*updates.lock().unwrap(), 0);
	assert!(info.lock().unwrap().response.completion.is_none());
}

#[tokio::test]
async fn stream_buffered_tool_items_set_first_token_when_they_become_visible() {
	for (kind, name, input) in [
		(
			"local_shell",
			"agentgateway__responses__local_shell_4",
			"{\"action\":{\"command\":[\"pwd\"],\"env\":{},\"timeout_ms\":1000,\"user\":null,\"working_directory\":\"/tmp\"}}",
		),
		(
			"shell",
			"agentgateway__responses__shell_5",
			"{\"action\":{\"commands\":[\"true\"],\"timeout_ms\":1000,\"max_output_length\":1024}}",
		),
		(
			"apply_patch",
			"agentgateway__responses__apply_patch_6",
			"{\"operation\":{\"type\":\"update_file\",\"path\":\"src/lib.rs\",\"diff\":\"@@\"}}",
		),
	] {
		let (logger, _, first, updates) = test_logger();
		let mut frames = vec![
			stream_message_start(),
			block_start(
				0,
				json!({"type":"tool_use","id":"call","name":name,"input":{}}),
			),
			block_delta(0, json!({"type":"input_json_delta","partial_json":input})),
			block_stop(0),
		];
		frames.extend(stream_terminal("tool_use", 1));
		let _ = translated_stream_with(frames, buffered_state(), logger, false).await;
		assert_eq!(*first.lock().unwrap(), 1, "{kind}");
		assert_eq!(*updates.lock().unwrap(), 1, "{kind}");
	}
}

fn text_stream_prefix() -> Vec<String> {
	vec![
		stream_message_start(),
		block_start(0, json!({"type":"text","text":""})),
		block_delta(0, json!({"type":"text_delta","text":"ok"})),
	]
}

fn assert_one_safe_stream_error(events: &[serde_json::Value], case: &str) {
	assert_eq!(
		events
			.iter()
			.filter(|event| event["type"] == "error")
			.count(),
		1,
		"{case}: {events:?}"
	);
	assert!(
		events.iter().all(|event| !matches!(
			event["type"].as_str(),
			Some("response.completed" | "response.incomplete" | "response.failed")
		)),
		"{case}: {events:?}"
	);
	let serialized = serde_json::to_string(events).expect("events serialize");
	for marker in [
		"signature-secret",
		"redacted-secret",
		"complete-tool-arguments",
	] {
		assert!(!serialized.contains(marker), "{case} leaked {marker}");
	}
}

#[tokio::test]
async fn stream_invalid_pre_terminal_sequences_emit_one_safe_error() {
	let start = stream_message_start();
	let cases = vec![
		(
			"malformed_json",
			vec!["event: message_start\ndata: {bad}\n\n".to_string()],
		),
		(
			"event_name_mismatch",
			vec![sse_event(
				"ping",
				json!({"type":"message_start","message":{}}),
			)],
		),
		(
			"wrong_index",
			vec![
				start.clone(),
				block_start(1, json!({"type":"text","text":""})),
			],
		),
		(
			"overlap",
			vec![
				start.clone(),
				block_start(0, json!({"type":"text","text":""})),
				block_start(1, json!({"type":"text","text":""})),
			],
		),
		(
			"delta_before_start",
			vec![
				start.clone(),
				block_delta(0, json!({"type":"text_delta","text":"bad"})),
			],
		),
		(
			"missing_signature",
			vec![
				start.clone(),
				block_start(0, json!({"type":"thinking","thinking":"","signature":""})),
				block_delta(0, json!({"type":"thinking_delta","thinking":"plan"})),
				block_stop(0),
			],
		),
		(
			"text_after_signature",
			vec![
				start.clone(),
				block_start(0, json!({"type":"thinking","thinking":"","signature":""})),
				block_delta(
					0,
					json!({"type":"signature_delta","signature":"signature-secret"}),
				),
				block_delta(0, json!({"type":"thinking_delta","thinking":"bad"})),
			],
		),
		(
			"message_delta_before_block_close",
			vec![
				start.clone(),
				block_start(0, json!({"type":"text","text":""})),
				stream_terminal("end_turn", 1)[0].clone(),
			],
		),
		(
			"message_stop_before_delta",
			vec![
				start.clone(),
				sse_event("message_stop", json!({"type":"message_stop"})),
			],
		),
		(
			"absent_stop_reason",
			vec![
				start.clone(),
				sse_event(
					"message_delta",
					json!({"type":"message_delta","delta":{"stop_reason":null,"stop_sequence":null},"usage":{"output_tokens":1}}),
				),
			],
		),
		(
			"inconsistent_stop_sequence",
			vec![
				start.clone(),
				sse_event(
					"message_delta",
					json!({"type":"message_delta","delta":{"stop_reason":"stop_sequence","stop_sequence":null},"usage":{"output_tokens":1}}),
				),
				sse_event("message_stop", json!({"type":"message_stop"})),
			],
		),
		(
			"upstream_done",
			vec![start.clone(), "data: [DONE]\n\n".to_string()],
		),
		("early_eof", vec![start.clone()]),
		(
			"in_band_error",
			vec![
				start,
				sse_event(
					"error",
					json!({"type":"error","error":{"type":"api_error","message":"complete-tool-arguments"}}),
				),
			],
		),
	];

	for (case, frames) in cases {
		let events = translated_stream(frames, State::default()).await;
		assert_one_safe_stream_error(&events, case);
	}
}

#[tokio::test]
async fn stream_decoder_and_body_failures_emit_one_safe_error() {
	let oversized = translated_body(
		axum_core::body::Body::from(format!("event: ping\ndata: {}\n\n", "x".repeat(128))),
		64,
	)
	.await;
	assert_one_safe_stream_error(&oversized, "oversized SSE frame");

	let truncated = translated_body(
		axum_core::body::Body::from("event: message_start\ndata: {"),
		1024,
	)
	.await;
	assert_one_safe_stream_error(&truncated, "truncated SSE frame");

	let body = axum_core::body::Body::from_stream(stream::iter(vec![
		Ok::<_, std::io::Error>(Bytes::from(stream_message_start())),
		Err(std::io::Error::other("upstream body secret")),
	]));
	let failed = translated_body(body, 1024).await;
	assert_one_safe_stream_error(&failed, "upstream body error");
}

#[tokio::test]
async fn stream_terminal_output_closes_without_polling_stalled_upstream() {
	let mut success = vec![stream_message_start()];
	success.extend(stream_terminal("end_turn", 1));
	let malformed = vec![
		stream_message_start(),
		"event: ping\ndata: {bad}\n\n".to_string(),
	];

	for (case, frames, terminal) in [
		("success", success, "response.completed"),
		("error", malformed, "error"),
	] {
		let chunks = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(frames.concat()))])
			.chain(stream::pending());
		let body = axum_core::body::Body::from_stream(chunks);
		let events = tokio::time::timeout(
			std::time::Duration::from_millis(250),
			translated_body(body, 1024 * 1024),
		)
		.await
		.unwrap_or_else(|_| panic!("{case} terminal did not close downstream"));
		assert_eq!(
			events.last().and_then(|event| event["type"].as_str()),
			Some(terminal)
		);
	}
}

#[tokio::test]
async fn stream_duplicate_delta_content_after_delta_and_open_terminal_are_invalid() {
	let mut duplicate = text_stream_prefix();
	duplicate.push(block_stop(0));
	duplicate.push(stream_terminal("end_turn", 1)[0].clone());
	duplicate.push(stream_terminal("end_turn", 1)[0].clone());

	let mut content_after = text_stream_prefix();
	content_after.push(block_stop(0));
	content_after.push(stream_terminal("end_turn", 1)[0].clone());
	content_after.push(block_start(1, json!({"type":"text","text":""})));

	let mut open_terminal = text_stream_prefix();
	open_terminal.push(stream_terminal("end_turn", 1)[0].clone());

	for (case, frames) in [
		("duplicate_message_delta", duplicate),
		("content_after_message_delta", content_after),
		("open_block_at_terminal", open_terminal),
	] {
		let events = translated_stream(frames, State::default()).await;
		assert_one_safe_stream_error(&events, case);
	}
}

#[tokio::test]
async fn stream_eof_after_partial_content_emits_completed_parts_then_one_error() {
	let open = text_stream_prefix();
	let mut closed = text_stream_prefix();
	closed.push(block_stop(0));
	let mut closed_then_open = closed.clone();
	closed_then_open.extend([
		block_start(1, json!({"type":"text","text":""})),
		block_delta(1, json!({"type":"text_delta","text":"invalid-open"})),
	]);
	// A block that closed cleanly before the EOF already streamed its done events; only a block
	// still open when the stream cuts out is missing them.
	for (case, frames, expected_output_text_done) in [
		("open", open, 0),
		("closed", closed, 1),
		("closed_then_open", closed_then_open, 1),
	] {
		let events = translated_stream(frames, State::default()).await;
		assert_one_safe_stream_error(&events, case);
		assert_eq!(
			events
				.iter()
				.filter(|event| event["type"] == "response.output_item.done")
				.count(),
			0,
			"{case}"
		);
		assert_eq!(
			events
				.iter()
				.filter(|event| event["type"] == "response.output_text.done")
				.count(),
			expected_output_text_done,
			"{case}"
		);
		assert_eq!(
			events
				.iter()
				.map(|event| event["sequence_number"].as_u64().unwrap())
				.collect::<Vec<_>>(),
			(0..events.len() as u64).collect::<Vec<_>>(),
			"{case}: sequence numbers must stay contiguous across a mid-stream EOF"
		);
	}
}

#[tokio::test]
async fn stream_malformed_final_tools_sequences_and_usage_regressions_are_invalid() {
	let state = buffered_state();
	for (case, partial) in [("malformed", "{\"x\":"), ("non_object", "[]")] {
		let frames = vec![
			stream_message_start(),
			block_start(
				0,
				json!({"type":"tool_use","id":"call","name":"weather","input":{}}),
			),
			block_delta(0, json!({"type":"input_json_delta","partial_json":partial})),
			block_stop(0),
		];
		let events = translated_stream(frames, state.clone()).await;
		assert_one_safe_stream_error(&events, case);
		assert_eq!(
			events
				.iter()
				.filter(|event| event["type"] == "response.output_item.done")
				.count(),
			0
		);
	}

	for (case, reason, sequence) in [
		("empty_stop_sequence", "stop_sequence", json!("")),
		("inverse_stop_sequence", "end_turn", json!("STOP")),
	] {
		let mut frames = vec![stream_message_start()];
		frames.extend(stream_terminal_with(
			reason,
			sequence,
			json!({"output_tokens":1}),
		));
		assert_one_safe_stream_error(&translated_stream(frames, State::default()).await, case);
	}

	for (field, initial, terminal) in [
		("output_tokens", 2, 1),
		("cache_read_input_tokens", 2, 1),
		("cache_creation_input_tokens", 2, 1),
	] {
		let mut start_usage = json!({"input_tokens":2,"output_tokens":0});
		start_usage[field] = json!(initial);
		let mut terminal_usage = json!({"output_tokens":2});
		terminal_usage[field] = json!(terminal);
		let mut frames = vec![stream_message_start_with_usage(start_usage)];
		frames.extend(stream_terminal_with(
			"end_turn",
			serde_json::Value::Null,
			terminal_usage,
		));
		assert_one_safe_stream_error(&translated_stream(frames, State::default()).await, field);
	}
}

#[tokio::test]
async fn stream_regressing_cumulative_usage_is_invalid() {
	let frames = vec![
		stream_message_start(),
		sse_event(
			"message_delta",
			json!({
				"type":"message_delta",
				"delta":{"stop_reason":"end_turn","stop_sequence":null},
				"usage":{"input_tokens":1,"output_tokens":0}
			}),
		),
		sse_event("message_stop", json!({"type":"message_stop"})),
	];
	let events = translated_stream(frames, State::default()).await;
	assert_one_safe_stream_error(&events, "regressing_cumulative_usage");
}

#[tokio::test]
async fn stream_duplicate_tool_ids_and_inconsistent_tool_stop_are_invalid() {
	let state = buffered_state();
	let duplicate = vec![
		stream_message_start(),
		block_start(
			0,
			json!({"type":"tool_use","id":"same","name":"weather","input":{}}),
		),
		block_stop(0),
		block_start(
			1,
			json!({"type":"tool_use","id":"same","name":"weather","input":{}}),
		),
	];
	let mut no_tool = vec![stream_message_start()];
	no_tool.extend(stream_terminal("tool_use", 1));
	let mut tool_end_turn = vec![
		stream_message_start(),
		block_start(
			0,
			json!({"type":"tool_use","id":"call","name":"weather","input":{}}),
		),
		block_stop(0),
	];
	tool_end_turn.extend(stream_terminal("end_turn", 1));

	for (case, frames) in [
		("duplicate_tool_id", duplicate),
		("tool_use_without_tool", no_tool),
		("tool_with_end_turn", tool_end_turn),
	] {
		let events = translated_stream(frames, state.clone()).await;
		assert_one_safe_stream_error(&events, case);
	}
}

#[tokio::test]
async fn stream_after_verified_terminal_is_silent_for_data_error_and_duplicate_terminal() {
	let mut frames = text_stream_prefix();
	frames.push(block_stop(0));
	frames.extend(stream_terminal("end_turn", 1));
	frames.extend([
		sse_event("ping", json!({"type":"ping"})),
		sse_event(
			"error",
			json!({"type":"error","error":{"message":"signature-secret"}}),
		),
		sse_event("message_stop", json!({"type":"message_stop"})),
	]);
	let (logger, _, _, updates) = test_logger();
	let events = translated_stream_with(frames, State::default(), logger, false).await;
	assert_eq!(
		events
			.iter()
			.filter(|event| event["type"] == "response.completed")
			.count(),
		1
	);
	assert_eq!(
		events
			.iter()
			.filter(|event| event["type"] == "error")
			.count(),
		0
	);
	assert_eq!(*updates.lock().unwrap(), 1);
}

#[tokio::test]
async fn stream_retained_byte_limit_emits_one_error() {
	let mut chunks = vec![
		stream_message_start(),
		block_start(0, json!({"type":"text","text":""})),
	];
	for _ in 0..61 {
		chunks.push(block_delta(
			0,
			json!({"type":"text_delta","text":"0123456789"}),
		));
	}
	let raw = chunks.concat();
	let body = axum_core::body::Body::from(raw);
	let output = translate_stream(
		body,
		600,
		StreamingUsageGuard::default(),
		"claude-sonnet-4-5",
		false,
		State::default(),
	)
	.collect()
	.await
	.expect("stream collection")
	.to_bytes();
	let text = String::from_utf8_lossy(&output);
	assert_eq!(text.matches("\"type\":\"error\"").count(), 1, "{text}");
}

#[tokio::test]
async fn stream_many_empty_completed_blocks_count_toward_retained_limit() {
	let mut frames = vec![stream_message_start()];
	for index in 0..12 {
		frames.push(block_start(index, json!({"type":"text","text":""})));
		frames.push(block_stop(index));
	}
	frames.extend(stream_terminal("end_turn", 1));
	let events = translated_body(axum_core::body::Body::from(frames.concat()), 512).await;
	assert_one_safe_stream_error(&events, "many empty completed blocks");
}

#[test]
fn stream_completed_output_byte_count_matches_serialized_vector() {
	let mut state = super::ResponsesStreamState::default();
	for (id, text) in [("one", ""), ("two", "content")] {
		state
			.retain_output(super::stream_message_item(
				id.to_string(),
				text.to_string(),
				super::responses::OutputStatus::Completed,
			))
			.expect("output byte count");
	}

	assert_eq!(
		state.retained_output_bytes + 2,
		serde_json::to_vec(&state.output).unwrap().len()
	);
}

#[test]
fn stream_adjacent_text_output_byte_count_matches_serialized_vector() {
	let mut state = super::ResponsesStreamState::default();
	state
		.retain_text_part(&super::StreamTextBlock {
			index: 0,
			output_index: 0,
			content_index: 0,
			item_id: "message".to_string(),
			text: "one".to_string(),
		})
		.expect("first text part");
	state
		.retain_text_part(&super::StreamTextBlock {
			index: 1,
			output_index: 0,
			content_index: 1,
			item_id: "message".to_string(),
			text: "two".to_string(),
		})
		.expect("adjacent text part");

	assert_eq!(
		state.retained_output_bytes + 2,
		serde_json::to_vec(&state.output).unwrap().len()
	);
}

#[tokio::test]
async fn stream_discarded_retained_limit_batch_keeps_emitted_sequences_contiguous() {
	let frame = sse_event(
		"message_start",
		json!({
			"type":"message_start",
			"message": {
				"id":"m".repeat(240), "type":"message", "role":"assistant",
				"content":[], "model":"claude-upstream", "stop_reason":null,
				"stop_sequence":null,
				"usage":{"input_tokens":2,"output_tokens":0}
			}
		}),
	);
	assert!(frame.len() < 512, "fixture must pass the decoder limit");
	let events = translated_body(axum_core::body::Body::from(frame), 512).await;
	assert_eq!(
		events
			.iter()
			.map(|event| event["sequence_number"].as_u64().unwrap())
			.collect::<Vec<_>>(),
		(0..events.len() as u64).collect::<Vec<_>>()
	);
}

#[test]
fn stream_sequence_overflow_falls_back_to_one_safe_error() {
	let mut state = super::ResponsesStreamState {
		sequence_number: u64::MAX,
		..Default::default()
	};
	assert!(state.sequence().is_err());
	let events = state.error_event();
	assert_eq!(events.len(), 1);
	assert!(matches!(
		events[0].1,
		types::responses::typed::ResponseStreamEvent::ResponseError(_)
	));
}

fn buffered_body(
	content: serde_json::Value,
	stop_reason: &str,
	stop_sequence: serde_json::Value,
	usage: serde_json::Value,
) -> serde_json::Value {
	json!({
		"id": "msg_upstream_123",
		"type": "message",
		"role": "assistant",
		"content": content,
		"model": "claude-upstream-model",
		"stop_reason": stop_reason,
		"stop_sequence": stop_sequence,
		"usage": usage
	})
}

fn buffered_usage() -> serde_json::Value {
	json!({
		"input_tokens": 10,
		"output_tokens": 5,
		"cache_creation_input_tokens": 1,
		"cache_read_input_tokens": 4,
		"service_tier": "standard"
	})
}

fn buffered_translate(
	body: serde_json::Value,
	state: &State,
) -> Box<dyn crate::types::ResponseType> {
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("response fixture should encode"));
	translate_response(&bytes, "claude-sonnet-4", state, 1024 * 1024)
		.expect("response should translate")
}

fn buffered_value(body: serde_json::Value, state: &State) -> serde_json::Value {
	let translated = buffered_translate(body, state);
	serde_json::from_slice(&translated.serialize().expect("response should serialize"))
		.expect("serialized Responses response")
}

#[test]
fn retain_response_item_rejects_incrementally_without_the_final_serialization_check() {
	// Isolates the per-item accounting in retain_response_item from translate_response's
	// separate final-serialization re-check (see
	// buffered_response_output_budget_accounts_for_final_serialization below). Without this,
	// a test that only calls translate_response can't tell incremental accounting apart from
	// "the final check alone happens to also catch this fixture" -- both mechanisms would
	// reject an already-oversized output, so only a direct call to the lower-level function
	// proves the per-item check is what's doing the rejecting, and that it stops as soon as
	// the running total crosses buffer_limit rather than accumulating every item first.
	let mut output = Vec::new();
	let mut retained_bytes = 0usize;
	let item = json!({"type": "function_call", "id": "fc_1", "arguments": "{}"});
	let item_bytes = serde_json::to_vec(&item)
		.expect("fixture should encode")
		.len();
	let buffer_limit = item_bytes * 3;

	for _ in 0..3 {
		retain_response_item(&mut output, item.clone(), buffer_limit, &mut retained_bytes)
			.expect("item should fit under the limit");
	}
	assert_eq!(output.len(), 3);
	assert_eq!(retained_bytes, item_bytes * 3);

	let err = retain_response_item(&mut output, item.clone(), buffer_limit, &mut retained_bytes);
	assert!(
		err.is_err(),
		"a 4th item pushing the running total past buffer_limit must be rejected immediately"
	);
	assert_eq!(
		output.len(),
		3,
		"the rejected item must not have been pushed onto output"
	);
	assert_eq!(
		retained_bytes,
		item_bytes * 3,
		"the rejected item must not have been charged against retained_bytes either"
	);
}

#[test]
fn buffered_response_output_is_bounded_by_buffer_limit() {
	// A malicious/compromised upstream can pair a long message id with a handful of minimal
	// tool_use blocks to try to amplify a small response body into a much larger translated
	// output: each translated `function_call` item embeds its own full copy of `message_id`
	// (see response_tool_output's `"id": format!("fc_{message_id}_{index}")`), so a message id
	// that appears once in the input is duplicated once per tool_use block in the output. The
	// non-streaming path must charge each generated item against buffer_limit the same way the
	// streaming path already does via ensure_retained_limit, instead of allocating unbounded
	// output. The chosen limit sits strictly between the input's own size and the amplified
	// output's size, so this specifically exercises the amplification guard rather than merely
	// rejecting an already-oversized input.
	let state = buffered_state();
	let content: Vec<serde_json::Value> = (0..5)
		.map(|index| {
			json!({
				"type": "tool_use",
				"id": format!("call_{index}"),
				"name": "weather",
				"input": {}
			})
		})
		.collect();
	let mut body = buffered_body(
		json!(content),
		"tool_use",
		serde_json::Value::Null,
		json!({"input_tokens": 1, "output_tokens": 1}),
	);
	body["id"] = json!("x".repeat(1000));
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("fixture should encode"));
	assert!(
		bytes.len() < 2048,
		"fixture input itself must stay well under the tested limit, or this only proves an \
		 oversized-input rejection rather than genuine output amplification (input was {} bytes)",
		bytes.len()
	);

	assert!(
		translate_response(&bytes, "claude-sonnet-4", &state, 2048).is_err(),
		"response output amplified far beyond the input size must be rejected"
	);
	assert!(
		translate_response(&bytes, "claude-sonnet-4", &state, 1024 * 1024).is_ok(),
		"the same response must still translate under a generous limit"
	);
}

#[test]
fn buffered_response_output_budget_accounts_for_final_serialization() {
	// The per-item size check runs on the compact serde_json::Value this module builds, but the
	// final response is round-tripped through the vendored async-openai LocalShellExecAction
	// struct, whose timeout_ms/user/working_directory fields have no skip_serializing_if -- an
	// absent optional field becomes an explicit `null` on the way back out, so the final
	// serialized bytes can be larger than what the per-item check measured.
	let state = buffered_state();
	let body = buffered_body(
		json!([{
			"type": "tool_use",
			"id": "call_4",
			"name": "agentgateway__responses__local_shell_4",
			"input": {"action": {"command": ["pwd"], "env": {}}}
		}]),
		"tool_use",
		serde_json::Value::Null,
		json!({"input_tokens": 1, "output_tokens": 1}),
	);
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("fixture should encode"));

	let translated = translate_response(&bytes, "claude-sonnet-4", &state, 1024 * 1024)
		.expect("should translate under a generous limit");
	let final_len = translated.serialize().expect("should serialize").len();

	assert!(
		translate_response(&bytes, "claude-sonnet-4", &state, final_len - 1).is_err(),
		"a limit just below the actual final serialized size must still be rejected"
	);
}

#[test]
fn buffered_response_ignores_copilot_extensions() {
	let mut body = buffered_body(
		json!([{"type":"text","text":"ok"}]),
		"end_turn",
		serde_json::Value::Null,
		buffered_usage(),
	);
	body["copilot_usage"] = json!({"token_details": []});
	body["stop_details"] = json!({"type": "end_turn"});
	body["usage"]["cache_creation"] = json!({"ephemeral_5m_input_tokens": 0});
	body["usage"]["inference_geo"] = json!("us");

	let value = buffered_value(body, &buffered_state());

	assert!(value.get("copilot_usage").is_none());
	assert!(value.get("stop_details").is_none());
	assert_eq!(value["output"][0]["content"][0]["text"], "ok");
}

#[test]
fn buffered_response_ignores_copilot_tool_caller() {
	let body = buffered_body(
		json!([{
			"type":"tool_use",
			"id":"call_0",
			"name":"weather",
			"input":{"city":"Seattle"},
			"caller":{"type":"direct"}
		}]),
		"tool_use",
		serde_json::Value::Null,
		buffered_usage(),
	);

	let value = buffered_value(body, &buffered_state());

	assert_eq!(value["output"][0]["type"], "function_call");
	assert!(value["output"][0].get("caller").is_none());
}

#[test]
fn buffered_response_rejects_non_direct_tool_callers() {
	for caller in [
		serde_json::Value::Null,
		json!({}),
		json!({"type":"server"}),
		json!({"type":"direct","future":true}),
		json!("direct"),
	] {
		let body = buffered_body(
			json!([{
				"type":"tool_use",
				"id":"call_0",
				"name":"weather",
				"input":{},
				"caller":caller
			}]),
			"tool_use",
			serde_json::Value::Null,
			buffered_usage(),
		);
		let bytes = Bytes::from(serde_json::to_vec(&body).expect("response fixture"));
		assert!(translate_response(&bytes, "claude-sonnet-4", &buffered_state(), 1024 * 1024).is_err());
	}
}

#[tokio::test]
async fn stream_rejects_non_direct_tool_caller() {
	let frames = vec![
		stream_message_start(),
		block_start(
			0,
			json!({
				"type":"tool_use",
				"id":"call_0",
				"name":"weather",
				"input":{},
				"caller":{"type":"server"}
			}),
		),
	];
	let events = translated_stream(frames, buffered_state()).await;
	assert_one_safe_stream_error(&events, "non_direct_tool_caller");
}

#[test]
fn buffered_response_preserves_order_wrappers_usage_and_stable_ids() {
	let state = buffered_state();
	let body = buffered_body(
		json!([
			{"type":"text","text":"one"},
			{"type":"text","text":"two"},
			{"type":"tool_use","id":"call_0","name":"weather","input":{"city":"Paris"}},
			{"type":"tool_use","id":"call_1","name":"agentgateway__responses__namespace_function_1","input":{"arguments":{"id":1}}},
			{"type":"tool_use","id":"call_2","name":"agentgateway__responses__namespace_custom_2","input":{"input":"select account"}},
			{"type":"tool_use","id":"call_3","name":"agentgateway__responses__custom_3","input":{"input":"print(1)"}},
			{"type":"tool_use","id":"call_4","name":"agentgateway__responses__local_shell_4","input":{"action":{"command":["pwd"],"env":{},"timeout_ms":1000,"user":null,"working_directory":"/tmp"}}},
			{"type":"tool_use","id":"call_5","name":"agentgateway__responses__shell_5","input":{"action":{"commands":["true"],"timeout_ms":1000,"max_output_length":1024}}},
			{"type":"tool_use","id":"call_6","name":"agentgateway__responses__apply_patch_6","input":{"operation":{"type":"update_file","path":"src/lib.rs","diff":"@@"}}},
			{"type":"text","text":"after"}
		]),
		"tool_use",
		serde_json::Value::Null,
		buffered_usage(),
	);
	let first = buffered_value(body.clone(), &state);
	let second = buffered_value(body, &state);

	assert_eq!(first["id"], "resp_msg_upstream_123");
	assert_eq!(first["status"], "completed");
	assert_eq!(first["model"], "claude-sonnet-4");
	assert_eq!(first["service_tier"], "default");
	assert_eq!(first["usage"]["input_tokens"], 15);
	assert_eq!(first["usage"]["input_tokens_details"]["cached_tokens"], 4);
	assert_eq!(first["usage"]["output_tokens"], 5);
	assert_eq!(
		first["usage"]["output_tokens_details"]["reasoning_tokens"],
		0
	);
	assert_eq!(first["usage"]["total_tokens"], 20);
	assert_eq!(first["output"], second["output"]);

	let output = first["output"].as_array().expect("output array");
	assert_eq!(output.len(), 9);
	assert_eq!(output[0]["type"], "message");
	assert_eq!(output[0]["id"], "msg_msg_upstream_123_0");
	assert_eq!(output[0]["phase"], "final_answer");
	assert_eq!(output[0]["content"][0]["text"], "one");
	assert_eq!(output[0]["content"][1]["text"], "two");
	assert_eq!(output[1]["type"], "function_call");
	assert_eq!(output[1]["id"], "fc_msg_upstream_123_2");
	assert_eq!(output[1]["call_id"], "call_0");
	assert_ne!(output[1]["id"], output[1]["call_id"]);
	assert_eq!(output[1]["arguments"], r#"{"city":"Paris"}"#);
	assert_eq!(output[1]["status"], "completed");
	assert_eq!(output[2]["namespace"], "crm");
	assert_eq!(output[2]["id"], "fc_msg_upstream_123_3");
	assert_eq!(output[2]["name"], "lookup");
	assert_eq!(output[2]["arguments"], r#"{"id":1}"#);
	assert_eq!(output[3]["type"], "custom_tool_call");
	assert_eq!(output[3]["id"], "ctc_msg_upstream_123_4");
	assert_eq!(output[3]["namespace"], "crm");
	assert_eq!(output[3]["input"], "select account");
	assert_eq!(output[4]["id"], "ctc_msg_upstream_123_5");
	assert_eq!(output[4]["name"], "python");
	assert_eq!(output[4]["input"], "print(1)");
	assert_eq!(output[5]["type"], "local_shell_call");
	assert_eq!(output[5]["id"], "lsc_msg_upstream_123_6");
	assert_eq!(output[5]["action"]["command"], json!(["pwd"]));
	assert_eq!(output[5]["action"]["working_directory"], "/tmp");
	assert_eq!(output[5]["status"], "completed");
	assert_eq!(output[6]["type"], "shell_call");
	assert_eq!(output[6]["id"], "shc_msg_upstream_123_7");
	assert_eq!(output[6]["action"]["commands"], json!(["true"]));
	assert_eq!(output[6]["action"]["max_output_length"], 1024);
	assert_eq!(output[6]["status"], "completed");
	assert_eq!(output[6]["environment"], json!({"type":"local"}));
	assert_eq!(output[7]["type"], "apply_patch_call");
	assert_eq!(output[7]["id"], "apc_msg_upstream_123_8");
	assert_eq!(output[7]["operation"]["type"], "update_file");
	assert_eq!(output[7]["operation"]["path"], "src/lib.rs");
	assert_eq!(output[7]["status"], "completed");
	assert_eq!(output[8]["type"], "message");
	assert_eq!(output[8]["id"], "msg_msg_upstream_123_9");
	assert_eq!(output[8]["content"][0]["text"], "after");
}

#[test]
fn buffered_response_retains_cache_write_only_in_telemetry() {
	let state = buffered_state();
	let translated = buffered_translate(
		buffered_body(
			json!([{"type":"text","text":"ok"}]),
			"end_turn",
			serde_json::Value::Null,
			buffered_usage(),
		),
		&state,
	);
	let telemetry = translated.to_llm_response(false);
	let value: serde_json::Value =
		serde_json::from_slice(&translated.serialize().expect("response should serialize"))
			.expect("serialized response");

	assert_eq!(value["usage"]["input_tokens"], 15);
	assert_eq!(value["usage"]["total_tokens"], 20);
	assert_eq!(telemetry.input_tokens, Some(10));
	assert_eq!(telemetry.output_tokens, Some(5));
	assert_eq!(telemetry.total_tokens, Some(15));
	assert_eq!(telemetry.cached_input_tokens, Some(4));
	assert_eq!(telemetry.cache_creation_input_tokens, Some(1));
	assert_eq!(telemetry.service_tier.as_deref(), Some("standard"));
	assert!(value["usage"].get("cache_creation_input_tokens").is_none());
	assert!(value["output"][0].get("encrypted_content").is_none());
}

#[test]
fn buffered_response_telemetry_uses_upstream_provider_model() {
	let state = buffered_state();
	let translated = buffered_translate(
		buffered_body(
			json!([{"type":"text","text":"ok"}]),
			"end_turn",
			serde_json::Value::Null,
			buffered_usage(),
		),
		&state,
	);
	let value: serde_json::Value =
		serde_json::from_slice(&translated.serialize().expect("response should serialize"))
			.expect("serialized response");
	let telemetry = translated.to_llm_response(false);

	assert_eq!(value["model"], "claude-sonnet-4");
	assert_eq!(
		telemetry.provider_model.as_deref(),
		Some("claude-upstream-model")
	);
}

#[rstest::rstest]
#[case::end_turn("end_turn", serde_json::Value::Null, "completed", None)]
#[case::stop_sequence("stop_sequence", json!("STOP"), "completed", None)]
#[case::refusal("refusal", serde_json::Value::Null, "completed", None)]
#[case::max_tokens(
	"max_tokens",
	serde_json::Value::Null,
	"incomplete",
	Some("max_output_tokens")
)]
#[case::context(
	"model_context_window_exceeded",
	serde_json::Value::Null,
	"incomplete",
	Some("max_output_tokens")
)]
fn buffered_terminal_mapping(
	#[case] stop_reason: &str,
	#[case] stop_sequence: serde_json::Value,
	#[case] status: &str,
	#[case] incomplete_reason: Option<&str>,
) {
	let value = buffered_value(
		buffered_body(json!([]), stop_reason, stop_sequence, buffered_usage()),
		&buffered_state(),
	);
	assert_eq!(value["status"], status);
	assert_eq!(
		value["incomplete_details"]["reason"].as_str(),
		incomplete_reason
	);
	if stop_reason == "refusal" {
		assert_eq!(value["output"][0]["content"][0]["type"], "refusal");
	} else {
		assert_eq!(value["output"], json!([]));
	}
}

#[test]
fn buffered_pause_turn_is_rejected() {
	let body = buffered_body(
		json!([]),
		"pause_turn",
		serde_json::Value::Null,
		buffered_usage(),
	);
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("response fixture"));
	let error = translate_response(&bytes, "claude-sonnet-4", &buffered_state(), 1024 * 1024)
		.err()
		.expect("pause turn must fail");
	assert_eq!(
		error.to_string(),
		"invalid response: invalid Anthropic Messages response"
	);
}

#[test]
fn buffered_empty_refusal_is_completed_with_an_empty_refusal_marker() {
	let value = buffered_value(
		buffered_body(
			json!([]),
			"refusal",
			serde_json::Value::Null,
			json!({"input_tokens":0,"output_tokens":0}),
		),
		&buffered_state(),
	);
	assert_eq!(value["status"], "completed");
	assert_eq!(value["output"].as_array().expect("output array").len(), 1);
	assert_eq!(value["output"][0]["content"][0]["type"], "refusal");
	assert_eq!(value["output"][0]["content"][0]["refusal"], "");
	assert_eq!(value["usage"]["total_tokens"], 0);
}

#[test]
fn buffered_refusal_maps_nonempty_text_to_refusal_content() {
	let value = buffered_value(
		buffered_body(
			json!([{"type":"text","text":"I cannot help with that."}]),
			"refusal",
			serde_json::Value::Null,
			buffered_usage(),
		),
		&buffered_state(),
	);
	assert_eq!(value["status"], "completed");
	assert_eq!(
		value["output"][0]["content"][0]["refusal"],
		"I cannot help with that."
	);
	assert_eq!(value["output"][0]["content"][0]["type"], "refusal");
}

#[rstest::rstest]
#[case::thinking(json!({"type":"thinking","thinking":"hidden","signature":"sig"}))]
#[case::redacted(json!({"type":"redacted_thinking","data":"opaque"}))]
fn buffered_rejects_unexpected_thinking_blocks(#[case] block: serde_json::Value) {
	let body = buffered_body(
		json!([block]),
		"end_turn",
		serde_json::Value::Null,
		buffered_usage(),
	);
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("response fixture"));
	let error = translate_response(&bytes, "claude-sonnet-4", &buffered_state(), 1024 * 1024)
		.err()
		.expect("thinking response must fail");
	assert_eq!(
		error.to_string(),
		"invalid response: invalid Anthropic Messages response"
	);
}

#[test]
fn buffered_thinking_tokens_map_to_public_and_private_usage() {
	let mut usage = buffered_usage();
	usage["output_tokens_details"] = json!({"thinking_tokens":3});
	let translated = buffered_translate(
		buffered_body(
			json!([{"type":"text","text":"ok"}]),
			"end_turn",
			serde_json::Value::Null,
			usage,
		),
		&buffered_state(),
	);
	let telemetry = translated.to_llm_response(false);
	let value: serde_json::Value =
		serde_json::from_slice(&translated.serialize().expect("response should serialize"))
			.expect("serialized response");
	assert_eq!(value["usage"]["output_tokens"], 5);
	assert_eq!(
		value["usage"]["output_tokens_details"]["reasoning_tokens"],
		3
	);
	assert_eq!(value["usage"]["total_tokens"], 20);
	assert_eq!(telemetry.reasoning_tokens, Some(3));
	assert_eq!(telemetry.output_tokens, Some(5));
	assert_eq!(telemetry.input_tokens, Some(10));
	assert_eq!(telemetry.total_tokens, Some(15));
}

#[rstest::rstest]
#[case::too_many(json!({"thinking_tokens":6}))]
#[case::unknown(json!({"thinking_tokens":3,"future":1}))]
fn buffered_rejects_invalid_thinking_token_details(#[case] details: serde_json::Value) {
	let mut usage = buffered_usage();
	usage["output_tokens_details"] = details;
	let body = buffered_body(
		json!([{"type":"text","text":"ok"}]),
		"end_turn",
		serde_json::Value::Null,
		usage,
	);
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("response fixture"));
	assert!(translate_response(&bytes, "claude-sonnet-4", &buffered_state(), 1024 * 1024).is_err());
}

#[test]
fn buffered_empty_thinking_token_details_are_absent() {
	let mut usage = buffered_usage();
	usage["output_tokens_details"] = json!({});
	let translated = buffered_translate(
		buffered_body(
			json!([{"type":"text","text":"ok"}]),
			"end_turn",
			serde_json::Value::Null,
			usage,
		),
		&buffered_state(),
	);
	let telemetry = translated.to_llm_response(false);
	let value: serde_json::Value =
		serde_json::from_slice(&translated.serialize().expect("response should serialize"))
			.expect("serialized response");
	assert_eq!(
		value["usage"]["output_tokens_details"]["reasoning_tokens"],
		0
	);
	assert_eq!(telemetry.reasoning_tokens, None);
}

#[test]
fn buffered_incomplete_marks_text_and_tool_items_incomplete() {
	let value = buffered_value(
		buffered_body(
			json!([
				{"type":"text","text":"partial"},
				{"type":"tool_use","id":"call_0","name":"weather","input":{}}
			]),
			"max_tokens",
			serde_json::Value::Null,
			buffered_usage(),
		),
		&buffered_state(),
	);
	assert_eq!(value["status"], "incomplete");
	for item in value["output"].as_array().expect("output") {
		assert_eq!(item["status"], "incomplete");
	}
}

#[test]
fn buffered_incomplete_uses_only_representable_wrapped_item_statuses() {
	let value = buffered_value(
		buffered_body(
			json!([
				{"type":"tool_use","id":"call_custom","name":"agentgateway__responses__custom_3","input":{"input":"work"}},
				{"type":"tool_use","id":"call_patch","name":"agentgateway__responses__apply_patch_6","input":{"operation":{"type":"delete_file","path":"old.txt"}}}
			]),
			"max_tokens",
			serde_json::Value::Null,
			buffered_usage(),
		),
		&buffered_state(),
	);
	assert_eq!(value["status"], "incomplete");
	assert!(value["output"][0].get("status").is_none());
	assert_eq!(value["output"][1]["status"], "completed");
}

#[rstest::rstest]
#[case::input_add(json!({"input_tokens": usize::MAX,"output_tokens":0,"cache_read_input_tokens":1}))]
#[case::cache_creation_add(json!({"input_tokens": usize::MAX,"output_tokens":0,"cache_creation_input_tokens":1}))]
#[case::input_width(json!({"input_tokens": u64::from(u32::MAX) + 1,"output_tokens":0}))]
#[case::cached_width(json!({"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":u64::from(u32::MAX) + 1}))]
#[case::output_width(json!({"input_tokens":0,"output_tokens":u64::from(u32::MAX) + 1}))]
#[case::total_add(json!({"input_tokens":u32::MAX,"output_tokens":1}))]
fn buffered_usage_overflow_is_rejected(#[case] usage: serde_json::Value) {
	let body = buffered_body(json!([]), "end_turn", serde_json::Value::Null, usage);
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("response fixture"));
	let error = translate_response(&bytes, "claude-sonnet-4", &buffered_state(), 1024 * 1024)
		.err()
		.expect("overflow must fail");
	assert_eq!(
		error.to_string(),
		"invalid response: invalid Anthropic Messages response"
	);
}

#[rstest::rstest]
#[case::standard("standard", "default")]
#[case::priority("priority", "priority")]
fn buffered_service_tier_mapping(#[case] upstream: &str, #[case] expected: &str) {
	let value = buffered_value(
		buffered_body(
			json!([]),
			"end_turn",
			serde_json::Value::Null,
			json!({"input_tokens":0,"output_tokens":0,"service_tier":upstream}),
		),
		&buffered_state(),
	);
	assert_eq!(value["service_tier"], expected);
}

#[test]
fn buffered_absent_service_tier_stays_absent() {
	let value = buffered_value(
		buffered_body(
			json!([]),
			"end_turn",
			serde_json::Value::Null,
			json!({"input_tokens":0,"output_tokens":0}),
		),
		&buffered_state(),
	);
	assert!(value.get("service_tier").is_none());
}

#[rstest::rstest]
#[case::bad_type("SENSITIVE_TYPE", json!({"type":"future","role":"assistant"}))]
#[case::bad_role("SENSITIVE_ROLE", json!({"type":"message","role":"user"}))]
#[case::unknown_envelope("SENSITIVE_ENVELOPE", json!({"type":"message","role":"assistant","future":"SENSITIVE_ENVELOPE"}))]
#[case::unknown_block("SENSITIVE_BLOCK", json!({"type":"message","role":"assistant","content":[{"type":"future","data":"SENSITIVE_BLOCK"}]}))]
#[case::unknown_text_field("SENSITIVE_TEXT", json!({"type":"message","role":"assistant","content":[{"type":"text","text":"ok","future":"SENSITIVE_TEXT"}]}))]
#[case::missing_signature("SENSITIVE_THINKING", json!({"type":"message","role":"assistant","content":[{"type":"thinking","thinking":"SENSITIVE_THINKING"}]}))]
#[case::empty_signature("SENSITIVE_SIGNATURE", json!({"type":"message","role":"assistant","content":[{"type":"thinking","thinking":"SENSITIVE_SIGNATURE","signature":""}]}))]
#[case::empty_redacted("SENSITIVE_REDACTED", json!({"type":"message","role":"assistant","content":[{"type":"redacted_thinking","data":""}]}))]
#[case::empty_tool_id("SENSITIVE_EMPTY_ID", json!({"type":"message","role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"","name":"weather","input":{"marker":"SENSITIVE_EMPTY_ID"}}]}))]
#[case::tool_json("SENSITIVE_TOOL_JSON", json!({"type":"message","role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"call_0","name":"weather","input":"SENSITIVE_TOOL_JSON"}]}))]
#[case::undeclared_tool("SENSITIVE_UNDECLARED", json!({"type":"message","role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"call_0","name":"SENSITIVE_UNDECLARED","input":{}}]}))]
#[case::malformed_wrapper("SENSITIVE_WRAPPER", json!({"type":"message","role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"call_0","name":"agentgateway__responses__shell_5","input":{"action":{"commands":"SENSITIVE_WRAPPER"}}}]}))]
#[case::duplicate_tool_id("SENSITIVE_DUPLICATE", json!({"type":"message","role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"SENSITIVE_DUPLICATE","name":"weather","input":{}},{"type":"tool_use","id":"SENSITIVE_DUPLICATE","name":"weather","input":{}}]}))]
#[case::tool_stop_without_tool("SENSITIVE_TOOL_STOP", json!({"type":"message","role":"assistant","stop_reason":"tool_use","content":[{"type":"text","text":"SENSITIVE_TOOL_STOP"}]}))]
#[case::tool_without_tool_stop("SENSITIVE_TOOL_REASON", json!({"type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"tool_use","id":"call_0","name":"weather","input":{"marker":"SENSITIVE_TOOL_REASON"}}]}))]
#[case::refusal_tool("SENSITIVE_REFUSAL", json!({"type":"message","role":"assistant","stop_reason":"refusal","content":[{"type":"tool_use","id":"call_0","name":"weather","input":{"marker":"SENSITIVE_REFUSAL"}}]}))]
#[case::sequence_without_reason("SENSITIVE_SEQUENCE", json!({"type":"message","role":"assistant","stop_sequence":"SENSITIVE_SEQUENCE"}))]
#[case::missing_sequence("SENSITIVE_MISSING_SEQUENCE", json!({"type":"message","role":"assistant","stop_reason":"stop_sequence","stop_sequence":null,"content":[{"type":"text","text":"SENSITIVE_MISSING_SEQUENCE"}]}))]
#[case::empty_sequence("SENSITIVE_EMPTY_SEQUENCE", json!({"type":"message","role":"assistant","stop_reason":"stop_sequence","stop_sequence":"","content":[{"type":"text","text":"SENSITIVE_EMPTY_SEQUENCE"}]}))]
#[case::wrong_sequence_type("SENSITIVE_SEQUENCE_TYPE", json!({"type":"message","role":"assistant","stop_reason":"stop_sequence","stop_sequence":{"marker":"SENSITIVE_SEQUENCE_TYPE"}}))]
#[case::unknown_stop("SENSITIVE_STOP", json!({"type":"message","role":"assistant","stop_reason":"SENSITIVE_STOP"}))]
#[case::unknown_tier("SENSITIVE_TIER", json!({"type":"message","role":"assistant","usage":{"input_tokens":0,"output_tokens":0,"service_tier":"SENSITIVE_TIER"}}))]
fn buffered_malformed_response_is_fixed_and_redacted(
	#[case] marker: &str,
	#[case] changes: serde_json::Value,
) {
	let mut body = buffered_body(
		json!([]),
		"end_turn",
		serde_json::Value::Null,
		json!({"input_tokens":0,"output_tokens":0}),
	);
	for (key, value) in changes.as_object().expect("object changes") {
		body[key] = value.clone();
	}
	let bytes = Bytes::from(serde_json::to_vec(&body).expect("response fixture"));
	let error = translate_response(&bytes, "claude-sonnet-4", &buffered_state(), 1024 * 1024)
		.err()
		.expect("malformed response must fail");
	let text = error.to_string();
	assert_eq!(
		text,
		"invalid response: invalid Anthropic Messages response"
	);
	assert!(!text.contains(marker));
}

#[test]
fn buffered_malformed_json_error_is_fixed_and_redacted() {
	let marker = "SENSITIVE_PARSE_MARKER";
	let bytes = Bytes::from(format!(r#"{{"marker":"{marker}""#));
	let error = translate_response(&bytes, "claude-sonnet-4", &buffered_state(), 1024 * 1024)
		.err()
		.expect("malformed JSON must fail");
	let text = error.to_string();
	assert_eq!(
		text,
		"invalid response: invalid Anthropic Messages response"
	);
	assert!(!text.contains(marker));
}

#[test]
fn tool_declarations_flatten_and_wrap_every_supported_kind() {
	let (body, state) = translate(&request(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations()
	})))
	.expect("supported tools should translate");
	let actual: serde_json::Value = serde_json::from_slice(&body).expect("Messages request");
	let tools = actual["tools"].as_array().expect("Messages tools");

	assert_eq!(tools.len(), 7);
	assert_eq!(tools[0]["name"], "weather");
	assert_eq!(
		tools[1]["name"],
		"agentgateway__responses__namespace_function_1"
	);
	assert_eq!(
		tools[2]["name"],
		"agentgateway__responses__namespace_custom_2"
	);
	assert_eq!(tools[3]["name"], "agentgateway__responses__custom_3");
	assert_eq!(tools[4]["name"], "agentgateway__responses__local_shell_4");
	assert_eq!(tools[5]["name"], "agentgateway__responses__shell_5");
	assert_eq!(tools[6]["name"], "agentgateway__responses__apply_patch_6");
	assert_eq!(
		tools[0]["input_schema"],
		tool_declarations()[0]["parameters"]
	);
	assert_eq!(tools[1]["input_schema"]["required"], json!(["arguments"]));
	assert_eq!(tools[2]["input_schema"]["required"], json!(["input"]));
	assert_eq!(
		tools[1]["description"],
		"Responses tool crm.lookup. CRM tools"
	);
	assert_eq!(
		tools[2]["description"],
		"Responses tool crm.query. CRM tools"
	);
	assert_eq!(tools[3]["description"], "Responses tool python. Run Python");
	assert_eq!(
		tools[4]["description"],
		"Execute one command on the client's local computer. Provide command as an argv array, env as string environment variables, and optional timeout_ms, user, and working_directory."
	);
	assert_eq!(
		tools[4]["input_schema"],
		json!({
			"type":"object",
			"properties":{"action":{
				"type":"object",
				"properties":{
					"command":{"type":"array","items":{"type":"string"}},
					"env":{"type":"object","additionalProperties":{"type":"string"}},
					"timeout_ms":{"type":["integer","null"],"minimum":0},
					"user":{"type":["string","null"]},
					"working_directory":{"type":["string","null"]}
				},
				"required":["command","env"],
				"additionalProperties":false
			}},
			"required":["action"],
			"additionalProperties":false
		})
	);
	assert_eq!(
		tools[5]["description"],
		"Execute one or more shell command strings in order on the client's local computer. Optional timeout_ms and max_output_length values limit total execution time and captured output."
	);
	assert_eq!(
		tools[5]["input_schema"],
		json!({
			"type":"object",
			"properties":{"action":{
				"type":"object",
				"properties":{
					"commands":{"type":"array","items":{"type":"string"}},
					"timeout_ms":{"type":["integer","null"],"minimum":0},
					"max_output_length":{"type":["integer","null"],"minimum":0}
				},
				"required":["commands"],
				"additionalProperties":false
			}},
			"required":["action"],
			"additionalProperties":false
		})
	);
	assert_eq!(
		tools[6]["description"],
		"Create, delete, or update one file in the client's local workspace. Create and update operations use path and diff. Delete operations use path."
	);
	assert_eq!(
		tools[6]["input_schema"],
		json!({
			"type":"object",
			"properties":{"operation":{"oneOf":[
				{
					"type":"object",
					"properties":{"type":{"const":"create_file"},"path":{"type":"string"},"diff":{"type":"string"}},
					"required":["type","path","diff"],
					"additionalProperties":false
				},
				{
					"type":"object",
					"properties":{"type":{"const":"delete_file"},"path":{"type":"string"}},
					"required":["type","path"],
					"additionalProperties":false
				},
				{
					"type":"object",
					"properties":{"type":{"const":"update_file"},"path":{"type":"string"},"diff":{"type":"string"}},
					"required":["type","path","diff"],
					"additionalProperties":false
				}
			]}},
			"required":["operation"],
			"additionalProperties":false
		})
	);
	assert_eq!(state.tools.len(), 7);
}

#[test]
fn namespace_function_schema_refs_are_rewritten_for_the_wrapper_nesting() {
	// Wrapping a namespace_function's caller-supplied `parameters` under properties.arguments
	// moves it one level deeper in the document. A root-relative $ref (into $defs, or a bare "#"
	// self-reference) must be rewritten to keep pointing at the same target instead of silently
	// resolving against the new wrapper root.
	let actual = translated(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{
			"type": "namespace",
			"name": "crm",
			"description": "CRM tools",
			"tools": [{
				"type": "function",
				"name": "lookup",
				"parameters": {
					"type": "object",
					"properties": {
						"customer": {"$ref": "#/$defs/Customer"},
						"self": {"$ref": "#"}
					},
					"$defs": {
						"Customer": {"type": "object", "properties": {"id": {"type": "integer"}}}
					}
				}
			}]
		}]
	}));
	let schema = &actual["tools"].as_array().expect("Messages tools")[0]["input_schema"];
	assert_eq!(
		schema["properties"]["arguments"]["properties"]["customer"]["$ref"],
		"#/properties/arguments/$defs/Customer"
	);
	assert_eq!(
		schema["properties"]["arguments"]["properties"]["self"]["$ref"],
		"#/properties/arguments"
	);
	// The referenced $defs entry itself must still be reachable at the rewritten location.
	assert_eq!(
		schema["properties"]["arguments"]["$defs"]["Customer"]["properties"]["id"]["type"],
		"integer"
	);
}

#[test]
fn namespace_function_schema_ref_rewriting_does_not_corrupt_non_schema_data() {
	// "const" (and similarly "enum"/"default") hold a literal data value, not a nested schema --
	// an object shaped like {"$ref": ...} under one of these keywords is just data that happens
	// to look like a ref, and must survive untouched. Meanwhile array-shaped applicators
	// ("items" tuple form, "oneOf") and map-shaped applicators ("$defs") must still have their
	// genuine nested-schema $refs rewritten.
	let actual = translated(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{
			"type": "namespace",
			"name": "crm",
			"description": "CRM tools",
			"tools": [{
				"type": "function",
				"name": "lookup",
				"parameters": {
					"type": "object",
					"properties": {
						"literal": {"const": {"$ref": "#/$defs/NotARef"}},
						"tuple": {"items": [{"$ref": "#/$defs/First"}, {"$ref": "#/$defs/Second"}]},
						"union": {"oneOf": [{"$ref": "#/$defs/First"}, {"type": "string"}]}
					},
					"$defs": {
						"First": {"type": "object"},
						"Second": {"type": "object"},
						"NotARef": {"type": "object"}
					}
				}
			}]
		}]
	}));
	let schema = &actual["tools"].as_array().expect("Messages tools")[0]["input_schema"];
	let args = &schema["properties"]["arguments"];
	// Untouched: this "$ref" is data under "const", not a schema reference.
	assert_eq!(
		args["properties"]["literal"]["const"]["$ref"],
		"#/$defs/NotARef"
	);
	// Rewritten: genuine schema references inside array- and map-shaped applicators.
	assert_eq!(
		args["properties"]["tuple"]["items"][0]["$ref"],
		"#/properties/arguments/$defs/First"
	);
	assert_eq!(
		args["properties"]["tuple"]["items"][1]["$ref"],
		"#/properties/arguments/$defs/Second"
	);
	assert_eq!(
		args["properties"]["union"]["oneOf"][0]["$ref"],
		"#/properties/arguments/$defs/First"
	);
}

#[test]
fn wrapped_tool_descriptions_preserve_client_punctuation() {
	let actual = translated(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{
			"type": "namespace",
			"name": "warehouse",
			"description": "Warehouse inventory tools.",
			"tools": [{
				"type": "custom",
				"name": "lookup",
				"description": "Use carefully!"
			}]
		}]
	}));
	assert_eq!(
		actual["tools"][0]["description"],
		"Responses tool warehouse.lookup. Warehouse inventory tools. Use carefully!"
	);
}

#[rstest::rstest]
#[case::auto(json!("auto"), json!({"type":"auto","disable_parallel_tool_use":true}))]
#[case::required(json!("required"), json!({"type":"any","disable_parallel_tool_use":true}))]
#[case::none(json!("none"), json!({"type":"none"}))]
#[case::function(json!({"type":"function","name":"weather"}), json!({"type":"tool","name":"weather","disable_parallel_tool_use":true}))]
#[case::namespace_function(json!({"type":"function","name":"lookup"}), json!({"type":"tool","name":"agentgateway__responses__namespace_function_1","disable_parallel_tool_use":true}))]
#[case::namespace_custom(json!({"type":"custom","name":"query"}), json!({"type":"tool","name":"agentgateway__responses__namespace_custom_2","disable_parallel_tool_use":true}))]
#[case::custom(json!({"type":"custom","name":"python"}), json!({"type":"tool","name":"agentgateway__responses__custom_3","disable_parallel_tool_use":true}))]
#[case::local_shell(json!({"type":"local_shell"}), json!({"type":"tool","name":"agentgateway__responses__local_shell_4","disable_parallel_tool_use":true}))]
#[case::shell(json!({"type":"shell"}), json!({"type":"tool","name":"agentgateway__responses__shell_5","disable_parallel_tool_use":true}))]
#[case::apply_patch(json!({"type":"apply_patch"}), json!({"type":"tool","name":"agentgateway__responses__apply_patch_6","disable_parallel_tool_use":true}))]
fn tool_choice_maps_supported_modes_and_declarations(
	#[case] choice: serde_json::Value,
	#[case] expected: serde_json::Value,
) {
	let actual = translated(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations(),
		"tool_choice": choice,
		"parallel_tool_calls": false
	}));

	assert_eq!(actual["tool_choice"], expected);
}

#[test]
fn tool_parallel_false_without_choice_emits_disabled_auto_choice() {
	let actual = translated(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations(),
		"parallel_tool_calls": false
	}));

	assert_eq!(
		actual["tool_choice"],
		json!({"type":"auto","disable_parallel_tool_use":true})
	);
}

#[test]
fn tool_choice_omitted_when_parallel_tool_calls_is_not_explicitly_false() {
	for (case, parallel_tool_calls) in [
		("absent", None),
		("null", Some(json!(null))),
		("true", Some(json!(true))),
	] {
		let mut request = json!({
			"input": "work",
			"model": "claude-sonnet-4-5",
			"tools": tool_declarations(),
		});
		if let Some(value) = parallel_tool_calls {
			request
				.as_object_mut()
				.expect("request object")
				.insert("parallel_tool_calls".to_string(), value);
		}
		let actual = translated(request);
		assert!(
			!actual
				.as_object()
				.expect("Messages request object")
				.contains_key("tool_choice"),
			"{case}: tool_choice must be omitted, not merely null, when parallel_tool_calls is not \
			 explicitly false"
		);
	}
}

#[test]
fn tool_history_preserves_calls_outputs_and_error_status() {
	let actual = translated(json!({
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations(),
		"input": [
			{"type":"function_call","call_id":"call_0","name":"weather","arguments":"{\"city\":\"Paris\"}","status":"completed"},
			{"type":"function_call_output","call_id":"call_0","output":"sunny","status":"completed"},
			{"type":"function_call","call_id":"call_1","namespace":"crm","name":"lookup","arguments":"{\"id\":1}"},
			{"type":"function_call_output","call_id":"call_1","output":"found"},
			{"type":"custom_tool_call","id":"item_2","call_id":"call_2","namespace":"crm","name":"query","input":"select account","status":"completed"},
			{"type":"custom_tool_call_output","call_id":"call_2","output":"account"},
			{"type":"custom_tool_call","id":"item_3","call_id":"call_3","name":"python","input":"print(1)"},
			{"type":"custom_tool_call_output","call_id":"call_3","output":"1"},
			{"type":"local_shell_call","id":"item_4","call_id":"call_4","status":"completed","action":{"command":["pwd"],"env":{},"timeout_ms":1000,"user":null,"working_directory":"/tmp"}},
			{"type":"local_shell_call_output","id":"call_4","output":"/tmp","status":"incomplete"},
			{"type":"shell_call","call_id":"call_5","status":"completed","action":{"commands":["true"],"timeout_ms":1000,"max_output_length":1024},"environment":{"type":"local"}},
			{"type":"shell_call_output","call_id":"call_5","output":[{"stdout":"","stderr":"","outcome":{"type":"exit","exit_code":0}}]},
			{"type":"apply_patch_call","call_id":"call_6","operation":{"type":"update_file","path":"src/lib.rs","diff":"@@"}},
			{"type":"apply_patch_call_output","call_id":"call_6","status":"failed","output":"conflict"}
		]
	}));
	let messages = actual["messages"].as_array().expect("Messages history");

	assert_eq!(messages.len(), 14);
	assert_eq!(
		messages[0]["content"][0],
		json!({"type":"tool_use","id":"call_0","name":"weather","input":{"city":"Paris"}})
	);
	assert_eq!(messages[1]["content"][0]["content"], "sunny");
	assert_eq!(
		messages[2]["content"][0]["name"],
		"agentgateway__responses__namespace_function_1"
	);
	assert_eq!(
		messages[2]["content"][0]["input"],
		json!({"arguments":{"id":1}})
	);
	assert_eq!(messages[3]["content"][0]["content"], "found");
	assert_eq!(
		messages[4]["content"][0]["input"],
		json!({"input":"select account"})
	);
	assert_eq!(messages[5]["content"][0]["content"], "account");
	assert_eq!(messages[7]["content"][0]["content"], "1");
	assert_eq!(
		messages[8]["content"][0]["input"]["action"]["working_directory"],
		"/tmp"
	);
	assert_eq!(messages[9]["content"][0]["content"], "/tmp");
	assert_eq!(messages[9]["content"][0]["is_error"], true);
	assert_eq!(
		messages[10]["content"][0]["input"]["action"]["max_output_length"],
		1024
	);
	assert_eq!(
		messages[11]["content"][0]["content"],
		r#"[{"stdout":"","stderr":"","outcome":{"type":"exit","exit_code":0}}]"#
	);
	assert_eq!(messages[13]["content"][0]["content"], "conflict");
	assert_eq!(messages[13]["content"][0]["is_error"], true);
}

#[test]
fn tool_call_with_incomplete_status_replays_as_history() {
	let actual = translated(json!({
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations(),
		"input": [
			{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{\"city\":\"Paris\"}","status":"incomplete"},
			{"type":"function_call_output","call_id":"call_1","output":"sunny"}
		]
	}));
	let messages = actual["messages"].as_array().expect("Messages history");
	assert_eq!(messages.len(), 2);
	assert_eq!(
		messages[0]["content"][0],
		json!({"type":"tool_use","id":"call_1","name":"weather","input":{"city":"Paris"}})
	);
}

#[rstest::rstest]
#[case::reserved(json!([{"type":"function","name":"agentgateway__responses__bad","parameters":{}}]))]
#[case::invalid_name(json!([{"type":"function","name":"bad.name","parameters":{}}]))]
#[case::long_name(json!([{"type":"function","name":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","parameters":{}}]))]
#[case::duplicate(json!([{"type":"function","name":"same","parameters":{}},{"type":"function","name":"same","parameters":{}}]))]
#[case::duplicate_namespace_child(json!([
	{"type":"namespace","name":"crm","description":"one","tools":[{"type":"function","name":"same","parameters":{}}]},
	{"type":"namespace","name":"crm","description":"two","tools":[{"type":"function","name":"same","parameters":{}}]}
]))]
#[case::duplicate_namespace(json!([
	{"type":"namespace","name":"crm","description":"one","tools":[{"type":"function","name":"one","parameters":{}}]},
	{"type":"namespace","name":"crm","description":"two","tools":[{"type":"function","name":"two","parameters":{}}]}
]))]
#[case::strict(json!([{"type":"function","name":"strict","parameters":{},"strict":true}]))]
#[case::deferred(json!([{"type":"function","name":"deferred","parameters":{},"defer_loading":true}]))]
#[case::grammar(json!([{"type":"custom","name":"grammar","format":{"type":"grammar","syntax":"lark","definition":"start: WORD"}}]))]
#[case::hosted(json!([{"type":"web_search"}]))]
fn tool_declaration_rejections(#[case] tools: serde_json::Value) {
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": tools
	})));
}

#[rstest::rstest]
#[case::missing_call_id(json!({"type":"function_call","name":"weather","arguments":"{}"}))]
#[case::missing_declaration(json!({"type":"function_call","call_id":"call_1","name":"missing","arguments":"{}"}))]
#[case::duplicate_call_id(json!([
	{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"},
	{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"}
]))]
#[case::output_before_call(json!({"type":"function_call_output","call_id":"call_1","output":"late"}))]
#[case::unknown_status(json!({"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}","status":"future"}))]
#[case::local_shell_missing_item_id(json!({"type":"local_shell_call","call_id":"call_1","status":"completed","action":{"command":["pwd"],"env":{}}}))]
#[case::mismatched_output(json!([
	{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"},
	{"type":"custom_tool_call_output","call_id":"call_1","output":"wrong"}
]))]
#[case::duplicate_output(json!([
	{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"},
	{"type":"function_call_output","call_id":"call_1","output":"one"},
	{"type":"function_call_output","call_id":"call_1","output":"two"}
]))]
#[case::intervening_user_text(json!([
	{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"},
	{"role":"user","content":"before the result"},
	{"type":"function_call_output","call_id":"call_1","output":"late"}
]))]
#[case::local_shell_output_missing_id(json!([
	{"type":"local_shell_call","id":"item_1","call_id":"call_1","status":"completed","action":{"command":["pwd"],"env":{}}},
	{"type":"local_shell_call_output","output":"pwd","status":"completed"}
]))]
fn tool_history_rejections(#[case] input: serde_json::Value) {
	let input = match input {
		serde_json::Value::Array(items) => items,
		item => vec![item],
	};
	assert!(translation_fails(json!({
		"input": input,
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations()
	})));
}

#[rstest::rstest]
#[case::local_shell_action(json!({"command":"pwd","env":{}}))]
#[case::shell_action(json!({"commands":"pwd"}))]
#[case::apply_patch_operation(json!({"type":"update_file","path":"src/lib.rs"}))]
fn tool_malformed_actions_are_rejected(#[case] malformed: serde_json::Value) {
	let (tools, item) = if malformed.get("command").is_some() {
		(
			json!([{"type":"local_shell"}]),
			json!({"type":"local_shell_call","id":"item_1","call_id":"call_1","status":"completed","action":malformed}),
		)
	} else if malformed.get("commands").is_some() {
		(
			json!([{"type":"shell","environment":{"type":"local"}}]),
			json!({"type":"shell_call","call_id":"call_1","status":"completed","action":malformed}),
		)
	} else {
		(
			json!([{"type":"apply_patch"}]),
			json!({"type":"apply_patch_call","call_id":"call_1","status":"completed","operation":malformed}),
		)
	};
	assert!(translation_fails(json!({
		"input": [item],
		"model": "claude-sonnet-4-5",
		"tools": tools
	})));
}

#[rstest::rstest]
#[case::unknown_tag(json!({"type":"remote"}))]
#[case::missing_container_id(json!({"type":"container_reference"}))]
#[case::local_unknown_field(json!({"type":"local","future":true}))]
#[case::local_skills_false(json!({"type":"local","skills":false}))]
#[case::local_skills_string(json!({"type":"local","skills":""}))]
#[case::local_skills_object(json!({"type":"local","skills":{}}))]
#[case::local_skills_null_entry(json!({"type":"local","skills":[null]}))]
fn tool_shell_declaration_rejects_malformed_environment(#[case] environment: serde_json::Value) {
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{"type":"shell","environment":environment}]
	})));
}

#[test]
fn tool_shell_declaration_rejects_hosted_and_effectful_local_environments() {
	assert!(translation_fails(json!({
		"input":"work",
		"model":"claude-sonnet-4-5",
		"tools":[{"type":"shell"}]
	})));
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{
			"type":"shell",
			"environment": {
				"type":"container_auto",
				"skills":[
					{"type":"skill_reference","skill_id":"skill_1","version":"1"},
					{"type":"inline","name":"lint","description":"Lint code","source":{"media_type":"application/zip","data":"eA=="}}
				]
			}
		}]
	})));
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{
			"type":"shell",
			"environment": {
				"type":"container_reference",
				"container_id":"container_1"
			}
		}]
	})));
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{
			"type":"shell",
			"environment": {
				"type":"local",
				"skills":[{"name":"lint","description":"Lint code","path":"/skills/lint"}]
			}
		}]
	})));
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [{
			"type":"shell",
			"environment":{"type":"container_auto","skills":[{"anything":true}]}
		}]
	})));
}

#[test]
fn tool_shell_history_rejects_container_auto_environment() {
	assert!(translation_fails(json!({
		"input": [{
			"type":"shell_call",
			"call_id":"call_1",
			"status":"completed",
			"action":{"commands":["true"]},
			"environment":{"type":"container_auto"}
		}],
		"model": "claude-sonnet-4-5",
		"tools": [{"type":"shell","environment":{"type":"local"}}]
	})));
}

#[rstest::rstest]
#[case(false.into())]
#[case(json!(""))]
#[case(json!({}))]
#[case(json!([null]))]
fn tool_shell_history_rejects_malformed_local_skills(#[case] skills: serde_json::Value) {
	assert!(translation_fails(json!({
		"input": [{
			"type":"shell_call",
			"call_id":"call_1",
			"status":"completed",
			"action":{"commands":["true"]},
			"environment":{"type":"local","skills":skills}
		}],
		"model": "claude-sonnet-4-5",
		"tools": [{"type":"shell","environment":{"type":"local"}}]
	})));
}

#[rstest::rstest]
#[case::function_failed(
	json!({"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"}),
	json!({"type":"function_call_output","call_id":"call_1","output":"bad","status":"failed"})
)]
#[case::custom_status(
	json!({"type":"custom_tool_call","id":"item_1","call_id":"call_1","name":"python","input":"x"}),
	json!({"type":"custom_tool_call_output","call_id":"call_1","output":"x","status":"completed"})
)]
#[case::shell_status(
	json!({"type":"shell_call","call_id":"call_1","status":"completed","action":{"commands":["true"]}}),
	json!({"type":"shell_call_output","call_id":"call_1","status":"completed","output":[]})
)]
fn tool_output_rejects_unsupported_status_fields(
	#[case] call: serde_json::Value,
	#[case] output: serde_json::Value,
) {
	assert!(translation_fails(json!({
		"input": [call, output],
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations()
	})));
}

#[test]
fn tool_choice_rejects_ambiguous_namespace_child_name() {
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": [
			{"type":"namespace","name":"one","description":"one","tools":[{"type":"function","name":"lookup","parameters":{}}]},
			{"type":"namespace","name":"two","description":"two","tools":[{"type":"function","name":"lookup","parameters":{}}]}
		],
		"tool_choice": {"type":"function","name":"lookup"}
	})));
}

#[test]
fn tool_rich_function_and_custom_outputs_preserve_supported_parts() {
	let actual = translated(json!({
		"model": "claude-sonnet-4-5",
		"tools": tool_declarations(),
		"input": [
			{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"},
			{"type":"function_call_output","call_id":"call_1","output":[
				{"type":"input_text","text":"forecast"},
				{"type":"input_image","image_url":"data:image/png;base64,aQ=="}
			]},
			{"type":"custom_tool_call","id":"item_2","call_id":"call_2","name":"python","input":"x"},
			{"type":"custom_tool_call_output","call_id":"call_2","output":[
				{"type":"input_file","file_data":"data:text/plain;base64,aGk="}
			]}
		]
	}));

	assert_eq!(
		actual["messages"][1]["content"][0]["content"][0]["text"],
		"forecast"
	);
	assert_eq!(
		actual["messages"][1]["content"][0]["content"][1]["type"],
		"image"
	);
	assert_eq!(
		actual["messages"][3]["content"][0]["content"][0]["source"]["data"],
		"hi"
	);
}

#[rstest::rstest]
#[case::required_without_tools(json!("required"))]
#[case::named_without_tools(json!({"type":"function","name":"weather"}))]
#[case::missing_declaration(json!({"type":"function","name":"missing"}))]
fn tool_choice_requires_matching_declaration(#[case] tool_choice: serde_json::Value) {
	let tools = (tool_choice["name"] == "missing").then(tool_declarations);
	assert!(translation_fails(json!({
		"input": "work",
		"model": "claude-sonnet-4-5",
		"tools": tools.unwrap_or_else(|| json!([])),
		"tool_choice": tool_choice
	})));
}

#[rstest::rstest]
#[case::timeout(json!({"type":"timeout"}))]
#[case::nonzero(json!({"type":"exit","exit_code":9}))]
fn tool_shell_failure_outcomes_are_errors(#[case] outcome: serde_json::Value) {
	let actual = translated(json!({
		"model": "claude-sonnet-4-5",
		"tools": [{"type":"shell","environment":{"type":"local"}}],
		"input": [
			{"type":"shell_call","call_id":"call_1","action":{"commands":["false"]}},
			{"type":"shell_call_output","call_id":"call_1","output":[{"stdout":"","stderr":"failure","outcome":outcome}]}
		]
	}));

	assert_eq!(actual["messages"][1]["content"][0]["is_error"], true);
}

#[test]
fn tool_shell_output_rejects_exit_code_outside_i32() {
	assert!(translation_fails(json!({
		"model": "claude-sonnet-4-5",
		"tools": [{"type":"shell","environment":{"type":"local"}}],
		"input": [
			{"type":"shell_call","call_id":"call_1","action":{"commands":["false"]}},
			{"type":"shell_call_output","call_id":"call_1","output":[{
				"stdout":"",
				"stderr":"failure",
				"outcome":{"type":"exit","exit_code":2147483648_i64}
			}]}
		]
	})));
}

#[test]
fn input_string_becomes_one_user_text_message() {
	let actual = translated(json!({
		"input": "hello",
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual,
		json!({
			"messages": [{
				"role": "user",
				"content": [{"type": "text", "text": "hello"}]
			}],
			"model": "claude-sonnet-4-5",
			"max_tokens": 4096
		})
	);
}

#[test]
fn instructions_and_leading_system_items_become_ordered_system_blocks() {
	let actual = translated(json!({
		"input": [
			{"role": "system", "content": "system item"},
			{
				"type": "message",
				"role": "developer",
				"content": [{"type": "input_text", "text": "developer item"}]
			},
			{"role": "user", "content": "question"}
		],
		"instructions": "instructions",
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["system"],
		json!([
			{"type": "text", "text": "instructions"},
			{"type": "text", "text": "system item"},
			{"type": "text", "text": "developer item"}
		])
	);
	assert_eq!(
		actual["messages"],
		json!([{
			"role": "user",
			"content": [{"type": "text", "text": "question"}]
		}])
	);
}

#[test]
fn raw_history_preserves_block_order_and_groups_only_adjacent_roles() {
	let actual = translated(json!({
		"input": [
			{
				"role": "user",
				"content": [
					{"type": "input_text", "text": "one"},
					{"type": "input_text", "text": "two"}
				]
			},
			{"role": "user", "content": "three"},
			{
				"type": "message",
				"role": "assistant",
				"phase": "commentary",
				"status": "completed",
				"content": [
					{"type": "output_text", "text": "four", "annotations": [], "logprobs": []},
					{"type": "output_text", "text": "five"}
				]
			},
			{"role": "assistant", "phase": "final_answer", "content": "six"},
			{"role": "user", "content": "seven"}
		],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"],
		json!([
			{
				"role": "user",
				"content": [
					{"type": "text", "text": "one"},
					{"type": "text", "text": "two"},
					{"type": "text", "text": "three"}
				]
			},
			{
				"role": "assistant",
				"content": [
					{"type": "text", "text": "four"},
					{"type": "text", "text": "five"},
					{"type": "text", "text": "six"}
				]
			},
			{
				"role": "user",
				"content": [{"type": "text", "text": "seven"}]
			}
		])
	);
}

#[test]
fn rejects_system_or_developer_items_after_conversation_starts() {
	for role in ["system", "developer"] {
		assert!(translation_fails(json!({
			"input": [
				{"role": "user", "content": "question"},
				{"role": role, "content": "late instruction"}
			],
			"model": "claude-sonnet-4-5"
		})));
	}
}

#[rstest::rstest]
#[case::type_absent("user", "{}", true)]
#[case::type_message("user", r#"{"type":"message"}"#, true)]
#[case::type_null("user", r#"{"type":null}"#, false)]
#[case::type_wrong_type("user", r#"{"type":false}"#, false)]
#[case::id_string("user", r#"{"id":"msg_1"}"#, true)]
#[case::id_empty_string("user", r#"{"id":""}"#, true)]
#[case::id_null("user", r#"{"id":null}"#, false)]
#[case::id_number("user", r#"{"id":1}"#, false)]
#[case::id_bool("user", r#"{"id":false}"#, false)]
#[case::id_array("user", r#"{"id":[]}"#, false)]
#[case::id_object("user", r#"{"id":{}}"#, false)]
#[case::unknown_effective_item_field("user", r#"{"future":"enabled"}"#, false)]
#[case::unknown_empty_item_field("user", r#"{"future":null}"#, false)]
#[case::non_assistant_phase("user", r#"{"phase":"commentary"}"#, false)]
#[case::assistant_phase("assistant", r#"{"phase":"commentary"}"#, true)]
fn raw_item_shape_policy(#[case] role: &str, #[case] extra: &str, #[case] accepted: bool) {
	let mut item = json!({"role": role, "content": "hello"});
	let extra: serde_json::Value = serde_json::from_str(extra).expect("valid test JSON");
	item
		.as_object_mut()
		.expect("message object")
		.extend(extra.as_object().expect("extra object").clone());

	assert_eq!(
		translate(&request(json!({
			"input": [item],
			"model": "claude-sonnet-4-5"
		})))
		.is_ok(),
		accepted
	);
}

#[rstest::rstest]
#[case::user_string_empty("user", json!(""), false)]
#[case::user_array_empty("user", json!([]), false)]
#[case::user_string_nonempty("user", json!("hello"), true)]
#[case::assistant_string_empty("assistant", json!(""), false)]
#[case::assistant_array_empty("assistant", json!([]), false)]
#[case::assistant_string_nonempty("assistant", json!("hello"), true)]
#[case::system_string_empty("system", json!(""), false)]
#[case::system_array_empty("system", json!([]), false)]
#[case::system_string_nonempty("system", json!("hello"), true)]
#[case::developer_string_empty("developer", json!(""), false)]
fn message_content_rejects_empty_and_accepts_nonempty(
	#[case] role: &str,
	#[case] content: serde_json::Value,
	#[case] accepted: bool,
) {
	assert_eq!(
		translate(&request(json!({
			"input": [{"role": role, "content": content}],
			"model": "claude-sonnet-4-5"
		})))
		.is_ok(),
		accepted
	);
}

#[rstest::rstest]
#[case::string_empty(json!(""), false)]
#[case::array_empty(json!([]), false)]
#[case::string_nonempty(json!("hello"), true)]
fn top_level_input_rejects_empty_and_accepts_nonempty(
	#[case] input: serde_json::Value,
	#[case] accepted: bool,
) {
	assert_eq!(
		translate(&request(json!({
			"input": input,
			"model": "claude-sonnet-4-5"
		})))
		.is_ok(),
		accepted
	);
}

#[test]
fn instructions_rejects_empty_string() {
	assert!(translation_fails(json!({
		"input": "hello",
		"model": "claude-sonnet-4-5",
		"instructions": ""
	})));
}

#[rstest::rstest]
#[case::user_unknown_effective_content_field("user", "input_text", r#"{"future":true}"#, false)]
#[case::user_unknown_empty_content_field("user", "input_text", r#"{"future":false}"#, false)]
#[case::system_unknown_effective_content_field(
	"system",
	"input_text",
	r#"{"future":"enabled"}"#,
	false
)]
#[case::developer_unknown_effective_content_field(
	"developer",
	"input_text",
	r#"{"future":"enabled"}"#,
	false
)]
#[case::assistant_unknown_effective_content_field(
	"assistant",
	"output_text",
	r#"{"future":1}"#,
	false
)]
#[case::assistant_unknown_empty_content_field(
	"assistant",
	"output_text",
	r#"{"future":null}"#,
	false
)]
fn content_part_shape_policy(
	#[case] role: &str,
	#[case] content_type: &str,
	#[case] extra: &str,
	#[case] accepted: bool,
) {
	let mut part = json!({"type": content_type, "text": "hello"});
	let extra: serde_json::Value = serde_json::from_str(extra).expect("valid test JSON");
	part
		.as_object_mut()
		.expect("content part object")
		.extend(extra.as_object().expect("extra object").clone());

	assert_eq!(
		translate(&request(json!({
			"input": [{"role": role, "content": [part]}],
			"model": "claude-sonnet-4-5"
		})))
		.is_ok(),
		accepted
	);
}

#[rstest::rstest]
#[case::metadata_absent("{}", true)]
#[case::annotations_empty_array(r#"{"annotations":[]}"#, true)]
#[case::annotations_false(r#"{"annotations":false}"#, false)]
#[case::annotations_null(r#"{"annotations":null}"#, false)]
#[case::annotations_empty_string(r#"{"annotations":""}"#, false)]
#[case::annotations_empty_object(r#"{"annotations":{}}"#, false)]
#[case::annotations_nonempty_array(r#"{"annotations":[{"type":"citation"}]}"#, false)]
#[case::logprobs_empty_array(r#"{"logprobs":[]}"#, true)]
#[case::logprobs_null(r#"{"logprobs":null}"#, true)]
#[case::logprobs_false(r#"{"logprobs":false}"#, false)]
#[case::logprobs_empty_string(r#"{"logprobs":""}"#, false)]
#[case::logprobs_empty_object(r#"{"logprobs":{}}"#, false)]
#[case::logprobs_nonempty_array(r#"{"logprobs":[{"token":"answer"}]}"#, false)]
fn assistant_content_metadata_shape_policy(#[case] extra: &str, #[case] accepted: bool) {
	let mut part = json!({"type": "output_text", "text": "answer"});
	let extra: serde_json::Value = serde_json::from_str(extra).expect("valid test JSON");
	part
		.as_object_mut()
		.expect("content part object")
		.extend(extra.as_object().expect("extra object").clone());

	assert_eq!(
		translate(&request(json!({
			"input": [{"role": "assistant", "content": [part]}],
			"model": "claude-sonnet-4-5"
		})))
		.is_ok(),
		accepted
	);
}

#[rstest::rstest]
#[case::phase_absent(None, None, "output_text", false, false, None, true)]
#[case::phase_commentary(Some("commentary"), None, "output_text", false, false, None, true)]
#[case::phase_final_answer(Some("final_answer"), None, "output_text", false, false, None, true)]
#[case::phase_other(Some("analysis"), None, "output_text", false, false, None, false)]
#[case::phase_null(None, None, "output_text", false, false, Some("phase"), false)]
#[case::status_absent(None, None, "output_text", false, false, None, true)]
#[case::status_completed(None, Some("completed"), "output_text", false, false, None, true)]
#[case::status_in_progress(None, Some("in_progress"), "output_text", false, false, None, false)]
#[case::status_incomplete(None, Some("incomplete"), "output_text", false, false, None, true)]
#[case::status_null(None, None, "output_text", false, false, Some("status"), false)]
#[case::refusal_content(None, None, "refusal", false, false, None, true)]
#[case::nonempty_annotations(None, None, "output_text", true, false, None, false)]
#[case::nonempty_logprobs(None, None, "output_text", false, true, None, false)]
fn assistant_history_policy(
	#[case] phase: Option<&str>,
	#[case] status: Option<&str>,
	#[case] content_type: &str,
	#[case] annotations: bool,
	#[case] logprobs: bool,
	#[case] null_field: Option<&str>,
	#[case] accepted: bool,
) {
	let content = if content_type == "refusal" {
		json!([{"type": "refusal", "refusal": "declined"}])
	} else {
		json!([{
			"type": "output_text",
			"text": "answer",
			"annotations": if annotations { vec![json!({"type": "citation"})] } else { Vec::new() },
			"logprobs": if logprobs { vec![json!({"token": "answer"})] } else { Vec::new() }
		}])
	};
	let mut message = json!({
		"role": "assistant",
		"content": content
	});
	if let Some(phase) = phase {
		message["phase"] = json!(phase);
	}
	if let Some(status) = status {
		message["status"] = json!(status);
	}
	if let Some(field) = null_field {
		message[field] = serde_json::Value::Null;
	}

	let result = translate(&request(json!({
		"input": [message],
		"model": "claude-sonnet-4-5"
	})));
	assert_eq!(result.is_ok(), accepted);
}

#[test]
fn assistant_refusal_history_replays_as_text_content() {
	// A prior turn's refusal (this crate's own output shape) must be replayable as history on
	// the next turn, since this route requires store:false and relies on the client resending
	// the API's own assistant content verbatim.
	let translated = translated(json!({
		"input": [{
			"role": "assistant",
			"content": [{"type": "refusal", "refusal": "I can't help with that."}]
		}],
		"model": "claude-sonnet-4-5"
	}));
	assert_eq!(
		translated["messages"][0]["content"][0]["text"],
		"I can't help with that."
	);
}

#[test]
fn assistant_incomplete_history_replays_as_continuation_prompt() {
	// A prior turn truncated by max_output_tokens (this crate's own output shape, status:
	// "incomplete") must be replayable as history so the client can continue the conversation --
	// store:false means the client, not this crate, carries that history forward.
	let translated = translated(json!({
		"input": [
			{
				"role": "assistant",
				"status": "incomplete",
				"content": [{"type": "output_text", "text": "The answer starts with", "annotations": [], "logprobs": []}]
			},
			{"role": "user", "content": "please continue"}
		],
		"model": "claude-sonnet-4-5"
	}));
	assert_eq!(
		translated["messages"][0]["content"][0]["text"],
		"The answer starts with"
	);
	assert_eq!(
		translated["messages"][1]["content"][0]["text"],
		"please continue"
	);
}

#[test]
fn assistant_refusal_history_rejects_unknown_field() {
	assert!(translation_fails(json!({
		"input": [{
			"role": "assistant",
			"content": [{"type": "refusal", "refusal": "declined", "future": "field"}]
		}],
		"model": "claude-sonnet-4-5"
	})));
}

#[test]
fn assistant_refusal_history_rejects_non_string_refusal() {
	assert!(translation_fails(json!({
		"input": [{
			"role": "assistant",
			"content": [{"type": "refusal", "refusal": null}]
		}],
		"model": "claude-sonnet-4-5"
	})));
}

#[test]
fn assistant_content_free_refusal_history_is_rejected_instead_of_replayed_as_empty_text() {
	assert!(translation_fails(json!({
		"input": [{
			"role": "assistant",
			"content": [{"type": "refusal", "refusal": ""}]
		}],
		"model": "claude-sonnet-4-5"
	})));
}

#[rstest::rstest]
#[case::include_absent("{}", true)]
#[case::include_null(r#"{"include":null}"#, false)]
#[case::include_empty(r#"{"include":[]}"#, false)]
#[case::include_encrypted_only(r#"{"include":["reasoning.encrypted_content"]}"#, false)]
#[case::include_other(r#"{"include":["message.output_text.logprobs"]}"#, false)]
#[case::include_mixed(
	r#"{"include":["reasoning.encrypted_content","message.output_text.logprobs"]}"#,
	false
)]
#[case::previous_response_id_empty(r#"{"previous_response_id":""}"#, true)]
#[case::previous_response_id_nonempty(r#"{"previous_response_id":"resp_1"}"#, false)]
#[case::conversation_empty(r#"{"conversation":{}}"#, false)]
#[case::conversation_nonempty(r#"{"conversation":{"id":"conv_1"}}"#, false)]
#[case::prompt_empty(r#"{"prompt":{}}"#, false)]
#[case::prompt_nonempty(r#"{"prompt":{"id":"pmpt_1"}}"#, false)]
#[case::background_absent("{}", true)]
#[case::background_false(r#"{"background":false}"#, true)]
#[case::background_null(r#"{"background":null}"#, false)]
#[case::background_empty_string(r#"{"background":""}"#, false)]
#[case::background_true(r#"{"background":true}"#, false)]
#[case::background_wrong_type(r#"{"background":0}"#, false)]
#[case::stream_options_absent("{}", true)]
#[case::stream_options_empty(r#"{"stream_options":{}}"#, false)]
#[case::stream_obfuscation_false(r#"{"stream_options":{"include_obfuscation":false}}"#, true)]
#[case::stream_obfuscation_null(r#"{"stream_options":{"include_obfuscation":null}}"#, false)]
#[case::stream_obfuscation_empty_string(r#"{"stream_options":{"include_obfuscation":""}}"#, false)]
#[case::stream_obfuscation_true(r#"{"stream_options":{"include_obfuscation":true}}"#, false)]
#[case::stream_obfuscation_wrong_type(r#"{"stream_options":{"include_obfuscation":0}}"#, false)]
#[case::stream_options_unknown_nonempty(r#"{"stream_options":{"future":"enabled"}}"#, false)]
#[case::stream_missing_options(r#"{"stream":true}"#, false)]
#[case::stream_empty_options(r#"{"stream":true,"stream_options":{}}"#, false)]
#[case::stream_explicit_obfuscation_false(
	r#"{"stream":true,"stream_options":{"include_obfuscation":false}}"#,
	true
)]
#[case::prompt_cache_key(r#"{"prompt_cache_key":"cache"}"#, false)]
#[case::prompt_cache_retention(r#"{"prompt_cache_retention":"24h"}"#, false)]
#[case::metadata(r#"{"metadata":{"key":"value"}}"#, false)]
#[case::user_absent("{}", true)]
#[case::user_null(r#"{"user":null}"#, true)]
#[case::user_empty_string(r#"{"user":""}"#, true)]
#[case::user_string(r#"{"user":"user_1"}"#, true)]
#[case::user_number(r#"{"user":1}"#, false)]
#[case::user_bool(r#"{"user":false}"#, false)]
#[case::user_array(r#"{"user":[]}"#, false)]
#[case::user_object(r#"{"user":{}}"#, false)]
#[case::safety_identifier(r#"{"safety_identifier":"safe_1"}"#, true)]
#[case::reasoning_summary_auto(r#"{"reasoning":{"summary":"auto"}}"#, false)]
#[case::reasoning_summary_concise(r#"{"reasoning":{"summary":"concise"}}"#, false)]
#[case::reasoning_summary_detailed(r#"{"reasoning":{"summary":"detailed"}}"#, false)]
#[case::reasoning_summary_invalid(r#"{"reasoning":{"summary":"verbose"}}"#, false)]
#[case::reasoning_false(r#"{"reasoning":false}"#, false)]
#[case::reasoning_unknown_empty(r#"{"reasoning":{"future":null}}"#, false)]
#[case::reasoning_unknown_nonempty(r#"{"reasoning":{"future":"enabled"}}"#, false)]
#[case::truncation_absent("{}", true)]
#[case::truncation_disabled(r#"{"truncation":"disabled"}"#, true)]
#[case::truncation_false(r#"{"truncation":false}"#, false)]
#[case::truncation_empty_string(r#"{"truncation":""}"#, false)]
#[case::truncation_null(r#"{"truncation":null}"#, false)]
#[case::truncation_auto(r#"{"truncation":"auto"}"#, false)]
#[case::truncation_other(r#"{"truncation":"future"}"#, false)]
#[case::max_tool_calls_null(r#"{"max_tool_calls":null}"#, true)]
#[case::max_tool_calls_nonempty(r#"{"max_tool_calls":1}"#, false)]
#[case::service_tier_null(r#"{"service_tier":null}"#, true)]
#[case::service_tier_nonempty(r#"{"service_tier":"auto"}"#, false)]
#[case::logprobs_absent("{}", true)]
#[case::logprobs_null(r#"{"logprobs":null}"#, true)]
#[case::logprobs_false(r#"{"logprobs":false}"#, true)]
#[case::logprobs_true(r#"{"logprobs":true}"#, false)]
#[case::logprobs_empty_string(r#"{"logprobs":""}"#, false)]
#[case::logprobs_empty_array(r#"{"logprobs":[]}"#, false)]
#[case::logprobs_empty_object(r#"{"logprobs":{}}"#, false)]
#[case::logprobs_number(r#"{"logprobs":1}"#, false)]
#[case::top_logprobs_null(r#"{"top_logprobs":null}"#, true)]
#[case::top_logprobs_nonempty(r#"{"top_logprobs":1}"#, false)]
#[case::text_verbosity_low(r#"{"text":{"verbosity":"low"}}"#, false)]
#[case::text_verbosity_medium(r#"{"text":{"verbosity":"medium"}}"#, false)]
#[case::text_verbosity_high(r#"{"text":{"verbosity":"high"}}"#, false)]
#[case::text_empty_string(r#"{"text":""}"#, false)]
#[case::text_unknown_empty(r#"{"text":{"future":null}}"#, false)]
#[case::text_unknown_nonempty(r#"{"text":{"future":"enabled"}}"#, false)]
#[case::text_format_default(r#"{"text":{}}"#, true)]
#[case::text_format_text(r#"{"text":{"format":{"type":"text"}}}"#, true)]
#[case::text_format_unknown_empty(r#"{"text":{"format":{"type":"text","strict":false}}}"#, false)]
#[case::text_format_unknown_nonempty(r#"{"text":{"format":{"type":"text","strict":true}}}"#, false)]
#[case::text_format_json_object(r#"{"text":{"format":{"type":"json_object"}}}"#, false)]
#[case::text_format_json_schema(
	r#"{"text":{"format":{"type":"json_schema","name":"answer","schema":{}}}}"#,
	false
)]
#[case::tools_absent("{}", true)]
#[case::tools_empty(r#"{"tools":[]}"#, true)]
#[case::tools_nonempty(r#"{"tools":[{"type":"function","name":"lookup"}]}"#, true)]
#[case::tool_choice_null(r#"{"tool_choice":null}"#, true)]
#[case::tool_choice_nonempty(r#"{"tool_choice":"auto"}"#, true)]
#[case::parallel_tool_calls_null(r#"{"parallel_tool_calls":null}"#, true)]
#[case::parallel_tool_calls_false(r#"{"parallel_tool_calls":false}"#, true)]
#[case::parallel_tool_calls_true(r#"{"parallel_tool_calls":true}"#, true)]
#[case::client_metadata(r#"{"client_metadata":{"session_id":"local-session"}}"#, false)]
#[case::unknown_null(r#"{"future_field":null}"#, false)]
#[case::unknown_false(r#"{"future_field":false}"#, false)]
#[case::unknown_empty_string(r#"{"future_field":""}"#, false)]
#[case::unknown_empty_array(r#"{"future_field":[]}"#, false)]
#[case::unknown_empty_object(r#"{"future_field":{}}"#, false)]
#[case::unknown_nonempty(r#"{"future_field":"enabled"}"#, false)]
fn top_level_policy(#[case] extra: &str, #[case] accepted: bool) {
	let mut value = json!({
		"input": "hello",
		"model": "claude-sonnet-4-5",
		"store": false
	});
	let extra: serde_json::Value = serde_json::from_str(extra).expect("valid test JSON");
	value
		.as_object_mut()
		.expect("request object")
		.extend(extra.as_object().expect("extra object").clone());

	let result = translate(&raw_request(value));
	assert_eq!(result.is_ok(), accepted);
}

#[rstest::rstest]
#[case::absent(None, false)]
#[case::false_value(Some(json!(false)), true)]
#[case::null(Some(serde_json::Value::Null), false)]
#[case::true_value(Some(json!(true)), false)]
#[case::wrong_type(Some(json!(0)), false)]
fn explicit_store_policy(#[case] store: Option<serde_json::Value>, #[case] accepted: bool) {
	let mut value = json!({"input":"hello","model":"claude-sonnet-4-5"});
	if let Some(store) = store {
		value["store"] = store;
	}
	assert_eq!(translate(&raw_request(value)).is_ok(), accepted);
}

#[rstest::rstest]
#[case::true_is_unsupported(
	r#"{"logprobs":true}"#,
	"unsupported conversion: unsupported Responses request option"
)]
#[case::empty_string_is_invalid(
	r#"{"logprobs":""}"#,
	"unsupported conversion: unsupported Responses logprobs"
)]
#[case::number_is_invalid(
	r#"{"logprobs":1}"#,
	"unsupported conversion: unsupported Responses logprobs"
)]
fn logprobs_errors_are_fixed(#[case] extra: &str, #[case] expected: &str) {
	let mut value = json!({
		"input": "hello",
		"model": "claude-sonnet-4-5"
	});
	let extra: serde_json::Value = serde_json::from_str(extra).expect("valid test JSON");
	value
		.as_object_mut()
		.expect("request object")
		.extend(extra.as_object().expect("extra object").clone());

	let error = translate(&request(value)).expect_err("request should be rejected");
	assert_eq!(error.to_string(), expected);
}

#[rstest::rstest]
#[case::reasoning_unknown(
	r#"{"reasoning":{"future":"enabled"}}"#,
	"unsupported conversion: Responses reasoning is unsupported"
)]
#[case::text_unknown(
	r#"{"text":{"future":"enabled"}}"#,
	"unsupported conversion: unsupported Responses text option"
)]
#[case::text_format_unknown(
	r#"{"text":{"format":{"type":"text","strict":true}}}"#,
	"unsupported conversion: unsupported Responses text format option"
)]
fn nested_allowlist_errors_are_fixed(#[case] extra: &str, #[case] expected: &str) {
	let mut value = json!({
		"input": "hello",
		"model": "claude-sonnet-4-5"
	});
	let extra: serde_json::Value = serde_json::from_str(extra).expect("valid test JSON");
	value
		.as_object_mut()
		.expect("request object")
		.extend(extra.as_object().expect("extra object").clone());

	let error = translate(&request(value)).expect_err("request should be rejected");
	assert_eq!(error.to_string(), expected);
}

#[test]
fn request_temperature_requires_anthropic_range() {
	for temperature in [0.0, 0.25, 1.0] {
		assert!(!translation_fails(json!({
			"input":"hello", "temperature":temperature
		})));
	}
	for temperature in [-0.1, 1.1] {
		assert!(translation_fails(json!({
			"input":"hello", "temperature":temperature
		})));
	}
}

#[rstest::rstest]
#[case::safety_wins(Some("safe"), Some("legacy"), Some("safe"))]
#[case::empty_safety_falls_back(Some(""), Some("legacy"), Some("legacy"))]
#[case::user_fallback(None, Some("legacy"), Some("legacy"))]
#[case::whitespace_is_preserved(Some("  "), Some("legacy"), Some("  "))]
#[case::empty_both(Some(""), Some(""), None)]
#[case::absent_both(None, None, None)]
fn request_metadata_maps_first_nonempty_identifier(
	#[case] safety_identifier: Option<&str>,
	#[case] user: Option<&str>,
	#[case] expected: Option<&str>,
) {
	let mut value = json!({"input":"hello"});
	if let Some(identifier) = safety_identifier {
		value["safety_identifier"] = json!(identifier);
	}
	if let Some(identifier) = user {
		value["user"] = json!(identifier);
	}
	let actual = translated(value);
	assert_eq!(
		actual
			.pointer("/metadata/user_id")
			.and_then(serde_json::Value::as_str),
		expected
	);
	assert_eq!(actual.get("metadata").is_some(), expected.is_some());
}

#[test]
fn copies_parameters_uses_neutral_defaults() {
	let (body, _) = translate(&request(json!({
		"input": "hello",
		"model": "claude-opus-4-1",
		"max_output_tokens": 123,
		"stream": true,
		"temperature": 0.25,
		"top_p": 0.75
	})))
	.expect("request should translate");
	let actual: serde_json::Value = serde_json::from_slice(&body).expect("valid Messages request");

	assert_eq!(actual["model"], "claude-opus-4-1");
	assert_eq!(actual["max_tokens"], 123);
	assert_eq!(actual["stream"], true);
	assert_eq!(actual["temperature"], 0.25);
	assert_eq!(actual["top_p"], 0.75);
	for field in [
		"stop_sequences",
		"top_k",
		"tools",
		"tool_choice",
		"metadata",
		"thinking",
		"output_config",
	] {
		assert!(actual.get(field).is_none());
	}
}

#[rstest::rstest]
#[case::effort_absent(None, Some(0.25), Some(0.75), true)]
#[case::effort_none(Some("none"), Some(0.25), Some(0.75), true)]
#[case::minimal_with_temperature(Some("minimal"), Some(0.25), None, false)]
#[case::low_with_temperature(Some("low"), Some(0.25), None, false)]
#[case::medium_with_temperature(Some("medium"), Some(0.25), None, false)]
#[case::high_with_temperature(Some("high"), Some(0.25), None, false)]
#[case::xhigh_with_temperature(Some("xhigh"), Some(0.25), None, false)]
#[case::active_with_top_p(Some("medium"), None, Some(0.75), false)]
#[case::active_without_sampling(Some("medium"), None, None, false)]
fn reasoning_sampling_policy(
	#[case] effort: Option<&str>,
	#[case] temperature: Option<f32>,
	#[case] top_p: Option<f32>,
	#[case] accepted: bool,
) {
	let mut value = json!({
		"input": "hello",
		"model": "claude-sonnet-4-5"
	});
	if let Some(effort) = effort {
		value["reasoning"] = json!({"effort": effort});
	}
	if let Some(temperature) = temperature {
		value["temperature"] = json!(temperature);
	}
	if let Some(top_p) = top_p {
		value["top_p"] = json!(top_p);
	}

	assert_eq!(translate(&request(value)).is_ok(), accepted);
}

#[test]
fn user_http_input_image_becomes_url_image_block_in_order() {
	let actual = translated(json!({
		"input": [{
			"role": "user",
			"content": [
				{"type": "input_text", "text": "before"},
				{"type": "input_image", "image_url": "http://example.com/image.png"},
				{"type": "input_text", "text": "after"}
			]
		}],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"][0]["content"],
		json!([
			{"type": "text", "text": "before"},
			{
				"type": "image",
				"source": {
					"type": "url",
					"url": "http://example.com/image.png"
				}
			},
			{"type": "text", "text": "after"}
		])
	);
}

#[test]
fn user_https_input_image_accepts_auto_detail() {
	let actual = translated(json!({
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"image_url": "https://example.com/image.webp",
				"detail": "auto"
			}]
		}],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"][0]["content"][0],
		json!({
			"type": "image",
			"source": {
				"type": "url",
				"url": "https://example.com/image.webp"
			}
		})
	);
}

#[rstest::rstest]
#[case("image/jpeg", "/9j/")]
#[case("image/png", "iVBORw==")]
#[case("image/gif", "R0lGODlh")]
#[case("image/webp", "UklGRg==")]
fn user_data_url_input_image_becomes_base64_image_block(
	#[case] media_type: &str,
	#[case] data: &str,
) {
	let actual = translated(json!({
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"image_url": format!("data:{media_type};base64,{data}")
			}]
		}],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"][0]["content"][0],
		json!({
			"type": "image",
			"source": {
				"type": "base64",
				"media_type": media_type,
				"data": data
			}
		})
	);
}

#[test]
fn input_image_rejects_file_id_even_when_null() {
	assert!(translation_fails(json!({
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"image_url": "https://example.com/image.png",
				"file_id": null
			}]
		}],
		"model": "claude-sonnet-4-5"
	})));
}

#[test]
fn pdf_data_url_input_file_becomes_base64_document() {
	let actual = translated(json!({
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_file",
				"file_data": "data:application/pdf;base64,JVBERi0xLjQK"
			}]
		}],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"][0]["content"][0],
		json!({
			"type": "document",
			"source": {
				"type": "base64",
				"media_type": "application/pdf",
				"data": "JVBERi0xLjQK"
			}
		})
	);
}

#[test]
fn text_data_url_input_file_decodes_to_titled_text_document() {
	let actual = translated(json!({
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_file",
				"file_data": "data:text/plain;base64,aGVsbG8gd29ybGQ=",
				"filename": "notes.txt",
				"detail": "low"
			}]
		}],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"][0]["content"][0],
		json!({
			"type": "document",
			"source": {
				"type": "text",
				"media_type": "text/plain",
				"data": "hello world"
			},
			"title": "notes.txt"
		})
	);
}

#[rstest::rstest]
#[case("http://example.com/report.pdf")]
#[case("https://example.com/report.pdf")]
fn url_input_file_becomes_url_document(#[case] file_url: &str) {
	let actual = translated(json!({
		"input": [{
			"role": "user",
			"content": [{"type": "input_file", "file_url": file_url}]
		}],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"][0]["content"][0],
		json!({
			"type": "document",
			"source": {"type": "url", "url": file_url}
		})
	);
}

#[test]
fn input_file_accepts_explicit_auto_detail() {
	let actual = translated(json!({
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_file",
				"file_url": "https://example.com/report.pdf",
				"detail": "auto"
			}]
		}],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(
		actual["messages"][0]["content"][0]["source"],
		json!({"type": "url", "url": "https://example.com/report.pdf"})
	);
}

#[test]
fn strict_json_schema_becomes_messages_output_format() {
	let schema = json!({
		"type": "object",
		"properties": {"answer": {"type": "string"}},
		"required": ["answer"],
		"additionalProperties": false
	});
	let actual = translated(json!({
		"input": "question",
		"model": "claude-sonnet-4-5",
		"text": {"format": {
			"type": "json_schema",
			"name": "answer",
			"schema": schema,
			"strict": true
		}}
	}));

	assert_eq!(
		actual["output_config"],
		json!({"format": {"type": "json_schema", "schema": schema}})
	);
}

#[test]
fn strict_json_schema_description_is_folded_into_the_schema() {
	let schema = json!({
		"type": "object",
		"properties": {"answer": {"type": "string"}},
		"required": ["answer"],
		"additionalProperties": false
	});
	let actual = translated(json!({
		"input": "question",
		"model": "claude-sonnet-4-5",
		"text": {"format": {
			"type": "json_schema",
			"name": "answer",
			"description": "answer the question concisely",
			"schema": schema,
			"strict": true
		}}
	}));

	assert_eq!(
		actual["output_config"]["format"]["schema"]["description"],
		json!("answer the question concisely")
	);
}

#[test]
fn strict_json_schema_description_does_not_overwrite_an_existing_schema_description() {
	let schema = json!({
		"type": "object",
		"properties": {"answer": {"type": "string"}},
		"description": "the schema's own description",
		"required": ["answer"],
		"additionalProperties": false
	});
	let actual = translated(json!({
		"input": "question",
		"model": "claude-sonnet-4-5",
		"text": {"format": {
			"type": "json_schema",
			"name": "answer",
			"description": "sibling description",
			"schema": schema,
			"strict": true
		}}
	}));

	assert_eq!(
		actual["output_config"]["format"]["schema"]["description"],
		json!("the schema's own description")
	);
}

#[test]
fn reasoning_history_is_rejected() {
	for item in [
		json!({"type": "reasoning"}),
		json!({"type": "reasoning", "summary": []}),
		json!({"type": "reasoning", "encrypted_content": "opaque"}),
	] {
		assert!(translation_fails(json!({
			"input": [item],
			"model": "claude-sonnet-4-5"
		})));
	}
}

#[test]
fn input_image_validation_rejects_invalid_sources_details_and_fields() {
	let cases = [
		("missing source", json!({"type": "input_image"})),
		(
			"relative URL",
			json!({"type": "input_image", "image_url": "/image.png"}),
		),
		(
			"non-HTTP URL",
			json!({"type": "input_image", "image_url": "ftp://example.com/image.png"}),
		),
		(
			"malformed HTTP URL",
			json!({"type": "input_image", "image_url": "https://"}),
		),
		(
			"malformed base64",
			json!({"type": "input_image", "image_url": "data:image/png;base64,%%%"}),
		),
		(
			"unsupported media type",
			json!({"type": "input_image", "image_url": "data:image/svg+xml;base64,PHN2Zz4="}),
		),
		(
			"file ID",
			json!({
				"type": "input_image",
				"image_url": "https://example.com/image.png",
				"file_id": "file-secret"
			}),
		),
		(
			"low detail",
			json!({
				"type": "input_image",
				"image_url": "https://example.com/image.png",
				"detail": "low"
			}),
		),
		(
			"high detail",
			json!({
				"type": "input_image",
				"image_url": "https://example.com/image.png",
				"detail": "high"
			}),
		),
		(
			"null detail",
			json!({
				"type": "input_image",
				"image_url": "https://example.com/image.png",
				"detail": null
			}),
		),
		(
			"unknown effective field",
			json!({
				"type": "input_image",
				"image_url": "https://example.com/image.png",
				"future": true
			}),
		),
	];

	for (name, part) in cases {
		assert!(
			translation_fails(json!({
				"input": [{"role": "user", "content": [part]}],
				"model": "claude-sonnet-4-5"
			})),
			"{name} should be rejected"
		);
	}
}

#[test]
fn input_file_validation_rejects_invalid_sources_data_details_and_fields() {
	let cases = [
		("missing source", json!({"type": "input_file"})),
		(
			"combined sources",
			json!({
				"type": "input_file",
				"file_data": "data:application/pdf;base64,JVBERg==",
				"file_url": "https://example.com/report.pdf"
			}),
		),
		(
			"null data",
			json!({"type": "input_file", "file_data": null}),
		),
		("null URL", json!({"type": "input_file", "file_url": null})),
		(
			"file ID",
			json!({"type": "input_file", "file_id": null, "file_url": "https://example.com/a"}),
		),
		(
			"relative URL",
			json!({"type": "input_file", "file_url": "report.pdf"}),
		),
		(
			"non-HTTP URL",
			json!({"type": "input_file", "file_url": "ftp://example.com/report.pdf"}),
		),
		(
			"malformed HTTP URL",
			json!({"type": "input_file", "file_url": "http://"}),
		),
		(
			"not a data URL",
			json!({"type": "input_file", "file_data": "JVBERg=="}),
		),
		(
			"malformed base64",
			json!({"type": "input_file", "file_data": "data:application/pdf;base64,%%%"}),
		),
		(
			"unsupported media type",
			json!({"type": "input_file", "file_data": "data:text/csv;base64,YQ=="}),
		),
		(
			"invalid UTF-8",
			json!({"type": "input_file", "file_data": "data:text/plain;base64,/w=="}),
		),
		(
			"high detail",
			json!({
				"type": "input_file",
				"file_url": "https://example.com/a",
				"detail": "high"
			}),
		),
		(
			"null detail",
			json!({
				"type": "input_file",
				"file_url": "https://example.com/a",
				"detail": null
			}),
		),
		(
			"invalid filename",
			json!({
				"type": "input_file",
				"file_url": "https://example.com/a",
				"filename": 1
			}),
		),
		(
			"unknown effective field",
			json!({
				"type": "input_file",
				"file_url": "https://example.com/a",
				"future": "secret-marker"
			}),
		),
	];

	for (name, part) in cases {
		assert!(
			translation_fails(json!({
				"input": [{"role": "user", "content": [part]}],
				"model": "claude-sonnet-4-5"
			})),
			"{name} should be rejected"
		);
	}
}

#[rstest::rstest]
#[case::input_image(json!({
	"type": "input_image",
	"image_url": "http://:443/path"
}))]
#[case::input_file(json!({
	"type": "input_file",
	"file_url": "http://:443/path"
}))]
fn media_url_rejects_authority_without_host(#[case] part: serde_json::Value) {
	assert!(translation_fails(json!({
		"input": [{"role": "user", "content": [part]}],
		"model": "claude-sonnet-4-5"
	})));
}

#[test]
fn media_is_rejected_outside_user_message_content() {
	for role in ["system", "developer", "assistant"] {
		for part in [
			json!({"type": "input_image", "image_url": "https://example.com/image.png"}),
			json!({"type": "input_file", "file_url": "https://example.com/report.pdf"}),
		] {
			assert!(
				translation_fails(json!({
					"input": [{"role": role, "content": [part]}],
					"model": "claude-sonnet-4-5"
				})),
				"{role} media should be rejected"
			);
		}
	}
}

#[test]
fn user_file_content_preserves_order_and_adjacent_user_grouping() {
	let actual = translated(json!({
		"input": [
			{
				"role": "user",
				"content": [
					{"type": "input_text", "text": "before"},
					{"type": "input_file", "file_url": "https://example.com/report.pdf"}
				]
			},
			{"role": "user", "content": "after"}
		],
		"model": "claude-sonnet-4-5"
	}));

	assert_eq!(actual["messages"].as_array().map(Vec::len), Some(1));
	assert_eq!(
		actual["messages"][0]["content"],
		json!([
			{"type": "text", "text": "before"},
			{
				"type": "document",
				"source": {"type": "url", "url": "https://example.com/report.pdf"}
			},
			{"type": "text", "text": "after"}
		])
	);
}

#[test]
fn output_format_rejects_non_strict_legacy_missing_and_unknown_shapes() {
	let formats = [
		json!({"type": "json_schema", "name": "answer", "schema": {}}),
		json!({"type": "json_schema", "name": "answer", "schema": {}, "strict": false}),
		json!({"type": "json_schema", "name": "answer", "schema": {}, "strict": null}),
		json!({"type": "json_schema", "name": "answer", "schema": {}, "strict": "true"}),
		json!({"type": "json_schema", "name": "answer", "strict": true}),
		json!({
			"type": "json_schema",
			"name": "answer",
			"schema": {},
			"strict": true,
			"future": "secret-marker"
		}),
		json!({
			"type": "json_schema",
			"name": "answer",
			"schema": {},
			"strict": true,
			"future": null
		}),
		json!({
			"type": "json_schema",
			"name": "answer",
			"schema": {},
			"strict": true,
			"future": false
		}),
		json!({"type": "json_object"}),
	];

	for format in formats {
		assert!(translation_fails(json!({
			"input": "question",
			"model": "claude-sonnet-4-5",
			"text": {"format": format}
		})));
	}
}

#[rstest::rstest]
#[case(None)]
#[case(Some("none"))]
fn absent_or_none_reasoning_emits_no_thinking_or_effort(#[case] effort: Option<&str>) {
	let mut value = json!({"input": "question", "model": "claude-sonnet-4-5"});
	if let Some(effort) = effort {
		value["reasoning"] = json!({"effort": effort});
	}
	let actual = translated(value);
	assert!(actual.get("thinking").is_none());
	assert!(actual.get("output_config").is_none());
}

#[rstest::rstest]
#[case::input_image("input_image")]
#[case::input_file("input_file")]
#[case::plain_text_format("plain_text_format")]
#[case::json_schema_format("json_schema_format")]
fn closed_nested_shapes_reject_inert_unknown_fields(#[case] shape: &str) {
	for inert in [json!(null), json!(false), json!(""), json!([]), json!({})] {
		let value = match shape {
			"input_image" => json!({
				"input": [{"role": "user", "content": [{
					"type": "input_image",
					"image_url": "https://example.com/image.png",
					"future": inert
				}]}],
				"model": "claude-sonnet-4-5"
			}),
			"input_file" => json!({
				"input": [{"role": "user", "content": [{
					"type": "input_file",
					"file_url": "https://example.com/report.pdf",
					"future": inert
				}]}],
				"model": "claude-sonnet-4-5"
			}),
			"plain_text_format" => json!({
				"input": "question",
				"model": "claude-sonnet-4-5",
				"text": {"format": {"type": "text", "future": inert}}
			}),
			"json_schema_format" => json!({
				"input": "question",
				"model": "claude-sonnet-4-5",
				"text": {"format": {
					"type": "json_schema",
					"name": "answer",
					"schema": {},
					"strict": true,
					"future": inert
				}}
			}),
			_ => unreachable!("known test shape"),
		};
		assert!(
			translation_fails(value),
			"{shape} accepted inert unknown field"
		);
	}
}
