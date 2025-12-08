use agent_core::prelude::Strng;
use agent_core::strng;
use async_openai::types::{
	ChatCompletionRequestToolMessageContent, ChatCompletionRequestToolMessageContentPart,
	FinishReason, ReasoningEffort,
};
use bytes::Bytes;
use chrono;
use itertools::Itertools;

use crate::http::{Body, Response};
use crate::llm::anthropic::types::{
	ContentBlock, ContentBlockDelta, MessagesErrorResponse, MessagesRequest, MessagesResponse,
	MessagesStreamEvent, StopReason, ThinkingInput, ToolResultContent, ToolResultContentPart,
};
use crate::llm::{AIError, InputFormat, LLMInfo};
use crate::telemetry::log::AsyncLog;
use crate::{parse, *};
use crate::llm::types::ResponseType;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("anthropic");
}
pub const DEFAULT_HOST_STR: &str = "api.anthropic.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);
pub const DEFAULT_PATH: &str = "/v1/messages";

impl Provider {
	pub async fn process_streaming(
		&self,
		log: AsyncLog<LLMInfo>,
		resp: Response,
		input_format: InputFormat,
	) -> Response {
		let buffer = http::response_buffer_limit(&resp);
		match input_format {
			InputFormat::Completions => resp.map(|b| translate_stream(b, buffer, log)),
			InputFormat::Messages => resp.map(|b| passthrough_stream(b, buffer, log)),
			InputFormat::Responses => {
				unreachable!("Responses format should not be routed to Anthropic provider")
			},
			InputFormat::CountTokens => {
				unreachable!("CountTokens should be handled by process_count_tokens_response")
			},
		}
	}
}

pub fn process_error(bytes: &Bytes) -> Result<llm::types::completions::typed::ChatCompletionErrorResponse, AIError> {
	let resp =
		serde_json::from_slice::<MessagesErrorResponse>(bytes).map_err(AIError::ResponseParsing)?;
	translate_error(resp)
}

pub fn process_response(
	bytes: &Bytes,
	input_format: InputFormat,
) -> Result<Box<dyn ResponseType>, AIError> {
	match input_format {
		InputFormat::Completions => {
			let resp =
				serde_json::from_slice::<MessagesResponse>(bytes).map_err(AIError::ResponseParsing)?;
			let openai = translate_response(resp);
			let passthrough = json::convert::<_, universal::passthrough::Response>(&openai)
				.map_err(AIError::ResponseParsing)?;
			Ok(Box::new(passthrough))
		},
		InputFormat::Messages => {
			let resp =
				serde_json::from_slice::<passthrough::Response>(bytes).map_err(AIError::ResponseParsing)?;

			Ok(Box::new(resp))
		},
		InputFormat::Responses => {
			unreachable!("Responses format should not be routed to Anthropic provider")
		},
		InputFormat::CountTokens => {
			unreachable!("CountTokens should be handled by process_count_tokens_response")
		},
	}
}

pub(super) fn translate_error(
	resp: MessagesErrorResponse,
) -> Result<universal::ChatCompletionErrorResponse, AIError> {
	Ok(universal::ChatCompletionErrorResponse {
		event_id: None,
		error: universal::ChatCompletionError {
			r#type: "invalid_request_error".to_string(),
			message: resp.error.message,
			param: None,
			code: None,
			event_id: None,
		},
	})
}

pub(super) fn translate_response(resp: MessagesResponse) -> universal::Response {
	// Convert Anthropic content blocks to OpenAI message content
	let mut tool_calls: Vec<universal::MessageToolCall> = Vec::new();
	let mut content = None;
	let mut reasoning_content = None;
	for block in resp.content {
		match block {
			types::ContentBlock::Text(types::ContentTextBlock { text, .. }) => {
				content = Some(text.clone())
			},
			ContentBlock::ToolUse {
				id, name, input, ..
			}
			| ContentBlock::ServerToolUse {
				id, name, input, ..
			} => {
				let Some(args) = serde_json::to_string(&input).ok() else {
					continue;
				};
				tool_calls.push(universal::MessageToolCall {
					id: id.clone(),
					r#type: universal::ToolType::Function,
					function: universal::FunctionCall {
						name: name.clone(),
						arguments: args,
					},
				});
			},
			ContentBlock::ToolResult { .. } => {
				// Should be on the request path, not the response path
				continue;
			},
			// For now we ignore Redacted and signature think through a better approach as this may be needed
			ContentBlock::Thinking { thinking, .. } => {
				reasoning_content = Some(thinking);
			},
			ContentBlock::RedactedThinking { .. } => {},

			// not currently supported
			types::ContentBlock::Image { .. } => continue,
			ContentBlock::Document(_) => continue,
			ContentBlock::SearchResult(_) => continue,
			ContentBlock::WebSearchToolResult { .. } => continue,
			ContentBlock::Unknown => continue,
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
		reasoning_content,
		extra: None,
	};
	let finish_reason = resp.stop_reason.as_ref().map(translate_stop_reason);
	// Only one choice for anthropic
	let choice = universal::ChatChoice {
		index: 0,
		message,
		finish_reason,
		logprobs: None,
	};

	let choices = vec![choice];
	// Convert usage from Anthropic format to OpenAI format
	let usage = universal::Usage {
		prompt_tokens: resp.usage.input_tokens as u32,
		completion_tokens: resp.usage.output_tokens as u32,
		total_tokens: (resp.usage.input_tokens + resp.usage.output_tokens) as u32,
		prompt_tokens_details: None,
		completion_tokens_details: None,
	};

	universal::Response {
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

pub(super) fn translate_request(req: universal::Request) -> types::MessagesRequest {
	let max_tokens = universal::max_tokens(&req);
	let stop_sequences = universal::stop_sequence(&req);
	// Anthropic has all system prompts in a single field. Join them
	let system = req
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

	// Convert messages to Anthropic format
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
				.map(|s| {
					vec![types::ContentBlock::Text(types::ContentTextBlock {
						text: s.to_string(),
						citations: None,
						cache_control: None,
					})]
				})
				.map(|content| types::Message { role, content })
		})
		.collect();

	let tools = if let Some(tools) = req.tools {
		let mapped_tools: Vec<_> = tools
			.iter()
			.map(|tool| types::Tool {
				name: tool.function.name.clone(),
				description: tool.function.description.clone(),
				input_schema: tool.function.parameters.clone().unwrap_or_default(),
				cache_control: None,
			})
			.collect();
		Some(mapped_tools)
	} else {
		None
	};
	let metadata = req.user.map(|user| types::Metadata {
		fields: HashMap::from([("user_id".to_string(), user)]),
	});

	let tool_choice = match req.tool_choice {
		Some(universal::ToolChoiceOption::Named(universal::NamedToolChoice {
			r#type: _,
			function,
		})) => Some(types::ToolChoice::Tool {
			name: function.name,
		}),
		Some(universal::ToolChoiceOption::Auto) => Some(types::ToolChoice::Auto),
		Some(universal::ToolChoiceOption::Required) => Some(types::ToolChoice::Any),
		Some(universal::ToolChoiceOption::None) => Some(types::ToolChoice::None),
		None => None,
	};
	let thinking = if let Some(budget) = req.vendor_extensions.thinking_budget_tokens {
		Some(types::ThinkingInput::Enabled {
			budget_tokens: budget,
		})
	} else {
		match &req.reasoning_effort {
			// Arbitrary constants come from LiteLLM defaults.
			// OpenRouter uses percentages which may be more appropriate though (https://openrouter.ai/docs/use-cases/reasoning-tokens#reasoning-effort-level)
			// Note: Anthropic's minimum budget_tokens is 1024
			Some(ReasoningEffort::Minimal) | Some(ReasoningEffort::Low) => {
				Some(types::ThinkingInput::Enabled {
					budget_tokens: 1024,
				})
			},
			Some(ReasoningEffort::Medium) => Some(types::ThinkingInput::Enabled {
				budget_tokens: 2048,
			}),
			Some(ReasoningEffort::High) => Some(types::ThinkingInput::Enabled {
				budget_tokens: 4096,
			}),
			None => None,
		}
	};
	types::MessagesRequest {
		messages,
		system: if system.is_empty() {
			None
		} else {
			Some(types::SystemPrompt::Text(system))
		},
		model: req.model.unwrap_or_default(),
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
	}
}

pub(super) fn translate_stream(b: Body, buffer_limit: usize, log: AsyncLog<LLMInfo>) -> Body {
	let mut message_id = None;
	let mut model = String::new();
	let created = chrono::Utc::now().timestamp() as u32;
	// let mut finish_reason = None;
	let mut input_tokens = 0;
	let mut saw_token = false;
	// https://docs.anthropic.com/en/docs/build-with-claude/streaming
	parse::sse::json_transform::<MessagesStreamEvent, universal::StreamResponse>(
		b,
		buffer_limit,
		move |f| {
			let mk = |choices: Vec<universal::ChatChoiceStream>, usage: Option<universal::Usage>| {
				Some(universal::StreamResponse {
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
				MessagesStreamEvent::MessageStart { message } => {
					message_id = Some(message.id);
					model = message.model.clone();
					input_tokens = message.usage.input_tokens;
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(message.usage.output_tokens as u64);
						r.response.input_tokens = Some(message.usage.input_tokens as u64);
						r.response.provider_model = Some(strng::new(&message.model))
					});
					// no need to respond with anything yet
					None
				},

				MessagesStreamEvent::ContentBlockStart { .. } => {
					// There is never(?) any content here
					None
				},
				MessagesStreamEvent::ContentBlockDelta { delta, .. } => {
					if !saw_token {
						saw_token = true;
						log.non_atomic_mutate(|r| {
							r.response.first_token = Some(Instant::now());
						});
					}
					let mut dr = universal::StreamResponseDelta::default();
					match delta {
						ContentBlockDelta::TextDelta { text } => {
							dr.content = Some(text);
						},
						ContentBlockDelta::ThinkingDelta { thinking } => dr.reasoning_content = Some(thinking),
						// TODO
						ContentBlockDelta::InputJsonDelta { .. } => {},
						ContentBlockDelta::SignatureDelta { .. } => {},
						ContentBlockDelta::CitationsDelta { .. } => {},
					};
					let choice = universal::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta: dr,
						finish_reason: None,
					};
					mk(vec![choice], None)
				},
				MessagesStreamEvent::MessageDelta { usage, delta: _ } => {
					// TODO
					// finish_reason = delta.stop_reason.as_ref().map(translate_stop_reason);
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(usage.output_tokens as u64);
						if let Some(inp) = r.response.input_tokens {
							r.response.total_tokens = Some(inp + usage.output_tokens as u64)
						}
					});
					mk(
						vec![],
						Some(universal::Usage {
							prompt_tokens: input_tokens as u32,
							completion_tokens: usage.output_tokens as u32,

							total_tokens: (input_tokens + usage.output_tokens) as u32,

							prompt_tokens_details: None,
							completion_tokens_details: None,
						}),
					)
				},
				MessagesStreamEvent::ContentBlockStop { .. } => None,
				MessagesStreamEvent::MessageStop => None,
				MessagesStreamEvent::Ping => None,
			}
		},
	)
}

pub(super) fn passthrough_stream(b: Body, buffer_limit: usize, log: AsyncLog<LLMInfo>) -> Body {
	let mut saw_token = false;
	// https://docs.anthropic.com/en/docs/build-with-claude/streaming
	parse::sse::json_passthrough::<MessagesStreamEvent>(b, buffer_limit, move |f| {
		// ignore errors... what else can we do?
		let Some(Ok(f)) = f else { return };

		// Extract info we need
		match f {
			MessagesStreamEvent::MessageStart { message } => {
				log.non_atomic_mutate(|r| {
					r.response.output_tokens = Some(message.usage.output_tokens as u64);
					r.response.input_tokens = Some(message.usage.input_tokens as u64);
					r.response.provider_model = Some(strng::new(&message.model))
				});
			},
			MessagesStreamEvent::ContentBlockDelta { .. } => {
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}
			},
			MessagesStreamEvent::MessageDelta { usage, delta: _ } => {
				log.non_atomic_mutate(|r| {
					r.response.output_tokens = Some(usage.output_tokens as u64);
					if let Some(inp) = r.response.input_tokens {
						r.response.total_tokens = Some(inp + usage.output_tokens as u64)
					}
				});
			},
			MessagesStreamEvent::ContentBlockStart { .. }
			| MessagesStreamEvent::ContentBlockStop { .. }
			| MessagesStreamEvent::MessageStop
			| MessagesStreamEvent::Ping => {},
		}
	})
}

pub(super) fn translate_anthropic_response(_req: universal::Response) -> types::MessagesResponse {
	// TODO: implement this
	types::MessagesResponse {
		id: "".to_string(),
		r#type: "".to_string(),
		role: Default::default(),
		content: vec![],
		model: "".to_string(),
		stop_reason: None,
		stop_sequence: None,
		usage: types::Usage {
			input_tokens: 0,
			output_tokens: 0,
			cache_creation_input_tokens: None,
			cache_read_input_tokens: None,
		},
	}
}
pub(super) fn translate_anthropic_request(req: types::MessagesRequest) -> universal::Request {
	let types::MessagesRequest {
		messages,
		system,
		model,
		max_tokens,
		stop_sequences,
		stream,
		temperature,
		top_p,
		top_k,
		tools,
		tool_choice,
		metadata,
		thinking,
	} = req;
	let mut msgs: Vec<universal::RequestMessage> = Vec::new();

	// Handle the system prompt (convert both string and block formats to string)
	if let Some(system) = system {
		let system_text = match system {
			types::SystemPrompt::Text(text) => text,
			types::SystemPrompt::Blocks(blocks) => {
				// Join all text blocks into a single string
				blocks
					.into_iter()
					.map(|block| match block {
						types::SystemContentBlock::Text { text, .. } => text,
					})
					.collect::<Vec<_>>()
					.join("\n")
			},
		};
		msgs.push(universal::RequestMessage::System(
			RequestSystemMessage::from(system_text),
		));
	}

	// Convert messages from Anthropic to universal format
	for msg in messages {
		match msg.role {
			types::Role::User => {
				let mut user_text = String::new();
				for block in msg.content {
					match block {
						types::ContentBlock::Text(types::ContentTextBlock { text, .. }) => {
							if !user_text.is_empty() {
								user_text.push('\n');
							}
							user_text.push_str(&text);
						},
						types::ContentBlock::ToolResult {
							tool_use_id,
							content,
							..
						} => {
							msgs.push(
								universal::RequestToolMessage {
									tool_call_id: tool_use_id,
									content: match content {
										ToolResultContent::Text(t) => t.into(),
										ToolResultContent::Array(parts) => {
											ChatCompletionRequestToolMessageContent::Array(
												parts
													.into_iter()
													.filter_map(|p| match p {
														ToolResultContentPart::Text { text, .. } => Some(
															ChatCompletionRequestToolMessageContentPart::Text(text.into()),
														),
														// Other types are not supported
														_ => None,
													})
													.collect_vec(),
											)
										},
									},
								}
								.into(),
							);
						},
						// Image content is not directly supported in universal::Message::User in this form.
						// This would require a different content format not represented here.
						types::ContentBlock::Image { .. } => {}, /* Image content is not directly supported in universal::Message::User in this form. */
						// This would require a different content format not represented here.
						// ToolUse blocks are expected from assistants, not users.
						types::ContentBlock::ServerToolUse { .. } | types::ContentBlock::ToolUse { .. } => {}, /* ToolUse blocks are expected from assistants, not users. */

						// Other content block types are not expected from the user in a request.
						_ => {},
					}
				}
				if !user_text.is_empty() {
					msgs.push(
						universal::RequestUserMessage {
							content: user_text.into(),
							name: None,
						}
						.into(),
					);
				}
			},
			types::Role::Assistant => {
				let mut assistant_text = None;
				let mut tool_calls = Vec::new();
				for block in msg.content {
					match block {
						types::ContentBlock::Text(types::ContentTextBlock { text, .. }) => {
							assistant_text = Some(text);
						},
						types::ContentBlock::ToolUse {
							id, name, input, ..
						} => {
							tool_calls.push(universal::MessageToolCall {
								id,
								r#type: universal::ToolType::Function,
								function: universal::FunctionCall {
									name,
									// It's assumed that the input is a JSON object that can be stringified.
									arguments: serde_json::to_string(&input).unwrap_or_default(),
								},
							});
						},
						ContentBlock::Thinking { .. } => {
							// TODO
						},
						ContentBlock::RedactedThinking { .. } => {
							// TODO
						},
						// Other content block types are not expected from the assistant in a request.
						_ => {},
					}
				}
				if assistant_text.is_some() || !tool_calls.is_empty() {
					msgs.push(
						universal::RequestAssistantMessage {
							content: assistant_text.map(Into::into),
							tool_calls: if tool_calls.is_empty() {
								None
							} else {
								Some(tool_calls)
							},
							..Default::default()
						}
						.into(),
					);
				}
			},
		}
	}

	let tools = tools
		.into_iter()
		.flat_map(|tools| tools.into_iter())
		.map(|tool| universal::Tool {
			r#type: universal::ToolType::Function,
			function: universal::FunctionObject {
				name: tool.name,
				description: tool.description,
				parameters: Some(tool.input_schema),
				strict: None,
			},
		})
		.collect_vec();
	let tool_choice = tool_choice.map(|choice| match choice {
		types::ToolChoice::Auto => universal::ToolChoiceOption::Auto,
		types::ToolChoice::Any => universal::ToolChoiceOption::Required,
		types::ToolChoice::Tool { name } => {
			universal::ToolChoiceOption::Named(universal::NamedToolChoice {
				r#type: universal::ToolType::Function,
				function: universal::FunctionName { name },
			})
		},
		types::ToolChoice::None => universal::ToolChoiceOption::None,
	});

	universal::Request {
		model: Some(model),
		messages: msgs,
		stream: Some(stream),
		temperature,
		top_p,
		max_completion_tokens: Some(max_tokens as u32),
		stop: if stop_sequences.is_empty() {
			None
		} else {
			Some(universal::Stop::StringArray(stop_sequences))
		},
		tools: if tools.is_empty() { None } else { Some(tools) },
		tool_choice,
		parallel_tool_calls: None,
		user: metadata.and_then(|m| m.fields.get("user_id").cloned()),

		vendor_extensions: RequestVendorExtensions {
			top_k,
			thinking_budget_tokens: thinking.and_then(|t| match t {
				ThinkingInput::Enabled { budget_tokens } => Some(budget_tokens),
				ThinkingInput::Disabled { .. } => None,
			}),
		},

		// The following OpenAI fields are not supported by Anthropic and are set to None:
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
		reasoning_effort: None,
	}
}

fn translate_stop_reason(resp: &types::StopReason) -> FinishReason {
	match resp {
		StopReason::EndTurn => universal::FinishReason::Stop,
		StopReason::MaxTokens => universal::FinishReason::Length,
		StopReason::StopSequence => universal::FinishReason::Stop,
		StopReason::ToolUse => universal::FinishReason::ToolCalls,
		StopReason::Refusal => universal::FinishReason::ContentFilter,
		StopReason::PauseTurn => universal::FinishReason::Stop,
		StopReason::ModelContextWindowExceeded => universal::FinishReason::Length,
	}
}
