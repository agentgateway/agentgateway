use agentgateway::llm::{AIProvider, copilot, custom, gemini, openai};
use agentgateway::test_helpers::ratelimitmock;
use agentgateway::types::agent::TrafficPolicy;
use tokio::sync::mpsc;
use url::Position;

use crate::common::prelude::*;

macro_rules! llm_body {
	($path:literal) => {
		include_bytes!(concat!("../../../llm/src/tests/", $path))
	};
}

#[tokio::test]
async fn llm_openai() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
		"{}",
	);

	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23
	});
	assert_llm(io, llm_body!("requests/completions/basic.json"), want).await;
}

#[rstest::rstest]
#[case::configured_key("configured-token")]
#[case::empty_key("")]
#[tokio::test]
async fn llm_copilot_explicit_auth(#[case] credential: &str) {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let mut provider = llm_named_provider(
		&mock,
		AIProvider::Copilot(copilot::Provider {
			model_override: None,
		}),
		false,
	);
	provider.policies =
		Some(serde_json::from_value(json!({"backendAuth": {"key": credential}})).unwrap());
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	let response = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/chat/completions",
		llm_body!("requests/completions/basic.json"),
	)
	.await;
	assert_eq!(response.status(), StatusCode::OK);
	read_body_raw(response.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		request.headers[header::AUTHORIZATION].to_str().unwrap(),
		format!("Bearer {credential}").trim_end()
	);
	assert_eq!(request.headers[header::CONTENT_TYPE], "application/json");
	assert_eq!(
		request.headers["editor-version"],
		concat!("agentgateway/", env!("CARGO_PKG_VERSION"))
	);
	assert_eq!(request.headers["openai-intent"], "conversation-agent");
	assert_eq!(request.headers["x-github-api-version"], "2025-10-01");
	assert_eq!(request.headers["x-initiator"], "agent");
	assert_eq!(request.headers["x-interaction-type"], "conversation-agent");
}

#[tokio::test]
async fn llm_streaming_response_transformation_sees_llm_context() {
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
			llm_body!("response/completions/stream-content_filter.json").to_vec(),
			"text/event-stream",
		))
		.mount(&mock)
		.await;
	let (_mock, mut bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
		"{}",
	);
	bind
		.attach_route_policy(json!({
			"transformations": {
				"response": {
					"set": {
						"x-llm-provider": "llm.provider",
						"x-llm-request-model": "llm.requestModel",
						"x-llm-streaming": "string(llm.streaming)"
					}
				}
			}
		}))
		.await;

	let mut body: Value =
		serde_json::from_slice(llm_body!("requests/completions/basic.json")).unwrap();
	body["stream"] = json!(true);
	let res = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(serde_json::to_vec(&body).unwrap()))
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let header = |name: &str| {
		res
			.headers()
			.get(name)
			.map(|v| v.to_str().unwrap().to_string())
	};
	assert_eq!(header("x-llm-provider").as_deref(), Some("openai"));
	assert_eq!(header("x-llm-request-model").as_deref(), Some("replaceme"));
	assert_eq!(header("x-llm-streaming").as_deref(), Some("true"));
	let text = String::from_utf8(read_body_raw(res.into_body()).await.to_vec()).unwrap();
	assert!(text.contains("data: "), "expected SSE body, got: {text}");
}

#[tokio::test]
async fn llm_openai_tokenize() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		true,
		"{}",
	);

	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23
	});
	assert_llm(io, llm_body!("requests/completions/basic.json"), want).await;
}

const GEMINI_SIGNATURE: &str = "opaque-thought-signature";
const GEMINI_SIGNATURE_CARRIER_PREFIX: &str = "agentgateway.gemini.thought-signature.v1:";

fn gemini_signature_carrier() -> String {
	format!(
		"{GEMINI_SIGNATURE_CARRIER_PREFIX}{}",
		json!({"call_id": "call_1", "signature": GEMINI_SIGNATURE})
	)
}

// A client replays the assistant turn it received from a Gemini-backed route. Only Gemini
// understands the signature, so every other provider must get a request without it.
fn gemini_signature_replay(api: &str, model: &str) -> (&'static str, Value) {
	let carrier = gemini_signature_carrier();
	match api {
		"messages" => (
			"/v1/messages",
			json!({
				"model": model,
				"max_tokens": 64,
				"messages": [
					{"role": "user", "content": "What is the weather?"},
					{"role": "assistant", "content": [
						{"type": "thinking", "thinking": "", "signature": carrier},
						{"type": "tool_use", "id": "call_1", "name": "get_weather", "input": {"city": "Paris"}}
					]},
					{"role": "user", "content": [
						{"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"}
					]}
				]
			}),
		),
		"responses" => (
			"/v1/responses",
			json!({
				"model": model,
				"input": [
					{"role": "user", "content": "What is the weather?"},
					{"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": carrier},
					{"type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{\"city\":\"Paris\"}"},
					{"type": "function_call_output", "call_id": "call_1", "output": "sunny"}
				]
			}),
		),
		_ => unreachable!("unknown client API {api}"),
	}
}

#[rstest::rstest]
#[case::messages_to_openai("messages", false)]
#[case::responses_to_gemini("responses", true)]
#[tokio::test]
async fn llm_gemini_thought_signature_follows_provider(#[case] api: &str, #[case] is_gemini: bool) {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let provider = if is_gemini {
		AIProvider::Gemini(gemini::Provider {
			model_override: None,
		})
	} else {
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		})
	};
	let (mock, mut bind, io) = setup_llm_mock(mock, provider, false, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {
				"routes": {
					"/v1/messages": "messages",
					"/v1/responses": "responses"
				}
			}
		}))
		.await;
	let model = if is_gemini {
		"gemini-3.8-flash"
	} else {
		"gpt-4o"
	};
	let (path, body) = gemini_signature_replay(api, model);

	let res = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.to_string().as_bytes(),
	)
	.await;
	// The canned upstream response may not match the format the request was translated to;
	// only the upstream request matters here.
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	let upstream = serde_json::from_slice::<Value>(&request.body).unwrap();
	let upstream_text = upstream.to_string();
	assert!(
		!upstream_text.contains(GEMINI_SIGNATURE_CARRIER_PREFIX),
		"carrier leaked upstream: {upstream_text}"
	);
	if is_gemini {
		let tool_call = upstream["messages"]
			.as_array()
			.unwrap()
			.iter()
			.find_map(|message| message["tool_calls"].get(0))
			.unwrap_or_else(|| panic!("no replayed tool call: {upstream_text}"));
		assert_eq!(
			tool_call["extra_content"]["google"]["thought_signature"],
			GEMINI_SIGNATURE
		);
	} else {
		assert!(
			!upstream_text.contains(GEMINI_SIGNATURE) && !upstream_text.contains("extra_content"),
			"signature leaked upstream: {upstream_text}"
		);
	}
}

#[tokio::test]
async fn llm_token_budget_persists_and_blocks_requests() {
	let pool =
		agentgateway::database::DatabasePool::connect_with_max_connections("sqlite::memory:", Some(1))
			.await
			.unwrap();
	let policy = json!({
		"apiKey": {
			"keys": [
				{
					"key": "sk-budget",
					"metadata": {"name": "budgeted-key"},
					"budgets": [{
						"name": "tokens",
						"limit": {"unit": "Tokens", "amount": 40},
						"window": {"rolling": "1h"},
						"onBudgetExceeded": "Block"
					}]
				},
				{
					"key": "sk-other-budget",
					"metadata": {"name": "budgeted-key"},
					"budgets": [{
						"name": "tokens",
						"limit": {"unit": "Tokens", "amount": 40},
						"window": {"rolling": "1h"},
						"onBudgetExceeded": "Block"
					}]
				}
			],
			"mode": "strict"
		}
	});

	let mock = body_mock(include_bytes!(
		"../../../llm/src/tests/response/completions/basic.json"
	))
	.await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
	);
	let config = agentgateway::config::parse_config("{}".to_string(), None).unwrap();
	config.budget_policy.initialize(pool.clone()).await.unwrap();
	let budget_policy = config.budget_policy.clone();
	let (_mock, mut bind, io) = setup_llm_named_provider_mock_with_config(mock, provider, config);
	bind.attach_route_policy(policy.clone()).await;

	let response = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions")
		.header("authorization", "Bearer sk-budget")
		.body(Body::from(
			include_bytes!("../../../llm/src/tests/requests/completions/basic.json").to_vec(),
		))
		.send(io.clone())
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::OK);
	response.into_body().collect().await.unwrap();

	// Display names are not identities: another key with the same metadata name and budget name
	// has an independent counter.
	let response = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions")
		.header("authorization", "Bearer sk-other-budget")
		.body(Body::from(
			include_bytes!("../../../llm/src/tests/requests/completions/basic.json").to_vec(),
		))
		.send(io)
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::OK);
	response.into_body().collect().await.unwrap();
	budget_policy.flush().await.unwrap();

	let mock = body_mock(include_bytes!(
		"../../../llm/src/tests/response/completions/basic.json"
	))
	.await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
	);
	let config = agentgateway::config::parse_config("{}".to_string(), None).unwrap();
	config.budget_policy.initialize(pool).await.unwrap();
	let (mock, mut bind, io) = setup_llm_named_provider_mock_with_config(mock, provider, config);
	bind.attach_route_policy(policy).await;

	let response = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions")
		.header("authorization", "Bearer sk-budget")
		.body(Body::from(
			include_bytes!("../../../llm/src/tests/requests/completions/basic.json").to_vec(),
		))
		.send(io)
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
	assert!(
		response
			.headers()
			.get(::http::header::RETRY_AFTER)
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.parse::<u64>().ok())
			.is_some_and(|seconds| seconds > 0)
	);
	assert_eq!(mock.received_requests().await.unwrap().len(), 0);
}

#[tokio::test]
async fn llm_detect_mode_passthrough_without_rewrite() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		name: "default".into(),
		provider: AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		host_override: Some(Target::Address(*mock.address())),
		path_override: None,
		path_prefix: None,
		tokenize: false,
		policies: serde_json::from_value(json!({
			"ai": {
				"routes": {
					"/v1/chat/completions": "detect"
				}
			}
		}))
		.unwrap(),
	};
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	let body = llm_body!("requests/completions/basic.json");

	let res = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions?trace=repro")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(body.to_vec()))
		.send(io.clone())
		.await
		.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterQuery],
		"/v1/chat/completions?trace=repro"
	);
	let upstream_body: Value =
		serde_json::from_slice(&request.body).expect("upstream request should be JSON");
	let original_body: Value = serde_json::from_slice(body).expect("original request should be JSON");
	assert_eq!(upstream_body, original_body);

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", "/v1/chat/completions?trace=repro"),
	])
	.await
	.unwrap();
	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23
	});
	assert!(is_json_subset(&want, &log), "want={want:#?} got={log:#?}");
}

#[tokio::test]
async fn llm_detect_mode_respects_model_rewrite() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		name: "default".into(),
		provider: AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		host_override: Some(Target::Address(*mock.address())),
		path_override: None,
		path_prefix: None,
		tokenize: false,
		policies: serde_json::from_value(json!({
			"ai": {
				"routes": {
					"/v1/chat/completions": "detect"
				},
				"overrides": {
					"model": "replaceme-overwrite"
				}
			}
		}))
		.unwrap(),
	};
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	let body = llm_body!("requests/completions/basic.json");

	let res = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions?trace=rewrite")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(body.to_vec()))
		.send(io.clone())
		.await
		.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterQuery],
		"/v1/chat/completions?trace=rewrite"
	);
	let upstream_body: Value =
		serde_json::from_slice(&request.body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["model"], "replaceme-overwrite");

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", "/v1/chat/completions?trace=rewrite"),
	])
	.await
	.unwrap();
	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme-overwrite",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23
	});
	assert!(is_json_subset(&want, &log), "want={want:#?} got={log:#?}");
}

async fn setup_local_llm_config(yaml: &str) -> TestBind {
	let t = setup_proxy_test("{}").unwrap();
	let resources = agentgateway::resource_manager::ResourceFetcher::direct(t.pi.upstream.clone());
	let normalized = agentgateway::types::local::NormalizedLocalConfig::from(
		t.pi.cfg.as_ref(),
		&resources,
		t.pi.cfg.gateway(),
		yaml,
	)
	.await
	.expect("local config normalizes");
	t.pi
		.stores
		.binds
		.sync_local(
			normalized.binds,
			normalized.listener_routes,
			normalized.listener_tcp_routes,
			normalized.policies,
			normalized.backends,
			normalized.route_groups,
			Default::default(),
		)
		.expect("sync local binds");
	t
}

#[tokio::test]
async fn llm_api_key_allowed_models_filters_discovery_and_requests() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let config = format!(
		r#"
llm:
  port: 0
  discovery: disabled
  policies:
    apiKey:
      keys:
      - key: sk-restricted
        metadata:
          name: restricted
        allowedModels:
        - openai/gpt-*
        - direct-model
      mode: strict
  models:
  - name: openai/*
    provider: openAI
    params:
      baseUrl: http://{}/v1
  - name: direct-model
    provider: openAI
    params:
      baseUrl: http://{}/v1
"#,
		mock.address(),
		mock.address(),
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));
	let authorization = [("authorization", "Bearer sk-restricted")];

	let mut models = list_models(io.clone(), &authorization).await;
	models.sort();
	assert_eq!(models, vec!["direct-model", "openai/*"]);

	let response = send_completions_with_model(io.clone(), "openai/gpt-4o", &authorization).await;
	assert_eq!(response.status(), StatusCode::OK);
	let _ = read_body_raw(response.into_body()).await;

	let response = send_completions_with_model(io, "openai/claude", &authorization).await;
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["error"]["code"], "model_not_allowed");
	assert_eq!(mock.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn llm_attached_rejection_is_stored_in_log_database() {
	let _guard = agentgateway::telemetry::log_store::setup(&agentgateway::database::DatabaseConfig {
		url: "sqlite::memory:".to_string(),
		max_connections: Some(1),
	})
	.await
	.unwrap();
	let t = setup_local_llm_config(
		r#"
gateways:
  default:
    port: 0
llm:
  gateways: default
  policies:
    apiKey:
      keys:
      - key: sk-test
      mode: strict
  models:
  - name: gpt-4o
    provider: openAI
"#,
	)
	.await;
	// No API key: rejected before the LLM handler runs, on a listener not named "llm".
	let response =
		send_completions_with_model(t.serve_http(strng::literal!("bind/0")), "gpt-4o", &[]).await;
	assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
	let _ = read_body_raw(response.into_body()).await;

	let request: agentgateway::telemetry::log_store::SearchRequest =
		serde_json::from_value(json!({"filters": {"httpStatus": [401]}})).unwrap();
	tokio::time::timeout(Duration::from_secs(10), async {
		while agentgateway::telemetry::log_store::search(request.clone())
			.await
			.unwrap()
			.logs
			.is_empty()
		{
			tokio::time::sleep(Duration::from_millis(50)).await;
		}
	})
	.await
	.expect("rejected request stored in log database");
}

#[tokio::test]
async fn llm_catalog_discovery() {
	let mut results = serde_json::Map::new();
	for (name, setting) in [
		("default", ""),
		("catalog", "discovery: catalog"),
		("disabled", "discovery: disabled"),
	] {
		let config = format!(
			r#"
llm:
  port: 0
  {setting}
  policies:
    apiKey:
      mode: strict
      keys:
      - key: unrestricted
        metadata:
          name: unrestricted
      - key: restricted
        metadata:
          name: restricted
        allowedModels: ["public/discovery-a", "discovery-*", "alias"]
  providers:
  - name: openai
    provider: openAI
    defaults:
      transformation:
        model: llmRequest.model.stripPrefix("public/")
  models:
  - name: public/discovery-*
    provider:
      reference: openai
  - name: public/discovery-a
    provider:
      reference: openai
  - name: discovery-*
    provider: openAI
  - name: hidden/*
    visibility: internal
    provider: openAI
  - name: unsafe/*
    provider: openAI
    transformation:
      model: request.headers["x-model"]
  - name: unknown/*
    provider:
      custom:
        formats:
        - type: completions
    params:
      baseUrl: http://localhost:9999/v1
  virtualModels:
  - name: alias
    routing:
      weighted:
        targets:
        - model: discovery-a
"#
		);
		let mut t = setup_local_llm_config(&config).await;
		Arc::get_mut(&mut t.pi).unwrap().model_catalog = agentgateway::llm::catalog::ModelCatalog::new(vec![
			agentgateway::ModelCatalogSource::Inline {
				inline: r#"{"providers":{"openai":{"models":{"discovery-a":{},"discovery-b":{}}},"anthropic":{"models":{"discovery-other":{}}}}}"#.to_string(),
			},
		]).await.unwrap();
		let io = t.serve_http(strng::literal!("bind/0"));
		for key in ["unrestricted", "restricted"] {
			let authorization = format!("Bearer {key}");
			let models = list_models(io.clone(), &[("authorization", &authorization)]).await;
			results.insert(format!("{name}/{key}"), serde_json::json!(models));
		}
	}
	insta::assert_json_snapshot!(results);
}

#[tokio::test]
async fn llm_model_list_metadata() {
	let config = r#"
llm:
  port: 0
  models:
  - name: gpt-5.5
    provider: openAI
  - name: fast
    provider: openAI
    params:
      model: gpt-5-mini
  - name: openai/*
    provider: openAI
    transformation:
      model: llmRequest.model.stripPrefix("openai/")
  - name: local-model
    provider: openAI
  virtualModels:
  - name: blend
    routing:
      weighted:
        targets:
        - model: gpt-5.5
          weight: 1
        - model: fast
          weight: 1
  - name: resilient
    routing:
      failover:
        targets:
        - model: gpt-5.5
          priority: 0
        - model: fast
          priority: 1
"#;
	let mut t = setup_local_llm_config(config).await;
	Arc::get_mut(&mut t.pi).unwrap().model_catalog = agentgateway::llm::catalog::ModelCatalog::new(vec![
		agentgateway::ModelCatalogSource::Inline {
			inline: r#"{"metadata":{"generatedAt":"2099-01-01T00:00:00Z"},"providers":{"openai":{"models":{
				"gpt-5.5":{"name":"GPT-5.5","released":"2026-04-23","limits":{"context":1050000,"input":922000,"output":128000},"modalities":{"input":["text","image","pdf"],"output":["text"]},"capabilities":{"reasoning":true,"structuredOutput":true,"toolCall":true,"temperature":false},"reasoning":{"effort":["low","high"]}},
				"gpt-5-mini":{"name":"GPT-5 mini","released":"2025-08","limits":{"context":400000,"output":64000},"modalities":{"input":["text","image"],"output":["text"]},"capabilities":{"reasoning":true,"toolCall":true,"temperature":true},"reasoning":{"effort":["low","medium"]}}
			}}}}"#.to_string(),
		},
	]).await.unwrap();
	let io = t.serve_http(strng::literal!("bind/0"));
	let res = send_request(io.clone(), Method::GET, "http://lo/v1/models").await;
	assert_eq!(res.status(), StatusCode::OK);
	let mut models: Value = serde_json::from_slice(&read_body_raw(res.into_body()).await).unwrap();

	let res = send_request(io.clone(), Method::GET, "http://lo/v1/models?limit=1").await;
	let limited: Value = serde_json::from_slice(&read_body_raw(res.into_body()).await).unwrap();
	assert_eq!(limited["data"].as_array().unwrap().len(), 1);

	let res = send_request(
		io.clone(),
		Method::GET,
		"http://lo/v1/models/openai/gpt-5.5",
	)
	.await;
	assert_eq!(res.status(), StatusCode::OK);
	let model: Value = serde_json::from_slice(&read_body_raw(res.into_body()).await).unwrap();
	assert_eq!(model["id"], "openai/gpt-5.5");
	assert_eq!(model["max_tokens"], 128000);

	let res = send_request(io, Method::GET, "http://lo/v1/models/missing").await;
	assert_eq!(res.status(), StatusCode::NOT_FOUND);
	// Entries without catalog metadata carry the config load time.
	for model in models["data"].as_array_mut().unwrap() {
		if ["local-model", "blend", "resilient"].contains(&model["id"].as_str().unwrap()) {
			model["created"] = json!(0);
		}
	}
	let listed = models["data"].as_array().unwrap();
	let described = |id: &str| {
		let mut model = listed.iter().find(|m| m["id"] == id).unwrap().clone();
		model.as_object_mut().unwrap().remove("id");
		model
	};
	assert_eq!(described("resilient")["max_tokens"], 64000);
	assert_eq!(described("resilient"), described("blend"));
	insta::assert_json_snapshot!(models);
}

#[tokio::test]
async fn llm_model_list_preferred_endpoints() {
	let config = r#"
llm:
  port: 0
  models:
  - name: gpt-5.5
    provider: openAI
  - name: text-embedding-3-small
    provider: openAI
  - name: gpt-image-2
    provider: openAI
  - name: claude-sonnet-4-5
    provider: anthropic
  - name: gemini-2.5-flash
    provider: gemini
  - name: gemini-2.5-flash-image
    provider: gemini
  - name: vertex/claude-sonnet-4-5
    provider: vertex
    params:
      model: claude-sonnet-4-5
      vertexProject: p
  - name: vertex/gemini-embedding-001
    provider: vertex
    params:
      model: gemini-embedding-001
      vertexProject: p
  - name: bedrock/claude-sonnet-4-5
    provider: bedrock
    params:
      model: anthropic.claude-sonnet-4-5
      awsRegion: us-east-1
  - name: bedrock/llama3
    provider: bedrock
    params:
      model: meta.llama3-70b-instruct-v1:0
      awsRegion: us-east-1
  - name: bedrock/titan-embed
    provider: bedrock
    params:
      model: amazon.titan-embed-text-v2:0
      awsRegion: us-east-1
  - name: azure/gpt-5.5
    provider: azure
    params:
      model: gpt-5.5
      azureResourceName: r
      azureResourceType: openAI
  - name: foundry/claude-sonnet-4-5
    provider: azure
    params:
      model: claude-sonnet-4-5
      azureResourceName: r
      azureResourceType: foundry
      azureProjectName: p
  - name: FLUX.1-schnell
    provider: togetherai
  - name: custom-embedder
    provider:
      custom:
        formats:
        - type: embeddings
    params:
      baseUrl: http://localhost:9999/v1
  - name: custom-everything
    provider:
      custom:
        formats:
        - type: completions
        - type: generateContent
        - type: anthropicTokenCount
        - type: geminiCountTokens
        - type: rerank
        - type: decisions
        - type: systemOne
        - type: realtime
    params:
      baseUrl: http://localhost:9999/v1
  virtualModels:
  - name: openai-and-azure
    routing:
      weighted:
        targets:
        - model: gpt-5.5
          weight: 1
        - model: azure/gpt-5.5
          weight: 1
"#;
	let mut t = setup_local_llm_config(config).await;
	Arc::get_mut(&mut t.pi).unwrap().model_catalog = agentgateway::llm::catalog::ModelCatalog::new(vec![
		agentgateway::ModelCatalogSource::Inline {
			inline: r#"{"metadata":{"generatedAt":"2099-01-01T00:00:00Z"},"providers":{
				"openai":{"models":{"gpt-5.5":{},"text-embedding-3-small":{"endpoints":["/embeddings"]},"gpt-image-2":{"endpoints":["/images/generations","/images/edits"]}}},
				"gcp.gemini":{"models":{"gemini-2.5-flash":{},"gemini-2.5-flash-image":{"endpoints":["/images/generations","/images/edits"]}}},
				"gcp.vertex_ai":{"models":{"gemini-embedding-001":{"endpoints":["/embeddings"]}}},
				"aws.bedrock":{"models":{"anthropic.claude-sonnet-4-5":{"modalities":{"input":["text"],"output":["text"]}},"meta.llama3-70b-instruct-v1:0":{"modalities":{"input":["text"],"output":["text"]}},"amazon.titan-embed-text-v2:0":{}}},
				"togetherai":{"models":{"FLUX.1-schnell":{"endpoints":["/images/generations","/images/edits"]}}}
			}}"#.to_string(),
		},
	]).await.unwrap();
	let io = t.serve_http(strng::literal!("bind/0"));
	let res = send_request(io, Method::GET, "http://lo/v1/models").await;
	assert_eq!(res.status(), StatusCode::OK);
	let models: Value = serde_json::from_slice(&read_body_raw(res.into_body()).await).unwrap();
	let endpoints = models["data"]
		.as_array()
		.unwrap()
		.iter()
		.map(|m| {
			(
				m["id"].as_str().unwrap().to_string(),
				m["preferred_endpoints"].clone(),
			)
		})
		.collect::<std::collections::BTreeMap<_, _>>();
	insta::assert_json_snapshot!(endpoints);
}

#[tokio::test]
async fn llm_local_router_handles_models_virtual_model_and_missing_model() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: real-model
    visibility: internal
    provider: openAI
    authorization:
      rules:
      - 'request.headers["x-model-auth"] == "yes"'
    params:
      baseUrl: http://{}/v1
    health:
      eviction: {{}}
      unhealthyExpression: 'response.code == 403'
  - name: prefix/*
    visibility: internal
    provider: openai
    params:
      baseUrl: http://{}/v1
    transformation:
      model: llmRequest.model.stripPrefix("prefix/")
  - name: direct-model
    provider: openAI
    authorization:
      rules:
      - 'request.headers["x-model-auth"] == "yes"'
    params:
      baseUrl: http://{}/v1
  virtualModels:
  - name: virtual-model
    routing:
      failover:
        targets:
        - model: real-model
          priority: 0
  - name: failover
    routing:
      failover:
        targets:
        - model: real-model
          priority: 0
        - model: prefix/without-prefix
          priority: 1
"#,
		mock.address(),
		mock.address(),
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));

	// check model list respects authorization
	{
		let model_ids = list_models(io.clone(), &[]).await;
		assert_eq!(model_ids, vec!["virtual-model", "failover"]);
		let model_ids = list_models(io.clone(), &[("x-model-auth", "yes")]).await;
		assert_eq!(model_ids, vec!["direct-model", "virtual-model", "failover"]);
	}

	// Virtual model
	{
		let res = send_completions_with_model(io.clone(), "virtual-model", &[]).await;
		assert_eq!(res.status(), StatusCode::FORBIDDEN);
		assert_eq!(
			mock
				.received_requests()
				.await
				.expect("upstream requests")
				.len(),
			0
		);

		let res =
			send_completions_with_model(io.clone(), "virtual-model", &[("x-model-auth", "yes")]).await;
		assert_eq!(res.status(), StatusCode::OK);

		let request = single_upstream_request(&mock).await;
		let upstream_body: Value =
			serde_json::from_slice(&request.body).expect("upstream request JSON");
		assert_eq!(upstream_body["model"], "real-model");
	}

	// Direct model
	{
		let res = send_completions_with_model(io.clone(), "direct-model", &[]).await;
		assert_eq!(res.status(), StatusCode::FORBIDDEN);
		assert_eq!(
			mock
				.received_requests()
				.await
				.expect("upstream requests")
				.len(),
			1
		);

		let res =
			send_completions_with_model(io.clone(), "direct-model", &[("x-model-auth", "yes")]).await;
		assert_eq!(res.status(), StatusCode::OK);
		let upstream_requests = mock.received_requests().await.expect("upstream requests");
		assert_eq!(upstream_requests.len(), 2);
		let upstream_body: Value =
			serde_json::from_slice(&upstream_requests[1].body).expect("upstream request JSON");
		assert_eq!(upstream_body["model"], "direct-model");
	}

	// Failover model
	{
		// First attempt: fails
		let res = send_completions_with_model(io.clone(), "failover", &[]).await;
		assert_eq!(res.status(), StatusCode::FORBIDDEN);
		assert_eq!(
			mock
				.received_requests()
				.await
				.expect("upstream requests")
				.len(),
			2
		);

		// Second attempt: failover to model without authz
		let res = send_completions_with_model(io.clone(), "failover", &[]).await;
		assert_eq!(res.status(), StatusCode::OK);
		let upstream_requests = mock.received_requests().await.expect("upstream requests");
		assert_eq!(upstream_requests.len(), 3);
		let upstream_body: Value =
			serde_json::from_slice(&upstream_requests[2].body).expect("upstream request JSON");
		// Model should be explicitly rewritten and have the prefix removed
		assert_eq!(upstream_body["model"], "without-prefix");
	}

	// Missing model
	{
		let res = send_completions_with_model(io, "missing-model", &[]).await;
		assert_eq!(res.status(), StatusCode::NOT_FOUND);
		let missing_body: Value =
			serde_json::from_slice(&read_body_raw(res.into_body()).await).expect("missing model JSON");
		assert_eq!(missing_body["error"]["code"], "model_not_found");
		assert_eq!(
			mock
				.received_requests()
				.await
				.expect("upstream requests")
				.len(),
			3
		);
	}
}

#[tokio::test]
async fn llm_policies_see_request_model() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let set = r#"
            x-request-model: llm.requestModel
            x-messages: string(llmRequest.messages.size())"#;
	// The AI backend reads them from a route policy; the model router from a provider policy,
	// which is only known once the model is resolved.
	let config = format!(
		r#"
gateways:
  default:
    port: 0
routes:
- hostnames: [ai.example]
  policies:
    transformations:
      request:
        set:{set}
  backends:
  - ai:
      name: ai
      hostOverride: {addr}
      provider:
        openAI:
          model: upstream-model
llm:
  models:
  - name: router-model
    provider: openAI
    params:
      model: upstream-model
      baseUrl: http://{addr}/v1
    requestTransformation:
      set:{set}
"#,
		addr = mock.address(),
	);
	let t = setup_local_llm_config(&config).await;

	for (host, model, encoding) in [
		("ai.example", "ai-model", None),
		("ai.example", "identity-model", Some("identity")),
		("lo", "router-model", None),
	] {
		let mut req = RequestBuilder::new(Method::POST, &format!("http://{host}/v1/chat/completions"));
		if let Some(encoding) = encoding {
			req = req.header(header::CONTENT_ENCODING, encoding);
		}
		let res = req
			.body(Body::from(completions_request_body_with_model(model)))
			.send(t.serve_http(strng::literal!("bind/0")))
			.await
			.expect("completions request");
		assert_eq!(res.status(), StatusCode::OK, "{model}");
	}
	let got: Vec<_> = mock
		.received_requests()
		.await
		.expect("request recording should be enabled")
		.iter()
		.map(|req| {
			let body: Value = serde_json::from_slice(&req.body).unwrap();
			(
				req.headers["x-request-model"].to_str().unwrap().to_string(),
				req.headers["x-messages"].to_str().unwrap().to_string(),
				body["model"].as_str().unwrap().to_string(),
			)
		})
		.collect();
	let want = |model: &str| {
		(
			model.to_string(),
			"2".to_string(),
			"upstream-model".to_string(),
		)
	};
	assert_eq!(
		got,
		vec![
			want("ai-model"),
			want("identity-model"),
			want("router-model")
		]
	);
}

#[tokio::test]
async fn llm_conditional_virtual_model_no_match_returns_json_error() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: real-model
    visibility: internal
    provider: openAI
    params:
      baseUrl: http://{}/v1
  virtualModels:
  - name: public-model
    routing:
      conditional:
        targets:
        - model: real-model
          when: request.headers["x-use-model"] == "true"
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));
	let res = send_completions_with_model(io, "public-model", &[]).await;

	assert_eq!(res.status(), StatusCode::BAD_REQUEST);
	let body: Value =
		serde_json::from_slice(&read_body_raw(res.into_body()).await).expect("error JSON");
	assert_eq!(body["error"]["code"], "virtual_model_no_matching_target");
	assert_eq!(body["error"]["type"], "invalid_request_error");
	assert_eq!(
		mock
			.received_requests()
			.await
			.expect("upstream requests")
			.len(),
		0
	);
}

#[tokio::test]
async fn llm_nested_virtual_models() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: fast
    visibility: internal
    provider: openAI
    params:
      baseUrl: http://{0}/v1
    transformation:
      max_tokens: "256"
  - name: other
    visibility: internal
    provider: openAI
    params:
      baseUrl: http://{0}/v1
  virtualModels:
  - name: entry
    routing:
      conditional:
        targets:
        - model: pool
          when: request.headers["x-route"] == "pool"
        - model: other
  - name: pool
    routing:
      weighted:
        targets:
        - model: fast
          weight: 1
  # Shadows the concrete model of the same name.
  - name: fast
    routing:
      weighted:
        targets:
        - model: fast
          weight: 1
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));

	for (model, headers) in [
		("entry", &[("x-route", "pool")][..]),
		("entry", &[][..]),
		("fast", &[][..]),
	] {
		let res = send_completions_with_model(io.clone(), model, headers).await;
		assert_eq!(res.status(), StatusCode::OK);
	}

	let got = mock
		.received_requests()
		.await
		.expect("upstream requests")
		.iter()
		.map(|request| {
			let body: Value = serde_json::from_slice(&request.body).expect("upstream request JSON");
			(body["model"].clone(), body["max_tokens"].clone())
		})
		.collect::<Vec<_>>();
	assert_eq!(
		got,
		vec![
			(json!("fast"), json!(256)),
			(json!("other"), Value::Null),
			(json!("fast"), json!(256)),
		]
	);
}

#[tokio::test]
async fn llm_model_router_handles_multipart_audio_detect_request() {
	let mock = body_mock(br#"{"text":"Hello world","usage":{"type":"duration","seconds":9}}"#).await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: openai/*
    provider: openAI
    transformation:
      model: llmRequest.model.stripPrefix("openai/")
    params:
      baseUrl: http://{}/v1
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));
	let body = multipart_audio_body("openai/audio-model");

	let res = send_multipart_audio(io.clone(), body.clone()).await;
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/v1/audio/transcriptions"
	);
	assert_multipart_audio_body(&request.body, "audio-model").await;

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", "/v1/audio/transcriptions"),
		("gen_ai.request.model", "audio-model"),
	])
	.await
	.unwrap();
	let want = json!({
		"gen_ai.provider.name": "openai",
		"gen_ai.operation.name": "transcription",
		"gen_ai.request.model": "audio-model",
		"gen_ai.usage.audio_seconds": 9.0
	});
	assert!(is_json_subset(&want, &log), "want={want:#?} got={log:#?}");
}

#[tokio::test]
async fn llm_detect_reads_usage_from_unrequested_event_stream() {
	// The client did not set `stream`, so only the response content type marks this as a stream.
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(ResponseTemplate::new(StatusCode::OK.as_u16()).set_body_raw(
			concat!(
				"data: {\"type\":\"image_generation.partial_image\",\"b64_json\":\"AAAA\"}\n\n",
				"data: {\"type\":\"image_generation.completed\",\"b64_json\":\"AAAA\",\"usage\":{",
				"\"input_tokens\":14,\"output_tokens\":45,\"total_tokens\":59,",
				"\"input_tokens_details\":{\"text_tokens\":14,\"image_tokens\":0}}}\n\n",
			),
			"text/event-stream",
		))
		.mount(&mock)
		.await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: stream-model
    provider: openAI
    params:
      baseUrl: http://{}/v1
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));

	let res = RequestBuilder::new(Method::POST, "http://lo/v1/images/generations")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(r#"{"model":"stream-model","prompt":"a cat"}"#))
		.send(io)
		.await
		.expect("image generation request");
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", "/v1/images/generations"),
		("gen_ai.request.model", "stream-model"),
	])
	.await
	.unwrap();
	let want = json!({
		"gen_ai.operation.name": "image_generation",
		"gen_ai.usage.input_tokens": 14,
		"gen_ai.usage.output_tokens": 45
	});
	assert!(is_json_subset(&want, &log), "want={want:#?} got={log:#?}");
}

#[tokio::test]
async fn llm_model_router_prices_mistral_ocr_pages() {
	// Mistral Document AI returns no token usage at all, /v1/ocr must resolve
	// to the detect route so usage extraction and catalog pricing run.
	let ocr_response = br#"{
		"pages": [
			{"index": 0, "markdown": "Title", "images": [], "dimensions": {"dpi": 200}},
			{"index": 1, "markdown": "body", "images": [], "dimensions": {"dpi": 200}}
		],
		"model": "mistral-ocr-latest",
		"usage_info": {"pages_processed": 4, "doc_size_bytes": 145349}
	}"#;
	let mock = body_mock(ocr_response).await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: mistral-ocr-latest
    provider: openAI
    params:
      baseUrl: http://{}/v1
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	t.pi
		.model_catalog
		.replace_sources(vec![agentgateway::ModelCatalogSource::Inline {
			inline: r#"{"providers":{"openai":{"models":{"mistral-ocr-latest":{"rates":{"perPage":"0.005"}}}}}}"#
				.to_string(),
		}])
		.await
		.expect("inline catalog loads");
	let io = t.serve_http(strng::literal!("bind/0"));

	let res = RequestBuilder::new(Method::POST, "http://lo/v1/ocr")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{"model":"mistral-ocr-latest","document":{"type":"document_url","document_url":"https://example.com/doc.pdf"}}"#
				.to_vec(),
		))
		.send(io.clone())
		.await
		.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/v1/ocr"
	);

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", "/v1/ocr"),
	])
	.await
	.unwrap();
	// 4 pages at $0.005/page
	let want = json!({
		"gen_ai.provider.name": "openai",
		"gen_ai.response.model": "mistral-ocr-latest",
		"agw.ai.usage.cost.total": "0.020"
	});
	assert!(is_json_subset(&want, &log), "want={want:#?} got={log:#?}");
}

#[tokio::test]
async fn llm_model_router_rewrites_path_model() {
	let mock = body_mock(llm_body!("response/bedrock/basic.json")).await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: internal-model
    visibility: internal
    provider: bedrock
    params:
      awsRegion: us-east-1
      hostOverride: "{}"
      pathPrefix: /
    transformation:
      model: '"global.openai.gpt-5.6-luna"'
  virtualModels:
  - name: gpt-5.6-luna
    routing:
      failover:
        targets:
        - model: internal-model
          priority: 0
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));
	let body = json!({"messages": [{"role": "user", "content": [{"text": "hi"}]}]});
	let res = RequestBuilder::new(
		Method::POST,
		"http://lo/model/gpt-5.6-luna/converse?trace=true",
	)
	.json(&body)
	.send(io)
	.await
	.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		request.url.path(),
		"/model/global.openai.gpt-5.6-luna/converse"
	);
	assert_eq!(request.url.query(), Some("trace=true"));
	assert_eq!(request.body_json::<Value>().unwrap(), body);
}

#[tokio::test]
async fn llm_model_router_rewrites_multipart_virtual_model() {
	let mock = body_mock(include_bytes!(
		"../../../llm/src/tests/response/completions/basic.json"
	))
	.await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: real-model
    visibility: internal
    provider: openAI
    params:
      baseUrl: http://{}/v1
    passthrough: detect
  virtualModels:
  - name: public-model
    routing:
      weighted:
        targets:
        - model: real-model
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));

	let res = send_multipart_audio(io, multipart_audio_body("public-model")).await;
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_multipart_audio_body(&requests[0].body, "real-model").await;
}

#[tokio::test]
async fn llm_model_router_applies_provider_model_to_opaque_multipart() {
	let mock = body_mock(include_bytes!(
		"../../../llm/src/tests/response/completions/basic.json"
	))
	.await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: public-model
    provider: openAI
    params:
      baseUrl: http://{}/v1
      model: upstream-model
    passthrough: opaque
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));

	let res = send_multipart_audio(io, multipart_audio_body("public-model")).await;
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_multipart_audio_body(&requests[0].body, "upstream-model").await;
}

#[tokio::test]
async fn llm_model_router_prefers_provider_model_after_multipart_virtual_routing() {
	let mock = body_mock(include_bytes!(
		"../../../llm/src/tests/response/completions/basic.json"
	))
	.await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: routed-model
    visibility: internal
    provider: openAI
    params:
      baseUrl: http://{}/v1
      model: upstream-model
    passthrough: detect
  virtualModels:
  - name: public-model
    routing:
      weighted:
        targets:
        - model: routed-model
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));

	let res = send_multipart_audio(io, multipart_audio_body("public-model")).await;
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_multipart_audio_body(&requests[0].body, "upstream-model").await;
}

#[tokio::test]
async fn llm_model_router_rewrites_failover_virtual_multipart_model() {
	let mock = body_mock(include_bytes!(
		"../../../llm/src/tests/response/completions/basic.json"
	))
	.await;
	let config = format!(
		r#"
llm:
  port: 0
  models:
  - name: real-model
    visibility: internal
    provider: openAI
    params:
      baseUrl: http://{}/v1
    passthrough: opaque
  virtualModels:
  - name: public-model
    routing:
      failover:
        targets:
        - model: real-model
          priority: 0
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/0"));

	let res = send_multipart_audio(io, multipart_audio_body("public-model")).await;
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_multipart_audio_body(&requests[0].body, "real-model").await;
}

#[tokio::test]
async fn llm_custom_rerank() {
	let mock = body_mock(llm_body!("response/cohere/rerank.json")).await;
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		name: "default".into(),
		provider: AIProvider::Custom(custom::Provider {
			model_override: None,
			provider_override: None,
			formats: vec![custom::ProviderFormatConfig {
				format: custom::ProviderFormat::Rerank,
				path: None,
			}],
		}),
		host_override: Some(Target::Address(*mock.address())),
		path_override: None,
		path_prefix: None,
		tokenize: false,
		policies: serde_json::from_value(json!({
			"ai": {"routes": {"/v1/rerank": "rerank"}}
		}))
		.unwrap(),
	};
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/rerank",
		llm_body!("requests/rerank/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let body: Value = serde_json::from_slice(&read_body_raw(res.into_body()).await).unwrap();
	assert_eq!(body["results"][0]["index"], 2);
	assert_eq!(body["results"][0]["relevance_score"], 0.91);

	let request = single_upstream_request(&mock).await;
	let upstream_body: Value =
		serde_json::from_slice(&request.body).expect("upstream request should be JSON");
	assert_eq!(
		upstream_body["query"],
		"What is the capital of the United States?"
	);
	assert_eq!(upstream_body["documents"].as_array().unwrap().len(), 3);
}

fn setup_custom_llm_provider_backend_mock(
	mock: MockServer,
	supported_formats: Vec<custom::ProviderFormat>,
) -> (MockServer, TestBind, MemoryClient) {
	setup_custom_llm_provider_backend_mock_with_formats(
		mock,
		supported_formats
			.into_iter()
			.map(|format| custom::ProviderFormatConfig { format, path: None })
			.collect(),
	)
}

fn setup_custom_llm_provider_backend_mock_with_formats(
	mock: MockServer,
	formats: Vec<custom::ProviderFormatConfig>,
) -> (MockServer, TestBind, MemoryClient) {
	let backend_name = "custom-ai";
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_bind(simple_bind())
		.with_raw_backend(custom_llm_backend_with_formats(
			backend_name,
			SimpleBackendReference::InlineBackend(Target::Address(*mock.address())),
			formats,
		));
	let mut route = basic_named_route(strng::format!("/{backend_name}"));
	route
		.inline_policies
		.push(TrafficPolicy::AI(Arc::new(agentgateway::llm::Policy {
			routes: [
				(
					strng::new("/v1/messages"),
					agentgateway::llm::RouteType::Messages,
				),
				(
					strng::new("/v1/chat/completions"),
					agentgateway::llm::RouteType::Completions,
				),
			]
			.into_iter()
			.collect(),
			..Default::default()
		})));
	let t = t.with_route(route);
	let io = t.serve_http(BIND_KEY);
	(mock, t, io)
}

#[tokio::test]
async fn llm_custom_provider_routes_to_provider_backend() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Completions]);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 200);
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/v1/chat/completions"
	);
	let upstream_body: Value =
		serde_json::from_slice(&request.body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["model"], "replaceme");
}

#[tokio::test]
async fn llm_custom_provider_uses_upstream_route_fallback() {
	let mock = body_mock(llm_body!("response/anthropic/basic.json")).await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Messages]);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 200);
	let response_body: Value =
		serde_json::from_slice(&read_body_raw(res.into_body()).await).expect("response is JSON");
	assert_eq!(response_body["object"], "chat.completion");
	assert_eq!(response_body["usage"]["prompt_tokens"], 40);
	assert_eq!(response_body["usage"]["completion_tokens"], 21);

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/v1/messages"
	);
	let upstream_body: Value =
		serde_json::from_slice(&request.body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["system"], "You are a helpful assistant.");
	assert_eq!(upstream_body["messages"][0]["role"], "user");
}

#[tokio::test]
async fn llm_custom_provider_messages_to_responses_for_responses_only_backend() {
	let mock = body_mock(llm_body!("response/responses/basic.json")).await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Responses]);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/messages",
		llm_body!("requests/messages/basic.json"),
	)
	.await;
	let status = res.status();
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(
		status,
		200,
		"unexpected response body: {}",
		String::from_utf8_lossy(&body)
	);
	let response_body: Value = serde_json::from_slice(&body).expect("response is JSON");
	assert_eq!(response_body["type"], "message");
	assert_eq!(response_body["content"][0]["type"], "text");

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/v1/responses"
	);
	let upstream_body: Value =
		serde_json::from_slice(&request.body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["model"], "claude-sonnet-4-20250514");
	assert_eq!(upstream_body["input"][0]["type"], "message");
	assert_eq!(upstream_body["input"][0]["role"], "user");
	assert_eq!(
		upstream_body["input"][0]["content"][0]["type"],
		"input_text"
	);
	assert_eq!(
		upstream_body["input"][0]["content"][0]["text"],
		"Hello, world"
	);
}

#[tokio::test]
async fn llm_custom_provider_messages_to_responses_accepts_cache_control() {
	let mock = body_mock(llm_body!("response/responses/basic.json")).await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Responses]);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/messages",
		llm_body!("requests/messages/cache_control_responses.json"),
	)
	.await;
	let status = res.status();
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(
		status,
		200,
		"unexpected response body: {}",
		String::from_utf8_lossy(&body)
	);
	let response_body: Value = serde_json::from_slice(&body).expect("response is JSON");
	assert_eq!(response_body["type"], "message");

	let request = single_upstream_request(&mock).await;
	let upstream_body: Value =
		serde_json::from_slice(&request.body).expect("upstream request should be JSON");
	assert!(
		upstream_body["input"]
			.as_array()
			.expect("Responses input should be an array")
			.iter()
			.filter_map(|item| item.get("content"))
			.flat_map(|content| content.as_array().into_iter().flatten())
			.any(|part| part.get("prompt_cache_breakpoint").is_some())
	);
}

#[tokio::test]
async fn llm_custom_provider_uses_format_path_override() {
	let mock = body_mock(llm_body!("response/anthropic/basic.json")).await;
	let (mock, _bind, io) = setup_custom_llm_provider_backend_mock_with_formats(
		mock,
		vec![custom::ProviderFormatConfig {
			format: custom::ProviderFormat::Messages,
			path: Some(strng::literal!("/api/messages")),
		}],
	);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 200);
	let _ = read_body_raw(res.into_body()).await;

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/api/messages"
	);
}

#[tokio::test]
async fn llm_custom_provider_rejects_unsupported_format_before_upstream_call() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Embeddings]);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 400);
	let body = read_body_raw(res.into_body()).await;
	assert!(
		String::from_utf8_lossy(&body)
			.contains("unsupported conversion: from Completions to provider custom"),
		"unexpected response body: {}",
		String::from_utf8_lossy(&body)
	);

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 0);
}

#[tokio::test]
async fn llm_rejects_unsupported_request_encoding_as_client_error() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Completions]);

	let res = send_completions_with_model(
		io,
		"replaceme",
		&[(header::CONTENT_ENCODING.as_str(), "snappy")],
	)
	.await;
	assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 0);
}

#[tokio::test]
async fn llm_maps_unsupported_upstream_response_encoding_to_bad_gateway() {
	let response_body = llm_body!("response/completions/basic.json");
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(
			ResponseTemplate::new(StatusCode::OK.as_u16())
				.insert_header(header::CONTENT_ENCODING.as_str(), "snappy")
				.set_body_raw(response_body, "application/json"),
		)
		.mount(&mock)
		.await;
	let (_mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Completions]);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
}

async fn recv_rate_limit_request(
	requests: &mut mpsc::UnboundedReceiver<
		agentgateway::http::remoteratelimit::proto::RateLimitRequest,
	>,
) -> agentgateway::http::remoteratelimit::proto::RateLimitRequest {
	tokio::time::timeout(Duration::from_secs(1), requests.recv())
		.await
		.expect("timed out waiting for rate limit request")
		.expect("rate limit request sender should be open")
}

fn completions_request_body(streaming: bool) -> Vec<u8> {
	let mut body: Value = serde_json::from_slice(llm_body!("requests/completions/basic.json"))
		.expect("request fixture should be valid JSON");
	if streaming {
		body["stream"] = json!(true);
	}
	serde_json::to_vec(&body).expect("request fixture should serialize")
}

fn completions_request_body_with_model(model: &str) -> Vec<u8> {
	let mut body: Value = serde_json::from_slice(llm_body!("requests/completions/basic.json"))
		.expect("request fixture should be valid JSON");
	body["model"] = json!(model);
	serde_json::to_vec(&body).expect("request fixture should serialize")
}

fn multipart_audio_body(model: &str) -> Vec<u8> {
	format!(
		concat!(
			"--audio-boundary\r\n",
			"Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n",
			"Content-Type: audio/wav\r\n",
			"\r\n",
			"fake-audio-public-model-bytes\r\n",
			"--audio-boundary\r\n",
			"Content-Disposition: form-data; name=\"model\"\r\n",
			"\r\n",
			"{}\r\n",
			"--audio-boundary--\r\n",
		),
		model,
	)
	.into_bytes()
}

async fn assert_multipart_audio_body(body: &[u8], expected_model: &str) {
	let stream = futures_util::stream::once(std::future::ready(Ok::<bytes::Bytes, multer::Error>(
		bytes::Bytes::copy_from_slice(body),
	)));
	let mut multipart = multer::Multipart::new(stream, "audio-boundary");
	let mut saw_file = false;
	let mut saw_model = false;
	while let Some(field) = multipart
		.next_field()
		.await
		.expect("upstream multipart body should parse")
	{
		match field.name() {
			Some("file") => {
				assert_eq!(field.file_name(), Some("audio.wav"));
				assert_eq!(field.content_type().map(|v| v.as_ref()), Some("audio/wav"));
				assert_eq!(
					field
						.bytes()
						.await
						.expect("file field should read")
						.as_ref(),
					b"fake-audio-public-model-bytes"
				);
				saw_file = true;
			},
			Some("model") => {
				assert_eq!(
					field.text().await.expect("model field should read"),
					expected_model
				);
				saw_model = true;
			},
			name => panic!("unexpected multipart field {name:?}"),
		}
	}
	assert!(saw_file, "multipart body should include the file field");
	assert!(saw_model, "multipart body should include the model field");
}

async fn send_multipart_audio(io: MemoryClient, body: Vec<u8>) -> Response {
	RequestBuilder::new(Method::POST, "http://lo/v1/audio/transcriptions")
		.header(
			header::CONTENT_TYPE,
			"multipart/form-data; boundary=audio-boundary",
		)
		.body(Body::from(body))
		.send(io)
		.await
		.expect("multipart audio request")
}

async fn send_completions_with_model(
	io: MemoryClient,
	model: &str,
	headers: &[(&str, &str)],
) -> Response {
	let request_body = completions_request_body_with_model(model);
	let mut request = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions");
	for (key, value) in headers {
		request = request.header(*key, *value);
	}
	request
		.body(Body::from(request_body))
		.send(io)
		.await
		.expect("completions request")
}

async fn list_models(io: MemoryClient, headers: &[(&str, &str)]) -> Vec<String> {
	let res = if headers.is_empty() {
		send_request(io, Method::GET, "http://lo/v1/models").await
	} else {
		send_request_headers(io, Method::GET, "http://lo/v1/models", headers).await
	};
	assert_eq!(res.status(), StatusCode::OK);
	let models: Value =
		serde_json::from_slice(&read_body_raw(res.into_body()).await).expect("models JSON");
	assert_eq!(models["object"], "list");
	models["data"]
		.as_array()
		.expect("model list")
		.iter()
		.map(|model| model["id"].as_str().expect("model id").to_string())
		.collect()
}

async fn single_upstream_request(mock: &MockServer) -> wiremock::Request {
	let mut requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	requests.pop().unwrap()
}

#[rstest::rstest]
#[case::retry_after(false)]
#[case::denial_after_allowed_check(true)]
#[tokio::test]
async fn llm_remote_ratelimit_response(#[case] check_requests: bool) {
	use agentgateway::http::remoteratelimit::proto;
	use proto::rate_limit_response::rate_limit::Unit;
	use proto::rate_limit_response::{Code, DescriptorStatus, RateLimit};

	struct RateLimitHeaders;

	#[async_trait::async_trait]
	impl ratelimitmock::Handler for RateLimitHeaders {
		async fn should_rate_limit(
			&mut self,
			request: &proto::RateLimitRequest,
		) -> Result<proto::RateLimitResponse, tonic::Status> {
			let statuses: Vec<_> = request
				.descriptors
				.iter()
				.map(|descriptor| {
					let (code, limit, remaining, reset, unit) = match descriptor.entries[0].key.as_str() {
						"requests" => (Code::Ok, 60, 56, 12, Unit::Minute),
						"tokens" => (Code::OverLimit, 100, 0, 38, Unit::Minute),
						"spend" => (Code::OverLimit, 1000, 0, 2712, Unit::Hour),
						key => panic!("unexpected descriptor: {key}"),
					};
					DescriptorStatus {
						code: code as i32,
						current_limit: Some(RateLimit {
							name: descriptor.entries[0].key.clone(),
							requests_per_unit: limit,
							unit: unit as i32,
						}),
						limit_remaining: remaining,
						duration_until_reset: Some(prost_types::Duration {
							seconds: reset,
							nanos: 0,
						}),
						..Default::default()
					}
				})
				.collect();
			Ok(proto::RateLimitResponse {
				overall_code: if statuses.iter().any(|s| s.code == Code::OverLimit as i32) {
					Code::OverLimit
				} else {
					Code::Ok
				} as i32,
				statuses,
				..Default::default()
			})
		}
	}

	let rate_limit = ratelimitmock::RateLimitMock::new(|| RateLimitHeaders)
		.spawn()
		.await;
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (mock, mut bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
		"{}",
	);
	let mut descriptors = vec![
		json!({"entries": [{"key": "tokens", "value": "\"model\""}], "type": "tokens"}),
		json!({"entries": [{"key": "spend", "value": "\"user\""}], "type": "tokens"}),
	];
	if check_requests {
		descriptors.insert(
			0,
			json!({
				"entries": [{"key": "requests", "value": "\"user\""}], "type": "requests"
			}),
		);
	}
	bind
		.attach_route_policy(json!({
			"remoteRateLimit": {
				"domain": "llm",
				"host": rate_limit.address.to_string(),
				"descriptors": descriptors,
			}
		}))
		.await;

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo",
		&completions_request_body(false),
	)
	.await;
	assert!(mock.received_requests().await.unwrap().is_empty());
	// Envoy's advisory selection keeps the first descriptor on a remaining-count tie.
	// Retry-After must instead wait for every denied window, and an earlier allowed
	// request check must not overwrite either set of denial headers.
	assert_eq!(
		json!({
			"status": res.status().as_u16(),
			"limit": res.hdr("x-ratelimit-limit"),
			"remaining": res.hdr("x-ratelimit-remaining"),
			"reset": res.hdr("x-ratelimit-reset"),
			"retry-after": res.headers().get(header::RETRY_AFTER).map(|v| v.to_str().unwrap()),
		}),
		json!({
			"status": 429,
			"limit": "100",
			"remaining": "0",
			"reset": "38",
			"retry-after": "2712",
		})
	);
}

async fn assert_llm_remote_rate_limit_cost(
	response_body: &[u8],
	request_body: &[u8],
	expected_cost: u64,
) {
	let (rate_limit_tx, mut rate_limit_rx) = mpsc::unbounded_channel();
	let rate_limit = ratelimitmock::RateLimitMock::new({
		let rate_limit_tx = rate_limit_tx.clone();
		move || RecordingRateLimit {
			requests: rate_limit_tx.clone(),
		}
	})
	.spawn()
	.await;

	let mock = body_mock(response_body).await;
	let (_mock, mut bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
		"{}",
	);
	bind
		.attach_route_policy(json!({
			"remoteRateLimit": {
				"domain": "llm",
				"host": rate_limit.address.to_string(),
				"descriptors": [{
					"entries": [{
						"key": "model",
						"value": "\"model\"",
					}],
					"type": "tokens",
					"cost": "llm.outputTokens * uint(1000) + llm.inputTokens",
				}],
			},
		}))
		.await;

	let res = send_request_body(io, Method::POST, "http://lo", request_body).await;
	assert_eq!(res.status(), 200);
	let _ = read_body_raw(res.into_body()).await;

	let initial_request = recv_rate_limit_request(&mut rate_limit_rx).await;
	let amend_request = recv_rate_limit_request(&mut rate_limit_rx).await;
	assert_eq!(initial_request.domain, "llm");
	assert_eq!(amend_request.domain, "llm");

	let initial = initial_request.descriptors.first().unwrap();
	assert_eq!(initial.entries[0].key, "model");
	assert_eq!(initial.entries[0].value, "model");
	assert_eq!(initial.hits_addend, Some(0));

	let amend = amend_request.descriptors.first().unwrap();
	assert_eq!(amend.entries[0].key, "model");
	assert_eq!(amend.entries[0].value, "model");
	assert_eq!(amend.hits_addend, Some(expected_cost));
}

#[tokio::test]
async fn llm_remote_rate_limit_cost_amends_response_tokens() {
	assert_llm_remote_rate_limit_cost(
		llm_body!("response/completions/basic.json"),
		&completions_request_body(false),
		23017,
	)
	.await;
}

#[tokio::test]
async fn llm_streaming_remote_rate_limit_cost_amends_response_tokens() {
	assert_llm_remote_rate_limit_cost(
		llm_body!("response/completions/stream.json"),
		&completions_request_body(true),
		286018,
	)
	.await;
}

#[rstest::rstest]
#[case::preserves_path(None, None, "/v1/messages?trace=repro")]
#[case::path_override(Some("/custom/chat/completions"), None, "/custom/chat/completions")]
#[case::path_prefix(None, Some("/v1/custom/"), "/v1/custom/responses")]
#[tokio::test]
async fn llm_openai_messages_translation_with_host_override_path_behavior(
	#[case] path_override: Option<&str>,
	#[case] path_prefix: Option<&str>,
	#[case] expected_url: &str,
) {
	let mock = body_mock(llm_body!("response/responses/basic.json")).await;
	let provider = agentgateway::test_helpers::proxymock::llm_named_provider(
		&mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
	);
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		path_override: path_override.map(strng::new),
		path_prefix: path_prefix.map(strng::new),
		..provider
	};
	let (mock, mut bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {
				"routes": {
					"/v1/chat/completions": "completions",
					"/v1/messages": "messages"
				}
			}
		}))
		.await;

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/messages?trace=repro",
		llm_body!("requests/messages/basic.json"),
	)
	.await;

	assert_eq!(res.status(), 200);
	let upstream = single_upstream_request(&mock).await;
	assert_eq!(
		&upstream.url[Position::BeforePath..Position::AfterQuery],
		expected_url
	);
}

#[tokio::test]
async fn llm_final_transformation_applies_after_messages_translation() {
	let mock = body_mock(llm_body!("response/responses/basic.json")).await;
	let (mock, mut bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
		"{}",
	);
	bind
		.attach_route_policy(json!({
			"ai": {
				"routes": { "/v1/messages": "messages" },
				"finalTransformations": {
					// Drop the converted tools.
					"tools": r#"fail("remove")"#,
					// Observe the converted input list.
					"converted_message_count": "llmRequest.input.size()"
				}
			}
		}))
		.await;

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/messages",
		br#"{
			"model": "gpt-4o",
			"max_tokens": 64,
			"system": "be brief",
			"messages": [{"role": "user", "content": "hello"}],
			"tools": [{
				"name": "get_weather",
				"description": "Look up the weather",
				"input_schema": {
					"type": "object",
					"properties": {"city": {"type": "string"}},
					"required": ["city"]
				}
			}]
		}"#,
	)
	.await;

	assert_eq!(res.status(), 200);
	let request = single_upstream_request(&mock).await;
	let upstream_body: Value = serde_json::from_slice(&request.body).expect("upstream request JSON");

	// The request really was converted to Responses format.
	assert_eq!(upstream_body["instructions"], json!("be brief"));
	// Indexing yields Null for a missing key, so assert on key presence.
	assert!(
		upstream_body.get("tools").is_none(),
		"tools should be removed, got: {upstream_body}"
	);
	assert_eq!(upstream_body["converted_message_count"], json!(1));
}

#[rstest::rstest]
#[case::preserves_path(None, "/v1/models", "/v1/models")]
#[case::path_prefix(Some("/openai/v1"), "/v1/models", "/openai/v1/models")]
#[case::path_prefix_with_query(
	Some("/openai/v1"),
	"/v1/models?foo=bar",
	"/openai/v1/models?foo=bar"
)]
#[case::path_prefix_non_default_path(Some("/openai/v1"), "/foo", "/openai/v1/foo")]
#[tokio::test]
async fn llm_openai_passthrough_applies_path_prefix(
	#[case] path_prefix: Option<&str>,
	#[case] request_path: &str,
	#[case] expected_url: &str,
) {
	let mock = body_mock(b"{}").await;
	let provider = agentgateway::test_helpers::proxymock::llm_named_provider(
		&mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		false,
	);
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		path_prefix: path_prefix.map(strng::new),
		..provider
	};
	let (mock, mut bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {
				"routes": {
					"*": "passthrough"
				}
			}
		}))
		.await;

	let res = send_request(io, Method::GET, &format!("http://lo{request_path}")).await;

	assert_eq!(res.status(), 200);
	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterQuery],
		expected_url
	);
}

// Providers without a DEFAULT_BASE_PATH (e.g. Gemini) prepend pathPrefix to the
// full incoming path rather than replacing /v1.
#[rstest::rstest]
#[case::preserves_path(None, "/some/path", "/some/path")]
#[case::path_prefix(Some("/my/prefix"), "/some/path", "/my/prefix/some/path")]
#[tokio::test]
async fn llm_non_openai_passthrough_prepends_path_prefix(
	#[case] path_prefix: Option<&str>,
	#[case] request_path: &str,
	#[case] expected_url: &str,
) {
	let mock = body_mock(b"{}").await;
	let provider = agentgateway::test_helpers::proxymock::llm_named_provider(
		&mock,
		AIProvider::Gemini(gemini::Provider {
			model_override: None,
		}),
		false,
	);
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		path_prefix: path_prefix.map(strng::new),
		..provider
	};
	let (mock, mut bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {
				"routes": {
					"/some/path": "passthrough"
				}
			}
		}))
		.await;

	let res = send_request(io, Method::GET, &format!("http://lo{request_path}")).await;

	assert_eq!(res.status(), 200);
	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterQuery],
		expected_url
	);
}

#[tokio::test]
async fn llm_log_body() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let x = serde_json::to_string(&json!({
		"config": {
			"logging": {
				"fields": {
					"add": {
						"prompt": "llm.prompt",
						"completion": "llm.completion"
					}
				}
			}
		}
	}))
	.unwrap();
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		true,
		x.as_str(),
	);

	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23,
		"completion": ["Sorry, I couldn't find the name of the LLM provider. Could you please provide more information or context?"],
		"prompt": [
			{"role":"system","content":"You are a helpful assistant."},
			{"role":"user","content":"What is the name of the LLM provider?"},
		]
	});
	assert_llm(io, llm_body!("requests/completions/basic.json"), want).await;
}

async fn assert_llm(io: MemoryClient, body: &[u8], want: Value) {
	let r = rand::rng().random::<u128>();
	let res = send_request_body(io.clone(), Method::POST, &format!("http://lo/{r}"), body).await;

	// Ensure body finishes
	let _ = read_body_raw(res.into_body()).await;
	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", &format!("/{r}")),
	])
	.await
	.unwrap();
	let valid = is_json_subset(&want, &log);
	assert!(valid, "want={want:#?} got={log:#?}");
}

#[derive(Clone)]
struct RecordingRateLimit {
	requests: mpsc::UnboundedSender<agentgateway::http::remoteratelimit::proto::RateLimitRequest>,
}

#[async_trait::async_trait]
impl ratelimitmock::Handler for RecordingRateLimit {
	async fn should_rate_limit(
		&mut self,
		request: &agentgateway::http::remoteratelimit::proto::RateLimitRequest,
	) -> Result<agentgateway::http::remoteratelimit::proto::RateLimitResponse, tonic::Status> {
		self
			.requests
			.send(request.clone())
			.expect("rate limit request receiver should be open");
		ratelimitmock::ok_response()
	}
}

fn llm_ir_on(config: Value) -> agentgateway::Config {
	let mut config = agentgateway::config::parse_config(config.to_string(), None).unwrap();
	config.llm_ir_enabled = true;
	config
}

#[rstest::rstest]
#[case::embeddings(
	custom::ProviderFormat::Embeddings, "embeddings",
	llm_body!("requests/embeddings/basic.json"), llm_body!("response/openai/embeddings.json")
)]
#[case::rerank(
	custom::ProviderFormat::Rerank, "rerank",
	llm_body!("requests/rerank/basic.json"), llm_body!("response/cohere/rerank.json")
)]
#[tokio::test]
async fn llm_ir_non_chat_endpoints_keep_native_handlers(
	#[case] format: custom::ProviderFormat,
	#[case] route: &str,
	#[case] body: &[u8],
	#[case] reply: &[u8],
) {
	let mock = body_mock(reply).await;
	let mut provider = llm_named_provider(
		&mock,
		AIProvider::Custom(custom::Provider {
			model_override: None,
			provider_override: None,
			formats: vec![custom::ProviderFormatConfig { format, path: None }],
		}),
		false,
	);
	provider.policies = Some(
		serde_json::from_value(json!({
			"ai": {"routes": {format!("/v1/{route}"): route}}
		}))
		.unwrap(),
	);
	let (mock, _bind, io) =
		setup_llm_named_provider_mock_with_config(mock, provider, llm_ir_on(json!({})));
	let path = format!("/{}/v1/{route}", rand::rng().random::<u128>());
	let response = send_request_body(io, Method::POST, &format!("http://lo{path}"), body).await;
	assert_eq!(response.status(), 200);
	assert_eq!(
		serde_json::from_slice::<Value>(&read_body_raw(response.into_body()).await).unwrap(),
		serde_json::from_slice::<Value>(reply).unwrap(),
	);
	let upstream = single_upstream_request(&mock).await;
	assert_eq!(
		serde_json::from_slice::<Value>(&upstream.body).unwrap(),
		serde_json::from_slice::<Value>(body).unwrap()
	);
	let log = request_log(&path).await;
	assert_eq!(log["agw.ai.converter"], "native");
	assert!(log.get("agw.ai.legacy_reason").is_none());
}

#[tokio::test]
async fn llm_ir_rejects_a_chat_model_without_a_supported_format() {
	let mock = body_mock(llm_body!("response/responses/basic.json")).await;
	let mut provider = llm_named_provider(
		&mock,
		AIProvider::Custom(custom::Provider {
			model_override: Some("backend-model".into()),
			provider_override: Some("embeddings-only".into()),
			formats: vec![custom::ProviderFormatConfig {
				format: custom::ProviderFormat::Embeddings,
				path: None,
			}],
		}),
		false,
	);
	provider.policies = Some(
		serde_json::from_value(json!({
			"ai": {"routes": {"/v1/messages": "messages"}}
		}))
		.unwrap(),
	);
	let (mock, _bind, io) =
		setup_llm_named_provider_mock_with_config(mock, provider, llm_ir_on(json!({})));
	let path = format!("/{}/v1/messages", rand::rng().random::<u128>());
	let body = json!({"model": "client-model", "max_tokens": 64, "messages": [{"role": "user", "content": "hi"}]}).to_string();
	let response = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.as_bytes(),
	)
	.await;
	assert_eq!(response.status(), 400);
	let response = String::from_utf8(read_body_raw(response.into_body()).await.to_vec()).unwrap();
	for detail in ["embeddings-only", "backend-model", "Messages", "Embeddings"] {
		assert!(response.contains(detail), "{response}");
	}
	assert!(mock.received_requests().await.unwrap().is_empty());
	let log = request_log(&path).await;
	assert_eq!(log["agw.ai.converter"], "llm_ir");
	assert!(log.get("agw.ai.legacy_reason").is_none());
}

fn openai_messages_provider(
	mock: &MockServer,
	tokenize: bool,
) -> agentgateway::types::local::LocalNamedAIProvider {
	let mut provider = llm_named_provider(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model_override: None,
			moderation: None,
		}),
		tokenize,
	);
	provider.policies =
		Some(serde_json::from_value(json!({"ai": {"routes": {"/v1/messages": "messages"}}})).unwrap());
	provider
}

// The thinking signature is not one the IR sealed, so the IR refuses the request while legacy
// forwards it. The status shows which converter ran, and the log records it and why.
#[rstest::rstest]
#[case::flag_off(None, false, Some("gpt-5"), 200, None)]
#[case::selected(Some(json!({})), false, Some("gpt-5"), 400, Some(("llm_ir", None)))]
#[case::prompt_captured(
	Some(json!({"config": {"logging": {"fields": {"add": {"prompt": "llm.prompt"}}}}})),
	false,
	Some("gpt-5"),
	400,
	Some(("llm_ir", None))
)]
#[case::tokenize(Some(json!({})), true, Some("gpt-5"), 200, Some(("legacy", Some("tokenize"))))]
// Nothing names a model, so the IR refuses the request before choosing where it goes.
#[case::missing_model(Some(json!({})), false, None, 400, Some(("llm_ir", None)))]
#[tokio::test]
async fn llm_ir_serves_only_selected_messages_requests(
	#[case] llm_ir: Option<Value>,
	#[case] tokenize: bool,
	#[case] model: Option<&str>,
	#[case] status: u16,
	#[case] converter: Option<(&str, Option<&str>)>,
) {
	let mock = body_mock(llm_body!("response/responses/basic.json")).await;
	let provider = openai_messages_provider(&mock, tokenize);
	let config = match llm_ir {
		Some(config) => llm_ir_on(config),
		None => agentgateway::config::parse_config("{}".to_string(), None).unwrap(),
	};
	let (mock, _bind, io) = setup_llm_named_provider_mock_with_config(mock, provider, config);
	let mut body = json!({
		"max_tokens": 64,
		"messages": [
			{"role": "user", "content": "hi"},
			{"role": "assistant", "content": [
				{"type": "thinking", "thinking": "", "signature": "legacy-signature"},
				{"type": "text", "text": "hello"}
			]},
			{"role": "user", "content": "again"}
		]
	});
	if let Some(model) = model {
		body["model"] = model.into();
	}

	let path = format!("/{}/v1/messages", rand::rng().random::<u128>());
	let res = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.to_string().as_bytes(),
	)
	.await;
	assert_eq!(res.status(), status);
	let _ = read_body_raw(res.into_body()).await;
	let upstream = mock.received_requests().await.unwrap();
	assert_eq!(upstream.len(), usize::from(status == 200));
	let log = request_log(&path).await;
	assert_eq!(
		(
			log.get("agw.ai.converter").and_then(Value::as_str),
			log.get("agw.ai.legacy_reason").and_then(Value::as_str),
		),
		match converter {
			Some((name, reason)) => (Some(name), reason),
			None => (None, None),
		}
	);
}

#[tokio::test]
async fn llm_ir_converts_the_request_and_its_reply() {
	struct UnusedIdentifierHasher;

	impl agent_llm_ir::IdentifierPseudonymizer for UnusedIdentifierHasher {
		fn pseudonymize(&self, _: &str) -> String {
			unreachable!("the request has no long safety identifier")
		}
	}

	let mock = body_mock(llm_body!("response/responses/basic.json")).await;
	let provider = openai_messages_provider(&mock, false);
	let (mock, _bind, io) =
		setup_llm_named_provider_mock_with_config(mock, provider, llm_ir_on(json!({})));
	let body = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"system": [{"type": "text", "text": "Be brief."}, {"type": "text", "text": "Use lists."}],
		"messages": [{"role": "user", "content": "hi"}]
	});

	let path = format!("/{}/v1/messages", rand::rng().random::<u128>());
	let res = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.to_string().as_bytes(),
	)
	.await;
	assert_eq!(res.status(), 200);
	let reply: Value = serde_json::from_slice(&read_body_raw(res.into_body()).await).unwrap();
	assert_eq!(reply["type"], "message");
	assert_eq!(
		reply["content"][0]["text"],
		"Hello! How can I assist you today?"
	);
	// Messages names OpenAI's `default` tier `standard`; legacy passes the OpenAI word through.
	assert_eq!(reply["usage"]["service_tier"], "standard");
	assert_llm_log(
		&path,
		json!({
			"gen_ai.request.model": "gpt-5",
			"gen_ai.response.model": "gpt-4o-2024-08-06",
			"gen_ai.usage.input_tokens": 10,
			"gen_ai.usage.output_tokens": 10,
		}),
	)
	.await;

	let request = single_upstream_request(&mock).await;
	let want = agent_llm_ir::prepare(
		body.to_string().as_bytes(),
		agent_llm_ir::ClientProtocol::Messages,
		agent_llm_ir::Upstream {
			identifier_pseudonymizer: &UnusedIdentifierHasher,
			scope: agent_llm_ir::StateScope {
				backend: "any",
				provider: "openai",
				model: "gpt-5",
			},
			protocol: agent_llm_ir::UpstreamProtocol::Responses {
				safety_identifier: true,
				reasoning_context: false,
			},
			default_max_tokens: None,
		},
	)
	.and_then(|prepared| prepared.render(agent_llm_ir::RenderSettings::ModelDefaults))
	.unwrap();
	assert_eq!(std::str::from_utf8(&request.body).unwrap(), want.body);
}

async fn assert_llm_log(path: &str, want: Value) {
	let log = request_log(path).await;
	assert!(is_json_subset(&want, &log), "want {want}, got {log}");
}

async fn request_log(path: &str) -> Value {
	agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", path)])
		.await
		.unwrap()
}

// A compressed provider stream is decompressed and sent on as it is converted, as legacy does.
#[rstest::rstest]
#[case::plain(None)]
#[case::gzip(Some("gzip"))]
#[case::deflate(Some("deflate"))]
#[case::brotli(Some("br"))]
#[case::zstd(Some("zstd"))]
#[tokio::test]
async fn llm_ir_streams_its_reply(#[case] encoding: Option<&str>) {
	const MESSAGE: &str = r#"{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}"#;
	let events = [
		r#"{"type":"response.created","sequence_number":0,"response":{"id":"resp_1","object":"response","model":"gpt-5","status":"in_progress","output":[]}}"#.to_string(),
		r#"{"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","status":"in_progress","content":[]}}"#.to_string(),
		r#"{"type":"response.content_part.added","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#.to_string(),
		r#"{"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello there"}"#.to_string(),
		r#"{"type":"response.output_text.done","sequence_number":4,"item_id":"msg_1","output_index":0,"content_index":0,"text":"Hello there"}"#.to_string(),
		r#"{"type":"response.content_part.done","sequence_number":5,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"Hello there","annotations":[]}}"#.to_string(),
		format!(r#"{{"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":{MESSAGE}}}"#),
		format!(
			r#"{{"type":"response.completed","sequence_number":7,"response":{{"id":"resp_1","object":"response","model":"gpt-5","status":"completed","output":[{MESSAGE}],"usage":{{"input_tokens":12,"input_tokens_details":{{"cached_tokens":4}},"output_tokens":5,"output_tokens_details":{{"reasoning_tokens":2}},"total_tokens":17}}}}}}"#
		),
	];
	let stream: String = events
		.iter()
		.map(|data| format!("data: {data}\n\n"))
		.collect();
	let reply = match encoding {
		None => {
			wiremock::ResponseTemplate::new(200).set_body_raw(stream.into_bytes(), "text/event-stream")
		},
		Some(encoding) => wiremock::ResponseTemplate::new(200)
			.set_body_raw(
				agentgateway::http::compression::encode_body(stream.as_bytes(), encoding)
					.await
					.unwrap(),
				"text/event-stream",
			)
			.insert_header("content-encoding", encoding),
	};
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(reply)
		.mount(&mock)
		.await;
	let provider = openai_messages_provider(&mock, false);
	let (_mock, _bind, io) =
		setup_llm_named_provider_mock_with_config(mock, provider, llm_ir_on(json!({})));
	let body = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"stream": true,
		"messages": [{"role": "user", "content": "hi"}]
	});

	let path = format!("/{}/v1/messages", rand::rng().random::<u128>());
	let res = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.to_string().as_bytes(),
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");
	assert_eq!(res.headers().get(header::CONTENT_ENCODING), None);
	let reply = read_body_raw(res.into_body()).await;
	let events: Vec<&str> = std::str::from_utf8(&reply)
		.unwrap()
		.lines()
		.filter_map(|line| line.strip_prefix("event: "))
		.collect();
	assert_eq!(
		events,
		[
			"message_start",
			"content_block_start",
			"content_block_delta",
			"content_block_stop",
			"message_delta",
			"message_stop"
		]
	);
	assert_llm_log(
		&path,
		json!({
			"gen_ai.response.model": "gpt-5",
			"gen_ai.usage.input_tokens": 12,
			"gen_ai.usage.output_tokens": 5,
			"gen_ai.usage.reasoning_tokens": 2,
		}),
	)
	.await;
}

#[tokio::test]
async fn llm_ir_answers_a_stream_that_fails_before_output_with_an_http_error() {
	let stream = [
		r#"{"type":"response.created","sequence_number":0,"response":{"id":"resp_1","object":"response","model":"gpt-5","status":"in_progress","output":[]}}"#,
		r#"{"type":"response.failed","sequence_number":1,"response":{"id":"resp_1","object":"response","model":"gpt-5","status":"failed","output":[],"error":{"code":"rate_limit_exceeded","message":"slow down"}}}"#,
	]
	.iter()
	.map(|data| format!("data: {data}\n\n"))
	.collect::<String>();
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(
			wiremock::ResponseTemplate::new(200).set_body_raw(stream.into_bytes(), "text/event-stream"),
		)
		.mount(&mock)
		.await;
	let provider = openai_messages_provider(&mock, false);
	let (_mock, _bind, io) =
		setup_llm_named_provider_mock_with_config(mock, provider, llm_ir_on(json!({})));
	let body = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"stream": true,
		"messages": [{"role": "user", "content": "hi"}]
	});

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/messages",
		body.to_string().as_bytes(),
	)
	.await;
	assert_eq!(res.status(), 429);
	assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
	let reply: Value = serde_json::from_slice(&read_body_raw(res.into_body()).await).unwrap();
	assert_eq!(reply["type"], "error");
	assert_eq!(reply["error"]["type"], "rate_limit_error");
}

/// How the IR's answer to a provider reply differs from legacy's, on purpose.
#[derive(Debug, Clone, Copy)]
enum Difference {
	/// The same message, status, headers and usage.
	None,
	/// The IR keeps reasoning as thinking blocks sealed for replay. Legacy drops it.
	Reasoning,
	/// The IR uses the Messages stop reason for a full context window. Legacy says `max_tokens`.
	ContextWindow,
	/// The IR drops a tool call cut short. Legacy sends its partial arguments as a string.
	TruncatedToolCall,
	/// The IR fails, before any output, where legacy invents an answer or a 200 stream.
	IrFails(u16),
	/// The IR ends the stream with an error event where legacy drops what it can't read.
	IrFailsAfterOutput,
	/// Both fail. The IR answers in the Messages error shape, legacy in plain text.
	BothFail,
}

// Every Responses reply fixture, served through the gateway with the IR off and on.
#[rstest::rstest]
#[case::basic("basic.json", Difference::None)]
#[case::content_filter("content_filter.json", Difference::None)]
#[case::context_window("context_window.json", Difference::ContextWindow)]
#[case::empty_tool_call("empty_tool_call.json", Difference::None)]
#[case::max_tokens("max_tokens.json", Difference::None)]
#[case::reasoning("reasoning.json", Difference::Reasoning)]
#[case::tool("tool.json", Difference::None)]
#[case::truncated_tool_call("truncated_tool_call.json", Difference::TruncatedToolCall)]
// OpenAI sends no reason; legacy says `max_tokens` here and `end_turn` when streamed.
#[case::incomplete_no_details("incomplete_no_details.json", Difference::IrFails(500))]
// Only Responses clients can ask for custom or hosted tools, so a Messages client never gets these.
#[case::custom_tool("custom-tool.json", Difference::BothFail)]
#[case::guardrails("guardrails.json", Difference::BothFail)]
#[case::failed("failed.json", Difference::BothFail)]
#[case::stream("stream.json", Difference::None)]
#[case::stream_content_filter("stream-content_filter.json", Difference::None)]
#[case::stream_context_window("stream-context_window.json", Difference::ContextWindow)]
#[case::stream_max_tokens("stream-max_tokens.json", Difference::None)]
#[case::stream_refusal("stream-refusal.json", Difference::None)]
#[case::stream_refusal_final_only("stream-refusal-final-only.json", Difference::None)]
#[case::stream_incomplete_no_details("stream-incomplete_no_details.json", Difference::IrFails(500))]
#[case::stream_error("stream-error.json", Difference::IrFails(429))]
// Text arrives before its item is announced, which the IR can't place.
#[case::stream_failed("stream-failed.json", Difference::IrFails(500))]
#[case::stream_custom_tool("stream-custom-tool.json", Difference::IrFails(500))]
#[case::stream_image("stream-image.json", Difference::IrFailsAfterOutput)]
#[tokio::test]
async fn llm_ir_differs_from_legacy_only_on_purpose(
	#[case] fixture: &str,
	#[case] difference: Difference,
) {
	let legacy = serve_responses_reply(fixture, false).await;
	let ir = serve_responses_reply(fixture, true).await;
	let error = ir.log.get("error").and_then(Value::as_str);
	match difference {
		Difference::None
		| Difference::Reasoning
		| Difference::ContextWindow
		| Difference::TruncatedToolCall => {
			assert_eq!(ir.status, 200);
			assert_eq!(legacy.status, 200);
			assert_eq!(ir.headers, legacy.headers);
			let mut message = ir.message.clone();
			match difference {
				Difference::Reasoning => {
					let content = message["content"].as_array_mut().unwrap();
					let thinking = content.iter().filter(|b| b["type"] == "thinking").count();
					assert!(thinking > 0, "{message}");
					content.retain(|b| b["type"] != "thinking");
				},
				Difference::ContextWindow => {
					assert_eq!(message["stop_reason"], "model_context_window_exceeded");
					assert_eq!(legacy.message["stop_reason"], "max_tokens");
					message["stop_reason"] = "max_tokens".into();
				},
				Difference::TruncatedToolCall => {
					assert_eq!(message["content"], json!([]));
					assert!(legacy.message["content"][0]["input"].is_string());
					message["content"] = legacy.message["content"].clone();
				},
				_ => {},
			}
			assert_eq!(message, legacy.message);
			assert_eq!(error, None);
		},
		Difference::IrFails(status) => {
			assert_eq!(legacy.status, 200);
			assert_eq!(ir.status, status);
			assert_eq!(ir.message["type"], "error");
			assert!(error.unwrap().starts_with("llm-ir reply failed: "));
		},
		Difference::IrFailsAfterOutput => {
			assert_eq!(ir.status, 200);
			assert_eq!(ir.headers, legacy.headers);
			assert_eq!(ir.message["error"]["type"], "api_error");
			assert_eq!(legacy.message["error"], Value::Null);
		},
		Difference::BothFail => {
			assert_eq!(legacy.status, 502);
			assert_eq!(ir.status, 500);
			assert_eq!(ir.message["type"], "error");
			assert!(error.unwrap().starts_with("llm-ir reply failed: "));
		},
	}
	// The IR also accounts what it read before failing, where legacy fails to parse at all.
	if !matches!(difference, Difference::BothFail) {
		assert_eq!(usage(&ir.log), usage(&legacy.log));
	}
	assert_eq!(ir.log["agw.ai.converter"], "llm_ir");
}

struct Served {
	status: u16,
	/// Every header but the date and length.
	headers: Vec<(String, String)>,
	/// What a Messages client assembles from the reply or its stream.
	message: Value,
	log: Value,
}

async fn serve_responses_reply(fixture: &str, llm_ir: bool) -> Served {
	let reply = std::fs::read(
		std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../llm/src/tests/response/responses")
			.join(fixture),
	)
	.unwrap();
	let stream = fixture.starts_with("stream");
	let content_type = if stream {
		"text/event-stream"
	} else {
		"application/json"
	};
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(reply, content_type))
		.mount(&mock)
		.await;
	let provider = openai_messages_provider(&mock, false);
	let config = if llm_ir {
		llm_ir_on(json!({}))
	} else {
		agentgateway::config::parse_config("{}".to_string(), None).unwrap()
	};
	let (_mock, _bind, io) = setup_llm_named_provider_mock_with_config(mock, provider, config);
	let body = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"stream": stream,
		"messages": [{"role": "user", "content": "hi"}]
	});
	let path = format!("/{}/v1/messages", rand::rng().random::<u128>());
	let res = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.to_string().as_bytes(),
	)
	.await;
	let status = res.status().as_u16();
	let headers = res
		.headers()
		.iter()
		.filter(|(name, _)| *name != header::DATE && *name != header::CONTENT_LENGTH)
		.map(|(name, value)| (name.to_string(), value.to_str().unwrap().to_string()))
		.collect();
	let is_stream = res
		.headers()
		.get(header::CONTENT_TYPE)
		.map(|v| v.as_bytes())
		== Some(b"text/event-stream".as_slice());
	let body = read_body_raw(res.into_body()).await;
	let message = match (status, is_stream) {
		// Legacy's own failures are plain text.
		(502, _) => Value::Null,
		(_, true) => assembled_message(&body),
		(_, false) => serde_json::from_slice(&body).unwrap(),
	};
	Served {
		status,
		headers,
		message: normalized(message),
		log: request_log(&path).await,
	}
}

/// Folds a Messages stream into the message it describes.
fn assembled_message(stream: &[u8]) -> Value {
	fn append(field: &mut Value, more: &Value) {
		*field = format!("{}{}", field.as_str().unwrap_or(""), more.as_str().unwrap()).into();
	}
	let mut message = Value::Null;
	for data in std::str::from_utf8(stream)
		.unwrap()
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
	{
		let event: Value = serde_json::from_str(data).unwrap();
		let index = event["index"].as_u64().unwrap_or_default() as usize;
		match event["type"].as_str().unwrap() {
			"message_start" => message = event["message"].clone(),
			"content_block_start" => message["content"]
				.as_array_mut()
				.unwrap()
				.push(event["content_block"].clone()),
			"content_block_delta" => {
				let block = &mut message["content"][index];
				let delta = &event["delta"];
				match delta["type"].as_str().unwrap() {
					"text_delta" => append(&mut block["text"], &delta["text"]),
					"thinking_delta" => append(&mut block["thinking"], &delta["thinking"]),
					"signature_delta" => append(&mut block["signature"], &delta["signature"]),
					"input_json_delta" => append(&mut block["partial_json"], &delta["partial_json"]),
					other => panic!("unexpected delta {other}"),
				}
			},
			"content_block_stop" => {
				let block = message["content"][index].as_object_mut().unwrap();
				if let Some(json) = block.remove("partial_json") {
					block.insert(
						"input".into(),
						serde_json::from_str(json.as_str().unwrap()).unwrap(),
					);
				}
			},
			"message_delta" => {
				for (key, value) in event["delta"].as_object().unwrap() {
					message[key] = value.clone();
				}
				for (key, value) in event["usage"].as_object().unwrap() {
					message["usage"][key] = value.clone();
				}
			},
			"error" => message["error"] = event["error"].clone(),
			"message_stop" | "ping" => {},
			other => panic!("unexpected event {other}"),
		}
	}
	message
}

/// Removes the differences that change no meaning.
fn normalized(mut message: Value) -> Value {
	let usage = &mut message["usage"];
	if let Some(usage) = usage.as_object_mut() {
		// The IR passes on a zero cache count that legacy leaves out.
		if usage.get("cache_read_input_tokens") == Some(&json!(0)) {
			usage.remove("cache_read_input_tokens");
		}
		// Legacy passes on OpenAI's name for the tier. Messages calls it `standard`.
		if usage.get("service_tier") == Some(&json!("default")) {
			usage.insert("service_tier".into(), "standard".into());
		}
	}
	message
}

/// The usage and model accounted in the log, without zero counts.
fn usage(log: &Value) -> Vec<(String, Value)> {
	log
		.as_object()
		.unwrap()
		.iter()
		.filter(|(key, value)| {
			(key.starts_with("gen_ai.usage.") && **value != json!(0)) || *key == "gen_ai.response.model"
		})
		.map(|(key, value)| (key.clone(), value.clone()))
		.collect()
}

/// A client protocol the provider serves too, with its route and a request for a reply.
#[derive(Debug, Clone, Copy)]
enum SameProtocol {
	Chat,
	Messages,
	Responses,
}

// A provider serving the client's own protocol gets what legacy sends it, and the client gets
// what legacy gives it. Legacy re-encodes a whole reply, where the IR sends the provider's bytes
// on, so those are compared as JSON; a stream is the same bytes either way.
#[rstest::rstest]
#[case::chat(SameProtocol::Chat, "completions/basic.json")]
#[case::chat_stream(SameProtocol::Chat, "completions/stream.json")]
#[case::messages(SameProtocol::Messages, "anthropic/basic.json")]
#[case::messages_stream(SameProtocol::Messages, "anthropic/stream_basic.json")]
#[case::responses(SameProtocol::Responses, "responses/basic.json")]
#[case::responses_stream(SameProtocol::Responses, "responses/stream.json")]
#[tokio::test]
async fn llm_ir_passes_through_what_legacy_does(
	#[case] protocol: SameProtocol,
	#[case] fixture: &str,
) {
	let legacy = serve_same_protocol(protocol, fixture, false).await;
	let ir = serve_same_protocol(protocol, fixture, true).await;
	assert_eq!(ir.upstream, legacy.upstream);
	assert_eq!(ir.status, 200);
	assert_eq!(ir.status, legacy.status);
	if fixture.contains("stream") {
		assert_eq!(
			String::from_utf8_lossy(&ir.reply),
			String::from_utf8_lossy(&legacy.reply)
		);
	} else {
		let json = |reply: &[u8]| serde_json::from_slice::<Value>(reply).unwrap();
		assert_eq!(json(&ir.reply), json(&legacy.reply));
	}
	assert_eq!(usage(&ir.log), usage(&legacy.log));
	assert!(!usage(&ir.log).is_empty(), "{}", ir.log);
	assert_eq!(ir.log["agw.ai.converter"], "llm_ir_passthrough");
}

struct PassedThrough {
	/// What the provider got.
	upstream: Value,
	status: u16,
	reply: Vec<u8>,
	log: Value,
}

async fn serve_same_protocol(protocol: SameProtocol, fixture: &str, llm_ir: bool) -> PassedThrough {
	let reply = std::fs::read(
		std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../llm/src/tests/response")
			.join(fixture),
	)
	.unwrap();
	let stream = fixture.contains("stream");
	let content_type = if stream {
		"text/event-stream"
	} else {
		"application/json"
	};
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(reply, content_type))
		.mount(&mock)
		.await;
	let (provider, route, kind, body) = match protocol {
		SameProtocol::Chat => (
			AIProvider::OpenAI(openai::Provider {
				model_override: None,
				moderation: None,
			}),
			"/v1/chat/completions",
			"completions",
			json!({"model": "gpt-5", "stream": stream, "seed": 7, "max_tokens": 64,
				"messages": [{"role": "user", "content": "hi"}]}),
		),
		SameProtocol::Messages => (
			AIProvider::Anthropic(serde_json::from_value(json!({})).unwrap()),
			"/v1/messages",
			"messages",
			json!({"model": "claude-haiku-4-5", "stream": stream, "max_tokens": 64,
				"metadata": {"user_id": "u1"},
				"messages": [{"role": "user", "content": "hi"}]}),
		),
		SameProtocol::Responses => (
			AIProvider::OpenAI(openai::Provider {
				model_override: None,
				moderation: None,
			}),
			"/v1/responses",
			"responses",
			json!({"model": "gpt-5", "stream": stream, "store": false,
				"input": [{"role": "user", "content": "hi"}]}),
		),
	};
	let mut provider = llm_named_provider(&mock, provider, false);
	provider.policies =
		Some(serde_json::from_value(json!({"ai": {"routes": {route: kind}}})).unwrap());
	let config = if llm_ir {
		llm_ir_on(json!({}))
	} else {
		agentgateway::config::parse_config("{}".to_string(), None).unwrap()
	};
	let (mock, _bind, io) = setup_llm_named_provider_mock_with_config(mock, provider, config);
	let path = format!("/{}{route}", rand::rng().random::<u128>());
	let res = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.to_string().as_bytes(),
	)
	.await;
	let status = res.status().as_u16();
	let reply = read_body_raw(res.into_body()).await.to_vec();
	let upstream = mock.received_requests().await.unwrap();
	assert_eq!(upstream.len(), 1);
	PassedThrough {
		upstream: serde_json::from_slice(&upstream[0].body).unwrap(),
		status,
		reply,
		log: request_log(&path).await,
	}
}

/// A Chat Completions client on Anthropic, with prompt enrichment and caching: the IR adds the
/// same prompts and cache marks legacy does.
#[rstest::rstest]
#[case::default_caching(json!({}))]
#[case::everything_cached(json!({"cacheTools": true, "minTokens": null}))]
#[case::an_earlier_turn(json!({"cacheSystem": false, "cacheMessageOffset": 1}))]
#[tokio::test]
async fn llm_ir_enriches_and_caches_as_legacy_does(#[case] caching: Value) {
	let legacy = serve_enriched(&caching, false).await;
	let ir = serve_enriched(&caching, true).await;
	assert_eq!(ir.log["agw.ai.converter"], "llm_ir", "{}", ir.log);
	assert_eq!(marked(&ir.upstream), marked(&legacy.upstream));
}

async fn serve_enriched(caching: &Value, llm_ir: bool) -> PassedThrough {
	let reply = std::fs::read(
		std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../llm/src/tests/response/anthropic/basic.json"),
	)
	.unwrap();
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(reply, "application/json"))
		.mount(&mock)
		.await;
	let provider = AIProvider::Anthropic(serde_json::from_value(json!({})).unwrap());
	let mut provider = llm_named_provider(&mock, provider, false);
	provider.policies = Some(
		serde_json::from_value(json!({"ai": {
			"routes": {"/v1/chat/completions": "completions"},
			"prompts": {
				"prepend": [{"role": "system", "content": "Rule one"}, {"role": "user", "content": "Context"}],
				"append": [{"role": "system", "content": "Rule two"}, {"role": "assistant", "content": "Noted"}],
			},
			"promptCaching": caching,
		}}))
		.unwrap(),
	);
	let config = if llm_ir {
		llm_ir_on(json!({}))
	} else {
		agentgateway::config::parse_config("{}".to_string(), None).unwrap()
	};
	let (mock, _bind, io) = setup_llm_named_provider_mock_with_config(mock, provider, config);
	let body = json!({"model": "claude-haiku-4-5", "max_tokens": 64,
		"tools": [{"type": "function", "function": {"name": "look", "parameters": {"type": "object"}}}],
		"messages": [
			{"role": "system", "content": "Be brief"},
			{"role": "user", "content": "a"},
			{"role": "assistant", "content": "b"},
			{"role": "user", "content": "c"}]});
	let path = format!("/{}/v1/chat/completions", rand::rng().random::<u128>());
	let res = send_request_body(
		io,
		Method::POST,
		&format!("http://lo{path}"),
		body.to_string().as_bytes(),
	)
	.await;
	let status = res.status().as_u16();
	let reply = read_body_raw(res.into_body()).await.to_vec();
	let upstream = mock.received_requests().await.unwrap();
	assert_eq!(
		upstream.len(),
		1,
		"{status}: {}",
		String::from_utf8_lossy(&reply)
	);
	PassedThrough {
		upstream: serde_json::from_slice(&upstream[0].body).unwrap(),
		status,
		reply,
		log: request_log(&path).await,
	}
}

/// What a Messages request says, with whether each part is marked for caching: the system
/// text, each text with its role, and the tools. Legacy joins system texts into one and keeps
/// adjacent turns of a role apart, where the IR sends a block for each and joins the turns, so
/// both are flattened; Anthropic reads them alike.
fn marked(request: &Value) -> Value {
	let blocks = |content: &Value| -> Vec<(String, bool)> {
		match content {
			Value::String(text) => vec![(text.clone(), false)],
			Value::Array(blocks) => blocks
				.iter()
				.map(|block| {
					let text = block["text"].as_str().unwrap().to_string();
					(text, block.get("cache_control").is_some())
				})
				.collect(),
			other => panic!("unexpected content {other}"),
		}
	};
	let system = blocks(&request["system"]);
	let system_text: Vec<&str> = system.iter().map(|(text, _)| text.as_str()).collect();
	// Only the last system block can carry a mark, so where it sits is the same.
	let system_marked = system.last().is_some_and(|(_, marked)| *marked);
	let turns: Vec<Value> = request["messages"]
		.as_array()
		.unwrap()
		.iter()
		.flat_map(|message| {
			blocks(&message["content"])
				.into_iter()
				.map(|(text, marked)| json!([message["role"], text, marked]))
		})
		.collect();
	json!({
		"system": [system_text.join("\n"), system_marked],
		"turns": turns,
		"tools": request["tools"]
			.as_array()
			.unwrap()
			.iter()
			.map(|tool| json!([tool["name"], tool.get("cache_control").is_some()]))
			.collect::<Vec<_>>(),
	})
}
