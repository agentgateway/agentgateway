use std::collections::HashSet;

use serde_json::{Value, json};

use super::*;
use crate::types::messages::Request;

fn request(tools: Value) -> Request {
	serde_json::from_value(json!({
		"model": "mock-model",
		"max_tokens": 1024,
		"messages": [{"role": "user", "content": "who won?"}],
		"tools": tools,
	}))
	.unwrap()
}

fn web_search_matchers() -> Vec<TypeMatch> {
	vec![TypeMatch::parse("web_search_*")]
}

#[test]
fn type_match_parse_and_matches() {
	let prefix = TypeMatch::parse("web_search_*");
	assert_eq!(prefix, TypeMatch::Prefix("web_search_".to_string()));
	assert!(prefix.matches("web_search_20250305"));
	assert!(prefix.matches("web_search_20260209"));
	assert!(!prefix.matches("bash_20250124"));

	let exact = TypeMatch::parse("web_search_20250305");
	assert!(exact.matches("web_search_20250305"));
	assert!(!exact.matches("web_search_20260209"));
}

#[test]
fn find_server_tools_matches_only_mapped_server_tools() {
	let req = request(json!([
		{"name": "Read", "description": "read a file", "input_schema": {"type": "object"}},
		{"type": "web_search_20250305", "name": "web_search", "max_uses": 8},
		{"type": "bash_20250124", "name": "bash"},
		{"type": "custom", "name": "Edit", "input_schema": {"type": "object"}},
	]));
	let found = find_server_tools(&req, &web_search_matchers(), &[]);
	assert_eq!(
		found,
		vec![InterceptedTool {
			name: "web_search".to_string(),
			tool_type: "web_search_20250305".to_string(),
			mapping: 0,
			max_uses: Some(8),
		}]
	);
	assert!(find_server_tools(&request(json!([])), &web_search_matchers(), &[]).is_empty());
	assert!(find_server_tools(&request(Value::Null), &web_search_matchers(), &[]).is_empty());
}

#[test]
fn rewrite_server_tools_keeps_name_and_cache_control() {
	let mut req = request(json!([
		{"name": "Read", "input_schema": {"type": "object"}},
		{"type": "web_search_20250305", "name": "web_search", "max_uses": 8, "cache_control": {"type": "ephemeral"}},
		{"type": "bash_20250124", "name": "bash"},
	]));
	let tools = find_server_tools(&req, &web_search_matchers(), &[]);
	let definitions = vec![ToolDefinition {
		description: Some("Search the web".to_string()),
		input_schema: json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}),
	}];
	rewrite_server_tools(&mut req, &tools, &definitions);
	let tools = req.rest["tools"].as_array().unwrap();
	assert_eq!(tools.len(), 3);
	assert_eq!(tools[0]["name"], json!("Read"));
	assert_eq!(
		tools[1],
		json!({
			"name": "web_search",
			"description": "Search the web",
			"input_schema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
			"cache_control": {"type": "ephemeral"},
		})
	);
	assert_eq!(tools[2], json!({"type": "bash_20250124", "name": "bash"}));
}

fn tool_use_message() -> Value {
	json!({
		"id": "msg_1",
		"type": "message",
		"role": "assistant",
		"model": "mock-model",
		"content": [
			{"type": "text", "text": "Let me look that up."},
			{"type": "tool_use", "id": "toolu_1", "name": "web_search", "input": {"query": "super bowl lx winner"}},
			{"type": "tool_use", "id": "toolu_2", "name": "Read", "input": {"path": "/tmp/x"}},
		],
		"stop_reason": "tool_use",
		"stop_sequence": null,
		"usage": {"input_tokens": 100, "output_tokens": 20, "cache_read_input_tokens": 50},
	})
}

#[test]
fn tool_uses_and_strip() {
	let mut message = tool_use_message();
	let calls = tool_uses(&message);
	assert_eq!(calls.len(), 2);
	assert_eq!(calls[0].name, "web_search");
	assert_eq!(calls[0].input, json!({"query": "super bowl lx winner"}));

	let ours: HashSet<&str> = HashSet::from(["web_search"]);
	assert_eq!(strip_tool_uses(&mut message, &ours), 1);
	assert_eq!(stop_reason(&message), Some("tool_use"));
	assert_eq!(tool_uses(&message).len(), 1);

	let theirs: HashSet<&str> = HashSet::from(["Read"]);
	assert_eq!(strip_tool_uses(&mut message, &theirs), 1);
	assert_eq!(stop_reason(&message), Some("end_turn"));
	assert_eq!(message["content"].as_array().unwrap().len(), 1);
	assert_eq!(strip_tool_uses(&mut message, &theirs), 0);
}

#[test]
fn fingerprint_is_order_independent() {
	let a = vec![
		ToolUse {
			id: "1".into(),
			name: "web_search".into(),
			input: json!({"query": "a"}),
		},
		ToolUse {
			id: "2".into(),
			name: "web_search".into(),
			input: json!({"query": "b"}),
		},
	];
	let mut b = a.clone();
	b.reverse();
	assert_eq!(fingerprint(&a), fingerprint(&b));
	let c = vec![a[0].clone()];
	assert_ne!(fingerprint(&a), fingerprint(&c));
}

#[test]
fn usage_totals_add_and_set() {
	let mut totals = UsageTotals::of(&tool_use_message());
	assert_eq!(
		totals,
		UsageTotals {
			input_tokens: 100,
			output_tokens: 20,
			cache_creation_input_tokens: None,
			cache_read_input_tokens: Some(50),
		}
	);
	totals.add(UsageTotals {
		input_tokens: 300,
		output_tokens: 40,
		cache_creation_input_tokens: Some(7),
		cache_read_input_tokens: None,
	});
	assert_eq!(totals.input_tokens, 400);
	assert_eq!(totals.output_tokens, 60);
	assert_eq!(totals.cache_creation_input_tokens, Some(7));
	assert_eq!(totals.cache_read_input_tokens, Some(50));

	let mut message = tool_use_message();
	set_usage(&mut message, totals);
	assert_eq!(
		message["usage"],
		json!({"input_tokens": 400, "output_tokens": 60, "cache_creation_input_tokens": 7, "cache_read_input_tokens": 50})
	);
}

#[test]
fn append_tool_turn_serializes_blocks() {
	let mut req = request(json!([]));
	let assistant = tool_use_message()["content"].as_array().unwrap().clone();
	let results = vec![tool_result_block(
		"toolu_1",
		vec![json!({"type": "text", "text": "Seattle won."})],
		false,
	)];
	append_tool_turn(&mut req, assistant.clone(), results);
	let body: Value = serde_json::to_value(&req).unwrap();
	let messages = body["messages"].as_array().unwrap();
	assert_eq!(messages.len(), 3);
	assert_eq!(messages[1]["role"], json!("assistant"));
	assert_eq!(messages[1]["content"], Value::Array(assistant));
	assert_eq!(messages[2]["role"], json!("user"));
	assert_eq!(
		messages[2]["content"],
		json!([{"type": "tool_result", "tool_use_id": "toolu_1", "content": [{"type": "text", "text": "Seattle won."}]}])
	);
	assert_eq!(body["model"], json!("mock-model"));
}

#[test]
fn tool_result_block_marks_errors() {
	let block = tool_result_block("toolu_1", vec![], true);
	assert_eq!(block["is_error"], json!(true));
	let block = tool_result_block("toolu_1", vec![], false);
	assert!(block.get("is_error").is_none());
}

#[test]
fn mcp_content_conversion_and_truncation() {
	let items = vec![
		json!({"type": "text", "text": "hello"}),
		json!({"type": "image", "data": "aGk=", "mimeType": "image/jpeg"}),
		json!({"type": "resource", "resource": {"uri": "file:///a", "text": "resource text"}}),
		json!({"type": "audio", "data": "x"}),
	];
	let out = mcp_content_to_tool_result(&items, 1024);
	assert_eq!(out.len(), 4);
	assert_eq!(out[0], json!({"type": "text", "text": "hello"}));
	assert_eq!(
		out[1],
		json!({"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": "aGk="}})
	);
	assert_eq!(out[2], json!({"type": "text", "text": "resource text"}));
	assert_eq!(out[3]["type"], json!("text"));
	assert!(out[3]["text"].as_str().unwrap().contains("audio"));

	let items = vec![
		json!({"type": "text", "text": "0123456789"}),
		json!({"type": "text", "text": "dropped"}),
	];
	let out = mcp_content_to_tool_result(&items, 4);
	assert_eq!(out.len(), 1);
	let text = out[0]["text"].as_str().unwrap();
	assert!(text.starts_with("0123"));
	assert!(text.contains("truncated"));

	let items = vec![json!({"type": "text", "text": "héllo"})];
	let out = mcp_content_to_tool_result(&items, 2);
	assert!(out[0]["text"].as_str().unwrap().starts_with("h\n"));
}

const STREAM: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"mock-model\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\n\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: ping\ndata: {\"type\":\"ping\"}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Let me \"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"search.\"}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"web_search\",\"input\":{}}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\": \\\"super\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\" bowl\\\"}\"}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":15}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

#[test]
fn parse_and_encode_sse_roundtrip() {
	let events = parse_sse(STREAM.as_bytes());
	assert_eq!(events.len(), 12);
	assert_eq!(events[0].event.as_deref(), Some("message_start"));
	assert_eq!(events[2].event.as_deref(), Some("ping"));
	let encoded = encode_sse(&events);
	assert_eq!(parse_sse(&encoded), events);

	let multiline = parse_sse(b"data: a\r\ndata: b\r\n\r\n: comment\n\ndata: c\n\n");
	assert_eq!(multiline.len(), 2);
	assert_eq!(multiline[0].data, "a\nb");
	assert_eq!(multiline[0].event, None);
	assert_eq!(multiline[1].data, "c");

	let unterminated = parse_sse(b"event: message_stop\ndata: {\"type\":\"message_stop\"}");
	assert_eq!(unterminated.len(), 1);
}

#[test]
fn accumulator_rebuilds_message() {
	let mut acc = MessageAccumulator::default();
	acc.feed_all(&parse_sse(STREAM.as_bytes()));
	assert!(acc.is_complete());
	let message = acc.finish().unwrap();
	assert_eq!(
		message,
		json!({
			"id": "msg_1",
			"type": "message",
			"role": "assistant",
			"model": "mock-model",
			"content": [
				{"type": "text", "text": "Let me search."},
				{"type": "tool_use", "id": "toolu_1", "name": "web_search", "input": {"query": "super bowl"}},
			],
			"stop_reason": "tool_use",
			"stop_sequence": null,
			"usage": {"input_tokens": 25, "output_tokens": 15},
		})
	);
	assert_eq!(
		UsageTotals::of(&message),
		UsageTotals {
			input_tokens: 25,
			output_tokens: 15,
			cache_creation_input_tokens: None,
			cache_read_input_tokens: None,
		}
	);

	let mut incomplete = MessageAccumulator::default();
	incomplete.feed_all(&parse_sse(STREAM.as_bytes())[..9]);
	assert!(!incomplete.is_complete());
	let message = incomplete.finish().unwrap();
	assert_eq!(
		message["content"][1]["input"],
		json!({"query": "super bowl"})
	);
	assert_eq!(message["stop_reason"], Value::Null);

	let mut empty = MessageAccumulator::default();
	empty.feed_all(&parse_sse(b"event: ping\ndata: {\"type\":\"ping\"}\n\n"));
	assert!(empty.finish().is_none());
}

#[test]
fn accumulator_keeps_thinking_and_unknown_blocks() {
	let stream = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig\"}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"redacted_thinking\",\"data\":\"opaque\"}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"future_block\",\"payload\":{\"a\":1}}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":3}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
	let mut acc = MessageAccumulator::default();
	acc.feed_all(&parse_sse(stream.as_bytes()));
	let message = acc.finish().unwrap();
	assert_eq!(
		message["content"],
		json!([
			{"type": "thinking", "thinking": "hmm", "signature": "sig"},
			{"type": "redacted_thinking", "data": "opaque"},
			{"type": "future_block", "payload": {"a": 1}},
		])
	);
	assert_eq!(message["stop_reason"], json!("end_turn"));
}

#[test]
fn synthesize_sse_roundtrips_through_accumulator() {
	let message = json!({
		"id": "msg_2",
		"type": "message",
		"role": "assistant",
		"model": "mock-model",
		"content": [
			{"type": "thinking", "thinking": "consider", "signature": "abc"},
			{"type": "text", "text": "Seattle won Super Bowl LX."},
			{"type": "tool_use", "id": "toolu_9", "name": "Read", "input": {"path": "/x"}},
			{"type": "redacted_thinking", "data": "zzz"},
		],
		"stop_reason": "tool_use",
		"stop_sequence": null,
		"usage": {"input_tokens": 400, "output_tokens": 60, "cache_read_input_tokens": 50},
	});
	let events = synthesize_sse(&message);
	assert_eq!(events[0].event.as_deref(), Some("message_start"));
	assert_eq!(
		events.last().unwrap().event.as_deref(),
		Some("message_stop")
	);
	let start: Value = serde_json::from_str(&events[0].data).unwrap();
	assert_eq!(start["message"]["content"], json!([]));
	assert_eq!(start["message"]["stop_reason"], Value::Null);
	assert_eq!(start["message"]["usage"]["input_tokens"], json!(400));
	assert_eq!(start["message"]["usage"]["output_tokens"], json!(0));

	let mut acc = MessageAccumulator::default();
	acc.feed_all(&events);
	assert!(acc.is_complete());
	assert_eq!(acc.finish().unwrap(), message);
}

#[test]
fn patch_usage_rewrites_start_and_delta() {
	let mut events = parse_sse(STREAM.as_bytes());
	patch_usage(
		&mut events,
		UsageTotals {
			input_tokens: 500,
			output_tokens: 70,
			cache_creation_input_tokens: None,
			cache_read_input_tokens: Some(9),
		},
	);
	let start: Value = serde_json::from_str(&events[0].data).unwrap();
	assert_eq!(
		start["message"]["usage"],
		json!({"input_tokens": 500, "output_tokens": 1, "cache_read_input_tokens": 9})
	);
	let delta: Value = serde_json::from_str(&events[10].data).unwrap();
	assert_eq!(delta["usage"], json!({"output_tokens": 70}));
	assert_eq!(delta["delta"]["stop_reason"], json!("tool_use"));

	let mut acc = MessageAccumulator::default();
	acc.feed_all(&events);
	assert_eq!(
		UsageTotals::of(&acc.finish().unwrap()),
		UsageTotals {
			input_tokens: 500,
			output_tokens: 70,
			cache_creation_input_tokens: None,
			cache_read_input_tokens: Some(9),
		}
	);
}

#[test]
fn keepalive_and_error_events_are_anthropic_shaped() {
	let ping = parse_sse(&ping_event());
	assert_eq!(ping.len(), 1);
	assert_eq!(ping[0].event.as_deref(), Some("ping"));
	assert_eq!(
		serde_json::from_str::<Value>(&ping[0].data).unwrap(),
		json!({"type": "ping"})
	);

	let error = parse_sse(&error_event("tool failed"));
	assert_eq!(error[0].event.as_deref(), Some("error"));
	let data: Value = serde_json::from_str(&error[0].data).unwrap();
	assert_eq!(data["type"], json!("error"));
	assert_eq!(data["error"]["type"], json!("api_error"));
	assert_eq!(data["error"]["message"], json!("tool failed"));
}

fn default_client_executed() -> Vec<TypeMatch> {
	DEFAULT_CLIENT_EXECUTED_TOOL_TYPES
		.iter()
		.map(|p| TypeMatch::parse(p))
		.collect()
}

#[test]
fn client_executed_list_is_configurable_with_the_vendor_defaults() {
	let defaults = default_client_executed();
	assert!(is_client_executed("bash_20250124", &defaults));
	assert!(is_client_executed("text_editor_20250728", &defaults));
	assert!(is_client_executed("computer_20251124", &defaults));
	assert!(is_client_executed("memory_20250818", &defaults));
	assert!(is_client_executed("local_shell", &defaults));
	assert!(!is_client_executed("web_search_20250305", &defaults));
	// An empty list trusts the mapping; a longer one guards more.
	assert!(!is_client_executed("bash_20250124", &[]));
	let stricter = vec![TypeMatch::parse("web_fetch_*")];
	assert!(is_client_executed("web_fetch_20250910", &stricter));
	assert!(!is_client_executed("bash_20250124", &stricter));
	let req: Request = serde_json::from_value(json!({
		"model": "m",
		"max_tokens": 1,
		"messages": [{"role": "user", "content": "hi"}],
		"tools": [
			{"type": "bash_20250124", "name": "bash"},
			{"type": "web_search_20250305", "name": "web_search"},
			{"type": "web_fetch_20250910", "name": "web_fetch"},
		],
	}))
	.unwrap();
	// A mapping that would match bash is ignored for it under the defaults...
	let matchers = vec![TypeMatch::parse("*")];
	let found = find_server_tools(&req, &matchers, &defaults);
	let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
	assert_eq!(names, ["web_search", "web_fetch"]);
	// ... and honoured when the operator empties the list.
	let found = find_server_tools(&req, &matchers, &[]);
	let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
	assert_eq!(names, ["bash", "web_search", "web_fetch"]);
	// Unmapped server tools exclude client-executed ones.
	let only_search = vec![TypeMatch::parse("web_search*")];
	assert_eq!(
		unmapped_server_tools(&req, &only_search, &defaults),
		["web_fetch_20250910"]
	);
	assert_eq!(
		unmapped_server_tools(&req, &only_search, &[]),
		["bash_20250124", "web_fetch_20250910"]
	);
}

#[test]
fn tools_and_forced_choice_are_removed() {
	let mut req: Request = serde_json::from_value(json!({
		"model": "m",
		"max_tokens": 1,
		"messages": [{"role": "user", "content": "hi"}],
		"tools": [
			{"name": "web_search", "input_schema": {"type": "object"}},
			{"name": "read", "input_schema": {"type": "object"}},
		],
		"tool_choice": {"type": "tool", "name": "web_search"},
	}))
	.unwrap();
	remove_tools(&mut req, &HashSet::from(["web_search"]));
	assert_eq!(req.rest["tools"].as_array().unwrap().len(), 1);
	assert_eq!(req.rest["tools"][0]["name"], json!("read"));
	assert!(req.rest.get("tool_choice").is_none());
}

#[test]
fn search_results_are_extracted_from_json_text_and_links() {
	let json_text = json!({"results": [
		{"title": "Super Bowl LX", "url": "https://example.com/sb", "content": "Seattle won 24-17."},
		{"name": "no url here"},
	]})
	.to_string();
	let found = extract_search_results(&json_text);
	assert_eq!(found.len(), 1);
	assert_eq!(found[0].url, "https://example.com/sb");
	assert_eq!(found[0].title, "Super Bowl LX");
	assert_eq!(found[0].snippet, "Seattle won 24-17.");

	let nested =
		json!({"web": {"results": [{"url": "https://a.example", "title": "A", "description": "d"}]}})
			.to_string();
	assert_eq!(extract_search_results(&nested)[0].title, "A");

	let array = json!([{"link": "https://b.example", "name": "B"}]).to_string();
	assert_eq!(extract_search_results(&array)[0].url, "https://b.example");

	let text = "Title: First\nURL: https://one.example\nSnippet: one\n\nTitle: Second\nURL: https://two.example\nsome text";
	let found = extract_search_results(text);
	assert_eq!(found.len(), 2);
	assert_eq!(found[1].url, "https://two.example");
	assert_eq!(found[1].snippet, "some text");

	let bare = "See https://x.example/a, and https://y.example/b.";
	let found = extract_search_results(bare);
	assert_eq!(found.len(), 2);
	assert_eq!(found[0].url, "https://x.example/a");
	assert_eq!(found[1].url, "https://y.example/b");

	assert!(extract_search_results("Seattle won.").is_empty());
	assert!(extract_search_results("{\"answer\": 42}").is_empty());
}

#[test]
fn native_search_blocks_are_built_and_flattened_on_replay() {
	let call = ToolUse {
		id: "srvtoolu_1".to_string(),
		name: "web_search".to_string(),
		input: json!({"query": "super bowl lx"}),
	};
	let results = vec![SearchResult {
		url: "https://example.com/sb".to_string(),
		title: "Super Bowl LX".to_string(),
		snippet: "Seattle won 24-17.".to_string(),
	}];
	let (use_block, result_block) = web_search_blocks(&call, &results);
	assert_eq!(use_block["type"], json!("server_tool_use"));
	assert_eq!(use_block["input"]["query"], json!("super bowl lx"));
	assert_eq!(result_block["type"], json!("web_search_tool_result"));
	assert_eq!(result_block["tool_use_id"], json!("srvtoolu_1"));
	assert_eq!(
		result_block["content"][0]["url"],
		json!("https://example.com/sb")
	);
	assert_eq!(result_block["content"][0]["encrypted_content"], json!(""));
	assert_eq!(
		result_block["content"][0]["snippet"],
		json!("Seattle won 24-17.")
	);

	let mut message = json!({"content": [{"type": "text", "text": "Seattle won."}]});
	prepend_blocks(&mut message, vec![use_block.clone(), result_block.clone()]);
	assert_eq!(message["content"].as_array().unwrap().len(), 3);
	assert_eq!(message["content"][2]["type"], json!("text"));

	// The stream shape carries the search input as a delta, like a tool_use block.
	let events = synthesize_sse(&json!({"content": [use_block.clone()], "usage": {}}));
	assert!(events.iter().any(|e| e.data.contains("input_json_delta")));

	// A replay of the synthesised pair becomes one assistant text block with the query and results.
	let mut req: Request = serde_json::from_value(json!({
		"model": "m",
		"max_tokens": 1,
		"messages": [
			{"role": "user", "content": "Who won?"},
			{"role": "assistant", "content": [use_block, result_block, {"type": "text", "text": "Seattle won."}]},
			{"role": "assistant", "content": [
				{"type": "server_tool_use", "id": "real", "name": "web_search", "input": {"query": "q"}},
				{"type": "web_search_tool_result", "tool_use_id": "real",
				 "content": [{"type": "web_search_result", "url": "https://r", "title": "R", "encrypted_content": "opaque"}]}]},
			{"role": "assistant", "content": [{"type": "web_search_tool_result", "tool_use_id": "err",
				"content": {"type": "web_search_tool_result_error", "error_code": "max_uses_exceeded"}}]},
		],
	}))
	.unwrap();
	assert_eq!(flatten_replayed_search_results(&mut req), 2);
	let replay = serde_json::to_value(&req.messages[1]).unwrap();
	let blocks = replay["content"].as_array().unwrap();
	assert_eq!(blocks.len(), 2, "{replay}");
	assert_eq!(blocks[0]["type"], json!("text"));
	let text = blocks[0]["text"].as_str().unwrap();
	assert!(
		text.starts_with("Web search for \"super bowl lx\":"),
		"{text}"
	);
	assert!(
		text.contains("URL: https://example.com/sb") && text.contains("Snippet: Seattle won 24-17."),
		"{text}"
	);
	assert_eq!(blocks[1]["text"], json!("Seattle won."));
	// Vendor-encrypted results are left untouched, with their server_tool_use.
	let encrypted = serde_json::to_value(&req.messages[2]).unwrap();
	assert_eq!(encrypted["content"][0]["type"], json!("server_tool_use"));
	assert_eq!(
		encrypted["content"][1]["type"],
		json!("web_search_tool_result")
	);
	let errored = serde_json::to_value(&req.messages[3]).unwrap();
	assert_eq!(errored["content"][0]["type"], json!("text"));
	assert!(
		errored["content"][0]["text"]
			.as_str()
			.unwrap()
			.contains("max_uses_exceeded")
	);
}

#[test]
fn mcp_tool_blocks_carry_the_call_and_its_result() {
	let call = ToolUse {
		id: "toolu_1".to_string(),
		name: "web_fetch".to_string(),
		input: json!({"url": "https://example.com"}),
	};
	let (use_block, result_block) = mcp_tool_blocks(
		&call,
		"search",
		"fetch",
		vec![json!({"type": "text", "text": "page text"})],
		false,
	);
	assert_eq!(use_block["type"], json!("mcp_tool_use"));
	assert_eq!(use_block["server_name"], json!("search"));
	assert_eq!(use_block["name"], json!("fetch"));
	assert_eq!(use_block["input"]["url"], json!("https://example.com"));
	assert_eq!(result_block["type"], json!("mcp_tool_result"));
	assert_eq!(result_block["tool_use_id"], json!("toolu_1"));
	assert_eq!(result_block["is_error"], json!(false));
	assert_eq!(result_block["content"][0]["text"], json!("page text"));
	// The stream shape carries the input as a delta, like a tool_use block.
	let events = synthesize_sse(&json!({"content": [use_block], "usage": {}}));
	assert!(events.iter().any(|e| e.data.contains("input_json_delta")));
	// The client's declaration is looked up by type and name.
	let req: Request = serde_json::from_value(json!({
		"model": "m",
		"max_tokens": 1,
		"messages": [{"role": "user", "content": "hi"}],
		"tools": [{"type": "web_search_20250305", "name": "web_search", "allowed_domains": ["example.com"]}],
	}))
	.unwrap();
	let tool = InterceptedTool {
		name: "web_search".to_string(),
		tool_type: "web_search_20250305".to_string(),
		mapping: 0,
		max_uses: None,
	};
	assert_eq!(
		declared_tool(&req, &tool)["allowed_domains"],
		json!(["example.com"])
	);
}
