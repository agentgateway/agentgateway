use std::time::Instant;

use agent_core::strng;
use tracing::debug;

use crate::http::Response;
use crate::llm::{AmendOnDrop, types};
use crate::parse;

pub mod from_messages {
	use itertools::Itertools;
	use messages::{ContentBlock, ToolResultContent, ToolResultContentPart};
	use types::completions::typed as completions;
	use types::messages::typed as messages;

	use crate::json;
	use crate::llm::{AIError, types};

	/// translate an Anthropic messages to an OpenAI completions request
	pub fn translate(req: &types::messages::Request) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, messages::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	fn translate_internal(req: messages::Request) -> completions::Request {
		let messages::Request {
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
			output_config,
		} = req;

		let adaptive_thinking_requested = thinking
			.as_ref()
			.is_some_and(|t| matches!(t, messages::ThinkingInput::Adaptive {}));
		let output_effort = output_config.as_ref().and_then(|cfg| cfg.effort);
		let reasoning_effort = if adaptive_thinking_requested {
			Some(match output_effort {
				Some(messages::ThinkingEffort::Low) => completions::ReasoningEffort::Low,
				Some(messages::ThinkingEffort::Medium) => completions::ReasoningEffort::Medium,
				Some(messages::ThinkingEffort::High) => completions::ReasoningEffort::High,
				Some(messages::ThinkingEffort::Max) => completions::ReasoningEffort::Xhigh,
				// Anthropic adaptive thinking defaults to high effort when omitted.
				None => completions::ReasoningEffort::High,
			})
		} else {
			None
		};
		let response_format = output_config
			.as_ref()
			.and_then(|cfg| cfg.format.as_ref())
			.map(|format| match format {
				messages::OutputFormat::JsonSchema { schema } => completions::ResponseFormat::JsonSchema {
					json_schema: completions::ResponseFormatJsonSchema {
						description: None,
						name: "structured_output".to_string(),
						schema: Some(schema.clone()),
						strict: None,
					},
				},
			});

		let mut msgs: Vec<completions::RequestMessage> = Vec::new();

		// Handle the system prompt (convert both string and block formats to string)
		if let Some(system) = system {
			let system_text = match system {
				messages::SystemPrompt::Text(text) => text,
				messages::SystemPrompt::Blocks(blocks) => {
					// Join all text blocks into a single string
					blocks
						.into_iter()
						.map(|block| match block {
							messages::SystemContentBlock::Text { text, .. } => text,
						})
						.collect::<Vec<_>>()
						.join("\n")
				},
			};
			msgs.push(completions::RequestMessage::System(
				completions::RequestSystemMessage::from(system_text),
			));
		}

		// Convert messages from Anthropic to universal format
		for msg in messages {
			match msg.role {
				messages::Role::User => {
					let mut user_text = String::new();
					for block in msg.content {
						match block {
							messages::ContentBlock::Text(messages::ContentTextBlock { text, .. }) => {
								if !user_text.is_empty() {
									user_text.push('\n');
								}
								user_text.push_str(&text);
							},
							messages::ContentBlock::ToolResult {
								tool_use_id,
								content,
								..
							} => {
								msgs.push(
									completions::RequestToolMessage {
										tool_call_id: tool_use_id,
										content: match content {
											ToolResultContent::Text(t) => t.into(),
											ToolResultContent::Array(parts) => {
												completions::RequestToolMessageContent::Array(
													parts
														.into_iter()
														.filter_map(|p| match p {
															ToolResultContentPart::Text { text, .. } => Some(
																completions::RequestToolMessageContentPart::Text(text.into()),
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
							messages::ContentBlock::Image { .. } => {}, /* Image content is not directly supported in universal::Message::User in this form. */
							// This would require a different content format not represented here.
							// ToolUse blocks are expected from assistants, not users.
							messages::ContentBlock::ServerToolUse { .. }
							| messages::ContentBlock::ToolUse { .. } => {}, /* ToolUse blocks are expected from assistants, not users. */

							// Other content block types are not expected from the user in a request.
							_ => {},
						}
					}
					if !user_text.is_empty() {
						msgs.push(
							completions::RequestUserMessage {
								content: user_text.into(),
								name: None,
							}
							.into(),
						);
					}
				},
				messages::Role::Assistant => {
					let mut assistant_text = None;
					let mut tool_calls: Vec<completions::MessageToolCalls> = Vec::new();
					for block in msg.content {
						match block {
							messages::ContentBlock::Text(messages::ContentTextBlock { text, .. }) => {
								assistant_text = Some(text);
							},
							messages::ContentBlock::ToolUse {
								id, name, input, ..
							} => {
								tool_calls.push(completions::MessageToolCalls::Function(
									completions::MessageToolCall {
										id,
										function: completions::FunctionCall {
											name,
											// It's assumed that the input is a JSON object that can be stringified.
											arguments: serde_json::to_string(&input).unwrap_or_default(),
										},
									},
								));
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
							completions::RequestAssistantMessage {
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
			.map(|tool| {
				completions::Tool::Function(completions::FunctionTool {
					function: completions::FunctionObject {
						name: tool.name,
						description: tool.description,
						parameters: Some(tool.input_schema),
						strict: None,
					},
				})
			})
			.collect_vec();
		let tool_choice = tool_choice.map(|choice| match choice {
			messages::ToolChoice::Auto => {
				completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto)
			},
			messages::ToolChoice::Any => {
				completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required)
			},
			messages::ToolChoice::Tool { name } => {
				completions::ToolChoiceOption::Function(completions::NamedToolChoice {
					function: completions::FunctionName { name },
				})
			},
			messages::ToolChoice::None => {
				completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None)
			},
		});

		// Carry request-scoped metadata forward for downstream guardrail providers (e.g. Bedrock Guardrails,
		// Model Armor) while still mapping the common `user_id` to OpenAI's `user` field.
		//
		// `messages.metadata.fields` is a string map, which fits cleanly into OpenAI's free-form `metadata` object.
		let user_id = metadata
			.as_ref()
			.and_then(|m| m.fields.get("user_id").cloned());
		let metadata_json = metadata.and_then(|m| {
			serde_json::to_value(m.fields)
				.ok()
				.filter(|v| !matches!(v, serde_json::Value::Object(map) if map.is_empty()))
		});

		completions::Request {
			model: Some(model),
			messages: msgs,
			stream: Some(stream),
			temperature,
			top_p,
			max_completion_tokens: Some(max_tokens as u32),
			stop: if stop_sequences.is_empty() {
				None
			} else {
				Some(completions::Stop::StringArray(stop_sequences))
			},
			tools: if tools.is_empty() { None } else { Some(tools) },
			tool_choice,
			parallel_tool_calls: None,
			user: user_id,

			vendor_extensions: completions::RequestVendorExtensions {
				top_k,
				thinking_budget_tokens: thinking.as_ref().and_then(|t| match t {
					messages::ThinkingInput::Enabled { budget_tokens } => Some(*budget_tokens),
					_ => None,
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
			response_format,
			seed: None,
			#[allow(deprecated)]
			function_call: None,
			#[allow(deprecated)]
			functions: None,
			metadata: metadata_json,
			#[allow(deprecated)]
			max_tokens: None,
			service_tier: None,
			web_search_options: None,
			stream_options: None,
			store: None,
			reasoning_effort,
		}
	}
}

pub fn passthrough_stream(
	mut log: AmendOnDrop,
	include_completion_in_log: bool,
	resp: Response,
) -> Response {
	let mut completion = include_completion_in_log.then(String::new);
	let buffer_limit = crate::http::response_buffer_limit(&resp);
	resp.map(|b| {
		let mut seen_provider = false;
		let mut saw_token = false;
		parse::sse::json_passthrough::<types::completions::typed::StreamResponse>(
			b,
			buffer_limit,
			move |f| {
				match f {
					Some(Ok(f)) => {
						if let Some(c) = completion.as_mut()
							&& let Some(delta) = f.choices.first().and_then(|c| c.delta.content.as_deref())
						{
							c.push_str(delta);
						}
						if !saw_token {
							saw_token = true;
							log.non_atomic_mutate(|r| {
								r.response.first_token = Some(Instant::now());
							});
						}
						if !seen_provider {
							seen_provider = true;
							log.non_atomic_mutate(|r| r.response.provider_model = Some(strng::new(&f.model)));
						}
						if let Some(u) = f.usage {
							log.non_atomic_mutate(|r| {
								r.response.input_tokens = Some(u.prompt_tokens as u64);
								r.response.output_tokens = Some(u.completion_tokens as u64);
								r.response.total_tokens = Some(u.total_tokens as u64);
								r.response.cached_input_tokens = u
									.prompt_tokens_details
									.as_ref()
									.and_then(|d| d.cached_tokens);
								r.response.reasoning_tokens = u
									.completion_tokens_details
									.as_ref()
									.and_then(|d| d.reasoning_tokens);
								if let Some(c) = completion.take() {
									r.response.completion = Some(vec![c]);
								}
							});

							log.report_rate_limit();
						}
					},
					Some(Err(e)) => {
						debug!("failed to parse streaming response: {e}");
					},
					None => {
						// We are done, try to set completion if we haven't already
						// This is useful in case we never see "usage"
						log.non_atomic_mutate(|r| {
							if let Some(c) = completion.take() {
								r.response.completion = Some(vec![c]);
							}
						});
					},
				}
			},
		)
	})
}
