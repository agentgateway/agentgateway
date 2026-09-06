use std::collections::HashSet;

use serde_json::{Value, json};

use super::*;
use crate::server_tools::{encode_sse, parse_sse};

fn request(extra: Value) -> Request {
	let mut body = json!({
		"model": "m",
		"messages": [{"role": "user", "content": "Who won?"}],
	});
	if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
		for (k, v) in extra {
			body.insert(k.clone(), v.clone());
		}
	}
	serde_json::from_value(body).unwrap()
}

fn matchers(patterns: &[&str]) -> Vec<TypeMatch> {
	patterns.iter().map(|p| TypeMatch::parse(p)).collect()
}

fn definition() -> ToolDefinition {
	ToolDefinition {
		description: Some("Search the web".to_string()),
		input_schema: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
	}
}

#[test]
fn web_search_options_becomes_a_function_tool() {
	let mut req = request(json!({
		"web_search_options": {"search_context_size": "low"},
		"tools": [{"type": "function", "function": {"name": "read", "parameters": {}}}],
	}));
	let tool = find_web_search(&req, &matchers(&["web_search*"])).unwrap();
	assert_eq!(tool.name, WEB_SEARCH_TOOL_NAME);
	assert_eq!(tool.tool_type, WEB_SEARCH_OPTIONS_TYPE);
	rewrite_web_search(&mut req, &tool, &definition());
	assert!(req.rest.get("web_search_options").is_none());
	let tools = req.tools.as_ref().unwrap();
	assert_eq!(tools.len(), 2);
	assert_eq!(tools[1]["function"]["name"], json!("web_search"));
	assert_eq!(tools[1]["function"]["description"], json!("Search the web"));
	assert_eq!(tools[1]["function"]["parameters"]["type"], json!("object"));
	// Nothing to do without the field, without a mapping, or when the client owns the name.
	assert!(find_web_search(&request(json!({})), &matchers(&["web_search*"])).is_none());
	assert!(
		find_web_search(
			&request(json!({"web_search_options": {}})),
			&matchers(&["file_search"])
		)
		.is_none()
	);
	let taken = request(json!({
		"web_search_options": {},
		"tools": [{"type": "function", "function": {"name": "web_search", "parameters": {}}}],
	}));
	assert!(find_web_search(&taken, &matchers(&["web_search*"])).is_none());
}

fn completion(tool_calls: Option<Value>, text: Option<&str>) -> Value {
	let mut message = json!({"role": "assistant", "content": text});
	if let Some(calls) = tool_calls {
		message["tool_calls"] = calls;
	}
	json!({
		"id": "chatcmpl-1",
		"object": "chat.completion",
		"created": 1,
		"model": "m",
		"choices": [{"index": 0, "message": message, "finish_reason": if text.is_some() { "stop" } else { "tool_calls" }}],
		"usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12},
	})
}

#[test]
fn tool_calls_are_read_and_stripped() {
	let mut response = completion(
		Some(json!([
			{"id": "call_1", "type": "function", "function": {"name": "web_search", "arguments": "{\"query\":\"x\"}"}},
			{"id": "call_2", "type": "function", "function": {"name": "read", "arguments": ""}},
		])),
		None,
	);
	let calls = tool_calls(&response);
	assert_eq!(calls.len(), 2);
	assert_eq!(calls[0].id, "call_1");
	assert_eq!(calls[0].input, json!({"query": "x"}));
	assert_eq!(calls[1].input, json!({}));
	assert_eq!(
		strip_tool_calls(&mut response, &HashSet::from(["web_search"])),
		1
	);
	assert_eq!(tool_calls(&response).len(), 1);
	assert_eq!(response["choices"][0]["finish_reason"], json!("tool_calls"));
	assert_eq!(strip_tool_calls(&mut response, &HashSet::from(["read"])), 1);
	assert!(
		response["choices"][0]["message"]
			.get("tool_calls")
			.is_none()
	);
	assert_eq!(response["choices"][0]["finish_reason"], json!("stop"));
}

#[test]
fn tool_turn_is_appended() {
	let mut req = request(json!({}));
	let assistant = json!({"role": "assistant", "content": null,
		"tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "web_search", "arguments": "{}"}}]});
	append_tool_turn(
		&mut req,
		assistant,
		vec![tool_message("call_1", "Seattle".to_string())],
	);
	assert_eq!(req.messages.len(), 3);
	assert_eq!(req.messages[1].role, "assistant");
	assert_eq!(req.messages[1].tool_calls.as_ref().unwrap().len(), 1);
	assert_eq!(req.messages[2].role, "tool");
	assert_eq!(req.messages[2].tool_call_id.as_deref(), Some("call_1"));
	assert_eq!(req.messages[2].message_text(), Some("Seattle"));
}

#[test]
fn usage_is_summed_and_written_back() {
	let first = json!({"usage": {"prompt_tokens": 10, "completion_tokens": 2, "prompt_tokens_details": {"cached_tokens": 4}}});
	let second = json!({"usage": {"prompt_tokens": 20, "completion_tokens": 3, "completion_tokens_details": {"reasoning_tokens": 1}}});
	let mut totals = UsageTotals::of(&first);
	totals.add(UsageTotals::of(&second));
	assert_eq!(totals.prompt_tokens, 30);
	assert_eq!(totals.completion_tokens, 5);
	assert_eq!(totals.cached_tokens, Some(4));
	assert_eq!(totals.reasoning_tokens, Some(1));
	let mut response = completion(None, Some("hi"));
	set_usage(&mut response, totals);
	assert_eq!(response["usage"]["prompt_tokens"], json!(30));
	assert_eq!(response["usage"]["total_tokens"], json!(35));
	assert_eq!(
		response["usage"]["prompt_tokens_details"]["cached_tokens"],
		json!(4)
	);
}

#[test]
fn chunks_are_accumulated_and_synthesized() {
	let chunk = |delta: Value, finish: Value| SseEvent {
		event: None,
		data: json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "m",
			"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
		.to_string(),
	};
	let events = vec![
		chunk(json!({"role": "assistant", "content": ""}), Value::Null),
		chunk(json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "web_search", "arguments": ""}}]}), Value::Null),
		chunk(json!({"tool_calls": [{"index": 0, "function": {"arguments": "{\"query\":"}}]}), Value::Null),
		chunk(json!({"tool_calls": [{"index": 0, "function": {"arguments": "\"x\"}"}}]}), Value::Null),
		chunk(json!({}), json!("tool_calls")),
		SseEvent {
			event: None,
			data: json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "m", "choices": [],
				"usage": {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110}})
			.to_string(),
		},
		SseEvent {
			event: None,
			data: "[DONE]".to_string(),
		},
	];
	let mut acc = ChunkAccumulator::default();
	acc.feed_all(&events);
	assert!(acc.is_complete());
	let message = acc.finish().unwrap();
	assert_eq!(message["object"], json!("chat.completion"));
	assert_eq!(message["choices"][0]["finish_reason"], json!("tool_calls"));
	assert_eq!(message["usage"]["prompt_tokens"], json!(100));
	let calls = tool_calls(&message);
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0].id, "call_1");
	assert_eq!(calls[0].input, json!({"query": "x"}));

	// A finished answer round-trips through synthesis and back.
	let answer = completion(None, Some("Seattle won."));
	let synthesized = synthesize_sse(&answer);
	assert_eq!(synthesized.last().unwrap().data, "[DONE]");
	let encoded = encode_sse(&synthesized);
	let text = String::from_utf8_lossy(&encoded);
	assert!(!text.contains("event:"), "{text}");
	let reparsed = parse_sse(&encoded);
	let mut acc = ChunkAccumulator::default();
	acc.feed_all(&reparsed);
	let rebuilt = acc.finish().unwrap();
	assert_eq!(
		rebuilt["choices"][0]["message"]["content"],
		json!("Seattle won.")
	);
	assert_eq!(rebuilt["choices"][0]["finish_reason"], json!("stop"));
	assert_eq!(rebuilt["usage"]["prompt_tokens"], json!(10));
}

#[test]
fn usage_is_patched_or_added() {
	let totals = UsageTotals {
		prompt_tokens: 300,
		completion_tokens: 30,
		cached_tokens: None,
		reasoning_tokens: None,
	};
	let mut with_usage = synthesize_sse(&completion(None, Some("hi")));
	patch_usage(&mut with_usage, totals);
	let mut acc = ChunkAccumulator::default();
	acc.feed_all(&with_usage);
	assert_eq!(acc.finish().unwrap()["usage"]["total_tokens"], json!(330));

	let mut without = synthesize_sse(&completion(None, Some("hi")));
	without.retain(|ev| !ev.data.contains("\"usage\""));
	let before = without.len();
	patch_usage(&mut without, totals);
	assert_eq!(without.len(), before + 1);
	assert_eq!(without.last().unwrap().data, "[DONE]");
	let mut acc = ChunkAccumulator::default();
	acc.feed_all(&without);
	assert_eq!(acc.finish().unwrap()["usage"]["prompt_tokens"], json!(300));
	assert!(String::from_utf8_lossy(&keepalive()).starts_with(':'));
	let err = String::from_utf8_lossy(&error_event("boom")).to_string();
	assert!(
		err.contains("server_tool_error") && err.ends_with("data: [DONE]\n\n"),
		"{err}"
	);
}

#[test]
fn function_tools_and_forced_choice_are_removed() {
	let mut req = request(json!({
		"tools": [
			{"type": "function", "function": {"name": "web_search", "parameters": {}}},
			{"type": "function", "function": {"name": "read", "parameters": {}}},
		],
		"tool_choice": {"type": "function", "function": {"name": "web_search"}},
	}));
	remove_function_tools(&mut req, &HashSet::from(["web_search"]));
	let tools = req.tools.as_ref().unwrap();
	assert_eq!(tools.len(), 1);
	assert_eq!(tools[0]["function"]["name"], json!("read"));
	assert!(req.tool_choice.is_none());
}
