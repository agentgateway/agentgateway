#![allow(clippy::panic, clippy::unwrap_used)]

use std::borrow::Cow;

use rstest::rstest;

use super::*;
use crate::prompt::{self, RequestRefusal as Refusal, Upstream};
use crate::protocols::{ClientProtocol, Read, UpstreamProtocol};
use crate::test_support::{self, SCOPE, Sum};

const CONFIG: Upstream<'static> = Upstream {
	identifier_pseudonymizer: &Sum,
	scope: SCOPE,
	protocol: UpstreamProtocol::Responses {
		safety_identifier: true,
		reasoning_context: true,
	},
	default_max_tokens: None,
};
const HEAD: &str = r#""model":"m","max_tokens":1"#;

/// Checks both ways of reading, and returns what they found.
fn problems(body: &str) -> Vec<String> {
	assert!(
		prompt::one_pass_parse_agrees(body.as_bytes(), ClientProtocol::Messages, CONFIG),
		"{body}"
	);
	match test_support::convert(body.as_bytes(), ClientProtocol::Messages, CONFIG) {
		Ok(_) => Vec::new(),
		Err(Refusal { problems }) => problems,
	}
}

fn with_messages(messages: &str) -> String {
	format!(r#"{{{HEAD},"messages":{messages}}}"#)
}

#[test]
fn fields_may_come_in_any_order() {
	let result = r#"{"content":"out","tool_use_id":"c","type":"tool_result"}"#;
	let call = r#"{"input":{},"name":"f","id":"c","type":"tool_use"}"#;
	let messages = format!(
		r#"[{{"content":"hi","role":"user"}},{{"content":[{call}],"role":"assistant"}},{{"content":[{result}],"role":"user"}}]"#
	);
	assert_eq!(problems(&with_messages(&messages)), Vec::<String>::new());
	let first = format!(r#"{{"messages":{messages},{HEAD}}}"#);
	let body = test_support::convert(first.as_bytes(), ClientProtocol::Messages, CONFIG)
		.unwrap()
		.body;
	assert_eq!(
		body,
		test_support::convert(
			with_messages(&messages).as_bytes(),
			ClientProtocol::Messages,
			CONFIG
		)
		.unwrap()
		.body
	);
	assert!(body.contains("out"));
}

#[test]
fn every_independent_problem_is_kept_in_order() {
	let body = r#"{"max_tokens":0,"extra":1,"messages":[
		{"role":"user","nope":1,"content":"a"},
		{"content":"no role"},
		{"role":"robot","content":"b"},
		5,
		{"role":"user","content":[{"type":"image"},7,{"text":"no type"},{"type":"text"}]}
	]}"#;
	assert_eq!(
		problems(body),
		[
			"`max_tokens` must be positive",
			"field `extra`",
			"missing `model`",
			"field `messages[0].nope`",
			"`messages[1]` needs `role` and `content`",
			"`messages[2]` role `robot`",
			"`messages[3]` must be an object",
			"`messages[4].content[1]` must be an object",
			"`messages[4].content[2]` has no type",
			"`messages[4].content[3]` has no text",
			"`messages[4].content[0]` has no source",
		]
	);
}

#[rstest]
#[case::model(r#"{"model":"m","model":"n","max_tokens":1,"messages":[]}"#.into(), "`request` repeats `model`")]
#[case::messages(r#"{"model":"m","max_tokens":1,"messages":[],"messages":[]}"#.into(), "`request` repeats `messages`")]
#[case::role(
	with_messages(r#"[{"role":"user","role":"user","content":"a"}]"#),
	"`messages[0]` repeats `role`"
)]
#[case::content(
	with_messages(r#"[{"role":"user","content":"a","content":"b"}]"#),
	"`messages[0]` repeats `content`"
)]
#[case::text(
	with_messages(r#"[{"role":"user","content":[{"type":"text","text":"a","text":"b"}]}]"#),
	"`messages[0].content[0]` repeats `text`"
)]
fn repeated_keys_are_refused_at_every_level(#[case] body: String, #[case] problem: &str) {
	assert_eq!(problems(&body), [problem]);
}

#[test]
fn the_first_repeat_in_each_object_is_the_only_one_reported() {
	let message =
		r#"{"role":"user","role":"user","output_config":{},"output_config":{},"content":"a"}"#;
	let other = r#"{"role":"user","content":"a","content":"b"}"#;
	assert_eq!(
		problems(&with_messages(&format!("[{message},{other}]"))),
		[
			"`messages[0]` repeats `output_config`",
			"`messages[1]` repeats `content`",
		]
	);
}

#[test]
fn system_messages_read_the_same_both_ways() {
	let body = with_messages(
		r#"[{"role":"system","content":[{"type":"text","text":"a"},{"type":"image"},3]},{"role":"user","content":"b"}]"#,
	);
	assert_eq!(
		problems(&body),
		[
			"`messages[0].content[1]` block type `image`",
			"`messages[0].content[2]` must be an object",
		]
	);
}

#[rstest]
#[case::surrogate(
	r#"[{"role":"user","content":"\ud800"}]"#,
	"`messages[0].content` must be a string or a list"
)]
#[case::number(
	r#"[{"role":"user","content":1e999}]"#,
	"`messages[0].content` must be a string or a list"
)]
#[case::surrogate_in_list(
	r#"[{"role":"user","content":["\ud800"]}]"#,
	"`messages[0].content[0]` must be an object"
)]
fn values_a_strict_read_refuses_are_left_to_the_level_by_level_read(
	#[case] messages: &str,
	#[case] problem: &str,
) {
	let body = with_messages(messages);
	assert!(read_body(&body).is_none());
	assert_eq!(problems(&body), [problem]);
}

#[rstest]
#[case::number("5")]
#[case::null("null")]
#[case::object("{}")]
#[case::empty("[]")]
#[case::null_message("[null]")]
#[case::missing_content(r#"[{"role":"user"}]"#)]
fn wrong_message_shapes_read_the_same_both_ways(#[case] messages: &str) {
	assert!(!problems(&with_messages(messages)).is_empty());
}

#[rstest]
#[case::number("5")]
#[case::null("null")]
#[case::object("{}")]
#[case::null_block("[null]")]
#[case::nested_list("[[]]")]
#[case::non_string_type(r#"[{"type":5}]"#)]
fn wrong_content_shapes_read_the_same_both_ways(#[case] content: &str) {
	let messages = format!(r#"[{{"role":"user","content":{content}}}]"#);
	assert!(!problems(&with_messages(&messages)).is_empty());
}

#[rstest]
#[case::empty("")]
#[case::number("5")]
#[case::list("[]")]
#[case::truncated("{")]
#[case::missing_fields("{}")]
#[case::trailing_data(r#"{"messages":[]} x"#)]
fn wrong_request_shapes_read_the_same_both_ways(#[case] body: &str) {
	assert!(!problems(body).is_empty());
}

#[test]
fn a_tool_results_content_is_read_with_its_block() {
	let body = with_messages(
		r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"c","content":"a\nb"}]}]"#,
	);
	let read = read_body(&body).unwrap();
	let messages = read.messages.unwrap();
	let Some(ContentRead::Blocks(blocks)) = &messages[0].content else {
		panic!("not blocks");
	};
	let Read::Done(block) = &blocks[0] else {
		panic!("not read");
	};
	assert!(matches!(&block.content, Some(ContentRead::Text(text)) if text == "a\nb"));
}

#[test]
fn content_on_any_other_block_is_left_to_the_level_by_level_read() {
	let body = with_messages(
		r#"[{"role":"user","content":[{"content":"x","type":"text","text":"a","extra":1}]}]"#,
	);
	assert!(read_body(&body).is_none());
	assert_eq!(
		problems(&body),
		[
			"field `messages[0].content[0].content`",
			"field `messages[0].content[0].extra`",
		]
	);
}

#[test]
fn a_tool_results_content_is_checked_after_its_other_fields_both_ways() {
	let body = with_messages(
		r#"[{"role":"assistant","content":[{"type":"tool_use","id":"c","name":"f","input":{}}]},
		{"role":"user","content":[{"type":"tool_result","content":[{"type":"text"}],"tool_use_id":"c","extra":1}]}]"#,
	);
	assert!(read_body(&body).is_some());
	assert_eq!(
		problems(&body),
		[
			"field `messages[1].content[0].extra`",
			"`messages[1].content[0].content[0]` has no text",
		]
	);
}

#[test]
fn text_stays_borrowed_unless_it_contains_an_escape() {
	let body =
		with_messages(r#"[{"role":"user","content":"plain"},{"role":"user","content":"a\nb"}]"#);
	let read = read_body(&body).unwrap();
	let messages = read.messages.unwrap();
	let is_borrowed = |at: usize| match &messages[at].content {
		Some(ContentRead::Text(text)) => matches!(text, Cow::Borrowed(_)),
		_ => panic!("not text"),
	};
	assert!(is_borrowed(0));
	assert!(!is_borrowed(1));
}
