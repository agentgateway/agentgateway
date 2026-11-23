//! Messages -> Responses request drop records and refusals. Translated requests are covered by the
//! request goldens.

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::conversion::ir::{self, DropReason, Dropped, Effort, Reasoning, ReasoningMode};
use crate::conversion::responses::from_messages;
use crate::{AIError, types};

const MESSAGES_FIXTURES: &str = "src/tests/requests/messages";

/// Fixtures that keep everything are omitted.
#[test]
fn fixture_drops() {
	let mut drops = Map::new();
	for name in fixture_names(MESSAGES_FIXTURES) {
		let path = Path::new(MESSAGES_FIXTURES).join(format!("{name}.json"));
		let input = fs::read_to_string(&path).expect("failed to read fixture");
		let request: types::messages::Request =
			serde_json::from_str(&input).expect("failed to parse fixture");
		match translate(&request) {
			Ok((_, dropped)) if dropped.is_empty() => {},
			Ok((_, dropped)) => {
				drops.insert(name, json!(dropped));
			},
			Err(error) => {
				drops.insert(name, json!({"error": error_text(&error)}));
			},
		}
	}
	insta::with_settings!({
		omit_expression => true,
		prepend_module_to_snapshot => false,
		snapshot_path => "../../tests/requests/messages",
	}, {
		insta::assert_json_snapshot!("dropped.responses", Value::Object(drops));
	});
}

#[test]
fn records_drops() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "gpt-5.6-terra",
		"max_tokens": 1024,
		"context_management": {"edits": [{"type": "clear_thinking_20251015"}]},
		"tools": [{"name": "lookup", "input_schema": {"type": "object"}, "x-trace-id": "abc"}],
		"stop_sequences": ["</end>"],
		"top_k": 40,
		"cache_control": {"type": "ephemeral"},
		"thinking": {"type": "enabled", "budget_tokens": 2048, "display": "summarized"},
		"output_config": {"effort": "low", "x-mode": "fast"},
		"system": [{"type": "text", "text": "cached", "cache_control": {"type": "ephemeral", "ttl": "1h", "scope": "turn"}}],
		"messages": [{
			"role": "user",
			"x-note": "kept by client",
			"content": [
				{"type": "text", "text": "hello", "citations": []},
				{"type": "mystery"},
				{"type": "document", "source": {"type": "url", "url": "https://example.com/a.pdf"}, "title": "a.pdf", "context": "ctx"},
				{"type": "tool_result", "tool_use_id": "call_1", "content": [{"type": "text", "text": "ok"}, {"type": "mystery"}]}
			]
		}]
	}))
	.expect("failed to parse request");
	let (_, dropped) = translate(&request).expect("translates");
	let dropped = dropped
		.iter()
		.map(|dropped| (dropped.path.as_str(), dropped.reason()))
		.collect::<Vec<_>>();
	let unread = |path| (path, DropReason::Unread);
	let unsupported = |path| (path, DropReason::Unsupported);
	assert_eq!(
		dropped,
		vec![
			unread("context_management"),
			unread("output_config.x-mode"),
			unread("messages[0].x-note"),
			unread("tools[0].x-trace-id"),
			unread("messages[0].content[1]"),
			unread("messages[0].content[3].content[1]"),
			unsupported("stop_sequences"),
			unsupported("top_k"),
			unsupported("cache_control"),
			unsupported("system[0].cache_control.ttl"),
			unread("system[0].cache_control.scope"),
			unsupported("thinking.display"),
			unsupported("thinking.budget_tokens"),
			unsupported("messages[0].content[0].citations"),
			unsupported("messages[0].content[2].context"),
			unsupported("messages[0].content[2].title"),
		]
	);
}

#[test]
fn redacts_client_keys() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "gpt-5.6-terra",
		"max_tokens": 1024,
		"x-user.42": true,
		"tools": [{"name": "lookup", "input_schema": {"type": "object"}, "x-trace.abc": "1"}],
		"messages": [{"role": "user", "content": [
			{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral", "tenant-7": "a"}},
			{"type": "mystery"}
		]}]
	}))
	.expect("failed to parse request");
	let (_, dropped) = translate(&request).expect("translates");
	let logged = dropped
		.iter()
		.map(|dropped| (dropped.path.as_str(), dropped.redacted_path().into_owned()))
		.collect::<Vec<_>>();
	assert_eq!(
		logged,
		vec![
			("x-user.42", "*".to_string()),
			("tools[0].x-trace.abc", "tools[0].*".to_string()),
			(
				"messages[0].content[1]",
				"messages[0].content[1]".to_string()
			),
			(
				"messages[0].content[0].cache_control.tenant-7",
				"messages[0].content[0].cache_control.*".to_string()
			),
		]
	);
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
	assert_eq!(refusal(user_content(image)), error);
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
	assert_eq!(refusal(user_content(part)), error);
}

#[test]
fn refuses_max_tokens_over_u32() {
	let mut request = user_content(json!({"type": "text", "text": "hello"}));
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
		translate(&request),
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

	let read = ir::messages::parse_request(&request, &mut Vec::new()).expect("reads");
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
	let read = ir::messages::parse_request(&request_with(fields), &mut Vec::new()).expect("reads");
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
	let (request, dropped_fields) = translate(&request_with(fields)).expect("translates");
	let paths = dropped_fields
		.iter()
		.map(|dropped| dropped.path.as_str())
		.collect::<Vec<_>>();
	assert_eq!(paths, dropped);
	let wire = serde_json::to_value(request).expect("responses request serializes");
	assert_eq!(wire["reasoning"], reasoning);
}

/// The request sent upstream and what was left out of it.
fn translate(
	request: &types::messages::Request,
) -> Result<(types::responses::Request, Vec<Dropped>), AIError> {
	let mut dropped = Vec::new();
	let request = ir::messages::parse_request(request, &mut dropped)?;
	let request = ir::responses::render_request(request, &mut dropped)?;
	Ok((request, dropped))
}

fn refusal(request: Value) -> String {
	let request: types::messages::Request =
		serde_json::from_value(request).expect("failed to parse request");
	match translate(&request) {
		Err(AIError::UnsupportedConversion(reason)) => reason.to_string(),
		other => panic!("expected a refusal, got {other:?}"),
	}
}

/// A one-message request with `fields` added at the top level.
fn request_with(fields: Value) -> types::messages::Request {
	let mut request = user_content(json!({"type": "text", "text": "hello"}));
	for (key, value) in fields.as_object().expect("fields are an object") {
		request[key] = value.clone();
	}
	serde_json::from_value(request).expect("failed to parse request")
}

fn user_content(part: Value) -> Value {
	json!({
		"model": "gpt-5.6-luna",
		"max_tokens": 1024,
		"messages": [{"role": "user", "content": [part]}],
	})
}

fn error_text(error: &AIError) -> String {
	format!("{}: {error}", error_kind(error))
}

fn error_kind(error: &AIError) -> &'static str {
	match error {
		AIError::UnsupportedConversion(_) => "unsupported_conversion",
		AIError::RequestParsing(..) => "request_parsing",
		AIError::RequestMarshal(_) => "request_marshal",
		_ => "other",
	}
}

/// Sorted, so the snapshot is stable.
fn fixture_names(dir: &str) -> Vec<String> {
	let mut names = fs::read_dir(dir)
		.expect("failed to read fixture directory")
		.map(|entry| entry.expect("failed to read fixture entry").path())
		.filter(|path| path.extension().is_some_and(|ext| ext == "json"))
		.map(|path| {
			path
				.file_stem()
				.expect("fixture has a name")
				.to_string_lossy()
				.to_string()
		})
		.collect::<Vec<_>>();
	names.sort();
	names
}
