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
fn vertex_gemini_uses_native_completions_and_compat_fallbacks() {
	let provider = AIProvider::Vertex(vertex::Provider {
		project_id: strng::new("test-project"),
		model: None,
		region: None,
	});
	let model = Some("google/gemini-2.5-flash-lite");

	assert_eq!(
		provider
			.chat_translation(InputFormat::Completions, model, None)
			.unwrap()
			.output,
		ChatFormat::VertexGemini
	);
	for input in [InputFormat::Messages, InputFormat::Responses] {
		assert_eq!(
			provider
				.chat_translation(input, model, None)
				.unwrap()
				.output,
			ChatFormat::OpenAICompletions
		);
	}
}

#[test]
fn gemini_inbound_selects_native_translation_only_for_gemini_upstreams() {
	let vertex = AIProvider::Vertex(vertex::Provider {
		project_id: strng::new("test-project"),
		model: None,
		region: None,
	});
	assert_eq!(
		vertex
			.chat_translation(InputFormat::Gemini, Some("gemini-2.5-flash"), None)
			.unwrap()
			.output,
		ChatFormat::VertexGemini
	);
	// Vertex with a non-Gemini model has no Gemini-input translation.
	assert!(
		vertex
			.chat_translation(InputFormat::Gemini, Some("claude-sonnet-4-5"), None)
			.is_err()
	);

	let gemini = AIProvider::Gemini(gemini::Provider { model: None });
	assert_eq!(
		gemini
			.chat_translation(InputFormat::Gemini, Some("gemini-2.5-flash"), None)
			.unwrap()
			.output,
		ChatFormat::VertexGemini
	);
	// Completions inbound on the Gemini API provider prefers our native conversion over the
	// OpenAI-compat shim, matching Vertex with a Gemini model.
	assert_eq!(
		gemini
			.chat_translation(InputFormat::Completions, Some("gemini-2.5-flash"), None)
			.unwrap()
			.output,
		ChatFormat::VertexGemini
	);
	// Messages and Responses clients still ride the compat shim: there is no conversion from
	// those formats to native Gemini.
	for input in [InputFormat::Messages, InputFormat::Responses] {
		assert_eq!(
			gemini
				.chat_translation(input, Some("gemini-2.5-flash"), None)
				.unwrap()
				.output,
			ChatFormat::OpenAICompletions
		);
	}
}

#[test]
fn gemini_inbound_to_non_gemini_upstream_is_unsupported() {
	let anthropic = AIProvider::Anthropic(anthropic::Provider { model: None });
	let Err(err) = anthropic.chat_translation(InputFormat::Gemini, Some("claude-opus-4"), None)
	else {
		panic!("expected unsupported conversion");
	};
	assert!(matches!(err, AIError::UnsupportedConversion(_)));
	let msg = err.to_string();
	assert!(msg.contains("Gemini") && msg.contains("anthropic"), "{msg}");

	let vertex = AIProvider::Vertex(vertex::Provider {
		project_id: strng::new("test-project"),
		model: None,
		region: None,
	});
	let Err(err) = vertex.chat_translation(InputFormat::Gemini, Some("claude-sonnet-4-5"), None)
	else {
		panic!("expected unsupported conversion");
	};
	let msg = err.to_string();
	assert!(msg.contains("Gemini") && msg.contains("vertex"), "{msg}");
}

#[test]
fn custom_provider_generate_content_advertises_the_native_chat_format() {
	let provider = custom_provider(custom::ProviderFormat::GenerateContent);

	// Native Gemini input takes the direct passthrough.
	assert_eq!(
		provider
			.chat_translation(InputFormat::Gemini, Some("gemini-2.5-flash"), None)
			.unwrap()
			.output,
		ChatFormat::VertexGemini
	);
	// Completions input prefers our native conversion over the compat shim, exactly like
	// Vertex with a Gemini model (the CHAT_TRANSLATIONS quirk).
	assert_eq!(
		provider
			.chat_translation(InputFormat::Completions, Some("gemini-2.5-flash"), None)
			.unwrap()
			.output,
		ChatFormat::VertexGemini
	);

	// A custom provider that does not declare the format has no Gemini-input translation.
	let undeclared = custom_provider(custom::ProviderFormat::Completions);
	assert!(
		undeclared
			.chat_translation(InputFormat::Gemini, Some("gemini-2.5-flash"), None)
			.is_err()
	);
}

#[tokio::test]
async fn custom_provider_completions_inbound_renders_native_gemini() {
	// With generateContent declared, the CHAT_TRANSLATIONS quirk applies to custom providers
	// too: OpenAI-compat input converts to the native request, and the conversion must not
	// assume a Vertex provider.
	let provider = custom_provider(custom::ProviderFormat::GenerateContent);
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{"model": "gemini-2.5-flash", "messages": [{"role": "user", "content": "hello"}]}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: mut forwarded,
		llm_request,
		upstream_route_type,
	} = provider
		.process_completions_request(
			&openai_test_backend_info(),
			None,
			req,
			false,
			&mut None,
			None,
		)
		.await
		.expect("completions request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(upstream_route_type, RouteType::GenerateContent);
	assert!(matches!(
		llm_request.provider_state,
		Some(ProviderState::VertexGemini)
	));

	provider
		.setup_request(
			&mut forwarded,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	assert_eq!(
		forwarded.uri().path(),
		"/v1beta/models/gemini-2.5-flash:generateContent"
	);

	let body = forwarded.into_body().collect().await.unwrap().to_bytes();
	let json: Value = serde_json::from_slice(&body).expect("forwarded body should be JSON");
	assert!(json.get("contents").is_some(), "{json}");
	assert!(json.get("messages").is_none(), "{json}");
}

#[test]
fn custom_provider_declaring_gemini_count_tokens_renders_passthrough() {
	// countTokens has no cross-provider conversion, so the render gate must accept exactly
	// the providers that speak it natively, including a custom provider declaring it.
	let req: types::gemini::CountTokensRequest =
		serde_json::from_value(json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}))
			.unwrap();

	let provider = custom_provider(custom::ProviderFormat::GeminiCountTokens);
	let body = provider
		.render_gemini_count_tokens_request(&req, "gemini-2.5-flash")
		.expect("declared format renders passthrough");
	let json: Value = serde_json::from_slice(&body).unwrap();
	assert!(json.get("contents").is_some(), "{json}");

	let undeclared = custom_provider(custom::ProviderFormat::Completions);
	assert!(
		undeclared
			.render_gemini_count_tokens_request(&req, "gemini-2.5-flash")
			.is_err()
	);
}

#[test]
fn gemini_render_is_passthrough_with_unknown_fields() {
	let provider = AIProvider::Vertex(vertex::Provider {
		project_id: strng::new("test-project"),
		model: None,
		region: None,
	});
	let translation = provider
		.chat_translation(InputFormat::Gemini, Some("gemini-2.5-flash"), None)
		.unwrap();

	let raw = json!({
		"contents": [{
			"role": "user",
			"parts": [
				{ "text": "describe this", "someNewPartField": 1 },
				{ "inlineData": { "mimeType": "image/png", "data": "AAAA" } }
			],
			"someNewContentField": true
		}],
		"systemInstruction": { "parts": [{ "text": "be brief" }] },
		"tools": [
			{ "functionDeclarations": [{
				"name": "get_weather",
				"parameters": { "type": "object" },
				"behavior": "BLOCKING"
			}] },
			{ "googleSearch": {} }
		],
		"toolConfig": { "functionCallingConfig": { "mode": "AUTO", "someNewKnob": 1 } },
		"generationConfig": {
			"temperature": 0.5,
			"thinkingConfig": { "thinkingLevel": "high", "someNewField": true },
			"responseModalities": ["TEXT"]
		},
		"safetySettings": [{
			"category": "HARM_CATEGORY_HATE_SPEECH",
			"threshold": "BLOCK_NONE",
			"someNewField": 1
		}],
		"modelArmorConfig": { "promptTemplateName": "projects/p/locations/l/templates/t" }
	});
	let inner: types::gemini::GenerateContentRequest =
		serde_json::from_value(raw.clone()).expect("valid request");
	let rendered = translation
		.render_request(
			types::ChatRequest::Gemini(inner),
			&ChatRequestContext {
				catalog: None,
				provider: &provider,
				headers: &HeaderMap::new(),
				prompt_caching: None,
			},
		)
		.expect("render");
	assert!(matches!(
		rendered.provider_state,
		Some(ProviderState::VertexGemini)
	));
	let out: Value = serde_json::from_slice(&rendered.body).expect("valid body");
	assert_eq!(
		out, raw,
		"render must pass unknown fields through untouched"
	);
}

#[test]
fn gemini_error_passes_google_shape_through() {
	let provider = AIProvider::Vertex(vertex::Provider {
		project_id: strng::new("test-project"),
		model: None,
		region: None,
	});
	let translation = provider
		.chat_translation(InputFormat::Gemini, Some("gemini-2.5-flash"), None)
		.unwrap();
	assert!(matches!(
		provider.chat_error_format(translation, Some("gemini-2.5-flash")),
		ChatErrorFormat::Google
	));

	let body = bytes::Bytes::from_static(
		br#"{"error":{"code":400,"message":"bad request","status":"INVALID_ARGUMENT"}}"#,
	);
	let out = translation
		.error(
			&body,
			::http::StatusCode::BAD_REQUEST,
			ChatErrorFormat::Google,
		)
		.expect("error translation");
	assert_eq!(out, body);
}

#[test]
fn strip_alt_query_removes_only_alt() {
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1beta/models/m:streamGenerateContent?alt=sse&key=abc",
		http::Method::POST,
		&[],
	);
	strip_alt_query(&mut req);
	assert_eq!(req.uri().query(), Some("key=abc"));

	let mut req = crate::http::tests_common::request(
		"https://example.com/v1beta/models/m:streamGenerateContent?alt=sse",
		http::Method::POST,
		&[],
	);
	strip_alt_query(&mut req);
	assert_eq!(req.uri().query(), None);

	let mut req = crate::http::tests_common::request(
		"https://example.com/v1beta/models/m:generateContent?key=abc",
		http::Method::POST,
		&[],
	);
	strip_alt_query(&mut req);
	assert_eq!(req.uri().query(), Some("key=abc"));
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

#[test]
fn streaming_amend_on_drop_uses_cache_inclusive_input_tokens() {
	let rate_limit =
		crate::http::localratelimit::RateLimit::try_from(crate::http::localratelimit::RateLimitSpec {
			max_tokens: 10,
			tokens_per_fill: 10,
			fill_interval: std::time::Duration::from_secs(60),
			limit_type: crate::http::localratelimit::RateLimitType::Tokens,
		})
		.unwrap();
	let mut request = llm_request_with_tokens(Some(5));
	request.cache_convention = CacheTokenConvention::InputExcludesCache;
	let log = AsyncLog::default();
	log.store(Some(LLMInfo {
		request,
		response: LLMResponse {
			input_tokens: Some(2),
			cached_input_tokens: Some(2),
			cache_creation_input_tokens: Some(1),
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
		("GPT-5", &[ChatFormat::OpenAIResponses][..]),
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
fn responses_to_messages_routing_follows_messages_capability() {
	let providers = [
		(
			"Copilot",
			AIProvider::Copilot(copilot::Provider { model: None }),
			"claude-sonnet-4-5",
		),
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
		assert_eq!(
			provider
				.chat_translation(InputFormat::Responses, Some(model))
				.expect("Responses-to-Messages routing should be available")
				.output,
			ChatFormat::AnthropicMessages,
			"{name} did not select Responses-to-Messages routing"
		);
	}
}

#[test]
fn responses_routing_preserves_non_messages_formats() {
	let providers = [
		(
			"OpenAI",
			AIProvider::OpenAI(openai::Provider {
				model: None,
				moderation: None,
			}),
			"gpt-5",
			ChatFormat::OpenAIResponses,
		),
		(
			"custom Responses",
			custom_provider(custom::ProviderFormat::Responses),
			"custom-model",
			ChatFormat::OpenAIResponses,
		),
		(
			"Bedrock",
			AIProvider::bedrock(bedrock::Provider {
				model: None,
				region: strng::new("us-west-2"),
				guardrail_identifier: None,
				guardrail_version: None,
			}),
			"anthropic.claude-sonnet-4-5-v1:0",
			ChatFormat::BedrockConverse,
		),
		(
			"Vertex Gemini",
			vertex_provider("gemini-2.0-flash"),
			"gemini-2.0-flash",
			ChatFormat::OpenAICompletions,
		),
	];

	for (name, provider, model, expected) in providers {
		assert_eq!(
			provider
				.chat_translation(InputFormat::Responses, Some(model))
				.expect("Responses routing should remain available")
				.output,
			expected,
			"{name} changed Responses routing"
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
		types::ChatRequest::Responses(request),
		&ChatRequestContext {
			provider: &provider,
			headers: &HeaderMap::new(),
			prompt_caching: None,
		},
	)
	.expect("Responses request should render as Messages");
	assert!(matches!(
		rendered.provider_state.as_ref(),
		Some(ProviderState::ResponsesToMessages { .. })
	));
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
			provider_state: rendered.provider_state.as_ref(),
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

#[test]
fn responses_to_messages_buffered_requires_matching_provider_state() {
	let translation = ChatTranslation {
		input: InputFormat::Responses,
		output: ChatFormat::AnthropicMessages,
	};
	let upstream = Bytes::from_static(b"{}");
	let wrong_state = ProviderState::VertexGemini;

	for provider_state in [None, Some(&wrong_state)] {
		let result = translation.render_response(
			&upstream,
			&ChatResponseContext {
				model: "claude-sonnet-4-5",
				buffer_limit: 1024 * 1024,
				provider_state,
			},
		);
		let Err(error) = result else {
			panic!("missing or wrong conversion state must fail")
		};

		assert_eq!(
			error.to_string(),
			"unsupported conversion: missing Responses-to-Messages state"
		);
	}
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

#[test]
fn copilot_claude_captured_codex_request_applies_provider_policy() {
	let request: types::responses::Request = serde_json::from_str(include_str!(
		"../../../llm/src/conversion/messages/fixtures/codex_cli_0_146_0.json"
	))
	.expect("captured Codex Responses request");
	let provider = AIProvider::Copilot(copilot::Provider { model: None });

	let rendered = ChatTranslation {
		input: InputFormat::Responses,
		output: ChatFormat::AnthropicMessages,
	}
	.render_request(
		types::ChatRequest::Responses(request),
		&ChatRequestContext {
			provider: &provider,
			headers: &HeaderMap::new(),
			prompt_caching: None,
		},
	)
	.expect("captured Codex request should render for Copilot Claude");

	assert!(matches!(
		rendered.provider_state,
		Some(ProviderState::ResponsesToMessages { .. })
	));
	let body: Value = serde_json::from_slice(&rendered.body).expect("Messages request");
	let tools = body["tools"].as_array().expect("Messages tools");
	assert_eq!(tools.len(), 10);
	assert!(tools.iter().any(|tool| tool["name"] == "exec_command"));
	assert!(!tools.iter().any(|tool| tool["name"] == "web_search"));
}

#[tokio::test]
async fn copilot_claude_responses_route_preserves_path_with_host_override() {
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
	assert_eq!(request.uri().path(), "/v1/responses");
}

#[tokio::test]
async fn copilot_claude_completions_route_preserves_path_with_host_override() {
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
		.uri("https://api.githubcopilot.com/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model":"Claude-Sonnet-4-5",
				"messages":[{"role":"user","content":"say hi"}]
			}"#
				.to_vec(),
		))
		.expect("request");

	let RequestResult::Success {
		mut request,
		llm_request,
		upstream_route_type,
	} = provider
		.process_completions_request(&backend_info, None, req, false, &mut None)
		.await
		.expect("Copilot Claude Completions request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(upstream_route_type, RouteType::Messages);
	assert!(llm_request.provider_state.is_none());
	provider
		.setup_request(
			&mut request,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			true,
		)
		.expect("Copilot request setup");
	assert_eq!(request.uri().path(), "/v1/chat/completions");
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
			llm::LogContentFields::default(),
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
			llm::LogContentFields::default(),
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
async fn copilot_claude_responses_stream_captures_function_call_telemetry() {
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
			"model":"claude-sonnet-4-5", "input":"weather", "stream":true, "store":false,
			"stream_options":{"include_obfuscation":false},
			"tools":[{"type":"function","name":"get_weather","parameters":{"type":"object"}}]
		}"#
				.to_vec(),
		))
		.unwrap();
	let RequestResult::Success { llm_request, .. } = provider
		.process_responses_request(&backend_info, None, request, false, &mut None)
		.await
		.expect("request translation")
	else {
		panic!("expected forwarded request")
	};
	let upstream = [
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_gateway\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_weather\",\"name\":\"get_weather\",\"input\":{}}}\n\n",
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Paris\\\"}\"}}\n\n",
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
	let log = AsyncLog::default();
	let translated = provider
		.process_response(
			PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
			llm_request,
			LLMResponsePolicies::default(),
			None,
			log.clone(),
			llm::LogContentFields {
				completion: false,
				tool_calls: true,
			},
			None,
			response,
		)
		.await
		.expect("composed streaming response");
	let _ = translated.collect().await.unwrap();
	let info = log.take().expect("stream telemetry");
	let output_messages = info.response.output_messages.expect("tool-call telemetry");
	assert_eq!(
		output_messages[0].finish_reason.as_deref(),
		Some("completed")
	);
	let tool_calls = output_messages[0].tool_calls();
	assert_eq!(tool_calls.len(), 1);
	assert_eq!(tool_calls[0].id.as_str(), "call_weather");
	assert_eq!(tool_calls[0].name.as_str(), "get_weather");
	assert_eq!(tool_calls[0].arguments, json!({"city":"Paris"}));
}

#[tokio::test]
async fn copilot_claude_responses_stream_missing_or_wrong_state_returns_sanitized_error() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	for provider_state in [None, Some(ProviderState::VertexGemini)] {
		let mut req = llm_request_with_tokens(None);
		req.input_format = InputFormat::Responses;
		req.request_model = "claude-sonnet-4-5".into();
		req.provider_state = provider_state;
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
			llm::LogContentFields::default(),
			None,
			response,
		);
		let Err(error) = result else {
			panic!("missing or wrong conversion state must fail")
		};
		let message = error.to_string();
		assert_eq!(
			message,
			"unsupported conversion: missing Responses-to-Messages state"
		);
		assert!(!message.contains(marker));
	}
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
fn messages_to_completions_buffered_preserves_reasoning_usage() {
	let upstream = Bytes::from_static(
		br#"{
			"id":"msg_usage",
			"type":"message",
			"role":"assistant",
			"content":[{"type":"text","text":"done"}],
			"model":"claude-upstream",
			"stop_reason":"end_turn",
			"stop_sequence":null,
			"usage":{
				"input_tokens":11,
				"output_tokens":7,
				"output_tokens_details":{"thinking_tokens":3}
			}
		}"#,
	);
	let translated = conversion::messages::from_completions::translate_response(&upstream)
		.expect("Messages response should translate")
		.serialize()
		.expect("translated response should serialize");
	let value: Value = serde_json::from_slice(&translated).expect("Completions response");

	assert_eq!(value["usage"]["prompt_tokens"], 11);
	assert_eq!(value["usage"]["completion_tokens"], 7);
	assert_eq!(
		value["usage"]["completion_tokens_details"]["reasoning_tokens"],
		3
	);
}

#[tokio::test]
async fn messages_to_completions_stream_preserves_merged_usage() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut req = llm_request_with_tokens(None);
	req.request_model = "claude-sonnet-4-5".into();
	let upstream = [
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_usage\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":11,\"output_tokens\":1,\"cache_creation_input_tokens\":2,\"cache_read_input_tokens\":5,\"output_tokens_details\":{\"thinking_tokens\":2}}}}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n",
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"reason\"}}\n\n",
		"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
		"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"input_tokens\":null,\"output_tokens\":7,\"output_tokens_details\":{\"thinking_tokens\":3}}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	]
	.concat();
	let mut response = Response::new(Body::from(upstream));
	response.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"text/event-stream".parse().unwrap(),
	);
	let log = AsyncLog::default();
	let translated = provider
		.process_response(
			PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
			req,
			LLMResponsePolicies::default(),
			None,
			log.clone(),
			llm::LogContentFields::default(),
			None,
			response,
		)
		.await
		.expect("Messages stream should translate");
	let body = translated.collect().await.unwrap().to_bytes();
	let terminal_usage = String::from_utf8_lossy(&body)
		.split("\n\n")
		.filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("data: ")))
		.map(|data| serde_json::from_str::<Value>(data).unwrap())
		.find_map(|chunk| chunk.get("usage").filter(|usage| !usage.is_null()).cloned())
		.expect("terminal usage chunk");

	assert_eq!(terminal_usage["prompt_tokens"], 11);
	assert_eq!(terminal_usage["completion_tokens"], 7);
	assert_eq!(terminal_usage["total_tokens"], 18);
	assert_eq!(terminal_usage["prompt_tokens_details"]["cached_tokens"], 5);
	assert_eq!(
		terminal_usage["prompt_tokens_details"]["cache_write_tokens"],
		2
	);
	assert_eq!(
		terminal_usage["completion_tokens_details"]["reasoning_tokens"],
		3
	);
	let info = log.take().expect("stream telemetry");
	assert_eq!(info.response.input_tokens, Some(11));
	assert_eq!(info.response.output_tokens, Some(7));
	assert_eq!(info.response.total_tokens, Some(18));
	assert_eq!(info.response.reasoning_tokens, Some(3));
}

#[tokio::test]
async fn messages_to_completions_stream_captures_completion_and_tool_calls() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut req = llm_request_with_tokens(None);
	req.request_model = "claude-sonnet-4-5".into();
	let upstream = [
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_tools\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":2,\"output_tokens\":0}}}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
		"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_weather\",\"name\":\"get_weather\",\"input\":{}}}\n\n",
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Paris\\\"}\"}}\n\n",
		"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
		"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":4}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	]
	.concat();
	let mut response = Response::new(Body::from(upstream));
	response.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"text/event-stream".parse().unwrap(),
	);
	let log = AsyncLog::default();
	let translated = provider
		.process_response(
			PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
			req,
			LLMResponsePolicies::default(),
			None,
			log.clone(),
			llm::LogContentFields {
				completion: true,
				tool_calls: true,
			},
			None,
			response,
		)
		.await
		.expect("Messages stream should translate");
	let _ = translated.collect().await.unwrap();
	let info = log.take().expect("stream telemetry");

	assert_eq!(info.response.completion, Some(vec!["Hello".to_string()]));
	let output_messages = info.response.output_messages.expect("tool-call telemetry");
	assert_eq!(
		output_messages[0].finish_reason.as_deref(),
		Some("tool_use")
	);
	let tool_calls = output_messages[0].tool_calls();
	assert_eq!(tool_calls.len(), 1);
	assert_eq!(tool_calls[0].id.as_str(), "call_weather");
	assert_eq!(tool_calls[0].name.as_str(), "get_weather");
	assert_eq!(tool_calls[0].arguments, json!({"city":"Paris"}));
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

fn openai_inline_moderation_param() -> openai::ModerationParam {
	openai::ModerationParam {
		model: strng::new("omni-moderation-latest"),
		policy: Some(openai::ModerationPolicyParam {
			input: Some(openai::ModerationConfigParam {
				mode: openai::ModerationMode::Block,
			}),
			output: Some(openai::ModerationConfigParam {
				mode: openai::ModerationMode::Score,
			}),
		}),
	}
}

fn openai_inline_moderation_value() -> Value {
	json!({
		"model": "omni-moderation-latest",
		"policy": {
			"input": { "mode": "block" },
			"output": { "mode": "score" }
		}
	})
}

fn openai_test_backend_info() -> crate::http::auth::BackendInfo {
	let inputs = crate::test_helpers::proxymock::setup_proxy_test("{}")
		.unwrap()
		.pi;
	crate::http::auth::BackendInfo {
		target: crate::types::agent::BackendTarget::Invalid,
		call_target: Target::from(("api.openai.com", 443)),
		inputs,
	}
}

#[tokio::test]
async fn openai_inline_moderation_injected_for_completions() {
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: Some(openai_inline_moderation_param()),
	});
	let backend_info = openai_test_backend_info();
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "gpt-5",
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		upstream_route_type,
		..
	} = provider
		.process_completions_request(&backend_info, None, req, false, &mut None, None)
		.await
		.expect("OpenAI completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(upstream_route_type, RouteType::Completions);
	assert_eq!(
		forwarded_json["moderation"],
		openai_inline_moderation_value()
	);
}

#[tokio::test]
async fn openai_inline_moderation_overrides_client_value_for_completions() {
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: Some(openai_inline_moderation_param()),
	});
	let backend_info = openai_test_backend_info();
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "gpt-5",
				"messages": [{"role": "user", "content": "hello"}],
				"moderation": {
					"model": "client-selected-model",
					"policy": {
						"input": { "mode": "score" },
						"output": { "mode": "score" }
					}
				}
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded, ..
	} = provider
		.process_completions_request(&backend_info, None, req, false, &mut None, None)
		.await
		.expect("OpenAI completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(
		forwarded_json["moderation"],
		openai_inline_moderation_value()
	);
}

#[tokio::test]
async fn openai_client_moderation_passthrough_without_config() {
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
	let backend_info = openai_test_backend_info();
	let req = ::http::Request::builder()
		.uri("/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
					"model": "gpt-5",
					"messages": [{"role": "user", "content": "hello"}],
					"moderation": {
						"model": "client-selected-model",
						"policy": {
							"input": {
								"mode": "future-mode",
								"future_option": { "enabled": true }
							},
							"output": { "mode": "block" }
						}
					},
					"future_top_level": { "enabled": true }
				}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded, ..
	} = provider
		.process_completions_request(&backend_info, None, req, false, &mut None, None)
		.await
		.expect("OpenAI completions request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(
		forwarded_json["moderation"],
		json!({
			"model": "client-selected-model",
			"policy": {
				"input": {
					"mode": "future-mode",
					"future_option": { "enabled": true }
				},
				"output": { "mode": "block" }
			}
		})
	);
	assert_eq!(
		forwarded_json["future_top_level"],
		json!({ "enabled": true })
	);
}

#[tokio::test]
async fn openai_inline_moderation_injected_for_responses() {
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: Some(openai_inline_moderation_param()),
	});
	let backend_info = openai_test_backend_info();
	let req = ::http::Request::builder()
		.uri("/v1/responses")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "gpt-5",
				"input": "hello"
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		upstream_route_type,
		..
	} = provider
		.process_responses_request(&backend_info, None, req, false, &mut None, None)
		.await
		.expect("OpenAI responses request should process")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(upstream_route_type, RouteType::Responses);
	assert_eq!(
		forwarded_json["moderation"],
		openai_inline_moderation_value()
	);
}

#[tokio::test]
async fn openai_inline_moderation_injected_after_messages_translation() {
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: Some(openai_inline_moderation_param()),
	});
	let backend_info = openai_test_backend_info();
	let req = ::http::Request::builder()
		.uri("/v1/messages")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "gpt-5",
				"max_tokens": 64,
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: forwarded,
		upstream_route_type,
		..
	} = provider
		.process_messages_request(&backend_info, None, req, false, &mut None, None)
		.await
		.expect("Anthropic messages request should translate to OpenAI completions")
	else {
		panic!("expected forwarded request");
	};

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(upstream_route_type, RouteType::Completions);
	assert_eq!(
		forwarded_json["moderation"],
		openai_inline_moderation_value()
	);
}

#[test]
fn openai_response_passthrough_preserves_moderation_fields() {
	let completion_response: types::completions::Response = serde_json::from_value(json!({
		"model": "gpt-5",
		"usage": null,
		"choices": [],
		"moderation": {
			"input": { "flagged": false },
			"output": { "flagged": true }
		}
	}))
	.expect("completion response should deserialize");
	let completion_roundtrip =
		serde_json::to_value(completion_response).expect("completion response should serialize");
	assert_eq!(
		completion_roundtrip["moderation"],
		json!({
			"input": { "flagged": false },
			"output": { "flagged": true }
		})
	);

	let responses_response: types::responses::Response = serde_json::from_value(json!({
		"id": "resp_123",
		"status": "completed",
		"output": [],
		"model": "gpt-5",
		"moderation": {
			"input": { "flagged": false },
			"output": { "flagged": true }
		}
	}))
	.expect("responses response should deserialize");
	let responses_roundtrip =
		serde_json::to_value(responses_response).expect("responses response should serialize");
	assert_eq!(
		responses_roundtrip["moderation"],
		json!({
			"input": { "flagged": false },
			"output": { "flagged": true }
		})
	);
}

#[tokio::test]
async fn openai_provider_normalizes_max_tokens_before_forwarding() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
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
		.process_completions_request(&backend_info, None, req, false, &mut None, None)
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

	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
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
		.process_completions_request(&backend_info, Some(&policy), req, false, &mut None, None)
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

	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
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
		.process_completions_request(&backend_info, None, req, false, &mut None, None)
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

async fn copilot_local_token_count(body: Value) -> u64 {
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
		.body(Body::from(serde_json::to_vec(&body).unwrap()))
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
	response.input_tokens
}

#[tokio::test]
async fn copilot_local_count_tokens_uses_messages_only() {
	let without_tools = copilot_local_token_count(json!({
		"model": "claude-sonnet-5",
		"messages": [{"role": "user", "content": "hello"}]
	}))
	.await;
	let with_tools = copilot_local_token_count(json!({
		"model": "claude-sonnet-5",
		"messages": [{"role": "user", "content": "hello"}],
		"tools": [{
			"name": "lookup",
			"input_schema": {"type": "object"}
		}]
	}))
	.await;

	assert!(without_tools > 0);
	assert_eq!(with_tools, without_tools);
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

fn gemini_generate_content_request(uri: &str) -> ::http::Request<Body> {
	::http::Request::builder()
		.uri(uri)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"contents": [{"role": "user", "parts": [{"text": "hello"}]}],
				"someNewTopLevelField": {"a": true}
			}"#
				.to_vec(),
		))
		.unwrap()
}

fn vertex_backend_info() -> crate::http::auth::BackendInfo {
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;
	crate::http::auth::BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("aiplatform.googleapis.com", 443)),
		inputs: setup_proxy_test("{}").unwrap().pi,
	}
}

#[tokio::test]
async fn gemini_generate_content_forwards_unknown_top_level_fields() {
	let provider = AIProvider::Vertex(vertex::Provider {
		model: None,
		region: None,
		project_id: strng::new("test-project"),
	});
	let req = gemini_generate_content_request(
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
	);

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		..
	} = provider
		.process_gemini_request(&vertex_backend_info(), None, req, false, &mut None, None)
		.await
		.expect("generateContent request should process")
	else {
		panic!("expected forwarded request");
	};
	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	assert!(!llm_request.streaming);

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert_eq!(forwarded_json["someNewTopLevelField"], json!({"a": true}));
	assert_eq!(forwarded_json["contents"][0]["parts"][0]["text"], "hello");
	assert!(
		forwarded_json.get("model").is_none(),
		"the model rides the path, not the body: {forwarded_json}"
	);
}

#[tokio::test]
async fn gemini_stream_without_alt_sse_is_rejected_with_google_shaped_400() {
	let provider = AIProvider::Vertex(vertex::Provider {
		model: None,
		region: None,
		project_id: strng::new("test-project"),
	});
	for uri in [
		"https://example.com/v1beta/models/gemini-2.5-flash:streamGenerateContent",
		"https://example.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=json",
	] {
		let RequestResult::Rejected(resp) = provider
			.process_gemini_request(
				&vertex_backend_info(),
				None,
				gemini_generate_content_request(uri),
				false,
				&mut None,
				None,
			)
			.await
			.expect("the non-SSE streaming variant is a client error, not a gateway failure")
		else {
			panic!("expected a direct response for {uri}");
		};

		assert_eq!(resp.status(), ::http::StatusCode::BAD_REQUEST);
		let body = resp.into_body().collect().await.unwrap().to_bytes();
		let body: Value = serde_json::from_slice(&body).expect("error body should be JSON");
		assert_eq!(body["error"]["code"], json!(400));
		assert_eq!(body["error"]["status"], json!("INVALID_ARGUMENT"));
		assert!(
			body["error"]["message"]
				.as_str()
				.is_some_and(|m| m.contains("alt=sse")),
			"{body}"
		);
	}
}

fn gemini_count_tokens_request(uri: &str) -> ::http::Request<Body> {
	::http::Request::builder()
		.uri(uri)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"contents": [{"role": "user", "parts": [{"text": "hello"}]}],
				"someNewField": {"a": true}
			}"#
				.to_vec(),
		))
		.unwrap()
}

#[tokio::test]
async fn gemini_count_tokens_passes_body_through_on_vertex() {
	use crate::http::auth::BackendInfo;
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
		call_target: Target::from(("aiplatform.googleapis.com", 443)),
		inputs,
	};
	let req =
		gemini_count_tokens_request("https://example.com/v1beta/models/gemini-2.5-flash:countTokens");

	let RequestResult::Success {
		request: forwarded,
		llm_request,
		upstream_route_type,
		..
	} = provider
		.process_gemini_count_tokens_request(&backend_info, None, req, &mut None)
		.await
		.expect("countTokens request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(upstream_route_type, RouteType::GeminiCountTokens);
	assert_eq!(llm_request.input_format, InputFormat::GeminiCountTokens);
	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	assert!(!llm_request.streaming);

	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert_eq!(forwarded_json["someNewField"], json!({"a": true}));
	assert_eq!(forwarded_json["contents"][0]["parts"][0]["text"], "hello");
	assert!(
		forwarded_json.get("model").is_none(),
		"the model rides the path, not the body: {forwarded_json}"
	);
}

#[tokio::test]
async fn gemini_count_tokens_applies_model_alias_and_rewrites_upstream_path() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::Gemini(gemini::Provider { model: None });
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from((gemini::DEFAULT_HOST_STR, 443)),
		inputs,
	};
	let policy = Policy {
		model_aliases: std::collections::HashMap::from([(
			strng::new("fast"),
			strng::new("gemini-2.5-flash"),
		)]),
		..Default::default()
	};
	let req = gemini_count_tokens_request("https://example.com/v1beta/models/fast:countTokens");

	let RequestResult::Success {
		request: mut forwarded,
		llm_request,
		upstream_route_type,
		..
	} = provider
		.process_gemini_count_tokens_request(&backend_info, Some(&policy), req, &mut None)
		.await
		.expect("countTokens request should process")
	else {
		panic!("expected forwarded request");
	};
	assert_eq!(llm_request.request_model, "gemini-2.5-flash");

	provider
		.setup_request(
			&mut forwarded,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	assert_eq!(
		forwarded.uri().path(),
		"/v1beta/models/gemini-2.5-flash:countTokens"
	);
	assert_eq!(forwarded.uri().query(), None);
	assert_eq!(
		forwarded.uri().authority().map(|a| a.as_str()),
		Some(gemini::DEFAULT_HOST_STR)
	);
}

#[tokio::test]
async fn gemini_count_tokens_on_non_gemini_upstream_is_unsupported() {
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
	let req =
		gemini_count_tokens_request("https://example.com/v1beta/models/claude-opus-4:countTokens");

	let err = provider
		.process_gemini_count_tokens_request(&backend_info, None, req, &mut None)
		.await
		.expect_err("countTokens against a non-Gemini upstream must be rejected");
	assert!(matches!(err, AIError::UnsupportedConversion(_)), "{err}");
}

#[test]
fn gemini_count_tokens_response_reports_total_tokens() {
	let provider = AIProvider::Gemini(gemini::Provider { model: None });
	let req = LLMRequest {
		input_tokens: None,
		input_format: InputFormat::GeminiCountTokens,
		cache_convention: CacheTokenConvention::pending(),
		request_model: "gemini-2.5-flash".into(),
		provider: "gcp.gemini".into(),
		streaming: false,
		params: Default::default(),
		prompt: None,
		provider_state: None,
	};
	let body = br#"{"totalTokens":31,"promptTokensDetails":[{"modality":"TEXT","tokenCount":31}]}"#;
	let buffered = BufferedResponse {
		parts: ::http::Response::new(()).into_parts().0,
		bytes: bytes::Bytes::from_static(body),
	};

	let log = AsyncLog::<llm::LLMInfo>::default();
	let resp = provider
		.process_gemini_count_tokens_response(req, buffered, None, &log)
		.expect("countTokens response should process");
	assert_eq!(
		log.take().expect("llm info").response.count_tokens,
		Some(31)
	);
	assert!(resp.headers().get(header::CONTENT_LENGTH).is_none());
}

#[tokio::test]
async fn anthropic_count_tokens_preserves_upstream_errors() {
	let provider = AIProvider::bedrock(bedrock::Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	});
	let req = LLMRequest {
		input_tokens: None,
		input_format: InputFormat::CountTokens,
		cache_convention: CacheTokenConvention::pending(),
		request_model: "us.anthropic.claude-haiku-4-5-20251001-v1:0".into(),
		provider: "aws.bedrock".into(),
		streaming: false,
		params: Default::default(),
		prompt: None,
		provider_state: None,
	};
	let body =
		bytes::Bytes::from_static(br#"{"message":"The provided model does not support CountTokens"}"#);
	let mut parts = ::http::Response::new(()).into_parts().0;
	parts.status = ::http::StatusCode::BAD_REQUEST;
	let buffered = BufferedResponse {
		parts,
		bytes: body.clone(),
	};

	let resp = provider
		.process_count_tokens_response(req, buffered, None, &Default::default())
		.expect("error response should process");
	assert_eq!(resp.status(), ::http::StatusCode::BAD_REQUEST);
	assert_eq!(resp.into_body().collect().await.unwrap().to_bytes(), body);
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
		.process_messages_request(&backend_info, None, req, false, &mut None, None)
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
		moderation: None,
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
		.process_completions_request(&backend_info, Some(&policy), req, false, &mut None, None)
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
async fn messages_to_completions_final_transformation() {
	use crate::llm::policy::Policy;

	async fn create_llm_request(vec_body: Vec<u8>, policy: Option<&Policy>) -> (Request, RouteType) {
		let provider = AIProvider::OpenAI(openai::Provider {
			model: None,
			moderation: None,
		});
		let backend_info = openai_test_backend_info();
		let req = ::http::Request::builder()
			.uri("/v1/messages")
			.header(::http::header::CONTENT_TYPE, "application/json")
			.body(Body::from(vec_body))
			.unwrap();
		let RequestResult::Success {
			request: forwarded,
			upstream_route_type,
			..
		} = provider
			.process_messages_request(&backend_info, policy, req, false, &mut None, None)
			.await
			.expect("Anthropic messages request should translate to OpenAI completions")
		else {
			panic!("expected forwarded request");
		};
		(forwarded, upstream_route_type)
	}
	let expr = |e: &str| std::sync::Arc::new(crate::cel::Expression::new_strict(e).unwrap());

	let policy = Policy {
		final_transformations: Some(
			[
				// Only true final-conversion: `system` became messages[0].
				(
					"converted_message_count".to_string(),
					expr("llmRequest.messages.size()"),
				),
				// Mutate a field carried through the conversion.
				("max_tokens".to_string(), expr("32")),
				("reasoning_effort".to_string(), expr("fail(\"remove\")")),
			]
			.into_iter()
			.collect(),
		),
		..Default::default()
	};

	let vec_body = br#"{
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
			}"#
		.to_vec();

	let (forwarded, upstream_route_type) = create_llm_request(vec_body.clone(), Some(&policy)).await;
	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");

	assert_eq!(upstream_route_type, RouteType::Completions);
	// The request really was converted to completions format.
	assert_eq!(forwarded_json["messages"][0]["role"], json!("system"));
	// 2 (system + user), not the 1 message the client sent.
	assert_eq!(forwarded_json["converted_message_count"], json!(2));
	assert_eq!(forwarded_json["max_tokens"], json!(32));
	// Indexing returns Null for a missing key too, so assert on key presence.
	let reasoning_effort = forwarded_json.get("reasoning_effort");
	assert!(
		reasoning_effort.is_none(),
		"reasoning_effort should be removed, got: {reasoning_effort:?}"
	);

	let (forwarded, upstream_route_type) = create_llm_request(vec_body, None).await;
	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert_eq!(upstream_route_type, RouteType::Completions);
	// The request really was converted to completions format.
	assert_eq!(forwarded_json["messages"][0]["role"], json!("system"));
	// 2 (system + user), not the 1 message the client sent.
	assert_eq!(forwarded_json["max_completion_tokens"], json!(64));
	// Indexing returns Null for a missing key too, so assert on key presence.
	let reasoning_effort = forwarded_json.get("reasoning_effort");
	assert!(
		reasoning_effort.is_some(),
		"reasoning_effort should not be empty, got: {reasoning_effort:?}"
	);
}

#[tokio::test]
async fn detect_final_transformations_skip_opaque_bodies() {
	use crate::llm::policy::Policy;

	async fn create_detect_request(
		content_type: &str,
		body: &[u8],
		policy: Option<&Policy>,
	) -> (Request, RouteType) {
		let provider = AIProvider::OpenAI(openai::Provider {
			model: None,
			moderation: None,
		});
		let backend_info = openai_test_backend_info();
		let req = ::http::Request::builder()
			.uri("/v1/passthrough")
			.header(::http::header::CONTENT_TYPE, content_type)
			.body(Body::from(body.to_vec()))
			.unwrap();
		let RequestResult::Success {
			request: forwarded,
			upstream_route_type,
			..
		} = provider
			.process_detect_request(&backend_info, policy, req, &mut None)
			.await
			.expect("detect request should process")
		else {
			panic!("expected forwarded request");
		};
		(forwarded, upstream_route_type)
	}

	let expr = |e: &str| std::sync::Arc::new(crate::cel::Expression::new_strict(e).unwrap());
	let policy = Policy {
		final_transformations: Some(
			[("max_tokens".to_string(), expr("32"))]
				.into_iter()
				.collect(),
		),
		..Default::default()
	};

	// Inner spaces and key order survive only if the body is never round-tripped through serde.
	let json_body = br#"{ "model": "gpt-4o", "max_tokens": 64 }"#;

	// A non-JSON content type is passed through opaquely even when the body happens to parse as
	// JSON, so final transformations must not rewrite (or re-serialize) it.
	let (forwarded, route_type) = create_detect_request("text/plain", json_body, Some(&policy)).await;
	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	assert_eq!(route_type, RouteType::Detect);
	assert_eq!(
		forwarded_body.as_ref(),
		json_body.as_slice(),
		"passthrough body must be forwarded byte-for-byte"
	);

	// A body that fails to parse falls back to raw passthrough, which must not become an error.
	let raw_body = b"\x00\x01not json at all";
	let (forwarded, route_type) =
		create_detect_request("application/octet-stream", raw_body, Some(&policy)).await;
	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	assert_eq!(route_type, RouteType::Detect);
	assert_eq!(forwarded_body.as_ref(), raw_body.as_slice());

	// Malformed JSON under a JSON content type takes the same raw fallback.
	let bad_json = br#"{"model": "gpt-4o", "#;
	let (forwarded, route_type) =
		create_detect_request("application/json", bad_json, Some(&policy)).await;
	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	assert_eq!(route_type, RouteType::Detect);
	assert_eq!(forwarded_body.as_ref(), bad_json.as_slice());

	// A genuine JSON detect body is still transformed: this is the behavior the guard preserves.
	let (forwarded, route_type) =
		create_detect_request("application/json", json_body, Some(&policy)).await;
	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert_eq!(route_type, RouteType::Detect);
	assert_eq!(forwarded_json["max_tokens"], json!(32));
	assert_eq!(forwarded_json["model"], json!("gpt-4o"));

	// Without a policy the JSON body is unchanged apart from the parse/serialize round trip.
	let (forwarded, _) = create_detect_request("application/json", json_body, None).await;
	let forwarded_body = forwarded.collect().await.unwrap().to_bytes();
	let forwarded_json: Value =
		serde_json::from_slice(&forwarded_body).expect("forwarded request should be JSON");
	assert_eq!(forwarded_json["max_tokens"], json!(64));
}

#[tokio::test]
async fn bedrock_transformed_provider_model_is_used_for_upstream_path() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::bedrock(bedrock::Provider {
		model: Some(strng::new(
			"bedrock-runtime/us/anthropic.claude-3-5-sonnet-20241022-v2:0",
		)),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	});
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("bedrock-runtime.us-east-1.amazonaws.com", 443)),
		inputs,
	};
	let policy = Policy {
		transformations: Some(
			[(
				"model".to_string(),
				std::sync::Arc::new(
					crate::cel::Expression::new_strict(
						r#"llmRequest.model.stripPrefix("bedrock-runtime/us/")"#,
					)
					.unwrap(),
				),
			)]
			.into_iter()
			.collect(),
		),
		..Default::default()
	};
	let expected_model = "anthropic.claude-3-5-sonnet-20241022-v2:0";

	let req = ::http::Request::builder()
		.uri("https://gateway.example.com/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			json!({
				"model": "client-model",
				"messages": [{"role": "user", "content": "hello"}],
				"stream": true,
			})
			.to_string(),
		))
		.unwrap();

	let RequestResult::Success {
		request: mut forwarded,
		llm_request,
		upstream_route_type,
	} = provider
		.process_completions_request(&backend_info, Some(&policy), req, false, &mut None, None)
		.await
		.expect("Bedrock completions request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(llm_request.request_model, expected_model);
	provider
		.setup_request(
			&mut forwarded,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("Bedrock upstream request should be finalized");
	assert_eq!(
		forwarded.uri().path(),
		format!("/model/{expected_model}/converse-stream")
	);
}

#[tokio::test]
async fn bedrock_provider_model_overrides_client_model() {
	use crate::http::auth::BackendInfo;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let configured_model = "anthropic.claude-3-5-sonnet-20241022-v2:0";
	let provider = AIProvider::bedrock(bedrock::Provider {
		model: Some(strng::new(configured_model)),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	});
	let inputs = setup_proxy_test("{}").unwrap().pi;
	let backend_info = BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from(("bedrock-runtime.us-east-1.amazonaws.com", 443)),
		inputs,
	};
	let req = ::http::Request::builder()
		.uri("https://gateway.example.com/v1/chat/completions")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{
				"model": "client-model",
				"messages": [{"role": "user", "content": "hello"}]
			}"#
				.to_vec(),
		))
		.unwrap();

	let RequestResult::Success {
		request: mut forwarded,
		llm_request,
		upstream_route_type,
	} = provider
		.process_completions_request(&backend_info, None, req, false, &mut None, None)
		.await
		.expect("Bedrock completions request should process")
	else {
		panic!("expected forwarded request");
	};

	assert_eq!(llm_request.request_model, configured_model);
	provider
		.setup_request(
			&mut forwarded,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("Bedrock upstream request should be finalized");
	assert_eq!(
		forwarded.uri().path(),
		format!("/model/{configured_model}/converse")
	);
}

#[tokio::test]
async fn llm_transformations_can_set_missing_model() {
	use crate::http::auth::BackendInfo;
	use crate::llm::policy::Policy;
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;

	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
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
		.process_completions_request(&backend_info, Some(&policy), req, false, &mut None, None)
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
		.process_messages_request(&backend_info, None, req, false, &mut None, None)
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
fn copilot_embeddings_response_adds_missing_openai_fields() {
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut request = llm_request_with_tokens(None);
	request.input_format = InputFormat::Embeddings;
	request.request_model = "text-embedding-3-small".into();
	let response = Bytes::from_static(
		br#"{"data":[{"embedding":[0.5,-0.25],"index":0,"object":"embedding"}],"usage":{"prompt_tokens":2,"total_tokens":2}}"#,
	);

	let (llm_response, body) = provider
		.process_embeddings_response(&request, &::http::HeaderMap::new(), response)
		.expect("Copilot embeddings response should normalize");
	let body: Value = serde_json::from_slice(&body).expect("normalized response should be JSON");

	assert_eq!(body["object"], json!("list"));
	assert_eq!(body["model"], json!("text-embedding-3-small"));
	assert_eq!(body["data"][0]["embedding"], json!([0.5, -0.25]));
	assert_eq!(llm_response.input_tokens, Some(2));
	assert_eq!(llm_response.total_tokens, Some(2));
}

#[test]
fn copilot_embeddings_response_preserves_missing_usage() {
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut request = llm_request_with_tokens(None);
	request.input_format = InputFormat::Embeddings;
	request.request_model = "text-embedding-3-small".into();
	let response =
		Bytes::from_static(br#"{"data":[{"embedding":[0.5],"index":0,"object":"embedding"}]}"#);

	let (_, body) = provider
		.process_embeddings_response(&request, &::http::HeaderMap::new(), response)
		.expect("Copilot embeddings response should normalize");
	let body: Value = serde_json::from_slice(&body).expect("normalized response should be JSON");

	assert!(body.get("usage").is_none());
}

#[test]
fn copilot_embeddings_response_preserves_explicit_openai_fields() {
	let provider = AIProvider::Copilot(copilot::Provider { model: None });
	let mut request = llm_request_with_tokens(None);
	request.input_format = InputFormat::Embeddings;
	request.request_model = "requested-model".into();
	let response = Bytes::from_static(
		br#"{"object":"upstream-list","model":"upstream-model","data":[{"embedding":[0.5],"index":0,"object":"embedding"}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#,
	);

	let (_, body) = provider
		.process_embeddings_response(&request, &::http::HeaderMap::new(), response)
		.expect("Copilot embeddings response should preserve explicit fields");
	let body: Value = serde_json::from_slice(&body).expect("normalized response should be JSON");

	assert_eq!(body["object"], json!("upstream-list"));
	assert_eq!(body["model"], json!("upstream-model"));
}

#[test]
fn copilot_embeddings_parse_error_logs_normalized_response() {
	#[derive(Clone)]
	struct LogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

	impl std::io::Write for LogWriter {
		fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
			self.0.lock().unwrap().extend_from_slice(buf);
			Ok(buf.len())
		}

		fn flush(&mut self) -> std::io::Result<()> {
			Ok(())
		}
	}

	let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
	let writer = LogWriter(logs.clone());
	let subscriber = tracing_subscriber::fmt()
		.with_ansi(false)
		.without_time()
		.with_writer(move || writer.clone())
		.finish();

	tracing::subscriber::with_default(subscriber, || {
		let provider = AIProvider::Copilot(copilot::Provider { model: None });
		let mut request = llm_request_with_tokens(None);
		request.input_format = InputFormat::Embeddings;
		request.request_model = "text-embedding-3-small".into();
		let response = Bytes::from_static(br#"{"usage":"invalid"}"#);

		assert!(
			provider
				.process_embeddings_response(&request, &::http::HeaderMap::new(), response)
				.is_err()
		);
	});

	let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
	assert!(logs.contains(r#""object":"list""#), "{logs}");
	assert!(
		logs.contains(r#""model":"text-embedding-3-small""#),
		"{logs}"
	);
}

#[test]
fn non_copilot_embeddings_response_still_requires_openai_fields() {
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
	let mut request = llm_request_with_tokens(None);
	request.input_format = InputFormat::Embeddings;
	let response = Bytes::from_static(
		br#"{"data":[{"embedding":[0.5],"index":0,"object":"embedding"}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#,
	);

	assert!(
		provider
			.process_embeddings_response(&request, &::http::HeaderMap::new(), response)
			.is_err()
	);
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

#[test]
fn non_copilot_providers_preserve_copilot_unsupported_beta_header() {
	let providers = [
		AIProvider::Anthropic(anthropic::Provider { model: None }),
		vertex_provider("anthropic/claude-sonnet-4-5"),
		AIProvider::azure(azure::Provider {
			model: None,
			resource_name: strng::new("example"),
			resource_type: azure::AzureResourceType::Foundry,
			api_version: None,
			project_name: Some(strng::new("project")),
		}),
		AIProvider::bedrock(bedrock::Provider {
			model: None,
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		}),
		custom_provider(custom::ProviderFormat::Messages),
	];
	let mut llm_request = llm_request_with_tokens(None);
	llm_request.request_model = "claude-sonnet-4-5".into();

	for provider in providers {
		let mut request = crate::http::tests_common::request(
			"https://example.com/v1/messages",
			http::Method::POST,
			&[],
		);
		request.headers_mut().insert(
			"anthropic-beta",
			HeaderValue::from_static(CLAUDE_CODE_2_1_217_BETA_HEADER),
		);

		provider
			.set_required_fields(&mut request, RouteType::Messages, Some(&llm_request))
			.expect("non-Copilot Messages headers");

		assert_eq!(
			request.headers()["anthropic-beta"],
			CLAUDE_CODE_2_1_217_BETA_HEADER,
			"{} applied Copilot beta policy",
			provider.provider()
		);
	}
}

#[test]
fn non_copilot_messages_providers_preserve_context_management() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "claude-sonnet-4-5",
		"max_tokens": 64,
		"messages": [{"role": "user", "content": "say hi"}],
		"context_management": {"edits": [{"type": "clear_tool_uses_20250919"}]},
		"some_future_anthropic_field": "preserved"
	}))
	.expect("Messages request");
	let providers = [
		AIProvider::Anthropic(anthropic::Provider { model: None }),
		vertex_provider("anthropic/claude-sonnet-4-5"),
		AIProvider::azure(azure::Provider {
			model: None,
			resource_name: strng::new("example"),
			resource_type: azure::AzureResourceType::Foundry,
			api_version: None,
			project_name: Some(strng::new("project")),
		}),
		custom_provider(custom::ProviderFormat::Messages),
	];

	for provider in providers {
		let translation = provider
			.chat_translation(InputFormat::Messages, Some("claude-sonnet-4-5"))
			.expect("Messages routing");
		let rendered = translation
			.render_request(
				types::ChatRequest::Messages(request.clone()),
				&ChatRequestContext {
					provider: &provider,
					headers: &HeaderMap::new(),
					prompt_caching: None,
				},
			)
			.expect("non-Copilot Messages body");
		let body: Value = serde_json::from_slice(&rendered.body).expect("Messages JSON");

		assert!(
			body.get("context_management").is_some(),
			"{} applied Copilot body policy",
			provider.provider()
		);
		assert_eq!(body["some_future_anthropic_field"], "preserved");
	}
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

	let translated = conversion::messages::from_completions::translate(&request, None)
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

	let translated = conversion::messages::from_completions::translate(&request, None)
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
			Default::default(),
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

#[tokio::test]
async fn upstream_encoding_is_applied_after_messages_response_translation() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
	let mut req = llm_request_with_tokens(None);
	req.input_format = InputFormat::Messages;
	req.request_model = "gpt-4o".into();
	req.streaming = false;
	let upstream_body = br#"{"id":"chatcmpl-1","object":"chat.completion","created":0,"model":"gpt-4o","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"Hello!"}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
	let compressed = crate::http::compression::encode_body(upstream_body, "br")
		.await
		.unwrap();
	let upstream = ::http::Response::builder()
		.header(::http::header::CONTENT_ENCODING, "br")
		.header(::http::header::CONTENT_LENGTH, compressed.len())
		.body(Body::from(compressed))
		.unwrap();

	let response = provider
		.process_response(
			PolicyClient::new(setup_proxy_test("{}").unwrap().pi),
			req,
			LLMResponsePolicies::default(),
			None,
			Default::default(),
			None,
			upstream,
		)
		.await
		.unwrap();

	// Keep the response plain while later response policies can still replace its body.
	assert!(
		!response
			.headers()
			.contains_key(::http::header::CONTENT_ENCODING)
	);
	assert!(
		response
			.extensions()
			.get::<crate::cel::BufferedBody>()
			.is_none()
	);
	let (parts, body) = response.into_parts();
	let mut body: Value = serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(body["type"], "message");
	assert_eq!(body["content"][0]["text"], "Hello!");
	// Stand in for a later response-body policy mutation.
	body["policy_applied"] = true.into();
	let mut response = Response::from_parts(parts, Body::from(serde_json::to_vec(&body).unwrap()));

	encode_deferred_response(&mut response);
	assert_eq!(response.headers()[::http::header::CONTENT_ENCODING], "br");
	assert!(
		!response
			.headers()
			.contains_key(::http::header::CONTENT_LENGTH)
	);
	let content_encoding = response.headers().typed_get::<ContentEncoding>();
	let (_, body) = crate::http::compression::to_bytes_with_decompression(
		response.into_body(),
		content_encoding.as_ref(),
		1024 * 1024,
	)
	.await
	.unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(body["type"], "message");
	assert_eq!(body["policy_applied"], true);
}

#[test]
fn openai_completions_error_translates_to_messages_client() {
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
	let mut req = llm_request_with_tokens(None);
	req.input_format = InputFormat::Messages;
	req.request_model = "gpt-4o".into();

	let error = Bytes::from_static(
		br#"{"error":{"message":"bad request","type":"invalid_request_error","param":null,"code":400}}"#,
	);
	let translated = provider
		.process_error(&req, ::http::StatusCode::BAD_REQUEST, &error, None)
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
		.process_error(&req, ::http::StatusCode::BAD_REQUEST, &error, None)
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
		.process_error(&req, ::http::StatusCode::BAD_REQUEST, &error, None)
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
			Default::default(),
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
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
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
	let provider = AIProvider::OpenAI(openai::Provider {
		model: None,
		moderation: None,
	});
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

#[test]
fn setup_request_custom_generate_content_defaults_to_the_native_path() {
	// A static configured path cannot carry the model or the streaming method, so the
	// default for the native Gemini chat format is the canonical Gemini API shape.
	let provider = custom_provider(custom::ProviderFormat::GenerateContent);
	for (streaming, expected_path, expected_query) in [
		(
			false,
			"/v1beta/models/gemini-2.5-flash:generateContent",
			None,
		),
		(
			true,
			"/v1beta/models/gemini-2.5-flash:streamGenerateContent",
			Some("alt=sse"),
		),
	] {
		let llm_request = LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Gemini,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "gemini-2.5-flash".into(),
			provider: Default::default(),
			streaming,
			params: Default::default(),
			prompt: None,
			provider_state: Some(ProviderState::VertexGemini),
		};
		let mut req = crate::http::tests_common::request(
			"https://gemini.example.com/v1beta/models/gemini-2.5-flash:generateContent",
			http::Method::POST,
			&[],
		);

		provider
			.setup_request(
				&mut req,
				RouteType::GenerateContent,
				Some(&llm_request),
				None,
				None,
				false,
			)
			.expect("setup_request should succeed");

		assert_eq!(req.uri().path(), expected_path, "streaming={streaming}");
		assert_eq!(req.uri().query(), expected_query, "streaming={streaming}");
	}
}

#[test]
fn setup_request_custom_count_tokens_defaults_to_the_native_path() {
	// Regression: with no configured path, countTokens used to fall through to the
	// OpenAI default and land on /v1/chat/completions.
	let provider = custom_provider(custom::ProviderFormat::GeminiCountTokens);
	let llm_request = LLMRequest {
		input_tokens: None,
		input_format: InputFormat::GeminiCountTokens,
		cache_convention: CacheTokenConvention::pending(),
		request_model: "gemini-2.5-flash".into(),
		provider: Default::default(),
		streaming: false,
		params: Default::default(),
		prompt: None,
		provider_state: None,
	};
	let mut req = crate::http::tests_common::request(
		"https://gemini.example.com/v1beta/models/gemini-2.5-flash:countTokens",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::GeminiCountTokens,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(
		req.uri().path(),
		"/v1beta/models/gemini-2.5-flash:countTokens"
	);
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

fn native_gemini_llm_request(request_model: &str, streaming: bool) -> LLMRequest {
	LLMRequest {
		input_tokens: None,
		input_format: InputFormat::Gemini,
		cache_convention: CacheTokenConvention::pending(),
		request_model: request_model.into(),
		provider: Default::default(),
		streaming,
		params: Default::default(),
		prompt: None,
		provider_state: Some(ProviderState::VertexGemini),
	}
}

#[test]
fn setup_request_gemini_native_builds_generate_content_path() {
	let provider = AIProvider::Gemini(gemini::Provider { model: None });
	let llm_request = native_gemini_llm_request("gemini-2.5-flash", false);
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Completions,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(
		req.uri().authority().map(|a| a.as_str()),
		Some("generativelanguage.googleapis.com")
	);
	assert_eq!(
		req.uri().path(),
		"/v1beta/models/gemini-2.5-flash:generateContent"
	);
	assert_eq!(req.uri().query(), None);
}

#[test]
fn setup_request_gemini_native_streaming_adds_alt_sse_and_strips_client_api_keys() {
	let provider = AIProvider::Gemini(gemini::Provider { model: None });
	// The client's own alt=sse is dropped in favour of the path-provided one. Credential query
	// parameters are stripped while unrelated parameters survive.
	let llm_request = native_gemini_llm_request("models/gemini-2.5-flash", true);
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=abc&%24key=def&keep=yes",
		http::Method::POST,
		&[("authorization", "Bearer AIzaOperatorKey")],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Completions,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(
		req.uri().path(),
		"/v1beta/models/gemini-2.5-flash:streamGenerateContent"
	);
	assert_eq!(req.uri().query(), Some("alt=sse&keep=yes"));
}

#[test]
fn setup_request_strips_query_api_keys_only_for_native_gemini() {
	for (provider, expected_query) in [
		(
			AIProvider::Gemini(gemini::Provider { model: None }),
			"alt=sse&keep=yes",
		),
		(
			AIProvider::Vertex(vertex::Provider {
				model: None,
				region: None,
				project_id: strng::new("test-project"),
			}),
			"alt=sse&key=abc&%24key=def&keep=yes",
		),
	] {
		let llm_request = native_gemini_llm_request("gemini-2.5-flash", true);
		let mut req = crate::http::tests_common::request(
			"https://proxy.example.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=abc&%24key=def&keep=yes",
			http::Method::POST,
			&[("authorization", "Bearer ya29.operator-token")],
		);

		provider
			.setup_request(
				&mut req,
				RouteType::Completions,
				Some(&llm_request),
				None,
				None,
				true,
			)
			.expect("setup_request should succeed");

		assert_eq!(
			req.uri().path(),
			"/v1beta/models/gemini-2.5-flash:streamGenerateContent"
		);
		assert_eq!(req.uri().query(), Some(expected_query));
	}
}

#[test]
fn setup_request_gemini_without_native_state_keeps_compat_path() {
	let provider = AIProvider::Gemini(gemini::Provider { model: None });
	let llm_request = LLMRequest {
		provider_state: None,
		..native_gemini_llm_request("gemini-2.5-flash", false)
	};
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1/chat/completions?key=abc&%24key=def",
		http::Method::POST,
		&[("authorization", "Bearer AIzaOperatorKey")],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Completions,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(req.uri().path(), "/v1beta/openai/chat/completions");
	assert_eq!(req.uri().query(), Some("key=abc&%24key=def"));
}

#[test]
fn native_copilot_messages_host_override_no_prefix_preserves_client_path() {
	// A native (unconverted) Copilot Messages request under a host override with no explicit
	// pathPrefix must keep trusting the client's own path, same as every other non-Custom provider.
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
fn setup_request_bedrock_sets_signing_region_with_host_override() {
	let provider = AIProvider::bedrock(bedrock::Provider {
		model: None,
		region: strng::new("ca-central-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	});
	let mut req = crate::http::tests_common::request(
		"https://bedrock-vpce.example.com/model/example/converse",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(&mut req, RouteType::Messages, None, None, None, true)
		.expect("setup_request should succeed");

	assert_eq!(
		req.uri().authority().map(|authority| authority.as_str()),
		Some("bedrock-vpce.example.com")
	);
	assert_eq!(
		req
			.extensions()
			.get::<bedrock::AwsRegion>()
			.map(|region| region.region.as_str()),
		Some("ca-central-1")
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
		llm::LogContentFields {
			completion: true,
			tool_calls: true,
		},
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
		llm::LogContentFields::default(),
		None,
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(
		info.response.completion.is_none(),
		"completion should not be set when log_content.completion is false"
	);
	assert!(
		info.response.output_messages.is_none(),
		"output messages should not be set when log_content.tool_calls is false"
	);
}

#[tokio::test]
async fn bedrock_from_messages_stream_captures_tool_calls() {
	let input_bytes =
		fs::read(fixture_path("response/bedrock/tool.bin")).expect("Failed to read fixture");
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
	let body = conversion::bedrock::from_messages::translate_stream(
		body,
		1024 * 1024,
		logger,
		"us.anthropic.claude-haiku-4-5-20251001-v1:0",
		"msg_123",
		llm::LogContentFields {
			completion: false,
			tool_calls: true,
		},
		None,
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(info.response.completion.is_none());
	let output_messages = info
		.response
		.output_messages
		.expect("output messages should be set for Bedrock tool calls");
	assert_eq!(
		output_messages[0].finish_reason.as_deref(),
		Some("tool_use")
	);
	let tool_calls = output_messages[0].tool_calls();
	assert_eq!(tool_calls.len(), 2);
	assert_eq!(tool_calls[0].name.as_str(), "top_song");
	assert_eq!(tool_calls[0].arguments, serde_json::json!({"sign": "WZPZ"}));
	assert_eq!(tool_calls[1].name.as_str(), "hello");
	assert_eq!(
		tool_calls[1].arguments,
		serde_json::json!({"sign": "world"})
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
	let body = conversion::messages::passthrough_stream(
		body,
		buffer_limit,
		logger,
		llm::LogContentFields {
			completion: true,
			tool_calls: true,
		},
	);
	// Consume the body to drive the stream to completion
	let output = body.collect().await.unwrap().to_bytes();
	assert!(output.ends_with(b"data: [DONE]\n\n"));
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
	let output = conversion::messages::passthrough_stream(
		body,
		1024 * 1024,
		logger,
		llm::LogContentFields::default(),
	)
	.collect()
	.await
	.expect("native Messages stream")
	.to_bytes();

	assert_eq!(output.as_ref(), expected);
}

#[tokio::test]
async fn copilot_messages_passthrough_preserves_native_sse_bytes() {
	let expected = concat!(
		": keep-this-comment\r\n",
		"id: upstream-7\r\n",
		"event: message_start\r\n",
		"data:{\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-upstream\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\r\n",
		"\r\n",
		"event: message_stop\r\n",
		"data: {\"type\":\"message_stop\"}\r\n",
		"\r\n",
		"data: [DONE]\r\n",
		"\r\n",
	);
	let split = expected.find("DONE").expect("done marker") + 2;
	let mut trailers = http::HeaderMap::new();
	trailers.insert(
		"x-upstream-trailer",
		http::HeaderValue::from_static("preserved"),
	);
	let body = Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
		vec![
			Ok::<_, std::convert::Infallible>(http_body::Frame::data(bytes::Bytes::copy_from_slice(
				&expected.as_bytes()[..split],
			))),
			Ok(http_body::Frame::data(bytes::Bytes::copy_from_slice(
				&expected.as_bytes()[split..],
			))),
			Ok(http_body::Frame::trailers(trailers.clone())),
		],
	)));
	let output = conversion::messages::passthrough_stream(
		body,
		1024 * 1024,
		agent_llm::StreamingUsageGuard::default(),
		llm::LogContentFields::default(),
	)
	.collect()
	.await
	.expect("Copilot Messages stream");

	assert_eq!(output.trailers(), Some(&trailers));
	assert_eq!(output.to_bytes().as_ref(), expected.as_bytes());
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
		llm::LogContentFields::default(),
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
		llm::LogContentFields::default(),
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
	let body = conversion::messages::passthrough_stream(
		body,
		buffer_limit,
		logger,
		llm::LogContentFields::default(),
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(
		info.response.completion.is_none(),
		"completion should not be set when log_content.completion is false"
	);
	assert!(
		info.response.output_messages.is_none(),
		"output messages should not be set when log_content.tool_calls is false"
	);
}

#[tokio::test]
async fn messages_passthrough_stream_captures_tool_calls() {
	let input_bytes =
		fs::read(fixture_path("response/anthropic/stream_tool.json")).expect("Failed to read fixture");
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
	let body = conversion::messages::passthrough_stream(
		body,
		1024 * 1024,
		logger,
		llm::LogContentFields {
			completion: false,
			tool_calls: true,
		},
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(info.response.completion.is_none());
	let output_messages = info
		.response
		.output_messages
		.expect("output messages should be set for Anthropic tool calls");
	assert_eq!(
		output_messages[0].finish_reason.as_deref(),
		Some("tool_use")
	);
	let tool_calls = output_messages[0].tool_calls();
	assert_eq!(tool_calls.len(), 1);
	assert_eq!(tool_calls[0].id.as_str(), "toolu_01A");
	assert_eq!(tool_calls[0].name.as_str(), "get_weather");
	assert_eq!(
		tool_calls[0].arguments,
		serde_json::json!({"location": "San Francisco"})
	);
}

#[tokio::test]
async fn responses_passthrough_stream_captures_completion_and_tool_calls() {
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
	let body = conversion::responses::passthrough_stream(
		body,
		buffer_limit,
		logger,
		llm::LogContentFields {
			completion: true,
			tool_calls: true,
		},
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	let completion = info
		.response
		.completion
		.expect("completion should be set for responses streaming");
	assert_eq!(completion.join(""), "Hello");
	let output_messages = info
		.response
		.output_messages
		.expect("output messages should be set for responses streaming");
	assert_eq!(
		output_messages[0].finish_reason.as_deref(),
		Some("completed")
	);
	let tool_calls = output_messages[0].tool_calls();
	assert_eq!(tool_calls.len(), 1);
	assert_eq!(tool_calls[0].id.as_str(), "call_xxx");
	assert_eq!(tool_calls[0].name.as_str(), "get_weather");
	assert_eq!(
		tool_calls[0].arguments,
		serde_json::json!({"location": "San Francisco"})
	);
}

#[tokio::test]
async fn responses_passthrough_stream_preserves_moderation_chunks() {
	let input_bytes = br#"event: response.completed
data: {"type":"response.completed","sequence_number":1,"response":{"created_at":123,"id":"resp_123","model":"gpt-5","object":"response","output":[],"status":"completed","moderation":{"input":{"flagged":false},"output":{"flagged":true}}}}

data: [DONE]

"#;
	let body = Body::from(input_bytes.to_vec());
	let log = AsyncLog::default();
	let llmresp = LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Responses,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "gpt-5".into(),
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
	let body = conversion::responses::passthrough_stream(
		body,
		buffer_limit,
		logger,
		llm::LogContentFields::default(),
	);
	let output = body.collect().await.unwrap().to_bytes();
	let text = String::from_utf8(output.to_vec()).expect("stream should be valid UTF-8");

	assert!(text.contains(r#""moderation":{"input":{"flagged":false},"output":{"flagged":true}}"#));
	assert!(text.contains("data: [DONE]"));
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
	let body = conversion::responses::passthrough_stream(
		body,
		buffer_limit,
		logger,
		llm::LogContentFields::default(),
	);
	let _ = body.collect().await.unwrap();
	let info = log2
		.take()
		.expect("log should have LLMInfo after stream completes");
	assert!(
		info.response.completion.is_none(),
		"completion should not be set when log_content.completion is false"
	);
	assert!(
		info.response.output_messages.is_none(),
		"output messages should not be set when log_content.tool_calls is false"
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
fn azure_foundry_messages_backend_uses_exclusive_convention() {
	let provider = AIProvider::azure(azure::Provider {
		model: None,
		resource_name: strng::new("example"),
		resource_type: azure::AzureResourceType::Foundry,
		api_version: None,
		project_name: Some(strng::new("project")),
	});
	assert_eq!(
		cache_convention_for(
			&provider,
			Some(custom::ProviderFormat::Messages),
			"claude-sonnet-4-5"
		),
		CacheTokenConvention::InputExcludesCache,
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
			&AIProvider::OpenAI(openai::Provider {
				model: None,
				moderation: None,
			}),
			Some(custom::ProviderFormat::Completions),
			"gpt-4o"
		),
		CacheTokenConvention::InputIncludesCache,
	);
}

#[test]
fn query_requests_sse_matches_alt_query_parameter() {
	let uri = |s: &str| s.parse::<::http::Uri>().expect("valid uri");
	assert!(query_requests_sse(&uri(
		"/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
	)));
	assert!(query_requests_sse(&uri(
		"/v1beta/models/gemini-2.5-flash:streamGenerateContent?key=abc&alt=sse"
	)));
	assert!(!query_requests_sse(&uri(
		"/v1beta/models/gemini-2.5-flash:streamGenerateContent"
	)));
	assert!(!query_requests_sse(&uri(
		"/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=json"
	)));
	assert!(!query_requests_sse(&uri(
		"/v1beta/models/gemini-2.5-flash:streamGenerateContent?halt=sse"
	)));
}
