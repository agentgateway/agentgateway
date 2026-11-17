//! A reply rewrite and output checks between the decoder and each client's writer:
//! restored text and tool-call arguments, and a stop, come out the way each client
//! protocol says them.

#![allow(clippy::panic, clippy::unwrap_used)]

use std::fmt::Write;

use agent_llm_ir::{
	BufferedReply, ClientProtocol, IncompleteCall, OutputChecks, ReplyContext, ReplyPlace,
	ReplyProgress, ReplyRewrite, ReplyRewrites, StateScope, StreamReply, UpstreamProtocol, Verdict,
};
use rstest::rstest;
use serde_json::{Value, json};

const SCOPE: StateScope<'static> = StateScope {
	backend: "account",
	provider: "anthropic",
	model: "claude-test",
};
const MAX_HELD_BYTES: usize = 1 << 20;

/// Restores whole tokens. A buffered reply sends each text in one piece.
struct Restore;

impl ReplyRewrite for Restore {
	fn text(&mut self, _: ReplyPlace, text: &str, out: &mut String) {
		out.push_str(
			&text
				.replace("<T0>", "a@b.com")
				.replace("<T1>", r#"say "hi""#),
		);
	}
}

/// Checks each part whole.
const fn whole_parts() -> OutputChecks {
	OutputChecks {
		window_bytes: usize::MAX,
		overlap_bytes: 0,
		max_held_bytes: MAX_HELD_BYTES,
		incomplete_call: IncompleteCall::Drop,
	}
}

#[derive(Debug, Clone, Copy)]
enum Client {
	Messages,
	Responses,
	Chat,
	Gemini,
}

impl Client {
	const fn protocol(self) -> ClientProtocol {
		match self {
			Self::Messages => ClientProtocol::Messages,
			Self::Responses => ClientProtocol::Responses,
			Self::Chat => ClientProtocol::ChatCompletions,
			Self::Gemini => ClientProtocol::Gemini,
		}
	}
}

/// A Messages provider's reply: text with a token, then a call whose argument
/// is a token.
fn provider_reply() -> Vec<u8> {
	json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "m",
		"content": [
			{"type": "text", "text": "Mail <T0> please"},
			{"type": "tool_use", "id": "toolu_1", "name": "send", "input": {"to": "<T1>"}},
		],
		"stop_reason": "tool_use", "stop_sequence": null,
		"usage": {"input_tokens": 3, "output_tokens": 7}})
	.to_string()
	.into_bytes()
}

fn client_reply(client: Client, rewrite: Box<dyn ReplyRewrite>) -> Value {
	let mut reply = BufferedReply::new(reply_context(client));
	reply.set_rewrites(ReplyRewrites::new(vec![rewrite]));
	reply.read(&provider_reply()).unwrap();
	let reply = reply.into_reply();
	assert_eq!(
		reply.status,
		200,
		"{}",
		String::from_utf8_lossy(&reply.body)
	);
	serde_json::from_slice(&reply.body).unwrap()
}

/// A provider that could take the stop sequence would never have sent the text after it,
/// so a rewrite or output check must not see that text either.
#[test]
fn a_rewrite_never_sees_text_after_an_emulated_stop() {
	let mut reply = BufferedReply::new(ReplyContext {
		show_reasoning: false,
		stop_sequences: vec![" <T0>".into()],
		..reply_context(Client::Messages)
	});
	reply.set_checks(whole_parts());
	reply.read(&provider_reply()).unwrap();
	let ReplyProgress::Waiting(held) = reply.progress() else {
		panic!("expected a held reply");
	};
	assert_eq!(held.check().text, "Mail");
	assert!(matches!(
		held.resolve(Verdict::Release),
		ReplyProgress::Ready
	));
	let reply: Value = serde_json::from_slice(&reply.into_reply().body).unwrap();
	assert_eq!(reply["content"], json!([{"type": "text", "text": "Mail"}]));
	assert_eq!(reply["stop_reason"], "stop_sequence");
	assert_eq!(reply["stop_sequence"], " <T0>");
}

/// The client's reply when every check is answered by `judge`.
fn checked_client_reply(client: Client, judge: impl Fn(ReplyPlace, &str) -> Verdict) -> Value {
	let reply = checked_http_reply(client, judge);
	assert_eq!(
		reply.status,
		200,
		"{}",
		String::from_utf8_lossy(&reply.body)
	);
	serde_json::from_slice(&reply.body).unwrap()
}

fn checked_http_reply(
	client: Client,
	judge: impl Fn(ReplyPlace, &str) -> Verdict,
) -> agent_llm_ir::HttpReply {
	let mut reply = BufferedReply::new(reply_context(client));
	reply.set_checks(whole_parts());
	reply.read(&provider_reply()).unwrap();
	let mut progress = reply.progress();
	while let ReplyProgress::Waiting(held) = progress {
		let check = held.check();
		let verdict = judge(check.place, check.text);
		progress = held.resolve(verdict);
	}
	reply.into_reply()
}

/// Answers checks of tool calls with `verdict`, and releases text.
fn call_verdict(verdict: Verdict) -> impl Fn(ReplyPlace, &str) -> Verdict {
	move |place, _| match place {
		ReplyPlace::ToolCall(_) => verdict.clone(),
		_ => Verdict::Release,
	}
}

const fn reply_context(client: Client) -> ReplyContext<'static> {
	ReplyContext {
		client: client.protocol(),
		upstream: UpstreamProtocol::Messages,
		scope: SCOPE,
		routes: &[],
		show_reasoning: true,
		include_usage: false,
		created: 1,
		fallback_reply_id: "reply-1",
		stop_sequences: Vec::new(),
		max_held_bytes: MAX_HELD_BYTES,
	}
}

/// The client's answer text and its calls' arguments, wherever its protocol puts them.
fn answer_and_arguments(client: Client, reply: &Value) -> (String, Vec<Value>) {
	let parse = |arguments: &Value| serde_json::from_str(arguments.as_str().unwrap()).unwrap();
	match client {
		Client::Messages => {
			let blocks = reply["content"].as_array().unwrap();
			let text = blocks
				.iter()
				.filter_map(|block| block["text"].as_str())
				.collect();
			let calls = blocks.iter().filter(|block| block["type"] == "tool_use");
			(text, calls.map(|call| call["input"].clone()).collect())
		},
		Client::Responses => {
			let items = reply["output"].as_array().unwrap();
			let text = items
				.iter()
				.filter(|item| item["type"] == "message")
				.flat_map(|item| item["content"].as_array().unwrap())
				.filter_map(|part| part["text"].as_str())
				.collect();
			let calls = items.iter().filter(|item| item["type"] == "function_call");
			(text, calls.map(|call| parse(&call["arguments"])).collect())
		},
		Client::Chat => {
			let message = &reply["choices"][0]["message"];
			let text = message["content"].as_str().unwrap_or_default().to_string();
			let calls = message["tool_calls"]
				.as_array()
				.cloned()
				.unwrap_or_default();
			(
				text,
				calls
					.iter()
					.map(|call| parse(&call["function"]["arguments"]))
					.collect(),
			)
		},
		Client::Gemini => {
			let parts = reply["candidates"][0]["content"]["parts"]
				.as_array()
				.unwrap();
			let text = parts
				.iter()
				.filter_map(|part| part["text"].as_str())
				.collect();
			let calls = parts.iter().filter_map(|part| part.get("functionCall"));
			(text, calls.map(|call| call["args"].clone()).collect())
		},
	}
}

/// The error a stop gives each client, as an HTTP body or as the stream's last event: the
/// protocol's invalid-request kind, which no client retries, and the stop's message.
fn stopped_with(client: Client, error: &Value, message: &str) -> bool {
	let error = match client {
		Client::Messages => &error["error"],
		Client::Responses if error["type"] == "response.failed" => &error["response"]["error"],
		Client::Responses | Client::Chat | Client::Gemini => &error["error"],
	};
	let kind = match client {
		Client::Messages => error["type"] == "invalid_request_error",
		Client::Responses | Client::Chat => error["code"] == "invalid_prompt",
		Client::Gemini => error["status"] == "INVALID_ARGUMENT",
	};
	kind && error["message"] == message
}

#[rstest]
fn restored_text_and_arguments_reach_every_client(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let reply = client_reply(client, Box::new(Restore));
	let (text, arguments) = answer_and_arguments(client, &reply);
	assert_eq!(text, "Mail a@b.com please", "{reply}");
	assert_eq!(arguments, [json!({"to": "say \"hi\""})], "{reply}");
}

#[rstest]
fn a_stop_reaches_every_client_as_an_error_with_its_message(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let reply = checked_http_reply(client, |_, text| {
		if text.contains("<T0>") {
			Verdict::Stop("Blocked.".into())
		} else {
			Verdict::Release
		}
	});
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(reply.status, 400, "{body}");
	assert!(stopped_with(client, &body, "Blocked."), "{body}");
}

#[rstest]
fn a_held_tool_call_reaches_every_client_once_released(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let reply = checked_client_reply(client, call_verdict(Verdict::Release));
	let (text, arguments) = answer_and_arguments(client, &reply);
	assert_eq!(text, "Mail <T0> please", "{reply}");
	assert_eq!(arguments, [json!({"to": "<T1>"})], "{reply}");
}

#[rstest]
fn a_stopped_held_tool_call_reaches_no_client(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let reply = checked_http_reply(client, call_verdict(Verdict::Stop("Blocked.".into())));
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(reply.status, 400, "{body}");
	assert!(stopped_with(client, &body, "Blocked."), "{body}");
}

/// As a provider that took the stop sequences would, they match its own text; a rewrite
/// then changes only what is left, and its output can't end the answer.
#[test]
fn stop_sequences_match_the_provider_s_text_before_a_rewrite() {
	let mut reply = BufferedReply::new(ReplyContext {
		stop_sequences: vec!["please".into(), "a@b".into()],
		..reply_context(Client::Messages)
	});
	reply.set_rewrites(ReplyRewrites::new(vec![Box::new(Restore)]));
	reply.read(&provider_reply()).unwrap();
	let reply: Value = serde_json::from_slice(&reply.into_reply().body).unwrap();
	assert_eq!(
		reply["content"],
		json!([{"type": "text", "text": "Mail a@b.com "}])
	);
	assert_eq!(reply["stop_sequence"], "please");
	assert_eq!(reply["usage"]["output_tokens"], 7);
}

#[rstest]
fn held_output_past_its_limit_fails_every_client(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let mut reply = BufferedReply::new(reply_context(client));
	reply.set_checks(OutputChecks {
		max_held_bytes: 8,
		..whole_parts()
	});
	reply.read(&provider_reply()).unwrap();
	assert!(matches!(reply.progress(), ReplyProgress::Ready));
	assert_eq!(reply.usage().output_tokens, Some(7));
	let reply = reply.into_reply();
	assert_eq!(
		reply.status,
		500,
		"{}",
		String::from_utf8_lossy(&reply.body)
	);
}

/// A Messages provider's stream: text, then usage.
fn provider_stream() -> Vec<u8> {
	[
		json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
			"role": "assistant", "model": "m", "content": [],
			"usage": {"input_tokens": 3, "output_tokens": 1}}}),
		json!({"type": "content_block_start", "index": 0,
			"content_block": {"type": "text", "text": ""}}),
		json!({"type": "content_block_delta", "index": 0,
			"delta": {"type": "text_delta", "text": "Hello"}}),
		json!({"type": "content_block_stop", "index": 0}),
		json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
			"usage": {"output_tokens": 9}}),
		json!({"type": "message_stop"}),
	]
	.iter()
	.fold(String::new(), |mut stream, event| {
		write!(
			stream,
			"event: {}\ndata: {event}\n\n",
			event["type"].as_str().unwrap()
		)
		.unwrap();
		stream
	})
	.into_bytes()
}

#[test]
fn usage_keeps_counting_while_output_is_held() {
	let mut reply = StreamReply::new(reply_context(Client::Chat), MAX_HELD_BYTES);
	reply.set_checks(whole_parts());
	reply.push(&provider_stream()).unwrap();
	assert!(reply.is_finished());
	assert_eq!(reply.usage().output_tokens, Some(9));
	let ReplyProgress::Waiting(held) = reply.progress() else {
		panic!("expected a held reply");
	};
	assert_eq!(held.check().text, "Hello");
	assert!(matches!(
		held.resolve(Verdict::Release),
		ReplyProgress::Ready
	));
	let output = String::from_utf8(reply.take_output().unwrap()).unwrap();
	assert!(output.contains("Hello"), "{output}");
}

/// A Messages provider's stream of one text in two deltas.
fn two_delta_stream(first: &str, second: &str) -> Vec<u8> {
	let delta = |text: &str| {
		json!({"type": "content_block_delta", "index": 0,
			"delta": {"type": "text_delta", "text": text}})
	};
	[
		json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
			"role": "assistant", "model": "m", "content": [],
			"usage": {"input_tokens": 3, "output_tokens": 1}}}),
		json!({"type": "content_block_start", "index": 0,
			"content_block": {"type": "text", "text": ""}}),
		delta(first),
		delta(second),
		json!({"type": "content_block_stop", "index": 0}),
		json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
			"usage": {"output_tokens": 9}}),
		json!({"type": "message_stop"}),
	]
	.iter()
	.fold(String::new(), |mut stream, event| {
		write!(
			stream,
			"event: {}\ndata: {event}\n\n",
			event["type"].as_str().unwrap()
		)
		.unwrap();
		stream
	})
	.into_bytes()
}

/// A stream whose text is checked `window_bytes` at a time, each window answered by `judge`.
fn checked_stream(
	client: Client,
	window_bytes: usize,
	judge: impl Fn(&str) -> Verdict,
) -> StreamReply<'static> {
	let mut reply = StreamReply::new(reply_context(client), MAX_HELD_BYTES);
	reply.set_checks(OutputChecks {
		window_bytes,
		..whole_parts()
	});
	reply.push(&two_delta_stream("Hello", " secret")).unwrap();
	let mut progress = reply.progress();
	while let ReplyProgress::Waiting(held) = progress {
		let verdict = judge(held.check().text);
		progress = held.resolve(verdict);
	}
	reply
}

fn stop_secret(text: &str) -> Verdict {
	if text.contains("secret") {
		Verdict::Stop("Blocked.".into())
	} else {
		Verdict::Release
	}
}

#[rstest]
fn a_stop_before_any_output_is_every_client_s_http_error(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let mut reply = checked_stream(client, usize::MAX, stop_secret);
	let Some(failure) = reply.early_failure() else {
		panic!("nothing went out");
	};
	let body: Value = serde_json::from_slice(&failure.body).unwrap();
	assert_eq!(failure.status, 400, "{body}");
	assert!(stopped_with(client, &body, "Blocked."), "{body}");
	assert_eq!(reply.take_output(), None);
	assert_eq!(reply.usage().output_tokens, Some(9));
}

#[rstest]
fn a_stop_after_output_ends_every_client_s_stream_with_an_error(
	#[values(Client::Messages, Client::Responses, Client::Chat, Client::Gemini)] client: Client,
) {
	let mut reply = checked_stream(client, "Hello".len(), stop_secret);
	assert!(reply.early_failure().is_none());
	let output = String::from_utf8(reply.take_output().unwrap()).unwrap();
	let events: Vec<Value> = output
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
		.filter_map(|data| serde_json::from_str(data).ok())
		.collect();
	let last = events.last().unwrap();
	assert!(stopped_with(client, last, "Blocked."), "{output}");
	assert!(output.contains("Hello"), "{output}");
	assert!(!output.contains("secret"), "{output}");
	let ended = match client {
		Client::Messages => output.contains("message_stop"),
		Client::Responses => {
			output.contains("response.completed") || output.contains("response.incomplete")
		},
		Client::Chat => output.contains("[DONE]") || output.contains(r#""finish_reason":""#),
		Client::Gemini => output.contains("finishReason"),
	};
	assert!(!ended, "{output}");
}
