#![allow(clippy::panic, clippy::unwrap_used)]

use crate::carrier::StateScope;
use crate::ir::conversation::{Content, ToolOutput, ToolPart};
use crate::prompt::{self, PreparedRequest, TOOL_ERROR_MARKER, Upstream};
use crate::protocols::{ClientProtocol, UpstreamProtocol};
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
const CALL: &str =
	r#"{"role":"assistant","content":[{"type":"tool_use","id":"c","name":"f","input":{}}]}"#;
const BAD_IMAGE: &str = r#"{"type":"image","source":{"type":"url"}}"#;
const ADAPTIVE: &str = r#""thinking":{"type":"adaptive"}"#;

fn body(extra: &str, messages: &str) -> String {
	format!(r#"{{"model":"m","max_tokens":1,{extra}"messages":{messages}}}"#)
}

fn result(fields: &str) -> String {
	format!(r#"{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"c",{fields}}}]}}"#)
}

fn problems(body: &str) -> Vec<String> {
	let mut problems = test_support::convert(body.as_bytes(), ClientProtocol::Messages, CONFIG)
		.unwrap_err()
		.problems;
	problems.sort();
	problems
}

#[test]
fn what_a_refused_block_or_message_contained_is_still_checked() {
	let no_id =
		format!(r#"[{{"role":"user","content":[{{"type":"tool_result","content":[{BAD_IMAGE}]}}]}}]"#);
	assert_eq!(
		problems(&body("", &no_id)),
		[
			"`messages[0].content[0].content[0].source` must be base64",
			"`messages[0].content[0]` has no `tool_use_id`",
		]
	);
	assert_eq!(
		problems(&body("", r#"[{"output_config":{"effort":"bad"}}]"#)),
		[
			"`messages[0].output_config.effort` `bad`",
			"`messages[0]` needs `role` and `content`",
		]
	);
	assert_eq!(
		problems(&body(
			"",
			r#"[{"role":"robot","content":"a","output_config":{"nope":1}}]"#
		)),
		[
			"`messages[0]` role `robot`",
			"field `messages[0].output_config.nope`",
		]
	);
	let bad_output = format!(
		"[{CALL},{}]",
		result(&format!(r#""is_error":true,"content":[{BAD_IMAGE}]"#))
	);
	assert_eq!(
		problems(&body("", &bad_output)),
		["`messages[1].content[0].content[0].source` must be base64"]
	);
}

#[test]
fn an_effort_counts_even_on_a_message_with_a_bad_role() {
	let bad_role = r#"[{"role":"robot","content":"a","output_config":{"effort":"low"}}]"#;
	assert_eq!(
		problems(&body(&format!("{ADAPTIVE},"), bad_role)),
		["`messages[0]` role `robot`"]
	);
	let no_role = r#"[{"content":"a","output_config":{"effort":"low"}}]"#;
	assert_eq!(
		problems(&body(&format!("{ADAPTIVE},"), no_role)),
		[
			"`messages[0]` needs `role` and `content`",
			"adaptive `thinking` without an effort",
		]
	);
}

#[test]
fn a_refused_text_block_still_starts_a_turn() {
	let messages = r#"[{"role":"assistant","content":"a","output_config":{"effort":"low"}},{"role":"user","content":[{"type":"text"}]}]"#;
	assert_eq!(
		problems(&body(&format!("{ADAPTIVE},"), messages)),
		[
			"`messages[1].content[0]` has no text",
			"adaptive `thinking` without an effort",
		]
	);
}

#[test]
fn top_k_is_dropped_and_recorded() {
	let converted = test_support::convert(
		body(r#""top_k":40,"#, r#"[{"role":"user","content":"hi"}]"#).as_bytes(),
		ClientProtocol::Messages,
		CONFIG,
	)
	.unwrap();
	let rendered: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert!(rendered.get("top_k").is_none());
	assert_eq!(converted.dropped, ["top_k"]);
}

#[test]
fn tool_strictness_is_preserved_and_defaults_to_false() {
	let extra = r#""tools":[{"name":"default","input_schema":{}},{"name":"strict","strict":true,"input_schema":{}},{"name":"loose","strict":false,"input_schema":{}}],"#;
	let converted = test_support::convert(
		body(extra, r#"[{"role":"user","content":"hi"}]"#).as_bytes(),
		ClientProtocol::Messages,
		CONFIG,
	)
	.unwrap();
	let rendered: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	for (tool, expected) in rendered["tools"]
		.as_array()
		.unwrap()
		.iter()
		.zip([false, true, false])
	{
		assert_eq!(tool["strict"], expected);
		assert_eq!(tool["parameters"], serde_json::json!({}));
	}
	assert!(converted.dropped.is_empty());
}

#[test]
fn tool_strictness_requires_a_boolean() {
	assert_eq!(
		problems(&body(
			r#""tools":[{"name":"f","strict":"true","input_schema":{}}],"#,
			r#"[{"role":"user","content":"hi"}]"#,
		)),
		["`tools[0].strict` has the wrong type"]
	);
}

#[test]
fn settings_are_checked_together() {
	assert_eq!(
		problems(&body(
			r#""temperature":"hot","thinking":{"type":"enabled","display":"summarized","budget_tokens":1},"#,
			"[]"
		)),
		[
			"`messages` is empty",
			"`temperature` has the wrong type",
			"`thinking.display` `summarized`",
			"`thinking.type` `enabled`",
			"field `thinking.budget_tokens`",
		]
	);
}

#[test]
fn wrong_shapes_use_this_targets_wording() {
	let messages = r#"[{"role":"system","content":5},{"role":"user","content":5}]"#;
	assert_eq!(
		problems(&body(r#""system":5,"#, messages)),
		[
			"`messages[0].content` must be text",
			"`messages[1].content` must be a string or a list",
			"`system` must be text",
		]
	);
}

#[test]
fn what_the_client_said_is_kept_apart_from_what_it_left_out() {
	let hi = r#"[{"role":"user","content":"hi"}]"#;
	let said = test_support::convert(
		body(r#""system":[],"stream":false,"#, hi).as_bytes(),
		ClientProtocol::Messages,
		CONFIG,
	)
	.unwrap();
	assert!(said.body.contains(r#""instructions":"""#));
	assert!(said.body.contains(r#""stream":false"#));
	let silent =
		test_support::convert(body("", hi).as_bytes(), ClientProtocol::Messages, CONFIG).unwrap();
	assert!(!silent.body.contains("instructions"));
	assert!(!silent.body.contains("stream"));

	let not_failed = format!("[{CALL},{}]", result(r#""is_error":false,"content":"a""#));
	let converted = test_support::convert(
		body("", &not_failed).as_bytes(),
		ClientProtocol::Messages,
		CONFIG,
	)
	.unwrap();
	assert_eq!(converted.dropped, ["tool_result.is_error"]);
}

#[test]
fn a_failed_result_with_an_image_keeps_its_parts_in_order() {
	let image =
		r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}}"#;
	let content = format!(r#""is_error":true,"content":[{{"type":"text","text":"a"}},{image}]"#);
	let converted = test_support::convert(
		body("", &format!("[{CALL},{}]", result(&content))).as_bytes(),
		ClientProtocol::Messages,
		CONFIG,
	)
	.unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	insta::assert_json_snapshot!(sent["input"][1], @r#"
	{
	  "type": "function_call_output",
	  "call_id": "c",
	  "output": [
	    {
	      "type": "input_text",
	      "text": "[tool error]\n"
	    },
	    {
	      "type": "input_text",
	      "text": "a"
	    },
	    {
	      "type": "input_image",
	      "image_url": "data:image/png;base64,iVBORw0KGgo="
	    }
	  ]
	}
	"#);
}

#[test]
fn a_failed_result_with_its_only_image_removed_is_valid_json() {
	let image =
		r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}"#;
	let body = format!(
		r#"{{"model":"m","max_tokens":1,"messages":[{CALL},{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"c","is_error":true,"content":[{image}]}}]}}]}}"#
	);
	let mut prepared = prepared(&body);
	for block in prepared
		.conversation_mut()
		.messages
		.iter_mut()
		.flat_map(|m| &mut m.blocks)
	{
		if let Content::ToolResult {
			output: ToolOutput::Mixed(parts),
			..
		} = &mut block.content
		{
			parts.retain(|part| matches!(part, ToolPart::Text(_)));
		}
	}
	let converted = prepared
		.render(prompt::RenderSettings::ModelDefaults)
		.unwrap();
	let sent: serde_json::Value = serde_json::from_str(&converted.body)
		.unwrap_or_else(|error| panic!("{error}: {}", converted.body));
	assert_eq!(sent["input"][1]["output"][0]["text"], TOOL_ERROR_MARKER);
}

fn prepared(body: &str) -> PreparedRequest<'_, 'static> {
	prompt::prepare(body.as_bytes(), ClientProtocol::Messages, CONFIG)
		.unwrap_or_else(|refusal| panic!("{refusal}"))
}

#[test]
fn an_edit_between_prepare_and_render_is_sent() {
	let body = body("", r#"[{"role":"user","content":"secret"}]"#);
	let mut prepared = prepared(&body);
	let Content::Text(text) = &mut prepared.conversation_mut().messages[0].blocks[0].content else {
		panic!("the message should contain one text block");
	};
	*text = "masked".into();
	let converted = prepared
		.render(prompt::RenderSettings::ModelDefaults)
		.unwrap();
	assert!(converted.body.contains("masked"), "{}", converted.body);
	assert!(!converted.body.contains("secret"), "{}", converted.body);
}

#[test]
fn prepare_refuses_with_every_problem_rendering_would_add() {
	let no_id =
		format!(r#"[{{"role":"user","content":[{{"type":"tool_result","content":[{BAD_IMAGE}]}}]}}]"#);
	let body = body("", &no_id);
	let mut problems = prompt::prepare(body.as_bytes(), ClientProtocol::Messages, CONFIG)
		.err()
		.unwrap()
		.problems;
	problems.sort();
	assert_eq!(
		problems,
		[
			"`messages[0].content[0].content[0].source` must be base64",
			"`messages[0].content[0]` has no `tool_use_id`",
		]
	);
}

#[test]
fn a_prepared_request_can_be_held_across_an_await() {
	fn assert_send<T: Send>(_: T) {}
	let body = body("", r#"[{"role":"user","content":"hi"}]"#);
	let config = CONFIG;
	assert_send(async {
		let prepared = prompt::prepare(body.as_bytes(), ClientProtocol::Messages, config)
			.unwrap_or_else(|refusal| panic!("{refusal}"));
		std::future::ready(()).await;
		prepared.render(prompt::RenderSettings::ModelDefaults)
	});
}

/// Each kind of block a Messages client can mark for the prompt cache.
const MARKED: &str = r#"{"model":"m","max_tokens":1,
	"system":[{"type":"text","text":"a"},
		{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}],
	"messages":[
		{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]},
		{"role":"assistant","content":[
			{"type":"text","text":"ok","cache_control":{"type":"ephemeral"}},
			{"type":"tool_use","id":"c","name":"f","input":{}},
			{"type":"tool_use","id":"d","name":"f","input":{}}]},
		{"role":"user","content":[
			{"type":"tool_result","tool_use_id":"c","content":"done",
				"cache_control":{"type":"ephemeral"}},
			{"type":"tool_result","tool_use_id":"d","content":[
				{"type":"text","text":"x","cache_control":{"type":"ephemeral"}},
				{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}]}]}"#;

fn for_model(model: &str) -> prompt::ConvertedRequest {
	let upstream = Upstream {
		scope: StateScope { model, ..SCOPE },
		..CONFIG
	};
	prompt::prepare(MARKED.as_bytes(), ClientProtocol::Messages, upstream)
		.unwrap()
		.render(prompt::RenderSettings::ModelDefaults)
		.unwrap()
}

#[test]
fn cache_breakpoints_reach_a_model_that_takes_them() {
	let converted = for_model("gpt-5.6");
	assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	// The system prompt goes in `input`, not `instructions`, so it can be marked.
	insta::assert_json_snapshot!(sent);
}

#[test]
fn other_models_get_no_cache_breakpoints() {
	let converted = for_model("gpt-5.4");
	assert_eq!(converted.dropped, ["cache_control"]);
	assert!(!converted.body.contains("prompt_cache_breakpoint"));
	let sent: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(sent["instructions"], "a\n\nb");
}

#[test]
fn models_before_reasoning_efforts_get_no_reasoning() {
	const EFFORT: &str = r#"{"model":"m","max_tokens":1,"output_config":{"effort":"low"},
		"messages":[{"role":"user","content":"hi"}]}"#;
	let convert = |model| {
		let upstream = Upstream {
			scope: StateScope { model, ..SCOPE },
			..CONFIG
		};
		test_support::convert(EFFORT.as_bytes(), ClientProtocol::Messages, upstream).unwrap()
	};
	let older = convert("gpt-4o");
	assert_eq!(older.dropped, ["reasoning effort"]);
	assert!(!older.body.contains("\"reasoning\""), "{}", older.body);
	let newer = convert("gpt-5.4");
	assert!(newer.dropped.is_empty(), "{:?}", newer.dropped);
	assert!(
		newer.body.contains(r#""reasoning":{"effort":"low"}"#),
		"{}",
		newer.body
	);
}
