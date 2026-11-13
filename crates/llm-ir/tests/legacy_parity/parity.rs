//! Checks that the LLM IR conversion core treats requests the way the typed
//! legacy request types do, wherever the gateway relies on the two agreeing.

use agent_llm::LogContentFields;
use agent_llm::conversion::responses::from_messages;
use agent_llm::types::{RequestType, messages};
use agent_llm_ir::ir::events::{Event, UsageFold};
use agent_llm_ir::protocols::responses::decoder::decode_buffered;
use agent_llm_ir::{
	ClientProtocol, IdentifierPseudonymizer, RenderSettings, StateScope, Upstream, UpstreamProtocol,
	prepare, read_model,
};
use bytes::Bytes;
use serde_json::Value;

struct NoCarriers;

impl IdentifierPseudonymizer for NoCarriers {
	fn pseudonymize(&self, _identifier: &str) -> String {
		String::new()
	}
}

fn upstream(model: &str) -> Upstream<'_> {
	Upstream {
		identifier_pseudonymizer: &NoCarriers,
		scope: StateScope {
			backend: "backend",
			provider: "openai",
			model,
		},
		protocol: UpstreamProtocol::Responses {
			safety_identifier: true,
			reasoning_context: true,
		},
		default_max_tokens: None,
	}
}

/// What the gateway reads as the model of a Messages request with no provider
/// override, path model or body mutation: see `read_body_resolving_model`.
fn legacy_model(body: &str) -> Result<Option<String>, serde_json::Error> {
	serde_json::from_str::<messages::Request>(body).map(|mut request| request.model().clone())
}

fn ir_model(body: &str) -> Result<Option<String>, serde_json::Error> {
	read_model(body.as_bytes()).map(|model| model.map(String::from))
}

#[test]
fn the_model_pre_read_matches_the_typed_request() {
	for body in [
		r#"{"model":"m","messages":[]}"#,
		" {\n\t\"messages\" : [ ] ,\r\n \"model\"\t:\t\"m\" } \n",
		r#"{"model":"gpt\u002d5","messages":[]}"#,
		r#"{"mod\u0065l":"m","messages":[]}"#,
		r#"{"model":"a\"b\\c\nd","messages":[]}"#,
		r#"{"model":"\ud83d\ude00","messages":[]}"#,
		r#"{"model":"","messages":[]}"#,
		r#"{"messages":[]}"#,
		r#"{"model":null,"messages":[]}"#,
		r#"{"metadata":{"model":"x"},"messages":[]}"#,
		r#"{"model":5,"messages":[]}"#,
		r#"{"model":{"name":"m"},"messages":[]}"#,
		r#"{"model":"a","model":"b","messages":[]}"#,
		r#"{"model":"a","messages":[],"model":null}"#,
		r#"{"model":"a","mod\u0065l":"b","messages":[]}"#,
		r#"{"model":"\ud800","messages":[]}"#,
		r#"{"model":"m","messages":[]} x"#,
		r#"["m"]"#,
		"",
	] {
		match (legacy_model(body), ir_model(body)) {
			(Ok(legacy), Ok(ir)) => assert_eq!(legacy, ir, "{body}"),
			(Err(_), Err(_)) => {},
			(legacy, ir) => panic!("{body}: legacy read {legacy:?}, the IR read {ir:?}"),
		}
	}
}

/// The pre-read checks only the model. Every other problem the typed request
/// refuses is left to the IR's conversion, which must refuse it too.
#[test]
fn bodies_refused_for_other_fields_are_refused_by_the_ir_conversion() {
	for body in [
		r#"{"model":"m","max_tokens":1}"#,
		r#"{"model":"m","max_tokens":1,"messages":5}"#,
		r#"{"model":"m","max_tokens":1,"messages":[{"role":"user","content":"\ud800"}]}"#,
		r#"{"model":"m","max_tokens":1,"messages":[{"role":"user","content":"a"}],"temperature":1e999}"#,
	] {
		assert!(legacy_model(body).is_err(), "{body}");
		assert_eq!(ir_model(body).unwrap().as_deref(), Some("m"), "{body}");
		assert!(
			prepare(body.as_bytes(), ClientProtocol::Messages, upstream("m"))
				.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
				.is_err(),
			"{body}"
		);
	}
}

/// Two differences are accepted. A breakpoint on a part inside a tool result
/// marks the result's last part, where legacy marks that part. A marked result
/// holding only text is one joined part, where legacy sends a part per block.
#[test]
fn cache_breakpoints_land_where_legacy_puts_them() {
	const MARK: &str = r#""cache_control":{"type":"ephemeral"}"#;
	const IMAGE: &str =
		r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}"#;
	for model in ["gpt-5.6", "gpt-5.6-mini", "gpt-6", "gpt-5.4", "gpt-5"] {
		let body = format!(
			r#"{{"model":"{model}","max_tokens":1,
			"system":[{{"type":"text","text":"a"}},{{"type":"text","text":"b",{MARK}}}],
			"messages":[
				{{"role":"user","content":[{{"type":"text","text":"hi",{MARK}}},{IMAGE},{MARK}}},
					{{"type":"text","text":"after"}}]}},
				{{"role":"assistant","content":[{{"type":"text","text":"ok",{MARK}}},
					{{"type":"tool_use","id":"c","name":"f","input":{{}}}},
					{{"type":"tool_use","id":"d","name":"f","input":{{}}}}]}},
				{{"role":"user","content":[
					{{"type":"tool_result","tool_use_id":"c","content":"done",{MARK}}},
					{{"type":"tool_result","tool_use_id":"d",{MARK},
						"content":[{{"type":"text","text":"x"}},{IMAGE}}}]}}]}}]}}"#
		);
		let legacy: messages::Request = serde_json::from_str(&body).unwrap();
		let legacy: Value =
			serde_json::from_slice(&from_messages::translate(&legacy).unwrap()).unwrap();
		let ir = prepare(body.as_bytes(), ClientProtocol::Messages, upstream(model))
			.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
			.unwrap();
		let ir: Value = serde_json::from_str(&ir.body).unwrap();
		assert_eq!(marked_parts(&ir), marked_parts(&legacy), "{model}");
		assert_eq!(
			ir.get("instructions").is_some(),
			legacy.get("instructions").is_some(),
			"{model}"
		);
	}
}

/// Each marked part, by its input item and its place in that item.
fn marked_parts(body: &Value) -> Vec<(usize, usize)> {
	let mut marked = Vec::new();
	for (item_at, item) in body["input"].as_array().into_iter().flatten().enumerate() {
		let parts = item.get("content").or_else(|| item.get("output"));
		for (part_at, part) in parts
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
			.enumerate()
		{
			if let Some(mark) = part.get("prompt_cache_breakpoint") {
				assert_eq!(mark, &serde_json::json!({"mode": "explicit"}));
				marked.push((item_at, part_at));
			}
		}
	}
	marked
}

/// Legacy sends `reasoning` only to models that take an effort, which leaves out
/// GPT-3.5, GPT-4 and ChatGPT-4o. One difference is accepted: where a model takes
/// an effort, the IR sends disabled thinking as effort `none` and legacy sends no
/// `reasoning`, leaving the model's default effort.
#[test]
fn reasoning_reaches_only_models_that_take_an_effort() {
	for (model, supports_effort) in [
		("gpt-3.5-turbo", false),
		("gpt-4", false),
		("gpt-4o-mini", false),
		("gpt-4.1", false),
		("chatgpt-4o-latest", false),
		("gpt-5.4", true),
		("o3", true),
	] {
		for settings in [
			r#""output_config":{"effort":"low"}"#,
			r#""thinking":{"type":"adaptive"},"output_config":{"effort":"high"}"#,
			r#""thinking":{"type":"disabled"}"#,
		] {
			let body = format!(
				r#"{{"model":"{model}","max_tokens":1,{settings},
				"messages":[{{"role":"user","content":"hi"}}]}}"#
			);
			let legacy: messages::Request = serde_json::from_str(&body).unwrap();
			let legacy: Value =
				serde_json::from_slice(&from_messages::translate(&legacy).unwrap()).unwrap();
			let ir = prepare(body.as_bytes(), ClientProtocol::Messages, upstream(model))
				.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
				.unwrap();
			let ir: Value = serde_json::from_str(&ir.body).unwrap();
			let thinking_off = settings.contains("disabled");
			let want = legacy.get("reasoning").cloned().or_else(|| {
				(thinking_off && supports_effort).then(|| serde_json::json!({"effort": "none"}))
			});
			assert_eq!(ir.get("reasoning"), want.as_ref(), "{model} {settings}");
		}
	}
}

#[test]
fn twelve_hour_cache_writes_are_counted_as_legacy_counts_them() {
	let body = r#"{"id":"r","object":"response","created_at":1,"model":"m","status":"completed",
		"output":[],"usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11,
		"input_tokens_details":{"cached_tokens":2,"cache_write_tokens":8,
			"cache_write_tokens_details":{"ttl_12h_tokens":5}},
		"output_tokens_details":{"reasoning_tokens":0}}}"#;
	let legacy = from_messages::translate_response(&Bytes::from(body))
		.unwrap()
		.to_llm_response(LogContentFields::default());
	let mut usage = UsageFold::default();
	decode_buffered(body.as_bytes(), &mut |event: Event<'_>| usage.event(&event)).unwrap();
	let ir = usage.total();
	assert_eq!(
		ir.cache_write_12h_tokens,
		legacy.cache_creation_12h_input_tokens
	);
	assert_eq!(ir.cache_write_tokens, legacy.cache_creation_input_tokens);
	assert_eq!(ir.cached_tokens, legacy.cached_input_tokens);
}
