use agentgateway::llm::{AIProvider, codex_subscription, custom, gemini, openai};
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
			model: None,
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

#[tokio::test]
async fn llm_openai_tokenize() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model: None,
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
			model: None,
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
			model: None,
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
			model: None,
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
			model: None,
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
      baseUrl: http://{}
  - name: direct-model
    provider: openAI
    params:
      baseUrl: http://{}
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

fn codex_completion_sse() -> String {
	let mut response: Value =
		serde_json::from_slice(llm_body!("response/responses/basic.json")).unwrap();
	response["model"] = json!("gpt-5.6-luna");
	response["output"][0]["content"][0]["text"] = json!("Hello!");
	let item = response["output"][0].clone();
	response["output"] = json!([]);
	[
		json!({"type": "response.created", "response": response}),
		json!({"type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": "Hello"}),
		json!({"type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": "!"}),
		json!({"type": "response.output_item.done", "output_index": 0, "item": item}),
		json!({"type": "response.completed", "response": response}),
	].iter().map(|event| format!("data: {event}\n\n")).collect()
}

#[tokio::test]
async fn llm_codex_subscription_completions_facade_and_openai_isolation() {
	for codex in [true, false] {
		for streaming in [false, true] {
			for include_usage in [None, Some(false), Some(true)] {
				let mock = MockServer::start().await;
				Mock::given(wiremock::matchers::method("GET"))
					.respond_with(ResponseTemplate::new(200).set_body_json(
						json!({"models": [{"slug": "gpt-5.6-luna", "visibility": "list", "supported_in_api": true}]}),
					))
					.mount(&mock)
					.await;
				let upstream_body = if codex {
					codex_completion_sse()
				} else if streaming {
					"data: {\"id\":\"chat_1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-5.6-luna\",\"created\":123,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello!\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_owned()
				} else {
					String::from_utf8_lossy(llm_body!("response/completions/basic.json")).into_owned()
				};
				Mock::given(wiremock::matchers::method("POST"))
					.respond_with(ResponseTemplate::new(200).set_body_raw(
						upstream_body.clone(),
						if codex || streaming {
							"text/event-stream"
						} else {
							"application/json"
						},
					))
					.mount(&mock)
					.await;
				let ai = if codex {
					AIProvider::CodexSubscription(codex_subscription::Provider {
						refresh_interval: Duration::from_secs(60),
						stale_while_revalidate: Duration::from_secs(60),
						allow_models: vec!["*".into()],
						deny_models: vec![],
					})
				} else {
					AIProvider::OpenAI(openai::Provider {
						model: None,
						moderation: None,
					})
				};
				let mut provider = llm_named_provider(&mock, ai, false);
				provider.path_prefix = Some(
					if codex {
						codex_subscription::DEFAULT_BASE_PATH
					} else {
						"/v1"
					}
					.into(),
				);
				let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
				let mut request = json!({"model": if codex { "openai/gpt-5.6-luna" } else { "gpt-5.6-luna" }, "messages": [{"role": "user", "content": "Hello"}], "temperature": 0.7, "max_tokens": 256, "stream": streaming});
				if let Some(include_usage) = include_usage {
					request["stream_options"] = json!({"include_usage": include_usage});
				}
				if include_usage == Some(true) {
					request["messages"].as_array_mut().unwrap().extend([
						json!({"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}),
						json!({"role": "tool", "name": "lookup", "tool_call_id": "call_1", "content": "found"}),
						json!({"role": "user", "content": "Hello"}),
					]);
				}
				if include_usage == Some(false) {
					let neutral = json!({"n": 1, "logprobs": false, "top_logprobs": null, "logit_bias": {}, "frequency_penalty": 0, "presence_penalty": 0, "top_p": 1, "stop": null, "seed": null, "store": false});
					request
						.as_object_mut()
						.unwrap()
						.extend(neutral.as_object().unwrap().clone());
				}
				let response = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions")
					.body(Body::from(request.to_string()))
					.send(io)
					.await
					.unwrap();
				let status = response.status();
				let content_type = response.headers()["content-type"]
					.to_str()
					.unwrap()
					.to_owned();
				let body = read_body_raw(response.into_body()).await;
				assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
				if streaming {
					assert!(content_type.starts_with("text/event-stream"));
					let body = String::from_utf8(body.to_vec()).unwrap();
					assert_eq!(body.matches("[DONE]").count(), 1, "{body}");
					if codex {
						assert!(!body.contains("response.output"));
						let chunks: Vec<Value> = body
							.lines()
							.filter_map(|line| line.strip_prefix("data: "))
							.filter(|line| *line != "[DONE]")
							.map(|line| serde_json::from_str(line).unwrap())
							.collect();
						assert_eq!(
							chunks
								.iter()
								.filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
								.collect::<String>(),
							"Hello!"
						);
						assert_eq!(
							chunks.iter().filter(|c| c["choices"] == json!([])).count(),
							usize::from(include_usage.unwrap_or_default())
						);
						for chunk in &chunks {
							assert_eq!(chunk["object"], "chat.completion.chunk");
							assert_eq!(chunk["model"], "gpt-5.6-luna");
							assert_eq!(chunk["id"], chunks[0]["id"]);
							assert_eq!(chunk["created"], chunks[0]["created"]);
						}
						assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
						assert_eq!(
							chunks
								.iter()
								.filter_map(|c| c["choices"][0]["finish_reason"].as_str())
								.collect::<Vec<_>>(),
							["stop"]
						);
					} else {
						assert_eq!(body, upstream_body);
					}
				} else {
					assert!(content_type.starts_with("application/json"));
					let body: Value = serde_json::from_slice(&body).unwrap();
					assert_eq!(body["object"], "chat.completion");
					if codex {
						assert_eq!(body["choices"][0]["message"]["content"], "Hello!");
						assert_eq!(body["choices"][0]["finish_reason"], "stop");
						assert!(body["usage"]["prompt_tokens"].is_number());
					}
				}
				let requests = mock.received_requests().await.unwrap();
				let upstream = requests.iter().find(|r| r.method == Method::POST).unwrap();
				assert_eq!(
					upstream.url.path(),
					if codex {
						"/backend-api/codex/responses"
					} else {
						"/v1/chat/completions"
					}
				);
				let body: Value = serde_json::from_slice(&upstream.body).unwrap();
				if codex {
					let mut expected_input =
						vec![json!({"role": "user", "content": [{"type": "input_text", "text": "Hello"}]})];
					if include_usage == Some(true) {
						expected_input.extend([
							json!({"type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{}"}),
							json!({"type": "function_call_output", "call_id": "call_1", "output": "found"}),
							json!({"role": "user", "content": [{"type": "input_text", "text": "Hello"}]}),
						]);
					}
					assert_eq!(body["input"], json!(expected_input));
					assert_eq!(body["store"], false);
					assert_eq!(body["stream"], true);
					assert_eq!(body["instructions"], "");
					for key in [
						"messages",
						"temperature",
						"max_tokens",
						"max_completion_tokens",
						"max_output_tokens",
						"stream_options",
					] {
						assert!(body.get(key).is_none(), "{key}");
					}
				} else {
					assert_eq!(body["messages"], request["messages"]);
					assert!(body.get("input").is_none());
					assert_eq!(body["temperature"], request["temperature"]);
					assert_eq!(body["max_completion_tokens"], 256);
				}
			}
		}
	}
}

#[tokio::test]
async fn llm_codex_subscription_completions_rejects_unsupported_and_upstream_errors() {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::method("GET"))
		.respond_with(ResponseTemplate::new(200).set_body_json(
			json!({"models": [{"slug": "gpt-5.6-luna", "visibility": "list", "supported_in_api": true}]}),
		))
		.mount(&mock)
		.await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	for extra in [
		json!({"n": 2}),
		json!({"n": 0}),
		json!({"n": "bad"}),
		json!({"logprobs": true}),
		json!({"audio": {"voice": "alloy"}}),
	] {
		let mut request =
			json!({"model": "openai/gpt-5.6-luna", "messages": [{"role": "user", "content": "Hello"}]});
		request
			.as_object_mut()
			.unwrap()
			.extend(extra.as_object().unwrap().clone());
		let response = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions")
			.body(Body::from(request.to_string()))
			.send(io.clone())
			.await
			.unwrap();
		let status = response.status();
		let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
		assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
		assert_eq!(body["error"]["type"], "invalid_request_error");
	}
	assert!(
		mock
			.received_requests()
			.await
			.unwrap()
			.iter()
			.all(|r| r.method != Method::POST)
	);
	for (status, wire, expected_status, expected_code) in [
		(429, json!({"error": {"type": "rate_limit_error", "code": "rate_limit", "message": "try later"}}).to_string(), StatusCode::TOO_MANY_REQUESTS, Some("rate_limit")),
		(200, "data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"server_error\",\"message\":\"failed\"}}}\n\n".into(), StatusCode::BAD_GATEWAY, Some("server_error")),
		(200, "data: {\"type\":\"error\",\"code\":\"server_error\",\"message\":\"failed\"}\n\n".into(), StatusCode::BAD_GATEWAY, Some("server_error")),
		(200, "data: [DONE]\n\n".into(), StatusCode::BAD_GATEWAY, None),
	] {
		let guard = Mock::given(wiremock::matchers::method("POST")).respond_with(ResponseTemplate::new(status).set_body_raw(wire, if status == 200 { "text/event-stream" } else { "application/json" })).mount_as_scoped(&mock).await;
		let response = send_completions_with_model(io.clone(), "openai/gpt-5.6-luna", &[]).await;
		let status = response.status();
		let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
		assert_eq!(status, expected_status, "{body}");
		assert!(body.get("choices").is_none());
		if let Some(code) = expected_code { assert_eq!(body["error"]["code"], code); }
		drop(guard);
	}
}

#[tokio::test]
async fn llm_codex_subscription_responses_contract_and_openai_isolation() {
	for codex in [true, false] {
		for streaming in [None, Some(false), Some(true)] {
			let mock = MockServer::start().await;
			Mock::given(wiremock::matchers::method("GET"))
				.and(wiremock::matchers::path("/backend-api/codex/models"))
				.respond_with(ResponseTemplate::new(200).set_body_json(json!({
					"models": [{"slug": "gpt-codex", "visibility": "list", "supported_in_api": true}]
				})))
				.mount(&mock)
				.await;
			let mut terminal: Value =
				serde_json::from_slice(llm_body!("response/responses/basic.json")).unwrap();
			terminal["output"].as_array_mut().unwrap().push(json!({
				"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "opaque-reasoning"
			}));
			terminal["output"].as_array_mut().unwrap().push(json!({
				"type": "function_call", "id": "fc_2", "call_id": "call_2", "name": "lookup", "arguments": "{}", "status": "completed"
			}));
			let done_events = terminal["output"]
				.as_array()
				.unwrap()
				.iter()
				.enumerate()
				.map(|(index, item)| {
					format!(
						"event: response.output_item.done\r\ndata: {}\r\n\r\n",
						json!({
							"type": "response.output_item.done", "output_index": index, "item": item
						})
					)
				})
				.collect::<String>();
			let mut wire_terminal = terminal.clone();
			// Real Codex sends full items before a terminal response with empty or missing output.
			if codex {
				if streaming.is_none() {
					wire_terminal.as_object_mut().unwrap().remove("output");
				} else {
					wire_terminal["output"] = json!([]);
				}
			}
			let sse = format!(
				"event: response.reasoning_summary_text.delta\r\ndata: {{\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"Thinking\"}}\r\n\r\nevent: response.function_call_arguments.delta\r\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_2\",\"delta\":\"{{}}\"}}\r\n\r\n{done_events}event: response.completed\r\ndata: {}\r\n\r\n",
				json!({
					"type": "response.completed", "sequence_number": 1, "response": wire_terminal
				})
			);
			let wire_body = if codex || streaming == Some(true) {
				sse.clone()
			} else {
				terminal.to_string()
			};
			let content_type = if codex || streaming == Some(true) {
				"text/event-stream"
			} else {
				"application/json"
			};
			Mock::given(wiremock::matchers::method("POST"))
				.respond_with(ResponseTemplate::new(200).set_body_raw(wire_body, content_type))
				.mount(&mock)
				.await;
			let ai = if codex {
				AIProvider::CodexSubscription(codex_subscription::Provider {
					refresh_interval: Duration::from_secs(60),
					stale_while_revalidate: Duration::from_secs(60),
					allow_models: vec!["*".into()],
					deny_models: vec![],
				})
			} else {
				AIProvider::OpenAI(openai::Provider {
					model: None,
					moderation: None,
				})
			};
			let mut provider = llm_named_provider(&mock, ai, false);
			provider.path_prefix = Some(
				if codex {
					codex_subscription::DEFAULT_BASE_PATH
				} else {
					"/v1"
				}
				.into(),
			);
			provider.policies = Some(
				serde_json::from_value(json!({"ai": {
					"routes": {"/v1/responses": "responses"},
					"overrides": {"max_output_tokens": 8192}
				}}))
				.unwrap(),
			);
			let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
			let mut body = json!({
				"model": if codex { "openai/gpt-codex" } else { "gpt-codex" },
				"input": [
					{"role": "developer", "content": [{"type": "input_text", "text": "Be helpful", "prompt_cache_breakpoint": {"mode": "explicit"}}]},
					{"role": "user", "content": [
						{"type": "input_text", "text": "Describe this"},
						{"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8=", "detail": "auto"}
					]},
					{"type": "reasoning", "id": "rs_0", "summary": [], "encrypted_content": "opaque-history"},
					{"type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{}"},
					{"type": "function_call_output", "call_id": "call_1", "output": [{"type": "input_text", "text": "found", "prompt_cache_breakpoint": {"mode": "explicit"}}]}
				],
				"instructions": "Preserve these instructions", "store": true,
				"max_output_tokens": 4096, "temperature": 0.5, "prompt_cache_retention": "24h", "safety_identifier": "user-test",
				"reasoning": {"effort": "high", "summary": "auto"},
				"tools": [{"type": "function", "name": "lookup", "parameters": {"type": "object", "properties": {}}, "strict": false}],
				"tool_choice": "auto", "parallel_tool_calls": true,
				"include": ["reasoning.encrypted_content", "web_search_call.action.sources"],
				"text": {"verbosity": "low", "format": {"type": "json_schema", "name": "answer", "schema": {"type": "object"}}},
				"stream_options": {"reasoning_summary_delivery": "sequential_cutoff"},
				"prompt_cache_key": "session-test", "service_tier": "auto",
				"prompt_cache_breakpoint": {"mode": "explicit"},
				"metadata": {"max_output_tokens": "nested preserved"}, "future_field": {"keep": true}
			});
			if let Some(streaming) = streaming {
				body["stream"] = json!(streaming);
			}
			let response = RequestBuilder::new(Method::POST, "http://lo/v1/responses")
				.body(Body::from(body.to_string()))
				.send(io)
				.await
				.unwrap();
			let status = response.status();
			let response_type = response.headers()["content-type"]
				.to_str()
				.unwrap()
				.to_owned();
			let received = read_body_raw(response.into_body()).await;
			assert_eq!(
				status,
				StatusCode::OK,
				"{}",
				String::from_utf8_lossy(&received)
			);
			if streaming == Some(true) {
				assert!(response_type.starts_with("text/event-stream"));
				assert_eq!(received.as_ref(), sse.as_bytes());
			} else {
				assert!(response_type.starts_with("application/json"));
				let response: Value = serde_json::from_slice(&received).unwrap();
				assert_eq!(response["output"], terminal["output"]);
				assert_eq!(response["usage"], terminal["usage"]);
			}
			let requests = mock.received_requests().await.unwrap();
			let upstream = requests.iter().find(|r| r.method == Method::POST).unwrap();
			assert_eq!(
				upstream.url.path(),
				if codex {
					"/backend-api/codex/responses"
				} else {
					"/v1/responses"
				}
			);
			let actual: Value = serde_json::from_slice(&upstream.body).unwrap();
			body["model"] = json!("gpt-codex");
			if codex {
				body["input"][0]["content"][0]
					.as_object_mut()
					.unwrap()
					.remove("prompt_cache_breakpoint");
				body["input"][4]["output"][0]
					.as_object_mut()
					.unwrap()
					.remove("prompt_cache_breakpoint");
				for key in [
					"max_output_tokens",
					"temperature",
					"prompt_cache_breakpoint",
					"prompt_cache_retention",
					"safety_identifier",
				] {
					body.as_object_mut().unwrap().remove(key);
				}
				body["store"] = json!(false);
				body["stream"] = json!(true);
			} else {
				body["max_output_tokens"] = json!(8192);
			}
			// Compare every supplied field, including opaque history, unknown fields and nested limits.
			for (key, value) in body.as_object().unwrap() {
				assert_eq!(&actual[key], value, "{key}, codex={codex}");
			}
			if codex {
				for key in [
					"max_output_tokens",
					"temperature",
					"prompt_cache_breakpoint",
					"prompt_cache_retention",
					"safety_identifier",
				] {
					assert!(actual.get(key).is_none(), "{key}");
				}
			}
		}
	}
}

#[tokio::test]
async fn llm_codex_subscription_models_uses_catalog_cache() {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::method("GET"))
		.and(wiremock::matchers::path("/backend-api/codex/models"))
		.respond_with(
			ResponseTemplate::new(StatusCode::OK.as_u16()).set_body_json(json!({
				"models": [
					{"slug": "hidden", "visibility": "hidden", "supported_in_api": true},
					{"slug": "gpt-codex", "visibility": "list", "supported_in_api": true, "priority": 1}
				]
			})),
		)
		.mount(&mock)
		.await;
	let mut provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	provider.path_prefix = Some(codex_subscription::DEFAULT_BASE_PATH.into());
	provider.policies = Some(
		serde_json::from_value(json!({
			"ai": {"routes": {"/v1/models": "models"}},
			"backendAuth": {
				"key": {"value": "catalog-token"},
				"credentials": [{
					"location": {"header": {"name": "chatgpt-account-id"}},
					"key": "account-id"
				}]
			}
		}))
		.expect("models route policy"),
	);
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	for _ in 0..2 {
		let response = send_request(io.clone(), Method::GET, "http://lo/v1/models").await;
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response.headers()[header::CACHE_CONTROL],
			"private, no-cache"
		);
		let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
		assert_eq!(body["object"], "list");
		assert_eq!(body["data"][0]["id"], "openai/gpt-codex");
		assert_eq!(body["data"][0]["object"], "model");
		assert_eq!(body["data"].as_array().unwrap().len(), 1);
	}

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterQuery],
		"/backend-api/codex/models?client_version=0.155.0-alpha.9.2",
	);
	assert_eq!(request.headers["accept"], "application/json");
	assert_eq!(request.headers["originator"], "codex_cli_rs");
	assert_eq!(
		request.headers["user-agent"],
		"codex_cli_rs/0.155.0-alpha.9.2"
	);
	assert_eq!(
		request.headers["authorization"],
		"Bearer test-codex-access-token"
	);
	assert_eq!(
		request.headers["chatgpt-account-id"],
		"test-codex-account-id"
	);
	assert_eq!(
		request.headers["x-openai-internal-codex-residency"],
		"test-codex-residency"
	);
}

#[tokio::test]
async fn llm_custom_provider_models_forwards_to_openai_compatible_catalog() {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::method("GET"))
		.and(wiremock::matchers::path("/v1/models"))
		.respond_with(
			ResponseTemplate::new(StatusCode::OK.as_u16()).set_body_json(json!({
				"object": "list",
				"data": [{"id": "moonshotai/Kimi-K3", "object": "model"}]
			})),
		)
		.mount(&mock)
		.await;
	let mut provider = llm_named_provider(
		&mock,
		AIProvider::Custom(custom::Provider {
			model: None,
			provider_override: None,
			formats: vec![custom::ProviderFormatConfig {
				format: custom::ProviderFormat::Completions,
				path: None,
			}],
		}),
		false,
	);
	provider.policies = Some(
		serde_json::from_value(json!({"ai": {"routes": {"/v1/models": "models"}}}))
			.expect("model-list route policy"),
	);
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	let response = send_request(io, Method::GET, "http://lo/v1/models").await;
	assert_eq!(response.status(), StatusCode::OK);
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["data"][0]["id"], "moonshotai/Kimi-K3");
	let request = single_upstream_request(&mock).await;
	assert_eq!(request.method, Method::GET);
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/v1/models"
	);
}

#[tokio::test]
async fn llm_codex_subscription_models_sanitizes_html_catalog_response() {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::method("GET"))
		.and(wiremock::matchers::path("/backend-api/codex/models"))
		.respond_with(
			ResponseTemplate::new(StatusCode::OK.as_u16())
				.insert_header("content-type", "text/html; charset=UTF-8")
				.set_body_string("<html>Cloudflare challenge</html>"),
		)
		.mount(&mock)
		.await;
	let mut provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	provider.policies = Some(
		serde_json::from_value(json!({"ai": {"routes": {"/v1/models": "models"}}}))
			.expect("models route policy"),
	);
	let (_mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let response = send_request(io, Method::GET, "http://lo/v1/models").await;
	assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
	assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["error"]["code"], "catalog_unavailable");
}

#[tokio::test]
async fn llm_codex_subscription_models_revalidates_with_etag() {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::method("GET"))
		.respond_with(
			ResponseTemplate::new(StatusCode::OK.as_u16())
				.insert_header("etag", "\"catalog-v1\"")
				.set_body_json(json!({
					"models": [{"slug": "gpt-codex", "visibility": "list", "supported_in_api": true}]
				})),
		)
		.mount(&mock)
		.await;
	let mut provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::ZERO,
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	provider.policies = Some(
		serde_json::from_value(json!({"ai": {"routes": {"/v1/models": "models"}}}))
			.expect("models route policy"),
	);
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let response = send_request(io.clone(), Method::GET, "http://lo/v1/models").await;
	assert_eq!(response.status(), StatusCode::OK);
	let _ = read_body_raw(response.into_body()).await;

	mock.reset().await;
	Mock::given(wiremock::matchers::method("GET"))
		.and(wiremock::matchers::header(
			"if-none-match",
			"\"catalog-v1\"",
		))
		.respond_with(ResponseTemplate::new(StatusCode::NOT_MODIFIED.as_u16()))
		.mount(&mock)
		.await;
	let response = send_request(io, Method::GET, "http://lo/v1/models").await;
	assert_eq!(response.status(), StatusCode::OK);
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["data"][0]["id"], "openai/gpt-codex");
	assert_eq!(mock.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn llm_codex_subscription_admits_catalog_models_before_inference() {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::method("GET"))
		.and(wiremock::matchers::path("/backend-api/codex/models"))
		.respond_with(
			ResponseTemplate::new(StatusCode::OK.as_u16()).set_body_json(json!({
				"models": [
					{"slug": "gpt-codex-hidden", "visibility": "hidden", "supported_in_api": true},
					{"slug": "gpt-codex-2026-08-25", "visibility": "list", "supported_in_api": true},
					{"slug": "gpt-codex-blocked", "visibility": "list", "supported_in_api": true}
				]
			})),
		)
		.mount(&mock)
		.await;
	Mock::given(wiremock::matchers::method("POST"))
		.respond_with(
			ResponseTemplate::new(StatusCode::OK.as_u16())
				.set_body_raw(codex_completion_sse(), "text/event-stream"),
		)
		.mount(&mock)
		.await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["gpt-codex-*".into()],
			deny_models: vec!["gpt-codex-blocked".into()],
		}),
		false,
	);
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let response = send_completions_with_model(io.clone(), "openai/gpt-codex-hidden", &[]).await;
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["error"]["type"], "invalid_request_error");
	assert_eq!(body["error"]["code"], "model_not_found");

	let response = send_completions_with_model(io.clone(), "openai/gpt-codex-blocked", &[]).await;
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["error"]["type"], "invalid_request_error");
	assert_eq!(body["error"]["code"], "model_not_allowed");

	for invalid in [
		"gpt-codex-2026-08-25",
		"other/gpt-codex-2026-08-25",
		"openai/",
		"openai/openai/gpt-codex-2026-08-25",
	] {
		let response = send_completions_with_model(io.clone(), invalid, &[]).await;
		assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	}
	let response = send_completions_with_model(io, "openai/gpt-codex-2026-08-25", &[]).await;
	assert_eq!(response.status(), StatusCode::OK);
	let _ = read_body_raw(response.into_body()).await;

	let requests = mock.received_requests().await.unwrap();
	assert_eq!(requests.len(), 2);
	let inference = requests
		.iter()
		.find(|request| request.method == Method::POST)
		.expect("only the admitted request reaches inference");
	let body: Value = serde_json::from_slice(&inference.body).unwrap();
	assert_eq!(body["model"], "gpt-codex-2026-08-25");
}

#[tokio::test]
async fn llm_codex_subscription_direct_backend_handles_control_model_before_catalog_admission() {
	let mock = MockServer::start().await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let response = send_completions_with_model(io, "codex-subscription-auth", &[]).await;
	assert_eq!(response.status(), StatusCode::OK);
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["object"], "chat.completion");
	assert_eq!(body["model"], "codex-subscription-auth");
	let instruction: Value = serde_json::from_str(
		body["choices"][0]["message"]["content"]
			.as_str()
			.expect("control response content"),
	)
	.expect("control response instruction");
	assert_eq!(instruction["status"], "authorized");
	assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn llm_codex_subscription_direct_backend_rejects_streaming_control_model() {
	let mock = MockServer::start().await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	let body = json!({
		"model": "codex-subscription-auth",
		"messages": [],
		"stream": true,
	});

	let response = RequestBuilder::new(Method::POST, "http://lo/v1/chat/completions")
		.body(Body::from(serde_json::to_vec(&body).unwrap()))
		.send(io)
		.await
		.expect("streaming control request");
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(
		body["error"]["code"],
		"codex_subscription_auth_streaming_unsupported"
	);
	assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn llm_codex_subscription_refreshes_model_unavailable_once() {
	let mock = MockServer::start().await;
	let catalog_requests = Arc::new(AtomicUsize::new(0));
	let inference_requests = Arc::new(AtomicUsize::new(0));
	Mock::given(wiremock::matchers::any())
		.respond_with({
			let catalog_requests = catalog_requests.clone();
			let inference_requests = inference_requests.clone();
			move |request: &wiremock::Request| {
				if request.method == Method::GET {
					catalog_requests.fetch_add(1, Ordering::SeqCst);
					ResponseTemplate::new(StatusCode::OK.as_u16()).set_body_json(json!({
						"models": [{"slug": "gpt-codex", "visibility": "list", "supported_in_api": true}]
					}))
				} else {
					let attempt = inference_requests.fetch_add(1, Ordering::SeqCst);
					if attempt == 0 {
						ResponseTemplate::new(StatusCode::NOT_FOUND.as_u16()).set_body_json(json!({
							"error": {"code": "model_not_found", "message": "retired"}
						}))
					} else {
						ResponseTemplate::new(StatusCode::OK.as_u16())
							.set_body_raw(codex_completion_sse(), "text/event-stream")
					}
				}
			}
		})
		.mount(&mock)
		.await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	let (_mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let response = send_completions_with_model(io, "openai/gpt-codex", &[]).await;
	assert_eq!(response.status(), StatusCode::OK);
	let _ = read_body_raw(response.into_body()).await;
	assert_eq!(catalog_requests.load(Ordering::SeqCst), 2);
	assert_eq!(inference_requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn llm_codex_subscription_does_not_refresh_for_message_only_match() {
	let mock = MockServer::start().await;
	let catalog_requests = Arc::new(AtomicUsize::new(0));
	let inference_requests = Arc::new(AtomicUsize::new(0));
	Mock::given(wiremock::matchers::any())
		.respond_with({
			let catalog_requests = catalog_requests.clone();
			let inference_requests = inference_requests.clone();
			move |request: &wiremock::Request| {
				if request.method == Method::GET {
					catalog_requests.fetch_add(1, Ordering::SeqCst);
					ResponseTemplate::new(StatusCode::OK.as_u16()).set_body_json(json!({
						"models": [{"slug": "gpt-codex", "visibility": "list", "supported_in_api": true}]
					}))
				} else {
					inference_requests.fetch_add(1, Ordering::SeqCst);
					ResponseTemplate::new(StatusCode::NOT_FOUND.as_u16())
						.insert_header("x-original", "yes")
						.set_body_json(json!({"error": {"code": "other", "message": "model_not_found"}}))
				}
			}
		})
		.mount(&mock)
		.await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	let (_mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let response = send_completions_with_model(io, "openai/gpt-codex", &[]).await;
	assert_eq!(response.status(), StatusCode::NOT_FOUND);
	assert_eq!(response.headers()["x-original"], "yes");
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["error"]["code"], "other");
	assert_eq!(catalog_requests.load(Ordering::SeqCst), 1);
	assert_eq!(inference_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn llm_codex_subscription_preserves_unavailable_response_when_refresh_removes_model() {
	let mock = MockServer::start().await;
	let catalog_requests = Arc::new(AtomicUsize::new(0));
	let inference_requests = Arc::new(AtomicUsize::new(0));
	Mock::given(wiremock::matchers::any())
		.respond_with({
			let catalog_requests = catalog_requests.clone();
			let inference_requests = inference_requests.clone();
			move |request: &wiremock::Request| {
				if request.method == Method::GET {
					let refresh = catalog_requests.fetch_add(1, Ordering::SeqCst) > 0;
					ResponseTemplate::new(StatusCode::OK.as_u16()).set_body_json(json!({
						"models": if refresh { vec![] } else { vec![json!({"slug": "gpt-codex", "visibility": "list", "supported_in_api": true})] }
					}))
				} else {
					inference_requests.fetch_add(1, Ordering::SeqCst);
					ResponseTemplate::new(StatusCode::NOT_FOUND.as_u16())
						.insert_header("x-original", "yes")
						.set_body_json(json!({"error": {"code": "model_not_found", "message": "retired"}}))
				}
			}
		})
		.mount(&mock)
		.await;
	let provider = llm_named_provider(
		&mock,
		AIProvider::CodexSubscription(codex_subscription::Provider {
			refresh_interval: Duration::from_secs(60),
			stale_while_revalidate: Duration::from_secs(60),
			allow_models: vec!["*".into()],
			deny_models: vec![],
		}),
		false,
	);
	let (_mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let response = send_completions_with_model(io, "openai/gpt-codex", &[]).await;
	assert_eq!(response.status(), StatusCode::NOT_FOUND);
	assert_eq!(response.headers()["x-original"], "yes");
	let body: Value = serde_json::from_slice(&read_body_raw(response.into_body()).await).unwrap();
	assert_eq!(body["error"]["code"], "model_not_found");
	assert_eq!(catalog_requests.load(Ordering::SeqCst), 2);
	assert_eq!(inference_requests.load(Ordering::SeqCst), 1);
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
async fn llm_model_router_handles_multipart_audio_detect_request() {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let config = format!(
		r#"
llm:
  port: 0
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
	let io = t.serve_http(strng::literal!("bind/0"));
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

	let request = single_upstream_request(&mock).await;
	assert_eq!(
		&request.url[Position::BeforePath..Position::AfterPath],
		"/v1/audio/transcriptions"
	);
	assert_eq!(request.body, body);

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
	let mock = body_mock(llm_body!("response/cohere/rerank.json")).await;
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
	route.inline_policies.push(TrafficPolicy::AI(
		agentgateway::llm::model_router::default_route_types(),
	));
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
	assert_eq!(response_body["usage"]["prompt_tokens"], 15);
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
			model: None,
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
#[case::path_prefix(None, Some("/v1/custom/"), "/v1/custom/chat/completions?trace=repro")]
#[tokio::test]
async fn llm_openai_messages_translation_with_host_override_path_behavior(
	#[case] path_override: Option<&str>,
	#[case] path_prefix: Option<&str>,
	#[case] expected_url: &str,
) {
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let provider = agentgateway::test_helpers::proxymock::llm_named_provider(
		&mock,
		AIProvider::OpenAI(openai::Provider {
			model: None,
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
	let mock = body_mock(llm_body!("response/completions/basic.json")).await;
	let (mock, mut bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider {
			model: None,
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
					// Drop a field the converter added.
					"reasoning_effort": r#"fail("remove")"#,
					// Observe the converted message list.
					"converted_message_count": "llmRequest.messages.size()"
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

	// The request really was converted to completions format.
	assert_eq!(upstream_body["messages"][0]["role"], json!("system"));
	// Indexing yields Null for a missing key, so assert on key presence.
	assert!(
		upstream_body.get("reasoning_effort").is_none(),
		"reasoning_effort should be removed, got: {upstream_body}"
	);
	assert_eq!(upstream_body["converted_message_count"], json!(2));
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
			model: None,
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
			model: None,
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
