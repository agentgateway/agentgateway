#![allow(clippy::unwrap_used)]

use std::borrow::Cow;
use std::num::NonZeroU64;

use rstest::rstest;
use serde_json::{Value, json};

use super::*;
use crate::ir::conversation::{
	Block, BlockId, Content, DataImage, Effort, Message, Role, TextPlace, ToolOutput, ToolPart,
};
use crate::test_support::{SCOPE, Sum, convert};

/// Each case's client stands for its pair: a Messages client on a Responses
/// provider, or a Responses client on a Messages provider.
const NOT_A_CASE: &str = "Chat Completions clients aren't a case here";

#[rstest]
fn a_parsed_request_reuses_escaped_text(
	#[values(
		ClientProtocol::Messages,
		ClientProtocol::Responses,
		ClientProtocol::ChatCompletions,
		ClientProtocol::Gemini
	)]
	client: ClientProtocol,
) {
	let text = "quoted \"text\"\n日本語 ".repeat(32);
	let body = match client {
		ClientProtocol::Messages | ClientProtocol::ChatCompletions => json!({
			"model": "m", "max_tokens": 64, "messages": [{"role": "user", "content": text}]
		}),
		ClientProtocol::Responses => json!({"model": "m", "max_output_tokens": 64, "input": text}),
		ClientProtocol::Gemini => json!({
			"contents": [{"role": "user", "parts": [{"text": text}]}],
			"generationConfig": {"maxOutputTokens": 64}
		}),
	};
	let cached = match client {
		ClientProtocol::Messages | ClientProtocol::ChatCompletions => &body["messages"][0]["content"],
		ClientProtocol::Responses => &body["input"],
		ClientProtocol::Gemini => &body["contents"][0]["parts"][0]["text"],
	}
	.as_str()
	.unwrap();
	let bytes = serde_json::to_vec(&body).unwrap();
	let upstream = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		protocol: if client == ClientProtocol::ChatCompletions {
			UpstreamProtocol::Messages
		} else {
			UpstreamProtocol::ChatCompletions
		},
		default_max_tokens: None,
	};
	let prepared = prepare_parsed(&body, client, upstream).unwrap();
	let texts: Vec<_> = prepared.conversation().texts().collect();
	assert_eq!(texts.len(), 1);
	let actual = texts[0].1.read();
	assert_eq!(actual, cached);
	assert_eq!(actual.as_ptr(), cached.as_ptr(), "reuse the decoded string");
	let rendered = prepared.render(RenderSettings::ModelDefaults).unwrap();
	let raw = convert(&bytes, client, upstream).unwrap();
	assert_eq!(rendered.body, raw.body);
	assert_eq!(rendered.dropped, raw.dropped);
}

#[rstest]
#[case::messages(ClientProtocol::Messages, r#"{"model":"m","max_tokens":64,"messages":[{"role":"user","content":"first","content":"last"}]}"#)]
#[case::chat(
	ClientProtocol::ChatCompletions,
	r#"{"model":"m","messages":[{"role":"user","content":null}]}"#
)]
#[case::responses(
	ClientProtocol::Responses,
	r#"{"model":"m","input":[{"role":"user","content":"first","content":"last"}]}"#
)]
#[case::gemini(
	ClientProtocol::Gemini,
	r#"{"contents":[{"role":"user","parts":[{"text":"first","text":"last"}]}]}"#
)]
#[case::gemini_snake_case(ClientProtocol::Gemini, r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generation_config":{"max_output_tokens":64}}"#)]
fn parsed_requests_validate_the_supplied_value(#[case] client: ClientProtocol, #[case] body: &str) {
	let parsed = serde_json::from_str(body).unwrap();
	let upstream = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		protocol: if client == ClientProtocol::ChatCompletions {
			UpstreamProtocol::Messages
		} else {
			UpstreamProtocol::ChatCompletions
		},
		default_max_tokens: NonZeroU64::new(64),
	};
	let canonical = serde_json::to_vec(&parsed).unwrap();
	let raw = prepare(&canonical, client, upstream)
		.and_then(|request| request.render(RenderSettings::ModelDefaults))
		.map(|request| (request.body, request.dropped, request.delivery));
	let cached = prepare_parsed(&parsed, client, upstream)
		.and_then(|request| request.render(RenderSettings::ModelDefaults))
		.map(|request| (request.body, request.dropped, request.delivery));
	assert_eq!(raw, cached);
}

#[rstest]
fn parsed_tool_json_is_borrowed_until_edited(
	#[values(UpstreamProtocol::Responses { safety_identifier: false, reasoning_context: false }, UpstreamProtocol::ChatCompletions, UpstreamProtocol::Gemini)]
	target: UpstreamProtocol,
) {
	let body = json!({
		"model": "m", "max_tokens": 64, "temperature": 0.5,
		"tools": [{"name": "f", "input_schema": {
			"type": "object", "properties": {"text": {"type": "string"}}
		}}],
		"messages": [
			{"role": "assistant", "content": [{"type": "tool_use", "id": "c", "name": "f", "input": {"text": "old\n日本語", "count": 42}}]},
			{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "c", "content": "result"}]}
		]
	});
	let upstream = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		protocol: target,
		default_max_tokens: NonZeroU64::new(64),
	};
	let mut prepared = prepare_parsed(&body, ClientProtocol::Messages, upstream).unwrap();
	let schema = prepared.conversation().tools[0].schema.as_ref();
	assert!(
		matches!(schema, crate::JsonRef::Parsed(value) if std::ptr::eq(value, &body["tools"][0]["input_schema"]))
	);
	let input = &prepared.conversation().messages[0].blocks[0].content;
	assert!(
		matches!(input, Content::ToolCall {input, ..} if matches!(input.as_ref(), crate::JsonRef::Parsed(value) if std::ptr::eq(value, &body["messages"][0]["content"][0]["input"])))
	);
	prepared.conversation_mut().edit_texts(|mut text| {
		if text.read().contains("old") {
			text.replace(String::from("new\n日本語"));
		}
	});
	assert_eq!(
		body["messages"][0]["content"][0]["input"]["text"],
		"old\n日本語"
	);
	let texts: Vec<_> = prepared
		.conversation()
		.texts()
		.map(|(_, text)| text.read().into_owned())
		.collect();
	assert!(texts.iter().any(|text| text == "new\n日本語"));
	let rendered = prepared.render(RenderSettings::ModelDefaults).unwrap();
	let sent: Value = serde_json::from_str(&rendered.body).unwrap();
	let arguments = match target {
		UpstreamProtocol::Responses { .. } => {
			serde_json::from_str(sent["input"][0]["arguments"].as_str().unwrap()).unwrap()
		},
		UpstreamProtocol::ChatCompletions => serde_json::from_str(
			sent["messages"][0]["tool_calls"][0]["function"]["arguments"]
				.as_str()
				.unwrap(),
		)
		.unwrap(),
		UpstreamProtocol::Gemini => sent["contents"][0]["parts"][0]["functionCall"]["args"].clone(),
		UpstreamProtocol::Messages => unreachable!("not one of the targets"),
	};
	assert_eq!(arguments["text"], "new\n日本語");
	assert_eq!(arguments["count"], 42);
}

#[rstest]
#[case::low(Effort::Low, "low")]
#[case::medium(Effort::Medium, "medium")]
#[case::high(Effort::High, "high")]
#[case::xhigh(Effort::XHigh, "xhigh")]
fn neutral_effort_renders_in_both_protocols(
	#[values(ClientProtocol::Messages, ClientProtocol::Responses)] client: ClientProtocol,
	#[case] effort: Effort,
	#[case] word: &str,
) {
	let mut request = prepared(client);
	request.conversation_mut().settings.effort = Some(effort);
	let body = rendered(request);
	let actual = match client {
		ClientProtocol::Messages => &body["reasoning"]["effort"],
		ClientProtocol::Responses => &body["output_config"]["effort"],
		ClientProtocol::Gemini => unreachable!("not one of the values"),
		ClientProtocol::ChatCompletions => unreachable!("{NOT_A_CASE}"),
	};
	assert_eq!(actual, word);
}

#[rstest]
#[case::high(Effort::High, "high")]
#[case::off(Effort::Off, "none")]
#[case::minimal(Effort::Minimal, "minimal")]
fn neutral_effort_overrides_client_effort(#[case] effort: Effort, #[case] word: &str) {
	let body = br#"{"model":"m","max_tokens":9,"thinking":{"type":"disabled"},"output_config":{"effort":"low"},"messages":[{"role":"user","content":"hi","output_config":{"effort":"medium"}}]}"#;

	let mut request = prepare(
		body,
		ClientProtocol::Messages,
		upstream(ClientProtocol::Messages),
	)
	.unwrap();
	request.conversation_mut().settings.effort = Some(effort);
	assert_eq!(rendered(request)["reasoning"]["effort"], word);
}

#[rstest]
fn neutral_images_render_in_both_protocols(
	#[values(ClientProtocol::Messages, ClientProtocol::Responses)] client: ClientProtocol,
	#[values(false, true)] tool_result: bool,
) {
	let mut request = prepared(client);
	let image = DataImage {
		media_type: "image/png".into(),
		data: Cow::Borrowed("YWJj"),
	};
	if tool_result {
		add_tool_result(request.conversation_mut(), ToolPart::DataImage(image));
	} else {
		request.conversation_mut().messages[0].blocks.push(Block {
			id: BlockId {
				message: 0,
				block: 1,
			},
			content: Content::DataImage(image),
		});
	}
	assert_image(&rendered(request), client, tool_result);
}

#[rstest]
fn raw_images_render_in_both_protocols(
	#[values(ClientProtocol::Messages, ClientProtocol::Responses)] client: ClientProtocol,
	#[values(false, true)] tool_result: bool,
) {
	let body = br#"{"model":"m","max_tokens":9,"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"YWJj"}}]}]}"#;

	let mut source = prepare(
		body,
		ClientProtocol::Messages,
		upstream(ClientProtocol::Messages),
	)
	.unwrap();
	let image = source.conversation_mut().messages[0]
		.blocks
		.remove(0)
		.content;
	let Content::Image(fields) = image else {
		unreachable!()
	};
	let mut request = prepared(client);
	if tool_result {
		add_tool_result(
			request.conversation_mut(),
			ToolPart::Image { index: 0, fields },
		);
	} else {
		request.conversation_mut().messages[0].blocks.push(Block {
			id: BlockId {
				message: 0,
				block: 1,
			},
			content: Content::Image(fields),
		});
	}
	assert_image(&rendered(request), client, tool_result);
}

#[test]
fn a_namespace_description_goes_before_its_tools_own() {
	let body = json!({"model": "m", "input": "hi", "tools": [{
		"type": "namespace", "name": "kernel", "description": "Runs code.",
		"tools": [
			{"type": "function", "name": "js", "description": "JavaScript.",
				"parameters": {"type": "object"}},
			{"type": "function", "name": "py", "parameters": {"type": "object"}},
		],
	}]})
	.to_string();
	let converted = convert(
		body.as_bytes(),
		ClientProtocol::Responses,
		upstream(ClientProtocol::Responses),
	)
	.unwrap();
	let body: Value = serde_json::from_str(&converted.body).unwrap();
	let descriptions: Vec<&Value> = body["tools"]
		.as_array()
		.unwrap()
		.iter()
		.map(|tool| &tool["description"])
		.collect();
	assert_eq!(descriptions, ["Runs code.\n\nJavaScript.", "Runs code."]);
	assert_eq!(converted.dropped, Vec::<&str>::new());
}

#[rstest]
fn rediscovered_tools_keep_both_search_results(
	#[values(
		UpstreamProtocol::Messages,
		UpstreamProtocol::ChatCompletions,
		UpstreamProtocol::Gemini
	)]
	protocol: UpstreamProtocol,
) {
	let tool = json!({"type":"namespace", "name":"repo", "tools":[{
		"type":"function", "name":"search", "description":"Search docs.",
		"strict":false, "parameters":{"type":"object", "properties":{}}
	}]});
	let body = json!({"model":"m", "max_output_tokens":64,
	"tools":[{"type":"tool_search", "execution":"client", "parameters":{"type":"object"}}],
	"input":[
		{"role":"user", "content":"Find docs."},
		{"type":"tool_search_call", "execution":"client", "call_id":"s1", "arguments":{}},
		{"type":"tool_search_output", "execution":"client", "call_id":"s1", "tools":[tool]},
		{"type":"tool_search_call", "execution":"client", "call_id":"s2", "arguments":{}},
		{"type":"tool_search_output", "execution":"client", "call_id":"s2", "tools":[tool]}
	]});
	let upstream = Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		protocol,
		default_max_tokens: None,
	};
	let bytes = serde_json::to_vec(&body).unwrap();
	let prepared = prepare(&bytes, ClientProtocol::Responses, upstream).unwrap();
	let results: Vec<_> = prepared
		.conversation()
		.messages
		.iter()
		.flat_map(|message| &message.blocks)
		.filter_map(|block| match &block.content {
			Content::ToolResult {
				call_id,
				output: ToolOutput::Text(text),
				..
			} => Some((call_id.as_str(), text.as_str())),
			_ => None,
		})
		.collect();
	assert_eq!(
		results,
		[
			("s1", "Loaded tools: search."),
			("s2", "Loaded tools: search.")
		]
	);
	let converted = prepared.render(RenderSettings::ModelDefaults).unwrap();
	assert_eq!(converted.reply.routes.len(), 2);
	assert_eq!(converted.body.matches("Loaded tools: search.").count(), 2);
	for (field, value) in [
		("parameters", json!({"type":"object", "required":["query"]})),
		("description", json!("Different docs.")),
		("strict", json!(true)),
		("namespace", Value::Null),
	] {
		let mut conflict = body.clone();
		if field == "namespace" {
			let mut flat = conflict["input"][4]["tools"][0]["tools"][0].clone();
			flat["name"] = json!("repo__search");
			conflict["input"][4]["tools"] = json!([flat]);
		} else {
			conflict["input"][4]["tools"][0]["tools"][0][field] = value;
		}
		let error = convert(
			&serde_json::to_vec(&conflict).unwrap(),
			ClientProtocol::Responses,
			upstream,
		)
		.unwrap_err();
		assert_eq!(
			error.problems,
			[if field == "namespace" {
				"`input[4].tools[0]` name `repo__search` collides"
			} else {
				"`input[4].tools[0].tools[0]` name `repo__search` collides"
			}],
			"{field}"
		);
	}
	let encoded = String::from_utf8(bytes).unwrap();
	let reordered = encoded.replacen(
		r#"{"type":"object","properties":{}}"#,
		r#"{"properties": {}, "type":"object"}"#,
		1,
	);
	assert_ne!(encoded, reordered);
	assert!(convert(reordered.as_bytes(), ClientProtocol::Responses, upstream).is_ok());
	let mut large = encoded.replace("\"properties\":{}", "\"maximum\":18446744073709551616");
	let at = large.rfind("18446744073709551616").unwrap();
	large.replace_range(at..at + 20, "18446744073709551617");
	let error = convert(large.as_bytes(), ClientProtocol::Responses, upstream).unwrap_err();
	assert_eq!(
		error.problems,
		["`input[4].tools[0].tools[0]` name `repo__search` collides"]
	);
}

/// A Messages request with text in every place: system, both roles, a tool call's
/// string arguments, and a tool result that is plain text and one that has an
/// image between texts.
const EVERY_TEXT: &[u8] = br#"{"model":"m","max_tokens":9,"system":"Mail a@b.com","messages":[
	{"role":"user","content":"I am a@b.com"},
	{"role":"assistant","content":[{"type":"text","text":"Looking"},
		{"type":"tool_use","id":"c","name":"f","input":{"to":"a@b.com","n":1,"note":"say \"a@b.com\""}},
		{"type":"tool_use","id":"d","name":"f","input":{}}]},
	{"role":"user","content":[
		{"type":"tool_result","tool_use_id":"c","content":"found a@b.com"},
		{"type":"tool_result","tool_use_id":"d","content":[{"type":"text","text":"see"},
			{"type":"image","source":{"type":"base64","media_type":"image/png","data":"YWJj"}},
			{"type":"text","text":"a@b.com"}]}]}]}"#;

#[test]
fn texts_lists_every_text_with_its_place() {
	let request = prepare(
		EVERY_TEXT,
		ClientProtocol::Messages,
		upstream(ClientProtocol::Messages),
	)
	.unwrap();
	let at = |message, block| BlockId { message, block };
	let texts: Vec<(TextPlace, String)> = request
		.conversation()
		.texts()
		.map(|(place, text)| (place, text.read().into_owned()))
		.collect();
	assert_eq!(
		texts,
		[
			(TextPlace::System(0), "Mail a@b.com".into()),
			(
				TextPlace::Message(Role::User, at(0, 0)),
				"I am a@b.com".into()
			),
			(
				TextPlace::Message(Role::Assistant, at(1, 0)),
				"Looking".into()
			),
			(
				TextPlace::ToolCall {
					block: at(1, 1),
					string: 0
				},
				"a@b.com".into()
			),
			(
				TextPlace::ToolCall {
					block: at(1, 1),
					string: 1
				},
				r#"say "a@b.com""#.into()
			),
			(
				TextPlace::ToolResult {
					block: at(2, 0),
					part: None
				},
				"found a@b.com".into()
			),
			(
				TextPlace::ToolResult {
					block: at(2, 1),
					part: Some(0)
				},
				"see".into()
			),
			(
				TextPlace::ToolResult {
					block: at(2, 1),
					part: Some(2)
				},
				"a@b.com".into()
			),
		]
	);
}

#[test]
fn an_edit_replaces_every_text_and_only_the_strings_it_changed() {
	let mut prepared = prepare(
		EVERY_TEXT,
		ClientProtocol::Messages,
		upstream(ClientProtocol::Messages),
	)
	.unwrap();
	let mut places = Vec::new();
	prepared.conversation_mut().edit_texts(|mut text| {
		let masked = text.read().replace("a@b.com", "[email]");
		if masked != text.read() {
			places.push(text.place());
			text.replace(masked);
		}
	});
	let converted = prepared.render(RenderSettings::ModelDefaults).unwrap();
	assert!(!converted.body.contains("a@b.com"), "{}", converted.body);
	assert_eq!(
		converted.body.matches("[email]").count(),
		6,
		"{}",
		converted.body
	);
	let body: Value = serde_json::from_str(&converted.body).unwrap();
	let call = body["input"]
		.as_array()
		.unwrap()
		.iter()
		.find(|item| item["call_id"] == "c")
		.unwrap();
	// Only the replaced strings are rewritten, escaped again for where they land.
	assert_eq!(
		call["arguments"],
		r#"{"to":"[email]","n":1,"note":"say \"[email]\""}"#
	);
	let at = |message, block| BlockId { message, block };
	assert_eq!(
		places,
		[
			TextPlace::System(0),
			TextPlace::Message(Role::User, at(0, 0)),
			TextPlace::ToolCall {
				block: at(1, 1),
				string: 0
			},
			TextPlace::ToolCall {
				block: at(1, 1),
				string: 1
			},
			TextPlace::ToolResult {
				block: at(2, 0),
				part: None
			},
			TextPlace::ToolResult {
				block: at(2, 1),
				part: Some(2)
			},
		]
	);
}

/// Replay carriers are opened only for the mapped upstream model.
#[test]
fn messages_carriers_are_opened_for_the_mapped_model() {
	let mapped = StateScope {
		model: "mapped",
		..SCOPE
	};
	let item = r#"{"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"e"}"#;
	let signature = crate::carrier::encode(
		mapped,
		crate::carrier::CarrierPurpose::MessagesSignature,
		item.as_bytes(),
	);
	let body = json!({
		"model": "claude",
		"max_tokens": 9,
		"messages": [
			{"role": "user", "content": "hi"},
			{"role": "assistant", "content": [{"type": "thinking", "thinking": "", "signature": signature}]},
			{"role": "user", "content": "again"}
		]
	})
	.to_string();
	let target = Upstream {
		scope: mapped,
		default_max_tokens: None,
		..upstream(ClientProtocol::Messages)
	};
	let request = rendered(prepare(body.as_bytes(), ClientProtocol::Messages, target).unwrap());
	assert_eq!(request["input"][1]["encrypted_content"], "e");

	let refused = prepare(
		body.as_bytes(),
		ClientProtocol::Messages,
		Upstream {
			scope: SCOPE,
			..target
		},
	)
	.and_then(|request| request.render(RenderSettings::ModelDefaults));
	assert!(refused.is_err(), "a carrier for another model was opened");
}

#[test]
fn excessive_stop_sequences_are_refused_before_reply_scanning() {
	let stops = std::iter::repeat_n("x", 65).collect::<Vec<_>>();
	let body = json!({
		"model": "m",
		"max_tokens": 9,
		"stop_sequences": stops,
		"messages": [{"role": "user", "content": "hi"}]
	})
	.to_string();
	let refusal = convert(
		body.as_bytes(),
		ClientProtocol::Messages,
		upstream(ClientProtocol::Messages),
	)
	.unwrap_err();
	assert_eq!(
		refusal.problems,
		["at most 64 stop sequences are supported"]
	);
}

#[rstest]
#[case::named(r#"{"model":"m","messages":[]}"#, Ok(Some("m")))]
#[case::escaped(r#"{"messages":[],"model":"gpt-5"}"#, Ok(Some("gpt-5")))]
#[case::absent(r#"{"messages":[]}"#, Ok(None))]
#[case::null(r#"{"model":null,"messages":[]}"#, Ok(None))]
#[case::not_a_name(r#"{"model":5}"#, Err(()))]
#[case::twice(r#"{"model":"a","model":"b"}"#, Err(()))]
#[case::not_an_object(r#"["m"]"#, Err(()))]
#[case::not_json(r#"{"model":"m""#, Err(()))]
fn reading_the_model_checks_its_shape(#[case] body: &str, #[case] model: Result<Option<&str>, ()>) {
	assert_eq!(
		read_model(body.as_bytes()).map_err(|_| ()),
		model.map(|model| model.map(Cow::Borrowed))
	);
}

#[test]
fn preparing_messages_for_a_messages_upstream_is_refused() {
	let body = br#"{"model":"m","max_tokens":9,"messages":[{"role":"user","content":"hi"}]}"#;
	assert!(
		prepare(
			body,
			ClientProtocol::Messages,
			Upstream {
				protocol: UpstreamProtocol::Messages,
				..upstream(ClientProtocol::Messages)
			}
		)
		.is_err()
	);
}

/// Only Messages requires an output limit, so only it needs one from the client or a default.
#[rstest]
#[case::messages(UpstreamProtocol::Messages, false)]
#[case::chat(UpstreamProtocol::ChatCompletions, true)]
#[case::gemini(UpstreamProtocol::Gemini, true)]
fn a_responses_client_leaves_out_its_limit_where_the_upstream_takes_none(
	#[case] protocol: UpstreamProtocol,
	#[case] converts: bool,
) {
	let body = br#"{"model":"m","input":"hi"}"#;
	let upstream = Upstream {
		default_max_tokens: None,
		protocol,
		..upstream(ClientProtocol::Responses)
	};
	let converted = prepare(body, ClientProtocol::Responses, upstream)
		.and_then(|request| request.render(RenderSettings::ModelDefaults));
	match converted {
		Ok(converted) => {
			assert!(converts);
			assert!(!converted.body.contains("max"), "{}", converted.body);
		},
		Err(refusal) => {
			assert!(!converts, "{refusal}");
			assert_eq!(
				refusal.problems,
				["no `max_output_tokens`, and no default for this model"]
			);
		},
	}
}

#[test]
fn a_caller_reads_the_request_across_an_await_then_edits_it() {
	let mut request = prepared(ClientProtocol::Messages);
	let conversation = request.conversation();
	let pending = async move {
		std::future::ready(()).await;
		(!conversation.messages.is_empty()).then_some(NonZeroU64::new(5))
	};
	let decided = ready(pending).unwrap();
	request.conversation_mut().settings.max_tokens = decided;
	assert_eq!(rendered(request)["max_output_tokens"], 5);
}

/// The gateway runs checks on a multi-threaded runtime.
/// This future has nothing outside to wait for, so one poll finishes it.
fn ready<F: std::future::Future + Send>(future: F) -> F::Output {
	let mut future = std::pin::pin!(future);
	let mut context = std::task::Context::from_waker(std::task::Waker::noop());
	match future.as_mut().poll(&mut context) {
		std::task::Poll::Ready(output) => output,
		std::task::Poll::Pending => unreachable!("a future with nothing to wait for is still pending"),
	}
}

#[rstest]
#[case::levels_override_the_name("gpt-4o", None, &[Effort::Low, Effort::High], Some("high"))]
#[case::unsupported_overrides_levels("gpt-5.4", Some(false), &[Effort::Medium], None)]
#[case::support_alone_does_not_imply_effort("gpt-4o", Some(true), &[], None)]
#[case::missing_facts_use_defaults("gpt-5.4", None, &[], Some("medium"))]
#[case::off_is_an_accepted_level("gpt-4o", None, &[Effort::Off], Some("none"))]
fn reasoning_controls_apply_to_the_edited_request(
	#[values(false, true)] chat: bool,
	#[case] model: &str,
	#[case] supported: Option<bool>,
	#[case] levels: &[Effort],
	#[case] expected: Option<&str>,
) {
	let body = br#"{"model":"m","max_tokens":9,"output_config":{"effort":"low"},
		"messages":[{"role":"user","content":"hi"}]}"#;
	let mut upstream = upstream(ClientProtocol::Messages);
	upstream.scope.model = model;
	if chat {
		upstream.protocol = UpstreamProtocol::ChatCompletions;
	}
	let mut request = prepare(body, ClientProtocol::Messages, upstream).unwrap();
	request.conversation_mut().settings.effort = Some(Effort::Medium);
	let mut controls = ReasoningControls::new(supported);
	for level in levels {
		controls = controls.with_effort(*level);
	}
	let settings = if chat {
		RenderSettings::ChatCompletions(controls)
	} else {
		RenderSettings::Responses(controls)
	};
	let converted = request.render(settings).unwrap();
	let sent: Value = serde_json::from_str(&converted.body).unwrap();
	let pointer = if chat {
		"/reasoning_effort"
	} else {
		"/reasoning/effort"
	};
	assert_eq!(
		sent.pointer(pointer).and_then(Value::as_str),
		expected,
		"{sent}"
	);
	if expected.is_none() {
		assert_eq!(
			converted.dropped,
			[if chat { "effort" } else { "reasoning effort" }]
		);
	} else {
		assert!(converted.dropped.is_empty(), "{:?}", converted.dropped);
	}
}

#[rstest]
#[case::gpt_5_4("gpt-5.4-mini", false, "none")]
#[case::other("deepseek-chat", true, "low")]
fn chat_capabilities_come_from_the_model_name(
	#[case] model: &str,
	#[case] native: bool,
	#[case] effort: &str,
) {
	let body = br#"{"model":"m","max_tokens":9,"stop_sequences":["END"],
		"output_config":{"effort":"low"},"tools":[{"name":"t","input_schema":{"type":"object"}}],
		"messages":[{"role":"user","content":"hi"}]}"#;
	let mut upstream = upstream(ClientProtocol::Messages);
	upstream.protocol = UpstreamProtocol::ChatCompletions;
	upstream.scope.model = model;
	let converted = prepare(body, ClientProtocol::Messages, upstream)
		.unwrap()
		.render(RenderSettings::ModelDefaults)
		.unwrap();
	let sent: Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(
		sent.get("stop"),
		native.then(|| json!(["END"])).as_ref(),
		"{sent}"
	);
	assert_eq!(
		converted.reply.emulated_stop_sequences,
		if native { vec![] } else { vec!["END"] }
	);
	assert_eq!(sent["reasoning_effort"], effort, "{sent}");
	assert_eq!(
		converted.dropped,
		if effort == "none" {
			vec!["effort beside tools"]
		} else {
			vec![]
		}
	);
}

fn prepared(client: ClientProtocol) -> PreparedRequest<'static, 'static> {
	let body: &[u8] = match client {
		ClientProtocol::Messages => {
			br#"{"model":"m","max_tokens":9,"messages":[{"role":"user","content":"hi"}]}"#
		},
		ClientProtocol::Responses => br#"{"model":"m","input":"hi"}"#,
		ClientProtocol::Gemini => unreachable!("these tests pair Messages and Responses"),
		ClientProtocol::ChatCompletions => unreachable!("{NOT_A_CASE}"),
	};
	prepare(body, client, upstream(client)).unwrap()
}

fn upstream(client: ClientProtocol) -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &Sum,
		scope: SCOPE,
		default_max_tokens: NonZeroU64::new(9),
		protocol: match client {
			ClientProtocol::Messages => UpstreamProtocol::Responses {
				safety_identifier: false,
				reasoning_context: false,
			},
			ClientProtocol::Responses => UpstreamProtocol::Messages,
			ClientProtocol::Gemini => unreachable!("these tests pair Messages and Responses"),
			ClientProtocol::ChatCompletions => unreachable!("{NOT_A_CASE}"),
		},
	}
}

fn rendered(request: PreparedRequest<'_, '_>) -> Value {
	let converted = request.render(RenderSettings::ModelDefaults).unwrap();
	serde_json::from_str(&converted.body).unwrap()
}

fn add_tool_result<'a>(conversation: &mut Conversation<'a>, image: ToolPart<'a>) {
	conversation.messages.push(Message {
		index: 1,
		role: Role::Assistant,
		messages_output_config: None,
		blocks: vec![Block {
			id: BlockId {
				message: 1,
				block: 0,
			},
			content: Content::ToolCall {
				id: "c".into(),
				name: "f".into(),
				namespace: None,
				input: serde_json::value::RawValue::from_string("{}".into())
					.unwrap()
					.into(),
			},
		}],
	});
	conversation.messages.push(Message {
		index: 2,
		role: Role::User,
		messages_output_config: None,
		blocks: vec![Block {
			id: BlockId {
				message: 2,
				block: 0,
			},
			content: Content::ToolResult {
				call_id: "c".into(),
				failed: Some(true),
				output: ToolOutput::Mixed(vec![image]),
			},
		}],
	});
}

fn assert_image(body: &Value, client: ClientProtocol, tool_result: bool) {
	match client {
		ClientProtocol::Messages => {
			let image = if tool_result {
				&body["input"][2]["output"][1]
			} else {
				&body["input"][0]["content"][1]
			};
			assert_eq!(image["type"], "input_image");
			assert_eq!(image["image_url"], "data:image/png;base64,YWJj");
		},
		ClientProtocol::Responses => {
			let image = if tool_result {
				&body["messages"][2]["content"][0]["content"][0]
			} else {
				&body["messages"][0]["content"][1]
			};
			assert_eq!(image["type"], "image");
			assert_eq!(
				image["source"],
				json!({"type":"base64","media_type":"image/png","data":"YWJj"})
			);
		},
		ClientProtocol::Gemini => unreachable!("not one of the values"),
		ClientProtocol::ChatCompletions => unreachable!("{NOT_A_CASE}"),
	}
}
