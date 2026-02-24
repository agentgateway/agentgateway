use std::time::Instant;

use agent_core::strng;

use crate::http::Body;
use crate::llm::AmendOnDrop;
use crate::llm::types::completions::typed as completions;
use crate::llm::types::messages::typed as messages;
use crate::parse;

pub mod from_completions {
	use std::collections::HashMap;
	use std::time::Instant;

	use agent_core::strng;
	use bytes::Bytes;

	use crate::http::Body;
	use crate::llm::types::ResponseType;
	use crate::llm::types::completions::typed as completions;
	use crate::llm::types::completions::typed::UsagePromptDetails;
	use crate::llm::types::messages::typed as messages;
	use crate::llm::{AIError, AmendOnDrop, types};

	use crate::{json, parse};

	/// translate an OpenAI completions request to an anthropic messages request
	pub fn translate(req: &types::completions::Request) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, completions::Request>(req).map_err(AIError::RequestMarshal)?;
		let model_id = typed.model.clone().unwrap_or_default();
		let xlated = translate_internal(typed, model_id);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	fn translate_internal(req: completions::Request, model_id: String) -> messages::Request {
		let max_tokens = req.max_tokens();
		let stop_sequences = req.stop_sequence();
		// Anthropic has all system prompts in a single field. Join them
		let system = req
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

		// Convert messages to Anthropic format
		let messages = req
			.messages
			.iter()
			.filter(|msg| completions::message_role(msg) != completions::SYSTEM_ROLE)
			.filter_map(|msg| {
				let role = match completions::message_role(msg) {
					completions::ASSISTANT_ROLE => messages::Role::Assistant,
					// Default to user for other roles
					_ => messages::Role::User,
				};

				completions::message_text(msg)
					.map(|s| {
						vec![messages::ContentBlock::Text(messages::ContentTextBlock {
							text: s.to_string(),
							citations: None,
							cache_control: None,
						})]
					})
					.map(|content| messages::Message { role, content })
			})
			.collect();

		let tools = if let Some(tools) = req.tools {
			let mapped_tools: Vec<_> = tools
				.iter()
				.filter_map(|tool| match tool {
					completions::Tool::Function(function_tool) => Some(messages::Tool {
						name: function_tool.function.name.clone(),
						description: function_tool.function.description.clone(),
						input_schema: function_tool
							.function
							.parameters
							.clone()
							.unwrap_or_default(),
						cache_control: None,
					}),
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

		let tool_choice = match req.tool_choice {
			Some(completions::ToolChoiceOption::Function(completions::NamedToolChoice { function })) => {
				Some(messages::ToolChoice::Tool {
					name: function.name,
				})
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto)) => {
				Some(messages::ToolChoice::Auto)
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required)) => {
				Some(messages::ToolChoice::Any)
			},
			Some(completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None)) => {
				Some(messages::ToolChoice::None)
			},
			_ => None,
		};
		let explicit_thinking_budget = req.vendor_extensions.thinking_budget_tokens;
		let thinking = if let Some(budget_tokens) = explicit_thinking_budget {
			Some(messages::ThinkingInput::Enabled { budget_tokens })
		} else {
			req
				.reasoning_effort
				.as_ref()
				.and_then(reasoning_effort_to_enabled_budget)
				.map(|budget_tokens| messages::ThinkingInput::Enabled { budget_tokens })
		};

		let response_format = match req.response_format {
			Some(completions::ResponseFormat::JsonSchema { json_schema }) => json_schema
				.schema
				.map(|schema| messages::OutputFormat::JsonSchema { schema }),
			Some(completions::ResponseFormat::JsonObject) => Some(messages::OutputFormat::JsonSchema {
				schema: serde_json::json!({
					"type": "object",
					"additionalProperties": true
				}),
			}),
			Some(completions::ResponseFormat::Text) | None => None,
		};
		let output_config = if response_format.is_some() {
			Some(messages::OutputConfig {
				effort: None,
				format: response_format,
			})
		} else {
			None
		};
		messages::Request {
			messages,
			system: if system.is_empty() {
				None
			} else {
				Some(messages::SystemPrompt::Text(system))
			},
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

	fn reasoning_effort_to_enabled_budget(effort: &completions::ReasoningEffort) -> Option<u64> {
		match effort {
			completions::ReasoningEffort::None => None,
			completions::ReasoningEffort::Minimal | completions::ReasoningEffort::Low => Some(1024),
			completions::ReasoningEffort::Medium => Some(2048),
			completions::ReasoningEffort::High | completions::ReasoningEffort::Xhigh => Some(4096),
		}
	}

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<messages::MessagesResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
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
					content = Some(text.clone())
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
			extra: None,
		};
		let finish_reason = resp.stop_reason.as_ref().map(super::translate_stop_reason);
		// Only one choice for anthropic
		let choice = completions::ChatChoice {
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
			prompt_tokens_details: resp
				.usage
				.cache_read_input_tokens
				.map(|i| UsagePromptDetails {
					cached_tokens: Some(i as u64),
				}),
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
			service_tier: None,
			system_fingerprint: None,
		}
	}

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		let res = serde_json::from_slice::<messages::MessagesErrorResponse>(bytes)
			.map_err(AIError::ResponseMarshal)?;
		let m = completions::ChatCompletionErrorResponse {
			event_id: None,
			error: completions::ChatCompletionError {
				r#type: "invalid_request_error".to_string(),
				message: res.error.message,
				param: None,
				code: None,
				event_id: None,
			},
		};
		Ok(Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
	}

	pub fn translate_stream(b: Body, buffer_limit: usize, log: AmendOnDrop) -> Body {
		let mut message_id = None;
		let mut model = String::new();
		let created = chrono::Utc::now().timestamp() as u32;
		// let mut finish_reason = None;
		let mut input_tokens = 0;
		let mut saw_token = false;
		// https://docs.anthropic.com/en/docs/build-with-claude/streaming
		parse::sse::json_transform::<messages::MessagesStreamEvent, completions::StreamResponse>(
			b,
			buffer_limit,
			move |f| {
				let mk = |choices: Vec<completions::ChatChoiceStream>,
				          usage: Option<completions::Usage>| {
					Some(completions::StreamResponse {
						id: message_id.clone().unwrap_or_else(|| "unknown".to_string()),
						model: model.clone(),
						object: "chat.completion.chunk".to_string(),
						system_fingerprint: None,
						service_tier: None,
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
						model = message.model.clone();
						input_tokens = message.usage.input_tokens;
						log.non_atomic_mutate(|r| {
							r.response.output_tokens = Some(message.usage.output_tokens as u64);
							r.response.input_tokens = Some(message.usage.input_tokens as u64);
							r.response.cached_input_tokens =
								message.usage.cache_read_input_tokens.map(|i| i as u64);
							r.response.cache_creation_input_tokens =
								message.usage.cache_creation_input_tokens.map(|i| i as u64);
							r.response.provider_model = Some(strng::new(&message.model))
						});
						// no need to respond with anything yet
						None
					},

					messages::MessagesStreamEvent::ContentBlockStart { .. } => {
						// There is never(?) any content here
						None
					},
					messages::MessagesStreamEvent::ContentBlockDelta { delta, .. } => {
						if !saw_token {
							saw_token = true;
							log.non_atomic_mutate(|r| {
								r.response.first_token = Some(Instant::now());
							});
						}
						let mut dr = completions::StreamResponseDelta::default();
						match delta {
							messages::ContentBlockDelta::TextDelta { text } => {
								dr.content = Some(text);
							},
							messages::ContentBlockDelta::ThinkingDelta { thinking } => {
								dr.reasoning_content = Some(thinking)
							},
							// TODO
							messages::ContentBlockDelta::InputJsonDelta { .. } => {},
							messages::ContentBlockDelta::SignatureDelta { .. } => {},
							messages::ContentBlockDelta::CitationsDelta { .. } => {},
						};
						let choice = completions::ChatChoiceStream {
							index: 0,
							logprobs: None,
							delta: dr,
							finish_reason: None,
						};
						mk(vec![choice], None)
					},
					messages::MessagesStreamEvent::MessageDelta { usage, delta: _ } => {
						// TODO
						// finish_reason = delta.stop_reason.as_ref().map(translate_stop_reason);
						log.non_atomic_mutate(|r| {
							r.response.cached_input_tokens = usage.cache_read_input_tokens.map(|i| i as u64);
							r.response.cache_creation_input_tokens =
								usage.cache_creation_input_tokens.map(|i| i as u64);
							r.response.output_tokens = Some(usage.output_tokens as u64);
							if let Some(inp) = r.response.input_tokens {
								r.response.total_tokens = Some(inp + usage.output_tokens as u64)
							}
						});
						mk(
							vec![],
							Some(completions::Usage {
								prompt_tokens: input_tokens as u32,
								completion_tokens: usage.output_tokens as u32,

								total_tokens: (input_tokens + usage.output_tokens) as u32,

								cache_read_input_tokens: usage.cache_read_input_tokens.map(|i| i as u64),
								prompt_tokens_details: usage.cache_read_input_tokens.map(|i| UsagePromptDetails {
									cached_tokens: Some(i as u64),
								}),
								cache_creation_input_tokens: usage.cache_creation_input_tokens.map(|i| i as u64),

								completion_tokens_details: None,
							}),
						)
					},
					messages::MessagesStreamEvent::ContentBlockStop { .. } => None,
					messages::MessagesStreamEvent::MessageStop => None,
					messages::MessagesStreamEvent::Ping => None,
				}
			},
		)
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

pub fn passthrough_stream(b: Body, buffer_limit: usize, log: AmendOnDrop) -> Body {
	let mut saw_token = false;
	// https://platform.claude.com/docs/en/build-with-claude/streaming
	parse::sse::json_passthrough::<messages::MessagesStreamEvent>(b, buffer_limit, move |f| {
		// ignore errors... what else can we do?
		let Some(Ok(f)) = f else { return };

		// Extract info we need
		match f {
			messages::MessagesStreamEvent::MessageStart { message } => {
				log.non_atomic_mutate(|r| {
					r.response.output_tokens = Some(message.usage.output_tokens as u64);
					r.response.input_tokens = Some(message.usage.input_tokens as u64);
					r.response.cached_input_tokens = message.usage.cache_read_input_tokens.map(|i| i as u64);
					r.response.cache_creation_input_tokens =
						message.usage.cache_creation_input_tokens.map(|i| i as u64);
					r.response.provider_model = Some(strng::new(&message.model))
				});
			},
			messages::MessagesStreamEvent::ContentBlockDelta { .. } => {
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}
			},
			messages::MessagesStreamEvent::MessageDelta { usage, delta: _ } => {
				log.non_atomic_mutate(|r| {
					r.response.output_tokens = Some(usage.output_tokens as u64);
					r.response.cached_input_tokens = usage.cache_read_input_tokens.map(|i| i as u64);
					r.response.cache_creation_input_tokens =
						usage.cache_creation_input_tokens.map(|i| i as u64);
					if let Some(inp) = r.response.input_tokens {
						r.response.total_tokens = Some(inp + usage.output_tokens as u64)
					}
				});
			},
			messages::MessagesStreamEvent::ContentBlockStart { .. }
			| messages::MessagesStreamEvent::ContentBlockStop { .. }
			| messages::MessagesStreamEvent::MessageStop
			| messages::MessagesStreamEvent::Ping => {},
		}
	})
}
