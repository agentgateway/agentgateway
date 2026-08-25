use std::collections::{HashMap, HashSet};
use std::time::Instant;

use agent_core::strng;
use axum_core::body::Body;
use base64::Engine;
use bytes::Bytes;

use crate::parse::sse::SseJsonEvent;
use crate::types::messages::typed as messages;
use crate::types::responses::typed as responses;
use crate::{AIError, LogContentFields, StreamingUsageGuard, json, parse, types};

#[derive(Debug, Clone, Default)]
pub struct State {
	tools: HashMap<String, DeclaredTool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeclaredTool {
	Function,
	Custom,
}

pub fn translate(req: &types::responses::Request) -> Result<(Vec<u8>, State), AIError> {
	let thinking_budget_tokens = req
		.vendor_extensions
		.as_ref()
		.and_then(|extensions| extensions.thinking_budget_tokens);
	let req = json::convert::<_, responses::CreateResponse>(req).map_err(AIError::RequestMarshal)?;
	translate_typed(req, thinking_budget_tokens)
}

fn translate_typed(
	req: responses::CreateResponse,
	thinking_budget_tokens: Option<u64>,
) -> Result<(Vec<u8>, State), AIError> {
	validate_typed_request(&req)?;
	let output_format = typed_output_format(req.text.as_ref());
	let reasoning_disabled = matches!(
		req
			.reasoning
			.as_ref()
			.and_then(|reasoning| reasoning.effort.as_ref()),
		Some(responses::ReasoningEffort::None)
	);
	let reasoning_effort = typed_reasoning_effort(req.reasoning.as_ref());
	let mut state = State::default();
	if req
		.temperature
		.is_some_and(|temperature| !temperature.is_finite() || !(0.0..=1.0).contains(&temperature))
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses temperature must be between 0 and 1"
		)));
	}
	let tools = translate_typed_tools(req.tools.as_deref(), &mut state)?;
	let tool_choice =
		translate_typed_tool_choice(req.tool_choice.as_ref(), req.parallel_tool_calls, &state)?;
	let (messages, system) = translate_typed_input(req.input, req.instructions, &state)?;
	let max_tokens = req
		.max_output_tokens
		.map(usize::try_from)
		.transpose()
		.map_err(|_| {
			AIError::UnsupportedConversion(strng::literal!("Responses max_output_tokens is too large"))
		})?
		// TODO: Replace this compatibility fallback when model metadata exposes output limits.
		.unwrap_or(4096);
	let output_config =
		(reasoning_effort.is_some() || output_format.is_some()).then_some(messages::OutputConfig {
			effort: reasoning_effort,
			format: output_format,
		});
	let thinking = match thinking_budget_tokens {
		Some(budget_tokens) => super::cap_thinking_budget_to_max_tokens(budget_tokens, max_tokens)
			.map(|budget_tokens| messages::ThinkingInput::Enabled { budget_tokens }),
		None if reasoning_disabled => Some(messages::ThinkingInput::Disabled {}),
		None => reasoning_effort.map(|_| messages::ThinkingInput::Adaptive {}),
	};
	let user_id = req
		.safety_identifier
		.as_deref()
		.filter(|value| !value.is_empty());
	let metadata = user_id.map(|user_id| messages::Metadata {
		fields: HashMap::from([("user_id".to_string(), user_id.to_string())]),
	});
	let translated = messages::Request {
		messages,
		system,
		model: req.model.clone().unwrap_or_default(),
		max_tokens,
		stop_sequences: Vec::new(),
		stream: req.stream.unwrap_or(false),
		temperature: req.temperature,
		top_p: req.top_p,
		top_k: None,
		tools: (!tools.is_empty()).then_some(tools),
		tool_choice,
		metadata,
		thinking,
		output_config,
	};
	let body = serde_json::to_vec(&translated).map_err(AIError::RequestMarshal)?;
	Ok((body, state))
}

fn validate_typed_request(req: &responses::CreateResponse) -> Result<(), AIError> {
	if req.store == Some(true) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses stored requests are unsupported"
		)));
	}
	if req.background == Some(true) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses background mode is unsupported"
		)));
	}
	if matches!(
		req.prompt_cache_retention,
		Some(responses::PromptCacheRetention::Hours24)
	) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses extended prompt cache retention is unsupported"
		)));
	}
	for (present, message) in [
		(
			req.previous_response_id.is_some(),
			"Responses previous_response_id is unsupported",
		),
		(
			req.conversation.is_some(),
			"Responses conversation is unsupported",
		),
		(req.prompt.is_some(), "Responses prompt is unsupported"),
		(
			req.max_tool_calls.is_some(),
			"Responses max_tool_calls is unsupported",
		),
		(
			req.moderation.is_some(),
			"Responses moderation is unsupported",
		),
	] {
		if present {
			return Err(AIError::UnsupportedConversion(strng::new(message)));
		}
	}
	if matches!(req.truncation, Some(responses::Truncation::Auto)) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses automatic truncation is unsupported"
		)));
	}
	if matches!(
		req.service_tier,
		Some(
			responses::ServiceTier::Flex
				| responses::ServiceTier::Scale
				| responses::ServiceTier::Priority
		)
	) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses service_tier cannot be preserved by Anthropic Messages"
		)));
	}
	if req
		.reasoning
		.as_ref()
		.is_some_and(|reasoning| reasoning.context.is_some() || reasoning.mode.is_some())
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses reasoning context and mode are unsupported"
		)));
	}
	if req
		.text
		.as_ref()
		.is_some_and(|text| text.verbosity.is_some())
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses text verbosity is unsupported"
		)));
	}
	Ok(())
}

fn typed_output_format(
	text: Option<&responses::ResponseTextParam>,
) -> Option<messages::OutputFormat> {
	let format = &text?.format;
	let mut schema = match format {
		responses::TextResponseFormatConfiguration::Text => return None,
		responses::TextResponseFormatConfiguration::JsonObject => {
			serde_json::json!({"type": "object", "additionalProperties": true})
		},
		responses::TextResponseFormatConfiguration::JsonSchema(format) => {
			let mut schema = format.schema.clone();
			if let Some(description) = &format.description
				&& let Some(object) = schema.as_object_mut()
				&& !object.contains_key("description")
			{
				object.insert(
					"description".to_string(),
					serde_json::Value::String(description.clone()),
				);
			}
			schema
		},
	};
	if schema.is_null() {
		schema = serde_json::json!({});
	}
	Some(messages::OutputFormat::JsonSchema { schema })
}

fn typed_reasoning_effort(
	reasoning: Option<&responses::Reasoning>,
) -> Option<messages::ThinkingEffort> {
	match reasoning.and_then(|reasoning| reasoning.effort.as_ref()) {
		None | Some(responses::ReasoningEffort::None) => None,
		Some(responses::ReasoningEffort::Minimal | responses::ReasoningEffort::Low) => {
			Some(messages::ThinkingEffort::Low)
		},
		Some(responses::ReasoningEffort::Medium) => Some(messages::ThinkingEffort::Medium),
		Some(responses::ReasoningEffort::High) => Some(messages::ThinkingEffort::High),
		Some(responses::ReasoningEffort::Xhigh) => Some(messages::ThinkingEffort::Xhigh),
		Some(responses::ReasoningEffort::Max) => Some(messages::ThinkingEffort::Max),
	}
}

fn translate_typed_tools(
	definitions: Option<&[responses::Tool]>,
	state: &mut State,
) -> Result<Vec<messages::Tool>, AIError> {
	let mut tools = Vec::new();
	for definition in definitions.unwrap_or_default() {
		let (name, kind, translated) = match definition {
			responses::Tool::Function(tool) => {
				if tool.defer_loading == Some(true)
					|| tool
						.allowed_callers
						.as_ref()
						.is_some_and(|callers| !callers.is_empty())
					|| tool.output_schema.is_some()
				{
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"Responses deferred, programmatic, and output-schema function tools are unsupported"
					)));
				}
				(
					tool.name.clone(),
					DeclaredTool::Function,
					messages::Tool::Custom(messages::CustomTool {
						tool_type: None,
						name: tool.name.clone(),
						description: tool.description.clone(),
						input_schema: tool
							.parameters
							.clone()
							.unwrap_or_else(|| serde_json::json!({})),
						strict: Some(tool.strict.unwrap_or(true)),
						cache_control: None,
					}),
				)
			},
			responses::Tool::Custom(tool) => {
				if !matches!(tool.format, responses::CustomToolParamFormat::Text)
					|| tool.defer_loading == Some(true)
					|| tool
						.allowed_callers
						.as_ref()
						.is_some_and(|callers| !callers.is_empty())
				{
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"Responses custom tool grammars, deferral, and programmatic callers are unsupported"
					)));
				}
				let content_description =
					format!("The {} content following the specified format", tool.name);
				(
					tool.name.clone(),
					DeclaredTool::Custom,
					messages::Tool::Custom(messages::CustomTool {
						tool_type: Some(messages::CustomToolType::Custom),
						name: tool.name.clone(),
						description: tool.description.clone(),
						input_schema: serde_json::json!({
							"type": "object",
							"properties": {
								"content": {
									"type": "string",
									"description": content_description
								}
							},
							"required": ["content"]
						}),
						strict: None,
						cache_control: None,
					}),
				)
			},
			unsupported => return Err(unsupported_tool(unsupported)),
		};
		if state.tools.insert(name, kind).is_some() {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses tool names must be unique"
			)));
		}
		tools.push(translated);
	}
	Ok(tools)
}

fn unsupported_tool(tool: &responses::Tool) -> AIError {
	let kind = match tool {
		responses::Tool::Namespace(_) => "namespace",
		responses::Tool::LocalShell => "local_shell",
		responses::Tool::Shell(_) => "shell",
		responses::Tool::ApplyPatch(_) => "apply_patch",
		responses::Tool::Function(_) | responses::Tool::Custom(_) => unreachable!(),
		_ => "built-in",
	};
	AIError::UnsupportedConversion(strng::new(format!(
		"Responses {kind} tools require a separate Anthropic Messages tool mapping"
	)))
}

fn translate_typed_tool_choice(
	choice: Option<&responses::ToolChoiceParam>,
	parallel_tool_calls: Option<bool>,
	state: &State,
) -> Result<Option<messages::ToolChoice>, AIError> {
	let disable_parallel_tool_use = (parallel_tool_calls == Some(false)).then_some(true);
	let Some(choice) = choice else {
		return Ok(
			(disable_parallel_tool_use.is_some() && !state.tools.is_empty()).then_some(
				messages::ToolChoice::Auto {
					disable_parallel_tool_use,
				},
			),
		);
	};
	let translated = match choice {
		responses::ToolChoiceParam::Mode(responses::ToolChoiceOptions::Auto) => {
			messages::ToolChoice::Auto {
				disable_parallel_tool_use,
			}
		},
		responses::ToolChoiceParam::Mode(responses::ToolChoiceOptions::Required) => {
			if state.tools.is_empty() {
				return Err(invalid_tool_choice());
			}
			messages::ToolChoice::Any {
				disable_parallel_tool_use,
			}
		},
		responses::ToolChoiceParam::Mode(responses::ToolChoiceOptions::None) => {
			messages::ToolChoice::None {}
		},
		responses::ToolChoiceParam::Function(tool) => {
			if !matches!(state.tools.get(&tool.name), Some(DeclaredTool::Function)) {
				return Err(invalid_tool_choice());
			}
			messages::ToolChoice::Tool {
				name: tool.name.clone(),
				disable_parallel_tool_use,
			}
		},
		responses::ToolChoiceParam::Custom(tool) => {
			if !matches!(state.tools.get(&tool.name), Some(DeclaredTool::Custom)) {
				return Err(invalid_tool_choice());
			}
			messages::ToolChoice::Tool {
				name: tool.name.clone(),
				disable_parallel_tool_use,
			}
		},
		_ => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses built-in and constrained tool choices are unsupported"
			)));
		},
	};
	Ok(Some(translated))
}

fn translate_typed_input(
	input: responses::InputParam,
	instructions: Option<String>,
	state: &State,
) -> Result<(Vec<messages::Message>, Option<messages::SystemPrompt>), AIError> {
	use responses::{InputItem, Item, MessageItem};

	let items = match input {
		responses::InputParam::Text(text) => {
			if text.is_empty() {
				return Err(empty_input());
			}
			vec![InputItem::from(responses::InputMessage {
				content: vec![responses::InputContent::InputText(
					responses::InputTextContent {
						text,
						prompt_cache_breakpoint: None,
					},
				)],
				role: responses::InputRole::User,
				status: None,
			})]
		},
		responses::InputParam::Items(items) if items.is_empty() => return Err(empty_input()),
		responses::InputParam::Items(items) => items,
	};

	let mut output = Vec::new();
	let mut system = instructions
		.filter(|instruction| !instruction.is_empty())
		.into_iter()
		.collect::<Vec<_>>();
	let mut calls = HashMap::new();
	let mut completed_outputs = HashSet::new();

	for item in items {
		match item {
			InputItem::EasyMessage(message) => match message.role {
				responses::Role::System | responses::Role::Developer => {
					system.extend(typed_system_content(message.content)?);
				},
				responses::Role::User => push_blocks(
					&mut output,
					messages::Role::User,
					typed_input_content(message.content)?,
				),
				responses::Role::Assistant => push_blocks(
					&mut output,
					messages::Role::Assistant,
					typed_assistant_content(message.content)?,
				),
			},
			InputItem::Item(Item::Message(MessageItem::Input(message))) => match message.role {
				responses::InputRole::System | responses::InputRole::Developer => {
					system.extend(typed_system_parts(message.content)?);
				},
				responses::InputRole::User => push_blocks(
					&mut output,
					messages::Role::User,
					typed_input_parts(message.content)?,
				),
			},
			InputItem::Item(Item::Message(MessageItem::Output(message))) => push_blocks(
				&mut output,
				messages::Role::Assistant,
				typed_output_message_content(message.content)?,
			),
			InputItem::Item(Item::FunctionCall(call)) => {
				if call.namespace.is_some() || !typed_caller_is_direct(call.caller.as_ref()) {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"Responses namespaced and programmatic function history is unsupported"
					)));
				}
				if matches!(state.tools.get(&call.name), Some(DeclaredTool::Custom)) {
					return Err(invalid_tool_history());
				}
				let input = serde_json::from_str(&call.arguments).map_err(|_| invalid_tool_history())?;
				record_typed_call(&mut calls, &call.call_id, DeclaredTool::Function)?;
				push_blocks(
					&mut output,
					messages::Role::Assistant,
					vec![messages::ContentBlock::ToolUse {
						id: call.call_id,
						name: call.name,
						input,
						caller: None,
						cache_control: None,
					}],
				);
			},
			InputItem::Item(Item::CustomToolCall(call)) => {
				if call.namespace.is_some() || !typed_caller_is_direct(call.caller.as_ref()) {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"Responses namespaced and programmatic custom tool history is unsupported"
					)));
				}
				if matches!(state.tools.get(&call.name), Some(DeclaredTool::Function)) {
					return Err(invalid_tool_history());
				}
				record_typed_call(&mut calls, &call.call_id, DeclaredTool::Custom)?;
				push_blocks(
					&mut output,
					messages::Role::Assistant,
					vec![messages::ContentBlock::ToolUse {
						id: call.call_id,
						name: call.name,
						input: serde_json::json!({"content": call.input}),
						caller: None,
						cache_control: None,
					}],
				);
			},
			InputItem::Item(Item::FunctionCallOutput(output_item)) => {
				if !typed_caller_is_direct(output_item.caller.as_ref()) {
					return Err(invalid_tool_history());
				}
				let is_error = matches!(
					output_item.status,
					Some(responses::OutputStatus::Incomplete)
				);
				let content = match output_item.output {
					responses::FunctionCallOutput::Text(text) => messages::ToolResultContent::Text(text),
					responses::FunctionCallOutput::Content(parts) => {
						messages::ToolResultContent::Array(typed_tool_result_parts(parts)?)
					},
				};
				push_typed_tool_output(
					&mut output,
					&calls,
					&mut completed_outputs,
					output_item.call_id,
					DeclaredTool::Function,
					content,
					is_error,
				)?;
			},
			InputItem::Item(Item::CustomToolCallOutput(output_item)) => {
				if !typed_caller_is_direct(output_item.caller.as_ref()) {
					return Err(invalid_tool_history());
				}
				let content = match output_item.output {
					responses::CustomToolCallOutputOutput::Text(text) => {
						messages::ToolResultContent::Text(text)
					},
					responses::CustomToolCallOutputOutput::List(parts) => {
						messages::ToolResultContent::Array(typed_tool_result_parts(parts)?)
					},
				};
				push_typed_tool_output(
					&mut output,
					&calls,
					&mut completed_outputs,
					output_item.call_id,
					DeclaredTool::Custom,
					content,
					false,
				)?;
			},
			InputItem::Item(Item::Reasoning(_)) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"Responses reasoning history is unsupported"
				)));
			},
			InputItem::Item(_) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"Responses built-in tool and hosted input history is unsupported"
				)));
			},
			InputItem::ItemReference(_) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"Responses item references are unsupported"
				)));
			},
			InputItem::Program(_) | InputItem::ProgramOutput(_) | InputItem::CompactionTrigger(_) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"Responses programmatic and compaction input is unsupported"
				)));
			},
		}
	}

	if output.is_empty() {
		return Err(empty_input());
	}
	validate_tool_result_order(&output)?;
	let system = (!system.is_empty()).then(|| {
		messages::SystemPrompt::Blocks(
			system
				.into_iter()
				.map(|text| messages::SystemContentBlock::Text {
					text,
					cache_control: None,
				})
				.collect(),
		)
	});
	Ok((output, system))
}

fn empty_input() -> AIError {
	AIError::UnsupportedConversion(strng::literal!("Responses input must not be empty"))
}

fn typed_system_content(content: responses::EasyInputContent) -> Result<Vec<String>, AIError> {
	match content {
		responses::EasyInputContent::Text(text) if !text.is_empty() => Ok(vec![text]),
		responses::EasyInputContent::Text(_) => Err(empty_input()),
		responses::EasyInputContent::ContentList(parts) => typed_system_parts(parts),
	}
}

fn typed_system_parts(parts: Vec<responses::InputContent>) -> Result<Vec<String>, AIError> {
	if parts.is_empty() {
		return Err(empty_input());
	}
	parts
		.into_iter()
		.map(|part| match part {
			responses::InputContent::InputText(text) if !text.text.is_empty() => Ok(text.text),
			_ => Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses system messages only support text content"
			))),
		})
		.collect()
}

fn typed_input_content(
	content: responses::EasyInputContent,
) -> Result<Vec<messages::ContentBlock>, AIError> {
	match content {
		responses::EasyInputContent::Text(text) if !text.is_empty() => Ok(vec![text_block(text)]),
		responses::EasyInputContent::Text(_) => Err(empty_input()),
		responses::EasyInputContent::ContentList(parts) => typed_input_parts(parts),
	}
}

fn typed_assistant_content(
	content: responses::EasyInputContent,
) -> Result<Vec<messages::ContentBlock>, AIError> {
	match content {
		responses::EasyInputContent::Text(text) if !text.is_empty() => Ok(vec![text_block(text)]),
		responses::EasyInputContent::Text(_) => Err(empty_input()),
		responses::EasyInputContent::ContentList(parts) => parts
			.into_iter()
			.map(|part| match part {
				responses::InputContent::InputText(text) if !text.text.is_empty() => {
					Ok(text_block(text.text))
				},
				_ => Err(AIError::UnsupportedConversion(strng::literal!(
					"Responses assistant history only supports text content"
				))),
			})
			.collect(),
	}
}

fn typed_output_message_content(
	content: Vec<responses::OutputMessageContent>,
) -> Result<Vec<messages::ContentBlock>, AIError> {
	if content.is_empty() {
		return Err(empty_input());
	}
	content
		.into_iter()
		.map(|part| match part {
			responses::OutputMessageContent::OutputText(text) if !text.text.is_empty() => {
				Ok(text_block(text.text))
			},
			_ => Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses assistant history must contain non-empty text"
			))),
		})
		.collect()
}

fn typed_input_parts(
	parts: Vec<responses::InputContent>,
) -> Result<Vec<messages::ContentBlock>, AIError> {
	if parts.is_empty() {
		return Err(empty_input());
	}
	parts.into_iter().map(typed_input_part).collect()
}

fn typed_input_part(part: responses::InputContent) -> Result<messages::ContentBlock, AIError> {
	match part {
		responses::InputContent::InputText(text) if !text.text.is_empty() => Ok(text_block(text.text)),
		responses::InputContent::InputText(_) => Err(empty_input()),
		responses::InputContent::InputImage(image) => typed_input_image(image),
		responses::InputContent::InputFile(file) => typed_input_file(file),
	}
}

fn typed_input_image(
	image: responses::InputImageContent,
) -> Result<messages::ContentBlock, AIError> {
	if image.detail != responses::ImageDetail::Auto {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses input image detail is unsupported"
		)));
	}
	if image.file_id.is_some() {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses input image file IDs are unsupported"
		)));
	}
	let url = image.image_url.ok_or_else(|| {
		AIError::UnsupportedConversion(strng::literal!("Responses input image URL is required"))
	})?;
	let source = if let Some((media_type, data)) = parse_base64_data_url(&url) {
		if !matches!(
			media_type,
			"image/jpeg" | "image/png" | "image/gif" | "image/webp"
		) {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"unsupported Responses input image media type"
			)));
		}
		serde_json::json!({"type": "base64", "media_type": media_type, "data": data})
	} else {
		if !is_absolute_http_url(&url) {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"invalid Responses input image URL"
			)));
		}
		serde_json::json!({"type": "url", "url": url})
	};
	Ok(messages::ContentBlock::Image(messages::ContentImageBlock {
		source,
		cache_control: None,
	}))
}

fn typed_input_file(file: responses::InputFileContent) -> Result<messages::ContentBlock, AIError> {
	if matches!(file.detail, Some(responses::FileInputDetail::High)) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses input file detail is unsupported"
		)));
	}
	if file.file_id.is_some() {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses input file IDs are unsupported"
		)));
	}
	let source = match (file.file_data, file.file_url) {
		(Some(data), None) => file_data_source(&data)?,
		(None, Some(url)) if is_absolute_http_url(&url) => {
			serde_json::json!({"type": "url", "url": url})
		},
		(None, Some(_)) => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"invalid Responses input file URL"
			)));
		},
		_ => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses input file requires exactly one source"
			)));
		},
	};
	Ok(messages::ContentBlock::Document(
		messages::ContentDocumentBlock {
			source,
			cache_control: None,
			citations: None,
			context: None,
			title: file.filename.filter(|name| !name.is_empty()),
		},
	))
}

fn typed_tool_result_parts(
	parts: Vec<responses::InputContent>,
) -> Result<Vec<messages::ToolResultContentPart>, AIError> {
	parts
		.into_iter()
		.map(|part| match typed_input_part(part)? {
			messages::ContentBlock::Text(text) => Ok(messages::ToolResultContentPart::Text {
				text: text.text,
				citations: None,
				cache_control: None,
			}),
			messages::ContentBlock::Image(image) => Ok(messages::ToolResultContentPart::Image {
				source: image.source,
				cache_control: None,
			}),
			messages::ContentBlock::Document(document) => Ok(messages::ToolResultContentPart::Document {
				source: document.source,
				cache_control: None,
				citations: None,
				context: None,
				title: document.title,
			}),
			_ => unreachable!("Responses input content maps to text, image, or document"),
		})
		.collect()
}

fn typed_caller_is_direct(caller: Option<&responses::ToolCallCaller>) -> bool {
	matches!(caller, None | Some(responses::ToolCallCaller::Direct))
}

fn record_typed_call(
	calls: &mut HashMap<String, DeclaredTool>,
	call_id: &str,
	kind: DeclaredTool,
) -> Result<(), AIError> {
	if call_id.is_empty() || calls.insert(call_id.to_string(), kind).is_some() {
		return Err(invalid_tool_history());
	}
	Ok(())
}

fn push_typed_tool_output(
	output: &mut Vec<messages::Message>,
	calls: &HashMap<String, DeclaredTool>,
	completed_outputs: &mut HashSet<String>,
	call_id: String,
	expected: DeclaredTool,
	content: messages::ToolResultContent,
	is_error: bool,
) -> Result<(), AIError> {
	if !matches!(calls.get(&call_id), Some(actual) if *actual == expected)
		|| !completed_outputs.insert(call_id.clone())
	{
		return Err(invalid_tool_history());
	}
	push_blocks(
		output,
		messages::Role::User,
		vec![messages::ContentBlock::ToolResult {
			tool_use_id: call_id,
			content,
			cache_control: None,
			is_error: is_error.then_some(true),
		}],
	);
	Ok(())
}

fn invalid_tool_choice() -> AIError {
	AIError::UnsupportedConversion(strng::literal!("invalid Responses tool choice"))
}

fn invalid_tool_history() -> AIError {
	AIError::UnsupportedConversion(strng::literal!("invalid Responses tool history"))
}

fn validate_tool_result_order(history: &[messages::Message]) -> Result<(), AIError> {
	let mut expected_results: Option<HashSet<String>> = None;
	for message in history {
		match message.role {
			messages::Role::Assistant => {
				if expected_results.is_some() {
					return Err(invalid_tool_history());
				}
				let calls = message
					.content
					.iter()
					.filter_map(|block| match block {
						messages::ContentBlock::ToolUse { id, .. } => Some(id.clone()),
						_ => None,
					})
					.collect::<HashSet<_>>();
				if !calls.is_empty() {
					expected_results = Some(calls);
				}
			},
			messages::Role::User => {
				let mut results = HashSet::new();
				let mut saw_other_content = false;
				for block in &message.content {
					if let messages::ContentBlock::ToolResult { tool_use_id, .. } = block {
						if saw_other_content || !results.insert(tool_use_id.clone()) {
							return Err(invalid_tool_history());
						}
					} else {
						saw_other_content = true;
					}
				}
				match expected_results.take() {
					Some(expected) if expected == results => {},
					Some(_) => return Err(invalid_tool_history()),
					None if !results.is_empty() => return Err(invalid_tool_history()),
					None => {},
				}
			},
			messages::Role::System => return Err(invalid_tool_history()),
		}
	}
	if expected_results.is_some() {
		return Err(invalid_tool_history());
	}
	Ok(())
}

fn parse_base64_data_url(url: &str) -> Option<(&str, &str)> {
	let (media_type, data) = crate::conversion::completions::parse_data_url(url)?;
	base64::engine::general_purpose::STANDARD
		.decode(data)
		.ok()?;
	Some((media_type, data))
}

fn is_absolute_http_url(url: &str) -> bool {
	let Ok(uri) = url.parse::<http::Uri>() else {
		return false;
	};
	matches!(uri.scheme_str(), Some("http" | "https"))
		&& uri.authority().is_some()
		&& uri.host().is_some_and(|host| !host.is_empty())
}

fn file_data_source(file_data: &str) -> Result<serde_json::Value, AIError> {
	let (media_type, data) = parse_base64_data_url(file_data).ok_or_else(|| {
		AIError::UnsupportedConversion(strng::literal!("invalid Responses input file data"))
	})?;
	match media_type {
		"application/pdf" => Ok(serde_json::json!({
			"type": "base64",
			"media_type": media_type,
			"data": data
		})),
		"text/plain" => {
			let bytes = base64::engine::general_purpose::STANDARD
				.decode(data)
				.map_err(|_| {
					AIError::UnsupportedConversion(strng::literal!("invalid Responses input file data"))
				})?;
			let text = String::from_utf8(bytes).map_err(|_| {
				AIError::UnsupportedConversion(strng::literal!("Responses text file must be UTF-8"))
			})?;
			Ok(serde_json::json!({
				"type": "text",
				"media_type": media_type,
				"data": text
			}))
		},
		_ => Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses input file media type"
		))),
	}
}

fn text_block(text: String) -> messages::ContentBlock {
	messages::ContentBlock::Text(messages::ContentTextBlock {
		text,
		citations: None,
		cache_control: None,
	})
}

fn push_blocks(
	output: &mut Vec<messages::Message>,
	role: messages::Role,
	content: Vec<messages::ContentBlock>,
) {
	if let Some(last) = output.last_mut()
		&& last.role == role
	{
		last.content.extend(content);
	} else {
		output.push(messages::Message { role, content });
	}
}

pub fn translate_error(_bytes: &Bytes, status: ::http::StatusCode) -> Result<Bytes, AIError> {
	let error_type = match status {
		::http::StatusCode::BAD_REQUEST => "invalid_request_error",
		::http::StatusCode::UNAUTHORIZED => "authentication_error",
		::http::StatusCode::FORBIDDEN => "permission_error",
		::http::StatusCode::NOT_FOUND => "not_found_error",
		::http::StatusCode::CONFLICT => "conflict_error",
		::http::StatusCode::PAYLOAD_TOO_LARGE => "request_too_large",
		::http::StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
		_ => "server_error",
	};
	let body = serde_json::json!({
		"error": {
			"message": format!(
				"Upstream Anthropic request failed with HTTP {}",
				status.as_u16()
			),
			"type": error_type,
			"param": null,
			"code": null,
		}
	});
	Ok(Bytes::from(
		serde_json::to_vec(&body).map_err(AIError::ResponseMarshal)?,
	))
}

pub fn translate_response(
	bytes: &Bytes,
	state: &State,
	buffer_limit: usize,
) -> Result<types::responses::Response, AIError> {
	let response: messages::MessagesResponse =
		serde_json::from_slice(bytes).map_err(|_| invalid_response())?;
	if response.r#type != "message"
		|| response.role != messages::Role::Assistant
		|| response.id.is_empty()
		|| response.model.is_empty()
	{
		return Err(invalid_response());
	}
	let upstream_usage = response.usage.clone();
	let stop_reason = response.stop_reason.ok_or_else(invalid_response)?;
	let (status, incomplete_reason) = terminal_status(stop_reason).ok_or_else(invalid_response)?;
	let output_status = if status == "completed" {
		responses::OutputStatus::Completed
	} else {
		responses::OutputStatus::Incomplete
	};
	let usage = responses_usage(&response.usage)?;
	let service_tier = response
		.usage
		.service_tier
		.as_deref()
		.map(public_service_tier)
		.transpose()?;
	let output = response_output(
		&response.id,
		response.content,
		output_status,
		message_phase(stop_reason),
		state,
		buffer_limit,
	)?;
	let builder =
		types::responses::ResponseBuilder::new(format!("resp_{}", response.id), response.model);
	let mut typed = builder.response(
		if output_status == responses::OutputStatus::Completed {
			responses::Status::Completed
		} else {
			responses::Status::Incomplete
		},
		Some(usage),
		None,
		incomplete_reason.map(|reason| responses::IncompleteDetails {
			reason: reason.to_string(),
		}),
	);
	typed.output = output;
	typed.service_tier = service_tier;
	let mut response =
		json::convert::<_, types::responses::Response>(&typed).map_err(AIError::ResponseParsing)?;
	response.set_provider_telemetry(
		u64::try_from(upstream_usage.input_tokens).map_err(|_| invalid_response())?,
		u64::try_from(upstream_usage.output_tokens).map_err(|_| invalid_response())?,
		upstream_usage.service_tier,
	);
	let final_bytes = serde_json::to_vec(&response)
		.map_err(|_| invalid_response())?
		.len();
	if final_bytes > buffer_limit {
		return Err(response_output_too_large());
	}
	Ok(response)
}

fn direct_tool_caller(caller: Option<&serde_json::Value>) -> bool {
	match caller {
		None => true,
		Some(serde_json::Value::Object(caller)) => {
			caller.len() == 1 && caller.get("type").and_then(serde_json::Value::as_str) == Some("direct")
		},
		Some(_) => false,
	}
}

fn terminal_status(
	stop_reason: messages::StopReason,
) -> Option<(&'static str, Option<&'static str>)> {
	match stop_reason {
		messages::StopReason::EndTurn
		| messages::StopReason::StopSequence
		| messages::StopReason::ToolUse => Some(("completed", None)),
		messages::StopReason::MaxTokens | messages::StopReason::ModelContextWindowExceeded => {
			Some(("incomplete", Some("max_output_tokens")))
		},
		messages::StopReason::Refusal | messages::StopReason::PauseTurn => None,
	}
}

/// `phase` labels an assistant message as intermediate commentary or the final answer. A
/// `tool_use` stop reason means the model is asking the client to run a tool and continue, so no
/// message in that turn is the final answer. Every other terminal stop reason ends the turn.
fn message_phase(stop_reason: messages::StopReason) -> responses::MessagePhase {
	match stop_reason {
		messages::StopReason::ToolUse => responses::MessagePhase::Commentary,
		_ => responses::MessagePhase::FinalAnswer,
	}
}

fn responses_usage(usage: &messages::Usage) -> Result<responses::ResponseUsage, AIError> {
	response_usage(
		usage.input_tokens,
		usage.output_tokens,
		usage.cache_read_input_tokens.unwrap_or_default(),
		usage.cache_creation_input_tokens,
		usage
			.output_tokens_details
			.as_ref()
			.and_then(|details| details.thinking_tokens),
	)
	.map_err(|_| invalid_response())
}

fn response_usage(
	input: usize,
	output: usize,
	cache_read: usize,
	cache_creation: Option<usize>,
	thinking: Option<usize>,
) -> Result<responses::ResponseUsage, ()> {
	let input_tokens = input
		.checked_add(cache_read)
		.and_then(|input| input.checked_add(cache_creation.unwrap_or_default()))
		.and_then(|input| u32::try_from(input).ok())
		.ok_or(())?;
	let output_tokens = u32::try_from(output).map_err(|_| ())?;
	let cache_write_tokens = cache_creation
		.map(u32::try_from)
		.transpose()
		.map_err(|_| ())?;
	let reasoning_tokens = thinking
		.map(u32::try_from)
		.transpose()
		.map_err(|_| ())?
		.unwrap_or_default();
	if reasoning_tokens > output_tokens {
		return Err(());
	}
	Ok(responses::ResponseUsage {
		input_tokens,
		output_tokens,
		total_tokens: input_tokens.checked_add(output_tokens).ok_or(())?,
		input_tokens_details: responses::InputTokenDetails {
			cached_tokens: u32::try_from(cache_read).map_err(|_| ())?,
			cache_write_tokens,
		},
		output_tokens_details: responses::OutputTokenDetails { reasoning_tokens },
	})
}

fn public_service_tier(service_tier: &str) -> Result<responses::ServiceTier, AIError> {
	match service_tier {
		"standard" => Ok(responses::ServiceTier::Default),
		"priority" => Ok(responses::ServiceTier::Priority),
		_ => Err(invalid_response()),
	}
}

fn response_output(
	message_id: &str,
	content: Vec<messages::ContentBlock>,
	status: responses::OutputStatus,
	phase: responses::MessagePhase,
	state: &State,
	buffer_limit: usize,
) -> Result<Vec<responses::OutputItem>, AIError> {
	let mut output = Vec::new();
	let mut pending_text: Option<(usize, Vec<responses::OutputMessageContent>)> = None;
	let mut retained_bytes = 0usize;
	for (index, block) in content.into_iter().enumerate() {
		if let messages::ContentBlock::Text(text) = block {
			let (_, parts) = pending_text.get_or_insert_with(|| (index, Vec::new()));
			parts.push(responses::OutputMessageContent::OutputText(
				responses::OutputTextContent {
					annotations: Vec::new(),
					logprobs: None,
					text: text.text,
				},
			));
			continue;
		}
		// Drop reasoning before flushing so surrounding text stays in one message item.
		if matches!(
			block,
			messages::ContentBlock::Thinking { .. } | messages::ContentBlock::RedactedThinking { .. }
		) {
			continue;
		}
		flush_response_text(
			&mut output,
			&mut pending_text,
			message_id,
			status,
			phase,
			buffer_limit,
			&mut retained_bytes,
		)?;
		match block {
			messages::ContentBlock::ToolUse {
				id,
				name,
				input,
				caller,
				..
			} => {
				if !direct_tool_caller(caller.as_ref()) {
					return Err(invalid_response());
				}
				let item = response_tool_output(message_id, index, status, &id, &name, input, state)?;
				retain_response_item(&mut output, item, buffer_limit, &mut retained_bytes)?;
			},
			_ => return Err(invalid_response()),
		}
	}
	flush_response_text(
		&mut output,
		&mut pending_text,
		message_id,
		status,
		phase,
		buffer_limit,
		&mut retained_bytes,
	)?;
	Ok(output)
}

fn retain_response_item(
	output: &mut Vec<responses::OutputItem>,
	item: responses::OutputItem,
	buffer_limit: usize,
	retained_bytes: &mut usize,
) -> Result<(), AIError> {
	let bytes = serde_json::to_vec(&item)
		.map_err(|_| invalid_response())?
		.len();
	let total = retained_bytes
		.checked_add(bytes)
		.filter(|total| *total <= buffer_limit)
		.ok_or_else(response_output_too_large)?;
	*retained_bytes = total;
	output.push(item);
	Ok(())
}

fn response_output_too_large() -> AIError {
	AIError::InvalidResponse(strng::literal!(
		"Anthropic Messages response output exceeds the configured size limit"
	))
}

fn flush_response_text(
	output: &mut Vec<responses::OutputItem>,
	pending: &mut Option<(usize, Vec<responses::OutputMessageContent>)>,
	message_id: &str,
	status: responses::OutputStatus,
	phase: responses::MessagePhase,
	buffer_limit: usize,
	retained_bytes: &mut usize,
) -> Result<(), AIError> {
	if let Some((index, content)) = pending.take() {
		let item = responses::OutputItem::Message(responses::OutputMessage {
			content,
			id: format!("msg_{message_id}_{index}"),
			role: responses::AssistantRole::Assistant,
			phase: Some(phase),
			status,
		});
		retain_response_item(output, item, buffer_limit, retained_bytes)?;
	}
	Ok(())
}

fn response_tool_output(
	message_id: &str,
	index: usize,
	status: responses::OutputStatus,
	call_id: &str,
	upstream_name: &str,
	input: serde_json::Value,
	state: &State,
) -> Result<responses::OutputItem, AIError> {
	let declared = state
		.tools
		.get(upstream_name)
		.ok_or_else(invalid_response)?;
	match declared {
		DeclaredTool::Function => Ok(responses::OutputItem::FunctionCall(
			responses::FunctionToolCall {
				arguments: serde_json::to_string(&input).map_err(|_| invalid_response())?,
				call_id: call_id.to_string(),
				namespace: None,
				name: upstream_name.to_string(),
				caller: None,
				id: Some(format!("fc_{message_id}_{index}")),
				status: Some(status),
			},
		)),
		DeclaredTool::Custom => {
			let content = input
				.as_object()
				.and_then(|input| input.get("content"))
				.and_then(serde_json::Value::as_str)
				.ok_or_else(invalid_response)?;
			stream_item(serde_json::json!({
				"type": "custom_tool_call",
				"id": format!("ctc_{message_id}_{index}"),
				"call_id": call_id,
				"name": upstream_name,
				"input": content,
			}))
			.map_err(|_| invalid_response())
		},
	}
}

struct StreamTextBlock {
	index: usize,
	output_index: u32,
	content_index: u32,
	item_id: String,
	text: String,
}

struct StreamToolBlock {
	index: usize,
	output_index: u32,
	item_id: String,
	call_id: String,
	upstream_name: String,
	json: String,
}

enum StreamBlock {
	Text(StreamTextBlock),
	Tool(StreamToolBlock),
	/// A thinking block being discarded. Track only its lifecycle so reasoning never reaches the
	/// client while malformed upstream streams are still rejected.
	DroppedThinking {
		index: usize,
		signature_seen: bool,
	},
	/// A redacted_thinking block being discarded. These blocks have no deltas.
	DroppedRedactedThinking {
		index: usize,
	},
}

#[derive(Default)]
struct ResponsesStreamState {
	sequence_number: u64,
	message_id: Option<String>,
	upstream_model: Option<String>,
	initial_usage: Option<messages::Usage>,
	terminal_usage: Option<messages::MessageDeltaUsage>,
	stop_reason: Option<messages::StopReason>,
	stop_sequence: Option<String>,
	active_block: Option<StreamBlock>,
	output: Vec<responses::OutputItem>,
	retained_output_bytes: usize,
	next_block_index: usize,
	tool_ids: HashSet<String>,
	tool_id_bytes: usize,
	saw_tool: bool,
	saw_message_delta: bool,
	terminated: bool,
	terminal_ready: bool,
	first_visible_at: Option<Instant>,
	completion: Option<String>,
	output_messages: Option<Vec<types::OutputMessage>>,
}

impl ResponsesStreamState {
	fn sequence(&mut self) -> Result<u64, ()> {
		let current = self.sequence_number;
		self.sequence_number = self.sequence_number.checked_add(1).ok_or(())?;
		Ok(current)
	}

	fn error_event(&mut self) -> Vec<(&'static str, responses::ResponseStreamEvent)> {
		if self.terminated {
			return Vec::new();
		}
		self.terminated = true;
		let sequence_number = self.sequence().unwrap_or(u64::MAX);
		vec![(
			"error",
			responses::ResponseStreamEvent::ResponseError(responses::ResponseErrorEvent {
				sequence_number,
				code: Some("server_error".to_string()),
				message: "Upstream Anthropic stream was invalid".to_string(),
				param: None,
			}),
		)]
	}

	fn retain_output(&mut self, item: responses::OutputItem) -> Result<(), ()> {
		let bytes = serde_json::to_vec(&item).map_err(|_| ())?.len();
		self.retained_output_bytes = self
			.retained_output_bytes
			.checked_add(bytes)
			.and_then(|total| total.checked_add(usize::from(!self.output.is_empty())))
			.ok_or(())?;
		self.output.push(item);
		Ok(())
	}

	fn retain_text_part(&mut self, block: &StreamTextBlock) -> Result<(), ()> {
		let output_index = usize::try_from(block.output_index).map_err(|_| ())?;
		if output_index == self.output.len() {
			if block.content_index != 0 {
				return Err(());
			}
			let mut item = stream_message_item(
				block.item_id.clone(),
				block.text.clone(),
				responses::OutputStatus::Completed,
			);
			set_output_item_status(&mut item, responses::OutputStatus::InProgress)?;
			return self.retain_output(item);
		}
		if output_index + 1 != self.output.len() {
			return Err(());
		}
		let responses::OutputItem::Message(message) = &mut self.output[output_index] else {
			return Err(());
		};
		if message.id != block.item_id
			|| message.status != responses::OutputStatus::InProgress
			|| message.content.len() != usize::try_from(block.content_index).map_err(|_| ())?
		{
			return Err(());
		}
		let part = responses::OutputMessageContent::OutputText(responses::OutputTextContent {
			annotations: Vec::new(),
			logprobs: None,
			text: block.text.clone(),
		});
		let added_bytes = serde_json::to_vec(&part)
			.map_err(|_| ())?
			.len()
			.checked_add(usize::from(!message.content.is_empty()))
			.ok_or(())?;
		self.retained_output_bytes = self
			.retained_output_bytes
			.checked_add(added_bytes)
			.ok_or(())?;
		message.content.push(part);
		Ok(())
	}

	fn retain_tool_id(&mut self, id: String) -> Result<(), ()> {
		let tool_id_bytes = self.tool_id_bytes.checked_add(id.len()).ok_or(())?;
		if !self.tool_ids.insert(id) {
			return Err(());
		}
		self.tool_id_bytes = tool_id_bytes;
		Ok(())
	}

	fn ensure_retained_limit(&self, limit: usize, downstream_model: &str) -> Result<(), ()> {
		fn add(total: &mut usize, bytes: usize) -> Result<(), ()> {
			*total = total.checked_add(bytes).ok_or(())?;
			Ok(())
		}

		let mut total = downstream_model.len();
		for value in [
			self.message_id.as_deref(),
			self.upstream_model.as_deref(),
			self.stop_sequence.as_deref(),
			self.completion.as_deref(),
			self
				.initial_usage
				.as_ref()
				.and_then(|usage| usage.service_tier.as_deref()),
		]
		.into_iter()
		.flatten()
		{
			add(&mut total, value.len())?;
		}
		if self.message_id.is_some() {
			add(
				&mut total,
				"resp_".len() + self.message_id.as_deref().map_or(0, str::len) + downstream_model.len(),
			)?;
		}
		add(&mut total, self.tool_id_bytes)?;
		if let Some(block) = &self.active_block {
			match block {
				// Dropped reasoning retains nothing.
				StreamBlock::DroppedThinking { .. } | StreamBlock::DroppedRedactedThinking { .. } => {},
				StreamBlock::Text(block) => {
					add(&mut total, block.item_id.len())?;
					add(&mut total, block.text.len())?;
				},
				StreamBlock::Tool(block) => {
					for value in [
						&block.item_id,
						&block.call_id,
						&block.upstream_name,
						&block.json,
					] {
						add(&mut total, value.len())?;
					}
				},
			}
		}
		add(&mut total, self.retained_output_bytes)?;
		add(&mut total, 2)?;
		(total <= limit).then_some(()).ok_or(())
	}

	fn mark_visible(&mut self) {
		self.first_visible_at.get_or_insert_with(Instant::now);
	}
}

fn stream_output_part(text: String) -> responses::OutputContent {
	responses::OutputContent::OutputText(responses::OutputTextContent {
		annotations: Vec::new(),
		logprobs: None,
		text,
	})
}

fn stream_message_item(
	item_id: String,
	text: String,
	status: responses::OutputStatus,
) -> responses::OutputItem {
	let in_progress = matches!(status, responses::OutputStatus::InProgress);
	responses::OutputItem::Message(responses::OutputMessage {
		content: if text.is_empty() && in_progress {
			Vec::new()
		} else {
			vec![responses::OutputMessageContent::OutputText(
				responses::OutputTextContent {
					annotations: Vec::new(),
					logprobs: None,
					text,
				},
			)]
		},
		id: item_id,
		role: responses::AssistantRole::Assistant,
		// Commentary and final answer cannot be told apart until the terminal stop reason
		// arrives, so the in-flight item omits phase and the terminal loop fills it in.
		phase: None,
		status,
	})
}

fn stream_item(value: serde_json::Value) -> Result<responses::OutputItem, ()> {
	serde_json::from_value(value).map_err(|_| ())
}

fn set_output_item_status(
	item: &mut responses::OutputItem,
	status: responses::OutputStatus,
) -> Result<(), ()> {
	match item {
		responses::OutputItem::Message(message) => message.status = status,
		responses::OutputItem::FunctionCall(call) => call.status = Some(status),
		responses::OutputItem::Reasoning(reasoning) => reasoning.status = Some(status),
		responses::OutputItem::CustomToolCall(_) => {},
		_ => return Err(()),
	}
	Ok(())
}

fn stream_tool_added_item(
	message_id: &str,
	index: usize,
	call_id: &str,
	upstream_name: &str,
	state: &State,
) -> Result<(String, responses::OutputItem), ()> {
	let declared = state.tools.get(upstream_name).ok_or(())?;
	let prefix = match declared {
		DeclaredTool::Function => "fc",
		DeclaredTool::Custom => "ctc",
	};
	let item_id = format!("{prefix}_{message_id}_{index}");
	let added = match declared {
		DeclaredTool::Function => Ok(responses::OutputItem::FunctionCall(
			responses::FunctionToolCall {
				arguments: String::new(),
				call_id: call_id.to_string(),
				namespace: None,
				name: upstream_name.to_string(),
				caller: None,
				id: Some(item_id.clone()),
				status: Some(responses::OutputStatus::InProgress),
			},
		)),
		DeclaredTool::Custom => stream_item(serde_json::json!({
			"type":"custom_tool_call", "id":item_id.clone(), "call_id":call_id,
			"name":upstream_name, "input":""
		})),
	};
	Ok((item_id, added?))
}

fn stream_usage(
	initial: &messages::Usage,
	terminal: &messages::MessageDeltaUsage,
) -> Result<responses::ResponseUsage, ()> {
	let thinking = stream_thinking_tokens(initial, terminal)?;
	if terminal
		.input_tokens
		.is_some_and(|value| value < initial.input_tokens)
		|| terminal
			.output_tokens
			.is_some_and(|value| value < initial.output_tokens)
		|| terminal
			.cache_read_input_tokens
			.is_some_and(|value| value < initial.cache_read_input_tokens.unwrap_or_default())
		|| terminal
			.cache_creation_input_tokens
			.is_some_and(|value| value < initial.cache_creation_input_tokens.unwrap_or_default())
	{
		return Err(());
	}
	let input = terminal.input_tokens.unwrap_or(initial.input_tokens);
	let output = terminal.output_tokens.unwrap_or(initial.output_tokens);
	let cache_read = terminal
		.cache_read_input_tokens
		.or(initial.cache_read_input_tokens)
		.unwrap_or_default();
	let cache_creation = terminal
		.cache_creation_input_tokens
		.or(initial.cache_creation_input_tokens);
	response_usage(input, output, cache_read, cache_creation, thinking)
}

fn stream_service_tier(tier: Option<&str>) -> Result<Option<responses::ServiceTier>, ()> {
	tier.map(public_service_tier).transpose().map_err(|_| ())
}

fn stream_thinking_tokens(
	initial: &messages::Usage,
	terminal: &messages::MessageDeltaUsage,
) -> Result<Option<usize>, ()> {
	let initial = initial
		.output_tokens_details
		.as_ref()
		.and_then(|details| details.thinking_tokens);
	let terminal = terminal
		.output_tokens_details
		.as_ref()
		.and_then(|details| details.thinking_tokens);
	if terminal
		.zip(initial)
		.is_some_and(|(terminal, initial)| terminal < initial)
	{
		return Err(());
	}
	Ok(terminal.or(initial))
}

fn commit_stream_telemetry(
	stream: &ResponsesStreamState,
	log: &StreamingUsageGuard,
) -> Result<(), ()> {
	let initial = stream.initial_usage.as_ref().ok_or(())?;
	let terminal = stream.terminal_usage.as_ref().ok_or(())?;
	let usage = stream_usage(initial, terminal)?;
	let input_tokens =
		u64::try_from(terminal.input_tokens.unwrap_or(initial.input_tokens)).map_err(|_| ())?;
	let output_tokens =
		u64::try_from(terminal.output_tokens.unwrap_or(initial.output_tokens)).map_err(|_| ())?;
	let total_tokens = input_tokens.checked_add(output_tokens).ok_or(())?;
	let cache_creation = terminal
		.cache_creation_input_tokens
		.or(initial.cache_creation_input_tokens)
		.map(|value| value as u64);
	let reasoning_tokens = stream_thinking_tokens(initial, terminal)?
		.map(u64::try_from)
		.transpose()
		.map_err(|_| ())?;
	let provider_model = stream.upstream_model.clone();
	let completion = stream.completion.clone();
	let first_token = stream.first_visible_at;
	let service_tier = initial.service_tier.clone();
	log.update(|info| {
		info.response.input_tokens = Some(input_tokens);
		info.response.output_tokens = Some(output_tokens);
		info.response.total_tokens = Some(total_tokens);
		info.response.reasoning_tokens = reasoning_tokens;
		info.response.cached_input_tokens = Some(u64::from(usage.input_tokens_details.cached_tokens));
		info.response.cache_creation_input_tokens = cache_creation;
		info.response.provider_model = provider_model.as_deref().map(strng::new);
		info.response.first_token = first_token;
		info.response.service_tier = service_tier.as_deref().map(strng::new);
		if let Some(completion) = completion.clone() {
			info.response.completion = Some(vec![completion]);
		}
		info.response.output_messages = stream.output_messages.clone();
	});
	Ok(())
}

pub fn translate_stream(
	body: Body,
	buffer_limit: usize,
	log: StreamingUsageGuard,
	model: &str,
	log_content: LogContentFields,
	conversion_state: State,
) -> Body {
	let mut stream = ResponsesStreamState {
		completion: log_content.completion.then(String::new),
		..Default::default()
	};
	let mut response_builder: Option<types::responses::ResponseBuilder> = None;
	let model = model.to_string();

	parse::sse::json_transform_multi::<messages::MessagesStreamEvent, responses::ResponseStreamEvent, _>(
		body,
		buffer_limit,
		move |event| {
			if stream.terminated {
				return Vec::new();
			}
			let data = match event {
				SseJsonEvent::Data(data) => data,
				SseJsonEvent::Done => return stream.error_event(),
				SseJsonEvent::Eof => return stream.error_event(),
				SseJsonEvent::Error => return stream.error_event(),
			};
			let Ok(event) = data else {
				return stream.error_event();
			};

			let sequence_checkpoint = stream.sequence_number;
			let result = (|| -> Result<Vec<(&'static str, responses::ResponseStreamEvent)>, ()> {
				match event {
					messages::MessagesStreamEvent::MessageStart { message } => {
						if stream.message_id.is_some()
							|| message.r#type != "message"
							|| message.role != messages::Role::Assistant
							|| !message.content.is_empty()
							|| message.stop_reason.is_some()
							|| message.stop_sequence.is_some()
							|| message.id.is_empty()
							|| message.model.is_empty()
						{
							return Err(());
						}
						let service_tier = stream_service_tier(message.usage.service_tier.as_deref())?;
						let upstream_model = message.model.clone();
						let builder = types::responses::ResponseBuilder::new(
							format!("resp_{}", message.id),
							upstream_model.clone(),
						);
						let mut snapshot = builder.response(responses::Status::InProgress, None, None, None);
						snapshot.service_tier = service_tier;
						let created =
							responses::ResponseStreamEvent::ResponseCreated(responses::ResponseCreatedEvent {
								sequence_number: stream.sequence()?,
								response: snapshot.clone(),
							});
						let in_progress = responses::ResponseStreamEvent::ResponseInProgress(
							responses::ResponseInProgressEvent {
								sequence_number: stream.sequence()?,
								response: snapshot,
							},
						);
						stream.message_id = Some(message.id);
						stream.upstream_model = Some(upstream_model);
						stream.initial_usage = Some(message.usage);
						response_builder = Some(builder);
						Ok(vec![
							("response.created", created),
							("response.in_progress", in_progress),
						])
					},
					messages::MessagesStreamEvent::ContentBlockStart {
						index,
						content_block,
					} => {
						if stream.message_id.is_none()
							|| stream.active_block.is_some()
							|| stream.saw_message_delta
							|| index != stream.next_block_index
						{
							return Err(());
						}
						let output_index = u32::try_from(stream.output.len()).map_err(|_| ())?;
						let message_id = stream.message_id.clone().ok_or(())?;
						stream.next_block_index = stream.next_block_index.checked_add(1).ok_or(())?;
						match content_block {
							messages::ContentBlock::Text(text) => {
								if !text.text.is_empty() {
									return Err(());
								}
								let (output_index, content_index, item_id, add_item) =
									if let Some(responses::OutputItem::Message(message)) = stream.output.last() {
										(
											u32::try_from(stream.output.len() - 1).map_err(|_| ())?,
											u32::try_from(message.content.len()).map_err(|_| ())?,
											message.id.clone(),
											false,
										)
									} else {
										(output_index, 0, format!("msg_{message_id}_{index}"), true)
									};
								stream.active_block = Some(StreamBlock::Text(StreamTextBlock {
									index,
									output_index,
									content_index,
									item_id: item_id.clone(),
									text: String::new(),
								}));
								let mut events = Vec::new();
								if add_item {
									events.push((
										"response.output_item.added",
										responses::ResponseStreamEvent::ResponseOutputItemAdded(
											responses::ResponseOutputItemAddedEvent {
												sequence_number: stream.sequence()?,
												output_index,
												item: stream_message_item(
													item_id.clone(),
													String::new(),
													responses::OutputStatus::InProgress,
												),
											},
										),
									));
								}
								events.push((
									"response.content_part.added",
									responses::ResponseStreamEvent::ResponseContentPartAdded(
										responses::ResponseContentPartAddedEvent {
											sequence_number: stream.sequence()?,
											item_id,
											output_index,
											content_index,
											part: stream_output_part(String::new()),
										},
									),
								));
								Ok(events)
							},
							messages::ContentBlock::ToolUse {
								id,
								name,
								input,
								caller,
								cache_control: _,
							} => {
								if id.is_empty()
									|| name.is_empty()
									|| input.as_object().is_none_or(|input| !input.is_empty())
									|| !direct_tool_caller(caller.as_ref())
								{
									return Err(());
								}
								stream.retain_tool_id(id.clone())?;
								stream.saw_tool = true;
								let (item_id, added) =
									stream_tool_added_item(&message_id, index, &id, &name, &conversion_state)?;
								stream.active_block = Some(StreamBlock::Tool(StreamToolBlock {
									index,
									output_index,
									item_id,
									call_id: id,
									upstream_name: name,
									json: String::new(),
								}));
								Ok(vec![(
									"response.output_item.added",
									responses::ResponseStreamEvent::ResponseOutputItemAdded(
										responses::ResponseOutputItemAddedEvent {
											sequence_number: stream.sequence()?,
											output_index,
											item: added,
										},
									),
								)])
							},
							// Adaptive thinking is the default for several Copilot Claude models, so a
							// thinking block is valid upstream output. Absorb it and its deltas rather
							// than terminating the stream. See response_output for the buffered path.
							messages::ContentBlock::Thinking { .. } => {
								stream.active_block = Some(StreamBlock::DroppedThinking {
									index,
									signature_seen: false,
								});
								Ok(Vec::new())
							},
							messages::ContentBlock::RedactedThinking { .. } => {
								stream.active_block = Some(StreamBlock::DroppedRedactedThinking { index });
								Ok(Vec::new())
							},
							_ => Err(()),
						}
					},
					messages::MessagesStreamEvent::ContentBlockDelta { index, delta } => {
						if stream.saw_message_delta {
							return Err(());
						}
						let mut block = stream.active_block.take().ok_or(())?;
						let events = match (&mut block, delta) {
							(StreamBlock::Text(block), messages::ContentBlockDelta::TextDelta { text })
								if block.index == index =>
							{
								block.text.push_str(&text);
								if let Some(completion) = stream.completion.as_mut() {
									completion.push_str(&text);
								}
								if !text.is_empty() {
									stream.mark_visible();
								}
								vec![(
									"response.output_text.delta",
									responses::ResponseStreamEvent::ResponseOutputTextDelta(
										responses::ResponseTextDeltaEvent {
											sequence_number: stream.sequence()?,
											item_id: block.item_id.clone(),
											output_index: block.output_index,
											content_index: block.content_index,
											delta: text,
											logprobs: None,
										},
									),
								)]
							},
							(StreamBlock::Text(block), messages::ContentBlockDelta::CitationsDelta { .. })
								if block.index == index =>
							{
								Vec::new()
							},
							(
								StreamBlock::Tool(block),
								messages::ContentBlockDelta::InputJsonDelta { partial_json },
							) if block.index == index => {
								block.json.push_str(&partial_json);
								let declared = conversion_state.tools.get(&block.upstream_name).ok_or(())?;
								match declared {
									DeclaredTool::Function => {
										if !partial_json.is_empty() {
											stream.mark_visible();
										}
										vec![(
											"response.function_call_arguments.delta",
											responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
												responses::ResponseFunctionCallArgumentsDeltaEvent {
													sequence_number: stream.sequence()?,
													item_id: block.item_id.clone(),
													output_index: block.output_index,
													delta: partial_json,
												},
											),
										)]
									},
									DeclaredTool::Custom => Vec::new(),
								}
							},
							(
								StreamBlock::DroppedThinking {
									index: block_index,
									signature_seen: false,
								},
								messages::ContentBlockDelta::ThinkingDelta { .. },
							) if *block_index == index => Vec::new(),
							(
								StreamBlock::DroppedThinking {
									index: block_index,
									signature_seen,
								},
								messages::ContentBlockDelta::SignatureDelta { signature },
							) if *block_index == index && !*signature_seen && !signature.is_empty() => {
								*signature_seen = true;
								Vec::new()
							},
							_ => return Err(()),
						};
						stream.active_block = Some(block);
						Ok(events)
					},
					messages::MessagesStreamEvent::ContentBlockStop { index } => {
						let block = stream.active_block.take().ok_or(())?;
						match block {
							StreamBlock::DroppedThinking {
								index: block_index,
								signature_seen: true,
							} if block_index == index => Ok(Vec::new()),
							StreamBlock::DroppedRedactedThinking { index: block_index }
								if block_index == index =>
							{
								Ok(Vec::new())
							},
							StreamBlock::Text(block) if block.index == index => {
								stream.retain_text_part(&block)?;
								Ok(vec![
									(
										"response.output_text.done",
										responses::ResponseStreamEvent::ResponseOutputTextDone(
											responses::ResponseTextDoneEvent {
												sequence_number: stream.sequence()?,
												item_id: block.item_id.clone(),
												output_index: block.output_index,
												content_index: block.content_index,
												text: block.text.clone(),
												logprobs: None,
											},
										),
									),
									(
										"response.content_part.done",
										responses::ResponseStreamEvent::ResponseContentPartDone(
											responses::ResponseContentPartDoneEvent {
												sequence_number: stream.sequence()?,
												item_id: block.item_id,
												output_index: block.output_index,
												content_index: block.content_index,
												part: stream_output_part(block.text),
											},
										),
									),
								])
							},
							StreamBlock::Tool(block) if block.index == index => {
								let raw = if block.json.is_empty() {
									"{}"
								} else {
									&block.json
								};
								let input: serde_json::Value = serde_json::from_str(raw).map_err(|_| ())?;
								let input_object = input.as_object().ok_or(())?;
								let declared = conversion_state.tools.get(&block.upstream_name).ok_or(())?;
								if matches!(declared, DeclaredTool::Custom)
									&& input_object
										.get("content")
										.and_then(serde_json::Value::as_str)
										.is_none()
								{
									return Err(());
								}
								let item = response_tool_output(
									stream.message_id.as_deref().ok_or(())?,
									index,
									responses::OutputStatus::InProgress,
									&block.call_id,
									&block.upstream_name,
									input.clone(),
									&conversion_state,
								)
								.map_err(|_| ())?;
								stream.retain_output(item.clone())?;
								match declared {
									DeclaredTool::Function => {
										let arguments = raw.to_string();
										Ok(vec![(
											"response.function_call_arguments.done",
											responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
												responses::ResponseFunctionCallArgumentsDoneEvent {
													name: Some(block.upstream_name.clone()),
													sequence_number: stream.sequence()?,
													item_id: block.item_id.clone(),
													output_index: block.output_index,
													arguments,
												},
											),
										)])
									},
									DeclaredTool::Custom => {
										let input = input_object["content"].as_str().ok_or(())?.to_string();
										if !input.is_empty() {
											stream.mark_visible();
										}
										Ok(vec![
											(
												"response.custom_tool_call_input.delta",
												responses::ResponseStreamEvent::ResponseCustomToolCallInputDelta(
													responses::ResponseCustomToolCallInputDeltaEvent {
														sequence_number: stream.sequence()?,
														output_index: block.output_index,
														item_id: block.item_id.clone(),
														delta: input.clone(),
													},
												),
											),
											(
												"response.custom_tool_call_input.done",
												responses::ResponseStreamEvent::ResponseCustomToolCallInputDone(
													responses::ResponseCustomToolCallInputDoneEvent {
														sequence_number: stream.sequence()?,
														output_index: block.output_index,
														item_id: block.item_id.clone(),
														input,
													},
												),
											),
										])
									},
								}
							},
							_ => Err(()),
						}
					},
					messages::MessagesStreamEvent::MessageDelta { delta, usage } => {
						if stream.message_id.is_none()
							|| stream.active_block.is_some()
							|| stream.saw_message_delta
							|| delta.stop_reason.is_none()
						{
							return Err(());
						}
						stream.saw_message_delta = true;
						stream.stop_reason = delta.stop_reason;
						stream.stop_sequence = delta.stop_sequence;
						stream.terminal_usage = Some(usage);
						Ok(Vec::new())
					},
					messages::MessagesStreamEvent::MessageStop => {
						if !stream.saw_message_delta || stream.active_block.is_some() {
							return Err(());
						}
						let initial = stream.initial_usage.clone().ok_or(())?;
						let terminal = stream.terminal_usage.clone().ok_or(())?;
						let usage = stream_usage(&initial, &terminal)?;
						let stop_reason = stream.stop_reason.ok_or(())?;
						if (matches!(stop_reason, messages::StopReason::ToolUse) && !stream.saw_tool)
							|| (stream.saw_tool
								&& matches!(
									stop_reason,
									messages::StopReason::EndTurn | messages::StopReason::StopSequence
								)) {
							return Err(());
						}
						if match stop_reason {
							messages::StopReason::StopSequence => !stream
								.stop_sequence
								.as_ref()
								.is_some_and(|value| !value.is_empty()),
							_ => stream.stop_sequence.is_some(),
						} {
							return Err(());
						}
						stream.ensure_retained_limit(buffer_limit, &model)?;
						let builder = response_builder.as_ref().ok_or(())?;
						let (status, incomplete_reason) = terminal_status(stop_reason).ok_or(())?;
						let output_status = if status == "completed" {
							responses::OutputStatus::Completed
						} else {
							responses::OutputStatus::Incomplete
						};
						for item in &mut stream.output {
							set_output_item_status(item, output_status)?;
							if let responses::OutputItem::Message(message) = item {
								message.phase = Some(message_phase(stop_reason));
							}
						}
						stream.retained_output_bytes = serde_json::to_vec(&stream.output)
							.map_err(|_| ())?
							.len()
							.checked_sub(2)
							.ok_or(())?;
						stream.ensure_retained_limit(buffer_limit, &model)?;
						let output = std::mem::take(&mut stream.output);
						if log_content.tool_calls {
							let content = output
								.iter()
								.filter_map(types::responses::output_item_tool_call_part)
								.collect::<Vec<_>>();
							if !content.is_empty() {
								stream.output_messages = Some(vec![types::OutputMessage {
									role: strng::literal!("assistant"),
									content,
									finish_reason: Some(strng::new(status)),
								}]);
							}
						}
						let mut events = Vec::with_capacity(output.len() + 1);
						for (output_index, item) in output.iter().cloned().enumerate() {
							events.push((
								"response.output_item.done",
								responses::ResponseStreamEvent::ResponseOutputItemDone(
									responses::ResponseOutputItemDoneEvent {
										sequence_number: stream.sequence()?,
										output_index: u32::try_from(output_index).map_err(|_| ())?,
										item,
									},
								),
							));
						}
						let mut response = builder.response(
							if status == "completed" {
								responses::Status::Completed
							} else {
								responses::Status::Incomplete
							},
							Some(usage.clone()),
							None,
							incomplete_reason.map(|reason| responses::IncompleteDetails {
								reason: reason.to_string(),
							}),
						);
						response.output = output;
						response.service_tier = stream_service_tier(initial.service_tier.as_deref())?;
						let sequence_number = stream.sequence()?;
						let event = if status == "completed" {
							responses::ResponseStreamEvent::ResponseCompleted(responses::ResponseCompletedEvent {
								sequence_number,
								response,
							})
						} else {
							responses::ResponseStreamEvent::ResponseIncomplete(
								responses::ResponseIncompleteEvent {
									sequence_number,
									response,
								},
							)
						};
						stream.terminal_ready = true;
						events.push((
							if status == "completed" {
								"response.completed"
							} else {
								"response.incomplete"
							},
							event,
						));
						Ok(events)
					},
					messages::MessagesStreamEvent::Ping => {
						// A ping is a content-free keepalive Anthropic may send at any point in the
						// stream, including before message_start while a request is queued -- unlike
						// every other event type, it carries no state that depends on message_id
						// already being set, so there is nothing to validate here.
						Ok(Vec::new())
					},
				}
			})();
			let result = result.and_then(|events| {
				stream.ensure_retained_limit(buffer_limit, &model)?;
				Ok(events)
			});
			match result {
				Ok(events) => {
					if stream.terminal_ready {
						if commit_stream_telemetry(&stream, &log).is_err() {
							stream.sequence_number = sequence_checkpoint;
							stream.error_event()
						} else {
							stream.terminated = true;
							events
						}
					} else {
						events
					}
				},
				Err(()) => {
					stream.sequence_number = sequence_checkpoint;
					stream.error_event()
				},
			}
		},
	)
}

fn invalid_response() -> AIError {
	AIError::InvalidResponse(strng::literal!("invalid Anthropic Messages response"))
}

#[cfg(test)]
mod tests;
