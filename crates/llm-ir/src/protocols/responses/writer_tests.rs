#![allow(clippy::unwrap_used)]

use rstest::rstest;
use serde_json::value::RawValue;

use super::*;
use crate::ir::events::{Arguments, JsonStr};
use crate::test_support::SCOPE;

#[rstest]
#[case::long_call_id("123456789", "n")]
#[case::long_name("c", "123456789")]
fn tool_identifiers_count_toward_the_output_limit_before_completion(
	#[case] id: &str,
	#[case] name: &str,
) {
	let mut writer = ResponseStream::new(SCOPE, &[], 0, 16);
	for part in 0..2 {
		writer.event(&Event::PartStart {
			part: PartId(part),
			kind: PartKind::ToolCall { call_id: id, name },
		});
	}
	assert_eq!(writer.state.reply.failure, Some(("server_error", 500)));
}

#[rstest]
#[case::in_order([0, 1])]
#[case::out_of_order([1, 0])]
fn tool_identifier_charges_are_released_for_direct_and_waiting_parts(#[case] order: [u32; 2]) {
	let raw = RawValue::from_string(r#""{}""#.to_string()).unwrap();
	let value = Final::ToolCall(Arguments::complete(JsonStr::new(&raw).unwrap()).unwrap());
	let mut writer = ResponseStream::new(SCOPE, &[], 0, 512);
	for part in 0..2 {
		writer.event(&Event::PartStart {
			part: PartId(part),
			kind: PartKind::ToolCall {
				call_id: "c",
				name: "n",
			},
		});
	}
	assert_eq!(writer.state.held_bytes, 4);
	for part in order {
		writer.event(&Event::PartEnd {
			part: PartId(part),
			value,
		});
		while writer.progress() == Progress::HasOutput {
			writer.take_stream();
		}
	}
	assert!(writer.state.reply.failure.is_none());
	assert_eq!(writer.state.held_bytes, 0);
	assert_eq!(writer.state.output.len(), 2);
}

#[test]
fn citations_count_toward_the_output_limit_before_the_part_ends() {
	let mut writer = ResponseStream::new(SCOPE, &[], 0, 128);
	writer.event(&Event::PartStart {
		part: PartId(0),
		kind: PartKind::Text,
	});
	for _ in 0..3 {
		writer.event(&Event::Citation {
			part: PartId(0),
			url: "https://example.com",
			title: Some("source"),
		});
	}
	assert_eq!(writer.state.reply.failure, Some(("server_error", 500)));
}

#[test]
fn a_buffered_reply_writes_parts_that_waited() {
	let text = RawValue::from_string(r#""hello""#.to_string()).unwrap();
	let mut reply = BufferedResponse::new(SCOPE, &[], 0, 512);
	for event in [
		Event::PartStart {
			part: PartId(0),
			kind: PartKind::Text,
		},
		Event::PartStart {
			part: PartId(1),
			kind: PartKind::Text,
		},
		Event::Citation {
			part: PartId(1),
			url: "https://example.com",
			title: Some("source"),
		},
		Event::Citation {
			part: PartId(0),
			url: "https://example.com",
			title: Some("source"),
		},
		Event::Delta {
			part: PartId(1),
			text: JsonStr::new(&text).unwrap(),
		},
		Event::PartEnd {
			part: PartId(1),
			value: Final::Text,
		},
		Event::Delta {
			part: PartId(0),
			text: JsonStr::new(&text).unwrap(),
		},
		Event::PartEnd {
			part: PartId(0),
			value: Final::Text,
		},
		Event::Finish(Outcome::Completed),
	] {
		reply.event(&event);
	}
	assert_eq!(reply.into_reply().status, 200);
}

/// Two overlapping parts, the later one ending first so it's held.
fn overlap(limit: usize, text: &RawValue) -> ResponseStream<'static> {
	let mut writer = ResponseStream::new(SCOPE, &[], 0, limit);
	for event in [
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
			text: JsonStr::new(text).unwrap(),
		},
		Event::PartEnd {
			part: PartId(1),
			value: Final::Text,
		},
		Event::Delta {
			part: PartId(0),
			text: JsonStr::new(text).unwrap(),
		},
		Event::PartEnd {
			part: PartId(0),
			value: Final::Text,
		},
	] {
		writer.event(&event);
	}
	writer
}

#[test]
fn the_limit_counts_output_and_held_parts_together() {
	let text = RawValue::from_string(format!("\"{}\"", "x".repeat(500))).unwrap();
	assert!(!overlap(2000, &text).state.ended);
	assert!(overlap(1000, &text).state.ended);
}

#[test]
fn open_text_is_charged_as_it_arrives() {
	let text = RawValue::from_string(r#""12345""#.to_string()).unwrap();
	let mut writer = ResponseStream::new(SCOPE, &[], 0, 6);
	writer.event(&Event::PartStart {
		part: PartId(0),
		kind: PartKind::Text,
	});
	writer.event(&Event::Delta {
		part: PartId(0),
		text: JsonStr::new(&text).unwrap(),
	});

	assert!(writer.state.ended);
	assert_eq!(writer.state.reply.failure, Some(("server_error", 500)));
}

#[rstest]
fn partial_calls_never_produce_an_output_item(
	#[values("", "{\"key\":", "{\"key\":1}")] text: &str,
	#[values(CallKind::Function, CallKind::Custom, CallKind::ToolSearch)] kind: CallKind,
) {
	let raw = to_raw_value(text).unwrap();
	let arguments = Arguments::Partial(JsonStr::new(&raw).unwrap());
	let routes = [ToolRoute {
		flat: "record".into(),
		name: "record".into(),
		namespace: None,
		kind,
	}];
	let mut stream = ResponseStream::new(SCOPE, &routes, 0, 1 << 16);
	let mut buffered = BufferedResponse::new(SCOPE, &routes, 0, 1 << 16);
	let mut sent = Vec::new();
	for event in [
		Event::PartStart {
			part: PartId(0),
			kind: PartKind::ToolCall {
				call_id: "call_1",
				name: "record",
			},
		},
		Event::PartEnd {
			part: PartId(0),
			value: Final::ToolCall(arguments),
		},
		Event::Usage(Usage {
			input_tokens: Some(10),
			output_tokens: Some(2048),
			..Usage::default()
		}),
		Event::Finish(Outcome::Incomplete {
			reason: StopReason::MaxTokens,
			raw: None,
		}),
	] {
		stream.event(&event);
		buffered.event(&event);
		while stream.progress() == Progress::HasOutput {
			let chunk = stream.take_stream();
			assert!(
				!std::str::from_utf8(&chunk)
					.unwrap()
					.contains("response.output_item"),
				"a client may dispatch any announced call, regardless of its status"
			);
			sent.extend(chunk);
		}
	}
	assert!(stream.early_failure().is_none());
	assert!(
		std::str::from_utf8(&sent)
			.unwrap()
			.contains("response.incomplete")
	);
	let reply = buffered.into_reply();
	assert_eq!(reply.status, 200);
	let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(body["output"], serde_json::json!([]));
	assert_eq!(body["incomplete_details"]["reason"], "max_output_tokens");
	assert_eq!(body["usage"]["output_tokens"], 2048);
}

#[test]
fn a_call_with_no_arguments_takes_an_empty_object() {
	let empty = RawValue::from_string(r#""""#.to_string()).unwrap();
	let arguments = Arguments::complete(JsonStr::new(&empty).unwrap()).unwrap();
	let mut reply = BufferedResponse::new(SCOPE, &[], 0, 1 << 16);
	for event in [
		Event::PartStart {
			part: PartId(0),
			kind: PartKind::ToolCall {
				call_id: "call_1",
				name: "list",
			},
		},
		Event::PartEnd {
			part: PartId(0),
			value: Final::ToolCall(arguments),
		},
		Event::Finish(Outcome::Completed),
	] {
		reply.event(&event);
	}
	let reply = reply.into_reply();
	assert_eq!(reply.status, 200);
	let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(body["output"][0]["arguments"], "{}");
}

#[test]
fn text_for_a_part_that_never_started_is_a_failure() {
	let text = RawValue::from_string(r#""lost""#.to_string()).unwrap();
	let mut writer = ResponseStream::new(SCOPE, &[], 0, 1024);
	writer.event(&Event::Delta {
		part: PartId(0),
		text: JsonStr::new(&text).unwrap(),
	});
	assert_eq!(writer.state.reply.failure, Some(("server_error", 500)));
}

#[test]
fn an_item_over_the_limit_is_never_sent_as_done() {
	let text = RawValue::from_string(format!("\"{}\"", "x".repeat(500))).unwrap();
	let mut writer = overlap(1000, &text);
	let mut out = Vec::new();
	while writer.progress() == Progress::HasOutput {
		out.extend(writer.take_stream());
	}
	let out = String::from_utf8(out).unwrap();
	assert!(!out.contains("response.output_item.done"), "{out}");
	assert!(out.contains("response.failed"), "{out}");
}

mod error_tests {
	#![allow(clippy::unwrap_used)]

	use super::super::*;

	fn reply(status: u16, kind: ErrorKind, field: Option<&str>) -> (u16, serde_json::Value) {
		let HttpReply { status, body } = error_reply_naming(status, kind, field);
		(status, serde_json::from_slice(&body).unwrap())
	}

	#[test]
	fn a_missing_model_isnt_a_failure_to_retry() {
		let (status, body) = reply(404, ErrorKind::NotFound, None);
		insta::assert_json_snapshot!(serde_json::json!({"status": status, "body": body}), @r#"
		{
		  "status": 404,
		  "body": {
		    "error": {
		      "type": "model_not_found",
		      "code": "model_not_found",
		      "message": "The provider doesn't have this model."
		    }
		  }
		}
		"#);
	}

	#[test]
	fn only_a_field_name_reaches_the_client() {
		let (status, body) = reply(400, ErrorKind::InvalidRequest, Some("generation_config"));
		assert_eq!(status, 400);
		assert_eq!(
			body["error"]["message"],
			"The provider refused `generation_config`."
		);
		let (_, body) = reply(400, ErrorKind::InvalidRequest, Some("Project 1234 failed"));
		assert_eq!(
			body["error"]["message"],
			"The provider refused the request."
		);
		let (_, body) = reply(429, ErrorKind::RateLimit, Some("input"));
		assert_eq!(
			body["error"]["message"],
			"The provider's rate limit was reached."
		);
	}
}
