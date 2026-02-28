use std::time::Instant;

use agent_core::strng;
use tracing::debug;

use crate::http::Response;
use crate::llm::{AmendOnDrop, types};
use crate::{llm, parse};
use bytes::Bytes;

/// Parse a Google error response, handling both single object and array-wrapped formats.
/// Google's OpenAI-compatible endpoints consistently return `[{"error": {...}}]`
/// rather than `{"error": {...}}` when using the Vertex AI shim.
pub(crate) fn parse_google_error(
	bytes: &Bytes,
) -> Result<types::completions::typed::GoogleErrorResponse, llm::AIError> {
	serde_json::from_slice::<types::completions::typed::GoogleErrorResponse>(bytes).or_else(|_| {
		serde_json::from_slice::<Vec<types::completions::typed::GoogleErrorResponse>>(bytes)
			.map_err(llm::AIError::ResponseParsing)?
			.into_iter()
			.next()
			.ok_or_else(|| {
				llm::AIError::InvalidResponse(agent_core::strng::literal!(
					"error response missing error details"
				))
			})
	})
}

pub(crate) fn google_error_type(error: &types::completions::typed::GoogleError) -> &'static str {
	match error.status.as_deref() {
		Some("INVALID_ARGUMENT") | Some("FAILED_PRECONDITION") => "invalid_request_error",
		Some("NOT_FOUND") => "not_found_error",
		Some("PERMISSION_DENIED") | Some("UNAUTHENTICATED") => "authentication_error",
		Some("RESOURCE_EXHAUSTED") => "rate_limit_error",
		_ => "api_error",
	}
}

pub(crate) fn parse_chat_completion_error(
	bytes: &Bytes,
) -> Result<types::completions::typed::ChatCompletionErrorResponse, llm::AIError> {
	serde_json::from_slice::<types::completions::typed::ChatCompletionErrorResponse>(bytes)
		.map_err(llm::AIError::ResponseParsing)
}

/// Translate a Google error response into an OpenAI Chat Completions error response.
pub fn translate_google_error(bytes: &Bytes) -> Result<Bytes, llm::AIError> {
	let res = parse_google_error(bytes)?;
	let m = types::completions::typed::ChatCompletionErrorResponse {
		event_id: None,
		error: types::completions::typed::ChatCompletionError {
			r#type: Some(google_error_type(&res.error).to_string()),
			message: res.error.message.clone(),
			param: None,
			code: res.error.status.clone(),
			event_id: None,
		},
	};
	Ok(Bytes::from(
		serde_json::to_vec(&m).map_err(llm::AIError::ResponseMarshal)?,
	))
}

pub(crate) fn parse_data_url(url: &str) -> Option<(&str, &str)> {
	let raw = url.strip_prefix("data:")?;
	let (meta, data) = raw.split_once(',')?;
	let (media_type, encoding) = meta.split_once(';')?;
	if !encoding.eq_ignore_ascii_case("base64") {
		return None;
	}
	Some((media_type, data))
}

pub(crate) fn extract_system_text(
	msg: &types::completions::typed::RequestMessage,
) -> Option<String> {
	fn normalize_text(text: &str) -> Option<String> {
		if text.trim().is_empty() {
			None
		} else {
			Some(text.to_string())
		}
	}

	match msg {
		types::completions::typed::RequestMessage::System(system) => match &system.content {
			types::completions::typed::RequestSystemMessageContent::Text(text) => normalize_text(text),
			types::completions::typed::RequestSystemMessageContent::Array(parts) => {
				let text = parts
					.iter()
					.map(|part| match part {
						types::completions::typed::RequestSystemMessageContentPart::Text(text) => {
							text.text.as_str()
						},
					})
					.filter(|text| !text.trim().is_empty())
					.collect::<Vec<_>>()
					.join("\n");
				normalize_text(&text)
			},
		},
		types::completions::typed::RequestMessage::Developer(developer) => match &developer.content {
			types::completions::typed::RequestDeveloperMessageContent::Text(text) => normalize_text(text),
			types::completions::typed::RequestDeveloperMessageContent::Array(parts) => {
				let text = parts
					.iter()
					.map(|part| match part {
						types::completions::typed::RequestDeveloperMessageContentPart::Text(text) => {
							text.text.as_str()
						},
					})
					.filter(|text| !text.trim().is_empty())
					.collect::<Vec<_>>()
					.join("\n");
				normalize_text(&text)
			},
		},
		_ => None,
	}
}

pub mod from_messages {
	use std::collections::{HashMap, HashSet};

	use itertools::Itertools;
	use messages::{ToolResultContent, ToolResultContentPart};
	use types::completions::typed as completions;
	use types::messages::typed as messages;

	use crate::json;
	use crate::llm::{AIError, types};

	use crate::llm::types::ResponseType;
	use crate::parse::sse::SseJsonEvent;
	use crate::telemetry::log::AsyncLog;
	use agent_core::strng;
	use bytes::Bytes;
	use serde_json::Value;
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
		let anthropic = translate_response_internal(resp)?;
		Ok(Box::new(anthropic))
	}

	fn translate_response_internal(
		resp: completions::Response,
	) -> Result<messages::MessagesResponse, AIError> {
		let completions::Response {
			id,
			choices,
			model,
			usage,
			..
		} = resp;
		// Anthropic only supports one choice
		let choice = choices
			.into_iter()
			.next()
			.ok_or_else(|| AIError::InvalidResponse(strng::literal!("chat response missing choices")))?;

		let mut content: Vec<messages::ContentBlock> = Vec::new();
		if let Some(text) = choice.message.content {
			content.push(messages::ContentBlock::Text(messages::ContentTextBlock {
				text,
				citations: None,
				cache_control: None,
			}));
		}
		if let Some(tool_calls) = choice.message.tool_calls {
			content.extend(tool_calls.into_iter().filter_map(|tc| match tc {
				completions::MessageToolCalls::Function(f) => {
					let input =
						serde_json::from_str::<serde_json::Value>(&f.function.arguments).unwrap_or_default();
					Some(messages::ContentBlock::ToolUse {
						id: f.id,
						name: f.function.name,
						input,
						cache_control: None,
					})
				},
				completions::MessageToolCalls::Custom(_) => None,
			}));
		}

		let stop_reason = choice
			.finish_reason
			.map(|r| match r {
				completions::FinishReason::Stop => messages::StopReason::EndTurn,
				completions::FinishReason::Length => messages::StopReason::MaxTokens,
				completions::FinishReason::ToolCalls => messages::StopReason::ToolUse,
				completions::FinishReason::ContentFilter => messages::StopReason::EndTurn,
				completions::FinishReason::FunctionCall => messages::StopReason::ToolUse,
			})
			.unwrap_or(messages::StopReason::EndTurn);

		Ok(messages::MessagesResponse {
			id,
			r#type: "message".to_string(),
			role: messages::Role::Assistant,
			model,
			stop_reason: Some(stop_reason),
			stop_sequence: None,
			usage: messages::Usage {
				input_tokens: usage
					.as_ref()
					.map(|u| u.prompt_tokens as usize)
					.unwrap_or(0),
				output_tokens: usage
					.as_ref()
					.map(|u| u.completion_tokens as usize)
					.unwrap_or(0),
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
			},
			content,
		})
	}

	pub fn translate_stream(
		b: crate::http::Body,
		buffer_limit: usize,
		log: AsyncLog<crate::llm::LLMInfo>,
	) -> crate::http::Body {
		#[derive(Debug)]
		struct PendingToolCall {
			id: Option<String>,
			name: Option<String>,
			pending_json: String,
		}

		#[derive(Debug, Default)]
		struct StreamState {
			sent_message_start: bool,
			sent_message_stop: bool,
			sent_first_token: bool,
			next_block_index: usize,
			text_block_index: Option<usize>,
			tool_block_indices: HashMap<u32, usize>,
			open_tool_blocks: HashSet<u32>,
			pending_tool_calls: HashMap<u32, PendingToolCall>,
			pending_stop_reason: Option<messages::StopReason>,
			pending_usage: Option<completions::Usage>,
		}

		fn push_event(
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			event: messages::MessagesStreamEvent,
		) {
			let name = event.event_name();
			events.push((name, event));
		}

		fn close_text_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) {
			if let Some(index) = state.text_block_index.take() {
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStop { index },
				);
			}
		}

		fn close_all_tool_blocks(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) {
			let mut blocks: Vec<(u32, usize)> = state
				.open_tool_blocks
				.iter()
				.filter_map(|tool_index| {
					state
						.tool_block_indices
						.get(tool_index)
						.map(|index| (*tool_index, *index))
				})
				.collect();
			blocks.sort_by_key(|(_, index)| *index);
			for (tool_index, index) in blocks {
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStop { index },
				);
				state.open_tool_blocks.remove(&tool_index);
			}
		}

		fn open_text_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) -> usize {
			if let Some(index) = state.text_block_index {
				return index;
			}
			close_all_tool_blocks(state, events);
			let index = state.next_block_index;
			state.next_block_index += 1;
			state.text_block_index = Some(index);
			push_event(
				events,
				messages::MessagesStreamEvent::ContentBlockStart {
					index,
					content_block: messages::ContentBlock::Text(messages::ContentTextBlock {
						text: "".to_string(),
						citations: None,
						cache_control: None,
					}),
				},
			);
			index
		}

		fn open_tool_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			tool_index: u32,
			id: String,
			name: String,
		) -> usize {
			close_text_block(state, events);
			let index = *state
				.tool_block_indices
				.entry(tool_index)
				.or_insert_with(|| {
					let idx = state.next_block_index;
					state.next_block_index += 1;
					idx
				});

			// Keep each tool-use block open across interleaved deltas to avoid
			// emitting duplicate start events for the same block index.
			if state.open_tool_blocks.insert(tool_index) {
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStart {
						index,
						content_block: messages::ContentBlock::ToolUse {
							id,
							name,
							input: Value::Object(serde_json::Map::new()),
							cache_control: None,
						},
					},
				);
			}
			index
		}

		fn maybe_set_first_token(state: &mut StreamState, log: &AsyncLog<crate::llm::LLMInfo>) {
			if state.sent_first_token {
				return;
			}
			state.sent_first_token = true;
			log.non_atomic_mutate(|r| {
				r.response.first_token = Some(Instant::now());
			});
		}

		fn flush_message_end(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			log: &AsyncLog<crate::llm::LLMInfo>,
			force: bool,
		) {
			if state.sent_message_stop {
				return;
			}
			let stop_reason = match state.pending_stop_reason.take() {
				Some(stop_reason) => stop_reason,
				None if force => messages::StopReason::EndTurn,
				None => return,
			};
			let usage = match state.pending_usage.take() {
				Some(usage) => Some(usage),
				None if force => None,
				None => {
					state.pending_stop_reason = Some(stop_reason);
					return;
				},
			};

			close_text_block(state, events);
			close_all_tool_blocks(state, events);

			let (input_tokens, output_tokens) = usage
				.as_ref()
				.map(|u| (u.prompt_tokens as usize, u.completion_tokens as usize))
				.unwrap_or((0, 0));

			push_event(
				events,
				messages::MessagesStreamEvent::MessageDelta {
					delta: messages::MessageDelta {
						stop_reason: Some(stop_reason),
						stop_sequence: None,
					},
					usage: messages::MessageDeltaUsage {
						input_tokens,
						output_tokens,
						cache_creation_input_tokens: None,
						cache_read_input_tokens: None,
					},
				},
			);
			push_event(events, messages::MessagesStreamEvent::MessageStop);
			state.sent_message_stop = true;

			if let Some(usage) = usage {
				log.non_atomic_mutate(|r| {
					r.response.input_tokens = Some(usage.prompt_tokens as u64);
					r.response.output_tokens = Some(usage.completion_tokens as u64);
					r.response.total_tokens = Some(usage.total_tokens as u64);
				});
			}
		}

		let mut state = StreamState::default();

		crate::parse::sse::json_transform_multi::<
			completions::StreamResponse,
			messages::MessagesStreamEvent,
			_,
		>(b, buffer_limit, move |evt| {
			let mut events: Vec<(&'static str, messages::MessagesStreamEvent)> = Vec::new();
			match evt {
				SseJsonEvent::Done => {
					flush_message_end(&mut state, &mut events, &log, true);
					return events;
				},
				SseJsonEvent::Data(Err(e)) => {
					tracing::warn!(
						"Failed to parse OpenAI stream response during translation: {}",
						e
					);
					return events;
				},
				SseJsonEvent::Data(Ok(f)) => {
					if !state.sent_message_start {
						state.sent_message_start = true;
						push_event(
							&mut events,
							messages::MessagesStreamEvent::MessageStart {
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
							},
						);

						log.non_atomic_mutate(|r| r.response.provider_model = Some(strng::new(&f.model)));
					}

					if let Some(usage) = f.usage {
						state.pending_usage = Some(usage);
					}

					if let Some(choice) = f.choices.first() {
						if let Some(content) = &choice.delta.content {
							let index = open_text_block(&mut state, &mut events);
							maybe_set_first_token(&mut state, &log);
							push_event(
								&mut events,
								messages::MessagesStreamEvent::ContentBlockDelta {
									index,
									delta: messages::ContentBlockDelta::TextDelta {
										text: content.clone(),
									},
								},
							);
						}

						if let Some(tool_calls) = &choice.delta.tool_calls {
							for tool_call in tool_calls {
								let tool_index = tool_call.index;
								let (should_open, id, name, pending_json) = {
									let entry =
										state
											.pending_tool_calls
											.entry(tool_index)
											.or_insert(PendingToolCall {
												id: None,
												name: None,
												pending_json: String::new(),
											});
									if let Some(id) = &tool_call.id {
										entry.id = Some(id.clone());
									}
									if let Some(function) = &tool_call.function {
										if let Some(name) = &function.name {
											entry.name = Some(name.clone());
										}
										if let Some(args) = &function.arguments {
											entry.pending_json.push_str(args);
										}
									}

									let should_open = entry.name.is_some();
									let id = entry.id.clone();
									let name = entry.name.clone();
									let pending_json = if should_open && !entry.pending_json.is_empty() {
										Some(std::mem::take(&mut entry.pending_json))
									} else {
										None
									};
									(should_open, id, name, pending_json)
								};

								if should_open {
									let id = id.unwrap_or_else(|| format!("tool_call_{tool_index}"));
									let name = name.unwrap_or_default();
									let index = open_tool_block(&mut state, &mut events, tool_index, id, name);

									if let Some(pending_json) = pending_json {
										maybe_set_first_token(&mut state, &log);
										let delta = messages::ContentBlockDelta::InputJsonDelta {
											partial_json: pending_json,
										};
										push_event(
											&mut events,
											messages::MessagesStreamEvent::ContentBlockDelta { index, delta },
										);
									}
								}
							}
						}

						if let Some(finish_reason) = &choice.finish_reason {
							let stop_reason = match finish_reason {
								completions::FinishReason::Stop => messages::StopReason::EndTurn,
								completions::FinishReason::Length => messages::StopReason::MaxTokens,
								completions::FinishReason::ToolCalls => messages::StopReason::ToolUse,
								completions::FinishReason::ContentFilter => messages::StopReason::Refusal,
								completions::FinishReason::FunctionCall => messages::StopReason::ToolUse,
							};
							state.pending_stop_reason = Some(stop_reason);
						}
					}

					if state.pending_stop_reason.is_some() && state.pending_usage.is_some() {
						flush_message_end(&mut state, &mut events, &log, false);
					}
				},
			}
			events
		})
	}

	fn normalized_error_type(status: ::http::StatusCode, error_type: Option<&str>) -> &str {
		error_type.unwrap_or(match status {
			::http::StatusCode::BAD_REQUEST => "invalid_request_error",
			::http::StatusCode::UNAUTHORIZED | ::http::StatusCode::FORBIDDEN => "authentication_error",
			::http::StatusCode::NOT_FOUND => "not_found_error",
			::http::StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
			_ => "api_error",
		})
	}

	pub fn translate_error(bytes: &Bytes, status: ::http::StatusCode) -> Result<Bytes, AIError> {
		let res = super::parse_chat_completion_error(bytes)?;
		let m = messages::MessagesErrorResponse {
			r#type: "error".to_string(),
			error: messages::MessagesError {
				r#type: normalized_error_type(status, res.error.r#type.as_deref()).to_string(),
				message: res.error.message,
			},
		};
		Ok(Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
	}

	/// Convert an Anthropic image source JSON value into an OpenAI-compatible URL string.
	/// Base64 sources become `data:` URIs; URL sources pass through directly.
	fn anthropic_source_to_url(source: &serde_json::Value) -> Option<String> {
		let source_type = source.get("type")?.as_str()?;
		match source_type {
			"base64" => {
				let media_type = source.get("media_type")?.as_str()?;
				let data = source.get("data")?.as_str()?;
				Some(format!("data:{media_type};base64,{data}"))
			},
			"url" => {
				let url = source.get("url")?.as_str()?;
				Some(url.to_string())
			},
			_ => None,
		}
	}

	#[allow(deprecated)]
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
				messages::SystemPrompt::Blocks(blocks) => blocks
					.into_iter()
					.map(|block| match block {
						messages::SystemContentBlock::Text { text, .. } => text,
					})
					.collect::<Vec<_>>()
					.join("\n"),
			};
			msgs.push(completions::RequestMessage::System(
				completions::RequestSystemMessage {
					content: completions::RequestSystemMessageContent::Text(system_text),
					name: None,
				},
			));
		}

		// Convert messages from Anthropic to universal format
		for msg in messages {
			match msg.role {
				messages::Role::User => {
					let mut parts: Vec<completions::RequestUserMessageContentPart> = Vec::new();

					for block in msg.content {
						match block {
							messages::ContentBlock::Text(messages::ContentTextBlock { text, .. }) => {
								parts.push(completions::RequestUserMessageContentPart::Text(
									completions::RequestMessageContentPartText { text },
								));
							},
							messages::ContentBlock::Image(messages::ContentImageBlock { source, .. }) => {
								if let Some(url) = anthropic_source_to_url(&source) {
									parts.push(completions::RequestUserMessageContentPart::ImageUrl(
										completions::RequestMessageContentPartImage {
											image_url: completions::ImageUrl { url, detail: None },
										},
									));
								}
							},
							messages::ContentBlock::ToolResult {
								tool_use_id,
								content,
								..
							} => {
								let tool_content = match content {
									ToolResultContent::Text(t) => completions::RequestToolMessageContent::Text(t),
									ToolResultContent::Array(arr) => completions::RequestToolMessageContent::Array(
										arr
											.into_iter()
											.filter_map(|p| match p {
												ToolResultContentPart::Text { text, .. } => {
													Some(completions::RequestToolMessageContentPart::Text(
														completions::RequestMessageContentPartText { text },
													))
												},
												_ => None,
											})
											.collect_vec(),
									),
								};
								msgs.push(completions::RequestMessage::Tool(
									completions::RequestToolMessage {
										content: tool_content,
										tool_call_id: tool_use_id,
									},
								));
							},
							messages::ContentBlock::ServerToolUse { .. }
							| messages::ContentBlock::ToolUse { .. } => {},
							_ => {},
						}
					}

					if !parts.is_empty() {
						msgs.push(completions::RequestMessage::User(
							completions::RequestUserMessage {
								content: completions::RequestUserMessageContent::Array(parts),
								name: None,
							},
						));
					}
				},
				messages::Role::Assistant => {
					let mut assistant_text = String::new();
					let mut tool_calls: Vec<completions::MessageToolCalls> = Vec::new();
					for block in msg.content {
						match block {
							messages::ContentBlock::Text(messages::ContentTextBlock { text, .. }) => {
								if !assistant_text.is_empty() {
									assistant_text.push('\n');
								}
								assistant_text.push_str(&text);
							},
							messages::ContentBlock::ToolUse {
								id, name, input, ..
							} => {
								tool_calls.push(completions::MessageToolCalls::Function(
									completions::MessageToolCall {
										id,
										function: completions::FunctionCall {
											name,
											arguments: serde_json::to_string(&input).unwrap_or_default(),
										},
									},
								));
							},
							messages::ContentBlock::Thinking { .. } => {
								// TODO
							},
							messages::ContentBlock::RedactedThinking { .. } => {
								// TODO
							},
							_ => {},
						}
					}
					if !assistant_text.is_empty() || !tool_calls.is_empty() {
						msgs.push(completions::RequestMessage::Assistant(
							completions::RequestAssistantMessage {
								content: if assistant_text.is_empty() {
									None
								} else {
									Some(completions::RequestAssistantMessageContent::Text(
										assistant_text,
									))
								},
								name: None,
								tool_calls: if tool_calls.is_empty() {
									None
								} else {
									Some(tool_calls)
								},
								refusal: None,
								audio: None,
								function_call: None,
							},
						));
					}
				},
			}
		}

		let tools: Vec<completions::Tool> = tools
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

		let mut parallel_tool_calls = None;
		let tool_choice = tool_choice.map(|choice| match choice {
			messages::ToolChoice::Auto {
				disable_parallel_tool_use,
			} => {
				parallel_tool_calls = disable_parallel_tool_use.map(|d| !d);
				completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto)
			},
			messages::ToolChoice::Any {
				disable_parallel_tool_use,
			} => {
				parallel_tool_calls = disable_parallel_tool_use.map(|d| !d);
				completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required)
			},
			messages::ToolChoice::Tool {
				name,
				disable_parallel_tool_use,
			} => {
				parallel_tool_calls = disable_parallel_tool_use.map(|d| !d);
				completions::ToolChoiceOption::Function(completions::NamedToolChoice {
					function: completions::FunctionName { name },
				})
			},
			messages::ToolChoice::None {} => {
				completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None)
			},
		});

		// Preserve the common `user_id` field when mapping Messages -> OpenAI chat completions.
		// The rest of `messages.metadata` is handled only on provider-native Messages paths.
		// Forwarding it here breaks OpenAI-compatible backends because `metadata` requires `store=true`,
		// which agentgateway does not set on this path.
		let user_id = metadata
			.as_ref()
			.and_then(|m| m.fields.get("user_id").cloned());

		let stop = if stop_sequences.is_empty() {
			None
		} else {
			Some(completions::Stop::StringArray(stop_sequences))
		};

		completions::Request {
			model: Some(model),
			messages: msgs,
			stream: Some(stream),
			temperature,
			top_p,
			max_completion_tokens: Some(max_tokens as u32),
			max_tokens: None,
			stop,
			tools: if tools.is_empty() { None } else { Some(tools) },
			tool_choice,
			user: user_id,
			vendor_extensions: completions::RequestVendorExtensions {
				top_k,
				thinking_budget_tokens: thinking.as_ref().and_then(|t| match t {
					messages::ThinkingInput::Enabled { budget_tokens } => Some(*budget_tokens),
					_ => None,
				}),
			},
			stream_options: if stream {
				Some(completions::StreamOptions {
					include_usage: Some(true),
					include_obfuscation: None,
				})
			} else {
				None
			},
			frequency_penalty: None,
			presence_penalty: None,
			seed: None,
			// Fields not applicable from Anthropic Messages
			store: None,
			reasoning_effort,
			metadata: None,
			logit_bias: None,
			logprobs: None,
			top_logprobs: None,
			n: None,
			modalities: None,
			prediction: None,
			audio: None,
			response_format,
			#[allow(deprecated)]
			function_call: None,
			#[allow(deprecated)]
			functions: None,
			service_tier: None,
			parallel_tool_calls,
			web_search_options: None,
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
