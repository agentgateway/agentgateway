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

fn namespace_state() -> State {
	let (_, state) = translate(&request(json!({
		"model": "claude-sonnet-5-5",
		"input": "Continue the task.",
		"tools": [
			{
				"type": "function",
				"name": "exec_command",
				"description": "Run a command.",
				"parameters": {"type": "object", "required": ["cmd"]},
				"strict": false
			},
			{
				"type": "namespace",
				"name": "multi_agent_v1",
				"description": "Tools for managing agents.",
				"tools": [{
					"type": "function",
					"name": "spawn_agent",
					"description": "Start an agent.",
					"parameters": {"type": "object", "required": ["task"]},
					"strict": false
				}]
			}
		],
		"tool_choice": "auto"
	})))
	.expect("namespace state request should translate");
	state
}

#[test]
fn multi_agent_namespace_functions_are_flattened_before_messages_translation() {
	let definitions = json!([
		{
			"type": "function",
			"name": "close_agent",
			"description": "Close an agent.",
			"parameters": {"type": "object", "required": ["target"]},
			"strict": false
		},
		{
			"type": "function",
			"name": "resume_agent",
			"description": "Resume an agent.",
			"parameters": {"type": "object", "required": ["id"]},
			"strict": false
		},
		{
			"type": "function",
			"name": "send_input",
			"description": "Send input to an agent.",
			"parameters": {"type": "object", "required": ["target"]},
			"strict": false
		},
		{
			"type": "function",
			"name": "spawn_agent",
			"description": "Start an agent.",
			"parameters": {"type": "object", "required": ["task"]},
			"strict": false
		},
		{
			"type": "function",
			"name": "wait_agent",
			"description": "Wait for an agent.",
			"parameters": {"type": "object", "required": ["targets"]},
			"strict": false
		}
	]);
	let tools = [
		json!({
			"type": "function",
			"name": "exec_command",
			"description": "Run a command.",
			"parameters": {"type": "object", "required": ["cmd"]},
			"strict": false
		}),
		json!({
			"type": "function",
			"name": "write_stdin",
			"description": "Write to a running command.",
			"parameters": {"type": "object", "required": ["session_id", "chars"]},
			"strict": false
		}),
		json!({
			"type": "function",
			"name": "request_user_input",
			"description": "Ask the user a question.",
			"parameters": {"type": "object", "required": ["questions"]},
			"strict": false
		}),
		json!({
			"type": "function",
			"name": "view_image",
			"description": "View a local image.",
			"parameters": {"type": "object", "required": ["path"]},
			"strict": false
		}),
		json!({
			"type": "function",
			"name": "get_goal",
			"description": "Get the current goal.",
			"parameters": {"type": "object"},
			"strict": false
		}),
		json!({
			"type": "function",
			"name": "create_goal",
			"description": "Create a goal.",
			"parameters": {"type": "object"},
			"strict": false
		}),
		json!({
			"type": "function",
			"name": "update_goal",
			"description": "Update the current goal.",
			"parameters": {"type": "object"},
			"strict": false
		}),
		json!({
			"type": "namespace",
			"name": "multi_agent_v1",
			"description": "Tools for managing agents.",
			"tools": definitions
		}),
	];
	let (body, _) = translate(&request(json!({
		"model": "claude-sonnet-5-5",
		"input": "Continue the task.",
		"tools": tools,
		"tool_choice": "auto"
	})))
	.expect("captured multi_agent_v1 namespace should translate");
	let body: serde_json::Value = serde_json::from_slice(&body).expect("Messages request JSON");
	let tools = body["tools"].as_array().expect("translated tools");
	let names = tools
		.iter()
		.map(|tool| tool["name"].as_str().expect("tool name"))
		.collect::<Vec<_>>();
	assert_eq!(
		names,
		[
			"exec_command",
			"write_stdin",
			"request_user_input",
			"view_image",
			"get_goal",
			"create_goal",
			"update_goal",
			"multi_agent_v1__close_agent",
			"multi_agent_v1__resume_agent",
			"multi_agent_v1__send_input",
			"multi_agent_v1__spawn_agent",
			"multi_agent_v1__wait_agent",
		]
	);
	assert_eq!(tools[0]["description"], "Run a command.");
	assert_eq!(tools[0]["input_schema"]["required"], json!(["cmd"]));
	assert_eq!(tools[0]["strict"], false);
	assert_eq!(
		tools[9]["description"],
		"Tools for managing agents.\n\nSend input to an agent."
	);
	assert_eq!(tools[9]["input_schema"]["required"], json!(["target"]));
	assert_eq!(tools[9]["strict"], false);
	assert_eq!(body["tool_choice"]["type"], "auto");
}

#[test]
fn namespaced_function_history_keeps_call_and_result_ids() {
	let (body, _) = translate(&request(json!({
		"model": "claude-sonnet-5-5",
		"input": [
			{"role": "user", "content": "Continue the task."},
			{
				"type": "function_call",
				"id": "fc_1",
				"call_id": "call_1",
				"namespace": "multi_agent_v1",
				"name": "spawn_agent",
				"arguments": "{\"task\":\"review\"}"
			},
			{
				"type": "function_call_output",
				"call_id": "call_1",
				"namespace": "multi_agent_v1",
				"name": "spawn_agent",
				"output": "agent_1"
			},
			{"role": "user", "content": "Wait for the result."}
		],
		"tools": [{
			"type": "namespace",
			"name": "multi_agent_v1",
			"description": "Tools for managing agents.",
			"tools": [{
				"type": "function",
				"name": "spawn_agent",
				"parameters": {"type": "object", "required": ["task"]},
				"strict": false
			}]
		}],
		"tool_choice": "auto"
	})))
	.expect("namespace call and result history should translate");
	let body: serde_json::Value = serde_json::from_slice(&body).expect("Messages request JSON");
	assert_eq!(body["messages"][1]["role"], "assistant");
	assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
	assert_eq!(body["messages"][1]["content"][0]["id"], "call_1");
	assert_eq!(
		body["messages"][1]["content"][0]["name"],
		"multi_agent_v1__spawn_agent"
	);
	assert_eq!(
		body["messages"][1]["content"][0]["input"],
		json!({"task": "review"})
	);
	assert_eq!(body["messages"][2]["role"], "user");
	assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
	assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "call_1");
	assert_eq!(body["messages"][2]["content"][0]["content"], "agent_1");
	assert_eq!(
		body["messages"][2]["content"][1]["text"],
		"Wait for the result."
	);
}

#[test]
fn namespace_forced_choices_resolve_aliases_and_reject_ambiguity_and_collisions() {
	let (body, _) = translate(&request(json!({
		"model": "claude",
		"input": "Start an agent.",
		"tools": [{
			"type": "namespace",
			"name": "multi_agent_v1",
			"description": "Agent controls.",
			"tools": [{
				"type": "function",
				"name": "spawn_agent",
				"parameters": {"type": "object"},
				"strict": false
			}]
		}],
		"tool_choice": {"type": "function", "name": "spawn_agent"}
	})))
	.expect("a unique forced namespace choice should resolve");
	let body: serde_json::Value = serde_json::from_slice(&body).expect("Messages request JSON");
	assert_eq!(body["tool_choice"]["type"], "tool");
	assert_eq!(body["tool_choice"]["name"], "multi_agent_v1__spawn_agent");

	for (tools, expected) in [
		(
			json!([
				{"type":"namespace","name":"one","description":"","tools":[{"type":"function","name":"js"}]},
				{"type":"namespace","name":"two","description":"","tools":[{"type":"function","name":"js"}]}
			]),
			"ambiguous namespaced tool choice: js; use namespace__function to select a member",
		),
		(
			json!([
				{"type":"function","name":"multi_agent_v1__spawn_agent"},
				{"type":"namespace","name":"multi_agent_v1","description":"","tools":[{"type":"function","name":"spawn_agent"}]}
			]),
			"duplicate upstream tool name: multi_agent_v1__spawn_agent",
		),
	] {
		let result = translate(&request(json!({
			"model": "claude",
			"input": "Choose a tool.",
			"tools": tools,
			"tool_choice": {"type": "function", "name": if expected.starts_with("ambiguous") { "js" } else { "spawn_agent" }}
		})));
		assert!(
			matches!(result, Err(crate::AIError::UnsupportedConversion(message)) if message.as_str() == expected),
			"expected {expected}"
		);
	}
}

#[test]
fn buffered_namespaced_response_restores_function_identity() {
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"id": "msg_1",
			"type": "message",
			"role": "assistant",
			"model": "upstream-model",
			"content": [
				{
					"type": "tool_use",
					"id": "call_1",
					"name": "multi_agent_v1__spawn_agent",
					"input": {"task": "review"}
				},
				{
					"type": "tool_use",
					"id": "call_2",
					"name": "exec_command",
					"input": {"cmd": "pwd"}
				}
			],
			"stop_reason": "tool_use",
			"stop_sequence": null,
			"usage": {"input_tokens": 1, "output_tokens": 2}
		}))
		.expect("valid Messages response"),
	);
	let response = translate_response(&body, &namespace_state(), 1024 * 1024)
		.expect("namespaced buffered response should translate");
	let value = serde_json::to_value(response).expect("serializable Responses result");
	assert_eq!(value["output"][0]["type"], "function_call");
	assert_eq!(value["output"][0]["namespace"], "multi_agent_v1");
	assert_eq!(value["output"][0]["name"], "spawn_agent");
	assert_eq!(value["output"][0]["call_id"], "call_1");
	assert_eq!(value["output"][0]["arguments"], "{\"task\":\"review\"}");
	assert!(value["output"][1].get("namespace").is_none());
	assert_eq!(value["output"][1]["name"], "exec_command");
	assert_eq!(value["output"][1]["call_id"], "call_2");
}

#[test]
fn buffered_nested_namespace_member_alias_is_restored_once() {
	let (_, state) = translate(&request(json!({
		"model": "request-model",
		"input": "work",
		"tools": [
			{
				"type": "namespace",
				"name": "alpha",
				"description": "",
				"tools": [{"type": "function", "name": "run", "parameters": {"type": "object"}}]
			},
			{
				"type": "namespace",
				"name": "beta",
				"description": "",
				"tools": [{"type": "function", "name": "alpha__run", "parameters": {"type": "object"}}]
			}
		],
		"tool_choice": "auto"
	})))
	.expect("overlapping namespace aliases should translate");
	let body = Bytes::from(
		serde_json::to_vec(&json!({
			"id": "msg_nested",
			"type": "message",
			"role": "assistant",
			"model": "upstream-model",
			"content": [{
				"type": "tool_use",
				"id": "call_nested_1",
				"name": "beta__alpha__run",
				"input": {"value": "preserve me"}
			}],
			"stop_reason": "tool_use",
			"stop_sequence": null,
			"usage": {"input_tokens": 1, "output_tokens": 2}
		}))
		.expect("valid Messages response"),
	);
	let response = translate_response(&body, &state, 1024 * 1024)
		.expect("nested namespaced buffered response should translate");
	let value = serde_json::to_value(response).expect("serializable Responses result");
	assert_eq!(value["output"].as_array().expect("output array").len(), 1);
	assert_eq!(value["output"][0]["type"], "function_call");
	assert_eq!(value["output"][0]["namespace"], "beta");
	assert_eq!(value["output"][0]["name"], "alpha__run");
	assert_eq!(value["output"][0]["call_id"], "call_nested_1");
	assert_eq!(
		value["output"][0]["arguments"],
		"{\"value\":\"preserve me\"}"
	);
}

fn namespaced_tool_stream_frames() -> Vec<String> {
	vec![
		message_start(1),
		sse_event(
			"content_block_start",
			json!({
				"type": "content_block_start",
				"index": 0,
				"content_block": {
					"type": "tool_use",
					"id": "call_1",
					"name": "multi_agent_v1__spawn_agent",
					"input": {}
				}
			}),
		),
		sse_event(
			"content_block_delta",
			json!({
				"type": "content_block_delta",
				"index": 0,
				"delta": {"type": "input_json_delta", "partial_json": "{\"task\":\"review\"}"}
			}),
		),
		sse_event(
			"content_block_stop",
			json!({"type": "content_block_stop", "index": 0}),
		),
	]
}

#[tokio::test]
async fn streamed_member_matching_another_alias_keeps_its_namespace() {
	let (_, state) = translate(&request(json!({
		"input": "work",
		"tools": [
			{"type": "namespace", "name": "alpha", "description": "", "tools": [
				{"type": "function", "name": "run", "parameters": {"type": "object"}}
			]},
			{"type": "namespace", "name": "beta", "description": "", "tools": [
				{"type": "function", "name": "alpha__run", "parameters": {"type": "object"}}
			]}
		]
	})))
	.expect("distinct aliases should translate");
	let mut frames: Vec<_> = namespaced_tool_stream_frames()
		.into_iter()
		.map(|frame| frame.replace("multi_agent_v1__spawn_agent", "beta__alpha__run"))
		.collect();
	frames.push(sse_event(
		"message_delta",
		json!({
			"type": "message_delta",
			"delta": {"stop_reason": "tool_use", "stop_sequence": null},
			"usage": {"output_tokens": 2}
		}),
	));
	frames.push(sse_event("message_stop", json!({"type": "message_stop"})));
	let events = collect_stream(frames, 1024 * 1024, state).await;
	for event in &events {
		if event["item"]["type"] == "function_call" {
			assert_eq!(event["item"]["namespace"], "beta");
			assert_eq!(event["item"]["name"], "alpha__run");
		}
	}
	let arguments = events
		.iter()
		.find(|event| event["type"] == "response.function_call_arguments.done")
		.expect("tool arguments should complete");
	assert_eq!(arguments["name"], "alpha__run");
	let completed = events
		.iter()
		.find(|event| event["type"] == "response.completed")
		.expect("tool response should complete");
	assert_eq!(completed["response"]["output"][0]["namespace"], "beta");
	assert_eq!(completed["response"]["output"][0]["name"], "alpha__run");
}

#[tokio::test]
async fn streamed_namespaced_tool_events_restore_identity_and_history() {
	let mut frames = namespaced_tool_stream_frames();
	frames.extend(terminal("tool_use", 1));
	let events = collect_stream_with_log_content(
		frames,
		1024 * 1024,
		namespace_state(),
		LogContentFields {
			completion: true,
			tool_calls: true,
		},
	)
	.await;
	let added = events
		.iter()
		.find(|event| event["type"] == "response.output_item.added")
		.expect("function item added");
	let item_id = added["item"]["id"].as_str().expect("function item id");
	assert_eq!(added["item"]["namespace"], "multi_agent_v1");
	assert_eq!(added["item"]["name"], "spawn_agent");
	assert_eq!(added["item"]["call_id"], "call_1");

	let delta = events
		.iter()
		.find(|event| event["type"] == "response.function_call_arguments.delta")
		.expect("function arguments delta");
	assert_eq!(delta["item_id"], item_id);
	assert_eq!(delta["delta"], "{\"task\":\"review\"}");

	let done = events
		.iter()
		.find(|event| event["type"] == "response.function_call_arguments.done")
		.expect("function arguments done");
	assert_eq!(done["name"], "spawn_agent");
	assert_eq!(done["item_id"], item_id);
	assert_eq!(done["arguments"], "{\"task\":\"review\"}");

	let item_done = events
		.iter()
		.find(|event| event["type"] == "response.output_item.done")
		.expect("function item done");
	assert_eq!(item_done["item"]["namespace"], "multi_agent_v1");
	assert_eq!(item_done["item"]["name"], "spawn_agent");
	assert_eq!(item_done["item"]["call_id"], "call_1");
	assert_eq!(item_done["item"]["arguments"], "{\"task\":\"review\"}");

	let completed = events
		.iter()
		.find(|event| event["type"] == "response.completed")
		.expect("completed response");
	assert_eq!(
		completed["response"]["output"][0]["namespace"],
		"multi_agent_v1"
	);
	assert_eq!(completed["response"]["output"][0]["name"], "spawn_agent");
	assert_eq!(completed["response"]["output"][0]["call_id"], "call_1");
	let response_bytes =
		serde_json::to_vec(&completed["response"]["output"]).expect("serialized output");
	let limited = collect_stream(
		namespaced_tool_stream_frames()
			.into_iter()
			.chain(terminal("tool_use", 1))
			.collect(),
		response_bytes.len() - 1,
		namespace_state(),
	)
	.await;
	assert_one_safe_error(&limited);
}

#[test]
fn function_output_requires_call_id() {
	for call_id in [None, Some(json!(null)), Some(json!(""))] {
		let mut output = json!({"type": "function_call_output", "output": "sunny"});
		if let Some(call_id) = call_id {
			output["call_id"] = call_id;
		}
		let result = translate(&request(json!({
			"model": "claude",
			"input": [
				{"type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{}"},
				output
			]
		})));
		assert!(matches!(
			result,
			Err(crate::AIError::UnsupportedConversion(_))
		));
	}
}

#[rstest::rstest]
#[case::absent(json!({}))]
#[case::empty(json!({"context_management": []}))]
fn absent_or_empty_context_management_is_a_no_op(#[case] extra: serde_json::Value) {
	let mut value = json!({"model": "claude", "input": "hello"});
	value
		.as_object_mut()
		.expect("request object")
		.extend(extra.as_object().expect("extra object").clone());

	let (body, _) = translate(&request(value)).expect("request without compaction should translate");
	let body: serde_json::Value = serde_json::from_slice(&body).expect("Messages JSON");
	assert!(body.get("context_management").is_none());
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
fn cache_breakpoints_map_text_media_and_system_blocks() {
	let input = serde_json::from_str(include_str!(
		"../../../tests/requests/responses/cache_control.json"
	))
	.expect("cache fixture");
	let (body, _) = translate(&request(input)).expect("cache request");
	let body: serde_json::Value = serde_json::from_slice(&body).expect("Messages body");
	for index in 0..3 {
		assert_eq!(
			body["messages"][0]["content"][index]["cache_control"],
			json!({"type": "ephemeral"})
		);
	}
	for index in 0..2 {
		assert_eq!(
			body["system"][index]["cache_control"],
			json!({"type": "ephemeral"})
		);
	}
}

#[test]
fn cache_breakpoints_map_assistant_and_tool_result_content() {
	let (body, _) = translate(&request(json!({
		"input": [
			{"role": "assistant", "content": [{"type": "input_text", "text": "plan", "prompt_cache_breakpoint": {"mode": "explicit"}}]},
			{"type": "function_call", "call_id": "call_1", "name": "weather", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "call_1", "output": [
				{"type": "input_text", "text": "result", "prompt_cache_breakpoint": {"mode": "explicit"}},
				{"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8=", "prompt_cache_breakpoint": {"mode": "explicit"}},
				{"type": "input_file", "file_data": "aGVsbG8=", "filename": "notes.txt", "prompt_cache_breakpoint": {"mode": "explicit"}}
			]}
		]
	}))).expect("cached history");
	let body: serde_json::Value = serde_json::from_slice(&body).expect("Messages body");
	assert_eq!(
		body["messages"][0]["content"][0]["cache_control"],
		json!({"type": "ephemeral"})
	);
	for index in 0..3 {
		assert_eq!(
			body["messages"][1]["content"][0]["content"][index]["cache_control"],
			json!({"type": "ephemeral"})
		);
	}
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

async fn collect_stream_with_log_content(
	frames: Vec<String>,
	buffer_limit: usize,
	state: State,
	log_content: LogContentFields,
) -> Vec<serde_json::Value> {
	let chunks = frames
		.into_iter()
		.map(|frame| Ok::<_, Infallible>(Bytes::from(frame)));
	let body = agent_http::Body::from_stream(stream::iter(chunks));
	let output = translate_stream(
		body,
		buffer_limit,
		StreamingUsageGuard::default(),
		"request-model",
		log_content,
		state,
	)
	.collect()
	.await
	.expect("translated stream should collect")
	.to_bytes();
	String::from_utf8(output.to_vec())
		.expect("translated SSE should be UTF-8")
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

async fn collect_stream_with_guard(
	frames: Vec<String>,
	buffer_limit: usize,
	state: State,
	guard: StreamingUsageGuard,
) -> Vec<serde_json::Value> {
	let chunks = frames
		.into_iter()
		.map(|frame| Ok::<_, Infallible>(Bytes::from(frame)));
	let body = agent_http::Body::from_stream(stream::iter(chunks));
	collect_stream_body(body, buffer_limit, state, guard).await
}

async fn collect_stream_body(
	body: agent_http::Body,
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
#[case::context_management_compaction(json!({"context_management": [{"type": "compaction", "compact_threshold": 1000}]}))]
#[case::context_management_compaction_default(json!({"context_management": [{"type": "compaction"}]}))]
#[case::priority_tier(json!({"service_tier": "priority"}))]
#[case::fast_tier(json!({"service_tier": "fast"}))]
#[case::ultrafast_tier(json!({"service_tier": "ultrafast"}))]
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

#[test]
fn empty_namespace_is_an_empty_container() {
	let (body, state) = translate(&request(json!({
		"model": "claude",
		"input": "hello",
		"tools": [{
			"type": "namespace",
			"name": "tools",
			"description": "Grouped tools",
			"tools": []
		}]
	})))
	.expect("empty namespace has no tools to translate");
	let body: serde_json::Value = serde_json::from_slice(&body).expect("Messages request JSON");
	assert!(body.get("tools").is_none());
	assert!(state.namespaces.is_empty());
}

#[test]
fn prompt_cache_prewarming_is_rejected_without_generation() {
	let error = translate(&request(json!({
		"model": "claude",
		"input": "Prepare this cache without generating an answer.",
		"prompt_cache_options": {"prewarm": true}
	})))
	.expect_err("Messages cannot preserve Responses cache prewarming");
	assert!(error.to_string().contains("prompt_cache_options"));
}

#[rstest::rstest]
#[case::call("in_progress", "completed")]
#[case::incomplete_call("incomplete", "completed")]
#[case::result("completed", "in_progress")]
fn in_progress_function_history_is_rejected(
	#[case] call_status: &str,
	#[case] result_status: &str,
) {
	assert!(translate(&request(json!({
		"model": "claude", "input": [
			{"type": "function_call", "call_id": "call_pending", "name": "get_weather", "arguments": "{}", "status": call_status},
			{"type": "function_call_output", "call_id": "call_pending", "output": "partial", "status": result_status}
		]
	}))).is_err());
}

#[test]
fn unmappable_buffered_citations_are_rejected() {
	let body = Bytes::from(
		json!({
			"id": "msg_citation", "type": "message", "role": "assistant", "model": "claude",
			"stop_reason": "end_turn", "usage": {"input_tokens": 1, "output_tokens": 1},
			"content": [{"type": "text", "text": "Cited answer", "citations": [{
				"type": "char_location", "cited_text": "source", "document_index": 0,
				"document_title": "document", "start_char_index": 0, "end_char_index": 6
			}]}]
		})
		.to_string(),
	);
	assert!(translate_response(&body, &State::default(), 1024 * 1024).is_err());
}

#[rstest::rstest]
#[case::singular(json!({"type": "citations_delta", "citation": {"type": "char_location", "document_index": 0}}))]
#[case::plural(json!({"type": "citations_delta", "citations": [{"type": "char_location", "document_index": 0}]}))]
#[tokio::test]
async fn unmappable_streamed_citations_emit_one_safe_error(#[case] delta: serde_json::Value) {
	let mut frames = vec![
		message_start(1),
		sse_event(
			"content_block_start",
			json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
		),
		sse_event(
			"content_block_delta",
			json!({"type": "content_block_delta", "index": 0, "delta": delta}),
		),
		sse_event(
			"content_block_stop",
			json!({"type": "content_block_stop", "index": 0}),
		),
	];
	frames.extend(terminal("end_turn", 1));
	assert_one_safe_error(&collect_stream(frames, 1024 * 1024, State::default()).await);
}

#[tokio::test]
async fn empty_citation_delta_preserves_text_and_usage() {
	let mut frames = vec![
		message_start(1),
		sse_event(
			"content_block_start",
			json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
		),
		sse_event(
			"content_block_delta",
			json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hello"}}),
		),
		sse_event(
			"content_block_delta",
			json!({"type": "content_block_delta", "index": 0, "delta": {"type": "citations_delta", "citations": []}}),
		),
		sse_event(
			"content_block_delta",
			json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": " world"}}),
		),
		sse_event(
			"content_block_stop",
			json!({"type": "content_block_stop", "index": 0}),
		),
	];
	frames.extend(terminal("end_turn", 3));
	let events = collect_stream(frames, 1024 * 1024, State::default()).await;
	let response = &events.last().unwrap()["response"];
	assert_eq!(response["status"], "completed");
	assert_eq!(response["output"][0]["content"][0]["text"], "hello world");
	assert_eq!(response["usage"]["output_tokens"], 3);
}

#[test]
fn namespaced_custom_tool_remains_unsupported() {
	let error = translate(&request(json!({
		"model": "claude",
		"input": "hello",
		"tools": [{
			"type": "namespace",
			"name": "tools",
			"description": "Grouped tools",
			"tools": [{"type": "custom", "name": "python", "format": {"type": "text"}}]
		}]
	})))
	.expect_err("custom namespace members have no Messages function mapping");
	assert!(
		error
			.to_string()
			.contains("namespaced custom tools cannot be converted")
	);
}

#[rstest::rstest]
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
	let body = agent_http::Body::from_stream(stream::iter(vec![Err::<Bytes, std::io::Error>(
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
	let body = agent_http::Body::from("data: {\"type\":\"message_start\"}\n\n");
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
	let upstream = agent_http::Body::from_stream(stream::poll_fn(move |cx| receiver.poll_recv(cx)));
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
