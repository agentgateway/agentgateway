use crate::llm::types::completions::typed as completions;
use crate::llm::types::messages::typed as messages;

pub mod from_completions {
	use crate::json;
	use crate::llm::types::ResponseType;
	use crate::llm::types::completions::typed as completions;
	use crate::llm::types::messages::typed as messages;
	use crate::llm::{AIError, types};
	use bytes::Bytes;
	use std::collections::HashMap;

	/// translate an OpenAI completions request to an anthropic messages request
	pub fn translate(req: &types::completions::Request) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, completions::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	fn translate_internal(req: completions::Request) -> messages::Request {
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
				.map(|tool| messages::Tool {
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
		let metadata = req.user.map(|user| messages::Metadata {
			fields: HashMap::from([("user_id".to_string(), user)]),
		});

		let tool_choice = match req.tool_choice {
			Some(completions::ToolChoiceOption::Named(completions::NamedToolChoice {
				r#type: _,
				function,
			})) => Some(messages::ToolChoice::Tool {
				name: function.name,
			}),
			Some(completions::ToolChoiceOption::Auto) => Some(messages::ToolChoice::Auto),
			Some(completions::ToolChoiceOption::Required) => Some(messages::ToolChoice::Any),
			Some(completions::ToolChoiceOption::None) => Some(messages::ToolChoice::None),
			None => None,
		};
		let thinking = if let Some(budget) = req.vendor_extensions.thinking_budget_tokens {
			Some(messages::ThinkingInput::Enabled {
				budget_tokens: budget,
			})
		} else {
			match &req.reasoning_effort {
				// Arbitrary constants come from LiteLLM defaults.
				// OpenRouter uses percentages which may be more appropriate though (https://openrouter.ai/docs/use-cases/reasoning-tokens#reasoning-effort-level)
				// Note: Anthropic's minimum budget_tokens is 1024
				Some(completions::ReasoningEffort::Minimal) | Some(completions::ReasoningEffort::Low) => {
					Some(messages::ThinkingInput::Enabled {
						budget_tokens: 1024,
					})
				},
				Some(completions::ReasoningEffort::Medium) => Some(messages::ThinkingInput::Enabled {
					budget_tokens: 2048,
				}),
				Some(completions::ReasoningEffort::High) => Some(messages::ThinkingInput::Enabled {
					budget_tokens: 4096,
				}),
				None => None,
			}
		};
		messages::Request {
			messages,
			system: if system.is_empty() {
				None
			} else {
				Some(messages::SystemPrompt::Text(system))
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
		let mut tool_calls: Vec<completions::MessageToolCall> = Vec::new();
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
					tool_calls.push(completions::MessageToolCall {
						id: id.clone(),
						r#type: completions::ToolType::Function,
						function: completions::FunctionCall {
							name: name.clone(),
							arguments: args,
						},
					});
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
			prompt_tokens_details: None,
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
		Ok(Bytes::from(serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?))
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
