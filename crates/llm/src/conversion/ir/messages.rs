//! Reads an Anthropic Messages request into the owned model.

use serde_json::{Map, Value};

use super::{
	CacheMark, Document, DropReason, Dropped, Effort, Message, OutputFormat, Part, Reasoning,
	ReasoningMode, Request, Role, Source, TextPart, Tool, ToolChoice, ToolResultPart,
};
use crate::types::messages::typed as wire;
use crate::{AIError, InputFormat, json as json_util, types};

/// Fields a custom tool reads. `cache_control` is held apart on the loose tool.
const CUSTOM_TOOL_FIELDS: [&str; 4] = ["name", "description", "input_schema", "strict"];

/// Top-level fields the typed request reads from the loose request's flattened extras.
const TYPED_EXTRAS: [&str; 5] = [
	"stop_sequences",
	"top_k",
	"tool_choice",
	"metadata",
	"thinking",
];

/// Reads a Messages request. Fails only on input that cannot be translated at all.
pub fn parse_request(
	req: &types::messages::Request,
	dropped: &mut Vec<Dropped>,
) -> Result<Request, AIError> {
	// The typed conversion erases keys it does not model, so account for them first.
	record_unread_keys(String::new, &req.rest, &TYPED_EXTRAS, dropped);
	if let Some(output_config) = &req.output_config {
		record_unread_keys(
			|| "output_config".to_string(),
			&output_config.rest,
			&["format"],
			dropped,
		);
	}
	for (index, message) in req.messages.iter().enumerate() {
		record_unread_keys(|| format!("messages[{index}]"), &message.rest, &[], dropped);
	}
	let typed = json_util::convert::<_, wire::Request>(req)
		.map_err(|err| AIError::RequestParsing(InputFormat::Messages, err))?;
	record_unread_tool_fields(req, &typed, dropped);
	let wire::Request {
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
		cache_control,
	} = typed;
	let wire::OutputConfig { effort, format } = output_config.unwrap_or_default();

	let messages = messages
		.into_iter()
		.enumerate()
		.map(|(index, message)| read_message(index, message, dropped))
		.collect();

	let (tool_choice, parallel_tool_calls) = tool_choice.map(read_tool_choice).unzip();

	Ok(Request {
		model,
		max_output_tokens: max_tokens,
		instructions: read_system(system),
		messages,
		temperature,
		top_p,
		top_k,
		stop_sequences,
		stream,
		reasoning: read_reasoning(thinking, effort),
		output_format: format.map(|format| match format {
			wire::OutputFormat::JsonSchema { schema } => OutputFormat::JsonSchema { schema },
		}),
		metadata: metadata.map_or_default(|metadata| metadata.fields.into_iter().collect()),
		tools: tools.into_iter().flatten().map(read_tool).collect(),
		tool_choice,
		parallel_tool_calls: parallel_tool_calls.flatten(),
		cache: cache_control.map(read_cache),
	})
}

/// Records keys of a loose object's flattened extras that the typed request does not read.
fn record_unread_keys(
	parent: impl FnOnce() -> String,
	extras: &Value,
	read: &[&str],
	dropped: &mut Vec<Dropped>,
) {
	let Value::Object(extras) = extras else {
		return;
	};
	let mut unread = extras
		.keys()
		.filter(|key| !read.contains(&key.as_str()))
		.peekable();
	if unread.peek().is_none() {
		return;
	}
	let parent = parent();
	for key in unread {
		dropped.push(Dropped::unread_key(&parent, key));
	}
}

fn read_system(system: Option<wire::SystemPrompt>) -> Vec<TextPart> {
	match system {
		None => Vec::new(),
		Some(wire::SystemPrompt::Text(text)) => vec![TextPart {
			text,
			citations: None,
			cache: None,
		}],
		Some(wire::SystemPrompt::Blocks(blocks)) => blocks
			.into_iter()
			.map(|block| match block {
				wire::SystemContentBlock::Text {
					text,
					cache_control,
				} => TextPart {
					text,
					citations: None,
					cache: cache_control.map(read_cache),
				},
			})
			.collect(),
	}
}

fn read_reasoning(
	thinking: Option<wire::ThinkingInput>,
	effort: Option<wire::ThinkingEffort>,
) -> Reasoning {
	let (mode, display) = match thinking {
		None => (None, None),
		Some(wire::ThinkingInput::Disabled {}) => (Some(ReasoningMode::Off), None),
		Some(wire::ThinkingInput::Adaptive { display }) => (Some(ReasoningMode::Adaptive), display),
		Some(wire::ThinkingInput::Enabled {
			budget_tokens,
			display,
		}) => (
			Some(ReasoningMode::Budget {
				tokens: budget_tokens,
				fallback_effort: read_effort(crate::types::anthropic_effort_for_thinking_budget(
					budget_tokens,
				)),
			}),
			display,
		),
	};
	Reasoning {
		mode,
		effort: effort.map(read_effort),
		display,
	}
}

fn read_effort(effort: wire::ThinkingEffort) -> Effort {
	match effort {
		wire::ThinkingEffort::Low => Effort::Low,
		wire::ThinkingEffort::Medium => Effort::Medium,
		wire::ThinkingEffort::High => Effort::High,
		wire::ThinkingEffort::Xhigh => Effort::Xhigh,
		wire::ThinkingEffort::Max => Effort::Max,
	}
}

fn read_tool(tool: wire::Tool) -> Tool {
	match tool {
		wire::Tool::Custom(wire::CustomTool {
			name,
			description,
			input_schema,
			strict,
			cache_control,
		}) => Tool::Function {
			name,
			description,
			parameters: input_schema,
			strict,
			cache: cache_control.map(read_cache),
		},
		wire::Tool::Server(wire::ServerTool {
			tool_type,
			name,
			cache_control,
			extra,
		}) => Tool::Server {
			kind: tool_type,
			name,
			cache: cache_control.map(read_cache),
			config: extra,
		},
	}
}

/// Messages asks whether to disable parallel calls; the model keeps whether to allow them.
fn read_tool_choice(choice: wire::ToolChoice) -> (ToolChoice, Option<bool>) {
	let (choice, disable_parallel_tool_use) = match choice {
		wire::ToolChoice::Auto {
			disable_parallel_tool_use,
		} => (ToolChoice::Auto, disable_parallel_tool_use),
		wire::ToolChoice::Any {
			disable_parallel_tool_use,
		} => (ToolChoice::Required, disable_parallel_tool_use),
		wire::ToolChoice::Tool {
			name,
			disable_parallel_tool_use,
		} => (ToolChoice::Tool(name), disable_parallel_tool_use),
		wire::ToolChoice::None {} => (ToolChoice::None, None),
	};
	(choice, disable_parallel_tool_use.map(|disable| !disable))
}

fn read_cache(cache: wire::CacheControlEphemeral) -> CacheMark {
	match cache {
		wire::CacheControlEphemeral::Ephemeral { ttl, extra } => CacheMark {
			ttl,
			unread: extra.into_iter().map(|(key, _)| key).collect(),
		},
	}
}

/// A custom tool erases fields it does not model. Server tools keep theirs as config.
fn record_unread_tool_fields(
	req: &types::messages::Request,
	typed: &wire::Request,
	dropped: &mut Vec<Dropped>,
) {
	let (Some(loose), Some(tools)) = (&req.tools, &typed.tools) else {
		return;
	};
	for (index, (loose, tool)) in loose.iter().zip(tools).enumerate() {
		let (wire::Tool::Custom(_), Value::Object(fields)) = (tool, &loose.rest) else {
			continue;
		};
		for key in fields.keys() {
			if !CUSTOM_TOOL_FIELDS.contains(&key.as_str()) {
				dropped.push(Dropped::unread_key(&format!("tools[{index}]"), key));
			}
		}
	}
}

fn read_message(index: usize, message: wire::Message, dropped: &mut Vec<Dropped>) -> Message {
	let role = match message.role {
		wire::Role::User => Role::User,
		wire::Role::Assistant => Role::Assistant,
		wire::Role::System => Role::System,
	};
	let parts = message
		.content
		.into_iter()
		.map(read_part)
		.collect::<Vec<_>>();
	for (part_index, part) in parts.iter().enumerate() {
		match part {
			Part::Unread => dropped.push(Dropped::new(
				format!("messages[{index}].content[{part_index}]"),
				DropReason::Unread,
			)),
			Part::ToolResult { content, .. } => {
				for (result_index, result) in content.iter().enumerate() {
					if matches!(result, ToolResultPart::Unread) {
						dropped.push(Dropped::new(
							format!("messages[{index}].content[{part_index}].content[{result_index}]"),
							DropReason::Unread,
						));
					}
				}
			},
			_ => {},
		}
	}
	Message { role, parts }
}

fn read_part(block: wire::ContentBlock) -> Part {
	match block {
		wire::ContentBlock::Text(text) => Part::Text(read_text(text)),
		wire::ContentBlock::Image(wire::ContentImageBlock {
			source,
			cache_control,
		}) => Part::Image {
			source: read_image_source(source),
			cache: cache_control.map(read_cache),
		},
		wire::ContentBlock::Document(wire::ContentDocumentBlock {
			source,
			cache_control,
			citations,
			context,
			title,
		}) => Part::Document(Document {
			source: read_document_source(source),
			title,
			context,
			citations,
			cache: cache_control.map(read_cache),
		}),
		wire::ContentBlock::Thinking {
			thinking,
			signature,
		} => Part::Reasoning {
			text: thinking,
			signature,
		},
		wire::ContentBlock::RedactedThinking { data } => Part::RedactedReasoning { data },
		wire::ContentBlock::ToolUse {
			id,
			name,
			input,
			cache_control,
		} => Part::ToolCall {
			id,
			name,
			arguments: input,
			cache: cache_control.map(read_cache),
		},
		wire::ContentBlock::ToolResult {
			tool_use_id,
			content,
			cache_control,
			is_error,
		} => Part::ToolResult {
			call_id: tool_use_id,
			content: read_tool_result_content(content),
			is_error,
			cache: cache_control.map(read_cache),
		},
		// Server tool calls and results are modeled once both halves can be normalized together.
		wire::ContentBlock::SearchResult(_)
		| wire::ContentBlock::ServerToolUse { .. }
		| wire::ContentBlock::WebSearchToolResult { .. }
		| wire::ContentBlock::Unknown => Part::Unread,
	}
}

fn read_text(text: wire::ContentTextBlock) -> TextPart {
	let wire::ContentTextBlock {
		text,
		citations,
		cache_control,
	} = text;
	TextPart {
		text,
		citations,
		cache: cache_control.map(read_cache),
	}
}

/// A plain string reads as one text part; destinations render both shapes the same.
fn read_tool_result_content(content: wire::ToolResultContent) -> Vec<ToolResultPart> {
	match content {
		wire::ToolResultContent::Text(text) => vec![ToolResultPart::Text(TextPart {
			text,
			citations: None,
			cache: None,
		})],
		wire::ToolResultContent::Array(parts) => parts
			.into_iter()
			.map(|part| match part {
				wire::ToolResultContentPart::Text {
					text,
					citations,
					cache_control,
				} => ToolResultPart::Text(TextPart {
					text,
					citations,
					cache: cache_control.map(read_cache),
				}),
				wire::ToolResultContentPart::ToolReference {
					tool_name,
					cache_control,
				} => ToolResultPart::ToolReference {
					tool_name,
					cache: cache_control.map(read_cache),
				},
				wire::ToolResultContentPart::Image {
					source,
					cache_control,
				} => ToolResultPart::Image {
					source: read_image_source(source),
					cache: cache_control.map(read_cache),
				},
				wire::ToolResultContentPart::Document {
					source,
					cache_control,
					citations,
					context,
					title,
				} => ToolResultPart::Document(Document {
					source: read_document_source(source),
					title,
					context,
					citations,
					cache: cache_control.map(read_cache),
				}),
				wire::ToolResultContentPart::SearchResult { .. } | wire::ToolResultContentPart::Unknown => {
					ToolResultPart::Unread
				},
			})
			.collect(),
	}
}

/// A source missing a field is kept as incomplete, so only a writer that sends it refuses it.
/// Takes the source by value to move payloads instead of copying them.
fn read_image_source(source: Value) -> Source {
	image_source(source).unwrap_or_else(Source::Incomplete)
}

fn image_source(source: Value) -> Result<Source, &'static str> {
	let mut source = source_object(source, "messages image source missing type")?;
	let kind = take_field(&mut source, "type", "messages image source missing type")?;
	Ok(match kind.as_str() {
		"base64" => Source::Base64 {
			media_type: take_field(
				&mut source,
				"media_type",
				"messages base64 image source missing media_type",
			)?,
			data: take_field(
				&mut source,
				"data",
				"messages base64 image source missing data",
			)?,
		},
		"url" => Source::Url(take_field(
			&mut source,
			"url",
			"messages url image source missing url",
		)?),
		"file" => Source::File(take_field(
			&mut source,
			"file_id",
			"messages file image source missing file_id",
		)?),
		_ => Source::Other(kind),
	})
}

fn read_document_source(source: Value) -> Source {
	document_source(source).unwrap_or_else(Source::Incomplete)
}

fn document_source(source: Value) -> Result<Source, &'static str> {
	let mut source = source_object(source, "messages document source missing type")?;
	let kind = take_field(&mut source, "type", "messages document source missing type")?;
	Ok(match kind.as_str() {
		"base64" => Source::Base64 {
			media_type: take_field(
				&mut source,
				"media_type",
				"messages base64 document source missing media_type",
			)?,
			data: take_field(
				&mut source,
				"data",
				"messages base64 document source missing data",
			)?,
		},
		"url" => Source::Url(take_field(
			&mut source,
			"url",
			"messages url document source missing url",
		)?),
		"file" => Source::File(take_field(
			&mut source,
			"file_id",
			"messages file document source missing file_id",
		)?),
		"text" => Source::Text(take_field(
			&mut source,
			"data",
			"messages text document source missing data",
		)?),
		_ => Source::Other(kind),
	})
}

fn source_object(source: Value, missing: &'static str) -> Result<Map<String, Value>, &'static str> {
	match source {
		Value::Object(source) => Ok(source),
		_ => Err(missing),
	}
}

fn take_field(
	source: &mut Map<String, Value>,
	field: &str,
	missing: &'static str,
) -> Result<String, &'static str> {
	match source.remove(field) {
		Some(Value::String(value)) => Ok(value),
		_ => Err(missing),
	}
}
