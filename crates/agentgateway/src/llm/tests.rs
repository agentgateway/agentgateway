use std::fs;
use std::path::{Path, PathBuf};

use agent_core::strng;
use http_body_util::BodyExt;
use serde_json::{Value, json};

use super::*;
use crate::http::x_headers::TRACEPARENT;

fn llm_request_with_tokens(input_tokens: Option<u64>) -> LLMRequest {
	LLMRequest {
		input_tokens,
		input_format: InputFormat::Completions,
		cache_convention: CacheTokenConvention::pending(),
		request_model: "test-model".into(),
		provider: "test-provider".into(),
		streaming: true,
		params: Default::default(),
		prompt: None,
		provider_state: None,
	}
}

#[test]
fn streaming_amend_on_drop_updates_local_rate_limit() {
	let rate_limit =
		crate::http::localratelimit::RateLimit::try_from(crate::http::localratelimit::RateLimitSpec {
			max_tokens: 10,
			tokens_per_fill: 10,
			fill_interval: std::time::Duration::from_secs(60),
			limit_type: crate::http::localratelimit::RateLimitType::Tokens,
		})
		.unwrap();
	let log = AsyncLog::default();
	log.store(Some(LLMInfo {
		request: llm_request_with_tokens(Some(2)),
		response: LLMResponse {
			input_tokens: Some(2),
			output_tokens: Some(4),
			..Default::default()
		},
	}));

	let mut amend = AmendOnDrop::new(
		log,
		LLMResponsePolicies {
			local_rate_limit: vec![rate_limit.clone()],
			..Default::default()
		},
		None,
		None,
	);
	amend.report_usage();

	assert!(
		rate_limit
			.check_llm_request(&llm_request_with_tokens(Some(7)))
			.is_err()
	);
	assert!(
		rate_limit
			.check_llm_request(&llm_request_with_tokens(Some(6)))
			.is_ok()
	);
}

fn test_root() -> &'static Path {
	Path::new("../llm/src/tests")
}

fn fixture_path(relative_path: &str) -> PathBuf {
	test_root().join(relative_path)
}

#[test]
fn copilot_claude_formats_prefer_messages() {
	for model in ["claude-sonnet-4", "Claude-Sonnet-4"] {
		assert_eq!(
			copilot::Provider::supported_formats_for_model(Some(model)),
			vec![ChatFormat::AnthropicMessages],
			"{model}"
		);
	}
}

#[test]
fn copilot_non_claude_formats_are_unchanged() {
	for (model, expected) in [
		("gpt-4o", &[ChatFormat::OpenAICompletions][..]),
		(
			"gpt-5.4",
			&[ChatFormat::OpenAICompletions, ChatFormat::OpenAIResponses][..],
		),
		("gpt-5", &[ChatFormat::OpenAIResponses][..]),
		("gemini-2.5-pro", &[ChatFormat::OpenAICompletions][..]),
		("mai-ds-r1", &[ChatFormat::OpenAIResponses][..]),
		("unknown-model", &[ChatFormat::OpenAICompletions][..]),
		("GPT-5", &[ChatFormat::OpenAICompletions][..]),
	] {
		assert_eq!(
			copilot::Provider::supported_formats_for_model(Some(model)).as_slice(),
			expected,
			"{model}"
		);
	}
}

#[test]
fn copilot_claude_surface_routes_stay_native() {
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let model = Some("Claude-Sonnet-4");

	for input in [
		InputFormat::Messages,
		InputFormat::Responses,
		InputFormat::Completions,
	] {
		assert_eq!(
			provider.chat_translation(input, model).unwrap().output,
			ChatFormat::AnthropicMessages,
			"{input:?}"
		);
	}
}

#[test]
fn responses_to_messages_routing_is_copilot_only() {
	let providers = [
		(
			"Anthropic",
			AIProvider::Anthropic(anthropic::Provider { model: None }),
			"claude-sonnet-4-5",
		),
		(
			"Azure Foundry",
			AIProvider::azure(azure::Provider {
				model: None,
				resource_name: strng::new("example"),
				resource_type: azure::AzureResourceType::Foundry,
				api_version: None,
				project_name: Some(strng::new("project")),
			}),
			"claude-sonnet-4-5",
		),
		(
			"Vertex",
			vertex_provider("anthropic/claude-sonnet-4-5"),
			"anthropic/claude-sonnet-4-5",
		),
		(
			"custom Messages",
			custom_provider(custom::ProviderFormat::Messages),
			"claude-sonnet-4-5",
		),
	];

	for (name, provider, model) in providers {
		assert!(
			provider
				.chat_translation(InputFormat::Responses, Some(model))
				.is_err(),
			"{name} unexpectedly enabled Responses-to-Messages routing"
		);
	}
}

#[test]
fn copilot_claude_responses_buffered_renderer() {
	let request: types::responses::Request = serde_json::from_value(json!({
		"input": "run a command",
		"model": "claude-sonnet-4-5",
		"store": false,
		"tools": [{"type": "shell", "environment": {"type": "local"}}]
	}))
	.expect("valid Responses request");
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let rendered = ChatTranslation {
		input: InputFormat::Responses,
		output: ChatFormat::AnthropicMessages,
	}
	.render_request(
		types::ChatRequest::Responses(&request),
		&ChatRequestContext {
			provider: &provider,
			headers: &HeaderMap::new(),
			prompt_caching: None,
		},
	)
	.expect("Responses request should render as Messages");
	let Some(ProviderState::ResponsesToMessages { state }) = rendered.provider_state else {
		panic!("expected Responses-to-Messages state");
	};
	let upstream = Bytes::from(
		serde_json::to_vec(&json!({
			"id":"msg_gateway",
			"type":"message",
			"role":"assistant",
			"content":[{
				"type":"tool_use",
				"id":"call_shell",
				"name":"agentgateway__responses__shell_0",
				"input":{"action":{"commands":["pwd"]}}
			}],
			"model":"claude-upstream",
			"stop_reason":"tool_use",
			"stop_sequence":null,
			"usage":{"input_tokens":2,"output_tokens":1}
		}))
		.expect("upstream fixture"),
	);
	let translated = ChatTranslation {
		input: InputFormat::Responses,
		output: ChatFormat::AnthropicMessages,
	}
	.render_response(
		&upstream,
		&ChatResponseContext {
			model: "claude-sonnet-4-5",
			buffer_limit: 1024 * 1024,
			tool_name_map: None,
			responses_to_messages_state: Some(state.as_ref()),
		},
	)
	.expect("buffered response should translate");
	let value: Value = serde_json::from_slice(
		&translated
			.serialize()
			.expect("translated response should serialize"),
	)
	.expect("Responses response");

	assert_eq!(value["model"], "claude-sonnet-4-5");
	assert_eq!(value["output"][0]["type"], "shell_call");
	assert_eq!(value["output"][0]["call_id"], "call_shell");
	assert_eq!(value["output"][0]["action"]["commands"], json!(["pwd"]));
}

#[tokio::test]
async fn copilot_claude_responses_request_uses_messages_route() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.githubcopilot.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("https://api.githubcopilot.com/v1/responses")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model":"Claude-Sonnet-4-5",
				"input":"say hi",
				"max_output_tokens":64,
				"store":false
			}"#
				.to_vec(),
		))
		.expect("request");

	let RequestResult::Success {
		mut request,
		llm_request,
		upstream_route_type,
	} = provider
		.process_responses_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("Copilot Claude Responses request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(upstream_route_type, RouteType::Messages);
	assert_eq!(llm_request.request_model, "Claude-Sonnet-4-5");
	assert!(matches!(
		llm_request.provider_state,
		Some(ProviderState::ResponsesToMessages { .. })
	));
	provider
		.setup_request(
			&mut request,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("Copilot request setup");
	assert_eq!(request.uri().path(), "/v1/messages");
	assert_eq!(request.headers()["anthropic-version"], "2023-06-01");

	let forwarded = request.collect().await.expect("forwarded body").to_bytes();
	let body: Value = serde_json::from_slice(&forwarded).expect("Messages JSON");
	assert_eq!(body["model"], "Claude-Sonnet-4-5");
	assert_eq!(body["max_tokens"], 64);
	assert_eq!(body["messages"][0]["role"], "user");
	assert_eq!(body["messages"][0]["content"][0]["text"], "say hi");
}

#[tokio::test]
async fn copilot_claude_responses_route_rewrites_path_even_with_host_override() {
	// A host override with no explicit pathPrefix normally means "trust the client's original
	// path" -- but here the body has been converted to Anthropic Messages, so the client's
	// original /v1/responses path is wrong regardless of host override; it must still become
	// /v1/messages.
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.githubcopilot.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("https://api.githubcopilot.com/v1/responses")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model":"Claude-Sonnet-4-5",
				"input":"say hi",
				"max_output_tokens":64,
				"store":false
			}"#
				.to_vec(),
		))
		.expect("request");

	let RequestResult::Success {
		mut request,
		llm_request,
		upstream_route_type,
	} = provider
		.process_responses_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("Copilot Claude Responses request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(upstream_route_type, RouteType::Messages);
	provider
		.setup_request(
			&mut request,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			true, // has_host_override = true, no path_prefix
		)
		.expect("Copilot request setup");
	assert_eq!(request.uri().path(), "/v1/messages");
}

#[tokio::test]
async fn copilot_claude_error_responses_route_preserves_status_and_redacts_provider_data() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut req = llm_request_with_tokens(None);
	req.input_format = InputFormat::Responses;
	req.request_model = "claude-sonnet-4-5".into();
	req.provider_state = Some(ProviderState::ResponsesToMessages {
		state: Arc::new(conversion::messages::from_responses::State::default()),
	});
	let marker = "SENSITIVE_SIGNATURE_REDACTED_DATA_AND_TOOL_ARGUMENTS";
	let upstream = Bytes::from(format!(
		r#"{{"type":"error","error":{{"type":"rate_limit_error","message":"{marker}"}}}}"#
	));

	let mut upstream_response = Response::new(Body::from(upstream));
	*upstream_response.status_mut() = ::http::StatusCode::TOO_MANY_REQUESTS;
	upstream_response.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"application/json".parse().expect("content type"),
	);
	let translated = provider
		.process_response(
			PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
			req,
			LLMResponsePolicies::default(),
			None,
			AsyncLog::default(),
			false,
			None,
			upstream_response,
		)
		.await
		.expect("Copilot Claude Responses error should translate");
	assert_eq!(translated.status(), ::http::StatusCode::TOO_MANY_REQUESTS);
	let translated = translated
		.collect()
		.await
		.expect("translated body")
		.to_bytes();
	let body: Value = serde_json::from_slice(&translated).expect("Responses error JSON");
	assert_eq!(body["error"]["type"], "rate_limit_error");
	assert_eq!(
		body["error"]["message"],
		"Upstream Anthropic request failed with HTTP 429"
	);
	assert!(!String::from_utf8_lossy(&translated).contains(marker));
}

#[tokio::test]
async fn copilot_claude_responses_stream_process_response_restores_wrapped_tool() {
	use crate::http::auth::BackendInfo;
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.githubcopilot.com", 443)),
		inputs,
	};
	let request = ::http::Request::builder()
		.uri("https://api.githubcopilot.com/v1/responses")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
			"model":"claude-sonnet-4-5", "input":"run pwd", "stream":true, "store":false,
			"stream_options":{"include_obfuscation":false},
			"tools":[{"type":"shell","environment":{"type":"local"}}]
		}"#
				.to_vec(),
		))
		.unwrap();
	let RequestResult::Success {
		llm_request,
		upstream_route_type,
		..
	} = provider
		.process_responses_request(&backend_info, None, request, false, &mut None)
		.await
		.expect("request translation")
	else {
		panic!("expected forwarded request")
	};
	assert_eq!(upstream_route_type, RouteType::Messages);
	assert!(matches!(
		llm_request.provider_state,
		Some(ProviderState::ResponsesToMessages { .. })
	));

	let upstream = [
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_gateway\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_shell\",\"name\":\"agentgateway__responses__shell_0\",\"input\":{}}}\n\n",
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"action\\\":{\\\"commands\\\":[\\\"pwd\\\"]}}\"}}\n\n",
		"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
		"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	]
	.concat();
	let mut response = Response::new(Body::from(upstream));
	response.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"text/event-stream".parse().unwrap(),
	);
	let translated = provider
		.process_response(
			PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
			llm_request,
			LLMResponsePolicies::default(),
			None,
			AsyncLog::default(),
			false,
			None,
			response,
		)
		.await
		.expect("composed streaming response");
	let body = translated
		.collect()
		.await
		.expect("translated stream")
		.to_bytes();
	let text = String::from_utf8(body.to_vec()).unwrap();
	let terminal: Value = text
		.split("\n\n")
		.filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("data: ")))
		.map(|data| serde_json::from_str::<Value>(data).unwrap())
		.find(|event| event["type"] == "response.completed")
		.expect("completed event");
	assert_eq!(terminal["response"]["output"][0]["type"], "shell_call");
	assert_eq!(terminal["response"]["output"][0]["call_id"], "call_shell");
	assert_eq!(
		terminal["response"]["output"][0]["action"]["commands"],
		json!(["pwd"])
	);
}

#[tokio::test]
async fn copilot_claude_responses_stream_missing_state_returns_sanitized_error() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut req = llm_request_with_tokens(None);
	req.input_format = InputFormat::Responses;
	req.request_model = "claude-sonnet-4-5".into();
	req.provider_state = None;
	let marker = "SENSITIVE_UPSTREAM_STREAM_BODY";
	let mut response = Response::new(Body::from(marker));
	response.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"text/event-stream".parse().expect("content type"),
	);

	let result = provider.process_streaming(
		PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
		req,
		LLMResponsePolicies::default(),
		None,
		AsyncLog::default(),
		false,
		None,
		response,
	);
	let Err(error) = result else {
		panic!("missing conversion state must fail")
	};
	let message = error.to_string();
	assert_eq!(
		message,
		"unsupported conversion: missing Responses-to-Messages state"
	);
	assert!(!message.contains(marker));
}

#[test]
fn native_messages_errors_preserve_valid_bodies_and_normalize_invalid_bodies() {
	let valid =
		Bytes::from_static(br#"{"type":"error","error":{"type":"api_error","message":"upstream"}}"#);
	let translation = ChatTranslation {
		input: InputFormat::Messages,
		output: ChatFormat::AnthropicMessages,
	};
	assert_eq!(
		translation
			.error(
				&valid,
				::http::StatusCode::BAD_GATEWAY,
				ChatErrorFormat::Anthropic,
			)
			.expect("valid native Messages error"),
		valid
	);

	let upstream = Bytes::from_static(b"native Messages provider body");
	let translated = ChatTranslation {
		input: InputFormat::Messages,
		output: ChatFormat::AnthropicMessages,
	}
	.error(
		&upstream,
		::http::StatusCode::BAD_GATEWAY,
		ChatErrorFormat::Anthropic,
	)
	.expect("invalid native Messages error should normalize");
	let value: Value = serde_json::from_slice(&translated).expect("Anthropic error JSON");
	assert_eq!(value["type"], "error");
	assert_eq!(value["error"]["type"], "api_error");
	assert_eq!(value["error"]["message"], "native Messages provider body");
}

#[test]
fn response_prompt_guard_headers_copies_request_traceparent() {
	let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
		.parse()
		.unwrap();
	let mut response_headers = ::http::HeaderMap::new();
	response_headers.insert("x-upstream", "value".parse().unwrap());

	let headers = response_prompt_guard_headers(&response_headers, Some(&traceparent));

	assert_eq!(headers.get("x-upstream").unwrap(), "value");
	assert_eq!(
		headers.get(TRACEPARENT).unwrap(),
		"00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
	);
	assert!(!response_headers.contains_key(TRACEPARENT));
}

#[test]
fn response_prompt_guard_headers_overwrites_upstream_traceparent() {
	let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
		.parse()
		.unwrap();
	let mut response_headers = ::http::HeaderMap::new();
	response_headers.insert(
		TRACEPARENT,
		"00-11111111111111111111111111111111-2222222222222222-01"
			.parse()
			.unwrap(),
	);

	let headers = response_prompt_guard_headers(&response_headers, Some(&traceparent));

	assert_eq!(
		headers.get(TRACEPARENT).unwrap(),
		"00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
	);
	assert_eq!(
		response_headers.get(TRACEPARENT).unwrap(),
		"00-11111111111111111111111111111111-2222222222222222-01"
	);
}

#[tokio::test]
async fn test_passthrough() {
	let input_path = fixture_path("requests/completions/full.json");
	let openai_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let openai_raw: Value = serde_json::from_str(openai_str).expect("Failed to parse input json");
	let openai: types::completions::Request =
		serde_json::from_str(openai_str).expect("Failed to parse input JSON");
	let t = serde_json::to_string_pretty(&openai).unwrap();
	let t2 = serde_json::to_string_pretty(&openai_raw).unwrap();
	assert_eq!(
		serde_json::from_str::<Value>(&t).unwrap(),
		serde_json::from_str::<Value>(&t2).unwrap(),
		"{t}\n{t2}"
	);
}

#[tokio::test]
async fn openai_provider_normalizes_max_tokens_before_forwarding() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.openai.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "gpt-5.4",
				"max_tokens": 1024,
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		..
	} = provider
		.process_completions_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("OpenAI completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert!(forwarded_json.get("max_tokens").is_none());
	assert_eq!(forwarded_json["max_completion_tokens"], json!(1024));
	assert_eq!(llm_request.params.max_tokens, Some(1024));
}

#[tokio::test]
async fn openai_provider_normalizes_max_tokens_after_model_alias() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.openai.com", 443)),
		inputs,
	};
	let policy = Policy {
		model_aliases: std::collections::HashMap::from([(
			strng::new("fast-model"),
			strng::new("gpt-5.4"),
		)]),
		..Default::default()
	};
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "fast-model",
				"max_tokens": 1024,
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		..
	} = provider
		.process_completions_request(&backend_info, Some(&policy), req, false, &mut None)
		.await
		.expect("OpenAI completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(forwarded_json["model"], json!("gpt-5.4"));
	assert!(forwarded_json.get("max_tokens").is_none());
	assert_eq!(forwarded_json["max_completion_tokens"], json!(1024));
	assert_eq!(llm_request.request_model, "gpt-5.4");
	assert_eq!(llm_request.params.max_tokens, Some(1024));
}

#[tokio::test]
async fn openai_provider_preserves_max_tokens_for_non_gpt_models() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("localhost", 11434)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "llama3.1",
				"max_tokens": 1024,
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		..
	} = provider
		.process_completions_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("OpenAI-compatible completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(forwarded_json["max_tokens"], json!(1024));
	assert!(forwarded_json.get("max_completion_tokens").is_none());
	assert_eq!(llm_request.params.max_tokens, Some(1024));
}

#[tokio::test]
async fn count_tokens_resolves_model_alias_once_for_upstream_request() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Anthropic(anthropic::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.anthropic.com", 443)),
		inputs,
	};
	let policy = Policy {
		model_aliases: std::collections::HashMap::from([
			(strng::new("short-name"), strng::new("middle-name")),
			(strng::new("middle-name"), strng::new("final-name")),
		]),
		..Default::default()
	};
	let req = ::http::Request::builder()
		.uri("/v1/messages/count_tokens")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "short-name",
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		..
	} = provider
		.process_count_tokens_request(&backend_info, req, Some(&policy), &mut None)
		.await
		.expect("count_tokens request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(forwarded_json["model"], json!("middle-name"));
	assert_eq!(llm_request.request_model, "middle-name");
}

#[tokio::test]
async fn copilot_count_tokens_uses_local_fallback() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.githubcopilot.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("/v1/messages/count_tokens")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "claude-sonnet-5",
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Rejected(response) = provider
		.process_count_tokens_request(&backend_info, req, None, &mut None)
		.await
		.expect("Copilot count_tokens request should process")
	else {
		panic!("expected local response");
	};

	assert_eq!(response.status(), ::http::StatusCode::OK);
	let body = response.into_body().collect().await.unwrap().to_bytes();
	let response: types::count_tokens::Response =
		serde_json::from_slice(&body).expect("valid count_tokens response");
	assert!(response.input_tokens > 0);
}

#[tokio::test]
async fn count_tokens_uses_native_endpoint_after_model_alias() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Vertex(vertex::Provider {
		model: None,
		region: None,
		project_id: strng::new("test-project"),
	});
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("us-central1-aiplatform.googleapis.com", 443)),
		inputs,
	};
	let policy = Policy {
		model_aliases: std::collections::HashMap::from([(
			strng::new("short-name"),
			strng::new("claude-3-5-sonnet"),
		)]),
		..Default::default()
	};
	let req = ::http::Request::builder()
		.uri("/v1/messages/count_tokens")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "short-name",
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		upstream_route_type,
		..
	} = provider
		.process_count_tokens_request(&backend_info, req, Some(&policy), &mut None)
		.await
		.expect("count_tokens request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(upstream_route_type, RouteType::AnthropicTokenCount);
	assert_eq!(forwarded_json["model"], json!("claude-3-5-sonnet"));
	assert_eq!(llm_request.request_model, "claude-3-5-sonnet");
}

#[tokio::test]
async fn vertex_anthropic_messages_prepares_vertex_body() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Vertex(vertex::Provider {
		model: None,
		region: Some(strng::new("us-central1")),
		project_id: strng::new("test-project"),
	});
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("us-central1-aiplatform.googleapis.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("/v1/messages")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "claude-haiku-4-5-20251001",
				"max_tokens": 64,
				"messages": [{"role": "user", "content": "say hi"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		upstream_route_type,
		..
	} = provider
		.process_messages_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("Vertex Anthropic messages request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(upstream_route_type, RouteType::Messages);
	assert!(forwarded_json.get("model").is_none());
	assert_eq!(
		forwarded_json["anthropic_version"],
		json!("vertex-2023-10-16")
	);
}

#[tokio::test]
async fn provider_model_is_set_before_llm_transformations() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::OpenAI(openai::Provider {
		model: Some("gcp/failover-model".into()),
	});
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.openai.com", 443)),
		inputs,
	};
	let policy = Policy {
		transformations: Some(
			[(
				"model".to_string(),
				std::sync::Arc::new(
					crate::cel::Expression::new_strict(r#"llmRequest.model.stripPrefix("gcp/")"#).unwrap(),
				),
			)]
			.into_iter()
			.collect(),
		),
		..Default::default()
	};
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "public-model",
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		..
	} = provider
		.process_completions_request(&backend_info, Some(&policy), req, false, &mut None)
		.await
		.expect("OpenAI completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(forwarded_json["model"], json!("failover-model"));
	assert_eq!(llm_request.request_model, "failover-model");
}

#[tokio::test]
async fn llm_transformations_can_set_missing_model() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.openai.com", 443)),
		inputs,
	};
	let policy = Policy {
		transformations: Some(
			[(
				"model".to_string(),
				std::sync::Arc::new(crate::cel::Expression::new_strict(r#""transformed-model""#).unwrap()),
			)]
			.into_iter()
			.collect(),
		),
		..Default::default()
	};
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		..
	} = provider
		.process_completions_request(&backend_info, Some(&policy), req, false, &mut None)
		.await
		.expect("OpenAI completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(forwarded_json["model"], json!("transformed-model"));
	assert_eq!(llm_request.request_model, "transformed-model");
}

#[tokio::test]
async fn copilot_anthropic_model_uses_messages_route() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.githubcopilot.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("/v1/messages")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "claude-sonnet-4",
				"max_tokens": 64,
				"messages": [{"role": "user", "content": "say hi"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		upstream_route_type,
	} = provider
		.process_messages_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("Copilot Anthropic messages request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(upstream_route_type, RouteType::Messages);
	assert_eq!(
		llm_request.cache_convention,
		CacheTokenConvention::InputExcludesCache
	);

	let mut setup_req =
		crate::http::tests_common::request("https://example.com/v1/messages", http::Method::POST, &[]);
	setup_req
		.headers_mut()
		.insert("anthropic-version", HeaderValue::from_static("2022-01-01"));
	provider
		.setup_request(
			&mut setup_req,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	assert_eq!(setup_req.uri().path(), "/v1/messages");
	assert_eq!(setup_req.headers()["anthropic-version"], "2023-06-01");

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert_eq!(forwarded_json["model"], json!("claude-sonnet-4"));
	assert_eq!(forwarded_json["max_tokens"], json!(64));
}

#[test]
fn copilot_non_messages_preserves_anthropic_version() {
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut req = crate::http::tests_common::request(
		"https://example.com/chat/completions",
		http::Method::POST,
		&[],
	);
	req
		.headers_mut()
		.insert("anthropic-version", HeaderValue::from_static("2022-01-01"));

	provider
		.set_required_fields(&mut req, RouteType::Completions, None)
		.expect("set_required_fields should succeed");

	assert_eq!(req.headers()["anthropic-version"], "2022-01-01");
}

// Captured verbatim from a real Claude Code 2.1.217 invocation against a Copilot-backed
// Anthropic Messages alias (primary Sonnet 5 call). advisor-tool-2026-03-01 is the only entry
// Copilot has confirmed it rejects; every other entry must survive untouched.
const CLAUDE_CODE_2_1_217_BETA_HEADER: &str = "claude-code-20250219,interleaved-thinking-2025-05-14,thinking-token-count-2026-05-13,context-management-2025-06-27,prompt-caching-scope-2026-01-05,mid-conversation-system-2026-04-07,advisor-tool-2026-03-01,effort-2025-11-24";

#[test]
fn copilot_messages_strips_context_management_and_unsupported_beta_header() {
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut req =
		crate::http::tests_common::request("https://example.com/v1/messages", http::Method::POST, &[]);
	req.headers_mut().insert(
		"anthropic-beta",
		HeaderValue::from_static(CLAUDE_CODE_2_1_217_BETA_HEADER),
	);

	provider
		.set_required_fields(&mut req, RouteType::Messages, None)
		.expect("set_required_fields should succeed");

	assert_eq!(
		req.headers()["anthropic-beta"],
		"claude-code-20250219,interleaved-thinking-2025-05-14,thinking-token-count-2026-05-13,context-management-2025-06-27,prompt-caching-scope-2026-01-05,mid-conversation-system-2026-04-07,effort-2025-11-24"
	);
}

#[test]
fn copilot_messages_beta_header_filter_handles_repeated_headers() {
	let mut headers = HeaderMap::new();
	headers.append(
		"anthropic-beta",
		HeaderValue::from_static("advisor-tool-2026-03-01"),
	);
	headers.append(
		"anthropic-beta",
		HeaderValue::from_static("claude-code-20250219, effort-2025-11-24"),
	);

	filter_copilot_unsupported_beta_headers(&mut headers);

	let values: Vec<_> = headers
		.get_all("anthropic-beta")
		.iter()
		.map(|v| v.to_str().unwrap())
		.collect();
	assert_eq!(values, vec!["claude-code-20250219,effort-2025-11-24"]);
}

#[test]
fn copilot_messages_beta_header_filter_removes_header_entirely_when_nothing_survives() {
	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		HeaderValue::from_static("advisor-tool-2026-03-01"),
	);

	filter_copilot_unsupported_beta_headers(&mut headers);

	assert!(!headers.contains_key("anthropic-beta"));
}

#[tokio::test]
async fn copilot_messages_request_body_omits_context_management_field() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.githubcopilot.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("/v1/messages")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.header("anthropic-beta", CLAUDE_CODE_2_1_217_BETA_HEADER)
		.body(Body::from(
			br#"{
				"model": "claude-sonnet-4",
				"max_tokens": 64,
				"stream": true,
				"messages": [{"role": "user", "content": "say hi"}],
				"context_management": {"edits": [{"type": "clear_tool_uses_20250919"}]},
				"some_future_anthropic_field": "should-remain"
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		upstream_route_type,
	} = provider
		.process_messages_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("Copilot Anthropic messages request should process")
	else {
		panic!("expected forwarded request");
	};

	let mut setup_req =
		crate::http::tests_common::request("https://example.com/v1/messages", http::Method::POST, &[]);
	setup_req.headers_mut().insert(
		"anthropic-beta",
		HeaderValue::from_static(CLAUDE_CODE_2_1_217_BETA_HEADER),
	);
	provider
		.setup_request(
			&mut setup_req,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	assert!(
		!setup_req.headers()["anthropic-beta"]
			.to_str()
			.unwrap()
			.contains("advisor-tool-2026-03-01")
	);

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert!(forwarded_json.get("context_management").is_none());
	assert_eq!(forwarded_json["model"], json!("claude-sonnet-4"));
	assert_eq!(forwarded_json["max_tokens"], json!(64));
	assert_eq!(forwarded_json["stream"], json!(true));
	assert_eq!(
		forwarded_json["some_future_anthropic_field"],
		json!("should-remain")
	);
}

#[tokio::test]
async fn non_copilot_messages_request_preserves_context_management_field() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Anthropic(anthropic::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("api.anthropic.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("/v1/messages")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.header("anthropic-beta", CLAUDE_CODE_2_1_217_BETA_HEADER)
		.body(Body::from(
			br#"{
				"model": "claude-sonnet-4",
				"max_tokens": 64,
				"messages": [{"role": "user", "content": "say hi"}],
				"context_management": {"edits": [{"type": "clear_tool_uses_20250919"}]}
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		upstream_route_type,
	} = provider
		.process_messages_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("Anthropic messages request should process")
	else {
		panic!("expected forwarded request");
	};

	let mut setup_req =
		crate::http::tests_common::request("https://example.com/v1/messages", http::Method::POST, &[]);
	setup_req.headers_mut().insert(
		"anthropic-beta",
		HeaderValue::from_static(CLAUDE_CODE_2_1_217_BETA_HEADER),
	);
	provider
		.setup_request(
			&mut setup_req,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	assert_eq!(
		setup_req.headers()["anthropic-beta"],
		CLAUDE_CODE_2_1_217_BETA_HEADER
	);

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert_eq!(
		forwarded_json["context_management"],
		json!({"edits": [{"type": "clear_tool_uses_20250919"}]})
	);
}

#[test]
fn openai_token_limit_normalization_keeps_explicit_max_completion_tokens() {
	let mut request: types::completions::Request = serde_json::from_value(json!({
		"model": "gpt-5.4",
		"max_tokens": 1024,
		"max_completion_tokens": 2048,
		"messages": [{"role": "user", "content": "hello"}]
	}))
	.expect("valid completions request");

	request.normalize_openai_token_limit();

	assert_eq!(request.max_tokens, None);
	assert_eq!(request.max_completion_tokens, Some(2048));
}

#[test]
fn test_adaptive_thinking_without_effort_maps_to_high_reasoning_effort() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"max_tokens": 256,
		"thinking": {
			"type": "adaptive"
		},
		"messages": [
			{
				"role": "user",
				"content": "Give one concise insight."
			}
		]
	}))
	.expect("valid messages request");

	let translated = conversion::completions::from_messages::translate(&request)
		.expect("messages->completions translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(translated.get("reasoning_effort"), Some(&json!("high")));
}

#[test]
fn test_completions_reasoning_effort_maps_to_enabled_thinking_budget() {
	let request: types::completions::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"messages": [
			{ "role": "user", "content": "Give one concise insight." }
		],
		"reasoning_effort": "minimal"
	}))
	.expect("valid completions request");

	let translated = conversion::messages::from_completions::translate(&request)
		.expect("completions->messages translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(
		translated["thinking"],
		json!({
			"type": "enabled",
			"budget_tokens": 1024
		})
	);
	assert!(translated.get("output_config").is_none());
}

#[test]
fn test_completions_json_schema_response_format_maps_to_anthropic_output_config() {
	let request: types::completions::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"messages": [
			{ "role": "user", "content": "Return one short summary." }
		],
		"response_format": {
			"type": "json_schema",
			"json_schema": {
				"name": "summary_schema",
				"schema": {
					"type": "object",
					"properties": { "summary": { "type": "string" } },
					"required": ["summary"],
					"additionalProperties": false
				}
			}
		}
	}))
	.expect("valid completions request");

	let translated = conversion::messages::from_completions::translate(&request)
		.expect("completions->messages translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(
		translated["output_config"]["format"],
		json!({
			"type": "json_schema",
			"schema": {
				"type": "object",
				"properties": { "summary": { "type": "string" } },
				"required": ["summary"],
				"additionalProperties": false
			}
		})
	);
}

#[test]
fn test_messages_output_config_format_maps_to_openai_response_format() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"max_tokens": 256,
		"output_config": {
			"format": {
				"type": "json_schema",
				"schema": {
					"type": "object",
					"properties": { "answer": { "type": "number" } },
					"required": ["answer"],
					"additionalProperties": false
				}
			}
		},
		"messages": [
			{
				"role": "user",
				"content": "What is 2+2?"
			}
		]
	}))
	.expect("valid messages request");

	let translated = conversion::completions::from_messages::translate(&request)
		.expect("messages->completions translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(translated["response_format"]["type"], json!("json_schema"));
	assert_eq!(
		translated["response_format"]["json_schema"]["name"],
		json!("structured_output")
	);
	assert_eq!(
		translated["response_format"]["json_schema"]["schema"],
		json!({
			"type": "object",
			"properties": { "answer": { "type": "number" } },
			"required": ["answer"],
			"additionalProperties": false
		})
	);
}

/// Verifies that `process_response` routes a non-success response through
/// the buffered error path even when the request has `streaming: true`.
///
/// Constructs a Bedrock 400 JSON error response and passes it through
/// `process_response` with a streaming `LLMRequest`. Asserts the returned
/// body is non-empty, valid JSON, and preserves the original error message.
#[tokio::test]
async fn process_response_routes_streaming_error_to_buffered_path() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let bedrock = AIProvider::bedrock(bedrock::Provider {
		model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	});

	let error_json = r#"{"message":"Expected toolResult blocks at messages.2.content for the following Ids: tooluse_abc123"}"#;

	let req = LLMRequest {
		input_tokens: None,
		input_format: InputFormat::Completions,
		cache_convention: CacheTokenConvention::pending(),
		request_model: "input-model".into(),
		provider: Default::default(),
		streaming: true,
		params: Default::default(),
		prompt: None,
		provider_state: None,
	};

	let body = Body::from(error_json.as_bytes().to_vec());
	let mut resp = Response::new(body);
	*resp.status_mut() = ::http::StatusCode::BAD_REQUEST;
	resp.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"application/json".parse().unwrap(),
	);

	let client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi);

	let result = bedrock
		.process_response(
			client,
			req,
			LLMResponsePolicies::default(),
			None,
			AsyncLog::default(),
			false,
			None,
			resp,
		)
		.await
		.expect("process_response should succeed for error responses");

	assert_eq!(result.status(), ::http::StatusCode::BAD_REQUEST);

	let result_body = result.collect().await.unwrap().to_bytes();
	assert!(
		!result_body.is_empty(),
		"error response body must not be empty",
	);

	let parsed: Value =
		serde_json::from_slice(&result_body).expect("translated error should be valid JSON");

	let message = parsed
		.pointer("/error/message")
		.and_then(|v| v.as_str())
		.unwrap_or_default();
	assert!(
		message.contains("toolResult"),
		"translated error should preserve the original message, got: {message}",
	);
}

#[test]
fn openai_completions_error_translates_to_messages_client() {
	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let mut req = llm_request_with_tokens(None);
	req.input_format = InputFormat::Messages;
	req.request_model = "gpt-4o".into();

	let error = Bytes::from_static(
		br#"{"error":{"message":"bad request","type":"invalid_request_error","param":null,"code":null}}"#,
	);
	let translated = provider
		.process_error(&req, ::http::StatusCode::BAD_REQUEST, &error)
		.expect("OpenAI error should translate to messages error");
	let body: Value = serde_json::from_slice(&translated).expect("translated error should be JSON");

	assert_eq!(body["type"], json!("error"));
	assert_eq!(body["error"]["type"], json!("invalid_request_error"));
	assert_eq!(body["error"]["message"], json!("bad request"));
}

#[test]
fn custom_messages_error_translates_to_completions_client() {
	let provider = custom_provider(custom::ProviderFormat::Messages);
	let mut req = llm_request_with_tokens(None);
	req.input_format = InputFormat::Completions;
	req.request_model = "claude-test".into();

	let error = Bytes::from_static(
		br#"{"type":"error","error":{"type":"invalid_request_error","message":"bad request"}}"#,
	);
	let translated = provider
		.process_error(&req, ::http::StatusCode::BAD_REQUEST, &error)
		.expect("Anthropic error should translate to completions error");
	let body: Value = serde_json::from_slice(&translated).expect("translated error should be JSON");

	assert_eq!(body["error"]["type"], json!("invalid_request_error"));
	assert_eq!(body["error"]["message"], json!("bad request"));
}

#[test]
fn foundry_claude_messages_error_uses_anthropic_shape() {
	let provider = AIProvider::azure(azure::Provider {
		model: None,
		resource_name: strng::new("example"),
		resource_type: azure::AzureResourceType::Foundry,
		api_version: None,
		project_name: Some(strng::new("project")),
	});
	let mut req = llm_request_with_tokens(None);
	req.input_format = InputFormat::Messages;
	req.request_model = "claude-haiku-4-5".into();

	let error = Bytes::from_static(
		br#"{"type":"error","error":{"type":"invalid_request_error","message":"bad request"}}"#,
	);
	let translated = provider
		.process_error(&req, ::http::StatusCode::BAD_REQUEST, &error)
		.expect("Foundry Claude messages error should stay Anthropic-shaped");
	let body: Value = serde_json::from_slice(&translated).expect("translated error should be JSON");

	assert_eq!(body["type"], json!("error"));
	assert_eq!(body["error"]["type"], json!("invalid_request_error"));
	assert_eq!(body["error"]["message"], json!("bad request"));
}

#[tokio::test]
async fn process_streaming_bedrock_completions_normalizes_sse_headers_and_done() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;
	let bedrock = AIProvider::bedrock(bedrock::Provider {
		model: Some(strng::new("openai.gpt-oss-120b-1:0")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	});

	let body = Body::from(
		fs::read(fixture_path("response/bedrock/basic.bin"))
			.expect("failed to read Bedrock streaming fixture"),
	);
	let mut resp = Response::new(body);
	resp.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"application/vnd.amazon.eventstream".parse().unwrap(),
	);
	resp.headers_mut().insert(
		crate::http::x_headers::X_AMZN_REQUESTID,
		"request_id".parse().unwrap(),
	);

	let client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi);
	let translated = bedrock
		.process_streaming(
			client,
			LLMRequest {
				input_tokens: None,
				input_format: InputFormat::Completions,
				cache_convention: CacheTokenConvention::pending(),
				request_model: "input-model".into(),
				provider: Default::default(),
				streaming: true,
				params: Default::default(),
				prompt: None,
				provider_state: None,
			},
			LLMResponsePolicies::default(),
			None,
			AsyncLog::default(),
			false,
			None,
			resp,
		)
		.expect("Bedrock streaming translation should succeed");

	crate::http::tests_common::assert_header(
		&translated,
		::http::header::CONTENT_TYPE,
		"text/event-stream",
	);

	let body = translated.collect().await.unwrap().to_bytes();
	let text = String::from_utf8(body.to_vec()).expect("stream should be valid UTF-8");
	assert!(
		text.ends_with("data: [DONE]\n\n"),
		"translated Bedrock completions stream must end with [DONE], got:\n{text}",
	);
	assert!(
		!text.contains("event: \n"),
		"translated Bedrock completions stream must not emit empty event fields:\n{text}",
	);
}

#[test]
fn setup_request_openai_applies_prefixed_path_without_host_override() {
	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1/messages?trace=repro",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Messages,
			None,
			None,
			Some("/v1/custom"),
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(
		req.uri().authority().map(|a| a.as_str()),
		Some("api.openai.com")
	);
	assert_eq!(req.uri().path(), "/v1/custom/chat/completions");
	assert_eq!(req.uri().query(), Some("trace=repro"));
}

#[test]
fn setup_request_openai_normalizes_trailing_slash_in_path_prefix() {
	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1/messages?trace=repro",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Messages,
			None,
			None,
			Some("/v1/custom/"),
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(req.uri().path(), "/v1/custom/chat/completions");
	assert_eq!(req.uri().query(), Some("trace=repro"));
}

#[test]
fn setup_request_custom_path_override_wins_over_format_path() {
	let provider = AIProvider::Custom(custom::Provider {
		model: None,
		provider_override: None,
		formats: vec![custom::ProviderFormatConfig {
			format: custom::ProviderFormat::Messages,
			path: Some(strng::literal!("/api/messages")),
		}],
	});
	let llm_request = LLMRequest {
		input_tokens: None,
		input_format: InputFormat::Completions,
		cache_convention: CacheTokenConvention::pending(),
		request_model: "input-model".into(),
		provider: Default::default(),
		streaming: false,
		params: Default::default(),
		prompt: None,
		provider_state: None,
	};
	let mut req = crate::http::tests_common::request(
		"https://proxy.example.com/v1/chat/completions?trace=repro",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Completions,
			Some(&llm_request),
			Some("/override/messages"),
			None,
			true,
		)
		.expect("setup_request should succeed");

	assert_eq!(req.uri().path(), "/override/messages");
	assert_eq!(req.uri().query(), None);
}

fn llm_request_for_path(request_model: &str) -> LLMRequest {
	LLMRequest {
		input_tokens: None,
		input_format: InputFormat::Messages,
		cache_convention: CacheTokenConvention::pending(),
		request_model: request_model.into(),
		provider: Default::default(),
		streaming: false,
		params: Default::default(),
		prompt: None,
		provider_state: None,
	}
}

fn assert_prefixed_host_override_path(
	provider: AIProvider,
	request_model: &str,
	expected_path: &str,
	expected_query: Option<&str>,
) {
	let llm_request = llm_request_for_path(request_model);
	let mut req = crate::http::tests_common::request(
		"https://proxy.example.com/v1/messages?trace=repro",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Messages,
			Some(&llm_request),
			None,
			Some("/proxy/"),
			true,
		)
		.expect("setup_request should succeed");

	assert_eq!(req.uri().path(), expected_path);
	assert_eq!(req.uri().query(), expected_query);
}

#[test]
fn native_copilot_messages_host_override_no_prefix_preserves_client_path() {
	// A native (unconverted) Copilot Messages request under a host override with no explicit
	// pathPrefix must keep trusting the client's own path, same as every other non-Custom
	// provider -- only a request that actually underwent Responses-to-Messages conversion
	// (ProviderState::ResponsesToMessages) needs its path forced to Copilot's canonical default.
	let llm_request = llm_request_for_path("gpt-4o");
	let mut req = crate::http::tests_common::request(
		"https://proxy.example.com/tenant/v1/messages?trace=repro",
		http::Method::POST,
		&[],
	);
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	provider
		.setup_request(
			&mut req,
			RouteType::Messages,
			Some(&llm_request),
			None,
			None,
			true,
		)
		.expect("setup_request should succeed");
	assert_eq!(req.uri().path(), "/tenant/v1/messages");
}

#[test]
fn setup_request_gemini_applies_path_prefix_with_host_override() {
	assert_prefixed_host_override_path(
		AIProvider::Gemini(gemini::Provider { model: None }),
		"gemini-2.5-pro",
		"/proxy/v1beta/openai/chat/completions",
		Some("trace=repro"),
	);
}

#[test]
fn setup_request_vertex_applies_path_prefix_with_host_override() {
	assert_prefixed_host_override_path(
		AIProvider::Vertex(vertex::Provider {
			model: None,
			region: Some(strng::new("us-central1")),
			project_id: strng::new("example-project"),
		}),
		"gemini-2.5-pro",
		"/proxy/v1/projects/example-project/locations/us-central1/endpoints/openapi/chat/completions",
		Some("trace=repro"),
	);
}

#[test]
fn setup_request_bedrock_applies_path_prefix_with_host_override() {
	assert_prefixed_host_override_path(
		AIProvider::bedrock(bedrock::Provider {
			model: None,
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
		}),
		"anthropic.claude-3-5-sonnet-20241022-v2:0",
		"/proxy/model/anthropic.claude-3-5-sonnet-20241022-v2:0/converse",
		Some("trace=repro"),
	);
}

#[test]
fn setup_request_azure_applies_path_prefix_with_host_override() {
	assert_prefixed_host_override_path(
		AIProvider::azure(azure::Provider {
			model: None,
			resource_name: strng::new("example"),
			resource_type: azure::AzureResourceType::OpenAI,
			api_version: Some(strng::new("2024-02-15-preview")),
			project_name: None,
		}),
		"gpt-4.1",
		"/proxy/openai/deployments/gpt-4.1/chat/completions",
		Some("api-version=2024-02-15-preview&trace=repro"),
	);
}

#[test]
fn completions_response_missing_message_and_usage_fields() {
	// Gemini's OpenAI-compat endpoint can omit `message` from choices and
	// `completion_tokens` from usage. Verify deserialization succeeds with defaults.
	let json = r#"{
		"id": "1",
		"object": "chat.completion",
		"created": 0,
		"model": "google/gemini-2.5-flash",
		"choices": [{"index": 0, "finish_reason": "length"}],
		"usage": {"prompt_tokens": 5, "total_tokens": 12}
	}"#;
	let resp: types::completions::Response = serde_json::from_str(json).unwrap();
	assert_eq!(resp.choices.len(), 1);
	assert_eq!(resp.choices[0].message.content, None);
	assert_eq!(resp.choices[0].message.role, None);
	let usage = resp.usage.unwrap();
	assert_eq!(usage.prompt_tokens, 5);
	assert_eq!(usage.completion_tokens, 0);
	assert_eq!(usage.total_tokens, 12);
}

#[test]
fn completions_to_messages_response_allows_missing_openai_metadata() {
	let body = Bytes::from_static(
		br#"{
			"id": "chatcmpl-1",
			"model": "gpt-5-mini",
			"choices": [{
				"message": {"role": "assistant", "content": "hi"},
				"finish_reason": "stop"
			}],
			"usage": {
				"completion_tokens": 16,
				"prompt_tokens": 9,
				"prompt_tokens_details": {"cached_tokens": 0},
				"total_tokens": 25
			},
			"copilot_usage": {
				"token_details": []
			}
		}"#,
	);

	conversion::completions::from_messages::translate_response(&body)
		.expect("messages response translation should not require OpenAI metadata");
}

#[tokio::test]
async fn bedrock_from_messages_stream_captures_completion() {
	let input_bytes =
		fs::read(fixture_path("response/bedrock/basic.bin")).expect("Failed to read fixture");
	let body = Body::from(input_bytes);
	let log = AsyncLog::default();
	let log2 = log.clone();
	let llmresp = LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "us.anthropic.claude-haiku-4-5-20251001-v1:0".into(),
			provider: "bedrock".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	};
	log.store(Some(llmresp));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let buffer_limit = 1024 * 1024;
	let body = conversion::bedrock::from_messages::translate_stream(
		body,
		buffer_limit,
		logger,
		"us.anthropic.claude-haiku-4-5-20251001-v1:0",
		"msg_123",
		true,
		None,
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	let completion = info
		.response
		.completion
		.expect("completion should be set for bedrock streaming");
	assert!(
		!completion.join("").is_empty(),
		"completion should contain response text"
	);
}

#[tokio::test]
async fn bedrock_from_messages_stream_skips_completion_when_disabled() {
	let input_bytes =
		fs::read(fixture_path("response/bedrock/basic.bin")).expect("Failed to read fixture");
	let body = Body::from(input_bytes);
	let log = AsyncLog::default();
	let log2 = log.clone();
	let llmresp = LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "us.anthropic.claude-haiku-4-5-20251001-v1:0".into(),
			provider: "bedrock".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	};
	log.store(Some(llmresp));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let buffer_limit = 1024 * 1024;
	let body = conversion::bedrock::from_messages::translate_stream(
		body,
		buffer_limit,
		logger,
		"us.anthropic.claude-haiku-4-5-20251001-v1:0",
		"msg_123",
		false,
		None,
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(
		info.response.completion.is_none(),
		"completion should not be set when include_completion_in_log is false"
	);
}

#[tokio::test]
async fn messages_passthrough_stream_captures_completion() {
	let input_path = fixture_path("response/anthropic/stream_basic.json");
	let mut input_bytes = fs::read(&input_path).expect("Failed to read fixture");
	input_bytes.extend_from_slice(b"data: [DONE]\n\n");
	let body = Body::from(input_bytes);
	let log = AsyncLog::default();
	let log2 = log.clone();
	let llmresp = LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "claude-haiku-4-5-20251001".into(),
			provider: "anthropic".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	};
	log.store(Some(llmresp));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let buffer_limit = 1024 * 1024;
	let body = conversion::messages::passthrough_stream(body, buffer_limit, logger, true, true);
	// Consume the body to drive the stream to completion
	let output = body.collect().await.unwrap().to_bytes();
	assert!(
		!output
			.windows(b"[DONE]".len())
			.any(|value| value == b"[DONE]")
	);
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	let completion = info
		.response
		.completion
		.expect("completion should be set for messages streaming");
	assert_eq!(
		completion.join(""),
		"Hi there! How are you doing today? Is there anything I can help you with?"
	);
}

#[tokio::test]
async fn messages_passthrough_stream_preserves_native_sse_bytes() {
	let input_bytes =
		fs::read(fixture_path("response/anthropic/stream_basic.json")).expect("Failed to read fixture");
	let expected = input_bytes.clone();
	let body = Body::from(input_bytes);
	let log = AsyncLog::default();
	log.store(Some(LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "claude-haiku-4-5-20251001".into(),
			provider: "anthropic".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	}));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let output = conversion::messages::passthrough_stream(body, 1024 * 1024, logger, false, false)
		.collect()
		.await
		.expect("native Messages stream")
		.to_bytes();

	assert_eq!(output.as_ref(), expected);
}

#[tokio::test]
async fn copilot_messages_done_stripping_forwards_other_sse_frames() {
	fn decode_sse(bytes: &[u8]) -> Vec<tokio_sse_codec::Frame<bytes::Bytes>> {
		let mut decoder = tokio_sse_codec::SseDecoder::<bytes::Bytes>::new();
		let mut buffer = bytes::BytesMut::from(bytes);
		let mut frames = Vec::new();
		while let Some(frame) =
			tokio_util::codec::Decoder::decode_eof(&mut decoder, &mut buffer).expect("valid SSE")
		{
			frames.push(frame);
		}
		frames
	}

	let expected = concat!(
		": keep-this-comment\r\n",
		"id: upstream-7\r\n",
		"event: message_start\r\n",
		"data:{\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\r\n",
		"\r\n",
		"event: message_stop\r\n",
		"data: {\"type\":\"message_stop\"}\r\n",
		"\r\n",
	);
	let input = format!("{expected}data: [DONE]\r\n\r\n");
	let output = conversion::messages::passthrough_stream(
		Body::from(input),
		1024 * 1024,
		agent_llm::StreamingUsageGuard::default(),
		false,
		true,
	)
	.collect()
	.await
	.expect("Copilot Messages stream")
	.to_bytes();

	assert_eq!(decode_sse(&output), decode_sse(expected.as_bytes()));
}

#[tokio::test]
async fn native_messages_stream_thinking_tokens_use_terminal_value() {
	let input = [
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"output_tokens_details\":{\"thinking_tokens\":2,\"future\":1}}}}\n\n",
		"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":5,\"output_tokens_details\":{\"thinking_tokens\":5,\"future\":2}}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	]
	.concat();
	let log = AsyncLog::default();
	let log2 = log.clone();
	log.store(Some(LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "claude-test".into(),
			provider: "anthropic".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	}));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let output = conversion::messages::passthrough_stream(
		Body::from(input.clone()),
		1024 * 1024,
		logger,
		false,
		false,
	)
	.collect()
	.await
	.expect("native Messages stream")
	.to_bytes();
	assert_eq!(output.as_ref(), input.as_bytes());
	let info = log2.take().expect("stream telemetry");
	assert_eq!(info.response.output_tokens, Some(5));
	assert_eq!(info.response.reasoning_tokens, Some(5));
}

#[tokio::test]
async fn native_messages_stream_thinking_tokens_fall_back_to_initial_value() {
	let input = [
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"output_tokens_details\":{\"thinking_tokens\":2,\"future\":1}}}}\n\n",
		"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":5}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	]
	.concat();
	let log = AsyncLog::default();
	let log2 = log.clone();
	log.store(Some(LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "claude-test".into(),
			provider: "anthropic".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	}));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let output = conversion::messages::passthrough_stream(
		Body::from(input.clone()),
		1024 * 1024,
		logger,
		false,
		false,
	)
	.collect()
	.await
	.expect("native Messages stream")
	.to_bytes();
	assert_eq!(output.as_ref(), input.as_bytes());
	let info = log2.take().expect("stream telemetry");
	assert_eq!(info.response.output_tokens, Some(5));
	assert_eq!(info.response.reasoning_tokens, Some(2));
}

#[tokio::test]
async fn messages_passthrough_stream_preserves_done_for_native_providers() {
	let input = Bytes::from_static(b"data: [DONE]\n\n");
	let body = Body::from(input.clone());
	let log = AsyncLog::default();
	log.store(Some(LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "claude-test".into(),
			provider: "custom".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	}));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let output = conversion::messages::passthrough_stream(body, 1024, logger, false, false)
		.collect()
		.await
		.expect("native Messages done marker")
		.to_bytes();

	assert_eq!(output, input);
}

#[tokio::test]
async fn messages_passthrough_stream_skips_completion_when_disabled() {
	let input_path = fixture_path("response/anthropic/stream_basic.json");
	let input_bytes = fs::read(&input_path).expect("Failed to read fixture");
	let body = Body::from(input_bytes);
	let log = AsyncLog::default();
	let log2 = log.clone();
	let llmresp = LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "claude-haiku-4-5-20251001".into(),
			provider: "anthropic".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	};
	log.store(Some(llmresp));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let buffer_limit = 1024 * 1024;
	let body = conversion::messages::passthrough_stream(body, buffer_limit, logger, false, false);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(
		info.response.completion.is_none(),
		"completion should not be set when include_completion_in_log is false"
	);
}

#[tokio::test]
async fn responses_passthrough_stream_captures_completion() {
	let input_path = fixture_path("response/responses/stream.json");
	let input_bytes = fs::read(&input_path).expect("Failed to read fixture");
	let body = Body::from(input_bytes);
	let log = AsyncLog::default();
	let log2 = log.clone();
	let llmresp = LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Responses,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "gpt-4.1-mini".into(),
			provider: "openai".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	};
	log.store(Some(llmresp));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let buffer_limit = 1024 * 1024;
	let body = conversion::responses::passthrough_stream(body, buffer_limit, logger, true);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	let completion = info
		.response
		.completion
		.expect("completion should be set for responses streaming");
	assert_eq!(completion.join(""), "Hello");
}

#[tokio::test]
async fn responses_passthrough_stream_skips_completion_when_disabled() {
	let input_path = fixture_path("response/responses/stream.json");
	let input_bytes = fs::read(&input_path).expect("Failed to read fixture");
	let body = Body::from(input_bytes);
	let log = AsyncLog::default();
	let log2 = log.clone();
	let llmresp = LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Responses,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "gpt-4.1-mini".into(),
			provider: "openai".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	};
	log.store(Some(llmresp));
	let logger = AmendOnDrop::new(log, LLMResponsePolicies::default(), None, None).into_llm();
	let buffer_limit = 1024 * 1024;
	let body = conversion::responses::passthrough_stream(body, buffer_limit, logger, false);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(
		info.response.completion.is_none(),
		"completion should not be set when include_completion_in_log is false"
	);
}

fn vertex_provider(model: &str) -> AIProvider {
	AIProvider::Vertex(vertex::Provider {
		model: Some(strng::new(model)),
		region: None,
		project_id: strng::new("test-project"),
	})
}

fn custom_provider(format: custom::ProviderFormat) -> AIProvider {
	AIProvider::Custom(custom::Provider {
		model: None,
		provider_override: None,
		formats: vec![custom::ProviderFormatConfig { format, path: None }],
	})
}

#[tokio::test]
async fn read_body_decodes_gzip_request_before_json_parse() {
	// Regression: a gzip-compressed request body (Content-Encoding: gzip) must be
	// decompressed before the JSON parse. Clients such as the Claude Code harness
	// gzip request bodies above a size threshold; previously the reader handed the
	// raw compressed bytes to serde_json and failed with a misleading
	// "LLM request body must be valid JSON" 400, even for tiny payloads.
	let provider = custom_provider(custom::ProviderFormat::Messages);

	let plaintext =
		br#"{"model":"claude-sonnet-4-5","max_tokens":8,"messages":[{"role":"user","content":"hi"}]}"#;
	let gz = crate::http::compression::encode_body(plaintext, "gzip")
		.await
		.expect("gzip encode");
	// The payload is genuinely compressed (gzip magic) and tiny, so this exercises
	// content-encoding decoding rather than the buffer-size path.
	assert_eq!(&gz[..2], &[0x1f, 0x8b]);

	let req = ::http::Request::builder()
		.uri("/v1/messages")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.header(::http::header::CONTENT_ENCODING, "gzip")
		.body(Body::from(gz.to_vec()))
		.unwrap();

	let (parts, parsed) = provider
		.read_body_and_default_model::<types::messages::Request>(None, req, &mut None)
		.await
		.expect("gzip request body should decode and parse as JSON");

	assert_eq!(parsed.model.as_deref(), Some("claude-sonnet-4-5"));
	// The encoding header is stripped now that the body is plaintext.
	assert!(
		parts
			.headers
			.get(::http::header::CONTENT_ENCODING)
			.is_none()
	);
}

#[tokio::test]
async fn read_body_still_parses_plaintext_request() {
	// A plaintext (unencoded) request body must continue to parse unchanged — the
	// decompression path is a no-op when no Content-Encoding is present.
	let provider = custom_provider(custom::ProviderFormat::Messages);

	let req = ::http::Request::builder()
		.uri("/v1/messages")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{"model":"claude-sonnet-4-5","max_tokens":8,"messages":[{"role":"user","content":"hi"}]}"#
				.to_vec(),
		))
		.unwrap();

	let (_parts, parsed) = provider
		.read_body_and_default_model::<types::messages::Request>(None, req, &mut None)
		.await
		.expect("plaintext request body should parse as JSON");

	assert_eq!(parsed.model.as_deref(), Some("claude-sonnet-4-5"));
}

#[test]
fn custom_provider_name_falls_back_to_custom() {
	let provider = custom_provider(custom::ProviderFormat::Completions);
	assert_eq!(provider.provider(), strng::literal!("custom"));
}

#[test]
fn custom_provider_override_drives_provider_name() {
	let provider = AIProvider::Custom(custom::Provider {
		model: None,
		provider_override: Some(strng::literal!("cohere")),
		formats: vec![custom::ProviderFormatConfig {
			format: custom::ProviderFormat::Rerank,
			path: None,
		}],
	});
	assert_eq!(provider.provider(), strng::literal!("cohere"));
}

#[test]
fn vertex_anthropic_model_uses_exclusive_convention() {
	let provider = vertex_provider("anthropic/claude-sonnet-4-5");
	assert_eq!(
		cache_convention_for(&provider, None, "anthropic/claude-sonnet-4-5"),
		CacheTokenConvention::InputExcludesCache,
	);
}

#[test]
fn vertex_non_anthropic_model_uses_inclusive_convention() {
	let provider = vertex_provider("gemini-2.0-flash");
	assert_eq!(
		cache_convention_for(&provider, None, "gemini-2.0-flash"),
		CacheTokenConvention::InputIncludesCache,
	);
}

#[test]
fn custom_messages_backend_uses_exclusive_convention() {
	let provider = custom_provider(custom::ProviderFormat::Messages);
	assert_eq!(
		cache_convention_for(
			&provider,
			Some(custom::ProviderFormat::Messages),
			"some-model"
		),
		CacheTokenConvention::InputExcludesCache,
	);
}

#[test]
fn custom_completions_backend_uses_inclusive_convention() {
	let provider = custom_provider(custom::ProviderFormat::Completions);
	assert_eq!(
		cache_convention_for(
			&provider,
			Some(custom::ProviderFormat::Completions),
			"some-model"
		),
		CacheTokenConvention::InputIncludesCache,
	);
}

#[test]
fn fixed_providers_classify_by_family() {
	assert_eq!(
		cache_convention_for(
			&AIProvider::Anthropic(anthropic::Provider { model: None }),
			None,
			"claude-sonnet-4-5"
		),
		CacheTokenConvention::InputExcludesCache,
	);
	assert_eq!(
		cache_convention_for(
			&AIProvider::OpenAI(openai::Provider { model: None }),
			Some(custom::ProviderFormat::Completions),
			"gpt-4o"
		),
		CacheTokenConvention::InputIncludesCache,
	);
}
