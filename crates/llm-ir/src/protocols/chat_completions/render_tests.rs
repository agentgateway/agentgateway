#![allow(clippy::panic, clippy::unwrap_used)]

use serde_json::{Value, json};

use crate::carrier::StateScope;
use crate::ir::conversation::Effort;
use crate::prompt::{self, ConvertedRequest, RequestRefusal as Refusal, Upstream};
use crate::protocols::{ClientProtocol, UpstreamProtocol};
use crate::test_support::{self, Sum};

fn effort(model: &str, tools: &Value) -> (Value, Vec<&'static str>) {
	let body = json!({"model": "c", "max_tokens": 9, "tools": tools, "output_config": {"effort": "low"},
		"messages": [{"role": "user", "content": "hi"}]});
	let config = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: StateScope {
			backend: "a",
			provider: "p",
			model,
		},
		protocol: UpstreamProtocol::ChatCompletions,
		default_max_tokens: None,
	};
	let converted = test_support::convert(
		body.to_string().as_bytes(),
		ClientProtocol::Messages,
		config,
	)
	.unwrap();
	let sent: Value = serde_json::from_str(&converted.body).unwrap();
	(sent["reasoning_effort"].clone(), converted.dropped)
}

#[test]
fn effort_follows_what_the_model_takes() {
	let tool = json!([{"name": "t", "input_schema": {"type": "object"}}]);
	assert_eq!(effort("gpt-5.4-mini", &json!([])).0, "low");
	let (word, dropped) = effort("gpt-5.4-mini", &tool);
	assert_eq!(word, "none");
	assert!(dropped.contains(&"effort beside tools"), "{dropped:?}");
	assert_eq!(effort("gpt-5.2", &tool).0, "low");
	assert_eq!(effort("gpt-5-mini", &tool).0, "low");
	assert_eq!(effort("deepseek-chat", &tool).0, "low");
	let (word, dropped) = effort("gpt-4o", &json!([]));
	assert_eq!(word, Value::Null);
	assert!(dropped.contains(&"effort"), "{dropped:?}");
}

#[test]
fn stop_sequences_follow_what_the_model_takes() {
	let body = json!({
		"model": "c",
		"max_tokens": 9,
		"stop_sequences": ["END"],
		"messages": [{"role": "user", "content": "hi"}],
	})
	.to_string();
	let convert = |model| {
		test_support::convert(
			body.as_bytes(),
			ClientProtocol::Messages,
			Upstream {
				identifier_pseudonymizer: &Sum,
				scope: StateScope {
					backend: "a",
					provider: "p",
					model,
				},
				protocol: UpstreamProtocol::ChatCompletions,
				default_max_tokens: None,
			},
		)
	};

	let emulated = convert("gpt-5.4-mini").unwrap();
	let sent: Value = serde_json::from_str(&emulated.body).unwrap();
	assert_eq!(sent.get("stop"), None);
	assert_eq!(emulated.reply.emulated_stop_sequences, ["END"]);
	let converted = convert("deepseek-chat").unwrap();
	let sent: Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(sent["stop"], json!(["END"]));
	assert!(converted.reply.emulated_stop_sequences.is_empty());
}

#[test]
fn tools_do_not_force_an_effort_the_catalog_excludes() {
	let body = json!({"model": "c", "max_tokens": 9,
		"tools": [{"name": "t", "input_schema": {"type": "object"}}],
		"output_config": {"effort": "high"},
		"messages": [{"role": "user", "content": "hi"}]})
	.to_string();
	let upstream = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: StateScope {
			backend: "a",
			provider: "p",
			model: "gpt-5.4-pro",
		},
		protocol: UpstreamProtocol::ChatCompletions,
		default_max_tokens: None,
	};
	let controls = crate::ReasoningControls::new(Some(true))
		.with_effort(Effort::Medium)
		.with_effort(Effort::High)
		.with_effort(Effort::XHigh);
	let converted = prompt::prepare(body.as_bytes(), ClientProtocol::Messages, upstream)
		.unwrap()
		.render(prompt::RenderSettings::ChatCompletions(controls))
		.unwrap();
	let sent: Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(sent.get("reasoning_effort"), None, "{sent}");
	assert_eq!(converted.dropped, ["effort"]);
}

fn upstream(model: &'static str) -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &Sum,
		scope: StateScope {
			backend: "a",
			provider: "p",
			model,
		},
		protocol: UpstreamProtocol::ChatCompletions,
		default_max_tokens: None,
	}
}

fn convert(client: ClientProtocol, body: &Value) -> Result<(Value, ConvertedRequest), Refusal> {
	let converted = test_support::convert(body.to_string().as_bytes(), client, upstream("c"))?;
	Ok((serde_json::from_str(&converted.body).unwrap(), converted))
}

fn sent(client: ClientProtocol, body: &Value) -> Value {
	convert(client, body)
		.unwrap_or_else(|refusal| panic!("{refusal}"))
		.0
}

fn problems(client: ClientProtocol, body: &Value) -> Vec<String> {
	match convert(client, body) {
		Ok((sent, _)) => panic!("converted: {sent}"),
		Err(refusal) => refusal.problems,
	}
}

fn responses(input: &Value, tools: &Value) -> Value {
	json!({"model": "c", "max_output_tokens": 9, "input": input, "tools": tools})
}

fn hello() -> Value {
	json!([{"role": "user", "content": "hi"}])
}

#[test]
fn namespaced_tools_are_flattened_and_routed() {
	let tools = json!([
		{"type": "namespace", "name": "mcp", "tools": [
			{"type": "function", "name": "read", "parameters": {"type": "object"}},
		]},
		{"type": "function", "name": "write", "parameters": {"type": "object"}, "strict": true},
	]);
	let (sent, converted) = convert(ClientProtocol::Responses, &responses(&hello(), &tools)).unwrap();
	assert_eq!(sent["tools"][0]["function"]["name"], "mcp__read");
	assert_eq!(sent["tools"][1]["function"]["name"], "write");
	assert_eq!(sent["tools"][1]["function"]["strict"], true);
	assert_eq!(sent["tools"][0]["function"].get("strict"), None);
	let routes: Vec<_> = converted
		.reply
		.routes
		.iter()
		.map(|route| {
			(
				route.flat.as_str(),
				route.namespace.as_deref(),
				route.name.as_str(),
			)
		})
		.collect();
	assert_eq!(
		routes,
		[("mcp__read", Some("mcp"), "read"), ("write", None, "write")]
	);
}

#[test]
fn a_tool_name_that_doesnt_fit_or_collides_is_refused() {
	let tools = json!([
		{"type": "function", "name": "a.b", "parameters": {"type": "object"}},
		{"type": "function", "name": "x".repeat(65), "parameters": {"type": "object"}},
		{"type": "namespace", "name": "mcp", "tools": [
			{"type": "function", "name": "read", "parameters": {"type": "object"}},
		]},
		{"type": "function", "name": "mcp__read", "parameters": {"type": "object"}},
	]);
	let problems = problems(ClientProtocol::Responses, &responses(&hello(), &tools));
	assert_eq!(
		problems,
		[
			"`tools[0]` name `a.b` doesn't fit".to_string(),
			format!("`tools[1]` name `{}` doesn't fit", "x".repeat(65)),
			"`tools[3]` name `mcp__read` collides".to_string(),
		]
	);
}

#[test]
fn a_custom_tool_is_a_function_of_one_string() {
	let tools = json!([{"type": "custom", "name": "patch", "description": "Edit files.",
		"format": {"type": "grammar", "syntax": "lark", "definition": "start: /.+/"}}]);
	let (sent, converted) = convert(ClientProtocol::Responses, &responses(&hello(), &tools)).unwrap();
	let function = &sent["tools"][0]["function"];
	assert_eq!(function["name"], "patch");
	assert_eq!(
		function["parameters"],
		json!({"type": "object", "properties": {"input": {"type": "string"}}, "required": ["input"]})
	);
	assert_eq!(
		function["description"],
		"Edit files.\n\nInput must match this grammar:\nstart: /.+/"
	);
	assert_eq!(converted.dropped, ["custom tool grammar enforcement"]);
	assert_eq!(
		converted.reply.routes[0].kind,
		crate::protocols::CallKind::Custom
	);
}

#[test]
fn a_custom_call_in_history_goes_back_as_its_function() {
	let tools = json!([{"type": "custom", "name": "patch"}]);
	let input = json!([
		{"role": "user", "content": "hi"},
		{"type": "custom_tool_call", "call_id": "c1", "name": "patch", "input": "x"},
		{"type": "custom_tool_call_output", "call_id": "c1", "output": "done"},
	]);
	let sent = sent(ClientProtocol::Responses, &responses(&input, &tools));
	let call = &sent["messages"][1]["tool_calls"][0];
	assert_eq!(call["function"]["name"], "patch");
	assert_eq!(call["function"]["arguments"], r#"{"input":"x"}"#);
	assert_eq!(
		sent["messages"][2],
		json!({"role": "tool", "tool_call_id": "c1", "content": "done"})
	);
}

#[test]
fn reasoning_effort_comes_from_the_responses_request() {
	let mut body = responses(&hello(), &json!([]));
	body["reasoning"] = json!({"effort": "high"});
	assert_eq!(
		sent(ClientProtocol::Responses, &body)["reasoning_effort"],
		"high"
	);
}

fn messages(content: &Value) -> Value {
	json!({"model": "c", "max_tokens": 9, "messages": content})
}

#[test]
fn system_text_settings_and_streaming_are_sent() {
	let body = json!({
		"model": "c",
		"max_tokens": 9,
		"system": [{"type": "text", "text": "one"}, {"type": "text", "text": "two"}],
		"temperature": 0.5,
		"top_p": 0.9,
		"stream": true,
		"messages": [{"role": "user", "content": "hi"}],
	});
	let sent = sent(ClientProtocol::Messages, &body);
	assert_eq!(
		sent,
		json!({
			"model": "c",
			"messages": [
				{"role": "system", "content": "one\n\ntwo"},
				{"role": "user", "content": "hi"},
			],
			"stream": true,
			"stream_options": {"include_usage": true},
			"max_completion_tokens": 9,
			"temperature": 0.5,
			"top_p": 0.9,
		})
	);
}

#[test]
fn top_k_is_refused() {
	let mut body = messages(&hello());
	body["top_k"] = json!(5);
	assert_eq!(
		problems(ClientProtocol::Messages, &body),
		["setting `top_k`"]
	);
}

#[test]
fn images_are_data_urls_and_a_results_images_follow_it() {
	let image = json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "YWJj"}});
	let body = messages(&json!([
		{"role": "user", "content": [{"type": "text", "text": "look"}, image]},
		{"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "shot", "input": {}}]},
		{"role": "user", "content": [
			{"type": "tool_result", "tool_use_id": "t1", "is_error": true, "content": [
				{"type": "text", "text": "partial"}, image,
			]},
			{"type": "text", "text": "and?"},
		]},
	]));
	let sent = sent(ClientProtocol::Messages, &body);
	let url = json!({"type": "image_url", "image_url": {"url": "data:image/png;base64,YWJj"}});
	assert_eq!(
		sent["messages"][0]["content"],
		json!([{"type": "text", "text": "look"}, url])
	);
	assert_eq!(
		sent["messages"][1],
		json!({"role": "assistant", "content": null, "tool_calls": [
			{"id": "t1", "type": "function", "function": {"name": "shot", "arguments": "{}"}},
		]})
	);
	assert_eq!(
		sent["messages"][2],
		json!({"role": "tool", "tool_call_id": "t1", "content": "[tool error]\npartial"})
	);
	assert_eq!(
		sent["messages"][3],
		json!({"role": "user", "content": [{"type": "text", "text": "and?"}, url]})
	);
}

#[test]
fn an_image_url_is_refused() {
	let body = messages(&json!([{"role": "user", "content": [
		{"type": "image", "source": {"type": "url", "url": "https://example.com/a.png"}},
	]}]));
	let problems = problems(ClientProtocol::Messages, &body);
	assert!(
		problems
			.iter()
			.any(|problem| problem.contains("must be base64")),
		"{problems:?}"
	);
}

#[test]
fn every_call_needs_its_result_and_every_result_its_call() {
	let call = |id| json!({"type": "tool_use", "id": id, "name": "f", "input": {}});
	let result = |id| json!({"type": "tool_result", "tool_use_id": id, "content": "ok"});
	let answered = messages(&json!([
		{"role": "user", "content": "hi"},
		{"role": "assistant", "content": [call("a"), call("b")]},
		{"role": "user", "content": [result("a"), result("b")]},
	]));
	let sent = sent(ClientProtocol::Messages, &answered);
	let roles: Vec<&str> = sent["messages"]
		.as_array()
		.unwrap()
		.iter()
		.map(|message| message["role"].as_str().unwrap())
		.collect();
	assert_eq!(roles, ["user", "assistant", "tool", "tool"]);
	assert_eq!(
		sent["messages"][1]["tool_calls"].as_array().unwrap().len(),
		2
	);

	let unanswered = messages(&json!([
		{"role": "user", "content": "hi"},
		{"role": "assistant", "content": [call("a"), call("b")]},
		{"role": "user", "content": [result("a")]},
	]));
	assert!(
		problems(ClientProtocol::Messages, &unanswered)
			.contains(&"tool calls with no result: b".to_string())
	);
}

#[test]
fn structured_output_is_a_named_json_schema() {
	let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
	let mut body = messages(&hello());
	body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
	assert_eq!(
		sent(ClientProtocol::Messages, &body)["response_format"],
		json!({"type": "json_schema", "json_schema": {"name": "structured_output", "schema": schema}})
	);

	let mut body = responses(&hello(), &json!([]));
	body["text"] = json!({"format": {"type": "json_object"}});
	assert_eq!(
		sent(ClientProtocol::Responses, &body)["response_format"],
		json!({"type": "json_object"})
	);
}

#[test]
fn a_user_id_is_sent_as_user_and_a_long_one_as_its_hash() {
	let mut body = messages(&hello());
	body["metadata"] = json!({"user_id": "u-1"});
	assert_eq!(sent(ClientProtocol::Messages, &body)["user"], "u-1");
	let long = "u".repeat(65);
	body["metadata"] = json!({"user_id": long});
	let user = sent(ClientProtocol::Messages, &body)["user"].clone();
	let user = user.as_str().unwrap();
	assert!(user.len() <= 64, "{user}");
	assert!(user.bytes().all(|byte| byte.is_ascii_hexdigit()), "{user}");
	body["metadata"] = json!({"user_id": "u", "tier": "x"});
	assert_eq!(
		problems(ClientProtocol::Messages, &body),
		["field `metadata.tier`"]
	);
}

fn replayed(item: &str, scope: StateScope<'_>) -> Value {
	let carrier = crate::carrier::encode(
		scope,
		crate::carrier::CarrierPurpose::MessagesSignature,
		item.as_bytes(),
	);
	messages(&json!([
		{"role": "user", "content": "hi"},
		{"role": "assistant", "content": [
			{"type": "thinking", "thinking": "", "signature": carrier},
			{"type": "text", "text": "answer"},
		]},
		{"role": "user", "content": "again"},
	]))
}

#[test]
fn replayed_reasoning_goes_back_in_the_field_it_came_in() {
	let scope = upstream("c").scope;
	let item = r#"{"reasoning_content":"think","reasoning_signature":"sig"}"#;
	let assistant = &sent(ClientProtocol::Messages, &replayed(item, scope))["messages"][1];
	assert_eq!(
		assistant,
		&json!({"role": "assistant", "content": "answer",
			"reasoning_content": "think", "reasoning_signature": "sig"})
	);
	let item = r#"{"reasoning":"think"}"#;
	let assistant = &sent(ClientProtocol::Messages, &replayed(item, scope))["messages"][1];
	assert_eq!(assistant["reasoning"], "think");
	assert_eq!(assistant.get("reasoning_content"), None);
}

#[test]
fn reasoning_from_another_scope_or_without_text_is_refused() {
	let other = StateScope {
		model: "other",
		..upstream("c").scope
	};
	let item = r#"{"reasoning_content":"think"}"#;
	let problems_seen = problems(ClientProtocol::Messages, &replayed(item, other));
	assert_eq!(
		problems_seen,
		["`messages[1].content[0]`: carrier does not match this destination"]
	);
	let item = r#"{"type":"reasoning"}"#;
	let scope = upstream("c").scope;
	assert_eq!(
		problems(ClientProtocol::Messages, &replayed(item, scope)),
		["carrier contains no reasoning"]
	);
}
