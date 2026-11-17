#![allow(clippy::unwrap_used)]

use std::num::NonZeroU64;

use rstest::rstest;
use serde_json::json;

use crate::carrier::encode;
use crate::prompt::{self, Upstream};
use crate::protocols::{ClientProtocol, UpstreamProtocol};
use crate::test_support::{self, SCOPE, Sum};

const CONFIG: Upstream<'static> = Upstream {
	identifier_pseudonymizer: &Sum,
	scope: SCOPE,
	default_max_tokens: NonZeroU64::new(9),
	protocol: UpstreamProtocol::Messages,
};

#[rstest]
#[case::positive("7", true)]
#[case::zero("0", true)]
#[case::negative("-1", false)]
#[case::fraction("1.5", false)]
#[case::string(r#""7""#, false)]
fn an_edited_top_k_is_rendered_and_checked(#[case] value: &str, #[case] valid: bool) {
	let body = br#"{"model":"m","input":"hi"}"#;
	let raw = serde_json::value::RawValue::from_string(value.to_string()).unwrap();
	let mut request = prompt::prepare(body, ClientProtocol::Responses, CONFIG).unwrap();
	request.conversation_mut().settings.top_k = Some(raw.as_ref().into());
	let result = request.render(prompt::RenderSettings::ModelDefaults);
	if valid {
		let converted = result.unwrap();
		let rendered: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
		assert_eq!(rendered["top_k"].to_string(), value);
	} else {
		assert!(
			result
				.unwrap_err()
				.problems
				.iter()
				.any(|problem| problem.contains("top_k"))
		);
	}
}

#[rstest]
#[case::no_effort(json!({"reasoning": {"effort": "none"}}), "`reasoning.effort`")]
#[case::minimal_effort(json!({"reasoning": {"effort": "minimal"}}), "`reasoning.effort`")]
#[case::json_object(json!({"text": {"format": {"type": "json_object"}}}), "`text.format`")]
fn what_messages_cannot_take_is_refused(#[case] extra: serde_json::Value, #[case] problem: &str) {
	let mut body = json!({"model": "m", "input": [{"role": "user", "content": "hi"}]});
	body
		.as_object_mut()
		.unwrap()
		.extend(extra.as_object().unwrap().clone());
	let refusal = test_support::convert(
		body.to_string().as_bytes(),
		ClientProtocol::Responses,
		CONFIG,
	)
	.unwrap_err();
	assert!(refusal.to_string().contains(problem), "{refusal}");
}

#[test]
fn codex_history_converts() {
	let meta = json!({"turn_id": "t"});
	let body = json!({
		"model": "m",
		"access_programs": {"cyber": "standard"},
		"reasoning": {"effort": "low"},
		"input": [
			{"type": "message", "role": "user", "internal_chat_message_metadata_passthrough": meta,
				"content": [{"type": "input_text", "text": "hi"}]},
			{"type": "configuration_update", "reasoning": {"effort": "medium"}},
			{"type": "configuration_update", "reasoning": {"effort": "high"}},
			{"type": "message", "role": "assistant", "internal_chat_message_metadata_passthrough": meta,
				"content": [{"type": "output_text", "text": "ok", "logprobs": []}]},
			{"type": "custom_tool_call", "call_id": "c", "name": "exec", "input": "ls",
				"internal_chat_message_metadata_passthrough": meta},
			{"type": "custom_tool_call_output", "call_id": "c", "output": "a",
				"internal_chat_message_metadata_passthrough": meta},
		],
	})
	.to_string();
	let converted =
		test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	insta::assert_json_snapshot!(json!({"dropped": converted.dropped, "sent": sent}));
}

#[test]
fn a_configuration_update_can_only_set_effort() {
	let body = json!({
		"model": "m",
		"input": [
			{"type": "configuration_update", "model": "x", "reasoning": {"effort": "max", "summary": "y"}},
			{"role": "user", "content": "hi"},
		],
	})
	.to_string();
	let mut problems = test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG)
		.unwrap_err()
		.problems;
	problems.sort();
	assert_eq!(
		problems,
		[
			"`input[0].reasoning.effort` `max`",
			"field `input[0].model`",
			"field `input[0].reasoning.summary`"
		]
	);
}

#[test]
fn later_developer_messages_keep_turns_alternating() {
	let body = json!({
		"model": "m",
		"input": [
			{"role": "user", "content": "first"},
			{"role": "assistant", "content": "reply"},
			{"role": "developer", "content": "between"},
			{"role": "assistant", "content": "more"},
			{"role": "developer", "content": "last"},
		],
	})
	.to_string();
	let converted =
		test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
	assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	let turns: Vec<_> = sent["messages"]
		.as_array()
		.unwrap()
		.iter()
		.map(|message| {
			let texts: Vec<_> = message["content"]
				.as_array()
				.unwrap()
				.iter()
				.map(|block| block["text"].as_str().unwrap())
				.collect();
			(message["role"].as_str().unwrap(), texts)
		})
		.collect();
	assert_eq!(
		turns,
		[
			("user", vec!["first"]),
			("assistant", vec!["reply"]),
			(
				"user",
				vec!["<system-reminder>\nbetween\n</system-reminder>"]
			),
			("assistant", vec!["more"]),
			("user", vec!["<system-reminder>\nlast\n</system-reminder>"]),
		]
	);
}

#[test]
fn a_developer_message_after_compaction_is_a_reminder_in_the_user_turn() {
	let body = json!({
		"model": "m",
		"input": [
			{"role": "user", "content": "Earlier conversation."},
			{"role": "developer", "content": "Instructions restored after compaction."},
			{"role": "user", "content": "Continue."},
		],
	})
	.to_string();
	let converted =
		test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
	assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(
		sent["messages"],
		json!([{"role": "user", "content": [
			{"type": "text", "text": "Earlier conversation."},
			{"type": "text", "text": "<system-reminder>\nInstructions restored after compaction.\n</system-reminder>"},
			{"type": "text", "text": "Continue.", "cache_control": {"type": "ephemeral"}},
		]}])
	);
}

#[test]
fn a_reminder_follows_the_tool_results_it_waited_for() {
	let body = json!({
		"model": "m",
		"input": [
			{"role": "user", "content": "look"},
			{"type": "function_call", "call_id": "c1", "name": "look", "arguments": "{}"},
			{"role": "developer", "content": "note"},
			{"type": "function_call_output", "call_id": "c1", "output": "seen"},
			{"role": "user", "content": "next"},
		],
		"tools": [{"type": "function", "name": "look", "parameters": {"type": "object"}}],
	})
	.to_string();
	let converted =
		test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(
		sent["messages"][2],
		json!({"role": "user", "content": [
			{"type": "tool_result", "tool_use_id": "c1", "content": [{"type": "text", "text": "seen"}]},
			{"type": "text", "text": "<system-reminder>\nnote\n</system-reminder>"},
			{"type": "text", "text": "next", "cache_control": {"type": "ephemeral"}},
		]})
	);
}

#[test]
fn keeping_reasoning_asks_for_its_beta() {
	let body = |context: &str| {
		json!({"model": "m", "input": "hi", "reasoning": {"context": context}}).to_string()
	};
	let converted = test_support::convert(
		body("all_turns").as_bytes(),
		ClientProtocol::Responses,
		CONFIG,
	)
	.unwrap();
	assert_eq!(
		converted.anthropic_beta,
		Some("context-management-2025-06-27")
	);
	let plain = test_support::convert(
		br#"{"model":"m","input":"hi"}"#,
		ClientProtocol::Responses,
		CONFIG,
	)
	.unwrap();
	assert_eq!(plain.anthropic_beta, None);
}

#[test]
fn a_full_request_keeps_its_exact_bytes() {
	let thinking = r#"{"type": "thinking", "thinking": "caf\u00e9 \"q\"", "signature": "s1"}"#;
	let carrier = encode(
		SCOPE,
		crate::carrier::CarrierPurpose::ResponsesEncryptedContent,
		thinking.as_bytes(),
	);
	let image = "data:image/png;base64,AAAA";
	let body = format!(
		r#"{{"model":"m","instructions":"be brief","max_output_tokens":64,"temperature":0.50,"top_p":1e0,
		"tools":[{{"type":"function","name":"look","description":"Look.","parameters":{{"type": "object", "properties": {{}}}},"strict":true}}],
		"tool_choice":{{"type":"function","name":"look"}},"parallel_tool_calls":false,
		"reasoning":{{"effort":"high","summary":"auto","context":"all_turns"}},
		"text":{{"format":{{"type":"json_schema","name":"out","schema":{{"type": "object"}},"strict":true}}}},
		"stream":true,
		"input":[
			{{"role":"user","content":[{{"type":"input_text","text":"look ☃"}},{{"type":"input_image","image_url":"{image}"}}]}},
			{{"role":"developer","content":"note"}},
			{{"type":"reasoning","summary":[],"encrypted_content":"{carrier}"}},
			{{"type":"function_call","call_id":"c1","name":"look","arguments":"{{\"at\": 1}}"}},
			{{"type":"function_call_output","call_id":"c1","output":[{{"type":"input_text","text":"seen"}},{{"type":"input_image","image_url":"{image}"}}]}},
			{{"role":"assistant","content":"done"}},
			{{"role":"user","content":"thanks"}}
		]}}"#
	);
	let converted =
		test_support::convert(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
	let expected = concat!(
		r#"{"model":"m","max_tokens":64,"system":[{"type":"text","text":"be brief","cache_control":{"type":"ephemeral"}}],"messages":["#,
		r#"{"role":"user","content":[{"type":"text","text":"look ☃"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}},{"type":"text","text":"<system-reminder>\nnote\n</system-reminder>"}]},"#,
		r#"{"role":"assistant","content":[{"type": "thinking", "thinking": "caf\u00e9 \"q\"", "signature": "s1"},{"type":"tool_use","id":"c1","name":"look","input":{"at": 1}}]},"#,
		r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":[{"type":"text","text":"seen"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}]},"#,
		r#"{"role":"assistant","content":[{"type":"text","text":"done"}]},"#,
		r#"{"role":"user","content":[{"type":"text","text":"thanks","cache_control":{"type":"ephemeral"}}]}]"#,
		r#","tools":[{"name":"look","description":"Look.","input_schema":{"type": "object", "properties": {}},"strict":true}]"#,
		r#","tool_choice":{"type":"tool","name":"look","disable_parallel_tool_use":true}"#,
		r#","temperature":0.50,"top_p":1e0"#,
		r#","thinking":{"type":"adaptive","display":"summarized"}"#,
		r#","output_config":{"effort":"high","format":{"type":"json_schema","schema":{"type": "object"}}}"#,
		r#","context_management":{"edits":[{"type":"clear_thinking_20251015","keep":"all"}]}"#,
		r#","stream":true}"#,
	);
	assert_eq!(converted.body, expected);
	assert_eq!(
		converted.anthropic_beta,
		Some("context-management-2025-06-27")
	);
}

#[test]
fn a_failed_tool_result_says_so() {
	let body = json!({"model": "m", "input": [
		{"type": "function_call", "call_id": "c", "name": "f", "arguments": "{}"},
		{"type": "function_call_output", "call_id": "c", "output": "no"},
	]})
	.to_string();
	let mut prepared = prompt::prepare(body.as_bytes(), ClientProtocol::Responses, CONFIG).unwrap();
	let conversation = prepared.conversation_mut();
	for block in conversation.messages.iter_mut().flat_map(|m| &mut m.blocks) {
		if let crate::ir::conversation::Content::ToolResult { failed, .. } = &mut block.content {
			*failed = Some(true);
		}
	}
	let converted = prepared
		.render(prompt::RenderSettings::ModelDefaults)
		.unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(sent["messages"][1]["content"][0]["is_error"], true);
}

#[rstest]
#[case::fields_in_any_order(r#"{"input":[{"content":"hi","role":"user"}],"model":"m"}"#)]
#[case::content_on_a_call(r#"{"model":"m","input":[{"type":"function_call","call_id":"c","name":"f","arguments":"{}","content":[]}]}"#)]
#[case::output_on_a_message(
	r#"{"model":"m","input":[{"role":"user","content":"a","output":"b"}]}"#
)]
#[case::content_on_an_output(
	r#"{"model":"m","input":[{"type":"function_call_output","call_id":"c","content":"a"}]}"#
)]
#[case::reasoning_ignores_content(r#"{"model":"m","input":[{"type":"reasoning","content":5}]}"#)]
#[case::repeated_content(r#"{"model":"m","input":[{"role":"user","content":"a","content":"b"}]}"#)]
#[case::repeated_output(r#"{"model":"m","input":[{"type":"function_call_output","call_id":"c","output":"a","output":"b"}]}"#)]
#[case::repeated_input(r#"{"model":"m","input":"a","input":"b"}"#)]
#[case::not_an_item(r#"{"model":"m","input":[5,{"role":"user","content":"a"}]}"#)]
#[case::repeated_block_field(
	r#"{"model":"m","input":[{"role":"user","content":[{"type":"text","text":"a","text":"b"}]}]}"#
)]
#[case::block_of_the_wrong_shape(r#"{"model":"m","input":[{"role":"user","content":[5]}]}"#)]
#[case::lone_surrogate(r#"{"model":"m","input":[{"role":"user","content":"\ud800"}]}"#)]
#[case::additional_tools(r#"{"model":"m","input":[{"type":"additional_tools","tools":[],"content":[]},{"role":"user","content":"a"}]}"#)]
#[case::repeated_tools_item(
	r#"{"model":"m","input":[{"type":"additional_tools","tools":[],"tools":[]}]}"#
)]
#[case::output_of_the_wrong_shape(
	r#"{"model":"m","input":[{"type":"function_call_output","call_id":"c","output":5}]}"#
)]
fn both_ways_of_reading_agree(#[case] body: &str) {
	assert!(
		prompt::one_pass_parse_agrees(body.as_bytes(), ClientProtocol::Responses, CONFIG),
		"{body}"
	);
}

/// A caching policy marks what it asks for in place of the renderer's own marks: the system
/// prompt if it is long enough, the turn before the last, and the last tool.
#[rstest]
#[case::everything(caching(true, true, true, None, 0), (true, Some(1), true))]
#[case::a_short_system_prompt(caching(true, false, false, Some(10), 0), (false, None, false))]
#[case::an_earlier_turn(caching(false, true, false, None, 1), (false, Some(0), false))]
#[case::nothing(caching(false, false, false, None, 0), (false, None, false))]
fn a_caching_policy_chooses_the_marks(
	#[case] caching: crate::PromptCaching,
	#[case] marked: (bool, Option<usize>, bool),
) {
	let (system_marked, message_marked, tool_marked) = marked;
	let body = json!({"model": "m", "max_output_tokens": 9, "instructions": "be brief",
		"tools": [{"type": "function", "name": "look", "parameters": {"type": "object"}}],
		"input": [
			{"role": "user", "content": "a"},
			{"role": "assistant", "content": "b"},
			{"role": "user", "content": "c"}]})
	.to_string();
	let converted = prompt::prepare(body.as_bytes(), ClientProtocol::Responses, CONFIG)
		.unwrap()
		.render(prompt::RenderSettings::Messages(caching))
		.unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	let mark = json!({"type": "ephemeral"});
	assert_eq!(sent["system"][0]["cache_control"] == mark, system_marked);
	for (at, message) in sent["messages"].as_array().unwrap().iter().enumerate() {
		let marked = message["content"][0]["cache_control"] == mark;
		assert_eq!(marked, message_marked == Some(at), "messages[{at}]");
	}
	assert_eq!(sent["tools"][0]["cache_control"] == mark, tool_marked);
}

fn caching(
	system: bool,
	messages: bool,
	tools: bool,
	min_tokens: Option<usize>,
	message_offset: usize,
) -> crate::PromptCaching {
	crate::PromptCaching {
		system,
		messages,
		tools,
		min_tokens,
		message_offset,
	}
}
