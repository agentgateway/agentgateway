use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::strng;
use agent_llm::server_tools::{MessageAccumulator, parse_sse};
use http::Method;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

use crate::llm::custom::ProviderFormat;
use crate::llm::policy::{
	ServerToolFailureMode, ServerToolMapping, ServerToolMcpTarget, ServerToolsConfig,
};
use crate::llm::{Policy, RouteType};
use crate::test_helpers::proxymock::{
	BIND_KEY, basic_named_route, custom_llm_backend, send_request_body, setup_proxy_test, simple_bind,
};
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference};

type Recorded = Arc<Mutex<Vec<Value>>>;

/// A streamable HTTP MCP server with one `search` tool.
struct McpUpstream {
	calls: Recorded,
	fail_calls: bool,
}

impl Respond for McpUpstream {
	fn respond(&self, req: &MockRequest) -> ResponseTemplate {
		let body: Value = serde_json::from_slice(&req.body).unwrap();
		let id = body.get("id").cloned().unwrap_or(Value::Null);
		let result = match body["method"].as_str() {
			Some("initialize") => json!({
				"protocolVersion": "2025-06-18",
				"capabilities": {"tools": {}},
				"serverInfo": {"name": "mock", "version": "0.0.1"},
			}),
			Some("notifications/initialized") => return ResponseTemplate::new(202),
			Some("tools/list") => json!({
				"tools": [{
					"name": "search",
					"description": "Search the web",
					"inputSchema": {
						"type": "object",
						"properties": {"query": {"type": "string"}},
						"required": ["query"],
					},
				}],
			}),
			Some("tools/call") => {
				self.calls.lock().unwrap().push(body["params"].clone());
				if self.fail_calls {
					return ResponseTemplate::new(500);
				}
				let query = body["params"]["arguments"]["query"]
					.as_str()
					.unwrap_or_default()
					.to_string();
				json!({
					"content": [{"type": "text", "text": format!("Result for '{query}': Seattle won Super Bowl LX 24-17.")}],
					"isError": false,
				})
			},
			other => panic!("unexpected MCP method {other:?}"),
		};
		let frame = format!(
			"data: {}\n\n",
			json!({"jsonrpc": "2.0", "id": id, "result": result})
		);
		ResponseTemplate::new(200)
			.insert_header("mcp-session-id", "upstream-session")
			.set_body_raw(frame, "text/event-stream")
	}
}

async fn mcp_server(fail_calls: bool) -> (MockServer, Recorded) {
	let calls = Recorded::default();
	let server = MockServer::start().await;
	Mock::given(method("POST"))
		.respond_with(McpUpstream {
			calls: calls.clone(),
			fail_calls,
		})
		.mount(&server)
		.await;
	Mock::given(method("DELETE"))
		.respond_with(ResponseTemplate::new(200))
		.mount(&server)
		.await;
	(server, calls)
}

#[derive(Clone, Copy)]
enum Model {
	/// Calls `web_search`, then answers once it has a tool result.
	SearchThenAnswer,
	/// Calls `web_search` with the same query forever.
	RepeatSearch,
	/// Calls `web_search` with a new query forever.
	EndlessSearch,
	/// Calls `web_search` and the client's `Read` tool in the same turn.
	MixedTools,
	/// Answers without calling any tool.
	AnswerOnly,
}

/// An OpenAI chat completions server.
struct LlmUpstream {
	requests: Recorded,
	model: Model,
	streaming: bool,
}

fn tool_call_json(id: &str, query: &str) -> Value {
	json!({
		"id": id,
		"type": "function",
		"function": {"name": "web_search", "arguments": json!({"query": query}).to_string()},
	})
}

impl LlmUpstream {
	fn answer(&self, calls: usize, has_tool_result: bool) -> (Vec<Value>, Option<&'static str>) {
		let search = |q: String| vec![tool_call_json(&format!("call_{calls}"), &q)];
		match self.model {
			Model::SearchThenAnswer if !has_tool_result => {
				(search("super bowl lx winner".to_string()), None)
			},
			Model::RepeatSearch => (search("super bowl lx winner".to_string()), None),
			Model::EndlessSearch => (search(format!("query {calls}")), None),
			Model::MixedTools if !has_tool_result => (
				vec![
					tool_call_json("call_ws", "super bowl lx winner"),
					json!({
						"id": "call_read",
						"type": "function",
						"function": {"name": "Read", "arguments": json!({"path": "/tmp/notes"}).to_string()},
					}),
				],
				None,
			),
			_ => (vec![], Some("Seattle won Super Bowl LX.")),
		}
	}
}

impl Respond for LlmUpstream {
	fn respond(&self, req: &MockRequest) -> ResponseTemplate {
		let body: Value = serde_json::from_slice(&req.body).unwrap();
		let mut requests = self.requests.lock().unwrap();
		requests.push(body.clone());
		let calls = requests.len();
		drop(requests);
		let has_tool_result = body["messages"]
			.as_array()
			.unwrap()
			.iter()
			.any(|m| m["role"] == "tool");
		let (tool_calls, text) = self.answer(calls, has_tool_result);
		let usage = json!({"prompt_tokens": 100 * calls, "completion_tokens": 10 * calls, "total_tokens": 110 * calls});
		if !self.streaming {
			let (message, finish) = if let Some(text) = text {
				(json!({"role": "assistant", "content": text}), "stop")
			} else {
				(
					json!({"role": "assistant", "content": null, "tool_calls": tool_calls}),
					"tool_calls",
				)
			};
			return ResponseTemplate::new(200).set_body_json(json!({
				"id": format!("chatcmpl-{calls}"),
				"object": "chat.completion",
				"created": 0,
				"model": "mock-model",
				"choices": [{"index": 0, "message": message, "finish_reason": finish}],
				"usage": usage,
			}));
		}
		let chunk = |delta: Value, finish: Option<&str>| {
			json!({
				"id": format!("chatcmpl-{calls}"),
				"object": "chat.completion.chunk",
				"created": 0,
				"model": "mock-model",
				"choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
			})
		};
		let mut chunks = vec![chunk(json!({"role": "assistant", "content": ""}), None)];
		if let Some(text) = text {
			let (a, b) = text.split_at(text.len() / 2);
			chunks.push(chunk(json!({"content": a}), None));
			chunks.push(chunk(json!({"content": b}), None));
			chunks.push(chunk(json!({}), Some("stop")));
		} else {
			for (index, call) in tool_calls.into_iter().enumerate() {
				chunks.push(chunk(
					json!({"tool_calls": [{
						"index": index,
						"id": call["id"],
						"type": "function",
						"function": {"name": call["function"]["name"], "arguments": ""},
					}]}),
					None,
				));
				chunks.push(chunk(
					json!({"tool_calls": [{"index": index, "function": {"arguments": call["function"]["arguments"]}}]}),
					None,
				));
			}
			chunks.push(chunk(json!({}), Some("tool_calls")));
		}
		chunks.push(json!({
			"id": format!("chatcmpl-{calls}"),
			"object": "chat.completion.chunk",
			"created": 0,
			"model": "mock-model",
			"choices": [],
			"usage": usage,
		}));
		let mut sse = String::new();
		for c in chunks {
			sse.push_str(&format!("data: {c}\n\n"));
		}
		sse.push_str("data: [DONE]\n\n");
		ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream")
	}
}

async fn llm_server(model: Model, streaming: bool) -> (MockServer, Recorded) {
	let requests = Recorded::default();
	let server = MockServer::start().await;
	Mock::given(method("POST"))
		.respond_with(LlmUpstream {
			requests: requests.clone(),
			model,
			streaming,
		})
		.mount(&server)
		.await;
	(server, requests)
}

struct Setup {
	llm_requests: Recorded,
	mcp_calls: Recorded,
	_llm: MockServer,
	_mcp: MockServer,
	bind: crate::test_helpers::proxymock::TestBind,
}

async fn setup(
	model: Model,
	streaming: bool,
	max_iterations: u32,
	failure_mode: ServerToolFailureMode,
	fail_calls: bool,
) -> Setup {
	let (llm, llm_requests) = llm_server(model, streaming).await;
	let (mcp, mcp_calls) = mcp_server(fail_calls).await;
	let config = ServerToolsConfig {
		tools: vec![ServerToolMapping {
			tool_type: "web_search_*".to_string(),
			mcp: ServerToolMcpTarget {
				backend: strng::format!("/{}", mcp.address()),
				target: None,
				tool: strng::literal!("search"),
			},
			description: None,
			input_schema: None,
		}],
		max_iterations,
		max_result_bytes: 64 * 1024,
		keepalive_interval: Duration::from_millis(20),
		failure_mode,
	};
	let policy = Policy {
		routes: [(strng::literal!("/v1/messages"), RouteType::Messages)]
			.into_iter()
			.collect(),
		server_tools: Some(Arc::new(config)),
		..Default::default()
	};
	let mut backend = custom_llm_backend(
		"llm",
		SimpleBackendReference::Backend(strng::format!("/{}", llm.address())),
		vec![ProviderFormat::Completions],
	);
	backend.inline_policies = vec![BackendTrafficPolicy::AI(Arc::new(policy))];
	let bind = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*llm.address())
		.with_mcp_backend(*mcp.address(), true, false)
		.with_raw_backend(backend)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/llm")));
	Setup {
		llm_requests,
		mcp_calls,
		_llm: llm,
		_mcp: mcp,
		bind,
	}
}

fn client_request(streaming: bool, tools: Value) -> Vec<u8> {
	json!({
		"model": "mock-model",
		"max_tokens": 256,
		"stream": streaming,
		"messages": [{"role": "user", "content": "Who won Super Bowl LX?"}],
		"tools": tools,
	})
	.to_string()
	.into_bytes()
}

fn web_search_tool() -> Value {
	json!([{"type": "web_search_20250305", "name": "web_search", "max_uses": 8}])
}

async fn send(setup: &Setup, body: Vec<u8>) -> (http::StatusCode, http::HeaderMap, bytes::Bytes) {
	let io = setup.bind.serve_http(BIND_KEY);
	let resp = send_request_body(io, Method::POST, "http://localhost/v1/messages", &body).await;
	let status = resp.status();
	let headers = resp.headers().clone();
	let body = resp.into_body().collect().await.unwrap().to_bytes();
	(status, headers, body)
}

fn recorded(r: &Recorded) -> Vec<Value> {
	r.lock().unwrap().clone()
}

#[tokio::test]
async fn buffered_turn_is_completed_with_the_tool_result() {
	let s = setup(
		Model::SearchThenAnswer,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let (status, _, body) = send(&s, client_request(false, web_search_tool())).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let message: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(message["stop_reason"], json!("end_turn"));
	assert_eq!(
		message["content"],
		json!([{"type": "text", "text": "Seattle won Super Bowl LX."}])
	);
	// Usage is summed across both model calls (100 + 200 in, 10 + 20 out).
	assert_eq!(message["usage"]["input_tokens"], json!(300));
	assert_eq!(message["usage"]["output_tokens"], json!(30));

	let calls = recorded(&s.mcp_calls);
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0]["name"], json!("search"));
	assert_eq!(
		calls[0]["arguments"],
		json!({"query": "super bowl lx winner"})
	);

	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 2);
	// The server tool reached the model as a custom function tool with the MCP schema.
	assert_eq!(
		requests[0]["tools"],
		json!([{
			"type": "function",
			"function": {
				"name": "web_search",
				"description": "Search the web",
				"parameters": {
					"type": "object",
					"properties": {"query": {"type": "string"}},
					"required": ["query"],
				},
			},
		}])
	);
	// The follow-up carries the model's tool call and the MCP result.
	let messages = requests[1]["messages"].as_array().unwrap();
	assert_eq!(messages.len(), 3);
	assert_eq!(messages[1]["role"], json!("assistant"));
	assert_eq!(
		messages[1]["tool_calls"][0]["function"]["name"],
		json!("web_search")
	);
	assert_eq!(messages[2]["role"], json!("tool"));
	assert_eq!(messages[2]["tool_call_id"], json!("call_1"));
	assert!(
		messages[2]["content"]
			.to_string()
			.contains("Seattle won Super Bowl LX 24-17."),
		"{}",
		messages[2]
	);
}

#[tokio::test]
async fn streaming_turn_is_held_back_and_completed() {
	let s = setup(
		Model::SearchThenAnswer,
		true,
		3,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let (status, headers, body) = send(&s, client_request(true, web_search_tool())).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	assert!(
		headers
			.get(http::header::CONTENT_TYPE)
			.and_then(|v| v.to_str().ok())
			.is_some_and(|v| v.starts_with("text/event-stream")),
		"{headers:?}"
	);
	let events = parse_sse(&body);
	// Exactly one message reaches the client, and it carries no tool call.
	assert_eq!(
		events
			.iter()
			.filter(|e| e.event.as_deref() == Some("message_start"))
			.count(),
		1
	);
	assert_eq!(
		events
			.iter()
			.filter(|e| e.event.as_deref() == Some("message_stop"))
			.count(),
		1
	);
	let mut acc = MessageAccumulator::default();
	acc.feed_all(&events);
	assert!(acc.is_complete());
	let message = acc.finish().unwrap();
	assert_eq!(message["stop_reason"], json!("end_turn"));
	assert_eq!(
		message["content"],
		json!([{"type": "text", "text": "Seattle won Super Bowl LX."}])
	);
	assert_eq!(message["usage"]["input_tokens"], json!(300));
	assert_eq!(message["usage"]["output_tokens"], json!(30));
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
	assert_eq!(recorded(&s.llm_requests).len(), 2);
}

#[tokio::test]
async fn repeated_call_ends_the_turn_without_a_tool_use() {
	let s = setup(
		Model::RepeatSearch,
		false,
		5,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let (status, _, body) = send(&s, client_request(false, web_search_tool())).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let message: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(message["stop_reason"], json!("end_turn"));
	assert!(
		!message["content"]
			.as_array()
			.unwrap()
			.iter()
			.any(|b| b["type"] == "tool_use"),
		"{message}"
	);
	// One search was executed; the identical second request ended the turn.
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
	assert_eq!(recorded(&s.llm_requests).len(), 2);
	assert_eq!(message["usage"]["input_tokens"], json!(300));
}

#[tokio::test]
async fn iteration_cap_ends_the_turn() {
	let s = setup(
		Model::EndlessSearch,
		true,
		2,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let (status, _, body) = send(&s, client_request(true, web_search_tool())).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let mut acc = MessageAccumulator::default();
	acc.feed_all(&parse_sse(&body));
	let message = acc.finish().unwrap();
	assert_eq!(message["stop_reason"], json!("end_turn"));
	assert!(
		!message["content"]
			.as_array()
			.unwrap()
			.iter()
			.any(|b| b["type"] == "tool_use"),
		"{message}"
	);
	assert_eq!(recorded(&s.mcp_calls).len(), 2);
	assert_eq!(recorded(&s.llm_requests).len(), 3);
	assert_eq!(message["usage"]["input_tokens"], json!(600));
	assert_eq!(message["usage"]["output_tokens"], json!(60));
}

#[tokio::test]
async fn client_max_uses_caps_iterations() {
	let s = setup(
		Model::EndlessSearch,
		false,
		5,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let tools = json!([{"type": "web_search_20250305", "name": "web_search", "max_uses": 1}]);
	let (status, _, body) = send(&s, client_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
	assert_eq!(recorded(&s.llm_requests).len(), 2);
}

#[tokio::test]
async fn mixed_client_tools_are_returned_untouched() {
	let s = setup(
		Model::MixedTools,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let tools = json!([
		{"type": "web_search_20250305", "name": "web_search"},
		{"name": "Read", "description": "read a file", "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}},
	]);
	let (status, _, body) = send(&s, client_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let message: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(message["stop_reason"], json!("tool_use"));
	let tool_uses: Vec<&str> = message["content"]
		.as_array()
		.unwrap()
		.iter()
		.filter(|b| b["type"] == "tool_use")
		.map(|b| b["name"].as_str().unwrap())
		.collect();
	assert_eq!(tool_uses, vec!["Read"]);
	assert_eq!(recorded(&s.mcp_calls).len(), 0);
	assert_eq!(recorded(&s.llm_requests).len(), 1);
}

#[tokio::test]
async fn fail_open_reports_the_error_to_the_model() {
	let s = setup(
		Model::SearchThenAnswer,
		false,
		3,
		ServerToolFailureMode::FailOpen,
		true,
	)
	.await;
	let (status, _, body) = send(&s, client_request(false, web_search_tool())).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 2);
	let tool_message = &requests[1]["messages"].as_array().unwrap()[2];
	assert_eq!(tool_message["role"], json!("tool"));
	assert!(
		tool_message["content"]
			.to_string()
			.contains("tool call failed"),
		"{tool_message}"
	);
}

#[tokio::test]
async fn fail_closed_ends_the_turn_with_an_error() {
	let s = setup(
		Model::SearchThenAnswer,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		true,
	)
	.await;
	let (status, _, _) = send(&s, client_request(false, web_search_tool())).await;
	assert_eq!(status, 502);
	assert_eq!(recorded(&s.llm_requests).len(), 1);

	// A streaming client already has its headers, so the failure arrives as an error event.
	let s = setup(
		Model::SearchThenAnswer,
		true,
		3,
		ServerToolFailureMode::FailClosed,
		true,
	)
	.await;
	let (status, _, body) = send(&s, client_request(true, web_search_tool())).await;
	assert_eq!(status, 200);
	let events = parse_sse(&body);
	let error = events
		.iter()
		.find(|e| e.event.as_deref() == Some("error"))
		.expect("error event");
	let data: Value = serde_json::from_str(&error.data).unwrap();
	assert_eq!(data["error"]["type"], json!("api_error"));
	assert!(
		!events
			.iter()
			.any(|e| e.event.as_deref() == Some("message_start")),
		"no partial message may leak: {events:?}"
	);
}

#[tokio::test]
async fn requests_without_server_tools_are_untouched() {
	let s = setup(
		Model::AnswerOnly,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let tools = json!([{"name": "Read", "input_schema": {"type": "object"}}]);
	let (status, _, body) = send(&s, client_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let message: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(message["stop_reason"], json!("end_turn"));
	assert_eq!(message["usage"]["input_tokens"], json!(100));
	assert!(recorded(&s.mcp_calls).is_empty());
	assert_eq!(recorded(&s.llm_requests).len(), 1);
	assert_eq!(
		recorded(&s.llm_requests)[0]["tools"][0]["function"]["name"],
		json!("Read")
	);
}
