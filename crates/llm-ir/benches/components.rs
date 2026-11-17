//! Schema and tool-call costs, isolated from conversation size.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use agent_llm_ir::{RenderSettings, prepare, prepare_parsed};
use divan::{Bencher, black_box};
use serde_json::{Value, json};

#[path = "support/workload.rs"]
mod workload;

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
	divan::main();
}

#[divan::bench(args = [4, 32, 128], min_time = 0.1, max_time = 1)]
fn gemini_schemas(b: Bencher, tools: usize) {
	let pair = workload::Pair::new(workload::Protocol::Messages, workload::Protocol::Gemini);
	let schema = json!({
			"$defs": {"Name": {"type": "string", "description": "A quoted \"name\" 日本語"}},
			"allOf": [
					{"type": "object", "properties": {"name": {"anyOf": [{"$ref": "#/$defs/Name"}, {"type": "null"}]}}, "required": ["name"]},
					{"properties": {"tags": {"type": "array", "items": {"type": ["string", "null"]}}}, "required": ["tags"]}
			]
	});
	let tools: Vec<_> = (0..tools)
		.map(|i| json!({"name": format!("tool_{i}"), "input_schema": schema}))
		.collect();
	let body = serde_json::to_vec(&json!({"model": "m", "max_tokens": 8192, "tools": tools, "messages": [{"role": "user", "content": "hi"}]})).unwrap();
	let convert = || {
		prepare(black_box(&body), pair.client.client(), pair.upstream())
			.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
			.unwrap()
	};
	let result: Value = serde_json::from_str(&convert().body).unwrap();
	let declarations = result["tools"][0]["functionDeclarations"]
		.as_array()
		.unwrap();
	assert_eq!(declarations.len(), tools.len());
	assert_eq!(
		declarations[0]["parameters"],
		json!({
				"type": "object",
				"properties": {
						"name": {"type": "string", "description": "A quoted \"name\" 日本語", "nullable": true},
						"tags": {"type": "array", "items": {"type": "string", "nullable": true}}
				},
				"required": ["name", "tags"]
		})
	);
	let parsed: Value = serde_json::from_slice(&body).unwrap();
	let converted = prepare_parsed(&parsed, pair.client.client(), pair.upstream())
		.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
		.unwrap();
	assert_eq!(
		serde_json::from_str::<Value>(&converted.body).unwrap(),
		result
	);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(convert);
}

#[divan::bench(args = [1, 16, 128], min_time = 0.1, max_time = 1)]
fn tool_calls_buffered(b: Bencher, count: u32) {
	let events = call_events(count);
	let context =
		workload::Pair::new(workload::Protocol::Chat, workload::Protocol::Messages).reply_context();
	let run = || {
		let mut writer = agent_llm_ir::protocols::chat_completions::writer::BufferedChat::new(
			context.scope,
			&[],
			1,
			usize::MAX,
		);
		for event in black_box(&events) {
			writer.event(event);
		}
		writer.into_reply()
	};
	let result: Value = serde_json::from_slice(&run().body).unwrap();
	assert_eq!(
		result["choices"][0]["message"]["tool_calls"]
			.as_array()
			.unwrap()
			.len(),
		count as usize
	);
	assert_eq!(
		result["choices"][0]["message"]["tool_calls"][0]["function"],
		json!({"name": "read_file", "arguments": r#"{"path":"a\nb.rs","limit":400}"#})
	);
	b.bench_local(run);
}

#[divan::bench(args = [1, 16, 128], min_time = 0.1, max_time = 1)]
fn tool_calls_stream(b: Bencher, count: u32) {
	let events = call_events(count);
	let context =
		workload::Pair::new(workload::Protocol::Chat, workload::Protocol::Messages).reply_context();
	let mut check = agent_llm_ir::protocols::chat_completions::writer::ChatStream::new(
		context.scope,
		&[],
		1,
		usize::MAX,
	);
	for event in &events {
		check.event(event);
	}
	let output = check.take_output();
	let data = workload::sse_data(&output);
	let calls: Vec<_> = data
		.iter()
		.filter_map(|event| event["choices"][0]["delta"]["tool_calls"].as_array())
		.flatten()
		.collect();
	assert_eq!(calls.len(), count as usize);
	for (index, call) in calls.iter().enumerate() {
		assert_eq!(call["index"], index);
		assert_eq!(
			call["function"],
			json!({"name": "read_file", "arguments": r#"{"path":"a\nb.rs","limit":400}"#})
		);
	}
	b.bench_local(|| {
		let mut writer = agent_llm_ir::protocols::chat_completions::writer::ChatStream::new(
			context.scope,
			&[],
			1,
			usize::MAX,
		);
		let mut sent = 0;
		for event in black_box(&events) {
			writer.event(event);
			sent += writer.take_output().len();
		}
		assert!(writer.early_failure().is_none());
		sent
	});
}

fn call_events(count: u32) -> Vec<agent_llm_ir::ir::events::Event<'static>> {
	use agent_llm_ir::ir::events::{Arguments, Event, Final, JsonStr, Outcome, PartId, PartKind};
	let raw: &serde_json::value::RawValue =
		serde_json::from_str(r#""{\"path\":\"a\\nb.rs\",\"limit\":400}""#).unwrap();
	let arguments = Arguments::complete(JsonStr::new(raw).unwrap()).unwrap();
	let mut events = Vec::new();
	for id in 0..count {
		events.push(Event::PartStart {
			part: PartId(id),
			kind: PartKind::ToolCall {
				call_id: "call_1",
				name: "read_file",
			},
		});
		events.push(Event::PartEnd {
			part: PartId(id),
			value: Final::ToolCall(arguments),
		});
	}
	events.push(Event::Finish(Outcome::Completed));
	events
}
