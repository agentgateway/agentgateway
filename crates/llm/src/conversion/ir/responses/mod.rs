//! OpenAI Responses: writes requests and reads buffered replies.

use agent_core::strng;

use crate::AIError;

pub mod request {
	use agent_core::strng;
	use rand::RngExt;
	use serde_json::{Map, Value, json};

	use super::unsupported;
	use crate::conversion::ir::{
		CacheMark, Document, DropReason, Drops, Effort, Message, OutputFormat, Part, Reasoning,
		ReasoningMode, Request, Role, Source, TextPart, Tool, ToolChoice, ToolResultPart,
	};
	use crate::types::responses::{RawInputItem, RequestInput, RequestReasoning};
	use crate::{AIError, conversion, types};

	/// Writes a Responses request. Refuses parts the destination would misread.
	pub fn render(req: Request, dropped: &mut Drops) -> Result<types::responses::Request, AIError> {
		let Request {
			model,
			max_output_tokens,
			instructions,
			messages,
			temperature,
			top_p,
			top_k,
			stop_sequences,
			stream,
			reasoning,
			output_format,
			metadata,
			tools,
			tool_choice,
			parallel_tool_calls,
			cache,
		} = req;
		let mut writer = RequestWriter {
			supports_cache: conversion::supports_prompt_cache_breakpoint(&model),
			dropped,
		};

		// Responses lacks these; drop them rather than fail the request (#2662).
		if !stop_sequences.is_empty() {
			writer.drop(|| "stop_sequences".to_string());
		}
		if top_k.is_some() {
			writer.drop(|| "top_k".to_string());
		}
		if cache.is_some() {
			writer.drop(|| "cache_control".to_string());
		}

		let (instructions, mut input) = writer.instructions(instructions);
		let reasoning = writer.reasoning(&model, reasoning);

		let mut rest = Map::new();
		if let Some(OutputFormat::JsonSchema { schema }) = output_format {
			rest.insert(
				"text".to_string(),
				json!({
					"format": {
						"type": "json_schema",
						"name": "structured_output",
						"schema": schema,
						"strict": false,
					}
				}),
			);
		}
		if !metadata.is_empty() {
			let metadata = metadata
				.into_iter()
				.map(|(key, value)| (key, Value::String(value)))
				.collect();
			rest.insert("metadata".to_string(), Value::Object(metadata));
		}
		let tools = writer.tools(tools);
		if !tools.is_empty() {
			rest.insert("tools".to_string(), Value::Array(tools));
		}
		if let Some(choice) = tool_choice {
			rest.insert("tool_choice".to_string(), write_tool_choice(choice));
		}
		if let Some(parallel_tool_calls) = parallel_tool_calls {
			rest.insert(
				"parallel_tool_calls".to_string(),
				Value::Bool(parallel_tool_calls),
			);
		}

		for (index, message) in messages.into_iter().enumerate() {
			writer.message(index, message, &mut input)?;
		}

		let max_output_tokens = u32::try_from(max_output_tokens).map_err(|_| {
			AIError::UnsupportedConversion(strng::literal!(
				"messages max_tokens exceeds responses max_output_tokens range"
			))
		})?;

		Ok(types::responses::Request {
			input: RequestInput::Items(input),
			model: Some(model),
			moderation: None,
			max_output_tokens: Some(max_output_tokens),
			temperature,
			top_p,
			stream: Some(stream),
			instructions,
			vendor_extensions: None,
			reasoning,
			rest: Value::Object(rest),
		})
	}

	/// State shared while writing one request.
	struct RequestWriter<'a> {
		supports_cache: bool,
		dropped: &'a mut Drops,
	}

	impl RequestWriter<'_> {
		fn drop(&mut self, path: impl FnOnce() -> String) {
			self.dropped.record(DropReason::Unsupported, path);
		}

		/// With a usable cache mark, blocks go in a system message so each keeps its breakpoint;
		/// otherwise they join `instructions`.
		fn instructions(&mut self, parts: Vec<TextPart>) -> (Option<String>, Vec<RawInputItem>) {
			let has_cache = parts.iter().any(|part| part.cache.is_some());
			if self.supports_cache && has_cache {
				let content = parts
					.into_iter()
					.enumerate()
					.map(|(index, part)| self.input_text(part, || format!("system[{index}]")))
					.collect();
				let mut input = Vec::new();
				push_input_message("system", content, &mut input);
				return (None, input);
			}
			let mut texts = Vec::with_capacity(parts.len());
			for (index, part) in parts.into_iter().enumerate() {
				if part.cache.is_some() {
					self.drop(|| format!("system[{index}].cache_control"));
				}
				texts.push(part.text);
			}
			let instructions = texts.join("\n");
			(
				(!instructions.is_empty()).then_some(instructions),
				Vec::new(),
			)
		}

		/// Responses takes only an effort: a budget uses its fallback, adaptive defaults to `high`.
		fn reasoning(&mut self, model: &str, reasoning: Reasoning) -> Option<RequestReasoning> {
			let Reasoning {
				mode,
				effort,
				display,
			} = reasoning;
			if !conversion::supports_reasoning_effort(model) {
				// Dropping the mode covers its display.
				if mode.is_some_and(|mode| mode != ReasoningMode::Off) {
					self.drop(|| "thinking".to_string());
				}
				if effort.is_some() {
					self.drop(|| "output_config.effort".to_string());
				}
				return None;
			}
			if display.is_some() {
				self.drop(|| "thinking.display".to_string());
			}
			let rendered = match mode {
				None => effort,
				Some(ReasoningMode::Off) => {
					if effort.is_some() {
						self.drop(|| "output_config.effort".to_string());
					}
					return None;
				},
				Some(ReasoningMode::Adaptive) => Some(effort.unwrap_or(Effort::High)),
				Some(ReasoningMode::Budget {
					fallback_effort, ..
				}) => {
					if effort.is_some() {
						self.drop(|| "thinking.budget_tokens".to_string());
					}
					Some(effort.unwrap_or(fallback_effort))
				},
			};
			rendered.map(|effort| RequestReasoning {
				effort: Some(effort_name(effort).to_string()),
				rest: Value::Object(Map::new()),
			})
		}

		fn tools(&mut self, tools: Vec<Tool>) -> Vec<Value> {
			let mut out = Vec::with_capacity(tools.len());
			for (index, tool) in tools.into_iter().enumerate() {
				match tool {
					Tool::Function {
						name,
						description,
						parameters,
						strict,
						cache,
					} => {
						if cache.is_some() {
							self.drop(|| format!("tools[{index}].cache_control"));
						}
						let mut value = Map::new();
						value.insert("type".to_string(), Value::String("function".to_string()));
						value.insert("name".to_string(), Value::String(name));
						if let Some(description) = description {
							value.insert("description".to_string(), Value::String(description));
						}
						value.insert("parameters".to_string(), parameters);
						// Responses defaults to strict schemas, making optional properties required.
						value.insert("strict".to_string(), Value::Bool(strict.unwrap_or(false)));
						out.push(Value::Object(value));
					},
					// Responses cannot run an Anthropic server tool; drop it rather than fail the request.
					Tool::Server { kind, .. } => {
						tracing::warn!(%kind, "unsupported server tool in responses conversion");
						self.drop(|| format!("tools[{index}]"));
					},
				}
			}
			out
		}

		// TODO(mk): drop paths assume a Messages client. A Responses reader should supply its own
		// source locations (`input[...]`).
		fn message(
			&mut self,
			index: usize,
			message: Message,
			out: &mut Vec<RawInputItem>,
		) -> Result<(), AIError> {
			match message.role {
				Role::User => self.user_message(index, message.parts, out),
				Role::Assistant => self.assistant_message(index, message.parts, out),
				Role::System => {
					self.system_message(index, message.parts, out);
					Ok(())
				},
			}
		}

		fn user_message(
			&mut self,
			index: usize,
			parts: Vec<Part>,
			out: &mut Vec<RawInputItem>,
		) -> Result<(), AIError> {
			let mut content = Vec::new();
			for (part_index, part) in parts.into_iter().enumerate() {
				let path = || format!("messages[{index}].content[{part_index}]");
				match part {
					Part::Text(text) => content.push(self.input_text(text, path)),
					Part::Image { source, cache } => {
						let mut value = image(source)?;
						self.mark(&mut value, cache, path);
						content.push(value);
					},
					Part::Document(document) => content.push(self.document(document, path)?),
					Part::ToolResult {
						call_id,
						content: result,
						is_error,
						cache,
					} => {
						push_input_message("user", std::mem::take(&mut content), out);
						let status = if is_error.unwrap_or_default() {
							"incomplete"
						} else {
							"completed"
						};
						let output = self.tool_result_output(result, cache, path)?;
						out.push(RawInputItem::from_value(json!({
							"type": "function_call_output",
							"call_id": call_id,
							"output": output,
							"status": status,
						})));
					},
					Part::Reasoning { .. } | Part::RedactedReasoning { .. } | Part::ToolCall { .. } => {
						self.drop(path);
					},
					Part::Unread => {},
				}
			}
			push_input_message("user", content, out);
			Ok(())
		}

		fn assistant_message(
			&mut self,
			index: usize,
			parts: Vec<Part>,
			out: &mut Vec<RawInputItem>,
		) -> Result<(), AIError> {
			let mut content = Vec::new();
			for (part_index, part) in parts.into_iter().enumerate() {
				let path = || format!("messages[{index}].content[{part_index}]");
				match part {
					Part::Text(TextPart {
						text,
						citations,
						cache,
					}) => {
						if citations.is_some() {
							self.drop(|| format!("{}.citations", path()));
						}
						let mut value = json!({
							"type": "output_text",
							"text": text,
							"annotations": [],
						});
						self.mark(&mut value, cache, path);
						content.push(value);
					},
					Part::ToolCall {
						id,
						name,
						arguments,
						cache,
					} => {
						if cache.is_some() {
							self.drop(|| format!("{}.cache_control", path()));
						}
						push_output_message(std::mem::take(&mut content), out);
						let arguments = serde_json::to_string(&arguments).map_err(AIError::RequestMarshal)?;
						// Messages preserves the call ID, not the optional Responses item ID.
						out.push(RawInputItem::from_value(json!({
							"type": "function_call",
							"call_id": id,
							"name": name,
							"arguments": arguments,
							"status": "completed",
						})));
					},
					// TODO: Round-trip summaries and OpenAI encrypted reasoning via signature/data, telling
					// them apart from foreign signatures.
					Part::Image { .. }
					| Part::Document(_)
					| Part::Reasoning { .. }
					| Part::RedactedReasoning { .. }
					| Part::ToolResult { .. } => self.drop(path),
					Part::Unread => {},
				}
			}
			push_output_message(content, out);
			Ok(())
		}

		fn system_message(&mut self, index: usize, parts: Vec<Part>, out: &mut Vec<RawInputItem>) {
			let mut content = Vec::new();
			for (part_index, part) in parts.into_iter().enumerate() {
				let path = || format!("messages[{index}].content[{part_index}]");
				match part {
					Part::Text(text) => content.push(self.input_text(text, path)),
					Part::Unread => {},
					_ => self.drop(path),
				}
			}
			push_input_message("system", content, out);
		}

		fn input_text(&mut self, part: TextPart, path: impl Fn() -> String) -> Value {
			let TextPart {
				text,
				citations,
				cache,
			} = part;
			if citations.is_some() {
				self.drop(|| format!("{}.citations", path()));
			}
			let mut value = text_value(text);
			self.mark(&mut value, cache, path);
			value
		}

		/// An unsendable document is an error, not a drop: answering without it is worse than refusing.
		fn document(
			&mut self,
			document: Document,
			path: impl Fn() -> String,
		) -> Result<Value, AIError> {
			let Document {
				source,
				title,
				context,
				citations,
				cache,
			} = document;
			if context.is_some() {
				self.drop(|| format!("{}.context", path()));
			}
			if citations.is_some() {
				self.drop(|| format!("{}.citations", path()));
			}
			// Only inline data carries a filename.
			if title.is_some() && !matches!(source, Source::Base64 { .. }) {
				self.drop(|| format!("{}.title", path()));
			}
			let mut value = match source {
				Source::Base64 { media_type, data } => {
					// OpenAI requires a filename alongside inline file data.
					let filename = match (title, media_type.as_str()) {
						(Some(title), _) => title,
						(None, "application/pdf") => "document.pdf".to_string(),
						(None, _) => "document".to_string(),
					};
					json!({
						"type": "input_file",
						"filename": filename,
						"file_data": format!("data:{media_type};base64,{data}"),
					})
				},
				Source::Url(url) => json!({
					"type": "input_file",
					"file_url": url,
				}),
				Source::File(file_id) => json!({
					"type": "input_file",
					"file_id": file_id,
				}),
				Source::Text(data) => json!({
					"type": "input_text",
					"text": data,
				}),
				Source::Other(_) => {
					return unsupported("messages document source cannot be represented by responses");
				},
				Source::Incomplete(reason) => return unsupported(reason),
			};
			self.mark(&mut value, cache, path);
			Ok(value)
		}

		/// A plain string when the result is only unmarked text, otherwise an array of parts.
		fn tool_result_output(
			&mut self,
			content: Vec<ToolResultPart>,
			cache: Option<CacheMark>,
			path: impl Fn() -> String,
		) -> Result<Value, AIError> {
			let mark_last = self.keep_cache(cache, &path);
			let requires_array = mark_last
				|| content.iter().any(|part| match part {
					ToolResultPart::Text(TextPart { cache, .. })
					| ToolResultPart::ToolReference { cache, .. } => self.supports_cache && cache.is_some(),
					ToolResultPart::Image { .. } | ToolResultPart::Document(_) => true,
					ToolResultPart::Unread => false,
				});
			if !requires_array {
				return Ok(Value::String(self.tool_result_text(content, &path)));
			}
			let mut values = Vec::with_capacity(content.len());
			for (index, part) in content.into_iter().enumerate() {
				let part_path = || format!("{}.content[{index}]", path());
				match part {
					ToolResultPart::Text(text) => values.push(self.input_text(text, part_path)),
					ToolResultPart::ToolReference { tool_name, cache } => {
						let mut value = text_value(tool_name);
						self.mark(&mut value, cache, part_path);
						values.push(value);
					},
					ToolResultPart::Image { source, cache } => {
						let mut value = image(source)?;
						self.mark(&mut value, cache, part_path);
						values.push(value);
					},
					ToolResultPart::Document(document) => values.push(self.document(document, part_path)?),
					ToolResultPart::Unread => {},
				}
			}
			if mark_last {
				if values.is_empty() {
					values.push(text_value(String::new()));
				}
				insert_breakpoint(values.last_mut().expect("values is not empty"));
			}
			Ok(Value::Array(values))
		}

		/// The text parts joined by newlines, moved rather than copied when there is only one.
		fn tool_result_text(
			&mut self,
			content: Vec<ToolResultPart>,
			path: impl Fn() -> String,
		) -> String {
			let mut texts = Vec::with_capacity(content.len());
			for (index, part) in content.into_iter().enumerate() {
				let part_path = || format!("{}.content[{index}]", path());
				let (text, cache) = match part {
					ToolResultPart::Text(TextPart {
						text,
						citations,
						cache,
					}) => {
						if citations.is_some() {
							self.drop(|| format!("{}.citations", part_path()));
						}
						(text, cache)
					},
					ToolResultPart::ToolReference { tool_name, cache } => (tool_name, cache),
					ToolResultPart::Unread => continue,
					ToolResultPart::Image { .. } | ToolResultPart::Document(_) => {
						unreachable!("media needs the array form")
					},
				};
				let kept = self.keep_cache(cache, part_path);
				assert!(!kept, "a kept cache mark needs the array form");
				texts.push(text);
			}
			if texts.len() == 1 {
				return texts.swap_remove(0);
			}
			texts.join("\n")
		}

		fn mark(&mut self, part: &mut Value, cache: Option<CacheMark>, path: impl FnOnce() -> String) {
			if self.keep_cache(cache, path) {
				insert_breakpoint(part);
			}
		}

		/// Whether a cache mark becomes a breakpoint. Records what the destination cannot keep.
		fn keep_cache(&mut self, cache: Option<CacheMark>, path: impl FnOnce() -> String) -> bool {
			let Some(CacheMark { ttl }) = cache else {
				return false;
			};
			if !self.supports_cache {
				self.drop(|| format!("{}.cache_control", path()));
				return false;
			}
			if ttl.is_some() {
				self.drop(|| format!("{}.cache_control.ttl", path()));
			}
			true
		}
	}

	fn write_tool_choice(choice: ToolChoice) -> Value {
		match choice {
			ToolChoice::Auto => Value::String("auto".to_string()),
			ToolChoice::Required => Value::String("required".to_string()),
			ToolChoice::None => Value::String("none".to_string()),
			ToolChoice::Tool(name) => json!({
				"type": "function",
				"name": name,
			}),
		}
	}

	const fn effort_name(effort: Effort) -> &'static str {
		match effort {
			Effort::Low => "low",
			Effort::Medium => "medium",
			Effort::High => "high",
			Effort::Xhigh => "xhigh",
			Effort::Max => "max",
		}
	}

	fn image(source: Source) -> Result<Value, AIError> {
		match source {
			Source::Base64 { media_type, data } => Ok(json!({
				"type": "input_image",
				"image_url": format!("data:{media_type};base64,{data}"),
				"detail": "auto",
			})),
			Source::Url(url) => Ok(json!({
				"type": "input_image",
				"image_url": url,
				"detail": "auto",
			})),
			Source::File(file_id) => Ok(json!({
				"type": "input_image",
				"file_id": file_id,
				"detail": "auto",
			})),
			Source::Text(_) | Source::Other(_) => {
				unsupported("messages image source cannot be represented by responses")
			},
			Source::Incomplete(reason) => unsupported(reason),
		}
	}

	/// An `input_text` part that takes ownership of `text`; `json!` would copy it.
	fn text_value(text: String) -> Value {
		let mut part = Map::new();
		part.insert("type".to_string(), Value::String("input_text".to_string()));
		part.insert("text".to_string(), Value::String(text));
		Value::Object(part)
	}

	fn insert_breakpoint(part: &mut Value) {
		part
			.as_object_mut()
			.expect("rendered parts are JSON objects")
			.entry("prompt_cache_breakpoint")
			.or_insert_with(|| json!({"mode": "explicit"}));
	}

	fn push_input_message(role: &'static str, content: Vec<Value>, out: &mut Vec<RawInputItem>) {
		if content.is_empty() {
			return;
		}
		let mut item = Map::new();
		item.insert("type".to_string(), Value::String("message".to_string()));
		item.insert("role".to_string(), Value::String(role.to_string()));
		item.insert("content".to_string(), Value::Array(content));
		out.push(RawInputItem::from_value(Value::Object(item)));
	}

	fn push_output_message(content: Vec<Value>, out: &mut Vec<RawInputItem>) {
		if content.is_empty() {
			return;
		}
		let mut item = Map::new();
		item.insert("type".to_string(), Value::String("message".to_string()));
		let id = format!("msg_{:016x}", rand::rng().random::<u64>());
		item.insert("id".to_string(), Value::String(id));
		item.insert("role".to_string(), Value::String("assistant".to_string()));
		item.insert("content".to_string(), Value::Array(content));
		item.insert("status".to_string(), Value::String("completed".to_string()));
		out.push(RawInputItem::from_value(Value::Object(item)));
	}
}

pub mod reply {
	use agent_core::strng;
	use serde::Deserialize;

	use super::unsupported;
	use crate::conversion::ir::{Citation, DropReason, Drops, Reply, ReplyPart, StopReason, Usage};
	use crate::types::responses::typed as wire;
	use crate::{AIError, conversion, types};

	/// Refuses a failed or unfinished reply rather than pass it on as finished.
	pub fn parse(reply: wire::Response, dropped: &mut Drops) -> Result<Reply, AIError> {
		if reply.error.is_some() {
			return unsupported("responses failed body cannot be represented by messages");
		}
		let stop_reason = match reply.status {
			wire::Status::Completed => StopReason::Completed,
			wire::Status::Incomplete => match &reply.incomplete_details {
				None => StopReason::Length,
				Some(details) => parse_incomplete_reason(&details.reason)?,
			},
			wire::Status::Failed => {
				return unsupported("responses failed body cannot be represented by messages");
			},
			wire::Status::InProgress | wire::Status::Queued | wire::Status::Cancelled => {
				return unsupported("non-final responses body cannot be represented by messages");
			},
		};

		let mut parts = Vec::new();
		for (index, item) in reply.output.into_iter().enumerate() {
			match item {
				wire::OutputItem::Message(message) => {
					if matches!(message.status, wire::OutputStatus::InProgress) {
						return unsupported("in-progress responses message cannot be represented by messages");
					}
					for (part_index, part) in message.content.into_iter().enumerate() {
						match part {
							wire::OutputMessageContent::OutputText(text) => {
								let path = || format!("output[{index}].content[{part_index}]");
								parts.push(ReplyPart::Text {
									citations: parse_citations(text.annotations, path, dropped),
									text: text.text,
								});
							},
							wire::OutputMessageContent::Refusal(refusal) => {
								parts.push(ReplyPart::Refusal(refusal.refusal));
							},
						}
					}
				},
				wire::OutputItem::FunctionCall(call) => {
					if call.namespace.is_some() || call.caller.is_some() {
						return unsupported(
							"responses namespaced/caller function calls cannot be represented by messages",
						);
					}
					if matches!(call.status, Some(wire::OutputStatus::InProgress)) {
						return unsupported(
							"in-progress responses function call cannot be represented by messages",
						);
					}
					parts.push(ReplyPart::ToolCall {
						arguments: conversion::tool_arguments_to_input(&call.arguments),
						id: call.call_id,
						name: call.name,
					});
				},
				// TODO: Carry reasoning summaries and encrypted reasoning, matching the request side.
				wire::OutputItem::Reasoning(_) => {
					dropped.record(DropReason::Unread, || format!("output[{index}]"));
				},
				_ => return unsupported("responses output item cannot be represented by messages"),
			}
		}

		Ok(Reply {
			id: reply.id,
			model: reply.model,
			parts,
			stop_reason,
			usage: reply.usage.as_ref().map(parse_usage),
			service_tier: reply
				.service_tier
				.as_ref()
				.and_then(types::serialize_str)
				.map(|tier| tier.to_string()),
		})
	}

	/// Providers word incomplete reasons differently, so these match loosely.
	pub fn parse_incomplete_reason(reason: &str) -> Result<StopReason, AIError> {
		let reason = reason.to_ascii_lowercase();
		if reason == "content_filter" {
			Ok(StopReason::ContentFilter)
		} else if reason.contains("max") || reason.contains("token") {
			Ok(StopReason::Length)
		} else if reason.contains("context") {
			Ok(StopReason::ContextWindow)
		} else {
			Err(AIError::UnsupportedConversion(strng::format!(
				"responses incomplete reason {reason:?} cannot be represented by messages"
			)))
		}
	}

	fn parse_citations(
		annotations: Vec<wire::Annotation>,
		path: impl Fn() -> String,
		dropped: &mut Drops,
	) -> Vec<Citation> {
		let mut citations = Vec::new();
		for (index, annotation) in annotations.into_iter().enumerate() {
			match annotation {
				wire::Annotation::UrlCitation(citation) => {
					// async-openai keeps these fields private, so read them back through JSON.
					#[derive(Deserialize)]
					struct Fields {
						url: String,
						title: String,
					}
					let Fields { url, title } = serde_json::to_value(citation)
						.and_then(serde_json::from_value)
						.expect("a URL citation round-trips through JSON");
					citations.push(Citation { url, title });
				},
				_ => dropped.record(DropReason::Unread, || {
					format!("{}.annotations[{index}]", path())
				}),
			}
		}
		citations
	}

	/// Token usage, with cached tokens still counted in `input_tokens`.
	pub fn parse_usage(usage: &wire::ResponseUsage) -> Usage {
		Usage {
			input_tokens: u64::from(usage.input_tokens),
			output_tokens: u64::from(usage.output_tokens),
			cache_read_tokens: Some(u64::from(usage.input_tokens_details.cached_tokens)),
			cache_write_tokens: usage
				.input_tokens_details
				.cache_write_tokens
				.and_then(|tokens| u64::try_from(tokens).ok()),
		}
	}
}

fn unsupported<T>(reason: &'static str) -> Result<T, AIError> {
	Err(AIError::UnsupportedConversion(strng::new(reason)))
}
