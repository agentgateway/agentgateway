#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

use super::*;
use crate::protocols::messages::decoder::MessagesDecoder;

const INCOMPLETE: &str = "The incomplete tool call could not be checked.";

fn config(window_bytes: usize, overlap_bytes: usize) -> OutputChecks {
	OutputChecks {
		window_bytes,
		overlap_bytes,
		max_held_bytes: 1 << 20,
		incomplete_call: IncompleteCall::Stop(INCOMPLETE.into()),
	}
}

/// What the client was sent.
#[derive(Debug, Default)]
struct Seen {
	text: String,
	calls: Vec<String>,
	call_parts: Vec<PartId>,
	finish: Option<String>,
	/// The message of the error that ended the reply.
	error: Option<String>,
}

impl Seen {
	fn event(&mut self, event: Event<'_>) {
		match event {
			Event::PartStart {
				part,
				kind: PartKind::ToolCall { .. },
			} => self.call_parts.push(part),
			Event::Delta { part, text } if !self.call_parts.contains(&part) => {
				self.text.push_str(&text.decode())
			},
			Event::PartEnd {
				value: Final::ToolCall(arguments),
				..
			} => self.calls.push(decode_string(arguments.raw()).into_owned()),
			Event::Finish(outcome) => self.finish = Some(format!("{outcome:?}")),
			Event::Error(error) => self.error = error.client_message.map(str::to_string),
			_ => {},
		}
	}
}

fn text_reply(texts: &[&str]) -> Vec<Value> {
	let deltas = texts.iter().map(|text| {
		json!({"type": "content_block_delta", "index": 0,
			"delta": {"type": "text_delta", "text": text}})
	});
	messages_reply(
		&json!({"type": "text", "text": ""}),
		deltas.collect(),
		"end_turn",
	)
}

fn call_reply(arguments: &[&str]) -> Vec<Value> {
	let deltas = arguments.iter().map(|arguments| {
		json!({"type": "content_block_delta", "index": 0,
			"delta": {"type": "input_json_delta", "partial_json": arguments}})
	});
	messages_reply(
		&json!({"type": "tool_use", "id": "c", "name": "search", "input": {}}),
		deltas.collect(),
		"tool_use",
	)
}

fn messages_reply(block: &Value, deltas: Vec<Value>, stop_reason: &str) -> Vec<Value> {
	let mut events = vec![
		json!({"type": "message_start", "message": {"id": "m", "type": "message",
			"role": "assistant", "model": "claude", "content": [],
			"usage": {"input_tokens": 3, "output_tokens": 1}}}),
		json!({"type": "content_block_start", "index": 0, "content_block": block}),
	];
	events.extend(deltas);
	events.extend([
		json!({"type": "content_block_stop", "index": 0}),
		json!({"type": "message_delta", "delta": {"stop_reason": stop_reason},
			"usage": {"output_tokens": 5}}),
		json!({"type": "message_stop"}),
	]);
	events
}

/// Feeds every event without answering any check.
fn feed(checks: &mut Checks, events: &[Value], seen: &mut Seen) {
	let mut decoder = MessagesDecoder::default();
	for event in events {
		decoder
			.push_json(event.to_string().as_bytes(), usize::MAX, &mut |event| {
				checks.event(event, &mut |event| seen.event(event))
			})
			.unwrap();
	}
}

/// What the client is sent, and the text of each check. Every check is answered by
/// `judge` after the provider event that asked for it, as the gateway answers after
/// each chunk.
fn run(
	config: OutputChecks,
	events: &[Value],
	mut judge: impl FnMut(&str) -> Verdict,
) -> (Seen, Vec<String>) {
	let mut checks = Checks::new(config);
	let mut decoder = MessagesDecoder::default();
	let mut seen = Seen::default();
	let mut asked = Vec::new();
	for event in events {
		decoder
			.push_json(event.to_string().as_bytes(), usize::MAX, &mut |event| {
				checks.event(event, &mut |event| seen.event(event))
			})
			.unwrap();
		while let Some(check) = checks.waiting() {
			let text = check.text.to_string();
			let verdict = judge(&text);
			asked.push(text);
			checks.resolve(verdict, &mut |event| seen.event(event));
		}
	}
	(seen, asked)
}

fn blocking(blocked: &'static str) -> impl FnMut(&str) -> Verdict {
	move |text| {
		if text.contains(blocked) {
			Verdict::Stop("Blocked.".into())
		} else {
			Verdict::Release
		}
	}
}

fn checked_call(name: &str, arguments: &str) -> String {
	json!({ "name": name, "arguments": arguments }).to_string()
}

#[test]
fn text_goes_out_a_judged_window_at_a_time() {
	let events = text_reply(&["Hello, ", "world! ", "How are ", "you?"]);
	let (seen, asked) = run(config(10, 0), &events, blocking("never"));
	assert_eq!(asked, ["Hello, world! ", "How are you?"]);
	assert_eq!(seen.text, "Hello, world! How are you?");
	assert_eq!(seen.finish.as_deref(), Some("Completed"));
}

#[test]
fn a_part_s_last_window_is_judged_at_its_end() {
	let (seen, asked) = run(
		config(10, 0),
		&text_reply(&["Hi", " there"]),
		blocking("never"),
	);
	assert_eq!(asked, ["Hi there"]);
	assert_eq!(seen.text, "Hi there");
}

#[test]
fn a_stopped_window_is_never_sent_and_the_reply_ends_with_its_error() {
	let events = text_reply(&["Fine text. ", "secret ", "stuff"]);
	let (seen, asked) = run(config(10, 0), &events, blocking("secret"));
	assert_eq!(asked, ["Fine text. ", "secret stuff"]);
	assert_eq!(seen.text, "Fine text. ");
	assert_eq!(seen.error.as_deref(), Some("Blocked."));
	assert_eq!(seen.finish, None);
}

#[test]
fn overlap_finds_text_split_across_windows() {
	let events = text_reply(&["0123456789", "secret"]);
	let (seen, asked) = run(config(10, 2), &events, blocking("89se"));
	assert_eq!(asked, ["0123456789", "89secret"]);
	assert_eq!(seen.text, "0123456789");
	assert_eq!(seen.error.as_deref(), Some("Blocked."));
}

#[test]
fn output_waits_for_its_verdict_and_goes_on_in_order() {
	let mut checks = Checks::new(config(10, 0));
	let mut seen = Seen::default();
	feed(&mut checks, &text_reply(&["0123456789", "rest"]), &mut seen);
	assert_eq!(checks.waiting().unwrap().text, "0123456789");
	assert_eq!(seen.text, "");
	assert_eq!(seen.finish, None);

	checks.resolve(Verdict::Release, &mut |event| seen.event(event));
	assert_eq!(checks.waiting().unwrap().text, "rest");
	assert_eq!(seen.text, "0123456789");
	checks.resolve(Verdict::Release, &mut |event| seen.event(event));
	assert!(checks.waiting().is_none());
	assert_eq!(seen.text, "0123456789rest");
	assert_eq!(seen.finish.as_deref(), Some("Completed"));
}

#[test]
fn a_tool_call_is_checked_once_with_its_name_and_all_arguments() {
	let arguments = r#"{"query":"secret","options":["one","","two"]}"#;
	let mut checks = Checks::new(config(10, 0));
	let mut seen = Seen::default();
	feed(
		&mut checks,
		&call_reply(&[r#"{"query":"sec"#, r#"ret","options":["one","","two"]}"#]),
		&mut seen,
	);
	let check = checks.waiting().unwrap();
	assert_eq!(check.place, ReplyPlace::ToolCall(PartId(0)));
	assert_eq!(check.text, checked_call("search", arguments));
	assert!(seen.calls.is_empty());

	checks.resolve(Verdict::Release, &mut |event| seen.event(event));
	assert_eq!(seen.calls, [arguments]);
	assert_eq!(seen.finish.as_deref(), Some("Completed"));
}

#[test]
fn a_stopped_tool_call_is_never_sent() {
	let events = call_reply(&[r#"{"query":"secret"}"#]);
	let (seen, _) = run(config(10, 0), &events, blocking("secret"));
	assert!(seen.calls.is_empty());
	assert_eq!(seen.error.as_deref(), Some("Blocked."));
	assert_eq!(seen.finish, None);
}

#[rstest]
#[case::stop(IncompleteCall::Stop(INCOMPLETE.into()), Some(INCOMPLETE), false)]
#[case::drop(IncompleteCall::Drop, None, false)]
#[case::send(IncompleteCall::Send, None, true)]
fn an_incomplete_tool_call_follows_the_caller_s_rule(
	#[case] rule: IncompleteCall,
	#[case] error: Option<&str>,
	#[case] sent: bool,
) {
	let config = OutputChecks {
		incomplete_call: rule,
		..config(10, 0)
	};
	let (seen, asked) = run(config, &call_reply(&[r#"{"query":"unfinished"#]), |_| {
		unreachable!("an incomplete call is not offered for checking")
	});
	assert!(asked.is_empty());
	assert_eq!(seen.error.as_deref(), error);
	assert_eq!(seen.calls.len(), usize::from(sent), "{:?}", seen.calls);
}

#[test]
fn held_output_past_its_limit_fails_the_reply() {
	let config = OutputChecks {
		max_held_bytes: 16,
		..config(10, 0)
	};
	let mut checks = Checks::new(config);
	let mut seen = Seen::default();
	feed(
		&mut checks,
		&text_reply(&["0123456789", "more than the limit"]),
		&mut seen,
	);
	assert!(checks.failed());
	assert!(checks.waiting().is_none());
	assert_eq!(seen.text, "");
	assert_eq!(seen.finish, None);
}

proptest! {
	#![proptest_config(ProptestConfig::with_cases(128))]

	#[test]
	fn tool_call_checking_does_not_depend_on_argument_delta_boundaries(
		query in r#"[a-z日本語→\n\\"]{0,32}"#,
		options in prop::collection::vec(".{0,16}", 0..4),
		cuts in prop::collection::vec(any::<usize>(), 0..24),
	) {
		let arguments = json!({"query": query, "options": options}).to_string();
		let pieces = crate::test_support::text_chunks(&arguments, &cuts);
		let (seen, asked) = run(config(10, 0), &call_reply(&pieces), blocking("never"));
		prop_assert_eq!(&asked, &[checked_call("search", &arguments)], "pieces: {:?}", pieces);
		prop_assert_eq!(&seen.calls, &[arguments.as_str()], "pieces: {:?}", pieces);
	}

	#[test]
	fn approved_windows_preserve_text_without_repeating_overlap(
		text in r"[a-z日本語→ \n]{0,128}", cuts in prop::collection::vec(any::<usize>(), 0..24),
		overlap in 0..24usize,
	) {
		let chunks = crate::test_support::text_chunks(&text, &cuts);
		let (seen, asked) = run(config(10, overlap), &text_reply(&chunks), blocking("BLOCKED"));
		prop_assert_eq!(&seen.text, &text);
		let mut reconstructed = String::new();
		for window in asked {
			let mut bytes = 0;
			for ch in reconstructed.chars().rev() {
				if bytes + ch.len_utf8() > overlap { break; }
				bytes += ch.len_utf8();
			}
			let prefix = &reconstructed[reconstructed.len() - bytes..];
			prop_assert!(window.starts_with(prefix));
			reconstructed.push_str(&window[bytes..]);
		}
		prop_assert_eq!(&reconstructed, &text);
	}
}
