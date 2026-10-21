#[cfg(test)]
#[path = "openai_compat_tests.rs"]
mod tests;

pub mod from_responses {
	use types::completions::typed as completions;
	use types::responses::typed as responses;

	use crate::{AIError, json, types};

	pub struct TranslatedRequest {
		pub request: completions::Request,
		pub namespaces: crate::conversion::namespace_tools::NamespaceToolMap,
	}

	/// Translate an OpenAI Responses request into an OpenAI-compatible chat completions request.
	pub fn translate(req: &types::responses::Request) -> Result<Vec<u8>, AIError> {
		let translated = translate_request(req)?;
		serde_json::to_vec(&translated.request).map_err(AIError::RequestMarshal)
	}

	pub fn translate_request(req: &types::responses::Request) -> Result<TranslatedRequest, AIError> {
		let typed = json::convert::<_, responses::CreateResponse>(req)
			.map_err(|err| AIError::RequestParsing(crate::InputFormat::Responses, err))?;
		translate_typed(typed, false)
	}

	pub fn translate_typed(
		mut typed: responses::CreateResponse,
		preserve_reasoning: bool,
	) -> Result<TranslatedRequest, AIError> {
		let namespaces =
			crate::conversion::namespace_tools::NamespaceToolMap::rewrite_request(&mut typed)?;
		Ok(TranslatedRequest {
			request: translate_internal(typed, preserve_reasoning),
			namespaces,
		})
	}

	fn cache_breakpoint(
		breakpoint: Option<responses::PromptCacheBreakpointConfig>,
	) -> Option<completions::PromptCacheBreakpointParam> {
		breakpoint.map(|_| completions::PromptCacheBreakpointParam {
			mode: completions::PromptCacheBreakpointParamMode::Explicit,
		})
	}

	fn translate_internal(
		req: responses::CreateResponse,
		preserve_reasoning: bool,
	) -> completions::Request {
		use responses::{
			EasyInputContent, InputContent, InputItem, InputMessage, InputParam, InputRole,
			InputTextContent, Item, MessageItem, OutputMessageContent, Role as ResponsesRole,
			TextResponseFormatConfiguration,
		};

		let mut messages: Vec<completions::RequestMessage> = Vec::new();

		if let Some(instructions) = &req.instructions {
			messages.push(completions::RequestMessage::Developer(
				completions::RequestDeveloperMessage {
					content: completions::RequestDeveloperMessageContent::Text(instructions.clone()),
					name: None,
				},
			));
		}

		let items = match &req.input {
			InputParam::Text(text) => vec![InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: text.clone(),
					prompt_cache_breakpoint: None,
				})],
				role: InputRole::User,
				status: None,
			})],
			InputParam::Items(items) => items.clone(),
		};

		for item in items {
			match item {
				InputItem::EasyMessage(msg) => match msg.role {
					ResponsesRole::User => {
						let content = match msg.content {
							EasyInputContent::Text(text) => completions::RequestUserMessageContent::Text(text),
							EasyInputContent::ContentList(parts) => {
								completions::RequestUserMessageContent::Array(
									parts
										.into_iter()
										.filter_map(|part| match part {
											InputContent::InputText(text) => {
												Some(completions::RequestUserMessageContentPart::Text(
													completions::RequestMessageContentPartText {
														text: text.text,
														prompt_cache_breakpoint: cache_breakpoint(text.prompt_cache_breakpoint),
													},
												))
											},
											_ => None,
										})
										.collect(),
								)
							},
						};
						messages.push(completions::RequestMessage::User(
							completions::RequestUserMessage {
								content,
								name: None,
							},
						));
					},
					ResponsesRole::Assistant => {
						let content = match msg.content {
							EasyInputContent::Text(text) => {
								completions::RequestAssistantMessageContent::Text(text)
							},
							EasyInputContent::ContentList(parts) => {
								completions::RequestAssistantMessageContent::Array(
									parts
										.into_iter()
										.filter_map(|part| match part {
											InputContent::InputText(text) => {
												Some(completions::RequestAssistantMessageContentPart::Text(
													completions::RequestMessageContentPartText {
														text: text.text,
														prompt_cache_breakpoint: cache_breakpoint(text.prompt_cache_breakpoint),
													},
												))
											},
											_ => None,
										})
										.collect(),
								)
							},
						};
						messages.push(completions::RequestMessage::Assistant(
							completions::RequestAssistantMessage {
								content: Some(content),
								..Default::default()
							},
						));
					},
					ResponsesRole::System | ResponsesRole::Developer => {
						let content = match msg.content {
							EasyInputContent::Text(text) => {
								completions::RequestDeveloperMessageContent::Text(text)
							},
							EasyInputContent::ContentList(parts) => {
								completions::RequestDeveloperMessageContent::Array(
									parts
										.into_iter()
										.filter_map(|part| match part {
											InputContent::InputText(text) => {
												Some(completions::RequestDeveloperMessageContentPart::Text(
													completions::RequestMessageContentPartText {
														text: text.text,
														prompt_cache_breakpoint: cache_breakpoint(text.prompt_cache_breakpoint),
													},
												))
											},
											_ => None,
										})
										.collect(),
								)
							},
						};
						messages.push(completions::RequestMessage::Developer(
							completions::RequestDeveloperMessage {
								content,
								name: None,
							},
						));
					},
				},
				InputItem::ItemReference(_) => continue,
				InputItem::Program(_) | InputItem::ProgramOutput(_) | InputItem::CompactionTrigger(_) => {
					tracing::debug!(
						"Skipping unsupported Responses input item for OpenAI-compatible chat completions"
					);
					continue;
				},
				InputItem::Item(item) => match item {
					Item::Message(msg_item) => match msg_item {
						MessageItem::Input(msg) => match msg.role {
							InputRole::User => {
								messages.push(completions::RequestMessage::User(
									completions::RequestUserMessage {
										content: completions::RequestUserMessageContent::Array(
											msg
												.content
												.into_iter()
												.filter_map(|content| match content {
													InputContent::InputText(text) => {
														Some(completions::RequestUserMessageContentPart::Text(
															completions::RequestMessageContentPartText {
																text: text.text,
																prompt_cache_breakpoint: cache_breakpoint(
																	text.prompt_cache_breakpoint,
																),
															},
														))
													},
													_ => None,
												})
												.collect(),
										),
										name: None,
									},
								));
							},
							InputRole::System => {
								messages.push(completions::RequestMessage::System(
									completions::RequestSystemMessage {
										content: completions::RequestSystemMessageContent::Array(
											msg
												.content
												.into_iter()
												.filter_map(|content| match content {
													InputContent::InputText(text) => {
														Some(completions::RequestSystemMessageContentPart::Text(
															completions::RequestMessageContentPartText {
																text: text.text,
																prompt_cache_breakpoint: cache_breakpoint(
																	text.prompt_cache_breakpoint,
																),
															},
														))
													},
													_ => None,
												})
												.collect(),
										),
										name: None,
									},
								));
							},
							InputRole::Developer => {
								messages.push(completions::RequestMessage::Developer(
									completions::RequestDeveloperMessage {
										content: completions::RequestDeveloperMessageContent::Array(
											msg
												.content
												.into_iter()
												.filter_map(|content| match content {
													InputContent::InputText(text) => {
														Some(completions::RequestDeveloperMessageContentPart::Text(
															completions::RequestMessageContentPartText {
																text: text.text,
																prompt_cache_breakpoint: cache_breakpoint(
																	text.prompt_cache_breakpoint,
																),
															},
														))
													},
													_ => None,
												})
												.collect(),
										),
										name: None,
									},
								));
							},
						},
						MessageItem::Output(msg) => {
							let text = msg
								.content
								.iter()
								.filter_map(|c| match c {
									OutputMessageContent::OutputText(t) => Some(t.text.clone()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n");

							let content = if text.is_empty() {
								None
							} else {
								Some(completions::RequestAssistantMessageContent::Text(text))
							};
							if preserve_reasoning
								&& let Some(completions::RequestMessage::Assistant(message)) = messages.last_mut()
								&& message.reasoning_signature.is_some()
							{
								message.content = content;
							} else {
								messages.push(completions::RequestMessage::Assistant(
									completions::RequestAssistantMessage {
										content,
										..Default::default()
									},
								));
							}
						},
					},
					Item::FunctionCall(call) => {
						let tool_call = completions::MessageToolCalls::Function(completions::MessageToolCall {
							id: call.call_id.clone(),
							function: completions::FunctionCall {
								name: call.name.clone(),
								arguments: call.arguments.clone(),
							},
						});
						if let Some(completions::RequestMessage::Assistant(message)) = messages.last_mut()
							&& (preserve_reasoning || message.tool_calls.is_some())
						{
							message
								.tool_calls
								.get_or_insert_with(Vec::new)
								.push(tool_call);
						} else {
							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									tool_calls: Some(vec![tool_call]),
									..Default::default()
								},
							));
						}
					},
					Item::Reasoning(reasoning)
						if preserve_reasoning && reasoning.encrypted_content.is_some() =>
					{
						if let Some(completions::RequestMessage::Assistant(message)) = messages.last_mut() {
							message.reasoning_signature = reasoning.encrypted_content;
						} else {
							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									reasoning_signature: reasoning.encrypted_content,
									..Default::default()
								},
							));
						}
					},
					Item::FunctionCallOutput(output) => {
						let Some(call_id) = output.call_id.filter(|call_id| !call_id.is_empty()) else {
							continue;
						};
						let output_text = match output.output {
							responses::FunctionCallOutput::Text(text) => text,
							responses::FunctionCallOutput::Content(parts) => parts
								.iter()
								.filter_map(|part| match part {
									InputContent::InputText(t) => Some(t.text.clone()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n"),
						};
						messages.push(completions::RequestMessage::Tool(
							completions::RequestToolMessage {
								content: completions::RequestToolMessageContent::Text(output_text),
								tool_call_id: call_id,
							},
						));
					},
					Item::CustomToolCall(call) => {
						let arguments = serde_json::to_string(&call.input).unwrap_or_else(|_| "{}".to_string());
						let tool_call = completions::MessageToolCalls::Function(completions::MessageToolCall {
							id: call.id.clone(),
							function: completions::FunctionCall {
								name: call.name.clone(),
								arguments,
							},
						});
						if let Some(completions::RequestMessage::Assistant(message)) = messages.last_mut()
							&& let Some(tool_calls) = &mut message.tool_calls
						{
							tool_calls.push(tool_call);
						} else {
							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									tool_calls: Some(vec![tool_call]),
									..Default::default()
								},
							));
						}
					},
					Item::CustomToolCallOutput(output) => {
						let text = match &output.output {
							responses::CustomToolCallOutputOutput::Text(t) => t.clone(),
							_ => continue,
						};
						messages.push(completions::RequestMessage::Tool(
							completions::RequestToolMessage {
								content: completions::RequestToolMessageContent::Text(text),
								tool_call_id: output.id.clone().unwrap_or_default(),
							},
						));
					},
					_ => continue,
				},
			}
		}

		let tools: Option<Vec<completions::Tool>> = req.tools.as_ref().map(|tools| {
			tools
				.iter()
				.filter_map(|tool| match tool {
					responses::Tool::Function(func) => {
						Some(completions::Tool::Function(completions::FunctionTool {
							function: completions::FunctionObject {
								name: func.name.clone(),
								description: func.description.clone(),
								parameters: func.parameters.clone(),
								strict: func.strict,
							},
						}))
					},
					_ => None,
				})
				.collect()
		});

		let tool_choice = req.tool_choice.as_ref().and_then(|tc| {
			use responses::{ToolChoiceFunction, ToolChoiceOptions, ToolChoiceParam};
			match tc {
				ToolChoiceParam::Option(ToolChoiceOptions::Auto) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto),
				),
				ToolChoiceParam::Option(ToolChoiceOptions::Required) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required),
				),
				ToolChoiceParam::Option(ToolChoiceOptions::None) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None),
				),
				ToolChoiceParam::Function(ToolChoiceFunction { name }) => Some(
					completions::ToolChoiceOption::Function(completions::NamedToolChoice {
						function: completions::FunctionName { name: name.clone() },
					}),
				),
				ToolChoiceParam::BuiltIn(_)
				| ToolChoiceParam::AllowedTools(_)
				| ToolChoiceParam::Mcp(_)
				| ToolChoiceParam::Custom(_)
				| ToolChoiceParam::ProgrammaticToolCalling
				| ToolChoiceParam::ApplyPatch
				| ToolChoiceParam::Shell => {
					tracing::warn!(
						"Unsupported tool choice for OpenAI-compatible chat completions: {:?}",
						tc
					);
					None
				},
			}
		});

		let reasoning_effort = req.reasoning.as_ref().and_then(|r| {
			r.effort.as_ref().and_then(|e| match e {
				responses::ReasoningEffort::Minimal => Some(completions::ReasoningEffort::Minimal),
				responses::ReasoningEffort::Low => Some(completions::ReasoningEffort::Low),
				responses::ReasoningEffort::Medium => Some(completions::ReasoningEffort::Medium),
				responses::ReasoningEffort::High => Some(completions::ReasoningEffort::High),
				responses::ReasoningEffort::Xhigh => Some(completions::ReasoningEffort::Xhigh),
				responses::ReasoningEffort::Max => Some(completions::ReasoningEffort::Max),
				responses::ReasoningEffort::None => None,
			})
		});

		let response_format = req.text.as_ref().and_then(|text| match &text.format {
			TextResponseFormatConfiguration::JsonSchema(json_schema) => {
				Some(completions::ResponseFormat::JsonSchema {
					json_schema: completions::ResponseFormatJsonSchema {
						description: json_schema.description.clone(),
						name: json_schema.name.clone(),
						schema: json_schema.schema.clone(),
						strict: json_schema.strict,
					},
				})
			},
			TextResponseFormatConfiguration::JsonObject => Some(completions::ResponseFormat::JsonObject),
			TextResponseFormatConfiguration::Text => None,
		});

		let stream = req.stream.unwrap_or(false);
		let stream_options = if stream {
			Some(completions::StreamOptions {
				include_usage: Some(true),
				include_obfuscation: None,
			})
		} else {
			None
		};

		#[allow(deprecated)]
		completions::Request {
			messages,
			tools,
			tool_choice,
			stream_options,
			reasoning_effort,
			response_format,
			stream: Some(stream),
			model: req.model.clone(),
			moderation: None,
			temperature: req.temperature,
			top_p: req.top_p,
			max_completion_tokens: req.max_output_tokens,
			parallel_tool_calls: req.parallel_tool_calls,
			vendor_extensions: completions::RequestVendorExtensions::default(),
			max_tokens: None,
			stop: None,
			user: None,
			frequency_penalty: None,
			presence_penalty: None,
			seed: None,
			store: None,
			metadata: None,
			logit_bias: None,
			logprobs: None,
			top_logprobs: None,
			n: None,
			modalities: None,
			prediction: None,
			audio: None,
			function_call: None,
			functions: None,
			service_tier: None,
			web_search_options: None,
		}
	}
}

pub mod to_responses {
	use std::collections::HashMap;
	use std::time::Instant;

	use agent_core::strng;
	use agent_http::Body;
	use bytes::Bytes;
	use rand::RngExt;
	use types::completions::typed as completions;
	use types::responses::typed as responses;

	use crate::parse::sse::SseJsonEvent;
	use crate::types::ResponseType;
	use crate::{AIError, StreamingUsageGuard, json, logged_response_parsing, parse, types};

	type LoggedToolCall = (Option<String>, Option<String>, String);
	type LoggedToolCalls = HashMap<u32, LoggedToolCall>;

	/// Translate an OpenAI-compatible chat completions response into an OpenAI Responses response.
	pub fn translate_response(
		bytes: &Bytes,
		model: &str,
		namespaces: Option<&crate::conversion::namespace_tools::NamespaceToolMap>,
	) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<completions::Response>(bytes)
			.map_err(logged_response_parsing(bytes))?;
		Ok(Box::new(translate_completion(
			resp,
			model,
			namespaces,
			|id| (id, None),
		)?))
	}

	/// `tool_id` maps an upstream tool-call id to the client's id and optional opaque state, sent
	/// as the `encrypted_content` of a reasoning item just before the call
	pub fn translate_completion(
		resp: completions::Response,
		model: &str,
		namespaces: Option<&crate::conversion::namespace_tools::NamespaceToolMap>,
		tool_id: fn(String) -> (String, Option<String>),
	) -> Result<types::responses::Response, AIError> {
		let mut typed = translate_response_internal(resp, model, tool_id);
		if let Some(namespaces) = namespaces {
			namespaces.restore_response(&mut typed);
		}
		json::convert(&typed).map_err(AIError::ResponseParsing)
	}

	fn translate_response_internal(
		resp: completions::Response,
		model: &str,
		tool_id: fn(String) -> (String, Option<String>),
	) -> responses::Response {
		let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());
		let response_builder = types::responses::ResponseBuilder::new(response_id, model.to_string());

		let choice = resp.choices.into_iter().next();
		let finish_reason = choice.as_ref().and_then(|c| c.finish_reason);

		let status = match finish_reason {
			Some(completions::FinishReason::Stop) | None => responses::Status::Completed,
			Some(completions::FinishReason::Length | completions::FinishReason::ContentFilter) => {
				responses::Status::Incomplete
			},
			Some(completions::FinishReason::ToolCalls)
			| Some(completions::FinishReason::FunctionCall) => responses::Status::Completed,
		};
		let item_status = item_status(&status);

		let mut outputs: Vec<responses::OutputItem> = Vec::new();
		let mut text_parts: Vec<responses::OutputMessageContent> = Vec::new();
		let mut tool_calls: Vec<responses::OutputItem> = Vec::new();

		if let Some(choice) = &choice {
			if choice.message.reasoning().is_some() || choice.message.reasoning_signature.is_some() {
				outputs.push(responses::OutputItem::Reasoning(responses::ReasoningItem {
					id: Some(format!("rs_{:016x}", rand::rng().random::<u64>())),
					summary: vec![],
					content: choice.message.reasoning().map(|text| {
						vec![responses::ReasoningItemContent::ReasoningText(
							responses::ReasoningTextContent {
								text: text.to_string(),
							},
						)]
					}),
					encrypted_content: choice.message.reasoning_signature.clone(),
					status: Some(item_status),
				}));
			}
			if let Some(content) = &choice.message.content {
				text_parts.push(responses::OutputMessageContent::OutputText(
					responses::OutputTextContent {
						annotations: vec![],
						logprobs: None,
						text: content.clone(),
					},
				));
			}
			if let Some(refusal) = choice.message.refusal.as_ref().filter(|r| !r.is_empty()) {
				text_parts.push(responses::OutputMessageContent::Refusal(
					responses::RefusalContent {
						refusal: refusal.clone(),
					},
				));
			}

			if let Some(tcs) = &choice.message.tool_calls {
				for tc in tcs {
					match tc {
						completions::MessageToolCalls::Function(f) => {
							let (call_id, state) = tool_id(f.id.clone());
							tool_calls.extend(state.map(reasoning_item));
							tool_calls.push(responses::OutputItem::FunctionCall(
								responses::FunctionToolCall {
									arguments: f.function.arguments.clone(),
									id: Some(call_id.clone()),
									call_id,
									name: f.function.name.clone(),
									caller: None,
									status: Some(item_status),
									namespace: None,
									r#async: None,
								},
							));
						},
						completions::MessageToolCalls::Custom(_) => {},
					}
				}
			}
		}

		if !text_parts.is_empty() {
			outputs.push(responses::OutputItem::Message(responses::OutputMessage {
				id: format!("msg_{:016x}", rand::rng().random::<u64>()),
				role: responses::AssistantRole::Assistant,
				phase: None,
				content: text_parts,
				status: item_status,
			}));
		}
		outputs.extend(tool_calls);

		let incomplete_details =
			finish_reason
				.and_then(incomplete_reason)
				.map(|reason| responses::IncompleteDetails {
					reason: reason.to_string(),
				});

		let usage = resp.usage.map(|u| responses::ResponseUsage {
			input_tokens: u.prompt_tokens,
			output_tokens: u.output_tokens(),
			total_tokens: u.total_tokens,
			input_tokens_details: responses::InputTokenDetails {
				cached_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
					.unwrap_or(0) as u32,
				cache_write_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
					.or(u.cache_creation_input_tokens)
					.map(|tokens| tokens as i64),
			},
			output_tokens_details: responses::OutputTokenDetails {
				reasoning_tokens: u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
					.unwrap_or(0) as u32,
			},
		});

		let mut response = response_builder.response(status, usage, None, incomplete_details);
		response.output = outputs;
		response
	}

	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
		namespaces: Option<std::sync::Arc<crate::conversion::namespace_tools::NamespaceToolMap>>,
	) -> Body {
		translate_stream_with_tool_id(b, buffer_limit, log, log_content, namespaces, |id| {
			(id, None)
		})
	}

	/// As [`translate_completion`]. A call's id is fixed when the call first appears, since
	/// `output_item.added` has already published it; the reasoning item is sent then too
	pub fn translate_stream_with_tool_id(
		b: Body,
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
		namespaces: Option<std::sync::Arc<crate::conversion::namespace_tools::NamespaceToolMap>>,
		tool_id: fn(String) -> (String, Option<String>),
	) -> Body {
		use responses::{
			FunctionToolCall, OutputContent, OutputItem, OutputStatus, OutputTextContent, ReasoningItem,
			RefusalContent, ResponseContentPartAddedEvent, ResponseFunctionCallArgumentsDeltaEvent,
			ResponseOutputItemAddedEvent, ResponseReasoningTextDeltaEvent, ResponseRefusalDeltaEvent,
			ResponseStreamEvent, ResponseTextDeltaEvent,
		};

		let mut last_token_at: Option<Instant> = None;
		let mut sent_created = false;
		let mut flushed = false;

		let mut sequence_number: u64 = 0;
		let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());
		let message_item_id = format!("msg_{:016x}", rand::rng().random::<u64>());
		let mut response_builder: Option<types::responses::ResponseBuilder> = None;

		// Output indexes follow arrival order. The message opens lazily so reasoning, which streams
		// first, takes index 0 as it does in the Responses API.
		let mut next_output_index: u32 = 0;
		let mut message_output_index: Option<u32> = None;
		// (item_id, output_index, text)
		let mut reasoning: Option<(String, u32, String)> = None;
		let mut reasoning_signature: Option<String> = None;
		let mut tool_calls: HashMap<u32, StreamedCall> = HashMap::new();
		let mut reasoning_items: Vec<(u32, OutputItem)> = Vec::new();
		let mut logged_tool_calls: Option<LoggedToolCalls> = log_content.tool_calls.then(HashMap::new);
		let mut completion = log_content.completion.then(String::new);
		let mut message_content = MessageContent::default();
		let mut pending_stop_reason: Option<completions::FinishReason> = None;
		let mut pending_usage: Option<completions::Usage> = None;

		let end = std::sync::Arc::new(parse::sse::StreamEnd::default());
		let seen_end = end.clone();
		let mut failed = false;
		let body = parse::sse::json_transform_multi::<completions::StreamEvent, ResponseStreamEvent, _>(
			b,
			buffer_limit,
			move |evt| {
				let mut events: Vec<(&'static str, ResponseStreamEvent)> = Vec::new();

				match evt {
					_ if failed => return events,
					SseJsonEvent::Eof | SseJsonEvent::Error => return events,
					SseJsonEvent::Done => {
						// Fall through to namespace restoration for the terminal events too.
						if !flushed {
							flushed = true;
							let response_builder = response_builder.get_or_insert_with(|| {
								types::responses::ResponseBuilder::new(response_id.clone(), String::new())
							});
							flush_end(
								&mut events,
								&mut sequence_number,
								&mut tool_calls,
								&mut reasoning_items,
								&mut reasoning,
								&mut reasoning_signature,
								&mut pending_stop_reason,
								&mut pending_usage,
								&message_item_id,
								&mut message_output_index,
								&mut next_output_index,
								&message_content,
								&log,
								response_builder,
								&mut completion,
								&mut logged_tool_calls,
							);
						}
					},
					SseJsonEvent::Data(Err(e)) => {
						tracing::warn!(
							"Failed to parse OpenAI-compatible stream response during translation: {}",
							e
						);
						return events;
					},
					SseJsonEvent::Data(Ok(completions::StreamEvent::Error(e))) => {
						failed = true;
						seen_end.fail();
						if !flushed {
							flushed = true;
							let response_builder = response_builder.get_or_insert_with(|| {
								types::responses::ResponseBuilder::new(response_id.clone(), String::new())
							});
							sequence_number += 1;
							events.push((
								"event",
								response_builder.failed_event(sequence_number, None, response_error(e.error)),
							));
						}
						return events;
					},
					SseJsonEvent::Data(Ok(completions::StreamEvent::Chunk(chunk))) => {
						if !sent_created {
							sent_created = true;
							let response_builder = response_builder.insert(
								types::responses::ResponseBuilder::new(response_id.clone(), chunk.model.clone()),
							);

							sequence_number += 1;
							events.push(("event", response_builder.created_event(sequence_number)));

							// `response.in_progress` follows `response.created` in the API's event order.
							sequence_number += 1;
							events.push((
								"event",
								ResponseStreamEvent::ResponseInProgress(responses::ResponseInProgressEvent {
									sequence_number,
									response: response_builder.response(
										responses::Status::InProgress,
										None,
										None,
										None,
									),
								}),
							));

							log.update(|r| {
								r.response.provider_model = Some(strng::new(&chunk.model));
								if let Some(st) = &chunk.service_tier {
									r.response.service_tier = Some(strng::new(st));
								}
							});
						}

						if let Some(usage) = chunk.usage {
							pending_usage = Some(usage);
						}

						if let Some(choice) = chunk.choices.first() {
							if let Some(signature) = &choice.delta.reasoning_signature {
								reasoning_signature = Some(signature.clone());
							}
							if choice.delta.reasoning().is_some() || choice.delta.reasoning_signature.is_some() {
								let (item_id, output_index, reasoning_text) = reasoning.get_or_insert_with(|| {
									let item_id = format!("rs_{:016x}", rand::rng().random::<u64>());
									let output_index = next_output_index;
									next_output_index += 1;
									sequence_number += 1;
									events.push((
										"event",
										ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
											sequence_number,
											output_index,
											item: OutputItem::Reasoning(ReasoningItem {
												id: Some(item_id.clone()),
												summary: vec![],
												content: None,
												encrypted_content: None,
												status: Some(OutputStatus::InProgress),
											}),
										}),
									));
									(item_id, output_index, String::new())
								});
								if let Some(delta) = choice.delta.reasoning() {
									reasoning_text.push_str(delta);

									record_token(&mut last_token_at, &log);

									sequence_number += 1;
									events.push((
										"event",
										ResponseStreamEvent::ResponseReasoningTextDelta(
											ResponseReasoningTextDeltaEvent {
												sequence_number,
												item_id: item_id.clone(),
												output_index: *output_index,
												content_index: 0,
												delta: delta.to_string(),
											},
										),
									));
								}
							}

							if let Some(content) = choice.delta.content.as_ref().filter(|c| !c.is_empty()) {
								if let Some(completion) = completion.as_mut() {
									completion.push_str(content);
								}
								message_content.text.push_str(content);
								let message_output_index = open_message(
									&mut events,
									&mut sequence_number,
									&mut message_output_index,
									&mut next_output_index,
									&message_item_id,
								);
								let (content_index, opened) = message_content.text_index();
								if opened {
									sequence_number += 1;
									events.push((
										"event",
										ResponseStreamEvent::ResponseContentPartAdded(ResponseContentPartAddedEvent {
											sequence_number,
											item_id: message_item_id.clone(),
											output_index: message_output_index,
											content_index,
											part: OutputContent::OutputText(OutputTextContent {
												text: String::new(),
												annotations: Vec::new(),
												logprobs: None,
											}),
										}),
									));
								}

								record_token(&mut last_token_at, &log);

								sequence_number += 1;
								events.push((
									"event",
									ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
										sequence_number,
										item_id: message_item_id.clone(),
										output_index: message_output_index,
										content_index,
										delta: content.clone(),
										logprobs: None,
									}),
								));
							}

							if let Some(refusal) = choice.delta.refusal.as_ref().filter(|r| !r.is_empty()) {
								if let Some(completion) = completion.as_mut() {
									completion.push_str(refusal);
								}
								message_content.refusal.push_str(refusal);
								let message_output_index = open_message(
									&mut events,
									&mut sequence_number,
									&mut message_output_index,
									&mut next_output_index,
									&message_item_id,
								);
								let (content_index, opened) = message_content.refusal_index();
								if opened {
									sequence_number += 1;
									events.push((
										"event",
										ResponseStreamEvent::ResponseContentPartAdded(ResponseContentPartAddedEvent {
											sequence_number,
											item_id: message_item_id.clone(),
											output_index: message_output_index,
											content_index,
											part: OutputContent::Refusal(RefusalContent {
												refusal: String::new(),
											}),
										}),
									));
								}

								record_token(&mut last_token_at, &log);

								sequence_number += 1;
								events.push((
									"event",
									ResponseStreamEvent::ResponseRefusalDelta(ResponseRefusalDeltaEvent {
										sequence_number,
										item_id: message_item_id.clone(),
										output_index: message_output_index,
										content_index,
										delta: refusal.clone(),
									}),
								));
							}

							if let Some(tcs) = &choice.delta.tool_calls {
								// The message keeps its place ahead of the tool calls.
								open_message(
									&mut events,
									&mut sequence_number,
									&mut message_output_index,
									&mut next_output_index,
									&message_item_id,
								);
								for tc in tcs {
									let tool_index = tc.index;
									if let Some(logged_tool_calls) = logged_tool_calls.as_mut() {
										let logged_entry = logged_tool_calls.entry(tool_index).or_default();
										if let Some(id) = &tc.id {
											logged_entry.0 = Some(id.clone());
										}
										if let Some(function) = &tc.function {
											if let Some(name) = &function.name {
												logged_entry.1 = Some(name.clone());
											}
											if let Some(args) = &function.arguments {
												logged_entry.2.push_str(args);
											}
										}
									}

									let is_new = !tool_calls.contains_key(&tool_index);

									if is_new {
										let item_id = format!("call_{:016x}", rand::rng().random::<u64>());
										let (call_id, state) = match &tc.id {
											Some(id) => tool_id(id.clone()),
											None => (item_id.clone(), None),
										};
										if let Some(state) = state {
											let output_index = next_output_index;
											next_output_index += 1;
											let item = reasoning_item(state);
											sequence_number += 1;
											events.push((
												"event",
												ResponseStreamEvent::ResponseOutputItemAdded(
													ResponseOutputItemAddedEvent {
														sequence_number,
														output_index,
														item: item.clone(),
													},
												),
											));
											sequence_number += 1;
											events.push((
												"event",
												ResponseStreamEvent::ResponseOutputItemDone(
													responses::ResponseOutputItemDoneEvent {
														sequence_number,
														output_index,
														item: item.clone(),
													},
												),
											));
											reasoning_items.push((output_index, item));
										}
										tool_calls.insert(
											tool_index,
											StreamedCall {
												item_id,
												call_id,
												name: String::new(),
												arguments: String::new(),
												output_index: next_output_index,
											},
										);
										next_output_index += 1;
									}
									let entry = tool_calls.get_mut(&tool_index).expect("inserted above");

									if let Some(function) = &tc.function {
										if let Some(name) = &function.name {
											entry.name = name.clone();
										}
										if let Some(args) = &function.arguments {
											entry.arguments.push_str(args);
										}
									}

									if is_new {
										record_token(&mut last_token_at, &log);

										sequence_number += 1;
										events.push((
											"event",
											ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
												sequence_number,
												output_index: entry.output_index,
												item: OutputItem::FunctionCall(FunctionToolCall {
													arguments: String::new(),
													call_id: entry.call_id.clone(),
													namespace: None,
													name: entry.name.clone(),
													caller: None,
													id: Some(entry.item_id.clone()),
													status: Some(OutputStatus::InProgress),
													r#async: None,
												}),
											}),
										));
									}

									if let Some(function) = &tc.function
										&& let Some(args) = &function.arguments
										&& !args.is_empty()
									{
										sequence_number += 1;
										events.push((
											"event",
											ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
												ResponseFunctionCallArgumentsDeltaEvent {
													sequence_number,
													item_id: entry.item_id.clone(),
													output_index: entry.output_index,
													delta: args.clone(),
												},
											),
										));
									}
								}
							}

							if let Some(reason) = &choice.finish_reason {
								pending_stop_reason = Some(*reason);
							}
						}

						if !flushed && pending_stop_reason.is_some() && pending_usage.is_some() {
							flushed = true;
							let response_builder = response_builder
								.as_ref()
								.expect("response builder is set after the first chunk");
							flush_end(
								&mut events,
								&mut sequence_number,
								&mut tool_calls,
								&mut reasoning_items,
								&mut reasoning,
								&mut reasoning_signature,
								&mut pending_stop_reason,
								&mut pending_usage,
								&message_item_id,
								&mut message_output_index,
								&mut next_output_index,
								&message_content,
								&log,
								response_builder,
								&mut completion,
								&mut logged_tool_calls,
							);
						}
					},
				}

				if let Some(namespaces) = &namespaces {
					for (_, event) in &mut events {
						namespaces.restore_event(event);
					}
				}
				events
			},
		);
		parse::sse::end_after_error(body, end)
	}

	fn response_error(error: completions::ChatCompletionError) -> responses::ResponseError {
		let code = match error.r#type.as_deref() {
			Some("rate_limit_error") => responses::ResponseErrorCode::RateLimitExceeded,
			Some("invalid_request_error") => responses::ResponseErrorCode::InvalidPrompt,
			_ => responses::ResponseErrorCode::ServerError,
		};
		responses::ResponseError {
			code,
			message: error.message,
			misalignment: None,
		}
	}

	/// Records the first token time, or the gap since the previous token.
	fn record_token(last_token_at: &mut Option<Instant>, log: &StreamingUsageGuard) {
		let now = Instant::now();
		match last_token_at.replace(now) {
			None => log.update(|r| r.response.first_token = Some(now)),
			Some(prev) => {
				let gap = now.duration_since(prev);
				log.update(|r| r.response.inter_chunk_latencies.record(gap));
			},
		}
	}

	/// Emits the message's `output_item.added` the first time it is needed and returns its index.
	fn open_message(
		events: &mut Vec<(&'static str, responses::ResponseStreamEvent)>,
		sequence_number: &mut u64,
		message_output_index: &mut Option<u32>,
		next_output_index: &mut u32,
		message_item_id: &str,
	) -> u32 {
		*message_output_index.get_or_insert_with(|| {
			let output_index = *next_output_index;
			*next_output_index += 1;
			*sequence_number += 1;
			events.push((
				"event",
				responses::ResponseStreamEvent::ResponseOutputItemAdded(
					responses::ResponseOutputItemAddedEvent {
						sequence_number: *sequence_number,
						output_index,
						item: responses::OutputItem::Message(responses::OutputMessage {
							content: Vec::new(),
							id: message_item_id.to_string(),
							role: responses::AssistantRole::Assistant,
							phase: None,
							status: responses::OutputStatus::InProgress,
						}),
					},
				),
			));
			output_index
		})
	}

	#[allow(clippy::too_many_arguments)]
	fn flush_end(
		events: &mut Vec<(&'static str, responses::ResponseStreamEvent)>,
		sequence_number: &mut u64,
		tool_calls: &mut HashMap<u32, StreamedCall>,
		reasoning_items: &mut Vec<(u32, responses::OutputItem)>,
		reasoning: &mut Option<(String, u32, String)>,
		reasoning_signature: &mut Option<String>,
		pending_stop_reason: &mut Option<completions::FinishReason>,
		pending_usage: &mut Option<completions::Usage>,
		message_item_id: &str,
		message_output_index: &mut Option<u32>,
		next_output_index: &mut u32,
		message_content: &MessageContent,
		log: &StreamingUsageGuard,
		response_builder: &types::responses::ResponseBuilder,
		completion: &mut Option<String>,
		logged_tool_calls: &mut Option<LoggedToolCalls>,
	) {
		use responses::{
			AssistantRole, FunctionToolCall, IncompleteDetails, InputTokenDetails, OutputItem,
			OutputMessage, OutputTokenDetails, ReasoningItem, ReasoningItemContent, ReasoningTextContent,
			ResponseFunctionCallArgumentsDoneEvent, ResponseOutputItemDoneEvent,
			ResponseReasoningTextDoneEvent, ResponseStreamEvent, ResponseUsage,
		};

		let stop_reason = pending_stop_reason.take();
		let usage = pending_usage.take();
		let response_status = match stop_reason.as_ref() {
			Some(completions::FinishReason::Stop)
			| Some(completions::FinishReason::ToolCalls)
			| Some(completions::FinishReason::FunctionCall)
			| None => responses::Status::Completed,
			Some(completions::FinishReason::Length | completions::FinishReason::ContentFilter) => {
				responses::Status::Incomplete
			},
		};
		let item_status = item_status(&response_status);
		let finish_reason = crate::types::serialize_str(&response_status);
		let tool_parts = logged_tool_calls.as_mut().and_then(|logged_tool_calls| {
			crate::conversion::completions::finalize_streaming_tool_calls(
				logged_tool_calls
					.drain()
					.map(|(idx, (id, name, arguments))| (idx, id, name, arguments)),
			)
		});
		let mut tool_parts = tool_parts;
		let mut finish_reason = finish_reason;
		log.update(|r| {
			if let Some(completion) = completion.take() {
				r.response.completion = Some(vec![completion]);
			}
			crate::conversion::completions::build_output_messages(
				&mut r.response,
				tool_parts.take(),
				finish_reason.take(),
			);
		});

		// Finished items, sorted into output order below — what `response.completed` carries.
		let mut output: Vec<(u32, OutputItem)> = std::mem::take(reasoning_items);

		if let Some((item_id, output_index, text)) = reasoning.take() {
			// A signature-only item has no text
			if !text.is_empty() {
				*sequence_number += 1;
				events.push((
					"event",
					ResponseStreamEvent::ResponseReasoningTextDone(ResponseReasoningTextDoneEvent {
						sequence_number: *sequence_number,
						item_id: item_id.clone(),
						output_index,
						content_index: 0,
						text: text.clone(),
					}),
				));
			}
			let item = OutputItem::Reasoning(ReasoningItem {
				id: Some(item_id),
				summary: vec![],
				content: (!text.is_empty()).then(|| {
					vec![ReasoningItemContent::ReasoningText(ReasoningTextContent {
						text,
					})]
				}),
				encrypted_content: reasoning_signature.take(),
				status: Some(item_status),
			});
			*sequence_number += 1;
			events.push((
				"event",
				ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
					sequence_number: *sequence_number,
					output_index,
					item: item.clone(),
				}),
			));
			output.push((output_index, item));
		}

		let message_output_index = open_message(
			events,
			sequence_number,
			message_output_index,
			next_output_index,
			message_item_id,
		);

		let mut sorted_tools: Vec<_> = tool_calls.drain().collect();
		sorted_tools.sort_by_key(|(_, call)| call.output_index);

		for (
			_,
			StreamedCall {
				item_id,
				call_id,
				name,
				arguments: buffer,
				output_index,
			},
		) in sorted_tools
		{
			*sequence_number += 1;
			events.push((
				"event",
				ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
					ResponseFunctionCallArgumentsDoneEvent {
						sequence_number: *sequence_number,
						output_index,
						name: Some(name.clone()),
						item_id: item_id.clone(),
						arguments: buffer.clone(),
					},
				),
			));

			let item = OutputItem::FunctionCall(FunctionToolCall {
				arguments: buffer,
				call_id,
				namespace: None,
				name,
				caller: None,
				id: Some(item_id),
				status: Some(item_status),
				r#async: None,
			});
			*sequence_number += 1;
			events.push((
				"event",
				ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
					sequence_number: *sequence_number,
					output_index,
					item: item.clone(),
				}),
			));
			output.push((output_index, item));
		}

		let content = message_content.finish(
			events,
			sequence_number,
			message_item_id,
			message_output_index,
		);

		let message = OutputItem::Message(OutputMessage {
			content,
			id: message_item_id.to_string(),
			role: AssistantRole::Assistant,
			phase: None,
			status: item_status,
		});
		*sequence_number += 1;
		events.push((
			"event",
			ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
				sequence_number: *sequence_number,
				output_index: message_output_index,
				item: message.clone(),
			}),
		));
		output.push((message_output_index, message));
		output.sort_by_key(|(output_index, _)| *output_index);
		let output: Vec<OutputItem> = output.into_iter().map(|(_, item)| item).collect();

		if let Some(ref u) = usage {
			log.update(|r| {
				r.response.input_tokens = Some(u.prompt_tokens as u64);
				r.response.output_tokens = Some(u.output_tokens() as u64);
				r.response.total_tokens = Some(u.total_tokens as u64);
				r.response.cached_input_tokens = u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens);
				r.response.cache_creation_input_tokens = u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
					.or(u.cache_creation_input_tokens);
				r.response.reasoning_tokens = u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens);
			});
		}

		let usage_obj = usage.map(|u| ResponseUsage {
			input_tokens: u.prompt_tokens,
			output_tokens: u.output_tokens(),
			total_tokens: u.total_tokens,
			input_tokens_details: InputTokenDetails {
				cached_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
					.unwrap_or(0) as u32,
				cache_write_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
					.or(u.cache_creation_input_tokens)
					.map(|tokens| tokens as i64),
			},
			output_tokens_details: OutputTokenDetails {
				reasoning_tokens: u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
					.unwrap_or(0) as u32,
			},
		});

		*sequence_number += 1;
		let mut done_event = match stop_reason.and_then(incomplete_reason) {
			Some(reason) => response_builder.incomplete_event(
				*sequence_number,
				usage_obj,
				IncompleteDetails {
					reason: reason.to_string(),
				},
			),
			None => response_builder.completed_event(*sequence_number, usage_obj),
		};

		match &mut done_event {
			ResponseStreamEvent::ResponseCompleted(e) => e.response.output = output,
			ResponseStreamEvent::ResponseIncomplete(e) => e.response.output = output,
			ResponseStreamEvent::ResponseFailed(e) => e.response.output = output,
			_ => {},
		}

		events.push(("event", done_event));
	}

	/// Keep full content for final items, which clients may use instead of deltas
	#[derive(Default)]
	struct MessageContent {
		text: String,
		text_index: Option<u32>,
		refusal: String,
		refusal_index: Option<u32>,
	}

	impl MessageContent {
		fn text_index(&mut self) -> (u32, bool) {
			let next = self.next_index();
			Self::index(&mut self.text_index, next)
		}

		fn refusal_index(&mut self) -> (u32, bool) {
			let next = self.next_index();
			Self::index(&mut self.refusal_index, next)
		}

		fn next_index(&self) -> u32 {
			u32::from(self.text_index.is_some()) + u32::from(self.refusal_index.is_some())
		}

		fn index(slot: &mut Option<u32>, next: u32) -> (u32, bool) {
			match *slot {
				Some(index) => (index, false),
				None => (*slot.insert(next), true),
			}
		}

		fn finish(
			&self,
			events: &mut Vec<(&'static str, responses::ResponseStreamEvent)>,
			sequence_number: &mut u64,
			item_id: &str,
			output_index: u32,
		) -> Vec<responses::OutputMessageContent> {
			use responses::{
				OutputContent, OutputMessageContent, OutputTextContent, RefusalContent,
				ResponseContentPartDoneEvent, ResponseRefusalDoneEvent, ResponseStreamEvent,
				ResponseTextDoneEvent,
			};

			let mut content = Vec::new();
			if let Some(index) = self.text_index {
				content.push((
					index,
					OutputMessageContent::OutputText(OutputTextContent {
						annotations: Vec::new(),
						logprobs: None,
						text: self.text.clone(),
					}),
				));
			}
			if let Some(index) = self.refusal_index {
				content.push((
					index,
					OutputMessageContent::Refusal(RefusalContent {
						refusal: self.refusal.clone(),
					}),
				));
			}
			content.sort_by_key(|(index, _)| *index);

			for (content_index, part) in &content {
				let content_index = *content_index;
				*sequence_number += 1;
				let (done, part) = match part {
					OutputMessageContent::OutputText(text) => (
						ResponseStreamEvent::ResponseOutputTextDone(ResponseTextDoneEvent {
							sequence_number: *sequence_number,
							item_id: item_id.to_string(),
							output_index,
							content_index,
							text: text.text.clone(),
							logprobs: None,
						}),
						OutputContent::OutputText(text.clone()),
					),
					OutputMessageContent::Refusal(refusal) => (
						ResponseStreamEvent::ResponseRefusalDone(ResponseRefusalDoneEvent {
							sequence_number: *sequence_number,
							item_id: item_id.to_string(),
							output_index,
							content_index,
							refusal: refusal.refusal.clone(),
						}),
						OutputContent::Refusal(refusal.clone()),
					),
				};
				events.push(("event", done));
				*sequence_number += 1;
				events.push((
					"event",
					ResponseStreamEvent::ResponseContentPartDone(ResponseContentPartDoneEvent {
						sequence_number: *sequence_number,
						item_id: item_id.to_string(),
						output_index,
						content_index,
						part,
					}),
				));
			}
			content.into_iter().map(|(_, part)| part).collect()
		}
	}

	/// `item_id` names the output item and its argument events; `call_id` is the upstream call id
	/// the client echoes in `function_call_output`
	struct StreamedCall {
		item_id: String,
		call_id: String,
		name: String,
		arguments: String,
		output_index: u32,
	}

	fn reasoning_item(encrypted_content: String) -> responses::OutputItem {
		responses::OutputItem::Reasoning(responses::ReasoningItem {
			id: Some(format!("rs_{:016x}", rand::rng().random::<u64>())),
			summary: Vec::new(),
			content: None,
			encrypted_content: Some(encrypted_content),
			status: Some(responses::OutputStatus::Completed),
		})
	}

	fn incomplete_reason(finish: completions::FinishReason) -> Option<&'static str> {
		match finish {
			completions::FinishReason::Length => Some("max_output_tokens"),
			completions::FinishReason::ContentFilter => Some("content_filter"),
			completions::FinishReason::Stop
			| completions::FinishReason::ToolCalls
			| completions::FinishReason::FunctionCall => None,
		}
	}

	fn item_status(status: &responses::Status) -> responses::OutputStatus {
		match status {
			responses::Status::Incomplete => responses::OutputStatus::Incomplete,
			_ => responses::OutputStatus::Completed,
		}
	}
}
