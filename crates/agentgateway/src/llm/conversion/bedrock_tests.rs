use agent_core::strng;
use http::HeaderMap;
use serde_json::json;

use super::*;
use crate::llm::bedrock::Provider;
use crate::llm::types;

#[test]
fn test_extract_beta_headers_variants() {
	let headers = HeaderMap::new();
	assert!(helpers::extract_beta_headers(&headers).unwrap().is_none());

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		"prompt-caching-2024-07-31".parse().unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![json!("prompt-caching-2024-07-31")]
	);

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		"cache-control-2024-08-15,computer-use-2024-10-22"
			.parse()
			.unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![
			json!("cache-control-2024-08-15"),
			json!("computer-use-2024-10-22"),
		]
	);

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		" cache-control-2024-08-15 , computer-use-2024-10-22 "
			.parse()
			.unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![
			json!("cache-control-2024-08-15"),
			json!("computer-use-2024-10-22"),
		]
	);

	let mut headers = HeaderMap::new();
	headers.append(
		"anthropic-beta",
		"cache-control-2024-08-15".parse().unwrap(),
	);
	headers.append("anthropic-beta", "computer-use-2024-10-22".parse().unwrap());
	let mut beta_features = helpers::extract_beta_headers(&headers)
		.unwrap()
		.unwrap()
		.into_iter()
		.map(|v| v.as_str().unwrap().to_string())
		.collect::<Vec<_>>();
	beta_features.sort();
	assert_eq!(
		beta_features,
		vec![
			"cache-control-2024-08-15".to_string(),
			"computer-use-2024-10-22".to_string(),
		]
	);
}

#[test]
fn test_metadata_from_header() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	// Simulate transformation CEL setting x-bedrock-metadata header
	let mut headers = HeaderMap::new();
	headers.insert(
		"x-bedrock-metadata",
		r#"{"user_id": "user123", "department": "engineering"}"#
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

	let out = super::from_messages::translate_internal(req, &provider, Some(&headers)).unwrap();
	let metadata = out.request_metadata.unwrap();

	assert_eq!(metadata.get("user_id"), Some(&"user123".to_string()));
	assert_eq!(metadata.get("department"), Some(&"engineering".to_string()));
}

#[test]
fn test_output_config_effort_without_thinking_is_passed_through() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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

	let out = super::from_messages::translate_internal(req, &provider, None).unwrap();
	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"output_config": {
				"effort": "high"
			}
		}))
	);
	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, Some(0.7));
	assert_eq!(inference.top_p, Some(0.8));
	assert_eq!(inference.top_k, Some(50));
}

#[test]
fn test_output_config_format_maps_to_converse_output_config() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let schema = json!({
		"type": "object",
		"properties": {
			"answer": { "type": "number" }
		},
		"required": ["answer"],
		"additionalProperties": false
	});
	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "What is 2+2?".to_string(),
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
			format: Some(messages::typed::OutputFormat::JsonSchema {
				schema: schema.clone(),
			}),
		}),
	};

	let out = super::from_messages::translate_internal(req, &provider, None).unwrap();
	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"output_config": {
				"effort": "high"
			}
		}))
	);
	assert_eq!(
		out.output_config,
		Some(types::bedrock::OutputConfig {
			text_format: Some(types::bedrock::OutputFormat {
				r#type: types::bedrock::OutputFormatType::JsonSchema,
				structure: types::bedrock::OutputFormatStructure {
					json_schema: types::bedrock::JsonSchemaDefinition {
						schema: serde_json::to_string(&schema).unwrap(),
						name: None,
						description: None,
					},
				},
			}),
		})
	);
}

#[test]
fn test_explicit_empty_output_config_is_preserved() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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

	let out = super::from_messages::translate_internal(req, &provider, None).unwrap();
	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"thinking": {
				"type": "adaptive"
			},
			"output_config": {}
		}))
	);

	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, Some(0.7));
	assert_eq!(inference.top_p, Some(0.8));
	assert_eq!(inference.top_k, Some(50));
}

#[test]
fn test_thinking_and_output_config_are_both_passed_through() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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

	let out = super::from_messages::translate_internal(req, &provider, None).unwrap();
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
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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
		tools: Some(vec![messages::typed::Tool {
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
		}]),
		tool_choice: Some(messages::typed::ToolChoice::Tool {
			name: "lookup".to_string(),
			disable_parallel_tool_use: None,
		}),
		thinking: Some(messages::typed::ThinkingInput::Adaptive {}),
		output_config: None,
	};

	let out = super::from_messages::translate_internal(req, &provider, None).unwrap();
	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, Some(0.7));
	assert_eq!(inference.top_p, Some(0.8));
	assert_eq!(inference.top_k, Some(50));

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
			}
		}))
	);
}

#[test]
fn test_enabled_thinking_applies_sampling_and_tool_choice_constraints() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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
		tools: Some(vec![messages::typed::Tool {
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
		}]),
		tool_choice: Some(messages::typed::ToolChoice::Auto {
			disable_parallel_tool_use: None,
		}),
		thinking: Some(messages::typed::ThinkingInput::Enabled {
			budget_tokens: 1024,
		}),
		output_config: None,
	};

	let out = super::from_messages::translate_internal(req, &provider, None).unwrap();
	let inference = out.inference_config.unwrap();
	assert_eq!(inference.temperature, None);
	assert_eq!(inference.top_p, None);
	assert_eq!(inference.top_k, None);

	let tool_choice = out
		.tool_config
		.as_ref()
		.and_then(|cfg| cfg.tool_choice.as_ref());
	assert!(matches!(tool_choice, Some(types::bedrock::ToolChoice::Any)));
}

#[test]
fn test_messages_image_url_to_bedrock_returns_error() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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

	let err = super::from_messages::translate_internal(req, &provider, None).unwrap_err();
	assert!(matches!(err, crate::llm::AIError::UnsupportedConversion(_)));
	assert!(
		err
			.to_string()
			.contains("URL image sources are unsupported")
	);
}

#[test]
fn test_metadata_from_completions_metadata_field() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	// OpenAI-style request metadata (agentgateway uses this to carry request-scoped guardrail knobs)
	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
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
			// Non-string values should be ignored by the Bedrock metadata bridge
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

	let out = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
	);
	let md = out.request_metadata.unwrap();

	// `metadata.user_id` should win over the `user`-derived value.
	assert_eq!(md.get("user_id"), Some(&"user123".to_string()));
	assert_eq!(md.get("department"), Some(&"engineering".to_string()));
	assert!(!md.contains_key("nonstr"));
}

#[test]
fn test_completions_json_schema_response_format_maps_to_converse_output_config() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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
				schema: Some(schema.clone()),
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

	let out = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
	);
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
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
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

	let out = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
	);

	assert_eq!(
		out.additional_model_request_fields,
		Some(json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": 4096
			}
		}))
	);
}

#[test]
fn test_completions_explicit_thinking_budget_forces_enabled_thinking() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
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

	let out = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
	);

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
fn test_responses_json_schema_text_format_maps_to_converse_output_config() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let schema = json!({
		"type": "object",
		"properties": {
			"city": { "type": "string" }
		},
		"required": ["city"],
		"additionalProperties": false
	});
	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gpt-4o",
		"max_output_tokens": 64,
		"input": "Extract the city name.",
		"text": {
			"format": {
				"type": "json_schema",
				"name": "city_schema",
				"description": "Structured city extraction",
				"schema": schema
			}
		}
	}))
	.expect("valid responses request");

	let translated = super::from_responses::translate(&req, &provider, None, None).unwrap();
	let translated: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(
		translated["outputConfig"]["textFormat"]["type"],
		json!("json_schema")
	);
	assert_eq!(
		translated["outputConfig"]["textFormat"]["structure"]["jsonSchema"]["name"],
		json!("city_schema")
	);
	assert_eq!(
		translated["outputConfig"]["textFormat"]["structure"]["jsonSchema"]["description"],
		json!("Structured city extraction")
	);
	assert_eq!(
		translated["outputConfig"]["textFormat"]["structure"]["jsonSchema"]["schema"],
		serde_json::to_string(&schema).unwrap()
	);
}

#[test]
fn test_responses_reasoning_effort_maps_to_enabled_thinking_budget() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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

	let translated = super::from_responses::translate(&req, &provider, None, None).unwrap();
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
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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

	let translated = super::from_responses::translate(&req, &provider, None, None).unwrap();
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
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
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

	let translated = super::from_responses::translate(&req, &provider, None, None).unwrap();
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
	let provider = Provider {
		model: Some(strng::new("amazon.titan-embed-text-v2:0")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let req = types::embeddings::Request {
		model: Some("text-embedding-3-small".to_string()),
		input: json!("hello world"),
		user: None,
		encoding_format: None,
		dimensions: Some(1024),
		rest: json!({}),
	};

	let translated = from_embeddings::translate(&req, &provider).unwrap();
	let bedrock_req: bedrock::AmazonTitanV2EmbeddingRequest =
		serde_json::from_slice(&translated).unwrap();

	assert_eq!(bedrock_req.input_text, "hello world");
	assert_eq!(bedrock_req.dimensions, Some(1024));
}

#[test]
fn test_embeddings_titan_with_encoding_format() {
	let provider = Provider {
		model: Some(strng::new("amazon.titan-embed-text-v2:0")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let req = types::embeddings::Request {
		model: Some("text-embedding-3-small".to_string()),
		input: json!("hello"),
		user: None,
		encoding_format: Some(types::embeddings::typed::EncodingFormat::Float),
		dimensions: None,
		rest: json!({"normalize": true}),
	};

	let translated = from_embeddings::translate(&req, &provider).unwrap();
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
	let provider = Provider {
		model: Some(strng::new("amazon.titan-embed-text-v2:0")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let req = types::embeddings::Request {
		model: Some("text-embedding-3-small".to_string()),
		input: json!(["hello", "world"]),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({}),
	};

	assert!(
		from_embeddings::translate(&req, &provider).is_err(),
		"Titan should reject array input"
	);
}

#[test]
fn test_embeddings_cohere_with_passthrough_fields() {
	let provider = Provider {
		model: Some(strng::new("cohere.embed-english-v3")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let req = types::embeddings::Request {
		model: Some("text-embedding-3-small".to_string()),
		input: json!(["hello", "world"]),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({"input_type": "search_document", "truncate": "END"}),
	};

	let translated = from_embeddings::translate(&req, &provider).unwrap();
	let bedrock_req: bedrock::CohereEmbeddingRequest = serde_json::from_slice(&translated).unwrap();

	assert_eq!(bedrock_req.texts, vec!["hello", "world"]);
	assert_eq!(bedrock_req.input_type, "search_document");
	assert_eq!(bedrock_req.truncate, Some("END".to_string()));
}

#[test]
fn test_embeddings_rejects_invalid_input() {
	let provider = Provider {
		model: Some(strng::new("cohere.embed-english-v3")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	for input in [json!(["hello", 42]), json!(42)] {
		let req = types::embeddings::Request {
			model: Some("text-embedding-3-small".to_string()),
			input,
			user: None,
			encoding_format: None,
			dimensions: None,
			rest: json!({}),
		};
		assert!(from_embeddings::translate(&req, &provider).is_err());
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
	assert_eq!(openai_resp.data.len(), 1);
	assert_eq!(openai_resp.data[0].embedding, vec![0.1, 0.2, 0.3]);
	assert_eq!(openai_resp.data[0].index, 0);
	assert_eq!(openai_resp.usage.prompt_tokens, 3);
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

	assert_eq!(openai_resp.data[0].embedding, vec![0.4, 0.5, 0.6]);
	assert_eq!(openai_resp.usage.prompt_tokens, 5);
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
		.and_then(|b| serde_json::from_slice::<types::embeddings::Response>(&b))
		.unwrap();

	assert_eq!(openai_resp.object, "list");
	assert_eq!(openai_resp.data.len(), 2);
	assert_eq!(openai_resp.data[0].embedding, vec![0.1, 0.2, 0.3]);
	assert_eq!(openai_resp.data[1].embedding, vec![0.4, 0.5, 0.6]);
	assert_eq!(openai_resp.data[0].index, 0);
	assert_eq!(openai_resp.data[1].index, 1);
	assert_eq!(openai_resp.usage.prompt_tokens, 10);
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
