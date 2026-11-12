#![allow(clippy::panic, clippy::unwrap_used)]

use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::*;
use crate::ir::events::ProviderError;
use crate::protocols::gemini::{Signature, read_signature};
use crate::protocols::messages::decoder::MessagesDecoder;
use crate::protocols::responses::decoder::decode_buffered as decode_responses;
use crate::test_support::SCOPE;

const LIMIT: usize = 1 << 20;

fn start() -> Value {
	json!({"type": "message_start", "message": {"id": "msg_1", "model": "claude-x",
		"usage": {"input_tokens": 10, "output_tokens": 1}}})
}

fn block(index: u32, block: &Value) -> Value {
	json!({"type": "content_block_start", "index": index, "content_block": block})
}

fn delta(index: u32, delta: &Value) -> Value {
	json!({"type": "content_block_delta", "index": index, "delta": delta})
}

fn stop(index: u32) -> Value {
	json!({"type": "content_block_stop", "index": index})
}

fn end(stop_reason: &str) -> [Value; 2] {
	[
		json!({"type": "message_delta", "delta": {"stop_reason": stop_reason},
			"usage": {"output_tokens": 5}}),
		json!({"type": "message_stop"}),
	]
}

fn text(index: u32, pieces: &[&str]) -> Vec<Value> {
	let mut events = vec![block(index, &json!({"type": "text", "text": ""}))];
	for piece in pieces {
		events.push(delta(index, &json!({"type": "text_delta", "text": piece})));
	}
	events.push(stop(index));
	events
}

fn thinking(index: u32, words: &str, signature: &str) -> Vec<Value> {
	vec![
		block(
			index,
			&json!({"type": "thinking", "thinking": "", "signature": ""}),
		),
		delta(index, &json!({"type": "thinking_delta", "thinking": words})),
		delta(
			index,
			&json!({"type": "signature_delta", "signature": signature}),
		),
		stop(index),
	]
}

fn call(index: u32, id: &str, arguments: &str) -> Vec<Value> {
	vec![
		block(
			index,
			&json!({"type": "tool_use", "id": id, "name": "read", "input": {}}),
		),
		delta(
			index,
			&json!({"type": "input_json_delta", "partial_json": arguments}),
		),
		stop(index),
	]
}

/// Each Messages event goes through the decoder into a Gemini stream. Returns
/// the data of every event the client got, or the HTTP error sent instead.
fn streamed(events: &[Value], show: bool) -> Result<Vec<Value>, HttpReply> {
	let mut writer = GeminiStream::new(SCOPE, LIMIT).show_thoughts(show);
	let mut decoder = MessagesDecoder::default();
	let mut sent = Vec::new();
	for event in events {
		let data = event.to_string();
		if decoder
			.push_json(data.as_bytes(), LIMIT, &mut |event| writer.event(&event))
			.is_err()
		{
			writer.fail(500);
		}
		while writer.progress() == Progress::HasOutput {
			sent.extend(writer.take_stream());
		}
	}
	if !decoder.is_finished() {
		writer.fail(500);
	}
	if let Some(reply) = writer.early_failure() {
		assert!(sent.is_empty());
		return Err(reply);
	}
	while writer.progress() == Progress::HasOutput {
		sent.extend(writer.take_stream());
	}
	Ok(client_events(&sent))
}

fn client_events(sent: &[u8]) -> Vec<Value> {
	let text = std::str::from_utf8(sent).unwrap();
	assert!(text.is_empty() || text.ends_with("\r\n\r\n"));
	text
		.split_terminator("\r\n\r\n")
		.map(|event| serde_json::from_str(event.strip_prefix("data: ").unwrap()).unwrap())
		.collect()
}

fn parts(event: &Value) -> Vec<Value> {
	event["candidates"][0]["content"]["parts"]
		.as_array()
		.cloned()
		.unwrap_or_default()
}

fn all_parts(events: &[Value]) -> Vec<Value> {
	events.iter().flat_map(parts).collect()
}

/// The provider items a signature contains.
fn items(signature: &Value) -> Vec<Value> {
	let Signature::Carriers(carriers) = read_signature(signature.as_str().unwrap()) else {
		panic!("not a gateway signature: {signature}");
	};
	carriers
		.iter()
		.map(|text| {
			let item =
				carrier::open(SCOPE, carrier::CarrierPurpose::GeminiThoughtSignature, text).unwrap();
			serde_json::from_slice(&item).unwrap()
		})
		.collect()
}

fn thinking_item(words: &str, signature: &str) -> Value {
	json!({"type": "thinking", "thinking": words, "signature": signature})
}

#[test]
fn each_delta_is_a_response_and_the_last_one_ends_the_reply() {
	let mut events = vec![start()];
	events.extend(text(0, &["Hel", "lo"]));
	events.extend(end("end_turn"));
	let sent = streamed(&events, false).unwrap();
	assert_eq!(
		sent,
		[
			json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Hel"}]}, "index": 0}],
				"modelVersion": "claude-x", "responseId": "msg_1"}),
			json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "lo"}]}, "index": 0}],
				"modelVersion": "claude-x", "responseId": "msg_1"}),
			json!({"candidates": [{"finishReason": "STOP", "index": 0}],
				"usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5, "totalTokenCount": 15},
				"modelVersion": "claude-x", "responseId": "msg_1"}),
		]
	);
}

#[test]
fn hidden_thinking_comes_back_in_the_signature_of_the_next_part() {
	let mut events = vec![start()];
	events.extend(thinking(0, "Plan.", "s1"));
	events.extend(text(1, &["Done", "."]));
	events.extend(end("end_turn"));
	let parts = all_parts(&streamed(&events, false).unwrap());
	assert_eq!(parts.len(), 2);
	assert_eq!(parts[0]["text"], "Done");
	assert_eq!(
		items(&parts[0]["thoughtSignature"]),
		[thinking_item("Plan.", "s1")]
	);
	assert_eq!(parts[1], json!({"text": "."}));
}

#[test]
fn shown_thinking_is_a_thought_part_and_its_state_rides_on_the_answer() {
	let mut events = vec![start()];
	events.extend(thinking(0, "Plan.", "s1"));
	events.extend(text(1, &["Done."]));
	events.extend(end("end_turn"));
	let parts = all_parts(&streamed(&events, true).unwrap());
	assert_eq!(parts[0], json!({"text": "Plan.", "thought": true}));
	assert_eq!(parts[1]["text"], "Done.");
	assert_eq!(
		items(&parts[1]["thoughtSignature"]),
		[thinking_item("Plan.", "s1")]
	);
}

#[test]
fn state_never_rides_on_a_thought_which_clients_drop() {
	let mut events = vec![start()];
	events.extend(thinking(0, "A.", "s1"));
	events.extend(thinking(1, "B.", "s2"));
	events.extend(text(2, &["Done."]));
	events.extend(end("end_turn"));
	let parts = all_parts(&streamed(&events, true).unwrap());
	assert_eq!(
		parts[..2],
		[
			json!({"text": "A.", "thought": true}),
			json!({"text": "B.", "thought": true})
		]
	);
	assert_eq!(
		items(&parts[2]["thoughtSignature"]),
		[thinking_item("A.", "s1"), thinking_item("B.", "s2")]
	);
}

#[test]
fn a_streamed_provider_reply_written_whole_joins_its_text() {
	let mut events = vec![start()];
	events.extend(thinking(0, "Plan.", "s1"));
	events.extend(text(1, &["Hel", "lo"]));
	events.extend(text(2, &[" again"]));
	events.extend(end("end_turn"));
	let mut writer = GeminiBuffered::new(SCOPE, LIMIT);
	let mut decoder = MessagesDecoder::default();
	for event in &events {
		let data = event.to_string();
		decoder
			.push_json(data.as_bytes(), LIMIT, &mut |event| writer.event(&event))
			.unwrap();
	}
	let reply: Value = serde_json::from_slice(&writer.into_reply().body).unwrap();
	let parts = parts(&reply);
	assert_eq!(parts.len(), 1);
	assert_eq!(parts[0]["text"], "Hello again");
	assert_eq!(
		items(&parts[0]["thoughtSignature"]),
		[thinking_item("Plan.", "s1")]
	);
}

#[test]
fn calls_go_out_whole_and_only_the_first_contains_the_state() {
	let mut events = vec![start()];
	events.extend(thinking(0, "A.", "s1"));
	events.extend(thinking(1, "B.", "s2"));
	events.extend(call(2, "toolu_1", r#"{"path":"a"}"#));
	events.extend(call(3, "toolu_2", ""));
	events.extend(end("tool_use"));
	let sent = streamed(&events, false).unwrap();
	let parts = all_parts(&sent);
	assert_eq!(parts.len(), 2);
	assert_eq!(
		parts[0]["functionCall"],
		json!({"id": "toolu_1", "name": "read", "args": {"path": "a"}})
	);
	assert_eq!(
		items(&parts[0]["thoughtSignature"]),
		[thinking_item("A.", "s1"), thinking_item("B.", "s2")]
	);
	assert_eq!(
		parts[1],
		json!({"functionCall": {"id": "toolu_2", "name": "read", "args": {}}})
	);
	assert_eq!(
		sent.last().unwrap()["candidates"][0]["finishReason"],
		"STOP"
	);
}

#[test]
fn state_with_nothing_after_it_rides_on_an_empty_text_part() {
	let mut events = vec![start()];
	events.extend(text(0, &["Done."]));
	events.extend(thinking(1, "After.", "s1"));
	events.extend(end("end_turn"));
	let sent = streamed(&events, false).unwrap();
	let last = sent.last().unwrap();
	let parts = parts(last);
	assert_eq!(parts.len(), 1);
	assert_eq!(parts[0]["text"], "");
	assert_eq!(
		items(&parts[0]["thoughtSignature"]),
		[thinking_item("After.", "s1")]
	);
	assert_eq!(last["candidates"][0]["finishReason"], "STOP");
}

#[test]
fn a_buffered_reply_joins_each_parts_text() {
	let body = json!({"id": "msg_1", "model": "claude-x", "content": [
		{"type": "thinking", "thinking": "Plan.", "signature": "s1"},
		{"type": "text", "text": "Done."},
		{"type": "tool_use", "id": "toolu_1", "name": "read", "input": {"path": "a"}}
	], "stop_reason": "tool_use", "usage": {"input_tokens": 10, "output_tokens": 5}});
	let mut writer = GeminiBuffered::new(SCOPE, LIMIT).show_thoughts(true);
	crate::protocols::messages::decoder::decode_buffered(body.to_string().as_bytes(), &mut |event| {
		writer.event(&event)
	})
	.unwrap();
	let reply = writer.into_reply();
	assert_eq!(reply.status, 200);
	let reply: Value = serde_json::from_slice(&reply.body).unwrap();
	let parts = parts(&reply);
	assert_eq!(parts[0], json!({"text": "Plan.", "thought": true}));
	assert_eq!(parts[1]["text"], "Done.");
	assert_eq!(
		items(&parts[1]["thoughtSignature"]),
		[thinking_item("Plan.", "s1")]
	);
	assert_eq!(
		parts[2],
		json!({"functionCall": {"id": "toolu_1", "name": "read", "args": {"path": "a"}}})
	);
	assert_eq!(reply["candidates"][0]["finishReason"], "STOP");
}

#[test]
fn usage_counts_cached_tokens_in_the_prompt_and_thoughts_apart() {
	let body = json!({"id": "resp_1", "model": "gpt-x", "status": "completed", "output": [
		{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Hi"}]}
	], "usage": {"input_tokens": 100, "input_tokens_details": {"cached_tokens": 40},
		"output_tokens": 30, "output_tokens_details": {"reasoning_tokens": 12}, "total_tokens": 130}});
	let mut writer = GeminiBuffered::new(SCOPE, LIMIT);
	decode_responses(body.to_string().as_bytes(), &mut |event| {
		writer.event(&event)
	})
	.unwrap();
	let reply: Value = serde_json::from_slice(&writer.into_reply().body).unwrap();
	assert_eq!(
		reply["usageMetadata"],
		json!({"promptTokenCount": 100, "cachedContentTokenCount": 40,
			"candidatesTokenCount": 18, "thoughtsTokenCount": 12, "totalTokenCount": 130})
	);
}

#[rstest::rstest]
#[case("max_tokens", "MAX_TOKENS")]
#[case("refusal", "SAFETY")]
#[case("end_turn", "STOP")]
fn each_ending_has_its_finish_reason(#[case] stop_reason: &str, #[case] finish: &str) {
	let mut events = vec![start()];
	events.extend(text(0, &["Hi"]));
	events.extend(end(stop_reason));
	let sent = streamed(&events, false).unwrap();
	assert_eq!(
		sent.last().unwrap()["candidates"][0]["finishReason"],
		finish
	);
}

#[test]
fn a_full_context_window_is_an_error() {
	let mut events = vec![start()];
	events.extend(end("model_context_window_exceeded"));
	let reply = streamed(&events, false).unwrap_err();
	assert_eq!(reply.status, 400);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(
		body,
		json!({"error": {"code": 400, "message": "The input is too long for the model.",
			"status": "INVALID_ARGUMENT"}})
	);
}

#[test]
fn a_failure_before_output_is_an_http_error_and_after_output_an_error_event() {
	let overloaded = json!({"type": "error", "error": {"type": "overloaded_error", "message": "x"}});
	let reply = streamed(&[start(), overloaded.clone()], false).unwrap_err();
	assert_eq!(reply.status, 503);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(body["error"]["status"], "UNAVAILABLE");

	let mut events = vec![start()];
	events.extend(text(0, &["Hel"])[..2].to_vec());
	events.push(overloaded);
	let sent = streamed(&events, false).unwrap();
	assert_eq!(parts(&sent[0]), [json!({"text": "Hel"})]);
	assert_eq!(
		sent.last().unwrap(),
		&json!({"error": {"code": 503, "message": "The provider is overloaded.", "status": "UNAVAILABLE"}})
	);
}

#[test]
fn a_call_cut_off_by_the_limit_is_left_out() {
	let mut events = vec![start()];
	events.extend(text(0, &["Hi"]));
	events.extend(call(1, "toolu_1", r#"{"pa"#));
	events.extend(end("max_tokens"));
	let sent = streamed(&events, false).unwrap();
	assert_eq!(all_parts(&sent), [json!({"text": "Hi"})]);
	assert_eq!(
		sent.last().unwrap()["candidates"][0]["finishReason"],
		"MAX_TOKENS"
	);
}

fn raw(text: &str) -> Box<RawValue> {
	RawValue::from_string(text.to_string()).unwrap()
}

#[test]
fn a_later_part_waits_for_the_first_and_its_text_stays_bounded() {
	let (first, second) = (raw(r#""first""#), raw(r#""second""#));
	let events = [
		Event::ResponseStart {
			id: "r",
			model: "m",
		},
		Event::PartStart {
			part: PartId(0),
			kind: PartKind::Text,
		},
		Event::PartStart {
			part: PartId(1),
			kind: PartKind::Text,
		},
		Event::Delta {
			part: PartId(1),
			text: JsonStr::new(&second).unwrap(),
		},
		Event::Delta {
			part: PartId(0),
			text: JsonStr::new(&first).unwrap(),
		},
		Event::PartEnd {
			part: PartId(0),
			value: Final::Text,
		},
		Event::PartEnd {
			part: PartId(1),
			value: Final::Text,
		},
		Event::Finish(Outcome::Completed),
	];
	let mut writer = GeminiStream::new(SCOPE, LIMIT);
	for event in &events {
		writer.event(event);
	}
	let sent = client_events(&writer.take_stream());
	assert_eq!(
		all_parts(&sent),
		[json!({"text": "first"}), json!({"text": "second"})]
	);

	let mut writer = GeminiStream::new(SCOPE, 3);
	for event in &events[..4] {
		writer.event(event);
	}
	assert_eq!(writer.progress(), Progress::Finished);
	assert_eq!(writer.early_failure().unwrap().status, 500);
}

#[test]
fn a_provider_error_event_is_classified_for_the_client() {
	let mut writer = GeminiStream::new(SCOPE, LIMIT);
	writer.event(&Event::Error(ProviderError {
		kind: ErrorKind::RateLimit,
		code: Some("secret"),
		message: Some("the gateway's account is over quota"),
		client_message: None,
	}));
	let reply = writer.early_failure().unwrap();
	assert_eq!(reply.status, 429);
	assert!(!String::from_utf8(reply.body).unwrap().contains("quota"));
}

#[test]
fn a_refused_field_and_a_refused_request_name_the_clients_field() {
	let reply = error_reply_naming(400, ErrorKind::Unknown, Some("generationConfig.topK"));
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(
		body["error"]["message"],
		"The provider refused `generationConfig.topK`."
	);
	let reply = refusal_reply(&RequestRefusal {
		problems: vec!["`tools[0].googleSearch` tool".into(), "field `seed`".into()],
	});
	assert_eq!(reply.status, 400);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(
		body,
		json!({"error": {"code": 400, "status": "INVALID_ARGUMENT",
			"message": "unsupported request: `tools[0].googleSearch` tool; field `seed`"}})
	);
}
