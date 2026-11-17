#![allow(clippy::panic, clippy::unwrap_used)]

use std::borrow::Cow;

use rstest::rstest;

use super::read_model;

fn model(body: &str) -> Option<Cow<'_, str>> {
	read_model(body.as_bytes()).unwrap_or_else(|error| panic!("{body}: {error}"))
}

#[test]
fn the_top_level_model_is_read_wherever_it_is() {
	assert_eq!(
		model(r#"{"model":"m","messages":[]}"#).as_deref(),
		Some("m")
	);
	let spaced = " {\n\t\"messages\" : [ ] ,\r\n \"model\"\t:\t\"m\" } \n";
	assert_eq!(model(spaced).as_deref(), Some("m"));
}

#[test]
fn a_plain_name_stays_borrowed() {
	assert!(matches!(
		model(r#"{"model":"m"}"#),
		Some(Cow::Borrowed("m"))
	));
}

#[test]
fn escapes_are_decoded_in_the_name_and_the_key() {
	assert_eq!(model(r#"{"model":"gpt\u002d5"}"#).as_deref(), Some("gpt-5"));
	assert_eq!(model(r#"{"mod\u0065l":"m"}"#).as_deref(), Some("m"));
	assert_eq!(
		model(r#"{"model":"a\"b\\c\nd"}"#).as_deref(),
		Some("a\"b\\c\nd")
	);
	assert_eq!(
		model(r#"{"model":"\ud83d\ude00"}"#).as_deref(),
		Some("\u{1f600}")
	);
}

#[test]
fn a_missing_or_null_model_is_none() {
	assert_eq!(model(r#"{"messages":[]}"#), None);
	assert_eq!(model(r#"{"model":null,"messages":[]}"#), None);
	assert_eq!(model("{}"), None);
}

#[test]
fn a_nested_model_is_not_the_request_model() {
	assert_eq!(model(r#"{"metadata":{"model":"x"},"messages":[]}"#), None);
	let both = r#"{"tools":[{"model":"x"}],"model":"m","messages":[{"model":"y"}]}"#;
	assert_eq!(model(both).as_deref(), Some("m"));
}

#[rstest]
#[case::number(r#"{"model":5}"#)]
#[case::boolean(r#"{"model":true}"#)]
#[case::object(r#"{"model":{"name":"m"}}"#)]
#[case::list(r#"{"model":["m"]}"#)]
#[case::repeated(r#"{"model":"a","model":"b"}"#)]
#[case::repeated_as_null(r#"{"model":"a","messages":[],"model":null}"#)]
#[case::repeated_with_an_escaped_key(r#"{"model":"a","mod\u0065l":"b"}"#)]
#[case::lone_surrogate(r#"{"model":"\ud800"}"#)]
#[case::trailing_text(r#"{"model":"m"} x"#)]
#[case::unterminated(r#"{"model":"m""#)]
#[case::not_an_object(r#"["m"]"#)]
#[case::a_string(r#""m""#)]
#[case::empty("")]
fn bodies_a_typed_read_refuses_are_refused(#[case] body: &str) {
	assert!(read_model(body.as_bytes()).is_err());
}
