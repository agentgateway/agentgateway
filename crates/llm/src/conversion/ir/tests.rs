//! Drop records, refusals and reasoning for Messages -> Responses requests, and drops and usage for
//! Responses -> Messages buffered replies. Translated requests and replies are covered by the goldens.

use std::fs;

use rstest::rstest;
use serde_json::{Value, json};

use crate::conversion::ir::{self, DropReason, Dropped, Effort, Reasoning, ReasoningMode};
use crate::conversion::responses::from_messages;
use crate::types::messages::typed::MessagesResponse;
use crate::types::responses::typed as responses;
use crate::{AIError, types};

#[test]
fn records_request_drops() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "gpt-5.6-terra",
		"max_tokens": 1024,
		"stop_sequences": ["</end>"],
		"top_k": 40,
		"cache_control": {"type": "ephemeral"},
		"thinking": {"type": "enabled", "budget_tokens": 2048, "display": "summarized"},
		"output_config": {"effort": "low"},
		"system": [{"type": "text", "text": "cached", "cache_control": {"type": "ephemeral", "ttl": "1h"}}],
		"messages": [{
			"role": "user",
			"content": [
				{"type": "text", "text": "hello", "citations": []},
				{"type": "mystery"},
				{"type": "document", "source": {"type": "url", "url": "https://example.com/a.pdf"}, "title": "a.pdf", "context": "ctx"},
				{"type": "tool_result", "tool_use_id": "call_1", "content": [{"type": "text", "text": "ok"}, {"type": "mystery"}]}
			]
		}]
	}))
	.expect("failed to parse request");
	let (_, dropped) = translate_request(&request).expect("translates");
	let dropped = dropped
		.iter()
		.map(|dropped| (dropped.path.as_str(), dropped.reason))
		.collect::<Vec<_>>();
	let unread = |path| (path, DropReason::Unread);
	let unsupported = |path| (path, DropReason::Unsupported);
	assert_eq!(
		dropped,
		vec![
			unread("messages[0].content[1]"),
			unread("messages[0].content[3].content[1]"),
			unsupported("stop_sequences"),
			unsupported("top_k"),
			unsupported("cache_control"),
			unsupported("system[0].cache_control.ttl"),
			unsupported("thinking.display"),
			unsupported("thinking.budget_tokens"),
			unsupported("messages[0].content[0].citations"),
			unsupported("messages[0].content[2].context"),
			unsupported("messages[0].content[2].title"),
		]
	);
}

#[test]
fn records_nothing_when_off() {
	let request = request_with(json!({"top_k": 40, "stop_sequences": ["</end>"]}));
	let mut dropped = ir::Drops::new(false);
	let request = ir::messages::request::parse(&request, &mut dropped).expect("parses");
	ir::responses::request::render(request, &mut dropped).expect("renders");
	assert_eq!(dropped.list, []);
}

#[test]
fn skips_path_when_off() {
	let mut dropped = ir::Drops::new(false);
	dropped.record(DropReason::Unsupported, || {
		panic!("path built while not recording")
	});
	assert_eq!(dropped.list, []);
}

#[rstest]
#[case::missing_type(json!({"data": "aGVsbG8="}), "messages image source missing type")]
#[case::base64_missing_media_type(
	json!({"type": "base64", "data": "aGVsbG8="}),
	"messages base64 image source missing media_type"
)]
#[case::base64_missing_data(
	json!({"type": "base64", "media_type": "image/png"}),
	"messages base64 image source missing data"
)]
#[case::url_missing_url(json!({"type": "url"}), "messages url image source missing url")]
#[case::file_missing_id(json!({"type": "file"}), "messages file image source missing file_id")]
#[case::text(
	json!({"type": "text", "data": "hello"}),
	"messages image source cannot be represented by responses"
)]
#[case::unknown(
	json!({"type": "unknown_source"}),
	"messages image source cannot be represented by responses"
)]
fn refuses_image(#[case] source: Value, #[case] error: &str) {
	let image = json!({"type": "image", "source": source});
	assert_eq!(refusal(user_content(&image)), error);
}

#[rstest]
#[case::base64_missing_media_type(
	json!({"type": "base64", "data": "aGVsbG8="}),
	"messages base64 document source missing media_type"
)]
#[case::text_missing_data(json!({"type": "text"}), "messages text document source missing data")]
#[case::content(
	json!({"type": "content", "content": []}),
	"messages document source cannot be represented by responses"
)]
fn refuses_document(
	#[case] source: Value,
	#[case] error: &str,
	#[values(false, true)] in_tool_result: bool,
) {
	let mut part = json!({"type": "document", "source": source});
	if in_tool_result {
		part = json!({"type": "tool_result", "tool_use_id": "call_1", "content": [part]});
	}
	assert_eq!(refusal(user_content(&part)), error);
}

#[test]
fn refuses_max_tokens_over_u32() {
	let mut request = user_content(&json!({"type": "text", "text": "hello"}));
	request["max_tokens"] = json!(u64::from(u32::MAX) + 1);
	assert_eq!(
		refusal(request),
		"messages max_tokens exceeds responses max_output_tokens range"
	);
}

#[test]
fn requires_max_tokens() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "gpt-5.6-luna",
		"messages": [{"role": "user", "content": "hello"}],
	}))
	.expect("failed to parse request");
	assert!(matches!(
		translate_request(&request),
		Err(AIError::RequestParsing(..))
	));
}

/// Responses has no media in assistant turns, so their sources go unchecked.
#[test]
fn skips_assistant_media() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "gpt-5.6-luna",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": "describe it"},
			{"role": "assistant", "content": [
				{"type": "text", "text": "Here it is."},
				{"type": "image", "source": {"type": "url"}},
				{"type": "document", "source": {"type": "text"}},
			]},
		],
	}))
	.expect("failed to parse request");
	let request =
		from_messages::translate_request(&request).expect("assistant media is skipped, not refused");
	let wire = serde_json::to_value(request).expect("responses request serializes");
	assert_eq!(
		wire["input"][1]["content"],
		json!([{"type": "output_text", "text": "Here it is.", "annotations": []}])
	);
}

#[rstest]
#[case::unset(None, None)]
#[case::disabled(Some(true), Some(false))]
#[case::enabled(Some(false), Some(true))]
fn parallel_tool_calls(#[case] disable: Option<bool>, #[case] expected: Option<bool>) {
	let mut tool_choice = json!({"type": "auto"});
	if let Some(disable) = disable {
		tool_choice["disable_parallel_tool_use"] = json!(disable);
	}
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "gpt-5.6-luna",
		"max_tokens": 1024,
		"tools": [{"name": "lookup", "input_schema": {"type": "object"}}],
		"tool_choice": tool_choice,
		"messages": [{"role": "user", "content": "hello"}],
	}))
	.expect("failed to parse request");

	let read = ir::messages::request::parse(&request, &mut ir::Drops::new(false)).expect("reads");
	assert_eq!(read.parallel_tool_calls, expected);
	let wire = serde_json::to_value(from_messages::translate_request(&request).expect("translates"))
		.expect("responses request serializes");
	assert_eq!(
		wire.get("parallel_tool_calls").cloned(),
		expected.map(Value::Bool)
	);
}

#[rstest]
#[case::budget(
	json!({"thinking": {"type": "enabled", "budget_tokens": 1024}}),
	Reasoning { mode: Some(BUDGET), effort: None, display: None }
)]
#[case::budget_with_effort(
	json!({
		"thinking": {"type": "enabled", "budget_tokens": 1024, "display": "summarized"},
		"output_config": {"effort": "max"},
	}),
	Reasoning { mode: Some(BUDGET), effort: Some(Effort::Max), display: Some("summarized".to_string()) }
)]
#[case::effort_only(
	json!({"output_config": {"effort": "medium"}}),
	Reasoning { mode: None, effort: Some(Effort::Medium), display: None }
)]
fn reads_reasoning(#[case] fields: Value, #[case] expected: Reasoning) {
	let read =
		ir::messages::request::parse(&request_with(fields), &mut ir::Drops::new(false)).expect("reads");
	assert_eq!(read.reasoning, expected);
}

const BUDGET: ReasoningMode = ReasoningMode::Budget {
	tokens: 1024,
	fallback_effort: Effort::Low,
};

#[rstest]
#[case::effort_only(json!({"output_config": {"effort": "medium"}}), json!({"effort": "medium"}), &[])]
#[case::adaptive_without_effort(json!({"thinking": {"type": "adaptive"}}), json!({"effort": "high"}), &[])]
#[case::disabled_with_effort(
	json!({"thinking": {"type": "disabled"}, "output_config": {"effort": "low"}}),
	Value::Null,
	&["output_config.effort"]
)]
#[case::effort_with_budget(
	json!({"thinking": {"type": "enabled", "budget_tokens": 1024}, "output_config": {"effort": "max"}}),
	json!({"effort": "max"}),
	&["thinking.budget_tokens"]
)]
fn sends_reasoning(#[case] fields: Value, #[case] reasoning: Value, #[case] dropped: &[&str]) {
	let (request, dropped_fields) = translate_request(&request_with(fields)).expect("translates");
	let paths = dropped_fields
		.iter()
		.map(|dropped| dropped.path.as_str())
		.collect::<Vec<_>>();
	assert_eq!(paths, dropped);
	let wire = serde_json::to_value(request).expect("responses request serializes");
	assert_eq!(wire["reasoning"], reasoning);
}

/// The request sent upstream and what was left out of it.
fn translate_request(
	request: &types::messages::Request,
) -> Result<(types::responses::Request, Vec<Dropped>), AIError> {
	let mut dropped = ir::Drops::new(true);
	let request = ir::messages::request::parse(request, &mut dropped)?;
	let request = ir::responses::request::render(request, &mut dropped)?;
	Ok((request, dropped.list))
}

fn refusal(request: Value) -> String {
	let request: types::messages::Request =
		serde_json::from_value(request).expect("failed to parse request");
	match translate_request(&request) {
		Err(AIError::UnsupportedConversion(reason)) => reason.to_string(),
		other => panic!("expected a refusal, got {other:?}"),
	}
}

/// A one-message request with `fields` added at the top level.
fn request_with(fields: Value) -> types::messages::Request {
	let mut request = user_content(&json!({"type": "text", "text": "hello"}));
	let Value::Object(fields) = fields else {
		panic!("fields are an object");
	};
	for (key, value) in fields {
		request[&key] = value;
	}
	serde_json::from_value(request).expect("failed to parse request")
}

fn user_content(part: &Value) -> Value {
	json!({
		"model": "gpt-5.6-luna",
		"max_tokens": 1024,
		"messages": [{"role": "user", "content": [part]}],
	})
}

#[test]
fn records_reply_drops() {
	let reply = reply(
		json!([
			{"type": "reasoning", "id": "rs_1", "summary": []},
			{"type": "message", "id": "msg_1", "role": "assistant", "status": "completed", "content": [{
				"type": "output_text",
				"text": "See the docs.",
				"annotations": [
					{"type": "url_citation", "url": "https://example.com", "title": "Example", "start_index": 0, "end_index": 3},
					{"type": "file_citation", "file_id": "file_1", "filename": "notes.txt", "index": 0},
				],
			}]},
			{"type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{\"q\":\"docs\"}", "status": "completed"},
		]),
		Value::Null,
	);

	let (output, dropped) = translate_reply(reply);
	let dropped = dropped
		.iter()
		.map(|dropped| (dropped.path.as_str(), dropped.reason))
		.collect::<Vec<_>>();
	assert_eq!(
		dropped,
		vec![
			("output[0]", DropReason::Unread),
			("output[1].content[0].annotations[1]", DropReason::Unread),
		]
	);

	let wire = serde_json::to_value(output).expect("messages reply serializes");
	assert_eq!(
		wire["content"],
		json!([
			{"type": "text", "text": "See the docs.", "citations": [{
				"type": "web_search_result_location",
				"url": "https://example.com",
				"title": "Example",
				"cited_text": "",
				"encrypted_index": "",
			}]},
			{"type": "tool_use", "id": "call_1", "name": "lookup", "input": {"q": "docs"}},
		])
	);
	assert_eq!(wire["stop_reason"], "tool_use");
}

/// Responses counts cached tokens inside `input_tokens`; Messages counts them apart.
#[test]
fn splits_cache_from_input() {
	let reply = reply(
		json!([]),
		json!({
			"input_tokens": 100,
			"input_tokens_details": {"cached_tokens": 30, "cache_write_tokens": 20},
			"output_tokens": 7,
			"output_tokens_details": {"reasoning_tokens": 0},
			"total_tokens": 107,
		}),
	);

	let (output, _) = translate_reply(reply);
	let wire = serde_json::to_value(output).expect("messages reply serializes");
	assert_eq!(
		wire["usage"],
		json!({
			"input_tokens": 50,
			"output_tokens": 7,
			"cache_read_input_tokens": 30,
			"cache_creation_input_tokens": 20,
			"service_tier": "default",
		})
	);
}

/// The client's reply and what was left out of it.
fn translate_reply(reply: responses::Response) -> (MessagesResponse, Vec<Dropped>) {
	let mut dropped = ir::Drops::new(true);
	let reply = ir::responses::reply::parse(reply, &mut dropped).expect("parses");
	(ir::messages::reply::render(reply), dropped.list)
}

/// The reasoning fixture with its output and usage replaced.
fn reply(output: Value, usage: Value) -> responses::Response {
	let fixture = fs::read_to_string("src/tests/response/responses/reasoning.json")
		.expect("failed to read fixture");
	let mut reply: Value = serde_json::from_str(&fixture).expect("fixture is JSON");
	reply["output"] = output;
	reply["usage"] = usage;
	serde_json::from_value(reply).expect("failed to parse reply")
}
