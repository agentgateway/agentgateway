#![allow(clippy::panic, clippy::unwrap_used)]

use serde_json::json;

use super::*;
use crate::carrier::{CarrierPurpose, StateScope, encode as encode_carrier};
use crate::protocols::messages::parse::{Parsed, parse};
use crate::test_support::SCOPE;

const OTHER: StateScope<'static> = StateScope {
	backend: "a",
	provider: "p",
	model: "another",
};

fn request(signature: &str, content: &serde_json::Value) -> Vec<u8> {
	json!({"model": "m", "max_tokens": 1, "messages": [
		{"role": "user", "content": "hi"},
		{"role": "assistant", "content": [
			{"type": "thinking", "thinking": "", "signature": signature},
			content,
		]},
		{"role": "user", "content": [
			{"type": "tool_result", "tool_use_id": "a", "content": "sunny"},
		]},
	]})
	.to_string()
	.into_bytes()
}

fn call() -> serde_json::Value {
	json!({"type": "tool_use", "id": "a", "name": "f", "input": {}})
}

const SIGNED_CALL: &[u8] = br#"{"functionCall":{"name":"f"},"thoughtSignature":"s"}"#;

fn encode(scope: StateScope<'_>, state: &[u8]) -> String {
	encode_carrier(scope, CarrierPurpose::MessagesSignature, state)
}

fn remove_reasoning(parsed: &mut Parsed<'_>) {
	for message in &mut parsed.conversation.messages {
		message
			.blocks
			.retain(|block| !matches!(block.content, Content::Reasoning { .. }));
	}
}

#[test]
fn rendering_decides_what_happens_to_settings_gemini_ignores() {
	let body = json!({
		"model": "m",
		"max_tokens": 1,
		"metadata": {"user_id": "u"},
		"output_config": {"made_up": {}},
		"context_management": {"edits": []},
		"thinking": {"type": "enabled", "budget_tokens": 9},
		"messages": [{"role": "user", "content": "hi", "output_config": {}}],
	})
	.to_string();
	let mut parsed = parse(body.as_bytes(), SCOPE);
	assert!(parsed.diagnostics.dropped.is_empty());
	let settings = &parsed.conversation.settings;
	assert_eq!(
		settings
			.messages_input
			.thinking
			.as_ref()
			.and_then(|thinking| thinking.mode),
		Some(crate::ir::conversation::MessagesThinkingMode::Enabled)
	);
	assert_eq!(
		settings
			.messages_input
			.metadata
			.map(|value| value.encoded()),
		Some(Cow::Borrowed(r#"{"user_id":"u"}"#))
	);
	parsed.conversation.settings.messages_input.metadata = None;
	parsed.conversation.messages[0].messages_output_config = None;
	let (_, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert_eq!(
		diagnostics.dropped,
		[
			"context_management",
			"thinking.budget_tokens",
			"output_config",
		]
	);
}

#[test]
fn a_carrier_from_another_scope_stays_until_rendering() {
	let body = request(&encode(OTHER, SIGNED_CALL), &call());
	let parsed = parse(&body, SCOPE);
	assert!(parsed.diagnostics.problems.is_empty());
	let (_, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert_eq!(diagnostics.problems.len(), 1, "{:?}", diagnostics.problems);
}

#[test]
fn removing_a_carrier_with_the_wrong_destination_rescues_the_request() {
	let body = request(&encode(OTHER, SIGNED_CALL), &call());
	let mut parsed = parse(&body, SCOPE);
	remove_reasoning(&mut parsed);
	let (rendered, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert_eq!(diagnostics.problems, Vec::<String>::new());
	assert!(!rendered.contains("thoughtSignature"), "{rendered}");
}

#[test]
fn a_signature_never_moves_to_another_block() {
	let body = request(&encode(SCOPE, SIGNED_CALL), &call());
	let mut parsed = parse(&body, SCOPE);
	let blocks = &mut parsed.conversation.messages[1].blocks;
	blocks[1].id.block = 9;
	let (_, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	let wanted = "`messages[1].content[9]` doesn't match its signature";
	assert!(diagnostics.problems.iter().any(|problem| problem == wanted));
}

#[test]
fn a_signed_call_left_without_its_call_is_refused() {
	let body = request(&encode(SCOPE, SIGNED_CALL), &call());
	let mut parsed = parse(&body, SCOPE);
	parsed.conversation.messages[1].blocks.pop();
	let (_, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert!(
		diagnostics
			.problems
			.iter()
			.any(|problem| problem.contains("has no call")),
		"{:?}",
		diagnostics.problems
	);
}

#[test]
fn signed_text_left_without_its_text_goes_back_alone() {
	let item = br#"{"text":"","thoughtSignature":"s"}"#;
	let content = json!({"type": "tool_use", "id": "a", "name": "f", "input": {}});
	let body = request(&encode(SCOPE, item), &content);
	let parsed = parse(&body, SCOPE);
	let (rendered, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert_eq!(diagnostics.problems, Vec::<String>::new());
	assert!(
		rendered.contains(r#"{"text":"","thoughtSignature":"s"}"#),
		"{rendered}"
	);
}

#[test]
fn text_left_after_removing_an_image_is_sent() {
	let carrier = encode(SCOPE, SIGNED_CALL);
	let image = json!({"type": "image", "source": {"type": "url", "url": "u"}});
	let body = json!({"model": "m", "max_tokens": 1, "messages": [
		{"role": "user", "content": "hi"},
		{"role": "assistant", "content": [
			{"type": "thinking", "thinking": "", "signature": carrier},
			call(),
		]},
		{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "a", "content": [
			{"type": "text", "text": "one"},
			image,
			{"type": "text", "text": "two"},
		]}]},
	]})
	.to_string();
	let mut parsed = parse(body.as_bytes(), SCOPE);
	let Content::ToolResult { output, .. } = &mut parsed.conversation.messages[2].blocks[0].content
	else {
		panic!("not a tool result");
	};
	let ToolOutput::Mixed(parts) = output else {
		panic!("not mixed");
	};
	parts.retain(|part| matches!(part, ToolPart::Text(_)));
	let (rendered, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert_eq!(diagnostics.problems, Vec::<String>::new());
	assert!(rendered.contains(r#""content":"one\ntwo""#), "{rendered}");
}

#[test]
fn an_image_in_a_tool_result_stays_with_its_call() {
	let carrier = encode(SCOPE, SIGNED_CALL);
	let conversation = |image: serde_json::Value| {
		json!({"model": "m", "max_tokens": 1, "messages": [
			{"role": "user", "content": "hi"},
			{"role": "assistant", "content": [
				{"type": "thinking", "thinking": "", "signature": carrier},
				call(),
			]},
			{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "a", "content": [
				{"type": "text", "text": "one"},
				image,
			]}]},
		]})
		.to_string()
	};

	let body = conversation(json!({"type": "image", "source": {
		"type": "base64", "media_type": "image/png", "data": "AAAA",
	}}));
	let parsed = parse(body.as_bytes(), SCOPE);
	let (rendered, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert_eq!(diagnostics.problems, Vec::<String>::new());
	assert!(
		rendered.contains(
			r#""response":{"content":"one"},"parts":[{"inlineData":{"mimeType":"image/png","data":"AAAA"}}]}}"#
		),
		"{rendered}"
	);

	let body = conversation(json!({"type": "image", "source": {"type": "url", "url": "u"}}));
	let parsed = parse(body.as_bytes(), SCOPE);
	let (_, diagnostics) = render(&parsed.conversation, &parsed.unread, parsed.diagnostics);
	assert_eq!(
		diagnostics.problems,
		[
			"field `messages[2].content[0].content[1].source.url`",
			"`messages[2].content[0].content[1].source` must be base64",
		]
	);
}

mod conversion_tests {
	#![allow(clippy::panic, clippy::unwrap_used)]

	use std::num::NonZeroU64;

	use rstest::rstest;
	use serde_json::{Value, json};

	use crate::carrier::{CarrierPurpose, StateScope, encode as encode_carrier};
	use crate::prompt::{
		self, ConvertedRequest, RenderSettings, RequestRefusal as Refusal, Upstream,
	};
	use crate::protocols::gemini::render::{ThinkingControls, ThinkingLevel, ThinkingLevels};
	use crate::protocols::{CallKind, ClientProtocol, UpstreamProtocol};
	use crate::test_support::{self, SCOPE, Sum};

	const CONFIG: Upstream<'static> = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		default_max_tokens: NonZeroU64::new(9),
		protocol: UpstreamProtocol::Gemini,
	};

	fn encode(scope: StateScope<'_>, state: &[u8]) -> String {
		encode_carrier(scope, CarrierPurpose::ResponsesEncryptedContent, state)
	}

	fn sent(body: &Value) -> Value {
		let converted = test_support::convert(
			body.to_string().as_bytes(),
			ClientProtocol::Responses,
			CONFIG,
		)
		.unwrap();
		serde_json::from_str(&converted.body).unwrap()
	}

	fn carrier(part: &Value) -> String {
		encode(SCOPE, part.to_string().as_bytes())
	}

	fn answer(text: &str) -> Value {
		json!({"type": "message", "role": "assistant",
			"content": [{"type": "output_text", "text": text}]})
	}

	fn reasoning(part: &Value) -> Value {
		json!({"type": "reasoning", "summary": [], "encrypted_content": carrier(part)})
	}

	#[test]
	fn codex_json_tool_results_keep_values_separate_from_output_metadata() {
		let metadata = "Wall time: 0.0081 seconds\nOutput:";
		let payload = json!({
			"quote": "line1\nline2 \"quoted\" \\path",
			"literal_escape": "\\n",
			"marker": "日本語-λ-✓",
			"nested": {"empty": "", "nil": null, "list": [0, false, {"key": "value"}]},
		});
		let body = json!({"model": "m", "input": [
			{"role": "user", "content": "Copy the tool result exactly."},
			{"type": "function_call", "call_id": "c", "name": "read", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "c", "output": [
				{"type": "input_text", "text": metadata},
				{"type": "input_text", "text": payload.to_string()},
				{"type": "input_text", "text": "End of result."},
			]},
		]});
		let converted = sent(&body);
		assert_eq!(
			converted["contents"][2]["parts"][0]["functionResponse"]["response"],
			json!({"output": [metadata, payload, "End of result."]})
		);
		let mut with_image = body.clone();
		with_image["input"][2]["output"]
			.as_array_mut()
			.unwrap()
			.push(json!({
				"type": "input_image", "image_url": "data:image/png;base64,AAAA",
			}));
		let converted = sent(&with_image);
		assert_eq!(
			converted["contents"][2]["parts"][0]["functionResponse"],
			json!({
				"name": "read",
				"response": {"output": [metadata, payload, "End of result."]},
				"parts": [{"inlineData": {"mimeType": "image/png", "data": "AAAA"}}],
			})
		);

		let failed = test_support::convert(
			json!({"model": "m", "max_tokens": 1, "messages": [
				{"role": "user", "content": "go"},
				{"role": "assistant", "content": [{
					"type": "tool_use", "id": "c", "name": "read", "input": {},
				}]},
				{"role": "user", "content": [{
					"type": "tool_result", "tool_use_id": "c", "is_error": true,
					"content": [{"type": "text", "text": payload.to_string()}],
				}]},
			]})
			.to_string()
			.as_bytes(),
			ClientProtocol::Messages,
			CONFIG,
		)
		.unwrap();
		let failed: Value = serde_json::from_str(&failed.body).unwrap();
		assert_eq!(
			failed["contents"][2]["parts"][0]["functionResponse"]["response"],
			json!({"content": format!("[tool error]\n{payload}")})
		);

		for text in [
			"line1\nline2 \"quoted\" \\path",
			"\\n",
			r#""a JSON string""#,
			"123",
			"true",
			"null",
			r#"{"unfinished": "#,
			r#"{"a":1,"a":2}"#,
			r#"{"number":1e999}"#,
			r#"{"text":"\uD800"}"#,
			"Output: {\"value\": 1}",
		] {
			let mut body = body.clone();
			body["input"][2]["output"] = json!([
				{"type": "input_text", "text": metadata},
				{"type": "input_text", "text": text},
			]);
			assert_eq!(
				sent(&body)["contents"][2]["parts"][0]["functionResponse"]["response"],
				json!({"content": format!("{metadata}\n{text}")}),
				"{text}"
			);
		}
	}

	#[rstest]
	#[case::object(json!({"quote": "a\nb", "literal": "\\n"}))]
	#[case::array(json!([null, false, {"quote": "a\nb", "literal": "\\n"}]))]
	fn a_complete_json_tool_result_is_structured(#[case] output: Value) {
		let converted = sent(&json!({"model": "m", "input": [
			{"role": "user", "content": "go"},
			{"type": "function_call", "call_id": "c", "name": "read", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "c", "output": output.to_string()},
		]}));
		assert_eq!(
			converted["contents"][2]["parts"][0]["functionResponse"]["response"],
			json!({"output": [output]})
		);
	}

	#[test]
	fn a_signed_call_goes_back_with_its_signature() {
		let signed = json!({"functionCall": {"name": "run", "args": {}}, "thoughtSignature": "s"});
		let sent = sent(&json!({"model": "m", "input": [
			{"role": "user", "content": "hi"},
			reasoning(&signed),
			{"type": "function_call", "call_id": "c", "name": "run", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "c", "output": "done"},
		]}));
		assert_eq!(
			sent["contents"][1]["parts"],
			json!([{"functionCall": {"name": "run", "args": {}}, "thoughtSignature": "s"}])
		);
		assert_eq!(
			sent["contents"][2]["parts"][0]["functionResponse"]["name"],
			"run"
		);
	}

	#[test]
	fn signed_text_after_its_answer_isnt_sent_twice() {
		let signed = json!({"text": "world", "thoughtSignature": "s"});
		let sent = sent(&json!({"model": "m", "input": [
			{"role": "user", "content": "hi"},
			answer("hello world"),
			reasoning(&signed),
			{"role": "user", "content": "again"},
		]}));
		assert_eq!(
			sent["contents"][1]["parts"],
			json!([{"text": "hello "}, {"text": "world", "thoughtSignature": "s"}])
		);
	}

	#[test]
	fn signed_text_before_a_call_keeps_its_place() {
		let signed_text = json!({"text": "ok", "thoughtSignature": "t"});
		let signed_call = json!({"functionCall": {"name": "run", "args": {}}, "thoughtSignature": "c"});
		let sent = sent(&json!({"model": "m", "input": [
			{"role": "user", "content": "hi"},
			answer("ok"),
			reasoning(&signed_text),
			reasoning(&signed_call),
			{"type": "function_call", "call_id": "c", "name": "run", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "c", "output": "done"},
		]}));
		assert_eq!(
			sent["contents"][1]["parts"],
			json!([
				{"text": "ok", "thoughtSignature": "t"},
				{"functionCall": {"name": "run", "args": {}}, "thoughtSignature": "c"},
			])
		);
	}

	#[test]
	fn full_requests_keep_their_exact_bytes() {
		let image = "data:image/png;base64,AAAA";
		let thought = r#"{"text": "caf\u00e9", "thought": true, "thoughtSignature": "s1"}"#;
		let carrier = |part: &str| encode(SCOPE, part.as_bytes());
		let with_tools = json!({
			"model": "m", "instructions": "be brief", "max_output_tokens": 64,
			"tools": [{"type": "function", "name": "look", "description": "Look.",
				"parameters": {"type": "object", "properties": {"at": {"type": "integer"}}}}],
			"tool_choice": {"type": "function", "name": "look"},
			"reasoning": {"effort": "high", "summary": "auto"},
			"text": {"format": {"type": "json_schema", "name": "out", "schema": {"type": "object"}}},
			"input": [
				{"role": "developer", "content": "first rule"},
				{"role": "user", "content": [
					{"type": "input_text", "text": "look ☃"},
					{"type": "input_image", "image_url": image},
				]},
				{"type": "reasoning", "summary": [], "encrypted_content": carrier(thought)},
				answer("fine ok"),
				reasoning(&json!({"text": "ok", "thoughtSignature": "s2"})),
				reasoning(&json!({"functionCall": {"name": "look", "args": {}}, "thoughtSignature": "s3"})),
				{"type": "function_call", "call_id": "c1", "name": "look", "arguments": "{\"at\": 1}"},
				{"role": "developer", "content": "later rule"},
				{"type": "function_call_output", "call_id": "c1", "output": [
					{"type": "input_text", "text": "seen"},
					{"type": "input_image", "image_url": image},
				]},
				{"role": "user", "content": "thanks"},
			],
		});
		let with_format = json!({
			"model": "m", "temperature": 0.5, "top_p": 1,
			"text": {"format": {"type": "json_schema", "name": "out", "schema": {"type": "object"}}},
			"input": [{"role": "user", "content": "hi"}],
		});
		let body = |request: &Value| {
			test_support::convert(
				request.to_string().as_bytes(),
				ClientProtocol::Responses,
				CONFIG,
			)
			.unwrap()
			.body
		};
		let (with_tools, with_format) = (body(&with_tools), body(&with_format));
		let expected_tools = concat!(
			r#"{"contents":["#,
			r#"{"role":"user","parts":["#,
			r#"{"text":"look ☃"}"#,
			r#",{"inlineData":{"mimeType":"image/png","data":"AAAA"}}]}"#,
			r#",{"role":"model","parts":["#,
			r#"{"text": "caf\u00e9", "thought": true, "thoughtSignature": "s1"}"#,
			r#",{"text":"fine "}"#,
			r#",{"text":"ok","thoughtSignature":"s2"}"#,
			r#",{"functionCall":{"name":"look","args":{"at": 1}},"thoughtSignature":"s3"}]}"#,
			r#",{"role":"user","parts":["#,
			r#"{"functionResponse":{"name":"look","response":{"content":"seen"},"parts":["#,
			r#"{"inlineData":{"mimeType":"image/png","data":"AAAA"}}]}}]}"#,
			r#",{"role":"user","parts":["#,
			r#"{"text":"<system-reminder>\nlater rule\n</system-reminder>"}"#,
			r#",{"text":"thanks"}]}]"#,
			r#","systemInstruction":{"parts":["#,
			r#"{"text":"be brief\n\nfirst rule\n\nWhen you give your final answer, after any tool calls, reply with only a JSON value matching this schema:\n{\"type\":\"object\"}"}]}"#,
			r#","tools":[{"functionDeclarations":[{"name":"look","description":"Look.","parameters":{"type":"object","properties":{"at":{"type":"integer"}}}}]}]"#,
			r#","toolConfig":{"functionCallingConfig":{"mode":"ANY","allowedFunctionNames":["look"]}}"#,
			r#","generationConfig":{"maxOutputTokens":64,"thinkingConfig":{"includeThoughts":true,"thinkingLevel":"high"}}}"#,
		);
		let expected_format = concat!(
			r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]"#,
			r#","generationConfig":{"maxOutputTokens":9,"temperature":0.5,"topP":1,"responseMimeType":"application/json","responseSchema":{"type":"object"}}}"#,
		);
		assert_eq!(with_tools, expected_tools);
		assert_eq!(with_format, expected_format);
	}

	#[test]
	fn custom_and_namespaced_tools_are_flattened_and_routed() {
		let body = json!({"model": "m", "input": [{"role": "user", "content": "hi"}], "tools": [
			{"type": "custom", "name": "apply_patch", "description": "Patch."},
			{"type": "namespace", "name": "docs", "tools": [
				{"type": "function", "name": "read", "strict": false,
					"parameters": {"type": "object", "properties": {}}},
			]},
		]})
		.to_string();
		let converted =
			test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		let declared = &sent["tools"][0]["functionDeclarations"];
		assert_eq!(declared[0]["name"], "apply_patch");
		assert_eq!(declared[0]["parameters"]["required"], json!(["input"]));
		assert_eq!(declared[1]["name"], "docs__read");
		let routes: Vec<_> = converted
			.reply
			.routes
			.iter()
			.map(|route| (route.flat.as_str(), route.namespace.as_deref(), route.kind))
			.collect();
		assert_eq!(
			routes,
			[
				("apply_patch", None, CallKind::Custom),
				("docs__read", Some("docs"), CallKind::Function)
			]
		);
		assert_eq!(converted.dropped, ["tool strict"]);
	}

	#[test]
	fn history_of_a_namespaced_custom_call_uses_the_flat_name() {
		let sent = sent(&json!({"model": "m", "input": [
			{"role": "user", "content": "hi"},
			{"type": "custom_tool_call", "call_id": "c", "name": "patch", "input": "text"},
			{"type": "custom_tool_call_output", "call_id": "c", "output": "ok"},
		]}));
		assert_eq!(
			sent["contents"][1]["parts"][0]["functionCall"],
			json!({"name": "patch", "args": {"input": "text"}})
		);
	}

	#[rstest]
	#[case("low", "low")]
	#[case("medium", "medium")]
	#[case("high", "high")]
	#[case("xhigh", "high")]
	fn effort_is_a_thinking_level(#[case] effort: &str, #[case] level: &str) {
		let sent = sent(
			&json!({"model": "m", "temperature": 0.5, "reasoning": {"effort": effort},
			"input": [{"role": "user", "content": "hi"}]}),
		);
		assert_eq!(
			sent["generationConfig"],
			json!({"maxOutputTokens": 9, "temperature": 0.5, "thinkingConfig": {"thinkingLevel": level}})
		);
	}

	#[test]
	fn no_effort_is_the_lowest_level_because_a_level_cant_be_off() {
		for effort in ["none", "minimal"] {
			let sent = sent(&json!({"model": "m", "reasoning": {"effort": effort},
				"input": [{"role": "user", "content": "hi"}]}));
			assert_eq!(
				sent["generationConfig"]["thinkingConfig"],
				json!({"thinkingLevel": "low"})
			);
		}
		let body = json!({"model": "m", "reasoning": {"effort": "none"},
			"input": [{"role": "user", "content": "hi"}]})
		.to_string();
		let converted =
			test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
		assert_eq!(converted.dropped, ["reasoning.effort none"]);
	}

	#[test]
	fn thinking_off_is_recorded_wherever_gemini_keeps_thinking() {
		let messages = json!({"model": "c", "max_tokens": 9, "thinking": {"type": "disabled"},
			"messages": [{"role": "user", "content": "hi"}]})
		.to_string();
		let responses = json!({"model": "c", "reasoning": {"effort": "none"},
			"input": [{"role": "user", "content": "hi"}]})
		.to_string();
		for (model, thinking, recorded) in [
			("gemini-3-pro", json!({"thinkingLevel": "low"}), true),
			("gemini-2.5-pro", json!({"thinkingBudget": 128}), true),
			("gemini-2.5-flash", json!({"thinkingBudget": 0}), false),
		] {
			let config = Upstream {
				scope: StateScope { model, ..SCOPE },
				protocol: UpstreamProtocol::Gemini,
				..CONFIG
			};
			for (client, body, field) in [
				(
					ClientProtocol::Messages,
					&messages,
					"thinking.type disabled",
				),
				(
					ClientProtocol::Responses,
					&responses,
					"reasoning.effort none",
				),
			] {
				let converted = test_support::convert(body.as_bytes(), client, config).unwrap();
				let sent: Value = serde_json::from_str(&converted.body).unwrap();
				assert_eq!(
					sent["generationConfig"]["thinkingConfig"], thinking,
					"{model}"
				);
				assert_eq!(
					converted.dropped.contains(&field),
					recorded,
					"{model} {client:?}: {:?}",
					converted.dropped
				);
			}
		}
	}

	#[rstest]
	#[case("gemini-2.5-flash", "none", 0)]
	#[case("gemini-2.5-pro", "none", 128)]
	#[case("gemini-2.5-flash", "minimal", 1024)]
	#[case("gemini-2.5-flash", "low", 1024)]
	#[case("gemini-2.5-pro", "medium", 2048)]
	#[case("gemini-2.5-pro", "high", 4096)]
	#[case("gemini-2.5-pro", "xhigh", 8192)]
	fn gemini_2_takes_a_budget_for_effort(
		#[case] model: &'static str,
		#[case] effort: &str,
		#[case] budget: u32,
	) {
		let config = Upstream {
			scope: StateScope { model, ..SCOPE },
			protocol: UpstreamProtocol::Gemini,
			..CONFIG
		};
		let body = json!({"model": "c", "reasoning": {"effort": effort},
			"input": [{"role": "user", "content": "hi"}]})
		.to_string();
		let converted =
			test_support::convert(body.as_bytes(), ClientProtocol::Responses, config).unwrap();
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		assert_eq!(
			sent["generationConfig"]["thinkingConfig"],
			json!({"thinkingBudget": budget})
		);
	}

	fn rendered_with(
		effort: &str,
		model: &str,
		settings: prompt::RenderSettings,
	) -> (Value, Vec<&'static str>) {
		let config = Upstream {
			scope: StateScope { model, ..SCOPE },
			protocol: UpstreamProtocol::Gemini,
			..CONFIG
		};
		let body = json!({"model": "c", "reasoning": {"effort": effort},
			"input": [{"role": "user", "content": "hi"}]})
		.to_string();
		let prepared = prompt::prepare(body.as_bytes(), ClientProtocol::Responses, config)
			.unwrap_or_else(|refusal| panic!("{refusal}"));
		let converted = prepared.render(settings).unwrap();
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		(
			sent["generationConfig"]["thinkingConfig"].clone(),
			converted.dropped,
		)
	}

	/// Gemma 4 only turns thinking on and off, with `high` and `minimal`.
	#[rstest]
	#[case("none", "minimal")]
	#[case("low", "minimal")]
	#[case("medium", "high")]
	#[case("high", "high")]
	fn gemma_4_by_name_is_sent_only_its_levels(#[case] effort: &str, #[case] level: &str) {
		let (sent, _) = rendered_with(
			effort,
			"gemma-4-31b-it",
			prompt::RenderSettings::ModelDefaults,
		);
		assert_eq!(sent, json!({"thinkingLevel": level}));
	}

	/// The catalog's levels win over the name, and effort goes to the nearest one, the higher
	/// of two as near.
	#[rstest]
	#[case(&[ThinkingLevel::Minimal, ThinkingLevel::Low, ThinkingLevel::Medium, ThinkingLevel::High], "minimal", "minimal")]
	#[case(&[ThinkingLevel::Low, ThinkingLevel::High], "medium", "high")]
	#[case(&[ThinkingLevel::Low, ThinkingLevel::High], "minimal", "low")]
	#[case(&[ThinkingLevel::Minimal, ThinkingLevel::High], "low", "minimal")]
	#[case(&[ThinkingLevel::Medium], "xhigh", "medium")]
	fn effort_goes_to_the_nearest_level_the_catalog_lists(
		#[case] levels: &[ThinkingLevel],
		#[case] effort: &str,
		#[case] level: &str,
	) {
		let levels = levels
			.iter()
			.fold(ThinkingLevels::default(), |levels, level| {
				levels.with(*level)
			});
		let (sent, _) = rendered_with(
			effort,
			"gemini-2.5-pro",
			prompt::RenderSettings::Gemini(ThinkingControls::Levels(levels)),
		);
		assert_eq!(sent, json!({"thinkingLevel": level}));
	}

	/// A budget stays in the catalog's range, and a model that can't stop thinking gets its
	/// smallest budget for `none`.
	#[rstest]
	#[case("xhigh", 0, 1000, true, 1000)]
	#[case("low", 2000, 32768, true, 2000)]
	#[case("none", 512, 24576, true, 0)]
	#[case("none", 128, 32768, false, 128)]
	fn a_budget_stays_in_the_catalogs_range(
		#[case] effort: &str,
		#[case] min: u32,
		#[case] max: u32,
		#[case] can_stop: bool,
		#[case] budget: u32,
	) {
		let (sent, _) = rendered_with(
			effort,
			"gemini-3-pro",
			prompt::RenderSettings::Gemini(ThinkingControls::Budget { min, max, can_stop }),
		);
		assert_eq!(sent, json!({"thinkingBudget": budget}));
	}

	/// Gemini stops on these itself, so the reply has none left to emulate.
	#[test]
	fn stop_sequences_are_sent_in_the_generation_config() {
		let convert = |stop: Value| {
			let body = json!({"model": "m", "max_tokens": 9, "stop_sequences": stop,
				"messages": [{"role": "user", "content": "hi"}]});
			test_support::convert(
				body.to_string().as_bytes(),
				ClientProtocol::Messages,
				CONFIG,
			)
			.unwrap()
		};
		let converted = convert(json!(["END", "STOP"]));
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		assert_eq!(
			sent["generationConfig"],
			json!({"maxOutputTokens": 9, "stopSequences": ["END", "STOP"]})
		);
		assert!(converted.reply.emulated_stop_sequences.is_empty());

		let sent: Value = serde_json::from_str(&convert(json!([])).body).unwrap();
		assert_eq!(sent["generationConfig"], json!({"maxOutputTokens": 9}));
	}

	#[test]
	fn text_format_becomes_a_json_reply() {
		let ask = |format: Value| {
			json!({"model": "m", "text": {"format": format},
				"input": [{"role": "user", "content": "hi"}]})
		};
		let config = |sent: Value| sent["generationConfig"].clone();
		assert_eq!(
			config(sent(&ask(json!({"type": "text"})))),
			json!({"maxOutputTokens": 9})
		);
		assert_eq!(
			config(sent(&ask(json!({"type": "json_object"})))),
			json!({"maxOutputTokens": 9, "responseMimeType": "application/json"})
		);
		let schema = json!({
			"$defs": {"Name": {"type": "string"}},
			"type": "object",
			"properties": {"a": {"$ref": "#/$defs/Name"}},
		});
		let format = json!({"type": "json_schema", "name": "n", "strict": true, "schema": schema});
		assert_eq!(
			config(sent(&ask(format))),
			json!({
				"maxOutputTokens": 9,
				"responseMimeType": "application/json",
				"responseSchema": {"type": "object", "properties": {"a": {"type": "string"}}},
			})
		);
	}

	#[test]
	fn with_tools_a_format_is_asked_for_not_enforced() {
		let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
		let body = json!({
			"model": "m",
			"instructions": "Be brief.",
			"tools": [{"type": "function", "name": "run",
				"parameters": {"type": "object", "properties": {}}}],
			"text": {"format": {"type": "json_schema", "name": "n", "schema": schema}},
			"input": [{"role": "user", "content": "hi"}],
		})
		.to_string();
		let converted =
			test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
		assert!(
			converted
				.dropped
				.contains(&"text.format enforcement beside tools")
		);
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		assert_eq!(sent["generationConfig"], json!({"maxOutputTokens": 9}));
		let system = sent["systemInstruction"]["parts"][0]["text"]
			.as_str()
			.unwrap();
		assert!(system.starts_with("Be brief."), "{system}");
		assert!(
			system.contains(r#"{"type":"object","properties":{"a":{"type":"string"}}}"#),
			"{system}"
		);
	}

	#[test]
	fn a_summary_asks_for_thoughts() {
		let ask = |summary: &str| {
			sent(&json!({"model": "m", "reasoning": {"summary": summary},
				"input": [{"role": "user", "content": "hi"}]}))
		};
		assert_eq!(
			ask("auto")["generationConfig"]["thinkingConfig"],
			json!({"includeThoughts": true})
		);
		assert!(
			ask("none")["generationConfig"]
				.get("thinkingConfig")
				.is_none()
		);
	}

	#[rstest]
	#[case::schema_missing(json!({"type": "json_schema"}), "has no `schema`")]
	#[case::grammar(json!({"type": "grammar"}), "type `grammar`")]
	#[case::unknown_field(json!({"type": "text", "made_up": 1}), "made_up")]
	fn a_text_format_this_cant_read_is_refused(#[case] format: Value, #[case] problem: &str) {
		let body = json!({"model": "m", "text": {"format": format},
			"input": [{"role": "user", "content": "hi"}]})
		.to_string();
		let refusal =
			test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap_err();
		assert!(refusal.to_string().contains(problem), "{refusal}");
	}

	fn from_claude_code(output_config: &Value, tools: &Value) -> Result<ConvertedRequest, Refusal> {
		let body = json!({
			"model": "m",
			"max_tokens": 9,
			"system": "Be brief.",
			"tools": tools,
			"output_config": output_config,
			"messages": [{"role": "user", "content": "hi"}],
		})
		.to_string();
		test_support::convert(body.as_bytes(), ClientProtocol::Messages, CONFIG)
	}

	#[test]
	fn claude_code_structured_output_becomes_a_json_reply_and_keeps_its_effort() {
		let schema = json!({
			"$defs": {"Name": {"type": "string"}},
			"type": "object",
			"properties": {"a": {"$ref": "#/$defs/Name"}},
		});
		let format = json!({"type": "json_schema", "schema": schema});
		let converted =
			from_claude_code(&json!({"effort": "low", "format": format}), &json!([])).unwrap();
		assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		let effort_only = from_claude_code(&json!({"effort": "low"}), &json!([])).unwrap();
		let effort_only: Value = serde_json::from_str(&effort_only.body).unwrap();
		assert_eq!(
			sent["generationConfig"],
			json!({
				"maxOutputTokens": 9,
				"responseMimeType": "application/json",
				"responseSchema": {"type": "object", "properties": {"a": {"type": "string"}}},
				"thinkingConfig": effort_only["generationConfig"]["thinkingConfig"],
			})
		);
	}

	#[test]
	fn an_enum_gemini_cannot_take_is_dropped_and_recorded() {
		let schema = json!({"type": "object", "properties": {
			"color": {"enum": ["red", "blue"]},
			"size": {"type": "integer", "enum": [1, 2]},
			"level": {"enum": [1, 2]},
		}});
		let tools = json!([{"name": "run", "input_schema": schema}]);
		let converted = from_claude_code(&json!({}), &tools).unwrap();
		assert_eq!(converted.dropped, ["tool schema keyword"]);
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		let properties = &sent["tools"][0]["functionDeclarations"][0]["parameters"]["properties"];
		assert_eq!(
			properties["color"],
			json!({"type": "string", "enum": ["red", "blue"]})
		);
		assert_eq!(properties["size"], json!({"type": "integer"}));
		assert_eq!(properties["level"].get("enum"), None, "{properties}");
	}

	#[test]
	fn with_tools_claude_code_structured_output_is_asked_for_not_enforced() {
		let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
		let tools = json!([{"name": "run", "input_schema": {"type": "object", "properties": {}}}]);
		let converted = from_claude_code(
			&json!({"format": {"type": "json_schema", "schema": schema}}),
			&tools,
		)
		.unwrap();
		assert_eq!(
			converted.dropped,
			["output_config.format enforcement beside tools"]
		);
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		assert_eq!(sent["generationConfig"], json!({"maxOutputTokens": 9}));
		let system = sent["systemInstruction"]["parts"][0]["text"]
			.as_str()
			.unwrap();
		assert!(system.starts_with("Be brief."), "{system}");
		assert!(
			system.contains(r#"{"type":"object","properties":{"a":{"type":"string"}}}"#),
			"{system}"
		);
	}

	#[rstest]
	#[case::empty(json!({}), "`output_config.format`")]
	#[case::schema_not_an_object(
		json!({"type": "json_schema", "schema": []}),
		"`output_config.format.schema` must be a valid object"
	)]
	#[case::named(
		json!({"type": "json_schema", "schema": {"type": "object"}, "name": "n"}),
		"field `output_config.format.name`"
	)]
	fn claude_code_structured_output_this_cant_send_is_refused(
		#[case] format: Value,
		#[case] problem: &str,
	) {
		let refusal = from_claude_code(&json!({"format": format}), &json!([])).unwrap_err();
		assert!(refusal.to_string().contains(problem), "{refusal}");
	}

	#[test]
	fn a_format_a_stage_set_replaces_claude_code_structured_output() {
		let body = json!({
			"model": "m",
			"max_tokens": 9,
			"output_config": {"format": {"type": "json_schema",
				"schema": {"type": "object", "properties": {"client": {"type": "string"}}}}},
			"messages": [{"role": "user", "content": "hi"}],
		})
		.to_string();
		let mut prepared = prompt::prepare(body.as_bytes(), ClientProtocol::Messages, CONFIG).unwrap();
		prepared.conversation_mut().settings.output_format =
			Some(crate::ir::conversation::OutputFormat::Json);
		let converted = prepared.render(RenderSettings::ModelDefaults).unwrap();
		let sent: Value = serde_json::from_str(&converted.body).unwrap();
		assert_eq!(
			sent["generationConfig"],
			json!({"maxOutputTokens": 9, "responseMimeType": "application/json"})
		);
	}

	#[test]
	fn images_arrive_in_messages_and_tool_results() {
		let url = "data:image/png;base64,AAAA";
		let sent = sent(&json!({"model": "m", "input": [
			{"role": "user", "content": [
				{"type": "input_text", "text": "look"},
				{"type": "input_image", "image_url": url},
			]},
			{"type": "function_call", "call_id": "c", "name": "shot", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "c", "output": [
				{"type": "input_text", "text": "done"},
				{"type": "input_image", "image_url": url},
			]},
		]}));
		let image = json!({"inlineData": {"mimeType": "image/png", "data": "AAAA"}});
		assert_eq!(sent["contents"][0]["parts"][1], image);
		assert_eq!(
			sent["contents"][2]["parts"][0]["functionResponse"],
			json!({"name": "shot", "response": {"content": "done"}, "parts": [image]})
		);
	}

	#[test]
	fn a_carrier_for_another_model_is_refused() {
		let other = StateScope {
			model: "other",
			..SCOPE
		};
		let carrier = encode(other, b"{}");
		let body = json!({"model": "m", "input": [
			{"role": "user", "content": "hi"},
			{"type": "reasoning", "encrypted_content": carrier},
			answer("ok"),
		]})
		.to_string();
		assert!(test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).is_err());
	}

	#[test]
	fn both_ways_of_reading_agree() {
		let body = json!({"model": "m", "input": [{"role": "user", "content": "hi"}]}).to_string();
		assert!(prompt::one_pass_parse_agrees(
			body.as_bytes(),
			ClientProtocol::Responses,
			CONFIG
		));
	}
}
