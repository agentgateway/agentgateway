use std::collections::HashSet;

use serde_json::{Value, json};

use super::*;
use crate::server_tools::parse_sse;

fn request(input: Value, tools: Value) -> Request {
	serde_json::from_value(json!({
		"model": "m",
		"input": input,
		"tools": tools,
	}))
	.unwrap()
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
fn builtin_tools_are_found_by_type_and_never_shadow_client_functions() {
	let req = request(
		json!("hi"),
		json!([
			{"type": "function", "name": "read", "parameters": {}},
			{"type": "web_search_preview"},
			{"type": "file_search", "vector_store_ids": ["vs_1"]},
			{"type": "mcp", "server_label": "docs", "server_url": "https://x"},
			{"type": "function", "name": "code_interpreter", "parameters": {}},
			{"type": "code_interpreter", "container": {"type": "auto"}},
		]),
	);
	let found = find_builtin_tools(&req, &matchers(&["web_search*", "code_interpreter"]), &[]);
	assert_eq!(found.len(), 1, "{found:?}");
	assert_eq!(found[0].name, "web_search_preview");
	assert_eq!(found[0].mapping, 0);
	assert!(found[0].max_uses.is_none());
	assert_eq!(
		client_tool_names(&req),
		HashSet::from(["read".to_string(), "code_interpreter".to_string()])
	);
}

#[test]
fn mcp_descriptors_carry_allowed_tools_and_approval() {
	let req = request(
		json!("hi"),
		json!([
			{"type": "web_search"},
			{"type": "mcp", "server_label": "docs", "server_url": "https://docs/mcp", "require_approval": "never", "allowed_tools": ["search"]},
			{"type": "mcp", "server_label": "code", "allowed_tools": {"tool_names": ["run"]},
			 "require_approval": {"never": {"tool_names": ["run"]}, "always": {"tool_names": ["rm"]}}},
			{"type": "mcp", "server_label": "plain"},
		]),
	);
	let found = find_mcp_descriptors(&req);
	assert_eq!(found.len(), 3);
	assert_eq!(found[0].index, 1);
	assert_eq!(found[0].server_label, "docs");
	assert_eq!(found[0].server_url.as_deref(), Some("https://docs/mcp"));
	assert!(found[0].allows("search"));
	assert!(!found[0].allows("other"));
	assert!(!found[0].requires_approval("search"));
	assert!(found[1].allows("run"));
	assert!(!found[1].requires_approval("run"));
	assert!(found[1].requires_approval("rm"));
	assert!(found[1].requires_approval("unlisted"));
	// The API default is to require approval for everything.
	assert!(found[2].allows("anything"));
	assert!(found[2].requires_approval("anything"));
}

#[test]
fn builtins_are_rewritten_in_place_and_descriptors_are_expanded() {
	let mut req = request(
		json!("hi"),
		json!([
			{"type": "web_search"},
			{"type": "mcp", "server_label": "docs"},
			{"type": "function", "name": "read", "parameters": {}},
		]),
	);
	let found = find_builtin_tools(&req, &matchers(&["web_search"]), &[]);
	rewrite_builtin_tools(&mut req, &found, &[definition()]);
	replace_tools(
		&mut req,
		vec![(
			1,
			vec![
				function_tool("search", &definition()),
				function_tool("fetch", &definition()),
			],
		)],
	);
	let tools = req.rest["tools"].as_array().unwrap();
	let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
	assert_eq!(names, ["web_search", "search", "fetch", "read"]);
	assert!(tools.iter().all(|t| t["type"] == "function"));
	assert_eq!(tools[0]["description"], json!("Search the web"));
	assert_eq!(tools[0]["parameters"]["type"], json!("object"));
}

#[test]
fn function_calls_are_read_and_stripped() {
	let mut response = json!({
		"id": "resp_1",
		"status": "completed",
		"output": [
			{"type": "reasoning", "id": "rs_1", "summary": []},
			{"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "web_search", "arguments": "{\"query\":\"x\"}"},
			{"type": "function_call", "id": "fc_2", "call_id": "call_2", "name": "read", "arguments": ""},
		],
	});
	let calls = function_calls(&response);
	assert_eq!(calls.len(), 2);
	assert_eq!(calls[0].id, "call_1");
	assert_eq!(calls[0].input, json!({"query": "x"}));
	assert_eq!(calls[1].input, json!({}));
	assert_eq!(
		strip_function_calls(&mut response, &HashSet::from(["web_search"])),
		1
	);
	let remaining: Vec<&str> = response["output"]
		.as_array()
		.unwrap()
		.iter()
		.map(|i| i["type"].as_str().unwrap())
		.collect();
	assert_eq!(remaining, ["reasoning", "function_call"]);
}

#[test]
fn tool_turn_is_appended_after_a_text_input() {
	let mut req = request(json!("Who won?"), json!([]));
	append_tool_turn(
		&mut req,
		vec![
			json!({"type": "function_call", "call_id": "call_1", "name": "web_search", "arguments": "{}"}),
		],
		vec![function_call_output("call_1", "Seattle".to_string())],
	);
	let RequestInput::Items(items) = &req.input else {
		panic!("input should be items");
	};
	let items: Vec<Value> = items
		.iter()
		.map(|i| serde_json::to_value(i).unwrap())
		.collect();
	assert_eq!(items.len(), 3);
	assert_eq!(items[0]["role"], json!("user"));
	assert_eq!(items[0]["content"], json!("Who won?"));
	assert_eq!(items[1]["type"], json!("function_call"));
	assert_eq!(items[2]["type"], json!("function_call_output"));
	assert_eq!(items[2]["output"], json!("Seattle"));
}

#[test]
fn mcp_content_is_flattened_and_capped() {
	let items = vec![
		json!({"type": "text", "text": "one"}),
		json!({"type": "image", "data": "AAAA", "mimeType": "image/png"}),
		json!({"type": "resource", "resource": {"uri": "file:///a", "text": "two"}}),
	];
	assert_eq!(
		mcp_content_to_output(&items, 1024),
		"one\n[image image/png]\ntwo"
	);
	let capped = mcp_content_to_output(&items, 6);
	assert!(capped.starts_with("one\n[i"), "{capped}");
	assert!(capped.ends_with(TRUNCATION_MARKER));
}

#[test]
fn usage_is_summed_and_written_back() {
	let first = json!({"usage": {"input_tokens": 10, "output_tokens": 2, "input_tokens_details": {"cached_tokens": 4}}});
	let second = json!({"usage": {"input_tokens": 20, "output_tokens": 3, "output_tokens_details": {"reasoning_tokens": 1}}});
	let mut totals = UsageTotals::of(&first);
	totals.add(UsageTotals::of(&second));
	assert_eq!(totals.input_tokens, 30);
	assert_eq!(totals.output_tokens, 5);
	assert_eq!(totals.cached_tokens, Some(4));
	assert_eq!(totals.reasoning_tokens, Some(1));
	let mut response = json!({"id": "resp", "output": []});
	set_usage(&mut response, totals);
	assert_eq!(response["usage"]["input_tokens"], json!(30));
	assert_eq!(response["usage"]["total_tokens"], json!(35));
	assert_eq!(
		response["usage"]["input_tokens_details"]["cached_tokens"],
		json!(4)
	);
}

fn completed(text: &str) -> Value {
	json!({
		"id": "resp_1",
		"object": "response",
		"status": "completed",
		"model": "m",
		"output": [
			{"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
			 "content": [{"type": "output_text", "text": text, "annotations": []}]},
		],
		"usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2},
	})
}

#[test]
fn final_response_comes_from_the_terminal_event() {
	let events = synthesize_sse(&completed("done"));
	let names: Vec<&str> = events.iter().map(|e| e.event.as_deref().unwrap()).collect();
	assert_eq!(
		names,
		[
			"response.created",
			"response.in_progress",
			"response.output_item.added",
			"response.content_part.added",
			"response.output_text.delta",
			"response.output_text.done",
			"response.content_part.done",
			"response.output_item.done",
			"response.completed",
		]
	);
	let final_resp = final_response(&events).unwrap();
	assert_eq!(final_resp["output"][0]["content"][0]["text"], json!("done"));
	let delta: Value = serde_json::from_str(&events[4].data).unwrap();
	assert_eq!(delta["delta"], json!("done"));
	assert_eq!(delta["sequence_number"], json!(4));
	// A stream without a terminal event has no response.
	assert!(final_response(&events[..8]).is_none());
	// Round-trips through the SSE encoder.
	let reparsed = parse_sse(&crate::server_tools::encode_sse(&events));
	assert_eq!(reparsed.len(), events.len());
}

#[test]
fn usage_is_patched_into_the_terminal_event() {
	let mut events = synthesize_sse(&completed("done"));
	patch_usage(
		&mut events,
		UsageTotals {
			input_tokens: 300,
			output_tokens: 30,
			cached_tokens: None,
			reasoning_tokens: None,
		},
	);
	let final_resp = final_response(&events).unwrap();
	assert_eq!(final_resp["usage"]["input_tokens"], json!(300));
	assert_eq!(final_resp["usage"]["output_tokens"], json!(30));
	assert_eq!(final_resp["usage"]["total_tokens"], json!(330));
	assert!(String::from_utf8_lossy(&keepalive()).starts_with(':'));
	assert!(String::from_utf8_lossy(&error_event("boom")).contains("server_tool_error"));
}

#[test]
fn response_is_rebuilt_from_deltas_when_the_terminal_output_is_empty() {
	// The shape the gateway's Completions-to-Responses stream translator produces: a generic
	// `event` name, text only in deltas, empty content on the done item and on the terminal event.
	let ev = |data: Value| SseEvent {
		event: Some("event".to_string()),
		data: data.to_string(),
	};
	let events = vec![
		ev(json!({"type": "response.created", "sequence_number": 0,
			"response": {"id": "resp_1", "object": "response", "status": "in_progress", "model": "m", "output": []}})),
		ev(
			json!({"type": "response.output_item.added", "output_index": 0,
			"item": {"type": "message", "id": "msg_1", "role": "assistant", "status": "in_progress", "content": []}}),
		),
		ev(
			json!({"type": "response.content_part.added", "item_id": "msg_1", "output_index": 0, "content_index": 0,
			"part": {"type": "output_text", "text": "", "annotations": []}}),
		),
		ev(
			json!({"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "Seattle "}),
		),
		ev(
			json!({"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "won."}),
		),
		ev(
			json!({"type": "response.output_item.added", "output_index": 1,
			"item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "web_search", "arguments": ""}}),
		),
		ev(
			json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "{\"query\":"}),
		),
		ev(
			json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "\"x\"}"}),
		),
		ev(
			json!({"type": "response.output_item.done", "output_index": 1,
			"item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "web_search", "arguments": "{\"query\":\"x\"}", "status": "completed"}}),
		),
		ev(
			json!({"type": "response.content_part.done", "item_id": "msg_1", "output_index": 0, "content_index": 0,
			"part": {"type": "output_text", "text": "", "annotations": []}}),
		),
		ev(
			json!({"type": "response.output_item.done", "output_index": 0,
			"item": {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed", "content": []}}),
		),
		ev(json!({"type": "response.completed", "sequence_number": 11,
			"response": {"id": "resp_1", "object": "response", "status": "completed", "model": "m", "output": [],
				"usage": {"input_tokens": 100, "output_tokens": 10, "total_tokens": 110}}})),
	];
	let response = final_response(&events).unwrap();
	assert_eq!(response["status"], json!("completed"));
	assert_eq!(response["usage"]["input_tokens"], json!(100));
	let output = response["output"].as_array().unwrap();
	assert_eq!(output.len(), 2);
	assert_eq!(output[0]["type"], json!("message"));
	assert_eq!(output[0]["content"][0]["text"], json!("Seattle won."));
	assert_eq!(output[1]["type"], json!("function_call"));
	assert_eq!(output[1]["arguments"], json!("{\"query\":\"x\"}"));
	let calls = function_calls(&response);
	assert_eq!(calls[0].input, json!({"query": "x"}));
	// Without the terminal event the stream is incomplete.
	assert!(final_response(&events[..11]).is_none());
}

#[test]
fn client_executed_builtins_are_skipped_and_unmapped_ones_listed() {
	let req = request(
		json!("hi"),
		json!([
			{"type": "local_shell"},
			{"type": "apply_patch"},
			{"type": "web_search"},
			{"type": "file_search", "vector_store_ids": ["vs_1"]},
			{"type": "mcp", "server_label": "docs"},
			{"type": "function", "name": "read", "parameters": {}},
		]),
	);
	let defaults = matchers(crate::server_tools::DEFAULT_CLIENT_EXECUTED_TOOL_TYPES);
	let found = find_builtin_tools(&req, &matchers(&["*"]), &defaults);
	let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
	assert_eq!(names, ["web_search", "file_search"]);
	// An empty list lets the mapping take the shell tools too.
	let found = find_builtin_tools(&req, &matchers(&["*"]), &[]);
	let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
	assert_eq!(
		names,
		["local_shell", "apply_patch", "web_search", "file_search"]
	);
	assert_eq!(
		unmapped_builtin_tools(&req, &matchers(&["web_search*"]), &defaults),
		["file_search"]
	);
}

#[test]
fn function_tools_and_forced_choice_are_removed() {
	let mut req: Request = serde_json::from_value(json!({
		"model": "m",
		"input": "hi",
		"tools": [
			{"type": "function", "name": "web_search", "parameters": {}},
			{"type": "function", "name": "read", "parameters": {}},
			{"type": "web_search"},
		],
		"tool_choice": {"type": "function", "name": "web_search"},
	}))
	.unwrap();
	remove_function_tools(&mut req, &HashSet::from(["web_search"]));
	let tools = req.rest["tools"].as_array().unwrap();
	assert_eq!(tools.len(), 2);
	assert_eq!(tools[0]["name"], json!("read"));
	assert_eq!(tools[1]["type"], json!("web_search"));
	assert!(req.rest.get("tool_choice").is_none());
}
