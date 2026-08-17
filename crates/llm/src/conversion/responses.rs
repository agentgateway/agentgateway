use std::collections::BTreeMap;
use std::time::Instant;

use agent_core::strng::{self, Strng};
use axum_core::body::Body;
use serde::Deserialize;

use crate::types::detect;
use crate::{OutputMessage, OutputMessagePart, StreamingUsageGuard, parse, types};

#[allow(clippy::large_enum_variant)] // The large variant is used 99% of the time so just always use it.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum StreamResponse {
	// Attempt to parse it properly
	Typed(types::responses::typed::ResponseStreamEvent),
	// Fallback to detect mode. This is useful if the provider breaks the spec (which happens with custom
	// providers but also OpenAI themselves, often)
	Raw(detect::StreamResponse),
}

pub fn passthrough_stream(
	b: Body,
	buffer_limit: usize,
	mut log: StreamingUsageGuard,
	log_content: crate::LogContentFields,
) -> Body {
	let mut saw_token = false;
	let mut completion = log_content.completion.then(String::new);
	let mut tool_calls = log_content.tool_calls.then(BTreeMap::new);
	parse::sse::json_passthrough::<StreamResponse>(b, buffer_limit, move |event| {
		let Some(Ok(event)) = event else {
			// Stream ended ([DONE]): flush completion if not already set via ResponseCompleted
			if event.is_none() {
				log.update(|r| {
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
					if r.response.output_messages.is_none() {
						r.response.output_messages = take_output_messages(&mut tool_calls, None);
					}
				});
			}
			return;
		};
		let event = match event {
			StreamResponse::Typed(e) => e,
			StreamResponse::Raw(raw) => {
				detect::amend_from_stream_response(&mut log, &raw);
				return;
			},
		};
		match event {
			types::responses::typed::ResponseStreamEvent::ResponseCreated(created) => {
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&created.response.model));
					r.response.service_tier = created
						.response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str);
					if let Some(usage) = &created.response.usage {
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
						r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
						r.response.cache_creation_input_tokens = usage
							.input_tokens_details
							.cache_write_tokens
							.map(|tokens| tokens as u64);
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
				});
			},
			types::responses::typed::ResponseStreamEvent::ResponseOutputTextDelta(ref delta) => {
				if !saw_token {
					saw_token = true;
					log.update(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}
				if let Some(c) = completion.as_mut() {
					c.push_str(&delta.delta);
				}
			},
			types::responses::typed::ResponseStreamEvent::ResponseOutputItemDone(done) => {
				if let Some(tool_calls) = tool_calls.as_mut()
					&& let Some(part) = types::responses::output_item_tool_call_part(&done.item)
				{
					tool_calls.insert(done.output_index, part);
				}
			},
			types::responses::typed::ResponseStreamEvent::ResponseCompleted(completed) => {
				let finish_reason = types::serialize_str(&completed.response.status);
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&completed.response.model));
					r.response.service_tier = completed
						.response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str);
					if let Some(usage) = &completed.response.usage {
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
						r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
						r.response.cache_creation_input_tokens = usage
							.input_tokens_details
							.cache_write_tokens
							.map(|tokens| tokens as u64);
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
					r.response.output_messages = take_output_messages(&mut tool_calls, finish_reason.clone());
				});
			},
			_ => {},
		}
	})
}

fn take_output_messages(
	tool_calls: &mut Option<BTreeMap<u32, OutputMessagePart>>,
	finish_reason: Option<Strng>,
) -> Option<Vec<OutputMessage>> {
	let content: Vec<_> = std::mem::take(tool_calls.as_mut()?).into_values().collect();
	(!content.is_empty()).then(|| {
		vec![OutputMessage {
			role: strng::literal!("assistant"),
			content,
			finish_reason,
		}]
	})
}

pub mod from_messages {
	use std::collections::{BTreeMap, HashMap, HashSet};
	use std::time::Instant;

	use agent_core::strng;
	use axum_core::body::Body;
	use bytes::Bytes;
	use rand::RngExt;
	use serde_json::{Map, Value, json};
	use types::messages::typed as messages;
	use types::responses::typed as responses;

	use crate::parse::sse::SseJsonEvent;
	use crate::types::ResponseType;
	use crate::{
		AIError, OutputMessagePart, StreamingUsageGuard, json as json_util, logged_response_parsing,
		parse, types,
	};

	pub fn translate(req: &types::messages::Request) -> Result<Vec<u8>, AIError> {
		validate_raw_request(req)?;
		let typed = json_util::convert::<_, messages::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed)?;
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	fn translate_internal(req: messages::Request) -> Result<types::responses::Request, AIError> {
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

		// Responses has no direct stop_sequences/top_k equivalent; these are
		// accepted and dropped rather than failing the conversion (see #2662).
		let _ = (stop_sequences, top_k);

		let (instructions, mut input) = translate_system_prompt(system)?;
		let mut rest = Map::new();
		if let Some(instructions) = instructions.filter(|s| !s.is_empty()) {
			rest.insert("instructions".to_string(), Value::String(instructions));
		}

		let output_config = output_config.unwrap_or_default();
		if let Some(reasoning) = translate_reasoning(thinking, output_config.effort)? {
			rest.insert(
				"reasoning".to_string(),
				serde_json::to_value(reasoning).map_err(AIError::RequestMarshal)?,
			);
		}
		if let Some(text) = translate_output_format(output_config.format) {
			rest.insert("text".to_string(), text);
		}

		if let Some(metadata) = metadata.filter(|m| !m.fields.is_empty()) {
			let metadata_fields = metadata.fields.into_iter().collect::<BTreeMap<_, _>>();
			rest.insert(
				"metadata".to_string(),
				serde_json::to_value(metadata_fields).map_err(AIError::RequestMarshal)?,
			);
		}

		let tool_values = translate_tools(tools)?;
		if !tool_values.is_empty() {
			rest.insert("tools".to_string(), Value::Array(tool_values));
		}
		if let Some((choice, parallel_tool_calls)) = translate_tool_choice(tool_choice)? {
			rest.insert("tool_choice".to_string(), choice);
			if let Some(parallel_tool_calls) = parallel_tool_calls {
				rest.insert(
					"parallel_tool_calls".to_string(),
					Value::Bool(parallel_tool_calls),
				);
			}
		}

		for msg in messages {
			translate_message(msg, &mut input)?;
		}

		let max_output_tokens = u32::try_from(max_tokens).map_err(|_| {
			AIError::UnsupportedConversion(strng::literal!(
				"messages max_tokens exceeds responses max_output_tokens range"
			))
		})?;

		Ok(types::responses::Request {
			input: types::responses::RequestInput::Items(input),
			model: Some(model),
			moderation: None,
			max_output_tokens: Some(max_output_tokens),
			temperature,
			top_p,
			stream: Some(stream),
			instructions: None,
			vendor_extensions: None,
			rest: Value::Object(rest),
		})
	}

	fn translate_system_prompt(
		system: Option<messages::SystemPrompt>,
	) -> Result<(Option<String>, Vec<types::responses::RawInputItem>), AIError> {
		let Some(system) = system else {
			return Ok((None, Vec::new()));
		};
		match system {
			messages::SystemPrompt::Text(text) => Ok((Some(text), Vec::new())),
			messages::SystemPrompt::Blocks(blocks) => {
				let has_cache_control = blocks.iter().any(|block| match block {
					messages::SystemContentBlock::Text { cache_control, .. } => cache_control.is_some(),
				});
				if has_cache_control {
					let mut parts = Vec::new();
					for block in blocks {
						match block {
							messages::SystemContentBlock::Text {
								text,
								cache_control,
							} => {
								let mut part = json!({
									"type": "input_text",
									"text": text,
								});
								add_prompt_cache_breakpoint(&mut part, cache_control);
								parts.push(part);
							},
						}
					}
					let mut input = Vec::new();
					flush_input_message("system", &mut parts, &mut input);
					return Ok((None, input));
				}

				let mut out = Vec::with_capacity(blocks.len());
				for block in blocks {
					match block {
						messages::SystemContentBlock::Text { text, .. } => out.push(text),
					}
				}
				Ok((Some(out.join("\n")), Vec::new()))
			},
		}
	}

	fn add_prompt_cache_breakpoint(
		value: &mut Value,
		cache_control: Option<messages::CacheControlEphemeral>,
	) {
		if cache_control.is_some()
			&& let Some(object) = value.as_object_mut()
		{
			object
				.entry("prompt_cache_breakpoint")
				.or_insert_with(|| json!({"mode": "explicit"}));
		}
	}

	fn translate_reasoning(
		thinking: Option<messages::ThinkingInput>,
		effort: Option<messages::ThinkingEffort>,
	) -> Result<Option<responses::Reasoning>, AIError> {
		match thinking {
			Some(messages::ThinkingInput::Adaptive {}) => Ok(Some(responses::Reasoning {
				context: None,
				effort: translate_effort(effort),
				mode: None,
				summary: None,
			})),
			Some(messages::ThinkingInput::Disabled {}) => {
				if effort.is_some() {
					unsupported("messages output_config.effort requires adaptive thinking")
				} else {
					Ok(None)
				}
			},
			Some(messages::ThinkingInput::Enabled { .. }) => Ok(Some(responses::Reasoning {
				context: None,
				effort: translate_effort(effort),
				mode: None,
				summary: None,
			})),
			None => {
				if effort.is_some() {
					unsupported("messages output_config.effort requires adaptive thinking")
				} else {
					Ok(None)
				}
			},
		}
	}

	fn translate_effort(
		effort: Option<messages::ThinkingEffort>,
	) -> Option<responses::ReasoningEffort> {
		Some(match effort {
			Some(messages::ThinkingEffort::Low) => responses::ReasoningEffort::Low,
			Some(messages::ThinkingEffort::Medium) => responses::ReasoningEffort::Medium,
			Some(messages::ThinkingEffort::High) => responses::ReasoningEffort::High,
			Some(messages::ThinkingEffort::Xhigh) => responses::ReasoningEffort::Xhigh,
			Some(messages::ThinkingEffort::Max) => responses::ReasoningEffort::Max,
			None => responses::ReasoningEffort::High,
		})
	}

	fn translate_output_format(format: Option<messages::OutputFormat>) -> Option<Value> {
		format.map(|format| match format {
			messages::OutputFormat::JsonSchema { schema } => json!({
				"format": {
					"type": "json_schema",
					"name": "structured_output",
					"schema": schema,
				}
			}),
		})
	}

	fn translate_tools(tools: Option<Vec<messages::Tool>>) -> Result<Vec<Value>, AIError> {
		let mut out = Vec::new();
		for tool in tools.into_iter().flatten() {
			match tool {
				messages::Tool::Custom(tool) => {
					let mut value = Map::new();
					value.insert("type".to_string(), Value::String("function".to_string()));
					value.insert("name".to_string(), Value::String(tool.name));
					if let Some(description) = tool.description {
						value.insert("description".to_string(), Value::String(description));
					}
					value.insert("parameters".to_string(), tool.input_schema);
					out.push(Value::Object(value));
				},
				// OpenAI Responses has no equivalent of an Anthropic server-executed
				// tool (e.g. web_search_20250305); drop it rather than fail the whole request.
				messages::Tool::Server(tool) => {
					tracing::warn!(
						"Unsupported server tool in responses conversion: {:?}",
						tool
					);
				},
			}
		}
		Ok(out)
	}

	fn translate_tool_choice(
		choice: Option<messages::ToolChoice>,
	) -> Result<Option<(Value, Option<bool>)>, AIError> {
		let Some(choice) = choice else {
			return Ok(None);
		};
		let mut parallel_tool_calls = None;
		let value = match choice {
			messages::ToolChoice::Auto {
				disable_parallel_tool_use,
			} => {
				parallel_tool_calls = disable_parallel_tool_use.map(|disable| !disable);
				Value::String("auto".to_string())
			},
			messages::ToolChoice::Any {
				disable_parallel_tool_use,
			} => {
				parallel_tool_calls = disable_parallel_tool_use.map(|disable| !disable);
				Value::String("required".to_string())
			},
			messages::ToolChoice::Tool {
				name,
				disable_parallel_tool_use,
			} => {
				parallel_tool_calls = disable_parallel_tool_use.map(|disable| !disable);
				json!({
					"type": "function",
					"name": name,
				})
			},
			messages::ToolChoice::None {} => Value::String("none".to_string()),
		};
		Ok(Some((value, parallel_tool_calls)))
	}

	fn translate_message(
		msg: messages::Message,
		out: &mut Vec<types::responses::RawInputItem>,
	) -> Result<(), AIError> {
		match msg.role {
			messages::Role::User => translate_user_message(msg.content, out),
			messages::Role::Assistant => translate_assistant_message(msg.content, out),
			messages::Role::System => translate_system_message(msg.content, out),
		}
	}

	fn translate_user_message(
		content: Vec<messages::ContentBlock>,
		out: &mut Vec<types::responses::RawInputItem>,
	) -> Result<(), AIError> {
		let mut parts = Vec::new();
		for block in content {
			match block {
				messages::ContentBlock::Text(text) => {
					validate_text_block(&text)?;
					let mut part = json!({
						"type": "input_text",
						"text": text.text,
					});
					add_prompt_cache_breakpoint(&mut part, text.cache_control);
					parts.push(part);
				},
				messages::ContentBlock::Image(image) => {
					let mut part = translate_image_source(&image.source)?;
					add_prompt_cache_breakpoint(&mut part, image.cache_control);
					parts.push(part);
				},
				messages::ContentBlock::ToolResult {
					tool_use_id,
					content,
					cache_control,
					is_error,
				} => {
					flush_input_message("user", &mut parts, out);
					let status = if is_error.unwrap_or_default() {
						"incomplete"
					} else {
						"completed"
					};
					let output = translate_tool_result_content(content, cache_control)?;
					out.push(types::responses::RawInputItem::from_value(json!({
						"type": "function_call_output",
						"call_id": tool_use_id,
						"output": output,
						"status": status,
					})));
				},
				messages::ContentBlock::Document(_)
				| messages::ContentBlock::SearchResult(_)
				| messages::ContentBlock::ServerToolUse { .. }
				| messages::ContentBlock::WebSearchToolResult { .. }
				| messages::ContentBlock::Thinking { .. }
				| messages::ContentBlock::RedactedThinking { .. }
				| messages::ContentBlock::ToolUse { .. }
				| messages::ContentBlock::Unknown => {
					return unsupported("messages user content block cannot be represented by responses");
				},
			}
		}
		flush_input_message("user", &mut parts, out);
		Ok(())
	}

	fn translate_assistant_message(
		content: Vec<messages::ContentBlock>,
		out: &mut Vec<types::responses::RawInputItem>,
	) -> Result<(), AIError> {
		let mut text_parts = Vec::new();
		for block in content {
			match block {
				messages::ContentBlock::Text(text) => {
					validate_text_block(&text)?;
					text_parts.push(json!({
						"type": "output_text",
						"text": text.text,
						"annotations": [],
					}));
				},
				messages::ContentBlock::ToolUse {
					id,
					name,
					input,
					cache_control: _,
				} => {
					flush_output_message(&mut text_parts, out);
					let arguments = serde_json::to_string(&input).map_err(AIError::RequestMarshal)?;
					out.push(types::responses::RawInputItem::from_value(json!({
						"type": "function_call",
						"id": id,
						"call_id": id,
						"name": name,
						"arguments": arguments,
						"status": "completed",
					})));
				},
				messages::ContentBlock::Thinking { .. }
				| messages::ContentBlock::RedactedThinking { .. } => {
					return unsupported("messages thinking history cannot be represented by responses");
				},
				_ => {
					return unsupported(
						"messages assistant content block cannot be represented by responses",
					);
				},
			}
		}
		flush_output_message(&mut text_parts, out);
		Ok(())
	}

	fn translate_system_message(
		content: Vec<messages::ContentBlock>,
		out: &mut Vec<types::responses::RawInputItem>,
	) -> Result<(), AIError> {
		let mut parts = Vec::new();
		for block in content {
			match block {
				messages::ContentBlock::Text(text) => {
					validate_text_block(&text)?;
					let mut part = json!({
						"type": "input_text",
						"text": text.text,
					});
					add_prompt_cache_breakpoint(&mut part, text.cache_control);
					parts.push(part);
				},
				_ => {
					return unsupported("messages system content block cannot be represented by responses");
				},
			}
		}
		flush_input_message("system", &mut parts, out);
		Ok(())
	}

	fn flush_input_message(
		role: &'static str,
		parts: &mut Vec<Value>,
		out: &mut Vec<types::responses::RawInputItem>,
	) {
		if parts.is_empty() {
			return;
		}
		out.push(types::responses::RawInputItem::from_value(json!({
			"type": "message",
			"role": role,
			"content": std::mem::take(parts),
		})));
	}

	fn flush_output_message(parts: &mut Vec<Value>, out: &mut Vec<types::responses::RawInputItem>) {
		if parts.is_empty() {
			return;
		}
		out.push(types::responses::RawInputItem::from_value(json!({
			"type": "message",
			"id": format!("msg_{:016x}", rand::rng().random::<u64>()),
			"role": "assistant",
			"content": std::mem::take(parts),
			"status": "completed",
		})));
	}

	fn validate_text_block(text: &messages::ContentTextBlock) -> Result<(), AIError> {
		reject_option(
			&text.citations,
			"messages text citations cannot be represented by responses",
		)
	}

	fn translate_image_source(source: &Value) -> Result<Value, AIError> {
		let source_type = source.get("type").and_then(Value::as_str).ok_or_else(|| {
			AIError::UnsupportedConversion(strng::literal!("messages image source missing type"))
		})?;
		match source_type {
			"base64" => {
				let media_type = source
					.get("media_type")
					.and_then(Value::as_str)
					.ok_or_else(|| {
						AIError::UnsupportedConversion(strng::literal!(
							"messages base64 image source missing media_type"
						))
					})?;
				let data = source.get("data").and_then(Value::as_str).ok_or_else(|| {
					AIError::UnsupportedConversion(strng::literal!(
						"messages base64 image source missing data"
					))
				})?;
				Ok(json!({
					"type": "input_image",
					"image_url": format!("data:{media_type};base64,{data}"),
					"detail": "auto",
				}))
			},
			"url" => {
				let url = source.get("url").and_then(Value::as_str).ok_or_else(|| {
					AIError::UnsupportedConversion(strng::literal!("messages url image source missing url"))
				})?;
				Ok(json!({
					"type": "input_image",
					"image_url": url,
					"detail": "auto",
				}))
			},
			"file" => {
				let file_id = source
					.get("file_id")
					.and_then(Value::as_str)
					.ok_or_else(|| {
						AIError::UnsupportedConversion(strng::literal!(
							"messages file image source missing file_id"
						))
					})?;
				Ok(json!({
					"type": "input_image",
					"file_id": file_id,
					"detail": "auto",
				}))
			},
			_ => unsupported("messages image source cannot be represented by responses"),
		}
	}

	fn translate_tool_result_content(
		content: messages::ToolResultContent,
		cache_control: Option<messages::CacheControlEphemeral>,
	) -> Result<Value, AIError> {
		match content {
			messages::ToolResultContent::Text(text) => {
				if cache_control.is_some() {
					let mut part = json!({
						"type": "input_text",
						"text": text,
					});
					add_prompt_cache_breakpoint(&mut part, cache_control);
					Ok(json!([part]))
				} else {
					Ok(Value::String(text))
				}
			},
			messages::ToolResultContent::Array(parts) => {
				let mut text_parts = Vec::new();
				let mut text_values = Vec::new();
				let has_cache_control = cache_control.is_some();
				for part in parts {
					match part {
						messages::ToolResultContentPart::Text {
							text,
							citations,
							cache_control,
						} => {
							reject_option(
								&citations,
								"messages tool_result citations cannot be represented by responses",
							)?;
							let mut value = json!({
								"type": "input_text",
								"text": &text,
							});
							add_prompt_cache_breakpoint(&mut value, cache_control);
							text_parts.push(text);
							text_values.push(value);
						},
						messages::ToolResultContentPart::Image { .. }
						| messages::ToolResultContentPart::Document { .. }
						| messages::ToolResultContentPart::SearchResult { .. } => {
							return unsupported(
								"messages non-text tool_result content cannot be represented by responses",
							);
						},
					}
				}
				if let Some(cache_control) = cache_control {
					if let Some(last) = text_values.last_mut() {
						add_prompt_cache_breakpoint(last, Some(cache_control));
					} else {
						text_values.push(json!({
							"type": "input_text",
							"text": "",
							"prompt_cache_breakpoint": {"mode": "explicit"},
						}));
					}
				}
				if has_cache_control
					|| text_values
						.iter()
						.any(|part| part.get("prompt_cache_breakpoint").is_some())
				{
					Ok(Value::Array(text_values))
				} else {
					Ok(Value::String(text_parts.join("\n")))
				}
			},
		}
	}

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<responses::Response>(bytes)
			.map_err(logged_response_parsing(bytes))?;
		let anthropic = translate_response_internal(resp)?;
		Ok(Box::new(anthropic))
	}

	fn translate_response_internal(
		resp: responses::Response,
	) -> Result<messages::MessagesResponse, AIError> {
		if resp.error.is_some() || matches!(resp.status, responses::Status::Failed) {
			return unsupported("responses failed body cannot be represented by messages");
		}
		if matches!(
			resp.status,
			responses::Status::InProgress | responses::Status::Queued | responses::Status::Cancelled
		) {
			return unsupported("non-final responses body cannot be represented by messages");
		}

		let service_tier = resp
			.service_tier
			.as_ref()
			.and_then(types::serialize_str)
			.map(|tier| tier.to_string());
		let usage = translate_usage(resp.usage.as_ref(), service_tier);
		let mut content = Vec::new();
		let mut saw_tool_call = false;
		let mut saw_refusal = false;

		for item in resp.output {
			match item {
				responses::OutputItem::Message(message) => {
					if matches!(message.status, responses::OutputStatus::InProgress) {
						return unsupported("in-progress responses message cannot be represented by messages");
					}
					for part in message.content {
						match part {
							responses::OutputMessageContent::OutputText(text) => {
								if !text.annotations.is_empty()
									|| text
										.logprobs
										.as_ref()
										.is_some_and(|logprobs| !logprobs.is_empty())
								{
									return unsupported(
										"responses text annotations/logprobs cannot be represented by messages",
									);
								}
								content.push(messages::ContentBlock::Text(messages::ContentTextBlock {
									text: text.text,
									citations: None,
									cache_control: None,
								}));
							},
							responses::OutputMessageContent::Refusal(refusal) => {
								saw_refusal = true;
								content.push(messages::ContentBlock::Text(messages::ContentTextBlock {
									text: refusal.refusal,
									citations: None,
									cache_control: None,
								}));
							},
						}
					}
				},
				responses::OutputItem::FunctionCall(call) => {
					if call.namespace.is_some() || call.caller.is_some() {
						return unsupported(
							"responses namespaced/caller function calls cannot be represented by messages",
						);
					}
					if matches!(call.status, Some(responses::OutputStatus::InProgress)) {
						return unsupported(
							"in-progress responses function call cannot be represented by messages",
						);
					}
					let input = parse_tool_arguments(&call.arguments)?;
					saw_tool_call = true;
					content.push(messages::ContentBlock::ToolUse {
						id: call.call_id,
						name: call.name,
						input,
						cache_control: None,
					});
				},
				responses::OutputItem::Reasoning(_) => {
					tracing::warn!(
						"Skipping unsupported Responses reasoning output during messages translation"
					);
				},
				_ => {
					return unsupported("responses output item cannot be represented by messages");
				},
			}
		}

		let stop_reason = translate_stop_reason(
			&resp.status,
			resp.incomplete_details.as_ref(),
			saw_tool_call,
			saw_refusal,
		)?;
		Ok(messages::MessagesResponse {
			id: resp.id,
			r#type: "message".to_string(),
			role: messages::Role::Assistant,
			content,
			model: resp.model,
			stop_reason: Some(stop_reason),
			stop_sequence: None,
			usage,
			input_audio_tokens: None,
			output_audio_tokens: None,
		})
	}

	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
	) -> Body {
		#[derive(Default)]
		struct ToolBlock {
			index: Option<usize>,
			id: Option<String>,
			name: Option<String>,
			arguments: String,
			emitted_arguments: bool,
			open: bool,
		}

		#[derive(Default)]
		struct StreamState {
			sent_message_start: bool,
			sent_message_stop: bool,
			sent_first_token: bool,
			next_block_index: usize,
			response_id: Option<String>,
			model: Option<String>,
			text_blocks: HashMap<(u32, u32), usize>,
			open_text_blocks: HashSet<(u32, u32)>,
			tool_blocks: HashMap<u32, ToolBlock>,
			pending_usage: Option<responses::ResponseUsage>,
			pending_stop_reason: Option<messages::StopReason>,
			saw_tool_call: bool,
			saw_refusal: bool,
		}

		fn push_event(
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			event: messages::MessagesStreamEvent,
		) {
			events.push(event.into_sse_tuple());
		}

		fn ensure_message_start(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			log: &StreamingUsageGuard,
		) {
			if state.sent_message_start {
				return;
			}
			state.sent_message_start = true;
			let id = state
				.response_id
				.clone()
				.unwrap_or_else(|| format!("msg_{:016x}", rand::rng().random::<u64>()));
			let model = state.model.clone().unwrap_or_default();
			push_event(
				events,
				messages::MessagesStreamEvent::MessageStart {
					message: messages::MessagesResponse {
						id,
						r#type: "message".to_string(),
						role: messages::Role::Assistant,
						content: vec![],
						model: model.clone(),
						stop_reason: None,
						stop_sequence: None,
						usage: messages::Usage {
							input_tokens: 0,
							output_tokens: 0,
							cache_creation_input_tokens: None,
							cache_read_input_tokens: None,
							service_tier: None,
						},
						input_audio_tokens: None,
						output_audio_tokens: None,
					},
				},
			);
			if !model.is_empty() {
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&model));
				});
			}
		}

		fn maybe_set_first_token(state: &mut StreamState, log: &StreamingUsageGuard) {
			if state.sent_first_token {
				return;
			}
			state.sent_first_token = true;
			log.update(|r| {
				r.response.first_token = Some(Instant::now());
			});
		}

		fn close_text_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			key: (u32, u32),
		) {
			if state.open_text_blocks.remove(&key)
				&& let Some(index) = state.text_blocks.get(&key)
			{
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStop { index: *index },
				);
			}
		}

		fn close_text_blocks_for_output(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			output_index: u32,
		) {
			let mut keys: Vec<_> = state
				.open_text_blocks
				.iter()
				.filter(|(out, _)| *out == output_index)
				.copied()
				.collect();
			keys.sort_by_key(|key| state.text_blocks.get(key).copied().unwrap_or_default());
			for key in keys {
				close_text_block(state, events, key);
			}
		}

		fn close_all_text_blocks(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) {
			let mut keys: Vec<_> = state.open_text_blocks.iter().copied().collect();
			keys.sort_by_key(|key| state.text_blocks.get(key).copied().unwrap_or_default());
			for key in keys {
				close_text_block(state, events, key);
			}
		}

		fn close_tool_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			output_index: u32,
		) {
			let Some(block) = state.tool_blocks.get_mut(&output_index) else {
				return;
			};
			if block.open {
				block.open = false;
				if let Some(index) = block.index {
					push_event(
						events,
						messages::MessagesStreamEvent::ContentBlockStop { index },
					);
				}
			}
		}

		fn close_all_tool_blocks(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) {
			let mut blocks: Vec<_> = state
				.tool_blocks
				.iter()
				.filter_map(|(output_index, block)| block.index.map(|idx| (*output_index, idx)))
				.collect();
			blocks.sort_by_key(|(_, index)| *index);
			for (output_index, _) in blocks {
				close_tool_block(state, events, output_index);
			}
		}

		fn open_text_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			key: (u32, u32),
		) -> usize {
			close_all_tool_blocks(state, events);
			let index = *state.text_blocks.entry(key).or_insert_with(|| {
				let index = state.next_block_index;
				state.next_block_index += 1;
				index
			});
			if state.open_text_blocks.insert(key) {
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStart {
						index,
						content_block: messages::ContentBlock::Text(messages::ContentTextBlock {
							text: String::new(),
							citations: None,
							cache_control: None,
						}),
					},
				);
			}
			index
		}

		fn open_tool_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			output_index: u32,
			id: Option<String>,
			name: Option<String>,
		) -> usize {
			close_all_text_blocks(state, events);
			state.saw_tool_call = true;
			let mut start = None;
			let index = {
				let block = state.tool_blocks.entry(output_index).or_default();
				if block.index.is_none() {
					block.index = Some(state.next_block_index);
					state.next_block_index += 1;
				}
				if block.id.is_none() {
					block.id = id;
				}
				if block.name.as_ref().is_none_or(|name| name.is_empty()) {
					block.name = name;
				}
				let index = block.index.unwrap_or_default();
				if !block.open {
					block.open = true;
					start = Some((
						index,
						block
							.id
							.clone()
							.unwrap_or_else(|| format!("call_{output_index}")),
						block.name.clone().unwrap_or_default(),
					));
				}
				index
			};
			if let Some((index, id, name)) = start {
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStart {
						index,
						content_block: messages::ContentBlock::ToolUse {
							id,
							name,
							input: Value::Object(Map::new()),
							cache_control: None,
						},
					},
				);
			}
			index
		}

		fn update_usage_log(
			log: &StreamingUsageGuard,
			usage: &responses::ResponseUsage,
			service_tier: Option<String>,
		) {
			log.update(|r| {
				r.response.input_tokens = Some(usage.input_tokens as u64);
				r.response.output_tokens = Some(usage.output_tokens as u64);
				r.response.total_tokens = Some(usage.total_tokens as u64);
				r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
				r.response.cache_creation_input_tokens = usage
					.input_tokens_details
					.cache_write_tokens
					.map(|tokens| tokens as u64);
				r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
				if let Some(service_tier) = &service_tier {
					r.response.service_tier = Some(strng::new(service_tier));
				}
			});
		}

		fn record_response_metadata(
			state: &mut StreamState,
			log: &StreamingUsageGuard,
			response: &responses::Response,
		) {
			state.response_id = Some(response.id.clone());
			state.model = Some(response.model.clone());
			log.update(|r| {
				r.response.provider_model = Some(strng::new(&response.model));
				r.response.service_tier = response
					.service_tier
					.as_ref()
					.and_then(types::serialize_str);
			});
			if let Some(usage) = &response.usage {
				state.pending_usage = Some(usage.clone());
				update_usage_log(
					log,
					usage,
					response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str)
						.map(|tier| tier.to_string()),
				);
			}
		}

		fn flush_message_end(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			log: &StreamingUsageGuard,
			completion: &mut Option<String>,
			tool_calls: &mut Option<BTreeMap<u32, OutputMessagePart>>,
			force: bool,
		) {
			if state.sent_message_stop {
				return;
			}
			if !state.sent_message_start && !force {
				return;
			}
			ensure_message_start(state, events, log);

			let stop_reason = state.pending_stop_reason.take().unwrap_or({
				if state.saw_tool_call {
					messages::StopReason::ToolUse
				} else {
					messages::StopReason::EndTurn
				}
			});
			let usage = state.pending_usage.take();
			let message_usage = translate_delta_usage(usage.as_ref());

			close_all_text_blocks(state, events);
			close_all_tool_blocks(state, events);

			push_event(
				events,
				messages::MessagesStreamEvent::MessageDelta {
					delta: messages::MessageDelta {
						stop_reason: Some(stop_reason),
						stop_sequence: None,
					},
					usage: message_usage,
				},
			);
			push_event(events, messages::MessagesStreamEvent::MessageStop);
			state.sent_message_stop = true;

			let finish_reason = types::serialize_str(&stop_reason);
			log.update(|r| {
				if let Some(c) = completion.take() {
					r.response.completion = Some(vec![c]);
				}
				r.response.output_messages = super::take_output_messages(tool_calls, finish_reason.clone());
			});
		}

		let mut state = StreamState::default();
		let mut completion = log_content.completion.then(String::new);
		let mut tool_calls = log_content.tool_calls.then(BTreeMap::new);

		parse::sse::json_transform_multi::<
			responses::ResponseStreamEvent,
			messages::MessagesStreamEvent,
			_,
		>(b, buffer_limit, move |evt| {
			let mut events: Vec<(&'static str, messages::MessagesStreamEvent)> = Vec::new();
			match evt {
				SseJsonEvent::Eof | SseJsonEvent::Error => return events,
				SseJsonEvent::Done => {
					flush_message_end(
						&mut state,
						&mut events,
						&log,
						&mut completion,
						&mut tool_calls,
						true,
					);
				},
				SseJsonEvent::Data(Err(e)) => {
					tracing::warn!(
						"Failed to parse OpenAI Responses stream during messages translation: {}",
						e
					);
				},
				SseJsonEvent::Data(Ok(event)) => match event {
					responses::ResponseStreamEvent::ResponseCreated(created) => {
						record_response_metadata(&mut state, &log, &created.response);
						ensure_message_start(&mut state, &mut events, &log);
					},
					responses::ResponseStreamEvent::ResponseOutputItemAdded(added) => {
						ensure_message_start(&mut state, &mut events, &log);
						if let responses::OutputItem::FunctionCall(call) = added.item {
							open_tool_block(
								&mut state,
								&mut events,
								added.output_index,
								Some(call.call_id),
								Some(call.name),
							);
							maybe_set_first_token(&mut state, &log);
						}
					},
					responses::ResponseStreamEvent::ResponseContentPartAdded(added) => {
						ensure_message_start(&mut state, &mut events, &log);
						match added.part {
							responses::OutputContent::OutputText(_) => {
								open_text_block(
									&mut state,
									&mut events,
									(added.output_index, added.content_index),
								);
							},
							responses::OutputContent::Refusal(_) => {
								state.pending_stop_reason = Some(messages::StopReason::Refusal);
								open_text_block(
									&mut state,
									&mut events,
									(added.output_index, added.content_index),
								);
							},
							responses::OutputContent::ReasoningText(_) => {
								tracing::warn!(
									"Skipping unsupported Responses reasoning stream content during messages translation"
								);
							},
						}
					},
					responses::ResponseStreamEvent::ResponseOutputTextDelta(delta) => {
						ensure_message_start(&mut state, &mut events, &log);
						let index = open_text_block(
							&mut state,
							&mut events,
							(delta.output_index, delta.content_index),
						);
						maybe_set_first_token(&mut state, &log);
						if let Some(c) = completion.as_mut() {
							c.push_str(&delta.delta);
						}
						push_event(
							&mut events,
							messages::MessagesStreamEvent::ContentBlockDelta {
								index,
								delta: messages::ContentBlockDelta::TextDelta { text: delta.delta },
							},
						);
					},
					responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta) => {
						ensure_message_start(&mut state, &mut events, &log);
						let index = open_tool_block(
							&mut state,
							&mut events,
							delta.output_index,
							Some(delta.item_id.clone()),
							None,
						);
						let block = state.tool_blocks.entry(delta.output_index).or_default();
						block.arguments.push_str(&delta.delta);
						block.emitted_arguments = true;
						maybe_set_first_token(&mut state, &log);
						push_event(
							&mut events,
							messages::MessagesStreamEvent::ContentBlockDelta {
								index,
								delta: messages::ContentBlockDelta::InputJsonDelta {
									partial_json: delta.delta,
								},
							},
						);
					},
					responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(done) => {
						ensure_message_start(&mut state, &mut events, &log);
						let index = open_tool_block(
							&mut state,
							&mut events,
							done.output_index,
							Some(done.item_id.clone()),
							done.name.clone(),
						);
						let block = state.tool_blocks.entry(done.output_index).or_default();
						if block.arguments.is_empty() {
							block.arguments = done.arguments.clone();
						}
						if !block.emitted_arguments && !done.arguments.is_empty() {
							block.emitted_arguments = true;
							maybe_set_first_token(&mut state, &log);
							push_event(
								&mut events,
								messages::MessagesStreamEvent::ContentBlockDelta {
									index,
									delta: messages::ContentBlockDelta::InputJsonDelta {
										partial_json: done.arguments,
									},
								},
							);
						}
					},
					responses::ResponseStreamEvent::ResponseRefusalDelta(delta) => {
						state.saw_refusal = true;
						state.pending_stop_reason = Some(messages::StopReason::Refusal);
						let key = (delta.output_index, delta.content_index);
						let index = open_text_block(&mut state, &mut events, key);
						if !delta.delta.is_empty() {
							maybe_set_first_token(&mut state, &log);
							if let Some(c) = completion.as_mut() {
								c.push_str(&delta.delta);
							}
							push_event(
								&mut events,
								messages::MessagesStreamEvent::ContentBlockDelta {
									index,
									delta: messages::ContentBlockDelta::TextDelta { text: delta.delta },
								},
							);
						}
					},
					responses::ResponseStreamEvent::ResponseRefusalDone(done) => {
						state.saw_refusal = true;
						state.pending_stop_reason = Some(messages::StopReason::Refusal);
						close_text_block(
							&mut state,
							&mut events,
							(done.output_index, done.content_index),
						);
					},
					responses::ResponseStreamEvent::ResponseContentPartDone(done) => {
						if let responses::OutputContent::Refusal(refusal) = done.part {
							state.saw_refusal = true;
							state.pending_stop_reason = Some(messages::StopReason::Refusal);
							let key = (done.output_index, done.content_index);
							let index = open_text_block(&mut state, &mut events, key);
							if !refusal.refusal.is_empty() {
								maybe_set_first_token(&mut state, &log);
								if let Some(c) = completion.as_mut() {
									c.push_str(&refusal.refusal);
								}
								push_event(
									&mut events,
									messages::MessagesStreamEvent::ContentBlockDelta {
										index,
										delta: messages::ContentBlockDelta::TextDelta {
											text: refusal.refusal,
										},
									},
								);
							}
						}
						close_text_block(
							&mut state,
							&mut events,
							(done.output_index, done.content_index),
						);
					},
					responses::ResponseStreamEvent::ResponseOutputItemDone(done) => match done.item {
						responses::OutputItem::FunctionCall(call) => {
							state.saw_tool_call = true;
							let index = open_tool_block(
								&mut state,
								&mut events,
								done.output_index,
								Some(call.call_id.clone()),
								Some(call.name.clone()),
							);
							let block = state.tool_blocks.entry(done.output_index).or_default();
							if block.arguments.is_empty() {
								block.arguments = call.arguments.clone();
							}
							if !block.emitted_arguments && !call.arguments.is_empty() {
								block.emitted_arguments = true;
								push_event(
									&mut events,
									messages::MessagesStreamEvent::ContentBlockDelta {
										index,
										delta: messages::ContentBlockDelta::InputJsonDelta {
											partial_json: call.arguments.clone(),
										},
									},
								);
							}
							if let Some(tool_calls) = tool_calls.as_mut() {
								let arguments = serde_json::from_str(&call.arguments)
									.unwrap_or_else(|_| Value::Object(Map::new()));
								tool_calls.insert(
									done.output_index,
									OutputMessagePart::ToolCall {
										id: strng::new(&call.call_id),
										name: strng::new(&call.name),
										arguments,
									},
								);
							}
							close_tool_block(&mut state, &mut events, done.output_index);
						},
						responses::OutputItem::Message(_) => {
							close_text_blocks_for_output(&mut state, &mut events, done.output_index);
						},
						_ => {
							tracing::warn!(
								"Skipping unsupported Responses output item during messages stream translation"
							);
						},
					},
					responses::ResponseStreamEvent::ResponseCompleted(completed) => {
						record_response_metadata(&mut state, &log, &completed.response);
						state.pending_stop_reason = Some(if state.saw_tool_call {
							messages::StopReason::ToolUse
						} else if state.saw_refusal {
							messages::StopReason::Refusal
						} else {
							messages::StopReason::EndTurn
						});
						flush_message_end(
							&mut state,
							&mut events,
							&log,
							&mut completion,
							&mut tool_calls,
							true,
						);
					},
					responses::ResponseStreamEvent::ResponseIncomplete(incomplete) => {
						record_response_metadata(&mut state, &log, &incomplete.response);
						state.pending_stop_reason = incomplete
							.response
							.incomplete_details
							.as_ref()
							.and_then(|details| incomplete_reason_to_stop_reason(&details.reason).ok());
						flush_message_end(
							&mut state,
							&mut events,
							&log,
							&mut completion,
							&mut tool_calls,
							true,
						);
					},
					responses::ResponseStreamEvent::ResponseFailed(failed) => {
						record_response_metadata(&mut state, &log, &failed.response);
						tracing::warn!(
							"Responses stream failed during messages translation; emitting error event"
						);
						flush_message_end(
							&mut state,
							&mut events,
							&log,
							&mut completion,
							&mut tool_calls,
							true,
						);
						push_event(
							&mut events,
							messages::MessagesStreamEvent::Error {
								error: messages::MessagesError {
									r#type: "api_error".to_string(),
									message: "responses stream failed".to_string(),
								},
							},
						);
					},
					responses::ResponseStreamEvent::ResponseError(error) => {
						tracing::warn!(
							"Responses stream error during messages translation: {}",
							error.message
						);
						flush_message_end(
							&mut state,
							&mut events,
							&log,
							&mut completion,
							&mut tool_calls,
							true,
						);
						push_event(
							&mut events,
							messages::MessagesStreamEvent::Error {
								error: messages::MessagesError {
									r#type: error
										.code
										.clone()
										.unwrap_or_else(|| "api_error".to_string()),
									message: error.message,
								},
							},
						);
					},
					// Intermediate progress event; no block state change needed.
					responses::ResponseStreamEvent::ResponseInProgress(_) => {},
					// Text finalization is handled by ResponseContentPartDone.
					responses::ResponseStreamEvent::ResponseOutputTextDone(_) => {},
				},
			}
			events
		})
	}

	pub fn translate_error(bytes: &Bytes, status: ::http::StatusCode) -> Result<Bytes, AIError> {
		crate::conversion::completions::from_messages::translate_error(bytes, status)
	}

	fn translate_usage(
		usage: Option<&responses::ResponseUsage>,
		service_tier: Option<String>,
	) -> messages::Usage {
		let Some(usage) = usage else {
			return messages::Usage {
				input_tokens: 0,
				output_tokens: 0,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
				service_tier,
			};
		};
		let cache_creation_input_tokens = usage
			.input_tokens_details
			.cache_write_tokens
			.map(|tokens| tokens as usize)
			.filter(|tokens| *tokens > 0);
		let cache_read_input_tokens = (usage.input_tokens_details.cached_tokens as usize)
			.checked_sub(0)
			.filter(|tokens| *tokens > 0);
		messages::Usage {
			input_tokens: (usage.input_tokens as usize)
				.saturating_sub(cache_creation_input_tokens.unwrap_or_default())
				.saturating_sub(cache_read_input_tokens.unwrap_or_default()),
			output_tokens: usage.output_tokens as usize,
			cache_creation_input_tokens,
			cache_read_input_tokens,
			service_tier,
		}
	}

	fn translate_delta_usage(
		usage: Option<&responses::ResponseUsage>,
	) -> messages::MessageDeltaUsage {
		let usage = translate_usage(usage, None);
		messages::MessageDeltaUsage {
			input_tokens: Some(usage.input_tokens),
			output_tokens: Some(usage.output_tokens),
			cache_creation_input_tokens: usage.cache_creation_input_tokens,
			cache_read_input_tokens: usage.cache_read_input_tokens,
		}
	}

	fn translate_stop_reason(
		status: &responses::Status,
		incomplete_details: Option<&responses::IncompleteDetails>,
		saw_tool_call: bool,
		saw_refusal: bool,
	) -> Result<messages::StopReason, AIError> {
		match status {
			responses::Status::Completed => {
				if saw_tool_call {
					Ok(messages::StopReason::ToolUse)
				} else if saw_refusal {
					Ok(messages::StopReason::Refusal)
				} else {
					Ok(messages::StopReason::EndTurn)
				}
			},
			responses::Status::Incomplete => {
				let Some(details) = incomplete_details else {
					return Ok(messages::StopReason::MaxTokens);
				};
				incomplete_reason_to_stop_reason(&details.reason)
			},
			responses::Status::Failed => {
				unsupported("responses failed status cannot be represented by messages")
			},
			responses::Status::InProgress | responses::Status::Queued | responses::Status::Cancelled => {
				unsupported("non-final responses status cannot be represented by messages")
			},
		}
	}

	fn incomplete_reason_to_stop_reason(reason: &str) -> Result<messages::StopReason, AIError> {
		let reason = reason.to_ascii_lowercase();
		if reason.contains("max") || reason.contains("token") {
			Ok(messages::StopReason::MaxTokens)
		} else if reason.contains("context") {
			Ok(messages::StopReason::ModelContextWindowExceeded)
		} else {
			Err(AIError::UnsupportedConversion(strng::format!(
				"responses incomplete reason {reason:?} cannot be represented by messages"
			)))
		}
	}

	fn parse_tool_arguments(arguments: &str) -> Result<Value, AIError> {
		serde_json::from_str(arguments).map_err(|_| {
			AIError::UnsupportedConversion(strng::literal!(
				"responses function call arguments are not valid JSON"
			))
		})
	}

	fn validate_raw_request(req: &types::messages::Request) -> Result<(), AIError> {
		// Validate only the fields we actually translate. Unknown request-level
		// fields are accepted and dropped rather than failing the conversion
		// (see #2662: "tune down validation very very very heavily").
		let _ = &req.rest;
		if let Some(system) = &req.system {
			match system {
				types::messages::TextBlock::Text(_) => {},
				types::messages::TextBlock::Array(parts) => {
					for part in parts {
						match part {
							types::messages::TextPart::Text { .. } => {},
							types::messages::TextPart::Unknown(_) => {},
						}
					}
				},
			}
		}
		for message in &req.messages {
			let _ = &message.rest;
			if let Some(types::messages::ContentBlock::Array(parts)) = &message.content {
				for part in parts {
					match part {
						types::messages::ContentPart::Text { .. } => {},
						types::messages::ContentPart::Unknown(_) => {},
					}
				}
			}
		}
		Ok(())
	}

	fn reject_option<T>(value: &Option<T>, reason: &'static str) -> Result<(), AIError> {
		if value.is_some() {
			unsupported(reason)
		} else {
			Ok(())
		}
	}

	fn unsupported<T>(reason: &'static str) -> Result<T, AIError> {
		Err(AIError::UnsupportedConversion(strng::new(reason)))
	}
}
