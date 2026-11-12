#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};

use super::*;
use crate::protocols::responses::decoder::ResponsesDecoder;

/// Restores `<T0>`-style tokens, keeping back a `<` that may start one.
struct Restore {
	tokens: &'static [(&'static str, &'static str)],
	kept: String,
}

fn restore(tokens: &'static [(&'static str, &'static str)]) -> Box<dyn ReplyRewrite> {
	Box::new(Restore {
		tokens,
		kept: String::new(),
	})
}

impl ReplyRewrite for Restore {
	fn text(&mut self, _: ReplyPlace, text: &str, out: &mut String) {
		let mut text = std::mem::take(&mut self.kept) + text;
		for (token, value) in self.tokens {
			text = text.replace(token, value);
		}
		let keep = text.rfind('<').filter(|at| !text[*at..].contains('>'));
		let keep = keep.unwrap_or(text.len());
		out.push_str(&text[..keep]);
		self.kept = text[keep..].to_string();
	}

	fn finish(&mut self, _: ReplyPlace, out: &mut String) {
		out.push_str(&std::mem::take(&mut self.kept));
	}
}

struct Rename;

impl ReplyRewrite for Rename {
	fn call(&mut self, call: &mut ToolCall) {
		call.name = format!("{}_v2", call.name);
	}
}

#[derive(Debug, Default, PartialEq)]
struct Seen {
	text: String,
	calls: Vec<(String, String)>,
	call_parts: Vec<PartId>,
	open: usize,
	finish: Option<String>,
	output_tokens: Option<u64>,
}

impl Seen {
	fn event(&mut self, event: Event<'_>) {
		match event {
			Event::PartStart {
				part,
				kind: PartKind::ToolCall { name, .. },
			} => {
				self.open += 1;
				self.call_parts.push(part);
				self.calls.push((name.to_string(), String::new()));
			},
			Event::PartStart { .. } => self.open += 1,
			Event::Delta { part, text } if !self.call_parts.contains(&part) => {
				self.text.push_str(&text.decode())
			},
			Event::PartEnd { value, .. } => {
				self.open -= 1;
				if let Final::ToolCall(arguments) = value {
					assert!(arguments.is_complete());
					self.calls.last_mut().unwrap().1 = decode_string(arguments.raw()).into_owned();
				}
			},
			Event::Finish(outcome) => self.finish = Some(format!("{outcome:?}")),
			Event::Usage(usage) => self.output_tokens = usage.output_tokens,
			_ => {},
		}
	}
}

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

fn call(index: usize, name: &str, deltas: &[&str]) -> Vec<Value> {
	let item = |arguments: &str, status: &str| {
		json!({"type": "function_call", "id": format!("fc{index}"), "call_id": format!("c{index}"),
			"name": name, "arguments": arguments, "status": status})
	};
	let arguments = deltas.concat();
	let mut events = vec![
		json!({"type": "response.output_item.added", "output_index": index,
		"item": item("", "in_progress")}),
	];
	events.extend(deltas.iter().map(|delta| {
		json!({"type": "response.function_call_arguments.delta", "output_index": index,
			"delta": delta})
	}));
	events.push(json!({"type": "response.function_call_arguments.done",
		"output_index": index, "arguments": arguments}));
	events.push(
		json!({"type": "response.output_item.done", "output_index": index,
		"item": item(&arguments, "completed")}),
	);
	events
}

fn reply(parts: Vec<Vec<Value>>) -> Vec<Value> {
	let mut events = vec![json!({"type": "response.created", "response": {"id": "r", "model": "m"}})];
	events.extend(parts.into_iter().flatten());
	events.push(
		json!({"type": "response.completed", "response": {"id": "r", "model": "m",
		"status": "completed", "output": [],
		"usage": {"input_tokens": 3, "output_tokens": 7, "total_tokens": 10}}}),
	);
	events
}

#[test]
fn with_no_rewrites_events_pass_through() {
	let events = reply(vec![answer(0, &["Hi <T", "0>"]), call(1, "f", &["{}"])]);
	let seen = run(Vec::new(), &events);
	assert_eq!(seen.text, "Hi <T0>");
	assert_eq!(seen.calls, [("f".to_string(), "{}".to_string())]);
}

#[test]
fn a_token_split_across_deltas_is_restored() {
	let events = reply(vec![answer(0, &["Mail <T", "0", "> now, <"])]);
	let seen = run(vec![restore(&[("<T0>", "a@b.com")])], &events);
	assert_eq!(seen.text, "Mail a@b.com now, <");
	assert_eq!(seen.open, 0);
	assert_eq!(seen.finish.as_deref(), Some("Completed"));
}

#[test]
fn a_restored_value_is_escaped_for_tool_call_arguments() {
	let events = reply(vec![call(0, "send", &[r#"{"to":"<T"#, r#"1>","n":1}"#])]);
	let seen = run(
		vec![restore(&[("<T1>", r#"say "hi""#)]), Box::new(Rename)],
		&events,
	);
	let (name, arguments) = &seen.calls[0];
	assert_eq!(name, "send_v2");
	assert_eq!(arguments, r#"{"to":"say \"hi\"","n":1}"#);
	assert_eq!(
		serde_json::from_str::<Value>(arguments).unwrap(),
		json!({"to": "say \"hi\"", "n": 1})
	);
}

fn run(rewrites: Vec<Box<dyn ReplyRewrite>>, events: &[Value]) -> Seen {
	let mut decoder = ResponsesDecoder::default();
	let mut rewrites = ReplyRewrites::new(rewrites);
	let mut seen = Seen::default();
	for event in events {
		decoder
			.push_json(event.to_string().as_bytes(), &mut |event| {
				rewrites.event(event, &mut |event| seen.event(event));
			})
			.unwrap();
	}
	decoder.finish().unwrap();
	seen
}

struct Unchanged;

impl ReplyRewrite for Unchanged {}

#[test]
fn text_a_rewrite_leaves_unchanged_keeps_the_provider_s_escapes() {
	let mut decoder = ResponsesDecoder::default();
	let mut rewrites = ReplyRewrites::new(vec![Box::new(Unchanged)]);
	let mut raw = Vec::new();
	// The provider escapes the accent; the decoder's checks see the same text.
	let mut events: Vec<String> = reply(vec![answer(0, &["café"])])
		.iter()
		.map(Value::to_string)
		.collect();
	events[3] = r#"{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"caf\u00e9"}"#.to_string();
	for event in &events {
		decoder
			.push_json(event.as_bytes(), &mut |event| {
				rewrites.event(event, &mut |event| {
					if let Event::Delta { text, .. } = event {
						raw.push(text.raw().to_string());
					}
				});
			})
			.unwrap();
	}
	assert_eq!(raw, [r#""caf\u00e9""#]);
}
