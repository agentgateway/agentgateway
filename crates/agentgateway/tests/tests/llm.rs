use agent_core::telemetry::testing;
use agentgateway::http::Response;
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
//   - Optional: VERTEX_ANTHROPIC_MODEL to run Anthropic-on-Vertex specific tests
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

macro_rules! send_messages_image_tests {
	($provider:expr, $env:expr, $model:expr, $send_fn:ident) => {
		#[tokio::test]
		async fn messages_image() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			$send_fn(&gw).await;
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
		async fn responses_streaming() {
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

macro_rules! provider_model_test {
	($(#[$meta:meta])* $name:ident, $provider:expr, $env:expr, $model:expr, $send_fn:ident $(, $arg:expr)* $(,)?) => {
		$(#[$meta])*
		#[tokio::test]
		async fn $name() {
			let Some(gw) = setup($provider, $env, $model).await else {
				return;
			};
			$send_fn(&gw $(, $arg)*).await;
		}
	};
}

macro_rules! provider_env_model_test {
	($(#[$meta:meta])* $name:ident, $provider:expr, $env:expr, $model_env:expr, $send_fn:ident $(, $arg:expr)* $(,)?) => {
		$(#[$meta])*
		#[tokio::test]
		async fn $name() {
			let Some(model) = require_env_value($model_env) else {
				return;
			};
			let Some(gw) = setup($provider, $env, &model).await else {
				return;
			};
			$send_fn(&gw $(, $arg)*).await;
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
              region: us-east5
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
	send_messages_image_tests!(
		"openAI",
		"OPENAI_API_KEY",
		"gpt-4o-mini",
		send_messages_with_image_url
	);
	send_messages_tool_tests!("openAI", "OPENAI_API_KEY", "gpt-4o-mini");
	send_embeddings_tests!(
		embeddings,
		"openAI",
		"OPENAI_API_KEY",
		"text-embedding-3-small",
		None
	);
	provider_model_test!(
		messages_count_tokens_completions_backend,
		"openAI",
		"OPENAI_API_KEY",
		"gpt-4o-mini",
		send_messages_count_tokens
	);
}

mod bedrock {
	use super::*;

	const MODEL_NOVA_PRO: &str = "us.amazon.nova-pro-v1:0";
	const MODEL_TITAN_EMBED: &str = "amazon.titan-embed-text-v2:0";
	const MODEL_COHERE_EMBED: &str = "cohere.embed-english-v3";
	const MODEL_HAIKU_45_PROFILE: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";
	const MODEL_HAIKU_45_BASE: &str = "anthropic.claude-haiku-4-5-20251001-v1:0";
	const MODEL_OPUS_46_PROFILE: &str = "us.anthropic.claude-opus-4-6-v1";

	provider_model_test!(
		completions,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_completions,
		false
	);
	provider_model_test!(
		completions_streaming,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_completions,
		true
	);
	provider_model_test!(
		responses,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_responses,
		false
	);
	provider_model_test!(
		responses_streaming,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_responses,
		true
	);
	provider_model_test!(
		messages,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_messages,
		false
	);
	provider_model_test!(
		messages_streaming,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_messages,
		true
	);
	provider_model_test!(
		messages_image,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_messages_with_image_base64
	);
	provider_model_test!(
		embeddings_titan,
		"bedrock",
		"",
		MODEL_TITAN_EMBED,
		send_embeddings,
		None
	);
	// Cohere does not respect overriding the dimension count.
	provider_model_test!(
		embeddings_cohere,
		"bedrock",
		"",
		MODEL_COHERE_EMBED,
		send_embeddings,
		Some(1024)
	);
	provider_model_test!(
		messages_count_tokens,
		"bedrock",
		"",
		MODEL_HAIKU_45_BASE,
		send_messages_count_tokens
	);
	provider_model_test!(
		messages_count_tokens_completions_backend,
		"bedrock",
		"",
		MODEL_NOVA_PRO,
		send_messages_count_tokens
	);
	provider_model_test!(
		structured_output_haiku_45,
		"bedrock",
		"",
		MODEL_HAIKU_45_PROFILE,
		send_completions_structured_json
	);
	provider_model_test!(
		thinking_haiku_45,
		"bedrock",
		"",
		MODEL_HAIKU_45_PROFILE,
		send_messages_thinking_enabled
	);
	provider_model_test!(
		adaptive_thinking_rejected_haiku_45,
		"bedrock",
		"",
		MODEL_HAIKU_45_PROFILE,
		send_messages_adaptive_thinking_rejected
	);
	provider_model_test!(
		completions_reasoning_effort_opus_46,
		"bedrock",
		"",
		MODEL_OPUS_46_PROFILE,
		send_completions_reasoning_effort
	);
	provider_model_test!(
		responses_reasoning_effort_opus_46,
		"bedrock",
		"",
		MODEL_OPUS_46_PROFILE,
		send_responses_reasoning_effort
	);
	provider_model_test!(
		responses_thinking_budget_opus_46,
		"bedrock",
		"",
		MODEL_OPUS_46_PROFILE,
		send_responses_thinking_budget
	);
	provider_model_test!(
		adaptive_thinking_opus_46,
		"bedrock",
		"",
		MODEL_OPUS_46_PROFILE,
		send_messages_adaptive_thinking
	);
	provider_model_test!(
		output_config_effort_opus_46,
		"bedrock",
		"",
		MODEL_OPUS_46_PROFILE,
		send_messages_output_config_effort
	);
}

mod anthropic {
	use super::*;
	send_completions_tests!(
		"anthropic",
		"ANTHROPIC_API_KEY",
		"claude-haiku-4-5-20251001"
	);
	send_messages_tests!(
		"anthropic",
		"ANTHROPIC_API_KEY",
		"claude-haiku-4-5-20251001"
	);
	send_messages_image_tests!(
		"anthropic",
		"ANTHROPIC_API_KEY",
		"claude-haiku-4-5-20251001",
		send_messages_with_image_url
	);

	#[tokio::test]
	#[ignore]
	async fn responses() {
		let Some(gw) = setup(
			"anthropic",
			"ANTHROPIC_API_KEY",
			"claude-haiku-4-5-20251001",
		)
		.await
		else {
			return;
		};
		send_responses(&gw, false).await;
	}

	#[tokio::test]
	#[ignore]
	async fn responses_streaming() {
		let Some(gw) = setup(
			"anthropic",
			"ANTHROPIC_API_KEY",
			"claude-haiku-4-5-20251001",
		)
		.await
		else {
			return;
		};
		send_responses(&gw, true).await;
	}

	#[tokio::test]
	async fn messages_count_tokens() {
		let Some(gw) = setup(
			"anthropic",
			"ANTHROPIC_API_KEY",
			"claude-haiku-4-5-20251001",
		)
		.await
		else {
			return;
		};
		send_messages_count_tokens(&gw).await;
	}
}

mod gemini {
	use super::*;
	send_completions_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
	send_completions_tool_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
	send_messages_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
	send_messages_image_tests!(
		"gemini",
		"GEMINI_API_KEY",
		"gemini-2.5-flash",
		send_messages_with_image_url
	);
	send_messages_tool_tests!("gemini", "GEMINI_API_KEY", "gemini-2.5-flash");
	provider_model_test!(
		messages_count_tokens_completions_backend,
		"gemini",
		"GEMINI_API_KEY",
		"gemini-2.5-flash",
		send_messages_count_tokens
	);
}

mod vertex {
	use super::*;
	send_completions_tests!("vertex", "", "google/gemini-2.5-flash-lite");
	send_completions_tool_tests!("vertex", "", "google/gemini-2.5-flash-lite");
	send_messages_tests!("vertex", "", "google/gemini-2.5-flash-lite");
	send_messages_image_tests!(
		"vertex",
		"",
		"google/gemini-2.5-flash-lite",
		send_messages_with_image_url
	);
	send_messages_tool_tests!("vertex", "", "google/gemini-2.5-flash-lite");

	// TODO(https://github.com/agentgateway/agentgateway/pull/909) support this
	provider_env_model_test!(
		completions_to_anthropic,
		"vertex",
		"",
		"VERTEX_ANTHROPIC_MODEL",
		send_completions,
		false
	);
	provider_env_model_test!(
		#[ignore]
		completions_streaming_to_anthropic,
		"vertex",
		"",
		"VERTEX_ANTHROPIC_MODEL",
		send_completions,
		true
	);
	provider_env_model_test!(
		messages_anthropic,
		"vertex",
		"",
		"VERTEX_ANTHROPIC_MODEL",
		send_messages,
		false
	);
	provider_env_model_test!(
		messages_streaming_anthropic,
		"vertex",
		"",
		"VERTEX_ANTHROPIC_MODEL",
		send_messages,
		true
	);
	provider_model_test!(
		embeddings,
		"vertex",
		"",
		"text-embedding-004",
		send_embeddings,
		None
	);
	provider_env_model_test!(
		messages_count_tokens,
		"vertex",
		"",
		"VERTEX_ANTHROPIC_MODEL",
		send_messages_count_tokens
	);
	provider_model_test!(
		messages_count_tokens_completions_backend,
		"vertex",
		"",
		"google/gemini-2.5-flash-lite",
		send_messages_count_tokens
	);
}

mod azureopenai {
	use super::*;
	send_completions_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_completions_tool_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_messages_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_messages_image_tests!(
		"azureOpenAI",
		"",
		"gpt-4o-mini",
		send_messages_with_image_url
	);
	send_messages_tool_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_responses_tests!("azureOpenAI", "", "gpt-4o-mini");
	send_embeddings_tests!(
		embeddings,
		"azureOpenAI",
		"",
		"text-embedding-3-small",
		None
	);
	provider_model_test!(
		messages_count_tokens_completions_backend,
		"azureOpenAI",
		"",
		"gpt-4o-mini",
		send_messages_count_tokens
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

async fn assert_log(path: &str, streaming: bool, test_id: &str) {
	assert_log_with_output_range(path, streaming, test_id, 1, 100).await;
}

async fn assert_log_with_output_range(
	path: &str,
	streaming: bool,
	test_id: &str,
	min: i64,
	max: i64,
) {
	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", path),
		("req.id", test_id),
	])
	.await
	.unwrap();
	let output = log
		.get("gen_ai.usage.output_tokens")
		.unwrap()
		.as_i64()
		.unwrap();
	assert!(
		(min..max).contains(&output),
		"unexpected output tokens: {output}; expected [{min}, {max})"
	);
	let stream = log.get("streaming").unwrap().as_bool().unwrap();
	assert_eq!(stream, streaming, "unexpected streaming value: {stream}");
}

async fn assert_count_log(path: &str, test_id: &str) {
	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", path),
		("req.id", test_id),
	])
	.await
	.unwrap();
	if let Some(stream) = log.get("streaming").and_then(serde_json::Value::as_bool) {
		assert!(!stream, "unexpected streaming value: {stream}");
	}
	if let Some(count) = log.get("token.count").and_then(serde_json::Value::as_u64) {
		assert!(count > 1 && count < 100, "unexpected count tokens: {count}");
	}
}

async fn assert_embeddings_log(path: &str, test_id: &str, expected: u64) {
	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", path),
		("req.id", test_id),
	])
	.await
	.unwrap();
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

fn require_env_value(var: &str) -> Option<String> {
	testing::setup_test_logging();
	let Ok(value) = std::env::var(var) else {
		warn!("environment variable {} not set, skipping test", var);
		return None;
	};

	if value.trim().is_empty() {
		warn!("environment variable {} is empty, skipping test", var);
		return None;
	}

	Some(value)
}

fn test_id_from_response(resp: &Response) -> String {
	resp
		.headers()
		.get("x-test-id")
		.and_then(|v| v.to_str().ok())
		.expect("response should include x-test-id header")
		.to_string()
}

async fn send_completions(gw: &AgentGateway, stream: bool) {
	send_completions_request(gw, stream, None, None, "give me a 1 word answer").await;
}

async fn send_completions_request(
	gw: &AgentGateway,
	stream: bool,
	max_tokens: Option<u32>,
	reasoning_effort: Option<&str>,
	prompt: &str,
) {
	let mut req = json!({
		"stream": stream,
		"messages": [{
			"role": "user",
			"content": prompt
		}]
	});

	if let Some(max_tokens) = max_tokens {
		req["max_tokens"] = json!(max_tokens);
	}
	if let Some(reasoning_effort) = reasoning_effort {
		req["reasoning_effort"] = json!(reasoning_effort);
	}

	let resp = gw
		.send_request_json("http://localhost/v1/chat/completions", req)
		.await;

	let test_id = test_id_from_response(&resp);
	let status = resp.status();

	if status != StatusCode::OK {
		let body = resp.into_body();
		let bytes = http_body_util::BodyExt::collect(body)
			.await
			.unwrap()
			.to_bytes();
		println!("Error response body: {:?}", String::from_utf8_lossy(&bytes));
		panic!("Request failed with status {status}");
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

	assert_log("/v1/chat/completions", stream, &test_id).await;
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

	// Tool-call cardinality is model-dependent; verify schema/shape instead of exact count.
	assert!(
		!tool_calls.is_empty(),
		"Response should contain at least one tool_use block for parallel request: {}",
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
				}],
				"tool_choice": {"type": "none"}
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
		!text_val.trim().is_empty(),
		"Final response should contain non-empty text: {}",
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

	let test_id = test_id_from_response(&resp);
	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/responses", stream, &test_id).await;
}

pub async fn send_messages(gw: &AgentGateway, stream: bool) {
	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 1024,
				"messages": [
					{"role": "user", "content": "give me a 1 word answer"}
				],
				"stream": stream
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/messages", stream, &test_id).await;
}

async fn send_messages_with_image_base64(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	const ONE_BY_ONE_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4//8/AwAI/AL+X0N5WQAAAABJRU5ErkJggg==";

	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 128,
				"messages": [
					{
						"role": "user",
						"content": [
							{"type": "text", "text": "Describe the image briefly."},
							{
								"type": "image",
								"source": {
									"type": "base64",
									"media_type": "image/png",
									"data": ONE_BY_ONE_PNG_B64
								}
							}
						]
					}
				]
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert_eq!(status, StatusCode::OK, "response: {body}");
	assert_log_with_output_range("/v1/messages", false, &test_id, 1, 300).await;
}

async fn send_messages_with_image_url(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 128,
				"messages": [
					{
						"role": "user",
						"content": [
							{"type": "text", "text": "Describe the image briefly."},
							{
								"type": "image",
								"source": {
									"type": "url",
									"url": "https://upload.wikimedia.org/wikipedia/commons/4/47/PNG_transparency_demonstration_1.png"
								}
							}
						]
					}
				]
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert_eq!(status, StatusCode::OK, "response: {body}");
	assert_log_with_output_range("/v1/messages", false, &test_id, 1, 300).await;
}

async fn send_messages_count_tokens(gw: &AgentGateway) {
	use http_body_util::BodyExt;

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

	let test_id = test_id_from_response(&resp);
	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert_eq!(status, StatusCode::OK, "response: {body}");
	let count = body
		.get("input_tokens")
		.or_else(|| body.get("inputTokens"))
		.and_then(serde_json::Value::as_u64)
		.expect("input_tokens/inputTokens should be a positive integer");
	assert!(
		(1..100).contains(&count),
		"unexpected input_tokens in response body: {count}"
	);
	assert_count_log("/v1/messages/count_tokens", &test_id).await;
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

	let test_id = test_id_from_response(&resp);
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
		&test_id,
		expected_dimensions.unwrap_or(256) as u64,
	)
	.await;
}

async fn send_messages_adaptive_thinking(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 4096,
				"thinking": {
					"type": "adaptive"
				},
				"output_config": {
					"effort": "high"
				},
				"messages": [{
					"role": "user",
					"content": "Summarize the benefits of automated testing in one sentence."
				}]
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	assert_eq!(resp.status(), StatusCode::OK);
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	let content = body.get("content").unwrap().as_array().unwrap();
	assert!(!content.is_empty(), "content should not be empty");

	assert_log_with_output_range("/v1/messages", false, &test_id, 1, 3000).await;
}

async fn send_completions_structured_json(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/chat/completions",
			json!({
				"stream": false,
				"messages": [{
					"role": "user",
					"content": "Return valid JSON with exactly one key named answer and a short string value."
				}],
				"response_format": {
					"type": "json_schema",
					"json_schema": {
						"name": "answer_schema",
						"strict": true,
						"schema": {
							"type": "object",
							"additionalProperties": false,
							"properties": {
								"answer": {
									"type": "string"
								}
							},
							"required": ["answer"]
						}
					}
				}
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert_eq!(status, StatusCode::OK, "response: {body}");

	let content = body["choices"][0]["message"]["content"]
		.as_str()
		.expect("structured output content should be a string");
	let parsed_content: serde_json::Value =
		serde_json::from_str(content).expect("structured output content should be valid json");
	let answer = parsed_content["answer"]
		.as_str()
		.expect("structured output should include answer string");
	assert!(
		!answer.is_empty(),
		"structured output answer should not be empty"
	);

	assert_log_with_output_range("/v1/chat/completions", false, &test_id, 1, 1000).await;
}

async fn send_messages_thinking_enabled(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 4096,
				"thinking": {
					"type": "enabled",
					"budget_tokens": 1024
				},
				"messages": [{
					"role": "user",
					"content": "Summarize the benefits of automated testing in one sentence."
				}]
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	assert_eq!(resp.status(), StatusCode::OK);
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	let content = body.get("content").unwrap().as_array().unwrap();
	assert!(!content.is_empty(), "content should not be empty");

	assert_log_with_output_range("/v1/messages", false, &test_id, 1, 1000).await;
}

async fn send_messages_output_config_effort(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 4096,
				"output_config": {
					"effort": "high"
				},
				"messages": [{
					"role": "user",
					"content": "Summarize the benefits of automated testing in one sentence."
				}]
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	assert_eq!(resp.status(), StatusCode::OK);
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	let content = body.get("content").unwrap().as_array().unwrap();
	assert!(!content.is_empty(), "content should not be empty");

	assert_log_with_output_range("/v1/messages", false, &test_id, 1, 1000).await;
}

async fn send_completions_reasoning_effort(gw: &AgentGateway) {
	send_completions_request(
		gw,
		false,
		Some(2048),
		Some("low"),
		"Summarize the benefits of automated testing in one sentence.",
	)
	.await;
}

async fn send_responses_reasoning_effort(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/responses",
			json!({
				"max_output_tokens": 2048,
				"input": "Summarize the benefits of automated testing in one sentence.",
				"reasoning": {
					"effort": "low"
				}
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert_eq!(status, StatusCode::OK, "response: {body}");

	assert_log_with_output_range("/v1/responses", false, &test_id, 1, 2000).await;
}

async fn send_responses_thinking_budget(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/responses",
			json!({
				"max_output_tokens": 4096,
				"input": "Summarize the benefits of automated testing in one sentence.",
				"reasoning": {
					"effort": "high"
				},
				"vendor_extensions": {
					"thinking_budget_tokens": 3072
				}
			}),
		)
		.await;

	let test_id = test_id_from_response(&resp);
	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert_eq!(status, StatusCode::OK, "response: {body}");

	assert_log_with_output_range("/v1/responses", false, &test_id, 1, 2000).await;
}

async fn send_messages_adaptive_thinking_rejected(gw: &AgentGateway) {
	use http_body_util::BodyExt;

	let resp = gw
		.send_request_json(
			"http://localhost/v1/messages",
			json!({
				"max_tokens": 4096,
				"thinking": {
					"type": "adaptive"
				},
				"messages": [{
					"role": "user",
					"content": "Summarize the benefits of automated testing in one sentence."
				}]
			}),
		)
		.await;
	let status = resp.status();
	let body = resp.into_body().collect().await.expect("collect body");
	let body: serde_json::Value = serde_json::from_slice(&body.to_bytes()).expect("parse json");
	assert!(
		status.is_client_error(),
		"expected client error for unsupported adaptive thinking, got status={status}, body={body}"
	);
}
