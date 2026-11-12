#![allow(clippy::panic, clippy::unwrap_used)]

use rstest::rstest;

use super::*;
use crate::ir::events::Arguments;
use crate::sse::{Frame, SseParser};

const LIMIT: usize = 1 << 16;

/// Each event as one line, with text decoded, so a test reads what a consumer gets.
fn describe(event: &Event<'_>) -> String {
	match event {
		Event::ResponseStart { id, model } => format!("start {id} {model}"),
		Event::PartStart { part, kind } => match kind {
			PartKind::ToolCall { call_id, name } => format!("call {} {call_id} {name}", part.0),
			kind => format!("open {} {kind:?}", part.0),
		},
		Event::Delta { part, text } => format!("delta {} {}", part.0, text.decode()),
		Event::PartEnd { part, value } => match value {
			Final::Text => format!("end {} text", part.0),
			Final::Refusal(text) => format!("end {} refusal {}", part.0, text.decode()),
			Final::ToolCall(Arguments::Complete(_)) => format!("end {} call complete", part.0),
			Final::ToolCall(arguments) => format!("end {} call partial {}", part.0, arguments.raw()),
			Final::Reasoning {
				text,
				encrypted,
				item,
			} => format!(
				"end {} reasoning {:?} {:?} {}",
				part.0,
				text.map(|text| text.decode().into_owned()),
				encrypted.map(|value| value.decode().into_owned()),
				item.get()
			),
		},
		Event::Usage(usage) => format!(
			"usage {:?} {:?} {:?} {:?} {:?}",
			usage.input_tokens,
			usage.output_tokens,
			usage.total_tokens,
			usage.cached_tokens,
			usage.reasoning_tokens
		),
		Event::Finish(outcome) => format!("finish {outcome:?}"),
		Event::Error(error) => format!("error {:?} {:?}", error.kind, error.code),
		Event::ServiceTier(tier) => format!("tier {tier}"),
		Event::StopSequence(sequence) => format!("stop sequence {sequence}"),
		Event::Citation { .. } | Event::MessageEnd { .. } => unreachable!("not decoded here"),
	}
}

/// An SSE stream of these payloads.
fn stream(payloads: &[&str]) -> Vec<u8> {
	payloads
		.iter()
		.flat_map(|data| ["data: ", *data, "\n\n"])
		.collect::<String>()
		.into_bytes()
}

struct Decoded {
	events: Vec<String>,
	errors: Vec<String>,
	finished: bool,
}

/// Feeds `stream` in pieces of `chunk` bytes, as a caller would.
fn decode_in(stream: &[u8], chunk: usize, limit: usize) -> Decoded {
	let mut decoder = CompletionsDecoder::new("requested", "reply-1");
	let mut sse = SseParser::new(usize::MAX);
	let (mut events, mut errors) = (Vec::new(), Vec::new());
	for piece in stream.chunks(chunk) {
		sse.push(piece);
		while let Some(Frame::Event(event)) = sse.next_frame() {
			let pushed = decoder.push_json(event.data, limit, &mut |event| {
				events.push(describe(&event));
			});
			if let Err(error) = pushed {
				errors.push(error.to_string());
			}
		}
	}
	if let Err(error) = decoder.finish() {
		errors.push(error.to_string());
	}
	Decoded {
		events,
		errors,
		finished: decoder.is_finished(),
	}
}

fn decode(payloads: &[&str]) -> Vec<String> {
	let decoded = decode_in(&stream(payloads), usize::MAX, LIMIT);
	assert_eq!(decoded.errors, Vec::<String>::new());
	assert!(decoded.finished);
	decoded.events
}

fn buffered(body: &str) -> Result<Vec<String>, DecodeError> {
	let mut events = Vec::new();
	decode_buffered(body.as_bytes(), "requested", "reply-1", &mut |event| {
		events.push(describe(&event));
	})?;
	Ok(events)
}

fn chunk(delta: &str, finish: &str) -> String {
	format!(
		r#"{{"id":"c1","model":"m","choices":[{{"index":0,"delta":{delta},"finish_reason":{finish}}}]}}"#
	)
}

const USAGE: &str = r#"{"id":"c1","model":"m","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":4},"completion_tokens_details":{"reasoning_tokens":2}}}"#;
const USAGE_EVENT: &str = "usage Some(10) Some(5) Some(15) Some(4) Some(2)";

fn text_stream() -> Vec<String> {
	vec![
		chunk(r#"{"role":"assistant","content":""}"#, "null"),
		chunk(r#"{"content":"Hel"}"#, "null"),
		chunk(r#"{"content":"lo é"}"#, "null"),
		chunk("{}", r#""stop""#),
		USAGE.to_string(),
		DONE.to_string(),
	]
}

fn refs(payloads: &[String]) -> Vec<&str> {
	payloads.iter().map(String::as_str).collect()
}

#[test]
fn text_streams_and_ends_at_done_after_usage() {
	assert_eq!(
		decode(&refs(&text_stream())),
		[
			"start c1 m",
			"open 0 Text",
			"delta 0 Hel",
			"delta 0 lo é",
			"end 0 text",
			USAGE_EVENT,
			"finish Completed",
		]
	);
}

#[test]
fn a_buffered_reply_gives_the_streams_events() {
	let body = r#"{"id":"c1","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"Hello é"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":4},"completion_tokens_details":{"reasoning_tokens":2}}}"#;
	assert_eq!(
		buffered(body).unwrap(),
		[
			"start c1 m",
			USAGE_EVENT,
			"open 0 Text",
			"delta 0 Hello é",
			"end 0 text",
			"finish Completed",
		]
	);
}

#[test]
fn a_buffered_reply_without_a_finish_reason_fails() {
	let body = r#"{"id":"c1","choices":[{"index":0,"message":{"content":"Hi"}}]}"#;
	assert!(matches!(buffered(body), Err(DecodeError::NoTerminalEvent)));
}

#[test]
fn a_missing_id_or_model_is_filled_in() {
	let events = decode(&[
		r#"{"choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"stop"}]}"#,
		DONE,
	]);
	assert_eq!(events[0], "start reply-1 requested");
}

#[rstest]
#[case::deepseek("reasoning_content")]
#[case::openrouter("reasoning")]
fn reasoning_keeps_the_field_it_came_in(#[case] field: &str) {
	let events = decode(&[
		&chunk(&format!(r#"{{"{field}":"Let me "}}"#), "null"),
		&chunk(&format!(r#"{{"{field}":"think"}}"#), "null"),
		&chunk(r#"{"content":"Answer"}"#, r#""stop""#),
		DONE,
	]);
	assert_eq!(
		events,
		[
			"start c1 m".to_string(),
			"open 0 Reasoning".to_string(),
			"delta 0 Let me ".to_string(),
			"delta 0 think".to_string(),
			format!(r#"end 0 reasoning Some("Let me think") None {{"{field}":"Let me think"}}"#),
			"open 1 Text".to_string(),
			"delta 1 Answer".to_string(),
			"end 1 text".to_string(),
			"finish Completed".to_string(),
		]
	);
}

#[test]
fn a_reasoning_signature_stays_with_its_reasoning() {
	let events = decode(&[
		&chunk(r#"{"reasoning_content":"hm"}"#, "null"),
		&chunk(r#"{"reasoning_signature":"sig"}"#, "null"),
		&chunk(r#"{"content":"ok"}"#, r#""stop""#),
		DONE,
	]);
	assert_eq!(
		events[3],
		r#"end 0 reasoning Some("hm") Some("sig") {"reasoning_content":"hm","reasoning_signature":"sig"}"#
	);
}

#[test]
fn withheld_reasoning_is_a_signature_without_text() {
	let events = decode(&[
		&chunk(r#"{"reasoning_signature":"sig"}"#, "null"),
		&chunk(r#"{"content":"ok"}"#, r#""stop""#),
		DONE,
	]);
	assert_eq!(
		events[1..3],
		[
			"open 0 Reasoning",
			r#"end 0 reasoning None Some("sig") {"reasoning_content":"","reasoning_signature":"sig"}"#,
		]
	);
}

fn call(index: u32, rest: &str) -> String {
	chunk(
		&format!(r#"{{"tool_calls":[{{"index":{index}{rest}}}]}}"#),
		"null",
	)
}

fn tool_stream() -> Vec<String> {
	vec![
		chunk(r#"{"role":"assistant","content":null}"#, "null"),
		call(
			0,
			r#","id":"call_a","type":"function","function":{"name":"read","arguments":""}"#,
		),
		call(
			1,
			r#","id":"call_b","type":"function","function":{"name":"list","arguments":"{}"}"#,
		),
		call(0, r#","function":{"arguments":"{\"pa"}"#),
		call(0, r#","function":{"arguments":"th\":\"a\"}"}"#),
		chunk("{}", r#""tool_calls""#),
		USAGE.to_string(),
		DONE.to_string(),
	]
}

#[test]
fn tool_calls_are_assembled_by_index_and_end_together() {
	assert_eq!(
		decode(&refs(&tool_stream())),
		[
			"start c1 m",
			"call 0 call_a read",
			"call 1 call_b list",
			"delta 1 {}",
			"delta 0 {\"pa",
			"delta 0 th\":\"a\"}",
			"end 0 call complete",
			"end 1 call complete",
			USAGE_EVENT,
			"finish Completed",
		]
	);
}

#[test]
fn arguments_before_a_name_go_out_once_the_call_starts() {
	let events = decode(&[
		&call(0, r#","id":"call_a","function":{"arguments":"{\"a\":"}"#),
		&call(0, r#","function":{"name":"read","arguments":"1}"}"#),
		&chunk("{}", r#""tool_calls""#),
		DONE,
	]);
	assert_eq!(
		events[1..],
		[
			"call 0 call_a read",
			"delta 0 {\"a\":1}",
			"end 0 call complete",
			"finish Completed",
		]
	);
}

#[test]
fn a_call_without_an_id_gets_one_from_the_reply() {
	let events = decode(&[
		&call(0, r#","function":{"name":"read","arguments":"{}"}"#),
		&chunk("{}", r#""tool_calls""#),
		DONE,
	]);
	assert_eq!(events[1], "call 0 call_c1_0 read");
}

#[test]
fn a_call_that_never_got_a_name_fails() {
	let decoded = decode_in(
		&stream(&[
			&call(0, r#","function":{"arguments":"{}"}"#),
			&chunk("{}", r#""tool_calls""#),
		]),
		usize::MAX,
		LIMIT,
	);
	assert_eq!(decoded.errors[0], "wrong type: tool call without a name");
}

#[test]
fn a_call_cut_off_by_the_limit_is_partial() {
	let events = decode(&[
		&call(
			0,
			r#","id":"call_a","function":{"name":"read","arguments":"{\"pa"}"#,
		),
		&chunk("{}", r#""length""#),
		DONE,
	]);
	assert_eq!(
		events[3..],
		[
			r#"end 0 call partial "{\"pa""#,
			r#"finish Incomplete { reason: MaxTokens, raw: Some("length") }"#,
		]
	);
}

#[test]
fn text_before_calls_closes_when_they_start() {
	let events = decode(&[
		&chunk(r#"{"content":"Let me look."}"#, "null"),
		&call(
			0,
			r#","id":"call_a","function":{"name":"read","arguments":"{}"}"#,
		),
		&chunk("{}", r#""tool_calls""#),
		DONE,
	]);
	assert_eq!(
		events[1..5],
		[
			"open 0 Text",
			"delta 0 Let me look.",
			"end 0 text",
			"call 1 call_a read",
		]
	);
}

#[rstest]
#[case::stop("stop", "finish Completed")]
#[case::tool_calls("tool_calls", "finish Completed")]
#[case::function_call("function_call", "finish Completed")]
#[case::length(
	"length",
	r#"finish Incomplete { reason: MaxTokens, raw: Some("length") }"#
)]
#[case::content_filter(
	"content_filter",
	r#"finish Incomplete { reason: ContentFilter, raw: Some("content_filter") }"#
)]
#[case::unknown("eos", r#"error Unknown Some("eos")"#)]
fn finish_reasons(#[case] reason: &str, #[case] wanted: &str) {
	let events = decode(&[
		&chunk(r#"{"content":"x"}"#, &format!(r#""{reason}""#)),
		DONE,
	]);
	assert_eq!(events.last().unwrap(), wanted);
}

#[rstest]
#[case::vllm("stop_reason")]
#[case::sglang("matched_stop")]
fn an_engine_can_name_the_stop_sequence(#[case] field: &str) {
	let payload = format!(
		r#"{{"id":"c1","choices":[{{"index":0,"delta":{{"content":"x"}},"finish_reason":"stop","{field}":"END"}}]}}"#
	);
	let events = decode(&[&payload, DONE]);
	assert_eq!(
		events[events.len() - 2..],
		["stop sequence END", "finish Completed"]
	);
	// A token number is not a sequence the client asked for.
	let payload = payload.replace(r#""END""#, "151643");
	let events = decode(&[&payload, DONE]);
	assert_eq!(events.last().unwrap(), "finish Completed");
	assert!(
		!events
			.iter()
			.any(|event| event.starts_with("stop sequence"))
	);
}

#[test]
fn usage_may_be_missing_or_come_with_the_last_choice() {
	let events = decode(&[&chunk(r#"{"content":"x"}"#, r#""stop""#), DONE]);
	assert!(!events.iter().any(|event| event.starts_with("usage")));
	assert_eq!(events.last().unwrap(), "finish Completed");

	let with_usage = r#"{"id":"c1","choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1},"service_tier":"default"}"#;
	let events = decode(&[with_usage, DONE]);
	assert_eq!(
		events[1..3],
		["tier default", "usage Some(3) Some(1) None None None"]
	);
}

#[test]
fn anthropic_style_cache_counts_are_read() {
	let usage = r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1,"cache_read_input_tokens":2,"cache_creation_input_tokens":1}}"#;
	let mut seen = None;
	let mut decoder = CompletionsDecoder::new("m", "reply-1");
	decoder
		.push_json(usage.as_bytes(), LIMIT, &mut |event| {
			if let Event::Usage(usage) = event {
				seen = Some(usage);
			}
		})
		.unwrap();
	let seen = seen.unwrap();
	assert_eq!(
		(seen.cached_tokens, seen.cache_write_tokens),
		(Some(2), Some(1))
	);
}

#[rstest]
#[case::reasoning_left_out(r#"{"prompt_tokens":19,"completion_tokens":30,"total_tokens":69,"completion_tokens_details":{"reasoning_tokens":20}}"#, Some(50))]
#[case::thoughts_hidden(
	r#"{"prompt_tokens":7,"completion_tokens":12,"total_tokens":402}"#,
	Some(395)
)]
#[case::openai(
	r#"{"prompt_tokens":17,"completion_tokens":23,"total_tokens":40}"#,
	Some(23)
)]
#[case::no_total(r#"{"prompt_tokens":17,"completion_tokens":23}"#, Some(23))]
#[case::only_total(r#"{"prompt_tokens":17,"total_tokens":40}"#, Some(23))]
#[case::nothing(r#"{"prompt_tokens":17}"#, None)]
fn output_counts_tokens_an_engine_left_out_of_completion_tokens(
	#[case] usage: &str,
	#[case] output: Option<u64>,
) {
	// Gemini's OpenAI-compatible endpoint counts reasoning only in `total_tokens`.
	let payload = format!(r#"{{"choices":[],"usage":{usage}}}"#);
	let mut seen = None;
	CompletionsDecoder::new("m", "reply-1")
		.push_json(payload.as_bytes(), LIMIT, &mut |event| {
			if let Event::Usage(usage) = event {
				seen = Some(usage.output_tokens);
			}
		})
		.unwrap();
	assert_eq!(seen, Some(output));
}

#[test]
fn the_stream_must_end_with_done_after_a_finish_reason() {
	let cut = decode_in(
		&stream(&[&chunk(r#"{"content":"x"}"#, r#""stop""#)]),
		usize::MAX,
		LIMIT,
	);
	assert_eq!(cut.errors, ["stream ended with no terminal event"]);
	assert!(!cut.events.iter().any(|event| event.starts_with("finish")));

	let early = decode_in(
		&stream(&[&chunk(r#"{"content":"x"}"#, "null"), DONE]),
		usize::MAX,
		LIMIT,
	);
	assert_eq!(early.errors[0], "stream ended with no terminal event");
}

#[test]
fn nothing_may_follow_done() {
	let mut payloads = text_stream();
	payloads.push(chunk(r#"{"content":"late"}"#, "null"));
	let decoded = decode_in(&stream(&refs(&payloads)), usize::MAX, LIMIT);
	// The caller stops at `is_finished`, so a well-behaved one never sends it.
	assert!(decoded.finished);
	assert_eq!(decoded.errors, ["out of order: chunk after the end"]);
}

#[test]
fn a_choice_after_the_finish_reason_fails() {
	let decoded = decode_in(
		&stream(&[
			&chunk(r#"{"content":"x"}"#, r#""stop""#),
			&chunk(r#"{"content":"y"}"#, "null"),
			DONE,
		]),
		usize::MAX,
		LIMIT,
	);
	assert_eq!(
		decoded.errors[0],
		"out of order: choice after finish_reason"
	);
}

#[test]
fn more_than_one_choice_is_unsupported() {
	let two = r#"{"id":"c1","choices":[{"index":0,"delta":{"content":"a"}},{"index":1,"delta":{"content":"b"}}]}"#;
	let decoded = decode_in(&stream(&[two, DONE]), usize::MAX, LIMIT);
	assert_eq!(decoded.errors[0], "unsupported output item choices");
	let second = r#"{"id":"c1","choices":[{"index":1,"delta":{"content":"b"}}]}"#;
	let decoded = decode_in(&stream(&[second]), usize::MAX, LIMIT);
	assert_eq!(decoded.errors[0], "unsupported output item choices");
}

#[test]
fn a_filter_report_before_the_reply_is_skipped() {
	let filters = r#"{"id":"","choices":[],"prompt_filter_results":[{"prompt_index":0}]}"#;
	let events = decode(&[filters, &chunk(r#"{"content":"x"}"#, r#""stop""#), DONE]);
	assert_eq!(events[0], "start c1 m");
}

#[test]
fn a_refusal_ends_with_its_whole_text() {
	let events = decode(&[
		&chunk(r#"{"refusal":"I can't"}"#, "null"),
		&chunk(r#"{"refusal":" help."}"#, r#""stop""#),
		DONE,
	]);
	assert_eq!(
		events[1..5],
		[
			"open 0 Refusal",
			"delta 0 I can't",
			"delta 0  help.",
			"end 0 refusal I can't help.",
		]
	);
}

#[test]
fn an_error_in_the_stream_ends_it() {
	let error = r#"{"error":{"message":"This model's maximum context length is 8192 tokens","type":"invalid_request_error","code":null}}"#;
	let decoded = decode_in(
		&stream(&[&chunk(r#"{"content":"x"}"#, "null"), error]),
		usize::MAX,
		LIMIT,
	);
	assert!(decoded.finished);
	assert_eq!(decoded.errors, Vec::<String>::new());
	assert_eq!(decoded.events.last().unwrap(), "error ContextTooLong None");
}

#[test]
fn after_a_decode_error_only_usage_is_read() {
	let decoded = decode_in(
		&stream(&[
			&chunk(r#"{"content":"x"}"#, "null"),
			"not json",
			&chunk(r#"{"content":"lost"}"#, r#""stop""#),
			USAGE,
			DONE,
		]),
		usize::MAX,
		LIMIT,
	);
	assert_eq!(decoded.errors.len(), 1, "{:?}", decoded.errors);
	assert!(decoded.finished);
	let after: Vec<&String> = decoded.events.iter().skip(3).collect();
	assert_eq!(after, [USAGE_EVENT]);
}

#[test]
fn a_payload_or_part_over_the_limit_fails() {
	let long = chunk(&format!(r#"{{"content":"{}"}}"#, "x".repeat(200)), "null");
	let decoded = decode_in(&stream(&[&long]), usize::MAX, 64);
	assert_eq!(
		decoded.errors[0],
		"non-terminal event over the 64 byte limit"
	);

	// Each payload fits; the part they build doesn't.
	let piece = chunk(&format!(r#"{{"content":"{}"}}"#, "x".repeat(40)), "null");
	let decoded = decode_in(&stream(&[piece.as_str(); 4]), usize::MAX, piece.len());
	assert_eq!(
		decoded.errors[0],
		format!("non-terminal event over the {} byte limit", piece.len())
	);
	let named = call(0, r#","id":"a","function":{"name":"f"}"#);
	let arguments = call(
		0,
		&format!(r#","function":{{"arguments":"{}"}}"#, "y".repeat(40)),
	);
	let mut payloads = vec![named.as_str()];
	payloads.extend([arguments.as_str(); 5]);
	let decoded = decode_in(&stream(&payloads), usize::MAX, arguments.len());
	assert_eq!(
		decoded.errors[0],
		format!("non-terminal event over the {} byte limit", arguments.len())
	);
}

#[test]
fn a_custom_tool_call_is_unsupported() {
	let custom = chunk(
		r#"{"tool_calls":[{"index":0,"id":"a","type":"custom","custom":{"name":"f","input":"x"}}]}"#,
		"null",
	);
	let decoded = decode_in(&stream(&[&custom]), usize::MAX, LIMIT);
	assert_eq!(
		decoded.errors[0],
		"unsupported output item custom tool call"
	);
}

#[rstest]
#[case::code(
	r#"{"error":{"code":"context_length_exceeded","message":"x"}}"#,
	ErrorKind::ContextTooLong
)]
#[case::wording(
	r#"{"error":{"message":"Input tokens exceed the configured limit of 272000 tokens.","type":"invalid_request_error"}}"#,
	ErrorKind::ContextTooLong
)]
#[case::numeric_code(
	r#"{"error":{"code":429,"type":"rate_limit_error","message":"slow"}}"#,
	ErrorKind::RateLimit
)]
#[case::quota(
	r#"{"error":{"code":"insufficient_quota","message":"x"}}"#,
	ErrorKind::Billing
)]
#[case::model(
	r#"{"error":{"code":"model_not_found","message":"x"}}"#,
	ErrorKind::NotFound
)]
#[case::key(
	r#"{"error":{"code":"invalid_api_key","message":"x"}}"#,
	ErrorKind::Authentication
)]
#[case::overloaded(
	r#"{"error":{"type":"overloaded_error","message":"x"}}"#,
	ErrorKind::Overloaded
)]
#[case::invalid(
	r#"{"error":{"type":"invalid_request_error","message":"x"}}"#,
	ErrorKind::InvalidRequest
)]
#[case::html("<html>Bad Gateway</html>", ErrorKind::Unknown)]
#[case::other_shape(r#"{"detail":"Not Found"}"#, ErrorKind::Unknown)]
fn http_error_bodies(#[case] body: &str, #[case] kind: ErrorKind) {
	assert_eq!(http_error_kind(body.as_bytes()), kind);
}

#[test]
fn an_http_error_names_the_field_it_refused() {
	let body = br#"{"error":{"message":"Unsupported value","type":"invalid_request_error","param":"temperature","code":null}}"#;
	assert_eq!(http_error_field(body).as_deref(), Some("temperature"));
	assert_eq!(http_error_field(b"{}"), None);
}
