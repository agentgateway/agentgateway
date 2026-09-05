use std::collections::BTreeMap;
use std::time::Instant;

use agent_core::strng::{self, Strng};
use axum_core::body::Body;
use bytes::Bytes;

use crate::types::completions::typed as completions;
use crate::types::messages::typed as messages;
use crate::{AIError, StreamingUsageGuard, parse};

const ANTHROPIC_MIN_THINKING_BUDGET_TOKENS: u64 = 1024;

fn cap_thinking_budget_to_max_tokens(budget_tokens: u64, max_tokens: usize) -> Option<u64> {
	let max_tokens = u64::try_from(max_tokens).unwrap_or(u64::MAX);
	if budget_tokens < ANTHROPIC_MIN_THINKING_BUDGET_TOKENS
		|| max_tokens <= ANTHROPIC_MIN_THINKING_BUDGET_TOKENS
	{
		return None;
	}
	Some(budget_tokens.min(max_tokens - 1))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cap_thinking_budget_enforces_anthropic_bounds() {
		assert_eq!(cap_thinking_budget_to_max_tokens(1, 4096), None);
		assert_eq!(cap_thinking_budget_to_max_tokens(1024, 1024), None);
		assert_eq!(cap_thinking_budget_to_max_tokens(1024, 1025), Some(1024));
		assert_eq!(cap_thinking_budget_to_max_tokens(8192, 4096), Some(4095));
	}
}

fn anthropic_error_type(status: ::http::StatusCode) -> &'static str {
	match status {
		::http::StatusCode::BAD_REQUEST => "invalid_request_error",
		::http::StatusCode::UNAUTHORIZED | ::http::StatusCode::FORBIDDEN => "authentication_error",
		::http::StatusCode::NOT_FOUND => "not_found_error",
		::http::StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
		_ => "api_error",
	}
}

pub fn translate_anthropic_error(
	bytes: &Bytes,
	status: ::http::StatusCode,
) -> Result<Bytes, AIError> {
	if serde_json::from_slice::<messages::MessagesErrorResponse>(bytes).is_ok() {
		return Ok(bytes.clone());
	}
	let m = messages::MessagesErrorResponse {
		r#type: "error".to_string(),
		error: messages::MessagesError {
			r#type: anthropic_error_type(status).to_string(),
			message: String::from_utf8_lossy(bytes).into_owned(),
		},
	};
	Ok(Bytes::from(
		serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
	))
}

/// Translate a Google error response into an Anthropic Messages error response.
pub fn translate_google_error(bytes: &Bytes) -> Result<Bytes, AIError> {
	let res = super::completions::parse_google_error(bytes)?;
	let m = messages::MessagesErrorResponse {
		r#type: "error".to_string(),
		error: messages::MessagesError {
			r#type: super::completions::google_error_type(&res.error).to_string(),
			message: res.error.message.clone(),
		},
	};
	Ok(Bytes::from(
		serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
	))
}

pub mod from_completions {
	use std::collections::HashMap;
	use std::time::Instant;

	use agent_core::strng;
	use axum_core::body::Body;
	use bytes::Bytes;

	use crate::conversion::completions::parse_data_url;
	use crate::types::ResponseType;
	use crate::types::completions::typed as completions;
	use crate::types::completions::typed::UsagePromptDetails;
	use crate::types::messages::typed as messages;
	use crate::{AIError, StreamingUsageGuard, json, logged_response_parsing, parse, types};

	fn cache_control(
		breakpoint: &Option<completions::PromptCacheBreakpointParam>,
	) -> Option<messages::CacheControlEphemeral> {
		breakpoint
			.as_ref()
			.map(|_| messages::CacheControlEphemeral::Ephemeral { ttl: None })
	}

	fn user_content_to_messages(
		content: &completions::RequestUserMessageContent,
	) -> Vec<messages::ContentBlock> {
		let mut out = Vec::new();
		match content {
			completions::RequestUserMessageContent::Text(text) => {
				if !text.trim().is_empty() {
					out.push(messages::ContentBlock::Text(messages::ContentTextBlock {
						text: text.clone(),
						citations: None,
						cache_control: None,
					}));
				}
			},
			completions::RequestUserMessageContent::Array(parts) => {
				for part in parts {
					match part {
						completions::RequestUserMessageContentPart::Text(text) => {
							if !text.text.trim().is_empty() {
								out.push(messages::ContentBlock::Text(messages::ContentTextBlock {
									text: text.text.clone(),
									citations: None,
									cache_control: cache_control(&text.prompt_cache_breakpoint),
								}));
							}
						},
						completions::RequestUserMessageContentPart::ImageUrl(image) => {
							let source = if let Some((media_type, data)) = parse_data_url(&image.image_url.url) {
								serde_json::json!({
									"type": "base64",
									"media_type": media_type,
									"data": data
								})
							} else {
								serde_json::json!({
									"type": "url",
									"url": image.image_url.url
								})
							};
							out.push(messages::ContentBlock::Image(messages::ContentImageBlock {
								source,
								cache_control: cache_control(&image.prompt_cache_breakpoint),
							}));
						},
						completions::RequestUserMessageContentPart::InputAudio(_)
						| completions::RequestUserMessageContentPart::File(_) => {},
					}
				}
			},
		}
		out
	}

	fn assistant_content_to_messages(
		msg: &completions::RequestAssistantMessage,
	) -> Vec<messages::ContentBlock> {
		let mut out = Vec::new();
		if let Some(content) = &msg.content {
			match content {
				completions::RequestAssistantMessageContent::Text(text) => {
					if !text.trim().is_empty() {
						out.push(messages::ContentBlock::Text(messages::ContentTextBlock {
							text: text.clone(),
							citations: None,
							cache_control: None,
						}));
					}
				},
				completions::RequestAssistantMessageContent::Array(parts) => {
					for part in parts {
						match part {
							completions::RequestAssistantMessageContentPart::Text(text) => {
								if !text.text.trim().is_empty() {
									out.push(messages::ContentBlock::Text(messages::ContentTextBlock {
										text: text.text.clone(),
										citations: None,
										cache_control: cache_control(&text.prompt_cache_breakpoint),
									}));
								}
							},
							completions::RequestAssistantMessageContentPart::Refusal(refusal) => {
								if !refusal.refusal.trim().is_empty() {
									out.push(messages::ContentBlock::Text(messages::ContentTextBlock {
										text: refusal.refusal.clone(),
										citations: None,
										cache_control: None,
									}));
								}
							},
						}
					}
				},
			}
		}
		if let Some(refusal) = &msg.refusal
			&& !refusal.trim().is_empty()
		{
			out.push(messages::ContentBlock::Text(messages::ContentTextBlock {
				text: refusal.clone(),
				citations: None,
				cache_control: None,
			}));
		}
		if let Some(tool_calls) = &msg.tool_calls {
			for tool_call in tool_calls {
				match tool_call {
					completions::MessageToolCalls::Function(call) => {
						let input = crate::conversion::tool_arguments_to_input(&call.function.arguments);
						out.push(messages::ContentBlock::ToolUse {
							id: call.id.clone(),
							name: call.function.name.clone(),
							input,
							cache_control: None,
						});
					},
					completions::MessageToolCalls::Custom(call) => {
						let input = crate::conversion::tool_arguments_to_input(&call.custom_tool.input);
						out.push(messages::ContentBlock::ToolUse {
							id: call.id.clone(),
							name: call.custom_tool.name.clone(),
							input,
							cache_control: None,
						});
					},
				}
			}
		}
		out
	}

	fn tool_content_to_messages(
		content: &completions::RequestToolMessageContent,
	) -> messages::ToolResultContent {
		match content {
			completions::RequestToolMessageContent::Text(text) => {
				messages::ToolResultContent::Text(text.clone())
			},
			completions::RequestToolMessageContent::Array(parts) => {
				let parts = parts
					.iter()
					.map(|part| match part {
						completions::RequestToolMessageContentPart::Text(text) => {
							messages::ToolResultContentPart::Text {
								text: text.text.clone(),
								citations: None,
								cache_control: cache_control(&text.prompt_cache_breakpoint),
							}
						},
					})
					.collect();
				messages::ToolResultContent::Array(parts)
			},
		}
	}

	/// translate an OpenAI completions request to an anthropic messages request
	pub fn translate(
		req: &types::completions::Request,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, completions::Request>(req).map_err(AIError::RequestMarshal)?;
		let model_id = typed.model.clone().unwrap_or_default();
		let xlated = translate_internal(typed, model_id, catalog);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	fn translate_internal(
		req: completions::Request,
		model_id: String,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> messages::Request {
		let max_tokens = req.max_tokens();
		let stop_sequences = req.stop_sequence();
		let mut system_blocks = Vec::new();
		for message in &req.messages {
			match message {
				completions::RequestMessage::System(message) => match &message.content {
					completions::RequestSystemMessageContent::Text(text) => {
						if !text.trim().is_empty() {
							system_blocks.push(messages::SystemContentBlock::Text {
								text: text.clone(),
								cache_control: None,
							});
						}
					},
					completions::RequestSystemMessageContent::Array(parts) => {
						for part in parts {
							let completions::RequestSystemMessageContentPart::Text(text) = part;
							if !text.text.trim().is_empty() {
								system_blocks.push(messages::SystemContentBlock::Text {
									text: text.text.clone(),
									cache_control: cache_control(&text.prompt_cache_breakpoint),
								});
							}
						}
					},
				},
				completions::RequestMessage::Developer(message) => match &message.content {
					completions::RequestDeveloperMessageContent::Text(text) => {
						if !text.trim().is_empty() {
							system_blocks.push(messages::SystemContentBlock::Text {
								text: text.clone(),
								cache_control: None,
							});
						}
					},
					completions::RequestDeveloperMessageContent::Array(parts) => {
						for part in parts {
							let completions::RequestDeveloperMessageContentPart::Text(text) = part;
							if !text.text.trim().is_empty() {
								system_blocks.push(messages::SystemContentBlock::Text {
									text: text.text.clone(),
									cache_control: cache_control(&text.prompt_cache_breakpoint),
								});
							}
						}
					},
				},
				_ => {},
			}
		}
		let system = if system_blocks.is_empty() {
			None
		} else if system_blocks.iter().any(|block| match block {
			messages::SystemContentBlock::Text { cache_control, .. } => cache_control.is_some(),
		}) {
			Some(messages::SystemPrompt::Blocks(system_blocks))
		} else {
			Some(messages::SystemPrompt::Text(
				system_blocks
					.into_iter()
					.map(|block| match block {
						messages::SystemContentBlock::Text { text, .. } => text,
					})
					.collect::<Vec<_>>()
					.join("\n"),
			))
		};

		// Convert messages to Anthropic format
		let messages = req
			.messages
			.iter()
			.filter_map(|msg| {
				let (role, content) = match msg {
					completions::RequestMessage::System(_) | completions::RequestMessage::Developer(_) => {
						return None;
					},
					completions::RequestMessage::User(user) => (
						messages::Role::User,
						user_content_to_messages(&user.content),
					),
					completions::RequestMessage::Assistant(assistant) => (
						messages::Role::Assistant,
						assistant_content_to_messages(assistant),
					),
					completions::RequestMessage::Tool(tool) => (
						messages::Role::User,
						vec![messages::ContentBlock::ToolResult {
							tool_use_id: tool.tool_call_id.clone(),
							content: tool_content_to_messages(&tool.content),
							cache_control: None,
							is_error: None,
						}],
					),
					completions::RequestMessage::Function(function) => {
						let mut blocks = Vec::new();
						if let Some(text) = &function.content
							&& !text.trim().is_empty()
						{
							blocks.push(messages::ContentBlock::Text(messages::ContentTextBlock {
								text: text.clone(),
								citations: None,
								cache_control: None,
							}));
						}
						(messages::Role::User, blocks)
					},
				};
				if content.is_empty() {
					None
				} else {
					Some(messages::Message { role, content })
				}
			})
			.collect();

		let tools = if let Some(tools) = req.tools {
			let mapped_tools: Vec<_> = tools
				.iter()
				.filter_map(|tool| match tool {
					completions::Tool::Function(function_tool) => {
						Some(messages::Tool::Custom(messages::CustomTool {
							name: function_tool.function.name.clone(),
							description: function_tool.function.description.clone(),
							input_schema: function_tool
								.function
								.parameters
								.clone()
								.unwrap_or_default(),
							cache_control: None,
						}))
					},
					_ => None,
				})
				.collect();
			Some(mapped_tools)
		} else {
			None
		};
		let metadata = req.user.map(|user| messages::Metadata {
			fields: HashMap::from([("user_id".to_string(), user)]),
		});

		let disable_parallel_tool_use = req.parallel_tool_calls.map(|p| !p);
		let has_tools = tools.as_ref().is_some_and(|tools| !tools.is_empty());
		let tool_choice = match req.tool_choice {
			Some(completions::ToolChoiceOption::Function(completions::NamedToolChoice { function })) => {
				Some(messages::ToolChoice::Tool {
					name: function.name,
					disable_parallel_tool_use,
				})
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto)) => {
				Some(messages::ToolChoice::Auto {
					disable_parallel_tool_use,
				})
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required)) => {
				Some(messages::ToolChoice::Any {
					disable_parallel_tool_use,
				})
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None)) => {
				Some(messages::ToolChoice::None {})
			},
			None if disable_parallel_tool_use.is_some() && has_tools => {
				Some(messages::ToolChoice::Auto {
					disable_parallel_tool_use,
				})
			},
			_ => None,
		};
		let capabilities = crate::model_catalog::anthropic_thinking_capabilities(&model_id, catalog);
		let explicit_budget = req.vendor_extensions.thinking_budget_tokens;
		let effort = req
			.reasoning_effort
			.as_ref()
			.and_then(crate::types::anthropic_effort_for_reasoning_effort);
		let (thinking, effort) = if let Some(budget_tokens) = explicit_budget
			&& capabilities.legacy
		{
			(
				super::cap_thinking_budget_to_max_tokens(budget_tokens, max_tokens)
					.map(|budget_tokens| messages::ThinkingInput::Enabled { budget_tokens }),
				None,
			)
		} else if (explicit_budget.is_some() || effort.is_some()) && capabilities.adaptive {
			(
				Some(messages::ThinkingInput::Adaptive {}),
				Some(effort.unwrap_or(messages::ThinkingEffort::High)),
			)
		} else {
			let budget_tokens =
				explicit_budget.or_else(|| effort.map(crate::types::thinking_budget_for_anthropic_effort));
			(
				budget_tokens
					.and_then(|budget_tokens| {
						super::cap_thinking_budget_to_max_tokens(budget_tokens, max_tokens)
					})
					.map(|budget_tokens| messages::ThinkingInput::Enabled { budget_tokens }),
				None,
			)
		};

		let response_format = match req.response_format {
			Some(completions::ResponseFormat::JsonSchema { json_schema }) => {
				Some(messages::OutputFormat::JsonSchema {
					schema: json_schema.schema,
				})
			},
			Some(completions::ResponseFormat::JsonObject) => Some(messages::OutputFormat::JsonSchema {
				schema: serde_json::json!({
					"type": "object",
					"additionalProperties": true
				}),
			}),
			Some(completions::ResponseFormat::Text) | None => None,
		};
		let output_config = if response_format.is_some() || effort.is_some() {
			Some(messages::OutputConfig {
				effort,
				format: response_format,
			})
		} else {
			None
		};
		messages::Request {
			messages,
			system,
			model: model_id,
			max_tokens,
			stop_sequences,
			stream: req.stream.unwrap_or(false),
			temperature: req.temperature,
			top_p: req.top_p,
			top_k: None, // OpenAI doesn't have top_k
			tools,
			tool_choice,
			metadata,
			thinking,
			output_config,
		}
	}

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<messages::MessagesResponse>(bytes)
			.map_err(logged_response_parsing(bytes))?;
		let openai = translate_response_internal(resp);
		let passthrough = json::convert::<_, types::completions::Response>(&openai)
			.map_err(AIError::ResponseParsing)?;
		Ok(Box::new(passthrough))
	}

	fn translate_response_internal(resp: messages::MessagesResponse) -> completions::Response {
		// Convert Anthropic content blocks to OpenAI message content
		let mut tool_calls: Vec<completions::MessageToolCalls> = Vec::new();
		let mut content = None;
		let mut reasoning_content = None;
		for block in resp.content {
			match block {
				messages::ContentBlock::Text(messages::ContentTextBlock { text, .. }) => {
					// A message may contain multiple text blocks (e.g. text split around
					// citations); concatenate them into the single OpenAI `content` field.
					content.get_or_insert_with(String::new).push_str(&text)
				},
				messages::ContentBlock::ToolUse {
					id, name, input, ..
				}
				| messages::ContentBlock::ServerToolUse {
					id, name, input, ..
				} => {
					let Some(args) = serde_json::to_string(&input).ok() else {
						continue;
					};
					tool_calls.push(completions::MessageToolCalls::Function(
						completions::MessageToolCall {
							id: id.clone(),
							function: completions::FunctionCall {
								name: name.clone(),
								arguments: args,
							},
						},
					));
				},
				messages::ContentBlock::ToolResult { .. } => {
					// Should be on the request path, not the response path
					continue;
				},
				// For now we ignore Redacted and signature think through a better approach as this may be needed
				messages::ContentBlock::Thinking { thinking, .. } => {
					reasoning_content = Some(thinking);
				},
				messages::ContentBlock::RedactedThinking { .. } => {},

				// not currently supported
				messages::ContentBlock::Image { .. } => continue,
				messages::ContentBlock::Document(_) => continue,
				messages::ContentBlock::SearchResult(_) => continue,
				messages::ContentBlock::WebSearchToolResult { .. } => continue,
				messages::ContentBlock::Unknown => continue,
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
			reasoning_content,
			reasoning_signature: None,
			extra: None,
		};
		let finish_reason = resp.stop_reason.as_ref().map(super::translate_stop_reason);
		// Only one choice for anthropic
		let choice = completions::ChatChoice {
			rest: Default::default(),
			index: 0,
			message,
			finish_reason,
			logprobs: None,
		};

		let choices = vec![choice];
		// Convert usage from Anthropic format to OpenAI format
		let usage = completions::Usage {
			prompt_tokens: resp.usage.input_tokens as u32,
			completion_tokens: resp.usage.output_tokens as u32,
			total_tokens: (resp.usage.input_tokens + resp.usage.output_tokens) as u32,
			cache_read_input_tokens: resp.usage.cache_read_input_tokens.map(|i| i as u64),
			prompt_tokens_details: match (
				resp.usage.cache_read_input_tokens,
				resp.usage.cache_creation_input_tokens,
			) {
				(None, None) => None,
				(cached_tokens, cache_write_tokens) => Some(UsagePromptDetails {
					cached_tokens: cached_tokens.map(|i| i as u64),
					audio_tokens: None,
					cache_write_tokens: cache_write_tokens.map(|i| i as u64),
					rest: Default::default(),
				}),
			},
			cache_creation_input_tokens: resp.usage.cache_creation_input_tokens.map(|i| i as u64),

			completion_tokens_details: None,
		};

		completions::Response {
			id: resp.id,
			object: "chat.completion".to_string(),
			// No date in anthropic response so just call it "now"
			created: chrono::Utc::now().timestamp() as u32,
			model: resp.model,
			choices,
			usage: Some(usage),
			service_tier: resp.usage.service_tier,
			system_fingerprint: None,
		}
	}

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		let res = serde_json::from_slice::<messages::MessagesErrorResponse>(bytes).ok();
		let m = completions::ChatCompletionErrorResponse {
			event_id: None,
			error: completions::ChatCompletionError {
				r#type: Some("invalid_request_error".to_string()),
				message: res
					.map(|res| res.error.message)
					.unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
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
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
	) -> Body {
		/// An ongoing `tool_use` block. `emitted_arguments` tracks whether we have put any argument
		/// text on the wire yet, so `content_block_stop` can synthesize `{}` when Anthropic
		/// streamed nothing at all.
		struct OngoingToolCall {
			tool_index: u32,
			emitted_arguments: bool,
		}

		let mut message_id = None;
		let mut model = String::new();
		// Anthropic states the role once in `message_start`, but the OpenAI chunk has no top-level
		// role field — it belongs on `choices[].delta`, so hold it until there is a chunk for it.
		let mut pending_role = None;
		let mut service_tier = None;
		let created = chrono::Utc::now().timestamp() as u32;
		// let mut finish_reason = None;
		let mut saw_token = false;
		let mut next_tool_index = 0u32;
		let mut ongoing_tool_calls: HashMap<usize, OngoingToolCall> = HashMap::new();
		let mut completion = log_content.completion.then(String::new);
		let mut tool_calls = super::StreamingToolCalls::new(log_content.tool_calls);

		// https://docs.anthropic.com/en/docs/build-with-claude/streaming
		let body = parse::sse::json_transform::<
			messages::MessagesStreamEvent,
			completions::StreamResponse,
		>(b, buffer_limit, move |f| {
			let mut mk = |mut choices: Vec<completions::ChatChoiceStream>,
			              usage: Option<completions::Usage>| {
				if let Some(first) = choices.first_mut() {
					first.delta.role = pending_role.take();
				}
				Some(completions::StreamResponse {
					id: message_id.clone().unwrap_or_else(|| "unknown".to_string()),
					model: model.clone(),
					object: "chat.completion.chunk".to_string(),
					system_fingerprint: None,
					service_tier: service_tier.clone(),
					created,
					choices,
					usage,
				})
			};
			// ignore errors... what else can we do?
			let f = f.ok()?;

			// Extract info we need
			match f {
				messages::MessagesStreamEvent::MessageStart { message } => {
					message_id = Some(message.id);
					pending_role = Some(match message.role {
						messages::Role::Assistant => completions::Role::Assistant,
						messages::Role::User => completions::Role::User,
						messages::Role::System => completions::Role::System,
					});
					model = message.model.clone();
					service_tier = message.usage.service_tier.clone();
					log.update(|r| {
						r.response.output_tokens = Some(message.usage.output_tokens as u64);
						r.response.input_tokens = Some(message.usage.input_tokens as u64);
						r.response.cached_input_tokens =
							message.usage.cache_read_input_tokens.map(|i| i as u64);
						r.response.cache_creation_input_tokens =
							message.usage.cache_creation_input_tokens.map(|i| i as u64);
						r.response.service_tier = message.usage.service_tier.as_deref().map(Into::into);
						r.response.provider_model = Some(strng::new(&message.model))
					});
					// no need to respond with anything yet
					None
				},

				messages::MessagesStreamEvent::ContentBlockStart {
					index,
					content_block,
				} => match content_block {
					messages::ContentBlock::ToolUse {
						id, name, input, ..
					}
					| messages::ContentBlock::ServerToolUse {
						id, name, input, ..
					} => {
						let tool_index = next_tool_index;
						next_tool_index += 1;
						tool_calls.start(index, id.as_str(), name.as_str(), &input);
						// `input` is always `{}` here: the payload arrives as `input_json_delta`s.
						ongoing_tool_calls.insert(
							index,
							OngoingToolCall {
								tool_index,
								emitted_arguments: false,
							},
						);

						let choice = completions::ChatChoiceStream {
							rest: Default::default(),
							index: 0,
							logprobs: None,
							delta: completions::StreamResponseDelta {
								tool_calls: Some(vec![completions::ChatCompletionMessageToolCallChunk {
									index: tool_index,
									id: Some(id),
									r#type: Some(completions::FunctionType::Function),
									function: Some(completions::FunctionCallStream {
										name: Some(name),
										arguments: None,
									}),
								}]),
								..Default::default()
							},
							finish_reason: None,
						};
						mk(vec![choice], None)
					},
					_ => None,
				},
				messages::MessagesStreamEvent::ContentBlockDelta { delta, index } => {
					if !saw_token {
						saw_token = true;
						log.update(|r| {
							r.response.first_token = Some(Instant::now());
						});
					}
					let mut dr = completions::StreamResponseDelta::default();
					let mut emit_chunk = true;
					match delta {
						messages::ContentBlockDelta::TextDelta { text } => {
							if let Some(completion) = completion.as_mut() {
								completion.push_str(&text);
							}
							dr.content = Some(text);
						},
						messages::ContentBlockDelta::ThinkingDelta { thinking } => {
							dr.reasoning_content = Some(thinking)
						},
						messages::ContentBlockDelta::InputJsonDelta { partial_json } => {
							tool_calls.append_arguments(index, &partial_json);
							match ongoing_tool_calls.get_mut(&index) {
								Some(_) if partial_json.is_empty() => emit_chunk = false,
								Some(ongoing) => {
									ongoing.emitted_arguments = true;
									dr.tool_calls = Some(vec![completions::ChatCompletionMessageToolCallChunk {
										index: ongoing.tool_index,
										id: None,
										r#type: None,
										function: Some(completions::FunctionCallStream {
											name: None,
											arguments: Some(partial_json),
										}),
									}]);
								},
								None => emit_chunk = false,
							}
						},
						messages::ContentBlockDelta::SignatureDelta { .. }
						| messages::ContentBlockDelta::CitationsDelta { .. } => {
							emit_chunk = false;
						},
					};
					if emit_chunk {
						let choice = completions::ChatChoiceStream {
							rest: Default::default(),
							index: 0,
							logprobs: None,
							delta: dr,
							finish_reason: None,
						};
						mk(vec![choice], None)
					} else {
						None
					}
				},
				messages::MessagesStreamEvent::MessageDelta { usage, delta } => {
					let finish_reason = delta.stop_reason.as_ref().map(super::translate_stop_reason);
					let log_finish_reason = delta
						.stop_reason
						.as_ref()
						.and_then(crate::types::serialize_str);
					log.update(|r| {
						if let Some(inp) = usage.input_tokens {
							r.response.input_tokens = Some(inp as u64);
						}
						if let Some(crt) = usage.cache_read_input_tokens {
							r.response.cached_input_tokens = Some(crt as u64);
						}
						if let Some(cwt) = usage.cache_creation_input_tokens {
							r.response.cache_creation_input_tokens = Some(cwt as u64);
						}
						if let Some(o) = usage.output_tokens {
							r.response.output_tokens = Some(o as u64);
						}
						if let Some(inp) = r.response.input_tokens
							&& let Some(o) = r.response.output_tokens
						{
							r.response.total_tokens = Some(inp + o)
						}
						if let Some(completion) = completion.take() {
							r.response.completion = Some(vec![completion]);
						}
						r.response.output_messages = tool_calls.take_output_messages(log_finish_reason.clone());
					});
					let choices = finish_reason.map_or_else(Vec::new, |finish_reason| {
						vec![completions::ChatChoiceStream {
							rest: Default::default(),
							index: 0,
							logprobs: None,
							delta: completions::StreamResponseDelta::default(),
							finish_reason: Some(finish_reason),
						}]
					});
					mk(
						choices,
						Some(completions::Usage {
							prompt_tokens: usage.input_tokens.unwrap_or_default() as u32,
							completion_tokens: usage.output_tokens.unwrap_or_default() as u32,

							total_tokens: (usage.input_tokens.unwrap_or_default()
								+ usage.output_tokens.unwrap_or_default()) as u32,

							cache_read_input_tokens: usage.cache_read_input_tokens.map(|i| i as u64),
							prompt_tokens_details: match (
								usage.cache_read_input_tokens,
								usage.cache_creation_input_tokens,
							) {
								(None, None) => None,
								(cached_tokens, cache_write_tokens) => Some(UsagePromptDetails {
									cached_tokens: cached_tokens.map(|i| i as u64),
									audio_tokens: None,
									cache_write_tokens: cache_write_tokens.map(|i| i as u64),
									rest: Default::default(),
								}),
							},
							cache_creation_input_tokens: usage.cache_creation_input_tokens.map(|i| i as u64),

							completion_tokens_details: None,
						}),
					)
				},
				messages::MessagesStreamEvent::ContentBlockStop { index } => {
					match ongoing_tool_calls.remove(&index) {
						Some(ongoing) if !ongoing.emitted_arguments => {
							// If no arguments were emitted for a tool call, send a synthetic `{}`
							// for compatibility.
							let choice = completions::ChatChoiceStream {
								rest: Default::default(),
								index: 0,
								logprobs: None,
								delta: completions::StreamResponseDelta {
									tool_calls: Some(vec![completions::ChatCompletionMessageToolCallChunk {
										index: ongoing.tool_index,
										id: None,
										r#type: None,
										function: Some(completions::FunctionCallStream {
											name: None,
											arguments: Some("{}".to_string()),
										}),
									}]),
									..Default::default()
								},
								finish_reason: None,
							};
							mk(vec![choice], None)
						},
						_ => None,
					}
				},
				messages::MessagesStreamEvent::MessageStop => {
					log.update(|r| {
						if let Some(completion) = completion.take() {
							r.response.completion = Some(vec![completion]);
						}
						if r.response.output_messages.is_none() {
							r.response.output_messages = tool_calls.take_output_messages(None);
						}
					});
					None
				},
				messages::MessagesStreamEvent::Ping => None,
				messages::MessagesStreamEvent::Error { error } => {
					tracing::warn!(
						"Messages stream error during completions translation: {}",
						error.message
					);
					None
				},
			}
		});

		parse::sse::append_done_on_success(body)
	}
}

fn translate_stop_reason(resp: &messages::StopReason) -> completions::FinishReason {
	match resp {
		messages::StopReason::EndTurn => completions::FinishReason::Stop,
		messages::StopReason::MaxTokens => completions::FinishReason::Length,
		messages::StopReason::StopSequence => completions::FinishReason::Stop,
		messages::StopReason::ToolUse => completions::FinishReason::ToolCalls,
		messages::StopReason::Refusal => completions::FinishReason::ContentFilter,
		messages::StopReason::PauseTurn => completions::FinishReason::Stop,
		messages::StopReason::ModelContextWindowExceeded => completions::FinishReason::Length,
	}
}

struct PendingStreamingToolCall {
	id: Strng,
	name: Strng,
	arguments: String,
}

pub(crate) struct StreamingToolCalls {
	calls: Option<BTreeMap<usize, PendingStreamingToolCall>>,
}

impl StreamingToolCalls {
	pub(crate) fn new(enabled: bool) -> Self {
		Self {
			calls: enabled.then(BTreeMap::new),
		}
	}

	pub(crate) fn start(
		&mut self,
		index: usize,
		id: impl Into<Strng>,
		name: impl Into<Strng>,
		input: &serde_json::Value,
	) {
		let Some(calls) = self.calls.as_mut() else {
			return;
		};
		let arguments = match input {
			serde_json::Value::Null => String::new(),
			serde_json::Value::Object(input) if input.is_empty() => String::new(),
			input => input.to_string(),
		};
		calls.insert(
			index,
			PendingStreamingToolCall {
				id: id.into(),
				name: name.into(),
				arguments,
			},
		);
	}

	pub(crate) fn append_arguments(&mut self, index: usize, arguments: &str) {
		if let Some(call) = self.calls.as_mut().and_then(|calls| calls.get_mut(&index)) {
			call.arguments.push_str(arguments);
		}
	}

	pub(crate) fn take_output_messages(
		&mut self,
		finish_reason: Option<Strng>,
	) -> Option<Vec<crate::OutputMessage>> {
		let content: Vec<_> = std::mem::take(self.calls.as_mut()?)
			.into_values()
			.map(|call| {
				let arguments = match serde_json::from_str(&call.arguments) {
					Ok(arguments) => arguments,
					Err(_) if call.arguments.trim().is_empty() => {
						serde_json::Value::Object(Default::default())
					},
					Err(_) => serde_json::Value::String(call.arguments),
				};
				crate::OutputMessagePart::ToolCall {
					id: call.id,
					name: call.name,
					arguments,
				}
			})
			.collect();
		(!content.is_empty()).then(|| {
			vec![crate::OutputMessage {
				role: strng::literal!("assistant"),
				content,
				finish_reason,
			}]
		})
	}
}

pub fn passthrough_stream(
	b: Body,
	buffer_limit: usize,
	log: StreamingUsageGuard,
	log_content: crate::LogContentFields,
) -> Body {
	let mut saw_token = false;
	let mut completion = log_content.completion.then(String::new);
	let mut tool_calls = StreamingToolCalls::new(log_content.tool_calls);
	// https://platform.claude.com/docs/en/build-with-claude/streaming
	parse::sse::json_passthrough::<messages::MessagesStreamEvent>(b, buffer_limit, move |f| {
		// ignore errors... what else can we do?
		let Some(Ok(f)) = f else {
			// Stream ended ([DONE]): flush completion if not already set via MessageDelta
			if f.is_none() {
				log.update(|r| {
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
					if r.response.output_messages.is_none() {
						r.response.output_messages = tool_calls.take_output_messages(None);
					}
				});
			}
			return;
		};

		// Extract info we need
		match f {
			messages::MessagesStreamEvent::MessageStart { message } => {
				log.update(|r| {
					r.response.output_tokens = Some(message.usage.output_tokens as u64);
					r.response.input_tokens = Some(message.usage.input_tokens as u64);
					r.response.cached_input_tokens = message.usage.cache_read_input_tokens.map(|i| i as u64);
					r.response.cache_creation_input_tokens =
						message.usage.cache_creation_input_tokens.map(|i| i as u64);
					r.response.service_tier = message.usage.service_tier.as_deref().map(Into::into);
					r.response.provider_model = Some(strng::new(&message.model))
				});
			},
			messages::MessagesStreamEvent::ContentBlockStart {
				index,
				content_block,
			} => match &content_block {
				messages::ContentBlock::ToolUse {
					id, name, input, ..
				}
				| messages::ContentBlock::ServerToolUse {
					id, name, input, ..
				} => tool_calls.start(index, id.as_str(), name.as_str(), input),
				_ => {},
			},
			messages::MessagesStreamEvent::ContentBlockDelta { index, delta } => {
				if !saw_token {
					saw_token = true;
					log.update(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}
				if let Some(c) = completion.as_mut()
					&& let messages::ContentBlockDelta::TextDelta { text } = &delta
				{
					c.push_str(text);
				}
				if let messages::ContentBlockDelta::InputJsonDelta { partial_json } = &delta {
					tool_calls.append_arguments(index, partial_json);
				}
			},
			messages::MessagesStreamEvent::MessageDelta { usage, delta } => {
				let finish_reason = delta
					.stop_reason
					.as_ref()
					.and_then(crate::types::serialize_str);
				log.update(|r| {
					if let Some(inp) = usage.input_tokens {
						r.response.input_tokens = Some(inp as u64);
					}
					if let Some(o) = usage.output_tokens {
						r.response.output_tokens = Some(o as u64);
					}
					if let Some(crt) = usage.cache_read_input_tokens {
						r.response.cached_input_tokens = Some(crt as u64);
					}
					if let Some(cwt) = usage.cache_creation_input_tokens {
						r.response.cache_creation_input_tokens = Some(cwt as u64);
					}
					if let Some(inp) = r.response.input_tokens
						&& let Some(o) = r.response.output_tokens
					{
						r.response.total_tokens = Some(inp + o)
					}
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
					r.response.output_messages = tool_calls.take_output_messages(finish_reason.clone());
				});
			},
			messages::MessagesStreamEvent::MessageStop => {
				log.update(|r| {
					if r.response.output_messages.is_none() {
						r.response.output_messages = tool_calls.take_output_messages(None);
					}
				});
			},
			messages::MessagesStreamEvent::ContentBlockStop { .. }
			| messages::MessagesStreamEvent::Ping => {},
			messages::MessagesStreamEvent::Error { .. } => {},
		}
	})
}
