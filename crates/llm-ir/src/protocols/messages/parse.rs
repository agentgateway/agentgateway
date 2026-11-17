//! Problems with the request itself are found here. What a provider can't take is
//! the renderer's to find, so a value one target ignores and another uses is kept,
//! not dropped.

use std::borrow::Cow;
use std::fmt;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, Visitor};

use crate::carrier::{self, StateScope};
use crate::ir::conversation::{
	Block, BlockId, CacheBreakpoint, Carrier, Content, Conversation, ImageFields, Message,
	MessagesThinking, MessagesThinkingDisplay, MessagesThinkingMode, Role, Settings, Text, Tool,
	ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::input::RequestInput;
use crate::json::{Fields, JsonRef, is_strict_object, text_of};
use crate::prompt::diagnostics::{Diagnostics, TokenLimit, Unread, UnreadConfig, UnreadImage};
use crate::prompt::passthrough::Reading;
use crate::protocols::{ContentRead, Read, read_fields};

pub struct Parsed<'a> {
	pub(crate) conversation: Conversation<'a>,
	pub(crate) diagnostics: Diagnostics,
	pub(crate) unread: Unread<'a>,
}

/// How a target words a value of the wrong shape.
#[derive(Clone, Copy)]
pub struct Shapes {
	pub(crate) system: &'static str,
	pub(crate) content: &'static str,
}

pub const MUST_BE_A_LIST: Shapes = Shapes {
	system: "must be a list",
	content: "must be a list",
};

/// `scope` is the target's: a carrier encoded for anywhere else stays invalid.
#[cfg(test)]
pub fn parse<'a>(body: &'a [u8], scope: StateScope<'_>) -> Parsed<'a> {
	parse_body(body, scope, MUST_BE_A_LIST, true)
}

/// `one_pass` changes only the cost: both ways must give the same result.
#[cfg(test)]
pub fn parse_body<'a>(
	body: &'a [u8],
	scope: StateScope<'_>,
	shapes: Shapes,
	one_pass: bool,
) -> Parsed<'a> {
	parse_input(body.into(), scope, shapes, one_pass)
}

pub fn parse_input<'a>(
	body: RequestInput<'a>,
	scope: StateScope<'_>,
	shapes: Shapes,
	one_pass: bool,
) -> Parsed<'a> {
	let mut parser = Parser {
		shapes,
		diagnostics: Diagnostics::default(),
		unread: Unread::default(),
		deferred: Vec::new(),
		loaded: Vec::new(),
		conversation: Conversation {
			settings: Settings::default(),
			system: None,
			tools: Vec::new(),
			messages: Vec::new(),
			cache_breakpoints: Vec::new(),
		},
	};
	if let Rest::Messages(messages) = parser.request(body, one_pass) {
		match messages {
			Some(raw) => parser.messages(raw, Opener { scope }),
			None => parser.diagnostics.problem("missing `messages`"),
		}
		parser.load_deferred_tools();
	}
	Parsed {
		conversation: parser.conversation,
		diagnostics: parser.diagnostics,
		unread: parser.unread,
	}
}

enum Rest<'a> {
	/// The body was refused before its fields were read.
	Refused,
	Messages(Option<Read<'a, Vec<MessageRead<'a>>>>),
}

/// Opens a replay carrier only for the upstream it names.
#[derive(Clone, Copy)]
struct Opener<'c> {
	scope: StateScope<'c>,
}

struct Parser<'a> {
	shapes: Shapes,
	diagnostics: Diagnostics,
	unread: Unread<'a>,
	/// Tools Claude Code marks as deferred, sent only once its `ToolSearch` loads them.
	deferred: Vec<String>,
	/// Tools a `tool_reference` in a search result has loaded.
	loaded: Vec<String>,
	conversation: Conversation<'a>,
}

impl<'a> Parser<'a> {
	fn request(&mut self, body: RequestInput<'a>, one_pass: bool) -> Rest<'a> {
		let (fields, mut messages) = match one_pass.then(|| body.read_raw::<BodyRead<'a>>()).flatten() {
			Some(read) => (read.other_fields, read.messages.map(Read::Done)),
			None => {
				let fields = body.fields();
				let Some(fields) = fields else {
					self.diagnostics.problem("body is not a JSON object");
					return Rest::Refused;
				};
				(fields, None)
			},
		};
		if self.diagnostics.reject_duplicates("request", &fields) {
			return Rest::Refused;
		}
		let mut has_model = false;
		let mut has_limit = false;
		for (name, raw) in &fields.0 {
			match name.as_ref() {
				"model" => {
					has_model = true;
					self.diagnostics.typed::<String>("model", raw);
				},
				"max_tokens" => match self.diagnostics.token_limit("max_tokens", raw) {
					TokenLimit::Positive(limit) => {
						self.conversation.settings.max_tokens = Some(limit);
						has_limit = true;
					},
					TokenLimit::Zero => has_limit = true,
					TokenLimit::Unreadable => {},
				},
				"stream" => self.conversation.settings.stream = self.diagnostics.typed("stream", raw),
				"temperature" => self.conversation.settings.temperature = Some(*raw),
				"top_p" => self.conversation.settings.top_p = Some(*raw),
				"top_k" => self.conversation.settings.top_k = Some(*raw),
				"stop_sequences" => {
					self.conversation.settings.stop_sequences = self
						.diagnostics
						.typed("stop_sequences", raw)
						.unwrap_or_default();
				},
				"system" => {
					self.conversation.system = text_blocks(
						&mut self.diagnostics,
						"system",
						*raw,
						self.shapes.system,
						&mut self.conversation.cache_breakpoints,
					)
					.map(|texts| texts.into_iter().map(Text::from).collect())
				},
				"messages" => messages = Some(Read::Raw(*raw)),
				"tools" => self.tools(*raw),
				"tool_choice" => self.tool_choice(*raw),
				"thinking" => {
					self.conversation.settings.messages_input.thinking = self.thinking_setting(*raw)
				},
				"output_config" => self.conversation.settings.messages_input.output_config = Some(*raw),
				"context_management" => {
					self.conversation.settings.messages_input.context_management = Some(*raw)
				},
				"metadata" => self.conversation.settings.messages_input.metadata = Some(*raw),
				other => self.diagnostics.problem(format!("field `{other}`")),
			}
		}
		if !has_model {
			self.diagnostics.problem("missing `model`");
		}
		if !has_limit {
			self.diagnostics.problem("missing `max_tokens`");
		}
		Rest::Messages(messages)
	}

	/// No other provider supports deferred tools, so a deferred tool is sent only once a
	/// search result has loaded it, as Anthropic would.
	fn load_deferred_tools(&mut self) {
		for name in &self.loaded {
			if !self.deferred.contains(name) {
				self
					.diagnostics
					.problem(format!("`tool_reference` names no deferred tool `{name}`"));
			}
		}
		let (deferred, loaded) = (&self.deferred, &self.loaded);
		self
			.conversation
			.tools
			.retain(|tool| !deferred.contains(&tool.name) || loaded.contains(&tool.name));
	}

	fn tool_choice(&mut self, raw: JsonRef<'a>) {
		let Some(fields) = self.diagnostics.object("tool_choice", raw) else {
			return;
		};
		let mut kind = None;
		let mut name = None;
		let mut parallel_off = false;
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => kind = self.diagnostics.typed::<String>("tool_choice.type", value),
				"name" => name = self.diagnostics.typed::<String>("tool_choice.name", value),
				"disable_parallel_tool_use" => {
					parallel_off = self
						.diagnostics
						.typed::<bool>("tool_choice.disable_parallel_tool_use", value)
						.unwrap_or(false);
				},
				other => self
					.diagnostics
					.problem(format!("field `tool_choice.{other}`")),
			}
		}
		let takes_name = kind.as_deref() == Some("tool");
		if name.is_some() && !takes_name {
			self.diagnostics.problem("field `tool_choice.name`");
		}
		if parallel_off && kind.as_deref() == Some("none") {
			self
				.diagnostics
				.problem("field `tool_choice.disable_parallel_tool_use`");
		}
		let choice = match kind.as_deref() {
			Some("auto") => Some(ToolChoice::Auto),
			Some("any") => Some(ToolChoice::Required),
			Some("none") => Some(ToolChoice::None),
			Some("tool") => match name {
				Some(name) => Some(ToolChoice::Tool(name)),
				None => {
					self.diagnostics.problem("`tool_choice` needs `name`");
					None
				},
			},
			Some(other) => {
				self.diagnostics.problem(format!("`tool_choice` `{other}`"));
				None
			},
			None => {
				self.diagnostics.problem("`tool_choice` needs `type`");
				None
			},
		};
		self.conversation.settings.tool_choice = choice;
		if parallel_off {
			self.conversation.settings.parallel_tool_calls = Some(false);
		}
	}

	fn thinking_setting(&mut self, raw: JsonRef<'a>) -> Option<MessagesThinking<'a>> {
		let fields = self.diagnostics.object("thinking", raw)?;
		let mut thinking = MessagesThinking {
			mode: None,
			display: None,
			budget_tokens: None,
		};
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => match self
					.diagnostics
					.typed::<String>("thinking.type", value)
					.as_deref()
				{
					Some("adaptive") => thinking.mode = Some(MessagesThinkingMode::Adaptive),
					Some("enabled") => thinking.mode = Some(MessagesThinkingMode::Enabled),
					Some("disabled") => thinking.mode = Some(MessagesThinkingMode::Disabled),
					None => {},
					Some(other) => self
						.diagnostics
						.problem(format!("`thinking.type` `{other}`")),
				},
				"display" => match self
					.diagnostics
					.typed::<String>("thinking.display", value)
					.as_deref()
				{
					Some("omitted") => thinking.display = Some(MessagesThinkingDisplay::Omitted),
					Some("summarized") => thinking.display = Some(MessagesThinkingDisplay::Summarized),
					// Claude Code's interactive progress view. It only changes what the
					// client shows, and refusing it costs a retry after every compaction.
					Some("updates") => self.diagnostics.drop_field("thinking.display updates"),
					None => {},
					Some(other) => self
						.diagnostics
						.problem(format!("`thinking.display` `{other}`")),
				},
				"budget_tokens" => thinking.budget_tokens = Some(*value),
				other => self
					.diagnostics
					.problem(format!("field `thinking.{other}`")),
			}
		}
		Some(thinking)
	}

	fn tools(&mut self, raw: JsonRef<'a>) {
		let Some(tools) = self.diagnostics.list("tools", raw) else {
			return;
		};
		for (index, tool) in tools.iter().enumerate() {
			let path = format!("tools[{index}]");
			let Some(fields) = self.diagnostics.object(&path, tool) else {
				continue;
			};
			let (mut name, mut description, mut schema, mut deferred) = (None, None, None, None);
			let mut strict = None;
			for (field, value) in &fields.0 {
				match field.as_ref() {
					"strict" => {
						strict = self
							.diagnostics
							.typed::<bool>(&format!("{path}.strict"), value)
					},
					"defer_loading" => {
						deferred = self
							.diagnostics
							.typed::<bool>(&format!("{path}.defer_loading"), value)
					},
					"name" => {
						name = self
							.diagnostics
							.typed::<String>(&format!("{path}.name"), value)
					},
					"description" => {
						description = self
							.diagnostics
							.typed::<String>(&format!("{path}.description"), value);
					},
					"input_schema" => schema = Some(*value),
					"cache_control" => self.diagnostics.drop_field("cache_control"),
					other => self.diagnostics.problem(format!("field `{path}.{other}`")),
				}
			}
			let (Some(name), Some(schema)) = (name, schema) else {
				self
					.diagnostics
					.problem(format!("`{path}` needs `name` and `input_schema`"));
				continue;
			};
			if !is_strict_object(schema) {
				self
					.diagnostics
					.problem(format!("`{path}.input_schema` must be a valid object"));
				continue;
			}
			if deferred == Some(true) {
				self.deferred.push(name.clone());
			}
			self.conversation.tools.push(Tool {
				path,
				name,
				namespace: None,
				description,
				schema: schema.into(),
				kind: ToolKind::Function { strict },
			});
		}
	}

	fn messages(&mut self, messages: Read<'a, Vec<MessageRead<'a>>>, opener: Opener<'_>) {
		let messages: Vec<_> = match messages {
			Read::Done(messages) => messages.into_iter().map(Read::Done).collect(),
			Read::Raw(raw) => {
				let Some(messages) = self.diagnostics.list("messages", raw) else {
					return;
				};
				messages.into_iter().map(Read::Raw).collect()
			},
		};
		if messages.is_empty() {
			self.diagnostics.problem("`messages` is empty");
		}
		for (index, message) in messages.into_iter().enumerate() {
			let path = format!("messages[{index}]");
			let (fields, mut content) = match message {
				Read::Done(message) => {
					if self
						.diagnostics
						.reject_duplicates(&path, &message.other_fields)
					{
						continue;
					}
					(message.other_fields, message.content.map(Read::Done))
				},
				Read::Raw(raw) => {
					let Some(fields) = self.diagnostics.object(&path, raw) else {
						continue;
					};
					(fields, None)
				},
			};
			let (mut role, mut output_config) = (None, None);
			for (name, value) in &fields.0 {
				match name.as_ref() {
					"role" => {
						role = self
							.diagnostics
							.typed::<String>(&format!("{path}.role"), value)
					},
					"content" => content = Some(Read::Raw(*value)),
					"output_config" => output_config = Some(*value),
					other => self.diagnostics.problem(format!("field `{path}.{other}`")),
				}
			}
			let (Some(role), Some(content)) = (role, content) else {
				self
					.diagnostics
					.problem(format!("`{path}` needs `role` and `content`"));
				self.unread_config(index, output_config, false);
				continue;
			};
			let role = match role.as_str() {
				"user" => Role::User,
				"assistant" => Role::Assistant,
				"system" => Role::System,
				other => {
					self.diagnostics.problem(format!("`{path}` role `{other}`"));
					self.unread_config(index, output_config, true);
					continue;
				},
			};
			let blocks = self.blocks(index, role, content, opener);
			self.conversation.messages.push(Message {
				index,
				role,
				blocks: link_reasoning(blocks),
				messages_output_config: output_config,
			});
		}
	}

	fn unread_config(&mut self, index: usize, raw: Option<JsonRef<'a>>, in_turn: bool) {
		if let Some(raw) = raw {
			self.unread.output_configs.push(UnreadConfig {
				index,
				raw,
				in_turn,
			});
		}
	}

	fn blocks(
		&mut self,
		message: usize,
		role: Role,
		content: Read<'a, ContentRead<'a, BlockRead<'a>>>,
		opener: Opener<'_>,
	) -> Vec<Block<'a>> {
		let path = format!("messages[{message}].content");
		let content = match content {
			Read::Done(content) => content,
			Read::Raw(raw) => match text_of(raw) {
				Some(text) => ContentRead::Text(text),
				None => {
					let shape = match role {
						Role::System => self.shapes.system,
						_ => self.shapes.content,
					};
					let Ok(blocks) = raw.list() else {
						self.diagnostics.problem(format!("`{path}` {shape}"));
						return Vec::new();
					};
					ContentRead::Blocks(blocks.into_iter().map(Read::Raw).collect())
				},
			},
		};
		let raw_blocks = match content {
			ContentRead::Text(text) => {
				return vec![Block {
					id: BlockId { message, block: 0 },
					content: Content::Text(text.into()),
				}];
			},
			ContentRead::Blocks(blocks) => blocks,
		};
		if role == Role::System {
			let mut texts = Vec::with_capacity(raw_blocks.len());
			for (at, raw) in raw_blocks.into_iter().enumerate() {
				let path = format!("{path}[{at}]");
				let Some((fields, _)) = self.block_fields(&path, raw) else {
					continue;
				};
				let Some(text) = block_text(&mut self.diagnostics, &path, &fields) else {
					continue;
				};
				if cache_control(&fields) {
					let id = BlockId {
						message,
						block: texts.len(),
					};
					self.mark(CacheBreakpoint::Block(id));
				}
				texts.push(text);
			}
			return texts
				.into_iter()
				.enumerate()
				.map(|(block, text)| Block {
					id: BlockId { message, block },
					content: Content::Text(text.into()),
				})
				.collect();
		}
		let mut blocks = Vec::with_capacity(raw_blocks.len());
		for (block, raw) in raw_blocks.into_iter().enumerate() {
			let id = BlockId { message, block };
			let path = id.path();
			let Some((fields, content)) = self.block_fields(&path, raw) else {
				continue;
			};
			let marked = cache_control(&fields);
			let content = match (role, fields.kind().as_deref()) {
				(_, Some("text")) => {
					let text = text_block(&mut self.diagnostics, &path, &fields);
					if text.is_none() && role == Role::User {
						self.unread.refused_text.push(message);
					}
					text.map(|text| Content::Text(text.into()))
				},
				(Role::User, Some("image")) => Some(Content::Image(ImageFields(fields))),
				(Role::User, Some("tool_result")) => self.tool_result(id, &fields, content),
				(Role::Assistant, Some("thinking")) => self.thinking_block(&path, &fields, opener),
				(Role::Assistant, Some("tool_use")) => self.tool_use(&path, &fields),
				(_, Some(other)) => {
					self
						.diagnostics
						.problem(format!("`{path}` block type `{other}`"));
					None
				},
				(_, None) => {
					self.diagnostics.problem(format!("`{path}` has no type"));
					None
				},
			};
			let can_mark = matches!(
				content,
				Some(Content::Text(_) | Content::Image(_) | Content::ToolResult { .. })
			);
			if can_mark && marked {
				self.mark(CacheBreakpoint::Block(id));
			}
			blocks.extend(content.map(|content| Block { id, content }));
		}
		blocks
	}

	fn mark(&mut self, at: CacheBreakpoint) {
		if !self.conversation.cache_breakpoints.contains(&at) {
			self.conversation.cache_breakpoints.push(at);
		}
	}

	/// A block's fields, and a tool result's content if it was read with them.
	fn block_fields(
		&mut self,
		path: &str,
		block: Read<'a, BlockRead<'a>>,
	) -> Option<(Fields<'a>, Option<ContentRead<'a>>)> {
		match block {
			Read::Done(block) => {
				let fields = self.object_fields(path, Read::Done(block.other_fields))?;
				Some((fields, block.content))
			},
			Read::Raw(raw) => Some((self.object_fields(path, Read::Raw(raw))?, None)),
		}
	}

	fn object_fields(&mut self, path: &str, object: Read<'a, Fields<'a>>) -> Option<Fields<'a>> {
		match object {
			Read::Done(fields) => (!self.diagnostics.reject_duplicates(path, &fields)).then_some(fields),
			Read::Raw(raw) => self.diagnostics.object(path, raw),
		}
	}

	/// `content` is the result's content when it was read with the block. Either way it's
	/// read after the other fields, so problems come in the same order.
	fn tool_result(
		&mut self,
		id: BlockId,
		fields: &Fields<'a>,
		content: Option<ContentRead<'a>>,
	) -> Option<Content<'a>> {
		let path = &id.path();
		let (mut call_id, mut failed) = (None, None);
		let mut content = content.map(Read::Done);
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"tool_use_id" => {
					call_id = self
						.diagnostics
						.typed::<String>(&format!("{path}.tool_use_id"), value)
				},
				"is_error" => {
					failed = self
						.diagnostics
						.typed(format_args!("{path}.is_error"), value)
				},
				"content" => content = Some(Read::Raw(*value)),
				// Read with the block's kind, in `blocks`.
				"cache_control" => {},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let mut output = ToolOutput::Text(Text::default());
		if let Some(content) = content {
			let (read, marked) = self.tool_output(&format!("{path}.content"), content);
			output = read;
			if marked {
				self.mark(CacheBreakpoint::Block(id));
			}
		}
		let Some(call_id) = call_id else {
			self
				.diagnostics
				.problem(format!("`{path}` has no `tool_use_id`"));
			if let ToolOutput::Mixed(parts) = output {
				for part in parts {
					if let ToolPart::Image { index, fields } = part {
						let path = format!("{path}.content[{index}]");
						self.unread.images.push(UnreadImage { path, fields });
					}
				}
			}
			return None;
		};
		Some(Content::ToolResult {
			call_id,
			failed,
			output,
		})
	}

	/// Also returns whether a part asked for a cache breakpoint.
	fn tool_output(
		&mut self,
		path: &str,
		content: Read<'a, ContentRead<'a>>,
	) -> (ToolOutput<'a>, bool) {
		let content = match content {
			Read::Done(content) => content,
			Read::Raw(raw) => match text_of(raw) {
				Some(text) => ContentRead::Text(text),
				None => {
					let Ok(blocks) = raw.list() else {
						self
							.diagnostics
							.problem(format!("`{path}` {}", self.shapes.content));
						return (ToolOutput::Text(Text::default()), false);
					};
					ContentRead::Blocks(blocks.into_iter().map(Read::Raw).collect())
				},
			},
		};
		let blocks = match content {
			ContentRead::Text(text) => return (ToolOutput::Text(text.into()), false),
			ContentRead::Blocks(blocks) => blocks,
		};
		let mut parts = Vec::with_capacity(blocks.len());
		let mut references = Vec::new();
		let mut marked = false;
		for (index, block) in blocks.into_iter().enumerate() {
			let path = format!("{path}[{index}]");
			let Some(fields) = self.object_fields(&path, block) else {
				continue;
			};
			marked |= cache_control(&fields);
			match fields.kind().as_deref() {
				Some("image") => parts.push(ToolPart::Image {
					index,
					fields: ImageFields(fields),
				}),
				Some("tool_reference") => references.extend(self.tool_reference(&path, &fields)),
				_ => parts.extend(
					block_text(&mut self.diagnostics, &path, &fields).map(|text| ToolPart::Text(text.into())),
				),
			}
		}
		if !references.is_empty() {
			let text = format!("Loaded tools: {}.", references.join(", "));
			parts.push(ToolPart::Text(text.into()));
			self.loaded.extend(references);
		}
		(joined(parts), marked)
	}

	/// What `ToolSearch` returns on Anthropic: the named deferred tool is loaded.
	fn tool_reference(&mut self, path: &str, fields: &Fields<'a>) -> Option<String> {
		let mut name = None;
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => {},
				"tool_name" => {
					name = self
						.diagnostics
						.typed::<String>(&format!("{path}.tool_name"), value)
				},
				"cache_control" => {},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		if name.is_none() {
			self
				.diagnostics
				.problem(format!("`{path}` has no `tool_name`"));
		}
		name
	}

	fn thinking_block(
		&mut self,
		path: &str,
		fields: &Fields<'a>,
		opener: Opener<'_>,
	) -> Option<Content<'a>> {
		let mut signature = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"thinking" => {
					self
						.diagnostics
						.typed::<String>(&format!("{path}.thinking"), value);
				},
				"signature" => {
					signature = self
						.diagnostics
						.typed::<String>(&format!("{path}.signature"), value);
				},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let Some(signature) = signature else {
			self
				.diagnostics
				.problem(format!("`{path}` has no signature"));
			return None;
		};
		let carrier = match carrier::open(
			opener.scope,
			carrier::CarrierPurpose::MessagesSignature,
			&signature,
		) {
			Ok(item) => Carrier::decoded(item),
			Err(error) => Carrier::invalid(error),
		};
		Some(Content::Reasoning {
			carrier,
			next: None,
		})
	}

	fn tool_use(&mut self, path: &str, fields: &Fields<'a>) -> Option<Content<'a>> {
		let (mut id, mut name, mut input) = (None, None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => {},
				"id" => {
					id = self
						.diagnostics
						.typed::<String>(&format!("{path}.id"), value)
				},
				"name" => {
					name = self
						.diagnostics
						.typed::<String>(&format!("{path}.name"), value)
				},
				"input" => input = Some(*value),
				"cache_control" => self.diagnostics.drop_field("cache_control"),
				"caller" => self.caller(&format!("{path}.caller"), *value),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let (Some(id), Some(name), Some(input)) = (id, name, input) else {
			self
				.diagnostics
				.problem(format!("`{path}` needs `id`, `name` and `input`"));
			return None;
		};
		if !is_strict_object(input) {
			self
				.diagnostics
				.problem(format!("`{path}.input` must be a valid object"));
			return None;
		}
		Some(Content::ToolCall {
			id,
			name,
			namespace: None,
			input: input.into(),
		})
	}

	/// Anthropic says who made a call. Only a direct call from the model has a
	/// meaning elsewhere; one made from its code execution has none.
	fn caller(&mut self, path: &str, raw: JsonRef<'a>) {
		let Some(fields) = self.diagnostics.object(path, raw) else {
			return;
		};
		if fields.kind().as_deref() != Some("direct") || fields.0.len() != 1 {
			self.diagnostics.problem(format!("`{path}` must be direct"));
		}
	}
}

/// Messages has no field for this, so the link comes from position, once, here.
fn link_reasoning(mut blocks: Vec<Block<'_>>) -> Vec<Block<'_>> {
	let mut next = None;
	for block in blocks.iter_mut().rev() {
		match &mut block.content {
			Content::Reasoning { next: link, .. } => *link = next,
			_ => next = Some(block.id),
		}
	}
	blocks
}

/// Text alone is assembled once, here. With an image, it stays in its place.
fn joined<'a>(mut parts: Vec<ToolPart<'a>>) -> ToolOutput<'a> {
	if parts.iter().any(|part| !matches!(part, ToolPart::Text(_))) {
		return ToolOutput::Mixed(parts);
	}
	if parts.len() == 1
		&& let Some(ToolPart::Text(text)) = parts.pop()
	{
		return ToolOutput::Text(text);
	}
	let texts: Vec<&str> = parts
		.iter()
		.filter_map(|part| match part {
			ToolPart::Text(text) => Some(text.as_str()),
			ToolPart::Image { .. } | ToolPart::DataImage(_) => None,
		})
		.collect();
	ToolOutput::Text(texts.join("\n").into())
}

fn text_blocks<'a>(
	diagnostics: &mut Diagnostics,
	path: &str,
	raw: JsonRef<'a>,
	wrong_shape: &str,
	marks: &mut Vec<CacheBreakpoint>,
) -> Option<Vec<Cow<'a, str>>> {
	if let Some(text) = text_of(raw) {
		return Some(vec![text]);
	}
	let Ok(blocks) = raw.list() else {
		diagnostics.problem(format!("`{path}` {wrong_shape}"));
		return None;
	};
	let mut texts = Vec::with_capacity(blocks.len());
	for (at, block) in blocks.iter().enumerate() {
		let path = format!("{path}[{at}]");
		let Some(fields) = diagnostics.object(&path, block) else {
			continue;
		};
		let Some(text) = block_text(diagnostics, &path, &fields) else {
			continue;
		};
		if cache_control(&fields) {
			marks.push(CacheBreakpoint::System(texts.len()));
		}
		texts.push(text);
	}
	Some(texts)
}

fn block_text<'a>(
	diagnostics: &mut Diagnostics,
	path: &str,
	fields: &Fields<'a>,
) -> Option<Cow<'a, str>> {
	match fields.kind().as_deref() {
		Some("text") => text_block(diagnostics, path, fields),
		Some(other) => {
			diagnostics.problem(format!("`{path}` block type `{other}`"));
			None
		},
		None => {
			diagnostics.problem(format!("`{path}` has no type"));
			None
		},
	}
}

fn text_block<'a>(
	diagnostics: &mut Diagnostics,
	path: &str,
	fields: &Fields<'a>,
) -> Option<Cow<'a, str>> {
	let mut text = None;
	for (name, raw) in &fields.0 {
		match name.as_ref() {
			"type" => {},
			"text" => text = diagnostics.text(format_args!("{path}.text"), raw),
			// Read by the caller, which knows where the text goes.
			"cache_control" => {},
			// The client echoes the sources we sent. Refusing would stop the session.
			"citations" => diagnostics.drop_field("citations"),
			other => diagnostics.problem(format!("field `{path}.{other}`")),
		}
	}
	if text.is_none() {
		diagnostics.problem(format!("`{path}` has no text"));
	}
	text
}

/// `null` asks for nothing, as an absent field does.
fn cache_control(fields: &Fields<'_>) -> bool {
	fields
		.0
		.iter()
		.any(|(name, raw)| name == "cache_control" && !raw.is_null())
}

pub(super) struct BodyRead<'a> {
	pub(super) other_fields: Fields<'a>,
	pub(super) messages: Option<Vec<MessageRead<'a>>>,
}

pub(super) struct MessageRead<'a> {
	pub(super) other_fields: Fields<'a>,
	pub(super) content: Option<ContentRead<'a, BlockRead<'a>>>,
}

/// A content block. A tool result's `content` is read in the same pass, so its text isn't
/// scanned once to capture it and again to read it.
pub(super) struct BlockRead<'a> {
	pub(super) other_fields: Fields<'a>,
	pub(super) content: Option<ContentRead<'a>>,
}

/// `None` means the body needs reading level by level, not that it's bad.
#[cfg(test)]
pub(super) fn read_body(text: &str) -> Option<BodyRead<'_>> {
	serde_json::from_str(text).ok()
}

impl<'de: 'a, 'a> Deserialize<'de> for BodyRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct BodyVisitor;

		impl<'de> Visitor<'de> for BodyVisitor {
			type Value = BodyRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a request object")
			}

			fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
				let (other_fields, messages) = read_fields(map, "messages")?;
				Ok(BodyRead {
					other_fields,
					messages,
				})
			}
		}

		deserializer.deserialize_map(BodyVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for MessageRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct MessageVisitor;

		impl<'de> Visitor<'de> for MessageVisitor {
			type Value = MessageRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a message object")
			}

			fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
				let (other_fields, content) = read_fields(map, "content")?;
				Ok(MessageRead {
					other_fields,
					content,
				})
			}
		}

		deserializer.deserialize_map(MessageVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for BlockRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct BlockVisitor;

		impl<'de> Visitor<'de> for BlockVisitor {
			type Value = BlockRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a content block")
			}

			fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
				let (other_fields, content) = read_fields(map, "content")?;
				// Elsewhere `content` is an unknown field, reported where it appears.
				if content.is_some() && other_fields.kind().as_deref() != Some("tool_result") {
					return Err(M::Error::custom("content outside a tool result"));
				}
				Ok(BlockRead {
					other_fields,
					content,
				})
			}
		}

		deserializer.deserialize_map(BlockVisitor)
	}
}

/// A Messages request the provider gets unconverted, read for the settings the caller records.
pub fn read<'a>(fields: &Fields<'a>, reading: &mut Reading<'a>) {
	for (name, raw) in &fields.0 {
		match name.as_ref() {
			"max_tokens" => reading.settings().max_tokens = Reading::limit(*raw),
			"temperature" => reading.settings().temperature = Some(*raw),
			"top_p" => reading.settings().top_p = Some(*raw),
			"top_k" => reading.settings().top_k = Some(*raw),
			"stream" => reading.stream("stream", *raw),
			"output_config" => reading.settings().messages_input.output_config = Some(*raw),
			_ => {},
		}
	}
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
