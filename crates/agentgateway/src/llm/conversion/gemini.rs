pub mod from_responses {
	use std::collections::HashMap;
	use std::time::Instant;

	use agent_core::strng;
	use bytes::Bytes;
	use rand::RngExt;
	use types::completions::typed as completions;
	use types::responses::typed as responses;

	use crate::http::Body;
	use crate::llm::types::ResponseType;
	use crate::llm::{AIError, AmendOnDrop, types};
	use crate::parse::sse::SseJsonEvent;
	use crate::{json, parse};

	pub fn translate(req: &types::responses::Request) -> Result<Vec<u8>, AIError> {
		let typed =
			json::convert::<_, responses::CreateResponse>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	fn translate_internal(req: responses::CreateResponse) -> completions::Request {
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
						let text = match &msg.content {
							EasyInputContent::Text(text) => text.clone(),
							EasyInputContent::ContentList(parts) => parts
								.iter()
								.filter_map(|p| match p {
									InputContent::InputText(t) => Some(t.text.as_str()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n"),
						};
						messages.push(completions::RequestMessage::User(
							completions::RequestUserMessage {
								content: completions::RequestUserMessageContent::Text(text),
								name: None,
							},
						));
					},
					ResponsesRole::Assistant => {
						let text = match &msg.content {
							EasyInputContent::Text(text) => text.clone(),
							EasyInputContent::ContentList(parts) => parts
								.iter()
								.filter_map(|p| match p {
									InputContent::InputText(t) => Some(t.text.as_str()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n"),
						};
						messages.push(completions::RequestMessage::Assistant(
							completions::RequestAssistantMessage {
								content: Some(completions::RequestAssistantMessageContent::Text(text)),
								..Default::default()
							},
						));
					},
					ResponsesRole::System | ResponsesRole::Developer => {
						let text = match &msg.content {
							EasyInputContent::Text(text) => text.clone(),
							EasyInputContent::ContentList(parts) => parts
								.iter()
								.filter_map(|p| match p {
									InputContent::InputText(t) => Some(t.text.as_str()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n"),
						};
						messages.push(completions::RequestMessage::Developer(
							completions::RequestDeveloperMessage {
								content: completions::RequestDeveloperMessageContent::Text(text),
								name: None,
							},
						));
					},
				},
				InputItem::ItemReference(_) => {
					continue;
				},
				InputItem::Item(item) => match item {
					Item::Message(msg_item) => match msg_item {
						MessageItem::Input(msg) => {
							let text_parts: Vec<String> = msg
								.content
								.iter()
								.filter_map(|c| match c {
									InputContent::InputText(t) => Some(t.text.clone()),
									_ => None,
								})
								.collect();
							let text = text_parts.join("\n");

							match msg.role {
								InputRole::User => {
									messages.push(completions::RequestMessage::User(
										completions::RequestUserMessage {
											content: completions::RequestUserMessageContent::Text(text),
											name: None,
										},
									));
								},
								InputRole::System => {
									messages.push(completions::RequestMessage::System(
										completions::RequestSystemMessage {
											content: completions::RequestSystemMessageContent::Text(text),
											name: None,
										},
									));
								},
								InputRole::Developer => {
									messages.push(completions::RequestMessage::Developer(
										completions::RequestDeveloperMessage {
											content: completions::RequestDeveloperMessageContent::Text(text),
											name: None,
										},
									));
								},
							}
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

							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									content: if text.is_empty() {
										None
									} else {
										Some(completions::RequestAssistantMessageContent::Text(text))
									},
									..Default::default()
								},
							));
						},
					},
					Item::FunctionCall(call) => {
						messages.push(completions::RequestMessage::Assistant(
							completions::RequestAssistantMessage {
								tool_calls: Some(vec![completions::MessageToolCalls::Function(
									completions::MessageToolCall {
										id: call.call_id.clone(),
										function: completions::FunctionCall {
											name: call.name.clone(),
											arguments: call.arguments.clone(),
										},
									},
								)]),
								..Default::default()
							},
						));
					},
					Item::FunctionCallOutput(output) => {
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
								tool_call_id: output.call_id,
							},
						));
					},
					Item::CustomToolCall(call) => {
						let arguments = serde_json::to_string(&call.input).unwrap_or_else(|_| "{}".to_string());
						messages.push(completions::RequestMessage::Assistant(
							completions::RequestAssistantMessage {
								tool_calls: Some(vec![completions::MessageToolCalls::Function(
									completions::MessageToolCall {
										id: call.id.clone(),
										function: completions::FunctionCall {
											name: call.name.clone(),
											arguments,
										},
									},
								)]),
								..Default::default()
							},
						));
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
					_ => {
						// TODO: Log unsupported Items?
						continue;
					},
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
				ToolChoiceParam::Mode(ToolChoiceOptions::Auto) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto),
				),
				ToolChoiceParam::Mode(ToolChoiceOptions::Required) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required),
				),
				ToolChoiceParam::Mode(ToolChoiceOptions::None) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None),
				),
				ToolChoiceParam::Function(ToolChoiceFunction { name }) => Some(
					completions::ToolChoiceOption::Function(completions::NamedToolChoice {
						function: completions::FunctionName { name: name.clone() },
					}),
				),
				ToolChoiceParam::Hosted(_)
				| ToolChoiceParam::AllowedTools(_)
				| ToolChoiceParam::Mcp(_)
				| ToolChoiceParam::Custom(_)
				| ToolChoiceParam::ApplyPatch
				| ToolChoiceParam::Shell => {
					tracing::warn!("Unsupported tool choice for Gemini: {:?}", tc);
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

	pub fn translate_response(bytes: &Bytes, model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		let resp =
			serde_json::from_slice::<completions::Response>(bytes).map_err(AIError::ResponseParsing)?;
		let typed = translate_response_internal(resp, model);
		let mut passthrough =
			json::convert::<_, types::responses::Response>(&typed).map_err(AIError::ResponseParsing)?;
		passthrough.rest = serde_json::Value::Object(serde_json::Map::new());
		if let Some(usage) = passthrough.usage.as_mut() {
			usage.rest = serde_json::Value::Object(serde_json::Map::new());
		}
		Ok(Box::new(passthrough))
	}

	fn translate_response_internal(resp: completions::Response, model: &str) -> responses::Response {
		let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());
		let response_builder = types::responses::ResponseBuilder::new(response_id, model.to_string());

		let choice = resp.choices.into_iter().next();

		let mut outputs: Vec<responses::OutputItem> = Vec::new();
		let mut text_parts: Vec<responses::OutputMessageContent> = Vec::new();
		let mut tool_calls: Vec<responses::OutputItem> = Vec::new();

		if let Some(choice) = &choice {
			if let Some(content) = &choice.message.content {
				text_parts.push(responses::OutputMessageContent::OutputText(
					responses::OutputTextContent {
						annotations: vec![],
						logprobs: None,
						text: content.clone(),
					},
				));
			}

			if let Some(tcs) = &choice.message.tool_calls {
				for tc in tcs {
					match tc {
						completions::MessageToolCalls::Function(f) => {
							tool_calls.push(responses::OutputItem::FunctionCall(
								responses::FunctionToolCall {
									arguments: f.function.arguments.clone(),
									call_id: f.id.clone(),
									name: f.function.name.clone(),
									id: Some(f.id.clone()),
									status: Some(responses::OutputStatus::Completed),
									namespace: None,
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
				status: responses::OutputStatus::Completed,
			}));
		}
		outputs.extend(tool_calls);

		let finish_reason = choice.as_ref().and_then(|c| c.finish_reason.as_ref());

		let status = match finish_reason {
			Some(completions::FinishReason::Stop) | None => responses::Status::Completed,
			Some(completions::FinishReason::Length) => responses::Status::Incomplete,
			Some(completions::FinishReason::ToolCalls)
			| Some(completions::FinishReason::FunctionCall) => responses::Status::Completed,
			Some(completions::FinishReason::ContentFilter) => responses::Status::Failed,
		};

		let incomplete_details = match finish_reason {
			Some(completions::FinishReason::Length) => Some(responses::IncompleteDetails {
				reason: "max_tokens".to_string(),
			}),
			_ => None,
		};

		let error = match finish_reason {
			Some(completions::FinishReason::ContentFilter) => Some(responses::ErrorObject {
				code: "content_filter".to_string(),
				message: "Content filtered".to_string(),
			}),
			_ => None,
		};

		let usage = resp.usage.map(|u| responses::ResponseUsage {
			input_tokens: u.prompt_tokens,
			output_tokens: usage_output_tokens(&u),
			total_tokens: u.total_tokens,
			input_tokens_details: responses::InputTokenDetails {
				cached_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
					.unwrap_or(0) as u32,
			},
			output_tokens_details: responses::OutputTokenDetails {
				reasoning_tokens: u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
					.unwrap_or(0) as u32,
			},
		});

		let mut response = response_builder.response(status, usage, error, incomplete_details);
		response.output = outputs;
		response
	}

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		let res = super::super::completions::parse_google_error(bytes)?;
		let m = completions::ChatCompletionErrorResponse {
			event_id: None,
			error: completions::ChatCompletionError {
				r#type: Some(super::super::completions::google_error_type(&res.error).to_string()),
				message: res.error.message.clone(),
				param: None,
				code: res.error.status.clone(),
				event_id: None,
			},
		};
		Ok(Bytes::from(
			serde_json::to_vec(&m).map_err(AIError::ResponseMarshal)?,
		))
	}

	pub fn translate_stream(b: Body, buffer_limit: usize, log: AmendOnDrop) -> Body {
		use responses::{
			AssistantRole, FunctionToolCall, OutputContent, OutputItem, OutputMessage, OutputStatus,
			OutputTextContent, ResponseContentPartAddedEvent, ResponseFunctionCallArgumentsDeltaEvent,
			ResponseOutputItemAddedEvent, ResponseStreamEvent, ResponseTextDeltaEvent,
		};

		let mut saw_token = false;
		let mut sent_created = false;
		let mut sent_content_part = false;
		let mut flushed = false;

		let mut sequence_number: u64 = 0;
		let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());
		let message_item_id = format!("msg_{:016x}", rand::rng().random::<u64>());
		let model_holder: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());

		let mut next_output_index: u32 = 1;
		let mut tool_calls: HashMap<u32, (String, String, String, u32)> = HashMap::new();
		let mut pending_stop_reason: Option<completions::FinishReason> = None;
		let mut pending_usage: Option<completions::Usage> = None;

		parse::sse::json_transform_multi::<completions::StreamResponse, ResponseStreamEvent, _>(
			b,
			buffer_limit,
			move |evt| {
				let mut events: Vec<(&'static str, ResponseStreamEvent)> = Vec::new();

				match evt {
					SseJsonEvent::Done => {
						if !flushed {
							flushed = true;
							flush_end(
								&mut events,
								&mut sequence_number,
								&mut tool_calls,
								&mut pending_stop_reason,
								&mut pending_usage,
								&message_item_id,
								&sent_content_part,
								&log,
								&response_id,
								&model_holder.borrow(),
							);
						}
						return events;
					},
					SseJsonEvent::Data(Err(e)) => {
						tracing::warn!(
							"Failed to parse Gemini stream response during translation: {}",
							e
						);
						return events;
					},
					SseJsonEvent::Data(Ok(chunk)) => {
						if !sent_created {
							sent_created = true;
							*model_holder.borrow_mut() = chunk.model.clone();

							let response_builder =
								types::responses::ResponseBuilder::new(response_id.clone(), chunk.model.clone());

							sequence_number += 1;
							events.push(("event", response_builder.created_event(sequence_number)));

							sequence_number += 1;
							events.push((
								"event",
								ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
									sequence_number,
									output_index: 0,
									item: OutputItem::Message(OutputMessage {
										content: Vec::new(),
										id: message_item_id.clone(),
										role: AssistantRole::Assistant,
										phase: None,
										status: OutputStatus::InProgress,
									}),
								}),
							));

							log.non_atomic_mutate(|r| {
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
							if let Some(content) = &choice.delta.content {
								if !sent_content_part {
									sent_content_part = true;
									sequence_number += 1;
									events.push((
										"event",
										ResponseStreamEvent::ResponseContentPartAdded(ResponseContentPartAddedEvent {
											sequence_number,
											item_id: message_item_id.clone(),
											output_index: 0,
											content_index: 0,
											part: OutputContent::OutputText(OutputTextContent {
												text: String::new(),
												annotations: Vec::new(),
												logprobs: None,
											}),
										}),
									));
								}

								if !saw_token {
									saw_token = true;
									log.non_atomic_mutate(|r| {
										r.response.first_token = Some(Instant::now());
									});
								}

								sequence_number += 1;
								events.push((
									"event",
									ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
										sequence_number,
										item_id: message_item_id.clone(),
										output_index: 0,
										content_index: 0,
										delta: content.clone(),
										logprobs: None,
									}),
								));
							}

							if let Some(tcs) = &choice.delta.tool_calls {
								for tc in tcs {
									let tool_index = tc.index;

									let is_new = !tool_calls.contains_key(&tool_index);

									let entry = tool_calls.entry(tool_index).or_insert_with(|| {
										let item_id = format!("call_{:016x}", rand::rng().random::<u64>());
										let output_index = next_output_index;
										next_output_index += 1;
										(item_id, String::new(), String::new(), output_index)
									});

									if let Some(function) = &tc.function {
										if let Some(name) = &function.name {
											entry.1 = name.clone();
										}
										if let Some(args) = &function.arguments {
											entry.2.push_str(args);
										}
									}

									if is_new {
										if !saw_token {
											saw_token = true;
											log.non_atomic_mutate(|r| {
												r.response.first_token = Some(Instant::now());
											});
										}

										sequence_number += 1;
										events.push((
											"event",
											ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
												sequence_number,
												output_index: entry.3,
												item: OutputItem::FunctionCall(FunctionToolCall {
													arguments: String::new(),
													call_id: entry.0.clone(),
													namespace: None,
													name: entry.1.clone(),
													id: Some(entry.0.clone()),
													status: Some(OutputStatus::InProgress),
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
													item_id: entry.0.clone(),
													output_index: entry.3,
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
							flush_end(
								&mut events,
								&mut sequence_number,
								&mut tool_calls,
								&mut pending_stop_reason,
								&mut pending_usage,
								&message_item_id,
								&sent_content_part,
								&log,
								&response_id,
								&model_holder.borrow(),
							);
						}
					},
				}

				events
			},
		)
	}

	fn usage_output_tokens(usage: &completions::Usage) -> u32 {
		if usage.completion_tokens == 0 && usage.total_tokens > 0 {
			return usage.total_tokens.saturating_sub(usage.prompt_tokens);
		}
		usage.completion_tokens
	}

	#[allow(clippy::too_many_arguments)]
	fn flush_end(
		events: &mut Vec<(&'static str, responses::ResponseStreamEvent)>,
		sequence_number: &mut u64,
		tool_calls: &mut HashMap<u32, (String, String, String, u32)>,
		pending_stop_reason: &mut Option<completions::FinishReason>,
		pending_usage: &mut Option<completions::Usage>,
		message_item_id: &str,
		sent_content_part: &bool,
		log: &AmendOnDrop,
		response_id: &str,
		model: &str,
	) {
		use responses::{
			AssistantRole, ErrorObject, FunctionToolCall, IncompleteDetails, InputTokenDetails,
			OutputContent, OutputItem, OutputMessage, OutputStatus, OutputTextContent,
			OutputTokenDetails, ResponseContentPartDoneEvent, ResponseFunctionCallArgumentsDoneEvent,
			ResponseOutputItemDoneEvent, ResponseStreamEvent, ResponseUsage,
		};

		let stop_reason = pending_stop_reason.take();
		let usage = pending_usage.take();

		let mut sorted_tools: Vec<_> = tool_calls.drain().collect();
		sorted_tools.sort_by_key(|(_, (_, _, _, output_index))| *output_index);

		for (_, (item_id, name, buffer, output_index)) in sorted_tools {
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

			*sequence_number += 1;
			events.push((
				"event",
				ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
					sequence_number: *sequence_number,
					output_index,
					item: OutputItem::FunctionCall(FunctionToolCall {
						arguments: buffer,
						call_id: item_id.clone(),
						namespace: None,
						name,
						id: Some(item_id),
						status: Some(OutputStatus::Completed),
					}),
				}),
			));
		}

		if *sent_content_part {
			*sequence_number += 1;
			events.push((
				"event",
				ResponseStreamEvent::ResponseContentPartDone(ResponseContentPartDoneEvent {
					sequence_number: *sequence_number,
					item_id: message_item_id.to_string(),
					output_index: 0,
					content_index: 0,
					part: OutputContent::OutputText(OutputTextContent {
						annotations: Vec::new(),
						logprobs: None,
						text: String::new(),
					}),
				}),
			));
		}

		*sequence_number += 1;
		events.push((
			"event",
			ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
				sequence_number: *sequence_number,
				output_index: 0,
				item: OutputItem::Message(OutputMessage {
					content: Vec::new(),
					id: message_item_id.to_string(),
					role: AssistantRole::Assistant,
					phase: None,
					status: OutputStatus::Completed,
				}),
			}),
		));

		if let Some(ref u) = usage {
			log.non_atomic_mutate(|r| {
				r.response.input_tokens = Some(u.prompt_tokens as u64);
				r.response.output_tokens = Some(usage_output_tokens(u) as u64);
				r.response.total_tokens = Some(u.total_tokens as u64);
				r.response.cached_input_tokens = u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens);
				r.response.reasoning_tokens = u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens);
			});
		}

		let usage_obj = usage.map(|u| ResponseUsage {
			input_tokens: u.prompt_tokens,
			output_tokens: usage_output_tokens(&u),
			total_tokens: u.total_tokens,
			input_tokens_details: InputTokenDetails {
				cached_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
					.unwrap_or(0) as u32,
			},
			output_tokens_details: OutputTokenDetails {
				reasoning_tokens: u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
					.unwrap_or(0) as u32,
			},
		});

		let response_builder =
			types::responses::ResponseBuilder::new(response_id.to_string(), model.to_string());

		*sequence_number += 1;
		let done_event = match stop_reason {
			Some(completions::FinishReason::Stop)
			| Some(completions::FinishReason::ToolCalls)
			| Some(completions::FinishReason::FunctionCall)
			| None => response_builder.completed_event(*sequence_number, usage_obj),
			Some(completions::FinishReason::Length) => response_builder.incomplete_event(
				*sequence_number,
				usage_obj,
				IncompleteDetails {
					reason: "max_tokens".to_string(),
				},
			),
			Some(completions::FinishReason::ContentFilter) => response_builder.failed_event(
				*sequence_number,
				usage_obj,
				ErrorObject {
					code: "content_filter".to_string(),
					message: "Content filtered".to_string(),
				},
			),
		};

		events.push(("event", done_event));
	}
}
