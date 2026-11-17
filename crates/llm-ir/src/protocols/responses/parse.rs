//! Fields only a provider of this format could use are dropped here, with a record.
//!
//! A one-pass read of a request body. The parser handles what this can't.

use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU64;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

use crate::carrier::{self, StateScope};
use crate::ir::conversation::{
	Block, BlockId, Carrier, Content, Conversation, DataImage, Effort, Message, OutputFormat, Role,
	Settings, Tool, ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::input::RequestInput;
use crate::json::{Fields, Json, JsonRef, Name, is_strict_object, push_string, text_of};
use crate::prompt::diagnostics::{Diagnostics, TokenLimit};
use crate::prompt::passthrough::{Reading, word};
use crate::protocols::{ContentRead, Read, is_image, read_fields};

/// What Codex's backend wants back on a later turn. It means nothing to another provider.
const PASSTHROUGH: &str = "internal_chat_message_metadata_passthrough";
/// Codex's search tool has no name of its own, so it goes upstream under this one.
const TOOL_SEARCH: &str = "tool_search";
const CUSTOM_SCHEMA: &str =
	r#"{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}"#;

pub struct Parsed<'a> {
	pub(crate) conversation: Conversation<'a>,
	pub(crate) diagnostics: Diagnostics,
}

/// `scope` is the target's: a carrier encoded for anywhere else stays invalid.
/// `default_max_tokens` applies when the client sends no limit, and `needs_limit`
/// says the target can't do without one. `one_pass` changes only the cost: both
/// ways must give the same result.
pub fn parse<'a>(
	body: RequestInput<'a>,
	scope: StateScope<'_>,
	default_max_tokens: Option<NonZeroU64>,
	needs_limit: bool,
	one_pass: bool,
) -> Parsed<'a> {
	let mut parser = Parser {
		scope,
		default_max_tokens,
		needs_limit,
		diagnostics: Diagnostics::default(),
		open_calls: Vec::new(),
		has_turn: false,
		conversation: Conversation {
			settings: Settings::default(),
			system: None,
			tools: Vec::new(),
			messages: Vec::new(),
			cache_breakpoints: Vec::new(),
		},
	};
	parser.request(body, one_pass);
	Parsed {
		conversation: parser.conversation,
		diagnostics: parser.diagnostics,
	}
}

struct Parser<'a, 'c> {
	scope: StateScope<'c>,
	default_max_tokens: Option<NonZeroU64>,
	needs_limit: bool,
	diagnostics: Diagnostics,
	open_calls: Vec<String>,
	/// A system message after this point stays where it is.
	has_turn: bool,
	conversation: Conversation<'a>,
}

impl<'a> Parser<'a, '_> {
	fn request(&mut self, body: RequestInput<'a>, one_pass: bool) {
		let (fields, mut input) = match one_pass
			.then(|| body.read_raw::<ResponsesRead<'a>>())
			.flatten()
		{
			Some(read) => (read.fields, read.input.map(Read::Done)),
			None => {
				let fields = body.fields();
				let Some(fields) = fields else {
					self.diagnostics.problem("body is not a JSON object");
					return;
				};
				(fields, None)
			},
		};
		if self.diagnostics.reject_duplicates("request", &fields) {
			return;
		}
		let (mut has_model, mut has_limit) = (false, false);
		for (name, raw) in &fields.0 {
			match name.as_ref() {
				"model" => {
					has_model = true;
					self.diagnostics.typed::<String>("model", raw);
				},
				"max_output_tokens" => match self.diagnostics.token_limit(name, raw) {
					TokenLimit::Positive(limit) => {
						self.conversation.settings.max_tokens = Some(limit);
						has_limit = true;
					},
					TokenLimit::Zero => has_limit = true,
					TokenLimit::Unreadable => {},
				},
				"stream" => self.conversation.settings.stream = self.diagnostics.typed(name, raw),
				"temperature" => self.conversation.settings.temperature = Some(*raw),
				"top_p" => self.conversation.settings.top_p = Some(*raw),
				"instructions" => {
					if let Some(text) = self.diagnostics.text(name, raw) {
						self.push_system(text);
					}
				},
				"input" => input = Some(Read::Raw(*raw)),
				"tools" => self.tools(name, *raw),
				"tool_choice" => self.conversation.settings.tool_choice = self.tool_choice(*raw),
				"parallel_tool_calls" => {
					self.conversation.settings.parallel_tool_calls = self.diagnostics.typed(name, raw);
				},
				"reasoning" => self.reasoning(*raw),
				"store" => match self.diagnostics.typed::<bool>(name, raw) {
					Some(false) => self.diagnostics.drop_field("store"),
					Some(true) => self.diagnostics.problem("`store: true`"),
					None => {},
				},
				"include" => self.include(*raw),
				"prompt_cache_key" => self.diagnostics.drop_field("prompt_cache_key"),
				"text" => self.text_setting(*raw),
				"client_metadata" => self.diagnostics.drop_field("client_metadata"),
				"access_programs" => self.diagnostics.drop_field("access_programs"),
				other => self.diagnostics.problem(format!("field `{other}`")),
			}
		}
		if !has_model {
			self.diagnostics.problem("missing `model`");
		}
		match input {
			Some(raw) => self.input(raw),
			None => self.diagnostics.problem("missing `input`"),
		}
		let settings = &mut self.conversation.settings;
		settings.max_tokens = settings.max_tokens.or(self.default_max_tokens);
		if self.needs_limit && settings.max_tokens.is_none() && !has_limit {
			self
				.diagnostics
				.problem("no `max_output_tokens`, and no default for this model");
		}
		if !self.open_calls.is_empty() {
			let calls = self.open_calls.join(", ");
			self
				.diagnostics
				.problem(format!("tool calls with no result: {calls}"));
		}
	}

	fn push_system(&mut self, text: Cow<'a, str>) {
		self
			.conversation
			.system
			.get_or_insert_with(Vec::new)
			.push(text.into());
	}

	fn reasoning(&mut self, raw: JsonRef<'_>) {
		let Some(fields) = self.diagnostics.object("reasoning", raw) else {
			return;
		};
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"effort" => self.effort("reasoning.effort", *value),
				"context" => {
					let context = self.diagnostics.typed::<String>("reasoning.context", value);
					match context.as_deref() {
						Some("all_turns") => self.conversation.settings.keep_earlier_reasoning = true,
						Some(other) => self
							.diagnostics
							.problem(format!("`reasoning.context` `{other}`")),
						None => {},
					}
				},
				"summary" => {
					let summary = self
						.diagnostics
						.typed::<Option<String>>("reasoning.summary", value);
					self.conversation.settings.show_reasoning_summary =
						matches!(summary, Some(Some(kind)) if kind != "none");
				},
				other => self
					.diagnostics
					.problem(format!("field `reasoning.{other}`")),
			}
		}
	}

	fn effort(&mut self, path: &str, raw: JsonRef<'_>) {
		let effort = self.diagnostics.typed::<String>(path, raw);
		let effort = match effort.as_deref() {
			Some("none") => Effort::Off,
			Some("minimal") => Effort::Minimal,
			Some("low") => Effort::Low,
			Some("medium") => Effort::Medium,
			Some("high") => Effort::High,
			Some("xhigh") => Effort::XHigh,
			Some(other) => {
				self.diagnostics.problem(format!("`{path}` `{other}`"));
				return;
			},
			None => return,
		};
		self.conversation.settings.effort = Some(effort);
	}

	fn include(&mut self, raw: JsonRef<'_>) {
		match self.diagnostics.typed::<Vec<String>>("include", raw) {
			Some(values)
				if values
					.iter()
					.all(|value| value == "reasoning.encrypted_content") =>
			{
				self.diagnostics.drop_field("include");
			},
			Some(_) => self.diagnostics.problem("`include` value"),
			None => {},
		}
	}

	fn text_setting(&mut self, raw: JsonRef<'a>) {
		let Some(fields) = self.diagnostics.object("text", raw) else {
			return;
		};
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"verbosity" => self.diagnostics.drop_field("text.verbosity"),
				"format" => self.text_format(*value),
				other => self.diagnostics.problem(format!("field `text.{other}`")),
			}
		}
	}

	fn text_format(&mut self, raw: JsonRef<'a>) {
		let Some(fields) = self.diagnostics.object("text.format", raw) else {
			return;
		};
		let mut schema = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"schema" => schema = Some(*value),
				"name" | "description" | "strict" => {
					self.diagnostics.drop_field("text.format name and strict");
				},
				other => self
					.diagnostics
					.problem(format!("field `text.format.{other}`")),
			}
		}
		self.conversation.settings.output_format = match (fields.kind().as_deref(), schema) {
			(Some("text"), None) => None,
			(Some("json_object"), None) => Some(OutputFormat::Json),
			(Some("json_schema"), Some(schema)) => Some(OutputFormat::JsonSchema(schema.into())),
			(Some("json_schema"), None) => {
				self.diagnostics.problem("`text.format` has no `schema`");
				None
			},
			(kind, _) => {
				self
					.diagnostics
					.problem(format!("`text.format` type `{}`", kind.unwrap_or_default()));
				None
			},
		};
	}

	fn tool_choice(&mut self, raw: JsonRef<'_>) -> Option<ToolChoice> {
		if raw.is_object() {
			return self.named_tool_choice(raw);
		}
		let choice = self.diagnostics.typed::<String>("tool_choice", raw);
		match choice.as_deref() {
			Some("auto") => Some(ToolChoice::Auto),
			Some("required") => Some(ToolChoice::Required),
			Some("none") => Some(ToolChoice::None),
			Some(other) => {
				self.diagnostics.problem(format!("`tool_choice` `{other}`"));
				None
			},
			None => None,
		}
	}

	fn named_tool_choice(&mut self, raw: JsonRef<'_>) -> Option<ToolChoice> {
		let fields = self.diagnostics.object("tool_choice", raw)?;
		let mut name = None;
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => {},
				"name" => name = self.diagnostics.typed::<String>("tool_choice.name", value),
				other => self
					.diagnostics
					.problem(format!("field `tool_choice.{other}`")),
			}
		}
		let kind = fields.kind();
		if kind.as_deref() != Some("function") {
			let kind = kind.as_deref().unwrap_or("none");
			self
				.diagnostics
				.problem(format!("`tool_choice.type` `{kind}`"));
			return None;
		}
		if name.is_none() {
			self.diagnostics.problem("`tool_choice` needs `name`");
		}
		name.map(ToolChoice::Tool)
	}

	fn tools(&mut self, path: &str, raw: JsonRef<'a>) {
		let Some(tools) = self.diagnostics.list(path, raw) else {
			return;
		};
		for (at, tool) in tools.iter().enumerate() {
			self.tool(&format!("{path}[{at}]"), *tool, None, None);
		}
	}

	/// A namespace's description goes before each of its tools' own, as there is no
	/// other field for it.
	fn tool(
		&mut self,
		path: &str,
		raw: JsonRef<'a>,
		namespace: Option<&str>,
		namespace_description: Option<&str>,
	) {
		let Some(fields) = self.diagnostics.object(path, raw) else {
			return;
		};
		let kind = fields.kind();
		if kind.as_deref() == Some("web_search") {
			self.diagnostics.drop_field("web_search tool");
			return;
		}
		let (mut name, mut description, mut schema, mut strict, mut format, mut nested) =
			(None, None, None, None, None, None);
		let mut execution = None;
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => {},
				"execution" => {
					execution = self
						.diagnostics
						.typed::<String>(&format!("{path}.execution"), value)
				},
				// Only Codex's search reads it. Every route sends each tool it has.
				"defer_loading" => {
					self
						.diagnostics
						.typed::<bool>(&format!("{path}.defer_loading"), value);
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
				"parameters" => schema = Some(*value),
				"strict" => {
					strict = self
						.diagnostics
						.typed::<bool>(&format!("{path}.strict"), value)
				},
				"format" => format = Some(*value),
				"tools" => nested = Some(*value),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let search = kind.as_deref() == Some(TOOL_SEARCH);
		if search && execution.as_deref() != Some("client") {
			self
				.diagnostics
				.problem(format!("`{path}` must run on the client"));
			return;
		}
		if !search && execution.is_some() {
			self
				.diagnostics
				.problem(format!("field `{path}.execution`"));
		}
		let Some(name) = name.or_else(|| search.then(|| TOOL_SEARCH.to_string())) else {
			self.diagnostics.problem(format!("`{path}` has no name"));
			return;
		};
		let (schema, kind) = match kind.as_deref() {
			Some("namespace") if namespace.is_none() => {
				let Some(tools) = nested.and_then(|raw| self.diagnostics.list(path, raw)) else {
					self.diagnostics.problem(format!("`{path}` has no tools"));
					return;
				};
				for (at, tool) in tools.iter().enumerate() {
					self.tool(
						&format!("{path}.tools[{at}]"),
						*tool,
						Some(&name),
						description.as_deref(),
					);
				}
				return;
			},
			Some("function" | TOOL_SEARCH) => {
				let Some(schema) = schema.filter(|schema| is_strict_object(schema)) else {
					self
						.diagnostics
						.problem(format!("`{path}.parameters` must be a valid object"));
					return;
				};
				let kind = if search {
					ToolKind::ToolSearch
				} else {
					ToolKind::Function { strict }
				};
				(schema, kind)
			},
			Some("custom") => {
				let grammar = self.grammar(path, format);
				let Ok(schema) = serde_json::from_str(CUSTOM_SCHEMA) else {
					return;
				};
				(schema, ToolKind::Custom { grammar })
			},
			other => {
				self
					.diagnostics
					.problem(format!("`{path}` tool type `{}`", other.unwrap_or("none")));
				return;
			},
		};
		let description = match (namespace_description, description) {
			(Some(group), Some(own)) => Some(format!("{group}\n\n{own}")),
			(Some(group), None) => Some(group.to_string()),
			(None, own) => own,
		};
		self.conversation.tools.push(Tool {
			path: path.to_string(),
			name,
			namespace: namespace.map(str::to_string),
			description,
			schema: schema.into(),
			kind,
		});
	}

	fn grammar(&mut self, path: &str, format: Option<JsonRef<'_>>) -> Option<String> {
		let fields = self
			.diagnostics
			.object(format_args!("{path}.format"), format?)?;
		let mut grammar = None;
		for (name, value) in &fields.0 {
			if name == "definition" {
				grammar = self
					.diagnostics
					.typed::<String>(&format!("{path}.format.definition"), value);
			}
		}
		grammar
	}

	fn input(&mut self, input: Read<'a, InputRead<'a>>) {
		let items: Vec<(usize, ItemRead<'a>)> = match input {
			Read::Done(InputRead::Text(text)) => {
				self.push(0, Role::User, vec![Content::Text(text.into())]);
				return;
			},
			Read::Done(InputRead::Items(items)) => items.into_iter().enumerate().collect(),
			Read::Raw(raw) => {
				if let Some(text) = text_of(raw) {
					self.push(0, Role::User, vec![Content::Text(text.into())]);
					return;
				}
				let Some(items) = self.diagnostics.list("input", raw) else {
					return;
				};
				let mut read = Vec::with_capacity(items.len());
				for (at, item) in items.into_iter().enumerate() {
					match raw_item(item) {
						Some(item) => read.push((at, item)),
						None => self
							.diagnostics
							.problem(format!("`input[{at}]` must be an object")),
					}
				}
				read
			},
		};
		// Tools first: a later item's call must find its route.
		for (at, item) in &items {
			let path = format!("input[{at}]");
			match item.fields.kind().as_deref() {
				Some("additional_tools") => {
					if self.diagnostics.reject_duplicates(&path, &item.fields) {
						continue;
					}
					for (name, value) in &item.fields.0 {
						match name.as_ref() {
							"type" | "id" | "role" => {},
							"tools" => self.tools(&format!("{path}.tools"), *value),
							other => self.diagnostics.problem(format!("field `{path}.{other}`")),
						}
					}
					self.no_nested(&path, item, true, true);
				},
				// Its other fields are read in turn order, with the result.
				Some("tool_search_output") => {
					for (name, value) in &item.fields.0 {
						if name == "tools" {
							self.tools(&format!("{path}.tools"), *value);
						}
					}
				},
				_ => {},
			}
		}
		for (at, item) in items {
			let path = format!("input[{at}]");
			if self.diagnostics.reject_duplicates(&path, &item.fields) {
				continue;
			}
			match item.fields.kind().as_deref() {
				Some("additional_tools") => {},
				Some("configuration_update") => {
					self.no_nested(&path, &item, true, true);
					self.configuration_update(&path, &item.fields);
				},
				Some("message") | None => self.message(at, &path, item),
				Some("reasoning") => {
					self.no_nested(&path, &item, false, true);
					self.reasoning_item(at, &path, &item.fields);
				},
				Some("function_call") => {
					self.no_nested(&path, &item, true, true);
					self.function_call(at, &path, &item.fields);
				},
				Some("custom_tool_call") => {
					self.no_nested(&path, &item, true, true);
					self.custom_tool_call(at, &path, &item.fields);
				},
				Some("function_call_output" | "custom_tool_call_output") => {
					self.no_nested(&path, &item, true, false);
					self.tool_output(at, &path, item);
				},
				Some("tool_search_call") => {
					self.no_nested(&path, &item, true, true);
					self.tool_search_call(at, &path, &item.fields);
				},
				Some("tool_search_output") => {
					self.no_nested(&path, &item, true, true);
					self.tool_search_output(at, &path, &item.fields);
				},
				Some(other) => self
					.diagnostics
					.problem(format!("`{path}` item type `{other}`")),
			}
		}
	}

	/// `content` and `output` are read apart from an item's other fields, so an item
	/// that has no use for them says so here.
	fn no_nested(&mut self, path: &str, item: &ItemRead<'a>, content: bool, output: bool) {
		if content && item.content.is_some() {
			self.diagnostics.problem(format!("field `{path}.content`"));
		}
		if output && item.output.is_some() {
			self.diagnostics.problem(format!("field `{path}.output`"));
		}
	}

	/// Sets the effort for the turns after it, so the last one is the one to send.
	fn configuration_update(&mut self, path: &str, fields: &Fields<'a>) {
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"reasoning" => {
					let path = format!("{path}.reasoning");
					let Some(fields) = self.diagnostics.object(&path, value) else {
						continue;
					};
					for (name, value) in &fields.0 {
						match name.as_ref() {
							"effort" => self.effort(&format!("{path}.effort"), *value),
							other => self.diagnostics.problem(format!("field `{path}.{other}`")),
						}
					}
				},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
	}

	fn message(&mut self, at: usize, path: &str, item: ItemRead<'a>) {
		self.no_nested(path, &item, false, true);
		let mut role = None;
		for (name, value) in &item.fields.0 {
			match name.as_ref() {
				"type" => {},
				"role" => {
					role = self
						.diagnostics
						.typed::<String>(&format!("{path}.role"), value)
				},
				"id" | "status" => self.diagnostics.drop_field("input item id and status"),
				PASSTHROUGH => self.diagnostics.drop_field(PASSTHROUGH),
				"phase" => self.diagnostics.drop_field("phase"),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let (Some(role), Some(content)) = (role, item.content) else {
			self
				.diagnostics
				.problem(format!("`{path}` needs `role` and `content`"));
			return;
		};
		if role == "user" {
			let parts = self.parts(&format!("{path}.content"), content);
			let blocks = parts.into_iter().map(Part::into_content).collect();
			self.push(at, Role::User, blocks);
			return;
		}
		let Some(texts) = self.texts(&format!("{path}.content"), content) else {
			return;
		};
		match role.as_str() {
			"assistant" => {
				let blocks = texts
					.into_iter()
					.map(|text| Content::Text(text.into()))
					.collect();
				self.push(at, Role::Assistant, blocks);
			},
			"developer" | "system" if !self.has_turn => {
				for text in texts {
					self.push_system(text);
				}
			},
			"developer" | "system" => {
				// Stays as sent even when empty, so its place in the turn order is kept.
				let blocks = texts
					.into_iter()
					.map(|text| Content::Text(text.into()))
					.collect();
				self.push_message(at, Role::System, blocks);
				self.has_turn = true;
			},
			other => self.diagnostics.problem(format!("`{path}` role `{other}`")),
		}
	}

	fn parts(&mut self, path: &str, content: Read<'a, ContentRead<'a>>) -> Vec<Part<'a>> {
		let blocks = match self.content(path, content) {
			Some(ContentRead::Text(text)) => return vec![Part::Text(text)],
			Some(ContentRead::Blocks(blocks)) => blocks,
			None => return Vec::new(),
		};
		let mut out = Vec::with_capacity(blocks.len());
		for (at, block) in blocks.into_iter().enumerate() {
			let path = format!("{path}[{at}]");
			let Some(fields) = self.block_fields(&path, block) else {
				continue;
			};
			if fields.kind().as_deref() == Some("input_image") {
				out.extend(self.image(&path, &fields).map(Part::Image));
			} else {
				let text = self.text_part(&format!("{path}[0]"), &fields);
				out.extend(text.map(Part::Text));
			}
		}
		out
	}

	fn content(&mut self, path: &str, content: Read<'a, ContentRead<'a>>) -> Option<ContentRead<'a>> {
		match content {
			Read::Done(content) => Some(content),
			Read::Raw(raw) => match text_of(raw) {
				Some(text) => Some(ContentRead::Text(text)),
				None => {
					let blocks = self.diagnostics.list(path, raw)?;
					Some(ContentRead::Blocks(
						blocks.into_iter().map(Read::Raw).collect(),
					))
				},
			},
		}
	}

	fn block_fields(&mut self, path: &str, block: Read<'a, Fields<'a>>) -> Option<Fields<'a>> {
		match block {
			Read::Raw(raw) => self.diagnostics.object(path, raw),
			Read::Done(fields) => (!self.diagnostics.reject_duplicates(path, &fields)).then_some(fields),
		}
	}

	fn image(&mut self, path: &str, fields: &Fields<'a>) -> Option<DataImage<'a>> {
		let mut url = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"image_url" => {
					url = self
						.diagnostics
						.text(format_args!("{path}.image_url"), value)
				},
				"detail" => self.diagnostics.drop_field("image detail"),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let image = url.and_then(data_image);
		if image.is_none() {
			self.diagnostics.problem(format!(
				"`{path}` must be a base64 data URL of a supported image"
			));
		}
		image
	}

	fn texts(&mut self, path: &str, content: Read<'a, ContentRead<'a>>) -> Option<Vec<Cow<'a, str>>> {
		let blocks = match self.content(path, content)? {
			ContentRead::Text(text) => return Some(vec![text]),
			ContentRead::Blocks(blocks) => blocks,
		};
		let mut texts = Vec::with_capacity(blocks.len());
		for (at, block) in blocks.into_iter().enumerate() {
			let path = format!("{path}[{at}]");
			if let Some(fields) = self.block_fields(&path, block) {
				texts.extend(self.text_part(&path, &fields));
			}
		}
		Some(texts)
	}

	fn text_part(&mut self, path: &str, fields: &Fields<'a>) -> Option<Cow<'a, str>> {
		match fields.kind().as_deref() {
			Some("input_text" | "output_text" | "text") => {
				let mut text = None;
				for (name, value) in &fields.0 {
					match name.as_ref() {
						"type" => {},
						"text" => text = self.diagnostics.text(format_args!("{path}.text"), value),
						"annotations" => self.diagnostics.drop_field("annotations"),
						"logprobs" => self.diagnostics.drop_field("logprobs"),
						other => self.diagnostics.problem(format!("field `{path}.{other}`")),
					}
				}
				if text.is_none() {
					self.diagnostics.problem(format!("`{path}` has no text"));
				}
				text
			},
			Some(other) => {
				self
					.diagnostics
					.problem(format!("`{path}` block type `{other}`"));
				None
			},
			None => {
				self.diagnostics.problem(format!("`{path}` has no type"));
				None
			},
		}
	}

	fn reasoning_item(&mut self, at: usize, path: &str, fields: &Fields<'a>) {
		let mut carrier_text = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" | "summary" | "content" => {},
				"encrypted_content" => {
					carrier_text = self
						.diagnostics
						.typed::<String>(&format!("{path}.encrypted_content"), value);
				},
				"id" | "status" => self.diagnostics.drop_field("input item id and status"),
				PASSTHROUGH => self.diagnostics.drop_field(PASSTHROUGH),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let Some(carrier_text) = carrier_text else {
			self.diagnostics.problem(format!("`{path}` has no carrier"));
			return;
		};
		let carrier = match carrier::open(
			self.scope,
			carrier::CarrierPurpose::ResponsesEncryptedContent,
			&carrier_text,
		) {
			Ok(item) => Carrier::decoded(item),
			Err(error) => Carrier::invalid(error),
		};
		self.push(
			at,
			Role::Assistant,
			vec![Content::Reasoning {
				carrier,
				next: None,
			}],
		);
	}

	fn function_call(&mut self, at: usize, path: &str, fields: &Fields<'a>) {
		let (mut call, mut name, mut namespace, mut arguments) = (None, None, None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => {},
				"call_id" => {
					call = self
						.diagnostics
						.typed::<String>(&format!("{path}.call_id"), value)
				},
				"name" => {
					name = self
						.diagnostics
						.typed::<String>(&format!("{path}.name"), value)
				},
				"namespace" => {
					namespace = self
						.diagnostics
						.typed::<String>(&format!("{path}.namespace"), value)
				},
				"arguments" => {
					arguments = self
						.diagnostics
						.text(format_args!("{path}.arguments"), value)
				},
				"id" | "status" => self.diagnostics.drop_field("input item id and status"),
				PASSTHROUGH => self.diagnostics.drop_field(PASSTHROUGH),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let (Some(call), Some(name), Some(arguments)) = (call, name, arguments) else {
			self
				.diagnostics
				.problem(format!("`{path}` needs `call_id`, `name` and `arguments`"));
			return;
		};
		let Some(input) = object_text(arguments) else {
			self
				.diagnostics
				.problem(format!("`{path}.arguments` must be a valid object"));
			return;
		};
		self.push_tool_call(at, call, name, namespace, input);
	}

	fn custom_tool_call(&mut self, at: usize, path: &str, fields: &Fields<'a>) {
		let (mut call, mut name, mut input) = (None, None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => {},
				"call_id" => {
					call = self
						.diagnostics
						.typed::<String>(&format!("{path}.call_id"), value)
				},
				"name" => {
					name = self
						.diagnostics
						.typed::<String>(&format!("{path}.name"), value)
				},
				"input" => {
					input = self
						.diagnostics
						.typed::<String>(&format!("{path}.input"), value)
				},
				"id" | "status" => self.diagnostics.drop_field("input item id and status"),
				PASSTHROUGH => self.diagnostics.drop_field(PASSTHROUGH),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let (Some(call), Some(name), Some(input)) = (call, name, input) else {
			self
				.diagnostics
				.problem(format!("`{path}` needs `call_id`, `name` and `input`"));
			return;
		};
		let mut arguments = String::from(r#"{"input":"#);
		push_string(&mut arguments, &input);
		arguments.push('}');
		if let Ok(input) = RawValue::from_string(arguments) {
			self.push_tool_call(at, call, name, None, input.into());
		}
	}

	fn push_tool_call(
		&mut self,
		at: usize,
		id: String,
		name: String,
		namespace: Option<String>,
		input: Json<'a>,
	) {
		self.open_calls.push(id.clone());
		self.push(
			at,
			Role::Assistant,
			vec![Content::ToolCall {
				id,
				name,
				namespace,
				input,
			}],
		);
	}

	fn tool_output(&mut self, at: usize, path: &str, item: ItemRead<'a>) {
		let mut call = None;
		for (name, value) in &item.fields.0 {
			match name.as_ref() {
				"type" => {},
				"call_id" => {
					call = self
						.diagnostics
						.typed::<String>(&format!("{path}.call_id"), value)
				},
				"id" | "status" => self.diagnostics.drop_field("input item id and status"),
				PASSTHROUGH => self.diagnostics.drop_field(PASSTHROUGH),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let output = item
			.output
			.map(|output| self.output(&format!("{path}.output"), output));
		let Some(call) = call else {
			self
				.diagnostics
				.problem(format!("`{path}` has no `call_id`"));
			return;
		};
		self.push_tool_result(
			at,
			path,
			call,
			output.unwrap_or(ToolOutput::Mixed(Vec::new())),
		);
	}

	fn tool_search_call(&mut self, at: usize, path: &str, fields: &Fields<'a>) {
		let (mut call, mut execution, mut arguments) = (None, None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" => {},
				"call_id" => {
					call = self
						.diagnostics
						.typed::<String>(&format!("{path}.call_id"), value)
				},
				"execution" => {
					execution = self
						.diagnostics
						.typed::<String>(&format!("{path}.execution"), value)
				},
				"arguments" => arguments = Some(*value),
				"id" | "status" => self.diagnostics.drop_field("input item id and status"),
				PASSTHROUGH => self.diagnostics.drop_field(PASSTHROUGH),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		if execution.as_deref() != Some("client") {
			self
				.diagnostics
				.problem(format!("`{path}` must run on the client"));
			return;
		}
		let (Some(call), Some(arguments)) = (call, arguments) else {
			self
				.diagnostics
				.problem(format!("`{path}` needs `call_id` and `arguments`"));
			return;
		};
		if !is_strict_object(arguments) {
			self
				.diagnostics
				.problem(format!("`{path}.arguments` must be a valid object"));
			return;
		}
		self.push_tool_call(at, call, TOOL_SEARCH.to_string(), None, arguments.into());
	}

	/// The tools it found were added to the list first, so the result only names them.
	fn tool_search_output(&mut self, at: usize, path: &str, fields: &Fields<'a>) {
		let (mut call, mut execution) = (None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"type" | "tools" => {},
				"call_id" => {
					call = self
						.diagnostics
						.typed::<String>(&format!("{path}.call_id"), value)
				},
				"execution" => {
					execution = self
						.diagnostics
						.typed::<String>(&format!("{path}.execution"), value)
				},
				"id" | "status" => self.diagnostics.drop_field("input item id and status"),
				PASSTHROUGH => self.diagnostics.drop_field(PASSTHROUGH),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		if execution.as_deref() != Some("client") {
			self
				.diagnostics
				.problem(format!("`{path}` must run on the client"));
			return;
		}
		let Some(call) = call else {
			self
				.diagnostics
				.problem(format!("`{path}` has no `call_id`"));
			return;
		};
		let found = format!("{path}.tools[");
		let names: Vec<&str> = self
			.conversation
			.tools
			.iter()
			.filter(|tool| tool.path.starts_with(&found))
			.map(|tool| tool.name.as_str())
			.collect();
		let text = if names.is_empty() {
			"No tools found.".to_string()
		} else {
			format!("Loaded tools: {}.", names.join(", "))
		};
		self.push_tool_result(at, path, call, ToolOutput::Text(text.into()));
	}

	fn push_tool_result(&mut self, at: usize, path: &str, call: String, output: ToolOutput<'a>) {
		match self.open_calls.iter().position(|open| *open == call) {
			Some(open) => {
				self.open_calls.remove(open);
			},
			None => self
				.diagnostics
				.problem(format!("`{path}` answers no open tool call")),
		}
		self.push(
			at,
			Role::User,
			vec![Content::ToolResult {
				call_id: call,
				failed: None,
				output,
			}],
		);
	}

	fn output(&mut self, path: &str, output: Read<'a, ContentRead<'a>>) -> ToolOutput<'a> {
		let blocks = match self.content(path, output) {
			Some(ContentRead::Text(text)) => return ToolOutput::Text(text.into()),
			Some(ContentRead::Blocks(blocks)) => Read::Done(ContentRead::Blocks(blocks)),
			None => return ToolOutput::Mixed(Vec::new()),
		};
		let parts = self.parts(path, blocks);
		ToolOutput::Mixed(parts.into_iter().map(Part::into_tool_part).collect())
	}

	fn push(&mut self, at: usize, role: Role, content: Vec<Content<'a>>) {
		if !content.is_empty() {
			self.push_message(at, role, content);
			self.has_turn = true;
		}
	}

	fn push_message(&mut self, at: usize, role: Role, content: Vec<Content<'a>>) {
		let blocks = content
			.into_iter()
			.enumerate()
			.map(|(block, content)| Block {
				id: BlockId { message: at, block },
				content,
			})
			.collect();
		self.conversation.messages.push(Message {
			index: at,
			role,
			blocks,
			messages_output_config: None,
		});
	}
}

fn raw_item(raw: JsonRef<'_>) -> Option<ItemRead<'_>> {
	let mut fields = raw.fields().ok()?;
	let mut take = |wanted: &str| {
		let at = fields.0.iter().position(|(name, _)| name == wanted)?;
		// A repeat stays behind, for `reject_duplicates` to find.
		let repeated = fields.0.iter().filter(|(name, _)| name == wanted).count() > 1;
		(!repeated).then(|| Read::Raw(fields.0.remove(at).1))
	};
	let (content, output) = (take("content"), take("output"));
	Some(ItemRead {
		fields,
		content,
		output,
	})
}

/// Text or an image, before knowing whether it's a block or a tool result's part.
enum Part<'a> {
	Text(Cow<'a, str>),
	Image(DataImage<'a>),
}

impl<'a> Part<'a> {
	fn into_content(self) -> Content<'a> {
		match self {
			Part::Text(text) => Content::Text(text.into()),
			Part::Image(image) => Content::DataImage(image),
		}
	}

	fn into_tool_part(self) -> ToolPart<'a> {
		match self {
			Part::Text(text) => ToolPart::Text(text.into()),
			Part::Image(image) => ToolPart::DataImage(image),
		}
	}
}

pub fn data_image(url: Cow<'_, str>) -> Option<DataImage<'_>> {
	let (media_type, data) = url.strip_prefix("data:")?.split_once(";base64,")?;
	if !is_image(media_type, data) {
		return None;
	}
	let media_type = media_type.to_string();
	let start = url.len() - data.len();
	let data = match url {
		Cow::Borrowed(url) => Cow::Borrowed(&url[start..]),
		Cow::Owned(url) => Cow::Owned(url[start..].to_string()),
	};
	Some(DataImage { media_type, data })
}

/// Tool arguments arrive as JSON text inside a JSON string.
pub fn object_text(text: Cow<'_, str>) -> Option<Json<'_>> {
	let raw = match text {
		Cow::Borrowed(text) => Cow::Borrowed(serde_json::from_str::<&RawValue>(text).ok()?),
		Cow::Owned(text) => Cow::Owned(RawValue::from_string(text).ok()?),
	};
	is_strict_object(raw.as_ref()).then(|| raw.into())
}

pub(super) struct ResponsesRead<'a> {
	pub(super) fields: Fields<'a>,
	pub(super) input: Option<InputRead<'a>>,
}

pub(super) enum InputRead<'a> {
	Text(Cow<'a, str>),
	Items(Vec<ItemRead<'a>>),
}

pub(super) struct ItemRead<'a> {
	pub(super) fields: Fields<'a>,
	pub(super) content: Option<Read<'a, ContentRead<'a>>>,
	pub(super) output: Option<Read<'a, ContentRead<'a>>>,
}

impl<'de: 'a, 'a> Deserialize<'de> for ResponsesRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct BodyVisitor;

		impl<'de> Visitor<'de> for BodyVisitor {
			type Value = ResponsesRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a request object")
			}

			fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
				let (fields, input) = read_fields(map, "input")?;
				Ok(ResponsesRead { fields, input })
			}
		}

		deserializer.deserialize_map(BodyVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for InputRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct InputVisitor;

		impl<'de> Visitor<'de> for InputVisitor {
			type Value = InputRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "text or a list of items")
			}

			fn visit_borrowed_str<E: Error>(self, text: &'de str) -> Result<Self::Value, E> {
				Ok(InputRead::Text(Cow::Borrowed(text)))
			}

			fn visit_str<E: Error>(self, text: &str) -> Result<Self::Value, E> {
				Ok(InputRead::Text(Cow::Owned(text.to_string())))
			}

			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
				let mut items = Vec::new();
				while let Some(item) = seq.next_element::<ItemRead<'de>>()? {
					items.push(item);
				}
				Ok(InputRead::Items(items))
			}
		}

		deserializer.deserialize_any(InputVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for ItemRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct ItemVisitor;

		impl<'de> Visitor<'de> for ItemVisitor {
			type Value = ItemRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "an input item")
			}

			fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
				let (mut fields, mut content, mut output) = (Vec::new(), None, None);
				while let Some(Name(name)) = map.next_key()? {
					let nested = match name.as_ref() {
						"content" => &mut content,
						"output" => &mut output,
						_ => {
							fields.push((name, map.next_value::<JsonRef<'de>>()?));
							continue;
						},
					};
					if nested.is_some() {
						return Err(M::Error::custom("repeated field"));
					}
					*nested = Some(Read::Done(map.next_value()?));
				}
				Ok(ItemRead {
					fields: Fields(fields),
					content,
					output,
				})
			}
		}

		deserializer.deserialize_map(ItemVisitor)
	}
}

/// A Responses request the provider gets unconverted, read for the settings the caller records.
pub fn read<'a>(fields: &Fields<'a>, reading: &mut Reading<'a>) {
	for (name, raw) in &fields.0 {
		match name.as_ref() {
			"max_output_tokens" => reading.settings().max_tokens = Reading::limit(*raw),
			"temperature" => reading.settings().temperature = Some(*raw),
			"top_p" => reading.settings().top_p = Some(*raw),
			"stream" => reading.stream("stream", *raw),
			"reasoning" => {
				let effort = raw
					.fields()
					.ok()
					.and_then(|reasoning| word(&reasoning, "effort"));
				if let Some(effort) = effort {
					reading.effort(&effort);
				}
			},
			_ => {},
		}
	}
}
