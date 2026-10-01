#[cfg(test)]
#[path = "openai_compat_tests.rs"]
mod tests;

#[path = "openai_compat_stream.rs"]
mod stream;
#[path = "openai_compat_tools.rs"]
mod tools;
pub use tools::ResponseToolMap;

pub mod from_responses {
	use agent_core::strng;
	use types::completions::typed as completions;
	use types::responses::typed as responses;

	use crate::{AIError, types};

	/// Translate an OpenAI Responses request into an OpenAI-compatible chat completions request.
	pub fn translate(req: &types::responses::Request) -> Result<Vec<u8>, AIError> {
		let xlated = translate_request(req)?;
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub fn translate_request(
		req: &types::responses::Request,
	) -> Result<types::completions::typed::Request, AIError> {
		translate_request_with_context(req).map(|(request, _)| request)
	}

	pub fn translate_request_with_context(
		req: &types::responses::Request,
	) -> Result<(completions::Request, super::ResponseToolMap), AIError> {
		let mut raw = serde_json::to_value(req).map_err(AIError::RequestMarshal)?;
		let tools = super::ResponseToolMap::normalize_request(&mut raw)?;
		validate_input_media(&raw)?;
		let typed =
			serde_json::from_value::<responses::CreateResponse>(raw).map_err(AIError::RequestMarshal)?;
		Ok((translate_internal(typed), tools))
	}

	fn validate_input_media(raw: &serde_json::Value) -> Result<(), AIError> {
		let Some(items) = raw.get("input").and_then(serde_json::Value::as_array) else {
			return Ok(());
		};
		for item in items {
			let Some(parts) = item.get("content").and_then(serde_json::Value::as_array) else {
				continue;
			};
			for part in parts {
				match part.get("type").and_then(serde_json::Value::as_str) {
					Some("input_image")
						if part
							.get("image_url")
							.and_then(serde_json::Value::as_str)
							.is_none() =>
					{
						return Err(AIError::UnsupportedConversion(strng::literal!(
							"Responses file-backed images cannot be represented by chat completions"
						)));
					},
					Some("input_file") => {
						return Err(AIError::UnsupportedConversion(strng::literal!(
							"Responses file inputs cannot be represented by chat completions"
						)));
					},
					_ => {},
				}
			}
		}
		Ok(())
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
														prompt_cache_breakpoint: text.prompt_cache_breakpoint,
													},
												))
											},
											InputContent::InputImage(image) => {
												let url = image.image_url?;
												Some(completions::RequestUserMessageContentPart::ImageUrl(
													completions::RequestMessageContentPartImage {
														image_url: completions::ImageUrl {
															url,
															detail: Some(image.detail),
														},
														prompt_cache_breakpoint: None,
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
														prompt_cache_breakpoint: text.prompt_cache_breakpoint,
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
														prompt_cache_breakpoint: text.prompt_cache_breakpoint,
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
					Item::Reasoning(reasoning) => {
						let text = reasoning
							.content
							.unwrap_or_default()
							.into_iter()
							.map(|part| {
								let async_openai::types::responses::ReasoningItemContent::ReasoningText(text) =
									part;
								text.text
							})
							.collect::<String>();
						if !text.is_empty() {
							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									reasoning_content: Some(text),
									..Default::default()
								},
							));
						}
					},
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
																prompt_cache_breakpoint: text.prompt_cache_breakpoint,
															},
														))
													},
													InputContent::InputImage(image) => {
														let url = image.image_url?;
														Some(completions::RequestUserMessageContentPart::ImageUrl(
															completions::RequestMessageContentPartImage {
																image_url: completions::ImageUrl {
																	url,
																	detail: Some(image.detail),
																},
																prompt_cache_breakpoint: None,
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
																prompt_cache_breakpoint: text.prompt_cache_breakpoint,
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
																prompt_cache_breakpoint: text.prompt_cache_breakpoint,
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
						let tool_call = completions::MessageToolCalls::Function(completions::MessageToolCall {
							id: call.call_id.clone(),
							function: completions::FunctionCall {
								name: call.name.clone(),
								arguments: call.arguments.clone(),
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
					_ => continue,
				},
			}
		}

		// Responses emits separate reasoning/message/tool items for one assistant turn.
		// Chat Completions reasoning providers require them on the same message.
		let mut merged = Vec::new();
		for message in messages {
			if let completions::RequestMessage::Assistant(mut next) = message {
				if let Some(completions::RequestMessage::Assistant(previous)) = merged.last_mut()
					&& (previous.content.is_none() || next.content.is_none())
				{
					if next.content.is_some() {
						previous.content = next.content.take();
					}
					if let Some(reasoning) = next.reasoning_content {
						previous
							.reasoning_content
							.get_or_insert_default()
							.push_str(&reasoning);
					}
					if let Some(calls) = next.tool_calls {
						previous.tool_calls.get_or_insert_default().extend(calls);
					}
				} else {
					merged.push(completions::RequestMessage::Assistant(next));
				}
			} else {
				merged.push(message);
			}
		}
		let messages = merged;

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
				| ToolChoiceParam::ProgrammaticToolCalling(_)
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
	use axum_core::body::Body;
	use bytes::Bytes;
	use rand::RngExt;
	use types::completions::typed as completions;
	use types::responses::typed as responses;

	use crate::types::ResponseType;
	use crate::{AIError, StreamingUsageGuard, json, logged_response_parsing, types};

	/// Translate an OpenAI-compatible chat completions response into an OpenAI Responses response.
	pub fn translate_response(bytes: &Bytes, model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		translate_response_with_context(bytes, model, &super::ResponseToolMap::default())
	}

	pub fn translate_response_with_context(
		bytes: &Bytes,
		model: &str,
		tools: &super::ResponseToolMap,
	) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<completions::Response>(bytes)
			.map_err(logged_response_parsing(bytes))?;
		let typed = translate_response_internal(resp, model);
		let mut raw = serde_json::to_value(typed).map_err(AIError::ResponseParsing)?;
		for item in raw["output"].as_array_mut().into_iter().flatten() {
			if item["type"] == "function_call" {
				tools.restore_call(item)?;
			}
		}
		let passthrough =
			json::convert::<_, types::responses::Response>(&raw).map_err(AIError::ResponseParsing)?;
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
			if let Some(reasoning) = &choice.message.reasoning_content {
				outputs.push(
					serde_json::from_value(serde_json::json!({
						"type": "reasoning", "id": format!("rs_{:016x}", rand::rng().random::<u64>()),
						"summary": [], "content": [{"type": "reasoning_text", "text": reasoning}]
					}))
					.expect("reasoning item is valid"),
				);
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

			if let Some(tcs) = &choice.message.tool_calls {
				for tc in tcs {
					match tc {
						completions::MessageToolCalls::Function(f) => {
							tool_calls.push(responses::OutputItem::FunctionCall(
								responses::FunctionToolCall {
									arguments: f.function.arguments.clone(),
									call_id: f.id.clone(),
									name: f.function.name.clone(),
									caller: None,
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
				cache_write_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
					.or(u.cache_creation_input_tokens)
					.map(|tokens| tokens as u32),
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

	pub fn translate_stream(
		body: Body,
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
	) -> Body {
		translate_stream_with_context(
			body,
			buffer_limit,
			log,
			log_content,
			super::ResponseToolMap::default(),
		)
	}

	pub fn translate_stream_with_context(
		body: Body,
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
		tools: super::ResponseToolMap,
	) -> Body {
		super::stream::translate(body, buffer_limit, log, log_content, tools)
	}

	pub(super) fn usage_output_tokens(usage: &completions::Usage) -> u32 {
		if usage.completion_tokens == 0 && usage.total_tokens > 0 {
			return usage.total_tokens.saturating_sub(usage.prompt_tokens);
		}
		usage.completion_tokens
	}
}
