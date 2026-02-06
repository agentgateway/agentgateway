use agent_core::telemetry::testing;
use http::StatusCode;
use serde_json::json;
use tracing::warn;

use crate::common::gateway::AgentGateway;

// This module provides real LLM integration tests. These require API keys!
// Note: AGENTGATEWAY_E2E=true must be set to run any of these tests.
//
// Required Environment Variables (per provider):
// - OpenAI: OPENAI_API_KEY
// - Anthropic: ANTHROPIC_API_KEY
// - Gemini: GEMINI_API_KEY
// - Vertex: VERTEX_PROJECT (requires GCP implicit auth)
// - Bedrock: (requires AWS implicit auth)
// - Azure OpenAI: AZURE_HOST (requires implicit auth)
//
// Examples:
//
// 1. Run all E2E tests for all providers:
//    AGENTGATEWAY_E2E=true ANTHROPIC_API_KEY=... OPENAI_API_KEY=... cargo test --test integration tests::llm::
//
// 2. Run all tests for a specific provider (e.g., OpenAI):
//    AGENTGATEWAY_E2E=true OPENAI_API_KEY=... cargo test --test integration tests::llm::openai::
//
// 3. Run a specific targeted test case (e.g., Bedrock messages):
//    AGENTGATEWAY_E2E=true cargo test --test integration tests::llm::bedrock::messages
//
// Note: Some providers (Bedrock, Vertex) use implicit environment auth (AWS/GCP) instead of explicit keys.

macro_rules! send_completions_tests {
	($provider:expr, $env:expr, $model:expr) => {
		#[tokio::test]
		async fn completions() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_completions(&gw, false).await;
		}

		#[tokio::test]
		async fn completions_streaming() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_completions(&gw, true).await;
		}
	};
}

macro_rules! send_messages_tests {
	($provider:expr, $env:expr, $model:expr) => {
		#[tokio::test]
		async fn messages() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_messages(&gw, false).await;
		}

		#[tokio::test]
		async fn messages_streaming() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_messages(&gw, true).await;
		}
	};
}

macro_rules! send_messages_tool_tests {
	($provider:expr, $env:expr, $model:expr) => {
		#[tokio::test]
		async fn messages_tool_use() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_messages_with_tools(&gw).await;
		}

		#[tokio::test]
		async fn messages_parallel_tool_use() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_messages_with_parallel_tools(&gw).await;
		}

		#[tokio::test]
		async fn messages_multi_turn_tool_use() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_messages_multi_turn_tool_use(&gw).await;
		}
	};
}

macro_rules! send_completions_tool_tests {
	($provider:expr, $env:expr, $model:expr) => {
		#[tokio::test]
		async fn completions_tool_use() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_completions_with_tools(&gw).await;
		}
	};
}

macro_rules! send_responses_tests {
	($provider:expr, $env:expr, $model:expr) => {
		#[tokio::test]
		async fn responses() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_responses(&gw, false).await;
		}

		#[tokio::test]
		async fn responses_stream() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_responses(&gw, true).await;
		}
	};
}

macro_rules! send_embeddings_tests {
	($(#[$meta:meta])* $name:ident, $provider:expr, $env:expr, $model:expr, $expected_dimensions:expr) => {
		$(#[$meta])*
		#[tokio::test]
		async fn $name() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			send_embeddings(&gw, $expected_dimensions).await;
		}
	};
}

fn llm_config(provider: &str, env: &str, model: &str) -> String {
	let policies = if provider == "azureOpenAI" {
		r#"
      policies:
        backendAuth:
          azure:
            developerImplicit: {}
"#
		.to_string()
	} else if !env.is_empty() {
		format!(
			r#"
      policies:
        backendAuth:
          key: ${env}
"#
		)
	} else {
		"".to_string()
	};
	let extra = if provider == "bedrock" {
		r#"
              region: us-west-2
              "#
	} else if provider == "vertex" {
		r#"
              projectId: $VERTEX_PROJECT
              region: us-central1
              "#
	} else if provider == "azureOpenAI" {
		r#"
              host: $AZURE_HOST
              "#
	} else {
		""
	};
	format!(
		r#"
config: {{}}
frontendPolicies:
  accessLog:
    add:
      streaming: llm.streaming
      # body: string(response.body)
      req.id: request.headers["x-test-id"]
      token.count: llm.countTokens
      embeddings: json(response.body).data[0].embedding.size()
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: llm
{policies}
      backends:
      - ai:
          name: llm
          policies:
            ai:
              routes:
                /v1/chat/completions: completions
                /v1/messages: messages
                /v1/messages/count_tokens: anthropicTokenCount
                /v1/responses: responses
                /v1/embeddings: embeddings
                "*": passthrough
          provider:
            {provider}:
              model: {model}
{extra}
"#
	)
}

// === Provider-Specific E2E Test Suites ===
// Each module below instantiates the test macros for a specific backend provider.

mod openai {
	use super::*;
	send_responses_tests!("openAI", "OPENAI_API_KEY", "gpt-4o-mini");
	send_completions_tests!("openAI", "OPENAI_API_KEY", "gpt-4o-mini");
	send_completions_tool_tests!("openAI", "OPENAI_API_KEY", "gpt-4o-mini");
	send_messages_tests!("openAI", "OPENAI_API_KEY", "gpt-4o-mini");
	send_messages_tool_tests!("openAI", "OPENAI_API_KEY", "gpt-4o-mini");
	send_embeddings_tests!(
		embeddings,
		"openAI",
		"OPENAI_API_KEY",
		"text-embedding-3-small",
		None
	);
}

mod bedrock {
	use super::*;
	send_completions_tests!("bedrock", "", "us.amazon.nova-pro-v1:0");
	send_responses_tests!("bedrock", "", "us.anthropic.claude-3-5-haiku-20241022-v1:0");
	send_messages_tests!("bedrock", "", "us.anthropic.claude-3-5-haiku-20241022-v1:0");
	send_messages_tool_tests!("bedrock", "", "us.anthropic.claude-3-5-haiku-20241022-v1:0");
	send_embeddings_tests!(
		embeddings_titan,
		"bedrock",
		"",
		"amazon.titan-embed-text-v2:0",
		None
	);
	// Cohere does not respect overriding the dimension count
	send_embeddings_tests!(
		embeddings_cohere,
		"bedrock",
		"",
		"cohere.embed-english-v3",
		Some(1024)
	);

	#[tokio::test]
	async fn token_count() {
		let Some(gw) = setup("bedrock", "", "anthropic.claude-3-5-haiku-20241022-v1:0").await else {
			return;
		};
		send_anthropic_token_count(&gw).await;
	}
}

mod anthropic {
	use super::*;
	send_completions_tests!("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307");
	send_messages_tests!("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307");

	#[tokio::test]
	#[ignore]
	async fn responses() {
		let Some(gw) = setup("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307").await else {
			return;
		};
		send_responses(&gw, false).await;
	}

	#[tokio::test]
	#[ignore]
	async fn responses_streaming() {
		let Some(gw) = setup("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307").await else {
			return;
		};
		send_responses(&gw, true).await;
	}

	#[tokio::test]
	async fn token_count() {
		let Some(gw) = setup("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307").await else {
			return;
		};
		send_anthropic_token_count(&gw).await;
	}
}

mod gemini {
	use super::*;
	send_completions_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
	send_completions_tool_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
	send_messages_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
	send_messages_tool_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
}

mod vertex {
	use super::*;
	send_completions_tests!("vertex", "", "google/gemini-2.5-flash-lite");
	send_completions_tool_tests!("vertex", "", "google/gemini-2.5-flash-lite");
	send_messages_tests!("vertex", "", "google/gemini-2.5-flash-lite");
	send_messages_tool_tests!("vertex", "", "google/gemini-2.5-flash-lite");

	#[tokio::test]
	async fn completions_to_anthropic() {
		let Some(gw) = setup("vertex", "", "anthropic/claude-3-haiku@20240307").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	#[ignore]
	/// TODO(https://github.com/agentgateway/agentgateway/pull/800) support this
	async fn completions_streaming_to_anthropic() {
		let Some(gw) = setup("vertex", "", "anthropic/claude-3-haiku@20240307").await else {
			return;
		};
		send_completions(&gw, true).await;
	}

	// During testing I have been unable to make embeddings work at all with Vertex, with or without Agentgateway.
	// This is plausibly from using the OpenAI compatible endpoint?
	send_embeddings_tests!(
		#[ignore]
		embeddings,
		"vertex",
		"",
		"text-embedding-004",
		None
	);

	#[tokio::test]
	async fn token_count() {
		let Some(gw) = setup("vertex", "", "anthropic/claude-3-haiku@20240307").await else {
			return;
		};
		send_anthropic_token_count(&gw).await;
	}
}

mod azureopenai {
	use super::*;
	send_completions_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_completions_tool_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_messages_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_messages_tool_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_responses_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_embeddings_tests!(
		embeddings,
		"azureOpenAI",
		"",
		"text-embedding-3-small",
		None
	);
}

pub async fn setup(provider: &str, env: &str, model: &str) -> Option<AgentGateway> {
	// Explicitly opt in to avoid accidentally using implicit configs
	if !require_env("AGENTGATEWAY_E2E") {
		return None;
	}
	if !env.is_empty() && !require_env(env) {
		return None;
	}
	if provider == "vertex" && !require_env("VERTEX_PROJECT") {
		return None;
	}
	if provider == "azureOpenAI" && !require_env("AZURE_HOST") {
		return None;
	}
	let gw = AgentGateway::new(llm_config(provider, env, model))
		.await
		.unwrap();
	Some(gw)
}

fn assert_log(path: &str, streaming: bool, test_id: &str) {
	let logs = agent_core::telemetry::testing::find(&[
		("scope", "request"),
		("http.path", path),
		("req.id", test_id),
	]);
	assert_eq!(logs.len(), 1, "{logs:?}");
	let log = logs.first().unwrap();
	let output = log
		.get("gen_ai.usage.output_tokens")
		.unwrap()
		.as_i64()
		.unwrap();
	assert!(
		(1..100).contains(&output),
		"unexpected output tokens: {output}"
	);
	let stream = log.get("streaming").unwrap().as_bool().unwrap();
	assert_eq!(stream, streaming, "unexpected streaming value: {stream}");
}

fn assert_count_log(path: &str, test_id: &str) {
	let logs = agent_core::telemetry::testing::find(&[
		("scope", "request"),
		("http.path", path),
		("req.id", test_id),
	]);
	assert_eq!(logs.len(), 1, "{logs:?}");
	let log = logs.first().unwrap();
	let count = log.get("token.count").unwrap().as_u64().unwrap();
	assert!(count > 1 && count < 100, "unexpected count tokens: {count}");
	let stream = log.get("streaming").unwrap().as_bool().unwrap();
	assert!(!stream, "unexpected streaming value: {stream}");
}

fn assert_embeddings_log(path: &str, test_id: &str, expected: u64) {
	let logs = agent_core::telemetry::testing::find(&[
		("scope", "request"),
		("http.path", path),
		("req.id", test_id),
	]);
	assert_eq!(logs.len(), 1, "{logs:?}");
	let log = logs.first().unwrap();
	let count = log.get("embeddings").unwrap().as_i64().unwrap();
	assert_eq!(count, expected as i64, "unexpected count tokens: {count}");
	let stream = log.get("streaming").unwrap().as_bool().unwrap();
	assert!(!stream, "unexpected streaming value: {stream}");
	let dim_count = log
		.get("gen_ai.embeddings.dimension.count")
		.unwrap()
		.as_u64()
		.unwrap();
	assert_eq!(dim_count, 256, "unexpected dimension count: {dim_count}");
	let enc_format = log
		.get("gen_ai.request.encoding_formats")
		.unwrap()
		.as_str()
		.unwrap();
	assert_eq!(
		enc_format, "float",
		"unexpected encoding format: {enc_format}"
	);
}

fn require_env(var: &str) -> bool {
	testing::setup_test_logging();
	let found = std::env::var(var).is_ok();
	if !found {
		warn!("environment variable {} not set, skipping test", var);
	}
	found
}

pub async fn send_completions(gw: &AgentGateway, stream: bool) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/chat/completions",
			json!({
			"stream": stream,
				"messages": [{
					"role": "user",
					"content": "give me a 1 word answer"
				}]
			}),
		)
		.await;

	let test_id = resp
		.headers()
		.get("x-test-id")
		.unwrap()
		.to_str()
		.unwrap()
		.to_string();

	if resp.status() != StatusCode::OK {
		let body = resp.into_body();
		let bytes = http_body_util::BodyExt::collect(body)
			.await
			.unwrap()
			.to_bytes();
		println!("Error response body: {:?}", String::from_utf8_lossy(&bytes));
		panic!("Request failed with status {}", StatusCode::BAD_REQUEST);
	}

	let body = resp.into_body();
	let bytes = http_body_util::BodyExt::collect(body)
		.await
		.unwrap()
		.to_bytes();
	let body_str = String::from_utf8_lossy(&bytes);
	if stream {
		assert!(
			body_str.contains("data: "),
			"Streaming response missing 'data: ' prefix: {}",
			body_str
		);
	} else {
		assert!(
			!body_str.contains("data: "),
			"Non-streaming response contains 'data: ' prefix: {}",
			body_str
		);
	}

	assert_log("/v1/chat/completions", stream, &test_id);
}

pub async fn send_completions_with_tools(gw: &AgentGateway) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/chat/completions",
			json!({
				"messages": [{
					"role": "user",
					"content": "What is the weather in New York?"
				}],
				"tool_choice": "required",
				"tools": [{
					"type": "function",
					"function": {
						"name": "get_weather",
						"description": "Get the current weather in a given location",
						"parameters": {
							"type": "object",
							"properties": {
								"location": {
									"type": "string",
									"description": "The city and state, e.g. San Francisco, CA"
								},
								"unit": { "type": "string", "enum": ["celsius", "fahrenheit"] }
							},
							"required": ["location"]
						}
					}
				}]
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	let body = resp.into_body();
	let bytes = http_body_util::BodyExt::collect(body)
		.await
		.unwrap()
		.to_bytes();
	let body_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

	let choices = body_json
		.get("choices")
		.unwrap()
		.as_array()
		.expect("choices should be an array");
	let message = choices[0].get("message").unwrap();
	let tool_calls = message
		.get("tool_calls")
		.unwrap()
		.as_array()
		.expect("tool_calls should be an array");
	assert!(
		!tool_calls.is_empty(),
		"Response should contain tool_calls: {}",
		body_json
	);
	assert_eq!(
		tool_calls[0].get("function").unwrap().get("name").unwrap(),
		"get_weather"
	);
}

pub async fn send_messages_with_tools(gw: &AgentGateway) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 1024,
				"messages": [{
					"role": "user",
					"content": "What is the weather in New York?"
				}],
				"tool_choice": {"type": "any"},
				"tools": [{
					"name": "get_weather",
					"description": "Get the current weather in a given location",
					"input_schema": {
						"type": "object",
						"properties": {
							"location": {
								"type": "string",
								"description": "The city and state, e.g. San Francisco, CA"
							},
							"unit": { "type": "string", "enum": ["celsius", "fahrenheit"] }
						},
						"required": ["location"]
					}
				}]
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	let body = resp.into_body();
	let bytes = http_body_util::BodyExt::collect(body)
		.await
		.unwrap()
		.to_bytes();
	let body_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

	// Verify Anthropic Response Schema
	// Expectation: {"content": [{"type": "tool_use", "name": "get_weather", ...}], ...}
	let content = body_json
		.get("content")
		.expect("Response missing 'content'")
		.as_array()
		.expect("content should be array");

	// Find the tool_use block
	let tool_use = content
		.iter()
		.find(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"));
	assert!(
		tool_use.is_some(),
		"Response should contain a tool_use block: {:?}",
		body_json
	);

	let tool_use = tool_use.unwrap();
	assert_eq!(tool_use.get("name").unwrap(), "get_weather");
	assert!(
		tool_use.get("input").is_some(),
		"tool_use should have input"
	);
}

pub async fn send_messages_with_parallel_tools(gw: &AgentGateway) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 1024,
				"messages": [{
					"role": "user",
					"content": "What is the weather in New York and London? Use the `get_weather` tool for each."
				}],
				"tools": [{
					"name": "get_weather",
					"description": "Get the current weather in a given location",
					"input_schema": {
						"type": "object",
						"properties": {
							"location": {
								"type": "string",
								"description": "The city and state, e.g. San Francisco, CA"
							}
						},
						"required": ["location"]
					}
				}]
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	let body = resp.into_body();
	let bytes = http_body_util::BodyExt::collect(body)
		.await
		.unwrap()
		.to_bytes();
	let body_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

	// Verify Anthropic Response Schema for Parallel Tools
	let content = body_json
		.get("content")
		.expect("Response missing 'content'")
		.as_array()
		.expect("content should be array");

	// Count tool_use blocks
	let tool_calls: Vec<_> = content
		.iter()
		.filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
		.collect();

	// Most modern models (GPT-4o, Gemini 1.5/2.0) will correctly call the tool twice.
	assert!(
		tool_calls.len() >= 2,
		"Response should contain at least 2 tool_use blocks for parallel request: {}",
		body_json
	);

	for tc in tool_calls {
		assert!(tc.get("name").is_some());
		assert!(tc.get("input").is_some());
	}
}

pub async fn send_messages_multi_turn_tool_use(gw: &AgentGateway) {
	// Turn 1: Request tool use
	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 1024,
				"messages": [{
					"role": "user",
					"content": "What is the weather in New York?"
				}],
				"tool_choice": {"type": "any"},
				"tools": [{
					"name": "get_weather",
					"description": "Get the current weather in a given location",
					"input_schema": {
						"type": "object",
						"properties": {
							"location": { "type": "string" }
						},
						"required": ["location"]
					}
				}]
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	let bytes = http_body_util::BodyExt::collect(resp.into_body())
		.await
		.unwrap()
		.to_bytes();
	let body_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

	let content = body_json
		.get("content")
		.unwrap()
		.as_array()
		.expect("content should be array");
	let tool_use = content
		.iter()
		.find(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
		.expect("Response should contain a tool_use block");
	let tool_use_id = tool_use.get("id").unwrap().as_str().unwrap().to_string();

	// Turn 2: Send tool result
	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 1024,
				"messages": [
					{
						"role": "user",
						"content": "What is the weather in New York?"
					},
					{
						"role": "assistant",
						"content": [tool_use]
					},
					{
						"role": "user",
						"content": [
							{
								"type": "tool_result",
								"tool_use_id": tool_use_id,
								"content": "The weather is sunny and 75 degrees."
							}
						]
					}
				],
				"tools": [{
					"name": "get_weather",
					"description": "Get the current weather in a given location",
					"input_schema": {
						"type": "object",
						"properties": {
							"location": { "type": "string" }
						},
						"required": ["location"]
					}
				}]
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	let bytes = http_body_util::BodyExt::collect(resp.into_body())
		.await
		.unwrap()
		.to_bytes();
	let body_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

	let content = body_json
		.get("content")
		.unwrap()
		.as_array()
		.expect("content should be array");
	let text = content
		.iter()
		.find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
		.expect("Final response should contain a text block");
	let text_val = text.get("text").unwrap().as_str().unwrap();
	assert!(
		text_val.contains("75") || text_val.to_lowercase().contains("sunny"),
		"Final response should incorporate tool result: {}",
		text_val
	);
}

async fn send_responses(gw: &AgentGateway, stream: bool) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/responses",
			json!({
				"max_output_tokens": 16,
				"input": "give me a 1 word answer",
				"stream": stream,
			}),
		)
		.await;

	let test_id = resp
		.headers()
		.get("x-test-id")
		.unwrap()
		.to_str()
		.unwrap()
		.to_string();

	assert_eq!(resp.status(), StatusCode::OK);

	// Consume body to ensure request log is emitted
	let body = resp.into_body();
	let _ = http_body_util::BodyExt::collect(body).await.unwrap();

	assert_log("/v1/responses", stream, &test_id);
}

pub async fn send_messages(gw: &AgentGateway, stream: bool) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 16,
				"messages": [
					{"role": "user", "content": "give me a 1 word answer"}
				],
				"stream": stream
			}),
		)
		.await;

	let test_id = resp
		.headers()
		.get("x-test-id")
		.unwrap()
		.to_str()
		.unwrap()
		.to_string();

	if resp.status() != StatusCode::OK {
		let status = resp.status();
		let body = resp.into_body();
		let bytes = http_body_util::BodyExt::collect(body)
			.await
			.unwrap()
			.to_bytes();
		println!("Error response body: {:?}", String::from_utf8_lossy(&bytes));
		panic!("Request failed with status {}", status);
	}

	let body = resp.into_body();
	let bytes = http_body_util::BodyExt::collect(body)
		.await
		.unwrap()
		.to_bytes();
	let body_str = String::from_utf8_lossy(&bytes);
	if stream {
		assert!(
			body_str.contains("event: "),
			"Anthropic streaming response missing 'event: ' prefix: {}",
			body_str
		);
	} else {
		assert!(
			!body_str.contains("event: "),
			"Anthropic non-streaming response contains 'event: ' prefix: {}",
			body_str
		);
	}

	assert_log("/v1/messages", stream, &test_id);
}

async fn send_anthropic_token_count(gw: &AgentGateway) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages/count_tokens",
			json!({
				"messages": [
					{"role": "user", "content": "give me a 1 word answer"}
				],
			}),
		)
		.await;

	let test_id = resp
		.headers()
		.get("x-test-id")
		.unwrap()
		.to_str()
		.unwrap()
		.to_string();

	assert_eq!(resp.status(), StatusCode::OK);
	assert_count_log("/v1/messages/count_tokens", &test_id);
}

async fn send_embeddings(gw: &AgentGateway, expected_dimensions: Option<usize>) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/embeddings",
			json!({
				"dimensions": 256,
				"encoding_format": "float",
				"input": "banana"
			}),
		)
		.await;

	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert_eq!(status, StatusCode::OK, "response: {body}");

	assert_eq!(body["object"], "list");
	let data = body["data"].as_array().expect("data array");
	assert_eq!(data.len(), 1, "expected one embedding");
	assert_eq!(data[0]["object"], "embedding");
	assert_eq!(data[0]["index"], 0);
	let embedding = data[0]["embedding"].as_array().expect("embedding array");
	assert_eq!(
		embedding.len(),
		expected_dimensions.unwrap_or(256),
		"expected {} dimensions",
		expected_dimensions.unwrap_or(256)
	);
	assert!(body["model"].is_string(), "expected model in response");
	let prompt_tokens = body["usage"]["prompt_tokens"].as_u64().unwrap();
	let total_tokens = body["usage"]["total_tokens"].as_u64().unwrap();
	assert!(prompt_tokens > 0, "expected non-zero prompt_tokens");
	assert_eq!(
		prompt_tokens, total_tokens,
		"embeddings should have prompt_tokens == total_tokens"
	);

	assert_embeddings_log(
		"/v1/embeddings",
		&gw.test_id,
		expected_dimensions.unwrap_or(256) as u64,
	);
}
