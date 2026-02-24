use rand::RngExt;
use tracing::trace;

use crate::http::Response;
use crate::llm::AIError;
use crate::llm::types::completions::typed::UsagePromptDetails;
use crate::llm::types::{bedrock, messages, responses};

#[cfg(test)]
#[path = "bedrock_tests.rs"]
mod tests;

pub mod from_embeddings {
	use crate::json;
	use crate::llm::AIError;
	use crate::llm::bedrock::Provider;
	use crate::llm::types;
	use crate::llm::types::ResponseType;

	pub fn translate(
		req: &types::embeddings::Request,
		provider: &Provider,
	) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, types::embeddings::typed::Request>(req)
			.map_err(AIError::RequestMarshal)?;

		let model = provider.model.as_deref().unwrap_or(&typed.model);

		// Bedrock has two embedding model families with incompatible APIs:
		// Cohere accepts batched text arrays; Titan accepts a single string.
		if model.contains("cohere") {
			let input = typed.input.as_strings();

			let bedrock_req = types::bedrock::CohereEmbeddingRequest {
				texts: input,
				input_type: req
					.rest
					.get("input_type")
					.and_then(|v| v.as_str())
					.unwrap_or("search_query")
					.to_string(),
				truncate: req
					.rest
					.get("truncate")
					.and_then(|v| v.as_str())
					.map(|s| s.to_string()),
			};
			serde_json::to_vec(&bedrock_req).map_err(AIError::RequestMarshal)
		} else {
			// Titan only accepts a single string; array input is rejected.
			let input = match &typed.input {
				types::embeddings::typed::EmbeddingInput::String(s) => s.to_string(),
				types::embeddings::typed::EmbeddingInput::Array(_) => {
					return Err(AIError::RequestParsing(serde::de::Error::custom(
						"Titan requires a single string input",
					)));
				},
			};
			let bedrock_req = types::bedrock::AmazonTitanV2EmbeddingRequest {
				input_text: input,
				dimensions: typed.dimensions,
				normalize: req.rest.get("normalize").and_then(|v| v.as_bool()),
				// Map OpenAI encoding_format → Titan embedding_types (Base64→Binary, Float→Float)
				embedding_types: typed.encoding_format.as_ref().map(|f| match f {
					types::embeddings::typed::EncodingFormat::Base64 => {
						vec![types::bedrock::BedrockEmbeddingType::Binary]
					},
					types::embeddings::typed::EncodingFormat::Float => {
						vec![types::bedrock::BedrockEmbeddingType::Float]
					},
				}),
			};
			serde_json::to_vec(&bedrock_req).map_err(AIError::RequestMarshal)
		}
	}

	pub fn translate_response(
		bytes: &[u8],
		headers: &http::HeaderMap,
		model: &str,
	) -> Result<Box<dyn ResponseType>, AIError> {
		if model.contains("cohere") {
			let resp: types::bedrock::CohereEmbeddingResponse =
				serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;

			// Cohere doesn't include token counts in the JSON body;
			// Bedrock surfaces them via response headers instead.
			let prompt_tokens = headers
				.get("x-amzn-bedrock-input-token-count")
				.and_then(|v| v.to_str().ok())
				.and_then(|v| v.parse::<u64>().ok())
				.unwrap_or(0);

			let typed_resp = types::embeddings::typed::Response {
				object: "list".to_string(),
				data: resp
					.embeddings
					.into_iter()
					.enumerate()
					.map(|(i, e)| types::embeddings::typed::Embedding {
						object: "embedding".to_string(),
						embedding: e,
						index: i as u32,
					})
					.collect(),
				model: model.to_string(),
				usage: types::embeddings::typed::Usage {
					prompt_tokens: prompt_tokens as u32,
					total_tokens: prompt_tokens as u32,
				},
			};
			// Convert the normalized internal typed response back to the passthrough-preserving OpenAI format
			let openai_resp = json::convert::<_, types::embeddings::Response>(&typed_resp)
				.map_err(AIError::ResponseParsing)?;
			Ok(Box::new(openai_resp))
		} else {
			let mut resp: types::bedrock::AmazonTitanV2EmbeddingResponse =
				serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;
			let typed_resp = types::embeddings::typed::Response {
				object: "list".to_string(),
				data: vec![types::embeddings::typed::Embedding {
					object: "embedding".to_string(),
					// Zero-clone optimization: Move the large vector out of the response body
					// to avoid expensive re-allocations during translation.
					embedding: if !resp.embedding.is_empty() {
						std::mem::take(&mut resp.embedding)
					} else {
						// When embedding_types is set, Titan returns results in embeddingsByType
						// instead of the top-level embedding field.
						resp
							.embeddings_by_type
							.remove("float")
							.and_then(|v| serde_json::from_value::<Vec<f32>>(v).ok())
							.unwrap_or_default()
					},
					index: 0,
				}],
				model: model.to_string(),
				usage: types::embeddings::typed::Usage {
					prompt_tokens: resp.input_text_token_count as u32,
					total_tokens: resp.input_text_token_count as u32,
				},
			};
			// Convert the normalized internal typed response back to the passthrough-preserving OpenAI format
			let openai_resp = json::convert::<_, types::embeddings::Response>(&typed_resp)
				.map_err(AIError::ResponseParsing)?;
			Ok(Box::new(openai_resp))
		}
	}

	pub fn translate_error(bytes: &bytes::Bytes) -> Result<bytes::Bytes, AIError> {
		// Bedrock usually returns the same error format for all models
		let res = serde_json::from_slice::<types::bedrock::ConverseErrorResponse>(bytes)
			.map_err(AIError::ResponseMarshal)?;
		let m = crate::llm::types::completions::typed::ChatCompletionErrorResponse {
			event_id: None,
			error: crate::llm::types::completions::typed::ChatCompletionError {
				r#type: "invalid_request_error".to_string(),
				message: res.message,
				param: None,
				code: None,
				event_id: None,
			},
		};
		Ok(bytes::Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
	}
}

pub mod from_completions {
	use std::collections::HashMap;
	use std::time::Instant;

	use bytes::Bytes;
	use itertools::Itertools;
	use types::bedrock;
	use types::completions::typed as completions;

	use super::helpers;
	use crate::http::Body;
	use crate::llm::bedrock::Provider;
	use crate::llm::types::ResponseType;
	use crate::llm::types::completions::typed::UsagePromptDetails;
	use crate::llm::{AIError, AmendOnDrop, types};

	use crate::{json, parse};

	/// translate an OpenAI completions request to a Bedrock converse  request
	pub fn translate(
		req: &types::completions::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, completions::Request>(req).map_err(AIError::RequestMarshal)?;
		let model_id = typed.model.clone().unwrap_or_default();
		let xlated = translate_internal(typed, model_id, provider, headers, prompt_caching);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub(super) fn translate_internal(
		req: completions::Request,
		model_id: String,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> bedrock::ConverseRequest {
		// Extract and join system prompts from completions format
		let system_text = req
			.messages
			.iter()
			.filter_map(|msg| {
				if completions::message_role(msg) == completions::SYSTEM_ROLE {
					completions::message_text(msg).map(|s| s.to_string())
				} else {
					None
				}
			})
			.collect::<Vec<String>>()
			.join("\n");

		let messages = req
			.messages
			.iter()
			.filter(|msg| completions::message_role(msg) != completions::SYSTEM_ROLE)
			.filter_map(|msg| {
				let role = match completions::message_role(msg) {
					completions::ASSISTANT_ROLE => bedrock::Role::Assistant,
					// Default to user for other roles
					_ => bedrock::Role::User,
				};

				completions::message_text(msg)
					.filter(|s| !s.trim().is_empty())
					.map(|s| vec![bedrock::ContentBlock::Text(s.to_string())])
					.map(|content| bedrock::Message { role, content })
			})
			.collect();

		let inference_config = bedrock::InferenceConfiguration {
			max_tokens: req.max_tokens(),
			temperature: req.temperature,
			top_p: req.top_p,
			// Map Anthropic-style vendor extension to Bedrock topK when provided
			top_k: req.vendor_extensions.top_k,
			stop_sequences: req.stop_sequence(),
		};

		// Build guardrail configuration if specified
		let guardrail_config = if let (Some(identifier), Some(version)) =
			(&provider.guardrail_identifier, &provider.guardrail_version)
		{
			Some(bedrock::GuardrailConfiguration {
				guardrail_identifier: identifier.to_string(),
				guardrail_version: version.to_string(),
				trace: Some("enabled".to_string()),
			})
		} else {
			None
		};

		// Build metadata from:
		// - OpenAI `user` field (normalized as user_id)
		// - OpenAI `metadata` field (agentgateway uses this to carry guardrail/model-armor knobs through Messages→Completions)
		// - x-bedrock-metadata header (set by ExtAuthz or transformation policy)
		let mut metadata = req
			.user
			.map(|user| HashMap::from([("user_id".to_string(), user)]))
			.unwrap_or_default();

		// Merge OpenAI request `metadata` when it is an object of string values
		if let Some(serde_json::Value::Object(obj)) = &req.metadata {
			for (k, v) in obj {
				if let serde_json::Value::String(s) = v {
					metadata.insert(k.clone(), s.clone());
				}
			}
		}

		// Extract metadata from x-bedrock-metadata header (set by ExtAuthz or transformation policy)
		if let Some(header_metadata) = super::helpers::extract_metadata_from_headers(headers) {
			metadata.extend(header_metadata);
		}

		let metadata = if metadata.is_empty() {
			None
		} else {
			Some(metadata)
		};

		let tool_choice = match req.tool_choice {
			Some(completions::ToolChoiceOption::Function(completions::NamedToolChoice { function })) => {
				Some(bedrock::ToolChoice::Tool {
					name: function.name,
				})
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto)) => {
				Some(bedrock::ToolChoice::Auto)
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required)) => {
				Some(bedrock::ToolChoice::Any)
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None)) => None,
			_ => None,
		};
		let tools = req.tools.map(|tools| {
			tools
				.into_iter()
				.filter_map(|tool| match tool {
					completions::Tool::Function(function_tool) => {
						let tool_spec = bedrock::ToolSpecification {
							name: function_tool.function.name,
							description: function_tool.function.description,
							input_schema: function_tool
								.function
								.parameters
								.map(bedrock::ToolInputSchema::Json),
						};

						Some(bedrock::Tool::ToolSpec(tool_spec))
					},
					_ => {
						tracing::warn!("Unsupported tool type in Bedrock conversion");
						None
					},
				})
				.collect_vec()
		});
		let tool_config = tools.map(|tools| bedrock::ToolConfiguration { tools, tool_choice });

		let explicit_thinking_budget = req.vendor_extensions.thinking_budget_tokens;
		let enabled_thinking_budget = explicit_thinking_budget.or_else(|| {
			req
				.reasoning_effort
				.as_ref()
				.and_then(reasoning_effort_to_enabled_budget)
		});

		let additional_model_request_fields = enabled_thinking_budget.map(|budget| {
			serde_json::json!({
				"thinking": {
					"type": "enabled",
					"budget_tokens": budget
				}
			})
		});
		let output_config = req
			.response_format
			.as_ref()
			.and_then(completions_response_format_to_bedrock_output_config);

		let supports_caching = helpers::supports_prompt_caching(&model_id);
		let system_content = if system_text.is_empty() {
			None
		} else {
			let mut system_blocks = vec![bedrock::SystemContentBlock::Text { text: system_text }];
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
					helpers::estimate_system_tokens(&system_blocks) >= min_tokens
				} else {
					true
				};
				if meets_minimum {
					system_blocks.push(bedrock::SystemContentBlock::CachePoint {
						cache_point: helpers::create_cache_point(),
					});
				}
			}
			Some(system_blocks)
		};

		let mut bedrock_request = bedrock::ConverseRequest {
			model_id,
			messages,
			system: system_content,
			inference_config: Some(inference_config),
			output_config,
			tool_config,
			guardrail_config,
			additional_model_request_fields,
			prompt_variables: None,
			additional_model_response_field_paths: None,
			request_metadata: metadata,
			performance_config: None,
		};

		if let Some(caching) = prompt_caching {
			if caching.cache_messages && supports_caching {
				helpers::insert_cache_point_in_last_user_message(&mut bedrock_request.messages);
			}
			if caching.cache_tools
				&& supports_caching
				&& let Some(ref mut tool_config) = bedrock_request.tool_config
				&& !tool_config.tools.is_empty()
			{
				tool_config
					.tools
					.push(bedrock::Tool::CachePoint(helpers::create_cache_point()));
			}
		}

		bedrock_request
	}

	fn reasoning_effort_to_enabled_budget(effort: &completions::ReasoningEffort) -> Option<u64> {
		match effort {
			completions::ReasoningEffort::None => None,
			completions::ReasoningEffort::Minimal | completions::ReasoningEffort::Low => Some(1024),
			completions::ReasoningEffort::Medium => Some(2048),
			completions::ReasoningEffort::High | completions::ReasoningEffort::Xhigh => Some(4096),
		}
	}

	fn completions_response_format_to_bedrock_output_config(
		response_format: &completions::ResponseFormat,
	) -> Option<bedrock::OutputConfig> {
		let (name, description, schema) = match response_format {
			completions::ResponseFormat::Text => return None,
			completions::ResponseFormat::JsonObject => (
				None,
				None,
				std::borrow::Cow::Owned(
					serde_json::json!({ "type": "object", "additionalProperties": true }),
				),
			),
			completions::ResponseFormat::JsonSchema { json_schema } => {
				let Some(schema) = json_schema.schema.as_ref() else {
					tracing::warn!(
						"Dropping response_format.json_schema for Bedrock conversion because schema is missing"
					);
					return None;
				};
				(
					Some(json_schema.name.clone()),
					json_schema.description.clone(),
					std::borrow::Cow::Borrowed(schema),
				)
			},
		};

		let Ok(schema_json) = serde_json::to_string(schema.as_ref()) else {
			tracing::warn!(
				"Dropping structured output for Bedrock conversion: schema is not serializable"
			);
			return None;
		};

		Some(bedrock::OutputConfig {
			text_format: Some(bedrock::OutputFormat {
				r#type: bedrock::OutputFormatType::JsonSchema,
				structure: bedrock::OutputFormatStructure {
					json_schema: bedrock::JsonSchemaDefinition {
						schema: schema_json,
						name,
						description,
					},
				},
			}),
		})
	}

	pub fn translate_response(bytes: &Bytes, model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<bedrock::ConverseResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
		let openai = translate_response_internal(resp, model)?;
		let passthrough = json::convert::<_, types::completions::Response>(&openai)
			.map_err(AIError::ResponseParsing)?;
		Ok(Box::new(passthrough))
	}

	fn translate_response_internal(
		resp: bedrock::ConverseResponse,
		model: &str,
	) -> Result<types::completions::typed::Response, AIError> {
		let adapter = super::ConverseResponseAdapter::from_response(resp, model)?;
		Ok(adapter.to_completions())
	}

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		let res = serde_json::from_slice::<bedrock::ConverseErrorResponse>(bytes)
			.map_err(AIError::ResponseMarshal)?;
		let m = completions::ChatCompletionErrorResponse {
			event_id: None,
			error: completions::ChatCompletionError {
				r#type: "invalid_request_error".to_string(),
				message: res.message,
				param: None,
				code: None,
				event_id: None,
			},
		};
		Ok(Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
	}

	pub fn translate_stream(
		b: Body,
		_buffer_limit: usize,
		log: AmendOnDrop,
		model: &str,
		message_id: &str,
	) -> Body {
		// This is static for all chunks!
		let created = chrono::Utc::now().timestamp() as u32;
		let mut saw_token = false;
		// Track tool call JSON buffers by content block index
		let mut tool_calls: HashMap<i32, String> = HashMap::new();
		let model = model.to_string();
		let message_id = message_id.to_string();
		parse::aws_sse::transform(b, move |f| {
			let res = bedrock::ConverseStreamOutput::deserialize(f).ok()?;
			let mk = |choices: Vec<completions::ChatChoiceStream>, usage: Option<completions::Usage>| {
				Some(completions::StreamResponse {
					id: message_id.to_string(),
					model: model.to_string(),
					object: "chat.completion.chunk".to_string(),
					system_fingerprint: None,
					service_tier: None,
					created,
					choices,
					usage,
				})
			};

			match res {
				bedrock::ConverseStreamOutput::ContentBlockStart(start) => {
					// Track tool call starts for streaming
					if let Some(bedrock::ContentBlockStart::ToolUse(tu)) = start.start {
						tool_calls.insert(start.content_block_index, String::new());
						// Emit the start of a tool call
						let d = completions::StreamResponseDelta {
							tool_calls: Some(vec![completions::ChatCompletionMessageToolCallChunk {
								index: start.content_block_index as u32,
								id: Some(tu.tool_use_id),
								r#type: Some(completions::FunctionType::Function),
								function: Some(completions::FunctionCallStream {
									name: Some(tu.name),
									arguments: None,
								}),
							}]),
							..Default::default()
						};
						let choice = completions::ChatChoiceStream {
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
				bedrock::ConverseStreamOutput::ContentBlockDelta(d) => {
					if !saw_token {
						saw_token = true;
						log.non_atomic_mutate(|r| {
							r.response.first_token = Some(Instant::now());
						});
					}

					let delta = d.delta.map(|delta| {
						let mut dr = completions::StreamResponseDelta::default();
						match delta {
							bedrock::ContentBlockDelta::ReasoningContent(
								bedrock::ReasoningContentBlockDelta::Text(t),
							) => {
								dr.reasoning_content = Some(t);
							},
							bedrock::ContentBlockDelta::ReasoningContent(
								bedrock::ReasoningContentBlockDelta::RedactedContent(_),
							) => {
								dr.reasoning_content = Some("[REDACTED]".to_string());
							},
							bedrock::ContentBlockDelta::ReasoningContent(_) => {},
							bedrock::ContentBlockDelta::Text(t) => {
								dr.content = Some(t);
							},
							bedrock::ContentBlockDelta::ToolUse(tu) => {
								// Accumulate tool call JSON and emit deltas
								if let Some(json_buffer) = tool_calls.get_mut(&d.content_block_index) {
									json_buffer.push_str(&tu.input);
									dr.tool_calls = Some(vec![completions::ChatCompletionMessageToolCallChunk {
										index: d.content_block_index as u32,
										id: None, // Only sent in the first chunk
										r#type: None,
										function: Some(completions::FunctionCallStream {
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
						let choice = completions::ChatChoiceStream {
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
				bedrock::ConverseStreamOutput::ContentBlockStop(stop) => {
					// Clean up tool call tracking for this content block
					tool_calls.remove(&stop.content_block_index);
					None
				},
				bedrock::ConverseStreamOutput::MessageStart(start) => {
					// Just send a blob with the role
					let choice = completions::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta: completions::StreamResponseDelta {
							role: Some(match start.role {
								bedrock::Role::Assistant => completions::Role::Assistant,
								bedrock::Role::User => completions::Role::User,
							}),
							..Default::default()
						},
						finish_reason: None,
					};
					mk(vec![choice], None)
				},
				bedrock::ConverseStreamOutput::MessageStop(stop) => {
					let finish_reason = Some(translate_stop_reason(&stop.stop_reason));

					// Just send a blob with the finish reason
					let choice = completions::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta: completions::StreamResponseDelta::default(),
						finish_reason,
					};
					mk(vec![choice], None)
				},
				bedrock::ConverseStreamOutput::Metadata(metadata) => {
					if let Some(usage) = metadata.usage {
						log.non_atomic_mutate(|r| {
							r.response.output_tokens = Some(usage.output_tokens as u64);
							r.response.input_tokens = Some(usage.input_tokens as u64);
							r.response.total_tokens = Some(usage.total_tokens as u64);
							r.response.cached_input_tokens = usage.cache_read_input_tokens.map(|i| i as u64);
							r.response.cache_creation_input_tokens =
								usage.cache_write_input_tokens.map(|i| i as u64);
						});

						mk(
							vec![],
							Some(completions::Usage {
								prompt_tokens: usage.input_tokens as u32,
								completion_tokens: usage.output_tokens as u32,
								total_tokens: usage.total_tokens as u32,
								cache_read_input_tokens: usage.cache_read_input_tokens.map(|i| i as u64),
								cache_creation_input_tokens: usage.cache_write_input_tokens.map(|i| i as u64),
								prompt_tokens_details: usage.cache_read_input_tokens.map(|i| UsagePromptDetails {
									cached_tokens: Some(i as u64),
								}),
								// TODO: can we get reasoning tokens?
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

	pub fn translate_stop_reason(
		resp: &bedrock::StopReason,
	) -> types::completions::typed::FinishReason {
		match resp {
			bedrock::StopReason::EndTurn => types::completions::typed::FinishReason::Stop,
			bedrock::StopReason::MaxTokens => types::completions::typed::FinishReason::Length,
			bedrock::StopReason::StopSequence => types::completions::typed::FinishReason::Stop,
			bedrock::StopReason::ContentFiltered => {
				types::completions::typed::FinishReason::ContentFilter
			},
			bedrock::StopReason::GuardrailIntervened => {
				types::completions::typed::FinishReason::ContentFilter
			},
			bedrock::StopReason::ToolUse => types::completions::typed::FinishReason::ToolCalls,
			bedrock::StopReason::ModelContextWindowExceeded => {
				types::completions::typed::FinishReason::Length
			},
		}
	}
}

pub mod from_messages {
	use std::collections::HashSet;
	use std::time::Instant;

	use bytes::Bytes;
	use types::bedrock;
	use types::messages::typed as messages;

	use super::helpers;
	use crate::http::Body;
	use crate::llm::bedrock::Provider;
	use crate::llm::types::ResponseType;
	use crate::llm::{AIError, AmendOnDrop, types};

	use crate::{json, parse};

	/// translate an Anthropic messages request to a Bedrock converse request
	pub fn translate(
		req: &types::messages::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
	) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, messages::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed, provider, headers);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub(super) fn translate_internal(
		req: messages::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
	) -> bedrock::ConverseRequest {
		let mut cache_points_used = 0;
		// Converse placement note (AWS docs):
		// - Anthropic-specific params are sent via additionalModelRequestFields for Converse:
		//   https://docs.aws.amazon.com/bedrock/latest/userguide/conversation-inference-call.html
		//   https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_Converse.html
		// - Adaptive thinking knob is thinking.type = "adaptive":
		//   https://docs.aws.amazon.com/bedrock/latest/userguide/claude-messages-adaptive-thinking.html
		// - Effort knob is output_config.effort in Anthropic request shape:
		//   https://docs.aws.amazon.com/bedrock/latest/userguide/model-parameters-anthropic-claude-messages-request-response.html
		let requested_thinking = req.thinking.as_ref();
		let requested_output_config = req.output_config.as_ref();
		let output_config = requested_output_config
			.and_then(|cfg| cfg.format.as_ref())
			.and_then(messages_output_format_to_bedrock_output_config);
		let requested_output_config_json = requested_output_config.and_then(|cfg| {
			let mut output_config = serde_json::Map::new();
			if let Some(effort) = cfg.effort {
				output_config.insert("effort".to_string(), serde_json::json!(effort));
			}
			if output_config.is_empty() {
				// Preserve an explicitly empty output_config when present in the input request.
				if cfg.format.is_none() {
					Some(serde_json::Value::Object(output_config))
				} else {
					None
				}
			} else {
				Some(serde_json::Value::Object(output_config))
			}
		});

		// Bedrock applies strict inference/tool-choice constraints only to explicit extended thinking.
		let thinking_enabled = requested_thinking
			.is_some_and(|thinking| matches!(thinking, messages::ThinkingInput::Enabled { .. }));

		// Convert system prompt to Bedrock format with cache point insertion
		// Note: Anthropic MessagesRequest.system is Option<SystemPrompt>, Bedrock wants Option<Vec<SystemContentBlock>>
		let system_content = req.system.as_ref().map(|sys| {
			let mut result = Vec::new();
			match sys {
				messages::SystemPrompt::Text(text) => {
					result.push(bedrock::SystemContentBlock::Text { text: text.clone() });
				},
				messages::SystemPrompt::Blocks(blocks) => {
					// Convert Anthropic system blocks to Bedrock system blocks with cache points
					for block in blocks {
						match block {
							messages::SystemContentBlock::Text {
								text,
								cache_control,
							} => {
								result.push(bedrock::SystemContentBlock::Text { text: text.clone() });
								// Insert cache point if this block has cache_control
								if cache_control.is_some() && cache_points_used < 4 {
									result.push(bedrock::SystemContentBlock::CachePoint {
										cache_point: helpers::create_cache_point(),
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
		let messages: Vec<bedrock::Message> = req
			.messages
			.into_iter()
			.map(|msg| {
				let role = match msg.role {
					messages::Role::Assistant => bedrock::Role::Assistant,
					messages::Role::User => bedrock::Role::User,
				};

				// Convert ContentBlocks from Anthropic → Bedrock, inserting cache points
				let mut content = Vec::with_capacity(msg.content.len() * 2);
				for block in msg.content {
					let (bedrock_block, has_cache_control) = match block {
						messages::ContentBlock::Text(messages::ContentTextBlock {
							text,
							cache_control,
							..
						}) => (bedrock::ContentBlock::Text(text), cache_control.is_some()),
						messages::ContentBlock::Image(messages::ContentImageBlock {
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
									bedrock::ContentBlock::Image(bedrock::ImageBlock {
										format,
										source: bedrock::ImageSource {
											bytes: data.to_string(),
										},
									}),
									cache_control.is_some(),
								)
							} else {
								continue;
							}
						},
						messages::ContentBlock::ToolUse {
							id,
							name,
							input,
							cache_control,
						} => (
							bedrock::ContentBlock::ToolUse(bedrock::ToolUseBlock {
								tool_use_id: id,
								name,
								input,
							}),
							cache_control.is_some(),
						),
						messages::ContentBlock::ToolResult {
							tool_use_id,
							content: tool_content,
							is_error,
							cache_control,
						} => {
							let bedrock_content = match tool_content {
								messages::ToolResultContent::Text(text) => {
									vec![bedrock::ToolResultContentBlock::Text(text)]
								},
								messages::ToolResultContent::Array(parts) => parts
									.into_iter()
									.filter_map(|part| match part {
										messages::ToolResultContentPart::Text { text, .. } => {
											Some(bedrock::ToolResultContentBlock::Text(text))
										},
										messages::ToolResultContentPart::Image { source, .. } => {
											if let Some(media_type) = source.get("media_type").and_then(|v| v.as_str())
												&& let Some(data) = source.get("data").and_then(|v| v.as_str())
											{
												let format = media_type
													.strip_prefix("image/")
													.unwrap_or(media_type)
													.to_string();
												Some(bedrock::ToolResultContentBlock::Image(
													bedrock::ImageBlock {
														format,
														source: bedrock::ImageSource {
															bytes: data.to_string(),
														},
													},
												))
											} else {
												None
											}
										},
										_ => None,
									})
									.collect(),
							};

							let status = is_error.map(|is_err| match is_err {
								true => bedrock::ToolResultStatus::Error,
								false => bedrock::ToolResultStatus::Success,
							});

							(
								bedrock::ContentBlock::ToolResult(bedrock::ToolResultBlock {
									tool_use_id,
									content: bedrock_content,
									status,
								}),
								cache_control.is_some(),
							)
						},
						messages::ContentBlock::Thinking {
							thinking,
							signature,
						} => (
							bedrock::ContentBlock::ReasoningContent(bedrock::ReasoningContentBlock::Structured {
								reasoning_text: bedrock::ReasoningText {
									text: thinking,
									signature: Some(signature),
								},
							}),
							false,
						),
						messages::ContentBlock::WebSearchToolResult { .. } => continue,
						messages::ContentBlock::RedactedThinking { .. } => continue,
						messages::ContentBlock::Document(_) => continue,
						messages::ContentBlock::SearchResult(_) => continue,
						messages::ContentBlock::ServerToolUse { .. } => continue,
						messages::ContentBlock::Unknown => continue,
					};

					content.push(bedrock_block);

					if has_cache_control && cache_points_used < 4 {
						content.push(bedrock::ContentBlock::CachePoint(
							helpers::create_cache_point(),
						));
						cache_points_used += 1;
					}
				}

				bedrock::Message { role, content }
			})
			.collect();

		// Build inference config from typed fields
		let inference_config = bedrock::InferenceConfiguration {
			max_tokens: req.max_tokens,
			// Extended thinking requires temperature/top_p/top_k to be unset.
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
			let bedrock_tools: Vec<bedrock::Tool> = {
				let mut result = Vec::with_capacity(tools.len() * 2);
				for tool in tools {
					let has_cache_control = tool.cache_control.is_some();

					result.push(bedrock::Tool::ToolSpec(bedrock::ToolSpecification {
						name: tool.name,
						description: tool.description,
						input_schema: Some(bedrock::ToolInputSchema::Json(tool.input_schema)),
					}));

					if has_cache_control && cache_points_used < 4 {
						result.push(bedrock::Tool::CachePoint(helpers::create_cache_point()));
						cache_points_used += 1;
					}
				}
				result
			};

			if bedrock_tools.is_empty() {
				None
			} else {
				let tool_choice = match req.tool_choice {
					Some(messages::ToolChoice::Auto) => {
						if thinking_enabled {
							Some(bedrock::ToolChoice::Any)
						} else {
							Some(bedrock::ToolChoice::Auto)
						}
					},
					Some(messages::ToolChoice::Any) => Some(bedrock::ToolChoice::Any),
					Some(messages::ToolChoice::Tool { name }) => {
						if thinking_enabled {
							Some(bedrock::ToolChoice::Any)
						} else {
							Some(bedrock::ToolChoice::Tool { name })
						}
					},
					Some(messages::ToolChoice::None) | None => {
						if thinking_enabled {
							Some(bedrock::ToolChoice::Any)
						} else {
							None
						}
					},
				};

				Some(bedrock::ToolConfiguration {
					tools: bedrock_tools,
					tool_choice,
				})
			}
		} else {
			None
		};

		// Build Anthropic model-specific fields under Converse's additionalModelRequestFields.
		let mut additional_fields = requested_thinking.map(|thinking| {
			let thinking_json = match thinking {
				messages::ThinkingInput::Enabled { budget_tokens } => serde_json::json!({
					"type": "enabled",
					"budget_tokens": budget_tokens
				}),
				messages::ThinkingInput::Disabled {} => serde_json::json!({
					"type": "disabled"
				}),
				messages::ThinkingInput::Adaptive {} => serde_json::json!({
					"type": "adaptive"
				}),
			};
			serde_json::json!({ "thinking": thinking_json })
		});
		let mut upsert_additional_field = |key: &str, value: serde_json::Value| {
			let fields =
				additional_fields.get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
			fields
				.as_object_mut()
				.expect("additional model request fields must be a JSON object")
				.insert(key.to_string(), value);
		};

		// Preserve explicit output_config in Anthropic's model-specific envelope.
		if let Some(output_config) = requested_output_config_json {
			upsert_additional_field("output_config", output_config);
		}

		// Extract beta headers from HTTP headers if provided
		let beta_headers = headers.and_then(|h| helpers::extract_beta_headers(h).ok().flatten());

		if let Some(beta_array) = beta_headers {
			// Add beta headers to additionalModelRequestFields.
			upsert_additional_field("anthropic_beta", serde_json::Value::Array(beta_array));
		}

		// Build guardrail configuration if provider has it configured
		let guardrail_config = if let (Some(identifier), Some(version)) =
			(&provider.guardrail_identifier, &provider.guardrail_version)
		{
			Some(bedrock::GuardrailConfiguration {
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
		if let Some(header_metadata) = helpers::extract_metadata_from_headers(headers) {
			metadata.extend(header_metadata);
		}

		let metadata = if metadata.is_empty() {
			None
		} else {
			Some(metadata)
		};

		bedrock::ConverseRequest {
			model_id: req.model,
			messages,
			system: system_content,
			inference_config: Some(inference_config),
			output_config,
			tool_config,
			guardrail_config,
			additional_model_request_fields: additional_fields,
			prompt_variables: None,
			additional_model_response_field_paths: None,
			request_metadata: metadata,
			performance_config: None,
		}
	}

	fn messages_output_format_to_bedrock_output_config(
		format: &messages::OutputFormat,
	) -> Option<bedrock::OutputConfig> {
		let schema = match format {
			messages::OutputFormat::JsonSchema { schema } => schema,
		};
		let Ok(schema_json) = serde_json::to_string(schema) else {
			tracing::warn!(
				"Dropping output_config.format for Bedrock conversion: schema is not serializable"
			);
			return None;
		};

		Some(bedrock::OutputConfig {
			text_format: Some(bedrock::OutputFormat {
				r#type: bedrock::OutputFormatType::JsonSchema,
				structure: bedrock::OutputFormatStructure {
					json_schema: bedrock::JsonSchemaDefinition {
						schema: schema_json,
						name: None,
						description: None,
					},
				},
			}),
		})
	}

	pub fn translate_response(bytes: &Bytes, model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<bedrock::ConverseResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
		let openai = translate_response_internal(resp, model)?;
		let passthrough =
			json::convert::<_, types::messages::Response>(&openai).map_err(AIError::ResponseParsing)?;
		Ok(Box::new(passthrough))
	}

	fn translate_response_internal(
		resp: bedrock::ConverseResponse,
		model: &str,
	) -> Result<types::messages::typed::MessagesResponse, AIError> {
		let adapter = super::ConverseResponseAdapter::from_response(resp, model)?;
		adapter.to_anthropic()
	}

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		let res = serde_json::from_slice::<bedrock::ConverseErrorResponse>(bytes)
			.map_err(AIError::ResponseMarshal)?;
		let m = types::messages::typed::MessagesErrorResponse {
			r#type: "".to_owned(),
			error: types::messages::typed::MessagesError {
				r#type: "invalid_request_error".to_string(),
				message: res.message,
			},
		};
		Ok(Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
	}

	pub fn translate_stream(
		b: Body,
		_buffer_limit: usize,
		log: AmendOnDrop,
		model: &str,
		_message_id: &str,
	) -> Body {
		let mut saw_token = false;
		let mut seen_blocks: HashSet<i32> = HashSet::new();
		let mut pending_stop_reason: Option<bedrock::StopReason> = None;
		let mut pending_usage: Option<bedrock::TokenUsage> = None;
		let model = model.to_string();
		parse::aws_sse::transform_multi(b, move |aws_event| {
			let event = match bedrock::ConverseStreamOutput::deserialize(aws_event) {
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
				bedrock::ConverseStreamOutput::MessageStart(_start) => {
					let event = messages::MessagesStreamEvent::MessageStart {
						message: messages::MessagesResponse {
							id: helpers::generate_anthropic_message_id(),
							r#type: "message".to_string(),
							role: messages::Role::Assistant,
							content: vec![],
							model: model.to_string(),
							stop_reason: None,
							stop_sequence: None,
							usage: messages::Usage {
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
				bedrock::ConverseStreamOutput::ContentBlockStart(start) => {
					seen_blocks.insert(start.content_block_index);
					let content_block = match start.start {
						Some(bedrock::ContentBlockStart::ToolUse(s)) => messages::ContentBlock::ToolUse {
							id: s.tool_use_id,
							name: s.name,
							input: serde_json::json!({}),
							cache_control: None,
						},
						Some(bedrock::ContentBlockStart::ReasoningContent) => {
							messages::ContentBlock::Thinking {
								thinking: String::new(),
								signature: String::new(),
							}
						},
						_ => messages::ContentBlock::Text(messages::ContentTextBlock {
							text: String::new(),
							citations: None,
							cache_control: None,
						}),
					};

					let event = messages::MessagesStreamEvent::ContentBlockStart {
						index: start.content_block_index as usize,
						content_block,
					};
					let (event_name, event_data) = event.into_sse_tuple();
					vec![(event_name, serde_json::to_value(event_data).unwrap())]
				},
				bedrock::ConverseStreamOutput::ContentBlockDelta(delta) => {
					let mut out = Vec::new();

					// Synthesize ContentStart for first text/thinking delta on this index
					let first_for_index = !seen_blocks.contains(&delta.content_block_index);
					if first_for_index {
						seen_blocks.insert(delta.content_block_index);

						if let Some(ref d) = delta.delta {
							let content_block = match d {
								bedrock::ContentBlockDelta::Text(_) => {
									Some(messages::ContentBlock::Text(messages::ContentTextBlock {
										text: String::new(),
										citations: None,
										cache_control: None,
									}))
								},
								bedrock::ContentBlockDelta::ReasoningContent(_) => {
									Some(messages::ContentBlock::Thinking {
										thinking: String::new(),
										signature: String::new(),
									})
								},
								bedrock::ContentBlockDelta::ToolUse(_) => None,
							};

							if let Some(cb) = content_block {
								let event = messages::MessagesStreamEvent::ContentBlockStart {
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
							bedrock::ContentBlockDelta::Text(text) => {
								messages::ContentBlockDelta::TextDelta { text }
							},
							bedrock::ContentBlockDelta::ReasoningContent(rc) => match rc {
								bedrock::ReasoningContentBlockDelta::Text(t) => {
									messages::ContentBlockDelta::ThinkingDelta { thinking: t }
								},
								bedrock::ReasoningContentBlockDelta::Signature(sig) => {
									messages::ContentBlockDelta::SignatureDelta { signature: sig }
								},
								bedrock::ReasoningContentBlockDelta::RedactedContent(_) => {
									messages::ContentBlockDelta::ThinkingDelta {
										thinking: "[REDACTED]".to_string(),
									}
								},
								bedrock::ReasoningContentBlockDelta::Unknown => {
									messages::ContentBlockDelta::ThinkingDelta {
										thinking: String::new(),
									}
								},
							},
							bedrock::ContentBlockDelta::ToolUse(tu) => {
								messages::ContentBlockDelta::InputJsonDelta {
									partial_json: tu.input,
								}
							},
						};

						let event = messages::MessagesStreamEvent::ContentBlockDelta {
							index: delta.content_block_index as usize,
							delta: anthropic_delta,
						};
						let (event_name, event_data) = event.into_sse_tuple();
						out.push((event_name, serde_json::to_value(event_data).unwrap()));
					}

					out
				},
				bedrock::ConverseStreamOutput::ContentBlockStop(stop) => {
					seen_blocks.remove(&stop.content_block_index);
					let event = messages::MessagesStreamEvent::ContentBlockStop {
						index: stop.content_block_index as usize,
					};
					let (event_name, event_data) = event.into_sse_tuple();
					vec![(event_name, serde_json::to_value(event_data).unwrap())]
				},
				bedrock::ConverseStreamOutput::MessageStop(stop) => {
					pending_stop_reason = Some(stop.stop_reason);
					vec![]
				},
				bedrock::ConverseStreamOutput::Metadata(meta) => {
					if let Some(usage) = meta.usage {
						pending_usage = Some(usage);
						log.non_atomic_mutate(|r| {
							r.response.output_tokens = Some(usage.output_tokens as u64);
							r.response.input_tokens = Some(usage.input_tokens as u64);
							r.response.total_tokens = Some(usage.total_tokens as u64);
							r.response.cached_input_tokens = usage.cache_read_input_tokens.map(|i| i as u64);
							r.response.cache_creation_input_tokens =
								usage.cache_write_input_tokens.map(|i| i as u64);
						});
					}

					let mut out = Vec::new();
					let stop = pending_stop_reason.take();
					let usage = pending_usage.take();

					if let (Some(stop_reason), Some(usage_data)) = (stop, usage) {
						let event = messages::MessagesStreamEvent::MessageDelta {
							delta: messages::MessageDelta {
								stop_reason: Some(translate_stop_reason(stop_reason)),
								stop_sequence: None,
							},
							usage: to_anthropic_message_delta_usage(usage_data),
						};
						let (event_name, event_data) = event.into_sse_tuple();
						out.push((event_name, serde_json::to_value(event_data).unwrap()));
					}

					let event = messages::MessagesStreamEvent::MessageStop;
					let (event_name, event_data) = event.into_sse_tuple();
					out.push((event_name, serde_json::to_value(event_data).unwrap()));

					out
				},
			}
		})
	}

	pub fn translate_stop_reason(
		stop_reason: bedrock::StopReason,
	) -> types::messages::typed::StopReason {
		match stop_reason {
			bedrock::StopReason::EndTurn => types::messages::typed::StopReason::EndTurn,
			bedrock::StopReason::MaxTokens => types::messages::typed::StopReason::MaxTokens,
			bedrock::StopReason::ModelContextWindowExceeded => {
				types::messages::typed::StopReason::ModelContextWindowExceeded
			},
			bedrock::StopReason::StopSequence => types::messages::typed::StopReason::StopSequence,
			bedrock::StopReason::ToolUse => types::messages::typed::StopReason::ToolUse,
			bedrock::StopReason::ContentFiltered | bedrock::StopReason::GuardrailIntervened => {
				types::messages::typed::StopReason::Refusal
			},
		}
	}

	fn to_anthropic_message_delta_usage(usage: bedrock::TokenUsage) -> messages::MessageDeltaUsage {
		messages::MessageDeltaUsage {
			input_tokens: usage.input_tokens,
			output_tokens: usage.output_tokens,
			cache_creation_input_tokens: usage.cache_write_input_tokens,
			cache_read_input_tokens: usage.cache_read_input_tokens,
		}
	}
}

pub mod from_responses {
	use std::collections::{HashMap, HashSet};
	use std::time::Instant;

	use bytes::Bytes;
	use helpers::*;
	use rand::RngExt;
	use responses::{
		AssistantRole, ErrorObject, FunctionToolCall, IncompleteDetails, InputTokenDetails,
		OutputContent, OutputItem, OutputMessage, OutputStatus, OutputTextContent, OutputTokenDetails,
		ResponseContentPartAddedEvent, ResponseContentPartDoneEvent, ResponseErrorEvent,
		ResponseFunctionCallArgumentsDeltaEvent, ResponseFunctionCallArgumentsDoneEvent,
		ResponseOutputItemAddedEvent, ResponseOutputItemDoneEvent, ResponseStreamEvent,
		ResponseTextDeltaEvent, ResponseUsage,
	};
	use types::bedrock;
	use types::responses::typed as responses;

	use super::helpers;
	use crate::http::Body;
	use crate::llm::bedrock::Provider;
	use crate::llm::types::ResponseType;
	use crate::llm::{AIError, AmendOnDrop, types};

	use crate::{json, parse};

	/// translate an OpenAI responses request to a Bedrock converse request
	pub fn translate(
		req: &types::responses::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		let typed =
			json::convert::<_, responses::CreateResponse>(req).map_err(AIError::RequestMarshal)?;
		let explicit_thinking_budget = extract_responses_thinking_budget_tokens(req);
		let model_id = typed.model.clone().unwrap_or_default();
		let xlated = translate_internal(
			typed,
			explicit_thinking_budget,
			model_id,
			provider,
			headers,
			prompt_caching,
		);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub(super) fn translate_internal(
		req: responses::CreateResponse,
		explicit_thinking_budget: Option<u64>,
		model_id: String,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> bedrock::ConverseRequest {
		use responses::{
			CustomToolCallOutput, CustomToolCallOutputOutput, EasyInputContent, FunctionCallOutput,
			InputContent, InputItem, InputMessage, InputParam, InputRole, InputTextContent, Item,
			MessageItem, OutputMessageContent, Role as ResponsesRole,
		};

		let supports_caching = req.model.as_deref().is_some_and(supports_prompt_caching);

		// Convert input to Bedrock messages and system content
		let mut messages: Vec<bedrock::Message> = Vec::new();
		let mut system_blocks: Vec<bedrock::SystemContentBlock> = Vec::new();

		if let Ok(json) = serde_json::to_string_pretty(&req.input) {
			tracing::debug!("Converting Responses input to Bedrock: {}", json);
		}

		// Convert Input format to items
		let items = match &req.input {
			InputParam::Text(text) => vec![InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: text.clone(),
				})],
				role: InputRole::User,
				status: None,
			})],
			InputParam::Items(items) => items.clone(),
		};

		let input_parts_to_blocks = |parts: &[InputContent]| {
			let mut blocks = Vec::new();
			tracing::debug!("Processing {} content parts", parts.len());
			for part in parts {
				match part {
					InputContent::InputText(input_text) => {
						tracing::debug!("Found InputText with text: {}", input_text.text);
						blocks.push(bedrock::ContentBlock::Text(input_text.text.clone()));
					},
					InputContent::InputImage(_) => {
						// Image support requires fetching URLs or resolving file_ids
						tracing::debug!("Image inputs not supported in Responses->Bedrock translation");
						continue;
					},
					InputContent::InputFile(_) => {
						tracing::debug!("Skipping InputFile");
						continue;
					},
				}
			}
			tracing::debug!("Created {} content blocks", blocks.len());
			blocks
		};

		// Process each input item
		for item in items {
			match item {
				InputItem::EasyMessage(msg) => {
					let role = match msg.role {
						ResponsesRole::User => bedrock::Role::User,
						ResponsesRole::Assistant => bedrock::Role::Assistant,
						ResponsesRole::System | ResponsesRole::Developer => {
							let text = match &msg.content {
								EasyInputContent::Text(text) => text.clone(),
								EasyInputContent::ContentList(parts) => parts
									.iter()
									.filter_map(|part| match part {
										InputContent::InputText(input_text) => Some(input_text.text.clone()),
										_ => None,
									})
									.collect::<Vec<_>>()
									.join("\n"),
							};
							system_blocks.push(bedrock::SystemContentBlock::Text { text });
							continue;
						},
					};

					let content = match &msg.content {
						EasyInputContent::Text(text) => {
							vec![bedrock::ContentBlock::Text(text.clone())]
						},
						EasyInputContent::ContentList(parts) => input_parts_to_blocks(parts),
					};

					messages.push(bedrock::Message { role, content });
				},
				InputItem::Item(Item::Message(MessageItem::Input(msg))) => {
					let role = match msg.role {
						InputRole::User => bedrock::Role::User,
						InputRole::System | InputRole::Developer => {
							let text = msg
								.content
								.iter()
								.filter_map(|part| match part {
									InputContent::InputText(input_text) => Some(input_text.text.clone()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n");
							system_blocks.push(bedrock::SystemContentBlock::Text { text });
							continue;
						},
					};

					let content = input_parts_to_blocks(&msg.content);
					messages.push(bedrock::Message { role, content });
				},
				InputItem::Item(Item::Message(MessageItem::Output(msg))) => {
					let content = msg
						.content
						.iter()
						.filter_map(|part| match part {
							OutputMessageContent::OutputText(output_text) => {
								Some(bedrock::ContentBlock::Text(output_text.text.clone()))
							},
							_ => None,
						})
						.collect::<Vec<_>>();
					if !content.is_empty() {
						messages.push(bedrock::Message {
							role: bedrock::Role::Assistant,
							content,
						});
					}
				},
				InputItem::Item(Item::FunctionCall(call)) => {
					let Ok(input) = serde_json::from_str::<serde_json::Value>(&call.arguments) else {
						tracing::warn!(
							"Skipping function_call with invalid JSON arguments for tool '{}': {}",
							call.name,
							call.arguments
						);
						continue;
					};

					messages.push(bedrock::Message {
						role: bedrock::Role::Assistant,
						content: vec![bedrock::ContentBlock::ToolUse(bedrock::ToolUseBlock {
							tool_use_id: call.call_id,
							name: call.name,
							input,
						})],
					});
				},
				InputItem::Item(Item::FunctionCallOutput(output)) => {
					let output_text = match output.output {
						FunctionCallOutput::Text(text) => text,
						FunctionCallOutput::Content(parts) => parts
							.iter()
							.filter_map(|part| match part {
								InputContent::InputText(input_text) => Some(input_text.text.clone()),
								_ => None,
							})
							.collect::<Vec<_>>()
							.join("\n"),
					};

					messages.push(bedrock::Message {
						role: bedrock::Role::User,
						content: vec![bedrock::ContentBlock::ToolResult(
							bedrock::ToolResultBlock {
								tool_use_id: output.call_id,
								content: vec![bedrock::ToolResultContentBlock::Text(output_text)],
								status: Some(bedrock::ToolResultStatus::Success),
							},
						)],
					});
				},
				InputItem::Item(Item::CustomToolCall(call)) => {
					messages.push(bedrock::Message {
						role: bedrock::Role::Assistant,
						content: vec![bedrock::ContentBlock::ToolUse(bedrock::ToolUseBlock {
							tool_use_id: call.call_id,
							name: call.name,
							input: serde_json::json!({ "input": call.input }),
						})],
					});
				},
				InputItem::Item(Item::CustomToolCallOutput(CustomToolCallOutput {
					call_id,
					output,
					..
				})) => {
					let output_text = match output {
						CustomToolCallOutputOutput::Text(text) => text,
						CustomToolCallOutputOutput::List(parts) => parts
							.iter()
							.filter_map(|part| match part {
								InputContent::InputText(input_text) => Some(input_text.text.clone()),
								_ => None,
							})
							.collect::<Vec<_>>()
							.join("\n"),
					};

					messages.push(bedrock::Message {
						role: bedrock::Role::User,
						content: vec![bedrock::ContentBlock::ToolResult(
							bedrock::ToolResultBlock {
								tool_use_id: call_id,
								content: vec![bedrock::ToolResultContentBlock::Text(output_text)],
								status: Some(bedrock::ToolResultStatus::Success),
							},
						)],
					});
				},
				_ => {
					tracing::debug!("Skipping unsupported Responses input item for Bedrock translation");
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
			let instructions_block = bedrock::SystemContentBlock::Text {
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
				system.push(bedrock::SystemContentBlock::CachePoint {
					cache_point: create_cache_point(),
				});
			}
		}

		let inference_config = bedrock::InferenceConfiguration {
			max_tokens: req.max_output_tokens.unwrap_or(4096) as usize,
			temperature: req.temperature,
			top_p: req.top_p,
			top_k: None,
			stop_sequences: vec![],
		};
		let output_config = req
			.text
			.as_ref()
			.and_then(responses_text_format_to_bedrock_output_config);
		let enabled_thinking_budget = explicit_thinking_budget.or_else(|| {
			req
				.reasoning
				.as_ref()
				.and_then(|r| r.effort.as_ref())
				.and_then(responses_reasoning_effort_to_enabled_budget)
		});
		let additional_model_request_fields = enabled_thinking_budget.map(|budget| {
			serde_json::json!({
				"thinking": {
					"type": "enabled",
					"budget_tokens": budget
				}
			})
		});

		// Convert tools from typed Responses API format to Bedrock format
		let (tools, tool_choice) = if let Some(response_tools) = &req.tools {
			let bedrock_tools: Vec<bedrock::Tool> = response_tools
				.iter()
				.filter_map(|tool_def| {
					use responses::Tool;
					match tool_def {
						Tool::Function(func) => Some(bedrock::Tool::ToolSpec(bedrock::ToolSpecification {
							name: func.name.clone(),
							description: func.description.clone(),
							input_schema: Some(bedrock::ToolInputSchema::Json(
								func.parameters.clone().unwrap_or_default(),
							)),
						})),
						_ => {
							tracing::warn!("Unsupported tool type in Responses API: {:?}", tool_def);
							None
						},
					}
				})
				.collect();

			let bedrock_tool_choice = req.tool_choice.as_ref().and_then(|tc| {
				use responses::{ToolChoiceFunction, ToolChoiceOptions, ToolChoiceParam};
				match tc {
					ToolChoiceParam::Mode(ToolChoiceOptions::Auto) => Some(bedrock::ToolChoice::Auto),
					ToolChoiceParam::Mode(ToolChoiceOptions::Required) => Some(bedrock::ToolChoice::Any),
					ToolChoiceParam::Mode(ToolChoiceOptions::None) => None,
					ToolChoiceParam::Function(ToolChoiceFunction { name }) => {
						Some(bedrock::ToolChoice::Tool { name: name.clone() })
					},
					ToolChoiceParam::Hosted(_) => {
						tracing::warn!("Hosted tool choice not supported for Bedrock");
						None
					},
					ToolChoiceParam::AllowedTools(_)
					| ToolChoiceParam::Mcp(_)
					| ToolChoiceParam::Custom(_)
					| ToolChoiceParam::ApplyPatch
					| ToolChoiceParam::Shell => {
						tracing::warn!("Unsupported tool choice for Bedrock: {:?}", tc);
						None
					},
				}
			});

			(bedrock_tools, bedrock_tool_choice)
		} else {
			(vec![], None)
		};

		let tool_config = if !tools.is_empty() {
			Some(bedrock::ToolConfiguration { tools, tool_choice })
		} else {
			None
		};

		let guardrail_config = if let (Some(identifier), Some(version)) =
			(&provider.guardrail_identifier, &provider.guardrail_version)
		{
			Some(bedrock::GuardrailConfiguration {
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

		let mut bedrock_request = bedrock::ConverseRequest {
			model_id,
			messages,
			system: system_content,
			inference_config: Some(inference_config),
			output_config,
			tool_config,
			guardrail_config,
			additional_model_request_fields,
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
					.push(bedrock::Tool::CachePoint(create_cache_point()));
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

		bedrock_request
	}

	fn extract_responses_thinking_budget_tokens(req: &types::responses::Request) -> Option<u64> {
		req
			.vendor_extensions
			.as_ref()
			.and_then(|v| v.thinking_budget_tokens)
	}

	fn responses_reasoning_effort_to_enabled_budget(
		effort: &responses::ReasoningEffort,
	) -> Option<u64> {
		match effort {
			responses::ReasoningEffort::None => None,
			responses::ReasoningEffort::Minimal | responses::ReasoningEffort::Low => Some(1024),
			responses::ReasoningEffort::Medium => Some(2048),
			responses::ReasoningEffort::High | responses::ReasoningEffort::Xhigh => Some(4096),
		}
	}

	fn responses_text_format_to_bedrock_output_config(
		text: &responses::ResponseTextParam,
	) -> Option<bedrock::OutputConfig> {
		let (name, description, schema) = match &text.format {
			responses::TextResponseFormatConfiguration::Text => return None,
			responses::TextResponseFormatConfiguration::JsonObject => (
				None,
				None,
				std::borrow::Cow::Owned(
					serde_json::json!({ "type": "object", "additionalProperties": true }),
				),
			),
			responses::TextResponseFormatConfiguration::JsonSchema(json_schema) => {
				let Some(schema) = json_schema.schema.as_ref() else {
					tracing::warn!(
						"Dropping text.format.json_schema for Bedrock conversion because schema is missing"
					);
					return None;
				};
				(
					Some(json_schema.name.clone()),
					json_schema.description.clone(),
					std::borrow::Cow::Borrowed(schema),
				)
			},
		};

		let Ok(schema_json) = serde_json::to_string(schema.as_ref()) else {
			tracing::warn!("Dropping text.format for Bedrock conversion: schema is not serializable");
			return None;
		};

		Some(bedrock::OutputConfig {
			text_format: Some(bedrock::OutputFormat {
				r#type: bedrock::OutputFormatType::JsonSchema,
				structure: bedrock::OutputFormatStructure {
					json_schema: bedrock::JsonSchemaDefinition {
						schema: schema_json,
						name,
						description,
					},
				},
			}),
		})
	}

	pub fn translate_response(bytes: &Bytes, model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<bedrock::ConverseResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
		let adapter = super::ConverseResponseAdapter::from_response(resp, model)?;
		let typed = adapter.to_responses_typed();
		let mut passthrough =
			json::convert::<_, types::responses::Response>(&typed).map_err(AIError::ResponseParsing)?;
		passthrough.rest = serde_json::Value::Object(serde_json::Map::new());
		if let Some(usage) = passthrough.usage.as_mut() {
			usage.rest = serde_json::Value::Object(serde_json::Map::new());
		}
		if matches!(adapter.stop_reason, bedrock::StopReason::ToolUse) {
			passthrough.status = "requires_action".to_string();
		}
		Ok(Box::new(passthrough))
	}

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		let res = serde_json::from_slice::<bedrock::ConverseErrorResponse>(bytes)
			.map_err(AIError::ResponseMarshal)?;
		let m = crate::llm::types::completions::typed::ChatCompletionErrorResponse {
			event_id: None,
			error: crate::llm::types::completions::typed::ChatCompletionError {
				r#type: "invalid_request_error".to_string(),
				message: res.message,
				param: None,
				code: None,
				event_id: None,
			},
		};
		Ok(Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
	}

	pub fn translate_stream(
		b: Body,
		_buffer_limit: usize,
		log: AmendOnDrop,
		model: &str,
		_message_id: &str,
	) -> Body {
		let mut saw_token = false;
		let mut pending_stop_reason: Option<bedrock::StopReason> = None;
		let mut pending_usage: Option<bedrock::TokenUsage> = None;
		let mut seen_blocks: HashSet<i32> = HashSet::new();

		// Track tool calls for streaming: (index -> (item_id, name, json_buffer))
		let mut tool_calls: HashMap<i32, (String, String, String)> = HashMap::new();

		// Track sequence numbers and item IDs
		let mut sequence_number: u64 = 0;
		let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());

		// Track message item ID for text content
		let message_item_id = format!("msg_{:016x}", rand::rng().random::<u64>());
		let model = model.to_string();

		let response_builder =
			crate::llm::types::responses::ResponseBuilder::new(response_id.clone(), model.clone());

		let make_output_part = |text: String| {
			OutputContent::OutputText(OutputTextContent {
				annotations: Vec::new(),
				logprobs: None,
				text,
			})
		};

		parse::aws_sse::transform_multi(b, move |aws_event| {
			tracing::debug!("Raw AWS event - headers: {:?}", aws_event.headers());
			if let Ok(body_str) = std::str::from_utf8(aws_event.payload()) {
				tracing::debug!("AWS event body: {}", body_str);
			}

			let event = match bedrock::ConverseStreamOutput::deserialize(aws_event) {
				Ok(e) => e,
				Err(e) => {
					tracing::error!(error = %e, "failed to deserialize bedrock stream event");
					sequence_number += 1;
					return vec![(
						"error",
						ResponseStreamEvent::ResponseError(ResponseErrorEvent {
							sequence_number,
							code: None,
							message: "Stream processing error".to_string(),
							param: None,
						}),
					)];
				},
			};

			match event {
				bedrock::ConverseStreamOutput::MessageStart(_start) => {
					let mut events: Vec<(&'static str, ResponseStreamEvent)> = Vec::new();

					sequence_number += 1;
					let created_event = response_builder.created_event(sequence_number);
					events.push(("event", created_event));

					sequence_number += 1;
					let item_added_event =
						ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
							sequence_number,
							output_index: 0,
							item: OutputItem::Message(OutputMessage {
								content: Vec::new(),
								id: message_item_id.clone(),
								role: AssistantRole::Assistant,
								status: OutputStatus::InProgress,
							}),
						});
					events.push(("event", item_added_event));

					events
				},
				bedrock::ConverseStreamOutput::ContentBlockStart(start) => {
					seen_blocks.insert(start.content_block_index);

					match start.start {
						Some(bedrock::ContentBlockStart::ToolUse(tu)) => {
							let tool_call_item_id = format!("call_{:016x}", rand::rng().random::<u64>());
							tool_calls.insert(
								start.content_block_index,
								(tool_call_item_id.clone(), tu.name.clone(), String::new()),
							);

							sequence_number += 1;
							let item_added_event =
								ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
									sequence_number,
									output_index: start.content_block_index as u32,
									item: OutputItem::FunctionCall(FunctionToolCall {
										arguments: String::new(),
										call_id: tool_call_item_id.clone(),
										name: tu.name,
										id: Some(tool_call_item_id),
										status: Some(OutputStatus::InProgress),
									}),
								});

							vec![("event", item_added_event)]
						},
						_ => {
							sequence_number += 1;
							let part_added_event =
								ResponseStreamEvent::ResponseContentPartAdded(ResponseContentPartAddedEvent {
									sequence_number,
									item_id: message_item_id.clone(),
									output_index: start.content_block_index as u32,
									content_index: 0,
									part: make_output_part(String::new()),
								});

							vec![("event", part_added_event)]
						},
					}
				},
				bedrock::ConverseStreamOutput::ContentBlockDelta(delta) => {
					let mut out: Vec<(&'static str, ResponseStreamEvent)> = Vec::new();

					if !saw_token {
						saw_token = true;
						log.non_atomic_mutate(|r| {
							r.response.first_token = Some(Instant::now());
						});
					}

					if let Some(d) = delta.delta {
						match d {
							bedrock::ContentBlockDelta::Text(text) => {
								sequence_number += 1;
								let delta_event =
									ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
										sequence_number,
										item_id: message_item_id.clone(),
										output_index: delta.content_block_index as u32,
										content_index: 0,
										delta: text,
										logprobs: None,
									});
								out.push(("event", delta_event));
							},
							bedrock::ContentBlockDelta::ReasoningContent(rc) => match rc {
								bedrock::ReasoningContentBlockDelta::Text(t) => {
									sequence_number += 1;
									let delta_event =
										ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
											sequence_number,
											item_id: message_item_id.clone(),
											output_index: delta.content_block_index as u32,
											content_index: 0,
											delta: t,
											logprobs: None,
										});
									out.push(("event", delta_event));
								},
								bedrock::ReasoningContentBlockDelta::RedactedContent(_) => {
									sequence_number += 1;
									let delta_event =
										ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
											sequence_number,
											item_id: message_item_id.clone(),
											output_index: delta.content_block_index as u32,
											content_index: 0,
											delta: "[REDACTED]".to_string(),
											logprobs: None,
										});
									out.push(("event", delta_event));
								},
								_ => {},
							},
							bedrock::ContentBlockDelta::ToolUse(tu) => {
								if let Some((item_id, _name, buffer)) =
									tool_calls.get_mut(&delta.content_block_index)
								{
									buffer.push_str(&tu.input);

									sequence_number += 1;
									let delta_event = ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
										ResponseFunctionCallArgumentsDeltaEvent {
											sequence_number,
											item_id: item_id.clone(),
											output_index: delta.content_block_index as u32,
											delta: tu.input,
										},
									);
									out.push(("event", delta_event));
								}
							},
						}
					}

					out
				},
				bedrock::ConverseStreamOutput::ContentBlockStop(stop) => {
					let mut events: Vec<(&'static str, ResponseStreamEvent)> = Vec::new();

					if let Some((item_id, name, buffer)) = tool_calls.remove(&stop.content_block_index) {
						sequence_number += 1;
						let args_done_event = ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
							ResponseFunctionCallArgumentsDoneEvent {
								name: Some(name.clone()),
								sequence_number,
								item_id: item_id.clone(),
								output_index: stop.content_block_index as u32,
								arguments: buffer.clone(),
							},
						);
						events.push(("event", args_done_event));

						sequence_number += 1;
						let item_done_event =
							ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
								sequence_number,
								output_index: stop.content_block_index as u32,
								item: OutputItem::FunctionCall(FunctionToolCall {
									arguments: buffer,
									call_id: item_id.clone(),
									name,
									id: Some(item_id),
									status: Some(OutputStatus::Completed),
								}),
							});
						events.push(("event", item_done_event));
					} else if seen_blocks.remove(&stop.content_block_index) {
						sequence_number += 1;
						let part_done_event =
							ResponseStreamEvent::ResponseContentPartDone(ResponseContentPartDoneEvent {
								sequence_number,
								item_id: message_item_id.clone(),
								output_index: stop.content_block_index as u32,
								content_index: 0,
								part: make_output_part(String::new()),
							});
						events.push(("event", part_done_event));
					}

					events
				},
				bedrock::ConverseStreamOutput::MessageStop(stop) => {
					pending_stop_reason = Some(stop.stop_reason);
					vec![]
				},
				bedrock::ConverseStreamOutput::Metadata(meta) => {
					if let Some(usage) = meta.usage {
						pending_usage = Some(usage);
						log.non_atomic_mutate(|r| {
							r.response.output_tokens = Some(usage.output_tokens as u64);
							r.response.input_tokens = Some(usage.input_tokens as u64);
							r.response.total_tokens = Some(usage.total_tokens as u64);
							r.response.cached_input_tokens = usage.cache_read_input_tokens.map(|i| i as u64);
							r.response.cache_creation_input_tokens =
								usage.cache_write_input_tokens.map(|i| i as u64);
						});
					}

					let mut out: Vec<(&'static str, ResponseStreamEvent)> = Vec::new();

					sequence_number += 1;
					let message_done_event =
						ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
							sequence_number,
							output_index: 0,
							item: OutputItem::Message(OutputMessage {
								content: Vec::new(),
								id: message_item_id.clone(),
								role: AssistantRole::Assistant,
								status: OutputStatus::Completed,
							}),
						});
					out.push(("event", message_done_event));

					let stop = pending_stop_reason.take();
					let usage_data = pending_usage.take();

					let usage_obj = usage_data.map(|u| ResponseUsage {
						input_tokens: u.input_tokens as u32,
						output_tokens: u.output_tokens as u32,
						total_tokens: (u.input_tokens + u.output_tokens) as u32,
						input_tokens_details: InputTokenDetails {
							cached_tokens: u.cache_read_input_tokens.unwrap_or(0) as u32,
						},
						output_tokens_details: OutputTokenDetails {
							reasoning_tokens: 0,
						},
					});

					sequence_number += 1;
					let done_event = match stop {
						Some(bedrock::StopReason::EndTurn) | Some(bedrock::StopReason::StopSequence) | None => {
							response_builder.completed_event(sequence_number, usage_obj)
						},
						Some(bedrock::StopReason::MaxTokens)
						| Some(bedrock::StopReason::ModelContextWindowExceeded) => response_builder.incomplete_event(
							sequence_number,
							usage_obj,
							IncompleteDetails {
								reason: "max_tokens".to_string(),
							},
						),
						Some(bedrock::StopReason::ContentFiltered)
						| Some(bedrock::StopReason::GuardrailIntervened) => response_builder.failed_event(
							sequence_number,
							usage_obj,
							ErrorObject {
								code: "content_filter".to_string(),
								message: "Content filtered by guardrails".to_string(),
							},
						),
						Some(bedrock::StopReason::ToolUse) => {
							response_builder.completed_event(sequence_number, usage_obj)
						},
					};

					out.push(("event", done_event));
					out
				},
			}
		})
	}
}

pub mod from_anthropic_token_count {
	use crate::llm::types::RequestType;
	use crate::llm::{AIError, types};

	pub fn translate(
		req: &types::count_tokens::Request,
		headers: &http::HeaderMap,
	) -> Result<Vec<u8>, AIError> {
		use base64::Engine;
		let anthropic_version = headers
			.get("anthropic-version")
			.and_then(|v| v.to_str().ok())
			.unwrap_or("2023-06-01");

		let body = req.to_anthropic()?;
		let mut body: serde_json::Map<String, serde_json::Value> =
			serde_json::from_slice(&body).map_err(AIError::RequestMarshal)?;

		// Remove the model field because its in the URL path not the body
		body.remove("model");

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

		let xlated = types::bedrock::CountTokensRequest {
			input: types::bedrock::CountTokensInputInvokeModel {
				invoke_model: types::bedrock::InvokeModelBody { body: body_b64 },
			},
		};
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}
}

mod helpers {
	use std::collections::HashMap;

	use crate::llm::AIError;
	use crate::llm::types::bedrock;

	pub fn create_cache_point() -> bedrock::CachePointBlock {
		bedrock::CachePointBlock {
			r#type: bedrock::CachePointType::Default,
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

	pub fn estimate_system_tokens(system: &[bedrock::SystemContentBlock]) -> usize {
		let word_count: usize = system
			.iter()
			.filter_map(|block| match block {
				bedrock::SystemContentBlock::Text { text } => Some(text.split_whitespace().count()),
				bedrock::SystemContentBlock::CachePoint { .. } => None,
			})
			.sum();
		(word_count * 13) / 10
	}

	pub fn insert_cache_point_in_last_user_message(messages: &mut [bedrock::Message]) {
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
			.push(bedrock::ContentBlock::CachePoint(create_cache_point()));

		tracing::debug!(
			"Inserted cachePoint before last message (in message at index {})",
			second_to_last_idx
		);
	}

	/// Extract metadata from x-bedrock-metadata header.
	/// Gateway operators can use CEL transformation to populate this header with extauthz data.
	pub fn extract_metadata_from_headers(
		headers: Option<&crate::http::HeaderMap>,
	) -> Option<HashMap<String, String>> {
		const BEDROCK_METADATA_HEADER: &str = "x-bedrock-metadata";

		let header_value = headers?.get(BEDROCK_METADATA_HEADER)?;
		let json_str = header_value.to_str().ok()?;
		let json = serde_json::from_str::<serde_json::Value>(json_str).ok()?;
		Some(extract_flat_metadata(&json))
	}

	/// Extract flat key-value pairs from JSON for Bedrock requestMetadata.
	/// Only extracts top-level primitive values (strings, numbers, booleans).
	pub fn extract_flat_metadata(value: &serde_json::Value) -> HashMap<String, String> {
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

	pub fn extract_beta_headers(
		headers: &crate::http::HeaderMap,
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

	pub fn generate_anthropic_message_id() -> String {
		let timestamp = chrono::Utc::now().timestamp_millis();
		let random: u32 = rand::random();
		format!("msg_{:x}{:08x}", timestamp, random)
	}
}

struct ConverseResponseAdapter {
	model: String,
	stop_reason: bedrock::StopReason,
	usage: Option<bedrock::TokenUsage>,
	message: bedrock::Message,
}

impl ConverseResponseAdapter {
	fn from_response(resp: bedrock::ConverseResponse, model: &str) -> Result<Self, AIError> {
		let bedrock::ConverseResponse {
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
			Some(bedrock::ConverseOutput::Message(msg)) => msg,
			_ => return Err(AIError::IncompleteResponse),
		};

		Ok(Self {
			model: model.to_string(),
			stop_reason,
			usage,
			message,
		})
	}

	fn to_completions(&self) -> crate::llm::types::completions::typed::Response {
		use crate::llm::types::completions::typed as completions;
		let mut tool_calls: Vec<completions::MessageToolCalls> = Vec::new();
		let mut content = None;
		let mut reasoning_content = None;
		for block in &self.message.content {
			match block {
				bedrock::ContentBlock::Text(text) => {
					content = Some(text.clone());
				},
				bedrock::ContentBlock::ReasoningContent(reasoning) => {
					// Extract text from either format
					let text = match reasoning {
						bedrock::ReasoningContentBlock::Structured { reasoning_text } => {
							reasoning_text.text.clone()
						},
						bedrock::ReasoningContentBlock::Simple { text } => text.clone(),
					};
					reasoning_content = Some(text);
				},
				bedrock::ContentBlock::ToolUse(tu) => {
					let Some(args) = serde_json::to_string(&tu.input).ok() else {
						continue;
					};
					tool_calls.push(completions::MessageToolCalls::Function(
						completions::MessageToolCall {
							id: tu.tool_use_id.clone(),
							function: completions::FunctionCall {
								name: tu.name.clone(),
								arguments: args,
							},
						},
					));
				},
				bedrock::ContentBlock::Image(_)
				| bedrock::ContentBlock::ToolResult(_)
				| bedrock::ContentBlock::CachePoint(_) => {
					continue;
				},
			}
		}

		let message = completions::ResponseMessage {
			role: completions::Role::Assistant,
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

		let choice = completions::ChatChoice {
			index: 0,
			message,
			finish_reason: Some(from_completions::translate_stop_reason(&self.stop_reason)),
			logprobs: None,
		};

		let usage = self
			.usage
			.map(|token_usage| completions::Usage {
				prompt_tokens: token_usage.input_tokens as u32,
				completion_tokens: token_usage.output_tokens as u32,
				total_tokens: token_usage.total_tokens as u32,
				completion_tokens_details: None,

				cache_read_input_tokens: token_usage.cache_read_input_tokens.map(|i| i as u64),
				prompt_tokens_details: token_usage
					.cache_read_input_tokens
					.map(|i| UsagePromptDetails {
						cached_tokens: Some(i as u64),
					}),
				cache_creation_input_tokens: token_usage.cache_write_input_tokens.map(|i| i as u64),
			})
			.unwrap_or_default();

		completions::Response {
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

	fn to_responses_typed(&self) -> responses::typed::Response {
		use crate::llm::types::responses::typed as responsest;
		let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());
		let response_builder =
			crate::llm::types::responses::ResponseBuilder::new(response_id, self.model.clone());

		// Convert Bedrock content blocks to Responses OutputItem
		let mut outputs: Vec<responsest::OutputItem> = Vec::new();

		// Group content by type for proper message construction
		let mut text_parts: Vec<responsest::OutputMessageContent> = Vec::new();
		let mut tool_calls: Vec<responsest::OutputItem> = Vec::new();

		for block in &self.message.content {
			match block {
				bedrock::ContentBlock::Text(text) => {
					text_parts.push(responsest::OutputMessageContent::OutputText(
						responsest::OutputTextContent {
							annotations: vec![],
							logprobs: None,
							text: text.clone(),
						},
					));
				},
				bedrock::ContentBlock::ReasoningContent(reasoning) => {
					let text = match reasoning {
						bedrock::ReasoningContentBlock::Structured { reasoning_text } => {
							reasoning_text.text.clone()
						},
						bedrock::ReasoningContentBlock::Simple { text } => text.clone(),
					};
					text_parts.push(responsest::OutputMessageContent::OutputText(
						responsest::OutputTextContent {
							annotations: vec![],
							logprobs: None,
							text,
						},
					));
				},
				bedrock::ContentBlock::ToolUse(tool_use) => {
					let arguments_str = serde_json::to_string(&tool_use.input).unwrap_or_default();
					tool_calls.push(responsest::OutputItem::FunctionCall(
						responsest::FunctionToolCall {
							arguments: arguments_str,
							call_id: tool_use.tool_use_id.clone(),
							name: tool_use.name.clone(),
							id: Some(tool_use.tool_use_id.clone()),
							status: Some(responsest::OutputStatus::Completed),
						},
					));
				},
				bedrock::ContentBlock::Image(_)
				| bedrock::ContentBlock::ToolResult(_)
				| bedrock::ContentBlock::CachePoint(_) => {
					// Skip these in responses (not part of output)
				},
			}
		}

		if !text_parts.is_empty() {
			outputs.push(responsest::OutputItem::Message(responsest::OutputMessage {
				id: format!("msg_{:016x}", rand::rng().random::<u64>()),
				role: responsest::AssistantRole::Assistant,
				content: text_parts,
				status: responsest::OutputStatus::Completed,
			}));
		}

		outputs.extend(tool_calls);

		let output = outputs;

		// Determine status from stop reason
		let status = match self.stop_reason {
			bedrock::StopReason::EndTurn | bedrock::StopReason::StopSequence => {
				responsest::Status::Completed
			},
			bedrock::StopReason::MaxTokens | bedrock::StopReason::ModelContextWindowExceeded => {
				responsest::Status::Incomplete
			},
			bedrock::StopReason::ToolUse => responsest::Status::Completed,
			bedrock::StopReason::ContentFiltered | bedrock::StopReason::GuardrailIntervened => {
				responsest::Status::Failed
			},
		};

		let incomplete_details = match self.stop_reason {
			bedrock::StopReason::MaxTokens | bedrock::StopReason::ModelContextWindowExceeded => {
				Some(responsest::IncompleteDetails {
					reason: "max_tokens".to_string(),
				})
			},
			_ => None,
		};

		let error = match self.stop_reason {
			bedrock::StopReason::ContentFiltered | bedrock::StopReason::GuardrailIntervened => {
				Some(responsest::ErrorObject {
					code: "content_filter".to_string(),
					message: "Content filtered by guardrails".to_string(),
				})
			},
			_ => None,
		};

		// Build usage
		let usage = self.usage.map(|u| responsest::ResponseUsage {
			input_tokens: u.input_tokens as u32,
			output_tokens: u.output_tokens as u32,
			total_tokens: (u.input_tokens + u.output_tokens) as u32,
			input_tokens_details: responsest::InputTokenDetails {
				cached_tokens: u.cache_read_input_tokens.unwrap_or(0) as u32,
			},
			output_tokens_details: responsest::OutputTokenDetails {
				reasoning_tokens: 0,
			},
		});

		let mut response = response_builder.response(status, usage, error, incomplete_details);
		response.output = output;
		response
	}

	fn to_anthropic(&self) -> Result<messages::typed::MessagesResponse, AIError> {
		use crate::llm::types::messages::typed as messagest;
		fn translate_content_block_to_anthropic(
			block: &bedrock::ContentBlock,
		) -> Option<messagest::ContentBlock> {
			match block {
				bedrock::ContentBlock::Text(text) => {
					Some(messagest::ContentBlock::Text(messagest::ContentTextBlock {
						text: text.clone(),
						citations: None,
						cache_control: None,
					}))
				},
				bedrock::ContentBlock::ReasoningContent(reasoning) => {
					// Extract text and signature from either format
					let (thinking_text, signature) = match reasoning {
						bedrock::ReasoningContentBlock::Structured { reasoning_text } => (
							reasoning_text.text.clone(),
							reasoning_text.signature.clone().unwrap_or_default(),
						),
						bedrock::ReasoningContentBlock::Simple { text } => (text.clone(), String::new()),
					};
					Some(messagest::ContentBlock::Thinking {
						thinking: thinking_text,
						signature,
					})
				},
				bedrock::ContentBlock::ToolUse(tool_use) => Some(messagest::ContentBlock::ToolUse {
					id: tool_use.tool_use_id.clone(),
					name: tool_use.name.clone(),
					input: tool_use.input.clone(),
					cache_control: None,
				}),
				bedrock::ContentBlock::Image(img) => Some(messagest::ContentBlock::Image(
					messagest::ContentImageBlock {
						source: serde_json::json!({
							"type": "base64",
							"media_type": format!("image/{}", img.format),
							"data": img.source.bytes
						}),
						cache_control: None,
					},
				)),
				bedrock::ContentBlock::ToolResult(_) => None, // Skip tool results in responses
				bedrock::ContentBlock::CachePoint(_) => None, // Skip cache points - they're metadata only
			}
		}
		let content: Vec<messagest::ContentBlock> = self
			.message
			.content
			.iter()
			.filter_map(translate_content_block_to_anthropic)
			.collect();

		let usage = self
			.usage
			.map(|u| messagest::Usage {
				input_tokens: u.input_tokens,
				output_tokens: u.output_tokens,
				cache_creation_input_tokens: u.cache_write_input_tokens,
				cache_read_input_tokens: u.cache_read_input_tokens,
			})
			.unwrap_or(messagest::Usage {
				input_tokens: 0,
				output_tokens: 0,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
			});

		Ok(messagest::MessagesResponse {
			id: helpers::generate_anthropic_message_id(),
			r#type: "message".to_string(),
			role: messagest::Role::Assistant,
			content,
			model: self.model.clone(),
			stop_reason: Some(from_messages::translate_stop_reason(self.stop_reason)),
			stop_sequence: None,
			usage,
		})
	}
}

pub fn message_id(resp: &Response) -> String {
	resp
		.headers()
		.get(crate::http::x_headers::X_AMZN_REQUESTID)
		.and_then(|s| s.to_str().ok().map(|s| s.to_owned()))
		.unwrap_or_else(|| format!("{:016x}", rand::rng().random::<u64>()))
}
