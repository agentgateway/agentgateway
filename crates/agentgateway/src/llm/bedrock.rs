use agent_core::prelude::Strng;
use agent_core::strng;
// Remove unused import - using universal::FinishReason instead
use bytes::Bytes;
use chrono;
use rand::Rng;
use tracing::trace;

use crate::http::Response;
use crate::llm::bedrock::types::{
	ContentBlock, ContentBlockDelta, ConverseErrorResponse, ConverseRequest, ConverseResponse,
	ConverseStreamOutput, StopReason,
};
use crate::llm::{AIError, BackendAdapter, LLMResponse, universal};
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
	const NAME: Strng = strng::literal!("bedrock");
}

// Implement BackendAdapter trait for Provider
impl BackendAdapter for Provider {
	type BReq = ConverseRequest;
	type BResp = ConverseResponse;
	type BStream = ConverseStreamOutput;

	/// Convert OpenAI request to Bedrock ConverseRequest format
	fn to_backend(&self, req: &universal::Request) -> Result<Self::BReq, AIError> {
		// Use provider's model if configured, otherwise use request model
		let model = if let Some(provider_model) = &self.model {
			provider_model.to_string()
		} else if let Some(req_model) = &req.model {
			req_model.clone()
		} else {
			return Err(AIError::MissingField("model not specified".into()));
		};

		// Use new direct OpenAI → Bedrock conversion
		Ok(translate_openai_request(req, self, &model))
	}

	/// Convert Bedrock ConverseResponse to OpenAI response format
	fn from_backend(&self, bresp: Self::BResp, model_id: &str) -> Result<universal::Response, AIError> {
		// Use existing translate_response method that already handles Bedrock → OpenAI conversion
		translate_response(bresp, self.model.as_deref().unwrap_or(model_id))
	}

	/// Convert Bedrock streaming events to OpenAI streaming format
	fn stream_map(&mut self, ev: Self::BStream) -> Result<Vec<universal::ChatChoiceStream>, AIError> {
		// Convert Bedrock streaming events to OpenAI ChatChoiceStream format
		let mut choices = Vec::new();

		match ev {
			ConverseStreamOutput::MessageStart(start) => {
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta {
						role: Some(match start.role {
							types::Role::Assistant => universal::Role::Assistant,
							types::Role::User => universal::Role::User,
						}),
						content: None,
						refusal: None,
						#[allow(deprecated)]
						function_call: None,
						tool_calls: None,
					},
					finish_reason: None,
				};
				choices.push(choice);
			},
			ConverseStreamOutput::ContentBlockDelta(delta) => {
				if let ContentBlockDelta::Text { text } = &delta.delta {
					let choice = universal::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta: universal::StreamResponseDelta {
							role: None,
							content: Some(text.clone()),
							refusal: None,
							#[allow(deprecated)]
							function_call: None,
							tool_calls: None,
						},
						finish_reason: None,
					};
					choices.push(choice);
				}
				// TODO: Handle ToolUse deltas for tool_calls
			},
			ConverseStreamOutput::MessageStop(stop) => {
				let finish_reason = Some(translate_stop_reason(&stop.stop_reason));
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta {
						role: None,
						content: None,
						refusal: None,
						#[allow(deprecated)]
						function_call: None,
						tool_calls: None,
					},
					finish_reason,
				};
				choices.push(choice);
			},
			// Skip other events for now - they don't map directly to OpenAI streaming format
			_ => {},
		}

		Ok(choices)
	}
}

impl Provider {
	pub async fn process_request(
		&self,
		mut req: universal::Request,
	) -> Result<ConverseRequest, AIError> {
		// Use provider's model if configured, otherwise keep the request model
		if let Some(provider_model) = &self.model {
			req.model = Some(provider_model.to_string());
		} else if req.model.is_none() {
			return Err(AIError::MissingField("model not specified".into()));
		}

		// Use direct OpenAI → Bedrock conversion
		Ok(translate_openai_request(&req, self, req.model.as_deref().unwrap_or_default()))
	}

	pub async fn process_response(
		&self,
		model: &str,
		bytes: &Bytes,
	) -> Result<universal::Response, AIError> {
		let model = self.model.as_deref().unwrap_or(model);
		let resp =
			serde_json::from_slice::<ConverseResponse>(bytes).map_err(AIError::ResponseParsing)?;

		// Bedrock response doesn't contain the model, so we pass through the model from the request into the response
		translate_response(resp, model)
	}

	pub async fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp =
			serde_json::from_slice::<ConverseErrorResponse>(bytes).map_err(AIError::ResponseParsing)?;
		translate_error(resp)
	}

	/// Process streaming responses with direct Anthropic Messages API SSE output
	pub(super) async fn process_streaming_messages(
		&self,
		log: AsyncLog<LLMResponse>,
		rate_limit: crate::store::LLMResponsePolicies,
		resp: Response,
		model: &str,
	) -> Response {
		// Extract message ID from AWS Request ID header if available, otherwise use random ID
		// Always prefix with msg_ for Anthropic client compatibility
		let message_id = resp
			.headers()
			.get(crate::http::x_headers::X_AMZN_REQUESTID)
			.and_then(|s| s.to_str().ok().map(|s| format!("msg_{}", s)))
			.unwrap_or_else(|| format!("msg_{:016x}", rand::rng().random::<u64>()));

		let (parts, body) = resp.into_parts();
		let anthropic_body = streaming::AnthropicStreamBody::new(
			body,
			message_id,
			self.model.as_deref().unwrap_or(model).to_string(),
			log,
			Some(rate_limit),
		);
		let mut sse_response = Response::from_parts(parts, crate::http::Body::new(anthropic_body));
		let headers = sse_response.headers_mut();
		headers.insert(
			"content-type",
			http::HeaderValue::from_static("text/event-stream; charset=utf-8"),
		);
		headers.insert("cache-control", http::HeaderValue::from_static("no-cache"));
		headers.insert("connection", http::HeaderValue::from_static("keep-alive"));
		headers.insert("x-accel-buffering", http::HeaderValue::from_static("no"));
		headers.remove("content-length");

		sse_response
	}

	pub(super) async fn process_streaming(
		&self,
		log: AsyncLog<LLMResponse>,
		resp: Response,
		model: &str,
	) -> Response {
		let model = self.model.as_deref().unwrap_or(model).to_string();
		// Bedrock doesn't return an ID, so get one from the request... if we can
		let message_id = resp
			.headers()
			.get(http::x_headers::X_AMZN_REQUESTID)
			.and_then(|s| s.to_str().ok().map(|s| s.to_owned()))
			.unwrap_or_else(|| format!("{:016x}", rand::rng().random::<u64>()));
		// This is static for all chunks!
		let created = chrono::Utc::now().timestamp() as u32;
		resp.map(move |b| {
			let mut saw_token = false;
			parse::aws_sse::transform::<universal::StreamResponse>(b, move |f| {
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
					ConverseStreamOutput::ContentBlockDelta(d) => {
						if !saw_token {
							saw_token = true;
							log.non_atomic_mutate(|r| {
								r.first_token = Some(Instant::now());
							});
						}
						match &d.delta {
							ContentBlockDelta::Text { text: s } => {
								let choice = universal::ChatChoiceStream {
									index: 0,
									logprobs: None,
									delta: universal::StreamResponseDelta {
										role: None,
										content: Some(s.clone()),
										refusal: None,
										#[allow(deprecated)]
										function_call: None,
										tool_calls: None,
									},
									finish_reason: None,
								};
								mk(vec![choice], None)
							},
							_ => None,
						}
					},
					ConverseStreamOutput::ContentBlockStart(_) => {
						// TODO support tool calls
						None
					},
					ConverseStreamOutput::ContentBlockStop(_) => {
						// No need to send anything here
						None
					},
					ConverseStreamOutput::MessageStart(start) => {
						// Just send a blob with the role
						let choice = universal::ChatChoiceStream {
							index: 0,
							logprobs: None,
							delta: universal::StreamResponseDelta {
								role: Some(match start.role {
									types::Role::Assistant => universal::Role::Assistant,
									types::Role::User => universal::Role::User,
								}),
								content: None,
								refusal: None,
								#[allow(deprecated)]
								function_call: None,
								tool_calls: None,
							},
							finish_reason: None,
						};
						mk(vec![choice], None)
					},
					ConverseStreamOutput::MessageStop(stop) => {
						let finish_reason = Some(translate_stop_reason(&stop.stop_reason));

						// Just send a blob with the finish reason
						let choice = universal::ChatChoiceStream {
							index: 0,
							logprobs: None,
							delta: universal::StreamResponseDelta {
								role: None,
								content: None,
								refusal: None,
								#[allow(deprecated)]
								function_call: None,
								tool_calls: None,
							},
							finish_reason,
						};
						mk(vec![choice], None)
					},
					ConverseStreamOutput::Metadata(metadata) => {
						if let Some(usage) = metadata.usage {
							log.non_atomic_mutate(|r| {
								r.output_tokens = Some(usage.output_tokens as u64);
								r.input_tokens_from_response = Some(usage.input_tokens as u64);
								r.total_tokens = Some(usage.total_tokens as u64);
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
					ConverseStreamOutput::InternalServerException(_error_event) => {
						None
					},
					ConverseStreamOutput::ModelStreamErrorException(_error_event) => {
						None
					},
					ConverseStreamOutput::ServiceUnavailableException(_error_event) => {
						None
					},
					ConverseStreamOutput::ThrottlingException(_error_event) => {
						None
					},
					ConverseStreamOutput::ValidationException(_error_event) => {
						None
					},
				}
			})
		})
	}

	pub fn get_path_for_model(&self, streaming: bool, model: &str) -> Strng {
		let model = self.model.as_deref().unwrap_or(model);
		if streaming {
			strng::format!("/model/{model}/converse-stream")
		} else {
			strng::format!("/model/{model}/converse")
		}
	}

	pub fn get_host(&self) -> Strng {
		strng::format!("bedrock-runtime.{}.amazonaws.com", self.region)
	}
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

pub(super) fn translate_response(
	resp: ConverseResponse,
	model: &str,
) -> Result<universal::Response, AIError> {
	// Get the output content from the response
	let output = resp.output.ok_or(AIError::IncompleteResponse)?;

	// Extract the message from the output
	let message = match output {
		types::ConverseOutput::Message { message: msg } => msg,
	};
	// Bedrock has a vec of possible content types, while openai allows 1 text content and many tool calls
	// Assume the bedrock response has only one text
	// Convert Bedrock content blocks to OpenAI message content
	let mut tool_calls: Vec<universal::MessageToolCall> = Vec::new();
	let mut content = None;
	for block in &message.content {
		match block {
			ContentBlock::Text(text) => {
				content = Some(text.clone());
			},
			ContentBlock::Image { .. } => continue, // Skip images in response for now
			ContentBlock::ToolResult(_) => {
				// There should not be a ToolResult in the response, only in the request
				continue;
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
			ContentBlock::Document(_) => continue, // Skip documents for now
			ContentBlock::CachePoint(_) => continue, // Skip cache points
			ContentBlock::ReasoningContent(_) => continue, // Skip reasoning content for now
		};
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
	};
	let finish_reason = resp.stop_reason.as_ref().map(translate_stop_reason);
	// Only one choice for Bedrock
	let choice = universal::ChatChoice {
		index: 0,
		message,
		finish_reason,
		logprobs: None,
	};
	let choices = vec![choice];

	// Convert usage from Bedrock format to OpenAI format
	let usage = if let Some(token_usage) = resp.usage {
		universal::Usage {
			prompt_tokens: token_usage.input_tokens as u32,
			completion_tokens: token_usage.output_tokens as u32,
			total_tokens: token_usage.total_tokens as u32,

			prompt_tokens_details: None,
			completion_tokens_details: None,
		}
	} else {
		// Fallback if usage is not provided
		universal::Usage::default()
	};

	// Generate a unique ID since it's not provided in the response
	let id = format!("bedrock-{}", chrono::Utc::now().timestamp_millis());

	// Log guardrail trace information if present
	if let Some(trace) = &resp.trace
		&& let Some(guardrail_trace) = &trace.guardrail
	{
		trace!("Bedrock guardrail trace: {:?}", guardrail_trace);
	}

	Ok(universal::Response {
		id,
		object: "chat.completion".to_string(),
		created: chrono::Utc::now().timestamp() as u32,
		model: model.to_string(),
		choices,
		usage: Some(usage),
		service_tier: None,
		system_fingerprint: None,
	})
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

/// Convert OpenAI request directly to Bedrock ConverseRequest format
pub(super) fn translate_openai_request(req: &universal::Request, provider: &Provider, model: &str) -> ConverseRequest {
	// Extract tool_results_meta from vendor bag for proper ToolResult.status handling
	let tool_results_meta = req.vendor
		.as_ref()
		.and_then(|vendor| vendor.get("anthropic"))
		.and_then(|anthropic| anthropic.get("tool_results_meta"))
		.and_then(|meta| meta.as_object())
		.map(|obj| {
			obj.iter()
				.filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
				.collect::<std::collections::HashMap<String, bool>>()
		});


	// Extract anthropic beta headers from vendor bag for additionalModelRequestFields
	let additional_model_request_fields = req.vendor
		.as_ref()
		.and_then(|vendor| vendor.get("anthropic"))
		.and_then(|anthropic| anthropic.get("headers"))
		.and_then(|headers| headers.get("beta"))
		.and_then(|beta| beta.as_array())
		.filter(|beta_array| !beta_array.is_empty())
		.map(|beta_array| {
			serde_json::json!({
				"anthropic_beta": beta_array
			})
		});

	// Extract system messages from Universal format
	let mut system_text = Vec::new();
	let mut messages: Vec<types::Message> = Vec::new();

	// Coalesce messages following Anthropic pattern: Assistant(tool_use) → User(tool_result + text)
	let mut i = 0;
	while i < req.messages.len() {
		match &req.messages[i] {
			universal::RequestMessage::System(sys_msg) => {
				let text = match &sys_msg.content {
					universal::RequestSystemMessageContent::Text(t) => t.clone(),
					universal::RequestSystemMessageContent::Array(blocks) => {
						blocks.iter()
							.filter_map(|block| match block {
								async_openai::types::ChatCompletionRequestSystemMessageContentPart::Text(text_part) => {
									Some(text_part.text.as_str())
								}
							})
							.collect::<Vec<_>>()
							.join("\n")
					}
				};
				system_text.push(text);
				i += 1;
			},
			universal::RequestMessage::Assistant(assistant_msg) => {
				// Convert assistant message
				let content = convert_openai_message_to_bedrock_content(&req.messages[i], tool_results_meta.as_ref());
				// Always include Assistant messages, especially those with tool_calls
				// Empty content check could miss tool_calls without text content
				let has_tool_calls = assistant_msg.tool_calls.as_ref().map_or(false, |calls| !calls.is_empty());
				if !content.is_empty() || has_tool_calls {
					messages.push(types::Message {
						role: types::Role::Assistant,
						content,
					});
				}

				// Check if this assistant has tool calls - if so, coalesce the following pattern:
				// Tool messages → optional User message into a single User message
				if has_tool_calls {
					i += 1; // Move past assistant

					// Collect tool_call_ids from the assistant to match against
					let tool_call_ids: std::collections::HashSet<String> = assistant_msg.tool_calls
						.as_ref()
						.unwrap()
						.iter()
						.map(|call| call.id.clone())
						.collect();

					// Step 1: Collect contiguous Tool messages that match the assistant's tool_calls
					let mut tool_results = Vec::new();
					while i < req.messages.len() {
						if let universal::RequestMessage::Tool(tool_msg) = &req.messages[i] {
							// Only collect tool results that match this assistant's tool calls
							if tool_call_ids.contains(&tool_msg.tool_call_id) {
								let tool_content = convert_openai_message_to_bedrock_content(&req.messages[i], tool_results_meta.as_ref());
								tool_results.extend(tool_content);
								i += 1;
							} else {
								break; // Tool doesn't match this assistant, stop collecting
							}
						} else {
							break; // Not a tool message, stop collecting
						}
					}

					// Step 2: Check for an optional trailing User message
					let mut user_content = Vec::new();
					if i < req.messages.len() {
						if let universal::RequestMessage::User(_) = &req.messages[i] {
							user_content = convert_openai_message_to_bedrock_content(&req.messages[i], tool_results_meta.as_ref());
							i += 1; // Consume the user message
						}
					}

					// Step 3: Create a single User message with tool_results FIRST, then user text
					let mut combined_content = tool_results;
					combined_content.extend(user_content);

					if !combined_content.is_empty() {
						messages.push(types::Message {
							role: types::Role::User,
							content: combined_content,
						});
					}
					continue; // We've already advanced i appropriately
				}

				i += 1;
			},
			_ => {
				// Handle other message types (User, Function, Developer)
				let role = match &req.messages[i] {
					universal::RequestMessage::User(_) => types::Role::User,
					universal::RequestMessage::Function(_) => types::Role::User, // Map deprecated function to user
					universal::RequestMessage::Developer(_) => types::Role::User, // Map developer to user
					universal::RequestMessage::Tool(_) => {
						// Orphaned tool message - create a User message with just tool results
						let mut tool_content = Vec::new();
						while i < req.messages.len() {
							if let universal::RequestMessage::Tool(_) = &req.messages[i] {
								let content = convert_openai_message_to_bedrock_content(&req.messages[i], tool_results_meta.as_ref());
								tool_content.extend(content);
								i += 1;
							} else {
								break;
							}
						}
						if !tool_content.is_empty() {
							messages.push(types::Message {
								role: types::Role::User,
								content: tool_content,
							});
						}
						continue; // Already advanced i
					},
					universal::RequestMessage::System(_) => unreachable!(), // Already handled above
					universal::RequestMessage::Assistant(_) => unreachable!(), // Already handled above
				};

				let content = convert_openai_message_to_bedrock_content(&req.messages[i], tool_results_meta.as_ref());
				if !content.is_empty() {
					messages.push(types::Message { role, content });
				}
				i += 1;
			}
		}
	}

	// Build system content blocks
	let system = if system_text.is_empty() {
		None
	} else {
		Some(vec![types::SystemContentBlock::Text(system_text.join("\n"))])
	};

	// Build inference configuration from OpenAI request fields
	let inference_config = types::InferenceConfiguration {
		max_tokens: req.max_completion_tokens.map(|t| t as i32).or(req.max_tokens.map(|t| t as i32)),
		temperature: req.temperature,
		top_p: req.top_p,
		stop_sequences: req.stop.as_ref().map(|stop| match stop {
			async_openai::types::Stop::String(s) => vec![s.clone()],
			async_openai::types::Stop::StringArray(arr) => arr.clone(),
		}),
	};

	// Build guardrail configuration if specified
	let guardrail_config = if let (Some(identifier), Some(version)) =
		(&provider.guardrail_identifier, &provider.guardrail_version)
	{
		Some(types::GuardrailConfiguration {
			guardrail_identifier: identifier.to_string(),
			guardrail_version: version.to_string(),
			trace: Some(types::GuardrailTrace::Enabled),
		})
	} else {
		None
	};

	// Convert tools to Bedrock format
	let tool_config = req.tools.as_ref().map(|tools| {
		let bedrock_tools = tools
			.iter()
			.map(|tool| {
				let tool_spec = types::ToolSpecification {
					name: tool.function.name.clone(),
					description: tool.function.description.clone(),
					input_schema: Some(types::ToolInputSchema::Json(tool.function.parameters.clone().unwrap_or_default())),
				};
				types::Tool::ToolSpec(tool_spec)
			})
			.collect();

		types::ToolConfiguration {
			tools: bedrock_tools,
			tool_choice: req.tool_choice.as_ref().and_then(|choice| {
				match choice {
					universal::ToolChoiceOption::None => {
						// OpenAI "none" -> No tool choice (omit tool_choice field entirely)
						None
					},
					universal::ToolChoiceOption::Auto => {
						Some(types::ToolChoice::Auto(types::AutoToolChoice {
							auto: serde_json::Value::Object(serde_json::Map::new()), // {}
						}))
					},
					universal::ToolChoiceOption::Required => {
						Some(types::ToolChoice::Any(types::AnyToolChoice {
							any: serde_json::Value::Object(serde_json::Map::new()), // {}
						}))
					},
					universal::ToolChoiceOption::Named(named) => {
						Some(types::ToolChoice::Tool(types::ToolChoiceSpecific {
							tool: types::ToolChoiceToolSpec {
								name: named.function.name.clone(),
							},
						}))
					},
				}
			}),
		}
	});

	ConverseRequest {
		model_id: model.to_string(),
		messages: Some(messages),
		system,
		inference_config: Some(inference_config),
		tool_config,
		guardrail_config,
		additional_model_request_fields,
		additional_model_response_field_paths: None,
		request_metadata: None,
		performance_config: None,
	}
}

/// Convert OpenAI message content to Bedrock ContentBlocks
fn convert_openai_message_to_bedrock_content(
	msg: &universal::RequestMessage, 
	tool_results_meta: Option<&std::collections::HashMap<String, bool>>
) -> Vec<ContentBlock> {
	let mut content = Vec::new();

	match msg {
		universal::RequestMessage::User(user_msg) => {
			match &user_msg.content {
				universal::RequestUserMessageContent::Text(text) => {
					content.push(ContentBlock::Text(text.clone()));
				},
				universal::RequestUserMessageContent::Array(parts) => {
					for part in parts {
						match part {
							async_openai::types::ChatCompletionRequestUserMessageContentPart::Text(text_part) => {
								content.push(ContentBlock::Text(text_part.text.clone()));
							},
							async_openai::types::ChatCompletionRequestUserMessageContentPart::ImageUrl(_image_part) => {
								// TODO: Convert image URLs to Bedrock image format
								// For now, skip images
							},
							async_openai::types::ChatCompletionRequestUserMessageContentPart::InputAudio(_) => {
								// TODO: Handle audio content
								// For now, skip audio
							},
						}
					}
				}
			}
		},
		universal::RequestMessage::Assistant(assistant_msg) => {
			if let Some(content_text) = &assistant_msg.content {
				let text = match content_text {
					universal::RequestAssistantMessageContent::Text(text) => text.clone(),
					universal::RequestAssistantMessageContent::Array(parts) => {
						// Convert array of content parts to text - for now, just extract text parts
						parts.iter().filter_map(|part| {
							match part {
								async_openai::types::ChatCompletionRequestAssistantMessageContentPart::Text(text_part) => {
									Some(text_part.text.as_str())
								}
								async_openai::types::ChatCompletionRequestAssistantMessageContentPart::Refusal(refusal_part) => {
									Some(refusal_part.refusal.as_str())
								}
							}
						}).collect::<Vec<_>>().join("\n")
					}
				};
				content.push(ContentBlock::Text(text));
			}
			if let Some(tool_calls) = &assistant_msg.tool_calls {
				for tool_call in tool_calls {
					content.push(ContentBlock::ToolUse(types::ToolUseBlock {
						tool_use_id: tool_call.id.clone(),
						name: tool_call.function.name.clone(),
						input: serde_json::from_str(&tool_call.function.arguments)
							.unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
					}));
				}
			}
		},
		universal::RequestMessage::Tool(tool_msg) => {
			let text = match &tool_msg.content {
				universal::RequestToolMessageContent::Text(text) => text.clone(),
				universal::RequestToolMessageContent::Array(parts) => {
					parts.iter().filter_map(|part| {
						match part {
							async_openai::types::ChatCompletionRequestToolMessageContentPart::Text(text_part) => {
								Some(text_part.text.as_str())
							}
						}
					}).collect::<Vec<_>>().join("\n")
				}
			};
			// Look up is_error from vendor bag to set appropriate Bedrock ToolResult.status
			let status = tool_results_meta
				.and_then(|meta| meta.get(&tool_msg.tool_call_id))
				.map(|is_error| {
					if *is_error {
						types::ToolResultStatus::Error
					} else {
						types::ToolResultStatus::Success
					}
				});

			content.push(ContentBlock::ToolResult(types::ToolResultBlock {
				tool_use_id: tool_msg.tool_call_id.clone(),
				content: vec![ContentBlock::Text(text)],
				status,
			}));
		},
		universal::RequestMessage::Function(func_msg) => {
			// Legacy function message - treat as tool result
			content.push(ContentBlock::ToolResult(types::ToolResultBlock {
				tool_use_id: func_msg.name.clone(), // Use function name as tool_use_id
				content: vec![ContentBlock::Text(func_msg.content.clone().unwrap_or_default())],
				status: None,
			}));
		},
		universal::RequestMessage::Developer(dev_msg) => {
			match &dev_msg.content {
				universal::RequestDeveloperMessageContent::Text(text) => {
					content.push(ContentBlock::Text(text.clone()));
				},
				universal::RequestDeveloperMessageContent::Array(parts) => {
					for part in parts {
						content.push(ContentBlock::Text(part.text.clone()));
					}
				}
			}
		},
		universal::RequestMessage::System(_) => {
			// System messages handled separately
		}
	}

	content
}



pub(super) mod types {
	//! Complete Bedrock Converse API types from vendors/messages
	
	use serde::{Deserialize, Serialize};
	use std::collections::HashMap;

/// Bedrock Converse request structure
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseRequest {
	/// Required: Model ID to invoke (URI parameter in actual API)
	#[serde(skip)]
	pub model_id: String,

	/// Optional: Array of messages (max conversation length varies by model)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub messages: Option<Vec<Message>>,

	/// Optional: System prompt content blocks
	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<Vec<SystemContentBlock>>,

	/// Optional: Inference configuration parameters
	#[serde(skip_serializing_if = "Option::is_none")]
	pub inference_config: Option<InferenceConfiguration>,

	/// Optional: Tool configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_config: Option<ToolConfiguration>,

	/// Optional: Guardrail configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_config: Option<GuardrailConfiguration>,

	/// Optional: Model-specific additional request fields
	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_request_fields: Option<serde_json::Value>,

	/// Optional: Response field paths to include
	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_response_field_paths: Option<Vec<String>>,

	/// Optional: Performance configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub performance_config: Option<PerformanceConfiguration>,

	/// Optional: Request metadata (string key-value pairs)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub request_metadata: Option<HashMap<String, String>>,
}

/// Message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
	pub role: ConversationRole,
	pub content: Vec<ContentBlock>,
}

/// Conversation roles
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConversationRole {
	User,
	Assistant,
}

// Alias for backward compatibility with existing bedrock.rs code
pub type Role = ConversationRole;

/// CRITICAL: ContentBlock is a strict UNION type - exactly one variant must be set
/// Multiple variants will cause validation errors
/// FORMAT: Tuple variants to match AWS SDK serialization exactly
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentBlock {
	/// Text content block
	#[serde(rename = "text")]
	Text(String),

	/// Image content block (Claude 3 only)  
	#[serde(rename = "image")]
	Image(ImageBlock),

	/// Document content block
	#[serde(rename = "document")]
	Document(DocumentBlock),

	/// Tool use content block
	#[serde(rename = "toolUse")]
	ToolUse(ToolUseBlock),

	/// Tool result content block
	#[serde(rename = "toolResult")]
	ToolResult(ToolResultBlock),

	/// Cache point for performance optimization
	#[serde(rename = "cachePoint")]
	CachePoint(CachePointBlock),

	/// Reasoning content (for thinking mode)
	#[serde(rename = "reasoningContent")]
	ReasoningContent(ReasoningContentBlock),
}

/// Image content block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageBlock {
	/// Image format
	pub format: ImageFormat,

	/// Image source
	pub source: ImageSource,
}

/// Supported image formats
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
	Png,
	Jpeg,
	Gif,
	Webp,
}

/// Image source types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageSource {
	/// Base64-encoded image data
	Bytes {
		#[serde(rename = "bytes")]
		data: String, // Base64 encoded
	},

	/// S3 location
	S3Location {
		#[serde(rename = "s3Location")]
		s3_location: S3Location,
	},
}

/// S3 location structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Location {
	pub uri: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub bucket_owner: Option<String>,
}

/// Document content block  
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentBlock {
	/// Document name (required, 1-200 chars)
	pub name: String,

	/// Document source (required)
	pub source: DocumentSource,

	/// Document format (optional)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub format: Option<String>,

	/// Citations configuration (optional)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<CitationsConfig>,

	/// Context information (optional)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub context: Option<String>,
}

/// Citations configuration for documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationsConfig {
	/// Whether citations are enabled
	pub enabled: bool,
}

/// Supported document formats
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
	Pdf,
	Csv,
	Doc,
	Docx,
	Xls,
	Xlsx,
	Html,
	Txt,
	Md,
}

/// Document source types  
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DocumentSource {
	/// Base64-encoded document data
	Bytes {
		#[serde(rename = "bytes")]
		data: String,
	},

	/// S3 location
	S3Location {
		#[serde(rename = "s3Location")]
		s3_location: S3Location,
	},
}

/// Tool use content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseBlock {
	/// Tool use identifier
	pub tool_use_id: String,

	/// Tool name
	pub name: String,

	/// Tool input (JSON object)
	pub input: serde_json::Value,
}

/// Tool result content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultBlock {
	/// Tool use ID this result corresponds to
	pub tool_use_id: String,

	/// Result content blocks - Must use ContentBlock to match Bedrock API
	pub content: Vec<ContentBlock>,

	/// Result status (Claude 3 only)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub status: Option<ToolResultStatus>,
}

/// Tool result status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolResultStatus {
	Success,
	Error,
}

/// Cache point for performance optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePointBlock {
	#[serde(rename = "type")]
	pub cache_type: CachePointType,
}

/// Cache point types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CachePointType {
	Default,
}

/// Reasoning content (chain of thought) - UNION type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningContentBlock {
	/// Reasoning text with optional signature
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reasoning_text: Option<ReasoningTextBlock>,

	/// Redacted content (base64-encoded)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub redacted_content: Option<String>,
}

/// Reasoning text block with signature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningTextBlock {
	pub text: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub signature: Option<String>,
}

/// System content blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemContentBlock {
	/// Text system content
	#[serde(rename = "text")]
	Text(String),

	/// Cache point in system
	#[serde(rename = "cachePoint")]
	CachePoint(CachePointBlock),
}

/// Inference configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceConfiguration {
	/// Maximum tokens to generate (minimum 1)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_tokens: Option<i32>,

	/// Stop sequences
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_sequences: Option<Vec<String>>,

	/// Temperature (0.0 to 1.0)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,

	/// Top-p nucleus sampling (0.0 to 1.0)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,
}

/// Tool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfiguration {
	/// Array of tools
	pub tools: Vec<Tool>,

	/// Tool choice configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_choice: Option<ToolChoice>,
}

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
	/// Cache point
	#[serde(rename = "cachePoint")]
	CachePoint(CachePointBlock),

	/// Tool specification
	#[serde(rename = "toolSpec")]
	ToolSpec(ToolSpecification),
}

/// Tool specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpecification {
	/// Tool name
	pub name: String,

	/// Tool description
	#[serde(skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,

	/// Input schema
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_schema: Option<ToolInputSchema>,
}

/// Tool input schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolInputSchema {
	/// JSON schema
	#[serde(rename = "json")]
	Json(serde_json::Value),
}

/// Tool choice options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
	/// Model decides (default)
	Auto(AutoToolChoice),

	/// Must use any tool  
	Any(AnyToolChoice),

	/// Must use specific tool (Claude 3 only)
	Tool(ToolChoiceSpecific),
}

/// Auto tool choice (Bedrock format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoToolChoice {
	pub auto: serde_json::Value, // Empty object {}
}

/// Any tool choice (Bedrock format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnyToolChoice {
	pub any: serde_json::Value, // Empty object {}
}

/// Specific tool choice (Bedrock format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceSpecific {
	pub tool: ToolChoiceToolSpec,
}

/// Tool specification for specific tool choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceToolSpec {
	pub name: String,
}

/// Guardrail trace setting
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardrailTrace {
	Enabled,
	Disabled,
}

/// Guardrail configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailConfiguration {
	/// Guardrail identifier
	pub guardrail_identifier: String,

	/// Guardrail version
	pub guardrail_version: String,

	/// Enable trace output
	#[serde(skip_serializing_if = "Option::is_none")]
	pub trace: Option<GuardrailTrace>,
}

/// Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceConfiguration {
	/// Latency optimization mode
	#[serde(skip_serializing_if = "Option::is_none")]
	pub latency: Option<LatencyMode>,
}

/// Latency optimization modes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LatencyMode {
	Standard,
	Optimized,
}

/// Converse response structure
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseResponse {
	/// Response output
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output: Option<ConverseOutput>,

	/// Stop reason
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_reason: Option<StopReason>,

	/// Token usage
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<TokenUsage>,

	/// Request metrics
	#[serde(skip_serializing_if = "Option::is_none")]
	pub metrics: Option<ConverseMetrics>,

	/// Error message (if this is actually an error response in success format)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub message: Option<String>,

	/// Additional model response fields
	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_response_fields: Option<serde_json::Value>,

	/// Performance configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub performance_config: Option<PerformanceConfiguration>,

	/// Trace information
	#[serde(skip_serializing_if = "Option::is_none")]
	pub trace: Option<ConverseTrace>,
}

/// Response output types
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ConverseOutput {
	/// Message output with nested message field
	Message { message: Message },
}

/// Stop reasons
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
	EndTurn,
	ToolUse,
	MaxTokens,
	StopSequence,
	GuardrailIntervened,
	ContentFiltered,
}

/// Token usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
	pub input_tokens: u32,
	pub output_tokens: u32,
	pub total_tokens: u32,

	/// Cache-specific token counts
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_read_input_tokens: Option<u32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_write_input_tokens: Option<u32>,
}

/// Request metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseMetrics {
	/// Latency in milliseconds
	pub latency_ms: u64,
}

/// Trace information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConverseTrace {
	/// Guardrail trace data
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail: Option<serde_json::Value>,
}

/// ConverseStream output event types
#[derive(Debug, Clone)]
pub enum ConverseStreamOutput {
	MessageStart(MessageStartEvent),
	ContentBlockStart(ContentBlockStartEvent),
	ContentBlockDelta(ContentBlockDeltaEvent),
	ContentBlockStop(ContentBlockStopEvent),
	MessageStop(MessageStopEvent),
	Metadata(ConverseStreamMetadataEvent),

	// Error events
	InternalServerException(StreamErrorEvent),
	ModelStreamErrorException(StreamErrorEvent),
	ServiceUnavailableException(StreamErrorEvent),
	ThrottlingException(StreamErrorEvent),
	ValidationException(StreamErrorEvent),
}

impl ConverseStreamOutput {
	/// Deserialize from AWS event-stream message
	pub fn deserialize(message: aws_event_stream_parser::Message) -> Result<Self, crate::llm::AIError> {
		// Extract event type from headers
		let event_type = message
			.headers
			.headers
			.iter()
			.find(|h| h.key.as_str() == ":event-type")
			.and_then(|v| match &v.value {
				aws_event_stream_parser::HeaderValue::String(s) => Some(s.as_str()),
				_ => None,
			})
			.ok_or_else(|| crate::llm::AIError::MissingField(":event-type header".into()))?;

		// Parse body based on event type
		match event_type {
			"messageStart" => Ok(ConverseStreamOutput::MessageStart(serde_json::from_slice(
				&message.body,
			).map_err(crate::llm::AIError::ResponseParsing)?)),
			"contentBlockStart" => Ok(ConverseStreamOutput::ContentBlockStart(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),
			"contentBlockDelta" => Ok(ConverseStreamOutput::ContentBlockDelta(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),
			"contentBlockStop" => Ok(ConverseStreamOutput::ContentBlockStop(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),
			"messageStop" => Ok(ConverseStreamOutput::MessageStop(serde_json::from_slice(
				&message.body,
			).map_err(crate::llm::AIError::ResponseParsing)?)),
			"metadata" => Ok(ConverseStreamOutput::Metadata(serde_json::from_slice(
				&message.body,
			).map_err(crate::llm::AIError::ResponseParsing)?)),

			// Error events
			"internalServerException" => Ok(ConverseStreamOutput::InternalServerException(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),
			"modelStreamErrorException" => Ok(ConverseStreamOutput::ModelStreamErrorException(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),
			"serviceUnavailableException" => Ok(ConverseStreamOutput::ServiceUnavailableException(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),
			"throttlingException" => Ok(ConverseStreamOutput::ThrottlingException(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),
			"validationException" => Ok(ConverseStreamOutput::ValidationException(
				serde_json::from_slice(&message.body).map_err(crate::llm::AIError::ResponseParsing)?,
			)),

			_unknown => Err(crate::llm::AIError::UnsupportedContent),
		}
	}
}

/// Message start event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStartEvent {
	pub role: ConversationRole,
}

/// Content block start event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockStartEvent {
	pub content_block_index: usize,
	pub start: ContentBlockStart,
}

/// Content block start types
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ContentBlockStart {
	ToolUse(ToolUseBlockStart),
	Text(String),      // Usually empty for text blocks
	Reasoning(String), // For reasoning blocks
}

/// Tool use block start
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseBlockStart {
	pub tool_use_id: String,
	pub name: String,
}

/// Content block delta event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockDeltaEvent {
	pub content_block_index: usize,
	pub delta: ContentBlockDelta,
}

/// Content block delta types
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ContentBlockDelta {
	/// Text content delta - matches {"text": "content"}
	Text { text: String },

	/// Tool use input delta - matches {"toolUse": {"input": "..."}}
	ToolUse {
		#[serde(rename = "toolUse")]
		tool_use: ToolUseBlockDelta,
	},

	/// Reasoning content delta
	ReasoningContent(ReasoningContentBlockDelta),

	/// Citations delta
	Citation(CitationsDelta),
}

/// Tool use block delta
#[derive(Debug, Clone, Deserialize)]
pub struct ToolUseBlockDelta {
	/// Incremental JSON input string
	pub input: String,
}

/// Reasoning content delta - UNION type (exactly one variant)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ReasoningContentBlockDelta {
	/// Text reasoning delta
	Text(String),
}

/// Citations delta with location tracking
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationsDelta {
	/// Citation location information
	#[serde(skip_serializing_if = "Option::is_none")]
	pub location: Option<CitationLocation>,

	/// Source content fragments
	#[serde(skip_serializing_if = "Option::is_none")]
	pub source_content: Option<Vec<CitationSourceContentDelta>>,

	/// Citation title
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,

	/// Legacy citations array (backward compatibility)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<Vec<Citation>>,
}

/// Citation location for incremental building
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationLocation {
	/// Start position
	pub start: usize,

	/// End position
	pub end: usize,

	/// Location type (page, character, block, etc.)
	pub location_type: String,
}

/// Citation source content delta
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationSourceContentDelta {
	/// Content fragment
	pub content: String,

	/// Fragment position
	pub position: usize,
}

/// Citation structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
	pub source: String,
	pub content: String,
}

/// Content block stop event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockStopEvent {
	pub content_block_index: usize,
}

/// Message stop event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStopEvent {
	pub stop_reason: StopReason,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_response_fields: Option<serde_json::Value>,
}

/// Stream metadata event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseStreamMetadataEvent {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<TokenUsage>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub metrics: Option<ConverseMetrics>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub performance_config: Option<PerformanceConfiguration>,
}

/// Bedrock error response
#[derive(Debug, Clone, Deserialize)]
pub struct ConverseErrorResponse {
	/// Error message
	pub message: String,

}

/// Stream error event structure
#[derive(Debug, Clone, Deserialize)]
pub struct StreamErrorEvent {
	pub message: String,
}

impl ConverseRequest {
	/// Create new request with model ID
	pub fn new(model_id: String) -> Self {
		Self {
			model_id,
			messages: None,
			system: None,
			inference_config: None,
			tool_config: None,
			guardrail_config: None,
			additional_model_request_fields: None,
			additional_model_response_field_paths: None,
			performance_config: None,
			request_metadata: None,
		}
	}

	/// Add messages
	pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
		self.messages = Some(messages);
		self
	}

	/// Add system prompt
	pub fn with_system(mut self, system: Vec<SystemContentBlock>) -> Self {
		self.system = Some(system);
		self
	}

	/// Add inference config
	pub fn with_inference_config(mut self, config: InferenceConfiguration) -> Self {
		self.inference_config = Some(config);
		self
	}

	/// Add tool configuration
	pub fn with_tools(mut self, tools: Vec<Tool>, tool_choice: Option<ToolChoice>) -> Self {
		self.tool_config = Some(ToolConfiguration { tools, tool_choice });
		self
	}
}

impl Message {
	/// Create user message with text
	pub fn user_text(text: String) -> Self {
		Self {
			role: ConversationRole::User,
			content: vec![ContentBlock::Text(text)],
		}
	}

	/// Create assistant message with text
	pub fn assistant_text(text: String) -> Self {
		Self {
			role: ConversationRole::Assistant,
			content: vec![ContentBlock::Text(text)],
		}
	}

	/// Create user message with tool result
	pub fn user_tool_result(
		tool_use_id: String,
		content: Vec<ContentBlock>,
		status: Option<ToolResultStatus>,
	) -> Self {
		Self {
			role: ConversationRole::User,
			content: vec![ContentBlock::ToolResult(ToolResultBlock {
				tool_use_id,
				content,
				status,
			})],
		}
	}
}

impl InferenceConfiguration {
	/// Create basic inference config
	pub fn new(max_tokens: i32) -> Self {
		Self {
			max_tokens: Some(max_tokens),
			stop_sequences: None,
			temperature: None,
			top_p: None,
		}
	}
}

} // end types module

/// Bedrock → Anthropic Messages API streaming conversion
pub mod streaming {
	use std::collections::{HashMap, VecDeque};
	use std::pin::Pin;
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::task::{Context, Poll, ready};
	use std::time::Instant;

	use bytes::Bytes;
	use pin_project_lite::pin_project;
	use tokio_util::codec::Decoder;
	use tracing::debug;

	use aws_event_stream_parser::EventStreamCodec;

	use crate::http::Body;
	use crate::llm::bedrock::types::{ConverseStreamOutput, ContentBlockDelta, StopReason};
	use crate::llm::{AIError, LLMResponse};
	use crate::telemetry::log::AsyncLog;
	use crate::store::LLMResponsePolicies;

	/// Maximum size for accumulated tool JSON input (2MB - matches standard payload limits in codebase)
	const MAX_TOOL_JSON_SIZE: usize = 2_097_152;

	/// Global memory limit for all streaming buffers to prevent DoS attacks
	const GLOBAL_BUFFER_LIMIT: usize = 50_000_000; // 50MB total across all streams

	/// Global counter for memory usage across all streaming operations
	static GLOBAL_BUFFER_USAGE: AtomicUsize = AtomicUsize::new(0);

	/// Anthropic Messages API streaming event types
	#[derive(Debug, Clone, serde::Serialize)]
	#[serde(tag = "type")]
	pub enum StreamEvent {
		#[serde(rename = "message_start")]
		MessageStart { message: MessagesResponse },
		#[serde(rename = "content_block_start")]
		ContentBlockStart {
			index: usize,
			content_block: ResponseContentBlock,
		},
		#[serde(rename = "ping")]
		Ping,
		#[serde(rename = "content_block_delta")]
		ContentBlockDelta { index: usize, delta: ContentDelta },
		#[serde(rename = "content_block_stop")]
		ContentBlockStop { index: usize },
		#[serde(rename = "message_delta")]
		MessageDelta { delta: MessageDelta },
		#[serde(rename = "message_stop")]
		MessageStop,
		#[serde(rename = "error")]
		Error { error: ErrorResponse },
	}

	/// Content delta types for streaming updates
	#[derive(Debug, Clone, serde::Serialize)]
	#[serde(tag = "type")]
	pub enum ContentDelta {
		#[serde(rename = "text_delta")]
		TextDelta { text: String },
		#[serde(rename = "input_json_delta")]
		InputJsonDelta { partial_json: String },
		#[serde(rename = "thinking_delta")]
		ThinkingDelta { thinking: String },
	}

	/// Message delta for usage and stop information
	#[derive(Debug, Clone, serde::Serialize)]
	pub struct MessageDelta {
		pub stop_reason: Option<String>,
		pub stop_sequence: Option<String>,
		pub usage: Option<Usage>,
	}

	/// Response content blocks
	#[derive(Debug, Clone, serde::Serialize)]
	#[serde(tag = "type")]
	pub enum ResponseContentBlock {
		#[serde(rename = "text")]
		Text(ResponseTextBlock),
		#[serde(rename = "tool_use")]
		ToolUse(ResponseToolUseBlock),
		#[serde(rename = "thinking")]
		Thinking(ResponseThinkingBlock),
		#[serde(rename = "redacted_thinking")]
		RedactedThinking(ResponseRedactedThinkingBlock),
	}

	#[derive(Debug, Clone, serde::Serialize)]
	pub struct ResponseTextBlock {
		pub text: String,
		pub citations: Option<serde_json::Value>,
	}

	#[derive(Debug, Clone, serde::Serialize)]
	pub struct ResponseToolUseBlock {
		pub id: String,
		pub name: String,
		pub input: serde_json::Value,
	}

	#[derive(Debug, Clone, serde::Serialize)]
	pub struct ResponseThinkingBlock {
		pub thinking: String,
		pub signature: String,
	}

	#[derive(Debug, Clone, serde::Serialize)]
	pub struct ResponseRedactedThinkingBlock {
		pub data: String,
	}

	/// Messages API response structure for message_start events
	#[derive(Debug, Clone, serde::Serialize)]
	pub struct MessagesResponse {
		pub id: String,
		#[serde(rename = "type")]
		pub r#type: String,
		pub role: String,
		pub content: Vec<ResponseContentBlock>,
		pub model: String,
		pub stop_reason: Option<String>,
		pub stop_sequence: Option<String>,
		pub usage: Usage,
		pub container: Option<serde_json::Value>,
	}

	/// Usage information
	#[derive(Debug, Clone, serde::Serialize)]
	pub struct Usage {
		pub input_tokens: u32,
		pub output_tokens: u32,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_creation_input_tokens: Option<u32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_read_input_tokens: Option<u32>,
	}

	/// Error response structure
	#[derive(Debug, Clone, serde::Serialize)]
	pub struct ErrorResponse {
		pub error_type: String,
		pub message: String,
	}

	/// Stream event processor that converts Bedrock events to Anthropic SSE format
	pub struct BedrockStreamProcessor {
		/// Current message ID (generated since Bedrock doesn't provide one)
		message_id: String,

		/// Current model name
		model: String,

		/// Buffer for accumulating tool input JSON strings by content block index
		tool_json_buffers: HashMap<usize, String>,

		/// Track content block metadata for correlation
		content_block_metadata: HashMap<usize, ContentBlockMetadata>,

		/// Whether we've seen the first token (for timing metrics)
		seen_first_token: bool,

		/// Accumulated usage information
		pub current_usage: Option<Usage>,
	}

	/// Metadata for tracking content blocks during streaming
	#[derive(Debug, Clone)]
	struct ContentBlockMetadata {
		pub block_type: ContentBlockType,
	}

	/// Types of content blocks we're tracking
	#[derive(Debug, Clone, PartialEq)]
	enum ContentBlockType {
		Text,
		ToolUse,
		Reasoning,
	}

	impl BedrockStreamProcessor {
		/// Create a new stream processor
		pub fn new(message_id: String, model: String) -> Self {
			Self {
				message_id,
				model,
				tool_json_buffers: HashMap::new(),
				content_block_metadata: HashMap::new(),
				seen_first_token: false,
				current_usage: None,
			}
		}

		/// Process a Bedrock stream event and convert to Anthropic events
		/// Returns Vec because some Bedrock events may produce multiple Anthropic events
		pub fn process_event(
			&mut self,
			bedrock_event: ConverseStreamOutput,
			log: &AsyncLog<LLMResponse>,
		) -> Result<Vec<StreamEvent>, AIError> {
			let mut events = Vec::new();

			match bedrock_event {
				ConverseStreamOutput::MessageStart(start_event) => {
					let event = self.handle_message_start(start_event)?;
					events.push(event);
				},

				ConverseStreamOutput::ContentBlockStart(start_event) => {
					let event = self.handle_content_block_start(start_event)?;
					events.push(event);
				},

				ConverseStreamOutput::ContentBlockDelta(delta_event) => {
					if let Some(event) = self.handle_content_block_delta(delta_event, log)? {
						events.push(event);
					}
				},

				ConverseStreamOutput::ContentBlockStop(stop_event) => {
					if let Some(event) = self.handle_content_block_stop(stop_event)? {
						events.push(event);
					}
				},

				ConverseStreamOutput::MessageStop(stop_event) => {
					let event = self.handle_message_stop(stop_event)?;
					events.push(event);
				},

				ConverseStreamOutput::Metadata(metadata_event) => {
					if let Some(event) = self.handle_metadata(metadata_event)? {
						events.push(event);
					}
				},

				// Error handling - convert to error events or log and continue
				ConverseStreamOutput::InternalServerException(error_event) => {
					events.push(StreamEvent::Error {
						error: ErrorResponse {
							error_type: "internal_server_exception".to_string(),
							message: format!("Internal server error: {:?}", error_event),
						},
					});
				},
				ConverseStreamOutput::ModelStreamErrorException(error_event) => {
					events.push(StreamEvent::Error {
						error: ErrorResponse {
							error_type: "model_stream_error_exception".to_string(),
							message: format!("Model stream error: {:?}", error_event),
						},
					});
				},
				ConverseStreamOutput::ServiceUnavailableException(error_event) => {
					events.push(StreamEvent::Error {
						error: ErrorResponse {
							error_type: "service_unavailable_exception".to_string(),
							message: format!("Service unavailable: {:?}", error_event),
						},
					});
				},
				ConverseStreamOutput::ThrottlingException(error_event) => {
					events.push(StreamEvent::Error {
						error: ErrorResponse {
							error_type: "throttling_exception".to_string(),
							message: format!("Throttling error: {:?}", error_event),
						},
					});
				},
				ConverseStreamOutput::ValidationException(error_event) => {
					events.push(StreamEvent::Error {
						error: ErrorResponse {
							error_type: "validation_exception".to_string(),
							message: format!("Validation error: {:?}", error_event),
						},
					});
				},
			}

			Ok(events)
		}

		/// Handle Bedrock MessageStart → Anthropic message_start
		fn handle_message_start(
			&mut self,
			_start_event: crate::llm::bedrock::types::MessageStartEvent,
		) -> Result<StreamEvent, AIError> {
			// Create initial message with empty content for message_start
			let message = MessagesResponse {
				id: self.message_id.clone(),
				r#type: "message".to_string(),
				role: "assistant".to_string(),
				content: Vec::new(),
				model: self.model.clone(),
				stop_reason: None,
				stop_sequence: None,
				usage: Usage {
					input_tokens: 0,  // TODO: Should be actual input token count
					output_tokens: 0,
					cache_creation_input_tokens: None,
					cache_read_input_tokens: None,
				},
				container: None,
			};

			Ok(StreamEvent::MessageStart { message })
		}

		/// Handle Bedrock ContentBlockStart → Anthropic content_block_start
		fn handle_content_block_start(
			&mut self,
			start_event: crate::llm::bedrock::types::ContentBlockStartEvent,
		) -> Result<StreamEvent, AIError> {
			let index = start_event.content_block_index as usize;

			let (content_block, metadata) = match start_event.start {
				crate::llm::bedrock::types::ContentBlockStart::ToolUse(tool_start) => {
					let metadata = ContentBlockMetadata {
						block_type: ContentBlockType::ToolUse,
					};

					let content_block = ResponseContentBlock::ToolUse(ResponseToolUseBlock {
						id: tool_start.tool_use_id,
						name: tool_start.name,
						input: serde_json::Value::Object(serde_json::Map::new()), // Empty initially
					});

					(content_block, metadata)
				},

				crate::llm::bedrock::types::ContentBlockStart::Text(_) => {
					let metadata = ContentBlockMetadata {
						block_type: ContentBlockType::Text,
					};

					let content_block = ResponseContentBlock::Text(ResponseTextBlock {
						text: String::new(),
						citations: None,
					});

					(content_block, metadata)
				},

				crate::llm::bedrock::types::ContentBlockStart::Reasoning(_) => {
					let metadata = ContentBlockMetadata {
						block_type: ContentBlockType::Reasoning,
					};

					let content_block = ResponseContentBlock::Thinking(ResponseThinkingBlock {
						thinking: String::new(),
						signature: String::new(),
					});

					(content_block, metadata)
				},
			};

			// Store metadata for delta processing
			self.content_block_metadata.insert(index, metadata);

			Ok(StreamEvent::ContentBlockStart {
				index,
				content_block,
			})
		}

		/// Handle Bedrock ContentBlockDelta → Anthropic content_block_delta
		fn handle_content_block_delta(
			&mut self,
			delta_event: crate::llm::bedrock::types::ContentBlockDeltaEvent,
			log: &AsyncLog<LLMResponse>,
		) -> Result<Option<StreamEvent>, AIError> {
			let index = delta_event.content_block_index as usize;

			// Mark first token seen for timing
			if !self.seen_first_token {
				self.seen_first_token = true;
				log.non_atomic_mutate(|r| {
					r.first_token = Some(Instant::now());
				});
			}

			let delta = match delta_event.delta {
				ContentBlockDelta::Text { text } => ContentDelta::TextDelta { text },

				ContentBlockDelta::ToolUse { tool_use } => {
					// Accumulate partial JSON for tool inputs with bounds checking
					let json_buffer = self.tool_json_buffers.entry(index).or_default();

					// Check both per-buffer and global memory limits to prevent DoS attacks
					let new_size = json_buffer.len() + tool_use.input.len();
					let current_global_usage = GLOBAL_BUFFER_USAGE.load(Ordering::Relaxed);
					
					if new_size > MAX_TOOL_JSON_SIZE || 
					   current_global_usage + tool_use.input.len() > GLOBAL_BUFFER_LIMIT {
						// Clean up buffer and return error
						if let Some(removed_buffer) = self.tool_json_buffers.remove(&index) {
							// Decrement global counter for removed buffer
							GLOBAL_BUFFER_USAGE.fetch_sub(removed_buffer.len(), Ordering::Relaxed);
						}
						return Err(crate::llm::AIError::RequestTooLarge);
					}
					
					// Update global counter for new data
					GLOBAL_BUFFER_USAGE.fetch_add(tool_use.input.len(), Ordering::Relaxed);

					json_buffer.push_str(&tool_use.input);

					// Return the partial JSON as input_json_delta
					ContentDelta::InputJsonDelta {
						partial_json: tool_use.input,
					}
				},

				ContentBlockDelta::ReasoningContent(reasoning_delta) => {
					// Map reasoning content to thinking deltas
					match reasoning_delta {
						crate::llm::bedrock::types::ReasoningContentBlockDelta::Text(text) => {
							ContentDelta::ThinkingDelta { thinking: text }
						},
					}
				},

				ContentBlockDelta::Citation(_citation_delta) => {
					// Citations are typically accumulated and attached to text blocks
					// For now, we'll skip them in the streaming interface
					return Ok(None);
				},
			};

			Ok(Some(StreamEvent::ContentBlockDelta { index, delta }))
		}

		/// Handle Bedrock ContentBlockStop → Anthropic content_block_stop
		fn handle_content_block_stop(
			&mut self,
			stop_event: crate::llm::bedrock::types::ContentBlockStopEvent,
		) -> Result<Option<StreamEvent>, AIError> {
			let index = stop_event.content_block_index as usize;

			// Clean up tool JSON buffer if present
			if let Some(metadata) = self.content_block_metadata.get(&index)
				&& metadata.block_type == ContentBlockType::ToolUse
				&& let Some(json_buffer) = self.tool_json_buffers.remove(&index)
			{
				// Update global counter when removing buffer
				GLOBAL_BUFFER_USAGE.fetch_sub(json_buffer.len(), Ordering::Relaxed);
				
				debug!("assembled tool input JSON (len={})", json_buffer.len());
			}

			// Clean up metadata
			self.content_block_metadata.remove(&index);

			Ok(Some(StreamEvent::ContentBlockStop { index }))
		}

		/// Handle Bedrock MessageStop → Anthropic message_stop
		fn handle_message_stop(
			&mut self,
			stop_event: crate::llm::bedrock::types::MessageStopEvent,
		) -> Result<StreamEvent, AIError> {
			// Convert stop reason
			let stop_reason = translate_stop_reason_to_anthropic(stop_event.stop_reason);

			let delta = MessageDelta {
				stop_reason: Some(stop_reason),
				stop_sequence: None, // Bedrock doesn't provide matched sequence details
				usage: self.current_usage.clone(),
			};

			Ok(StreamEvent::MessageDelta { delta })
		}

		/// Handle Bedrock Metadata events
		fn handle_metadata(
			&mut self,
			metadata_event: crate::llm::bedrock::types::ConverseStreamMetadataEvent,
		) -> Result<Option<StreamEvent>, AIError> {
			if let Some(bedrock_usage) = metadata_event.usage {
				let usage = Usage {
					input_tokens: bedrock_usage.input_tokens as u32,
					output_tokens: bedrock_usage.output_tokens as u32,
					cache_creation_input_tokens: None,
					cache_read_input_tokens: None,
				};
				self.current_usage = Some(usage.clone());

				// Anthropic requires message_delta before message_stop
				let delta = MessageDelta {
					stop_reason: None,
					stop_sequence: None,
					usage: Some(usage),
				};
				return Ok(Some(StreamEvent::MessageDelta { delta }));
			}
			Ok(None)
		}

		/// Finalize the stream and return the final message_stop event
		pub fn finalize(&mut self) -> Result<StreamEvent, AIError> {
			// Clean up any remaining buffers
			if !self.tool_json_buffers.is_empty() {
				let total_size: usize = self.tool_json_buffers.values().map(|s| s.len()).sum();
				GLOBAL_BUFFER_USAGE.fetch_sub(total_size, Ordering::Relaxed);
				
				debug!(
					"clearing {} tool JSON buffers at stream end (total size: {} bytes)",
					self.tool_json_buffers.len(),
					total_size
				);
				self.tool_json_buffers.clear();
			}

			Ok(StreamEvent::MessageStop)
		}
	}

	/// Serialize Anthropic StreamEvent to SSE format
	fn serialize_anthropic_event_to_sse(event: &StreamEvent) -> Result<Bytes, AIError> {
		let event_type = match event {
			StreamEvent::MessageStart { .. } => "message_start",
			StreamEvent::ContentBlockStart { .. } => "content_block_start",
			StreamEvent::Ping => "ping",
			StreamEvent::ContentBlockDelta { .. } => "content_block_delta",
			StreamEvent::ContentBlockStop { .. } => "content_block_stop",
			StreamEvent::MessageDelta { .. } => "message_delta",
			StreamEvent::MessageStop => "message_stop",
			StreamEvent::Error { .. } => "error",
		};

		let json_data = serde_json::to_string(event).map_err(AIError::ResponseMarshal)?;
		let sse_frame = format!("event: {}\ndata: {}\n\n", event_type, json_data);
		Ok(Bytes::from(sse_frame))
	}

	pin_project! {
		/// Custom Body that transforms Bedrock EventStream to Anthropic SSE inline
		pub struct AnthropicStreamBody {
			#[pin]
			upstream: Body,
			decoder: EventStreamCodec,
			decode_buffer: bytes::BytesMut,
			outbound_frames: VecDeque<Bytes>,
			processor: BedrockStreamProcessor,
			log: AsyncLog<LLMResponse>,
			rate_limit: Option<LLMResponsePolicies>,
			finished: bool,
		}
	}

	impl AnthropicStreamBody {
		pub fn new(
			upstream: Body,
			message_id: String,
			model: String,
			log: AsyncLog<LLMResponse>,
			rate_limit: Option<LLMResponsePolicies>,
		) -> Self {
			Self {
				upstream,
				decoder: EventStreamCodec,
				decode_buffer: bytes::BytesMut::new(),
				outbound_frames: VecDeque::new(),
				processor: BedrockStreamProcessor::new(message_id, model),
				log,
				rate_limit,
				finished: false,
			}
		}
	}

	impl http_body::Body for AnthropicStreamBody {
		type Data = Bytes;
		type Error = Box<dyn std::error::Error + Send + Sync>;

		fn poll_frame(
			self: Pin<&mut Self>,
			cx: &mut Context<'_>,
		) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
			let mut this = self.project();

			loop {
				// 1) If we have queued frames, yield one
				if let Some(frame_data) = this.outbound_frames.pop_front() {
					return Poll::Ready(Some(Ok(http_body::Frame::data(frame_data))));
				}

				// 2) If finished and no more frames, we're done
				if *this.finished {
					return Poll::Ready(None);
				}

				// 3) Try to decode complete EventStream messages from buffer
				let mut decoded_any = false;
				loop {
					match this.decoder.decode(this.decode_buffer) {
						Ok(Some(message)) => {
							decoded_any = true;
							// Process Bedrock event → 0..N Anthropic events
							if let Ok(bedrock_event) = ConverseStreamOutput::deserialize(message) {
								match this.processor.process_event(bedrock_event, this.log) {
									Ok(anthropic_events) => {
										// Convert each Anthropic event to SSE frame and queue
										for event in anthropic_events {
											let sse_bytes = serialize_anthropic_event_to_sse(&event)
												.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
											this.outbound_frames.push_back(sse_bytes);
										}
									},
									Err(_e) => {
										// Skip processing errors, continue with stream
									},
								}
							}
						},
						Ok(None) => {
							// Need more input data
							break;
						},
						Err(_e) => {
							// Skip malformed EventStream messages
							break;
						},
					}
				}

				// If we decoded events and now have frames queued, yield one
				if decoded_any && !this.outbound_frames.is_empty() {
					continue; // Go back to step 1 to yield frame
				}

				// 4) Poll upstream for more data
				match ready!(this.upstream.as_mut().poll_frame(cx)) {
					Some(Ok(frame)) => {
						if let Some(data) = frame.data_ref() {
							this.decode_buffer.extend_from_slice(data);
							// Continue the loop to decode immediately
							continue;
						}
						// Frame with no data, poll again
						continue;
					},
					Some(Err(e)) => {
						return Poll::Ready(Some(Err(
							Box::new(e) as Box<dyn std::error::Error + Send + Sync>
						)));
					},
					None => {
						// Upstream finished - emit final message_stop and mark finished
						*this.finished = true;

						if let (Some(usage), Some(rate_limit)) =
							(&this.processor.current_usage, this.rate_limit.take())
						{
							this.log.non_atomic_mutate(|llm_resp| {
								llm_resp.input_tokens_from_response = Some(usage.input_tokens as u64);
								llm_resp.output_tokens = Some(usage.output_tokens as u64);
								llm_resp.total_tokens = Some((usage.input_tokens + usage.output_tokens) as u64);

								// Call amend_tokens with actual usage
								crate::llm::amend_tokens(rate_limit, llm_resp);
							});
						}

						match this.processor.finalize() {
							Ok(final_event) => {
								let sse_bytes = serialize_anthropic_event_to_sse(&final_event)
									.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
								return Poll::Ready(Some(Ok(http_body::Frame::data(sse_bytes))));
							},
							Err(_e) => {
								// No final event, just finish
								return Poll::Ready(None);
							},
						}
					},
				}
			}
		}
	}

	/// Translate Bedrock stop reasons to Anthropic stop reasons
	fn translate_stop_reason_to_anthropic(stop_reason: StopReason) -> String {
		match stop_reason {
			StopReason::EndTurn => "end_turn".to_string(),
			StopReason::MaxTokens => "max_tokens".to_string(),  
			StopReason::StopSequence => "stop_sequence".to_string(),
			StopReason::ContentFiltered => "stop_sequence".to_string(), // Map to stop_sequence 
			StopReason::GuardrailIntervened => "stop_sequence".to_string(), // Map to stop_sequence
			StopReason::ToolUse => "tool_use".to_string(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::llm::universal;
	use serde_json::json;
	use std::collections::HashMap;

	#[test]
	fn test_anthropic_beta_extraction() {
		// Create a Universal request with anthropic beta headers in vendor bag
		let mut vendor = HashMap::new();
		vendor.insert("anthropic".to_string(), json!({
			"headers": {
				"beta": ["computer-use-2025-01-24", "fine-grained-tool-streaming-2025-05-14"]
			}
		}));

		let request = universal::Request {
			model: "claude-3-sonnet-20240229".to_string(),
			messages: vec![universal::RequestMessage::User(universal::RequestUserMessage {
				content: universal::RequestUserMessageContent::Text("test".to_string()),
				name: None,
			})],
			temperature: None,
			max_completion_tokens: Some(100),
			max_tokens: None,
			top_p: None,
			stop: None,
			stream: None,
			tools: None,
			tool_choice: None,
			vendor: Some(vendor),
		};

		let provider = Provider::new("aws".to_string(), "bedrock".to_string());
		let converse_request = translate_openai_request(&request, &provider, "anthropic.claude-3-sonnet-20240229-v1:0");

		// Verify that additional_model_request_fields contains anthropic_beta
		assert!(converse_request.additional_model_request_fields.is_some());
		let additional_fields = converse_request.additional_model_request_fields.unwrap();
		
		let expected = json!({
			"anthropic_beta": ["computer-use-2025-01-24", "fine-grained-tool-streaming-2025-05-14"]
		});
		
		assert_eq!(additional_fields, expected);
	}

	#[test]
	fn test_no_beta_headers() {
		// Create a Universal request without beta headers
		let request = universal::Request {
			model: "claude-3-sonnet-20240229".to_string(),
			messages: vec![universal::RequestMessage::User(universal::RequestUserMessage {
				content: universal::RequestUserMessageContent::Text("test".to_string()),
				name: None,
			})],
			temperature: None,
			max_completion_tokens: Some(100),
			max_tokens: None,
			top_p: None,
			stop: None,
			stream: None,
			tools: None,
			tool_choice: None,
			vendor: None,
		};

		let provider = Provider::new("aws".to_string(), "bedrock".to_string());
		let converse_request = translate_openai_request(&request, &provider, "anthropic.claude-3-sonnet-20240229-v1:0");

		// Verify that additional_model_request_fields is None when no beta headers
		assert!(converse_request.additional_model_request_fields.is_none());
	}

	#[test]
	fn test_empty_beta_headers() {
		// Create a Universal request with empty beta headers array
		let mut vendor = HashMap::new();
		vendor.insert("anthropic".to_string(), json!({
			"headers": {
				"beta": []
			}
		}));

		let request = universal::Request {
			model: "claude-3-sonnet-20240229".to_string(),
			messages: vec![universal::RequestMessage::User(universal::RequestUserMessage {
				content: universal::RequestUserMessageContent::Text("test".to_string()),
				name: None,
			})],
			temperature: None,
			max_completion_tokens: Some(100),
			max_tokens: None,
			top_p: None,
			stop: None,
			stream: None,
			tools: None,
			tool_choice: None,
			vendor: Some(vendor),
		};

		let provider = Provider::new("aws".to_string(), "bedrock".to_string());
		let converse_request = translate_openai_request(&request, &provider, "anthropic.claude-3-sonnet-20240229-v1:0");

		// Verify that additional_model_request_fields is None when beta headers array is empty
		assert!(converse_request.additional_model_request_fields.is_none());
	}
}
