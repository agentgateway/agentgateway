#![allow(clippy::panic, clippy::unwrap_used)]

use super::{Sequencer, *};
use crate::carrier;
use crate::ir::events::{Event, Final, PartId, PartKind};
use crate::test_support::SCOPE;

fn raw(text: &str) -> Box<RawValue> {
	RawValue::from_string(text.to_string()).unwrap()
}

#[test]
fn cached_tokens_that_overflow_leave_no_input() {
	let usage = Usage {
		input_tokens: Some(5),
		cached_tokens: Some(u64::MAX),
		cache_write_tokens: Some(1),
		..Usage::default()
	};
	assert_eq!(client_usage(usage, None).input_tokens, 0);
}

#[test]
fn a_reply_with_no_ending_is_a_failure() {
	assert_eq!(ReplyWriter::default().finish().status, 500);
}

#[test]
fn an_unreadable_value_never_becomes_a_guess() {
	let mut writer = ReplyWriter::default();
	writer.op(Op::ToolStart { id: "a", name: "n" });
	writer.op(Op::Arguments { raw: r#""{""# });
	writer.op(Op::BlockEnd);
	let usage = Usage::default();
	writer.op(Op::End {
		stop_reason: "tool_use",
		stop_sequence: None,
		usage,
		service_tier: None,
	});
	assert_eq!(writer.finish().status, 500);
}

#[test]
fn a_whole_reply_keeps_text_as_the_provider_escaped_it() {
	let mut writer = ReplyWriter::default();
	writer.op(Op::TextStart);
	writer.op(Op::Text {
		raw: r#""caf\u00e9 ""#,
	});
	writer.op(Op::Text { raw: r#""\"ok\"""# });
	writer.op(Op::BlockEnd);
	writer.op(Op::End {
		stop_reason: "end_turn",
		stop_sequence: None,
		usage: Usage::default(),
		service_tier: None,
	});
	let body = String::from_utf8(writer.finish().body).unwrap();
	assert!(body.contains(r#""text":"caf\u00e9 \"ok\"""#), "{body}");
}

#[test]
fn the_carrier_contains_the_provider_item_byte_for_byte() {
	let mut writer = ReplyWriter::default();
	let mut sequencer = Sequencer::new(SCOPE, 1024);
	let start = Event::PartStart {
		part: PartId(0),
		kind: PartKind::Reasoning,
	};
	sequencer.event(&start, &mut writer);
	let text = r#"{ "type": "reasoning", "summary": [], "z": 1 }"#;
	let item = raw(text);
	let value = Final::Reasoning {
		text: None,
		encrypted: None,
		item: &item,
	};
	sequencer.event(
		&Event::PartEnd {
			part: PartId(0),
			value,
		},
		&mut writer,
	);
	let Some(ReplyBlock::Thinking { signature, .. }) = writer.content.first() else {
		panic!("no thinking block");
	};
	assert_eq!(
		carrier::open(SCOPE, carrier::CarrierPurpose::MessagesSignature, signature,).unwrap(),
		text.as_bytes()
	);
}

mod sequencer_tests {
	#![allow(clippy::unwrap_used)]

	use rstest::rstest;
	use serde_json::value::RawValue;

	use super::super::*;
	use crate::ir::events::{Arguments, Final, JsonStr, PartKind};
	use crate::test_support::SCOPE;

	#[derive(Default)]
	struct Log(Vec<String>);

	impl Sink for Log {
		fn op(&mut self, op: Op<'_>) {
			self.0.push(match op {
				Op::Start { .. } => "start".to_string(),
				Op::TextStart => "text".to_string(),
				Op::ThinkingStart => "thinking".to_string(),
				Op::ToolStart { id, .. } => format!("tool {id}"),
				Op::Text { raw } | Op::Thinking { raw } | Op::Arguments { raw } => raw.to_string(),
				Op::Signature { .. } => "signature".to_string(),
				Op::Citation { url, .. } => format!("citation {url}"),
				Op::BlockEnd => "end".to_string(),
				Op::End { stop_reason, .. } => format!("stop {stop_reason}"),
				Op::Fail(failure) => format!("fail {}", failure.kind.as_str()),
			});
		}
	}

	fn raw(text: &str) -> Box<RawValue> {
		RawValue::from_string(text.to_string()).unwrap()
	}

	fn string(raw: &RawValue) -> JsonStr<'_> {
		JsonStr::new(raw).unwrap()
	}

	fn call(sequencer: &mut Sequencer<'_>, log: &mut Log, part: u32, id: &str) {
		let kind = PartKind::ToolCall {
			call_id: id,
			name: "n",
		};
		sequencer.event(
			&Event::PartStart {
				part: PartId(part),
				kind,
			},
			log,
		);
	}

	fn end_call(sequencer: &mut Sequencer<'_>, log: &mut Log, part: u32, arguments: &str) {
		let arguments = raw(arguments);
		let value = Final::ToolCall(Arguments::complete(string(&arguments)).unwrap());
		sequencer.event(
			&Event::PartEnd {
				part: PartId(part),
				value,
			},
			log,
		);
	}

	#[test]
	fn queued_citations_count_toward_the_holding_limit() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 128);
		let text = raw(r#""hi""#);
		for part in [0, 2, 4] {
			call(&mut sequencer, &mut log, part, "c");
			for event in [
				Event::PartStart {
					part: PartId(part + 1),
					kind: PartKind::Text,
				},
				Event::Citation {
					part: PartId(part + 1),
					url: "https://example.com",
					title: Some("source"),
				},
				Event::Delta {
					part: PartId(part + 1),
					text: string(&text),
				},
				Event::PartEnd {
					part: PartId(part + 1),
					value: Final::Text,
				},
			] {
				sequencer.event(&event, &mut log);
			}
			end_call(&mut sequencer, &mut log, part, r#""{}""#);
		}
		assert!(!sequencer.is_ended());
		assert_eq!(
			log
				.0
				.iter()
				.filter(|op| *op == "citation https://example.com")
				.count(),
			3
		);

		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 128);
		call(&mut sequencer, &mut log, 0, "c");
		sequencer.event(
			&Event::PartStart {
				part: PartId(1),
				kind: PartKind::Text,
			},
			&mut log,
		);
		for _ in 0..3 {
			sequencer.event(
				&Event::Citation {
					part: PartId(1),
					url: "https://example.com",
					title: Some("source"),
				},
				&mut log,
			);
		}
		assert!(sequencer.is_ended());
		assert_eq!(log.0, ["fail api_error"]);
	}

	#[rstest]
	#[case::long_call_id("123456789", "n")]
	#[case::long_name("c", "123456789")]
	fn tool_identifiers_count_toward_the_holding_limit(#[case] id: &str, #[case] name: &str) {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 16);
		for part in 0..2 {
			sequencer.event(
				&Event::PartStart {
					part: PartId(part),
					kind: PartKind::ToolCall { call_id: id, name },
				},
				&mut log,
			);
		}
		assert!(sequencer.is_ended());
		assert_eq!(log.0, ["fail api_error"]);
	}

	#[test]
	fn tool_identifier_charges_are_released_after_sending() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 6);
		for part in 0..3 {
			call(&mut sequencer, &mut log, part, "c");
			assert_eq!(sequencer.held_bytes, 2);
			end_call(&mut sequencer, &mut log, part, r#""{}""#);
			assert!(!sequencer.is_ended());
			assert_eq!(sequencer.held_bytes, 0);
		}
	}

	#[test]
	fn calls_keep_their_start_order_when_they_end_out_of_order() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 1024);
		call(&mut sequencer, &mut log, 0, "a");
		call(&mut sequencer, &mut log, 1, "b");
		end_call(&mut sequencer, &mut log, 1, r#""{}""#);
		assert!(log.0.is_empty(), "b waits for a: {:?}", log.0);
		end_call(&mut sequencer, &mut log, 0, r#""{}""#);
		let args = r#""{}""#;
		assert_eq!(
			log.0,
			["start", "tool a", args, "end", "tool b", args, "end"]
		);
	}

	#[test]
	fn a_call_is_not_sent_before_its_final_value() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 1024);
		call(&mut sequencer, &mut log, 0, "a");
		let delta = raw(r#""part""#);
		let text = string(&delta);
		sequencer.event(
			&Event::Delta {
				part: PartId(0),
				text,
			},
			&mut log,
		);
		assert!(log.0.is_empty());
		assert!(!sequencer.output_started());
	}

	#[test]
	fn too_much_held_is_a_failure() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 6);
		call(&mut sequencer, &mut log, 0, "a");
		call(&mut sequencer, &mut log, 1, "b");
		end_call(&mut sequencer, &mut log, 1, r#""1234567890""#);
		assert_eq!(log.0, ["fail api_error"]);
		assert!(sequencer.is_ended());
	}

	#[test]
	fn text_for_a_part_that_never_started_is_a_failure() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 1024);
		let text = raw(r#""lost""#);
		sequencer.event(
			&Event::Delta {
				part: PartId(0),
				text: string(&text),
			},
			&mut log,
		);
		assert_eq!(log.0, ["fail api_error"]);
		assert!(sequencer.is_ended());
	}

	#[test]
	fn waiting_text_is_charged_as_it_arrives() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 4);
		for part in [PartId(0), PartId(1)] {
			sequencer.event(
				&Event::PartStart {
					part,
					kind: PartKind::Text,
				},
				&mut log,
			);
		}
		let text = raw(r#""12345""#);
		sequencer.event(
			&Event::Delta {
				part: PartId(1),
				text: string(&text),
			},
			&mut log,
		);

		assert_eq!(log.0, ["fail api_error"]);
		assert!(sequencer.is_ended());
	}

	#[test]
	fn a_sent_part_no_longer_counts_as_held() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 20);
		let start = |part| Event::PartStart {
			part: PartId(part),
			kind: PartKind::Text,
		};
		let streamed = raw(r#""0123456789""#);
		sequencer.event(&start(0), &mut log);
		let text = string(&streamed);
		let delta = Event::Delta {
			part: PartId(0),
			text,
		};
		sequencer.event(&delta, &mut log);
		let value = Final::Text;
		sequencer.event(
			&Event::PartEnd {
				part: PartId(0),
				value,
			},
			&mut log,
		);
		call(&mut sequencer, &mut log, 1, "a");
		call(&mut sequencer, &mut log, 2, "b");
		end_call(&mut sequencer, &mut log, 2, r#""1234567890""#);
		assert!(!sequencer.is_ended(), "{:?}", log.0);
		end_call(&mut sequencer, &mut log, 1, r#""{}""#);
		let waited = r#""1234567890""#;
		let expected = [
			"start",
			"text",
			r#""0123456789""#,
			"end",
			"tool a",
			r#""{}""#,
			"end",
			"tool b",
			waited,
			"end",
		];
		assert_eq!(log.0, expected);
	}

	#[test]
	fn nothing_follows_a_failure() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 1024);
		sequencer.fail(&Failure::new(ErrorType::Api, "x"), &mut log);
		sequencer.fail(&Failure::new(ErrorType::Overloaded, "y"), &mut log);
		sequencer.event(&Event::Finish(Outcome::Completed), &mut log);
		assert_eq!(log.0, ["fail api_error"]);
	}

	#[test]
	fn a_failure_before_any_output_can_be_an_http_error() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 1024);
		sequencer.event(
			&Event::ResponseStart {
				id: "r",
				model: "m",
			},
			&mut log,
		);
		assert!(!sequencer.output_started());
		assert!(log.0.is_empty());
	}

	#[test]
	fn the_signature_goes_out_before_the_block_ends_and_before_later_content() {
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, 1024);
		let start = |part, kind| Event::PartStart {
			part: PartId(part),
			kind,
		};
		sequencer.event(&start(0, PartKind::Reasoning), &mut log);
		sequencer.event(&start(1, PartKind::Text), &mut log);
		let hi = raw(r#""hi""#);
		let text = string(&hi);
		sequencer.event(
			&Event::Delta {
				part: PartId(1),
				text,
			},
			&mut log,
		);
		assert!(log.0.is_empty(), "text waits for reasoning: {:?}", log.0);
		let item = raw(r#"{"type":"reasoning","summary":[]}"#);
		let value = Final::Reasoning {
			text: None,
			encrypted: None,
			item: &item,
		};
		sequencer.event(
			&Event::PartEnd {
				part: PartId(0),
				value,
			},
			&mut log,
		);
		let value = Final::Text;
		sequencer.event(
			&Event::PartEnd {
				part: PartId(1),
				value,
			},
			&mut log,
		);
		let want = [
			"start",
			"thinking",
			"signature",
			"end",
			"text",
			r#""hi""#,
			"end",
		];
		assert_eq!(log.0, want);
	}

	#[test]
	fn streamed_parts_do_not_count_as_held() {
		let item = raw(r#"{"type":"reasoning"}"#);
		let thought = raw(r#""a thought longer than the limit""#);
		let answer = raw(r#""an answer longer than the limit""#);
		let mut log = Log::default();
		let mut sequencer = Sequencer::new(SCOPE, item.get().len());
		for (part, kind, text) in [
			(0, PartKind::Reasoning, &thought),
			(1, PartKind::Text, &answer),
		] {
			sequencer.event(
				&Event::PartStart {
					part: PartId(part),
					kind,
				},
				&mut log,
			);
			sequencer.event(
				&Event::Delta {
					part: PartId(part),
					text: string(text),
				},
				&mut log,
			);
			let value = match kind {
				PartKind::Reasoning => Final::Reasoning {
					text: Some(string(text)),
					encrypted: None,
					item: &item,
				},
				_ => Final::Text,
			};
			sequencer.event(
				&Event::PartEnd {
					part: PartId(part),
					value,
				},
				&mut log,
			);
		}

		let want = [
			"start",
			"thinking",
			thought.get(),
			"signature",
			"end",
			"text",
			answer.get(),
			"end",
		];
		assert_eq!(log.0, want);
	}
}
