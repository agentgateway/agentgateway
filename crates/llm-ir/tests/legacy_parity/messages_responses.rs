//! Messages and Responses clients. Messages requests for a Responses provider, Responses requests
//! for a Messages provider, both for a Gemini provider, and Gemini client requests for a
//! Messages or Responses provider. Responses replies are written back for a Messages client, and
//! Gemini replies for both. Each IR result is snapshotted beside the legacy snapshots as
//! `<case>.llm-ir-<format>.snap`, named for the format the IR wrote.

use std::fs;
use std::num::NonZeroU64;

use agent_llm::conversion::responses::from_messages;
use agent_llm::types::messages;
use agent_llm::{InputFormat, LogContentFields};
use agent_llm_ir::ir::events::UsageFold;
use agent_llm_ir::{ClientProtocol, ReplyContext, StateScope, Upstream, UpstreamProtocol, Usage};
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::{Value, json};

use super::Disposition::*;
use super::{
	Disposition, FixedIdentifier, IrRequest, assert_every_input_listed, assert_reply_refusal,
	assert_request_refusal, describe, drive_buffered_reply, drive_streamed_reply, ir_request,
	ir_usage, legacy_stream_usage, legacy_usage, model_of, reply_input, reply_snapshot, snapshot,
	snapshot_beside_input, sse_data,
};

const MESSAGES: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	(
		"cache_control",
		Changed(
			"a mark on a part inside a tool result lands on the result's last part, where legacy marks that part; marks Responses has no place for are dropped and recorded",
		),
	),
	("cache_control_dropped_blocks", OutsideRoute),
	("cache_control_dropped_server_tools", OutsideRoute),
	("cache_control_reasoning_prefix", OutsideRoute),
	("cache_control_responses", Equivalent),
	(
		"cache_control_unsupported",
		Changed("system blocks are joined with a blank line, where legacy joins them with one newline"),
	),
	("citations", Equivalent),
	("document_inline", OutsideRoute),
	("gpt_adaptive_thinking_with_tools", OutsideRoute),
	(
		"metadata",
		Unsupported("`metadata` fields other than `user_id`"),
	),
	(
		"reasoning",
		Unsupported("`output_format` and summarized thinking display"),
	),
	(
		"reasoning_enabled",
		Unsupported("`thinking.type` `enabled` with a token budget"),
	),
	(
		"reasoning_replay",
		Unsupported("redacted thinking, and thinking signed by another provider, which legacy drops"),
	),
	(
		"reasoning_unsupported_model",
		Unsupported(
			"adaptive thinking without an effort, even on a model that takes none, where legacy sends no reasoning",
		),
	),
	(
		"responses_agent_subset",
		Unsupported("image URLs, file references and extra `metadata` fields"),
	),
	(
		"server_tools",
		Unsupported("server tools such as web search"),
	),
	("stop_sequences", Equivalent),
	(
		"structured-output",
		Changed(
			"structured output leaves schema strictness to the provider default, where legacy explicitly disables it",
		),
	),
	("system_message", Equivalent),
	("system_message_mid_conversation", OutsideRoute),
	("tool_history_without_tools", Equivalent),
	(
		"tool_reference",
		Changed(
			"a `tool_reference` block becomes text naming the loaded tools, where legacy sends the bare name",
		),
	),
	(
		"tool_result_empty",
		LegacyBug("Messages allows a `tool_result` without `content`; legacy refuses the request"),
	),
	(
		"tool_result_error",
		Changed(
			"a failed tool result is marked in its output text, where legacy sets its status to incomplete",
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

const DIRECTORY: &str = "src/tests/requests/messages";

#[test]
fn every_messages_request_golden_has_a_disposition() {
	assert_every_input_listed(DIRECTORY, MESSAGES.iter().map(|(name, _)| *name));
}

#[test]
fn messages_requests_on_a_responses_provider() {
	for (name, disposition) in MESSAGES {
		if matches!(disposition, OutsideRoute) {
			continue;
		}
		let body =
			fs::read_to_string(super::legacy_fixture(DIRECTORY).join(format!("{name}.json"))).unwrap();
		let legacy = legacy_request(&body);
		let ir = ir_request(&body, ClientProtocol::Messages, responses_upstream(&body));
		snapshot(
			DIRECTORY,
			&format!("{name}.llm-ir-responses"),
			&describe(*disposition),
			&body,
			&ir,
		);
		match (disposition, &legacy, &ir) {
			(Equivalent, Ok(legacy), Ok(ir)) => assert_eq!(
				expected_responses_request(legacy),
				without_representation(&ir.request),
				"{name}"
			),
			(Changed(_), Ok(legacy), Ok(ir)) => assert_messages_request_change(name, legacy, ir),
			(LegacyBug(_), Err(error), Ok(ir)) => {
				assert_eq!(*name, "tool_result_empty");
				assert!(error.contains("missing field `content`"), "{error}");
				let mut explicit_empty: Value = serde_json::from_str(&body).unwrap();
				let result = &mut explicit_empty["messages"][2]["content"][0];
				assert_eq!(result["type"], "tool_result");
				assert!(result.get("content").is_none());
				result["content"] = json!("");
				let legacy = legacy_request(&explicit_empty.to_string()).unwrap();
				assert_eq!(
					without_representation(&ir.request),
					expected_responses_request(&legacy),
					"{name}: missing content must behave like explicit empty content"
				);
				assert!(ir.dropped.is_empty());
				assert!(ir.emulated_stop_sequences.is_empty());
			},
			(Unsupported(_), Ok(_), Err(error)) => {
				assert_request_refusal(
					name,
					ClientProtocol::Messages,
					responses_upstream(&body).protocol,
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
}

fn assert_messages_request_change(name: &str, legacy: &Value, ir: &IrRequest) {
	let mut expected = expected_responses_request(legacy);
	let expected_dropped: &[&str] = match name {
		"cache_control" => {
			assert_eq!(expected["input"][10]["call_id"], "call_mixed");
			let output = &mut expected["input"][10]["output"];
			let mark = output[1]
				.as_object_mut()
				.unwrap()
				.remove("prompt_cache_breakpoint")
				.unwrap();
			assert_eq!(mark, json!({"mode": "explicit"}));
			assert!(output[2].get("prompt_cache_breakpoint").is_none());
			output[2]["prompt_cache_breakpoint"] = mark;
			&["cache_control"]
		},
		"cache_control_unsupported" => {
			assert_eq!(
				expected["instructions"],
				"Stable instructions\nCached instructions"
			);
			expected["instructions"] = json!("Stable instructions\n\nCached instructions");
			&["cache_control"]
		},
		"structured-output" => {
			let strict = expected["text"]["format"]
				.as_object_mut()
				.unwrap()
				.remove("strict");
			assert_eq!(strict, Some(json!(false)));
			&["top_k"]
		},
		"tool_reference" => {
			let text = &mut expected["input"][2]["output"][0]["text"];
			assert_eq!(*text, "mcp__example__list_widgets");
			*text = json!("Loaded tools: mcp__example__list_widgets.");
			&[]
		},
		"tool_result_error" => {
			let output = &mut expected["input"][2];
			assert_eq!(output["output"], "Error: location not found");
			assert_eq!(
				output.as_object_mut().unwrap().remove("status"),
				Some(json!("incomplete"))
			);
			output["output"] = json!("[tool error]\nError: location not found");
			&[]
		},
		_ => panic!("{name}: intentional change needs specific assertions"),
	};
	assert_eq!(
		without_representation(&ir.request),
		expected,
		"{name}: changes beyond those declared"
	);
	assert_eq!(ir.dropped, expected_dropped, "{name}: dropped fields");
	assert!(ir.emulated_stop_sequences.is_empty(), "{name}");
}

fn legacy_request(body: &str) -> Result<Value, String> {
	let request: messages::Request = serde_json::from_str(body).map_err(|err| err.to_string())?;
	let request = from_messages::translate(&request).map_err(|err| err.to_string())?;
	Ok(serde_json::from_slice(&request).unwrap())
}

fn responses_upstream(body: &str) -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &FixedIdentifier,
		scope: StateScope {
			backend: "golden",
			provider: "openai",
			model: model_of(body),
		},
		protocol: UpstreamProtocol::Responses {
			safety_identifier: true,
			reasoning_context: false,
		},
		default_max_tokens: None,
	}
}

fn messages_upstream(body: &str) -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &FixedIdentifier,
		scope: StateScope {
			backend: "golden",
			provider: "anthropic",
			model: model_of(body),
		},
		protocol: UpstreamProtocol::Messages,
		default_max_tokens: NonZeroU64::new(4096),
	}
}

/// Legacy uses provider-side storage by default. The IR must explicitly disable it and
/// request encrypted reasoning for replay. These policy fields stay in the comparison.
fn expected_responses_request(legacy: &Value) -> Value {
	assert!(legacy.get("store").is_none());
	assert!(legacy.get("include").is_none());
	let mut expected = legacy.clone();
	expected["store"] = json!(false);
	expected["include"] = json!(["reasoning.encrypted_content"]);
	let mut ids = std::collections::BTreeSet::new();
	for item in expected["input"].as_array_mut().unwrap() {
		if item["type"] == "message" && item["role"] == "assistant" {
			// Legacy invents an unreferenced id for assistant text. Calls and results
			// keep their ids and are compared exactly.
			let id = item.as_object_mut().unwrap().remove("id").unwrap();
			let id = id.as_str().unwrap();
			let suffix = id.strip_prefix("msg_").unwrap();
			assert_eq!(suffix.len(), 16);
			assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
			assert!(ids.insert(id.to_string()), "duplicate assistant id");
		}
	}
	assert!(
		!expected["input"]
			.as_array()
			.unwrap()
			.iter()
			.any(|item| item["type"] == "item_reference")
	);
	without_representation(&expected)
}

/// Only explicit protocol defaults and JSON argument formatting are equivalent here.
/// Nondefault values, ids, storage policy, and cache marks remain visible.
fn without_representation(request: &Value) -> Value {
	let mut request = request.clone();
	let object = request.as_object_mut().unwrap();
	if object.get("stream") == Some(&json!(false)) {
		object.remove("stream");
	}
	for item in object
		.get_mut("input")
		.and_then(Value::as_array_mut)
		.into_iter()
		.flatten()
	{
		let item = item.as_object_mut().unwrap();
		if item.get("status") == Some(&json!("completed")) {
			item.remove("status");
		}
		if item.get("type") == Some(&json!("function_call"))
			&& let Some(arguments) = item.get_mut("arguments")
			&& let Some(text) = arguments.as_str()
		{
			*arguments = serde_json::from_str(text).unwrap();
		}
		for key in ["content", "output"] {
			for part in item
				.get_mut(key)
				.and_then(Value::as_array_mut)
				.into_iter()
				.flatten()
			{
				let part = part.as_object_mut().unwrap();
				if part.get("type") == Some(&json!("output_text"))
					&& part.get("annotations") == Some(&json!([]))
				{
					part.remove("annotations");
				}
				if part.get("type") == Some(&json!("input_image"))
					&& part.get("detail") == Some(&json!("auto"))
				{
					part.remove("detail");
				}
			}
		}
	}
	request
}

/// Responses requests for a Messages provider. Legacy has no such conversion, so there is
/// nothing to compare with: each snapshot is reviewed on its own, and this list pins whether
/// the IR converts the request.
const RESPONSES: &[(&str, Outcome)] = &[
	(
		"assistant-history",
		Refuses("a `refusal` block in assistant history"),
	),
	("basic", Converts),
	(
		"cache_control",
		Refuses("`prompt_cache_breakpoint` on input parts, and `input_file` blocks"),
	),
	(
		"cache_control_tool_outputs",
		Refuses("`prompt_cache_breakpoint` inside tool outputs"),
	),
	("codex-assistant-history", Converts),
	("empty-message", Converts),
	(
		"input-list",
		Refuses("reasoning items without a carrier the gateway signed"),
	),
	("input-media", Refuses("`input_file` blocks")),
	(
		"instructions",
		Refuses("the `flex` service tier, which Messages doesn't have"),
	),
	("namespace-tools", Converts),
	("namespace-tools-long", Converts),
	(
		"parallel-tool-call",
		Refuses("call arguments that aren't a JSON object, and the output answering that call"),
	),
	("structured-output", Converts),
];

#[derive(Debug, Clone, Copy)]
enum Outcome {
	Converts,
	Refuses(&'static str),
}

use Outcome::*;

const RESPONSES_DIRECTORY: &str = "src/tests/requests/responses";

#[test]
fn every_responses_request_golden_has_an_outcome() {
	assert_every_input_listed(RESPONSES_DIRECTORY, RESPONSES.iter().map(|(name, _)| *name));
}

#[test]
fn responses_requests_on_a_messages_provider() {
	for (name, outcome) in RESPONSES {
		let body =
			fs::read_to_string(super::legacy_fixture(RESPONSES_DIRECTORY).join(format!("{name}.json")))
				.unwrap();
		let ir = ir_request(&body, ClientProtocol::Responses, messages_upstream(&body));
		let description = match outcome {
			Converts => "converts; legacy has no Responses-to-Messages conversion".to_string(),
			Refuses(what) => format!("refuses {what}"),
		};
		snapshot(
			RESPONSES_DIRECTORY,
			&format!("{name}.llm-ir-messages"),
			&description,
			&body,
			&ir,
		);
		match (outcome, &ir) {
			(Converts, Ok(_)) => {},
			(Refuses(_), Err(error)) => assert_request_refusal(
				name,
				ClientProtocol::Responses,
				UpstreamProtocol::Messages,
				error,
			),
			(outcome, ir) => panic!(
				"{name}: {outcome:?}, but the IR gave {:?}",
				ir.as_ref().map(|_| "a request")
			),
		}
	}
}

/// Replies from a Responses provider, written for a Messages client. Names starting with
/// `stream` are streamed.
const RESPONSES_REPLIES: &[(&str, Disposition)] = &[
	("basic", LegacyBug(SERVICE_TIER)),
	("content_filter", Equivalent),
	("context_window", Changed(CONTEXT_WINDOW)),
	("custom-tool", OutsideRoute),
	("empty_tool_call", Equivalent),
	(
		"failed",
		Changed("the IR records the usage a failed reply reports, where legacy records none"),
	),
	("guardrails", OutsideRoute),
	("incomplete_no_details", Changed(INCOMPLETE_WITHOUT_REASON)),
	("max_tokens", Equivalent),
	(
		"reasoning",
		Changed(
			"the reasoning item reaches the client as a thinking block whose signature carries it for replay, where legacy drops it; legacy also writes the `default` service tier, which Messages doesn't have",
		),
	),
	("stream", Equivalent),
	("stream-content_filter", Equivalent),
	("stream-context_window", Changed(CONTEXT_WINDOW)),
	("stream-custom-tool", OutsideRoute),
	(
		"stream-error",
		Changed(
			"a failure before any output is an HTTP error, as Anthropic sends it, where legacy starts the stream and then sends an `error` event",
		),
	),
	(
		"stream-failed",
		Changed(
			"a delta for an output item the provider never announced makes the reply unreadable, so it fails before any output, where legacy streams the text and then the provider's failure",
		),
	),
	(
		"stream-image",
		Unsupported("image generation output, which legacy drops before ending normally"),
	),
	(
		"stream-incomplete_no_details",
		Changed(INCOMPLETE_WITHOUT_REASON),
	),
	("stream-max_tokens", Equivalent),
	("stream-refusal", Equivalent),
	("stream-refusal-final-only", Equivalent),
	("tool", LegacyBug(SERVICE_TIER)),
	(
		"truncated_tool_call",
		LegacyBug(
			"legacy writes the cut-off arguments as a string `input`, which Messages doesn't allow, and records usage the provider didn't report as zero; the IR leaves out the incomplete call",
		),
	),
];

const SERVICE_TIER: &str = "legacy passes on OpenAI's `default` service tier, which Messages doesn't have; the IR writes `standard`";
const CONTEXT_WINDOW: &str = "running out of context window stops with Messages' `model_context_window_exceeded`, where legacy reports `max_tokens`";
const INCOMPLETE_WITHOUT_REASON: &str =
	"an incomplete reply without a reason fails, where legacy reports a normal stop";

const REPLIES: &str = "src/tests/response/responses";

#[test]
fn every_responses_reply_golden_has_a_disposition() {
	assert_every_input_listed(REPLIES, RESPONSES_REPLIES.iter().map(|(name, _)| *name));
}

#[tokio::test]
async fn responses_replies_for_a_messages_client() {
	for (name, disposition) in RESPONSES_REPLIES {
		if matches!(disposition, OutsideRoute) {
			continue;
		}
		let body = fs::read(super::legacy_fixture(REPLIES).join(format!("{name}.json"))).unwrap();
		let (legacy, ir) = if name.starts_with("stream") {
			(legacy_streamed_reply(&body).await, ir_streamed_reply(&body))
		} else {
			(legacy_buffered_reply(&body), ir_buffered_reply(&body))
		};
		let case = format!("{name}.llm-ir-messages");
		reply_snapshot(
			REPLIES,
			&case,
			*disposition,
			&body,
			&ir.client.report(),
			&ir.usage,
		);
		let (legacy, ir) = (
			without_reply_representation(legacy),
			without_reply_representation(ir),
		);
		match disposition {
			Equivalent => assert_eq!(legacy, ir, "{name}"),
			Changed(_) if *name == "failed" => {
				assert_eq!(
					legacy.client.report(),
					json!({"failed": "unsupported conversion: responses failed body cannot be represented by messages"})
				);
				assert_http_failure(&ir.client, 500, "api_error", "The provider failed.");
				assert_eq!(legacy.usage, Value::Null);
				assert_eq!(
					ir.usage,
					json!({
						"input_tokens": 100,
						"output_tokens": 5,
						"total_tokens": 105,
						"cached_tokens": 20,
						"cache_write_tokens": 30,
						"reasoning_tokens": 3,
					})
				);
			},
			Changed(_) | LegacyBug(_) => assert_messages_reply_change(name, &body, &legacy, &ir),
			Unsupported(_) => {
				assert_eq!(*name, "stream-image");
				assert!(ir.client.fails() && !legacy.client.fails());
				assert_reply_refusal(
					&body,
					reply_context(),
					true,
					"unsupported output item image_generation_call",
				);
				let ClientReply::Events(events) = &ir.client else {
					panic!("{name}: expected a stream error");
				};
				assert_eq!(events.last().unwrap()["type"], "error");
				assert_eq!(
					events
						.iter()
						.filter(|event| event["type"] == "error")
						.count(),
					1
				);
				assert!(!events.iter().any(|event| event["type"] == "message_stop"));
				assert_eq!(ir.usage, legacy.usage);
			},
			OutsideRoute => unreachable!("not converted"),
		}
	}
}

fn assert_messages_reply_change(
	name: &str,
	body: &[u8],
	legacy: &MessagesReply,
	ir: &MessagesReply,
) {
	let mut expected = legacy.client.report();
	let mut expected_usage = legacy.usage.clone();
	if name == "truncated_tool_call" {
		for field in ["input_tokens", "output_tokens", "total_tokens"] {
			assert_eq!(expected_usage[field], 0);
			expected_usage[field] = Value::Null;
		}
	}
	assert_eq!(
		ir.usage, expected_usage,
		"{name}: usage changes beyond those declared"
	);
	match name {
		"basic" | "tool" | "reasoning" => {
			assert_eq!(expected["body"]["usage"]["service_tier"], "default");
			expected["body"]["usage"]["service_tier"] = json!("standard");
			if name == "reasoning" {
				let thinking = assert_replayed_reasoning(body, &ir.client.report());
				expected["body"]["content"]
					.as_array_mut()
					.unwrap()
					.insert(0, thinking);
			}
		},
		"context_window" => {
			assert_eq!(expected["body"]["stop_reason"], "max_tokens");
			expected["body"]["stop_reason"] = json!("model_context_window_exceeded");
		},
		"stream-context_window" => {
			assert_eq!(expected["events"][1]["type"], "message_delta");
			assert_eq!(expected["events"][1]["delta"]["stop_reason"], "max_tokens");
			expected["events"][1]["delta"]["stop_reason"] = json!("model_context_window_exceeded");
		},
		"incomplete_no_details" | "stream-incomplete_no_details" => {
			if name == "incomplete_no_details" {
				assert_eq!(expected["body"]["stop_reason"], "max_tokens");
				assert_eq!(expected["body"]["content"], json!([]));
			} else {
				let events = expected["events"].as_array().unwrap();
				assert_eq!(
					events
						.iter()
						.map(|event| event["type"].as_str().unwrap())
						.collect::<Vec<_>>(),
					["message_start", "message_delta", "message_stop"]
				);
				assert_eq!(events[1]["delta"]["stop_reason"], "end_turn");
			}
			assert_http_failure(
				&ir.client,
				500,
				"api_error",
				"the provider stopped early with no usable reason",
			);
			return;
		},
		"stream-error" => {
			let events = expected["events"].as_array().unwrap();
			assert_eq!(
				events
					.iter()
					.map(|event| event["type"].as_str().unwrap())
					.collect::<Vec<_>>(),
				["message_start", "error"]
			);
			assert_eq!(
				events[1]["error"],
				json!({"type": "rate_limit_error", "message": "provider failed"})
			);
			assert_http_failure(
				&ir.client,
				429,
				"rate_limit_error",
				"The provider's rate limit was reached.",
			);
			return;
		},
		"stream-failed" => {
			let events = expected["events"].as_array().unwrap();
			assert_eq!(
				events
					.iter()
					.map(|event| event["type"].as_str().unwrap())
					.collect::<Vec<_>>(),
				[
					"message_start",
					"content_block_start",
					"content_block_delta",
					"content_block_stop",
					"content_block_start",
					"content_block_delta",
					"error"
				]
			);
			assert_eq!(events[2]["delta"]["text"], "Let me check.");
			assert_eq!(events[5]["delta"]["partial_json"], "{\"city\":");
			assert_eq!(
				events[6]["error"],
				json!({"type": "api_error", "message": "provider failed"})
			);
			assert_reply_refusal(
				body,
				reply_context(),
				true,
				"out of order: event for an item that is not open",
			);
			assert_http_failure(
				&ir.client,
				500,
				"api_error",
				"The provider's reply could not be read.",
			);
			return;
		},
		"truncated_tool_call" => {
			assert_eq!(
				expected["body"]["content"],
				json!([{"type": "tool_use", "id": "call_1", "name": "get_weather", "input": "{\"location\": \"Par"}])
			);
			expected["body"]["content"] = json!([]);
		},
		_ => panic!("{name}: intentional reply change needs specific assertions"),
	}
	assert_eq!(
		ir.client.report(),
		expected,
		"{name}: client changes beyond those declared"
	);
}

fn assert_replayed_reasoning(body: &[u8], reply: &Value) -> Value {
	let thinking = reply["body"]["content"][0].clone();
	assert_eq!(thinking["type"], "thinking");
	assert_eq!(thinking["thinking"], "");
	let request = json!({"model": REPLY_SCOPE.model, "max_tokens": 1, "messages": [
		{"role": "user", "content": "hello"},
		{"role": "assistant", "content": reply["body"]["content"]},
		{"role": "user", "content": "continue"},
	]})
	.to_string();
	let converted = ir_request(
		&request,
		ClientProtocol::Messages,
		responses_upstream(&request),
	)
	.unwrap();
	let replayed: Vec<_> = converted.request["input"]
		.as_array()
		.unwrap()
		.iter()
		.filter(|item| item["type"] == "reasoning")
		.collect();
	let provider: Value = serde_json::from_slice(body).unwrap();
	assert_eq!(
		replayed,
		vec![&provider["output"][0]],
		"the thinking signature must replay the provider's reasoning item"
	);
	thinking
}

fn assert_http_failure(reply: &ClientReply, status: u16, kind: &str, message: &str) {
	let ClientReply::Failed { detail } = reply else {
		panic!("expected an HTTP error before any output");
	};
	let (code, body) = detail.split_once(' ').unwrap();
	assert_eq!(code.parse::<u16>().unwrap(), status);
	assert_eq!(
		serde_json::from_str::<Value>(body).unwrap(),
		json!({"type": "error", "error": {"type": kind, "message": message}})
	);
}

/// What a Messages client gets, and the usage accounting read.
#[derive(Debug, PartialEq)]
struct MessagesReply {
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

	fn fails(&self) -> bool {
		match self {
			Self::Failed { .. } => true,
			Self::Events(events) => events.iter().any(|event| event["type"] == "error"),
			Self::Body(_) => false,
		}
	}
}

fn legacy_buffered_reply(body: &[u8]) -> MessagesReply {
	match from_messages::translate_response(&Bytes::copy_from_slice(body)) {
		Ok(reply) => MessagesReply {
			usage: legacy_usage(&reply.to_llm_response(LogContentFields::default())),
			client: ClientReply::Body(serde_json::from_slice(&reply.serialize().unwrap()).unwrap()),
		},
		Err(err) => MessagesReply {
			client: ClientReply::Failed {
				detail: err.to_string(),
			},
			usage: Value::Null,
		},
	}
}

async fn legacy_streamed_reply(body: &[u8]) -> MessagesReply {
	let (info, guard) = legacy_stream_usage(InputFormat::Messages);
	let output = from_messages::translate_stream(
		agent_http::Body::from(body.to_vec()),
		1024 * 1024,
		guard,
		LogContentFields::default(),
	)
	.collect()
	.await
	.unwrap()
	.to_bytes();
	let usage = legacy_usage(&info.lock().unwrap().response);
	MessagesReply {
		client: ClientReply::Events(sse_data(&output)),
		usage,
	}
}

fn ir_buffered_reply(body: &[u8]) -> MessagesReply {
	let (reply, usage) = drive_buffered_reply(body, reply_context());
	MessagesReply {
		client: match reply.status {
			200 => ClientReply::Body(serde_json::from_slice(&reply.body).unwrap()),
			status => ClientReply::Failed {
				detail: format!("{status} {}", String::from_utf8_lossy(&reply.body)),
			},
		},
		usage: ir_usage(usage),
	}
}

fn ir_streamed_reply(body: &[u8]) -> MessagesReply {
	let (reply, usage) = drive_streamed_reply(body, reply_context());
	MessagesReply {
		client: match reply {
			Ok(out) => ClientReply::Events(sse_data(&out)),
			Err(reply) => ClientReply::Failed {
				detail: format!("{} {}", reply.status, String::from_utf8_lossy(&reply.body)),
			},
		},
		usage: ir_usage(usage),
	}
}

const fn reply_context() -> ReplyContext<'static> {
	ReplyContext {
		client: ClientProtocol::Messages,
		upstream: UpstreamProtocol::Responses {
			safety_identifier: true,
			reasoning_context: false,
		},
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
	model: "gpt-5",
};

/// Leaves out what differs only in representation: deltas that a writer splits or joins
/// differently, and a zero cached-token count that one side writes and the other leaves out.
fn without_reply_representation(reply: MessagesReply) -> MessagesReply {
	let MessagesReply { client, mut usage } = reply;
	if usage["cached_tokens"] == json!(0) {
		usage["cached_tokens"] = Value::Null;
	}
	let client = match client {
		ClientReply::Body(mut body) => {
			without_zero_cache_reads(&mut body["usage"]);
			ClientReply::Body(body)
		},
		ClientReply::Events(events) => {
			let mut joined: Vec<Value> = Vec::new();
			for mut event in events {
				without_zero_cache_reads(&mut event["usage"]);
				without_zero_cache_reads(&mut event["message"]["usage"]);
				if let Some(previous) = joined.last_mut()
					&& event["type"] == "content_block_delta"
					&& previous["type"] == "content_block_delta"
					&& previous["index"] == event["index"]
					&& previous["delta"]["type"] == event["delta"]["type"]
				{
					let delta = event["delta"].as_object().unwrap();
					for (field, value) in delta.iter().filter(|(field, _)| *field != "type") {
						let joined = format!(
							"{}{}",
							previous["delta"][field].as_str().unwrap(),
							value.as_str().unwrap()
						);
						previous["delta"][field] = Value::String(joined);
					}
					continue;
				}
				joined.push(event);
			}
			ClientReply::Events(joined)
		},
		failed @ ClientReply::Failed { .. } => failed,
	};
	MessagesReply { client, usage }
}

fn without_zero_cache_reads(usage: &mut Value) {
	if let Some(usage) = usage.as_object_mut()
		&& usage.get("cache_read_input_tokens") == Some(&json!(0))
	{
		usage.remove("cache_read_input_tokens");
	}
}

// Gemini upstream.
//
// Legacy serves Messages and Responses clients on Gemini through Google's OpenAI-compatible
// Chat Completions endpoint: it converts each request to Chat Completions, and Google converts
// that to Gemini. The IR sends native `generateContent`. The bodies are in different formats, so
// each is reduced by `asks` to what it asks of the model, and those are compared. Legacy's
// goldens send only some inputs to that endpoint; the others are outside its route.

const GEMINI_MODEL: &str = "gemini-2.5-pro";

fn gemini_upstream() -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &FixedIdentifier,
		scope: StateScope {
			backend: "golden",
			provider: "gemini",
			model: GEMINI_MODEL,
		},
		protocol: UpstreamProtocol::Gemini,
		default_max_tokens: NonZeroU64::new(4096),
	}
}

const MESSAGES_ON_GEMINI: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	(
		"cache_control",
		LegacyBug(
			"legacy drops an image inside a tool result, which the IR sends with that result; a system-role message after the first turn also stays in its place as a reminder, where legacy moves it into the system prompt",
		),
	),
	("cache_control_dropped_blocks", OutsideRoute),
	("cache_control_dropped_server_tools", OutsideRoute),
	("cache_control_reasoning_prefix", OutsideRoute),
	("cache_control_responses", OutsideRoute),
	("cache_control_unsupported", Equivalent),
	("citations", Equivalent),
	("document_inline", OutsideRoute),
	("gpt_adaptive_thinking_with_tools", Equivalent),
	("metadata", Equivalent),
	("reasoning", Unsupported("the `output_format` field")),
	(
		"reasoning_enabled",
		Changed(
			"a thinking budget is dropped and recorded, leaving the model's default thinking, where legacy turns the budget into an effort",
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
		Unsupported("server tools such as web search, which legacy drops"),
	),
	("stop_sequences", Equivalent),
	(
		"structured-output",
		LegacyBug(
			"legacy rounds `temperature` and `top_p` through 32-bit floats, sending 0.699999988 for 0.7; the IR also drops `additionalProperties` from the reply schema and records it",
		),
	),
	(
		"system_message",
		Changed(
			"a system-role message after the first turn becomes a `<system-reminder>` in its place, where legacy moves it into the system prompt",
		),
	),
	("system_message_mid_conversation", OutsideRoute),
	("tool_history_without_tools", Equivalent),
	(
		"tool_reference",
		Changed(
			"a `tool_reference` block becomes text naming the loaded tools, where legacy sends the bare name",
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
		Unsupported("an image URL inside a tool result, which legacy drops"),
	),
	(
		"tool_result_unknown_part",
		Unsupported("unknown block types, which legacy drops"),
	),
	("tools", Changed(SCHEMA_KEYWORDS)),
	(
		"unknown_content",
		Unsupported("unknown block types, which legacy drops"),
	),
];

const SCHEMA_KEYWORDS: &str = "schema keywords Gemini doesn't take, such as `additionalProperties`, are dropped and recorded, where legacy leaves them to Google's endpoint";

#[test]
fn every_messages_request_golden_has_a_gemini_disposition() {
	assert_every_input_listed(DIRECTORY, MESSAGES_ON_GEMINI.iter().map(|(name, _)| *name));
}

#[test]
fn messages_requests_on_a_gemini_provider() {
	for (name, disposition) in MESSAGES_ON_GEMINI {
		if matches!(disposition, OutsideRoute) {
			continue;
		}
		let body =
			fs::read_to_string(super::legacy_fixture(DIRECTORY).join(format!("{name}.json"))).unwrap();
		let legacy = legacy_gemini_request(&body, ClientProtocol::Messages);
		let ir = ir_request(&body, ClientProtocol::Messages, gemini_upstream());
		snapshot(
			DIRECTORY,
			&format!("{name}.llm-ir-gemini"),
			&describe(*disposition),
			&body,
			&ir,
		);
		compare_on_gemini(
			name,
			&body,
			ClientProtocol::Messages,
			*disposition,
			&legacy,
			&ir,
		);
	}
}

const RESPONSES_ON_GEMINI: &[(&str, Disposition)] = &[
	("assistant-history", OutsideRoute),
	("basic", Equivalent),
	(
		"cache_control",
		Unsupported(
			"`prompt_cache_breakpoint` on input parts, and `input_file` blocks, which legacy drops",
		),
	),
	("cache_control_tool_outputs", OutsideRoute),
	(
		"codex-assistant-history",
		Changed(
			"the caller's default output limit is sent when the client sets none, where legacy sends no limit",
		),
	),
	("empty-message", OutsideRoute),
	(
		"input-list",
		Unsupported("reasoning items without a carrier the gateway signed, which legacy drops"),
	),
	("input-media", OutsideRoute),
	(
		"instructions",
		Unsupported("`service_tier`, which legacy drops"),
	),
	("namespace-tools", Changed(SCHEMA_KEYWORDS)),
	("namespace-tools-long", OutsideRoute),
	(
		"parallel-tool-call",
		Unsupported("call arguments that aren't a JSON object, which legacy passes on as text"),
	),
	("structured-output", OutsideRoute),
];

#[test]
fn every_responses_request_golden_has_a_gemini_disposition() {
	assert_every_input_listed(
		RESPONSES_DIRECTORY,
		RESPONSES_ON_GEMINI.iter().map(|(name, _)| *name),
	);
}

#[test]
fn responses_requests_on_a_gemini_provider() {
	for (name, disposition) in RESPONSES_ON_GEMINI {
		if matches!(disposition, OutsideRoute) {
			continue;
		}
		let body =
			fs::read_to_string(super::legacy_fixture(RESPONSES_DIRECTORY).join(format!("{name}.json")))
				.unwrap();
		let legacy = legacy_gemini_request(&body, ClientProtocol::Responses);
		let ir = ir_request(&body, ClientProtocol::Responses, gemini_upstream());
		snapshot(
			RESPONSES_DIRECTORY,
			&format!("{name}.llm-ir-gemini"),
			&describe(*disposition),
			&body,
			&ir,
		);
		compare_on_gemini(
			name,
			&body,
			ClientProtocol::Responses,
			*disposition,
			&legacy,
			&ir,
		);
	}
}

fn compare_on_gemini(
	name: &str,
	body: &str,
	client: ClientProtocol,
	disposition: Disposition,
	legacy: &Result<Value, String>,
	ir: &Result<IrRequest, String>,
) {
	match (disposition, legacy, ir) {
		(Equivalent, Ok(legacy), Ok(ir)) => {
			assert_eq!(completions_asks(legacy), gemini_asks(&ir.request), "{name}")
		},
		(Changed(_) | LegacyBug(_), Ok(legacy), Ok(ir)) => {
			assert_gemini_request_change(name, client, legacy, ir);
		},
		(LegacyBug(_), Err(error), Ok(ir)) => {
			assert_eq!(client, ClientProtocol::Messages);
			assert_eq!(name, "tool_result_empty");
			assert!(error.contains("missing field `content`"), "{error}");
			let mut explicit_empty: Value = serde_json::from_str(body).unwrap();
			let result = &mut explicit_empty["messages"][2]["content"][0];
			assert_eq!(result["type"], "tool_result");
			assert!(result.get("content").is_none());
			result["content"] = json!("");
			let legacy = legacy_gemini_request(&explicit_empty.to_string(), client).unwrap();
			assert_eq!(
				gemini_asks(&ir.request),
				completions_asks(&legacy),
				"{name}: missing content must behave like explicit empty content"
			);
			assert!(ir.dropped.is_empty(), "{name}");
			assert!(ir.emulated_stop_sequences.is_empty(), "{name}");
		},
		(Unsupported(_), Ok(_), Err(error)) => {
			assert_request_refusal(name, client, UpstreamProtocol::Gemini, error);
		},
		(disposition, legacy, ir) => panic!(
			"{name}: {disposition:?}, but legacy gave {:?} and the IR {:?}",
			legacy.as_ref().map(|_| "a request"),
			ir.as_ref().map(|_| "a request"),
		),
	}
}

fn assert_gemini_request_change(
	name: &str,
	client: ClientProtocol,
	legacy: &Value,
	ir: &IrRequest,
) {
	let mut expected = completions_asks(legacy);
	let expected_dropped: &[&str] = match (client, name) {
		(ClientProtocol::Messages, "cache_control") => {
			assert_eq!(
				expected["system"],
				"First system block\n\nSecond system block\n\nSystem-role block"
			);
			expected["system"] = json!("First system block\n\nSecond system block");
			let entries = expected["entries"].as_array_mut().unwrap();
			assert_eq!(entries.len(), 12);
			assert_eq!(entries[10], json!({"result": ["lookup", ""]}));
			assert_eq!(
				entries[11],
				json!({"result": ["lookup", "before image\nafter image"]})
			);
			entries.insert(11, json!({"image": ["user", "image/png", "aGVsbG8="]}));
			entries.push(json!({"image": ["user", "image/png", "aGVsbG8="]}));
			entries.push(
				json!({"text": ["user", "<system-reminder>\nSystem-role block\n</system-reminder>"]}),
			);
			&["cache_control"]
		},
		(ClientProtocol::Messages, "reasoning_enabled") => {
			assert_eq!(expected["thinks"], true);
			expected["thinks"] = json!(false);
			&["thinking.budget_tokens"]
		},
		(ClientProtocol::Messages, "structured-output") => {
			assert_eq!(expected["temperature"], json!(f64::from(0.7f32)));
			assert_eq!(expected["top_p"], json!(f64::from(0.8f32)));
			expected["temperature"] = json!(0.7);
			expected["top_p"] = json!(0.8);
			let keyword = expected["reply_schema"]
				.as_object_mut()
				.unwrap()
				.remove("additionalProperties");
			assert_eq!(keyword, Some(json!(false)));
			&["tool schema keyword"]
		},
		(ClientProtocol::Messages, "system_message") => {
			assert_eq!(
				expected["system"],
				"The following skills are available for use."
			);
			expected["system"] = Value::Null;
			expected["entries"].as_array_mut().unwrap().push(json!({"text": ["user", "<system-reminder>\nThe following skills are available for use.\n</system-reminder>"]}));
			&["cache_control"]
		},
		(ClientProtocol::Messages, "tool_reference") => {
			let result = &mut expected["entries"][2]["result"];
			assert_eq!(*result, json!(["ToolSearch", "mcp__example__list_widgets"]));
			result[1] = json!("Loaded tools: mcp__example__list_widgets.");
			&["cache_control"]
		},
		(ClientProtocol::Messages, "tool_result_error") => {
			let result = &mut expected["entries"][2]["result"];
			assert_eq!(*result, json!(["get_weather", "Error: location not found"]));
			result[1] = json!("[tool error]\nError: location not found");
			&[]
		},
		(ClientProtocol::Messages, "tools") | (ClientProtocol::Responses, "namespace-tools") => {
			let schemas: &[(usize, &str)] = match client {
				ClientProtocol::Messages => &[(1, "get_time"), (2, "get_forecast")],
				ClientProtocol::Responses => &[
					(0, "mcp__node_repl__js"),
					(1, "other_kernel__js"),
					(2, "finish"),
				],
				_ => unreachable!("only Messages and Responses reach this case"),
			};
			for &(index, tool) in schemas {
				assert_eq!(expected["tools"][index][0], tool);
				let keyword = expected["tools"][index][2]
					.as_object_mut()
					.unwrap()
					.remove("additionalProperties");
				assert_eq!(keyword, Some(json!(false)), "{name}: {tool}");
			}
			&["tool strict", "tool schema keyword"]
		},
		(ClientProtocol::Responses, "codex-assistant-history") => {
			assert_eq!(expected["max_tokens"], Value::Null);
			expected["max_tokens"] = json!(4096);
			&["input item id and status"]
		},
		_ => panic!("{name}: intentional change needs specific assertions for {client:?}"),
	};
	assert_eq!(
		gemini_asks(&ir.request),
		expected,
		"{name}: changes beyond those declared"
	);
	assert_eq!(ir.dropped, expected_dropped, "{name}: dropped fields");
	assert!(ir.emulated_stop_sequences.is_empty(), "{name}");
}

fn legacy_gemini_request(body: &str, client: ClientProtocol) -> Result<Value, String> {
	match client {
		ClientProtocol::Messages => {
			let request: messages::Request = serde_json::from_str(body).map_err(|err| err.to_string())?;
			let request = agent_llm::conversion::completions::from_messages::translate_request(&request)
				.map_err(|err| err.to_string())?;
			Ok(serde_json::to_value(request).unwrap())
		},
		ClientProtocol::Responses => {
			let request: agent_llm::types::responses::Request =
				serde_json::from_str(body).map_err(|err| err.to_string())?;
			let translated =
				agent_llm::conversion::openai_compat::from_responses::translate_request(&request)
					.map_err(|err| err.to_string())?;
			Ok(serde_json::to_value(translated.request).unwrap())
		},
		_ => unreachable!("only Messages and Responses have legacy Gemini comparisons"),
	}
}

/// A Chat Completions request reduced to what it asks of the model: system text, an ordered
/// list of texts, images, calls and results, the tools it offers, and the settings both
/// formats have. A result is named by its call, as Gemini names it. Whether the model is
/// asked to think is kept, but not how hard, which each side maps to Gemini on its own.
fn completions_asks(request: &Value) -> Value {
	let mut system = Vec::new();
	let mut entries = Vec::new();
	let mut calls = std::collections::HashMap::new();
	for message in request["messages"].as_array().unwrap() {
		let role = message["role"].as_str().unwrap();
		match role {
			"system" | "developer" => system.extend(completions_texts(&message["content"])),
			"user" | "assistant" => {
				for part in completions_parts(&message["content"]) {
					entries.push(match part {
						Part::Text(text) => json!({"text": [role, text]}),
						Part::Image(media_type, data) => json!({"image": [role, media_type, data]}),
					});
				}
				for call in message["tool_calls"].as_array().into_iter().flatten() {
					let function = &call["function"];
					let text = function["arguments"].as_str().unwrap();
					let arguments: Value =
						serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()));
					calls.insert(call["id"].clone(), function["name"].clone());
					entries.push(json!({"call": [function["name"], arguments]}));
				}
			},
			"tool" => entries.push(json!({"result": [
				calls[&message["tool_call_id"]],
				completions_texts(&message["content"]).join("\n"),
			]})),
			other => panic!("unexpected Chat Completions role {other}"),
		}
	}
	let tools: Vec<Value> = request["tools"]
		.as_array()
		.into_iter()
		.flatten()
		.map(|tool| {
			let function = &tool["function"];
			json!([
				function["name"],
				function["description"],
				function["parameters"]
			])
		})
		.collect();
	let tool_choice = match &request["tool_choice"] {
		Value::Null => Value::Null,
		Value::String(mode) => json!(mode),
		choice => json!(["required", choice["function"]["name"]]),
	};
	let stop = match &request["stop"] {
		Value::String(stop) => json!([stop]),
		stop => stop.clone(),
	};
	json!({
		"system": (!system.is_empty()).then(|| system.join("\n\n")),
		"entries": entries,
		"tools": tools,
		"tool_choice": tool_choice,
		"max_tokens": request.get("max_completion_tokens").or_else(|| request.get("max_tokens")),
		"temperature": request["temperature"],
		"top_p": request["top_p"],
		"stop": stop,
		"reply_schema": request["response_format"]["json_schema"]["schema"],
		"thinks": request["reasoning_effort"].as_str().is_some_and(|effort| effort != "none"),
	})
}

enum Part {
	Text(String),
	Image(String, String),
}

fn completions_parts(content: &Value) -> Vec<Part> {
	match content {
		Value::Null => Vec::new(),
		Value::String(text) => vec![Part::Text(text.clone())],
		Value::Array(parts) => parts
			.iter()
			.map(|part| match part["type"].as_str().unwrap() {
				"text" => Part::Text(part["text"].as_str().unwrap().to_string()),
				"image_url" => {
					let url = part["image_url"]["url"].as_str().unwrap();
					let (media_type, data) = url
						.strip_prefix("data:")
						.and_then(|url| url.split_once(";base64,"))
						.unwrap_or(("url", url));
					Part::Image(media_type.to_string(), data.to_string())
				},
				other => panic!("unexpected Chat Completions part {other}"),
			})
			.collect(),
		other => panic!("unexpected Chat Completions content {other}"),
	}
}

fn completions_texts(content: &Value) -> Vec<String> {
	completions_parts(content)
		.into_iter()
		.filter_map(|part| match part {
			Part::Text(text) => Some(text),
			Part::Image(..) => None,
		})
		.collect()
}

/// `completions_asks` for a Gemini request. Thought parts carry reasoning to replay, which
/// Chat Completions has no place for, so they are left out.
fn gemini_asks(request: &Value) -> Value {
	let mut entries = Vec::new();
	for content in request["contents"].as_array().unwrap() {
		let role = match content["role"].as_str().unwrap() {
			"user" => "user",
			"model" => "assistant",
			other => panic!("unexpected Gemini role {other}"),
		};
		for part in content["parts"].as_array().unwrap() {
			if part["thought"] == json!(true) {
				continue;
			}
			if let Some(text) = part.get("text") {
				entries.push(json!({"text": [role, text]}));
			} else if let Some(image) = part.get("inlineData") {
				entries.push(json!({"image": [role, image["mimeType"], image["data"]]}));
			} else if let Some(call) = part.get("functionCall") {
				entries.push(json!({"call": [call["name"], call["args"]]}));
			} else if let Some(result) = part.get("functionResponse") {
				entries.push(json!({"result": [result["name"], result["response"]["content"]]}));
				for image in result["parts"].as_array().into_iter().flatten() {
					let image = &image["inlineData"];
					entries.push(json!({"image": ["user", image["mimeType"], image["data"]]}));
				}
			} else {
				panic!("unexpected Gemini part {part}");
			}
		}
	}
	let tools: Vec<Value> = request["tools"]
		.as_array()
		.into_iter()
		.flatten()
		.flat_map(|tool| tool["functionDeclarations"].as_array().unwrap())
		.map(|tool| json!([tool["name"], tool["description"], tool["parameters"]]))
		.collect();
	let calling = &request["toolConfig"]["functionCallingConfig"];
	let tool_choice = match (calling["mode"].as_str(), &calling["allowedFunctionNames"]) {
		(None, _) => Value::Null,
		(Some("ANY"), Value::Array(names)) => json!(["required", names[0]]),
		(Some("ANY"), _) => json!("required"),
		(Some(mode), _) => json!(mode.to_lowercase()),
	};
	let config = &request["generationConfig"];
	let thinking = &config["thinkingConfig"];
	json!({
		"system": request["systemInstruction"]["parts"][0]["text"],
		"entries": entries,
		"tools": tools,
		"tool_choice": tool_choice,
		"max_tokens": config.get("maxOutputTokens"),
		"temperature": config["temperature"],
		"top_p": config["topP"],
		"stop": config["stopSequences"],
		"reply_schema": config["responseSchema"],
		"thinks": thinking.get("thinkingLevel").is_some()
			|| thinking["thinkingBudget"].as_u64().is_some_and(|budget| budget > 128),
	})
}

/// Gemini replies, written for a Messages client and for a Responses client. Legacy never reads
/// a native Gemini reply for these clients, because Google's compatible endpoint converts it
/// first; legacy's native Gemini passthrough does read its usage, and that is compared. Names
/// starting with `stream` are streamed.
const GEMINI_REPLIES: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	(
		"blocked",
		LegacyBug(
			"legacy records the output tokens Gemini didn't report as zero; the IR leaves them unknown",
		),
	),
	(
		"guardrails",
		Unsupported(
			"code execution, server tool calls, function responses and media in a reply, which the client can't be given",
		),
	),
	("reasoning", Equivalent),
	("stream_tool", Equivalent),
	("tool", Equivalent),
];

const GEMINI_REPLIES_DIRECTORY: &str = "src/tests/response/vertex-gemini";

const GEMINI_SCOPE: StateScope<'static> = StateScope {
	backend: "golden",
	provider: "gemini",
	model: GEMINI_MODEL,
};

#[test]
fn every_gemini_reply_golden_has_a_disposition() {
	assert_every_input_listed(
		GEMINI_REPLIES_DIRECTORY,
		GEMINI_REPLIES.iter().map(|(name, _)| *name),
	);
}

#[test]
fn gemini_replies_for_messages_and_responses_clients() {
	use agent_llm::types::ResponseType;

	for (name, disposition) in GEMINI_REPLIES {
		let body =
			fs::read(super::legacy_fixture(GEMINI_REPLIES_DIRECTORY).join(format!("{name}.json")))
				.unwrap();
		let streamed = name.starts_with("stream");
		let payloads = if streamed {
			sse_data(&body)
		} else {
			vec![serde_json::from_slice(&body).unwrap()]
		};
		let mut legacy = UsageFold::default();
		for payload in payloads {
			let reply: agent_llm::types::gemini::Response = serde_json::from_value(payload).unwrap();
			let usage = reply.to_llm_response(LogContentFields::default());
			legacy.merge(Usage {
				input_tokens: usage.input_tokens,
				output_tokens: usage.output_tokens,
				total_tokens: usage.total_tokens,
				cached_tokens: usage.cached_input_tokens,
				reasoning_tokens: usage.reasoning_tokens,
				..Usage::default()
			});
		}

		let convert = |client| {
			let context = ReplyContext {
				client,
				upstream: UpstreamProtocol::Gemini,
				scope: GEMINI_SCOPE,
				routes: &[],
				show_reasoning: false,
				include_usage: false,
				created: 1,
				fallback_reply_id: "reply-1",
				stop_sequences: Vec::new(),
				max_held_bytes: usize::MAX,
			};
			if streamed {
				let (reply, usage) = drive_streamed_reply(&body, context);
				let report = match reply {
					Ok(out) => json!({"events": sse_data(&out)}),
					Err(reply) => json!({"failed": reply.status}),
				};
				(report, usage)
			} else {
				let (reply, usage) = drive_buffered_reply(&body, context);
				let report = match reply.status {
					200 => json!({"body": serde_json::from_slice::<Value>(&reply.body).unwrap()}),
					status => json!({"failed": status}),
				};
				(report, usage)
			}
		};
		let (messages, messages_usage) = convert(ClientProtocol::Messages);
		let (responses, usage) = convert(ClientProtocol::Responses);
		assert_eq!(
			messages_usage, usage,
			"{name}: clients must record the same provider usage"
		);
		let failed = messages.get("failed").is_some() || responses.get("failed").is_some();
		let report = json!({
			"disposition": describe(*disposition),
			"messages": messages,
			"responses": responses,
			"usage": ir_usage(usage),
		});
		let case = format!("{name}.llm-ir");
		snapshot_beside_input(
			GEMINI_REPLIES_DIRECTORY,
			&case,
			&reply_input(&body),
			&report,
		);
		match disposition {
			Equivalent => {
				assert!(!failed, "{name}: a client reply failed");
				assert_eq!(ir_usage(usage), ir_usage(legacy.total()), "{name}");
			},
			LegacyBug(_) | Changed(_) => {
				assert!(!failed, "{name}: a client reply failed");
				assert_eq!(
					*name, "blocked",
					"intentional usage change needs specific assertions"
				);
				let mut expected = legacy.total();
				assert_eq!(expected.output_tokens, Some(0));
				expected.output_tokens = None;
				assert_eq!(
					usage, expected,
					"{name}: only output tokens change from zero to unknown"
				);
			},
			Unsupported(_) => {
				assert_eq!(*name, "guardrails");
				assert_eq!(messages, json!({"failed": 500}));
				assert_eq!(responses, json!({"failed": 500}));
				for client in [ClientProtocol::Messages, ClientProtocol::Responses] {
					let context = ReplyContext {
						client,
						upstream: UpstreamProtocol::Gemini,
						scope: GEMINI_SCOPE,
						..reply_context()
					};
					assert_reply_refusal(&body, context, streamed, "unsupported output item part");
				}
			},
			OutsideRoute => unreachable!("{name}: not converted"),
		}
	}
}

/// Gemini client requests, on a Messages and on a Responses provider. Legacy only
/// passes a Gemini client's request on to Gemini, so there is nothing to compare.
const GEMINI_CLIENT_REQUESTS: &[(&str, Outcome)] = &[
	(
		"image-inline",
		Refuses("a file reference and its `videoMetadata`"),
	),
	(
		"passthrough-fields",
		Refuses("fields the core doesn't know, such as `modelArmorConfig`"),
	),
	("structured-output", Converts),
	("text", Refuses("`seed` and `cachedContent`")),
	(
		"thinking",
		Refuses("thought signatures Gemini wrote, which only Gemini can read"),
	),
	(
		"tools",
		Refuses("Google's own tools, and a thought signature Gemini wrote"),
	),
];

const GEMINI_CLIENT_DIRECTORY: &str = "src/tests/requests/gemini";
const POLICIES_DIRECTORY: &str = "src/tests/requests/policies";

#[test]
fn every_gemini_client_request_golden_has_an_outcome() {
	assert_every_input_listed(
		GEMINI_CLIENT_DIRECTORY,
		GEMINI_CLIENT_REQUESTS.iter().map(|(name, _)| *name),
	);
}

#[test]
fn gemini_client_requests_on_messages_and_responses_providers() {
	let cases = GEMINI_CLIENT_REQUESTS
		.iter()
		.map(|(name, outcome)| (GEMINI_CLIENT_DIRECTORY, *name, *outcome))
		.chain([(POLICIES_DIRECTORY, "gemini_with_system", Converts)]);
	let mut mismatches = Vec::new();
	for (directory, name, outcome) in cases {
		let body =
			fs::read_to_string(super::legacy_fixture(directory).join(format!("{name}.json"))).unwrap();
		let upstreams = [
			(
				"messages",
				gemini_client_upstream("anthropic", UpstreamProtocol::Messages),
			),
			(
				"responses",
				gemini_client_upstream(
					"openai",
					UpstreamProtocol::Responses {
						safety_identifier: true,
						reasoning_context: false,
					},
				),
			),
		];
		for (target, upstream) in upstreams {
			let target_protocol = upstream.protocol;
			let ir = ir_request(&body, ClientProtocol::Gemini, upstream);
			let description = match outcome {
				Converts => "converts; legacy only passes a Gemini request on to Gemini".to_string(),
				Refuses(what) => format!("refuses {what}"),
			};
			snapshot(
				directory,
				&format!("{name}.llm-ir-{target}"),
				&description,
				&body,
				&ir,
			);
			match (outcome, &ir) {
				(Converts, Ok(_)) => {},
				(Refuses(_), Err(error)) => {
					assert_request_refusal(name, ClientProtocol::Gemini, target_protocol, error)
				},
				(outcome, ir) => mismatches.push(format!(
					"{name} on {target}: {outcome:?}, but the IR gave {:?}",
					ir.as_ref().map(|_| "a request")
				)),
			}
		}
	}
	assert_eq!(mismatches, Vec::<String>::new());
}

/// A Gemini client's model is in its URL; the caller maps it to this one.
fn gemini_client_upstream(provider: &'static str, protocol: UpstreamProtocol) -> Upstream<'static> {
	Upstream {
		identifier_pseudonymizer: &FixedIdentifier,
		scope: StateScope {
			backend: "golden",
			provider,
			model: "golden-model",
		},
		protocol,
		default_max_tokens: NonZeroU64::new(4096),
	}
}

#[test]
fn normalization_preserves_storage_policy_and_message_ids() {
	for request in [
		json!({"store": false, "include": ["reasoning.encrypted_content"]}),
		json!({"input": [{"type": "message", "role": "assistant", "id": "msg_1"}]}),
	] {
		let mut changed = request.clone();
		if changed.get("store").is_some() {
			changed.as_object_mut().unwrap().remove("store");
			changed.as_object_mut().unwrap().remove("include");
		} else {
			changed["input"][0].as_object_mut().unwrap().remove("id");
		}
		assert_ne!(
			without_representation(&request),
			without_representation(&changed)
		);
	}
}
