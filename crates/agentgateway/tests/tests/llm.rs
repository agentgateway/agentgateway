use crate::common::gateway::AgentGateway;
use agent_core::telemetry::testing;
use http::StatusCode;
use serde_json::json;
use tracing::warn;

fn llm_config(provider: &str, env: &str, model: &str) -> String {
	format!(
		r#"
config: {{}}
frontendPolicies:
  accessLog:
    add:
      streaming: llm.streaming
      body: string(response.body)
      req.id: request.headers["x-test-id"]
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: llm
      policies:
        backendAuth:
          key: ${env}
      backends:
      - ai:
          name: llm
          policies:
            ai:
              routes:
                /v1/chat/completions: completions
                /v1/messages: messages
                /v1/responses: responses
                /v1/count: anthropicTokenCount
                "*": passthrough
          provider:
            {provider}:
              model: {model}
"#
	)
}

#[tokio::test]
async fn test_openai_responses() {
	if !require_env("OPENAI_API_KEY") {
		return;
	}
	let gw = AgentGateway::new(llm_config("openAI", "OPENAI_API_KEY", "gpt-4.1-nano"))
		.await
		.unwrap();
	let resp = gw
		.send_request_json(
			"http://localhost/v1/responses",
			json!({
				"max_output_tokens": 16,
				"input": "give me a 1 word answer"
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/responses", false, &gw.test_id);
}

#[tokio::test]
async fn test_openai_responses_stream() {
	if !require_env("OPENAI_API_KEY") {
		return;
	}
	let gw = AgentGateway::new(llm_config("openAI", "OPENAI_API_KEY", "gpt-4.1-nano"))
		.await
		.unwrap();
	let resp = gw
		.send_request_json(
			"http://localhost/v1/responses",
			json!({
				"max_output_tokens": 16,
				"input": "give me a 1 word answer",
				"stream": true,
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/responses", true, &gw.test_id);
}

#[tokio::test]
async fn test_openai_completions() {
	if !require_env("OPENAI_API_KEY") {
		return;
	}
	let gw = AgentGateway::new(llm_config("openAI", "OPENAI_API_KEY", "gpt-4.1-nano"))
		.await
		.unwrap();
	let resp = gw
		.send_request_json(
			"http://localhost/v1/chat/completions",
			json!({
				"messages": [{
					"role": "user",
					"content": "give me a 1 word answer"
				}]
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/chat/completions", false, &gw.test_id);
}

#[tokio::test]
async fn test_openai_completions_streaming() {
	if !require_env("OPENAI_API_KEY") {
		return;
	}
	let gw = AgentGateway::new(llm_config("openAI", "OPENAI_API_KEY", "gpt-4.1-nano"))
		.await
		.unwrap();
	let resp = gw
		.send_request_json(
			"http://localhost/v1/chat/completions",
			json!({
				"messages": [{
					"role": "user",
					"content": "give me a 1 word answer"
				}],
				"stream": true
			}),
		)
		.await;

	assert_eq!(resp.status(), StatusCode::OK);
	assert_log("/v1/chat/completions", true, &gw.test_id);
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
		output > 1 && output < 100,
		"unexpected output tokens: {output}"
	);
	let stream = log.get("streaming").unwrap().as_bool().unwrap();
	assert_eq!(stream, streaming, "unexpected streaming value: {stream}");
}

fn require_env(var: &str) -> bool {
	testing::setup_test_logging();
	let found = std::env::var(var).is_ok();
	if !found {
		warn!("environment variable {} not set, skipping test", var);
	}
	found
}
