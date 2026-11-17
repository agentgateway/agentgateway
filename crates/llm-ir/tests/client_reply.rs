//! How a caller drives a provider's reply through the core, for each client protocol.
//! `streamed_reply`, `buffered_reply` and `error_reply` are the examples to copy. An
//! iterator of chunks stands in for the transport. Each client reads a Messages provider,
//! except the Messages client, which reads a Responses provider.

#![allow(clippy::unwrap_used)]

use std::fmt::Write;
use std::io;
use std::num::NonZeroU64;

use agent_llm_ir::{
	BufferedReply, ClientEndpoint, ClientProtocol, Delivery, HttpReply, IdentifierPseudonymizer,
	RenderSettings, ReplyContext, RequestRefusal, StateScope, StreamReply, Upstream,
	UpstreamProtocol, Usage, error_reply, prepare,
};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use rstest::rstest;
use serde_json::{Value, json};

/// Bounds one provider event. A Responses provider's terminal event over it arrives
/// in pieces.
const MAX_EVENT_BYTES: usize = 1024;
/// Bounds each decoded block and what a writer keeps while another part streams.
const MAX_HELD_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Copy)]
enum Client {
	Chat,
	Messages,
	Responses,
	Gemini,
}

#[derive(Debug, PartialEq)]
enum ClientReply {
	/// Sent in place of a stream, and nothing else is.
	Error(HttpReply),
	/// The bytes sent after each provider chunk that produced output, in order.
	Stream(Vec<Vec<u8>>),
}

struct Reply {
	client: ClientReply,
	usage: Usage,
	tool_calls: u32,
}

/// A provider reply with a 2xx status. Any other status goes to `error_reply`.
fn streamed_reply(
	context: ReplyContext<'_>,
	chunks: impl IntoIterator<Item = io::Result<Vec<u8>>>,
) -> Reply {
	let mut chunks = chunks.into_iter();
	let mut driver = StreamReply::new(context, MAX_EVENT_BYTES);
	let mut sent = Vec::new();
	while !driver.is_finished() {
		match chunks.next() {
			Some(Ok(chunk)) => {
				// Decode errors fail output but later chunks may still report usage.
				let _ = driver.push(&chunk);
				sent.extend(driver.take_output());
			},
			Some(Err(_)) => {
				driver.fail();
				break;
			},
			None => {
				let _ = driver.finish();
				break;
			},
		}
	}
	let client = match driver.early_failure() {
		Some(reply) => {
			assert!(sent.is_empty(), "an early failure comes before any output");
			ClientReply::Error(reply)
		},
		None => {
			sent.extend(driver.take_output());
			ClientReply::Stream(sent)
		},
	};
	Reply {
		client,
		usage: driver.usage(),
		tool_calls: driver.telemetry().tool_calls,
	}
}

/// A provider reply with a 2xx status, whose body the caller has already bounded.
fn buffered_reply(context: ReplyContext<'_>, body: &[u8]) -> (HttpReply, Usage) {
	let mut driver = BufferedReply::new(context);
	let _ = driver.read(body);
	let usage = driver.usage();
	(driver.into_reply(), usage)
}

const fn context(client: Client) -> ReplyContext<'static> {
	ReplyContext {
		client: client_protocol(client),
		upstream: upstream_protocol(client),
		scope: scope(client),
		routes: &[],
		show_reasoning: false,
		include_usage: false,
		created: 1_700_000_000,
		fallback_reply_id: "reply-1",
		stop_sequences: Vec::new(),
		max_held_bytes: MAX_HELD_BYTES,
	}
}

const fn client_protocol(client: Client) -> ClientProtocol {
	match client {
		Client::Chat => ClientProtocol::ChatCompletions,
		Client::Messages => ClientProtocol::Messages,
		Client::Responses => ClientProtocol::Responses,
		Client::Gemini => ClientProtocol::Gemini,
	}
}

const fn upstream_protocol(client: Client) -> UpstreamProtocol {
	match client {
		Client::Messages => RESPONSES,
		Client::Chat | Client::Responses | Client::Gemini => UpstreamProtocol::Messages,
	}
}

const fn scope(client: Client) -> StateScope<'static> {
	match client {
		Client::Messages => StateScope {
			backend: "account",
			provider: "openai",
			model: "gpt-test",
		},
		Client::Chat | Client::Responses | Client::Gemini => SCOPE,
	}
}

const RESPONSES: UpstreamProtocol = UpstreamProtocol::Responses {
	safety_identifier: false,
	reasoning_context: false,
};

const SCOPE: StateScope<'static> = StateScope {
	backend: "account",
	provider: "anthropic",
	model: "claude-test",
};

const MESSAGES: Upstream<'static> = Upstream {
	identifier_pseudonymizer: &Checksum,
	scope: SCOPE,
	protocol: UpstreamProtocol::Messages,
	default_max_tokens: None,
};

/// A small deterministic hash for identifier tests.
struct Checksum;

impl IdentifierPseudonymizer for Checksum {
	fn pseudonymize(&self, identifier: &str) -> String {
		let mut hash = 0xcbf2_9ce4_8422_2325u64;
		for byte in identifier.bytes() {
			hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
		}
		format!("{hash:016x}")
	}
}

#[rstest]
fn a_reply_streams_to_the_client(
	#[values(Client::Chat, Client::Messages, Client::Responses, Client::Gemini)] client: Client,
) {
	let (context, input) = match client {
		Client::Messages => (
			context(client),
			chunk(&[
				RESPONSE_CREATED,
				MESSAGE_ADDED,
				&output_text_delta("Hello"),
				&message_done("Hello"),
				COMPLETED,
			]),
		),
		Client::Chat | Client::Responses => (
			context(client),
			chunk(&[
				MESSAGE_START,
				TEXT_START,
				&text_delta("Hello"),
				TEXT_STOP,
				END_TURN,
				MESSAGE_STOP,
			]),
		),
		// Shown thoughts, and the state that rides on the answer.
		Client::Gemini => (
			ReplyContext {
				show_reasoning: true,
				..context(client)
			},
			provider_reply(
				&[thinking_block(0, "Plan.", "sig-1"), text_block(1, "Hello")],
				"end_turn",
			),
		),
	};
	let reply = streamed_reply(context, [input]);
	insta::assert_snapshot!(
		format!("{}_a_reply_streams_to_the_client", name(client)),
		sse(stream(&reply))
	);
	assert_eq!(reply.usage, usage(10, 0, 5));
}

#[rstest]
fn output_is_sent_after_each_chunk(
	#[values(Client::Chat, Client::Messages, Client::Responses, Client::Gemini)] client: Client,
) {
	let chunks = match client {
		Client::Messages => [
			chunk(&[RESPONSE_CREATED, MESSAGE_ADDED, &output_text_delta("Hel")]),
			chunk(&[&output_text_delta("lo")]),
			chunk(&[&message_done("Hello"), COMPLETED]),
		],
		Client::Chat | Client::Responses | Client::Gemini => [
			chunk(&[MESSAGE_START, TEXT_START, &text_delta("Hel")]),
			chunk(&[&text_delta("lo")]),
			chunk(&[TEXT_STOP, END_TURN, MESSAGE_STOP]),
		],
	};
	let reply = streamed_reply(context(client), chunks);
	let sent: Vec<String> = stream(&reply)
		.iter()
		.map(|piece| text(client, &client_events(piece)))
		.collect();
	assert_eq!(sent[..2], ["Hel", "lo"], "{client:?}");
	assert!(sent[2..].iter().all(String::is_empty), "{client:?}");
}

#[test]
fn a_chat_client_that_asked_gets_usage_before_done() {
	let reply = streamed_reply(
		ReplyContext {
			include_usage: true,
			..context(Client::Chat)
		},
		[chunk(&[
			CACHED_MESSAGE_START,
			TEXT_START,
			&text_delta("Hi"),
			TEXT_STOP,
			END_TURN,
			MESSAGE_STOP,
		])],
	);
	insta::assert_snapshot!(sse(stream(&reply)));
}

#[test]
fn an_emulated_stop_sequence_ends_a_chat_answer_but_not_the_accounting() {
	let reply = streamed_reply(
		ReplyContext {
			stop_sequences: vec!["END".into()],
			..context(Client::Chat)
		},
		[chunk(&[
			CACHED_MESSAGE_START,
			TEXT_START,
			&text_delta("one END two"),
			TEXT_STOP,
			END_TURN,
			MESSAGE_STOP,
		])],
	);
	insta::assert_snapshot!(sse(stream(&reply)));
	assert_eq!(reply.usage, usage(10, 20, 5));
}

/// The client sends back what it got, as Chat Completions clients do: the call with its id,
/// and the call's result. The thinking before the call goes back to the provider with it.
#[test]
fn thinking_before_a_chat_call_goes_back_with_the_next_turn() {
	let mut events = vec![CACHED_MESSAGE_START];
	events.extend(THINKING);
	events.extend(CALL);
	events.extend([TOOL_USE, MESSAGE_STOP]);
	let reply = streamed_reply(context(Client::Chat), [chunk(&events)]);
	let chunks = client_events(&stream(&reply).concat());
	let reasoning: String = chunks
		.iter()
		.filter_map(|chunk| chunk["choices"][0]["delta"]["reasoning_content"].as_str())
		.collect();
	assert_eq!(reasoning, "Check the weather.");
	let call = chunks
		.iter()
		.find_map(|chunk| chunk["choices"][0]["delta"]["tool_calls"].get(0))
		.unwrap();
	assert_eq!(call["function"]["arguments"], r#"{"city":"Oslo"}"#);
	let id = call["id"].as_str().unwrap();
	assert!(id.starts_with("toolu_1__agentgateway_llm_ir_"), "{id}");
	assert!(
		id.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
		"{id}"
	);

	let next = json!({"model": "claude-test", "max_completion_tokens": 100, "messages": [
		{"role": "user", "content": "Weather in Oslo?"},
		{"role": "assistant", "content": null, "reasoning_content": reasoning,
			"tool_calls": [{"id": id, "type": "function", "function": call["function"]}]},
		{"role": "tool", "tool_call_id": id, "content": "Sunny"}]});
	let upstream = Upstream {
		default_max_tokens: NonZeroU64::new(1),
		..MESSAGES
	};
	let next = next.to_string();
	let converted = prepare(next.as_bytes(), ClientProtocol::ChatCompletions, upstream)
		.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
		.unwrap();
	let sent: Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(
		sent["messages"][1]["content"],
		json!([{"type": "thinking", "thinking": "Check the weather.", "signature": "c2lnbmVk"},
			{"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {"city": "Oslo"}}])
	);
	assert_eq!(sent["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
	assert_eq!(converted.dropped, ["assistant reasoning text"]);
}

#[test]
fn a_terminal_event_over_the_limit_is_read_in_pieces() {
	let reply = streamed_reply(
		context(Client::Messages),
		[chunk(&[
			RESPONSE_CREATED,
			MESSAGE_ADDED,
			&output_text_delta("Hello"),
			&message_done("Hello"),
			&with_long_instructions(COMPLETED),
		])],
	);
	let events = client_events(&stream(&reply).concat());
	assert_eq!(events.last().unwrap()["type"], "message_stop");
	assert_eq!(text(Client::Messages, &events), "Hello");
	assert_eq!(reply.usage, usage(10, 0, 5));
}

#[test]
fn another_responses_event_over_the_limit_fails_the_reply() {
	let in_progress = with_long_instructions(r#"{"type":"response.in_progress","response":{}}"#);
	let reply = streamed_reply(
		context(Client::Messages),
		[
			chunk(&[RESPONSE_CREATED, MESSAGE_ADDED, &output_text_delta("Hello")]),
			chunk(&[&in_progress]),
			chunk(&[&message_done("Hello"), COMPLETED]),
		],
	);
	let events = client_events(&stream(&reply).concat());
	assert_eq!(text(Client::Messages, &events), "Hello");
	assert_eq!(events.last().unwrap()["type"], "error");
	assert_eq!(reply.usage, usage(10, 0, 5));
}

#[test]
fn a_stop_sequence_ends_a_messages_answer_but_not_the_accounting() {
	let call_added = r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_1","name":"lookup"}}"#;
	let call_arguments =
		r#"{"type":"response.function_call_arguments.delta","output_index":1,"delta":"{}"}"#;
	let call_done = r#"{"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","call_id":"call_1","name":"lookup","arguments":"{}"}}"#;
	let reply = streamed_reply(
		ReplyContext {
			stop_sequences: vec!["STOP".to_string()],
			..context(Client::Messages)
		},
		[
			chunk(&[
				RESPONSE_CREATED,
				MESSAGE_ADDED,
				&output_text_delta("Hello ST"),
			]),
			chunk(&[
				&output_text_delta("OP world"),
				&message_done("Hello STOP world"),
				call_added,
				call_arguments,
				call_done,
				COMPLETED,
			]),
		],
	);
	insta::assert_snapshot!(sse(stream(&reply)));
	// The provider's whole reply is still accounted for.
	assert_eq!(reply.usage, usage(10, 0, 5));
	assert_eq!(reply.tool_calls, 1);
}

#[test]
fn a_stop_sequence_ends_a_buffered_messages_answer() {
	let body = json!({"id": "r", "model": "m", "status": "completed",
		"output": [{"type": "message", "id": "m", "role": "assistant", "status": "completed",
			"content": [{"type": "output_text", "text": "ok STOP more", "annotations": []}]}],
		"usage": {"input_tokens": 3, "output_tokens": 7, "total_tokens": 10}});
	let (reply, total) = buffered_reply(
		ReplyContext {
			stop_sequences: vec!["STOP".into()],
			..context(Client::Messages)
		},
		body.to_string().as_bytes(),
	);
	assert_eq!(reply.status, 200);
	let reply: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(reply["content"], json!([{"type": "text", "text": "ok "}]));
	assert_eq!(reply["stop_reason"], "stop_sequence");
	assert_eq!(reply["stop_sequence"], "STOP");
	assert_eq!(total, usage(3, 0, 7));
}

/// The provider's status stays, so a client's retry logic sees what the provider said.
#[test]
fn a_responses_http_error_keeps_its_status_for_a_messages_client() {
	let body = br#"{"error":{"type":"invalid_request_error","code":null,"param":"temperature","message":"Unsupported value"}}"#;
	insta::assert_json_snapshot!(http(&error_reply(ClientProtocol::Messages, RESPONSES, 400, body)), @r#"
	{
	  "status": 400,
	  "body": {
	    "type": "error",
	    "error": {
	      "type": "invalid_request_error",
	      "message": "The provider refused `temperature`."
	    }
	  }
	}
	"#);
}

#[rstest]
fn a_token_limit_never_exposes_the_last_responses_tool_call(
	#[values(false, true)] block_stop: bool,
	#[values("", "{\"key\":", "{\"key\":1}")] arguments: &str,
) {
	let start = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_1","name":"record","input":{}}}"#;
	let delta = json!({"type": "content_block_delta", "index": 0,
		"delta": {"type": "input_json_delta", "partial_json": arguments}})
	.to_string();
	let terminal = r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":2048}}"#;
	let mut chunks = vec![chunk(&[MESSAGE_START, start, &delta])];
	if block_stop {
		chunks.push(chunk(&[TEXT_STOP]));
	}
	chunks.push(chunk(&[terminal, MESSAGE_STOP]));
	let reply = streamed_reply(context(Client::Responses), chunks);
	let events = client_events(&stream(&reply).concat());
	let terminal = events.last().unwrap();
	assert_eq!(terminal["type"], "response.incomplete");
	let response = &terminal["response"];
	assert_eq!(
		response["incomplete_details"]["reason"],
		"max_output_tokens"
	);
	assert_eq!(response["output"], json!([]));
	assert!(
		!events
			.iter()
			.any(|event| event["type"] == "response.output_item.added"
				|| event["type"] == "response.output_item.done")
	);
	assert_eq!(reply.usage, usage(10, 0, 2048));
	assert_eq!(response["usage"]["output_tokens"], 2048);
}

#[test]
fn a_buffered_token_limit_never_exposes_the_last_responses_tool_call() {
	let body = br#"{"id":"msg_1","model":"claude-test","content":[{"type":"tool_use","id":"call_1","name":"record","input":{"key":1}}],"stop_reason":"max_tokens","usage":{"input_tokens":10,"output_tokens":2048}}"#;
	let (reply, counts) = buffered_reply(context(Client::Responses), body);
	assert_eq!(reply.status, 200);
	let response: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(response["status"], "incomplete");
	assert_eq!(response["output"], json!([]));
	assert_eq!(counts, usage(10, 0, 2048));
}

#[test]
fn a_completed_tool_call_is_sent_to_a_responses_client_once() {
	let start = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_1","name":"record","input":{}}}"#;
	let delta = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"key\":1}"}}"#;
	let terminal =
		r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}"#;
	let reply = streamed_reply(
		context(Client::Responses),
		[
			chunk(&[MESSAGE_START, start, delta]),
			chunk(&[TEXT_STOP]),
			chunk(&[terminal, MESSAGE_STOP]),
		],
	);
	let events = client_events(&stream(&reply).concat());
	let calls: Vec<_> = events
		.iter()
		.filter(|event| event["type"] == "response.output_item.done")
		.collect();
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0]["item"]["type"], "function_call");
	assert_eq!(calls[0]["item"]["status"], "completed");
	assert_eq!(calls[0]["item"]["arguments"], r#"{"key":1}"#);
	assert_eq!(events.last().unwrap()["type"], "response.completed");
	assert_eq!(reply.usage, usage(10, 0, 5));
}

#[rstest]
fn cache_writes_reach_the_responses_client(#[values(false, true)] streaming: bool) {
	let counts = json!({"input_tokens":3,"output_tokens":5,
		"cache_creation_input_tokens":4894,"cache_read_input_tokens":0});
	let response = if streaming {
		let start = json!({"type":"message_start", "message": {
			"id":"msg_1", "model":"claude-test", "usage":counts}})
		.to_string();
		let reply = streamed_reply(
			context(Client::Responses),
			[chunk(&[
				&start,
				TEXT_START,
				&text_delta("OK"),
				TEXT_STOP,
				END_TURN,
				MESSAGE_STOP,
			])],
		);
		let events = client_events(&stream(&reply).concat());
		events.last().unwrap()["response"].clone()
	} else {
		let body = json!({"id":"msg_1", "model":"claude-test",
			"content":[{"type":"text","text":"OK"}], "stop_reason":"end_turn", "usage":counts});
		let (reply, _) = buffered_reply(context(Client::Responses), body.to_string().as_bytes());
		serde_json::from_slice(&reply.body).unwrap()
	};
	assert_eq!(response["usage"]["input_tokens"], 4897);
	assert_eq!(
		response["usage"]["input_tokens_details"]["cache_write_tokens"],
		4894
	);
	assert_eq!(
		response["usage"]["input_tokens_details"]["cached_tokens"],
		0
	);
}

#[rstest]
#[case::event_limit(2 * MAX_EVENT_BYTES, 1)]
#[case::block_limit(MAX_EVENT_BYTES * 6 / 10, MAX_HELD_BYTES / (MAX_EVENT_BYTES * 6 / 10) + 1)]
fn an_event_over_the_limit_fails_a_responses_reply_but_usage_is_still_read(
	#[case] bytes: usize,
	#[case] count: usize,
) {
	let delta = text_delta(&"x".repeat(bytes));
	let deltas = vec![delta.as_str(); count];
	let reply = streamed_reply(
		context(Client::Responses),
		[
			chunk(&[MESSAGE_START, TEXT_START, &text_delta("Hel")]),
			chunk(&deltas),
			chunk(&[TEXT_STOP, END_TURN, MESSAGE_STOP]),
		],
	);
	let events = client_events(&stream(&reply).concat());
	assert_eq!(events.last().unwrap()["type"], "response.failed");
	assert_eq!(reply.usage, usage(10, 0, 5));
}

/// No legacy route writes for a Gemini client, so its whole reply is pinned here.
#[test]
fn a_buffered_gemini_reply_uses_the_same_parts() {
	let body = r#"{"id":"msg_1","model":"claude-test","content":[{"type":"text","text":"Hello"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":5}}"#;
	let (reply, total) = buffered_reply(context(Client::Gemini), body.as_bytes());
	insta::assert_json_snapshot!(http(&reply));
	assert_eq!(total, usage(10, 0, 5));

	let (reply, total) = buffered_reply(context(Client::Gemini), b"{\"content\":");
	insta::assert_json_snapshot!(http(&reply), @r#"
	{
	  "status": 500,
	  "body": {
	    "error": {
	      "code": 500,
	      "message": "The provider's reply could not be served.",
	      "status": "INTERNAL"
	    }
	  }
	}
	"#);
	assert_eq!(total, Usage::default());
}

#[test]
fn gemini_reasoning_comes_back_through_a_tool_round_and_later_turns() {
	let mut history = vec![user("Read a.")];
	next_gemini_request(&history, MESSAGES).unwrap();

	let first = streamed_gemini_reply(&[
		thinking_block(0, "Need a.", "sig-1"),
		text_block(1, "Reading."),
		call_block(2, "toolu_1", r#"{"path":"a"}"#),
	]);
	history.push(recorded(&first));
	history.push(function_response("toolu_1", "A"));
	let sent = next_gemini_request(&history, MESSAGES).unwrap();
	assert_eq!(
		sent["messages"][1]["content"],
		json!([
			thinking("Need a.", "sig-1"),
			{"type": "text", "text": "Reading."},
			{"type": "tool_use", "id": "toolu_1", "name": "read", "input": {"path": "a"}}
		])
	);

	let second = streamed_gemini_reply(&[
		thinking_block(0, "Got A.", "sig-2"),
		call_block(1, "toolu_2", r#"{"path":"b"}"#),
		call_block(2, "toolu_3", r#"{"path":"c"}"#),
	]);
	history.push(recorded(&second));
	history.push(json!({"role": "user", "parts": [
		{"functionResponse": {"id": "toolu_2", "name": "read", "response": {"output": "B"}}},
		{"functionResponse": {"id": "toolu_3", "name": "read", "response": {"output": "C"}}}
	]}));
	let third = streamed_gemini_reply(&[
		thinking_block(0, "All read.", "sig-3"),
		text_block(1, "Done."),
	]);
	history.push(recorded(&third));
	history.push(user("Thanks."));
	let sent = next_gemini_request(&history, MESSAGES).unwrap();
	let assistant: Vec<&Value> = sent["messages"]
		.as_array()
		.unwrap()
		.iter()
		.filter(|message| message["role"] == "assistant")
		.map(|message| &message["content"][0])
		.collect();
	assert_eq!(
		assistant,
		[
			&thinking("Need a.", "sig-1"),
			&thinking("Got A.", "sig-2"),
			&thinking("All read.", "sig-3"),
		]
	);
}

#[test]
fn gemini_reasoning_comes_back_after_a_session_is_saved_and_resumed() {
	let reply = streamed_reply(
		context(Client::Gemini),
		[provider_reply(
			&[
				thinking_block(0, "Need a.", "sig-1"),
				call_block(1, "toolu_1", "{}"),
			],
			"tool_use",
		)],
	);
	// Saved as text and read back by another process, with the padding a client
	// that keeps the signature as bytes may leave off.
	let mut model = recorded(&reply);
	let signature = model["parts"][0]["thoughtSignature"].as_str().unwrap();
	let unpadded = STANDARD_NO_PAD.encode(STANDARD.decode(signature).unwrap());
	model["parts"][0]["thoughtSignature"] = json!(unpadded);
	let saved = serde_json::to_string(&model).unwrap();
	let history = [
		user("Read a."),
		serde_json::from_str(&saved).unwrap(),
		function_response("toolu_1", "A"),
	];
	let sent = next_gemini_request(&history, MESSAGES).unwrap();
	assert_eq!(
		sent["messages"][1]["content"][0],
		thinking("Need a.", "sig-1")
	);

	// A session moved to another model can't replay this one's reasoning.
	let elsewhere = Upstream {
		scope: StateScope {
			model: "claude-other",
			..SCOPE
		},
		..MESSAGES
	};
	let refusal = next_gemini_request(&history, elsewhere).unwrap_err();
	assert!(refusal.problems[0].ends_with("carrier does not match this destination"));
}

#[test]
fn a_responses_providers_reasoning_comes_back_to_gemini_byte_for_byte() {
	let item =
		json!({"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "opaque"});
	let body = json!({"id": "resp_1", "model": "gpt-test", "status": "completed", "output": [
		item,
		{"type": "function_call", "call_id": "call_1", "name": "read", "arguments": "{}"}
	], "usage": {"input_tokens": 10, "output_tokens": 5}});
	let scope = StateScope {
		provider: "openai",
		model: "gpt-test",
		..SCOPE
	};
	let mut driver = BufferedReply::new(ReplyContext {
		scope,
		upstream: RESPONSES,
		..context(Client::Gemini)
	});
	driver.read(body.to_string().as_bytes()).unwrap();
	let reply: Value = serde_json::from_slice(&driver.into_reply().body).unwrap();
	let model = json!({"role": "model", "parts": reply["candidates"][0]["content"]["parts"]});
	let responses = Upstream {
		scope,
		protocol: RESPONSES,
		..MESSAGES
	};
	let sent = next_gemini_request(
		&[user("Read."), model, function_response("call_1", "A")],
		responses,
	)
	.unwrap();
	assert_eq!(sent["input"][1], item);
	assert_eq!(sent["input"][2]["call_id"], "call_1");
}

// What a Messages provider sends.

const MESSAGE_START: &str = r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-test","usage":{"input_tokens":10,"output_tokens":1}}}"#;
const CACHED_MESSAGE_START: &str = r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-test","usage":{"input_tokens":10,"cache_read_input_tokens":20,"output_tokens":1}}}"#;
const TEXT_START: &str =
	r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
const TEXT_STOP: &str = r#"{"type":"content_block_stop","index":0}"#;
const END_TURN: &str =
	r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#;
const TOOL_USE: &str =
	r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}"#;
const MESSAGE_STOP: &str = r#"{"type":"message_stop"}"#;
const THINKING: [&str; 4] = [
	r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
	r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Check the weather."}}"#,
	r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"c2lnbmVk"}}"#,
	r#"{"type":"content_block_stop","index":0}"#,
];
const CALL: [&str; 4] = [
	r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"weather","input":{}}}"#,
	r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}"#,
	r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"Oslo\"}"}}"#,
	r#"{"type":"content_block_stop","index":1}"#,
];

fn text_delta(text: &str) -> String {
	format!(
		r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{text}"}}}}"#
	)
}

fn text_block(index: u32, text: &str) -> Vec<String> {
	vec![
		format!(
			r#"{{"type":"content_block_start","index":{index},"content_block":{{"type":"text","text":""}}}}"#
		),
		format!(
			r#"{{"type":"content_block_delta","index":{index},"delta":{{"type":"text_delta","text":"{text}"}}}}"#
		),
		format!(r#"{{"type":"content_block_stop","index":{index}}}"#),
	]
}

fn thinking_block(index: u32, thinking: &str, signature: &str) -> Vec<String> {
	vec![
		format!(
			r#"{{"type":"content_block_start","index":{index},"content_block":{{"type":"thinking","thinking":"","signature":""}}}}"#
		),
		format!(
			r#"{{"type":"content_block_delta","index":{index},"delta":{{"type":"thinking_delta","thinking":"{thinking}"}}}}"#
		),
		format!(
			r#"{{"type":"content_block_delta","index":{index},"delta":{{"type":"signature_delta","signature":"{signature}"}}}}"#
		),
		format!(r#"{{"type":"content_block_stop","index":{index}}}"#),
	]
}

fn call_block(index: u32, id: &str, arguments: &str) -> Vec<String> {
	vec![
		format!(
			r#"{{"type":"content_block_start","index":{index},"content_block":{{"type":"tool_use","id":"{id}","name":"read","input":{{}}}}}}"#
		),
		format!(
			r#"{{"type":"content_block_delta","index":{index},"delta":{{"type":"input_json_delta","partial_json":{}}}}}"#,
			json!(arguments)
		),
		format!(r#"{{"type":"content_block_stop","index":{index}}}"#),
	]
}

/// A whole Messages provider reply in one chunk: these blocks, then the ending.
fn provider_reply(blocks: &[Vec<String>], stop_reason: &str) -> io::Result<Vec<u8>> {
	let mut events = vec![MESSAGE_START.to_string()];
	events.extend(blocks.iter().flatten().cloned());
	events.push(format!(
		r#"{{"type":"message_delta","delta":{{"stop_reason":"{stop_reason}"}},"usage":{{"output_tokens":5}}}}"#
	));
	events.push(MESSAGE_STOP.to_string());
	chunk(&events)
}

// What a Responses provider sends.

const RESPONSE_CREATED: &str =
	r#"{"type":"response.created","response":{"id":"resp_1","model":"gpt-test"}}"#;
const MESSAGE_ADDED: &str =
	r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"message"}}"#;
const COMPLETED: &str = r#"{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#;

fn output_text_delta(text: &str) -> String {
	format!(
		r#"{{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"{text}"}}"#
	)
}

fn message_done(text: &str) -> String {
	format!(
		r#"{{"type":"response.output_item.done","output_index":0,"item":{{"type":"message","content":[{{"type":"output_text","text":"{text}"}}]}}}}"#
	)
}

/// Real terminal and progress events repeat the request, so they can be large.
fn with_long_instructions(event: &str) -> String {
	let instructions = format!(r#""instructions":"{}","#, "x".repeat(4 * MAX_EVENT_BYTES));
	event.replacen(
		r#""response":{"#,
		&format!(r#""response":{{{instructions}"#),
		1,
	)
}

/// One provider chunk holding these events.
fn chunk(events: &[impl AsRef<str>]) -> io::Result<Vec<u8>> {
	Ok(
		events
			.iter()
			.fold(String::new(), |mut stream, data| {
				write!(stream, "data: {}\n\n", data.as_ref()).unwrap();
				stream
			})
			.into_bytes(),
	)
}

// What the client gets.

fn stream(reply: &Reply) -> &[Vec<u8>] {
	match &reply.client {
		ClientReply::Stream(sent) => sent,
		ClientReply::Error(error) => unreachable!("expected a stream, got {error:?}"),
	}
}

/// The JSON data of each event the client got, read without the core's SSE parser.
/// Chat's `[DONE]` is left out.
fn client_events(sent: &[u8]) -> Vec<Value> {
	std::str::from_utf8(sent)
		.unwrap()
		.replace("\r\n", "\n")
		.split_terminator("\n\n")
		.filter_map(|event| event.lines().find_map(|line| line.strip_prefix("data: ")))
		.filter(|data| *data != "[DONE]")
		.map(|data| serde_json::from_str(data).unwrap())
		.collect()
}

/// The answer text in these client events.
fn text(client: Client, events: &[Value]) -> String {
	events
		.iter()
		.flat_map(|event| match client {
			Client::Chat => vec![&event["choices"][0]["delta"]["content"]],
			Client::Messages => vec![&event["delta"]["text"]],
			Client::Responses if event["type"] == "response.output_text.delta" => {
				vec![&event["delta"]]
			},
			Client::Responses => vec![],
			Client::Gemini => event["candidates"][0]["content"]["parts"]
				.as_array()
				.map(|parts| parts.iter().map(|part| &part["text"]).collect())
				.unwrap_or_default(),
		})
		.filter_map(Value::as_str)
		.collect()
}

/// The client's complete stream with one compact JSON payload per event.
fn sse(sent: &[Vec<u8>]) -> String {
	let sent = String::from_utf8(sent.concat())
		.unwrap()
		.replace("\r\n", "\n");
	sent
		.split_terminator("\n\n")
		.map(|event| {
			event
				.lines()
				.map(|line| match line.strip_prefix("data: ") {
					Some(data) => match serde_json::from_str::<Value>(data) {
						Ok(data) => format!("data: {}", serde_json::to_string(&data).unwrap()),
						Err(_) => line.to_string(),
					},
					None => line.to_string(),
				})
				.collect::<Vec<_>>()
				.join("\n")
		})
		.collect::<Vec<_>>()
		.join("\n\n")
}

fn http(reply: &HttpReply) -> Value {
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	json!({"status": reply.status, "body": body})
}

/// `input` excludes the `cached` tokens, as a Messages provider counts them.
fn usage(input: u64, cached: u64, output: u64) -> Usage {
	Usage {
		input_tokens: Some(input + cached),
		output_tokens: Some(output),
		total_tokens: Some(input + cached + output),
		cached_tokens: (cached > 0).then_some(cached),
		..Usage::default()
	}
}

const fn name(client: Client) -> &'static str {
	match client {
		Client::Chat => "chat",
		Client::Messages => "messages",
		Client::Responses => "responses",
		Client::Gemini => "gemini",
	}
}

// A Gemini client's later turns.

fn streamed_gemini_reply(blocks: &[Vec<String>]) -> Reply {
	let context = ReplyContext {
		show_reasoning: true,
		..context(Client::Gemini)
	};
	let stop_reason = if blocks
		.iter()
		.flatten()
		.any(|event| event.contains("tool_use"))
	{
		"tool_use"
	} else {
		"end_turn"
	};
	streamed_reply(context, [provider_reply(blocks, stop_reason)])
}

/// A model turn as Gemini CLI 0.67.0 records it in history: thought parts are dropped,
/// and adjacent text parts are joined into the first, which keeps its other fields.
fn recorded(reply: &Reply) -> Value {
	let mut kept: Vec<Value> = Vec::new();
	let events = client_events(&stream(reply).concat());
	let parts = events.iter().flat_map(|event| {
		event["candidates"][0]["content"]["parts"]
			.as_array()
			.cloned()
			.unwrap_or_default()
	});
	for part in parts {
		if part["thought"] == json!(true) {
			continue;
		}
		let plain_text = |part: &Value| part["text"].is_string() && part.get("functionCall").is_none();
		match kept.last_mut() {
			Some(last) if plain_text(last) && plain_text(&part) => {
				let joined = format!(
					"{}{}",
					last["text"].as_str().unwrap(),
					part["text"].as_str().unwrap()
				);
				last["text"] = json!(joined);
			},
			_ => kept.push(part),
		}
	}
	json!({"role": "model", "parts": kept})
}

fn user(text: &str) -> Value {
	json!({"role": "user", "parts": [{"text": text}]})
}

fn function_response(id: &str, output: &str) -> Value {
	json!({"role": "user", "parts": [{"functionResponse": {"id": id, "name": "read", "response": {"output": output}}}]})
}

/// The Messages request a Gemini client's turn becomes.
fn next_gemini_request(
	contents: &[Value],
	upstream: Upstream<'_>,
) -> Result<Value, RequestRefusal> {
	let body = json!({
		"contents": contents,
		"tools": [{"functionDeclarations": [{"name": "read", "parametersJsonSchema": {"type": "object"}}]}],
		"generationConfig": {"maxOutputTokens": 1000, "thinkingConfig": {"includeThoughts": true}}
	});
	let converted = prepare(
		body.to_string().as_bytes(),
		ClientEndpoint::Gemini(Delivery::Streaming),
		upstream,
	)?
	.render(RenderSettings::ModelDefaults)?;
	assert!(converted.reply.show_reasoning);
	Ok(serde_json::from_str(&converted.body).unwrap())
}

fn thinking(text: &str, signature: &str) -> Value {
	json!({"type": "thinking", "thinking": text, "signature": signature})
}
