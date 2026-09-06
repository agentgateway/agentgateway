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
	ServerToolFailureMode, ServerToolMapping, ServerToolMcpServer, ServerToolMcpTarget,
	ServerToolsConfig, UnmappedServerTools,
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
	/// Answer searches with structured JSON results instead of a sentence.
	json_results: bool,
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
				let text = if self.json_results {
					json!({"results": [{
						"title": "Super Bowl LX",
						"url": "https://example.com/super-bowl-lx",
						"content": format!("Result for '{query}': Seattle won Super Bowl LX 24-17."),
					}]})
					.to_string()
				} else {
					format!("Result for '{query}': Seattle won Super Bowl LX 24-17.")
				};
				json!({
					"content": [{"type": "text", "text": text}],
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
	mcp_server_with(fail_calls, false).await
}

async fn mcp_server_with(fail_calls: bool, json_results: bool) -> (MockServer, Recorded) {
	let calls = Recorded::default();
	let server = MockServer::start().await;
	Mock::given(method("POST"))
		.respond_with(McpUpstream {
			calls: calls.clone(),
			fail_calls,
			json_results,
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
	/// Calls the MCP server's `search` tool by its own name, then answers.
	McpSearchThenAnswer,
}

/// An OpenAI chat completions server.
struct LlmUpstream {
	requests: Recorded,
	model: Model,
	streaming: bool,
}

fn named_call_json(id: &str, name: &str, query: &str) -> Value {
	json!({
		"id": id,
		"type": "function",
		"function": {"name": name, "arguments": json!({"query": query}).to_string()},
	})
}

fn tool_call_json(id: &str, query: &str) -> Value {
	named_call_json(id, "web_search", query)
}

impl LlmUpstream {
	fn answer(&self, calls: usize, has_tool_result: bool) -> (Vec<Value>, Option<&'static str>) {
		let search = |q: String| vec![tool_call_json(&format!("call_{calls}"), &q)];
		match self.model {
			Model::SearchThenAnswer if !has_tool_result => {
				(search("super bowl lx winner".to_string()), None)
			},
			Model::McpSearchThenAnswer if !has_tool_result => (
				vec![named_call_json(
					&format!("call_{calls}"),
					"search",
					"super bowl lx winner",
				)],
				None,
			),
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

/// Knobs the Messages tests vary beyond the common ones.
struct SetupOpts {
	tool_type: &'static str,
	unmapped: UnmappedServerTools,
	json_results: bool,
}

impl Default for SetupOpts {
	fn default() -> Self {
		Self {
			tool_type: "web_search_*",
			unmapped: UnmappedServerTools::Drop,
			json_results: false,
		}
	}
}

async fn setup(
	model: Model,
	streaming: bool,
	max_iterations: u32,
	failure_mode: ServerToolFailureMode,
	fail_calls: bool,
) -> Setup {
	setup_with(
		model,
		streaming,
		max_iterations,
		failure_mode,
		fail_calls,
		SetupOpts::default(),
	)
	.await
}

async fn setup_with(
	model: Model,
	streaming: bool,
	max_iterations: u32,
	failure_mode: ServerToolFailureMode,
	fail_calls: bool,
	opts: SetupOpts,
) -> Setup {
	let (llm, llm_requests) = llm_server(model, streaming).await;
	let (mcp, mcp_calls) = mcp_server_with(fail_calls, opts.json_results).await;
	let config = ServerToolsConfig {
		tools: vec![ServerToolMapping {
			tool_type: opts.tool_type.to_string(),
			mcp: ServerToolMcpTarget {
				backend: strng::format!("/{}", mcp.address()),
				target: None,
				tool: strng::literal!("search"),
			},
			description: None,
			input_schema: None,
		}],
		mcp_servers: vec![],
		max_iterations,
		max_result_bytes: 64 * 1024,
		keepalive_interval: Duration::from_millis(20),
		failure_mode,
		unmapped: opts.unmapped,
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
	// One search was executed; the identical second request was answered with an error result and
	// the tools withdrawn, and the model was asked once more to conclude.
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 3);
	assert!(
		requests[2]
			.get("tools")
			.is_none_or(|t| t.as_array().is_some_and(Vec::is_empty)),
		"{}",
		requests[2]
	);
	let last = requests[2]["messages"]
		.as_array()
		.unwrap()
		.last()
		.unwrap()
		.clone();
	assert_eq!(last["role"], json!("tool"));
	assert!(
		last["content"].to_string().contains("limit reached"),
		"{last}"
	);
	assert_eq!(message["usage"]["input_tokens"], json!(600));
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
	// Two searches ran, then the cap answered the third call with an error and the model was
	// asked to conclude without the tool.
	assert_eq!(recorded(&s.mcp_calls).len(), 2);
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 4);
	assert!(
		requests[3]
			.get("tools")
			.is_none_or(|t| t.as_array().is_some_and(Vec::is_empty)),
		"{}",
		requests[3]
	);
	assert_eq!(message["usage"]["input_tokens"], json!(1000));
	assert_eq!(message["usage"]["output_tokens"], json!(100));
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
	// One search, then the second call hits max_uses and the model concludes.
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
	assert_eq!(recorded(&s.llm_requests).len(), 3);
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

// --- OpenAI Responses clients ---

/// The Responses route with a `web_search*` mapping and the MCP server declared as `search`.
async fn setup_responses(model: Model, streaming: bool, skip_approval: bool) -> Setup {
	let (llm, llm_requests) = llm_server(model, streaming).await;
	let (mcp, mcp_calls) = mcp_server(false).await;
	let config = ServerToolsConfig {
		tools: vec![ServerToolMapping {
			tool_type: "web_search*".to_string(),
			mcp: ServerToolMcpTarget {
				backend: strng::format!("/{}", mcp.address()),
				target: None,
				tool: strng::literal!("search"),
			},
			description: None,
			input_schema: None,
		}],
		mcp_servers: vec![ServerToolMcpServer {
			label: Some(strng::literal!("search")),
			url: None,
			backend: strng::format!("/{}", mcp.address()),
			target: None,
			skip_approval,
		}],
		max_iterations: 3,
		max_result_bytes: 64 * 1024,
		keepalive_interval: Duration::from_millis(20),
		failure_mode: ServerToolFailureMode::FailClosed,
		unmapped: UnmappedServerTools::Drop,
	};
	let policy = Policy {
		routes: [(strng::literal!("/v1/responses"), RouteType::Responses)]
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

fn responses_request(streaming: bool, tools: Value) -> Vec<u8> {
	json!({
		"model": "mock-model",
		"input": "Who won Super Bowl LX?",
		"stream": streaming,
		"tools": tools,
	})
	.to_string()
	.into_bytes()
}

async fn send_responses(
	setup: &Setup,
	body: Vec<u8>,
) -> (http::StatusCode, http::HeaderMap, bytes::Bytes) {
	let io = setup.bind.serve_http(BIND_KEY);
	let resp = send_request_body(io, Method::POST, "http://localhost/v1/responses", &body).await;
	let status = resp.status();
	let headers = resp.headers().clone();
	let body = resp.into_body().collect().await.unwrap().to_bytes();
	(status, headers, body)
}

fn output_text(response: &Value) -> Vec<String> {
	response["output"]
		.as_array()
		.unwrap()
		.iter()
		.filter(|item| item["type"] == "message")
		.flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
		.filter_map(|part| part["text"].as_str().map(str::to_string))
		.collect()
}

#[tokio::test]
async fn responses_builtin_tool_is_fulfilled() {
	let s = setup_responses(Model::SearchThenAnswer, false, false).await;
	let (status, _, body) = send_responses(
		&s,
		responses_request(false, json!([{"type": "web_search"}])),
	)
	.await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let response: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(response["status"], json!("completed"));
	assert_eq!(output_text(&response), ["Seattle won Super Bowl LX."]);
	assert!(
		response["output"]
			.as_array()
			.unwrap()
			.iter()
			.all(|item| item["type"] != "function_call"),
		"{response}"
	);
	// Usage is summed across both model calls (100 + 200 in, 10 + 20 out).
	assert_eq!(response["usage"]["input_tokens"], json!(300));
	assert_eq!(response["usage"]["output_tokens"], json!(30));

	let calls = recorded(&s.mcp_calls);
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0]["name"], json!("search"));
	assert_eq!(
		calls[0]["arguments"],
		json!({"query": "super bowl lx winner"})
	);

	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 2);
	// The built-in reached the model as a function tool named after its type, with the MCP schema.
	assert_eq!(requests[0]["tools"][0]["type"], json!("function"));
	assert_eq!(
		requests[0]["tools"][0]["function"]["name"],
		json!("web_search")
	);
	assert_eq!(
		requests[0]["tools"][0]["function"]["parameters"]["properties"]["query"]["type"],
		json!("string")
	);
	// The follow-up carries the model's call and the MCP result.
	let messages = requests[1]["messages"].as_array().unwrap();
	assert!(
		messages.iter().any(|m| m["role"] == "tool"),
		"{}",
		requests[1]
	);
	assert!(
		requests[1]
			.to_string()
			.contains("Seattle won Super Bowl LX 24-17."),
		"{}",
		requests[1]
	);
}

#[tokio::test]
async fn responses_stream_is_held_back_and_completed() {
	let s = setup_responses(Model::SearchThenAnswer, true, false).await;
	let (status, headers, body) =
		send_responses(&s, responses_request(true, json!([{"type": "web_search"}]))).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	assert!(
		headers
			.get(http::header::CONTENT_TYPE)
			.and_then(|v| v.to_str().ok())
			.is_some_and(|v| v.starts_with("text/event-stream")),
		"{headers:?}"
	);
	let events = parse_sse(&body);
	let final_response = agent_llm::responses_tools::final_response(&events)
		.unwrap_or_else(|| panic!("no terminal event in {}", String::from_utf8_lossy(&body)));
	assert_eq!(output_text(&final_response), ["Seattle won Super Bowl LX."]);
	assert_eq!(final_response["usage"]["input_tokens"], json!(300));
	assert_eq!(final_response["usage"]["output_tokens"], json!(30));
	assert!(
		events.iter().any(|e| {
			e.event.as_deref() == Some("response.output_text.delta")
				|| e.data.contains("\"response.output_text.delta\"")
		}),
		"{}",
		String::from_utf8_lossy(&body)
	);
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
	assert_eq!(recorded(&s.llm_requests).len(), 2);
}

#[tokio::test]
async fn responses_mcp_server_tools_are_exposed_by_name() {
	let s = setup_responses(Model::McpSearchThenAnswer, false, false).await;
	let tools = json!([{
		"type": "mcp",
		"server_label": "search",
		"server_url": "https://mcp.example.com/mcp",
		"require_approval": "never",
	}]);
	let (status, _, body) = send_responses(&s, responses_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let response: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(output_text(&response), ["Seattle won Super Bowl LX."]);

	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 2);
	assert_eq!(
		requests[0]["tools"][0]["function"]["name"],
		json!("search"),
		"{}",
		requests[0]
	);
	assert_eq!(
		requests[0]["tools"][0]["function"]["description"],
		json!("Search the web")
	);
	let calls = recorded(&s.mcp_calls);
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0]["name"], json!("search"));
}

#[tokio::test]
async fn responses_mcp_server_needing_approval_is_rejected_unless_skipped() {
	let s = setup_responses(Model::McpSearchThenAnswer, false, false).await;
	let tools = json!([{"type": "mcp", "server_label": "search"}]);
	let (status, _, body) = send_responses(&s, responses_request(false, tools.clone())).await;
	assert_eq!(status, 400, "{}", String::from_utf8_lossy(&body));
	assert!(
		String::from_utf8_lossy(&body).contains("requires approval"),
		"{}",
		String::from_utf8_lossy(&body)
	);
	assert!(recorded(&s.llm_requests).is_empty());

	let s = setup_responses(Model::McpSearchThenAnswer, false, true).await;
	let (status, _, body) = send_responses(&s, responses_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
}

#[tokio::test]
async fn responses_without_server_tools_are_untouched() {
	let s = setup_responses(Model::AnswerOnly, false, false).await;
	let tools = json!([{"type": "function", "name": "read", "parameters": {"type": "object"}}]);
	let (status, _, body) = send_responses(&s, responses_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 1);
	assert_eq!(requests[0]["tools"][0]["function"]["name"], json!("read"));
	assert!(recorded(&s.mcp_calls).is_empty());
}

// --- OpenAI Chat Completions clients ---

/// The Chat Completions route with a `web_search*` mapping.
async fn setup_completions(model: Model, streaming: bool) -> Setup {
	let (llm, llm_requests) = llm_server(model, streaming).await;
	let (mcp, mcp_calls) = mcp_server(false).await;
	let config = ServerToolsConfig {
		tools: vec![ServerToolMapping {
			tool_type: "web_search*".to_string(),
			mcp: ServerToolMcpTarget {
				backend: strng::format!("/{}", mcp.address()),
				target: None,
				tool: strng::literal!("search"),
			},
			description: None,
			input_schema: None,
		}],
		mcp_servers: vec![],
		max_iterations: 3,
		max_result_bytes: 64 * 1024,
		keepalive_interval: Duration::from_millis(20),
		failure_mode: ServerToolFailureMode::FailClosed,
		unmapped: UnmappedServerTools::Drop,
	};
	let policy = Policy {
		routes: [(
			strng::literal!("/v1/chat/completions"),
			RouteType::Completions,
		)]
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

fn completions_request(streaming: bool, web_search: bool) -> Vec<u8> {
	let mut body = json!({
		"model": "mock-model",
		"messages": [{"role": "user", "content": "Who won Super Bowl LX?"}],
		"stream": streaming,
	});
	if web_search {
		body["web_search_options"] = json!({"search_context_size": "low"});
	}
	body.to_string().into_bytes()
}

async fn send_completions(
	setup: &Setup,
	body: Vec<u8>,
) -> (http::StatusCode, http::HeaderMap, bytes::Bytes) {
	let io = setup.bind.serve_http(BIND_KEY);
	let resp = send_request_body(
		io,
		Method::POST,
		"http://localhost/v1/chat/completions",
		&body,
	)
	.await;
	let status = resp.status();
	let headers = resp.headers().clone();
	let body = resp.into_body().collect().await.unwrap().to_bytes();
	(status, headers, body)
}

#[tokio::test]
async fn completions_web_search_options_is_fulfilled() {
	let s = setup_completions(Model::SearchThenAnswer, false).await;
	let (status, _, body) = send_completions(&s, completions_request(false, true)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let completion: Value = serde_json::from_slice(&body).unwrap();
	let choice = &completion["choices"][0];
	assert_eq!(choice["finish_reason"], json!("stop"));
	assert_eq!(
		choice["message"]["content"],
		json!("Seattle won Super Bowl LX.")
	);
	assert!(
		choice["message"].get("tool_calls").is_none(),
		"{completion}"
	);
	// Usage is summed across both model calls (100 + 200 in, 10 + 20 out).
	assert_eq!(completion["usage"]["prompt_tokens"], json!(300));
	assert_eq!(completion["usage"]["completion_tokens"], json!(30));

	let calls = recorded(&s.mcp_calls);
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0]["name"], json!("search"));

	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 2);
	// The field is gone and the model sees a function tool with the MCP schema instead.
	assert!(
		requests[0].get("web_search_options").is_none(),
		"{}",
		requests[0]
	);
	assert_eq!(
		requests[0]["tools"][0]["function"]["name"],
		json!("web_search")
	);
	assert_eq!(
		requests[0]["tools"][0]["function"]["description"],
		json!("Search the web")
	);
	let messages = requests[1]["messages"].as_array().unwrap();
	assert_eq!(messages.len(), 3);
	assert_eq!(messages[1]["role"], json!("assistant"));
	assert_eq!(messages[2]["role"], json!("tool"));
	assert_eq!(messages[2]["tool_call_id"], json!("call_1"));
	assert!(
		messages[2]["content"]
			.as_str()
			.unwrap()
			.contains("Seattle won Super Bowl LX 24-17."),
		"{}",
		messages[2]
	);
}

#[tokio::test]
async fn completions_stream_is_held_back_and_completed() {
	let s = setup_completions(Model::SearchThenAnswer, true).await;
	let (status, headers, body) = send_completions(&s, completions_request(true, true)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	assert!(
		headers
			.get(http::header::CONTENT_TYPE)
			.and_then(|v| v.to_str().ok())
			.is_some_and(|v| v.starts_with("text/event-stream")),
		"{headers:?}"
	);
	let events = parse_sse(&body);
	assert_eq!(events.last().unwrap().data, "[DONE]");
	let mut acc = agent_llm::completions_tools::ChunkAccumulator::default();
	acc.feed_all(&events);
	let completion = acc.finish().unwrap();
	assert_eq!(
		completion["choices"][0]["message"]["content"],
		json!("Seattle won Super Bowl LX.")
	);
	assert!(
		completion["choices"][0]["message"]
			.get("tool_calls")
			.is_none()
	);
	assert_eq!(completion["usage"]["prompt_tokens"], json!(300));
	assert_eq!(completion["usage"]["completion_tokens"], json!(30));
	assert_eq!(recorded(&s.mcp_calls).len(), 1);
	assert_eq!(recorded(&s.llm_requests).len(), 2);
}

#[tokio::test]
async fn completions_without_web_search_options_are_untouched() {
	let s = setup_completions(Model::AnswerOnly, false).await;
	let (status, _, body) = send_completions(&s, completions_request(false, false)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 1);
	assert!(requests[0].get("tools").is_none(), "{}", requests[0]);
	assert!(recorded(&s.mcp_calls).is_empty());
}

// --- interception guards and native result blocks ---

#[tokio::test]
async fn client_executed_tool_types_are_never_intercepted() {
	// A mapping broad enough to match everything still leaves bash to the client.
	let s = setup_with(
		Model::AnswerOnly,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
		SetupOpts {
			tool_type: "*",
			..Default::default()
		},
	)
	.await;
	let tools = json!([
		{"type": "bash_20250124", "name": "bash"},
		{"type": "web_search_20250305", "name": "web_search"},
	]);
	let (status, _, body) = send(&s, client_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 1);
	let names: Vec<&str> = requests[0]["tools"]
		.as_array()
		.unwrap()
		.iter()
		.map(|t| t["function"]["name"].as_str().unwrap())
		.collect();
	assert_eq!(names, ["web_search"], "{}", requests[0]);
}

#[tokio::test]
async fn unmapped_server_tools_are_rejected_when_configured() {
	let tools = json!([{"type": "web_fetch_20250910", "name": "web_fetch"}]);
	let s = setup_with(
		Model::AnswerOnly,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
		SetupOpts {
			unmapped: UnmappedServerTools::Reject,
			..Default::default()
		},
	)
	.await;
	let (status, _, body) = send(&s, client_request(false, tools.clone())).await;
	assert_eq!(status, 400, "{}", String::from_utf8_lossy(&body));
	assert!(
		String::from_utf8_lossy(&body).contains("web_fetch_20250910"),
		"{}",
		String::from_utf8_lossy(&body)
	);
	assert!(recorded(&s.llm_requests).is_empty());

	// By default the tool is left to the provider.
	let s = setup(
		Model::AnswerOnly,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let (status, _, _) = send(&s, client_request(false, tools)).await;
	assert_eq!(status, 200);
	assert_eq!(recorded(&s.llm_requests).len(), 1);
}

fn block_types(message: &Value) -> Vec<String> {
	message["content"]
		.as_array()
		.unwrap()
		.iter()
		.map(|b| b["type"].as_str().unwrap().to_string())
		.collect()
}

#[tokio::test]
async fn native_search_result_blocks_are_synthesised() {
	let s = setup_with(
		Model::SearchThenAnswer,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
		SetupOpts {
			json_results: true,
			..Default::default()
		},
	)
	.await;
	let (status, _, body) = send(&s, client_request(false, web_search_tool())).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let message: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		block_types(&message),
		["server_tool_use", "web_search_tool_result", "text"],
		"{message}"
	);
	let content = message["content"].as_array().unwrap();
	assert_eq!(content[0]["name"], json!("web_search"));
	assert_eq!(content[0]["input"]["query"], json!("super bowl lx winner"));
	assert_eq!(content[1]["tool_use_id"], content[0]["id"]);
	assert_eq!(
		content[1]["content"][0]["url"],
		json!("https://example.com/super-bowl-lx")
	);
	assert_eq!(content[1]["content"][0]["title"], json!("Super Bowl LX"));
	assert_eq!(content[2]["text"], json!("Seattle won Super Bowl LX."));

	// Streaming rebuilds the same message.
	let s = setup_with(
		Model::SearchThenAnswer,
		true,
		3,
		ServerToolFailureMode::FailClosed,
		false,
		SetupOpts {
			json_results: true,
			..Default::default()
		},
	)
	.await;
	let (status, _, body) = send(&s, client_request(true, web_search_tool())).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let mut acc = MessageAccumulator::default();
	acc.feed_all(&parse_sse(&body));
	let message = acc.finish().unwrap();
	assert_eq!(
		block_types(&message),
		["server_tool_use", "web_search_tool_result", "text"],
		"{message}"
	);
	assert_eq!(
		message["content"][0]["input"]["query"],
		json!("super bowl lx winner")
	);
}

#[tokio::test]
async fn plain_text_search_output_keeps_the_text_only_answer() {
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
	assert_eq!(block_types(&message), ["text"], "{message}");
}

#[tokio::test]
async fn replayed_search_results_are_flattened_for_the_provider() {
	let s = setup(
		Model::AnswerOnly,
		false,
		3,
		ServerToolFailureMode::FailClosed,
		false,
	)
	.await;
	let body = json!({
		"model": "mock-model",
		"max_tokens": 256,
		"messages": [
			{"role": "user", "content": "Who won Super Bowl LX?"},
			{"role": "assistant", "content": [
				{"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {"query": "super bowl lx winner"}},
				{"type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
					{"type": "web_search_result", "url": "https://example.com/sb", "title": "Super Bowl LX", "page_age": null, "encrypted_content": "", "snippet": "Seattle won 24-17."}
				]},
				{"type": "text", "text": "Seattle won."},
			]},
			{"role": "user", "content": "By how much?"},
		],
		"tools": web_search_tool(),
	})
	.to_string()
	.into_bytes();
	let (status, _, body) = send(&s, body).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 1);
	let messages = requests[0]["messages"].as_array().unwrap();
	// The pair became assistant text the provider can read; nothing dangles.
	let assistant = messages
		.iter()
		.find(|m| m["role"] == "assistant")
		.unwrap_or_else(|| panic!("no assistant message in {}", requests[0]));
	let content = assistant["content"].to_string();
	assert!(
		content.contains("Web search for \\\"super bowl lx winner\\\":")
			&& content.contains("URL: https://example.com/sb")
			&& content.contains("Snippet: Seattle won 24-17.")
			&& content.contains("Seattle won."),
		"{content}"
	);
	assert!(assistant.get("tool_calls").is_none(), "{assistant}");
	assert!(
		messages.iter().all(|m| m["role"] != "tool"),
		"{}",
		requests[0]
	);
}

#[tokio::test]
async fn responses_colliding_mcp_tool_names_are_prefixed() {
	let s = setup_responses(Model::AnswerOnly, false, false).await;
	let tools = json!([
		{"type": "function", "name": "search", "parameters": {"type": "object"}},
		{"type": "mcp", "server_label": "search", "require_approval": "never"},
	]);
	let (status, _, body) = send_responses(&s, responses_request(false, tools)).await;
	assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
	let requests = recorded(&s.llm_requests);
	assert_eq!(requests.len(), 1);
	let mut names: Vec<&str> = requests[0]["tools"]
		.as_array()
		.unwrap()
		.iter()
		.map(|t| t["function"]["name"].as_str().unwrap())
		.collect();
	names.sort_unstable();
	assert_eq!(names, ["search", "search_search"], "{}", requests[0]);
}
