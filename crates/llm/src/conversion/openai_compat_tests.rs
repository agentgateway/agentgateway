use bytes::Bytes;
use serde_json::{Value, json};

use super::to_responses;

fn translate(input: &[u8]) -> Value {
	let response =
		to_responses::translate_response(&Bytes::copy_from_slice(input), "input-model", None)
			.expect("Chat Completions response should translate");
	serde_json::from_slice(&response.serialize().expect("response should serialize"))
		.expect("translated response should be JSON")
}

async fn translate_stream(input: &'static str) -> Vec<Value> {
	use http_body_util::BodyExt;
	let body = to_responses::translate_stream(
		agent_http::Body::from(input),
		1 << 20,
		crate::StreamingUsageGuard::default(),
		crate::LogContentFields::default(),
		None,
	);
	let bytes = body.collect().await.unwrap().to_bytes();
	std::str::from_utf8(&bytes)
		.unwrap()
		.lines()
		.filter_map(|l| l.strip_prefix("data: "))
		.map(|d| serde_json::from_str(d).unwrap())
		.collect()
}

const STREAMED_CALL: &str = r#"data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_up","type":"function","function":{"name":"f","arguments":"{\"a\""}}]}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}

data: [DONE]

"#;

#[tokio::test]
async fn streamed_calls_keep_the_upstream_call_id() {
	let events = translate_stream(STREAMED_CALL).await;
	let added = events
		.iter()
		.find(|e| e["type"] == "response.output_item.added" && e["item"]["type"] == "function_call")
		.unwrap();
	let item_id = added["item"]["id"].as_str().unwrap();
	assert_ne!(item_id, "call_up");
	assert_eq!(added["item"]["call_id"], "call_up");

	let deltas: Vec<_> = events
		.iter()
		.filter(|e| e["type"] == "response.function_call_arguments.delta")
		.map(|e| &e["item_id"])
		.collect();
	assert_eq!(deltas, [item_id, item_id]);

	let done = events
		.iter()
		.find(|e| e["type"] == "response.completed")
		.unwrap();
	let call = &done["response"]["output"][1];
	assert_eq!(
		(&call["id"], &call["call_id"]),
		(&json!(item_id), &json!("call_up"))
	);
}

#[tokio::test]
async fn a_late_upstream_id_does_not_change_the_call_id() {
	let events = translate_stream(
		r#"data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"f","arguments":"{\"a\""}}]}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_up","function":{"arguments":":1}"}}]}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}

data: [DONE]

"#,
	)
	.await;
	let mut call_ids: Vec<_> = events
		.iter()
		.filter(|e| e["item"]["type"] == "function_call")
		.map(|e| &e["item"]["call_id"])
		.collect();
	call_ids.push(&events.last().unwrap()["response"]["output"][1]["call_id"]);
	assert_eq!(call_ids.len(), 3);
	assert!(call_ids.iter().all(|id| *id == call_ids[0]), "{call_ids:?}");
}

const CUT_OFF_REPLY: &str = r#"{"id":"c","object":"chat.completion","created":0,"model":"m","choices":[{"index":0,"message":{"role":"assistant","reasoning_content":"thinking","content":"par","tool_calls":[{"id":"call_up","type":"function","function":{"name":"f","arguments":"{\"a\":"}}]},"finish_reason":"length"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;

const CUT_OFF_STREAM: &str = r#"data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"reasoning_content":"thinking"}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"content":"par"}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_up","type":"function","function":{"name":"f","arguments":"{\"a\":"}}]}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"length"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}

data: [DONE]

"#;

fn statuses(output: &Value) -> Vec<(&str, &str)> {
	output
		.as_array()
		.unwrap()
		.iter()
		.map(|i| (i["type"].as_str().unwrap(), i["status"].as_str().unwrap()))
		.collect()
}

#[test]
fn a_cut_off_reply_marks_its_items_incomplete() {
	let response = translate(CUT_OFF_REPLY.as_bytes());
	assert_eq!(response["status"], "incomplete");
	assert_eq!(
		statuses(&response["output"]),
		[
			("reasoning", "incomplete"),
			("message", "incomplete"),
			("function_call", "incomplete")
		]
	);
}

#[tokio::test]
async fn a_cut_off_stream_marks_its_items_incomplete() {
	let events = translate_stream(CUT_OFF_STREAM).await;
	let done: Vec<_> = events
		.iter()
		.filter(|e| e["type"] == "response.output_item.done")
		.map(|e| {
			(
				e["item"]["type"].as_str().unwrap(),
				e["item"]["status"].as_str().unwrap(),
			)
		})
		.collect();
	assert_eq!(
		done,
		[
			("reasoning", "incomplete"),
			("function_call", "incomplete"),
			("message", "incomplete")
		]
	);
	let end = events.last().unwrap();
	assert_eq!(end["type"], "response.incomplete");
	assert_eq!(
		statuses(&end["response"]["output"]),
		[
			("reasoning", "incomplete"),
			("message", "incomplete"),
			("function_call", "incomplete")
		]
	);
}

fn default_tool_ids(b: agent_http::Body) -> agent_http::Body {
	to_responses::translate_stream(
		b,
		1 << 20,
		crate::StreamingUsageGuard::default(),
		crate::LogContentFields::default(),
		None,
	)
}

fn split_tool_ids(b: agent_http::Body) -> agent_http::Body {
	to_responses::translate_stream_with_tool_id(
		b,
		1 << 20,
		crate::StreamingUsageGuard::default(),
		crate::LogContentFields::default(),
		None,
		|id| {
			let (base, state) = id.split_once("__thought__").unwrap();
			(base.to_string(), Some(state.to_string()))
		},
	)
}

/// The id repeats on the second argument delta, as some engines send it
#[rstest::rstest]
#[case::unchanged_by_default(default_tool_ids, "call_1__thought__sig", None, &[
	("response.output_item.added", "message"),
	("response.output_item.added", "function_call"),
	("response.output_item.done", "function_call"),
	("response.output_item.done", "message"),
])]
#[case::state_sent_once(split_tool_ids, "call_1", Some("sig"), &[
	("response.output_item.added", "message"),
	("response.output_item.added", "reasoning"),
	("response.output_item.done", "reasoning"),
	("response.output_item.added", "function_call"),
	("response.output_item.done", "function_call"),
	("response.output_item.done", "message"),
])]
#[tokio::test]
async fn streamed_tool_ids(
	#[case] convert: fn(agent_http::Body) -> agent_http::Body,
	#[case] call_id: &str,
	#[case] state: Option<&str>,
	#[case] items: &[(&str, &str)],
) {
	use http_body_util::BodyExt;
	let input = r#"data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1__thought__sig","type":"function","function":{"name":"f","arguments":"{\"a\""}}]}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1__thought__sig","function":{"arguments":":1}"}}]}}]}

data: {"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}

data: [DONE]

"#;
	let bytes = convert(agent_http::Body::from(input))
		.collect()
		.await
		.unwrap()
		.to_bytes();
	let events: Vec<Value> = std::str::from_utf8(&bytes)
		.unwrap()
		.lines()
		.filter_map(|l| l.strip_prefix("data: "))
		.map(|d| serde_json::from_str(d).unwrap())
		.collect();
	let seen: Vec<_> = events
		.iter()
		.filter(|e| e["item"].is_object())
		.map(|e| {
			(
				e["type"].as_str().unwrap(),
				e["item"]["type"].as_str().unwrap(),
			)
		})
		.collect();
	assert_eq!(seen, items);
	let output = events.last().unwrap()["response"]["output"]
		.as_array()
		.unwrap();
	let call = output
		.iter()
		.find(|i| i["type"] == "function_call")
		.unwrap();
	assert_eq!(call["call_id"], call_id);
	let reasoning: Vec<_> = output
		.iter()
		.filter(|i| i["type"] == "reasoning")
		.map(|i| i["encrypted_content"].as_str().unwrap())
		.collect();
	assert_eq!(reasoning, Vec::from_iter(state));
}
