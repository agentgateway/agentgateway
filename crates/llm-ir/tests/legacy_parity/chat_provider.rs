//! Runs legacy goldens through the LLM IR with an OpenAI-compatible Chat Completions provider:
//! Messages and Responses requests rendered for it, and its replies written back for Messages
//! and Responses clients. The IR's result is snapshotted beside the legacy snapshots as `<case>.llm-ir-completions.snap` for
//! requests and `<case>.llm-ir-messages.snap` or `<case>.llm-ir-responses.snap` for replies.
//!
//! Legacy converts Messages requests to Chat Completions for OpenAI-compatible providers, and
//! Responses requests for Gemini's OpenAI-compatible endpoint; inputs its goldens don't send
//! there are outside the route. Replies are compared as what the client can read from them:
//! text, reasoning, calls, how the reply ended, and the usage accounting read.

use std::fs;
use std::num::NonZeroU64;

use agent_llm::conversion::{completions, openai_compat};
use agent_llm::types::{messages, responses};
use agent_llm::{InputFormat, LogContentFields};
use agent_llm_ir::{ClientProtocol, ReplyContext, StateScope, Upstream, UpstreamProtocol};
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::{Value, json};

use super::Disposition::*;
use super::{
	Disposition, FixedIdentifier, IrRequest, assert_every_input_listed, assert_reply_refusal,
	assert_request_refusal, describe, drive_buffered_reply, drive_streamed_reply, ir_request,
	ir_usage, legacy_stream_usage, legacy_usage, model_of, reply_snapshot, snapshot, sse_data,
};

/// The clients legacy serves from a Chat Completions provider.
#[derive(Debug, Clone, Copy)]
enum Client {
	Messages,
	Responses,
}

impl Client {
	const fn protocol(self) -> ClientProtocol {
		match self {
			Self::Messages => ClientProtocol::Messages,
			Self::Responses => ClientProtocol::Responses,
		}
	}
}

const CACHE_MARKS: &str = "cache marks are dropped and recorded, where legacy sends `prompt_cache_breakpoint` on Chat Completions parts";
const SYSTEM_JOINED: &str =
	"system blocks are joined with a blank line, where legacy sends each as its own part";
const TOP_K: &str = "`top_k`, which Chat Completions has no field for and legacy drops";
const NO_EFFORT: &str = "adaptive thinking without an effort sends no `reasoning_effort`, leaving the model's default, where legacy sends `none` beside tools";

const MESSAGES: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	(
		"cache_control",
		Changed(
			"cache marks are dropped and recorded, where legacy sends `prompt_cache_breakpoint`; system and assistant text blocks are joined; a tool result's images follow the results in a user message; no effort is sent beside tools, where legacy sends `none`",
		),
	),
	("cache_control_dropped_blocks", OutsideRoute),
	("cache_control_dropped_server_tools", OutsideRoute),
	("cache_control_reasoning_prefix", OutsideRoute),
	("cache_control_responses", OutsideRoute),
	("cache_control_unsupported", Changed(SYSTEM_JOINED)),
	("citations", Equivalent),
	("document_inline", OutsideRoute),
	("gpt_adaptive_thinking_with_tools", Changed(NO_EFFORT)),
	(
		"metadata",
		Unsupported("`metadata` fields other than `user_id`"),
	),
	("reasoning", Unsupported("`output_format`")),
	(
		"reasoning_enabled",
		Changed(
			"a thinking budget is dropped and recorded and no effort is sent, where legacy turns the budget into effort `low`",
		),
	),
	(
		"reasoning_replay",
		Unsupported("redacted thinking, and thinking signed by another provider, which legacy drops"),
	),
	("reasoning_unsupported_model", Equivalent),
	("responses_agent_subset", OutsideRoute),
	(
		"server_tools",
		Unsupported("server tools such as web search"),
	),
	("stop_sequences", Unsupported(TOP_K)),
	("structured-output", Unsupported(TOP_K)),
	("system_message", Changed(CACHE_MARKS)),
	("system_message_mid_conversation", OutsideRoute),
	("tool_history_without_tools", Equivalent),
	(
		"tool_reference",
		Changed(
			"a `tool_reference` block becomes text naming the loaded tools, where legacy sends the bare name; cache marks are dropped and recorded; no effort is sent beside tools, where legacy sends `none`",
		),
	),
	(
		"tool_result_empty",
		LegacyBug("Messages allows a `tool_result` without `content`; legacy refuses the request"),
	),
	(
		"tool_result_error",
		LegacyBug(
			"legacy drops `is_error`, so the model can't tell a failed call; the IR marks the output",
		),
	),
	(
		"tool_result_image",
		Unsupported("an image URL inside a tool result"),
	),
	(
		"tool_result_unknown_part",
		Unsupported("unknown block types, which legacy drops"),
	),
	("tools", Equivalent),
	(
		"unknown_content",
		Unsupported("unknown block types, which legacy drops"),
	),
];

const MESSAGES_DIRECTORY: &str = "src/tests/requests/messages";

#[test]
fn every_messages_request_golden_has_a_chat_disposition() {
	assert_every_input_listed(MESSAGES_DIRECTORY, MESSAGES.iter().map(|(name, _)| *name));
}

#[test]
fn messages_requests_on_a_chat_provider() {
	for (name, disposition) in MESSAGES {
		compare_request(MESSAGES_DIRECTORY, name, *disposition, Client::Messages);
	}
}

const RESPONSES: &[(&str, Disposition)] = &[
	("assistant-history", OutsideRoute),
	("basic", Equivalent),
	(
		"cache_control",
		Unsupported("`prompt_cache_breakpoint` on input parts, and `input_file` blocks"),
	),
	("cache_control_tool_outputs", OutsideRoute),
	(
		"codex-assistant-history",
		Changed(
			"a developer message is sent as a system message, where legacy keeps the `developer` role",
		),
	),
	("empty-message", OutsideRoute),
	(
		"input-list",
		Unsupported("reasoning items without a carrier the gateway signed"),
	),
	("input-media", OutsideRoute),
	(
		"instructions",
		Unsupported("`service_tier`, which legacy drops"),
	),
	("namespace-tools", Equivalent),
	("namespace-tools-long", OutsideRoute),
	(
		"parallel-tool-call",
		Unsupported("call arguments that aren't a JSON object, and the output answering that call"),
	),
	("structured-output", OutsideRoute),
];

const RESPONSES_DIRECTORY: &str = "src/tests/requests/responses";

#[test]
fn every_responses_request_golden_has_a_chat_disposition() {
	assert_every_input_listed(RESPONSES_DIRECTORY, RESPONSES.iter().map(|(name, _)| *name));
}

#[test]
fn responses_requests_on_a_chat_provider() {
	for (name, disposition) in RESPONSES {
		compare_request(RESPONSES_DIRECTORY, name, *disposition, Client::Responses);
	}
}

/// The output limit the IR needs for a Responses request without one, as the gateway would
/// supply it from its model catalog. Legacy sends no limit, so expected requests add it.
const DEFAULT_MAX_TOKENS: u64 = 4096;

fn compare_request(directory: &str, name: &str, disposition: Disposition, client: Client) {
	if matches!(disposition, OutsideRoute) {
		return;
	}
	let body =
		fs::read_to_string(super::legacy_fixture(directory).join(format!("{name}.json"))).unwrap();
	let legacy = legacy_request(&body, client);
	let ir = ir_request(&body, client.protocol(), chat_upstream(&body));
	snapshot(
		directory,
		&format!("{name}.llm-ir-completions"),
		&describe(disposition),
		&body,
		&ir,
	);
	match (disposition, &legacy, &ir) {
		(Equivalent, Ok(legacy), Ok(ir)) => assert_eq!(
			expected_chat_request(legacy),
			without_representation(&ir.request),
			"{name}"
		),
		(Changed(_) | LegacyBug(_), Ok(legacy), Ok(ir)) => {
			assert_request_change(name, client, legacy, ir);
		},
		(LegacyBug(_), Err(error), Ok(ir)) => {
			assert!(matches!(client, Client::Messages));
			assert_eq!(name, "tool_result_empty");
			assert!(error.contains("missing field `content`"), "{error}");
			let mut explicit_empty: Value = serde_json::from_str(&body).unwrap();
			let result = &mut explicit_empty["messages"][2]["content"][0];
			assert_eq!(result["type"], "tool_result");
			assert!(result.get("content").is_none());
			result["content"] = json!("");
			let legacy = legacy_request(&explicit_empty.to_string(), client).unwrap();
			assert_eq!(
				without_representation(&ir.request),
				expected_chat_request(&legacy),
				"{name}: missing content must behave like explicit empty content"
			);
			assert!(ir.dropped.is_empty(), "{name}");
			assert!(ir.emulated_stop_sequences.is_empty(), "{name}");
		},
		(Unsupported(_), Ok(_), Err(error)) => {
			assert_request_refusal(
				name,
				client.protocol(),
				UpstreamProtocol::ChatCompletions,
				error,
			);
		},
		(disposition, legacy, ir) => panic!(
			"{name}: {disposition:?}, but legacy gave {:?} and the IR {:?}",
			legacy.as_ref().map(|_| "a request"),
			ir.as_ref().map(|_| "a request"),
		),
	}
}

fn assert_request_change(name: &str, client: Client, legacy: &Value, ir: &IrRequest) {
	let mut expected = expected_chat_request(legacy);
	let cache_marks: &[(usize, usize)] = match (client, name) {
		(Client::Messages, "cache_control") => &[
			(0, 1),
			(1, 1),
			(2, 0),
			(4, 0),
			(5, 0),
			(6, 0),
			(7, 1),
			(8, 0),
		],
		(Client::Messages, "system_message") => &[(0, 1)],
		(Client::Messages, "tool_reference") => &[(2, 0)],
		_ => &[],
	};
	for &(message, part) in cache_marks {
		let mark = expected["messages"][message]["content"][part]
			.as_object_mut()
			.unwrap()
			.remove("prompt_cache_breakpoint");
		assert_eq!(mark, Some(json!({"mode": "explicit"})), "{name}");
	}
	let expected_dropped: &[&str] = match (client, name) {
		(Client::Messages, "cache_control") => {
			assert_eq!(
				expected.as_object_mut().unwrap().remove("reasoning_effort"),
				Some(json!("none"))
			);
			let messages = expected["messages"].as_array_mut().unwrap();
			assert_eq!(
				messages[0]["content"],
				json!([
					{"type": "text", "text": "First system block"},
					{"type": "text", "text": "Second system block"},
				])
			);
			messages[0]["content"] =
				json!([{"type": "text", "text": "First system block\n\nSecond system block"}]);
			assert_eq!(
				messages[1]["content"],
				json!([
					{"type": "text", "text": "First assistant block"},
					{"type": "text", "text": "Second assistant block"},
				])
			);
			messages[1]["content"] =
				json!([{"type": "text", "text": "First assistant blockSecond assistant block"}]);
			assert_eq!(messages[7]["tool_call_id"], "call_mixed");
			assert_eq!(
				messages[7]["content"],
				json!([
					{"type": "text", "text": "before image"},
					{"type": "text", "text": "after image"},
				])
			);
			messages[7]["content"] = json!([{"type": "text", "text": "before image\nafter image"}]);
			messages.insert(
				8,
				json!({"role": "user", "content": [
					{"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8="}},
					{"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8="}},
				]}),
			);
			&["cache_control"]
		},
		(Client::Messages, "cache_control_unsupported") => {
			assert_eq!(
				expected["messages"][0]["content"],
				json!([
					{"type": "text", "text": "Stable instructions"},
					{"type": "text", "text": "Cached instructions"},
				])
			);
			expected["messages"][0]["content"] =
				json!([{"type": "text", "text": "Stable instructions\n\nCached instructions"}]);
			&["cache_control"]
		},
		(Client::Messages, "gpt_adaptive_thinking_with_tools") => {
			assert_eq!(
				expected.as_object_mut().unwrap().remove("reasoning_effort"),
				Some(json!("none"))
			);
			&[]
		},
		(Client::Messages, "reasoning_enabled") => {
			assert_eq!(
				expected.as_object_mut().unwrap().remove("reasoning_effort"),
				Some(json!("low"))
			);
			&["thinking.budget_tokens"]
		},
		(Client::Messages, "system_message") => &["cache_control"],
		(Client::Messages, "tool_reference") => {
			assert_eq!(
				expected.as_object_mut().unwrap().remove("reasoning_effort"),
				Some(json!("none"))
			);
			let text = &mut expected["messages"][2]["content"][0]["text"];
			assert_eq!(*text, "mcp__example__list_widgets");
			*text = json!("Loaded tools: mcp__example__list_widgets.");
			&["cache_control"]
		},
		(Client::Messages, "tool_result_error") => {
			assert_eq!(expected["messages"][2]["tool_call_id"], "toolu_01");
			let text = &mut expected["messages"][2]["content"][0]["text"];
			assert_eq!(*text, "Error: location not found");
			*text = json!("[tool error]\nError: location not found");
			&[]
		},
		(Client::Responses, "codex-assistant-history") => {
			assert_eq!(expected["messages"][0]["role"], "developer");
			expected["messages"][0]["role"] = json!("system");
			&["input item id and status"]
		},
		_ => panic!("{name}: intentional change needs specific assertions for {client:?}"),
	};
	assert_eq!(
		without_representation(&ir.request),
		expected,
		"{name}: changes beyond those declared"
	);
	assert_eq!(ir.dropped, expected_dropped, "{name}: dropped fields");
	assert!(ir.emulated_stop_sequences.is_empty(), "{name}");
}

fn legacy_request(body: &str, client: Client) -> Result<Value, String> {
	let request = match client {
		Client::Messages => {
			let request: messages::Request = serde_json::from_str(body).map_err(|err| err.to_string())?;
			completions::from_messages::translate(&request)
		},
		Client::Responses => {
			let request: responses::Request =
				serde_json::from_str(body).map_err(|err| err.to_string())?;
			openai_compat::from_responses::translate(&request)
		},
	};
	let request = request.map_err(|err| err.to_string())?;
	Ok(serde_json::from_slice(&request).unwrap())
}

fn chat_upstream(body: &str) -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &FixedIdentifier,
		scope: StateScope {
			backend: "golden",
			provider: "openai",
			model: model_of(body),
		},
		protocol: UpstreamProtocol::ChatCompletions,
		default_max_tokens: NonZeroU64::new(DEFAULT_MAX_TOKENS),
	}
}

/// The gateway supplies a default output limit only where legacy sent no limit.
/// Keep it in the expected request so losing an explicit limit cannot normalize away.
fn expected_chat_request(legacy: &Value) -> Value {
	let mut expected = legacy.clone();
	if expected.get("max_completion_tokens").is_none() {
		expected["max_completion_tokens"] = json!(DEFAULT_MAX_TOKENS);
	}
	without_representation(&expected)
}

/// Only `stream: false`, assistant tool-call content null versus absent, lone text
/// strings versus text parts, and JSON argument formatting are equivalent here.
fn without_representation(request: &Value) -> Value {
	let mut request = request.clone();
	let object = request.as_object_mut().unwrap();
	if object.get("stream") == Some(&json!(false)) {
		object.remove("stream");
	}
	for message in object
		.get_mut("messages")
		.and_then(Value::as_array_mut)
		.into_iter()
		.flatten()
	{
		let message = message.as_object_mut().unwrap();
		match message.get("content") {
			Some(Value::Null)
				if message.get("role") == Some(&json!("assistant"))
					&& message
						.get("tool_calls")
						.and_then(Value::as_array)
						.is_some_and(|calls| !calls.is_empty()) =>
			{
				message.remove("content");
			},
			Some(Value::String(text)) => {
				let parts = json!([{"type": "text", "text": text}]);
				message.insert("content".to_string(), parts);
			},
			_ => {},
		}
		for call in message
			.get_mut("tool_calls")
			.and_then(Value::as_array_mut)
			.into_iter()
			.flatten()
		{
			let arguments = &mut call["function"]["arguments"];
			if let Some(text) = arguments.as_str() {
				*arguments = serde_json::from_str(text).unwrap();
			}
		}
	}
	request
}

/// Chat Completions replies, written for a Messages client. Names starting with `stream` are
/// streamed.
const REPLIES_FOR_MESSAGES: &[(&str, Disposition)] = &[
	("audio", Equivalent),
	("basic", Equivalent),
	("cache_write", Equivalent),
	("content_filter", Equivalent),
	("gemini_thinking", Equivalent),
	("gemini_with_completion_tokens", Equivalent),
	("gemini_zero_completion_tokens", Equivalent),
	("guardrails", OutsideRoute),
	("missing_model", Equivalent),
	("mistral_reasoning", Unsupported(MISTRAL)),
	("namespace-tools", OutsideRoute),
	("namespace-tools-stream", OutsideRoute),
	("openrouter_reasoning", Equivalent),
	("reasoning", Equivalent),
	("reasoning_omitted", Equivalent),
	("stop_sequence", Equivalent),
	("stream", Equivalent),
	("stream-content_filter", Equivalent),
	("stream-gemini_thinking", Equivalent),
	("stream-vllm_reasoning", Equivalent),
	("stream_mistral_reasoning", Unsupported(MISTRAL)),
	("stream_tool_empty_content", Equivalent),
	("tool_call", Equivalent),
	(
		"truncated_tool_call",
		LegacyBug(
			"legacy writes the cut-off arguments as a string `input`, which Messages doesn't allow; the IR leaves out the incomplete call",
		),
	),
];

/// The same replies, written for a Responses client.
const REPLIES_FOR_RESPONSES: &[(&str, Disposition)] = &[
	("audio", Equivalent),
	("basic", Equivalent),
	("cache_write", OutsideRoute),
	("content_filter", OutsideRoute),
	("gemini_thinking", Equivalent),
	(
		"gemini_with_completion_tokens",
		LegacyBug(INCOMPLETE_REASON),
	),
	(
		"gemini_zero_completion_tokens",
		LegacyBug(INCOMPLETE_REASON),
	),
	("guardrails", OutsideRoute),
	("missing_model", Equivalent),
	("mistral_reasoning", Unsupported(MISTRAL)),
	("namespace-tools", OutsideRoute),
	("namespace-tools-stream", OutsideRoute),
	("openrouter_reasoning", Changed(REASONING_SHOWN)),
	("reasoning", OutsideRoute),
	("reasoning_omitted", OutsideRoute),
	("stop_sequence", OutsideRoute),
	("stream", Equivalent),
	("stream-content_filter", OutsideRoute),
	("stream-gemini_thinking", Equivalent),
	("stream-vllm_reasoning", Changed(REASONING_SHOWN)),
	("stream_mistral_reasoning", Unsupported(MISTRAL)),
	(
		"stream_tool_empty_content",
		Changed("the provider's call id is kept, where legacy's stream makes up a new one"),
	),
	("tool_call", Equivalent),
	(
		"truncated_tool_call",
		LegacyBug(
			"legacy marks a call whose arguments were cut off as completed, and sends incomplete reason `max_tokens`, which Responses doesn't have; the IR omits the call so clients cannot dispatch it and writes `max_output_tokens`",
		),
	),
];

const MISTRAL: &str = "Mistral's `content` list with `thinking` parts, whose thinking legacy drops";
const INCOMPLETE_REASON: &str = "legacy ends with incomplete reason `max_tokens`, which Responses doesn't have; the IR writes `max_output_tokens`";
const REASONING_SHOWN: &str =
	"reasoning text is shown only to a client that asked for summaries, where legacy always sends it";

const REPLIES: &str = "src/tests/response/completions";

#[test]
fn every_chat_reply_golden_has_a_disposition() {
	for listed in [REPLIES_FOR_MESSAGES, REPLIES_FOR_RESPONSES] {
		assert_every_input_listed(REPLIES, listed.iter().map(|(name, _)| *name));
	}
}

#[tokio::test]
async fn chat_replies_for_a_messages_client() {
	for (name, disposition) in REPLIES_FOR_MESSAGES {
		compare_reply(name, *disposition, Client::Messages).await;
	}
}

#[tokio::test]
async fn chat_replies_for_a_responses_client() {
	for (name, disposition) in REPLIES_FOR_RESPONSES {
		compare_reply(name, *disposition, Client::Responses).await;
	}
}

/// What a client can read from a reply, and the usage accounting read.
#[derive(Debug, PartialEq)]
struct Reply {
	client: ClientReply,
	usage: Value,
}

#[derive(Debug, PartialEq)]
enum ClientReply {
	Body(Value),
	/// The `data` of each event.
	Events(Vec<Value>),
	/// Legacy's failures are errors without an HTTP reply; both are compared as failures only.
	Failed {
		detail: String,
	},
}

impl ClientReply {
	/// The reply as its snapshot records it.
	fn report(&self) -> Value {
		match self {
			Self::Body(body) => json!({"body": body}),
			Self::Events(events) => json!({"events": events}),
			Self::Failed { detail } => json!({"failed": detail}),
		}
	}
}

async fn compare_reply(name: &str, disposition: Disposition, client: Client) {
	if matches!(disposition, OutsideRoute) {
		return;
	}
	let body = fs::read(super::legacy_fixture(REPLIES).join(format!("{name}.json"))).unwrap();
	let streamed = name.starts_with("stream");
	let (legacy, ir) = if streamed {
		(
			legacy_streamed_reply(&body, client).await,
			ir_streamed_reply(&body, client),
		)
	} else {
		(
			legacy_buffered_reply(&body, client),
			ir_buffered_reply(&body, client),
		)
	};
	let suffix = match client {
		Client::Messages => "messages",
		Client::Responses => "responses",
	};
	let case = format!("{name}.llm-ir-{suffix}");
	reply_snapshot(
		REPLIES,
		&case,
		disposition,
		&body,
		&ir.client.report(),
		&ir.usage,
	);
	match disposition {
		Equivalent => assert_eq!(readable(&legacy, client), readable(&ir, client), "{name}"),
		Changed(_) | LegacyBug(_) => assert_reply_change(name, client, &legacy, &ir),
		Unsupported(_) => {
			assert!(readable(&ir, client)["reply"]["failed"] == true);
			assert!(readable(&legacy, client)["reply"]["failed"] != true);
			assert!(matches!(
				name,
				"mistral_reasoning" | "stream_mistral_reasoning"
			));
			assert_reply_refusal(
				&body,
				reply_context(client),
				streamed,
				"wrong type: content",
			);
		},
		OutsideRoute => unreachable!("not converted"),
	}
}

fn assert_reply_change(name: &str, client: Client, legacy: &Reply, ir: &Reply) {
	let mut expected = readable(legacy, client);
	match (client, name) {
		(Client::Messages, "truncated_tool_call") => {
			assert_eq!(
				expected["reply"]["calls"],
				json!([["call_1", "get_weather", "{\"location\": \"Par"]])
			);
			expected["reply"]["calls"] = json!([]);
		},
		(
			Client::Responses,
			"gemini_with_completion_tokens" | "gemini_zero_completion_tokens" | "truncated_tool_call",
		) => {
			assert_eq!(
				expected["reply"]["ended"],
				json!(["incomplete", "max_tokens"])
			);
			expected["reply"]["ended"] = json!(["incomplete", "max_output_tokens"]);
			if name == "truncated_tool_call" {
				let (ClientReply::Body(legacy), ClientReply::Body(ir)) = (&legacy.client, &ir.client)
				else {
					panic!("{name}: expected buffered replies");
				};
				assert_eq!(legacy["output"][0]["status"], "completed");
				assert_eq!(ir["output"], json!([]));
				assert_eq!(
					expected["reply"]["calls"],
					json!([["call_1", "get_weather", "{\"location\": \"Par"]])
				);
				expected["reply"]["calls"] = json!([]);
			}
		},
		(Client::Responses, "openrouter_reasoning" | "stream-vllm_reasoning") => {
			let text = match name {
				"openrouter_reasoning" => "Okay, the user wants me to write a story",
				_ => "The user wants a sum.",
			};
			assert_eq!(expected["reply"]["reasoning"], json!([text]));
			expected["reply"]["reasoning"] = json!([""]);
		},
		(Client::Responses, "stream_tool_empty_content") => {
			let calls = expected["reply"]["calls"].as_array_mut().unwrap();
			assert_eq!(calls.len(), 1);
			let generated = calls[0][0].as_str().unwrap().strip_prefix("call_").unwrap();
			assert_eq!(generated.len(), 16);
			assert!(generated.bytes().all(|byte| byte.is_ascii_hexdigit()));
			assert_eq!(calls[0][1], "Bash");
			assert_eq!(calls[0][2], json!({"command": "pwd"}));
			calls[0][0] = json!("toolu_empty_content");
		},
		_ => panic!("{name}: intentional reply change needs specific assertions for {client:?}"),
	}
	assert_eq!(
		readable(ir, client),
		expected,
		"{name}: changes beyond those declared"
	);
}

fn legacy_buffered_reply(body: &[u8], client: Client) -> Reply {
	let body = Bytes::copy_from_slice(body);
	let reply = match client {
		Client::Messages => completions::from_messages::translate_response(&body, "input-model"),
		Client::Responses => {
			openai_compat::to_responses::translate_response(&body, "input-model", None)
		},
	};
	match reply {
		Ok(reply) => Reply {
			usage: legacy_usage(&reply.to_llm_response(LogContentFields::default())),
			client: ClientReply::Body(serde_json::from_slice(&reply.serialize().unwrap()).unwrap()),
		},
		Err(err) => Reply {
			client: ClientReply::Failed {
				detail: err.to_string(),
			},
			usage: Value::Null,
		},
	}
}

async fn legacy_streamed_reply(body: &[u8], client: Client) -> Reply {
	let format = match client {
		Client::Messages => InputFormat::Messages,
		Client::Responses => InputFormat::Responses,
	};
	let (info, guard) = legacy_stream_usage(format);
	let input = agent_http::Body::from(body.to_vec());
	let output = match client {
		Client::Messages => completions::from_messages::translate_stream(
			input,
			1024 * 1024,
			guard,
			LogContentFields::default(),
		),
		Client::Responses => openai_compat::to_responses::translate_stream(
			input,
			1024 * 1024,
			guard,
			LogContentFields::default(),
			None,
		),
	};
	let output = output.collect().await.unwrap().to_bytes();
	let usage = legacy_usage(&info.lock().unwrap().response);
	Reply {
		client: ClientReply::Events(sse_data(&output)),
		usage,
	}
}

fn ir_buffered_reply(body: &[u8], client: Client) -> Reply {
	let (reply, usage) = drive_buffered_reply(body, reply_context(client));
	Reply {
		client: match reply.status {
			200 => ClientReply::Body(serde_json::from_slice(&reply.body).unwrap()),
			status => ClientReply::Failed {
				detail: format!("{status} {}", String::from_utf8_lossy(&reply.body)),
			},
		},
		usage: ir_usage(usage),
	}
}

fn ir_streamed_reply(body: &[u8], client: Client) -> Reply {
	let (reply, usage) = drive_streamed_reply(body, reply_context(client));
	Reply {
		client: match reply {
			Ok(out) => ClientReply::Events(sse_data(&out)),
			Err(reply) => ClientReply::Failed {
				detail: format!("{} {}", reply.status, String::from_utf8_lossy(&reply.body)),
			},
		},
		usage: ir_usage(usage),
	}
}

const fn reply_context(client: Client) -> ReplyContext<'static> {
	ReplyContext {
		client: client.protocol(),
		upstream: UpstreamProtocol::ChatCompletions,
		scope: REPLY_SCOPE,
		routes: &[],
		show_reasoning: false,
		include_usage: false,
		created: 1,
		fallback_reply_id: "reply-1",
		stop_sequences: Vec::new(),
		max_held_bytes: usize::MAX,
	}
}

const REPLY_SCOPE: StateScope<'static> = StateScope {
	backend: "golden",
	provider: "openai",
	model: "input-model",
};

/// What the client reads, apart from how its format splits it: the answer's text, each
/// reasoning text, each call with parsed arguments, how the reply ended, and the usage
/// accounting read. A zero cached or reasoning count that one side writes and the other leaves
/// out is the same count.
fn readable(reply: &Reply, client: Client) -> Value {
	let mut usage = reply.usage.clone();
	for field in ["cached_tokens", "reasoning_tokens"] {
		if usage[field] == json!(0) {
			usage[field] = Value::Null;
		}
	}
	let read = match (&reply.client, client) {
		(ClientReply::Failed { .. }, _) => json!({"failed": true}),
		(ClientReply::Body(body), Client::Messages) => messages_body(body),
		(ClientReply::Events(events), Client::Messages) => messages_events(events),
		(ClientReply::Body(body), Client::Responses) => responses_body(body),
		(ClientReply::Events(events), Client::Responses) => {
			assert_response_stream_ids(events);
			match events
				.iter()
				.rev()
				.find(|event| event.get("response").is_some())
			{
				Some(last) if last["type"] == "response.failed" => json!({"failed": true}),
				Some(last) => responses_body(&last["response"]),
				None => json!({"failed": true}),
			}
		},
	};
	json!({"reply": read, "usage": usage})
}

/// Item ids are generated independently, but every event must refer to the item
/// announced at its output index. Check before reducing a stream to its final body.
fn assert_response_stream_ids(events: &[Value]) {
	let mut response_id = None;
	let mut items = std::collections::BTreeMap::new();
	let mut ids = std::collections::BTreeSet::new();
	for event in events {
		if let Some(id) = event["response"].get("id") {
			assert!(id.as_str().is_some_and(|id| !id.is_empty()));
			assert_eq!(response_id.get_or_insert(id), &id, "response id changed");
		}
		if event["type"] == "response.output_item.added" {
			let index = event["output_index"].as_u64().unwrap();
			let item = &event["item"];
			let id = item["id"].as_str().unwrap();
			assert!(!id.is_empty());
			assert!(ids.insert(id), "duplicate item id");
			assert!(
				items.insert(index, item).is_none(),
				"duplicate output index"
			);
		}
		if let Some(id) = event.get("item_id") {
			let index = event["output_index"].as_u64().unwrap();
			assert_eq!(id, &items[&index]["id"], "event references another item");
		}
		if event["type"] == "response.output_item.done" {
			let index = event["output_index"].as_u64().unwrap();
			let added = items[&index];
			assert_eq!(event["item"]["id"], added["id"]);
			assert_eq!(event["item"]["call_id"], added["call_id"]);
		}
		if matches!(
			event["type"].as_str(),
			Some("response.completed" | "response.incomplete" | "response.failed")
		) {
			for (index, item) in event["response"]["output"]
				.as_array()
				.into_iter()
				.flatten()
				.enumerate()
			{
				let added = items[&(index as u64)];
				assert_eq!(item["id"], added["id"]);
				assert_eq!(item["call_id"], added["call_id"]);
			}
		}
	}
}

fn messages_body(body: &Value) -> Value {
	let mut read = Readable::default();
	for block in body["content"].as_array().unwrap() {
		match block["type"].as_str().unwrap() {
			"text" => read.text.push_str(block["text"].as_str().unwrap()),
			"thinking" => read.reasoning.push(block["thinking"].clone()),
			"tool_use" => read
				.calls
				.push(json!([block["id"], block["name"], block["input"]])),
			other => panic!("unexpected Messages block {other}"),
		}
	}
	read.ended = json!([body["stop_reason"], body["stop_sequence"]]);
	read.into_value()
}

fn messages_events(events: &[Value]) -> Value {
	let mut blocks: Vec<Value> = Vec::new();
	let mut ended = Value::Null;
	for event in events {
		match event["type"].as_str().unwrap() {
			"content_block_start" => {
				let mut block = event["content_block"].clone();
				if block["type"] == "tool_use" {
					block["input"] = json!("");
				}
				blocks.push(block);
			},
			"content_block_delta" => {
				let block = blocks.last_mut().unwrap();
				let delta = &event["delta"];
				let field = match delta["type"].as_str().unwrap() {
					"text_delta" => "text",
					"thinking_delta" => "thinking",
					"input_json_delta" => "input",
					_ => continue,
				};
				let source = if field == "input" {
					"partial_json"
				} else {
					field
				};
				let joined = format!(
					"{}{}",
					block[field].as_str().unwrap(),
					delta[source].as_str().unwrap()
				);
				block[field] = Value::String(joined);
			},
			"message_delta" => {
				ended = json!([
					event["delta"]["stop_reason"],
					event["delta"]["stop_sequence"]
				]);
			},
			"error" => return json!({"failed": true}),
			_ => {},
		}
	}
	for block in &mut blocks {
		if block["type"] == "tool_use" {
			let input = block["input"].as_str().unwrap();
			block["input"] = if input.is_empty() {
				json!({})
			} else {
				serde_json::from_str(input).unwrap()
			};
		}
	}
	let mut body = json!({"content": blocks, "stop_reason": ended[0], "stop_sequence": ended[1]});
	if ended.is_null() {
		body["stop_reason"] = json!("unfinished");
	}
	messages_body(&body)
}

fn responses_body(response: &Value) -> Value {
	let mut read = Readable::default();
	for item in response["output"].as_array().unwrap() {
		match item["type"].as_str().unwrap() {
			"message" => {
				for part in item["content"].as_array().unwrap() {
					match part["type"].as_str().unwrap() {
						"output_text" => read.text.push_str(part["text"].as_str().unwrap()),
						"refusal" => read.text.push_str(part["refusal"].as_str().unwrap()),
						other => panic!("unexpected Responses part {other}"),
					}
				}
			},
			"reasoning" => {
				let texts: Vec<&str> = ["summary", "content"]
					.iter()
					.flat_map(|field| item[*field].as_array().into_iter().flatten())
					.filter_map(|part| part["text"].as_str())
					.collect();
				read.reasoning.push(json!(texts.concat()));
			},
			"function_call" => {
				// Legacy can write cut-off arguments, which are kept as text.
				let text = item["arguments"].as_str().unwrap();
				let arguments = serde_json::from_str(text).unwrap_or_else(|_| json!(text));
				read
					.calls
					.push(json!([item["call_id"], item["name"], arguments]));
			},
			other => panic!("unexpected Responses output {other}"),
		}
	}
	read.ended = json!([response["status"], response["incomplete_details"]["reason"]]);
	read.into_value()
}

#[derive(Default)]
struct Readable {
	text: String,
	reasoning: Vec<Value>,
	calls: Vec<Value>,
	ended: Value,
}

impl Readable {
	fn into_value(self) -> Value {
		json!({
			"text": self.text,
			"reasoning": self.reasoning,
			"calls": self.calls,
			"ended": self.ended,
		})
	}
}

#[test]
fn normalization_preserves_output_limits_and_user_null_content() {
	for (request, changed) in [
		(
			json!({"max_completion_tokens": DEFAULT_MAX_TOKENS}),
			json!({}),
		),
		(
			json!({"messages": [{"role": "user", "content": null}]}),
			json!({"messages": [{"role": "user"}]}),
		),
	] {
		assert_ne!(
			without_representation(&request),
			without_representation(&changed)
		);
	}
}

#[test]
fn response_stream_references_are_checked_before_folding() {
	let events = vec![
		json!({"type": "response.output_item.added", "output_index": 0,
			"item": {"type": "message", "id": "msg_1"}}),
		json!({"type": "response.output_text.delta", "output_index": 0,
			"item_id": "wrong", "delta": "hello"}),
		json!({"type": "response.completed", "response": {"status": "completed", "output": []}}),
	];
	let reply = Reply {
		client: ClientReply::Events(events),
		usage: Value::Null,
	};
	assert!(std::panic::catch_unwind(|| readable(&reply, Client::Responses)).is_err());
}
