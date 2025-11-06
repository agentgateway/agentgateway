use agent_core::prelude::Strng;
use agent_core::strng;
use async_openai::types::{ChatCompletionMessageToolCallChunk, FunctionCallStream};
use bytes::Bytes;
use chrono;
use itertools::Itertools;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::trace;

use crate::http::{Body, Response};
use crate::llm::anthropic::types as anthropic;
use crate::llm::bedrock::types::{
	ContentBlock, ConverseErrorResponse, ConverseRequest, ConverseResponse, StopReason,
};
use crate::llm::openai::responses;
use crate::llm::{AIError, LLMInfo, universal};
use crate::telemetry::log::AsyncLog;
use crate::*;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // Optional: model override for Bedrock API path
	pub region: Strng, // Required: AWS region
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("aws.bedrock");
}

impl Provider {
	pub fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		// Log the raw error response for debugging
		if let Ok(raw_str) = std::str::from_utf8(bytes) {
			tracing::debug!(
				model = ?self.model,
				region = %self.region,
				"Bedrock error response body: {}", raw_str
			);
		}
		let resp =
			serde_json::from_slice::<ConverseErrorResponse>(bytes).map_err(AIError::ResponseParsing)?;
		translate_error(resp)
	}

	pub async fn process_streaming(
		&self,
		log: AsyncLog<LLMInfo>,
		resp: Response,
		model: &str,
		input_format: crate::llm::InputFormat,
	) -> Response {
		let model = self.model.as_deref().unwrap_or(model).to_string();

		// Bedrock doesn't return an ID, so get one from the request... if we can
		let message_id = resp
			.headers()
			.get(http::x_headers::X_AMZN_REQUESTID)
			.and_then(|s| s.to_str().ok().map(|s| s.to_owned()))
			.unwrap_or_else(|| format!("{:016x}", rand::rng().random::<u64>()));

		match input_format {
			crate::llm::InputFormat::Completions => resp.map(|b| {
				translate_stream_to_completions(b, log.clone(), model.clone(), message_id.clone())
			}),
			crate::llm::InputFormat::Messages => {
				resp.map(|body| translate_stream_to_messages(body, log, model, message_id))
			},
			crate::llm::InputFormat::Responses => {
				resp.map(|body| translate_stream_to_responses(body, log, model, message_id))
			},
		}
	}

	pub fn get_path_for_route(
		&self,
		route_type: super::RouteType,
		streaming: bool,
		model: &str,
	) -> Strng {
		let model = self.model.as_deref().unwrap_or(model);
		match route_type {
			super::RouteType::AnthropicTokenCount => strng::format!("/model/{model}/count-tokens"),
			_ if streaming => strng::format!("/model/{model}/converse-stream"),
			_ => strng::format!("/model/{model}/converse"),
		}
	}

	pub fn get_host(&self) -> Strng {
		strng::format!("bedrock-runtime.{}.amazonaws.com", self.region)
	}
}

pub fn process_response(
	model: &str,
	bytes: &Bytes,
	input_format: crate::llm::InputFormat,
) -> Result<Box<dyn crate::llm::ResponseType>, AIError> {
	let resp = serde_json::from_slice::<ConverseResponse>(bytes).map_err(AIError::ResponseParsing)?;

	match input_format {
		crate::llm::InputFormat::Completions => {
			let openai_resp = translate_response_to_completions(resp, model)?;
			let passthrough = crate::json::convert::<_, universal::passthrough::Response>(&openai_resp)
				.map_err(AIError::ResponseParsing)?;
			Ok(Box::new(passthrough))
		},
		crate::llm::InputFormat::Messages => {
			let anthropic_resp = translate_response_to_messages(resp, model)?;
			let passthrough =
				crate::json::convert::<_, crate::llm::anthropic::passthrough::Response>(&anthropic_resp)
					.map_err(AIError::ResponseParsing)?;
			Ok(Box::new(passthrough))
		},
		crate::llm::InputFormat::Responses => {
			let responses_resp = translate_response_to_responses(resp, model)?;
			let passthrough = crate::json::convert::<
				_,
				crate::llm::openai::responses::passthrough::Response,
			>(&responses_resp)
			.map_err(AIError::ResponseParsing)?;
			Ok(Box::new(passthrough))
		},
	}
}

pub(super) fn translate_count_tokens_request(
	req: anthropic::CountTokensRequest,
	anthropic_version: &str,
) -> Result<types::CountTokensRequest, AIError> {
	use base64::Engine;

	let mut body = req.rest;

	// AWS Bedrock's count-tokens endpoint wraps InvokeModel, which requires a valid
	// Anthropic Messages API request. The `max_tokens` parameter is required by Anthropic's API.
	// We set it to 1 (the minimum valid value) since token counting doesn't generate output.
	body
		.entry("max_tokens")
		.or_insert(serde_json::Value::Number(1.into()));
	body
		.entry("anthropic_version")
		.or_insert(serde_json::Value::String(anthropic_version.into()));

	let body_json = serde_json::to_vec(&body).map_err(AIError::RequestMarshal)?;
	let body_b64 = base64::engine::general_purpose::STANDARD.encode(&body_json);

	Ok(types::CountTokensRequest {
		input: types::CountTokensInputInvokeModel {
			invoke_model: types::InvokeModelBody { body: body_b64 },
		},
	})
}

pub(super) fn process_count_tokens_request(
	count_req: anthropic::CountTokensRequest,
	anthropic_version: &str,
) -> Result<Vec<u8>, AIError> {
	let bedrock_req = translate_count_tokens_request(count_req, anthropic_version)?;
	serde_json::to_vec(&bedrock_req).map_err(AIError::RequestMarshal)
}

pub fn translate_count_tokens_response(bedrock_bytes: &[u8]) -> Result<Vec<u8>, AIError> {
	let resp: types::CountTokensResponse =
		serde_json::from_slice(bedrock_bytes).map_err(AIError::ResponseParsing)?;
	serde_json::to_vec(&resp).map_err(AIError::ResponseMarshal)
}

pub async fn process_count_tokens_response(
	resp: crate::http::Response,
) -> Result<crate::http::Response, anyhow::Error> {
	use crate::http;

	let lim = http::response_buffer_limit(&resp);
	let (parts, body) = resp.into_parts();
	let bytes = http::read_body_with_limit(body, lim).await?;

	if parts.status.is_success() {
		let response_bytes = translate_count_tokens_response(&bytes)
			.map_err(|e| anyhow::anyhow!("Failed to translate count_tokens response: {}", e))?;

		let mut parts = parts;
		parts.headers.remove(http::header::CONTENT_LENGTH);

		Ok(crate::http::Response::from_parts(
			parts,
			response_bytes.into(),
		))
	} else {
		Ok(crate::http::Response::from_parts(parts, bytes.into()))
	}
}

/// Extract flat key-value pairs from JSON for Bedrock requestMetadata.
/// Only extracts top-level primitive values (strings, numbers, booleans).
fn extract_flat_metadata(value: &serde_json::Value) -> HashMap<String, String> {
	let mut metadata = HashMap::new();

	if let serde_json::Value::Object(obj) = value {
		for (key, val) in obj {
			match val {
				serde_json::Value::String(s) => {
					metadata.insert(key.clone(), s.clone());
				},
				serde_json::Value::Number(n) => {
					metadata.insert(key.clone(), n.to_string());
				},
				serde_json::Value::Bool(b) => {
					metadata.insert(key.clone(), b.to_string());
				},
				_ => {}, // Skip nested objects, arrays, null
			}
		}
	}

	metadata
}

/// Extract metadata from x-bedrock-metadata header.
/// Gateway operators can use CEL transformation to populate this header with extauthz data.
fn extract_metadata_from_headers(
	headers: Option<&http::HeaderMap>,
) -> Option<HashMap<String, String>> {
	const BEDROCK_METADATA_HEADER: &str = "x-bedrock-metadata";

	let header_value = headers?.get(BEDROCK_METADATA_HEADER)?;
	let json_str = header_value.to_str().ok()?;
	let json = serde_json::from_str::<serde_json::Value>(json_str).ok()?;
	Some(extract_flat_metadata(&json))
}

pub(super) fn translate_error(
	resp: ConverseErrorResponse,
) -> Result<universal::ChatCompletionErrorResponse, AIError> {
	Ok(universal::ChatCompletionErrorResponse {
		event_id: None,
		error: universal::ChatCompletionError {
			r#type: "invalid_request_error".to_string(),
			message: resp.message,
			param: None,
			code: None,
			event_id: None,
		},
	})
}

pub(super) fn translate_response_to_completions(
	resp: ConverseResponse,
	model: &str,
) -> Result<universal::Response, AIError> {
	let adapter = ConverseResponseAdapter::from_response(resp, model)?;
	Ok(adapter.to_universal())
}

fn translate_stop_reason(resp: &StopReason) -> universal::FinishReason {
	match resp {
		StopReason::EndTurn => universal::FinishReason::Stop,
		StopReason::MaxTokens => universal::FinishReason::Length,
		StopReason::StopSequence => universal::FinishReason::Stop,
		StopReason::ContentFiltered => universal::FinishReason::ContentFilter,
		StopReason::GuardrailIntervened => universal::FinishReason::ContentFilter,
		StopReason::ToolUse => universal::FinishReason::ToolCalls,
	}
}

pub(super) fn translate_request_completions(
	req: universal::Request,
	provider: &Provider,
	headers: Option<&http::HeaderMap>,
	prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
) -> ConverseRequest {
	// Extract and join system prompts from universal format
	let system_text = req
		.messages
		.iter()
		.filter_map(|msg| {
			if universal::message_role(msg) == universal::SYSTEM_ROLE {
				universal::message_text(msg).map(|s| s.to_string())
			} else {
				None
			}
		})
		.collect::<Vec<String>>()
		.join("\n");

	let messages = req
		.messages
		.iter()
		.filter(|msg| universal::message_role(msg) != universal::SYSTEM_ROLE)
		.filter_map(|msg| {
			let role = match universal::message_role(msg) {
				universal::ASSISTANT_ROLE => types::Role::Assistant,
				// Default to user for other roles
				_ => types::Role::User,
			};

			universal::message_text(msg)
				.filter(|s| !s.trim().is_empty())
				.map(|s| vec![ContentBlock::Text(s.to_string())])
				.map(|content| types::Message { role, content })
		})
		.collect();

	let inference_config = types::InferenceConfiguration {
		max_tokens: universal::max_tokens(&req),
		temperature: req.temperature,
		top_p: req.top_p,
		// Map Anthropic-style vendor extension to Bedrock topK when provided
		top_k: req.vendor_extensions.top_k,
		stop_sequences: universal::stop_sequence(&req),
	};

	// Build guardrail configuration if specified
	let guardrail_config = if let (Some(identifier), Some(version)) =
		(&provider.guardrail_identifier, &provider.guardrail_version)
	{
		Some(types::GuardrailConfiguration {
			guardrail_identifier: identifier.to_string(),
			guardrail_version: version.to_string(),
			trace: Some("enabled".to_string()),
		})
	} else {
		None
	};

	// Build metadata from user field and x-bedrock-metadata header
	let mut metadata = req
		.user
		.map(|user| HashMap::from([("user_id".to_string(), user)]))
		.unwrap_or_default();

	// Extract metadata from x-bedrock-metadata header (set by ExtAuthz or transformation policy)
	if let Some(header_metadata) = extract_metadata_from_headers(headers) {
		metadata.extend(header_metadata);
	}

	let metadata = if metadata.is_empty() {
		None
	} else {
		Some(metadata)
	};

	let tool_choice = match req.tool_choice {
		Some(universal::ToolChoiceOption::Named(universal::NamedToolChoice {
			r#type: _,
			function,
		})) => Some(types::ToolChoice::Tool {
			name: function.name,
		}),
		Some(universal::ToolChoiceOption::Auto) => Some(types::ToolChoice::Auto),
		Some(universal::ToolChoiceOption::Required) => Some(types::ToolChoice::Any),
		Some(universal::ToolChoiceOption::None) => None,
		None => None,
	};
	let tools = req.tools.map(|tools| {
		tools
			.into_iter()
			.map(|tool| {
				let tool_spec = types::ToolSpecification {
					name: tool.function.name,
					description: tool.function.description,
					input_schema: tool.function.parameters.map(types::ToolInputSchema::Json),
				};

				types::Tool::ToolSpec(tool_spec)
			})
			.collect_vec()
	});
	let tool_config = tools.map(|tools| types::ToolConfiguration { tools, tool_choice });

	// Handle thinking configuration similar to Anthropic
	let thinking = if let Some(budget) = req.vendor_extensions.thinking_budget_tokens {
		Some(serde_json::json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": budget
			}
		}))
	} else {
		match &req.reasoning_effort {
			// Note: Anthropic's minimum budget_tokens is 1024
			Some(universal::ReasoningEffort::Minimal) | Some(universal::ReasoningEffort::Low) => {
				Some(serde_json::json!({
					"thinking": {
						"type": "enabled",
						"budget_tokens": 1024
					}
				}))
			},
			Some(universal::ReasoningEffort::Medium) => Some(serde_json::json!({
				"thinking": {
					"type": "enabled",
					"budget_tokens": 2048
				}
			})),
			Some(universal::ReasoningEffort::High) => Some(serde_json::json!({
				"thinking": {
					"type": "enabled",
					"budget_tokens": 4096
				}
			})),
			None => None,
		}
	};

	let model_id = req.model.unwrap_or_default();
	let supports_caching = supports_prompt_caching(&model_id);
	let system_content = if system_text.is_empty() {
		None
	} else {
		let mut system_blocks = vec![types::SystemContentBlock::Text { text: system_text }];
		tracing::debug!(
			"Prompt caching policy: {:?}, model: {}, supports caching: {}",
			prompt_caching.map(|c| (c.cache_system, c.cache_messages, c.cache_tools)),
			model_id,
			supports_caching
		);
		if let Some(caching) = prompt_caching
			&& caching.cache_system
			&& supports_caching
		{
			let meets_minimum = if let Some(min_tokens) = caching.min_tokens {
				estimate_system_tokens(&system_blocks) >= min_tokens
			} else {
				true
			};
			if meets_minimum {
				system_blocks.push(types::SystemContentBlock::CachePoint {
					cache_point: create_cache_point(),
				});
			}
		}
		Some(system_blocks)
	};

	let mut bedrock_request = ConverseRequest {
		model_id,
		messages,
		system: system_content,
		inference_config: Some(inference_config),
		tool_config,
		guardrail_config,
		additional_model_request_fields: thinking,
		prompt_variables: None,
		additional_model_response_field_paths: None,
		request_metadata: metadata,
		performance_config: None,
	};

	if let Some(caching) = prompt_caching {
		if caching.cache_messages && supports_caching {
			insert_cache_point_in_last_user_message(&mut bedrock_request.messages);
		}
		if caching.cache_tools
			&& supports_caching
			&& let Some(ref mut tool_config) = bedrock_request.tool_config
			&& !tool_config.tools.is_empty()
		{
			tool_config
				.tools
				.push(types::Tool::CachePoint(create_cache_point()));
		}
	}

	bedrock_request
}

pub(super) fn translate_stream_to_completions(
	b: Body,
	log: AsyncLog<LLMInfo>,
	model: String,
	message_id: String,
) -> Body {
	// This is static for all chunks!
	let created = chrono::Utc::now().timestamp() as u32;
	let mut saw_token = false;
	// Track tool call JSON buffers by content block index
	let mut tool_calls: HashMap<i32, String> = HashMap::new();

	parse::aws_sse::transform(b, move |f| {
		let res = types::ConverseStreamOutput::deserialize(f).ok()?;
		let mk = |choices: Vec<universal::ChatChoiceStream>, usage: Option<universal::Usage>| {
			Some(universal::StreamResponse {
				id: message_id.clone(),
				model: model.clone(),
				object: "chat.completion.chunk".to_string(),
				system_fingerprint: None,
				service_tier: None,
				created,
				choices,
				usage,
			})
		};

		match res {
			types::ConverseStreamOutput::ContentBlockStart(start) => {
				// Track tool call starts for streaming
				if let Some(types::ContentBlockStart::ToolUse(tu)) = start.start {
					tool_calls.insert(start.content_block_index, String::new());
					// Emit the start of a tool call
					let d = universal::StreamResponseDelta {
						tool_calls: Some(vec![ChatCompletionMessageToolCallChunk {
							index: start.content_block_index as u32,
							id: Some(tu.tool_use_id),
							r#type: Some(universal::ToolType::Function),
							function: Some(FunctionCallStream {
								name: Some(tu.name),
								arguments: None,
							}),
						}]),
						..Default::default()
					};
					let choice = universal::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta: d,
						finish_reason: None,
					};
					mk(vec![choice], None)
				} else {
					// Text/reasoning starts don't need events in Universal format
					None
				}
			},
			types::ConverseStreamOutput::ContentBlockDelta(d) => {
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}

				let delta = d.delta.map(|delta| {
					let mut dr = universal::StreamResponseDelta::default();
					match delta {
						types::ContentBlockDelta::ReasoningContent(
							types::ReasoningContentBlockDelta::Text(t),
						) => {
							dr.reasoning_content = Some(t);
						},
						types::ContentBlockDelta::ReasoningContent(
							types::ReasoningContentBlockDelta::RedactedContent(_),
						) => {
							dr.reasoning_content = Some("[REDACTED]".to_string());
						},
						types::ContentBlockDelta::ReasoningContent(_) => {},
						types::ContentBlockDelta::Text(t) => {
							dr.content = Some(t);
						},
						types::ContentBlockDelta::ToolUse(tu) => {
							// Accumulate tool call JSON and emit deltas
							if let Some(json_buffer) = tool_calls.get_mut(&d.content_block_index) {
								json_buffer.push_str(&tu.input);
								dr.tool_calls = Some(vec![ChatCompletionMessageToolCallChunk {
									index: d.content_block_index as u32,
									id: None, // Only sent in the first chunk
									r#type: None,
									function: Some(FunctionCallStream {
										name: None,
										arguments: Some(tu.input),
									}),
								}]);
							}
						},
					};
					dr
				});

				if let Some(delta) = delta {
					let choice = universal::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta,
						finish_reason: None,
					};
					mk(vec![choice], None)
				} else {
					None
				}
			},
			types::ConverseStreamOutput::ContentBlockStop(stop) => {
				// Clean up tool call tracking for this content block
				tool_calls.remove(&stop.content_block_index);
				None
			},
			types::ConverseStreamOutput::MessageStart(start) => {
				// Just send a blob with the role
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta {
						role: Some(match start.role {
							types::Role::Assistant => universal::Role::Assistant,
							types::Role::User => universal::Role::User,
						}),
						..Default::default()
					},
					finish_reason: None,
				};
				mk(vec![choice], None)
			},
			types::ConverseStreamOutput::MessageStop(stop) => {
				let finish_reason = Some(translate_stop_reason(&stop.stop_reason));

				// Just send a blob with the finish reason
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta::default(),
					finish_reason,
				};
				mk(vec![choice], None)
			},
			types::ConverseStreamOutput::Metadata(metadata) => {
				if let Some(usage) = metadata.usage {
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
					});

					mk(
						vec![],
						Some(universal::Usage {
							prompt_tokens: usage.input_tokens as u32,
							completion_tokens: usage.output_tokens as u32,
							total_tokens: usage.total_tokens as u32,
							prompt_tokens_details: None,
							completion_tokens_details: None,
						}),
					)
				} else {
					None
				}
			},
		}
	})
}

pub(super) fn translate_request_messages(
	req: anthropic::MessagesRequest,
	provider: &Provider,
	headers: Option<&http::HeaderMap>,
) -> Result<ConverseRequest, AIError> {
	let mut cache_points_used = 0;

	// Check if thinking is enabled (Bedrock constraint: thinking requires specific tool/temp settings)
	let thinking_enabled = req.thinking.is_some();

	// Convert system prompt to Bedrock format with cache point insertion
	// Note: Anthropic MessagesRequest.system is Option<SystemPrompt>, Bedrock wants Option<Vec<SystemContentBlock>>
	let system_content = req.system.as_ref().map(|sys| {
		let mut result = Vec::new();
		match sys {
			anthropic::SystemPrompt::Text(text) => {
				result.push(types::SystemContentBlock::Text { text: text.clone() });
			},
			anthropic::SystemPrompt::Blocks(blocks) => {
				// Convert Anthropic system blocks to Bedrock system blocks with cache points
				for block in blocks {
					match block {
						anthropic::SystemContentBlock::Text {
							text,
							cache_control,
						} => {
							result.push(types::SystemContentBlock::Text { text: text.clone() });
							// Insert cache point if this block has cache_control
							if cache_control.is_some() && cache_points_used < 4 {
								result.push(types::SystemContentBlock::CachePoint {
									cache_point: create_cache_point(),
								});
								cache_points_used += 1;
							}
						},
					}
				}
			},
		}
		result
	});

	// Convert typed Anthropic messages to Bedrock messages
	let messages: Vec<types::Message> = req
		.messages
		.into_iter()
		.map(|msg| {
			let role = match msg.role {
				anthropic::Role::Assistant => types::Role::Assistant,
				anthropic::Role::User => types::Role::User,
			};

			// Convert ContentBlocks from Anthropic → Bedrock, inserting cache points
			let mut content = Vec::with_capacity(msg.content.len() * 2);
			for block in msg.content {
				let (bedrock_block, has_cache_control) = match block {
					anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
						text,
						cache_control,
						..
					}) => (ContentBlock::Text(text), cache_control.is_some()),
					anthropic::ContentBlock::Image(anthropic::ContentImageBlock {
						source,
						cache_control,
					}) => {
						if let Some(media_type) = source.get("media_type").and_then(|v| v.as_str())
							&& let Some(data) = source.get("data").and_then(|v| v.as_str())
						{
							let format = media_type
								.strip_prefix("image/")
								.unwrap_or(media_type)
								.to_string();
							(
								ContentBlock::Image(types::ImageBlock {
									format,
									source: types::ImageSource {
										bytes: data.to_string(),
									},
								}),
								cache_control.is_some(),
							)
						} else {
							continue;
						}
					},
					anthropic::ContentBlock::ToolUse {
						id,
						name,
						input,
						cache_control,
					} => (
						ContentBlock::ToolUse(types::ToolUseBlock {
							tool_use_id: id,
							name,
							input,
						}),
						cache_control.is_some(),
					),
					anthropic::ContentBlock::ToolResult {
						tool_use_id,
						content: tool_content,
						is_error,
						cache_control,
					} => {
						let bedrock_content = match tool_content {
							anthropic::ToolResultContent::Text(text) => {
								vec![types::ToolResultContentBlock::Text(text)]
							},
							anthropic::ToolResultContent::Array(parts) => parts
								.into_iter()
								.filter_map(|part| match part {
									anthropic::ToolResultContentPart::Text { text, .. } => {
										Some(types::ToolResultContentBlock::Text(text))
									},
									anthropic::ToolResultContentPart::Image { source, .. } => {
										if let Some(media_type) = source.get("media_type").and_then(|v| v.as_str())
											&& let Some(data) = source.get("data").and_then(|v| v.as_str())
										{
											let format = media_type
												.strip_prefix("image/")
												.unwrap_or(media_type)
												.to_string();
											Some(types::ToolResultContentBlock::Image(types::ImageBlock {
												format,
												source: types::ImageSource {
													bytes: data.to_string(),
												},
											}))
										} else {
											None
										}
									},
									_ => None,
								})
								.collect(),
						};

						let status = is_error.map(|is_err| match is_err {
							true => types::ToolResultStatus::Error,
							false => types::ToolResultStatus::Success,
						});

						(
							ContentBlock::ToolResult(types::ToolResultBlock {
								tool_use_id,
								content: bedrock_content,
								status,
							}),
							cache_control.is_some(),
						)
					},
					anthropic::ContentBlock::Thinking {
						thinking,
						signature,
					} => (
						ContentBlock::ReasoningContent(types::ReasoningContentBlock::Structured {
							reasoning_text: types::ReasoningText {
								text: thinking,
								signature: Some(signature),
							},
						}),
						false,
					),
					anthropic::ContentBlock::WebSearchToolResult { .. } => continue,
					anthropic::ContentBlock::RedactedThinking { .. } => continue,
					anthropic::ContentBlock::Document(_) => continue,
					anthropic::ContentBlock::SearchResult(_) => continue,
					anthropic::ContentBlock::ServerToolUse { .. } => continue,
					anthropic::ContentBlock::Unknown => continue,
				};

				content.push(bedrock_block);

				if has_cache_control && cache_points_used < 4 {
					content.push(ContentBlock::CachePoint(create_cache_point()));
					cache_points_used += 1;
				}
			}

			types::Message { role, content }
		})
		.collect();

	// Build inference config from typed fields
	let inference_config = types::InferenceConfiguration {
		max_tokens: req.max_tokens,
		// When thinking is enabled, temperature/top_p/top_k must be None (Bedrock constraint)
		temperature: if thinking_enabled {
			None
		} else {
			req.temperature
		},
		top_p: if thinking_enabled { None } else { req.top_p },
		top_k: if thinking_enabled { None } else { req.top_k },
		stop_sequences: req.stop_sequences,
	};

	// Convert typed tools to Bedrock tool config
	// NOTE: Only send toolConfig if we have at least one tool. Bedrock rejects empty tools arrays.
	let tool_config = if let Some(tools) = req.tools {
		let bedrock_tools: Vec<types::Tool> = {
			let mut result = Vec::with_capacity(tools.len() * 2);
			for tool in tools {
				let has_cache_control = tool.cache_control.is_some();

				result.push(types::Tool::ToolSpec(types::ToolSpecification {
					name: tool.name,
					description: tool.description,
					input_schema: Some(types::ToolInputSchema::Json(tool.input_schema)),
				}));

				if has_cache_control && cache_points_used < 4 {
					result.push(types::Tool::CachePoint(create_cache_point()));
					cache_points_used += 1;
				}
			}
			result
		};

		if bedrock_tools.is_empty() {
			None
		} else {
			let tool_choice = match req.tool_choice {
				Some(anthropic::ToolChoice::Auto) => {
					if thinking_enabled {
						Some(types::ToolChoice::Any)
					} else {
						Some(types::ToolChoice::Auto)
					}
				},
				Some(anthropic::ToolChoice::Any) => Some(types::ToolChoice::Any),
				Some(anthropic::ToolChoice::Tool { name }) => {
					if thinking_enabled {
						Some(types::ToolChoice::Any)
					} else {
						Some(types::ToolChoice::Tool { name })
					}
				},
				Some(anthropic::ToolChoice::None) | None => {
					if thinking_enabled {
						Some(types::ToolChoice::Any)
					} else {
						None
					}
				},
			};

			Some(types::ToolConfiguration {
				tools: bedrock_tools,
				tool_choice,
			})
		}
	} else {
		None
	};

	// Convert thinking from typed field and handle beta headers
	let mut additional_fields = req.thinking.map(|thinking| match thinking {
		anthropic::ThinkingInput::Enabled { budget_tokens } => serde_json::json!({
			"thinking": {
				"type": "enabled",
				"budget_tokens": budget_tokens
			}
		}),
		anthropic::ThinkingInput::Disabled {} => serde_json::json!({
			"thinking": {
				"type": "disabled"
			}
		}),
	});

	// Extract beta headers from HTTP headers if provided
	let beta_headers = headers.and_then(|h| extract_beta_headers(h).ok().flatten());

	if let Some(beta_array) = beta_headers {
		// Add beta headers to additionalModelRequestFields
		match additional_fields {
			Some(ref mut fields) => {
				if let Some(existing_obj) = fields.as_object_mut() {
					existing_obj.insert(
						"anthropic_beta".to_string(),
						serde_json::Value::Array(beta_array),
					);
				}
			},
			None => {
				let mut fields = serde_json::Map::new();
				fields.insert(
					"anthropic_beta".to_string(),
					serde_json::Value::Array(beta_array),
				);
				additional_fields = Some(serde_json::Value::Object(fields));
			},
		}
	}

	// Build guardrail configuration if provider has it configured
	let guardrail_config = if let (Some(identifier), Some(version)) =
		(&provider.guardrail_identifier, &provider.guardrail_version)
	{
		Some(types::GuardrailConfiguration {
			guardrail_identifier: identifier.to_string(),
			guardrail_version: version.to_string(),
			trace: Some("enabled".to_string()),
		})
	} else {
		None
	};

	// Build metadata from request field and x-bedrock-metadata header
	let mut metadata = req.metadata.map(|m| m.fields).unwrap_or_default();

	// Extract metadata from x-bedrock-metadata header (set by ExtAuthz or transformation policy)
	if let Some(header_metadata) = extract_metadata_from_headers(headers) {
		metadata.extend(header_metadata);
	}

	let metadata = if metadata.is_empty() {
		None
	} else {
		Some(metadata)
	};

	let bedrock_request = ConverseRequest {
		model_id: req.model,
		messages,
		system: system_content,
		inference_config: Some(inference_config),
		tool_config,
		guardrail_config,
		additional_model_request_fields: additional_fields,
		prompt_variables: None,
		additional_model_response_field_paths: None,
		request_metadata: metadata,
		performance_config: None,
	};

	Ok(bedrock_request)
}

pub(super) fn translate_request_responses(
	req: &responses::CreateResponse,
	provider: &Provider,
	headers: Option<&http::HeaderMap>,
	prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
) -> Result<ConverseRequest, AIError> {
	use responses::{
		ContentType, Input, InputContent, InputItem, InputMessage, Role as ResponsesRole,
	};

	let supports_caching = supports_prompt_caching(&req.model);

	// Convert input to Bedrock messages and system content
	let mut messages: Vec<types::Message> = Vec::new();
	let mut system_blocks: Vec<types::SystemContentBlock> = Vec::new();

	if let Ok(json) = serde_json::to_string_pretty(&req.input) {
		tracing::debug!("Converting Responses input to Bedrock: {}", json);
	}

	// Convert Input format to items
	let items = match &req.input {
		Input::Text(text) => {
			vec![InputItem::Message(InputMessage {
				kind: Default::default(),
				role: ResponsesRole::User,
				content: InputContent::TextInput(text.clone()),
			})]
		},
		Input::Items(items) => items.clone(),
	};

	// Process each input item
	for item in items {
		match item {
			InputItem::Message(msg) => {
				// Extract role and content
				let role = match msg.role {
					ResponsesRole::User => types::Role::User,
					ResponsesRole::Assistant => types::Role::Assistant,
					ResponsesRole::System | ResponsesRole::Developer => {
						// System and developer messages go to system array
						let text = match &msg.content {
							InputContent::TextInput(t) => t.clone(),
							InputContent::InputItemContentList(parts) => {
								// Extract text from content parts
								parts
									.iter()
									.filter_map(|part| match part {
										ContentType::InputText(input_text) => Some(input_text.text.clone()),
										_ => None,
									})
									.collect::<Vec<_>>()
									.join("\n")
							},
						};
						system_blocks.push(types::SystemContentBlock::Text { text });
						continue;
					},
				};

				// Convert content to Bedrock content blocks
				let content = match &msg.content {
					InputContent::TextInput(text) => {
						vec![ContentBlock::Text(text.clone())]
					},
					InputContent::InputItemContentList(parts) => {
						let mut blocks = Vec::new();
						tracing::debug!("Processing {} content parts", parts.len());
						for part in parts {
							match part {
								ContentType::InputText(input_text) => {
									tracing::debug!("Found InputText with text: {}", input_text.text);
									blocks.push(ContentBlock::Text(input_text.text.clone()));
								},
								ContentType::InputImage(_) => {
									// Image support requires fetching URLs or resolving file_ids
									tracing::error!("Image inputs not supported in Responses->Bedrock translation");
									return Err(AIError::UnsupportedContent);
								},
								ContentType::InputFile(_) => {
									tracing::debug!("Skipping InputFile");
									continue;
								},
							}
						}
						tracing::debug!("Created {} content blocks", blocks.len());
						blocks
					},
				};

				messages.push(types::Message { role, content });
			},
			InputItem::Custom(custom_value) => {
				#[derive(serde::Deserialize)]
				struct CustomItem {
					#[serde(rename = "type")]
					item_type: Option<String>,
					call_id: Option<String>,
					name: Option<String>,
					arguments: Option<String>,
					output: Option<serde_json::Value>,
				}

				match serde_json::from_value::<CustomItem>(custom_value.clone()) {
					Ok(item) => {
						match item.item_type.as_deref() {
							Some("function_call") => {
								if let (Some(call_id), Some(name), Some(arguments)) =
									(item.call_id, item.name, item.arguments)
								{
									// Parse tool arguments, skip this tool call if JSON is invalid
									let Ok(input) = serde_json::from_str::<serde_json::Value>(&arguments) else {
										tracing::warn!(
											"Skipping function_call with invalid JSON arguments for tool '{}': {}",
											name,
											arguments
										);
										continue;
									};

									messages.push(types::Message {
										role: types::Role::Assistant,
										content: vec![ContentBlock::ToolUse(types::ToolUseBlock {
											tool_use_id: call_id,
											name,
											input,
										})],
									});
								}
							},
							Some("function_call_output") => {
								if let (Some(call_id), Some(output)) = (item.call_id, item.output) {
									let result_content = if let Some(output_str) = output.as_str() {
										vec![types::ToolResultContentBlock::Text(output_str.to_string())]
									} else {
										let json_str = serde_json::to_string(&output).unwrap_or_default();
										vec![types::ToolResultContentBlock::Text(json_str)]
									};

									messages.push(types::Message {
										role: types::Role::User,
										content: vec![ContentBlock::ToolResult(types::ToolResultBlock {
											tool_use_id: call_id,
											content: result_content,
											status: Some(types::ToolResultStatus::Success),
										})],
									});
								}
							},
							_ => {
								// Unknown custom type, skip
								tracing::warn!("Unknown custom input item type: {:?}", item.item_type);
								continue;
							},
						}
					},
					Err(e) => {
						tracing::warn!("Failed to parse custom input item: {}", e);
						continue;
					},
				}
			},
		}
	}

	let mut system_content = if system_blocks.is_empty() {
		None
	} else {
		Some(system_blocks)
	};

	// Add instructions field to system content if present
	if let Some(instructions) = &req.instructions {
		let instructions_block = types::SystemContentBlock::Text {
			text: instructions.clone(),
		};
		if let Some(ref mut system) = system_content {
			system.insert(0, instructions_block);
		} else {
			system_content = Some(vec![instructions_block]);
		}
	}

	// Apply system prompt caching if configured
	if let Some(caching) = prompt_caching
		&& caching.cache_system
		&& supports_caching
		&& let Some(ref mut system) = system_content
	{
		let meets_minimum = if let Some(min_tokens) = caching.min_tokens {
			estimate_system_tokens(system) >= min_tokens
		} else {
			true
		};
		if meets_minimum {
			system.push(types::SystemContentBlock::CachePoint {
				cache_point: create_cache_point(),
			});
		}
	}

	let inference_config = types::InferenceConfiguration {
		max_tokens: req.max_output_tokens.unwrap_or(4096) as usize,
		temperature: req.temperature,
		top_p: req.top_p,
		top_k: None,
		stop_sequences: vec![],
	};

	// Convert tools from typed Responses API format to Bedrock format
	let (tools, tool_choice) = if let Some(response_tools) = &req.tools {
		let bedrock_tools: Vec<types::Tool> = response_tools
			.iter()
			.filter_map(|tool_def| {
				use responses::ToolDefinition;
				match tool_def {
					ToolDefinition::Function(func) => Some(types::Tool::ToolSpec(types::ToolSpecification {
						name: func.name.clone(),
						description: func.description.clone(),
						input_schema: Some(types::ToolInputSchema::Json(func.parameters.clone())),
					})),
					_ => {
						tracing::warn!("Unsupported tool type in Responses API: {:?}", tool_def);
						None
					},
				}
			})
			.collect();

		let bedrock_tool_choice = req.tool_choice.as_ref().and_then(|tc| {
			use responses::{ToolChoice, ToolChoiceMode};
			match tc {
				ToolChoice::Mode(ToolChoiceMode::Auto) => Some(types::ToolChoice::Auto),
				ToolChoice::Mode(ToolChoiceMode::Required) => Some(types::ToolChoice::Any),
				ToolChoice::Mode(ToolChoiceMode::None) => None,
				ToolChoice::Function { name } => Some(types::ToolChoice::Tool { name: name.clone() }),
				ToolChoice::Hosted { .. } => {
					tracing::warn!("Hosted tool choice not supported for Bedrock");
					None
				},
			}
		});

		(bedrock_tools, bedrock_tool_choice)
	} else {
		(vec![], None)
	};

	let tool_config = if !tools.is_empty() {
		Some(types::ToolConfiguration { tools, tool_choice })
	} else {
		None
	};

	let guardrail_config = if let (Some(identifier), Some(version)) =
		(&provider.guardrail_identifier, &provider.guardrail_version)
	{
		Some(types::GuardrailConfiguration {
			guardrail_identifier: identifier.to_string(),
			guardrail_version: version.to_string(),
			trace: Some("enabled".to_string()),
		})
	} else {
		None
	};

	// Extract metadata from request body and merge with headers (consistent with Messages/Completions)
	let mut metadata = req.metadata.clone().unwrap_or_default();

	if let Some(header_metadata) = extract_metadata_from_headers(headers) {
		metadata.extend(header_metadata);
	}

	let metadata = if metadata.is_empty() {
		None
	} else {
		Some(metadata)
	};

	let mut bedrock_request = ConverseRequest {
		model_id: req.model.clone(),
		messages,
		system: system_content,
		inference_config: Some(inference_config),
		tool_config,
		guardrail_config,
		additional_model_request_fields: None,
		prompt_variables: None,
		additional_model_response_field_paths: None,
		request_metadata: metadata,
		performance_config: None,
	};

	// Apply user message and tool caching
	if let Some(caching) = prompt_caching {
		if caching.cache_messages && supports_caching {
			insert_cache_point_in_last_user_message(&mut bedrock_request.messages);
		}
		if caching.cache_tools
			&& supports_caching
			&& let Some(ref mut tool_config) = bedrock_request.tool_config
			&& !tool_config.tools.is_empty()
		{
			tool_config
				.tools
				.push(types::Tool::CachePoint(create_cache_point()));
		}
	}

	tracing::debug!(
		"Bedrock request - messages: {}, system blocks: {}, tools: {}, tool_choice: {:?}",
		bedrock_request.messages.len(),
		bedrock_request
			.system
			.as_ref()
			.map(|s| s.len())
			.unwrap_or(0),
		bedrock_request
			.tool_config
			.as_ref()
			.map(|tc| tc.tools.len())
			.unwrap_or(0),
		bedrock_request
			.tool_config
			.as_ref()
			.and_then(|tc| tc.tool_choice.as_ref())
	);

	Ok(bedrock_request)
}

pub(super) fn translate_response_to_messages(
	bedrock_resp: ConverseResponse,
	model: &str,
) -> Result<anthropic::MessagesResponse, AIError> {
	let adapter = ConverseResponseAdapter::from_response(bedrock_resp, model)?;
	adapter.to_anthropic()
}

/// Translate Bedrock streaming events to Anthropic Messages SSE format
///
/// This function converts Bedrock's binary event stream to Anthropic's SSE format
/// using typed MessagesStreamEvent structs for compile-time safety.
///
/// Note: Some events are synthesized when Bedrock doesn't emit ContentBlockStart
/// events for text/thinking content.
pub(super) fn translate_stream_to_messages(
	b: Body,
	log: AsyncLog<LLMInfo>,
	model: String,
	_message_id: String,
) -> Body {
	let mut saw_token = false;
	let mut seen_blocks: HashSet<i32> = HashSet::new();
	let mut pending_stop_reason: Option<types::StopReason> = None;
	let mut pending_usage: Option<types::TokenUsage> = None;

	parse::aws_sse::transform_multi(b, move |aws_event| {
		let event = match types::ConverseStreamOutput::deserialize(aws_event) {
			Ok(e) => e,
			Err(e) => {
				tracing::error!(error = %e, "failed to deserialize bedrock stream event");
				return vec![(
					"error",
					serde_json::json!({
						"type": "error",
						"error": {
							"type": "api_error",
							"message": "Stream processing error"
						}
					}),
				)];
			},
		};

		match event {
			types::ConverseStreamOutput::MessageStart(_start) => {
				let event = anthropic::MessagesStreamEvent::MessageStart {
					message: anthropic::MessagesResponse {
						id: generate_anthropic_message_id(),
						r#type: "message".to_string(),
						role: anthropic::Role::Assistant,
						content: vec![],
						model: model.clone(),
						stop_reason: None,
						stop_sequence: None,
						usage: anthropic::Usage {
							input_tokens: 0,
							output_tokens: 0,
							cache_creation_input_tokens: None,
							cache_read_input_tokens: None,
						},
					},
				};
				let (event_name, event_data) = event.into_sse_tuple();
				vec![(event_name, serde_json::to_value(event_data).unwrap())]
			},
			types::ConverseStreamOutput::ContentBlockStart(start) => {
				seen_blocks.insert(start.content_block_index);
				let content_block = match start.start {
					Some(types::ContentBlockStart::ToolUse(s)) => anthropic::ContentBlock::ToolUse {
						id: s.tool_use_id,
						name: s.name,
						input: serde_json::json!({}),
						cache_control: None,
					},
					Some(types::ContentBlockStart::ReasoningContent) => anthropic::ContentBlock::Thinking {
						thinking: String::new(),
						signature: String::new(),
					},
					_ => anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
						text: String::new(),
						citations: None,
						cache_control: None,
					}),
				};

				let event = anthropic::MessagesStreamEvent::ContentBlockStart {
					index: start.content_block_index as usize,
					content_block,
				};
				let (event_name, event_data) = event.into_sse_tuple();
				vec![(event_name, serde_json::to_value(event_data).unwrap())]
			},
			types::ConverseStreamOutput::ContentBlockDelta(delta) => {
				let mut out = Vec::new();

				// Synthesize ContentStart for first text/thinking delta on this index
				let first_for_index = !seen_blocks.contains(&delta.content_block_index);
				if first_for_index {
					seen_blocks.insert(delta.content_block_index);

					if let Some(ref d) = delta.delta {
						let content_block = match d {
							types::ContentBlockDelta::Text(_) => {
								Some(anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
									text: String::new(),
									citations: None,
									cache_control: None,
								}))
							},
							types::ContentBlockDelta::ReasoningContent(_) => {
								Some(anthropic::ContentBlock::Thinking {
									thinking: String::new(),
									signature: String::new(),
								})
							},
							types::ContentBlockDelta::ToolUse(_) => None,
						};

						if let Some(cb) = content_block {
							let event = anthropic::MessagesStreamEvent::ContentBlockStart {
								index: delta.content_block_index as usize,
								content_block: cb,
							};
							let (event_name, event_data) = event.into_sse_tuple();
							out.push((event_name, serde_json::to_value(event_data).unwrap()));
						}
					}
				}

				if let Some(d) = delta.delta {
					if !saw_token {
						saw_token = true;
						log.non_atomic_mutate(|r| {
							r.response.first_token = Some(Instant::now());
						});
					}

					let anthropic_delta = match d {
						types::ContentBlockDelta::Text(text) => {
							anthropic::ContentBlockDelta::TextDelta { text }
						},
						types::ContentBlockDelta::ReasoningContent(rc) => match rc {
							types::ReasoningContentBlockDelta::Text(t) => {
								anthropic::ContentBlockDelta::ThinkingDelta { thinking: t }
							},
							types::ReasoningContentBlockDelta::Signature(sig) => {
								anthropic::ContentBlockDelta::SignatureDelta { signature: sig }
							},
							types::ReasoningContentBlockDelta::RedactedContent(_) => {
								anthropic::ContentBlockDelta::ThinkingDelta {
									thinking: "[REDACTED]".to_string(),
								}
							},
							types::ReasoningContentBlockDelta::Unknown => {
								anthropic::ContentBlockDelta::ThinkingDelta {
									thinking: String::new(),
								}
							},
						},
						types::ContentBlockDelta::ToolUse(tu) => anthropic::ContentBlockDelta::InputJsonDelta {
							partial_json: tu.input,
						},
					};

					let event = anthropic::MessagesStreamEvent::ContentBlockDelta {
						index: delta.content_block_index as usize,
						delta: anthropic_delta,
					};
					let (event_name, event_data) = event.into_sse_tuple();
					out.push((event_name, serde_json::to_value(event_data).unwrap()));
				}

				out
			},
			types::ConverseStreamOutput::ContentBlockStop(stop) => {
				seen_blocks.remove(&stop.content_block_index);
				let event = anthropic::MessagesStreamEvent::ContentBlockStop {
					index: stop.content_block_index as usize,
				};
				let (event_name, event_data) = event.into_sse_tuple();
				vec![(event_name, serde_json::to_value(event_data).unwrap())]
			},
			types::ConverseStreamOutput::MessageStop(stop) => {
				pending_stop_reason = Some(stop.stop_reason);
				vec![]
			},
			types::ConverseStreamOutput::Metadata(meta) => {
				if let Some(usage) = meta.usage {
					pending_usage = Some(usage);
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
					});
				}

				let mut out = Vec::new();
				let stop = pending_stop_reason.take();
				let usage = pending_usage.take();

				if let (Some(stop_reason), Some(usage_data)) = (stop, usage) {
					let event = anthropic::MessagesStreamEvent::MessageDelta {
						delta: anthropic::MessageDelta {
							stop_reason: Some(translate_stop_reason_to_anthropic(stop_reason)),
							stop_sequence: None,
						},
						usage: to_anthropic_message_delta_usage(usage_data),
					};
					let (event_name, event_data) = event.into_sse_tuple();
					out.push((event_name, serde_json::to_value(event_data).unwrap()));
				}

				let event = anthropic::MessagesStreamEvent::MessageStop;
				let (event_name, event_data) = event.into_sse_tuple();
				out.push((event_name, serde_json::to_value(event_data).unwrap()));

				out
			},
		}
	})
}

pub(super) fn translate_response_to_responses(
	bedrock_resp: ConverseResponse,
	model: &str,
) -> Result<crate::llm::openai::responses::passthrough::Response, AIError> {
	use responses::{Content, FunctionCall, OutputContent, OutputMessage, OutputStatus};

	let adapter = ConverseResponseAdapter::from_response(bedrock_resp, model)?;

	// Generate response ID
	let id = format!("resp_{:016x}", rand::rng().random::<u64>());

	// Convert Bedrock content blocks to Responses OutputContent
	let mut outputs = Vec::new();

	// Group content by type for proper message construction
	let mut text_parts = Vec::new();
	let mut tool_calls = Vec::new();

	for block in &adapter.message.content {
		match block {
			ContentBlock::Text(text) => {
				text_parts.push(Content::OutputText(responses::OutputText {
					text: text.clone(),
					annotations: vec![],
				}));
			},
			ContentBlock::ReasoningContent(reasoning) => {
				let text = match reasoning {
					types::ReasoningContentBlock::Structured { reasoning_text } => {
						reasoning_text.text.clone()
					},
					types::ReasoningContentBlock::Simple { text } => text.clone(),
				};
				text_parts.push(Content::OutputText(responses::OutputText {
					text,
					annotations: vec![],
				}));
			},
			ContentBlock::ToolUse(tool_use) => {
				let arguments_str = serde_json::to_string(&tool_use.input).unwrap_or_default();
				tool_calls.push(OutputContent::FunctionCall(FunctionCall {
					id: tool_use.tool_use_id.clone(),
					call_id: tool_use.tool_use_id.clone(),
					name: tool_use.name.clone(),
					arguments: arguments_str,
					status: OutputStatus::Completed,
				}));
			},
			ContentBlock::Image(_) | ContentBlock::ToolResult(_) | ContentBlock::CachePoint(_) => {
				// Skip these in responses (not part of output)
			},
		}
	}

	if !text_parts.is_empty() {
		outputs.push(OutputContent::Message(OutputMessage {
			id: format!("msg_{:016x}", rand::rng().random::<u64>()),
			role: responses::Role::Assistant,
			content: text_parts,
			status: OutputStatus::Completed,
		}));
	}

	outputs.extend(tool_calls);

	let output = outputs;

	// Determine status from stop reason
	let status = match adapter.stop_reason {
		StopReason::EndTurn | StopReason::StopSequence => "completed",
		StopReason::MaxTokens => "incomplete",
		StopReason::ToolUse => "requires_action",
		StopReason::ContentFiltered | StopReason::GuardrailIntervened => "failed",
	}
	.to_string();

	// Build usage
	let usage = adapter
		.usage
		.map(|u| crate::llm::openai::responses::passthrough::Usage {
			input_tokens: u.input_tokens as u64,
			output_tokens: u.output_tokens as u64,
			rest: serde_json::Value::Object(serde_json::Map::new()),
		});

	Ok(crate::llm::openai::responses::passthrough::Response {
		id,
		status,
		output,
		model: adapter.model,
		usage,
		rest: serde_json::Value::Object(serde_json::Map::new()),
	})
}

pub(super) fn translate_stream_to_responses(
	b: Body,
	log: AsyncLog<LLMInfo>,
	model: String,
	_message_id: String,
) -> Body {
	let mut saw_token = false;
	let mut pending_stop_reason: Option<types::StopReason> = None;
	let mut pending_usage: Option<types::TokenUsage> = None;
	let mut seen_blocks: HashSet<i32> = HashSet::new();

	// Track tool calls for streaming: (index -> (item_id, name, json_buffer))
	let mut tool_calls: HashMap<i32, (String, String, String)> = HashMap::new();

	// Track sequence numbers and item IDs
	let mut sequence_number: u64 = 0;
	let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());

	// Track message item ID for text content
	let message_item_id = format!("msg_{:016x}", rand::rng().random::<u64>());

	parse::aws_sse::transform_multi(b, move |aws_event| {
		tracing::debug!("Raw AWS event - headers: {:?}", aws_event.headers);
		if let Ok(body_str) = std::str::from_utf8(&aws_event.body) {
			tracing::debug!("AWS event body: {}", body_str);
		}

		let event = match types::ConverseStreamOutput::deserialize(aws_event) {
			Ok(e) => e,
			Err(e) => {
				tracing::error!(error = %e, "failed to deserialize bedrock stream event");
				return vec![(
					"error",
					serde_json::json!({
						"type": "error",
						"error": {
							"message": "Stream processing error"
						}
					}),
				)];
			},
		};

		match event {
			types::ConverseStreamOutput::MessageStart(_start) => {
				let mut events = Vec::new();

				sequence_number += 1;
				let created_event = serde_json::json!({
					"type": "response.created",
					"sequence_number": sequence_number,
					"response": {
						"id": response_id.clone(),
						"object": "response",
						"model": model.clone(),
						"created_at": chrono::Utc::now().timestamp() as u64,
						"status": "in_progress"
					}
				});
				events.push(("event", created_event));

				sequence_number += 1;
				let item_added_event = serde_json::json!({
					"type": "response.output_item.added",
					"sequence_number": sequence_number,
					"output_index": 0,
					"item": {
						"type": "message",
						"id": message_item_id.clone(),
						"role": "assistant",
						"status": "in_progress",
						"content": []
					}
				});
				events.push(("event", item_added_event));

				events
			},
			types::ConverseStreamOutput::ContentBlockStart(start) => {
				seen_blocks.insert(start.content_block_index);

				match start.start {
					Some(types::ContentBlockStart::ToolUse(tu)) => {
						let tool_call_item_id = format!("call_{:016x}", rand::rng().random::<u64>());
						tool_calls.insert(
							start.content_block_index,
							(tool_call_item_id.clone(), tu.name.clone(), String::new()),
						);

						sequence_number += 1;
						let item_added_event = serde_json::json!({
							"type": "response.output_item.added",
							"sequence_number": sequence_number,
							"output_index": start.content_block_index as u32,
							"item": {
								"type": "function_call",
								"id": tool_call_item_id,
								"call_id": tool_call_item_id,
								"name": tu.name,
								"arguments": "",
								"status": "in_progress"
							}
						});

						vec![("event", item_added_event)]
					},
					Some(types::ContentBlockStart::Text) => {
						sequence_number += 1;
						let part_added_event = serde_json::json!({
							"type": "response.content_part.added",
							"sequence_number": sequence_number,
							"item_id": message_item_id.clone(),
							"output_index": start.content_block_index as u32,
							"content_index": 0,
							"part": {
								"type": "text",
								"text": ""
							}
						});

						vec![("event", part_added_event)]
					},
					_ => {
						sequence_number += 1;
						let part_added_event = serde_json::json!({
							"type": "response.content_part.added",
							"sequence_number": sequence_number,
							"item_id": message_item_id.clone(),
							"output_index": start.content_block_index as u32,
							"content_index": 0,
							"part": {
								"type": "text",
								"text": ""
							}
						});

						vec![("event", part_added_event)]
					},
				}
			},
			types::ConverseStreamOutput::ContentBlockDelta(delta) => {
				let mut out = Vec::new();

				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}

				if let Some(d) = delta.delta {
					match d {
						types::ContentBlockDelta::Text(text) => {
							sequence_number += 1;
							let delta_event = serde_json::json!({
								"type": "response.output_text.delta",
								"sequence_number": sequence_number,
								"item_id": message_item_id.clone(),
								"output_index": delta.content_block_index as u32,
								"content_index": 0,
								"delta": text
							});
							out.push(("event", delta_event));
						},
						types::ContentBlockDelta::ReasoningContent(rc) => match rc {
							types::ReasoningContentBlockDelta::Text(t) => {
								sequence_number += 1;
								let delta_event = serde_json::json!({
									"type": "response.output_text.delta",
									"sequence_number": sequence_number,
									"item_id": message_item_id.clone(),
									"output_index": delta.content_block_index as u32,
									"content_index": 0,
									"delta": t
								});
								out.push(("event", delta_event));
							},
							types::ReasoningContentBlockDelta::RedactedContent(_) => {
								sequence_number += 1;
								let delta_event = serde_json::json!({
									"type": "response.output_text.delta",
									"sequence_number": sequence_number,
									"item_id": message_item_id.clone(),
									"output_index": delta.content_block_index as u32,
									"content_index": 0,
									"delta": "[REDACTED]"
								});
								out.push(("event", delta_event));
							},
							_ => {},
						},
						types::ContentBlockDelta::ToolUse(tu) => {
							if let Some((item_id, _name, buffer)) = tool_calls.get_mut(&delta.content_block_index)
							{
								buffer.push_str(&tu.input);

								sequence_number += 1;
								let delta_event = serde_json::json!({
									"type": "response.function_call_arguments.delta",
									"sequence_number": sequence_number,
									"item_id": item_id.clone(),
									"output_index": delta.content_block_index as u32,
									"delta": tu.input
								});
								out.push(("event", delta_event));
							}
						},
					}
				}

				out
			},
			types::ConverseStreamOutput::ContentBlockStop(stop) => {
				let mut events = Vec::new();

				if let Some((item_id, name, buffer)) = tool_calls.remove(&stop.content_block_index) {
					sequence_number += 1;
					let args_done_event = serde_json::json!({
						"type": "response.function_call_arguments.done",
						"sequence_number": sequence_number,
						"item_id": item_id.clone(),
						"output_index": stop.content_block_index as u32,
						"name": name.clone(),
						"arguments": buffer.clone()
					});
					events.push(("event", args_done_event));

					sequence_number += 1;
					let item_done_event = serde_json::json!({
						"type": "response.output_item.done",
						"sequence_number": sequence_number,
						"output_index": stop.content_block_index as u32,
						"item": {
							"type": "function_call",
							"id": item_id.clone(),
							"call_id": item_id,
							"name": name,
							"arguments": buffer,
							"status": "completed"
						}
					});
					events.push(("event", item_done_event));
				} else if seen_blocks.remove(&stop.content_block_index) {
					sequence_number += 1;
					let part_done_event = serde_json::json!({
						"type": "response.content_part.done",
						"sequence_number": sequence_number,
						"item_id": message_item_id.clone(),
						"output_index": stop.content_block_index as u32,
						"content_index": 0,
						"part": {
							"type": "text"
						}
					});
					events.push(("event", part_done_event));
				}

				events
			},
			types::ConverseStreamOutput::MessageStop(stop) => {
				pending_stop_reason = Some(stop.stop_reason);
				vec![]
			},
			types::ConverseStreamOutput::Metadata(meta) => {
				if let Some(usage) = meta.usage {
					pending_usage = Some(usage);
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
					});
				}

				let mut out = Vec::new();

				sequence_number += 1;
				let message_done_event = serde_json::json!({
					"type": "response.output_item.done",
					"sequence_number": sequence_number,
					"output_index": 0,
					"item": {
						"type": "message",
						"id": message_item_id.clone(),
						"role": "assistant",
						"status": "completed"
					}
				});
				out.push(("event", message_done_event));

				let stop = pending_stop_reason.take();
				let usage_data = pending_usage.take();

				let usage_obj = usage_data.map(|u| {
					serde_json::json!({
						"input_tokens": u.input_tokens as u32,
						"output_tokens": u.output_tokens as u32,
						"total_tokens": (u.input_tokens + u.output_tokens) as u32,
						"input_tokens_details": {
							"cached_tokens": u.cache_read_input_tokens.unwrap_or(0) as u32
						},
						"output_tokens_details": {
							"reasoning_tokens": 0
						}
					})
				});

				sequence_number += 1;
				let done_event = match stop {
					Some(StopReason::EndTurn) | Some(StopReason::StopSequence) | None => {
						serde_json::json!({
							"type": "response.completed",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "completed",
								"usage": usage_obj
							}
						})
					},
					Some(StopReason::MaxTokens) => {
						serde_json::json!({
							"type": "response.incomplete",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "incomplete",
								"usage": usage_obj,
								"incomplete_details": {
									"reason": "max_tokens"
								}
							}
						})
					},
					Some(StopReason::ContentFiltered) | Some(StopReason::GuardrailIntervened) => {
						serde_json::json!({
							"type": "response.failed",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "failed",
								"usage": usage_obj,
								"error": {
									"code": "content_filter",
									"message": "Content filtered by guardrails"
								}
							}
						})
					},
					Some(StopReason::ToolUse) => {
						serde_json::json!({
							"type": "response.completed",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "completed",
								"usage": usage_obj
							}
						})
					},
				};

				out.push(("event", done_event));
				out
			},
		}
	})
}

fn generate_anthropic_message_id() -> String {
	let timestamp = chrono::Utc::now().timestamp_millis();
	let random: u32 = rand::random();
	format!("msg_{:x}{:08x}", timestamp, random)
}

fn translate_stop_reason_to_anthropic(stop_reason: StopReason) -> anthropic::StopReason {
	match stop_reason {
		StopReason::EndTurn => anthropic::StopReason::EndTurn,
		StopReason::MaxTokens => anthropic::StopReason::MaxTokens,
		StopReason::StopSequence => anthropic::StopReason::StopSequence,
		StopReason::ToolUse => anthropic::StopReason::ToolUse,
		StopReason::ContentFiltered | StopReason::GuardrailIntervened => anthropic::StopReason::Refusal,
	}
}

fn to_anthropic_message_delta_usage(
	usage: types::TokenUsage,
) -> crate::llm::anthropic::types::MessageDeltaUsage {
	crate::llm::anthropic::types::MessageDeltaUsage {
		input_tokens: usage.input_tokens,
		output_tokens: usage.output_tokens,
		cache_creation_input_tokens: usage.cache_write_input_tokens,
		cache_read_input_tokens: usage.cache_read_input_tokens,
	}
}

fn translate_content_block_to_anthropic(block: &ContentBlock) -> Option<anthropic::ContentBlock> {
	match block {
		ContentBlock::Text(text) => Some(anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
			text: text.clone(),
			citations: None,
			cache_control: None,
		})),
		ContentBlock::ReasoningContent(reasoning) => {
			// Extract text and signature from either format
			let (thinking_text, signature) = match reasoning {
				types::ReasoningContentBlock::Structured { reasoning_text } => (
					reasoning_text.text.clone(),
					reasoning_text.signature.clone().unwrap_or_default(),
				),
				types::ReasoningContentBlock::Simple { text } => (text.clone(), String::new()),
			};
			Some(anthropic::ContentBlock::Thinking {
				thinking: thinking_text,
				signature,
			})
		},
		ContentBlock::ToolUse(tool_use) => Some(anthropic::ContentBlock::ToolUse {
			id: tool_use.tool_use_id.clone(),
			name: tool_use.name.clone(),
			input: tool_use.input.clone(),
			cache_control: None,
		}),
		ContentBlock::Image(img) => Some(anthropic::ContentBlock::Image(
			anthropic::ContentImageBlock {
				source: serde_json::json!({
					"type": "base64",
					"media_type": format!("image/{}", img.format),
					"data": img.source.bytes
				}),
				cache_control: None,
			},
		)),
		ContentBlock::ToolResult(_) => None, // Skip tool results in responses
		ContentBlock::CachePoint(_) => None, // Skip cache points - they're metadata only
	}
}

fn create_cache_point() -> types::CachePointBlock {
	types::CachePointBlock {
		r#type: types::CachePointType::Default,
	}
}

pub fn supports_prompt_caching(model_id: &str) -> bool {
	let model_lower = model_id.to_lowercase();
	if model_lower.contains("anthropic.claude") {
		let excluded = ["claude-instant", "claude-v1", "claude-v2"];
		if excluded.iter().any(|pattern| model_lower.contains(pattern)) {
			return false;
		}
		return true;
	}
	if model_lower.contains("amazon.nova") {
		return true;
	}
	false
}

fn estimate_system_tokens(system: &[types::SystemContentBlock]) -> usize {
	let word_count: usize = system
		.iter()
		.filter_map(|block| match block {
			types::SystemContentBlock::Text { text } => Some(text.split_whitespace().count()),
			types::SystemContentBlock::CachePoint { .. } => None,
		})
		.sum();
	(word_count * 13) / 10
}

fn insert_cache_point_in_last_user_message(messages: &mut [types::Message]) {
	// Strategy: Cache everything BEFORE the last message (not including it)
	// This caches the conversation history but not the current turn's input
	//
	// Example:
	//   [User: "Hello", Assistant: "Hi", User: "How are you?"]
	//   Cache point goes after "Hi" (before current "How are you?")
	//
	// This way:
	//   - Conversation history: cached (cheap reads on subsequent turns)
	//   - Current input: full price (it's new each turn anyway)

	let len = messages.len();

	// If we have 0-1 messages, no point caching (nothing to reuse yet)
	if len < 2 {
		return;
	}

	// Insert cache point in the second-to-last message
	// This caches all history BEFORE the current turn
	let second_to_last_idx = len - 2;
	messages[second_to_last_idx]
		.content
		.push(ContentBlock::CachePoint(create_cache_point()));

	tracing::debug!(
		"Inserted cachePoint before last message (in message at index {})",
		second_to_last_idx
	);
}

pub(super) fn extract_beta_headers(
	headers: &http::HeaderMap,
) -> Result<Option<Vec<serde_json::Value>>, AIError> {
	let mut beta_features = Vec::new();

	// Collect all anthropic-beta header values
	for value in headers.get_all("anthropic-beta") {
		let header_str = value
			.to_str()
			.map_err(|_| AIError::MissingField("Invalid anthropic-beta header value".into()))?;

		// Handle comma-separated values within a single header
		for feature in header_str.split(',') {
			let trimmed = feature.trim();
			if !trimmed.is_empty() {
				// Add each beta feature as a string value in the array
				beta_features.push(serde_json::Value::String(trimmed.to_string()));
			}
		}
	}

	if beta_features.is_empty() {
		Ok(None)
	} else {
		Ok(Some(beta_features))
	}
}

struct ConverseResponseAdapter {
	model: String,
	stop_reason: StopReason,
	usage: Option<types::TokenUsage>,
	message: types::Message,
}

impl ConverseResponseAdapter {
	fn from_response(resp: ConverseResponse, model: &str) -> Result<Self, AIError> {
		let ConverseResponse {
			output,
			stop_reason,
			usage,
			metrics: _,
			trace,
			additional_model_response_fields: _,
			performance_config: _,
		} = resp;

		if let Some(trace) = trace.as_ref()
			&& let Some(guardrail_trace) = &trace.guardrail
		{
			trace!("Bedrock guardrail trace: {:?}", guardrail_trace);
		}

		let message = match output {
			Some(types::ConverseOutput::Message(msg)) => msg,
			_ => return Err(AIError::IncompleteResponse),
		};

		Ok(Self {
			model: model.to_string(),
			stop_reason,
			usage,
			message,
		})
	}

	fn to_universal(&self) -> universal::Response {
		let mut tool_calls: Vec<universal::MessageToolCall> = Vec::new();
		let mut content = None;
		let mut reasoning_content = None;
		for block in &self.message.content {
			match block {
				ContentBlock::Text(text) => {
					content = Some(text.clone());
				},
				ContentBlock::ReasoningContent(reasoning) => {
					// Extract text from either format
					let text = match reasoning {
						types::ReasoningContentBlock::Structured { reasoning_text } => {
							reasoning_text.text.clone()
						},
						types::ReasoningContentBlock::Simple { text } => text.clone(),
					};
					reasoning_content = Some(text);
				},
				ContentBlock::ToolUse(tu) => {
					let Some(args) = serde_json::to_string(&tu.input).ok() else {
						continue;
					};
					tool_calls.push(universal::MessageToolCall {
						id: tu.tool_use_id.clone(),
						r#type: universal::ToolType::Function,
						function: universal::FunctionCall {
							name: tu.name.clone(),
							arguments: args,
						},
					});
				},
				ContentBlock::Image(_) | ContentBlock::ToolResult(_) | ContentBlock::CachePoint(_) => {
					continue;
				},
			}
		}

		let message = universal::ResponseMessage {
			role: universal::Role::Assistant,
			content,
			tool_calls: if tool_calls.is_empty() {
				None
			} else {
				Some(tool_calls)
			},
			#[allow(deprecated)]
			function_call: None,
			refusal: None,
			audio: None,
			extra: None,
			reasoning_content,
		};

		let choice = universal::ChatChoice {
			index: 0,
			message,
			finish_reason: Some(translate_stop_reason(&self.stop_reason)),
			logprobs: None,
		};

		let usage = self
			.usage
			.map(|token_usage| universal::Usage {
				prompt_tokens: token_usage.input_tokens as u32,
				completion_tokens: token_usage.output_tokens as u32,
				total_tokens: token_usage.total_tokens as u32,
				prompt_tokens_details: None,
				completion_tokens_details: None,
			})
			.unwrap_or_default();

		universal::Response {
			id: format!("bedrock-{}", chrono::Utc::now().timestamp_millis()),
			object: "chat.completion".to_string(),
			created: chrono::Utc::now().timestamp() as u32,
			model: self.model.clone(),
			choices: vec![choice],
			usage: Some(usage),
			service_tier: None,
			system_fingerprint: None,
		}
	}

	fn to_anthropic(&self) -> Result<anthropic::MessagesResponse, AIError> {
		let content: Vec<anthropic::ContentBlock> = self
			.message
			.content
			.iter()
			.filter_map(translate_content_block_to_anthropic)
			.collect();

		let usage = self
			.usage
			.map(|u| anthropic::Usage {
				input_tokens: u.input_tokens,
				output_tokens: u.output_tokens,
				cache_creation_input_tokens: u.cache_write_input_tokens,
				cache_read_input_tokens: u.cache_read_input_tokens,
			})
			.unwrap_or(anthropic::Usage {
				input_tokens: 0,
				output_tokens: 0,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
			});

		Ok(anthropic::MessagesResponse {
			id: generate_anthropic_message_id(),
			r#type: "message".to_string(),
			role: anthropic::Role::Assistant,
			content,
			model: self.model.clone(),
			stop_reason: Some(translate_stop_reason_to_anthropic(self.stop_reason)),
			stop_sequence: None,
			usage,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use ::http::HeaderMap;
	use serde_json::json;

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

		let req = anthropic::MessagesRequest {
			model: "anthropic.claude-3-sonnet".to_string(),
			messages: vec![anthropic::Message {
				role: anthropic::Role::User,
				content: vec![anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				})],
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
		};

		let out = translate_request_messages(req, &provider, Some(&headers)).unwrap();
		let metadata = out.request_metadata.unwrap();

		assert_eq!(metadata.get("user_id"), Some(&"user123".to_string()));
		assert_eq!(metadata.get("department"), Some(&"engineering".to_string()));
	}

	#[test]
	fn test_translate_request_messages_maps_top_k_from_typed() {
		let provider = Provider {
			model: Some(strng::new("anthropic.claude-3")),
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

		let req = anthropic::MessagesRequest {
			model: "anthropic.claude-3".to_string(),
			messages: vec![anthropic::Message {
				role: anthropic::Role::User,
				content: vec![anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
					text: "hello".to_string(),
					citations: None,
					cache_control: None,
				})],
			}],
			system: None,
			max_tokens: 256,
			stop_sequences: vec![],
			stream: false,
			temperature: Some(0.7),
			top_p: Some(0.9),
			top_k: Some(7),
			tools: None,
			tool_choice: None,
			metadata: None,
			thinking: None,
		};

		let out = translate_request_messages(req, &provider, None).unwrap();
		let inf = out.inference_config.unwrap();
		assert_eq!(inf.top_k, Some(7));
	}

	#[test]
	fn test_extract_beta_headers_variants() {
		let headers = HeaderMap::new();
		assert!(extract_beta_headers(&headers).unwrap().is_none());

		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			"prompt-caching-2024-07-31".parse().unwrap(),
		);
		assert_eq!(
			extract_beta_headers(&headers).unwrap().unwrap(),
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
			extract_beta_headers(&headers).unwrap().unwrap(),
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
			extract_beta_headers(&headers).unwrap().unwrap(),
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
		let mut beta_features = extract_beta_headers(&headers)
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
}

pub(super) mod types {
	use std::collections::HashMap;

	use bytes::Bytes;
	use serde::{Deserialize, Serialize};

	#[derive(Copy, Clone, Deserialize, Serialize, Debug, Default, PartialEq)]
	#[serde(rename_all = "camelCase")]
	pub enum Role {
		#[default]
		User,
		Assistant,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub enum ContentBlock {
		Text(String),
		Image(ImageBlock),
		ToolResult(ToolResultBlock),
		ToolUse(ToolUseBlock),
		ReasoningContent(ReasoningContentBlock),
		CachePoint(CachePointBlock),
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub struct ImageBlock {
		pub format: String,
		pub source: ImageSource,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub struct ImageSource {
		pub bytes: String,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(untagged)]
	pub enum ReasoningContentBlock {
		// New format from Bedrock: { "reasoningText": { "text": "...", "signature": "..." } }
		Structured {
			#[serde(rename = "reasoningText")]
			reasoning_text: ReasoningText,
		},
		// Legacy/simple format: { "text": "..." }
		Simple {
			text: String,
		},
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub struct ReasoningText {
		pub text: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		pub signature: Option<String>,
	}
	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub struct ToolResultBlock {
		/// The ID of the tool request that this is the result for.
		pub tool_use_id: String,
		/// The content for tool result content block.
		pub content: Vec<ToolResultContentBlock>,
		/// The status for the tool result content block.
		/// This field is only supported Anthropic Claude 3 models.
		pub status: Option<ToolResultStatus>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "camelCase")]
	pub enum ToolResultStatus {
		Error,
		Success,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub struct ToolUseBlock {
		/// The ID for the tool request.
		pub tool_use_id: String,
		/// The name of the tool that the model wants to use.
		pub name: String,
		/// The input to pass to the tool.
		pub input: serde_json::Value,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub enum ToolResultContentBlock {
		/// A tool result that is text.
		Text(String),
		/// A tool result that is an image.
		Image(ImageBlock),
		/// A tool result that is JSON format data.
		Json(serde_json::Value),
		/// A tool result that is video.
		Video(serde_json::Value),
	}
	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase", untagged)]
	pub enum SystemContentBlock {
		CachePoint {
			#[serde(rename = "cachePoint")]
			cache_point: CachePointBlock,
		},
		Text {
			text: String,
		},
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub struct Message {
		pub role: Role,
		pub content: Vec<ContentBlock>,
	}

	#[derive(Clone, Serialize, Debug, PartialEq)]
	pub struct InferenceConfiguration {
		/// The maximum number of tokens to generate before stopping.
		#[serde(rename = "maxTokens")]
		pub max_tokens: usize,
		/// Amount of randomness injected into the response.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub temperature: Option<f32>,
		/// Use nucleus sampling.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f32>,
		/// Only sample from the top K options for each subsequent token (if supported by model).
		#[serde(rename = "topK", skip_serializing_if = "Option::is_none")]
		pub top_k: Option<usize>,
		/// The stop sequences to use.
		#[serde(rename = "stopSequences", skip_serializing_if = "Vec::is_empty")]
		pub stop_sequences: Vec<String>,
	}

	#[derive(Clone, Serialize, Debug)]
	pub struct ConverseRequest {
		/// Specifies the model or throughput with which to run inference.
		#[serde(rename = "modelId")]
		pub model_id: String,
		/// The messages that you want to send to the model.
		pub messages: Vec<Message>,
		/// A prompt that provides instructions or context to the model.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub system: Option<Vec<SystemContentBlock>>,
		/// Inference parameters to pass to the model.
		#[serde(rename = "inferenceConfig", skip_serializing_if = "Option::is_none")]
		pub inference_config: Option<InferenceConfiguration>,
		/// Configuration information for the tools that the model can use.
		#[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
		pub tool_config: Option<ToolConfiguration>,
		/// Configuration information for a guardrail.
		#[serde(rename = "guardrailConfig", skip_serializing_if = "Option::is_none")]
		pub guardrail_config: Option<GuardrailConfiguration>,
		/// Additional model request fields.
		#[serde(
			rename = "additionalModelRequestFields",
			skip_serializing_if = "Option::is_none"
		)]
		pub additional_model_request_fields: Option<serde_json::Value>,
		/// Prompt variables.
		#[serde(rename = "promptVariables", skip_serializing_if = "Option::is_none")]
		pub prompt_variables: Option<HashMap<String, PromptVariableValues>>,
		/// Additional model response field paths.
		#[serde(
			rename = "additionalModelResponseFieldPaths",
			skip_serializing_if = "Option::is_none"
		)]
		pub additional_model_response_field_paths: Option<Vec<String>>,
		/// Request metadata.
		#[serde(rename = "requestMetadata", skip_serializing_if = "Option::is_none")]
		pub request_metadata: Option<HashMap<String, String>>,
		/// Performance configuration.
		#[serde(rename = "performanceConfig", skip_serializing_if = "Option::is_none")]
		pub performance_config: Option<PerformanceConfiguration>,
	}

	#[derive(Clone, Serialize, Deserialize, Debug)]
	pub struct CountTokensRequest {
		pub input: CountTokensInputInvokeModel,
	}

	#[derive(Clone, Serialize, Deserialize, Debug)]
	#[serde(rename_all = "camelCase")]
	pub struct CountTokensInputInvokeModel {
		pub invoke_model: InvokeModelBody,
	}

	#[derive(Clone, Serialize, Deserialize, Debug)]
	pub struct InvokeModelBody {
		pub body: String,
	}

	#[derive(Clone, Serialize, Deserialize, Debug)]
	pub struct CountTokensResponse {
		#[serde(alias = "inputTokens")]
		pub input_tokens: i32,
	}

	#[derive(Clone, Serialize, Debug)]
	pub struct ToolConfiguration {
		/// An array of tools that you want to pass to a model.
		pub tools: Vec<Tool>,
		/// If supported by model, forces the model to request a tool.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_choice: Option<ToolChoice>,
	}

	#[derive(Clone, std::fmt::Debug, ::serde::Serialize)]
	#[serde(rename_all = "camelCase")]
	pub enum Tool {
		/// CachePoint to include in the tool configuration.
		CachePoint(CachePointBlock),
		/// The specification for the tool.
		ToolSpec(ToolSpecification),
	}

	#[derive(Clone, std::fmt::Debug, ::serde::Serialize, ::serde::Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub struct CachePointBlock {
		/// Specifies the type of cache point within the CachePointBlock.
		pub r#type: CachePointType,
	}

	#[derive(
		Clone,
		Eq,
		Ord,
		PartialEq,
		PartialOrd,
		std::fmt::Debug,
		std::hash::Hash,
		::serde::Serialize,
		::serde::Deserialize,
	)]
	#[serde(rename_all = "camelCase")]
	pub enum CachePointType {
		Default,
	}

	#[derive(Clone, Serialize, Debug, PartialEq)]
	pub struct GuardrailConfiguration {
		/// The unique identifier of the guardrail
		#[serde(rename = "guardrailIdentifier")]
		pub guardrail_identifier: String,
		/// The version of the guardrail
		#[serde(rename = "guardrailVersion")]
		pub guardrail_version: String,
		/// Whether to enable trace output from the guardrail
		#[serde(rename = "trace", skip_serializing_if = "Option::is_none")]
		pub trace: Option<String>,
	}

	#[derive(Clone, Serialize, Debug, PartialEq)]
	pub struct PromptVariableValues {
		// TODO: Implement prompt variable values
	}

	#[derive(Clone, Serialize, Deserialize, Debug)]
	pub struct PerformanceConfiguration {
		// TODO: Implement performance configuration
	}

	/// The actual response from the Bedrock Converse API (matches AWS SDK ConverseOutput)
	#[derive(Debug, Deserialize, Clone)]
	pub struct ConverseResponse {
		/// The result from the call to Converse
		pub output: Option<ConverseOutput>,
		/// The reason why the model stopped generating output
		#[serde(rename = "stopReason")]
		pub stop_reason: StopReason,
		/// The total number of tokens used in the call to Converse
		pub usage: Option<TokenUsage>,
		/// Metrics for the call to Converse
		#[allow(dead_code)]
		pub metrics: Option<ConverseMetrics>,
		/// Additional fields in the response that are unique to the model
		#[allow(dead_code)]
		#[serde(rename = "additionalModelResponseFields")]
		pub additional_model_response_fields: Option<serde_json::Value>,
		/// A trace object that contains information about the Guardrail behavior
		pub trace: Option<ConverseTrace>,
		/// Model performance settings for the request
		#[serde(rename = "performanceConfig")]
		#[allow(dead_code)]
		pub performance_config: Option<PerformanceConfiguration>,
	}

	#[derive(Debug, Deserialize, Clone)]
	pub struct ConverseErrorResponse {
		pub message: String,
	}

	/// The actual content output from the model
	#[derive(Debug, Deserialize, Clone)]
	#[serde(rename_all = "camelCase")]
	pub enum ConverseOutput {
		Message(Message),
		#[serde(other)]
		Unknown,
	}

	/// Token usage information
	#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
	pub struct TokenUsage {
		/// The number of input tokens which were used
		#[serde(rename = "inputTokens")]
		pub input_tokens: usize,
		/// The number of output tokens which were used
		#[serde(rename = "outputTokens")]
		pub output_tokens: usize,
		/// The total number of tokens used
		#[serde(rename = "totalTokens")]
		pub total_tokens: usize,
		/// The number of input tokens read from cache (optional)
		#[serde(
			rename = "cacheReadInputTokens",
			skip_serializing_if = "Option::is_none"
		)]
		pub cache_read_input_tokens: Option<usize>,
		/// The number of input tokens written to cache (optional)
		#[serde(
			rename = "cacheWriteInputTokens",
			skip_serializing_if = "Option::is_none"
		)]
		pub cache_write_input_tokens: Option<usize>,
	}

	/// Metrics for the Converse call
	#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
	pub struct ConverseMetrics {
		/// Latency in milliseconds
		#[serde(rename = "latencyMs")]
		pub latency_ms: u64,
	}

	/// Trace information for Guardrail behavior
	#[derive(Clone, Debug, Serialize, Deserialize)]
	pub struct ConverseTrace {
		/// Guardrail trace information
		#[serde(rename = "guardrail", skip_serializing_if = "Option::is_none")]
		pub guardrail: Option<serde_json::Value>,
	}

	/// Reason for stopping the response generation.
	#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub enum StopReason {
		ContentFiltered,
		EndTurn,
		GuardrailIntervened,
		MaxTokens,
		StopSequence,
		ToolUse,
	}

	#[derive(Clone, Debug, Serialize)]
	#[serde(rename_all = "camelCase")]
	pub enum ToolChoice {
		/// The model must request at least one tool (no text is generated).
		Any,
		/// (Default). The Model automatically decides if a tool should be called or whether to generate text instead.
		Auto,
		/// The Model must request the specified tool. Only supported by Anthropic Claude 3 models.
		Tool { name: String },
		/// The `Unknown` variant represents cases where new union variant was received. Consider upgrading the SDK to the latest available version.
		/// An unknown enum variant
		///
		/// _Note: If you encounter this error, consider upgrading your SDK to the latest version._
		/// The `Unknown` variant represents cases where the server sent a value that wasn't recognized
		/// by the client. This can happen when the server adds new functionality, but the client has not been updated.
		/// To investigate this, consider turning on debug logging to print the raw HTTP response.
		#[non_exhaustive]
		#[allow(dead_code)]
		Unknown,
	}

	#[derive(Clone, std::fmt::Debug, ::serde::Serialize)]
	#[serde(rename_all = "camelCase")]
	pub struct ToolSpecification {
		/// The name for the tool.
		pub name: String,
		/// The description for the tool.
		pub description: Option<String>,
		/// The input schema for the tool in JSON format.
		pub input_schema: Option<ToolInputSchema>,
	}

	#[derive(Clone, Debug, Serialize)]
	#[serde(rename_all = "camelCase")]
	pub enum ToolInputSchema {
		Json(serde_json::Value),
	}

	// This is NOT deserialized directly, see the associated method
	#[derive(Clone, Debug)]
	pub enum ConverseStreamOutput {
		/// The messages output content block delta.
		ContentBlockDelta(ContentBlockDeltaEvent),
		/// Start information for a content block.
		#[allow(unused)]
		ContentBlockStart(ContentBlockStartEvent),
		/// Stop information for a content block.
		#[allow(unused)]
		ContentBlockStop(ContentBlockStopEvent),
		/// Message start information.
		MessageStart(MessageStartEvent),
		/// Message stop information.
		MessageStop(MessageStopEvent),
		/// Metadata for the converse output stream.
		Metadata(ConverseStreamMetadataEvent),
	}

	impl ConverseStreamOutput {
		pub fn deserialize(m: aws_event_stream_parser::Message) -> anyhow::Result<Self> {
			let Some(v) = m
				.headers
				.headers
				.iter()
				.find(|h| h.key.as_str() == ":event-type")
				.and_then(|v| match &v.value {
					aws_event_stream_parser::HeaderValue::String(s) => Some(s.to_string()),
					_ => None,
				})
			else {
				anyhow::bail!("no event type header")
			};
			Ok(match v.as_str() {
				"contentBlockDelta" => ConverseStreamOutput::ContentBlockDelta(serde_json::from_slice::<
					ContentBlockDeltaEvent,
				>(&m.body)?),
				"contentBlockStart" => ConverseStreamOutput::ContentBlockStart(serde_json::from_slice::<
					ContentBlockStartEvent,
				>(&m.body)?),
				"contentBlockStop" => ConverseStreamOutput::ContentBlockStop(serde_json::from_slice::<
					ContentBlockStopEvent,
				>(&m.body)?),
				"messageStart" => {
					ConverseStreamOutput::MessageStart(serde_json::from_slice::<MessageStartEvent>(&m.body)?)
				},
				"messageStop" => {
					ConverseStreamOutput::MessageStop(serde_json::from_slice::<MessageStopEvent>(&m.body)?)
				},
				"metadata" => ConverseStreamOutput::Metadata(serde_json::from_slice::<
					ConverseStreamMetadataEvent,
				>(&m.body)?),
				m => anyhow::bail!("unexpected event type: {m}"),
			})
		}
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub struct ContentBlockDeltaEvent {
		/// The delta for a content block delta event.
		pub delta: Option<ContentBlockDelta>,
		/// The block index for a content block delta event.
		#[allow(dead_code)]
		pub content_block_index: i32,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	#[allow(unused)]
	pub struct ContentBlockStartEvent {
		/// Start information about a content block start event.
		pub start: Option<ContentBlockStart>,
		/// The index for a content block start event.
		pub content_block_index: i32,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	#[allow(unused)]
	pub struct ContentBlockStopEvent {
		/// The index for a content block.
		pub content_block_index: i32,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub struct MessageStartEvent {
		/// The role for the message.
		pub role: Role,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub struct MessageStopEvent {
		/// The reason why the model stopped generating output.
		pub stop_reason: StopReason,
		/// The additional model response fields.
		#[allow(dead_code)]
		pub additional_model_response_fields: Option<serde_json::Value>,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub struct ConverseStreamMetadataEvent {
		/// Usage information for the conversation stream event.
		pub usage: Option<TokenUsage>,
		/// The metrics for the conversation stream metadata event.
		#[allow(dead_code)]
		pub metrics: Option<ConverseMetrics>,
		/// Model performance configuration metadata for the conversation stream event.
		#[allow(dead_code)]
		pub performance_config: Option<PerformanceConfiguration>,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub enum ContentBlockDelta {
		ReasoningContent(ReasoningContentBlockDelta),
		Text(String),
		ToolUse(#[allow(unused)] ToolUseBlockDelta),
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub struct ToolUseBlockDelta {
		#[allow(unused)]
		pub input: String,
	}

	#[derive(Clone, Debug, Deserialize)]
	pub enum ReasoningContentBlockDelta {
		#[serde(rename = "redactedContent")]
		RedactedContent(#[allow(unused)] Bytes),
		#[serde(rename = "signature")]
		Signature(#[allow(unused)] String),
		#[serde(rename = "text")]
		Text(String),
		#[non_exhaustive]
		Unknown,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub enum ContentBlockStart {
		/// Information about a tool that the model is requesting to use.
		#[allow(dead_code)]
		ToolUse(ToolUseBlockStart),
		/// Reasoning/thinking content block start
		#[allow(dead_code)]
		ReasoningContent,
		/// Text content block start
		#[allow(dead_code)]
		Text,
	}

	#[derive(Clone, Debug, Deserialize)]
	#[serde(rename_all = "camelCase")]
	pub struct ToolUseBlockStart {
		/// The ID for the tool request.
		#[allow(dead_code)]
		pub tool_use_id: String,
		/// The name of the tool that the model is requesting to use.
		#[allow(dead_code)]
		pub name: String,
	}
}
