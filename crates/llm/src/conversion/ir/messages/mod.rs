//! Anthropic Messages: reads requests and writes replies.

pub mod request {
	use serde_json::{Map, Value};

	use crate::conversion::ir::{
		CacheMark, Document, DropReason, Drops, Effort, Message, OutputFormat, Part, Reasoning,
		ReasoningMode, Request, Role, Source, TextPart, Tool, ToolChoice, ToolResultPart,
	};
	use crate::types::messages::typed as wire;
	use crate::{AIError, InputFormat, json as json_util, types};

	/// Reads a Messages request. Fails only on input that cannot be translated at all.
	pub fn parse(req: &types::messages::Request, dropped: &mut Drops) -> Result<Request, AIError> {
		let typed = json_util::convert::<_, wire::Request>(req)
			.map_err(|err| AIError::RequestParsing(InputFormat::Messages, err))?;
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

	const fn read_effort(effort: wire::ThinkingEffort) -> Effort {
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
			wire::CacheControlEphemeral::Ephemeral { ttl } => CacheMark { ttl },
		}
	}

	fn read_message(index: usize, message: wire::Message, dropped: &mut Drops) -> Message {
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
				Part::Unread => dropped.record(DropReason::Unread, || {
					format!("messages[{index}].content[{part_index}]")
				}),
				Part::ToolResult { content, .. } => {
					for (result_index, result) in content.iter().enumerate() {
						if matches!(result, ToolResultPart::Unread) {
							dropped.record(DropReason::Unread, || {
								format!("messages[{index}].content[{part_index}].content[{result_index}]")
							});
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
					wire::ToolResultContentPart::SearchResult { .. }
					| wire::ToolResultContentPart::Unknown => ToolResultPart::Unread,
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

	fn source_object(
		source: Value,
		missing: &'static str,
	) -> Result<Map<String, Value>, &'static str> {
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
}

pub mod reply {
	use serde_json::{Value, json};

	use crate::conversion::ir::{Citation, Reply, ReplyPart, StopReason, Usage};
	use crate::types::messages::typed as wire;

	/// Every reply part has a Messages form, so nothing is dropped.
	pub fn render(reply: Reply) -> wire::MessagesResponse {
		let Reply {
			id,
			model,
			parts,
			stop_reason,
			usage,
			service_tier,
		} = reply;
		let stop_reason = render_stop_reason(stop_reason, &parts);
		wire::MessagesResponse {
			content: parts.into_iter().map(render_part).collect(),
			stop_reason: Some(stop_reason),
			usage: wire::Usage {
				service_tier,
				..usage.map_or_default(render_usage)
			},
			..wire::MessagesResponse::start(id, model)
		}
	}

	/// A completed turn with a tool call or refusal has its own stop reason.
	pub fn render_stop_reason(reason: StopReason, parts: &[ReplyPart]) -> wire::StopReason {
		match reason {
			StopReason::Completed
				if parts
					.iter()
					.any(|part| matches!(part, ReplyPart::ToolCall { .. })) =>
			{
				wire::StopReason::ToolUse
			},
			StopReason::Completed
				if parts
					.iter()
					.any(|part| matches!(part, ReplyPart::Refusal(_))) =>
			{
				wire::StopReason::Refusal
			},
			StopReason::Completed => wire::StopReason::EndTurn,
			StopReason::Length => wire::StopReason::MaxTokens,
			StopReason::ContentFilter => wire::StopReason::Refusal,
			StopReason::ContextWindow => wire::StopReason::ModelContextWindowExceeded,
		}
	}

	fn render_part(part: ReplyPart) -> wire::ContentBlock {
		match part {
			ReplyPart::Text { text, citations } => wire::ContentBlock::Text(wire::ContentTextBlock {
				text,
				citations: render_citations(citations),
				cache_control: None,
			}),
			ReplyPart::Refusal(text) => wire::ContentBlock::text(text),
			ReplyPart::ToolCall {
				id,
				name,
				arguments,
			} => wire::ContentBlock::ToolUse {
				id,
				name,
				input: arguments,
				cache_control: None,
			},
		}
	}

	fn render_citations(citations: Vec<Citation>) -> Option<Value> {
		if citations.is_empty() {
			return None;
		}
		// A URL citation has no cited text or replay index; leave them empty rather than invent them.
		let citations = citations
			.into_iter()
			.map(|Citation { url, title }| {
				json!({
					"type": "web_search_result_location",
					"url": url,
					"title": title,
					"cited_text": "",
					"encrypted_index": "",
				})
			})
			.collect();
		Some(Value::Array(citations))
	}

	/// Messages counts cache reads and writes apart from `input_tokens`.
	pub fn render_usage(usage: Usage) -> wire::Usage {
		let Usage {
			input_tokens,
			output_tokens,
			cache_read_tokens,
			cache_write_tokens,
		} = usage;
		let reported = |tokens: Option<u64>| tokens.filter(|tokens| *tokens > 0).map(token_count);
		wire::Usage {
			input_tokens: token_count(
				input_tokens
					.saturating_sub(cache_write_tokens.unwrap_or_default())
					.saturating_sub(cache_read_tokens.unwrap_or_default()),
			),
			output_tokens: token_count(output_tokens),
			cache_creation_input_tokens: reported(cache_write_tokens),
			cache_read_input_tokens: reported(cache_read_tokens),
			..Default::default()
		}
	}

	/// Messages counts tokens as `usize`; a count past its range saturates.
	fn token_count(tokens: u64) -> usize {
		usize::try_from(tokens).unwrap_or(usize::MAX)
	}
}
