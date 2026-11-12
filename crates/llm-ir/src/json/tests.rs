#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn names_are_borrowed_unless_they_contain_an_escape() {
	let fields: Fields<'_> = serde_json::from_str(r#"{"type":1,"mod\u0065l":2}"#).unwrap();
	assert!(matches!(&fields.0[0].0, Cow::Borrowed("type")));
	assert!(matches!(&fields.0[1].0, Cow::Owned(name) if name == "model"));
}

#[test]
fn text_is_borrowed_unless_it_contains_an_escape() {
	let raw = |json: &str| RawValue::from_string(json.to_string()).unwrap();
	let (plain, escaped, number) = (raw(r#""a é""#), raw(r#""a\nb""#), raw("1"));
	assert!(matches!(text_of(&plain), Some(Cow::Borrowed("a é"))));
	assert!(matches!(text_of(&escaped), Some(Cow::Owned(text)) if text == "a\nb"));
	assert!(text_of(&number).is_none());
	assert!(text_of(&raw(r#""\ud800""#)).is_none());
}

mod strings_tests {
	use proptest::prelude::*;
	use serde_json::{Value, json};

	use super::super::{JsonRef, decode_string, replace_string_values, string_values};

	#[test]
	fn string_values_skip_keys_and_escaped_quotes() {
		let json = r#"{"a":{"b":["x","y:"],"c":"\"}"} , "d" : "e\\", "f":[1,"g"]}"#;
		let values: Vec<_> = string_values(json)
			.map(|range| decode_string(&json[range]))
			.collect();
		assert_eq!(values, ["x", "y:", "\"}", "e\\", "g"]);
	}

	#[test]
	fn only_replaced_strings_are_rewritten_and_escaped() {
		let json = r#"{"a": "x", "n": 1.50, "b": ["y"]}"#;
		let edited = replace_string_values(json, |at, text| (at == 1).then(|| format!("\"{text}\"")));
		assert_eq!(
			edited.as_deref(),
			Some(r#"{"a": "x", "n": 1.50, "b": ["\"y\""]}"#)
		);
		assert_eq!(replace_string_values(json, |_, _| None), None);
	}

	proptest! {
		#![proptest_config(ProptestConfig::with_cases(64))]

		#[test]
		fn raw_and_parsed_json_visit_and_edit_the_same_strings(
			rows in prop::collection::vec((any::<String>(), prop::collection::vec(any::<String>(), 0..8)), 0..8),
			replacement in any::<String>(),
		) {
			let value = Value::Object(rows.into_iter().map(|(key, texts)| {
				(key, json!({"texts": texts, "other": [null, true, 42]}))
			}).collect());
			let original = value.clone();
			let raw = serde_json::value::to_raw_value(&value).unwrap();
			let raw = JsonRef::Raw(&raw);
			let parsed = JsonRef::Parsed(&value);
			prop_assert_eq!(raw.strings().collect::<Vec<_>>(), parsed.strings().collect::<Vec<_>>());
			prop_assert!(parsed.replace_strings(|_, _| None).is_none());
			let edit = |at, _: std::borrow::Cow<'_, str>| (at % 2 == 0).then(|| replacement.clone());
			let raw = raw.replace_strings(edit).map(|value| serde_json::to_value(value).unwrap());
			let parsed = parsed.replace_strings(edit).map(|value| serde_json::to_value(value).unwrap());
			prop_assert_eq!(raw, parsed);
			prop_assert_eq!(value, original);
		}
	}
}

mod write_tests {
	use proptest::prelude::*;

	use super::super::*;

	proptest! {
		#![proptest_config(ProptestConfig::with_cases(128))]

		#[test]
		fn strings_are_written_as_serde_writes_them(
			text in prop::collection::vec(prop_oneof![any::<char>(),
				prop::sample::select(vec!['"', '\\', '\n', '\r', '\t', '\0', '\u{1f}', 'é', '🦀'])], 0..256),
			padding in 0..128usize,
		) {
			let text = format!("{}{}", "x".repeat(padding), text.into_iter().collect::<String>());
			let mut out = String::new();
			push_string(&mut out, &text);
			prop_assert_eq!(out, serde_json::to_string(&text).unwrap());
		}

	}
}
