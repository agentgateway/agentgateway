#![allow(clippy::unwrap_used)]

use rstest::rstest;

use super::*;
use crate::carrier;
use crate::protocols::messages::writer::Op;
use crate::sse::{Frame, SseParser};
use crate::test_support::SCOPE;

const LIMIT: usize = 1 << 20;

/// One line per event, with JSON strings as the provider wrote them.
fn line(event: Event<'_>) -> String {
	match event {
		Event::ResponseStart { id, model } => format!("start {id} {model}"),
		Event::PartStart { part, kind } => match kind {
			PartKind::Text => format!("{} text", part.0),
			PartKind::Refusal => format!("{} refusal", part.0),
			PartKind::Reasoning => format!("{} reasoning", part.0),
			PartKind::ToolCall { call_id, name } => format!("{} call {call_id} {name}", part.0),
		},
		Event::Delta { part, text } => format!("{} delta {}", part.0, text.raw()),
		Event::PartEnd { part, value } => match value {
			Final::Text => format!("{} end", part.0),
			Final::Refusal(text) => format!("{} end refusal {}", part.0, text.raw()),
			Final::ToolCall(arguments) => {
				let complete = if arguments.is_complete() {
					"complete"
				} else {
					"partial"
				};
				let text: String = serde_json::from_str(arguments.raw()).unwrap();
				format!("{} end {complete} {text}", part.0)
			},
			Final::Reasoning {
				text,
				encrypted,
				item,
			} => format!(
				"{} end text {} signature {} item {}",
				part.0,
				text.map_or("-", |text| text.raw()),
				encrypted.map_or("-", |signature| signature.raw()),
				item.get()
			),
		},
		Event::Usage(usage) => format!(
			"usage in {:?} out {:?} total {:?} cached {:?} reasoning {:?}",
			usage.input_tokens,
			usage.output_tokens,
			usage.total_tokens,
			usage.cached_tokens,
			usage.reasoning_tokens
		),
		Event::ServiceTier(tier) => format!("tier {tier}"),
		Event::Finish(Outcome::Completed) => "finish completed".to_string(),
		Event::Finish(Outcome::Incomplete { reason, raw }) => {
			format!("finish {reason:?} {}", raw.unwrap_or("-"))
		},
		Event::Error(error) => format!(
			"error {:?} {} {}",
			error.kind,
			error.code.unwrap_or("-"),
			error.message.unwrap_or("-")
		),
		other => format!("{other:?}"),
	}
}

/// Each payload as one SSE event's data. The result is the first error, or `finish`.
fn streamed(payloads: &[&str]) -> (Vec<String>, Result<(), DecodeError>) {
	let mut decoder = GeminiDecoder::new("requested", "reply-1");
	let mut lines = Vec::new();
	let mut result = Ok(());
	for payload in payloads {
		let pushed = decoder.push_json(payload.as_bytes(), LIMIT, &mut |event| {
			lines.push(line(event))
		});
		if result.is_ok() {
			result = pushed;
		}
	}
	(lines, result.and_then(|_| decoder.finish()))
}

fn buffered(body: &str) -> (Vec<String>, Result<(), DecodeError>) {
	let mut lines = Vec::new();
	let result = decode_buffered(body.as_bytes(), "requested", "reply-1", &mut |event| {
		lines.push(line(event))
	});
	(lines, result)
}

fn reply(parts: &str, finish: &str) -> String {
	format!(
		r#"{{"responseId":"r1","modelVersion":"gemini-test","candidates":[{{"content":{{"role":"model","parts":[{parts}]}}{finish}}}]}}"#
	)
}

const STOP: &str = r#","finishReason":"STOP""#;

#[test]
fn text_streams_as_one_part_across_chunks() {
	let (lines, result) = streamed(&[
		&reply(r#"{"text":"Hel"}"#, ""),
		&reply(r#"{"text":"lo"}"#, ""),
		&reply(r#"{"text":""}"#, STOP),
	]);
	result.unwrap();
	assert_eq!(
		lines,
		[
			"start r1 gemini-test",
			"0 text",
			r#"0 delta "Hel""#,
			r#"0 delta "lo""#,
			"0 end",
			"finish completed",
		]
	);
}

#[test]
fn a_buffered_reply_gives_the_events_a_stream_gives() {
	let body = reply(
		r#"{"text":"Think.","thought":true},{"text":"Hello"},{"functionCall":{"name":"f","args":{"a":1}}}"#,
		STOP,
	);
	let (whole, result) = buffered(&body);
	result.unwrap();
	let (split, result) = streamed(&[
		&reply(r#"{"text":"Think.","thought":true}"#, ""),
		&reply(r#"{"text":"Hello"}"#, ""),
		&reply(r#"{"functionCall":{"name":"f","args":{"a":1}}}"#, STOP),
	]);
	result.unwrap();
	assert_eq!(whole, split);
	assert_eq!(
		whole,
		[
			"start r1 gemini-test",
			"0 reasoning",
			r#"0 delta "Think.""#,
			r#"0 end text "Think." signature - item {"text":"Think.","thought":true}"#,
			"1 text",
			r#"1 delta "Hello""#,
			"1 end",
			"2 call call_r1_0 f",
			r#"2 end complete {"a":1}"#,
			"finish completed",
		]
	);
}

#[test]
fn a_buffered_reply_without_a_finish_reason_fails() {
	let (_, result) = buffered(&reply(r#"{"text":"Hello"}"#, ""));
	assert!(matches!(result, Err(DecodeError::NoTerminalEvent)));
	let (_, result) = buffered("not json");
	assert!(matches!(result, Err(DecodeError::InvalidJson(_))));
}

#[test]
fn a_thought_signature_closes_its_thought_and_stays_in_its_item() {
	let (lines, result) = streamed(&[
		&reply(r#"{"text":"Con","thought":true}"#, ""),
		&reply(
			r#"{"text":"sider.","thought":true,"thoughtSignature":"sig"}"#,
			"",
		),
		&reply(r#"{"text":"More.","thought":true}"#, ""),
		&reply(r#"{"text":"Answer"}"#, STOP),
	]);
	result.unwrap();
	assert_eq!(
		lines[1..6],
		[
			"0 reasoning",
			r#"0 delta "Con""#,
			r#"0 delta "sider.""#,
			r#"0 end text "Consider." signature "sig" item {"text":"Consider.","thought":true,"thoughtSignature":"sig"}"#,
			"1 reasoning",
		]
	);
	assert_eq!(
		lines[7],
		r#"1 end text "More." signature - item {"text":"More.","thought":true}"#
	);
}

#[test]
fn a_late_text_signature_becomes_a_part_of_its_own() {
	let (lines, result) = streamed(&[
		&reply(r#"{"text":"Hel"}"#, ""),
		&reply(r#"{"text":"lo"}"#, ""),
		&reply(r#"{"text":"","thoughtSignature":"late"}"#, STOP),
	]);
	result.unwrap();
	assert_eq!(
		lines[4..],
		[
			"0 end",
			"1 reasoning",
			r#"1 end text - signature "late" item {"text":"","thoughtSignature":"late"}"#,
			"finish completed",
		]
	);
	let item: Box<RawValue> =
		RawValue::from_string(r#"{"text":"","thoughtSignature":"late"}"#.into()).unwrap();
	assert!(signs_text(&item));
}

#[test]
fn a_signature_on_text_follows_that_text() {
	let (lines, result) = streamed(&[&reply(r#"{"text":"Hi","thoughtSignature":"s"}"#, STOP)]);
	result.unwrap();
	assert_eq!(
		lines[1..],
		[
			"0 text",
			r#"0 delta "Hi""#,
			"0 end",
			"1 reasoning",
			r#"1 end text - signature "s" item {"text":"Hi","thoughtSignature":"s"}"#,
			"finish completed",
		]
	);
}

#[test]
fn a_signed_thought_with_no_text_keeps_its_signature() {
	for before in [r#"{"text":"Hi"},"#, ""] {
		let parts = format!(r#"{before}{{"text":"","thought":true,"thoughtSignature":"s"}}"#);
		let (lines, result) = streamed(&[&reply(&parts, STOP)]);
		result.unwrap();
		assert!(
			lines.iter().any(|line| line.contains(r#"signature "s""#)),
			"{before}: {lines:?}"
		);
	}
}

#[test]
fn parallel_calls_keep_their_order_and_the_first_call_signature() {
	let (lines, result) = streamed(&[&reply(
		r#"{"functionCall":{"id":"a","name":"f","args":{"x":1}},"thoughtSignature":"s"},{"functionCall":{"name":"g"}},{"functionCall":{"name":"f","args":{}}}"#,
		STOP,
	)]);
	result.unwrap();
	assert_eq!(
		lines[1..],
		[
			"0 reasoning",
			r#"0 end text - signature "s" item {"functionCall":{"id":"a","name":"f","args":{"x":1}},"thoughtSignature":"s"}"#,
			"1 call a f",
			r#"1 end complete {"x":1}"#,
			"2 call call_r1_1 g",
			r#"2 end complete {}"#,
			"3 call call_r1_2 f",
			r#"3 end complete {}"#,
			"finish completed",
		]
	);
	let item: Box<RawValue> =
		RawValue::from_string(r#"{"functionCall":{"name":"f"},"thoughtSignature":"s"}"#.into())
			.unwrap();
	assert!(!signs_text(&item));
}

#[test]
fn a_call_closes_open_text() {
	let (lines, result) = streamed(&[
		&reply(r#"{"text":"Let me look."}"#, ""),
		&reply(r#"{"functionCall":{"name":"f","args":{}}}"#, STOP),
	]);
	result.unwrap();
	assert_eq!(
		lines[1..5],
		[
			"0 text",
			r#"0 delta "Let me look.""#,
			"0 end",
			"1 call call_r1_0 f"
		]
	);
}

#[rstest]
#[case::not_an_object(r#"{"functionCall":{"name":"f","args":[1]}}"#, "args")]
#[case::no_name(r#"{"functionCall":{"args":{}}}"#, "functionCall")]
fn a_call_gemini_could_not_have_meant_fails(#[case] part: &str, #[case] what: &str) {
	let (lines, result) = streamed(&[&reply(part, STOP)]);
	assert!(
		matches!(result, Err(DecodeError::WrongType(found)) if found == what),
		"{result:?}"
	);
	assert!(!lines.iter().any(|line| line.contains("call")));
}

#[rstest]
#[case::stop("STOP", "finish completed")]
#[case::max_tokens("MAX_TOKENS", "finish MaxTokens MAX_TOKENS")]
#[case::safety("SAFETY", "finish ContentFilter SAFETY")]
#[case::recitation("RECITATION", "finish ContentFilter RECITATION")]
#[case::prohibited("PROHIBITED_CONTENT", "finish ContentFilter PROHIBITED_CONTENT")]
#[case::missing_signature(
	"MISSING_THOUGHT_SIGNATURE",
	"error InvalidRequest MISSING_THOUGHT_SIGNATURE why"
)]
#[case::malformed_call("MALFORMED_FUNCTION_CALL", "error Unknown MALFORMED_FUNCTION_CALL why")]
#[case::unknown("SOMETHING_NEW", "error Unknown SOMETHING_NEW why")]
fn each_finish_reason_has_one_meaning(#[case] reason: &str, #[case] ending: &str) {
	let finish = format!(r#","finishReason":"{reason}","finishMessage":"why""#);
	let (lines, result) = streamed(&[&reply(r#"{"text":"Hi"}"#, &finish)]);
	result.unwrap();
	assert_eq!(lines.last().unwrap(), ending);
	assert_eq!(lines[lines.len() - 2], "0 end", "the open part ends first");
}

#[test]
fn a_blocked_prompt_is_filtered_content_with_its_usage() {
	let (lines, result) = buffered(
		r#"{"candidates":[],"promptFeedback":{"blockReason":"SAFETY"},"usageMetadata":{"promptTokenCount":12,"totalTokenCount":12}}"#,
	);
	result.unwrap();
	assert_eq!(
		lines,
		[
			"start reply-1 requested",
			"usage in Some(12) out None total Some(12) cached None reasoning None",
			"finish ContentFilter SAFETY",
		]
	);
}

#[test]
fn feedback_without_a_block_reason_is_not_an_ending() {
	let (lines, result) = streamed(&[
		r#"{"promptFeedback":{"safetyRatings":[]}}"#,
		&reply(r#"{"text":"Hi"}"#, STOP),
	]);
	result.unwrap();
	assert_eq!(lines.last().unwrap(), "finish completed");
	let (_, result) = streamed(&[r#"{"promptFeedback":{}}"#]);
	assert!(matches!(result, Err(DecodeError::NoTerminalEvent)));
}

#[test]
fn blocked_output_ends_the_open_part_as_filtered() {
	let (lines, result) = streamed(&[
		&reply(r#"{"text":"Once upon"}"#, ""),
		&reply("", r#","finishReason":"RECITATION""#),
	]);
	result.unwrap();
	assert_eq!(lines[3..], ["0 end", "finish ContentFilter RECITATION"]);
}

#[test]
fn an_error_in_the_stream_ends_it() {
	let (lines, result) = streamed(&[
		&reply(r#"{"text":"Hi"}"#, ""),
		r#"{"error":{"code":429,"message":"slow down","status":"RESOURCE_EXHAUSTED"}}"#,
	]);
	result.unwrap();
	assert_eq!(
		lines.last().unwrap(),
		"error RateLimit RESOURCE_EXHAUSTED slow down"
	);
	let mut decoder = GeminiDecoder::new("m", "reply-1");
	decoder
		.push_json(br#"{"error":{"status":"UNAVAILABLE"}}"#, LIMIT, &mut |_| {})
		.unwrap();
	assert!(decoder.is_finished());
	assert!(matches!(
		decoder.push_json(&reply("", STOP).into_bytes(), LIMIT, &mut |_| {}),
		Err(DecodeError::OutOfOrder("chunk after the end"))
	));
}

#[rstest]
#[case::bad_key(
	r#"{"error":{"code":400,"message":"API key not valid.","status":"INVALID_ARGUMENT","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"API_KEY_INVALID"}]}}"#,
	ErrorKind::Authentication
)]
#[case::too_long(
	r#"{"error":{"code":400,"message":"The input token count (2000000) exceeds the maximum number of tokens allowed (1048576).","status":"INVALID_ARGUMENT"}}"#,
	ErrorKind::ContextTooLong
)]
#[case::invalid(
	r#"{"error":{"code":400,"message":"bad","status":"INVALID_ARGUMENT"}}"#,
	ErrorKind::InvalidRequest
)]
#[case::precondition(
	r#"{"error":{"code":400,"status":"FAILED_PRECONDITION"}}"#,
	ErrorKind::InvalidRequest
)]
#[case::unauthenticated(
	r#"{"error":{"code":401,"status":"UNAUTHENTICATED"}}"#,
	ErrorKind::Authentication
)]
#[case::denied(
	r#"{"error":{"code":403,"status":"PERMISSION_DENIED"}}"#,
	ErrorKind::Permission
)]
#[case::not_found(r#"{"error":{"code":404,"status":"NOT_FOUND"}}"#, ErrorKind::NotFound)]
#[case::credit(
	r#"{"error":{"code":402,"status":"RESOURCE_EXHAUSTED"}}"#,
	ErrorKind::Billing
)]
#[case::rate(
	r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED"}}"#,
	ErrorKind::RateLimit
)]
#[case::overloaded(
	r#"{"error":{"code":503,"status":"UNAVAILABLE"}}"#,
	ErrorKind::Overloaded
)]
#[case::internal(r#"{"error":{"code":500,"status":"INTERNAL"}}"#, ErrorKind::Unknown)]
#[case::not_json("<html>bad gateway</html>", ErrorKind::Unknown)]
#[case::no_error(r#"{"candidates":[]}"#, ErrorKind::Unknown)]
fn http_errors_are_classified_by_status_and_detail(#[case] body: &str, #[case] kind: ErrorKind) {
	assert_eq!(http_error_kind(body.as_bytes()), kind);
}

#[test]
fn an_http_error_names_the_first_refused_field() {
	let body = r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","details":[
		{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"x"},
		{"@type":"type.googleapis.com/google.rpc.BadRequest","fieldViolations":[
			{"field":"generation_config.temperature","description":"out of range"},
			{"field":"contents[0]"}]}]}}"#;
	assert_eq!(
		http_error_field(body.as_bytes()).as_deref(),
		Some("generation_config.temperature")
	);
	assert_eq!(http_error_field(br#"{"error":{"code":400}}"#), None);
	assert_eq!(http_error_field(b"not json"), None);
}

#[test]
fn usage_merges_across_chunks_and_counts_thoughts_as_output() {
	let (lines, result) = streamed(&[
		r#"{"usageMetadata":{"promptTokenCount":10,"trafficType":"ON_DEMAND"}}"#,
		&reply(r#"{"text":"Hi"}"#, STOP).replacen(
			r#"{"responseId""#,
			r#"{"usageMetadata":{"candidatesTokenCount":4,"thoughtsTokenCount":6,"cachedContentTokenCount":8,"totalTokenCount":20,"serviceTier":"priority"},"responseId""#,
			1,
		),
	]);
	result.unwrap();
	let usage: Vec<&String> = lines
		.iter()
		.filter(|line| line.starts_with("usage") || line.starts_with("tier"))
		.collect();
	assert_eq!(
		usage,
		[
			"tier default",
			"usage in Some(10) out None total None cached None reasoning None",
			"tier priority",
			"usage in Some(10) out Some(10) total Some(20) cached Some(8) reasoning Some(6)",
		]
	);
}

#[rstest]
#[case::answer_only(r#""candidatesTokenCount":4"#, Some(4), None)]
#[case::thoughts_only(r#""thoughtsTokenCount":6"#, Some(6), Some(6))]
#[case::neither(r#""promptTokenCount":1"#, None, None)]
fn output_tokens_are_what_gemini_reported(
	#[case] counts: &str,
	#[case] output: Option<u64>,
	#[case] reasoning: Option<u64>,
) {
	let mut usage = None;
	let body =
		format!(r#"{{"usageMetadata":{{{counts}}},"promptFeedback":{{"blockReason":"OTHER"}}}}"#);
	decode_buffered(body.as_bytes(), "m", "reply-1", &mut |event| {
		if let Event::Usage(seen) = event {
			usage = Some(seen);
		}
	})
	.unwrap();
	let usage = usage.unwrap();
	assert_eq!(usage.output_tokens, output);
	assert_eq!(usage.reasoning_tokens, reasoning);
}

#[test]
fn output_counts_that_overflow_fail() {
	let body = format!(
		r#"{{"usageMetadata":{{"candidatesTokenCount":{},"thoughtsTokenCount":1}}}}"#,
		u64::MAX
	);
	let (_, result) = streamed(&[&body]);
	assert!(matches!(
		result,
		Err(DecodeError::WrongType("usageMetadata"))
	));
}

#[test]
fn usage_after_a_decode_error_is_still_reported() {
	let (lines, result) = streamed(&[
		&reply(r#"{"text":"Hi"}"#, ""),
		&reply(r#"{"executableCode":{"code":"1"}}"#, ""),
		&reply(r#"{"text":"ignored"}"#, ""),
		r#"{"candidates":[{"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2}}"#,
	]);
	assert!(matches!(result, Err(DecodeError::UnsupportedItem(item)) if item == "part"));
	assert_eq!(
		lines.last().unwrap(),
		"usage in Some(3) out Some(2) total None cached None reasoning None"
	);
	assert!(!lines.iter().any(|line| line.contains("ignored")));
}

#[test]
fn draining_stops_at_the_ending() {
	let mut decoder = GeminiDecoder::new("m", "reply-1");
	assert!(decoder.push_json(b"not json", LIMIT, &mut |_| {}).is_err());
	decoder
		.push_json(&reply("", "").into_bytes(), LIMIT, &mut |_| {})
		.unwrap();
	assert!(!decoder.is_finished());
	decoder
		.push_json(&reply("", STOP).into_bytes(), LIMIT, &mut |_| {})
		.unwrap();
	assert!(decoder.is_finished());
	decoder.finish().unwrap();
}

#[rstest]
#[case::two_candidates(
	r#"{"candidates":[{"content":{"parts":[{"text":"a"}]}},{"content":{"parts":[{"text":"b"}]}}]}"#,
	"candidates"
)]
#[case::code(
	r#"{"candidates":[{"content":{"parts":[{"executableCode":{}}]}}]}"#,
	"part"
)]
#[case::image(
	r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":"AA=="}}]}}]}"#,
	"part"
)]
fn output_the_client_cannot_be_given_fails(#[case] payload: &str, #[case] item: &str) {
	let (lines, result) = streamed(&[payload]);
	assert!(
		matches!(&result, Err(DecodeError::UnsupportedItem(found)) if found == item),
		"{result:?}"
	);
	assert!(!lines.iter().any(|line| line.contains("delta")));
}

#[test]
fn a_text_that_is_not_a_string_fails() {
	let (_, result) = streamed(&[&reply(r#"{"text":7}"#, STOP)]);
	assert!(matches!(result, Err(DecodeError::WrongType("text"))));
	let (_, result) = streamed(&[&reply(r#"{"text":"a","thoughtSignature":7}"#, STOP)]);
	assert!(matches!(
		result,
		Err(DecodeError::WrongType("thoughtSignature"))
	));
}

#[test]
fn a_payload_over_the_limit_fails() {
	let payload = reply(r#"{"text":"Hello"}"#, "");
	let mut decoder = GeminiDecoder::new("m", "reply-1");
	assert!(matches!(
		decoder.push_json(payload.as_bytes(), payload.len() - 1, &mut |_| {}),
		Err(DecodeError::EventTooLarge { .. })
	));
}

#[test]
fn a_part_built_from_chunks_is_bounded_by_its_decoded_size() {
	for thought in ["", r#","thought":true"#] {
		// Escaped, each chunk's text is 12 bytes; decoded, it is 4.
		let chunk = reply(&format!(r#"{{"text":"\u00e9\u00e9"{thought}}}"#), "");
		let limit = chunk.len();
		let mut decoder = GeminiDecoder::new("m", "reply-1");
		for _ in 0..limit / 4 {
			decoder
				.push_json(chunk.as_bytes(), limit, &mut |_| {})
				.unwrap();
		}
		assert!(
			matches!(
				decoder.push_json(chunk.as_bytes(), limit, &mut |_| {}),
				Err(DecodeError::EventTooLarge { .. })
			),
			"{thought}"
		);
	}
}

#[test]
fn a_reply_names_its_model_or_the_requested_one() {
	let (lines, _) = buffered(r#"{"candidates":[{"finishReason":"STOP"}]}"#);
	assert_eq!(lines[0], "start reply-1 requested");
	let (lines, _) = buffered(&reply("", STOP));
	assert_eq!(lines[0], "start r1 gemini-test");
}

#[test]
fn the_same_events_come_at_every_chunk_size() {
	let payloads = [
		r#"{"usageMetadata":{"promptTokenCount":5}}"#.to_string(),
		reply(r#"{"text":"Plan é 🦀","thought":true}"#, ""),
		reply(r#"{"text":"\n","thought":true,"thoughtSignature":"t"}"#, ""),
		reply(r#"{"text":"Hello \"world\""}"#, ""),
		reply(
			r#"{"functionCall":{"name":"f","args":{"q":"é"}},"thoughtSignature":"c"},{"functionCall":{"name":"g","args":{}}}"#,
			r#","finishReason":"STOP""#,
		),
	];
	let mut stream = String::new();
	for payload in payloads {
		stream.push_str("data: ");
		stream.push_str(&payload);
		stream.push_str("\r\n\r\n");
	}
	let read = |size: usize| {
		let mut decoder = GeminiDecoder::new("m", "reply-1");
		let mut sse = SseParser::new(LIMIT);
		let mut lines = Vec::new();
		for piece in stream.as_bytes().chunks(size) {
			sse.push(piece);
			while let Some(frame) = sse.next_frame() {
				let Frame::Event(event) = frame else {
					unreachable!("no event is over the limit");
				};
				decoder
					.push_json(event.data, LIMIT, &mut |event| lines.push(line(event)))
					.unwrap();
			}
		}
		decoder.finish().unwrap();
		sse.finish().unwrap();
		lines
	};
	let whole = read(stream.len());
	assert_eq!(whole.last().unwrap(), "finish completed");
	assert_eq!(
		whole.iter().filter(|line| line.contains(" call ")).count(),
		2
	);
	for size in 1..stream.len() {
		assert_eq!(read(size), whole, "chunks of {size} bytes");
	}
}

/// Each block the client gets, with the provider part its signature contains.
#[derive(Default)]
struct Blocks(Vec<String>);

impl Sink for Blocks {
	fn op(&mut self, op: Op<'_>) {
		match op {
			Op::TextStart => self.0.push("text".to_string()),
			Op::ThinkingStart => self.0.push("thinking".to_string()),
			Op::ToolStart { name, .. } => self.0.push(format!("tool {name}")),
			Op::Signature { value } => {
				let item = carrier::open(SCOPE, carrier::CarrierPurpose::MessagesSignature, value).unwrap();
				let last = self.0.last_mut().unwrap();
				last.push(' ');
				last.push_str(std::str::from_utf8(&item).unwrap());
			},
			Op::End { stop_reason, .. } => self.0.push(format!("stop {stop_reason}")),
			Op::Fail(failure) => self.0.push(format!("fail {}", failure.message)),
			_ => {},
		}
	}
}

fn messages_blocks(parts: &[&str]) -> (Vec<String>, usize) {
	let mut compat = GeminiMessagesCompat::new(SCOPE, 1 << 20);
	let mut blocks = Blocks::default();
	let mut decoder = GeminiDecoder::new("m", "reply-1");
	for (at, part) in parts.iter().enumerate() {
		let finish = if at + 1 == parts.len() {
			r#","finishReason":"STOP""#
		} else {
			""
		};
		let payload = format!(r#"{{"candidates":[{{"content":{{"parts":[{part}]}}{finish}}}]}}"#);
		decoder
			.push_json(payload.as_bytes(), 1 << 20, &mut |event| {
				compat.event(&event, &mut blocks)
			})
			.unwrap();
	}
	decoder.finish().unwrap();
	(blocks.0, compat.dropped_text_signatures())
}

#[test]
fn a_late_text_signature_is_dropped_and_a_call_signature_kept() {
	let (blocks, dropped) = messages_blocks(&[
		r#"{"text":"Let me "}"#,
		r#"{"text":"look.","thoughtSignature":"text-sig"}"#,
		r#"{"functionCall":{"name":"f","args":{}},"thoughtSignature":"call-sig"}"#,
	]);
	assert_eq!(dropped, 1);
	assert_eq!(
		blocks,
		[
			"text",
			r#"thinking {"functionCall":{"name":"f","args":{}},"thoughtSignature":"call-sig"}"#,
			"tool f",
			"stop tool_use",
		]
	);
}

#[test]
fn a_signature_before_any_text_is_kept() {
	let (blocks, dropped) =
		messages_blocks(&[r#"{"text":"","thoughtSignature":"s"}"#, r#"{"text":"Hi"}"#]);
	assert_eq!(dropped, 0);
	assert_eq!(
		blocks,
		[
			r#"thinking {"text":"","thoughtSignature":"s"}"#,
			"text",
			"stop end_turn",
		]
	);
}

#[test]
fn a_signature_on_streamed_text_is_late() {
	let (blocks, dropped) = messages_blocks(&[r#"{"text":"Hi","thoughtSignature":"s"}"#]);
	assert_eq!(dropped, 1);
	assert_eq!(blocks, ["text", "stop end_turn"]);
}

#[test]
fn a_signed_thought_after_text_is_kept() {
	let (blocks, dropped) = messages_blocks(&[
		r#"{"text":"Hi"}"#,
		r#"{"text":"","thought":true,"thoughtSignature":"s"}"#,
	]);
	assert_eq!(dropped, 0);
	assert_eq!(
		blocks,
		[
			"text",
			r#"thinking {"text":"","thought":true,"thoughtSignature":"s"}"#,
			"stop end_turn",
		]
	);
}
