use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum_core::body::Body;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};

use super::{from_responses, to_responses};

fn translate(input: &[u8]) -> Value {
	let response = to_responses::translate_response(&Bytes::copy_from_slice(input), "input-model")
		.expect("Chat Completions response should translate");
	serde_json::from_slice(&response.serialize().expect("response should serialize"))
		.expect("translated response should be JSON")
}

#[test]
fn buffered_failed_response_preserves_error() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["finish_reason"] = json!("content_filter");
	let input = serde_json::to_vec(&input).unwrap();

	let response = translate(&input);

	assert_eq!(response["status"], json!("failed"));
	assert_eq!(
		response["error"],
		json!({"code": "content_filter", "message": "Content filtered"})
	);
}

fn codex_request() -> crate::types::responses::Request {
	serde_json::from_slice(include_bytes!(
		"../tests/requests/responses/codex-tools.json"
	))
	.unwrap()
}

#[test]
fn responses_image_is_preserved_in_chat_completions_translation() {
	let request: crate::types::responses::Request = serde_json::from_value(json!({
		"model": "moonshotai/Kimi-K3",
		"input": [
			{"role": "user", "content": [
				{"type": "input_text", "text": "Describe this image."},
				{"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8=", "detail": "high"}
			]},
			{"type": "message", "role": "user", "content": [
				{"type": "input_text", "text": "Describe this second image."},
				{"type": "input_image", "image_url": "https://example.com/image.png", "detail": "low"}
			]}
		]
	}))
	.unwrap();

	let translated = from_responses::translate_request(&request).unwrap();
	let translated = serde_json::to_value(translated).unwrap();
	let content = &translated["messages"][0]["content"];
	assert_eq!(content[0]["text"], "Describe this image.");
	assert_eq!(
		content[1]["image_url"]["url"],
		"data:image/png;base64,aGVsbG8="
	);
	assert_eq!(content[1]["image_url"]["detail"], "high");
	let item_content = &translated["messages"][1]["content"];
	assert_eq!(item_content[0]["text"], "Describe this second image.");
	assert_eq!(
		item_content[1]["image_url"]["url"],
		"https://example.com/image.png"
	);
	assert_eq!(item_content[1]["image_url"]["detail"], "low");
}

#[test]
fn responses_file_backed_image_is_rejected_instead_of_dropped() {
	let request: crate::types::responses::Request = serde_json::from_value(json!({
		"model": "moonshotai/Kimi-K3",
		"input": [{"role": "user", "content": [
			{"type": "input_image", "file_id": "file_123", "detail": "auto"}
		]}]
	}))
	.unwrap();
	let err = from_responses::translate_request(&request).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
}

fn chunk(delta: Value, finish: Value) -> Value {
	json!({"id": "chatcmpl_test", "object": "chat.completion.chunk", "created": 1,
		"model": "moonshotai/Kimi-K3", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
}

async fn stream(chunks: Vec<Value>, done: bool, limit: usize) -> Vec<Value> {
	let (_, tools) = from_responses::translate_request_with_context(&codex_request()).unwrap();
	let mut input: String = chunks
		.into_iter()
		.map(|c| format!("data: {c}\n\n"))
		.collect();
	if done {
		input.push_str("data: [DONE]\n\n");
	}
	let body = to_responses::translate_stream_with_context(
		axum_core::body::Body::from(input),
		limit,
		Default::default(),
		Default::default(),
		tools,
	);
	let bytes = body.collect().await.unwrap().to_bytes();
	String::from_utf8(bytes.to_vec())
		.unwrap()
		.split("\n\n")
		.filter(|s| !s.is_empty())
		.map(|frame| {
			let mut lines = frame.lines();
			let event = lines.next().unwrap().strip_prefix("event: ").unwrap();
			let data: Value =
				serde_json::from_str(lines.next().unwrap().strip_prefix("data: ").unwrap()).unwrap();
			assert_eq!(
				event, data["type"],
				"SSE event names must match their JSON type"
			);
			data
		})
		.collect()
}

fn translated_body(body: Body) -> Body {
	let (_, tools) = from_responses::translate_request_with_context(&codex_request()).unwrap();
	to_responses::translate_stream_with_context(
		body,
		1024 * 1024,
		Default::default(),
		Default::default(),
		tools,
	)
}

fn wire_chunk(delta: Value, finish: Value) -> Bytes {
	Bytes::from(format!("data: {}\n\n", chunk(delta, finish)))
}

fn wire_events(wire: &str) -> Vec<Value> {
	wire
		.split("\n\n")
		.filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("data: ")))
		.map(|data| serde_json::from_str(data).expect("complete JSON data event"))
		.collect()
}

fn delayed_body(frames: Vec<Bytes>, delay: Duration) -> Body {
	Body::from_stream(
		futures_util::stream::iter(frames).then(move |frame| async move {
			tokio::time::sleep(delay).await;
			Ok::<_, std::io::Error>(frame)
		}),
	)
}

#[tokio::test(start_paused = true)]
async fn long_reasoning_and_tool_arguments_keep_connection_alive_without_executing_partial_tools() {
	for reasoning_only in [true, false] {
		let input = "*** Begin Patch\n+\"привет\\世界\"\n*** End Patch";
		let arguments = json!({"input": input}).to_string();
		let pieces: Vec<_> = arguments.chars().collect();
		let mut frames = Vec::new();
		for i in 0..6 {
			let delta = if reasoning_only {
				json!({"reasoning_content": "thinking "})
			} else {
				let piece: String = pieces[i * pieces.len() / 6..(i + 1) * pieces.len() / 6]
					.iter()
					.collect();
				let mut tool = json!({"index": 0, "function": {"arguments": piece}});
				if i == 0 {
					tool["id"] = json!("call_slow");
					tool["function"]["name"] = json!("agw_tool_2");
				}
				json!({"tool_calls": [tool]})
			};
			frames.push(wire_chunk(delta, Value::Null));
		}
		frames.push(wire_chunk(
			json!({}),
			json!(if reasoning_only { "stop" } else { "tool_calls" }),
		));
		frames.push(Bytes::from_static(b"data: [DONE]\n\n"));
		let start = tokio::time::Instant::now();
		let mut last = start;
		let mut body = translated_body(delayed_body(frames, Duration::from_secs(60)));
		let mut wire = String::new();
		while let Some(frame) = body.frame().await {
			let bytes = frame.unwrap().into_data().unwrap();
			let now = tokio::time::Instant::now();
			assert!(
				now - last <= Duration::from_secs(15),
				"downstream went silent"
			);
			last = now;
			wire.push_str(std::str::from_utf8(&bytes).unwrap());
			if now - start < Duration::from_secs(480) {
				assert!(!wire.contains("response.output_item.done"));
				assert!(!wire.contains("response.custom_tool_call_input"));
			}
		}
		assert_eq!(last - start, Duration::from_secs(480));
		assert!(wire.matches(": keepalive\n\n").count() >= 24);
		let events = wire_events(&wire);
		for (i, event) in events.iter().enumerate() {
			assert_eq!(event["sequence_number"], i + 1);
		}
		assert_eq!(events.last().unwrap()["type"], "response.completed");
		let item = &events.last().unwrap()["response"]["output"][0];
		if reasoning_only {
			assert_eq!(item["content"][0]["text"], "thinking ".repeat(6));
		} else {
			assert_eq!(item["input"], input);
			assert_eq!(
				wire
					.matches("event: response.custom_tool_call_input.done")
					.count(),
				1
			);
		}
	}
}

#[tokio::test(start_paused = true)]
async fn silent_upstream_fails_after_five_minutes_despite_keepalives() {
	let start = tokio::time::Instant::now();
	let upstream =
		Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
	let wire = translated_body(upstream)
		.collect()
		.await
		.unwrap()
		.to_bytes();
	let wire = std::str::from_utf8(&wire).unwrap();
	assert_eq!(
		tokio::time::Instant::now() - start,
		Duration::from_secs(300)
	);
	assert_eq!(wire.matches(": keepalive\n\n").count(), 19);
	let events = wire_events(wire);
	assert_eq!(events.len(), 1);
	assert_eq!(events[0]["type"], "response.failed");
	assert_eq!(events[0]["response"]["error"]["code"], "upstream_timeout");
	assert_eq!(
		events[0]["response"]["error"]["message"],
		"upstream SSE stream made no progress for 300 seconds"
	);
	assert_eq!(events[0]["response"]["error"]["retryable"], true);
	assert_eq!(events[0]["response"]["error"]["recovery"], "retry_request");
	assert_eq!(events[0]["response"]["error"]["partial_output"], false);
}

#[tokio::test(start_paused = true)]
async fn empty_data_and_metadata_events_cannot_extend_progress_deadline() {
	let mut empty_choices = chunk(json!({}), Value::Null);
	empty_choices["choices"] = json!([]);
	let mut usage_only = empty_choices.clone();
	usage_only["usage"] = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
	for event in [
		empty_choices,
		usage_only,
		chunk(json!({}), Value::Null),
		chunk(json!({"role": "assistant"}), Value::Null),
		chunk(
			json!({"content": "", "reasoning_content": "", "tool_calls": []}),
			Value::Null,
		),
		chunk(
			json!({"tool_calls": [{"index": 0, "id": "", "function": {"name": "", "arguments": ""}}]}),
			Value::Null,
		),
	] {
		let start = tokio::time::Instant::now();
		let frames = vec![Bytes::from(format!("data: {event}\n\n")); 30];
		let bytes = translated_body(delayed_body(frames, Duration::from_secs(30)))
			.collect()
			.await
			.unwrap()
			.to_bytes();
		let wire = std::str::from_utf8(&bytes).unwrap();
		assert_eq!(
			tokio::time::Instant::now() - start,
			Duration::from_secs(300),
			"{event}"
		);
		assert!(wire.contains(": keepalive\n\n"));
		assert!(!wire.contains("response.output_item.done"));
		let events = wire_events(wire);
		assert_eq!(
			events.last().unwrap()["response"]["error"]["code"],
			"upstream_timeout"
		);
	}
}

#[tokio::test(start_paused = true)]
async fn finish_reason_counts_once_and_usage_does_not_extend_it() {
	for send_done in [true, false] {
		let start = tokio::time::Instant::now();
		let prefix = delayed_body(
			vec![wire_chunk(json!({}), json!("stop"))],
			Duration::from_secs(240),
		);
		let usage = json!({"id": "chatcmpl_test", "model": "kimi", "created": 1, "choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}});
		let mut suffix =
			vec![Bytes::from(format!("data: {usage}\n\n")); if send_done { 1 } else { 30 }];
		if send_done {
			suffix.push(Bytes::from_static(b"data: [DONE]\n\n"));
		}
		let body = Body::from_stream(
			prefix
				.into_data_stream()
				.chain(delayed_body(suffix, Duration::from_secs(30)).into_data_stream()),
		);
		let bytes = translated_body(body).collect().await.unwrap().to_bytes();
		let events = wire_events(std::str::from_utf8(&bytes).unwrap());
		let last = events.last().unwrap();
		if send_done {
			assert_eq!(
				tokio::time::Instant::now() - start,
				Duration::from_secs(300)
			);
			assert_eq!(last["type"], "response.completed");
			assert_eq!(last["response"]["usage"]["total_tokens"], 15);
		} else {
			assert_eq!(
				tokio::time::Instant::now() - start,
				Duration::from_secs(540)
			);
			assert_eq!(last["type"], "response.failed");
			assert_eq!(last["response"]["error"]["code"], "upstream_timeout");
		}
	}
}

#[tokio::test(start_paused = true)]
async fn upstream_comments_and_partial_frames_cannot_extend_progress_deadline() {
	for fragment in [b": upstream heartbeat\n\n".as_slice(), b" ".as_slice()] {
		let start = tokio::time::Instant::now();
		let upstream = delayed_body(
			vec![Bytes::copy_from_slice(fragment); 20],
			Duration::from_secs(30),
		);
		let bytes = translated_body(upstream)
			.collect()
			.await
			.unwrap()
			.to_bytes();
		let wire = std::str::from_utf8(&bytes).unwrap();
		assert_eq!(
			tokio::time::Instant::now() - start,
			Duration::from_secs(300)
		);
		assert!(wire.contains(": keepalive\n\n"));
		assert_eq!(wire_events(wire).last().unwrap()["type"], "response.failed");
		assert_eq!(
			wire_events(wire).last().unwrap()["response"]["error"]["code"],
			"upstream_timeout"
		);
	}
}

#[tokio::test(start_paused = true)]
async fn idle_deadline_resets_after_valid_upstream_progress() {
	let start = tokio::time::Instant::now();
	let dropped = Arc::new(AtomicBool::new(false));
	let prefix = delayed_body(
		vec![wire_chunk(
			json!({"reasoning_content": "progress"}),
			Value::Null,
		)],
		Duration::from_secs(240),
	);
	let body = Body::from_stream(
		prefix
			.into_data_stream()
			.chain(pending_body_after(Vec::new(), dropped.clone()).into_data_stream()),
	);
	let bytes = translated_body(body).collect().await.unwrap().to_bytes();
	assert_eq!(
		tokio::time::Instant::now() - start,
		Duration::from_secs(540)
	);
	assert_eq!(
		wire_events(std::str::from_utf8(&bytes).unwrap())
			.last()
			.unwrap()["type"],
		"response.failed"
	);
	assert!(dropped.load(Ordering::Relaxed));
}

#[tokio::test(start_paused = true)]
async fn ordinary_text_progress_does_not_add_unnecessary_keepalives() {
	let frames = vec![
		wire_chunk(json!({"content": "ordinary "}), Value::Null),
		wire_chunk(json!({"content": "text"}), json!("stop")),
		Bytes::from_static(b"data: [DONE]\n\n"),
	];
	let bytes = translated_body(delayed_body(frames, Duration::from_secs(5)))
		.collect()
		.await
		.unwrap()
		.to_bytes();
	let wire = std::str::from_utf8(&bytes).unwrap();
	assert!(!wire.contains("keepalive"));
	assert_eq!(
		wire_events(wire).last().unwrap()["response"]["output"][0]["content"][0]["text"],
		"ordinary text"
	);
}

#[tokio::test(start_paused = true)]
async fn heartbeat_does_not_corrupt_a_fragmented_upstream_frame() {
	let full = wire_chunk(json!({"content": "hello"}), json!("stop"));
	let split = full.len() / 2;
	let frames = vec![
		full.slice(..split),
		full.slice(split..),
		Bytes::from_static(b"data: [DONE]\n\n"),
	];
	let bytes = translated_body(delayed_body(frames, Duration::from_secs(60)))
		.collect()
		.await
		.unwrap()
		.to_bytes();
	let wire = std::str::from_utf8(&bytes).unwrap();
	assert!(wire.contains(": keepalive\n\n"));
	let events = wire_events(wire);
	assert_eq!(
		events.last().unwrap()["response"]["output"][0]["content"][0]["text"],
		"hello"
	);
}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
	fn drop(&mut self) {
		self.0.store(true, Ordering::Relaxed);
	}
}

fn pending_body_after(
	prefix: Vec<Result<Bytes, std::io::Error>>,
	dropped: Arc<AtomicBool>,
) -> Body {
	let guard = Dropped(dropped);
	let pending = futures_util::stream::poll_fn(move |_| {
		let _ = &guard;
		std::task::Poll::Pending::<Option<Result<Bytes, std::io::Error>>>
	});
	Body::from_stream(futures_util::stream::iter(prefix).chain(pending))
}

#[tokio::test(start_paused = true)]
async fn terminal_error_eof_and_cancellation_release_upstream_and_stop_heartbeats() {
	for suffix in ["done", "malformed", "transport_error", "eof", "cancel"] {
		let dropped = Arc::new(AtomicBool::new(false));
		let mut frames = vec![Ok(wire_chunk(
			json!({"tool_calls": [{"index": 0, "id": "partial", "function": {"name": "agw_tool_2", "arguments": "{\"input\":\"unfinished"}}]}),
			Value::Null,
		))];
		match suffix {
			"done" => frames.push(Ok(Bytes::from_static(b"data: [DONE]\n\n"))),
			"malformed" => frames.push(Ok(Bytes::from_static(b"data: {broken}\n\n"))),
			"transport_error" => frames.push(Err(std::io::Error::other("connection reset"))),
			_ => {},
		}
		let upstream = if suffix == "eof" {
			Body::from_stream(futures_util::stream::iter(frames))
		} else {
			pending_body_after(frames, dropped.clone())
		};
		let mut body = translated_body(upstream);
		if suffix == "cancel" {
			body.frame().await.unwrap().unwrap();
			let heartbeat = body.frame().await.unwrap().unwrap().into_data().unwrap();
			assert_eq!(heartbeat.as_ref(), b": keepalive\n\n");
			drop(body);
			assert!(dropped.load(Ordering::Relaxed));
			continue;
		}
		let bytes = body.collect().await.unwrap().to_bytes();
		let wire = std::str::from_utf8(&bytes).unwrap();
		assert!(!wire.contains("keepalive"));
		assert!(!wire.contains("response.output_item.done"));
		assert_eq!(wire_events(wire).last().unwrap()["type"], "response.failed");
		assert_eq!(
			wire_events(wire).last().unwrap()["response"]["error"]["code"],
			"upstream_protocol_error"
		);
		assert!(suffix == "eof" || dropped.load(Ordering::Relaxed));
	}
	let dropped = Arc::new(AtomicBool::new(false));
	let body = pending_body_after(
		vec![
			Ok(wire_chunk(json!({"content": "done"}), json!("stop"))),
			Ok(Bytes::from_static(b"data: [DONE]\n\n")),
		],
		dropped.clone(),
	);
	let mut body = translated_body(body);
	while let Some(frame) = body.frame().await {
		let bytes = frame.unwrap().into_data().unwrap();
		if std::str::from_utf8(&bytes)
			.unwrap()
			.contains("response.completed")
		{
			assert!(
				dropped.load(Ordering::Relaxed),
				"release upstream with terminal batch"
			);
		}
	}
}

#[tokio::test]
async fn all_parallel_tools_are_validated_before_any_executable_done() {
	for bad in ["{\"cmd\":", "{\"input\":42}"] {
		let name = if bad.contains("input") {
			"agw_tool_2"
		} else {
			"agw_tool_3"
		};
		let events = stream(vec![chunk(json!({"tool_calls": [
			{"index": 0, "id": "valid", "function": {"name": "agw_tool_3", "arguments": "{\"cmd\":\"pwd\"}"}},
			{"index": 1, "id": "invalid", "function": {"name": name, "arguments": bad}}
		]}), json!("tool_calls"))], true, 4096).await;
		assert_eq!(events.last().unwrap()["type"], "response.failed");
		assert!(
			!events
				.iter()
				.any(|event| event["type"] == "response.output_item.done")
		);
	}
	for finish in ["length", "content_filter"] {
		let events = stream(vec![chunk(json!({"tool_calls": [{"index": 0, "id": "valid_but_not_completed", "function": {"name": "agw_tool_3", "arguments": "{\"cmd\":\"pwd\"}"}}]}), json!(finish))], true, 4096).await;
		assert!(
			!events
				.iter()
				.any(|event| event["type"] == "response.output_item.done")
		);
		assert_ne!(events.last().unwrap()["type"], "response.completed");
	}
}

#[test]
fn custom_tool_history_uses_call_id_and_json_object_input() {
	let (request, _) = from_responses::translate_request_with_context(&codex_request()).unwrap();
	let request = serde_json::to_value(request).unwrap();
	assert_eq!(
		request["messages"][2]["content"],
		"I'll update the file and check the directory."
	);
	assert_eq!(request["messages"][2]["tool_calls"][0]["id"], "call_patch");
	assert_eq!(request["messages"][3]["tool_call_id"], "call_patch");
	assert_eq!(request["messages"][4]["tool_call_id"], "call_exec");
	let args: Value = serde_json::from_str(
		request["messages"][2]["tool_calls"][0]["function"]["arguments"]
			.as_str()
			.unwrap(),
	)
	.unwrap();
	assert!(args["input"].as_str().unwrap().contains("+привет\n"));
	let names: Vec<_> = request["tools"]
		.as_array()
		.unwrap()
		.iter()
		.map(|t| t["function"]["name"].as_str().unwrap())
		.collect();
	assert_eq!(
		names.iter().collect::<std::collections::HashSet<_>>().len(),
		names.len()
	);
}

#[test]
fn custom_tool_unary_response_restores_namespace_and_raw_input() {
	let (_, tools) = from_responses::translate_request_with_context(&codex_request()).unwrap();
	let input = "*** Begin Patch\n+\"привет\\世界\"\n*** End Patch";
	let bytes = Bytes::from(
		serde_json::to_vec(&json!({
			"id": "chatcmpl_test", "object": "chat.completion", "created": 1, "model": "kimi",
			"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {"role": "assistant",
				"tool_calls": [{"type": "function", "id": "call_upstream", "function": {
					"name": "agw_tool_2", "arguments": json!({"input": input}).to_string()
				}}]}}]
		}))
		.unwrap(),
	);
	let result = to_responses::translate_response_with_context(&bytes, "kimi", &tools).unwrap();
	let result: Value = serde_json::from_slice(&result.serialize().unwrap()).unwrap();
	let call = &result["output"][0];
	assert_eq!(call["type"], "custom_tool_call");
	assert_eq!(call["namespace"], "functions");
	assert_eq!(call["name"], "apply_patch");
	assert_eq!(call["call_id"], "call_upstream");
	assert_eq!(call["input"], input);
	insta::assert_json_snapshot!("codex_custom_tool_response", call);
}

#[tokio::test]
async fn custom_tool_stream_handles_parallel_calls_split_names_and_json_escapes() {
	let input = "*** Begin Patch\n+\"привет\\世界\"\n*** End Patch";
	let args = json!({"input": input}).to_string();
	let mut chunks = vec![chunk(json!({"content": "Checking"}), Value::Null)];
	chunks.push(chunk(json!({"tool_calls": [{"index": 1, "id": "call_exec", "function": {"name": "agw_", "arguments": "{\"cmd\":"}}, {"index": 0, "id": "call_patch", "function": {"name": "agw_tool_2"}}]}), Value::Null));
	for character in args.chars() {
		chunks.push(chunk(
			json!({"tool_calls": [{"index": 0, "function": {"arguments": character.to_string()}}]}),
			Value::Null,
		));
	}
	chunks.push(chunk(
		json!({"tool_calls": [{"index": 1, "function": {"name": "tool_3", "arguments": "\"pwd\"}"}}]}),
		json!("tool_calls"),
	));
	chunks.push(json!({"id": "chatcmpl_test", "object": "chat.completion.chunk", "created": 1,
		"model": "kimi", "choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}));
	let events = stream(chunks, true, 1024 * 1024).await;
	for (i, event) in events.iter().enumerate() {
		assert_eq!(event["sequence_number"], i + 1);
	}
	let final_response = &events.last().unwrap()["response"];
	assert_eq!(final_response["status"], "completed");
	assert_eq!(final_response["usage"]["total_tokens"], 15);
	assert_eq!(
		final_response["output"][0]["content"][0]["text"],
		"Checking"
	);
	assert_eq!(final_response["output"][1]["input"], input);
	assert_eq!(final_response["output"][1]["call_id"], "call_patch");
	assert_eq!(final_response["output"][2]["name"], "exec_command");
	assert_eq!(final_response["output"][2]["namespace"], "functions");
	assert_eq!(final_response["output"][2]["call_id"], "call_exec");
	let delta: String = events
		.iter()
		.filter(|e| e["type"] == "response.custom_tool_call_input.delta")
		.map(|e| e["delta"].as_str().unwrap())
		.collect();
	assert_eq!(delta, input);
	let done_items: Vec<_> = events
		.iter()
		.filter(|e| e["type"] == "response.output_item.done")
		.map(|e| e["item"].clone())
		.collect();
	assert_eq!(json!(done_items), final_response["output"]);
}

#[tokio::test]
async fn stream_requires_finish_reason_and_preserves_text_without_logging() {
	for done in [true, false] {
		let events = stream(
			vec![chunk(json!({"content": "Hello"}), json!("stop"))],
			done,
			4096,
		)
		.await;
		assert_eq!(
			events.last().unwrap()["response"]["output"][0]["content"][0]["text"],
			"Hello"
		);
		let events = stream(
			vec![chunk(json!({"content": "Truncated"}), Value::Null)],
			done,
			4096,
		)
		.await;
		assert_eq!(events.last().unwrap()["type"], "response.failed");
		assert!(!events.iter().any(|e| e["type"] == "response.completed"));
	}
}

#[tokio::test]
async fn recoverable_failures_preserve_partial_output_and_recovery_metadata() {
	let events = stream(
		vec![chunk(
			json!({"reasoning_content": "Still thinking"}),
			Value::Null,
		)],
		false,
		4096,
	)
	.await;
	let failure = events.last().unwrap();
	assert_eq!(failure["type"], "response.failed");
	assert_eq!(
		failure["response"]["error"]["code"],
		"upstream_protocol_error"
	);
	assert_eq!(failure["response"]["error"]["retryable"], true);
	assert_eq!(
		failure["response"]["error"]["recovery"],
		"continue_from_partial_output"
	);
	assert_eq!(failure["response"]["error"]["partial_output"], true);
	assert_eq!(
		failure["response"]["output"][0]["content"][0]["text"],
		"Still thinking"
	);
}

#[tokio::test]
async fn invalid_custom_arguments_fail_instead_of_executing_json() {
	let events = stream(
		vec![chunk(
			json!({"tool_calls": [{"index": 0, "id": "call_bad",
		"function": {"name": "agw_tool_2", "arguments": "{\"input\": 42}"}}]}),
			json!("tool_calls"),
		)],
		true,
		4096,
	)
	.await;
	assert_eq!(events.last().unwrap()["type"], "response.failed");
	assert!(
		!events
			.iter()
			.any(|e| e["type"] == "response.output_item.done")
	);
}

#[test]
fn unsupported_hosted_tools_and_server_state_are_rejected() {
	for extra in [
		json!({"tools": [{"type": "web_search"}]}),
		json!({"previous_response_id": "resp_old"}),
	] {
		let mut request = json!({"model": "kimi", "input": "hi"});
		request
			.as_object_mut()
			.unwrap()
			.extend(extra.as_object().unwrap().clone());
		let request = serde_json::from_value(request).unwrap();
		assert!(from_responses::translate_request(&request).is_err());
	}
}

#[tokio::test]
async fn reasoning_survives_tool_call_replay() {
	let events = stream(
		vec![
			chunk(
				json!({"reasoning_content": "Inspect the file before editing."}),
				Value::Null,
			),
			chunk(
				json!({"tool_calls": [{"index": 0, "id": "call_reasoning", "function": {
			"name": "agw_tool_3", "arguments": "{\"cmd\":\"pwd\"}"}}]}),
				json!("tool_calls"),
			),
		],
		true,
		4096,
	)
	.await;
	let output = events.last().unwrap()["response"]["output"]
		.as_array()
		.unwrap();
	assert_eq!(output[0]["type"], "reasoning");
	let mut history = output.clone();
	history.push(
		json!({"type": "function_call_output", "call_id": "call_reasoning", "output": "/workspace"}),
	);
	let request = serde_json::from_value(json!({"model": "kimi", "input": history})).unwrap();
	let replay = from_responses::translate_request(&request).unwrap();
	let replay = serde_json::to_value(replay).unwrap();
	assert_eq!(
		replay["messages"][0]["reasoning_content"],
		"Inspect the file before editing."
	);
	assert_eq!(
		replay["messages"][0]["tool_calls"][0]["id"],
		"call_reasoning"
	);
	assert_eq!(replay["messages"][1]["tool_call_id"], "call_reasoning");
}

#[tokio::test]
async fn accumulated_stream_is_bounded_and_incomplete_tools_are_not_completed() {
	let events = stream(
		vec![
			chunk(json!({"content": "a".repeat(1500)}), Value::Null),
			chunk(json!({"content": "a".repeat(1500)}), Value::Null),
			chunk(json!({"content": "a".repeat(1500)}), json!("stop")),
		],
		true,
		4096,
	)
	.await;
	assert_eq!(events.last().unwrap()["type"], "response.failed");
	let events = stream(
		vec![chunk(
			json!({"tool_calls": [{"index": 0, "id": "partial", "function": {
		"name": "agw_tool_2", "arguments": "{\"input\":\"unfinished"}}]}),
			json!("length"),
		)],
		true,
		4096,
	)
	.await;
	assert_eq!(events.last().unwrap()["type"], "response.incomplete");
	assert_eq!(
		events.last().unwrap()["response"]["incomplete_details"]["reason"],
		"max_output_tokens"
	);
	assert!(
		!events
			.iter()
			.any(|e| e["type"] == "response.output_item.done")
	);
}
