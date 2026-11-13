#![allow(clippy::panic, clippy::unwrap_used)]

use std::num::NonZeroU64;

use serde_json::{Value, json};

use crate::carrier::StateScope;
use crate::prompt::{self, ConvertedRequest, RequestRefusal as Refusal, Upstream};
use crate::protocols::chat_completions::{CarriedReasoning, encode_call_id};
use crate::protocols::{ClientProtocol, UpstreamProtocol};
use crate::test_support::{self, SCOPE, Sum};

const MESSAGES: UpstreamProtocol = UpstreamProtocol::Messages;
const RESPONSES: UpstreamProtocol = UpstreamProtocol::Responses {
	safety_identifier: false,
	reasoning_context: false,
};
const GEMINI: UpstreamProtocol = UpstreamProtocol::Gemini;

fn upstream(protocol: UpstreamProtocol) -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		protocol,
		default_max_tokens: None,
	}
}

fn convert_on(upstream: Upstream<'_>, body: &Value) -> Result<(Value, ConvertedRequest), Refusal> {
	let body = body.to_string();
	assert!(
		prompt::one_pass_parse_agrees(body.as_bytes(), ClientProtocol::ChatCompletions, upstream),
		"{body}"
	);
	let converted =
		test_support::convert(body.as_bytes(), ClientProtocol::ChatCompletions, upstream)?;
	Ok((serde_json::from_str(&converted.body).unwrap(), converted))
}

fn sent(protocol: UpstreamProtocol, body: &Value) -> (Value, ConvertedRequest) {
	convert_on(upstream(protocol), body).unwrap_or_else(|refusal| panic!("{refusal}"))
}

fn problems(protocol: UpstreamProtocol, body: &Value) -> Vec<String> {
	match convert_on(upstream(protocol), body) {
		Ok((sent, _)) => panic!("converted: {sent}"),
		Err(refusal) => refusal.problems,
	}
}

fn chat(messages: &Value) -> Value {
	json!({"model": "m", "max_completion_tokens": 50, "messages": messages})
}

fn hello() -> Value {
	json!([{"role": "user", "content": "hi"}])
}

/// What the writer puts in a call's id: the reasoning before the call, encoded with it.
fn encoded(call: &str, items: &[Value], scope: StateScope<'_>) -> String {
	let items: Vec<_> = items.iter().map(|item| (0, item.clone())).collect();
	encoded_at(call, &items, scope)
}

fn encoded_at(call: &str, items: &[(usize, Value)], scope: StateScope<'_>) -> String {
	let items: Vec<_> = items
		.iter()
		.map(|(text_offset, item)| CarriedReasoning {
			text_offset: *text_offset,
			item: item.to_string(),
		})
		.collect();
	encode_call_id(scope, call, &items)
}

fn called(id: &str) -> Value {
	json!([
		{"role": "user", "content": "weather?"},
		{"role": "assistant", "content": "Looking.", "tool_calls": [
			{"id": id, "type": "function", "function": {"name": "f", "arguments": "{\"city\":\"Oslo\"}"}}]},
		{"role": "tool", "tool_call_id": id, "content": "sunny"}
	])
}

#[test]
fn a_conversation_becomes_a_messages_request() {
	let body = json!({"model": "m", "max_completion_tokens": 50, "temperature": 0.5,
		"stop": "END", "tool_choice": "auto", "parallel_tool_calls": false,
		"tools": [{"type": "function", "function": {"name": "f", "description": "d",
			"parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}},
			{"type": "function", "function": {"name": "g"}}],
		"messages": [{"role": "system", "content": "Be brief."},
			{"role": "developer", "content": [{"type": "text", "text": "Use tools."}]}]});
	let mut body = body;
	body["messages"]
		.as_array_mut()
		.unwrap()
		.extend(called("call_1").as_array().unwrap().iter().cloned());
	let (sent, converted) = sent(MESSAGES, &body);
	assert_eq!(
		sent,
		json!({"model": "m", "max_tokens": 50,
			"system": [{"type": "text", "text": "Be brief."},
				{"type": "text", "text": "Use tools.", "cache_control": {"type": "ephemeral"}}],
			"messages": [
				{"role": "user", "content": [{"type": "text", "text": "weather?"}]},
				{"role": "assistant", "content": [{"type": "text", "text": "Looking."},
					{"type": "tool_use", "id": "call_1", "name": "f", "input": {"city": "Oslo"}}]},
				{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_1",
					"content": [{"type": "text", "text": "sunny"}],
					"cache_control": {"type": "ephemeral"}}]}],
			"tools": [{"name": "f", "description": "d",
				"input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}},
				{"name": "g", "input_schema": {"type": "object", "properties": {}}}],
			"tool_choice": {"type": "auto", "disable_parallel_tool_use": true},
			"temperature": 0.5, "stop_sequences": ["END"]})
	);
	assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
}

#[test]
fn a_null_field_is_left_out() {
	let mut body = chat(&hello());
	for field in [
		"stop",
		"tools",
		"n",
		"logprobs",
		"seed",
		"audio",
		"user",
		"response_format",
	] {
		body[field] = Value::Null;
	}
	let (_, converted) = sent(MESSAGES, &body);
	assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
}

#[test]
fn what_no_other_provider_can_accept_is_refused_together() {
	let mut body = chat(&hello());
	let refused = json!({"n": 2, "logprobs": true, "top_logprobs": 2, "seed": 7,
		"presence_penalty": 0.5, "frequency_penalty": 1, "logit_bias": {"50256": -100},
		"modalities": ["text", "audio"], "audio": {"voice": "x"},
		"prediction": {"type": "content", "content": "x"}, "store": true,
		"service_tier": "flex", "web_search_options": {}, "functions": []});
	for (name, value) in refused.as_object().unwrap() {
		body[name] = value.clone();
	}
	let mut problems = problems(MESSAGES, &body);
	problems.sort();
	assert_eq!(
		problems,
		[
			"`frequency_penalty` other than 0",
			"`logit_bias`",
			"`logprobs`",
			"`modalities` other than text",
			"`n` other than 1",
			"`presence_penalty` other than 0",
			"`service_tier` `flex`",
			"`store: true`",
			"field `audio`",
			"field `functions`",
			"field `prediction`",
			"field `seed`",
			"field `top_logprobs`",
			"field `web_search_options`",
		]
	);
}

#[test]
fn defaults_and_identifiers_are_dropped_with_a_record() {
	let mut body = chat(&hello());
	let dropped = json!({"n": 1, "logprobs": false, "presence_penalty": 0, "logit_bias": {},
		"modalities": ["text"], "store": false, "service_tier": "auto", "user": "u",
		"safety_identifier": "s", "prompt_cache_key": "k", "metadata": {"a": "b"},
		"verbosity": "low"});
	for (name, value) in dropped.as_object().unwrap() {
		body[name] = value.clone();
	}
	let (_, converted) = sent(MESSAGES, &body);
	let mut dropped = converted.dropped;
	dropped.sort_unstable();
	assert_eq!(
		dropped,
		[
			"metadata",
			"prompt_cache_key",
			"safety_identifier",
			"service_tier",
			"store",
			"user",
			"verbosity"
		]
	);
}

#[test]
fn max_completion_tokens_wins_over_max_tokens() {
	let mut body = chat(&hello());
	body["max_tokens"] = json!(10);
	let (sent, converted) = sent(MESSAGES, &body);
	assert_eq!(sent["max_tokens"], 50);
	assert_eq!(
		converted.dropped,
		["max_tokens beside max_completion_tokens"]
	);

	body
		.as_object_mut()
		.unwrap()
		.remove("max_completion_tokens");
	let (sent, converted) = super::tests::sent(MESSAGES, &body);
	assert_eq!(sent["max_tokens"], 10);
	assert!(converted.dropped.is_empty());
}

#[test]
fn without_a_limit_messages_needs_a_default() {
	let body = json!({"model": "m", "messages": hello()});
	assert_eq!(
		problems(MESSAGES, &body),
		["no `max_completion_tokens`, and no default for this model"]
	);
	let with_default = Upstream {
		default_max_tokens: NonZeroU64::new(77),
		..upstream(MESSAGES)
	};
	let (sent, _) = convert_on(with_default, &body).unwrap();
	assert_eq!(sent["max_tokens"], 77);
	let (sent, _) = sent_on_responses(&body);
	assert_eq!(sent.get("max_output_tokens"), None);
}

fn sent_on_responses(body: &Value) -> (Value, ConvertedRequest) {
	sent(RESPONSES, body)
}

#[test]
fn reading_in_one_pass_agrees_with_reading_level_by_level() {
	let call = r#"{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}"#;
	let messages = [
		r#"[{"role":"user","content":"a \"quoted\"\nline"}]"#,
		r#"[{"content":[{"text":"hi","type":"text"}],"role":"user"}]"#,
		r#"[{"role":"system","content":[{"type":"text","text":"s"}]},{"role":"user","content":"hi"}]"#,
		&format!(
			r#"[{{"role":"user","content":"hi"}},{{"role":"assistant","content":null,"tool_calls":[{call}]}},{{"role":"tool","tool_call_id":"c","content":"out"}}]"#
		),
		&format!(
			r#"[{{"role":"user","content":"hi"}},{{"role":"assist\u0061nt","content":null,"tool_calls":[{call}]}},{{"role":"tool","tool_call_id":"c","content":[{{"type":"text","text":"out"}}]}}]"#
		),
		r#"[{"role":"user","content":null}]"#,
		r#"[{"role":"user"}]"#,
		r#"[{"role":"user","content":"a","content":"b"}]"#,
		r#"[{"role":"user","role":"user","content":"a"}]"#,
		r#"[{"role":"user","content":[{"type":"text","text":"a","text":"b"}]}]"#,
		r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://x"}}]}]"#,
		r#"[{"role":"user","content":[5]}]"#,
		r#"[{"role":"user","content":5}]"#,
		r#"[{"role":"user","content":"hi","extra":1}]"#,
		r#"[5]"#,
		"null",
		"{}",
	];
	for messages in messages {
		let body = format!(r#"{{"model":"m","max_tokens":1,"messages":{messages}}}"#);
		for protocol in [MESSAGES, RESPONSES, UpstreamProtocol::Gemini] {
			assert!(
				prompt::one_pass_parse_agrees(
					body.as_bytes(),
					ClientProtocol::ChatCompletions,
					upstream(protocol)
				),
				"{body}"
			);
		}
	}
}

#[test]
fn a_zero_limit_is_refused() {
	let mut body = chat(&hello());
	body["max_completion_tokens"] = json!(0);
	assert_eq!(
		problems(MESSAGES, &body),
		["`max_completion_tokens` must be positive"]
	);
}

#[test]
fn a_client_that_asks_for_usage_gets_it_at_the_end() {
	let mut body = chat(&hello());
	body["stream"] = json!(true);
	let (_, converted) = sent(MESSAGES, &body);
	assert!(converted.delivery.is_streaming() && !converted.reply.include_usage);
	body["stream_options"] = json!({"include_usage": true, "include_obfuscation": false});
	let (_, converted) = sent(MESSAGES, &body);
	assert!(converted.delivery.is_streaming() && converted.reply.include_usage);
	assert_eq!(converted.dropped, ["stream_options.include_obfuscation"]);
}

#[test]
fn images_must_be_data_urls() {
	let image = |url: &str| {
		chat(
			&json!([{"role": "user", "content": [{"type": "text", "text": "what?"},
			{"type": "image_url", "image_url": {"url": url, "detail": "high"}}]}]),
		)
	};
	let (sent, converted) = sent(MESSAGES, &image("data:image/png;base64,iVBORw0KGgo="));
	assert_eq!(
		sent["messages"][0]["content"][1],
		json!({"type": "image", "source": {"type": "base64", "media_type": "image/png",
			"data": "iVBORw0KGgo="}, "cache_control": {"type": "ephemeral"}})
	);
	assert_eq!(converted.dropped, ["image detail"]);
	assert_eq!(
		problems(MESSAGES, &image("https://example.com/cat.png")),
		["`messages[0].content[1]` must be a base64 data URL of a supported image"]
	);
	let audio = chat(&json!([{"role": "user", "content": [
		{"type": "input_audio", "input_audio": {"data": "AAAA", "format": "wav"}},
		{"type": "file", "file": {"file_id": "f"}}]}]));
	assert_eq!(
		problems(MESSAGES, &audio),
		[
			"`messages[0].content[0]` part type `input_audio`",
			"`messages[0].content[1]` part type `file`"
		]
	);
}

#[test]
fn settings_messages_cant_take_are_refused_in_this_clients_words() {
	let mut body = chat(&hello());
	body["reasoning_effort"] = json!("minimal");
	body["response_format"] = json!({"type": "json_object"});
	assert_eq!(
		problems(MESSAGES, &body),
		[
			"`reasoning_effort` `minimal` has no Messages form",
			"`response_format` `json_object` has no Messages form"
		]
	);
	let (sent, _) = sent_on_responses(&body);
	assert_eq!(sent["reasoning"]["effort"], "minimal");
	assert_eq!(sent["text"]["format"]["type"], "json_object");
}

#[test]
fn a_json_schema_is_converted_and_its_name_dropped() {
	let mut body = chat(&hello());
	let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
	body["response_format"] = json!({"type": "json_schema",
		"json_schema": {"name": "answer", "strict": true, "schema": schema}});
	body["reasoning_effort"] = json!("high");
	let (sent, converted) = sent(MESSAGES, &body);
	assert_eq!(
		sent["output_config"],
		json!({"effort": "high", "format": {"type": "json_schema", "schema": schema}})
	);
	assert_eq!(
		sent["thinking"],
		json!({"type": "adaptive", "display": "summarized"}),
		"a client that wants reasoning sees its text"
	);
	assert_eq!(converted.dropped, ["response_format name and strict"]);
}

#[test]
fn tools_other_than_functions_are_refused() {
	let mut body = chat(&hello());
	body["tools"] = json!([{"type": "custom", "custom": {"name": "c"}},
		{"type": "function", "function": {"name": "f", "parameters": []}}]);
	body["tool_choice"] = json!({"type": "allowed_tools", "allowed_tools": {}});
	assert_eq!(
		problems(MESSAGES, &body),
		[
			"field `tools[0].custom`",
			"`tools[0]` tool type `custom`",
			"`tools[1].function.parameters` must be a valid object",
			"field `tool_choice.allowed_tools`",
			"`tool_choice` type `allowed_tools`"
		]
	);
}

#[test]
fn a_named_tool_choice_is_converted() {
	let mut body = chat(&hello());
	body["tools"] = json!([{"type": "function", "function": {"name": "f"}}]);
	body["tool_choice"] = json!({"type": "function", "function": {"name": "f"}});
	let (sent, _) = sent(MESSAGES, &body);
	assert_eq!(sent["tool_choice"], json!({"type": "tool", "name": "f"}));
}

#[test]
fn every_call_needs_its_result_and_every_result_its_call() {
	let mut messages = called("call_1");
	messages.as_array_mut().unwrap().pop();
	messages
		.as_array_mut()
		.unwrap()
		.push(json!({"role": "tool", "tool_call_id": "call_9", "content": "x"}));
	let problems = problems(MESSAGES, &chat(&messages));
	assert!(
		problems.contains(&"`messages[2]` answers no open tool call".to_string()),
		"{problems:?}"
	);
	assert!(
		problems.contains(&"tool calls with no result: call_1".to_string()),
		"{problems:?}"
	);
}

#[test]
fn results_of_parallel_calls_are_one_turn() {
	let messages = json!([
		{"role": "user", "content": "both?"},
		{"role": "assistant", "content": null, "refusal": null, "tool_calls": [
			{"id": "a", "type": "function", "function": {"name": "f", "arguments": "{}"}},
			{"id": "b", "type": "function", "function": {"name": "f", "arguments": ""}}]},
		{"role": "tool", "tool_call_id": "a", "content": "1"},
		{"role": "tool", "tool_call_id": "b", "content": [{"type": "text", "text": "2"}]}
	]);
	let (sent, _) = sent(GEMINI, &chat(&messages));
	let turns = sent["contents"].as_array().unwrap();
	assert_eq!(turns.len(), 3, "{sent}");
	assert_eq!(turns[2]["parts"].as_array().unwrap().len(), 2, "{sent}");
}

#[test]
fn echoed_reasoning_text_is_dropped_and_a_past_refusal_refused() {
	let mut messages = called("call_1");
	messages[1]["reasoning_content"] = json!("I thought.");
	let (_, converted) = sent(MESSAGES, &chat(&messages));
	assert_eq!(converted.dropped, ["assistant reasoning text"]);
	messages[1]["refusal"] = json!("No.");
	messages[0]["name"] = json!("ann");
	assert_eq!(
		problems(MESSAGES, &chat(&messages)),
		["field `messages[0].name`", "field `messages[1].refusal`"]
	);
}

#[test]
fn a_result_may_name_its_calls_function() {
	let mut messages = called("call_1");
	messages[2]["name"] = json!("f");
	let (_, converted) = sent(MESSAGES, &chat(&messages));
	assert_eq!(converted.dropped, ["tool result name"]);
	messages[2]["name"] = json!("g");
	assert_eq!(
		problems(MESSAGES, &chat(&messages)),
		["`messages[2].name` isn't the name of its call"]
	);
}

#[test]
fn a_system_message_after_the_first_turn_stays_in_place() {
	let messages = json!([{"role": "user", "content": "hi"},
		{"role": "assistant", "content": "hello"},
		{"role": "system", "content": "Now be terse."},
		{"role": "user", "content": "again"}]);
	let (sent, _) = sent_on_responses(&chat(&messages));
	let roles: Vec<&str> = sent["input"]
		.as_array()
		.unwrap()
		.iter()
		.map(|item| item["role"].as_str().unwrap())
		.collect();
	assert_eq!(roles, ["user", "assistant", "system", "user"]);
}

#[test]
fn reasoning_encoded_into_a_call_id_goes_back_before_the_call() {
	let thinking = json!({"type": "thinking", "thinking": "Use f.", "signature": "c2ln"});
	let id = encoded("toolu_1", std::slice::from_ref(&thinking), SCOPE);
	let (sent, converted) = sent(MESSAGES, &chat(&called(&id)));
	assert_eq!(
		sent["messages"][1]["content"],
		json!([thinking, {"type": "text", "text": "Looking."},
			{"type": "tool_use", "id": "toolu_1", "name": "f", "input": {"city": "Oslo"}}])
	);
	assert_eq!(sent["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
	assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
}

#[test]
fn reasoning_keeps_its_position_around_chat_text() {
	let item =
		|id: &str| json!({"type": "reasoning", "id": id, "summary": [], "encrypted_content": "e"});
	let text = "Visible text.";
	let id = encoded_at(
		"call_1",
		&[(0, item("rs_before")), (text.len(), item("rs_after"))],
		SCOPE,
	);
	let messages = json!([
		{"role": "user", "content": "go"},
		{"role": "assistant", "content": text, "tool_calls": [
			{"id": id, "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
		{"role": "tool", "tool_call_id": id, "content": "done"}
	]);
	let (sent, _) = sent_on_responses(&chat(&messages));
	let order: Vec<&str> = sent["input"]
		.as_array()
		.unwrap()
		.iter()
		.map(|item| item["type"].as_str().unwrap_or("message"))
		.collect();
	assert_eq!(
		order,
		[
			"message",
			"reasoning",
			"message",
			"reasoning",
			"function_call",
			"function_call_output"
		]
	);
}

#[test]
fn a_later_calls_reasoning_goes_right_before_it() {
	let item =
		|id: &str| json!({"type": "reasoning", "id": id, "summary": [], "encrypted_content": "e"});
	let first = encoded("call_1", &[item("rs_1")], SCOPE);
	let second = encoded_at("call_2", &[("Two calls.".len(), item("rs_2"))], SCOPE);
	let messages = json!([
		{"role": "user", "content": "both?"},
		{"role": "assistant", "content": "Two calls.", "tool_calls": [
			{"id": first, "type": "function", "function": {"name": "f", "arguments": "{}"}},
			{"id": second, "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
		{"role": "tool", "tool_call_id": first, "content": "1"},
		{"role": "tool", "tool_call_id": second, "content": "2"}
	]);
	let (sent, _) = sent_on_responses(&chat(&messages));
	let order: Vec<String> = sent["input"]
		.as_array()
		.unwrap()
		.iter()
		.map(|item| {
			let kind = item["type"].as_str().unwrap_or("message");
			let id = item["call_id"]
				.as_str()
				.or_else(|| item["id"].as_str())
				.unwrap_or("");
			format!("{kind} {id}").trim().to_string()
		})
		.collect();
	assert_eq!(
		order,
		[
			"message",
			"reasoning rs_1",
			"message",
			"function_call call_1",
			"reasoning rs_2",
			"function_call call_2",
			"function_call_output call_1",
			"function_call_output call_2"
		]
	);
}

#[test]
fn reasoning_encoded_for_another_scope_is_refused() {
	let other = StateScope {
		model: "other",
		..SCOPE
	};
	let id = encoded("toolu_1", &[json!({"type": "thinking"})], other);
	// The Messages renderer names the block in a Responses client's words.
	let problems = problems(MESSAGES, &chat(&called(&id)));
	assert_eq!(problems.len(), 1, "{problems:?}");
	assert!(
		problems[0].ends_with(": carrier does not match this destination"),
		"{problems:?}"
	);
}

#[test]
fn a_gemini_call_keeps_its_signature_and_text_signatures_are_dropped() {
	let call = json!({"functionCall": {"name": "f", "args": {"city": "Oslo"}},
		"thoughtSignature": "c2ln"});
	let text = json!({"text": "Looking.", "thoughtSignature": "dGV4dA"});
	let id = encoded("call_1", &[text, call], SCOPE);
	let (sent, converted) = sent(GEMINI, &chat(&called(&id)));
	assert_eq!(
		sent["contents"][1]["parts"],
		json!([{"text": "Looking."}, {"functionCall": {"name": "f", "args": {"city": "Oslo"}},
			"thoughtSignature": "c2ln"}])
	);
	assert_eq!(converted.dropped, ["reasoning signatures on text"]);
}

#[test]
fn a_chat_client_on_a_chat_provider_isnt_converted() {
	let refusal =
		convert_on(upstream(UpstreamProtocol::ChatCompletions), &chat(&hello())).unwrap_err();
	assert_eq!(
		refusal.problems,
		["same-protocol requests are not supported by the conversion pipeline"]
	);
}

#[test]
fn unknown_fields_and_roles_are_refused() {
	let mut body = chat(&json!([{"role": "function", "name": "f", "content": "x"},
		{"role": "user", "content": "hi"}]));
	body["future"] = json!(1);
	assert_eq!(
		problems(MESSAGES, &body),
		["field `future`", "`messages[0]` role `function`"]
	);
}

mod read_tests {
	#![allow(clippy::expect_used)]

	use super::super::read_body;

	#[test]
	fn common_requests_are_read_in_one_pass() {
		let call = r#"{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}"#;
		let body = format!(
			r#"{{"model":"m","messages":[{{"role":"system","content":"s"}},{{"role":"user","content":[{{"type":"text","text":"hi"}}]}},{{"role":"assistant","content":null,"tool_calls":[{call}]}},{{"role":"tool","tool_call_id":"c","content":"out"}}]}}"#
		);
		let read = read_body(&body).expect("read in one pass");
		assert_eq!(read.messages.map(|messages| messages.len()), Some(4));
	}

	#[test]
	fn null_content_outside_an_assistant_message_is_read_level_by_level() {
		assert!(read_body(r#"{"messages":[{"role":"user","content":null}]}"#).is_none());
	}
}
