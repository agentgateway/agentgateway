//! A Chat Completions client on each provider the core converts to: the same reply cases
//! from a Messages, a Responses and a Gemini provider, read however the transport splits
//! them, and sent back as the next turn.

#![allow(clippy::unwrap_used)]

use std::fmt::Write;

use agent_llm_ir::{
	BufferedReply, ClientProtocol, HttpReply, IdentifierPseudonymizer, RenderSettings, ReplyContext,
	ReplyPlace, ReplyRewrite, ReplyRewrites, StateScope, StreamReply, Upstream, UpstreamProtocol,
	Usage, prepare,
};
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_HELD_BYTES: usize = 1 << 20;
const CREATED: u64 = 1_700_000_000;
const MODEL: &str = "test-model";

#[derive(Debug, Clone, Copy)]
enum Provider {
	Messages,
	Responses,
	Gemini,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Case {
	Text,
	/// Reasoning, then two calls at once.
	Tools,
	/// Text, then the output limit.
	Cut,
	/// Text, then a provider error in the stream.
	Failed,
}

proptest! {
	#![proptest_config(ProptestConfig::with_cases(64))]

	#[test]
	fn reply_content_and_accounting_do_not_depend_on_chunk_boundaries(
		cuts in prop::collection::vec(any::<usize>(), 0..32),
	) {
		for provider in [Provider::Messages, Provider::Responses, Provider::Gemini] {
			for case in [Case::Text, Case::Tools, Case::Cut, Case::Failed] {
				let input = provider_stream(provider, case);
				let whole = streamed(provider, input.chunks(input.len()));
				let mut offsets: Vec<_> = cuts.iter().map(|cut| cut % (input.len() + 1)).collect();
				offsets.extend([0, input.len()]);
				offsets.sort_unstable();
				offsets.dedup();
				let split = streamed(provider, offsets.windows(2).map(|pair| &input[pair[0]..pair[1]]));
				prop_assert_eq!(split, whole, "{:?} {:?}", provider, case);
			}
		}
	}
}

proptest! {
	#![proptest_config(ProptestConfig::with_cases(64))]

	#[test]
	fn buffered_and_streamed_text_agree_after_stops_and_rewrites(
		parts in prop::collection::vec("[abc日本語→ \n\"\\\\]{0,24}", 1..8),
		stop_at in prop::option::of(0..8usize),
		cuts in prop::collection::vec(any::<usize>(), 0..24),
		output_tokens in 0..128u64,
	) {
		let mut parts = parts;
		parts.insert(0, "a".into());
		if let Some(at) = stop_at {
			let at = 1 + at % parts.len();
			parts.splice(at..at, ["ST".into(), "OP".into()]);
		}
		let text = parts.concat();
		let expected = text[..text.find("STOP").unwrap_or(text.len())].replace('a', "A");
		let (input, body) = messages_text_reply(&parts, output_tokens);
		let context = || ReplyContext {
			// A rewrite can introduce A, but only the provider's text can match a stop.
			stop_sequences: vec!["STOP".into(), "A".into()],
			..context(Provider::Messages)
		};
		let mut buffered = BufferedReply::new(context());
		buffered.set_rewrites(ReplyRewrites::new(vec![Box::new(UppercaseA)]));
		buffered.read(&body).unwrap();
		let usage = buffered.usage();
		let reply = buffered.into_reply();
		prop_assert_eq!(reply.status, 200);
		let body: Value = serde_json::from_slice(&reply.body).unwrap();
		let mut streamed = StreamReply::new(context(), MAX_EVENT_BYTES);
		streamed.set_rewrites(ReplyRewrites::new(vec![Box::new(UppercaseA)]));
		let mut offsets: Vec<_> = cuts.iter().map(|cut| cut % (input.len() + 1)).collect();
		offsets.extend([0, input.len()]);
		offsets.sort_unstable();
		offsets.dedup();
		let mut sent = Vec::new();
		for pair in offsets.windows(2) {
			streamed.push(&input[pair[0]..pair[1]]).unwrap();
			sent.extend(streamed.take_output().unwrap_or_default());
		}
		streamed.finish().unwrap();
		sent.extend(streamed.take_output().unwrap_or_default());
		prop_assert_eq!(streamed.early_failure(), None);
		prop_assert!(!streamed.telemetry().failed);
		prop_assert_eq!(streamed.usage(), usage);
		prop_assert_eq!(usage.input_tokens, Some(30));
		prop_assert_eq!(usage.cached_tokens, Some(20));
		prop_assert_eq!(usage.output_tokens, Some(output_tokens));
		let events: Vec<_> = std::str::from_utf8(&sent).unwrap()
			.split_terminator("\n\n")
			.map(|event| event.strip_prefix("data: ").unwrap().to_string()).collect();
		prop_assert_eq!(events.last().map(String::as_str), Some("[DONE]"));
		prop_assert_eq!(events.iter().filter(|event| *event == "[DONE]").count(), 1);
		let answer = folded(&events);
		prop_assert_eq!(&answer, &message(&body));
		prop_assert_eq!(answer["content"].as_str().unwrap(), expected);
		prop_assert_eq!(&answer["finish_reason"], "stop");
		let stream_usage: Value = serde_json::from_str(&events[events.len() - 2]).unwrap();
		prop_assert_eq!(&stream_usage["usage"], &body["usage"]);
	}
}

#[rstest]
fn a_stream_and_a_whole_reply_say_the_same(
	#[values(Provider::Messages, Provider::Responses, Provider::Gemini)] provider: Provider,
	#[values(Case::Text, Case::Tools, Case::Cut)] case: Case,
) {
	let input = provider_stream(provider, case);
	let stream = streamed(provider, input.chunks(input.len()));
	let events = &stream.events;
	let (reply, usage) = buffered(provider, &provider_body(provider, case));
	assert_eq!(
		reply.status,
		200,
		"{}",
		String::from_utf8_lossy(&reply.body)
	);
	let body: Value = serde_json::from_slice(&reply.body).unwrap();
	assert_eq!(folded(events), message(&body), "{provider:?} {case:?}");
	assert_eq!(stream.usage, usage, "{provider:?} {case:?}");
	assert_eq!(usage.input_tokens, Some(30));
	assert_eq!(usage.cached_tokens, Some(20));
	assert_eq!(usage.output_tokens, Some(5));
	let usage: Value = serde_json::from_str(&events[events.len() - 2]).unwrap();
	assert_eq!(usage["usage"], body["usage"], "{provider:?} {case:?}");
}

#[rstest]
fn parallel_calls_go_out_whole_and_numbered(
	#[values(Provider::Messages, Provider::Responses, Provider::Gemini)] provider: Provider,
) {
	let input = provider_stream(provider, Case::Tools);
	let stream = streamed(provider, input.chunks(5));
	assert_eq!(stream.tool_calls, 2);
	assert!(!stream.failed);
	let events = stream.events;
	let calls: Vec<Value> = chunks(&events)
		.iter()
		.filter_map(|chunk| chunk["choices"][0]["delta"].get("tool_calls"))
		.map(|calls| {
			let [call] = calls.as_array().unwrap().as_slice() else {
				unreachable!("one whole call per chunk: {calls}");
			};
			json!([call["index"], call["function"]])
		})
		.collect();
	assert_eq!(
		calls,
		[
			json!([0, {"name": "lookup", "arguments": "{\"key\":0}"}]),
			json!([1, {"name": "lookup", "arguments": "{\"key\":1}"}]),
		]
	);
	assert_eq!(folded(&events)["finish_reason"], "tool_calls");
}

#[rstest]
fn a_reply_cut_off_by_the_limit_finishes_with_length(
	#[values(Provider::Messages, Provider::Responses, Provider::Gemini)] provider: Provider,
) {
	let input = provider_stream(provider, Case::Cut);
	let events = streamed(provider, input.chunks(input.len())).events;
	let folded = folded(&events);
	assert_eq!(folded["content"], "Hello, wor");
	assert_eq!(folded["finish_reason"], "length");
	assert_eq!(events.last().unwrap(), "[DONE]");
}

#[rstest]
fn a_provider_error_after_output_ends_the_stream_with_an_error(
	#[values(Provider::Messages, Provider::Responses, Provider::Gemini)] provider: Provider,
) {
	let input = provider_stream(provider, Case::Failed);
	let stream = streamed(provider, input.chunks(input.len()));
	assert_eq!(stream.early_failure, None);
	assert!(stream.failed);
	let events = stream.events;
	assert_eq!(folded(&events)["content"], "Hello");
	let last: Value = serde_json::from_str(events.last().unwrap()).unwrap();
	assert_eq!(last["error"]["type"], "server_error", "{last}");
	assert_eq!(last["error"]["code"], "server_is_overloaded", "{last}");
	assert!(!events.iter().any(|event| event == "[DONE]"));
}

/// The client sends back what it got, as Chat Completions clients do. What the provider
/// needs to continue from its reasoning goes back to it, in its own form, before the calls.
#[rstest]
#[case::messages(Provider::Messages, json!([
	{"type": "thinking", "thinking": "Plan.", "signature": "c2lnbmVk"},
	{"type": "tool_use", "id": "toolu_a", "name": "lookup", "input": {"key": 0}},
	{"type": "tool_use", "id": "toolu_b", "name": "lookup", "input": {"key": 1}}]))]
#[case::responses(Provider::Responses, json!([
	{"type": "reasoning", "id": "rs_0", "summary": [{"type": "summary_text", "text": "Plan."}],
		"encrypted_content": "c2lnbmVk"},
	{"type": "function_call", "call_id": "call_a", "name": "lookup", "arguments": "{\"key\":0}"},
	{"type": "function_call", "call_id": "call_b", "name": "lookup", "arguments": "{\"key\":1}"}]))]
#[case::gemini(Provider::Gemini, json!([
	{"functionCall": {"name": "lookup", "args": {"key": 0}}, "thoughtSignature": "c2lnbmVk"},
	{"functionCall": {"name": "lookup", "args": {"key": 1}}}]))]
fn reasoning_goes_back_to_the_provider_with_the_next_turn(
	#[case] provider: Provider,
	#[case] expected: Value,
) {
	let input = provider_stream(provider, Case::Tools);
	let reply = folded(&streamed(provider, input.chunks(7)).events);
	let calls = reply["tool_calls"].as_array().unwrap();
	let mut messages = vec![
		json!({"role": "user", "content": "Look up both keys."}),
		json!({"role": "assistant", "content": null, "reasoning_content": reply["reasoning_content"],
			"tool_calls": calls}),
	];
	for call in calls {
		messages.push(json!({"role": "tool", "tool_call_id": call["id"], "content": "found"}));
	}
	let next = json!({"model": MODEL, "max_completion_tokens": 100, "messages": messages});
	let upstream = Upstream {
		identifier_pseudonymizer: &IdentifierHash,
		scope: SCOPE,
		protocol: upstream_protocol(provider),
		default_max_tokens: None,
	};
	let next = next.to_string();
	let converted = prepare(next.as_bytes(), ClientProtocol::ChatCompletions, upstream)
		.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
		.unwrap_or_else(|refusal| unreachable!("{refusal}"));
	let sent: Value = serde_json::from_str(&converted.body).unwrap();
	let turn = match provider {
		Provider::Messages => sent["messages"][1]["content"].clone(),
		Provider::Responses => Value::Array(sent["input"].as_array().unwrap()[1..4].to_vec()),
		Provider::Gemini => sent["contents"][1]["parts"].clone(),
	};
	assert_eq!(without_status(turn), expected, "{sent}");
}

/// The client's output and accounting after the provider stream ends.
#[derive(Debug, PartialEq)]
struct Streamed {
	/// The data of each event.
	events: Vec<String>,
	early_failure: Option<HttpReply>,
	usage: Usage,
	tool_calls: u32,
	failed: bool,
}

fn streamed<'a>(provider: Provider, pieces: impl IntoIterator<Item = &'a [u8]>) -> Streamed {
	let mut driver = StreamReply::new(context(provider), MAX_EVENT_BYTES);
	let mut out = Vec::new();
	for piece in pieces {
		driver.push(piece).unwrap();
		out.extend(driver.take_output().unwrap_or_default());
	}
	driver.finish().unwrap();
	out.extend(driver.take_output().unwrap_or_default());
	let events = std::str::from_utf8(&out)
		.unwrap()
		.split_terminator("\n\n")
		.map(|event| event.strip_prefix("data: ").unwrap().to_string())
		.collect();
	Streamed {
		events,
		early_failure: driver.early_failure(),
		usage: driver.usage(),
		tool_calls: driver.telemetry().tool_calls,
		failed: driver.telemetry().failed,
	}
}

fn buffered(provider: Provider, body: &[u8]) -> (HttpReply, Usage) {
	let mut driver = BufferedReply::new(context(provider));
	driver.read(body).unwrap();
	let usage = driver.usage();
	(driver.into_reply(), usage)
}

const fn context(provider: Provider) -> ReplyContext<'static> {
	ReplyContext {
		client: ClientProtocol::ChatCompletions,
		upstream: upstream_protocol(provider),
		scope: SCOPE,
		routes: &[],
		show_reasoning: false,
		include_usage: true,
		created: CREATED,
		fallback_reply_id: "reply-1",
		stop_sequences: Vec::new(),
		max_held_bytes: MAX_HELD_BYTES,
	}
}

const fn upstream_protocol(provider: Provider) -> UpstreamProtocol {
	match provider {
		Provider::Messages => UpstreamProtocol::Messages,
		Provider::Responses => UpstreamProtocol::Responses {
			safety_identifier: false,
			reasoning_context: false,
		},
		Provider::Gemini => UpstreamProtocol::Gemini,
	}
}

fn chunks(events: &[String]) -> Vec<Value> {
	events
		.iter()
		.filter(|data| *data != "[DONE]")
		.map(|data| serde_json::from_str(data).unwrap())
		.collect()
}

/// A streamed reply as the message a client builds from it.
fn folded(events: &[String]) -> Value {
	let mut content = String::new();
	let mut reasoning = String::new();
	let mut calls = Vec::new();
	let mut finish_reason = Value::Null;
	for chunk in chunks(events) {
		let Some(choice) = chunk["choices"].get(0) else {
			continue;
		};
		let delta = &choice["delta"];
		content.push_str(delta["content"].as_str().unwrap_or(""));
		reasoning.push_str(delta["reasoning_content"].as_str().unwrap_or(""));
		for call in delta["tool_calls"].as_array().into_iter().flatten() {
			let mut call = call.clone();
			call.as_object_mut().unwrap().remove("index");
			calls.push(call);
		}
		if !choice["finish_reason"].is_null() {
			finish_reason = choice["finish_reason"].clone();
		}
	}
	json!({"content": content, "reasoning_content": reasoning, "tool_calls": calls,
		"finish_reason": finish_reason})
}

/// A whole reply in the shape of `folded`.
fn message(body: &Value) -> Value {
	let choice = &body["choices"][0];
	let message = &choice["message"];
	json!({"content": message["content"].as_str().unwrap_or(""),
		"reasoning_content": message["reasoning_content"].as_str().unwrap_or(""),
		"tool_calls": message.get("tool_calls").cloned().unwrap_or(json!([])),
		"finish_reason": choice["finish_reason"]})
}

/// Responses items may have a status, which isn't what's compared here.
fn without_status(mut items: Value) -> Value {
	for item in items.as_array_mut().unwrap() {
		item.as_object_mut().unwrap().remove("status");
	}
	items
}

const SCOPE: StateScope<'static> = StateScope {
	backend: "account",
	provider: "provider",
	model: MODEL,
};

/// A small deterministic hash for identifier tests.
struct IdentifierHash;

impl IdentifierPseudonymizer for IdentifierHash {
	fn pseudonymize(&self, identifier: &str) -> String {
		let hash = identifier.bytes().fold(0u8, |hash, byte| hash ^ byte);
		format!("{hash:02x}")
	}
}

/// The provider's stream as SSE, with an event name where the provider sends one.
fn provider_stream(provider: Provider, case: Case) -> Vec<u8> {
	let events = match provider {
		Provider::Messages => messages_events(case),
		Provider::Responses => responses_events(case),
		Provider::Gemini => gemini_chunks(case),
	};
	events
		.iter()
		.map(|event| match event["type"].as_str() {
			Some(name) => format!("event: {name}\ndata: {event}\n\n"),
			None => format!("data: {event}\n\n"),
		})
		.collect::<String>()
		.into_bytes()
}

fn provider_body(provider: Provider, case: Case) -> Vec<u8> {
	let body = match provider {
		Provider::Messages => messages_body(case),
		Provider::Responses => responses_events(case).pop().unwrap()["response"].take(),
		Provider::Gemini => gemini_body(case),
	};
	body.to_string().into_bytes()
}

fn messages_events(case: Case) -> Vec<Value> {
	let mut events = vec![json!({"type": "message_start", "message": {"id": "msg_1",
		"model": MODEL, "usage": {"input_tokens": 10, "cache_read_input_tokens": 20,
		"output_tokens": 1}}})];
	let text = |events: &mut Vec<Value>, deltas: &[&str]| {
		events.push(json!({"type": "content_block_start", "index": 0,
			"content_block": {"type": "text", "text": ""}}));
		for delta in deltas {
			events.push(json!({"type": "content_block_delta", "index": 0,
				"delta": {"type": "text_delta", "text": delta}}));
		}
	};
	let stop = match case {
		Case::Text => {
			text(&mut events, &["Hello", ", world"]);
			"end_turn"
		},
		Case::Cut => {
			text(&mut events, &["Hello", ", wor"]);
			"max_tokens"
		},
		Case::Failed => {
			text(&mut events, &["Hello"]);
			events.push(json!({"type": "error",
				"error": {"type": "overloaded_error", "message": "Overloaded"}}));
			return events;
		},
		Case::Tools => {
			events.push(json!({"type": "content_block_start", "index": 0,
				"content_block": {"type": "thinking", "thinking": "", "signature": ""}}));
			events.push(json!({"type": "content_block_delta", "index": 0,
				"delta": {"type": "thinking_delta", "thinking": "Plan."}}));
			events.push(json!({"type": "content_block_delta", "index": 0,
				"delta": {"type": "signature_delta", "signature": "c2lnbmVk"}}));
			events.push(json!({"type": "content_block_stop", "index": 0}));
			for (at, id) in [(1, "toolu_a"), (2, "toolu_b")] {
				events.push(json!({"type": "content_block_start", "index": at,
					"content_block": {"type": "tool_use", "id": id, "name": "lookup", "input": {}}}));
				events.push(json!({"type": "content_block_delta", "index": at,
					"delta": {"type": "input_json_delta", "partial_json": "{\"key\":"}}));
				events.push(json!({"type": "content_block_delta", "index": at,
					"delta": {"type": "input_json_delta", "partial_json": format!("{}}}", at - 1)}}));
				events.push(json!({"type": "content_block_stop", "index": at}));
			}
			events.push(
				json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
				"usage": {"output_tokens": 5}}),
			);
			events.push(json!({"type": "message_stop"}));
			return events;
		},
	};
	events.push(json!({"type": "content_block_stop", "index": 0}));
	events.push(
		json!({"type": "message_delta", "delta": {"stop_reason": stop},
		"usage": {"output_tokens": 5}}),
	);
	events.push(json!({"type": "message_stop"}));
	events
}

fn messages_body(case: Case) -> Value {
	let (content, stop) = match case {
		Case::Text => (
			json!([{"type": "text", "text": "Hello, world"}]),
			"end_turn",
		),
		Case::Cut => (
			json!([{"type": "text", "text": "Hello, wor"}]),
			"max_tokens",
		),
		Case::Tools => (
			json!([{"type": "thinking", "thinking": "Plan.", "signature": "c2lnbmVk"},
				{"type": "tool_use", "id": "toolu_a", "name": "lookup", "input": {"key": 0}},
				{"type": "tool_use", "id": "toolu_b", "name": "lookup", "input": {"key": 1}}]),
			"tool_use",
		),
		Case::Failed => unreachable!("a failure isn't a whole reply"),
	};
	json!({"id": "msg_1", "type": "message", "role": "assistant", "model": MODEL,
		"content": content, "stop_reason": stop,
		"usage": {"input_tokens": 10, "cache_read_input_tokens": 20, "output_tokens": 5}})
}

fn responses_events(case: Case) -> Vec<Value> {
	let response = |status: &str, output: &[Value]| {
		let mut response = json!({"id": "resp_1", "object": "response", "created_at": 1,
			"model": MODEL, "status": status, "output": output,
			"usage": {"input_tokens": 30, "input_tokens_details": {"cached_tokens": 20},
				"output_tokens": 5, "output_tokens_details": {"reasoning_tokens": 0},
				"total_tokens": 35}});
		if status == "incomplete" {
			response["incomplete_details"] = json!({"reason": "max_output_tokens"});
		}
		response
	};
	let mut events =
		vec![json!({"type": "response.created", "response": response("in_progress", &[])})];
	let mut output = Vec::new();
	let text = |events: &mut Vec<Value>, output: &mut Vec<Value>, deltas: &[&str], done: bool| {
		let text = deltas.concat();
		let part = json!({"type": "output_text", "text": text, "annotations": []});
		let item = json!({"id": "msg_0", "type": "message", "role": "assistant",
			"status": "completed", "content": [part]});
		events.push(
			json!({"type": "response.output_item.added", "output_index": 0,
			"item": {"id": "msg_0", "type": "message", "role": "assistant",
				"status": "in_progress", "content": []}}),
		);
		events.push(
			json!({"type": "response.content_part.added", "output_index": 0,
			"content_index": 0, "item_id": "msg_0",
			"part": {"type": "output_text", "text": "", "annotations": []}}),
		);
		for delta in deltas {
			events.push(
				json!({"type": "response.output_text.delta", "output_index": 0,
				"content_index": 0, "item_id": "msg_0", "delta": delta}),
			);
		}
		if done {
			events.push(
				json!({"type": "response.output_text.done", "output_index": 0,
				"content_index": 0, "item_id": "msg_0", "text": text}),
			);
			events.push(
				json!({"type": "response.content_part.done", "output_index": 0,
				"content_index": 0, "item_id": "msg_0", "part": part}),
			);
			events.push(
				json!({"type": "response.output_item.done", "output_index": 0,
				"item": item}),
			);
			output.push(item);
		}
	};
	let terminal = match case {
		Case::Text => {
			text(&mut events, &mut output, &["Hello", ", world"], true);
			("response.completed", "completed")
		},
		Case::Cut => {
			text(&mut events, &mut output, &["Hello", ", wor"], true);
			("response.incomplete", "incomplete")
		},
		Case::Failed => {
			text(&mut events, &mut output, &["Hello"], false);
			events.push(json!({"type": "error", "code": "server_is_overloaded",
				"message": "Overloaded", "param": null}));
			return numbered(events);
		},
		Case::Tools => {
			let reasoning = json!({"type": "reasoning", "id": "rs_0",
				"summary": [{"type": "summary_text", "text": "Plan."}],
				"encrypted_content": "c2lnbmVk", "status": "completed"});
			events.push(
				json!({"type": "response.output_item.added", "output_index": 0,
				"item": {"type": "reasoning", "id": "rs_0", "summary": []}}),
			);
			events.push(
				json!({"type": "response.output_item.done", "output_index": 0,
				"item": reasoning}),
			);
			output.push(reasoning);
			for (at, call) in [(1, "a"), (2, "b")] {
				let arguments = json!({"key": at - 1}).to_string();
				let item = json!({"id": format!("fc_{call}"), "type": "function_call",
					"call_id": format!("call_{call}"), "name": "lookup", "arguments": arguments,
					"status": "completed"});
				let mut start = item.clone();
				start["arguments"] = json!("");
				start["status"] = json!("in_progress");
				events.push(
					json!({"type": "response.output_item.added", "output_index": at,
					"item": start}),
				);
				events.push(json!({"type": "response.function_call_arguments.delta",
					"output_index": at, "item_id": format!("fc_{call}"), "delta": arguments}));
				events.push(json!({"type": "response.function_call_arguments.done",
					"output_index": at, "item_id": format!("fc_{call}"), "arguments": arguments}));
				events.push(
					json!({"type": "response.output_item.done", "output_index": at,
					"item": item}),
				);
				output.push(item);
			}
			("response.completed", "completed")
		},
	};
	let (name, status) = terminal;
	events.push(json!({"type": name, "response": response(status, &output)}));
	numbered(events)
}

fn numbered(mut events: Vec<Value>) -> Vec<Value> {
	for (sequence, event) in events.iter_mut().enumerate() {
		event["sequence_number"] = sequence.into();
	}
	events
}

fn gemini_chunks(case: Case) -> Vec<Value> {
	let chunk = |parts: Value, finish: Option<&str>| {
		let mut chunk = json!({"candidates": [{"content": {"role": "model", "parts": parts}}],
			"responseId": "g_1", "modelVersion": MODEL});
		if let Some(finish) = finish {
			chunk["candidates"][0]["finishReason"] = json!(finish);
			chunk["usageMetadata"] = json!({"promptTokenCount": 30, "candidatesTokenCount": 5,
				"cachedContentTokenCount": 20, "totalTokenCount": 35});
		}
		chunk
	};
	match case {
		Case::Text => vec![
			chunk(json!([{"text": "Hello"}]), None),
			chunk(json!([{"text": ", world"}]), Some("STOP")),
		],
		Case::Cut => vec![
			chunk(json!([{"text": "Hello"}]), None),
			chunk(json!([{"text": ", wor"}]), Some("MAX_TOKENS")),
		],
		Case::Failed => vec![
			chunk(json!([{"text": "Hello"}]), None),
			json!({"error": {"code": 503, "message": "Overloaded", "status": "UNAVAILABLE"}}),
		],
		Case::Tools => vec![
			chunk(json!([{"text": "Plan.", "thought": true}]), None),
			chunk(
				json!([{"functionCall": {"name": "lookup", "args": {"key": 0}},
						"thoughtSignature": "c2lnbmVk"},
					{"functionCall": {"name": "lookup", "args": {"key": 1}}}]),
				Some("STOP"),
			),
		],
	}
}

/// The stream's chunks as one reply.
fn gemini_body(case: Case) -> Value {
	let chunks = gemini_chunks(case);
	let mut body = chunks.last().unwrap().clone();
	let parts: Vec<Value> = chunks
		.iter()
		.flat_map(|chunk| {
			chunk["candidates"][0]["content"]["parts"]
				.as_array()
				.unwrap()
				.clone()
		})
		.collect();
	body["candidates"][0]["content"]["parts"] = Value::Array(parts);
	body
}

struct UppercaseA;

impl ReplyRewrite for UppercaseA {
	fn text(&mut self, _: ReplyPlace, text: &str, out: &mut String) {
		out.push_str(&text.replace('a', "A"));
	}
}

fn messages_text_reply(parts: &[String], output_tokens: u64) -> (Vec<u8>, Vec<u8>) {
	let message = json!({"id": "msg_1", "type": "message", "role": "assistant", "model": MODEL,
		"content": [{"type": "text", "text": parts.concat()}],
		"stop_reason": "end_turn", "stop_sequence": null,
		"usage": {"input_tokens": 10, "cache_read_input_tokens": 20, "output_tokens": output_tokens}});
	let mut events = vec![
		json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
			"role": "assistant", "model": MODEL, "content": [],
			"usage": {"input_tokens": 10, "cache_read_input_tokens": 20, "output_tokens": 0}}}),
		json!({"type": "content_block_start", "index": 0,
			"content_block": {"type": "text", "text": ""}}),
	];
	for text in parts {
		events.push(json!({"type": "content_block_delta", "index": 0,
			"delta": {"type": "text_delta", "text": text}}));
	}
	events.extend([
		json!({"type": "content_block_stop", "index": 0}),
		json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
			"usage": {"output_tokens": output_tokens}}),
		json!({"type": "message_stop"}),
	]);
	let stream = events.iter().fold(String::new(), |mut stream, event| {
		write!(
			stream,
			"event: {}\ndata: {event}\n\n",
			event["type"].as_str().unwrap()
		)
		.unwrap();
		stream
	});
	(stream.into_bytes(), message.to_string().into_bytes())
}
