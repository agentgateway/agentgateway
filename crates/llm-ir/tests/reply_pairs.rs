//! Public reply drivers across all converted protocol pairs: buffered replies,
//! streams and provider HTTP errors use the client's format.
//! Same-protocol replies are observed for usage while forwarded unchanged.

#![allow(clippy::unwrap_used)]

use agent_llm_ir::{
	BufferedReply, ClientProtocol, ObservedReply, ReplyContext, StateScope, StreamReply,
	UpstreamProtocol, error_reply,
};
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

const MODEL: &str = "test-model";
const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_HELD_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Copy)]
enum Provider {
	Messages,
	Responses,
	Chat,
	Gemini,
}

#[derive(Debug, Clone, Copy)]
enum Client {
	Messages,
	Responses,
	Chat,
	Gemini,
}

#[rstest]
#[case::messages_to_responses(Provider::Messages, Client::Responses)]
#[case::messages_to_chat(Provider::Messages, Client::Chat)]
#[case::messages_to_gemini(Provider::Messages, Client::Gemini)]
#[case::responses_to_messages(Provider::Responses, Client::Messages)]
#[case::responses_to_chat(Provider::Responses, Client::Chat)]
#[case::responses_to_gemini(Provider::Responses, Client::Gemini)]
#[case::chat_to_messages(Provider::Chat, Client::Messages)]
#[case::chat_to_responses(Provider::Chat, Client::Responses)]
#[case::chat_to_gemini(Provider::Chat, Client::Gemini)]
#[case::gemini_to_messages(Provider::Gemini, Client::Messages)]
#[case::gemini_to_responses(Provider::Gemini, Client::Responses)]
#[case::gemini_to_chat(Provider::Gemini, Client::Chat)]
fn a_streamed_reply_is_the_clients_own_stream(#[case] provider: Provider, #[case] client: Client) {
	let input = provider_stream(provider);
	for size in [7, input.len()] {
		let mut driver = StreamReply::new(pair(provider, client), MAX_EVENT_BYTES);
		let mut out = Vec::new();
		for chunk in input.chunks(size) {
			driver.push(chunk).unwrap();
			out.extend(driver.take_output().unwrap_or_default());
		}
		driver.finish().unwrap();
		out.extend(driver.take_output().unwrap_or_default());
		assert_eq!(driver.early_failure(), None);
		let sent = String::from_utf8(out).unwrap();
		assert_eq!(streamed_text(client, &sent), "Hello, \"world\"", "{sent}");
		assert!(sent.contains(stream_end(client)), "{sent}");
		assert_eq!(driver.usage().output_tokens, Some(5));
	}
}

#[test]
fn a_captured_token_limit_stream_keeps_usage_without_exposing_the_call() {
	let input = include_bytes!("fixtures/messages-max-tokens-open-tool.sse");
	for size in [1, 7, input.len()] {
		let mut driver = StreamReply::new(pair(Provider::Messages, Client::Responses), MAX_EVENT_BYTES);
		let mut out = Vec::new();
		for chunk in input.chunks(size) {
			driver.push(chunk).unwrap();
			out.extend(driver.take_output().unwrap_or_default());
		}
		driver.finish().unwrap();
		out.extend(driver.take_output().unwrap_or_default());
		assert_eq!(driver.early_failure(), None);
		assert_eq!(driver.usage().input_tokens, Some(2795));
		assert_eq!(driver.usage().cached_tokens, Some(2773));
		assert_eq!(driver.usage().output_tokens, Some(2048));
		let events: Vec<Value> = std::str::from_utf8(&out)
			.unwrap()
			.lines()
			.filter_map(|line| line.strip_prefix("data: "))
			.map(|data| serde_json::from_str(data).unwrap())
			.collect();
		let terminal = events.last().unwrap();
		assert_eq!(terminal["type"], "response.incomplete");
		let response = &terminal["response"];
		assert_eq!(response["usage"]["total_tokens"], 4843);
		assert_eq!(response["output"], json!([]));
		assert!(
			!events
				.iter()
				.any(|event| event["type"] == "response.output_item.added"
					|| event["type"] == "response.output_item.done")
		);
	}
}

#[rstest]
#[case::messages_to_responses(Provider::Messages, Client::Responses)]
#[case::messages_to_chat(Provider::Messages, Client::Chat)]
#[case::messages_to_gemini(Provider::Messages, Client::Gemini)]
#[case::responses_to_messages(Provider::Responses, Client::Messages)]
#[case::responses_to_chat(Provider::Responses, Client::Chat)]
#[case::responses_to_gemini(Provider::Responses, Client::Gemini)]
#[case::chat_to_messages(Provider::Chat, Client::Messages)]
#[case::chat_to_responses(Provider::Chat, Client::Responses)]
#[case::chat_to_gemini(Provider::Chat, Client::Gemini)]
#[case::gemini_to_messages(Provider::Gemini, Client::Messages)]
#[case::gemini_to_responses(Provider::Gemini, Client::Responses)]
#[case::gemini_to_chat(Provider::Gemini, Client::Chat)]
fn a_whole_reply_is_the_clients_own_reply(#[case] provider: Provider, #[case] client: Client) {
	let mut driver = BufferedReply::new(pair(provider, client));
	driver.read(provider_body(provider).as_bytes()).unwrap();
	assert_eq!(driver.usage().output_tokens, Some(5));
	let reply = driver.into_reply();
	assert_eq!(
		reply.status,
		200,
		"{}",
		String::from_utf8_lossy(&reply.body)
	);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(
		body.pointer(answer(client)).unwrap(),
		"Hello, \"world\"",
		"{body}"
	);
}

#[test]
fn gemini_text_signatures_are_not_replayed_as_duplicate_messages_text() {
	let body = json!({"candidates": [{"content": {"role": "model", "parts": [
		{"text": "Hello, world", "thoughtSignature": "signed-text"}
	]}, "finishReason": "STOP"}], "responseId": "g_1", "modelVersion": MODEL});
	let mut driver = BufferedReply::new(pair(Provider::Gemini, Client::Messages));
	driver.read(body.to_string().as_bytes()).unwrap();
	let reply: Value = serde_json::from_slice(&driver.into_reply().body).unwrap();
	assert_eq!(
		reply["content"],
		json!([{"type": "text", "text": "Hello, world"}])
	);
}

/// Call ids made up for a reply that names none must differ between turns, or the
/// next request carries the same call id twice.
#[rstest]
#[case::chat(Provider::Chat)]
#[case::gemini(Provider::Gemini)]
fn calls_in_replies_without_ids_get_different_ids_each_turn(#[case] provider: Provider) {
	let body = match provider {
		Provider::Chat => json!({"object": "chat.completion", "created": 1, "model": MODEL,
			"choices": [{"index": 0, "message": {"role": "assistant", "tool_calls": [
				{"index": 0, "type": "function",
					"function": {"name": "lookup", "arguments": "{}"}}
			]}, "finish_reason": "tool_calls"}]}),
		Provider::Gemini => json!({"candidates": [{"content": {"role": "model", "parts": [
			{"functionCall": {"name": "lookup", "args": {}}}
		]}, "finishReason": "STOP"}], "modelVersion": MODEL}),
		Provider::Messages | Provider::Responses => unreachable!("their replies always name an id"),
	};
	let call_id = |fallback_reply_id| {
		let mut driver = BufferedReply::new(ReplyContext {
			fallback_reply_id,
			..pair(provider, Client::Messages)
		});
		driver.read(body.to_string().as_bytes()).unwrap();
		let reply: Value = serde_json::from_slice(&driver.into_reply().body).unwrap();
		reply["content"][0]["id"].as_str().unwrap().to_string()
	};
	assert_ne!(call_id("attempt-1"), call_id("attempt-2"));
}

/// The stream stopped before the provider said it was done.
#[rstest]
fn a_stream_cut_short_before_output_is_an_http_error(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let provider = match client {
		Client::Messages => Provider::Responses,
		_ => Provider::Messages,
	};
	let mut driver = StreamReply::new(pair(provider, client), MAX_EVENT_BYTES);
	assert!(driver.finish().is_err());
	assert_eq!(driver.take_output(), None);
	assert_eq!(driver.early_failure().unwrap().status, 500);
}

#[rstest]
#[case::messages(Provider::Responses, Client::Messages, "messages")]
#[case::responses(Provider::Messages, Client::Responses, "responses")]
#[case::chat(Provider::Messages, Client::Chat, "chat")]
#[case::gemini(Provider::Messages, Client::Gemini, "gemini")]
fn a_transport_failure_after_output_ends_with_one_client_error(
	#[case] provider: Provider,
	#[case] client: Client,
	#[case] snapshot: &str,
) {
	let input = unfinished_text_stream(provider);
	let (sent, _) = failed_stream(provider, client, &input, &[]);
	let events: Vec<Value> = sent
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
		.map(|data| serde_json::from_str(data).unwrap())
		.collect();
	let is_error = |event: &&Value| match client {
		Client::Messages => event["type"] == "error",
		Client::Responses => event["type"] == "response.failed",
		Client::Chat | Client::Gemini => event.get("error").is_some(),
	};
	assert_eq!(events.iter().filter(is_error).count(), 1);
	assert!(is_error(&events.last().unwrap()));
	assert!(!sent.contains(stream_end(client)), "{sent}");
	insta::assert_snapshot!(format!("transport_failure_after_output_{snapshot}"), sent);
}

proptest! {
	#![proptest_config(ProptestConfig::with_cases(64))]

	#[test]
	fn a_transport_failure_after_output_does_not_depend_on_chunk_boundaries(
		cuts in prop::collection::vec(any::<usize>(), 0..24),
	) {
		for provider in [Provider::Messages, Provider::Responses, Provider::Chat, Provider::Gemini] {
			let input = unfinished_text_stream(provider);
			for client in [Client::Messages, Client::Responses, Client::Chat, Client::Gemini] {
				if same_protocol(provider, client) {
					continue;
				}
				let whole = failed_stream(provider, client, &input, &[]);
				let split = failed_stream(provider, client, &input, &cuts);
				prop_assert_eq!(split, whole, "{:?} -> {:?}", provider, client);
			}
		}
	}
}

#[rstest]
#[case::messages(
	Provider::Messages,
	r#"{"type":"error","error":{"type":"rate_limit_error","message":"Slow down"}}"#
)]
#[case::responses(
	Provider::Responses,
	r#"{"error":{"type":"requests","code":"rate_limit_exceeded","message":"Slow down"}}"#
)]
#[case::chat(
	Provider::Chat,
	r#"{"error":{"type":"requests","code":"rate_limit_exceeded","message":"Slow down"}}"#
)]
#[case::gemini(
	Provider::Gemini,
	r#"{"error":{"code":429,"message":"Slow down","status":"RESOURCE_EXHAUSTED"}}"#
)]
fn a_provider_http_error_is_the_clients(
	#[case] provider: Provider,
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
	#[case] body: &str,
) {
	let reply = error_reply(
		client_protocol(client),
		upstream_protocol(provider),
		429,
		body.as_bytes(),
	);
	assert_eq!(reply.status, 429);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	match client {
		Client::Messages => assert_eq!(body["error"]["type"], "rate_limit_error", "{body}"),
		Client::Responses => assert_eq!(body["error"]["code"], "rate_limit_exceeded", "{body}"),
		Client::Chat => assert_eq!(
			body["error"],
			json!({"message": "The provider's rate limit was reached.", "type": "rate_limit_error",
				"param": null, "code": "rate_limit_exceeded"})
		),
		Client::Gemini => assert_eq!(body["error"]["status"], "RESOURCE_EXHAUSTED", "{body}"),
	}
}

#[rstest]
#[case::chat(
	Provider::Chat,
	r#"{"error":{"type":"invalid_request_error","param":"temperature"}}"#,
	"The provider refused `temperature`."
)]
#[case::gemini(
	Provider::Gemini,
	r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","details":[{"fieldViolations":[{"field":"generation_config.temperature"}]}]}}"#,
	"The provider refused `generation_config.temperature`."
)]
fn a_messages_http_error_names_the_refused_field(
	#[case] provider: Provider,
	#[case] body: &str,
	#[case] message: &str,
) {
	let reply = error_reply(
		ClientProtocol::Messages,
		upstream_protocol(provider),
		400,
		body.as_bytes(),
	);
	assert_eq!(reply.status, 400);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(body["error"]["type"], "invalid_request_error");
	assert_eq!(body["error"]["message"], message);
}

/// A provider serving the client's own protocol: the caller sends its reply on unchanged,
/// and only reads it for usage.
#[rstest]
fn a_passthrough_stream_is_read_for_usage(
	#[values(
		Provider::Messages,
		Provider::Responses,
		Provider::Chat,
		Provider::Gemini
	)]
	provider: Provider,
) {
	let input = provider_stream(provider);
	for size in [7, input.len()] {
		let mut observed = ObservedReply::new(
			upstream_protocol(provider),
			MODEL,
			MAX_EVENT_BYTES,
			MAX_HELD_BYTES,
		);
		for chunk in input.chunks(size) {
			observed.push(chunk).unwrap();
		}
		observed.finish().unwrap();
		assert_eq!(observed.usage().input_tokens, Some(3));
		assert_eq!(observed.usage().output_tokens, Some(5));
		assert_eq!(observed.telemetry().model.as_deref(), Some(MODEL));
	}
}

#[rstest]
fn a_passthrough_body_is_read_for_usage(
	#[values(
		Provider::Messages,
		Provider::Responses,
		Provider::Chat,
		Provider::Gemini
	)]
	provider: Provider,
) {
	let mut observed = ObservedReply::new(
		upstream_protocol(provider),
		MODEL,
		MAX_EVENT_BYTES,
		MAX_HELD_BYTES,
	);
	observed.read(provider_body(provider).as_bytes()).unwrap();
	assert_eq!(observed.usage().input_tokens, Some(3));
	assert_eq!(observed.usage().output_tokens, Some(5));
}

/// What arrived is still counted, so a client that leaves mid-stream is still billed.
#[test]
fn a_passthrough_stream_cut_short_keeps_its_usage() {
	let input = provider_stream(Provider::Messages);
	let mut observed = ObservedReply::new(
		UpstreamProtocol::Messages,
		MODEL,
		MAX_EVENT_BYTES,
		MAX_HELD_BYTES,
	);
	observed.push(&input[..input.len() / 2]).unwrap();
	assert!(observed.finish().is_err());
	assert_eq!(observed.usage().input_tokens, Some(3));
}

fn unfinished_text_stream(provider: Provider) -> Vec<u8> {
	let input = String::from_utf8(provider_stream(provider)).unwrap();
	let count = match provider {
		Provider::Messages => 3,
		Provider::Responses => 4,
		Provider::Chat | Provider::Gemini => 1,
	};
	input
		.split_inclusive("\n\n")
		.take(count)
		.collect::<String>()
		.into_bytes()
}

fn failed_stream(
	provider: Provider,
	client: Client,
	input: &[u8],
	cuts: &[usize],
) -> (String, agent_llm_ir::Usage) {
	let mut offsets: Vec<_> = cuts.iter().map(|cut| cut % (input.len() + 1)).collect();
	offsets.extend([0, input.len()]);
	offsets.sort_unstable();
	offsets.dedup();
	let mut driver = StreamReply::new(pair(provider, client), MAX_EVENT_BYTES);
	let mut sent = Vec::new();
	for chunk in offsets.windows(2) {
		driver.push(&input[chunk[0]..chunk[1]]).unwrap();
		sent.extend(driver.take_output().unwrap_or_default());
	}
	assert!(!driver.is_finished());
	assert_eq!(
		streamed_text(client, std::str::from_utf8(&sent).unwrap()),
		"Hello"
	);
	let usage = driver.usage();
	if matches!(provider, Provider::Messages) {
		assert_eq!(usage.input_tokens, Some(3));
		assert_eq!(usage.output_tokens, Some(1));
	}
	driver.fail();
	// Read the failure before finish so finish cannot hide a broken fail implementation.
	let error = driver.take_output().unwrap();
	assert!(!error.is_empty());
	sent.extend(error);
	assert_eq!(driver.early_failure(), None);
	assert!(driver.finish().is_err());
	assert_eq!(driver.take_output(), None);
	assert_eq!(driver.usage(), usage);
	(String::from_utf8(sent).unwrap(), usage)
}

const fn pair(provider: Provider, client: Client) -> ReplyContext<'static> {
	ReplyContext {
		client: client_protocol(client),
		upstream: upstream_protocol(provider),
		scope: StateScope {
			backend: "account",
			provider: "provider",
			model: MODEL,
		},
		routes: &[],
		show_reasoning: false,
		include_usage: true,
		created: 1_700_000_000,
		fallback_reply_id: "reply-1",
		stop_sequences: Vec::new(),
		max_held_bytes: MAX_HELD_BYTES,
	}
}

const fn same_protocol(provider: Provider, client: Client) -> bool {
	matches!(
		(provider, client),
		(Provider::Messages, Client::Messages)
			| (Provider::Responses, Client::Responses)
			| (Provider::Chat, Client::Chat)
			| (Provider::Gemini, Client::Gemini)
	)
}

const fn client_protocol(client: Client) -> ClientProtocol {
	match client {
		Client::Messages => ClientProtocol::Messages,
		Client::Responses => ClientProtocol::Responses,
		Client::Chat => ClientProtocol::ChatCompletions,
		Client::Gemini => ClientProtocol::Gemini,
	}
}

const fn upstream_protocol(provider: Provider) -> UpstreamProtocol {
	match provider {
		Provider::Messages => UpstreamProtocol::Messages,
		Provider::Responses => UpstreamProtocol::Responses {
			safety_identifier: false,
			reasoning_context: false,
		},
		Provider::Chat => UpstreamProtocol::ChatCompletions,
		Provider::Gemini => UpstreamProtocol::Gemini,
	}
}

/// Where each client's protocol puts the answer in a whole reply.
const fn answer(client: Client) -> &'static str {
	match client {
		Client::Messages => "/content/0/text",
		Client::Responses => "/output/0/content/0/text",
		Client::Chat => "/choices/0/message/content",
		Client::Gemini => "/candidates/0/content/parts/0/text",
	}
}

/// How each client's protocol ends a finished stream.
const fn stream_end(client: Client) -> &'static str {
	match client {
		Client::Messages => "event: message_stop",
		Client::Responses => "event: response.completed",
		Client::Chat => "data: [DONE]",
		Client::Gemini => r#""finishReason":"STOP""#,
	}
}

/// The answer text a client reads from its stream's deltas.
fn streamed_text(client: Client, sent: &str) -> String {
	let events = sent
		.split("\n")
		.filter_map(|line| line.trim_end().strip_prefix("data: "))
		.filter(|data| *data != "[DONE]")
		.map(|data| serde_json::from_str::<Value>(data).unwrap());
	let mut text = String::new();
	for event in events {
		let delta = match client {
			Client::Messages => event.pointer("/delta/text"),
			Client::Responses => (event["type"] == "response.output_text.delta").then(|| &event["delta"]),
			Client::Chat => event.pointer("/choices/0/delta/content"),
			Client::Gemini => event.pointer("/candidates/0/content/parts/0/text"),
		};
		text.push_str(delta.and_then(Value::as_str).unwrap_or_default());
	}
	text
}

/// Each provider's stream contains quoted answer text.
fn provider_stream(provider: Provider) -> Vec<u8> {
	match provider {
		Provider::Messages => include_bytes!("fixtures/messages-text.sse").to_vec(),
		Provider::Responses => include_bytes!("fixtures/responses-text.sse").to_vec(),
		Provider::Chat => include_bytes!("fixtures/chat-completions-text.sse").to_vec(),
		Provider::Gemini => include_bytes!("fixtures/gemini-text.sse").to_vec(),
	}
}

/// Each provider's whole reply contains quoted answer text.
fn provider_body(provider: Provider) -> String {
	let body = match provider {
		Provider::Messages => json!({"id": "msg_1", "type": "message", "role": "assistant",
			"model": MODEL, "content": [{"type": "text", "text": "Hello, \"world\""}],
			"stop_reason": "end_turn", "stop_sequence": null,
			"usage": {"input_tokens": 3, "output_tokens": 5}}),
		Provider::Responses => responses_body(),
		Provider::Chat => json!({"id": "chatcmpl_1", "object": "chat.completion", "created": 1,
			"model": MODEL, "choices": [{"index": 0, "message": {"role": "assistant",
			"content": "Hello, \"world\""}, "finish_reason": "stop"}],
			"usage": {"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8}}),
		Provider::Gemini => json!({"candidates": [{"content": {"role": "model",
			"parts": [{"text": "Hello, \"world\""}]}, "finishReason": "STOP"}],
			"usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 5,
			"totalTokenCount": 8}, "responseId": "g_1", "modelVersion": MODEL}),
	};
	body.to_string()
}

fn responses_body() -> Value {
	json!({"id": "resp_1", "object": "response", "model": MODEL, "status": "completed",
		"output": [{"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
		"content": [{"type": "output_text", "text": "Hello, \"world\"", "annotations": []}]}],
		"usage": {"input_tokens": 3, "output_tokens": 5, "total_tokens": 8}})
}
