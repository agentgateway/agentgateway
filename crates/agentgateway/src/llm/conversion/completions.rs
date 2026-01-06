use std::time::Instant;

use agent_core::strng;
use tracing::debug;

use crate::http::Response;
use crate::llm::{amend_tokens, types};
use crate::store::LLMResponsePolicies;
use crate::telemetry::log::AsyncLog;
use crate::{llm, parse};

pub mod from_messages {
	use itertools::Itertools;
	use messages::{ContentBlock, ThinkingInput, ToolResultContent, ToolResultContentPart};
	use types::completions::typed as completions;
	use types::messages::typed as messages;

	use crate::json;
	use crate::llm::{AIError, types};

	use crate::llm::types::ResponseType;
	use crate::telemetry::log::AsyncLog;
	use agent_core::strng;
	use bytes::Bytes;
	use std::time::Instant;

	/// translate an Anthropic messages to an OpenAI completions request
	pub fn translate(req: &types::messages::Request) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, messages::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp =
			serde_json::from_slice::<completions::Response>(bytes).map_err(AIError::ResponseParsing)?;
		let anthropic = translate_response_internal(resp);
		Ok(Box::new(anthropic))
	}

	fn translate_response_internal(resp: completions::Response) -> types::messages::Response {
		// Anthropic only supports one choice
		let choice = resp
			.choices
			.into_iter()
			.next()
			.unwrap_or(completions::ChatChoice {
				index: 0,
				message: completions::ResponseMessage {
					content: Some("".to_string()),
					refusal: None,
					tool_calls: None,
					role: completions::Role::Assistant,
					#[allow(deprecated)]
					function_call: None,
					audio: None,
					reasoning_content: None,
					extra: None,
				},
				finish_reason: None,
				logprobs: None,
			});

		let content = if let Some(content) = choice.message.content {
			vec![types::messages::Content {
				text: Some(content),
				rest: Default::default(),
			}]
		} else if let Some(tool_calls) = choice.message.tool_calls {
			tool_calls
				.into_iter()
				.filter_map(|tc| match tc {
					completions::MessageToolCalls::Function(f) => {
						let mut rest = serde_json::Map::new();
						rest.insert("type".to_string(), "tool_use".into());
						rest.insert("id".to_string(), f.id.into());
						rest.insert("name".to_string(), f.function.name.into());
						if let Ok(input) = serde_json::from_str::<serde_json::Value>(&f.function.arguments) {
							rest.insert("input".to_string(), input);
						}
						Some(types::messages::Content {
							text: None,
							rest: rest.into(),
						})
					},
					completions::MessageToolCalls::Custom(_) => None,
				})
				.collect()
		} else {
			vec![]
		};

		let stop_reason = choice
			.finish_reason
			.map(|r| match r {
				completions::FinishReason::Stop => "end_turn",
				completions::FinishReason::Length => "max_tokens",
				completions::FinishReason::ToolCalls => "tool_use",
				completions::FinishReason::ContentFilter => "refusal",
				completions::FinishReason::FunctionCall => "tool_use",
			})
			.unwrap_or("end_turn"); // Default?

		types::messages::Response {
			model: resp.model,
			usage: types::messages::Usage {
				input_tokens: resp
					.usage
					.as_ref()
					.map(|u| u.prompt_tokens as u64)
					.unwrap_or(0),
				output_tokens: resp
					.usage
					.as_ref()
					.map(|u| u.completion_tokens as u64)
					.unwrap_or(0),
				rest: Default::default(),
			},
			content,
			rest: {
				let mut map = serde_json::Map::new();
				map.insert("id".to_string(), resp.id.into());
				map.insert("type".to_string(), "message".into());
				map.insert("role".to_string(), "assistant".into());
				map.insert("stop_reason".to_string(), stop_reason.into());
				map.into()
			},
		}
	}

	pub fn translate_stream(
		b: crate::http::Body,
		buffer_limit: usize,
		log: AsyncLog<crate::llm::LLMInfo>,
	) -> crate::http::Body {
		let mut sent_message_start = false;
		let mut sent_content_block_start = false;

		crate::parse::sse::json_transform_multi::<
			completions::StreamResponse,
			messages::MessagesStreamEvent,
			_,
		>(b, buffer_limit, move |f| {
			// We return Vec<T> which implements IntoIterator<Item = T>

			let f = match f {
				Ok(val) => val,
				_ => return Vec::new(),
			};

			let mut events = Vec::new();

			if !sent_message_start {
				sent_message_start = true;
				events.push(messages::MessagesStreamEvent::MessageStart {
					message: messages::MessagesResponse {
						id: f.id.clone(),
						r#type: "message".to_string(),
						role: messages::Role::Assistant,
						content: vec![],
						model: f.model.clone(),
						stop_reason: None,
						stop_sequence: None,
						usage: messages::Usage {
							input_tokens: 0,
							output_tokens: 0,
							cache_creation_input_tokens: None,
							cache_read_input_tokens: None,
						},
					},
				});

				log.non_atomic_mutate(|r| r.response.provider_model = Some(strng::new(&f.model)));
			}

			if let Some(choice) = f.choices.first() {
				if let Some(content) = &choice.delta.content {
					if !sent_content_block_start {
						sent_content_block_start = true;
						events.push(messages::MessagesStreamEvent::ContentBlockStart {
							index: 0,
							content_block: messages::ContentBlock::Text(messages::ContentTextBlock {
								text: "".to_string(),
								citations: None,
								cache_control: None,
							}),
						});

						log.non_atomic_mutate(|r| {
							r.response.first_token = Some(Instant::now());
						});
					}

					events.push(messages::MessagesStreamEvent::ContentBlockDelta {
						index: 0,
						delta: messages::ContentBlockDelta::TextDelta {
							text: content.clone(),
						},
					});
				}

				if let Some(finish_reason) = &choice.finish_reason {
					if sent_content_block_start {
						events.push(messages::MessagesStreamEvent::ContentBlockStop { index: 0 });
					}

					let stop_reason = match finish_reason {
						completions::FinishReason::Stop => messages::StopReason::EndTurn,
						completions::FinishReason::Length => messages::StopReason::MaxTokens,
						completions::FinishReason::ToolCalls => messages::StopReason::ToolUse,
						completions::FinishReason::ContentFilter => messages::StopReason::Refusal,
						completions::FinishReason::FunctionCall => messages::StopReason::ToolUse,
					};

					events.push(messages::MessagesStreamEvent::MessageDelta {
						delta: messages::MessageDelta {
							stop_reason: Some(stop_reason),
							stop_sequence: None,
						},
						usage: messages::MessageDeltaUsage {
							input_tokens: 0,
							output_tokens: 0,
							cache_creation_input_tokens: None,
							cache_read_input_tokens: None,
						},
					});

					events.push(messages::MessagesStreamEvent::MessageStop);
				}
			}

			if let Some(usage) = f.usage {
				log.non_atomic_mutate(|r| {
					r.response.input_tokens = Some(usage.prompt_tokens as u64);
					r.response.output_tokens = Some(usage.completion_tokens as u64);
					r.response.total_tokens = Some(usage.total_tokens as u64);
				});
			}

			events
		})
	}

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		let res = serde_json::from_slice::<completions::ChatCompletionErrorResponse>(bytes)
			.map_err(AIError::ResponseMarshal)?;
		let m = messages::MessagesErrorResponse {
			r#type: "error".to_string(),
			error: messages::MessagesError {
				r#type: res.error.r#type,
				message: res.error.message,
			},
		};
		Ok(Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
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
		} = req;
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
			user: metadata.and_then(|m| m.fields.get("user_id").cloned()),

			vendor_extensions: completions::RequestVendorExtensions {
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
}

pub fn passthrough_stream(
	log: AsyncLog<llm::LLMInfo>,
	include_completion_in_log: bool,
	rate_limit: LLMResponsePolicies,
	resp: Response,
) -> Response {
	let mut completion = include_completion_in_log.then(String::new);
	let buffer_limit = crate::http::response_buffer_limit(&resp);
	resp.map(|b| {
		let mut seen_provider = false;
		let mut saw_token = false;
		let mut rate_limit = Some(rate_limit);
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
								if let Some(c) = completion.take() {
									r.response.completion = Some(vec![c]);
								}

								if let Some(rl) = rate_limit.take() {
									amend_tokens(rl, r);
								}
							});
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
