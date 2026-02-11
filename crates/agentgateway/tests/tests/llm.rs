use agent_core::telemetry::testing;
use http::StatusCode;
use serde_json::json;
use tracing::warn;

use crate::common::gateway::AgentGateway;

// This module provides real LLM integration tests. These require API keys!
// Example running all tests:
//     AZURE_HOST=xxx.azure.com \
//     VERTEX_PROJECT=octo-386314 \
//     GEMINI_API_KEY=`cat ~/.secrets/gemini` \
//     ANTHROPIC_API_KEY=`cat ~/.secrets/anthropic` \
//     OPENAI_API_KEY=`cat ~/.secrets/openai`
//     AGENTGATEWAY_E2E=true \
//     cargo test --test integration tests::llm::
//
// Note: AGENTGATEWAY_E2E must be set to run any tests.

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

mod openai {
	use super::*;
	#[tokio::test]
	async fn responses() {
		let Some(gw) = setup("openAI", "OPENAI_API_KEY", "gpt-4.1-nano").await else {
			return;
		};
		send_responses(&gw, false).await;
	}

	#[tokio::test]
	async fn responses_stream() {
		let Some(gw) = setup("openAI", "OPENAI_API_KEY", "gpt-4.1-nano").await else {
			return;
		};
		send_responses(&gw, true).await;
	}

	#[tokio::test]
	async fn completions() {
		let Some(gw) = setup("openAI", "OPENAI_API_KEY", "gpt-4.1-nano").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	async fn completions_streaming() {
		let Some(gw) = setup("openAI", "OPENAI_API_KEY", "gpt-4.1-nano").await else {
			return;
		};
		send_completions(&gw, true).await;
	}

	#[tokio::test]
	#[ignore] // TODO
	async fn messages() {
		let Some(gw) = setup("openAI", "OPENAI_API_KEY", "gpt-4.1-nano").await else {
			return;
		};
		send_messages(&gw, false).await;
	}

	#[tokio::test]
	#[ignore] // TODO
	async fn messages_streaming() {
		let Some(gw) = setup("openAI", "OPENAI_API_KEY", "gpt-4.1-nano").await else {
			return;
		};
		send_messages(&gw, true).await;
	}

	#[tokio::test]
	async fn embeddings() {
		let Some(gw) = setup("openAI", "OPENAI_API_KEY", "text-embedding-3-small").await else {
			return;
		};
		send_embeddings(&gw, None).await;
	}
}

mod bedrock {
	use super::*;

	#[tokio::test]
	async fn completions() {
		let Some(gw) = setup("bedrock", "", "us.amazon.nova-pro-v1:0").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	async fn completions_streaming() {
		let Some(gw) = setup("bedrock", "", "us.amazon.nova-pro-v1:0").await else {
			return;
		};
		send_completions(&gw, true).await;
	}

	#[tokio::test]
	async fn responses() {
		let Some(gw) = setup("bedrock", "", "us.amazon.nova-pro-v1:0").await else {
			return;
		};
		send_responses(&gw, false).await;
	}

	#[tokio::test]
	async fn responses_streaming() {
		let Some(gw) = setup("bedrock", "", "us.amazon.nova-pro-v1:0").await else {
			return;
		};
		send_responses(&gw, true).await;
	}

	#[tokio::test]
	async fn messages() {
		let Some(gw) = setup("bedrock", "", "us.amazon.nova-pro-v1:0").await else {
			return;
		};
		send_messages(&gw, false).await;
	}

	#[tokio::test]
	async fn messages_streaming() {
		let Some(gw) = setup("bedrock", "", "us.amazon.nova-pro-v1:0").await else {
			return;
		};
		send_messages(&gw, true).await;
	}

	#[tokio::test]
	async fn embeddings_titan() {
		let Some(gw) = setup("bedrock", "", "amazon.titan-embed-text-v2:0").await else {
			return;
		};
		send_embeddings(&gw, None).await;
	}

	#[tokio::test]
	async fn embeddings_cohere() {
		let Some(gw) = setup("bedrock", "", "cohere.embed-english-v3").await else {
			return;
		};
		// Cohere does not respect overriding the dimension count
		send_embeddings(&gw, Some(1024)).await;
	}

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

	#[tokio::test]
	async fn completions() {
		let Some(gw) = setup("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	async fn completions_streaming() {
		let Some(gw) = setup("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307").await else {
			return;
		};
		send_completions(&gw, true).await;
	}

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
	async fn messages() {
		let Some(gw) = setup("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307").await else {
			return;
		};
		send_messages(&gw, false).await;
	}

	#[tokio::test]
	async fn messages_streaming() {
		let Some(gw) = setup("anthropic", "ANTHROPIC_API_KEY", "claude-3-haiku-20240307").await else {
			return;
		};
		send_messages(&gw, true).await;
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

	#[tokio::test]
	async fn completions() {
		let Some(gw) = setup("gemini", "GEMINI_API_KEY", "gemini-2.5-flash").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	async fn completions_streaming() {
		let Some(gw) = setup("gemini", "GEMINI_API_KEY", "gemini-2.5-flash").await else {
			return;
		};
		send_completions(&gw, true).await;
	}
}

mod vertex {
	use super::*;

	#[tokio::test]
	async fn completions() {
		let Some(gw) = setup("vertex", "", "google/gemini-2.5-flash-lite").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	async fn completions_to_anthropic() {
		let Some(gw) = setup("vertex", "", "claude-haiku-4-5@20251001").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	#[ignore]
	/// TODO(https://github.com/agentgateway/agentgateway/pull/909) support this
	async fn completions_streaming_to_anthropic() {
		let Some(gw) = setup("vertex", "", "claude-haiku-4-5@20251001").await else {
			return;
		};
		send_completions(&gw, true).await;
	}

	#[tokio::test]
	async fn completions_streaming() {
		let Some(gw) = setup("vertex", "", "google/gemini-2.5-flash-lite").await else {
			return;
		};
		send_completions(&gw, true).await;
	}

	#[tokio::test]
	async fn messages() {
		let Some(gw) = setup("vertex", "", "claude-haiku-4-5@20251001").await else {
			return;
		};
		send_messages(&gw, false).await;
	}

	#[tokio::test]
	async fn messages_streaming() {
		let Some(gw) = setup("vertex", "", "claude-haiku-4-5@20251001").await else {
			return;
		};
		send_messages(&gw, true).await;
	}

	#[tokio::test]
	async fn embeddings() {
		let Some(gw) = setup("vertex", "", "text-embedding-004").await else {
			return;
		};
		send_embeddings(&gw, None).await;
	}

	#[tokio::test]
	async fn token_count() {
		let Some(gw) = setup("vertex", "", "claude-haiku-4-5@20251001").await else {
			return;
		};
		send_anthropic_token_count(&gw).await;
	}
}

mod azureopenai {
	use super::*;

	#[tokio::test]
	async fn completions() {
		let Some(gw) = setup("azureOpenAI", "", "gpt-4o-mini").await else {
			return;
		};
		send_completions(&gw, false).await;
	}

	#[tokio::test]
	async fn completions_streaming() {
		let Some(gw) = setup("azureOpenAI", "", "gpt-4o-mini").await else {
			return;
		};
		send_completions(&gw, true).await;
	}

	#[tokio::test]
	async fn responses() {
		let Some(gw) = setup("azureOpenAI", "", "gpt-4o-mini").await else {
			return;
		};
		send_responses(&gw, false).await;
	}

	#[tokio::test]
	async fn responses_stream() {
		let Some(gw) = setup("azureOpenAI", "", "gpt-4o-mini").await else {
			return;
		};
		send_responses(&gw, true).await;
	}

	#[tokio::test]
	async fn embeddings() {
		let Some(gw) = setup("azureOpenAI", "", "text-embedding-3-small").await else {
			return;
		};
		send_embeddings(&gw, None).await;
	}
}

async fn setup(provider: &str, env: &str, model: &str) -> Option<AgentGateway> {
	// Explicitly opt in to avoid accidentally using implicit configs
	if !require_env("AGENTGATEWAY_E2E") {
		return None;
	}
	if !env.is_empty() && !require_env("OPENAI_API_KEY") {
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

async fn send_completions(gw: &AgentGateway, stream: bool) {
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

	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/chat/completions", stream, &gw.test_id);
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

	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/responses", stream, &gw.test_id);
}

async fn send_messages(gw: &AgentGateway, stream: bool) {
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

	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/messages", stream, &gw.test_id);
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

	assert_eq!(resp.status(), StatusCode::OK);
	assert_count_log("/v1/messages/count_tokens", &gw.test_id);
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
