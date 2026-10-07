use std::io;

use agent_core::strng;
use bytes::Bytes;
use http::HeaderMap;
use http_body_util::BodyExt;
use serde_json::json;

use super::*;
use crate::bedrock::Provider;
use crate::types;

#[tokio::test]
async fn test_append_done_on_success_omits_done_after_error() {
	let mut body = crate::parse::sse::append_done_on_success(agent_http::Body::from_stream(
		futures_util::stream::iter(vec![
			Ok::<_, axum_core::Error>(Bytes::from_static(b"data: chunk\n\n")),
			Err(axum_core::Error::new(io::Error::other("boom"))),
		]),
	));

	let first = body
		.frame()
		.await
		.expect("first frame should be present")
		.expect("first frame should succeed")
		.into_data()
		.expect("first frame should contain data");
	assert_eq!(first, Bytes::from_static(b"data: chunk\n\n"));

	let second = body.frame().await.expect("error frame should be present");
	assert!(second.is_err(), "upstream error should be forwarded");
	assert!(
		body.frame().await.is_none(),
		"stream must terminate after an upstream error without appending [DONE]"
	);
}

#[tokio::test]
async fn test_append_done_on_success_does_not_repoll_after_eof() {
	let mut body = crate::parse::sse::append_done_on_success(agent_http::Body::from_stream(
		futures_util::stream::iter(vec![Ok::<_, axum_core::Error>(Bytes::from_static(
			b"data: chunk\n\n",
		))]),
	));

	assert!(body.frame().await.is_some(), "data frame should be present");
	assert!(
		body.frame().await.is_some(),
		"[DONE] frame should be present"
	);
	assert!(body.frame().await.is_none(), "stream should report EOF");
	assert!(body.frame().await.is_none(), "stream must remain at EOF");
}

#[test]
fn test_extract_beta_headers_variants() {
	let headers = HeaderMap::new();
	assert!(helpers::extract_beta_headers(&headers).unwrap().is_none());

	let mut headers = HeaderMap::new();
	headers.insert("anthropic-beta", "computer-use-2025-01-24".parse().unwrap());
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![json!("computer-use-2025-01-24")]
	);

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		"cache-control-2024-08-15,computer-use-2025-01-24,tool-examples-2025-10-29"
			.parse()
			.unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![
			json!("computer-use-2025-01-24"),
			json!("tool-examples-2025-10-29"),
		]
	);

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		" cache-control-2024-08-15 , computer-use-2025-01-24 "
			.parse()
			.unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![json!("computer-use-2025-01-24"),]
	);

	let mut headers = HeaderMap::new();
	headers.append(
		"anthropic-beta",
		"cache-control-2024-08-15".parse().unwrap(),
	);
	headers.append(
		"anthropic-beta",
		"interleaved-thinking-2025-05-14".parse().unwrap(),
	);
	// Converse rejects tool-search-tool ("not currently supported on the Converse APIs"), so it
	// must not be forwarded on this path; Claude tool search goes through InvokeModel instead
	// (agentgateway/agentgateway#3818 section 4).
	headers.append(
		"anthropic-beta",
		"tool-search-tool-2025-10-19".parse().unwrap(),
	);
	let mut beta_features = helpers::extract_beta_headers(&headers)
		.unwrap()
		.unwrap()
		.into_iter()
		.map(|v| v.as_str().unwrap().to_string())
		.collect::<Vec<_>>();
	beta_features.sort();
	assert_eq!(
		beta_features,
		vec!["interleaved-thinking-2025-05-14".to_string()]
	);

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		"prompt-caching-2024-07-31".parse().unwrap(),
	);
	assert!(helpers::extract_beta_headers(&headers).unwrap().is_none());
}

#[test]
fn test_metadata_from_header() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	// Simulate transformation CEL setting x-bedrock-metadata header
	let mut headers = HeaderMap::new();
	headers.insert(
		"x-bedrock-metadata",
		r#"{"user_id": "user123", "department": "engineering", "json_user": "{\"device_id\":\"abc\"}", "bad?key": "bad{}"}"#
			.parse()
			.unwrap(),
	);

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: None,
		top_k: None,
		top_p: None,
		tools: None,
		tool_choice: None,
		thinking: None,
		output_config: None,
	};

	let (out, _) =
		super::from_messages::translate_internal(req, &provider, Some(&headers), None).unwrap();
	let metadata = out.request_metadata.unwrap();

	assert_eq!(metadata.get("user_id"), Some(&"user123".to_string()));
	assert_eq!(metadata.get("department"), Some(&"engineering".to_string()));
	assert_eq!(
		metadata.get("json_user"),
		Some(&r#"{"device_id":"abc"}"#.to_string())
	);
	assert_eq!(metadata.get("bad?key"), Some(&"bad{}".to_string()));
}

#[test]
fn test_output_config_effort_without_thinking_is_passed_through() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: Some(0.7),
		top_k: Some(50),
		top_p: Some(0.8),
		tools: None,
		tool_choice: None,
		thinking: None,
		output_config: Some(messages::typed::OutputConfig {
			effort: Some(messages::typed::ThinkingEffort::High),
			format: None,
		}),
	};

	let (out, _) = super::from_messages::translate_internal(req, &provider, None, None).unwrap();
	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"top_k": 50,
			"output_config": {
				"effort": "high"
			}
		}))
	);
	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, Some(0.7));
	assert_eq!(inference.top_p, Some(0.8));
}

#[test]
fn test_explicit_empty_output_config_is_preserved() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: Some(0.7),
		top_k: Some(50),
		top_p: Some(0.8),
		tools: None,
		tool_choice: None,
		thinking: Some(messages::typed::ThinkingInput::Adaptive {}),
		output_config: Some(messages::typed::OutputConfig {
			effort: None,
			format: None,
		}),
	};

	let (out, _) = super::from_messages::translate_internal(req, &provider, None, None).unwrap();
	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"thinking": {
				"type": "adaptive"
			},
			"top_k": 50,
			"output_config": {}
		}))
	);

	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, Some(0.7));
	assert_eq!(inference.top_p, Some(0.8));
}

#[test]
fn test_thinking_and_output_config_are_both_passed_through() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: None,
		top_k: None,
		top_p: None,
		tools: None,
		tool_choice: None,
		thinking: Some(messages::typed::ThinkingInput::Enabled {
			budget_tokens: 1024,
		}),
		output_config: Some(messages::typed::OutputConfig {
			effort: Some(messages::typed::ThinkingEffort::High),
			format: None,
		}),
	};

	let (out, _) = super::from_messages::translate_internal(req, &provider, None, None).unwrap();
	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": 1024
			},
			"output_config": {
				"effort": "high"
			}
		}))
	);
}

#[test]
fn test_adaptive_thinking_preserves_sampling_and_tool_choice() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: Some(0.7),
		top_k: Some(50),
		top_p: Some(0.8),
		tools: Some(vec![messages::typed::Tool::Custom(
			messages::typed::CustomTool {
				strict: None,
				name: "lookup".to_string(),
				description: Some("Lookup tool".to_string()),
				input_schema: json!({
					"type": "object",
					"properties": {
						"q": { "type": "string" }
					},
					"required": ["q"]
				}),
				cache_control: None,
			},
		)]),
		tool_choice: Some(messages::typed::ToolChoice::Tool {
			name: "lookup".to_string(),
			disable_parallel_tool_use: None,
		}),
		thinking: Some(messages::typed::ThinkingInput::Adaptive {}),
		output_config: None,
	};

	let (out, _) = super::from_messages::translate_internal(req, &provider, None, None).unwrap();
	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, Some(0.7));
	assert_eq!(inference.top_p, Some(0.8));

	let tool_choice = out
		.tool_config
		.as_ref()
		.and_then(|cfg| cfg.tool_choice.as_ref());
	assert!(matches!(
		tool_choice,
		Some(types::bedrock::ToolChoice::Tool { name }) if name == "lookup"
	));

	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"thinking": {
				"type": "adaptive"
			},
			"top_k": 50
		}))
	);
}

#[test]
fn test_enabled_thinking_applies_sampling_and_tool_choice_constraints() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: Some(0.7),
		top_k: Some(50),
		top_p: Some(0.8),
		tools: Some(vec![messages::typed::Tool::Custom(
			messages::typed::CustomTool {
				strict: None,
				name: "lookup".to_string(),
				description: Some("Lookup tool".to_string()),
				input_schema: json!({
					"type": "object",
					"properties": {
						"q": { "type": "string" }
					},
					"required": ["q"]
				}),
				cache_control: None,
			},
		)]),
		tool_choice: Some(messages::typed::ToolChoice::Auto {
			disable_parallel_tool_use: None,
		}),
		thinking: Some(messages::typed::ThinkingInput::Enabled {
			budget_tokens: 1024,
		}),
		output_config: None,
	};

	let (out, _) = super::from_messages::translate_internal(req, &provider, None, None).unwrap();
	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, None);
	assert_eq!(inference.top_p, None);

	let tool_choice = out
		.tool_config
		.as_ref()
		.and_then(|cfg| cfg.tool_choice.as_ref());
	assert!(matches!(tool_choice, Some(types::bedrock::ToolChoice::Any)));
}

#[test]
fn test_messages_image_url_to_bedrock_returns_error() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Image(
				messages::typed::ContentImageBlock {
					source: json!({
						"type": "url",
						"url": "https://example.com/sample.jpg"
					}),
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: None,
		top_k: None,
		top_p: None,
		tools: None,
		tool_choice: None,
		thinking: None,
		output_config: None,
	};

	let err = super::from_messages::translate_internal(req, &provider, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("URL image sources are unsupported")
	);
}

#[test]
fn test_completions_image_data_url_maps_to_converse_image_block() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::completions::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_tokens": 64,
		"messages": [{
			"role": "user",
			"content": [
				{ "type": "text", "text": "What is in this image?" },
				{
					"type": "image_url",
					"image_url": {
						"url": "data:image/jpeg;base64,/9j/4AAQSkZJRg=="
					}
				}
			]
		}]
	}))
	.expect("valid completions request");

	let translated = super::from_completions::translate(&req, &provider, None, None, None)
		.unwrap()
		.body;
	let translated: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	let content = translated["messages"][0]["content"]
		.as_array()
		.expect("user message content");
	assert_eq!(content[0]["text"], json!("What is in this image?"));
	assert_eq!(content[1]["image"]["format"], json!("jpeg"));
	assert_eq!(
		content[1]["image"]["source"]["bytes"],
		json!("/9j/4AAQSkZJRg==")
	);
}

#[test]
fn test_completions_image_url_to_bedrock_returns_error() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};
	let req: types::completions::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"messages": [{
			"role": "user",
			"content": [{
				"type": "image_url",
				"image_url": { "url": "https://example.com/sample.jpg" }
			}]
		}]
	}))
	.expect("valid completions request");

	let err = super::from_completions::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
}

#[test]
fn test_completions_request_metadata_only_uses_bedrock_header() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
		moderation: None,
		messages: vec![types::completions::typed::RequestMessage::User(
			types::completions::typed::RequestUserMessage {
				content: types::completions::typed::RequestUserMessageContent::Text("Hello".to_string()),
				name: None,
			},
		)],
		stream: None,
		temperature: None,
		top_p: None,
		max_completion_tokens: Some(16),
		stop: None,
		tools: None,
		tool_choice: None,
		parallel_tool_calls: None,
		user: Some("user456".to_string()),
		vendor_extensions: Default::default(),
		frequency_penalty: None,
		logit_bias: None,
		logprobs: None,
		top_logprobs: None,
		n: None,
		modalities: None,
		prediction: None,
		audio: None,
		presence_penalty: None,
		response_format: None,
		seed: None,
		#[allow(deprecated)]
		function_call: None,
		#[allow(deprecated)]
		functions: None,
		metadata: Some(json!({
			"user_id": "user123",
			"department": "engineering",
			"json_user": r#"{"device_id":"from-body"}"#,
			"nonstr": 123
		})),
		#[allow(deprecated)]
		max_tokens: None,
		service_tier: None,
		web_search_options: None,
		stream_options: None,
		store: None,
		reasoning_effort: None,
	};
	let mut headers = HeaderMap::new();
	headers.insert(
		"x-bedrock-metadata",
		r#"{"json_user": "{\"device_id\":\"from-header\"}", "bad?key": "bad{}"}"#
			.parse()
			.unwrap(),
	);

	let (out, _) = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		Some(&headers),
		None,
		None,
	)
	.unwrap();
	let md = out.request_metadata.unwrap();

	assert!(!md.contains_key("user_id"));
	assert!(!md.contains_key("department"));
	assert_eq!(
		md.get("json_user"),
		Some(&r#"{"device_id":"from-header"}"#.to_string())
	);
	assert_eq!(md.get("bad?key"), Some(&"bad{}".to_string()));
	assert!(!md.contains_key("nonstr"));
}

#[test]
fn test_completions_json_schema_response_format_maps_to_converse_output_config() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let schema = json!({
		"type": "object",
		"properties": {
			"summary": { "type": "string" }
		},
		"required": ["summary"],
		"additionalProperties": false
	});

	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
		moderation: None,
		messages: vec![types::completions::typed::RequestMessage::User(
			types::completions::typed::RequestUserMessage {
				content: types::completions::typed::RequestUserMessageContent::Text(
					"Summarize".to_string(),
				),
				name: None,
			},
		)],
		stream: None,
		temperature: None,
		top_p: None,
		max_completion_tokens: Some(16),
		stop: None,
		tools: None,
		tool_choice: None,
		parallel_tool_calls: None,
		user: None,
		vendor_extensions: Default::default(),
		frequency_penalty: None,
		logit_bias: None,
		logprobs: None,
		top_logprobs: None,
		n: None,
		modalities: None,
		prediction: None,
		audio: None,
		presence_penalty: None,
		response_format: Some(types::completions::typed::ResponseFormat::JsonSchema {
			json_schema: types::completions::typed::ResponseFormatJsonSchema {
				description: Some("Structured summary".to_string()),
				name: "summary_schema".to_string(),
				schema: schema.clone(),
				strict: Some(true),
			},
		}),
		seed: None,
		#[allow(deprecated)]
		function_call: None,
		#[allow(deprecated)]
		functions: None,
		metadata: None,
		#[allow(deprecated)]
		max_tokens: None,
		service_tier: None,
		web_search_options: None,
		stream_options: None,
		store: None,
		reasoning_effort: None,
	};

	let (out, _) = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
		None,
	)
	.unwrap();
	assert_eq!(
		out.output_config,
		Some(types::bedrock::OutputConfig {
			text_format: Some(types::bedrock::OutputFormat {
				r#type: types::bedrock::OutputFormatType::JsonSchema,
				structure: types::bedrock::OutputFormatStructure {
					json_schema: types::bedrock::JsonSchemaDefinition {
						schema: serde_json::to_string(&schema).unwrap(),
						name: Some("summary_schema".to_string()),
						description: Some("Structured summary".to_string()),
					},
				},
			}),
		})
	);
}

#[test]
fn test_completions_reasoning_effort_maps_to_enabled_thinking_budget() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
		moderation: None,
		messages: vec![types::completions::typed::RequestMessage::User(
			types::completions::typed::RequestUserMessage {
				content: types::completions::typed::RequestUserMessageContent::Text(
					"Deeply analyze this topic".to_string(),
				),
				name: None,
			},
		)],
		stream: None,
		temperature: None,
		top_p: None,
		max_completion_tokens: Some(64),
		stop: None,
		tools: None,
		tool_choice: None,
		parallel_tool_calls: None,
		user: None,
		vendor_extensions: Default::default(),
		frequency_penalty: None,
		logit_bias: None,
		logprobs: None,
		top_logprobs: None,
		n: None,
		modalities: None,
		prediction: None,
		audio: None,
		presence_penalty: None,
		response_format: None,
		seed: None,
		#[allow(deprecated)]
		function_call: None,
		#[allow(deprecated)]
		functions: None,
		metadata: None,
		#[allow(deprecated)]
		max_tokens: None,
		service_tier: None,
		web_search_options: None,
		stream_options: None,
		store: None,
		reasoning_effort: Some(types::completions::typed::ReasoningEffort::Xhigh),
	};

	let (out, _) = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
		None,
	)
	.unwrap();

	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": 8192
			}
		}))
	);
}

#[test]
fn test_completions_explicit_thinking_budget_forces_enabled_thinking() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
		moderation: None,
		messages: vec![types::completions::typed::RequestMessage::User(
			types::completions::typed::RequestUserMessage {
				content: types::completions::typed::RequestUserMessageContent::Text(
					"Deeply analyze this topic".to_string(),
				),
				name: None,
			},
		)],
		stream: None,
		temperature: None,
		top_p: None,
		max_completion_tokens: Some(64),
		stop: None,
		tools: None,
		tool_choice: None,
		parallel_tool_calls: None,
		user: None,
		vendor_extensions: types::completions::typed::RequestVendorExtensions {
			top_k: None,
			thinking_budget_tokens: Some(3072),
		},
		frequency_penalty: None,
		logit_bias: None,
		logprobs: None,
		top_logprobs: None,
		n: None,
		modalities: None,
		prediction: None,
		audio: None,
		presence_penalty: None,
		response_format: None,
		seed: None,
		#[allow(deprecated)]
		function_call: None,
		#[allow(deprecated)]
		functions: None,
		metadata: None,
		#[allow(deprecated)]
		max_tokens: None,
		service_tier: None,
		web_search_options: None,
		stream_options: None,
		store: None,
		reasoning_effort: Some(types::completions::typed::ReasoningEffort::High),
	};

	let (out, _) = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
		None,
	)
	.unwrap();

	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": 3072
			}
		}))
	);
}

#[test]
fn test_responses_request_metadata_only_uses_bedrock_header() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 16,
		"input": "Hello",
		"metadata": {
			"safe": "ok",
			"json_user": "{\"device_id\":\"from-body\"}"
		}
	}))
	.expect("valid responses request");

	let mut headers = HeaderMap::new();
	headers.insert(
		"x-bedrock-metadata",
		r#"{"json_user": "{\"device_id\":\"from-header\"}", "bad?key": "bad{}"}"#
			.parse()
			.unwrap(),
	);

	let translated = super::from_responses::translate(&req, &provider, Some(&headers), None, None)
		.unwrap()
		.body;
	let translated: serde_json::Value = serde_json::from_slice(&translated).unwrap();
	let metadata = translated["requestMetadata"]
		.as_object()
		.expect("requestMetadata object");

	assert!(!metadata.contains_key("safe"));
	assert_eq!(
		translated["requestMetadata"]["json_user"],
		r#"{"device_id":"from-header"}"#
	);
	assert_eq!(translated["requestMetadata"]["bad?key"], "bad{}");
}

#[test]
fn test_responses_reasoning_effort_maps_to_enabled_thinking_budget() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-5",
		"max_output_tokens": 64,
		"input": "Classify the intent.",
		"reasoning": {
			"effort": "high"
		}
	}))
	.expect("valid responses request");

	let translated = super::from_responses::translate(&req, &provider, None, None, None)
		.unwrap()
		.body;
	let translated: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(
		translated["additionalModelRequestFields"],
		json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": 4096
			}
		})
	);
}

#[test]
fn test_responses_explicit_thinking_budget_forces_enabled_thinking() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-5",
		"max_output_tokens": 64,
		"input": "Classify the intent.",
		"reasoning": {
			"effort": "high"
		},
		"vendor_extensions": {
			"thinking_budget_tokens": 3072
		}
	}))
	.expect("valid responses request");

	let translated = super::from_responses::translate(&req, &provider, None, None, None)
		.unwrap()
		.body;
	let translated: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(
		translated["additionalModelRequestFields"],
		json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": 3072
			}
		})
	);
}

#[test]
fn test_responses_vendor_extension_thinking_budget_forces_enabled_thinking() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-5",
		"max_output_tokens": 64,
		"input": "Classify the intent.",
		"vendor_extensions": {
			"thinking_budget_tokens": 3072
		}
	}))
	.expect("valid responses request");

	let translated = super::from_responses::translate(&req, &provider, None, None, None)
		.unwrap()
		.body;
	let translated: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(
		translated["additionalModelRequestFields"],
		json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": 3072
			}
		})
	);
}

#[test]
fn test_embeddings_translation_titan() {
	let req = types::embeddings::Request {
		model: Some("amazon.titan-embed-text-v2:0".to_string()),
		input: json!("hello world"),
		user: None,
		encoding_format: None,
		dimensions: Some(1024),
		rest: json!({}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let bedrock_req: bedrock::AmazonTitanV2EmbeddingRequest =
		serde_json::from_slice(&translated).unwrap();

	assert_eq!(bedrock_req.input_text, "hello world");
	assert_eq!(bedrock_req.dimensions, Some(1024));
}

#[test]
fn test_embeddings_titan_with_encoding_format() {
	let req = types::embeddings::Request {
		model: Some("amazon.titan-embed-text-v2:0".to_string()),
		input: json!("hello"),
		user: None,
		encoding_format: Some(types::embeddings::typed::EncodingFormat::Float),
		dimensions: None,
		rest: json!({"normalize": true}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let bedrock_req: bedrock::AmazonTitanV2EmbeddingRequest =
		serde_json::from_slice(&translated).unwrap();

	assert_eq!(bedrock_req.normalize, Some(true));
	assert!(
		matches!(&bedrock_req.embedding_types, Some(v) if v.len() == 1),
		"expected one embedding type"
	);
}

#[test]
fn test_embeddings_titan_rejects_array_input() {
	let req = types::embeddings::Request {
		model: Some("amazon.titan-embed-text-v2:0".to_string()),
		input: json!(["hello", "world"]),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({}),
	};

	assert!(
		from_embeddings::translate(&req).is_err(),
		"Titan should reject array input"
	);
}

#[test]
fn test_embeddings_cohere_with_passthrough_fields() {
	let req = types::embeddings::Request {
		model: Some("cohere.embed-english-v3".to_string()),
		input: json!(["hello", "world"]),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({"input_type": "search_document", "truncate": "END"}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let bedrock_req: bedrock::CohereEmbeddingRequest = serde_json::from_slice(&translated).unwrap();

	assert_eq!(bedrock_req.texts, vec!["hello", "world"]);
	assert_eq!(bedrock_req.input_type, "search_document");
	assert_eq!(bedrock_req.truncate, Some("END".to_string()));
	assert_eq!(bedrock_req.output_dimension, None);
}

#[test]
fn test_embeddings_translation_nova() {
	let req = types::embeddings::Request {
		model: Some("amazon.nova-2-multimodal-embeddings-v1:0".to_string()),
		input: json!("hello world"),
		user: None,
		encoding_format: None,
		dimensions: Some(1024),
		rest: json!({}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let bedrock_req: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(bedrock_req["taskType"], "SINGLE_EMBEDDING");
	let params = &bedrock_req["singleEmbeddingParams"];
	assert_eq!(params["embeddingPurpose"], "GENERIC_INDEX");
	assert_eq!(params["embeddingDimension"], 1024);
	assert_eq!(params["text"]["truncationMode"], "END");
	assert_eq!(params["text"]["value"], "hello world");
}

#[test]
fn test_embeddings_nova_omits_dimension_when_unset() {
	let req = types::embeddings::Request {
		model: Some("amazon.nova-2-multimodal-embeddings-v1:0".to_string()),
		input: json!("hello"),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let bedrock_req: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert!(
		bedrock_req["singleEmbeddingParams"]
			.get("embeddingDimension")
			.is_none(),
		"embeddingDimension should be omitted so the model default applies"
	);
}

#[test]
fn test_embeddings_nova_with_passthrough_fields() {
	let req = types::embeddings::Request {
		model: Some("amazon.nova-2-multimodal-embeddings-v1:0".to_string()),
		input: json!("hello"),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({"embedding_purpose": "GENERIC_RETRIEVAL", "truncation_mode": "NONE"}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let bedrock_req: bedrock::NovaEmbeddingRequest = serde_json::from_slice(&translated).unwrap();

	assert_eq!(
		bedrock_req.single_embedding_params.embedding_purpose,
		"GENERIC_RETRIEVAL"
	);
	assert_eq!(
		bedrock_req.single_embedding_params.text.truncation_mode,
		"NONE"
	);
}

#[test]
fn test_embeddings_nova_rejects_array_input() {
	let req = types::embeddings::Request {
		model: Some("amazon.nova-2-multimodal-embeddings-v1:0".to_string()),
		input: json!(["hello", "world"]),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({}),
	};

	assert!(
		from_embeddings::translate(&req).is_err(),
		"Nova should reject array input"
	);
}

#[test]
fn test_embeddings_rejects_invalid_input() {
	for input in [json!(["hello", 42]), json!(42)] {
		let req = types::embeddings::Request {
			model: Some("cohere.embed-english-v3".to_string()),
			input,
			user: None,
			encoding_format: None,
			dimensions: None,
			rest: json!({}),
		};
		assert!(from_embeddings::translate(&req).is_err());
	}
}

#[test]
fn test_embeddings_response_translation_titan() {
	let model = "amazon.titan-embed-text-v2:0";
	let bedrock_resp = json!({
		"embedding": [0.1, 0.2, 0.3],
		"inputTextTokenCount": 3
	});
	let bytes = serde_json::to_vec(&bedrock_resp).unwrap();
	let headers = HeaderMap::new();

	let translated = from_embeddings::translate_response(&bytes, &headers, model).unwrap();
	let openai_resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::Response>(&b))
		.unwrap();

	assert_eq!(openai_resp.object, "list");
	assert_eq!(openai_resp.usage.unwrap().prompt_tokens, 3);
}

#[test]
fn test_embeddings_response_titan_embeddings_by_type_fallback() {
	let model = "amazon.titan-embed-text-v2:0";
	let bedrock_resp = json!({
		"embeddingsByType": {
			"float": [0.4, 0.5, 0.6]
		},
		"inputTextTokenCount": 5
	});
	let bytes = serde_json::to_vec(&bedrock_resp).unwrap();
	let headers = HeaderMap::new();

	let translated = from_embeddings::translate_response(&bytes, &headers, model).unwrap();
	let openai_resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::Response>(&b))
		.unwrap();

	assert_eq!(openai_resp.usage.unwrap().prompt_tokens, 5);
}

#[test]
fn test_embeddings_response_translation_cohere() {
	let model = "cohere.embed-english-v3";
	let bedrock_resp = json!({
		"embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]],
		"id": "123",
		"texts": ["hello", "world"]
	});
	let bytes = serde_json::to_vec(&bedrock_resp).unwrap();
	let mut headers = HeaderMap::new();
	headers.insert("x-amzn-bedrock-input-token-count", "10".parse().unwrap());

	let translated = from_embeddings::translate_response(&bytes, &headers, model).unwrap();
	let openai_resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::typed::Response>(&b))
		.unwrap();

	assert_eq!(openai_resp.object, "list");
	assert_eq!(
		openai_resp.data[0].embedding,
		vec![0.1_f32, 0.2_f32, 0.3_f32]
	);
	assert_eq!(
		openai_resp.data[1].embedding,
		vec![0.4_f32, 0.5_f32, 0.6_f32]
	);
	assert_eq!(openai_resp.usage.prompt_tokens, 10);
}

#[test]
fn test_embeddings_response_translation_cohere_v4_uses_float_vectors() {
	let model = "cohere.embed-v4:0";
	let bedrock_resp = json!({
		"embeddings": {
			"float": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]],
			"int8": [[1, 2, 3], [4, 5, 6]]
		},
		"id": "123",
		"texts": ["hello", "world"]
	});
	let bytes = serde_json::to_vec(&bedrock_resp).unwrap();
	let headers = HeaderMap::new();

	let translated = from_embeddings::translate_response(&bytes, &headers, model).unwrap();
	let openai_resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::typed::Response>(&b))
		.unwrap();

	assert_eq!(
		openai_resp.data[0].embedding,
		vec![0.1_f32, 0.2_f32, 0.3_f32]
	);
	assert_eq!(
		openai_resp.data[1].embedding,
		vec![0.4_f32, 0.5_f32, 0.6_f32]
	);
}

#[test]
fn test_embeddings_response_translation_cohere_v4_requires_float_vectors() {
	let model = "cohere.embed-v4:0";
	let bedrock_resp = json!({
		"embeddings": {
			"uint8": [[1, 2, 3]],
			"int8": [[-1, 0, 1]]
		},
		"id": "123",
		"texts": ["hello"]
	});
	let bytes = serde_json::to_vec(&bedrock_resp).unwrap();
	let headers = HeaderMap::new();

	let err = match from_embeddings::translate_response(&bytes, &headers, model) {
		Ok(_) => panic!("expected a response without float embeddings to fail"),
		Err(err) => err,
	};

	assert!(matches!(err, crate::AIError::ResponseParsing(_)));
	assert!(
		err
			.to_string()
			.contains("Cohere response did not include float embeddings; received types: int8, uint8")
	);
}

#[test]
fn test_embeddings_response_translation_nova() {
	let model = "amazon.nova-2-multimodal-embeddings-v1:0";
	let bedrock_resp = json!({
		"embeddings": [{"embedding": [0.25, 0.5, -0.75], "embeddingType": "TEXT"}]
	});
	let bytes = serde_json::to_vec(&bedrock_resp).unwrap();
	let mut headers = HeaderMap::new();
	headers.insert("x-amzn-bedrock-input-token-count", "7".parse().unwrap());

	let translated = from_embeddings::translate_response(&bytes, &headers, model).unwrap();
	let openai_resp: serde_json::Value = translated
		.serialize()
		.and_then(|b| serde_json::from_slice(&b))
		.unwrap();

	assert_eq!(openai_resp["object"], "list");
	assert_eq!(openai_resp["data"][0]["object"], "embedding");
	assert_eq!(openai_resp["data"][0]["index"], 0);
	assert_eq!(
		openai_resp["data"][0]["embedding"],
		json!([0.25, 0.5, -0.75])
	);
	assert_eq!(openai_resp["usage"]["prompt_tokens"], 7);
	assert_eq!(openai_resp["usage"]["total_tokens"], 7);
}

#[test]
fn test_embeddings_error_translation() {
	let error_body =
		bytes::Bytes::from(serde_json::to_vec(&json!({"message": "Model not found"})).unwrap());

	let translated = from_embeddings::translate_error(&error_body).unwrap();
	let error_resp: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(error_resp["error"]["type"], "invalid_request_error");
	assert_eq!(error_resp["error"]["message"], "Model not found");
}

#[test]
fn test_completions_error_translation_wraps_non_json_body() {
	let error_body = Bytes::from_static(
		b"<html><body><center>The plain HTTP request was sent to HTTPS port</center></body></html>",
	);

	let translated = from_completions::translate_error(&error_body).unwrap();
	let error_resp: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(error_resp["error"]["type"], "invalid_request_error");
	assert!(
		error_resp["error"]["message"]
			.as_str()
			.unwrap()
			.contains("plain HTTP request")
	);
}

fn make_message(role: types::bedrock::Role, text: &str) -> types::bedrock::Message {
	types::bedrock::Message {
		role,
		content: vec![types::bedrock::ContentBlock::Text(text.to_string())],
	}
}

fn has_cache_point(msg: &types::bedrock::Message) -> bool {
	msg
		.content
		.iter()
		.any(|b| matches!(b, types::bedrock::ContentBlock::CachePoint(_)))
}

#[test]
fn test_insert_cache_point_default_offset() {
	let mut msgs = vec![
		make_message(types::bedrock::Role::User, "Hello"),
		make_message(types::bedrock::Role::Assistant, "Hi"),
		make_message(types::bedrock::Role::User, "How are you?"),
	];
	helpers::insert_message_cache_point(&mut msgs, 0);
	assert!(has_cache_point(&msgs[1]));
	assert!(!has_cache_point(&msgs[0]));
	assert!(!has_cache_point(&msgs[2]));
}

#[test]
fn test_insert_cache_point_offset_shifts_back() {
	let mut msgs = vec![
		make_message(types::bedrock::Role::User, "a"),
		make_message(types::bedrock::Role::Assistant, "b"),
		make_message(types::bedrock::Role::User, "c"),
		make_message(types::bedrock::Role::Assistant, "d"),
		make_message(types::bedrock::Role::User, "e"),
	];
	helpers::insert_message_cache_point(&mut msgs, 2);
	// default position is index 3 (len-2), offset 2 → index 1
	assert!(has_cache_point(&msgs[1]));
	for (i, msg) in msgs.iter().enumerate() {
		if i != 1 {
			assert!(!has_cache_point(msg));
		}
	}
}

#[test]
fn test_insert_cache_point_offset_clamps_to_zero() {
	let mut msgs = vec![
		make_message(types::bedrock::Role::User, "a"),
		make_message(types::bedrock::Role::Assistant, "b"),
		make_message(types::bedrock::Role::User, "c"),
	];
	// offset 100 should clamp to index 0
	helpers::insert_message_cache_point(&mut msgs, 100);
	assert!(has_cache_point(&msgs[0]));
	assert!(!has_cache_point(&msgs[1]));
	assert!(!has_cache_point(&msgs[2]));
}

#[test]
fn test_insert_cache_point_single_message_noop() {
	let mut msgs = vec![make_message(types::bedrock::Role::User, "only")];
	helpers::insert_message_cache_point(&mut msgs, 0);
	assert!(!has_cache_point(&msgs[0]));
}

#[test]
fn test_insert_cache_point_empty_messages_noop() {
	let mut msgs: Vec<types::bedrock::Message> = vec![];
	helpers::insert_message_cache_point(&mut msgs, 0);
	assert!(msgs.is_empty());
}

#[test]
fn test_bedrock_tool_name_sanitizes_long_mcp_names() {
	let long_name = "mcp__plugin_atlassian_atlassian__createCompassComponentRelationship";
	assert!(long_name.len() > super::BEDROCK_TOOL_NAME_MAX_LEN);

	let mut map = super::BedrockToolNameMap::default();
	let sanitized = map.register(long_name);

	assert!(sanitized.len() <= super::BEDROCK_TOOL_NAME_MAX_LEN);
	assert!(
		sanitized
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
	);
	assert_eq!(map.restore(&sanitized), long_name);
}

#[test]
fn test_bedrock_tool_name_preserves_valid_short_names() {
	let mut map = super::BedrockToolNameMap::default();
	let name = "get_weather";
	assert_eq!(map.register(name), name);
	assert_eq!(map.restore(name), name);
	assert!(map.is_empty());
}

#[test]
fn test_bedrock_tool_name_sanitizes_invalid_characters() {
	let mut map = super::BedrockToolNameMap::default();
	let sanitized = map.register("my.tool/name");
	assert_eq!(sanitized, "my_tool_name");
}

#[test]
fn test_messages_long_tool_names_fit_bedrock_tool_config() {
	use types::messages::typed as messages;

	let long_name = "mcp__plugin_atlassian_atlassian__createCompassComponentRelationship";
	let provider = Provider {
		model_override: None,
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::Request {
		model: "anthropic.claude-sonnet-4-20250514-v1:0".to_string(),
		max_tokens: 1024,
		messages: vec![messages::Message {
			role: messages::Role::User,
			content: vec![messages::ContentBlock::Text(messages::ContentTextBlock {
				text: "hello".to_string(),
				citations: None,
				cache_control: None,
			})],
		}],
		tools: Some(vec![messages::Tool::Custom(messages::CustomTool {
			strict: None,
			name: long_name.to_string(),
			description: Some("test".to_string()),
			input_schema: serde_json::json!({"type": "object"}),
			cache_control: None,
		})]),
		tool_choice: None,
		system: None,
		metadata: None,
		stop_sequences: vec![],
		stream: false,
		temperature: None,
		top_p: None,
		top_k: None,
		thinking: None,
		output_config: None,
	};

	let (out, tool_map) =
		super::from_messages::translate_internal(req, &provider, None, None).unwrap();
	let bedrock_name = out
		.tool_config
		.as_ref()
		.and_then(|tc| tc.tools.first())
		.and_then(|tool| match tool {
			types::bedrock::Tool::ToolSpec(spec) => Some(spec.name.clone()),
			_ => None,
		})
		.expect("tool spec");

	assert!(bedrock_name.len() <= super::BEDROCK_TOOL_NAME_MAX_LEN);
	assert_eq!(tool_map.restore(&bedrock_name), long_name);
}

#[test]
fn test_messages_long_tool_name_round_trip_response() {
	use types::messages::typed as messages;

	let long_name = "mcp__plugin_atlassian_atlassian__createCompassComponentRelationship";
	let provider = Provider {
		model_override: None,
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req = messages::Request {
		model: "anthropic.claude-sonnet-4-20250514-v1:0".to_string(),
		max_tokens: 1024,
		messages: vec![messages::Message {
			role: messages::Role::User,
			content: vec![messages::ContentBlock::Text(messages::ContentTextBlock {
				text: "hello".to_string(),
				citations: None,
				cache_control: None,
			})],
		}],
		tools: Some(vec![messages::Tool::Custom(messages::CustomTool {
			strict: None,
			name: long_name.to_string(),
			description: Some("test".to_string()),
			input_schema: serde_json::json!({"type": "object"}),
			cache_control: None,
		})]),
		tool_choice: None,
		system: None,
		metadata: None,
		stop_sequences: vec![],
		stream: false,
		temperature: None,
		top_p: None,
		top_k: None,
		thinking: None,
		output_config: None,
	};

	let (bedrock_req, tool_map) =
		super::from_messages::translate_internal(req, &provider, None, None).unwrap();
	let bedrock_name = bedrock_req
		.tool_config
		.as_ref()
		.and_then(|tc| tc.tools.first())
		.and_then(|tool| match tool {
			types::bedrock::Tool::ToolSpec(spec) => Some(spec.name.clone()),
			_ => None,
		})
		.expect("sanitized tool name");
	let model = "anthropic.claude-sonnet-4-20250514-v1:0";

	let bedrock_response = json!({
		"output": {
			"message": {
				"role": "assistant",
				"content": [{
					"toolUse": {
						"toolUseId": "toolu_01TestRoundTrip",
						"name": bedrock_name,
						"input": {"query": "test"}
					}
				}]
			}
		},
		"stopReason": "tool_use",
		"usage": {
			"inputTokens": 100,
			"outputTokens": 20,
			"totalTokens": 120
		}
	});
	let bytes = Bytes::from(serde_json::to_vec(&bedrock_response).unwrap());

	let response = super::from_messages::translate_response(&bytes, model, Some(&tool_map)).unwrap();
	let response_json: serde_json::Value =
		serde_json::from_slice(&response.serialize().unwrap()).unwrap();
	let tool_use_name = response_json["content"]
		.as_array()
		.and_then(|blocks| {
			blocks.iter().find_map(|block| {
				block
					.get("name")
					.and_then(|n| n.as_str())
					.filter(|_| block.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
			})
		})
		.expect("tool use block in response");

	assert_eq!(tool_use_name, long_name);
}

#[test]
fn test_responses_assistant_input_image_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "assistant",
			"content": [{
				"type": "input_image",
				"image_url": "data:image/png;base64,iVBORw0KGgo=",
				"detail": "auto"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("image inputs are only supported on user messages")
	);
}

#[test]
fn test_responses_input_image_remote_url_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"image_url": "https://example.com/sample.png",
				"detail": "auto"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("remote URLs and file_ids are unsupported")
	);
}

#[test]
fn test_responses_input_image_non_base64_data_url_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"image_url": "data:image/png,iVBORw0KGgo=",
				"detail": "auto"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("image data URLs must be base64-encoded")
	);
}

#[test]
fn test_responses_input_image_non_image_data_url_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"image_url": "data:application/octet-stream;base64,iVBORw0KGgo=",
				"detail": "auto"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("image data URLs must use a non-empty image/* media type")
	);
}

#[test]
fn test_responses_input_image_empty_media_type_data_url_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"image_url": "data:;base64,iVBORw0KGgo=",
				"detail": "auto"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("image data URLs must use a non-empty image/* media type")
	);
}

#[test]
fn test_responses_input_image_file_id_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_image",
				"file_id": "file-abc123",
				"detail": "auto"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
}

#[test]
fn test_responses_system_input_file_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"type": "message",
			"role": "system",
			"content": [{
				"type": "input_file",
				"file_id": "file-abc123"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("bedrock document inputs are only supported on user messages"),
		"unexpected error: {err}"
	);
}

#[test]
fn test_responses_input_file_id_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_file",
				"file_id": "file-abc123"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err.to_string().contains("file_id is unsupported"),
		"unexpected error: {err}"
	);
}

#[test]
fn test_responses_input_file_remote_url_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_file",
				"file_url": "https://example.com/report.pdf",
				"filename": "report.pdf"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err.to_string().contains("remote URLs are unsupported"),
		"unexpected error: {err}"
	);
}

#[test]
fn test_responses_input_file_unknown_format_is_rejected() {
	let provider = Provider {
		model_override: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
		endpoint_preference: Default::default(),
		runtime_anthropic_api: Default::default(),
	};

	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": [{
			"role": "user",
			"content": [{
				"type": "input_file",
				"file_data": "data:application/octet-stream;base64,dGVzdA==",
				"filename": "archive.zip"
			}]
		}]
	}))
	.expect("valid responses request");

	let err = super::from_responses::translate(&req, &provider, None, None, None).unwrap_err();
	assert!(matches!(err, crate::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("document format could not be determined"),
		"unexpected error: {err}"
	);
}

// ── from_messages_invoke tests ────────────────────────────────────────────────
//
// A faithful port of router0's internal/bedrock/sanitize_test.go, asserted on the body
// translate_request emits (it returns only the bytes, so router0's dropped-list checks become
// observable output checks). preserve_order is on workspace-wide, so byte-for-byte survival is
// checked by substring. The agentgateway-only cases (defer_loading, PDF documents, streaming)
// follow at the end.

// Measured capability rows are matched by substring on the model id (bedrock.rs capability_for).
const HAIKU: &str = "global.anthropic.claude-haiku-4-5-20251001-v1:0";
const SONNET46: &str = "global.anthropic.claude-sonnet-4-6";
const OPUS48: &str = "global.anthropic.claude-opus-4-8";
const SONNET55: &str = "global.anthropic.claude-sonnet-5-5";
const OPUS55: &str = "global.anthropic.claude-opus-5-5";

fn invoke(model: &str, body: serde_json::Value) -> serde_json::Value {
	invoke_betas(model, body, &[])
}

fn invoke_betas(model: &str, body: serde_json::Value, betas: &[&str]) -> serde_json::Value {
	let mut headers = HeaderMap::new();
	for b in betas {
		headers.append("anthropic-beta", b.parse().unwrap());
	}
	let req = types::ChatRequest::Messages(serde_json::from_value(body).unwrap());
	let out = from_messages_invoke::translate_request(req, &headers, model).unwrap();
	serde_json::from_slice(&out).unwrap()
}

fn betas_of(v: &serde_json::Value) -> Vec<String> {
	v.get("anthropic_beta")
		.and_then(|b| b.as_array())
		.map(|a| {
			a.iter()
				.filter_map(|b| b.as_str().map(str::to_string))
				.collect()
		})
		.unwrap_or_default()
}

fn raw(v: &serde_json::Value) -> String {
	serde_json::to_string(v).unwrap()
}

fn simple() -> serde_json::Value {
	json!({"max_tokens": 50, "messages": [{"role": "user", "content": "hi"}]})
}

/// A tagged system-reminder text block inside a message, if the message carries one.
fn reminder_text(msg: &serde_json::Value) -> Option<String> {
	msg["content"].as_array()?.iter().find_map(|b| {
		b.get("text")
			.and_then(|t| t.as_str())
			.filter(|t| t.contains("<system-reminder>"))
			.map(str::to_string)
	})
}

/// user "go", assistant tool_use, system (cached) "remember", user tool_result for the tool_use.
fn mid_system() -> serde_json::Value {
	json!({
		"max_tokens": 50,
		"messages": [
			{"role": "user", "content": "go"},
			{"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "X", "input": {}}]},
			{"role": "system", "content": [{"type": "text", "text": "remember", "cache_control": {"type": "ephemeral"}}]},
			{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}
		]
	})
}

#[test]
fn invoke_envelope_matches_claude_codes_bedrock_mode() {
	// model and stream come from the URL on InvokeModel; the version is fixed; the betas keep order.
	let v = invoke_betas(
		SONNET55,
		json!({
			"model": "anthropic.claude-sonnet-5-5",
			"stream": true,
			"max_tokens": 50,
			"messages": [{"role": "user", "content": "hi"}]
		}),
		&[
			"claude-code-20250219,interleaved-thinking-2025-05-14",
			"effort-2025-11-24",
		],
	);
	assert!(
		v.get("model").is_none(),
		"model must be removed for InvokeModel"
	);
	assert!(
		v.get("stream").is_none(),
		"stream must be removed for InvokeModel"
	);
	assert_eq!(v["anthropic_version"], "bedrock-2023-05-31");
	assert_eq!(
		betas_of(&v),
		[
			"claude-code-20250219",
			"interleaved-thinking-2025-05-14",
			"effort-2025-11-24"
		]
		.map(String::from)
	);
}

#[test]
fn invoke_no_betas_means_no_field() {
	let v = invoke(SONNET55, simple());
	assert!(v.get("anthropic_beta").is_none());
}

#[test]
fn invoke_drops_fields_bedrock_rejects() {
	// InvokeModel closes the top-level schema (unknown keys 400 "Extra inputs are not permitted");
	// sampling and stop_sequences pass through.
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"messages": [{"role": "user", "content": "hi"}],
			"container": "c",
			"mcp_servers": [],
			"service_tier": "auto",
			"temperature": 1,
			"top_k": 4,
			"stop_sequences": ["x"]
		}),
	);
	for k in ["container", "mcp_servers", "service_tier"] {
		assert!(v.get(k).is_none(), "{k} must be dropped");
	}
	for k in ["temperature", "top_k", "stop_sequences"] {
		assert!(v.get(k).is_some(), "{k} must pass through");
	}
}

#[test]
fn invoke_metadata_keeps_only_user_id() {
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"metadata": {"user_id": "u-123", "session": "s-1", "trace": "t-1"},
			"messages": [{"role": "user", "content": "hi"}]
		}),
	);
	assert_eq!(v["metadata"], json!({"user_id": "u-123"}));
}

#[test]
fn invoke_filters_betas_per_model() {
	// Only accepted betas survive; dangerous-tool-use is accepted but field-coupled, so it is
	// dropped here (no safeguards field). Sonnet 4.6 additionally rejects the claude46 betas.
	let sent = "claude-code-20250219,dangerous-tool-use-2026-09-03,prompt-caching-scope-2026-01-05,thinking-display-updates-2026-08-18,tool-search-tool-2025-10-19,made-up-2099-01-01";
	assert_eq!(
		betas_of(&invoke_betas(SONNET55, simple(), &[sent])),
		[
			"claude-code-20250219",
			"thinking-display-updates-2026-08-18",
			"tool-search-tool-2025-10-19"
		]
		.map(String::from)
	);
	assert_eq!(
		betas_of(&invoke_betas(SONNET46, simple(), &[sent])),
		["claude-code-20250219", "tool-search-tool-2025-10-19"].map(String::from)
	);
}

#[test]
fn invoke_renames_advanced_tool_use_to_tool_search() {
	// Bedrock rejects the advanced-tool-use umbrella beta and takes its tool-search part.
	let v = invoke_betas(OPUS55, simple(), &["advanced-tool-use-2025-11-20"]);
	assert_eq!(
		betas_of(&v),
		["tool-search-tool-2025-10-19"].map(String::from)
	);
}

#[test]
fn invoke_every_rejected_beta_is_otherwise_accepted() {
	// Each per-model-rejected beta is in the shared accepted set, so a capable model keeps it while
	// Sonnet 4.6 drops it (router0 TestEveryRejectedBetaIsAlsoKnownAsAccepted). dangerous-tool-use
	// is excluded: it is field-coupled and never travels without a safeguards field.
	for b in [
		"thinking-display-updates-2026-08-18",
		"thinking-binding-controls-2026-08-01",
		"inline-tools-2026-09-15",
		"mid-conversation-system-clear-at-2026-08-21",
	] {
		assert!(
			betas_of(&invoke_betas(SONNET55, simple(), &[b])).contains(&b.to_string()),
			"{b} must be kept on a capable model"
		);
		assert!(
			!betas_of(&invoke_betas(SONNET46, simple(), &[b])).contains(&b.to_string()),
			"{b} must be rejected on Sonnet 4.6"
		);
	}
}

#[test]
fn invoke_dangerous_tool_use_beta_travels_only_with_safeguards() {
	// Without a safeguards field the beta is dropped even on models that accept the field.
	for target in [HAIKU, OPUS48, SONNET55, OPUS55] {
		let v = invoke_betas(
			target,
			simple(),
			&["claude-code-20250219,dangerous-tool-use-2026-09-03"],
		);
		assert!(
			!betas_of(&v).contains(&"dangerous-tool-use-2026-09-03".to_string()),
			"{target}: dangerous-tool-use must not travel without safeguards"
		);
		assert!(betas_of(&v).contains(&"claude-code-20250219".to_string()));
	}
}

#[test]
fn invoke_safeguards_stay_with_their_beta_where_supported() {
	let body = json!({
		"max_tokens": 50,
		"messages": [{"role": "user", "content": "hi"}],
		"safeguards": {"auto_mode": true}
	});
	// Kept, and the beta added even if the client forgot it.
	for target in [HAIKU, OPUS48, SONNET55, OPUS55] {
		let v = invoke(target, body.clone());
		assert!(
			v.get("safeguards").is_some(),
			"{target}: safeguards must stay"
		);
		assert!(
			betas_of(&v).contains(&"dangerous-tool-use-2026-09-03".to_string()),
			"{target}: the dangerous-tool-use beta must be added for the field"
		);
	}
	// Sonnet 4.6 does not take the field; it goes, and its orphaned beta with it.
	let v = invoke_betas(SONNET46, body, &["dangerous-tool-use-2026-09-03"]);
	assert!(v.get("safeguards").is_none());
	assert!(v.get("anthropic_beta").is_none());
}

#[test]
fn invoke_signed_thinking_and_untouched_blocks_keep_their_bytes() {
	// Blocks Bedrock accepts are re-encoded verbatim (preserve_order), including a signed thinking
	// block, a Claude redacted_thinking block, a tool_use, and odd characters that must not be
	// HTML-escaped (router0 TestSignedThinkingReplaysByteForByte / TestUntouchedBlocksKeepTheirBytes).
	use base64::Engine;
	let claude = base64::prelude::BASE64_STANDARD.encode("claude-opaque");
	let signed =
		json!({"type": "thinking", "thinking": "weigh <a> & <b>", "signature": "EuYBCkQ4Zm9v==abc<>&"});
	let redacted = json!({"type": "redacted_thinking", "data": claude});
	let tool_use =
		json!({"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"path": "a<b>"}});
	let body = json!({
		"max_tokens": 50,
		"messages": [
			{"role": "user", "content": "go"},
			{"role": "assistant", "content": [signed.clone(), redacted.clone(), tool_use.clone()]},
			{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"}]}
		]
	});
	for target in [HAIKU, SONNET46, OPUS48, SONNET55, OPUS55] {
		let s = raw(&invoke(target, body.clone()));
		for want in [&signed, &redacted, &tool_use] {
			assert!(
				s.contains(&raw(want)),
				"{target}: a passed-through block changed or was dropped"
			);
		}
	}
}

#[test]
fn invoke_removes_only_foreign_and_unsigned_reasoning() {
	// Unsigned thinking and a GPT redacted_thinking (data decodes to an "rsn_" prefix) are both
	// rejected by Bedrock and dropped; a signed block and a Claude redacted block stay. router0
	// also strips a synthetic unsigned placeholder, which this Claude-only path never produces.
	use base64::Engine;
	let gpt = base64::prelude::BASE64_STANDARD.encode("rsn_abcdef");
	let claude = base64::prelude::BASE64_STANDARD.encode("claude-opaque");
	let v = invoke(
		OPUS55,
		json!({
			"max_tokens": 50,
			"messages": [
				{"role": "user", "content": "go"},
				{"role": "assistant", "content": [
					{"type": "redacted_thinking", "data": gpt},
					{"type": "thinking", "thinking": "forged", "signature": ""},
					{"type": "thinking", "thinking": "no sig key at all"},
					{"type": "redacted_thinking", "data": claude.clone()},
					{"type": "text", "text": "answer"}
				]}
			]
		}),
	);
	let content = v["messages"][1]["content"].as_array().unwrap();
	assert_eq!(
		content.len(),
		2,
		"only the Claude redacted block and the text survive"
	);
	assert_eq!(content[0]["data"], claude);
	assert_eq!(content[1]["type"], "text");
}

#[test]
fn invoke_drops_assistant_turn_left_empty() {
	// An assistant turn whose only block is dropped must not reach Bedrock as an empty message.
	use base64::Engine;
	let gpt = base64::prelude::BASE64_STANDARD.encode("rsn_abc");
	let v = invoke(
		OPUS55,
		json!({
			"max_tokens": 50,
			"messages": [
				{"role": "user", "content": "a"},
				{"role": "assistant", "content": [{"type": "redacted_thinking", "data": gpt}]},
				{"role": "user", "content": "b"}
			]
		}),
	);
	assert_eq!(v["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn invoke_drops_empty_text_keeps_whitespace() {
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"messages": [
				{"role": "user", "content": [
					{"type": "text", "text": ""},
					{"type": "text", "text": "  \n"},
					{"type": "text", "text": "real"}
				]},
				{"role": "user", "content": [
					{"type": "tool_result", "tool_use_id": "t", "content": [
						{"type": "text", "text": ""},
						{"type": "text", "text": "out"}
					]}
				]}
			]
		}),
	);
	let msgs = v["messages"].as_array().unwrap();
	let user0 = msgs[0]["content"].as_array().unwrap();
	assert_eq!(user0.len(), 2, "empty text goes, whitespace stays");
	assert_eq!(user0[0]["text"], "  \n");
	assert_eq!(
		msgs[1]["content"][0]["content"].as_array().unwrap().len(),
		1,
		"empty tool_result part goes"
	);
}

#[test]
fn invoke_drops_empty_string_message() {
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"messages": [
				{"role": "user", "content": "a"},
				{"role": "assistant", "content": ""},
				{"role": "user", "content": "b"}
			]
		}),
	);
	assert_eq!(v["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn invoke_repairs_non_object_tool_use_input() {
	for input in [json!([]), json!("paris"), json!(null), json!(3)] {
		let v = invoke(
			OPUS55,
			json!({
				"max_tokens": 50,
				"messages": [
					{"role": "user", "content": "a"},
					{"role": "assistant", "content": [{"type": "tool_use", "id": "t", "name": "X", "input": input.clone()}]},
					{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t", "content": "ok"}]}
				]
			}),
		);
		assert_eq!(
			v["messages"][1]["content"][0]["input"],
			json!({}),
			"a non-object tool_use input ({input}) must become an empty object"
		);
	}
}

#[test]
fn invoke_moves_leading_system_into_system() {
	let body = json!({
		"max_tokens": 50,
		"system": [{"type": "text", "text": "top"}],
		"messages": [
			{"role": "system", "content": [{"type": "text", "text": "lead", "cache_control": {"type": "ephemeral"}}]},
			{"role": "user", "content": "hi"}
		]
	});
	for target in [HAIKU, SONNET55] {
		let v = invoke(target, body.clone());
		let sys = v["system"].as_array().unwrap();
		assert_eq!(
			sys.len(),
			2,
			"{target}: the leading system message appends to system"
		);
		assert_eq!(sys[1]["text"], "lead");
		assert!(
			sys[1]["cache_control"].is_object(),
			"{target}: the cache marker travels with it"
		);
		assert_eq!(v["messages"].as_array().unwrap().len(), 1);
	}
}

#[test]
fn invoke_mid_conversation_system_native_where_supported() {
	for target in [OPUS48, SONNET55, OPUS55] {
		let v = invoke(target, mid_system());
		let msgs = v["messages"].as_array().unwrap();
		assert_eq!(
			msgs.len(),
			4,
			"{target}: mid-conversation system stays its own turn"
		);
		assert_eq!(msgs[2]["role"], "system");
	}
}

#[test]
fn invoke_mid_conversation_system_fallback_keeps_position_and_cache() {
	// Models with no mid-conversation system role fold the text into the next user turn, after the
	// tool_result that must follow the tool_use, carrying the cache marker.
	for target in [HAIKU, SONNET46] {
		let v = invoke(target, mid_system());
		let msgs = v["messages"].as_array().unwrap();
		assert_eq!(msgs.len(), 3);
		assert_eq!(msgs[2]["role"], "user");
		let blocks = msgs[2]["content"].as_array().unwrap();
		assert_eq!(blocks.len(), 2);
		assert_eq!(blocks[0]["type"], "tool_result");
		assert!(
			blocks[1]["text"]
				.as_str()
				.unwrap()
				.contains("<system-reminder>")
		);
		assert!(blocks[1]["text"].as_str().unwrap().contains("remember"));
		assert!(blocks[1]["cache_control"].is_object());
		assert!(v.get("system").is_none());
	}
}

#[test]
fn invoke_mid_conversation_system_branches() {
	let roles = |v: &serde_json::Value| {
		v["messages"]
			.as_array()
			.unwrap()
			.iter()
			.map(|m| m["role"].as_str().unwrap().to_string())
			.collect::<Vec<_>>()
			.join(",")
	};
	let run = |target: &str, msgs: serde_json::Value| {
		invoke(target, json!({"max_tokens": 50, "messages": msgs}))
	};

	let developer = json!([
		{"role": "user", "content": "go"},
		{"role": "assistant", "content": "ok"},
		{"role": "developer", "content": "note"},
		{"role": "user", "content": "next"}
	]);
	let plain = json!([
		{"role": "user", "content": "go"},
		{"role": "assistant", "content": [{"type": "tool_use", "id": "t", "name": "X", "input": {}}]},
		{"role": "system", "content": "note"},
		{"role": "user", "content": "then"}
	]);
	let before = json!([
		{"role": "user", "content": "go"},
		{"role": "system", "content": "note"},
		{"role": "assistant", "content": "ok"}
	]);
	let last = json!([
		{"role": "user", "content": "go"},
		{"role": "assistant", "content": "ok"},
		{"role": "system", "content": "note"}
	]);

	// Native: developer is renamed to system; every position holds.
	assert_eq!(
		roles(&run(OPUS55, developer.clone())),
		"user,assistant,system,user"
	);
	assert_eq!(
		roles(&run(OPUS55, plain.clone())),
		"user,assistant,system,user"
	);
	assert_eq!(roles(&run(OPUS55, before.clone())), "user,system,assistant");
	assert_eq!(roles(&run(OPUS55, last.clone())), "user,assistant,system");

	// Fallback: the text folds into a user turn in place, tagged as a system-reminder.
	let m = run(HAIKU, developer);
	assert_eq!(roles(&m), "user,assistant,user");
	assert!(reminder_text(&m["messages"][2]).is_some_and(|t| t.contains("note")));

	let m = run(HAIKU, plain);
	assert_eq!(roles(&m), "user,assistant,user");
	let blocks = m["messages"][2]["content"].as_array().unwrap();
	assert_eq!(blocks.len(), 2);
	assert_eq!(blocks[0]["text"], "then");
	assert!(reminder_text(&m["messages"][2]).is_some_and(|t| t.contains("note")));

	let m = run(HAIKU, before);
	assert_eq!(roles(&m), "user,user,assistant");
	assert!(reminder_text(&m["messages"][1]).is_some_and(|t| t.contains("note")));

	let m = run(HAIKU, last);
	assert_eq!(roles(&m), "user,assistant,user");
	assert!(reminder_text(&m["messages"][2]).is_some_and(|t| t.contains("note")));
}

#[test]
fn invoke_tool_fields() {
	// Server tools drop; a "custom" wrapper is hoisted; strict and input_examples are gated.
	let tools = json!([
		{"name": "a", "input_schema": {"type": "object"}, "strict": true},
		{"name": "b", "input_schema": {"type": "object"}, "custom": {"defer_loading": true}},
		{"name": "c", "input_schema": {"type": "object"}, "defer_loading": true, "eager_input_streaming": true},
		{"name": "d", "input_schema": {"type": "object"}, "input_examples": [{"x": 1}]},
		{"type": "web_search_20250305", "name": "web_search"},
		{"type": "bash_20250124", "name": "bash"},
		{"type": "tool_search_tool_regex_20251119", "name": "tool_search_tool_regex"}
	]);
	let body =
		json!({"max_tokens": 50, "messages": [{"role": "user", "content": "hi"}], "tools": tools});

	let v = invoke(SONNET55, body.clone());
	let got = v["tools"].as_array().unwrap();
	let names: Vec<&str> = got.iter().map(|t| t["name"].as_str().unwrap()).collect();
	assert_eq!(
		names,
		["a", "b", "c", "d", "bash", "tool_search_tool_regex"]
	);
	assert!(
		got[0].get("strict").is_none(),
		"strict is dropped on Sonnet 5.5"
	);
	assert!(
		got[1].get("custom").is_none(),
		"the custom wrapper is hoisted away"
	);
	assert_eq!(got[1]["defer_loading"], json!(true));
	assert_eq!(got[2]["defer_loading"], json!(true));
	assert_eq!(got[2]["eager_input_streaming"], json!(true));
	assert!(
		got[3].get("input_examples").is_none(),
		"input_examples needs its beta"
	);

	// strict stays on Haiku; input_examples stays with its beta.
	let v = invoke_betas(HAIKU, body, &["tool-examples-2025-10-29"]);
	let got = v["tools"].as_array().unwrap();
	assert_eq!(got[0]["strict"], json!(true));
	assert!(got[3]["input_examples"].is_array());
}

#[test]
fn invoke_cache_markers_respect_bedrock_rules() {
	// Counted in order tools, system, messages: at most four, scope stripped, 1h dropped after 5m.
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"tools": [{"name": "a", "input_schema": {"type": "object"}, "cache_control": {"type": "ephemeral", "scope": "global"}}],
			"system": [
				{"type": "text", "text": "s1", "cache_control": {"type": "ephemeral"}},
				{"type": "text", "text": "s2", "cache_control": {"type": "ephemeral", "ttl": "1h"}}
			],
			"messages": [{"role": "user", "content": [
				{"type": "text", "text": "m1", "cache_control": {"type": "ephemeral"}},
				{"type": "text", "text": "m2", "cache_control": {"type": "ephemeral"}}
			]}]
		}),
	);
	let s = raw(&v);
	assert_eq!(
		s.matches("\"cache_control\"").count(),
		4,
		"the fifth marker is stripped"
	);
	assert!(!s.contains("\"scope\""), "scope is not allowed on Bedrock");
	assert!(
		!s.contains("\"ttl\""),
		"the 1h ttl after a 5m marker is dropped"
	);

	// A 1h marker before any 5m marker is valid and stays.
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"system": [
				{"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "1h"}},
				{"type": "text", "text": "b", "cache_control": {"type": "ephemeral"}}
			],
			"messages": [{"role": "user", "content": "hi"}]
		}),
	);
	assert_eq!(v["system"][0]["cache_control"]["ttl"], "1h");
}

#[test]
fn invoke_context_management_keeps_only_supported_edits() {
	let cm = |edits: serde_json::Value| json!({"max_tokens": 50, "messages": [{"role": "user", "content": "hi"}], "context_management": {"edits": edits}});
	let clear_thinking = json!({"type": "clear_thinking_20251015", "keep": "all"});
	let compact = json!({"type": "compact_20260112"});
	let clear_tools =
		json!({"type": "clear_tool_uses_20250919", "trigger": {"type": "tool_uses", "value": 2}});

	// Sonnet 5.5 keeps all three; its edits need the context-management beta, compact needs its own.
	let v = invoke(
		SONNET55,
		cm(json!([
			clear_thinking.clone(),
			compact.clone(),
			clear_tools.clone()
		])),
	);
	assert_eq!(
		v["context_management"]["edits"].as_array().unwrap().len(),
		3
	);
	assert_eq!(
		betas_of(&v),
		["context-management-2025-06-27", "compact-2026-01-12"].map(String::from)
	);

	// Haiku takes neither clear_thinking (thinking off) nor compact; only clear_tools stays.
	let v = invoke(
		HAIKU,
		cm(json!([clear_thinking.clone(), compact, clear_tools])),
	);
	let edits = v["context_management"]["edits"].as_array().unwrap();
	assert_eq!(edits.len(), 1);
	assert_eq!(edits[0]["type"], "clear_tool_uses_20250919");

	// clear_thinking stays once thinking is on.
	let v = invoke(
		HAIKU,
		json!({
			"max_tokens": 50,
			"thinking": {"type": "enabled", "budget_tokens": 1024},
			"messages": [{"role": "user", "content": "hi"}],
			"context_management": {"edits": [clear_thinking.clone()]}
		}),
	);
	assert!(v.get("context_management").is_some());

	// Nothing left: the field goes.
	let v = invoke(OPUS48, cm(json!([clear_thinking])));
	assert!(v.get("context_management").is_none());
}

#[test]
fn invoke_output_config_format_only_where_supported() {
	let body = json!({
		"max_tokens": 50,
		"messages": [{"role": "user", "content": "hi"}],
		"output_config": {"effort": "high", "format": {"type": "json_schema", "schema": {}}}
	});
	for target in [OPUS48, SONNET55, OPUS55] {
		let v = invoke(target, body.clone());
		assert!(
			v["output_config"].get("format").is_none(),
			"{target}: format is dropped"
		);
		assert_eq!(
			v["output_config"]["effort"], "high",
			"{target}: the rest of output_config stays"
		);
	}
	for target in [HAIKU, SONNET46] {
		assert!(
			invoke(target, body.clone())["output_config"]["format"].is_object(),
			"{target}: format stays"
		);
	}
	// output_config with only a rejected format is removed entirely.
	let v = invoke(
		SONNET55,
		json!({"max_tokens": 50, "messages": [{"role": "user", "content": "hi"}], "output_config": {"format": {"type": "json_schema", "schema": {}}}}),
	);
	assert!(v.get("output_config").is_none());
}

#[test]
fn invoke_thinking_subfields_follow_their_betas() {
	let body = json!({
		"max_tokens": 50,
		"messages": [{"role": "user", "content": "hi"}],
		"thinking": {"type": "adaptive", "display": "updates", "block_binding": {"prefix_mismatch_behavior": "error"}}
	});
	let betas = &["thinking-display-updates-2026-08-18,thinking-binding-controls-2026-08-01"];
	let v = invoke_betas(SONNET55, body.clone(), betas);
	assert_eq!(v["thinking"]["display"], "updates");
	assert!(v["thinking"]["block_binding"].is_object());
	// Sonnet 4.6 rejects both betas, so both sub-fields go and the type stays.
	let v = invoke_betas(SONNET46, body, betas);
	assert!(v["thinking"].get("display").is_none());
	assert!(v["thinking"].get("block_binding").is_none());
	assert_eq!(v["thinking"]["type"], "adaptive");
	assert!(v.get("anthropic_beta").is_none());
}

#[test]
fn invoke_disabled_thinking_omitted_where_thinking_is_always_on() {
	let body = json!({"max_tokens": 50, "thinking": {"type": "disabled"}, "messages": [{"role": "user", "content": "hi"}]});
	for target in [SONNET55, OPUS55] {
		assert!(
			invoke(target, body.clone()).get("thinking").is_none(),
			"{target}: cannot turn thinking off"
		);
	}
	for target in [HAIKU, SONNET46, OPUS48] {
		assert_eq!(
			invoke(target, body.clone())["thinking"]["type"],
			"disabled",
			"{target}: takes disabled thinking"
		);
	}
}

#[test]
fn invoke_does_not_rewrite_sampling_or_tool_choice() {
	let v = invoke(
		OPUS55,
		json!({
			"max_tokens": 50,
			"temperature": 0.5,
			"top_p": 0.9,
			"top_k": 4,
			"thinking": {"type": "enabled", "budget_tokens": 1024},
			"tools": [{"name": "a", "input_schema": {"type": "object"}}],
			"tool_choice": {"type": "any"},
			"messages": [{"role": "user", "content": "hi"}]
		}),
	);
	for k in ["temperature", "top_p", "top_k", "thinking", "tool_choice"] {
		assert!(v.get(k).is_some(), "{k} must pass through untouched");
	}
}

#[test]
fn invoke_unmeasured_claude_model_is_handled_like_sonnet_55() {
	let v = invoke_betas(
		"global.anthropic.claude-opus-9",
		json!({
			"max_tokens": 50,
			"messages": [{"role": "user", "content": "hi"}],
			"safeguards": {"auto_mode": true},
			"output_config": {"format": {"type": "json_schema", "schema": {}}}
		}),
		&["dangerous-tool-use-2026-09-03,claude-code-20250219"],
	);
	assert!(
		v.get("safeguards").is_some(),
		"an unmeasured model keeps safeguards like Sonnet 5.5"
	);
	assert!(
		v.get("output_config").is_none(),
		"an unmeasured model drops format like Sonnet 5.5"
	);
	let mut betas = betas_of(&v);
	betas.sort();
	assert_eq!(
		betas,
		["claude-code-20250219", "dangerous-tool-use-2026-09-03"].map(String::from)
	);
}

#[test]
fn invoke_matches_capability_for_us_prefixed_ids() {
	// capability_for matches on a substring, so a us.* id resolves to the same row as global.*.
	let body = json!({"max_tokens": 50, "thinking": {"type": "disabled"}, "messages": [{"role": "user", "content": "hi"}]});
	assert!(
		invoke("us.anthropic.claude-haiku-4-5-20251001-v1:0", body.clone())
			.get("thinking")
			.is_some(),
		"a us.* Haiku id takes disabled thinking"
	);
	assert!(
		invoke("us.anthropic.claude-sonnet-5-5", body)
			.get("thinking")
			.is_none(),
		"a us.* Sonnet 5.5 id drops disabled thinking"
	);
}

#[test]
fn invoke_preserves_defer_loading_on_tools() {
	// Tool search marks deferred tools with defer_loading: true. If it is lost on the round-trip,
	// Bedrock gets full schemas every turn and tool search never activates (#3240 C2).
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"tools": [{"name": "search", "description": "d", "input_schema": {"type": "object"}, "defer_loading": true}],
			"messages": [{"role": "user", "content": "hi"}]
		}),
	);
	assert_eq!(v["tools"][0]["defer_loading"], json!(true));
}

#[test]
fn invoke_preserves_pdf_document_blocks() {
	// Converse drops PDFs; the InvokeModel passthrough must keep document blocks verbatim (#3240 C5).
	let doc = json!({"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "JVBERi0xLjQK"}});
	let v = invoke(
		SONNET55,
		json!({
			"max_tokens": 50,
			"messages": [{"role": "user", "content": [doc.clone(), {"type": "text", "text": "summarize"}]}]
		}),
	);
	assert!(
		raw(&v).contains(&raw(&doc)),
		"a PDF document block must survive to Bedrock"
	);
}

// invoke-with-response-stream wraps each native Anthropic SSE event, base64-encoded, in a
// {"bytes": "...", "p": "..."} envelope inside an AWS event-stream frame. The inner payload is
// native Anthropic (content_block_delta), NOT Converse (contentBlockDelta), so it must be
// re-emitted verbatim with its own event name rather than translated through the Converse path.
#[tokio::test]
async fn invoke_stream_unwraps_frames_to_native_anthropic_sse() {
	use aws_smithy_eventstream::frame::write_message_to;
	use base64::Engine;
	use bytes::BytesMut;

	use crate::parse::aws_sse::Message;

	let inner =
		br#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#;
	let b64 = base64::prelude::BASE64_STANDARD.encode(inner);
	let envelope = format!(r#"{{"bytes":"{b64}","p":"abcdef"}}"#);

	let mut encoded = BytesMut::new();
	let msg = Message::new(Bytes::from(envelope.into_bytes()));
	write_message_to(&msg, &mut encoded).expect("frame should encode");

	let body = agent_http::Body::from(Bytes::from(encoded.to_vec()));
	let out = from_messages_invoke::unwrap_invoke_frames(body, 1024 * 1024);
	let bytes = out
		.collect()
		.await
		.expect("body should complete")
		.to_bytes();
	let text = std::str::from_utf8(&bytes).expect("output should be utf8");

	assert_eq!(
		text,
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n"
	);
}
