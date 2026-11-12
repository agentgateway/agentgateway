#![allow(clippy::panic, clippy::unwrap_used)]

use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::*;
use crate::ir::events::{Arguments, ProviderError};
use crate::protocols::CallKind;
use crate::protocols::chat_completions::{open_call_id, split_call_id};
use crate::test_support::SCOPE;

/// One provider event, written so a test reads as the reply it stands for.
#[derive(Clone, Copy)]
enum Step {
	Text(u32),
	Delta(u32, &'static str),
	TextEnd(u32),
	Refusal(u32, &'static str),
	/// A reasoning item, and its text if shown.
	Reasoning(u32, &'static str, Option<&'static str>),
	CallStart(u32, &'static str, &'static str),
	/// Its arguments as text, and whether they're complete.
	CallEnd(u32, &'static str, bool),
	Used,
	Finish(Outcome<'static>),
	Error(ErrorKind),
}
use Step::*;

const USAGE: crate::ir::events::Usage = crate::ir::events::Usage {
	input_tokens: Some(30),
	output_tokens: Some(5),
	total_tokens: Some(35),
	cached_tokens: Some(20),
	cache_write_tokens: None,
	cache_write_12h_tokens: None,
	reasoning_tokens: Some(2),
};
const DONE: Outcome<'static> = Outcome::Completed;
const CUT: Outcome<'static> = Outcome::Incomplete {
	reason: StopReason::MaxTokens,
	raw: None,
};
const THINKING: &str = r#"{"type":"thinking","thinking":"Plan.","signature":"c2ln"}"#;

fn string(text: &str) -> Box<RawValue> {
	RawValue::from_string(serde_json::to_string(text).unwrap()).unwrap()
}

fn feed(steps: &[Step], sink: &mut dyn FnMut(&Event<'_>)) {
	sink(&Event::ResponseStart {
		id: "r1",
		model: "m1",
	});
	for step in steps {
		match *step {
			Text(part) => sink(&Event::PartStart {
				part: PartId(part),
				kind: PartKind::Text,
			}),
			Delta(part, text) => {
				let text = string(text);
				sink(&Event::Delta {
					part: PartId(part),
					text: JsonStr::new(&text).unwrap(),
				});
			},
			TextEnd(part) => sink(&Event::PartEnd {
				part: PartId(part),
				value: Final::Text,
			}),
			Refusal(part, text) => {
				let text = string(text);
				sink(&Event::PartStart {
					part: PartId(part),
					kind: PartKind::Refusal,
				});
				sink(&Event::PartEnd {
					part: PartId(part),
					value: Final::Refusal(JsonStr::new(&text).unwrap()),
				});
			},
			Reasoning(part, item, text) => {
				let item = RawValue::from_string(item.to_string()).unwrap();
				let text = text.map(string);
				sink(&Event::PartStart {
					part: PartId(part),
					kind: PartKind::Reasoning,
				});
				sink(&Event::PartEnd {
					part: PartId(part),
					value: Final::Reasoning {
						text: text.as_deref().map(|text| JsonStr::new(text).unwrap()),
						encrypted: None,
						item: &item,
					},
				});
			},
			CallStart(part, call_id, name) => sink(&Event::PartStart {
				part: PartId(part),
				kind: PartKind::ToolCall { call_id, name },
			}),
			CallEnd(part, arguments, complete) => {
				let arguments = string(arguments);
				let arguments = JsonStr::new(&arguments).unwrap();
				let arguments = if complete {
					Arguments::complete(arguments).unwrap()
				} else {
					Arguments::Partial(arguments)
				};
				sink(&Event::PartEnd {
					part: PartId(part),
					value: Final::ToolCall(arguments),
				});
			},
			Used => sink(&Event::Usage(USAGE)),
			Finish(outcome) => sink(&Event::Finish(outcome)),
			Error(kind) => sink(&Event::Error(ProviderError {
				kind,
				code: None,
				message: None,
				client_message: None,
			})),
		}
	}
}

struct Streamed {
	/// The data of each event.
	events: Vec<String>,
	early_failure: Option<HttpReply>,
}

fn streamed(steps: &[Step], include_usage: bool) -> Streamed {
	streamed_with(steps, include_usage, &[], usize::MAX)
}

fn streamed_with(
	steps: &[Step],
	include_usage: bool,
	routes: &[ToolRoute],
	limit: usize,
) -> Streamed {
	let mut writer = ChatStream::new(SCOPE, routes, 7, limit).include_usage(include_usage);
	feed(steps, &mut |event| writer.event(event));
	let out = String::from_utf8(writer.take_output()).unwrap();
	Streamed {
		events: out
			.split_terminator("\n\n")
			.map(|event| event.strip_prefix("data: ").unwrap().to_string())
			.collect(),
		early_failure: writer.early_failure(),
	}
}

fn buffered(steps: &[Step]) -> HttpReply {
	let mut writer = BufferedChat::new(SCOPE, &[], 7, usize::MAX);
	feed(steps, &mut |event| writer.event(event));
	writer.into_reply()
}

fn body(reply: &HttpReply) -> Value {
	serde_json::from_slice(&reply.body).unwrap()
}

/// Each chunk's delta and finish reason, or what else the event is.
fn deltas(streamed: &Streamed) -> Vec<String> {
	streamed
		.events
		.iter()
		.map(|data| {
			if data == "[DONE]" {
				return data.clone();
			}
			let chunk: Value = serde_json::from_str(data).unwrap();
			if let Some(error) = chunk.get("error") {
				return format!("error {}", error["code"]);
			}
			match chunk["choices"].as_array().unwrap().as_slice() {
				[] => format!("usage {}", chunk["usage"]),
				[choice] => match choice["finish_reason"].as_str() {
					Some(reason) => format!("finish {reason}"),
					None => choice["delta"].to_string(),
				},
				_ => panic!("more than one choice: {data}"),
			}
		})
		.collect()
}

fn hello() -> Vec<Step> {
	vec![
		Text(0),
		Delta(0, "Hel"),
		Delta(0, "lo \"x\""),
		TextEnd(0),
		Used,
		Finish(DONE),
	]
}

#[test]
fn text_streams_as_it_comes_and_ends_with_done() {
	let streamed = streamed(&hello(), false);
	assert_eq!(
		deltas(&streamed),
		[
			r#"{"role":"assistant","content":""}"#,
			r#"{"content":"Hel"}"#,
			r#"{"content":"lo \"x\""}"#,
			"finish stop",
			"[DONE]"
		]
	);
	let first: Value = serde_json::from_str(&streamed.events[0]).unwrap();
	assert_eq!(first["id"], "chatcmpl-r1");
	assert_eq!(first["object"], "chat.completion.chunk");
	assert_eq!(first["created"], 7);
	assert_eq!(first["model"], "m1");
}

#[test]
fn usage_comes_last_only_for_a_client_that_asked() {
	let streamed = streamed(&hello(), true);
	let deltas = deltas(&streamed);
	assert_eq!(
		deltas[3..],
		[
			"finish stop".to_string(),
			format!(
				"usage {}",
				json!({"prompt_tokens": 30, "completion_tokens": 5, "total_tokens": 35,
					"prompt_tokens_details": {"cached_tokens": 20},
					"completion_tokens_details": {"reasoning_tokens": 2}})
			),
			"[DONE]".to_string()
		]
	);
}

#[test]
fn a_whole_reply_is_one_completion() {
	let reply = buffered(&hello());
	assert_eq!(reply.status, 200);
	assert_eq!(
		body(&reply),
		json!({"id": "chatcmpl-r1", "object": "chat.completion", "created": 7, "model": "m1",
			"choices": [{"index": 0, "message": {"role": "assistant",
				"content": "Hello \"x\"", "refusal": null},
				"logprobs": null, "finish_reason": "stop"}],
			"usage": {"prompt_tokens": 30, "completion_tokens": 5, "total_tokens": 35,
				"prompt_tokens_details": {"cached_tokens": 20},
				"completion_tokens_details": {"reasoning_tokens": 2}}})
	);
}

#[test]
fn reasoning_text_is_reasoning_content_and_its_item_rides_in_the_next_call_id() {
	let steps = [
		Reasoning(0, THINKING, Some("Plan.")),
		Text(1),
		Delta(1, "Checking."),
		TextEnd(1),
		CallStart(2, "toolu_1", "f"),
		CallEnd(2, r#"{"city":"Oslo"}"#, true),
		Finish(DONE),
	];
	let reply = body(&buffered(&steps));
	let message = &reply["choices"][0]["message"];
	assert_eq!(message["reasoning_content"], "Plan.");
	assert_eq!(message["content"], "Checking.");
	assert_eq!(reply["choices"][0]["finish_reason"], "tool_calls");
	let call = &message["tool_calls"][0];
	assert_eq!(
		call["function"],
		json!({"name": "f", "arguments": "{\"city\":\"Oslo\"}"})
	);
	let (id, carrier) = split_call_id(call["id"].as_str().unwrap());
	assert_eq!(id, "toolu_1");
	let items = open_call_id(SCOPE, id, carrier.unwrap()).unwrap();
	assert_eq!(items[0].text_offset, 0);
	assert_eq!(items[0].item, THINKING.as_bytes());

	let streamed = streamed(&steps, false);
	let deltas = deltas(&streamed);
	assert_eq!(deltas[1], r#"{"reasoning_content":"Plan."}"#);
	assert_eq!(deltas[2], r#"{"content":"Checking."}"#);
	let chunk: Value = serde_json::from_str(&streamed.events[3]).unwrap();
	let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
	assert_eq!(call["index"], 0);
	assert_eq!(call["id"], message["tool_calls"][0]["id"]);
}

#[test]
fn a_call_id_records_reasoning_after_visible_text() {
	let steps = [
		Text(0),
		Delta(0, "Checking."),
		TextEnd(0),
		Reasoning(1, THINKING, None),
		CallStart(2, "toolu_1", "f"),
		CallEnd(2, "{}", true),
		Finish(DONE),
	];
	let reply = body(&buffered(&steps));
	let id = reply["choices"][0]["message"]["tool_calls"][0]["id"]
		.as_str()
		.unwrap();
	let (id, carrier) = split_call_id(id);
	let items = open_call_id(SCOPE, id, carrier.unwrap()).unwrap();
	assert_eq!(items[0].text_offset, "Checking.".len());
	assert_eq!(items[0].item, THINKING.as_bytes());
}

#[test]
fn each_call_contains_the_reasoning_since_the_one_before() {
	let steps = [
		Reasoning(0, r#"{"n":1}"#, None),
		CallStart(1, "a", "f"),
		CallEnd(1, "", true),
		CallStart(2, "b", "f"),
		CallEnd(2, "{}", true),
		Reasoning(3, r#"{"n":3}"#, None),
		CallStart(4, "c", "f"),
		CallEnd(4, "{}", true),
		Reasoning(5, r#"{"n":5}"#, None),
		Finish(DONE),
	];
	let reply = body(&buffered(&steps));
	let calls = reply["choices"][0]["message"]["tool_calls"]
		.as_array()
		.unwrap();
	let carried: Vec<(String, Vec<String>)> = calls
		.iter()
		.map(|call| {
			let (id, carrier) = split_call_id(call["id"].as_str().unwrap());
			let items = carrier.map_or(Vec::new(), |carrier| {
				let items = open_call_id(SCOPE, id, carrier).unwrap();
				items
					.into_iter()
					.map(|item| String::from_utf8(item.item).unwrap())
					.collect()
			});
			(id.to_string(), items)
		})
		.collect();
	assert_eq!(
		carried,
		[
			("a".to_string(), vec![r#"{"n":1}"#.to_string()]),
			("b".to_string(), vec![]),
			("c".to_string(), vec![r#"{"n":3}"#.to_string()]),
		]
	);
	assert_eq!(
		calls[0]["function"]["arguments"], "{}",
		"no arguments are an empty object"
	);
}

#[test]
fn parallel_calls_are_numbered_and_restore_their_names() {
	let routes = [ToolRoute {
		flat: "ns__f".into(),
		namespace: None,
		name: "f".into(),
		kind: CallKind::Function,
	}];
	let steps = [
		CallStart(0, "a", "ns__f"),
		CallStart(1, "b", "g"),
		CallEnd(1, "{\"y\":2}", true),
		CallEnd(0, "{\"x\":1}", true),
		Finish(DONE),
	];
	let streamed = streamed_with(&steps, false, &routes, usize::MAX);
	let calls: Vec<Value> = streamed.events[1..3]
		.iter()
		.map(|data| {
			let chunk: Value = serde_json::from_str(data).unwrap();
			chunk["choices"][0]["delta"]["tool_calls"][0].clone()
		})
		.collect();
	assert_eq!(
		calls,
		[
			json!({"index": 0, "id": "a", "type": "function",
				"function": {"name": "f", "arguments": "{\"x\":1}"}}),
			json!({"index": 1, "id": "b", "type": "function",
				"function": {"name": "g", "arguments": "{\"y\":2}"}}),
		]
	);
	assert_eq!(deltas(&streamed)[3..], ["finish tool_calls", "[DONE]"]);
}

#[test]
fn a_later_part_waits_and_goes_out_whole() {
	let steps = [
		Text(0),
		Delta(0, "first "),
		Refusal(1, "no"),
		Text(2),
		Delta(2, "third"),
		Delta(0, "still first"),
		TextEnd(0),
		Delta(2, " more"),
		TextEnd(2),
		Finish(DONE),
	];
	let streamed = streamed(&steps, false);
	assert_eq!(
		deltas(&streamed)[1..],
		[
			r#"{"content":"first "}"#,
			r#"{"content":"still first"}"#,
			r#"{"refusal":"no"}"#,
			r#"{"content":"third more"}"#,
			"finish stop",
			"[DONE]"
		]
	);
	let message = &body(&buffered(&steps))["choices"][0]["message"];
	assert_eq!(message["content"], "first still firstthird more");
	assert_eq!(message["refusal"], "no");
}

#[test]
fn a_call_cut_off_by_the_limit_is_left_out() {
	let steps = [
		Text(0),
		Delta(0, "Calling."),
		TextEnd(0),
		Reasoning(1, r#"{"n":1}"#, None),
		CallStart(2, "a", "f"),
		CallEnd(2, "{\"x\":", false),
		Finish(CUT),
	];
	let message = body(&buffered(&steps));
	assert_eq!(message["choices"][0]["finish_reason"], "length");
	assert_eq!(message["choices"][0]["message"].get("tool_calls"), None);
	assert_eq!(
		deltas(&streamed(&steps, false))[2..],
		["finish length", "[DONE]"]
	);
}

#[test]
fn a_cut_off_call_in_a_completed_reply_fails_it() {
	let steps = [
		CallStart(0, "a", "f"),
		CallEnd(0, "{\"x\":", false),
		Finish(DONE),
	];
	let streamed = streamed(&steps, false);
	assert!(streamed.events.is_empty());
	assert_eq!(streamed.early_failure.unwrap().status, 500);
	assert_eq!(buffered(&steps).status, 500);
}

#[test]
fn a_failure_before_output_is_an_http_error_and_nothing_else() {
	let streamed = streamed(&[Error(ErrorKind::RateLimit)], false);
	assert!(streamed.events.is_empty());
	let reply = streamed.early_failure.unwrap();
	assert_eq!(reply.status, 429);
	assert_eq!(
		body(&reply),
		json!({"error": {"message": "The provider's rate limit was reached.",
			"type": "rate_limit_error", "param": null, "code": "rate_limit_exceeded"}})
	);
}

#[test]
fn a_failure_after_output_ends_the_stream_without_done() {
	let streamed = streamed(
		&[Text(0), Delta(0, "Hel"), Error(ErrorKind::Overloaded), Used],
		false,
	);
	assert_eq!(
		deltas(&streamed)[1..],
		[r#"{"content":"Hel"}"#, r#"error "server_is_overloaded""#]
	);
	assert_eq!(streamed.early_failure, None);
}

#[test]
fn running_out_of_context_is_a_client_error() {
	let steps = [Finish(Outcome::Incomplete {
		reason: StopReason::ContextWindow,
		raw: None,
	})];
	let reply = buffered(&steps);
	assert_eq!(reply.status, 400);
	assert_eq!(body(&reply)["error"]["code"], "context_length_exceeded");
}

#[test]
fn arguments_that_arent_an_object_fail_the_reply() {
	let reply = buffered(&[
		CallStart(0, "a", "f"),
		CallEnd(0, "[1]", true),
		Finish(DONE),
	]);
	assert_eq!(reply.status, 500);
}

#[test]
fn too_much_waiting_output_fails_the_reply() {
	let steps = [Text(0), Text(1), Delta(1, "0123456789"), Delta(0, "a")];
	let streamed = streamed_with(&steps, false, &[], 8);
	assert_eq!(streamed.early_failure.unwrap().status, 500);
	let streamed = streamed_with(&steps, false, &[], 64);
	assert_eq!(streamed.early_failure, None);
}

#[test]
fn a_reply_that_never_ends_is_a_server_error() {
	let reply = buffered(&[Text(0), Delta(0, "Hel")]);
	assert_eq!(reply.status, 500);
	assert_eq!(body(&reply)["error"]["type"], "server_error");
}

#[test]
fn a_refused_field_is_named_in_the_clients_error() {
	let reply = error_reply_naming(400, ErrorKind::InvalidRequest, Some("temperature"));
	assert_eq!(reply.status, 400);
	assert_eq!(
		body(&reply)["error"],
		json!({"message": "The provider refused `temperature`.",
			"type": "invalid_request_error", "param": null, "code": "invalid_prompt"})
	);
	assert_eq!(error_reply(418, ErrorKind::Unknown).status, 500);
}
