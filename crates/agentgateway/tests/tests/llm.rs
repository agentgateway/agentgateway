use agentgateway::llm::{AIProvider, custom, gemini, openai};
use agentgateway::test_helpers::ratelimitmock;
use tokio::sync::mpsc;
use url::Position;

use crate::common::prelude::*;

#[tokio::test]
async fn llm_openai() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
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
	assert_llm(
		io,
		include_bytes!("../../src/llm/tests/requests/completions/basic.json"),
		want,
	)
	.await;
}

#[tokio::test]
async fn llm_openai_tokenize() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
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
	assert_llm(
		io,
		include_bytes!("../../src/llm/tests/requests/completions/basic.json"),
		want,
	)
	.await;
}

#[tokio::test]
async fn llm_detect_mode_passthrough_without_rewrite() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		name: "default".into(),
		provider: AIProvider::OpenAI(openai::Provider { model: None }),
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
	let body = include_bytes!("../../src/llm/tests/requests/completions/basic.json");

	let res = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions?trace=repro")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(body.to_vec()))
		.send(io.clone())
		.await
		.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterQuery],
		"/v1/chat/completions?trace=repro"
	);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
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
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		name: "default".into(),
		provider: AIProvider::OpenAI(openai::Provider { model: None }),
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
	let body = include_bytes!("../../src/llm/tests/requests/completions/basic.json");

	let res = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions?trace=rewrite")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(body.to_vec()))
		.send(io.clone())
		.await
		.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterQuery],
		"/v1/chat/completions?trace=rewrite"
	);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
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
	t.pi.stores.binds.sync_local(
		normalized.binds,
		normalized.listener_routes,
		normalized.listener_tcp_routes,
		normalized.policies,
		normalized.backends,
		normalized.route_groups,
		Default::default(),
	);
	t
}

#[tokio::test]
async fn llm_local_router_handles_models_virtual_model_and_missing_model() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let config = format!(
		r#"
llm:
  port: 4000
  models:
  - name: real-model
    visibility: internal
    provider: openAI
    authorization:
      rules:
      - 'request.headers["x-model-auth"] == "yes"'
    params:
      baseUrl: http://{}
    health:
      eviction: {{}}
      unhealthyExpression: 'response.code == 403'
  - name: prefix/*
    visibility: internal
    provider: openai
    params:
      baseUrl: http://{}
    transformation:
      model: llmRequest.model.stripPrefix("prefix/")
  - name: direct-model
    provider: openAI
    authorization:
      rules:
      - 'request.headers["x-model-auth"] == "yes"'
    params:
      baseUrl: http://{}
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
	let io = t.serve_http(strng::literal!("bind/4000"));

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

		let upstream_requests = mock.received_requests().await.expect("upstream requests");
		assert_eq!(upstream_requests.len(), 1);
		let upstream_body: Value =
			serde_json::from_slice(&upstream_requests[0].body).expect("upstream request JSON");
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
async fn llm_conditional_virtual_model_no_match_returns_json_error() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let config = format!(
		r#"
llm:
  port: 4000
  models:
  - name: real-model
    visibility: internal
    provider: openAI
    params:
      baseUrl: http://{}
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
	let io = t.serve_http(strng::literal!("bind/4000"));
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
async fn llm_model_router_handles_multipart_audio_detect_request() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let config = format!(
		r#"
llm:
  port: 4000
  models:
  - name: real-model
    provider: openAI
    params:
      baseUrl: http://{}
    passthrough: detect
"#,
		mock.address()
	);
	let t = setup_local_llm_config(&config).await;
	let io = t.serve_http(strng::literal!("bind/4000"));
	let body = concat!(
		"--audio-boundary\r\n",
		"Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n",
		"Content-Type: audio/wav\r\n",
		"\r\n",
		"fake-audio-bytes\r\n",
		"--audio-boundary\r\n",
		"Content-Disposition: form-data; name=\"model\"\r\n",
		"\r\n",
		"real-model\r\n",
		"--audio-boundary--\r\n",
	)
	.as_bytes();

	let res = RequestBuilder::new(Method::POST, "http://lo/v1/audio/transcriptions")
		.header(
			header::CONTENT_TYPE,
			"multipart/form-data; boundary=audio-boundary",
		)
		.body(Body::from(body.to_vec()))
		.send(io.clone())
		.await
		.unwrap();
	assert_eq!(res.status(), StatusCode::OK);
	let _ = read_body_raw(res.into_body()).await;

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterPath],
		"/v1/audio/transcriptions"
	);
	assert_eq!(requests[0].body, body);

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", "/v1/audio/transcriptions"),
	])
	.await
	.unwrap();
	let want = json!({
		"gen_ai.provider.name": "openai",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23
	});
	assert!(is_json_subset(&want, &log), "want={want:#?} got={log:#?}");
}

#[tokio::test]
async fn llm_custom_rerank() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/cohere/rerank.json"
	))
	.await;
	let provider = agentgateway::types::local::LocalNamedAIProvider {
		name: "default".into(),
		provider: AIProvider::Custom(custom::Provider {
			model: None,
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
		include_bytes!("../../src/llm/tests/requests/rerank/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let body: Value =
		serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(body["results"][0]["index"], 2);
	assert_eq!(body["results"][0]["relevance_score"], 0.91);

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
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
		))
		.with_route(basic_named_route(strng::format!("/{backend_name}")));
	let io = t.serve_http(BIND_KEY);
	(mock, t, io)
}

#[tokio::test]
async fn llm_custom_provider_routes_to_provider_backend() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Completions]);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 200);
	let _ = res.into_body().collect().await.unwrap();

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterPath],
		"/v1/chat/completions"
	);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["model"], "replaceme");
}

#[tokio::test]
async fn llm_custom_provider_uses_upstream_route_fallback() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/anthropic/basic.json"
	))
	.await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Messages]);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 200);
	let response_body: Value =
		serde_json::from_slice(&read_body_raw(res.into_body()).await).expect("response is JSON");
	assert_eq!(response_body["object"], "chat.completion");
	assert_eq!(response_body["usage"]["prompt_tokens"], 15);
	assert_eq!(response_body["usage"]["completion_tokens"], 21);

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterPath],
		"/v1/messages"
	);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["system"], "You are a helpful assistant.");
	assert_eq!(upstream_body["messages"][0]["role"], "user");
}

#[tokio::test]
async fn llm_custom_provider_uses_format_path_override() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/anthropic/basic.json"
	))
	.await;
	let (mock, _bind, io) = setup_custom_llm_provider_backend_mock_with_formats(
		mock,
		vec![custom::ProviderFormatConfig {
			format: custom::ProviderFormat::Messages,
			path: Some(strng::literal!("/api/messages")),
		}],
	);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 200);
	let _ = res.into_body().collect().await.unwrap();

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterPath],
		"/api/messages"
	);
}

#[tokio::test]
async fn llm_custom_provider_rejects_unsupported_format_before_upstream_call() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Embeddings]);

	let res = send_completions_with_model(io, "replaceme", &[]).await;
	assert_eq!(res.status(), 503);
	let body = res.into_body().collect().await.unwrap().to_bytes();
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
	let mut body: Value = serde_json::from_slice(include_bytes!(
		"../../src/llm/tests/requests/completions/basic.json"
	))
	.expect("request fixture should be valid JSON");
	if streaming {
		body["stream"] = json!(true);
	}
	serde_json::to_vec(&body).expect("request fixture should serialize")
}

fn completions_request_body_with_model(model: &str) -> Vec<u8> {
	let mut body: Value = serde_json::from_slice(include_bytes!(
		"../../src/llm/tests/requests/completions/basic.json"
	))
	.expect("request fixture should be valid JSON");
	body["model"] = json!(model);
	serde_json::to_vec(&body).expect("request fixture should serialize")
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
		AIProvider::OpenAI(openai::Provider { model: None }),
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
	let _ = res.into_body().collect().await.unwrap();

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
		include_bytes!("../../src/llm/tests/response/completions/basic.json"),
		&completions_request_body(false),
		23017,
	)
	.await;
}

#[tokio::test]
async fn llm_streaming_remote_rate_limit_cost_amends_response_tokens() {
	assert_llm_remote_rate_limit_cost(
		include_bytes!("../../src/llm/tests/response/completions/stream.json"),
		&completions_request_body(true),
		286018,
	)
	.await;
}

#[rstest::rstest]
#[case::preserves_path(None, None, "/v1/messages?trace=repro")]
#[case::path_override(Some("/custom/chat/completions"), None, "/custom/chat/completions")]
#[case::path_prefix(None, Some("/v1/custom/"), "/v1/custom/chat/completions?trace=repro")]
#[tokio::test]
async fn llm_openai_messages_translation_with_host_override_path_behavior(
	#[case] path_override: Option<&str>,
	#[case] path_prefix: Option<&str>,
	#[case] expected_url: &str,
) {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
	let provider = agentgateway::test_helpers::proxymock::llm_named_provider(
		&mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
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
		include_bytes!("../../src/llm/tests/requests/messages/basic.json"),
	)
	.await;

	assert_eq!(res.status(), 200);
	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	let upstream = &requests[0];
	assert_eq!(
		&upstream.url[Position::BeforePath..Position::AfterQuery],
		expected_url
	);
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
		AIProvider::OpenAI(openai::Provider { model: None }),
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
	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterQuery],
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
		AIProvider::Gemini(gemini::Provider { model: None }),
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
	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterQuery],
		expected_url
	);
}

#[tokio::test]
async fn llm_log_body() {
	let mock = body_mock(include_bytes!(
		"../../src/llm/tests/response/completions/basic.json"
	))
	.await;
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
		AIProvider::OpenAI(openai::Provider { model: None }),
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
	assert_llm(
		io,
		include_bytes!("../../src/llm/tests/requests/completions/basic.json"),
		want,
	)
	.await;
}

async fn assert_llm(io: MemoryClient, body: &[u8], want: Value) {
	let r = rand::rng().random::<u128>();
	let res = send_request_body(io.clone(), Method::POST, &format!("http://lo/{r}"), body).await;

	// Ensure body finishes
	let _ = res.into_body().collect().await.unwrap();
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
