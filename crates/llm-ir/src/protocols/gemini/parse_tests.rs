#![allow(clippy::panic, clippy::unwrap_used)]

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;
use serde_json::json;

use crate::carrier::{self, StateScope};
use crate::prompt::{ConvertedRequest, RequestRefusal as Refusal, Upstream};
use crate::protocols::{ClientEndpoint, Delivery, UpstreamProtocol};
use crate::test_support::{self, SCOPE, Sum};

const MESSAGES: Upstream<'static> = Upstream {
	identifier_pseudonymizer: &Sum,
	scope: SCOPE,
	protocol: UpstreamProtocol::Messages,
	default_max_tokens: None,
};

const RESPONSES: Upstream<'static> = Upstream {
	identifier_pseudonymizer: &Sum,
	scope: SCOPE,
	protocol: UpstreamProtocol::Responses {
		safety_identifier: false,
		reasoning_context: false,
	},
	default_max_tokens: None,
};

const CLIENT: ClientEndpoint = ClientEndpoint::Gemini(Delivery::Buffered);

fn convert(body: &serde_json::Value, upstream: Upstream<'_>) -> Result<ConvertedRequest, Refusal> {
	test_support::convert(body.to_string().as_bytes(), CLIENT, upstream)
}

/// The Messages request a Gemini body becomes, with a limit unless it sets one.
fn on_messages(mut body: serde_json::Value) -> serde_json::Value {
	let config = body
		.as_object_mut()
		.unwrap()
		.entry("generationConfig")
		.or_insert(json!({}));
	config
		.as_object_mut()
		.unwrap()
		.entry("maxOutputTokens")
		.or_insert(json!(100));
	serde_json::from_str(&convert(&body, MESSAGES).unwrap().body).unwrap()
}

fn problems(body: &serde_json::Value, upstream: Upstream<'_>) -> Vec<String> {
	match convert(body, upstream) {
		Ok(converted) => panic!("converted: {}", converted.body),
		Err(refusal) => refusal.problems,
	}
}

fn hi() -> serde_json::Value {
	json!([{"role": "user", "parts": [{"text": "hi"}]}])
}

/// What a reply writer puts in `thoughtSignature` for these provider items.
fn signed(scope: StateScope<'_>, items: &[serde_json::Value]) -> String {
	let carriers: Vec<String> = items
		.iter()
		.map(|item| {
			carrier::encode(
				scope,
				carrier::CarrierPurpose::GeminiThoughtSignature,
				item.to_string().as_bytes(),
			)
		})
		.collect();
	STANDARD.encode(carriers.join(" "))
}

fn thinking(text: &str) -> serde_json::Value {
	json!({"type": "thinking", "thinking": text, "signature": format!("sig-{text}")})
}

#[test]
fn text_system_and_sampling_convert() {
	let sent = on_messages(json!({
		"systemInstruction": {"parts": [{"text": "Be brief."}, {"text": "Use metric."}]},
		"contents": [
			{"role": "user", "parts": [{"text": "hi"}]},
			{"role": "model", "parts": [{"text": "hello"}]},
			{"parts": [{"text": "weather?"}]}
		],
		"generationConfig": {
			"temperature": 0.5, "topP": 0.9, "topK": 40,
			"maxOutputTokens": 77, "stopSequences": ["END"]
		}
	}));
	assert_eq!(
		sent,
		json!({
			"model": "m",
			"system": [{"type": "text", "text": "Be brief."},
				{"type": "text", "text": "Use metric.", "cache_control": {"type": "ephemeral"}}],
			"messages": [
				{"role": "user", "content": [{"type": "text", "text": "hi"}]},
				{"role": "assistant", "content": [{"type": "text", "text": "hello"}]},
				{"role": "user", "content": [{"type": "text", "text": "weather?", "cache_control": {"type": "ephemeral"}}]}
			],
			"max_tokens": 77,
			"stop_sequences": ["END"],
			"top_k": 40,
			"temperature": 0.5,
			"top_p": 0.9,
			"stream": false
		})
	);
}

#[test]
fn snake_case_names_are_read_and_a_name_given_both_ways_is_a_repeat() {
	let body = json!({
		"system_instruction": {"parts": [{"text": "Be brief."}]},
		"contents": hi(),
		"generation_config": {"max_output_tokens": 5}
	});
	let sent: serde_json::Value =
		serde_json::from_str(&convert(&body, MESSAGES).unwrap().body).unwrap();
	assert_eq!(sent["system"][0]["text"], "Be brief.");
	assert_eq!(sent["max_tokens"], 5);
	let body = json!({"contents": hi(), "generationConfig": {}, "generation_config": {}});
	assert_eq!(
		problems(&body, RESPONSES),
		["`request` repeats `generationConfig`"]
	);
}

#[test]
fn a_body_model_is_dropped_because_the_url_names_it() {
	let body = json!({"model": "gemini-x", "contents": hi()});
	let converted = convert(&body, RESPONSES).unwrap();
	assert_eq!(converted.dropped, ["model in the body"]);
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(sent["model"], "m");
}

#[rstest]
fn the_url_says_whether_to_stream(#[values(false, true)] stream: bool) {
	let converted = test_support::convert(
		json!({"contents": hi()}).to_string().as_bytes(),
		ClientEndpoint::Gemini(if stream {
			Delivery::Streaming
		} else {
			Delivery::Buffered
		}),
		RESPONSES,
	)
	.unwrap();
	assert_eq!(converted.delivery.is_streaming(), stream);
}

#[test]
fn messages_needs_a_limit_the_client_or_the_caller_gives() {
	let body = json!({"contents": hi()});
	assert_eq!(
		problems(&body, MESSAGES),
		["no `generationConfig.maxOutputTokens`, and no default for this model"]
	);
	let upstream = Upstream {
		default_max_tokens: std::num::NonZeroU64::new(64),
		..MESSAGES
	};
	let sent: serde_json::Value =
		serde_json::from_str(&convert(&body, upstream).unwrap().body).unwrap();
	assert_eq!(sent["max_tokens"], 64);
	assert!(convert(&body, RESPONSES).is_ok());
}

#[test]
fn images_convert_in_user_turns_only() {
	let image = json!({"inlineData": {"mimeType": "image/png", "data": "aGVsbG8="}});
	let sent =
		on_messages(json!({"contents": [{"role": "user", "parts": [{"text": "look"}, image]}]}));
	assert_eq!(
		sent["messages"][0]["content"][1],
		json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGVsbG8="},
			"cache_control": {"type": "ephemeral"}})
	);
	let audio = json!({"inlineData": {"mimeType": "audio/wav", "data": "aGVsbG8="}});
	let body = json!({"contents": [
		{"role": "user", "parts": [audio]},
		{"role": "model", "parts": [image]},
		{"role": "user", "parts": [{"fileData": {"fileUri": "gs://b/f", "mimeType": "image/png"}}]}
	]});
	assert_eq!(
		problems(&body, RESPONSES),
		[
			"`contents[0].parts[0].inlineData` is not a supported base64 image",
			"`contents[1].parts[0].inlineData` doesn't belong in a model turn",
			"`contents[2].parts[0]` file reference",
		]
	);
}

#[test]
fn calls_and_results_pair_by_id_and_results_go_first() {
	let sent = on_messages(json!({"contents": [
		{"role": "user", "parts": [{"text": "weather?"}]},
		{"role": "model", "parts": [
			{"functionCall": {"id": "a", "name": "weather", "args": {"city": "Oslo"}}},
			{"functionCall": {"id": "b", "name": "weather", "args": {"city": "Rome"}}}
		]},
		{"role": "user", "parts": [
			{"text": "both done"},
			{"functionResponse": {"id": "b", "name": "weather", "response": {"output": "sun"}}},
			{"functionResponse": {"id": "a", "name": "weather", "response": {"error": "no station"}}}
		]}
	]}));
	assert_eq!(
		sent["messages"][1]["content"],
		json!([
			{"type": "tool_use", "id": "a", "name": "weather", "input": {"city": "Oslo"}},
			{"type": "tool_use", "id": "b", "name": "weather", "input": {"city": "Rome"}}
		])
	);
	assert_eq!(
		sent["messages"][2]["content"],
		json!([
			{"type": "tool_result", "tool_use_id": "b", "content": [{"type": "text", "text": "sun"}]},
			{"type": "tool_result", "tool_use_id": "a", "content": [{"type": "text", "text": "no station"}], "is_error": true},
			{"type": "text", "text": "both done", "cache_control": {"type": "ephemeral"}}
		])
	);
}

/// A single `output`, `content` or `error` text goes on as text; any other response goes on
/// as its JSON.
#[rstest]
#[case::output(json!({"output": "a\n\"b\""}), Some("a\n\"b\""), false)]
#[case::content(json!({"content": "A"}), Some("A"), false)]
#[case::error_text(json!({"error": "no station"}), Some("no station"), true)]
#[case::error_object(json!({"error": {"code": 7}}), Some(r#"{"code":7}"#), true)]
#[case::output_not_text(json!({"output": 5}), None, false)]
#[case::more_than_the_text(json!({"output": "a", "more": 1}), None, false)]
#[case::other_field(json!({"lines": 2}), None, false)]
#[case::empty(json!({}), None, false)]
fn a_function_response_goes_on_as_its_text_or_its_json(
	#[case] response: serde_json::Value,
	#[case] text: Option<&str>,
	#[case] failed: bool,
) {
	let sent = on_messages(json!({"contents": [
		{"role": "user", "parts": [{"text": "go"}]},
		{"role": "model", "parts": [{"functionCall": {"id": "a", "name": "read"}}]},
		{"role": "user", "parts": [{"functionResponse": {"id": "a", "name": "read", "response": response}}]}
	]}));
	let result = &sent["messages"][2]["content"][0];
	let json = response.to_string();
	assert_eq!(result["content"][0]["text"], text.unwrap_or(&json));
	assert_eq!(result["is_error"].as_bool().unwrap_or(false), failed);
}

#[test]
fn calls_without_ids_pair_with_results_by_name_in_order() {
	let sent = on_messages(json!({"contents": [
		{"role": "user", "parts": [{"text": "go"}]},
		{"role": "model", "parts": [
			{"functionCall": {"name": "read", "args": {"path": "a"}}},
			{"functionCall": {"name": "list"}},
			{"functionCall": {"name": "read", "args": {"path": "b"}}}
		]},
		{"role": "user", "parts": [
			{"functionResponse": {"name": "read", "response": {"content": "A"}}},
			{"functionResponse": {"id": "made-up-by-the-client", "name": "read", "response": {"lines": 2}}},
			{"functionResponse": {"name": "list", "response": {"output": "a b"}}}
		]}
	]}));
	let ids: Vec<&serde_json::Value> = sent["messages"][1]["content"]
		.as_array()
		.unwrap()
		.iter()
		.map(|call| &call["id"])
		.collect();
	assert_eq!(
		ids,
		["gemini_call_1_0", "gemini_call_1_1", "gemini_call_1_2"]
	);
	assert_eq!(sent["messages"][1]["content"][1]["input"], json!({}));
	let results: Vec<(&serde_json::Value, &serde_json::Value)> = sent["messages"][2]["content"]
		.as_array()
		.unwrap()
		.iter()
		.map(|result| (&result["tool_use_id"], &result["content"][0]["text"]))
		.collect();
	assert_eq!(
		results,
		[
			(&json!("gemini_call_1_0"), &json!("A")),
			(&json!("gemini_call_1_2"), &json!(r#"{"lines":2}"#)),
			(&json!("gemini_call_1_1"), &json!("a b")),
		]
	);
}

#[test]
fn a_result_with_no_call_and_a_call_with_no_result_are_refused() {
	let body = json!({"contents": [
		{"role": "user", "parts": [{"text": "go"}]},
		{"role": "model", "parts": [
			{"functionCall": {"id": "a", "name": "read"}},
			{"functionCall": {"id": "b", "name": "list"}}
		]},
		{"role": "user", "parts": [
			{"functionResponse": {"id": "a", "name": "read", "response": {}}},
			{"functionResponse": {"id": "c", "name": "read", "response": {}}}
		]}
	]});
	assert_eq!(
		problems(&body, RESPONSES),
		[
			"`contents[2].parts[1].functionResponse` answers no open function call",
			"tool calls with no result: b",
		]
	);
}

#[test]
fn images_in_a_result_go_with_it() {
	let sent = on_messages(json!({"contents": [
		{"role": "user", "parts": [{"text": "shot"}]},
		{"role": "model", "parts": [{"functionCall": {"id": "a", "name": "shot"}}]},
		{"role": "user", "parts": [{"functionResponse": {"id": "a", "name": "shot",
			"response": {"output": "taken"},
			"parts": [{"inlineData": {"mimeType": "image/png", "data": "aGVsbG8="}}]}}]}
	]}));
	assert_eq!(
		sent["messages"][2]["content"][0]["content"],
		json!([
			{"type": "text", "text": "taken"},
			{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGVsbG8="}}
		])
	);
}

#[test]
fn gemini_schemas_become_json_schema() {
	let converted = convert(
		&json!({
			"contents": hi(),
			"tools": [{"functionDeclarations": [
				{"name": "find", "description": "Finds.", "parameters": {
					"type": "OBJECT",
					"properties": {
						"max_hits": {"type": "INTEGER", "nullable": true, "minimum": 1},
						"tags": {"type": "ARRAY", "items": {"type": "STRING"}, "max_items": "5"}
					},
					"required": ["max_hits"],
					"propertyOrdering": ["max_hits", "tags"]
				}},
				{"name": "raw", "parametersJsonSchema": {"type": "object", "additionalProperties": false}},
				{"name": "bare"}
			]}]
		}),
		RESPONSES,
	)
	.unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	let schemas: Vec<&serde_json::Value> = sent["tools"]
		.as_array()
		.unwrap()
		.iter()
		.map(|tool| &tool["parameters"])
		.collect();
	assert_eq!(
		schemas,
		[
			&json!({
				"type": "object",
				"properties": {
					"max_hits": {"type": ["integer", "null"], "minimum": 1},
					"tags": {"type": "array", "items": {"type": "string"}, "maxItems": 5}
				},
				"required": ["max_hits"]
			}),
			&json!({"type": "object", "additionalProperties": false}),
			&json!({"type": "object", "properties": {}}),
		]
	);
	assert_eq!(converted.dropped, ["schema propertyOrdering"]);
}

#[test]
fn google_tools_and_tool_options_with_no_meaning_elsewhere_are_refused() {
	let body = json!({
		"contents": hi(),
		"tools": [
			{"functionDeclarations": [
				{"name": "a", "behavior": "NON_BLOCKING"},
				{"name": "b", "parameters": {"type": "OBJECT", "not": {}}},
				{"name": "c", "parameters": {}, "parametersJsonSchema": {}}
			]},
			{"googleSearch": {}},
			{"codeExecution": {}},
			{"urlContext": {}}
		],
		"cachedContent": "cachedContents/x"
	});
	assert_eq!(
		problems(&body, RESPONSES),
		[
			"`tools[0].functionDeclarations[0].behavior` `NON_BLOCKING`",
			"`tools[0].functionDeclarations[1].parameters` schema keyword `not`",
			"`tools[0].functionDeclarations[2]` has both `parameters` and `parametersJsonSchema`",
			"`tools[1].googleSearch` tool",
			"`tools[2].codeExecution` tool",
			"`tools[3].urlContext` tool",
			"field `cachedContent`",
		]
	);
}

#[rstest]
#[case(json!({"mode": "AUTO"}), json!({"type": "auto"}))]
#[case(json!({}), json!({"type": "auto"}))]
#[case(json!({"mode": "ANY"}), json!({"type": "any"}))]
#[case(json!({"mode": "ANY", "allowedFunctionNames": ["f"]}), json!({"type": "tool", "name": "f"}))]
#[case(json!({"mode": "NONE"}), json!({"type": "none"}))]
fn function_calling_modes_convert(
	#[case] config: serde_json::Value,
	#[case] choice: serde_json::Value,
) {
	let sent = on_messages(json!({
		"contents": hi(),
		"tools": [{"functionDeclarations": [{"name": "f"}, {"name": "g"}]}],
		"toolConfig": {"functionCallingConfig": config}
	}));
	assert_eq!(sent["tool_choice"], choice);
}

#[rstest]
#[case(json!({"mode": "ANY", "allowedFunctionNames": ["f", "g"]}), "`toolConfig.functionCallingConfig.allowedFunctionNames` with more than one name")]
#[case(json!({"mode": "AUTO", "allowedFunctionNames": ["f"]}), "`toolConfig.functionCallingConfig.allowedFunctionNames` without mode `ANY`")]
#[case(json!({"mode": "VALIDATED"}), "`toolConfig.functionCallingConfig.mode` `VALIDATED`")]
fn function_calling_a_target_cant_follow_is_refused(
	#[case] config: serde_json::Value,
	#[case] problem: &str,
) {
	let body = json!({
		"contents": hi(),
		"tools": [{"functionDeclarations": [{"name": "f"}, {"name": "g"}]}],
		"toolConfig": {"functionCallingConfig": config}
	});
	assert_eq!(problems(&body, RESPONSES), [problem]);
}

#[rstest]
fn structured_output_converts_from_either_schema_field(
	#[values("responseSchema", "responseJsonSchema")] schema_field: &str,
) {
	let sent = on_messages(json!({
		"contents": hi(),
		"generationConfig": {"responseMimeType": "application/json",
			schema_field: {"type": "object", "properties": {"n": {"type": "integer"}}}}
	}));
	assert_eq!(
		sent["output_config"]["format"],
		json!({"type": "json_schema", "schema": {"type": "object", "properties": {"n": {"type": "integer"}}}})
	);
}

#[test]
fn generation_settings_a_target_cant_follow_are_refused() {
	let body = json!({
		"contents": hi(),
		"generationConfig": {
			"candidateCount": 2,
			"responseModalities": ["TEXT", "IMAGE"],
			"responseSchema": {"type": "OBJECT"},
			"seed": 7,
			"thinkingConfig": {"thinkingLevel": "HIGH", "thinkingBudget": 10}
		}
	});
	assert_eq!(
		problems(&body, RESPONSES),
		[
			"`generationConfig.candidateCount` other than 1",
			"`generationConfig.responseModalities` other than text",
			"field `generationConfig.seed`",
			"`generationConfig.thinkingConfig` has both a level and a budget",
			"a response schema needs `responseMimeType` `application/json`",
		]
	);
}

#[rstest]
#[case(json!({"thinkingLevel": "HIGH"}), "high", &[])]
#[case(json!({"thinkingLevel": "low"}), "low", &[])]
#[case(json!({"thinkingBudget": 0}), "none", &[])]
#[case(json!({"thinkingBudget": 1024}), "low", &["thinkingConfig.thinkingBudget"])]
#[case(json!({"thinkingBudget": 8192}), "xhigh", &["thinkingConfig.thinkingBudget"])]
#[case(json!({"thinkingBudget": -1}), "medium", &["thinkingConfig.thinkingBudget"])]
fn thinking_settings_become_an_effort(
	#[case] thinking: serde_json::Value,
	#[case] effort: &str,
	#[case] dropped: &[&str],
) {
	let body = json!({"contents": hi(), "generationConfig": {"thinkingConfig": thinking}});
	let converted = convert(&body, RESPONSES).unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(sent["reasoning"]["effort"], effort);
	assert_eq!(converted.dropped, dropped);
}

#[rstest]
fn included_thoughts_are_shown(#[values(true, false)] include: bool) {
	let body = json!({"contents": hi(),
		"generationConfig": {"thinkingConfig": {"includeThoughts": include}}});
	assert_eq!(
		convert(&body, RESPONSES).unwrap().reply.show_reasoning,
		include
	);
}

#[test]
fn a_signature_brings_back_reasoning_before_the_part_it_rode_on() {
	let signature = signed(SCOPE, &[thinking("plan"), thinking("more")]);
	let sent = on_messages(json!({"contents": [
		{"role": "user", "parts": [{"text": "go"}]},
		{"role": "model", "parts": [
			{"text": "Looking.", "thought": true},
			{"functionCall": {"id": "a", "name": "read"}, "thoughtSignature": signature}
		]},
		{"role": "user", "parts": [{"functionResponse": {"id": "a", "name": "read", "response": {"output": "x"}}}]}
	]}));
	assert_eq!(
		sent["messages"][1]["content"],
		json!([
			thinking("plan"),
			thinking("more"),
			{"type": "tool_use", "id": "a", "name": "read", "input": {}}
		])
	);
}

#[test]
fn a_signature_on_an_empty_text_part_brings_back_reasoning_at_the_end_of_a_turn() {
	let signature = signed(SCOPE, &[thinking("done")]);
	let sent = on_messages(json!({"contents": [
		{"role": "user", "parts": [{"text": "go"}]},
		{"role": "model", "parts": [{"text": "Answer."}, {"text": "", "thoughtSignature": signature}]},
		{"role": "user", "parts": [{"text": "more"}]}
	]}));
	assert_eq!(
		sent["messages"][1]["content"],
		json!([{"type": "text", "text": "Answer."}, thinking("done")])
	);
}

#[test]
fn reasoning_comes_back_for_a_responses_provider_too() {
	let item = json!({"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "e"});
	let signature = signed(SCOPE, std::slice::from_ref(&item));
	let converted = convert(
		&json!({"contents": [
			{"role": "user", "parts": [{"text": "go"}]},
			{"role": "model", "parts": [{"text": "Hi.", "thoughtSignature": signature}]},
			{"role": "user", "parts": [{"text": "more"}]}
		]}),
		RESPONSES,
	)
	.unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(sent["input"][1], item);
	assert_eq!(sent["input"][2]["role"], "assistant");
}

#[test]
fn unsigned_thoughts_and_placeholder_signatures_are_dropped_and_recorded() {
	let converted = convert(
		&json!({"contents": [
			{"role": "user", "parts": [{"text": "go"}]},
			{"role": "model", "parts": [
				{"text": "Musing.", "thought": true},
				{"functionCall": {"id": "a", "name": "read"}, "thoughtSignature": "skip_thought_signature_validator"}
			]},
			{"role": "user", "parts": [{"functionResponse": {"id": "a", "name": "read", "response": {}}}]}
		]}),
		RESPONSES,
	)
	.unwrap();
	assert_eq!(
		converted.dropped,
		["thought text", "placeholder thought signature"]
	);
}

#[test]
fn a_signature_from_elsewhere_is_refused() {
	let other = StateScope {
		model: "other",
		..SCOPE
	};
	let body = json!({"contents": [
		{"role": "user", "parts": [{"text": "go"}]},
		{"role": "model", "parts": [
			{"text": "A.", "thoughtSignature": "CjEBc2lnbmF0dXJl"},
			{"text": "B.", "thoughtSignature": signed(other, &[thinking("x")])}
		]},
		{"role": "user", "parts": [{"text": "more"}], "thoughtSignature": "x"}
	]});
	let problems = problems(&body, MESSAGES);
	assert_eq!(
		problems[..3],
		[
			"`contents[1].parts[0].thoughtSignature` was not written by this gateway",
			"field `contents[2].thoughtSignature`",
			"no `generationConfig.maxOutputTokens`, and no default for this model",
		]
	);
	assert_eq!(problems.len(), 4);
	assert!(problems[3].ends_with("carrier does not match this destination"));
}

#[test]
fn a_gemini_client_on_a_gemini_provider_is_refused() {
	let upstream = Upstream {
		protocol: UpstreamProtocol::Gemini,
		..RESPONSES
	};
	assert_eq!(
		problems(&json!({"contents": hi()}), upstream),
		["same-protocol requests are not supported by the conversion pipeline"]
	);
}

mod read_tests {
	#![allow(clippy::panic, clippy::unwrap_used)]

	use rstest::rstest;

	use super::super::*;
	use crate::prompt::{self, RequestRefusal as Refusal, Upstream};
	use crate::protocols::{ClientEndpoint, Delivery, UpstreamProtocol};
	use crate::test_support::{self, SCOPE, Sum};

	const CLIENT: ClientEndpoint = ClientEndpoint::Gemini(Delivery::Buffered);
	const MESSAGES: Upstream<'static> = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		protocol: UpstreamProtocol::Messages,
		default_max_tokens: None,
	};

	/// Checks both ways of reading, and returns what they found.
	fn problems(body: &str) -> Vec<String> {
		assert!(
			prompt::one_pass_parse_agrees(body.as_bytes(), CLIENT, MESSAGES),
			"{body}"
		);
		match test_support::convert(body.as_bytes(), CLIENT, MESSAGES) {
			Ok(_) => Vec::new(),
			Err(Refusal { problems }) => problems,
		}
	}

	fn with_contents(contents: &str) -> String {
		format!(r#"{{"generationConfig":{{"maxOutputTokens":8}},"contents":{contents}}}"#)
	}

	fn texts<'a>(read: &BodyRead<'a>) -> Vec<Cow<'a, str>> {
		let Some(List(turns)) = &read.contents else {
			panic!("no contents");
		};
		let mut texts = Vec::new();
		for turn in turns {
			let Read::Done(TurnRead {
				parts: Some(List(parts)),
				..
			}) = turn
			else {
				panic!("turn not read");
			};
			for part in parts {
				let Read::Done(PartRead {
					text: Some(text), ..
				}) = part
				else {
					panic!("part not read");
				};
				texts.push(text.clone());
			}
		}
		texts
	}

	#[test]
	fn turns_parts_and_text_are_read_in_one_pass() {
		let body = with_contents(
			r#"[{"role":"user","parts":[{"text":"hi"}]},{"parts":[{"text":"a\nb"}],"role":"model"}]"#,
		);
		let read = read_body(&body).unwrap();
		let texts = texts(&read);
		assert_eq!(texts, ["hi", "a\nb"]);
		assert!(matches!(texts[0], Cow::Borrowed(_)));
		assert!(matches!(texts[1], Cow::Owned(_)));
		assert_eq!(problems(&body), Vec::<String>::new());
	}

	#[test]
	fn function_responses_are_read_with_their_part() {
		let body = with_contents(
			r#"[{"role":"user","parts":[{"text":"go"}]},{"role":"model","parts":[{"functionCall":{"name":"read"}}]},{"role":"user","parts":[{"functionResponse":{"name":"read","response":{"output":"a\nb"}}}]}]"#,
		);
		let read = read_body(&body).unwrap();
		let Some(List(turns)) = &read.contents else {
			panic!("no contents");
		};
		let Read::Done(TurnRead {
			parts: Some(List(parts)),
			..
		}) = &turns[2]
		else {
			panic!("turn not read");
		};
		assert!(matches!(
			&parts[0],
			Read::Done(PartRead {
				function_response: Some(FunctionResponseRead {
					response: Some(_),
					..
				}),
				..
			})
		));
		assert_eq!(problems(&body), Vec::<String>::new());
	}

	#[rstest]
	#[case::output(r#"{"output":"a"}"#)]
	#[case::error_object(r#"{"error":{"code":7}}"#)]
	#[case::more_than_the_text(r#"{"output":"a","more":1}"#)]
	#[case::not_an_object(r#""a""#)]
	#[case::lone_surrogate(r#"{"output":"\ud800"}"#)]
	fn function_responses_read_the_same_both_ways(#[case] response: &str) {
		let body = with_contents(&format!(
			r#"[{{"role":"user","parts":[{{"text":"go"}}]}},{{"role":"model","parts":[{{"functionCall":{{"name":"read"}}}}]}},{{"role":"user","parts":[{{"functionResponse":{{"name":"read","response":{response}}}}}]}}]"#
		));
		problems(&body);
	}

	#[rstest]
	#[case::repeated_parts(
		r#"[{"role":"user","parts":[{"text":"a"}],"parts":[]}]"#,
		"`contents[0]` repeats `parts`"
	)]
	#[case::snake_case_repeat(
		r#"[{"role":"user","parts":[{"text":"a"}],"parts_":[]}]"#,
		"`contents[0]` repeats `parts`"
	)]
	#[case::repeated_text(
		r#"[{"role":"user","parts":[{"text":"a","text":"b"}]}]"#,
		"`contents[0].parts[0]` repeats `text`"
	)]
	#[case::text_not_a_string(
		r#"[{"role":"user","parts":[{"text":5}]}]"#,
		"`contents[0].parts[0].text` has the wrong type"
	)]
	#[case::lone_surrogate(
		r#"[{"role":"user","parts":[{"text":"\ud800"}]}]"#,
		"`contents[0].parts[0].text` has the wrong type"
	)]
	#[case::repeated_function_response(
		r#"[{"role":"user","parts":[{"functionResponse":{"name":"a"},"functionResponse":{"name":"b"}}]}]"#,
		"`contents[0].parts[0]` repeats `functionResponse`"
	)]
	#[case::snake_case_function_response(
		r#"[{"role":"user","parts":[{"function_response":{"name":"a","response":{}}}]}]"#,
		"`contents[0].parts[0].functionResponse` answers no open function call"
	)]
	#[case::repeated_response(
		r#"[{"role":"user","parts":[{"functionResponse":{"name":"a","response":{},"response":{}}}]}]"#,
		"`contents[0].parts[0].functionResponse` repeats `response`"
	)]
	#[case::text_and_function_response(
		r#"[{"role":"user","parts":[{"text":"a","functionResponse":{"name":"a"}}]}]"#,
		"`contents[0].parts[0]` has more than one kind of data"
	)]
	#[case::function_response_and_other_data(
		r#"[{"role":"user","parts":[{"functionResponse":{"name":"a"},"inlineData":{}}]}]"#,
		"`contents[0].parts[0]` has more than one kind of data"
	)]
	#[case::other_data_first(
		r#"[{"role":"user","parts":[{"inlineData":{},"extra":1,"text":"a","more":2}]}]"#,
		"field `contents[0].parts[0].extra`"
	)]
	fn unusual_bodies_are_left_to_the_level_by_level_read(
		#[case] contents: &str,
		#[case] problem: &str,
	) {
		let body = with_contents(contents);
		assert!(read_body(&body).is_none());
		assert_eq!(problems(&body)[0], problem);
	}

	#[rstest]
	#[case::not_a_list("5")]
	#[case::empty("[]")]
	#[case::not_a_turn("[5]")]
	#[case::no_parts(r#"[{"role":"user"}]"#)]
	#[case::parts_not_a_list(r#"[{"role":"user","parts":{}}]"#)]
	#[case::not_a_part(r#"[{"role":"user","parts":[5]}]"#)]
	#[case::bad_role(r#"[{"role":"robot","parts":[{"text":"a"}]}]"#)]
	#[case::escaped_role(r#"[{"role":"user","parts":[{"text":"a"}],"x":1}]"#)]
	#[case::thought_in_a_user_turn(r#"[{"role":"user","parts":[{"text":"a","thought":true}]}]"#)]
	#[case::no_data(r#"[{"role":"user","parts":[{"x":1}]}]"#)]
	fn wrong_shapes_read_the_same_both_ways(#[case] contents: &str) {
		assert!(!problems(&with_contents(contents)).is_empty());
	}

	#[rstest]
	#[case::empty("")]
	#[case::list("[]")]
	#[case::truncated("{")]
	#[case::missing_contents("{}")]
	#[case::snake_case_contents(r#"{"contents":[],"contents_":[]}"#)]
	#[case::trailing_data(r#"{"contents":[]} x"#)]
	fn wrong_request_shapes_read_the_same_both_ways(#[case] body: &str) {
		assert!(!problems(body).is_empty());
	}
}
