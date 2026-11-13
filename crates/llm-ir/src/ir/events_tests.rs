#![allow(clippy::unwrap_used)]

use std::fmt::Write;

use rstest::rstest;

use super::*;

fn raw(text: &str) -> Box<RawValue> {
	RawValue::from_string(text.to_string()).unwrap()
}

#[test]
fn text_that_cannot_be_decoded_is_not_a_json_string() {
	let lone = raw(r#""\uD800""#);
	assert!(JsonStr::new(&lone).is_none());
	assert!(matches!(
		JsonStr::read(&lone, "text"),
		Err(DecodeError::InvalidJson(_))
	));
}

#[test]
fn other_values_are_the_wrong_type() {
	let number = raw("1");
	assert!(JsonStr::new(&number).is_none());
	assert!(matches!(
		JsonStr::read(&number, "text"),
		Err(DecodeError::WrongType("text"))
	));
}

#[rstest]
#[case::empty(r#""""#, true)]
#[case::object(r#""{\"a\":1}""#, true)]
#[case::cut_off(r#""{""#, false)]
#[case::repeated_key(r#""{\"a\":1,\"a\":2}""#, false)]
fn only_strict_json_or_nothing_is_complete(#[case] text: &str, #[case] complete: bool) {
	let value = raw(text);
	assert_eq!(
		Arguments::complete(JsonStr::new(&value).unwrap()).is_some(),
		complete
	);
}

#[rstest]
fn plain_text_is_borrowed_as_serde_reads_it(
	#[values(r#""""#, r#""plain é 🦀""#, r#""a/b""#)] text: &str,
) {
	let value = raw(text);
	let decoded = JsonStr::new(&value).unwrap().decode();
	assert!(matches!(decoded, Cow::Borrowed(_)));
	assert_eq!(decoded, serde_json::from_str::<String>(text).unwrap());
}

#[test]
fn escaped_text_decodes() {
	let pair = raw(r#""😀 \\u \n""#);
	assert_eq!(JsonStr::new(&pair).unwrap().decode(), "\u{1F600} \\u \n");
}

mod decoder_tests {
	#![allow(clippy::unwrap_used)]

	use std::error::Error;

	use super::super::*;

	#[test]
	fn bad_json_keeps_its_cause() {
		let error: DecodeError = serde_json::from_str::<u8>("x").unwrap_err().into();
		let mut causes = std::iter::successors(error.source(), |cause| (*cause).source());
		assert!(causes.any(|cause| cause.is::<serde_json::Error>()));
	}

	#[test]
	fn messages_payload_limit_still_allows_draining_usage() {
		use crate::protocols::messages::decoder::MessagesDecoder;

		let mut decoder = MessagesDecoder::default();
		let limit = 128;
		let mut usage = None;
		let mut sink = |event: Event<'_>| {
			if let Event::Usage(value) = event {
				usage = Some(value);
			}
		};
		assert!(matches!(
			decoder.push_json(&vec![b' '; limit + 1], limit, &mut sink),
			Err(DecodeError::EventTooLarge { limit: 128 })
		));
		decoder
			.push_json(
				br#"{"type":"message_delta","usage":{"output_tokens":7}}"#,
				limit,
				&mut sink,
			)
			.unwrap();
		decoder
			.push_json(br#"{"type":"message_stop"}"#, limit, &mut sink)
			.unwrap();
		decoder.finish().unwrap();
		assert_eq!(usage.unwrap().output_tokens, Some(7));
	}

	#[test]
	fn gemini_accepts_a_buffered_payload_without_sse() {
		use crate::protocols::gemini::decoder::GeminiDecoder;

		let mut decoder = GeminiDecoder::new("model", "reply-1");
		let mut finished = false;
		decoder
			.push_json(
				br#"{"candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"STOP"}]}"#,
				1024,
				&mut |event| finished |= matches!(event, Event::Finish(_)),
			)
			.unwrap();
		decoder.finish().unwrap();
		assert!(finished);
	}

	#[test]
	fn completions_done_without_a_finish_reason_is_a_failure() {
		use crate::protocols::chat_completions::decoder::CompletionsDecoder;

		let mut decoder = CompletionsDecoder::new("model", "reply-1");
		assert!(decoder.push_json(b"[DONE]", 1024, &mut |_| {}).is_err());
	}

	#[test]
	fn responses_done_does_not_replace_a_terminal_event() {
		use crate::protocols::responses::decoder::ResponsesDecoder;

		let mut decoder = ResponsesDecoder::default();
		assert!(matches!(
			decoder.push_json(b"[DONE]", &mut |_| {}),
			Err(DecodeError::NoTerminalEvent)
		));
		assert!(!decoder.is_finished());
		assert!(matches!(
			decoder.finish(),
			Err(DecodeError::NoTerminalEvent)
		));
	}
}

mod observers_tests {

	use super::super::*;

	fn usage(input: Option<u64>, output: Option<u64>) -> Usage {
		Usage {
			input_tokens: input,
			output_tokens: output,
			..Usage::default()
		}
	}

	fn merged(all: &[Usage]) -> Usage {
		let mut fold = UsageFold::default();
		for usage in all {
			fold.merge(*usage);
		}
		fold.total()
	}

	#[test]
	fn no_usage_stays_unknown() {
		assert_eq!(merged(&[]), Usage::default());
		assert_eq!(merged(&[Usage::default()]).input_tokens, None);
	}

	#[test]
	fn zero_stays_different_from_unknown() {
		assert_eq!(merged(&[usage(Some(0), None)]).input_tokens, Some(0));
		assert_eq!(merged(&[usage(Some(0), None)]).output_tokens, None);
	}

	#[test]
	fn a_missing_value_never_erases_a_known_one() {
		let got = merged(&[usage(Some(3), Some(4)), usage(None, None)]);
		assert_eq!(got, usage(Some(3), Some(4)));
	}

	proptest::proptest! {
		#![proptest_config(proptest::test_runner::Config::with_cases(128))]

		#[test]
		fn usage_order_and_repeats_do_not_change_totals(
			counts in proptest::collection::vec(proptest::array::uniform7(proptest::option::of(proptest::prelude::any::<u64>())), 0..32),
		) {
			let values: Vec<_> = counts.iter().map(|v| Usage {
					input_tokens: v[0], output_tokens: v[1], total_tokens: v[2], cached_tokens: v[3],
					cache_write_tokens: v[4], cache_write_12h_tokens: v[5], reasoning_tokens: v[6],
			}).collect();
			let expected = merged(&values);
			let mut reversed = values.clone();
			reversed.reverse();
			reversed.extend_from_slice(&values);
			proptest::prop_assert_eq!(merged(&reversed), expected);
			for (index, actual) in [expected.input_tokens, expected.output_tokens, expected.total_tokens,
				expected.cached_tokens, expected.cache_write_tokens, expected.cache_write_12h_tokens,
				expected.reasoning_tokens].into_iter().enumerate() {
				proptest::prop_assert_eq!(actual, counts.iter().filter_map(|v| v[index]).max());
			}
		}
	}
}

#[rstest]
#[case::plain(r#""hello""#)]
#[case::short_escapes(r#""a\"b\\c\/d\n\r\t\b\f""#)]
#[case::one_byte_unicode(r#""A\u001f""#)]
#[case::two_byte_unicode(r#""é߿""#)]
#[case::three_byte_unicode(r#""ࠀ€￿""#)]
#[case::surrogate_pair(r#""x😀y""#)]
#[case::escaped_backslash_before_u(r#""\\u0041""#)]
#[case::literal_non_ascii(r#""é😀""#)]
fn decoded_len_counts_the_decoded_text(#[case] text: &str) {
	let value = raw(text);
	let text = JsonStr::read(&value, "text").unwrap();
	assert_eq!(text.decoded_len(), text.decode().len());
}

#[test]
fn a_lone_trailing_surrogate_is_not_a_json_string() {
	assert!(JsonStr::new(&raw(r#""\uDC00""#)).is_none());
}

proptest::proptest! {
	#[test]
	fn decoded_len_matches_decode(text in "\\PC{0,40}", spell_as_unicode in proptest::collection::vec(proptest::bool::ANY, 40)) {
		// Any spelling a provider may use: each character plain, short-escaped, or as `\u` units.
		let mut json = String::from('"');
		for (at, c) in text.chars().enumerate() {
			if spell_as_unicode[at] {
				for unit in c.encode_utf16(&mut [0; 2]) {
					write!(json, "\\u{unit:04x}").unwrap();
				}
			} else {
				let escaped = serde_json::to_string(&c.to_string()).unwrap();
				json.push_str(&escaped[1..escaped.len() - 1]);
			}
		}
		json.push('"');
		let value = raw(&json);
		let checked = JsonStr::read(&value, "text").unwrap();
		proptest::prop_assert_eq!(checked.decoded_len(), text.len());
		proptest::prop_assert_eq!(checked.decode(), text);
	}

	#[test]
	fn joined_strings_decode_to_joined_text(parts in proptest::collection::vec("\\PC{0,20}", 0..5)) {
		let values: Vec<_> = parts.iter().map(|part| raw(&serde_json::to_string(part).unwrap())).collect();
		let mut joined = OpenText::default();
		for value in &values {
			joined.push_json(JsonStr::read(value, "text").unwrap());
		}
		let decoded: String = serde_json::from_str(&joined.finish()).unwrap();
		proptest::prop_assert_eq!(decoded, parts.concat());
	}
}
