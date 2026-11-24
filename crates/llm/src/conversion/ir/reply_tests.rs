//! Responses -> Messages buffered reply drop records and usage. Translated replies are covered by
//! the reply goldens.

use std::fs;

use serde_json::{Value, json};

use crate::conversion::ir::{self, DropReason, Dropped};
use crate::types::messages::typed::MessagesResponse;
use crate::types::responses::typed as responses;

#[test]
fn records_drops() {
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

	let (output, dropped) = translate(reply);
	let dropped = dropped
		.iter()
		.map(|dropped| (dropped.path.as_str(), dropped.reason()))
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

	let (output, _) = translate(reply);
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
fn translate(reply: responses::Response) -> (MessagesResponse, Vec<Dropped>) {
	let mut dropped = Vec::new();
	let reply = ir::responses::parse_reply(reply, &mut dropped).expect("parses");
	(ir::messages::render_reply(reply), dropped)
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
