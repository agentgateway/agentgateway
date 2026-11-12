#![allow(clippy::unwrap_used)]

use rstest::rstest;
use serde_json::{Value, json};

use super::*;
use crate::protocols::Delivery;

const GEMINI: ClientEndpoint = ClientEndpoint::Gemini(Delivery::Streaming);

#[rstest]
#[case::messages(ClientEndpoint::Messages, json!({"model": "m", "max_tokens": 7,
	"temperature": 0.5, "stream": true, "output_config": {"effort": "high"}}))]
#[case::responses(ClientEndpoint::Responses, json!({"model": "m", "max_output_tokens": 7,
	"temperature": 0.5, "stream": true, "reasoning": {"effort": "high"}}))]
#[case::chat(ClientEndpoint::ChatCompletions, json!({"model": "m", "max_tokens": 3,
	"max_completion_tokens": 7, "temperature": 0.5, "stream": true, "reasoning_effort": "high",
	"stream_options": {"include_usage": true}}))]
#[case::gemini(GEMINI, json!({"generationConfig": {"maxOutputTokens": 7, "temperature": 0.5,
	"thinkingConfig": {"thinkingLevel": "HIGH"}}}))]
#[case::gemini_snake_case(GEMINI, json!({"generation_config": {"max_output_tokens": 7,
	"temperature": 0.5, "thinking_config": {"thinking_level": "high"}}}))]
fn the_recorded_settings_are_read(
	#[case] client: ClientEndpoint,
	#[case] body: Value,
	#[values(false, true)] parsed: bool,
) {
	let bytes = body.to_string();
	let parsed = parsed.then(|| ParsedRequest::new(&body).unwrap());
	let settings = match &parsed {
		Some(parsed) => parsed.read_passthrough(client),
		None => read_passthrough(bytes.as_bytes(), client),
	}
	.unwrap();
	assert_eq!(settings.max_tokens, NonZeroU64::new(7));
	assert_eq!(
		settings
			.temperature
			.map(|value| value.read::<f64>().unwrap()),
		Some(0.5)
	);
	assert_eq!(settings.stream, Some(true));
	match client {
		ClientEndpoint::Messages => assert!(settings.messages_input.output_config.is_some()),
		_ => assert_eq!(settings.effort, Some(Effort::High)),
	}
}

#[rstest]
#[case::plain(r#"{"model":"m","messages":[]}"#, Some("m"))]
#[case::escaped(r#"{"mod\u0065l":"gpt\u002d5"}"#, Some("gpt-5"))]
#[case::null(r#"{"model":null}"#, None)]
#[case::missing(r#"{"messages":[]}"#, None)]
#[case::nested(r#"{"metadata":{"model":"x"},"messages":[{"model":"y"}]}"#, None)]
fn the_model_is_read_as_read_model_reads_it(#[case] body: &str, #[case] model: Option<&str>) {
	assert_eq!(
		crate::read_model(body.as_bytes()).unwrap().as_deref(),
		model
	);
	let fields = RequestFields::read(body.as_bytes()).unwrap();
	assert_eq!(fields.model().unwrap().as_deref(), model);
	let value = serde_json::from_str(body).unwrap();
	let parsed = ParsedRequest::new(&value).unwrap();
	assert_eq!(parsed.model().unwrap().as_deref(), model);
}

#[rstest]
#[case::number(r#"{"model":5}"#)]
#[case::object(r#"{"model":{"name":"m"}}"#)]
#[case::repeated(r#"{"model":"a","model":"b"}"#)]
#[case::repeated_as_null(r#"{"model":"a","model":null}"#)]
#[case::repeated_with_an_escaped_key(r#"{"model":"a","mod\u0065l":"b"}"#)]
#[case::lone_surrogate(r#"{"model":"\ud800"}"#)]
#[case::not_an_object(r#"["m"]"#)]
fn a_model_read_model_refuses_is_refused(#[case] body: &str) {
	assert!(crate::read_model(body.as_bytes()).is_err());
	let model = RequestFields::read(body.as_bytes()).and_then(|fields| {
		fields.model()?;
		Ok(())
	});
	assert!(model.is_err(), "{body}");
}

/// A model before the conversation is read without reading the rest, which here isn't JSON.
#[rstest]
#[case::first(r#"{"model":"m","messages":[{"#, Some("m"))]
#[case::after_settings(r#"{"max_tokens":7,"model":"gpt-5","messages":[{"#, Some("gpt-5"))]
#[case::null(r#"{"model":null,"messages":[{"#, None)]
#[case::missing(r#"{"max_tokens":7}"#, None)]
fn a_leading_model_is_read_without_the_conversation(
	#[case] body: &str,
	#[case] model: Option<&str>,
) {
	assert_eq!(
		leading_model(body.as_bytes()).unwrap(),
		LeadingModel::Read(model.map(Cow::Borrowed))
	);
}

/// Every byte but the model's value is kept, whitespace and number formats included.
#[rstest]
#[case::spaced(
	r#"{ "max_tokens": 1.0, "model" : "ab" , "messages": [] }"#,
	Some(r#"{ "max_tokens": 1.0, "model" : "x\"y" , "messages": [] }"#)
)]
#[case::null(
	r#"{"model":null,"messages":[]}"#,
	Some(r#"{"model":"x\"y","messages":[]}"#)
)]
#[case::after_the_conversation(
	r#"{"messages":[ {"model":"n"} ],"model":"a"}"#,
	Some(r#"{"messages":[ {"model":"n"} ],"model":"x\"y"}"#)
)]
#[case::missing(r#"{"max_tokens":7}"#, None)]
#[case::broken_before_it(r#"{"max_tokens":,"model":"a"}"#, None)]
fn the_model_is_replaced_in_place(#[case] body: &str, #[case] replaced: Option<&str>) {
	let replaced_body = replace_model(body.as_bytes(), "x\"y");
	assert_eq!(
		replaced_body
			.as_deref()
			.map(|body| std::str::from_utf8(body).unwrap()),
		replaced
	);
}

#[rstest]
#[case::messages(r#"{"max_tokens":7,"messages":[],"model":"m"}"#)]
#[case::escaped(r#"{"messages":[],"model":"m"}"#)]
#[case::tools(r#"{"tools":[],"model":"m"}"#)]
fn a_model_after_the_conversation_isnt_looked_for(#[case] body: &str) {
	assert_eq!(
		leading_model(body.as_bytes()).unwrap(),
		LeadingModel::AfterConversation
	);
}

#[rstest]
#[case::number(r#"{"model":5,"messages":[]}"#)]
#[case::broken_before_it(r#"{"max_tokens":,"model":"m"}"#)]
#[case::not_an_object(r#"["m"]"#)]
#[case::trailing(r#"{"max_tokens":7} x"#)]
fn a_leading_model_that_cant_be_read_is_refused(#[case] body: &str) {
	assert!(leading_model(body.as_bytes()).is_err(), "{body}");
}

/// A parsed body that isn't a string model is refused too. One that repeats a field can't be
/// told apart from one that doesn't.
#[rstest]
#[case::number(json!({"model": 5}))]
#[case::object(json!({"model": {"name": "m"}}))]
#[case::not_an_object(json!(["m"]))]
fn a_parsed_model_that_isnt_a_name_is_refused(#[case] body: Value) {
	let model = ParsedRequest::new(&body).and_then(|parsed| {
		parsed.model()?;
		Ok(())
	});
	assert!(model.is_err(), "{body}");
}

/// The provider gets the request unconverted, so only it decides about what the conversion
/// pipeline refuses: unknown fields, and replay state it signed itself.
#[rstest]
#[case::messages(ClientEndpoint::Messages, json!({"model": "m", "max_tokens": 1,
	"service_tier": "auto", "messages": [{"role": "assistant", "content": [
		{"type": "thinking", "thinking": "hm", "signature": "provider-signed"},
		{"type": "text", "text": "Hi", "citations": null}]}]}))]
#[case::responses(ClientEndpoint::Responses, json!({"model": "m", "store": true,
	"previous_response_id": "resp_1", "input": [
		{"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "provider-sealed"},
		{"type": "item_reference", "id": "msg_0"},
		{"role": "assistant", "content": [{"type": "output_text", "text": "Hi", "logprobs": []}]}]}))]
#[case::chat(ClientEndpoint::ChatCompletions, json!({"model": "m", "n": 2, "seed": 1,
	"logprobs": true, "messages": [{"role": "assistant", "content": "Hi", "audio": null}]}))]
#[case::gemini(GEMINI, json!({"cachedContent": "c", "contents": [{"role": "model", "parts": [
	{"text": "thinking", "thought": true},
	{"text": "Hi", "thoughtSignature": "provider-signed"}]}]}))]
fn what_conversion_refuses_is_left_to_the_provider(
	#[case] client: ClientEndpoint,
	#[case] body: Value,
) {
	assert!(read_passthrough(body.to_string().as_bytes(), client).is_ok());
	assert!(
		ParsedRequest::new(&body)
			.unwrap()
			.read_passthrough(client)
			.is_ok()
	);
}

#[rstest]
#[case::not_an_object(b"[]".as_slice(), "the request isn't a JSON object")]
#[case::a_repeated_field(br#"{"model":"a","model":"b"}"#.as_slice(), "the request repeats `model`")]
#[case::stream_not_a_boolean(br#"{"stream":"yes"}"#.as_slice(), "`stream` must be a boolean")]
fn what_decides_how_the_reply_is_read_must_be_readable(#[case] body: &[u8], #[case] problem: &str) {
	let refused = read_passthrough(body, ClientEndpoint::Messages)
		.err()
		.unwrap();
	assert!(refused.problems[0].starts_with(problem), "{refused:?}");
	if let Ok(body) = serde_json::from_slice::<Value>(body)
		&& !refused.problems[0].contains("repeats")
	{
		let refused = ParsedRequest::new(&body)
			.and_then(|parsed| {
				parsed.read_passthrough(ClientEndpoint::Messages)?;
				Ok(())
			})
			.unwrap_err();
		assert!(refused.problems[0].starts_with(problem), "{refused:?}");
	}
}

/// Recording settings never reads the conversation, so only the provider judges it.
#[rstest]
#[case::not_a_list(br#"{"model":"m","messages":5}"#.as_slice())]
#[case::repeated(br#"{"model":"m","messages":[],"messages":[]}"#.as_slice())]
fn the_conversation_is_left_to_the_provider(#[case] body: &[u8]) {
	assert!(read_passthrough(body, ClientEndpoint::Messages).is_ok());
}

#[test]
fn gemini_streaming_comes_from_the_url() {
	let settings = read_passthrough(b"{}", ClientEndpoint::Gemini(Delivery::Buffered)).unwrap();
	assert_eq!(settings.stream, Some(false));
}

/// A settings reader that works on fields read from the body.
fn read_passthrough(body: &[u8], client: ClientEndpoint) -> Result<Settings<'_>, RequestRefusal> {
	RequestFields::read(body)?.read_passthrough(client)
}
