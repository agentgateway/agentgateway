#![allow(clippy::unwrap_used)]

use rstest::rstest;

use super::*;

const TEXT: &str = "the quick brown fox jumps over the lazy dog, then does it again";

#[rstest]
#[case::changed_summary(&["first"], "second", false)]
#[case::empty_final_summary(&["first"], "", false)]
#[case::empty_delta(&[""], "second", false)]
#[case::unicode_deltas(&["a ", "🦀"], "a 🦀", true)]
#[case::final_only(&[], "final only", true)]
fn reasoning_summaries_must_match_their_deltas(
	#[case] deltas: &[&str],
	#[case] summary: &str,
	#[case] agrees: bool,
) {
	use serde_json::json;

	let mut decoder = ResponsesDecoder::default();
	for event in [
		json!({"type":"response.created","response":{"id":"r","model":"m"}}),
		json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","summary":[]}}),
	] {
		decoder
			.push_json(event.to_string().as_bytes(), &mut |_| {})
			.unwrap();
	}
	for delta in deltas {
		let event =
			json!({"type":"response.reasoning_summary_text.delta","output_index":0,"delta":delta});
		decoder
			.push_json(event.to_string().as_bytes(), &mut |_| {})
			.unwrap();
	}
	let event = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","summary":[{"type":"summary_text","text":summary}],"encrypted_content":"opaque"}});
	let mut ended = false;
	let result = decoder.push_json(event.to_string().as_bytes(), &mut |event| {
		ended |= matches!(event, Event::PartEnd { .. })
	});
	if agrees {
		result.unwrap();
		assert!(ended);
	} else {
		assert!(matches!(
			result,
			Err(DecodeError::FinalDiffers("reasoning summary"))
		));
		assert!(!ended);
	}
}

#[test]
fn oversized_terminal_does_not_hide_late_output_after_a_long_key() {
	let terminal = format!(
		r#"{{"type":"response.completed","response":{{"output":[{{"type":"message","{}":0}},{{"type":"reasoning","encrypted_content":"secret"}}]}}}}"#,
		"x".repeat(33)
	);
	for size in [0, 1, 7, terminal.len()] {
		let mut decoder = ResponsesDecoder::default();
		for event in [
			br#"{"type":"response.created","response":{"id":"r","model":"m"}}"#.as_slice(),
			br#"{"type":"response.output_item.added","output_index":0,"item":{"type":"message"}}"#,
			br#"{"type":"response.output_item.done","output_index":0,"item":{"type":"message","content":[]}}"#,
		] {
			decoder.push_json(event, &mut |_| {}).unwrap();
		}
		let mut finished = false;
		let mut sink = |event: Event<'_>| finished |= matches!(event, Event::Finish(_));
		let result = if size == 0 {
			decoder.push_json(terminal.as_bytes(), &mut sink)
		} else {
			for piece in terminal.as_bytes().chunks(size) {
				decoder.push_json_piece(piece, 16).unwrap();
			}
			decoder.end_json_pieces(16, &mut sink)
		};
		assert!(
			matches!(result, Err(DecodeError::LateOutput("output"))),
			"chunk size {size}"
		);
		assert!(!finished);
	}
}

#[test]
fn terminal_pieces_match_whole_event_validation() {
	let valid = r#"{"typ\u0065":"response.completed","respon\u0073e":{"outp\u0075t":[],"instructions":"escaped \u0065 and raw é 🦀","usag\u0065":{"input_tokens":3,"output_tokens":7}}}"#.as_bytes().to_vec();
	let mut invalid = br#"{"type":"response.completed","response":{"instructions":""#.to_vec();
	invalid.extend_from_slice(b"\xff");
	invalid.extend_from_slice(br#"","usage":{"output_tokens":7}}}"#);
	let duplicate =
		br#"{"type":"response.completed","response":{"usage":{},"usag\u0065":{}}}"#.to_vec();
	for (input, accepts) in [(valid, true), (invalid, false), (duplicate, false)] {
		let mut expected = None;
		for size in [0, 1, 2, 7, input.len()] {
			let mut decoder = ResponsesDecoder::default();
			decoder
				.push_json(
					br#"{"type":"response.created","response":{"id":"r","model":"m"}}"#,
					&mut |_| {},
				)
				.unwrap();
			let mut events = Vec::new();
			let mut sink = |event: Event<'_>| events.push(format!("{event:?}"));
			let result = if size == 0 {
				decoder.push_json(&input, &mut sink)
			} else {
				input
					.chunks(size)
					.try_for_each(|piece| decoder.push_json_piece(piece, 16))
					.and_then(|_| decoder.end_json_pieces(16, &mut sink))
			};
			assert_eq!(result.is_ok(), accepts, "{result:?}, chunk size {size}");
			let outcome = (result.is_ok(), events, decoder.is_finished());
			if let Some(expected) = &expected {
				assert_eq!(&outcome, expected, "chunk size {size}");
			} else {
				expected = Some(outcome);
			}
		}
	}
}

#[test]
fn a_check_ignores_how_text_was_split() {
	let whole = Check::of(TEXT);
	for size in 1..=TEXT.len() {
		let mut split = Check::EMPTY;
		for piece in TEXT.as_bytes().chunks(size) {
			split.add(std::str::from_utf8(piece).unwrap());
			split.add("");
		}
		assert!(split == whole, "pieces of {size} bytes");
	}
}

#[test]
fn a_check_sees_a_change_at_any_byte() {
	let whole = Check::of(TEXT);
	for at in 0..TEXT.len() {
		let mut changed = TEXT.as_bytes().to_vec();
		changed[at] = b'#';
		let changed = String::from_utf8(changed).unwrap();
		assert!(Check::of(&changed) != whole, "byte {at}");
	}
	assert!(Check::of(&TEXT[..TEXT.len() - 1]) != whole);
}

#[test]
fn twelve_hour_cache_writes_are_read() {
	let body = r#"{"id":"r","model":"m","status":"completed","output":[],"usage":{
		"input_tokens":10,"output_tokens":1,"total_tokens":11,
		"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":8,
			"cache_write_tokens_details":{"ttl_12h_tokens":5}}}}"#;
	let mut usage = None;
	decode_buffered(body.as_bytes(), &mut |event| {
		if let Event::Usage(seen) = event {
			usage = Some(seen);
		}
	})
	.unwrap();
	let usage = usage.unwrap();
	assert_eq!(usage.cache_write_tokens, Some(8));
	assert_eq!(usage.cache_write_12h_tokens, Some(5));
}
