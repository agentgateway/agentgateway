#![allow(clippy::unwrap_used)]

use rstest::rstest;

use super::*;

fn scan(input: &[u8]) -> Result<Scanned, DecodeError> {
	let mut scanner = TerminalScanner::new(16);
	scanner.push(input)?;
	scanner.finish()
}

fn event(response: &str) -> String {
	format!(r#"{{"type":"response.completed","response":{response}}}"#)
}

#[test]
fn keeps_what_the_decoder_needs_in_any_field_order() {
	let input = r#"{"response":{"usage":{"input_tokens":3},"output":[
		{"id":"a","type":"message","content":[{"text":"skipped \"quote\" é"}]},
		{"encrypted_content":"secret","type":"reasoning"},
		{"type":"reasoning","encrypted_content":null}
	]},"sequence_number":1.5e3,"extra":[true,false,null,-0.1],"type":"response.incomplete"}"#;
	let scanned = scan(input.as_bytes()).unwrap();
	assert_eq!(scanned.kind, Some(Terminal::Incomplete));
	assert_eq!(scanned.usage.unwrap().input_tokens, Some(3));
	let seen: Vec<_> = scanned
		.output
		.iter()
		.map(|item| (item.reasoning, item.has_state))
		.collect();
	assert_eq!(seen, [(false, false), (true, true), (true, false)]);
}

#[rstest]
fn long_keys_do_not_hide_later_output_items(#[values(MAX_KEY + 1, MAX_KEY * 6 + 1)] length: usize) {
	let input = event(&format!(
		r#"{{"output":[{{"type":"message","{}":0}},{{"type":"reasoning","encrypted_content":"secret"}}]}}"#,
		"x".repeat(length)
	));
	for size in [1, 7, input.len()] {
		let mut scanner = TerminalScanner::new(16);
		for piece in input.as_bytes().chunks(size) {
			scanner.push(piece).unwrap();
		}
		let scanned = scanner.finish().unwrap();
		assert_eq!(scanned.output.len(), 2);
		assert!(scanned.output[1].reasoning);
		assert!(scanned.output[1].has_state);
	}
}

#[test]
fn escaped_keys_are_decoded_before_matching() {
	let input = br#"{"typ\u0065":"response.completed","respon\u0073e":{"usag\u0065":{"output_tokens":7},"outp\u0075t":[{"ty\u0070e":"reasoning","encrypted_\u0063ontent":"secret"}]}}"#;
	for size in 1..=input.len() {
		let mut scanner = TerminalScanner::new(16);
		for piece in input.chunks(size) {
			scanner.push(piece).unwrap();
		}
		let scanned = scanner.finish().unwrap();
		assert_eq!(scanned.kind, Some(Terminal::Completed));
		assert_eq!(scanned.usage.unwrap().output_tokens, Some(7));
		assert_eq!(scanned.output.len(), 1);
		assert!(scanned.output[0].reasoning);
		assert!(scanned.output[0].has_state);
	}
}

#[rstest]
#[case::invalid_lead(b"\xff")]
#[case::unexpected_continuation(b"\x80")]
#[case::overlong(b"\xc0\xaf")]
#[case::surrogate(b"\xed\xa0\x80")]
#[case::out_of_range(b"\xf4\x90\x80\x80")]
#[case::truncated(b"\xe2\x82")]
fn skipped_strings_still_require_valid_utf8(#[case] invalid: &[u8]) {
	let mut input = br#"{"type":"response.completed","response":{"instructions":""#.to_vec();
	input.extend_from_slice(invalid);
	input.extend_from_slice(br#"","output":[]}}"#);
	assert!(serde_json::from_slice::<serde_json::Value>(&input).is_err());
	for at in 0..=input.len() {
		let mut scanner = TerminalScanner::new(16);
		let result = scanner
			.push(&input[..at])
			.and_then(|_| scanner.push(&input[at..]))
			.and_then(|_| scanner.finish());
		assert!(result.is_err(), "invalid {invalid:?}, split at {at}");
	}
}

#[test]
fn keeps_the_provider_outcome() {
	let scanned = scan(
		br#"{"type":"response.failed","response":{"error":{"code":"c","message":"m"},"incomplete_details":{"reason":"r"}}}"#,
	)
	.unwrap();
	assert_eq!(scanned.code.as_deref(), Some("c"));
	assert_eq!(scanned.message.as_deref(), Some("m"));
	assert_eq!(scanned.reason.as_deref(), Some("r"));
}

#[test]
fn null_values_mean_absent() {
	let input = event(r#"{"usage":null,"error":null,"incomplete_details":null,"output":[]}"#);
	assert!(scan(input.as_bytes()).unwrap().usage.is_none());
}

#[test]
fn split_at_every_byte_gives_the_same_result() {
	let escaped = concat!("\\", "u0041");
	let input = event(&format!(
		r#"{{"usage":{{"output_tokens":7}},"output":[{{"type":"message","x":"é 🦀 {escaped}"}}]}}"#
	));
	for at in 0..input.len() {
		let (left, right) = input.as_bytes().split_at(at);
		let mut scanner = TerminalScanner::new(16);
		scanner.push(left).unwrap();
		scanner.push(right).unwrap();
		let scanned = scanner.finish().unwrap();
		assert_eq!(
			scanned.usage.unwrap().output_tokens,
			Some(7),
			"split at {at}"
		);
		assert_eq!(scanned.output.len(), 1);
	}
}

#[test]
fn other_events_are_too_large() {
	let got = scan(br#"{"type":"response.output_item.done","item":{}}"#);
	assert!(matches!(got, Err(DecodeError::EventTooLarge { limit: 16 })));
}

#[rstest]
#[case::empty("")]
#[case::truncated_object("{")]
#[case::missing_value(r#"{"a":}"#)]
#[case::trailing_object_comma(r#"{"a":1,}"#)]
#[case::missing_colon(r#"{"a" 1}"#)]
#[case::unquoted_key(r#"{a:1}"#)]
#[case::leading_zero(r#"{"a":01}"#)]
#[case::bare_minus(r#"{"a":-}"#)]
#[case::missing_fraction(r#"{"a":1.}"#)]
#[case::missing_exponent(r#"{"a":1e}"#)]
#[case::truncated_true(r#"{"a":tru}"#)]
#[case::truncated_null(r#"{"a":nul}"#)]
#[case::invalid_escape(r#"{"a":"\q"}"#)]
#[case::invalid_unicode_escape(r#"{"a":"\u12g4"}"#)]
#[case::literal_newline("{\"a\":\"\n\"}")]
#[case::trailing_array_comma(r#"{"a":[1,]}"#)]
#[case::unclosed_array(r#"{"a":[1}"#)]
#[case::mismatched_close(r#"{"a":{"b":1]}"#)]
#[case::extra_close(r#"{"a":1}}"#)]
#[case::trailing_data(r#"{"a":1} x"#)]
fn rejects_what_serde_json_rejects(#[case] body: &str) {
	assert!(
		serde_json::from_str::<serde_json::Value>(body).is_err(),
		"{body}"
	);
	assert!(scan(body.as_bytes()).is_err(), "{body}");
}

#[test]
fn requires_type_and_response() {
	assert!(matches!(
		scan(br#"{"response":{}}"#),
		Err(DecodeError::WrongType("type"))
	));
	assert!(matches!(
		scan(br#"{"type":"response.completed"}"#),
		Err(DecodeError::WrongType("response"))
	));
}

#[rstest]
#[case::response_list(event("[]"))]
#[case::output_object(event(r#"{"output":{}}"#))]
#[case::non_object_item(event(r#"{"output":[1]}"#))]
#[case::missing_item_type(event(r#"{"output":[{"id":"a"}]}"#))]
#[case::repeated_output(event(r#"{"output":[],"output":[]}"#))]
#[case::repeated_item_type(event(r#"{"output":[{"type":"a","type":"b"}]}"#))]
#[case::escaped_duplicate_usage(event(r#"{"usage":{},"usag\u0065":{}}"#))]
#[case::escaped_duplicate_type(r#"{"type":"response.completed","typ\u0065":"response.completed","response":{}}"#.into())]
fn rejects_fields_it_would_otherwise_misread(#[case] input: String) {
	assert!(scan(input.as_bytes()).is_err(), "{input}");
}

#[test]
fn nesting_is_limited() {
	let input = format!(r#"{{"a":{}{}}}"#, "[".repeat(200), "]".repeat(200));
	assert!(scan(input.as_bytes()).is_err());
}

#[test]
fn a_large_kept_value_is_refused() {
	let input = event(&format!(
		r#"{{"error":{{"message":"{}"}}}}"#,
		"x".repeat(MAX_KEPT_VALUE)
	));
	assert!(scan(input.as_bytes()).is_err());
}

#[test]
fn the_skipped_output_takes_no_memory() {
	let mut scanner = TerminalScanner::new(16);
	scanner
		.push(br#"{"type":"response.completed","response":{"output":[{"type":"message","text":""#)
		.unwrap();
	let block = [b'x'; 4096];
	for _ in 0..256 {
		scanner.push(&block).unwrap();
	}
	scanner.push(br#""}]}}"#).unwrap();
	assert!(scanner.kept.capacity() < 256);
	assert_eq!(scanner.finish().unwrap().output.len(), 1);
}
