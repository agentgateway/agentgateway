#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use rstest::rstest;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::*;
use crate::ir::events::{Arguments, JsonStr};
use crate::protocols::responses::decoder::ResponsesDecoder;

#[derive(Debug, Default, PartialEq)]
struct Seen {
	text: String,
	calls: Vec<String>,
	parts_open: usize,
	stop: Option<String>,
	completed: bool,
	output_tokens: Option<u64>,
}

impl Seen {
	fn event(&mut self, event: Event<'_>) {
		match event {
			Event::Delta { text, .. } => self.text.push_str(&text.decode()),
			Event::PartStart { kind, .. } => {
				self.parts_open += 1;
				if let PartKind::ToolCall { name, .. } = kind {
					self.calls.push(name.to_string());
				}
			},
			Event::PartEnd { .. } => self.parts_open -= 1,
			Event::StopSequence(stop) => self.stop = Some(stop.to_string()),
			Event::Finish(outcome) => self.completed = matches!(outcome, Outcome::Completed),
			Event::Usage(usage) => self.output_tokens = usage.output_tokens,
			_ => {},
		}
	}
}

fn stream(stops: &[&str], events: &[Value]) -> Seen {
	let mut decoder = ResponsesDecoder::default();
	let mut stop = stop_at(stops);
	let mut seen = Seen::default();
	for event in events {
		decoder
			.push_json(event.to_string().as_bytes(), &mut |event| {
				stop.event(event, &mut |event| seen.event(event));
			})
			.unwrap();
	}
	decoder.finish().unwrap();
	seen
}

/// One answer streamed as `deltas`, at output `index`.
fn answer(index: usize, deltas: &[&str]) -> Vec<Value> {
	let message = |text: &str, status: &str| {
		json!({"type": "message", "id": format!("m{index}"), "role": "assistant",
			"status": status,
			"content": [{"type": "output_text", "text": text, "annotations": []}]})
	};
	let text = deltas.concat();
	let mut events = vec![
		json!({"type": "response.output_item.added", "output_index": index,
			"item": message("", "in_progress")}),
		json!({"type": "response.content_part.added", "output_index": index,
			"content_index": 0, "part": {"type": "output_text", "text": "", "annotations": []}}),
	];
	events.extend(deltas.iter().map(|delta| {
		json!({"type": "response.output_text.delta", "output_index": index,
			"content_index": 0, "delta": delta})
	}));
	events.push(
		json!({"type": "response.output_text.done", "output_index": index,
		"content_index": 0, "text": text}),
	);
	events.push(
		json!({"type": "response.output_item.done", "output_index": index,
		"item": message(&text, "completed")}),
	);
	events
}

fn started() -> Value {
	json!({"type": "response.created", "response": {"id": "r", "model": "m"}})
}

fn ended(kind: &str, extra: &Value) -> Value {
	let mut response = json!({"id": "r", "model": "m", "output": [],
		"usage": {"input_tokens": 3, "output_tokens": 7, "total_tokens": 10}});
	if let (Some(response), Some(extra)) = (response.as_object_mut(), extra.as_object()) {
		response.extend(extra.clone());
	}
	json!({"type": kind, "response": response})
}

fn completed() -> Value {
	ended("response.completed", &json!({"status": "completed"}))
}

fn one_answer(stops: &[&str], deltas: &[&str], end: Value) -> Seen {
	let mut events = vec![started()];
	events.extend(answer(0, deltas));
	events.push(end);
	stream(stops, &events)
}

fn stop_at(stops: &[&str]) -> StopAt {
	StopAt::new(stops.iter().map(|stop| stop.to_string()).collect())
}

#[test]
fn a_stop_split_across_chunks_ends_the_answer() {
	let incomplete = ended(
		"response.incomplete",
		&json!({"status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"}}),
	);
	let seen = one_answer(
		&["</block>"],
		&["Verdict: <bl", "ock>no</bl", "ock> and more"],
		incomplete,
	);
	assert_eq!(seen.text, "Verdict: <block>no");
	assert_eq!(seen.stop.as_deref(), Some("</block>"));
	assert_eq!(seen.parts_open, 0);
	assert!(seen.completed, "{seen:?}");
	assert_eq!(seen.output_tokens, Some(7));
}

proptest! {
	#![proptest_config(ProptestConfig::with_cases(128))]

	#[test]
	fn stop_matching_does_not_depend_on_text_splits(
		text in r"[a-c →\n]{0,96}", stop in r"[a-c →\n]{0,8}",
		cuts in prop::collection::vec(any::<usize>(), 0..24),
	) {
		let chunks = crate::test_support::text_chunks(&text, &cuts);
		let seen = one_answer(&[&stop], &chunks, completed());
		let at = (!stop.is_empty()).then(|| text.find(&stop)).flatten();
		prop_assert_eq!(seen.text, text[..at.unwrap_or(text.len())].to_string());
		prop_assert_eq!(seen.stop, at.map(|_| stop));
		prop_assert_eq!(seen.parts_open, 0);
		prop_assert_eq!(seen.output_tokens, Some(7));
	}
}

#[test]
fn nothing_after_a_stop_reaches_the_client() {
	let call = |status: &str, arguments: &str| {
		json!({"type": "function_call", "id": "fc", "call_id": "call_1", "name": "run",
			"arguments": arguments, "status": status})
	};
	let mut events = vec![started()];
	events.extend(answer(0, &["x END y"]));
	events.extend([
		json!({"type": "response.output_item.added", "output_index": 1,
			"item": call("in_progress", "")}),
		json!({"type": "response.function_call_arguments.delta", "output_index": 1,
			"item_id": "fc", "delta": "{}"}),
		json!({"type": "response.function_call_arguments.done", "output_index": 1,
			"item_id": "fc", "arguments": "{}"}),
		json!({"type": "response.output_item.done", "output_index": 1,
			"item": call("completed", "{}")}),
	]);
	events.extend(answer(2, &["more"]));
	events.push(completed());
	let seen = stream(&["END"], &events);
	assert_eq!(seen.text, "x ");
	assert!(seen.calls.is_empty(), "{seen:?}");
	assert_eq!(seen.parts_open, 0);
	assert_eq!(seen.stop.as_deref(), Some("END"));
	assert!(seen.completed, "{seen:?}");
	assert_eq!(seen.output_tokens, Some(7));
}

#[test]
fn a_part_open_when_the_stop_matches_still_ends() {
	let call = RawValue::from_string(r#""{\"a\":1}""#.to_string()).unwrap();
	let text = RawValue::from_string(r#""x END y""#.to_string()).unwrap();
	let call = JsonStr::new(&call).unwrap();
	let mut stop = stop_at(&["END"]);
	let mut seen = Seen::default();
	for event in [
		Event::PartStart {
			part: PartId(0),
			kind: PartKind::ToolCall {
				call_id: "call_1",
				name: "run",
			},
		},
		Event::PartStart {
			part: PartId(1),
			kind: PartKind::Text,
		},
		Event::Delta {
			part: PartId(1),
			text: JsonStr::new(&text).unwrap(),
		},
		Event::Delta {
			part: PartId(0),
			text: call,
		},
		Event::PartEnd {
			part: PartId(0),
			value: Final::ToolCall(Arguments::complete(call).unwrap()),
		},
		Event::PartEnd {
			part: PartId(1),
			value: Final::Text,
		},
		Event::Finish(Outcome::Completed),
	] {
		stop.event(event, &mut |event| seen.event(event));
	}
	assert_eq!(seen.text, r#"x {"a":1}"#);
	assert_eq!(seen.calls, ["run"]);
	assert_eq!(seen.parts_open, 0);
	assert_eq!(seen.stop.as_deref(), Some("END"));
}

#[rstest]
#[case::independent_text("X", "OP more", &[(0, "a "), (1, "X")], true)]
#[case::no_match_across_parts("OP b", "uff", &[(0, "a "), (1, "OP b"), (0, "STuff")], false)]
#[case::flush_other_prefix("ST", "OP more", &[(0, "a "), (1, "ST")], true)]
fn overlapping_text_keeps_prefixes_with_their_parts(
	#[case] other: &str,
	#[case] remaining: &str,
	#[case] expected: &[(u32, &str)],
	#[case] matched: bool,
) {
	let first = string_value("a ST").unwrap();
	let other = string_value(other).unwrap();
	let remaining = string_value(remaining).unwrap();
	let mut stop = stop_at(&["STOP"]);
	let mut seen = Seen::default();
	let mut deltas = Vec::new();
	for event in [
		Event::PartStart {
			part: PartId(0),
			kind: PartKind::Text,
		},
		Event::Delta {
			part: PartId(0),
			text: JsonStr::new(&first).unwrap(),
		},
		Event::PartStart {
			part: PartId(1),
			kind: PartKind::Text,
		},
		Event::Delta {
			part: PartId(1),
			text: JsonStr::new(&other).unwrap(),
		},
		Event::Delta {
			part: PartId(0),
			text: JsonStr::new(&remaining).unwrap(),
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
	] {
		stop.event(event, &mut |event| {
			if let Event::Delta { part, text } = event {
				deltas.push((part.0, text.decode().into_owned()));
			}
			seen.event(event);
		});
	}
	assert_eq!(
		deltas,
		expected
			.iter()
			.map(|&(id, text)| (id, text.to_string()))
			.collect::<Vec<_>>()
	);
	assert_eq!(seen.stop.as_deref(), matched.then_some("STOP"));
	assert_eq!(seen.parts_open, 0);
	assert!(seen.completed);
}

#[test]
fn overlapping_provider_text_is_written_to_the_right_blocks() {
	use crate::protocols::messages::writer::{ReplyWriter, Sequencer};
	use crate::test_support::SCOPE;

	let mut first = answer(0, &["a ST", "OP more"]).into_iter();
	let mut second = answer(1, &["X"]).into_iter();
	let mut events = vec![started()];
	events.extend(first.by_ref().take(3));
	events.extend(second.by_ref().take(3));
	events.extend(first);
	events.extend(second);
	events.push(completed());
	let mut decoder = ResponsesDecoder::default();
	let mut stop = stop_at(&["STOP"]);
	let mut seen = Seen::default();
	let mut sequencer = Sequencer::new(SCOPE, 1024);
	let mut writer = ReplyWriter::default();
	for event in events {
		decoder
			.push_json(event.to_string().as_bytes(), &mut |event| {
				stop.event(event, &mut |event| {
					seen.event(event);
					sequencer.event(&event, &mut writer);
				});
			})
			.unwrap();
	}
	decoder.finish().unwrap();
	let reply = writer.finish();
	assert_eq!(reply.status, 200);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(body["content"][0]["text"], "a ");
	assert_eq!(body["content"][1]["text"], "X");
	assert_eq!(body["content"].as_array().unwrap().len(), 2);
	assert_eq!(body["stop_reason"], "stop_sequence");
	assert_eq!(seen.parts_open, 0);
	assert_eq!(seen.output_tokens, Some(7));
}

#[test]
fn a_streamed_response_stops_and_its_checks_still_pass() {
	let message = |text: &str, status: &str| {
		json!({"type": "message", "id": "m", "role": "assistant", "status": status,
			"content": [{"type": "output_text", "text": text, "annotations": []}]})
	};
	let events = [
		json!({"type": "response.created", "response": {"id": "r", "model": "m"}}),
		json!({"type": "response.output_item.added", "output_index": 0,
			"item": message("", "in_progress")}),
		json!({"type": "response.content_part.added", "output_index": 0, "content_index": 0,
			"part": {"type": "output_text", "text": "", "annotations": []}}),
		json!({"type": "response.output_text.delta", "output_index": 0, "content_index": 0,
			"delta": "<severity>1</sev"}),
		json!({"type": "response.output_text.delta", "output_index": 0, "content_index": 0,
			"delta": "erity> because"}),
		json!({"type": "response.output_text.done", "output_index": 0, "content_index": 0,
			"text": "<severity>1</severity> because"}),
		json!({"type": "response.output_item.done", "output_index": 0,
			"item": message("<severity>1</severity> because", "completed")}),
		json!({"type": "response.incomplete", "response": {"id": "r", "model": "m",
			"output": [], "incomplete_details": {"reason": "max_output_tokens"},
			"usage": {"input_tokens": 3, "output_tokens": 7, "total_tokens": 10}}}),
	];
	let seen = stream(&["</severity>"], &events);
	assert_eq!(seen.text, "<severity>1");
	assert_eq!(seen.stop.as_deref(), Some("</severity>"));
	assert_eq!(seen.parts_open, 0);
	assert!(seen.completed, "{seen:?}");
	assert_eq!(seen.output_tokens, Some(7));
}
