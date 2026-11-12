//! A Chat Completions client's request. Each field is converted, dropped with a record, or
//! refused. What a provider of another format can't accept is refused: more than one choice,
//! log probabilities, audio, predicted output, a seed, penalties, remote images and files.
//!
//! A field sent as `null` is read as left out, as OpenAI reads it.
//!
//! Reads a request's messages and their content in the same pass as the body, so long
//! text is scanned once instead of once per level.

use std::borrow::Cow;
use std::fmt;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, Visitor};

use super::{open_call_id, split_call_id};
use crate::ir::conversation::{
	Block, BlockId, Carrier, Content, Conversation, Effort, Message, OutputFormat, Role, Settings,
	Text, Tool, ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::input::RequestInput;
use crate::json::{Fields, Json, JsonRef, is_strict_object, text_of};
use crate::prompt::Upstream;
use crate::prompt::diagnostics::{Diagnostics, TokenLimit};
use crate::prompt::passthrough::{Reading, field};
use crate::protocols::gemini::render::signs_text;
use crate::protocols::responses::parse::{data_image, object_text};
use crate::protocols::{ContentRead, Read, UpstreamProtocol, read_fields};

/// A function declared without `parameters` takes none.
const NO_PARAMETERS: &str = r#"{"type":"object","properties":{}}"#;

pub struct Parsed<'a> {
	pub(crate) conversation: Conversation<'a>,
	pub(crate) diagnostics: Diagnostics,
}

/// `upstream` decides which replay carriers match the destination, the default output limit, and the
/// settings refused in this client's words rather than the target renderer's. `one_pass`
/// changes only the cost: both ways must give the same result.
pub fn parse<'a>(body: RequestInput<'a>, upstream: Upstream<'_>, one_pass: bool) -> Parsed<'a> {
	let mut parser = Parser {
		upstream,
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
	upstream: Upstream<'c>,
	diagnostics: Diagnostics,
	/// Each call's id and name, until its result.
	open_calls: Vec<(String, String)>,
	/// A system message after this point stays where it is.
	has_turn: bool,
	conversation: Conversation<'a>,
}

/// A tool call read from an assistant message, with the reasoning encoded into its id.
struct Call<'a> {
	id: String,
	name: String,
	input: Json<'a>,
	reasoning: Vec<PositionedCarrier>,
}

struct PositionedCarrier {
	text_offset: usize,
	carrier: Carrier,
}

impl<'a> Parser<'a, '_> {
	fn request(&mut self, body: RequestInput<'a>, one_pass: bool) {
		let (fields, mut messages) = match one_pass.then(|| body.read_raw::<BodyRead<'a>>()).flatten() {
			Some(read) => (read.other_fields, read.messages.map(Read::Done)),
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
		let mut has_model = false;
		let (mut limit, mut legacy_limit) = (None, None);
		for (name, raw) in &fields.0 {
			if raw.is_null() {
				continue;
			}
			let settings = &mut self.conversation.settings;
			match name.as_ref() {
				"model" => {
					has_model = true;
					self.diagnostics.typed::<String>("model", raw);
				},
				"messages" => messages = Some(Read::Raw(*raw)),
				"max_completion_tokens" => limit = Some(self.diagnostics.token_limit(name, raw)),
				"max_tokens" => legacy_limit = Some(self.diagnostics.token_limit(name, raw)),
				"temperature" => settings.temperature = Some(*raw),
				"top_p" => settings.top_p = Some(*raw),
				"stream" => settings.stream = self.diagnostics.typed(name, raw),
				"stream_options" => self.stream_options(*raw),
				"stop" => self.stop(*raw),
				"n" => {
					if self
						.diagnostics
						.typed::<u64>(name, raw)
						.is_some_and(|n| n != 1)
					{
						self.diagnostics.problem("`n` other than 1");
					}
				},
				"presence_penalty" | "frequency_penalty" => {
					if self
						.diagnostics
						.typed::<f64>(name, raw)
						.is_some_and(|value| value != 0.0)
					{
						self.diagnostics.problem(format!("`{name}` other than 0"));
					}
				},
				"logit_bias" => {
					let bias = self.diagnostics.object(name, raw);
					if bias.is_some_and(|bias| !bias.0.is_empty()) {
						self.diagnostics.problem("`logit_bias`");
					}
				},
				"logprobs" => {
					if self.diagnostics.typed::<bool>(name, raw) == Some(true) {
						self.diagnostics.problem("`logprobs`");
					}
				},
				"modalities" => {
					let modalities = self.diagnostics.typed::<Vec<String>>(name, raw);
					if modalities.is_some_and(|list| list.iter().any(|kind| kind != "text")) {
						self.diagnostics.problem("`modalities` other than text");
					}
				},
				"store" => match self.diagnostics.typed::<bool>(name, raw) {
					Some(false) => self.diagnostics.drop_field("store"),
					Some(true) => self.diagnostics.problem("`store: true`"),
					None => {},
				},
				"service_tier" => match self.diagnostics.typed::<String>(name, raw).as_deref() {
					Some("auto" | "default") => self.diagnostics.drop_field("service_tier"),
					Some(other) => self
						.diagnostics
						.problem(format!("`service_tier` `{other}`")),
					None => {},
				},
				"user" => self.diagnostics.drop_field("user"),
				"safety_identifier" => self.diagnostics.drop_field("safety_identifier"),
				"prompt_cache_key" => self.diagnostics.drop_field("prompt_cache_key"),
				"prompt_cache_retention" => self.diagnostics.drop_field("prompt_cache_retention"),
				"metadata" => self.diagnostics.drop_field("metadata"),
				"verbosity" => self.diagnostics.drop_field("verbosity"),
				"tools" => self.tools(*raw),
				"tool_choice" => self.conversation.settings.tool_choice = self.tool_choice(*raw),
				"parallel_tool_calls" => {
					self.conversation.settings.parallel_tool_calls = self.diagnostics.typed(name, raw);
				},
				"response_format" => self.response_format(*raw),
				"reasoning_effort" => self.effort(*raw),
				other => self.diagnostics.problem(format!("field `{other}`")),
			}
		}
		if !has_model {
			self.diagnostics.problem("missing `model`");
		}
		match messages {
			Some(messages) => self.messages(messages),
			None => self.diagnostics.problem("missing `messages`"),
		}
		self.limit(limit, legacy_limit);
		if !self.open_calls.is_empty() {
			let calls: Vec<&str> = self.open_calls.iter().map(|(id, _)| id.as_str()).collect();
			let calls = calls.join(", ");
			self
				.diagnostics
				.problem(format!("tool calls with no result: {calls}"));
		}
	}

	/// `max_completion_tokens` replaced `max_tokens`, so it wins when a client sends both.
	fn limit(&mut self, limit: Option<TokenLimit>, legacy: Option<TokenLimit>) {
		let limit = match (limit, legacy) {
			(Some(limit), Some(_)) => {
				self
					.diagnostics
					.drop_field("max_tokens beside max_completion_tokens");
				Some(limit)
			},
			(limit, legacy) => limit.or(legacy),
		};
		let settings = &mut self.conversation.settings;
		match limit {
			Some(TokenLimit::Positive(limit)) => settings.max_tokens = Some(limit),
			Some(TokenLimit::Zero | TokenLimit::Unreadable) => {},
			None => {
				settings.max_tokens = self.upstream.default_max_tokens;
				if settings.max_tokens.is_none() && self.upstream.protocol == UpstreamProtocol::Messages {
					self
						.diagnostics
						.problem("no `max_completion_tokens`, and no default for this model");
				}
			},
		}
	}

	fn stream_options(&mut self, raw: JsonRef<'_>) {
		let Some(fields) = self.diagnostics.object("stream_options", raw) else {
			return;
		};
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"include_usage" => {
					let include = self
						.diagnostics
						.typed::<bool>("stream_options.include_usage", value);
					self.conversation.settings.include_usage = include == Some(true);
				},
				"include_obfuscation" => self
					.diagnostics
					.drop_field("stream_options.include_obfuscation"),
				other => self
					.diagnostics
					.problem(format!("field `stream_options.{other}`")),
			}
		}
	}

	fn stop(&mut self, raw: JsonRef<'_>) {
		let stops = match text_of(raw) {
			Some(stop) => vec![stop.into_owned()],
			None => self
				.diagnostics
				.typed::<Vec<String>>("stop", raw)
				.unwrap_or_default(),
		};
		self.conversation.settings.stop_sequences = stops;
	}

	/// Messages has no effort below `low`, so a client hears that here rather than from the
	/// renderer, in another client's words.
	fn effort(&mut self, raw: JsonRef<'_>) {
		let word = self.diagnostics.typed::<String>("reasoning_effort", raw);
		let effort = match word.as_deref() {
			Some("none") => Effort::Off,
			Some("minimal") => Effort::Minimal,
			Some("low") => Effort::Low,
			Some("medium") => Effort::Medium,
			Some("high") => Effort::High,
			Some("xhigh") => Effort::XHigh,
			Some(other) => {
				self
					.diagnostics
					.problem(format!("`reasoning_effort` `{other}`"));
				return;
			},
			None => return,
		};
		let below_low = matches!(effort, Effort::Off | Effort::Minimal);
		if below_low && self.upstream.protocol == UpstreamProtocol::Messages {
			let word = word.unwrap_or_default();
			self
				.diagnostics
				.problem(format!("`reasoning_effort` `{word}` has no Messages form"));
			return;
		}
		let settings = &mut self.conversation.settings;
		settings.effort = Some(effort);
		// Chat Completions has no way to ask for reasoning text, and engines that give it send
		// it unasked, so a client that wants reasoning gets its text.
		settings.show_reasoning_summary = effort != Effort::Off;
	}

	fn response_format(&mut self, raw: JsonRef<'a>) {
		let Some(fields) = self.diagnostics.object("response_format", raw) else {
			return;
		};
		let mut schema = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"json_schema" => schema = self.json_schema(*value),
				other => self
					.diagnostics
					.problem(format!("field `response_format.{other}`")),
			}
		}
		let format = match (fields.kind().as_deref(), schema) {
			(Some("text"), None) => None,
			(Some("json_object"), None) if self.upstream.protocol == UpstreamProtocol::Messages => {
				self
					.diagnostics
					.problem("`response_format` `json_object` has no Messages form");
				None
			},
			(Some("json_object"), None) => Some(OutputFormat::Json),
			(Some("json_schema"), Some(schema)) => Some(OutputFormat::JsonSchema(schema.into())),
			(Some("json_schema"), None) => {
				self
					.diagnostics
					.problem("`response_format` has no `json_schema.schema`");
				None
			},
			(kind, _) => {
				let kind = kind.unwrap_or_default();
				self
					.diagnostics
					.problem(format!("`response_format` type `{kind}`"));
				None
			},
		};
		self.conversation.settings.output_format = format;
	}

	fn json_schema(&mut self, raw: JsonRef<'a>) -> Option<JsonRef<'a>> {
		let fields = self
			.diagnostics
			.object("response_format.json_schema", raw)?;
		let mut schema = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"schema" => schema = Some(*value),
				"name" | "description" | "strict" => self
					.diagnostics
					.drop_field("response_format name and strict"),
				other => self
					.diagnostics
					.problem(format!("field `response_format.json_schema.{other}`")),
			}
		}
		schema
	}

	fn tools(&mut self, raw: JsonRef<'a>) {
		let Some(tools) = self.diagnostics.list("tools", raw) else {
			return;
		};
		for (at, tool) in tools.into_iter().enumerate() {
			self.tool(&format!("tools[{at}]"), tool);
		}
	}

	fn tool(&mut self, path: &str, raw: JsonRef<'a>) {
		let Some(fields) = self.diagnostics.object(path, raw) else {
			return;
		};
		let mut function = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"function" => function = Some(*value),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let kind = fields.kind();
		if kind.as_deref() != Some("function") {
			let kind = kind.unwrap_or_default();
			self
				.diagnostics
				.problem(format!("`{path}` tool type `{kind}`"));
			return;
		}
		let Some(function) = function else {
			self
				.diagnostics
				.problem(format!("`{path}` has no `function`"));
			return;
		};
		let path = format!("{path}.function");
		let Some(fields) = self.diagnostics.object(&path, function) else {
			return;
		};
		let (mut name, mut description, mut schema, mut strict) = (None, None, None, None);
		for (field, value) in &fields.0 {
			let at = format!("{path}.{field}");
			match field.as_ref() {
				"name" => name = self.diagnostics.typed::<String>(&at, value),
				"description" => description = self.diagnostics.typed::<String>(&at, value),
				"parameters" => schema = Some(*value),
				"strict" => strict = self.diagnostics.typed::<Option<bool>>(&at, value).flatten(),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let Some(name) = name else {
			self.diagnostics.problem(format!("`{path}` has no name"));
			return;
		};
		let schema = match schema {
			Some(schema) if is_strict_object(schema) => schema,
			Some(_) => {
				self
					.diagnostics
					.problem(format!("`{path}.parameters` must be a valid object"));
				return;
			},
			None => match serde_json::from_str(NO_PARAMETERS) {
				Ok(schema) => schema,
				Err(err) => unreachable!("a fixed schema: {err}"),
			},
		};
		self.conversation.tools.push(Tool {
			path,
			name,
			namespace: None,
			description,
			schema: schema.into(),
			kind: ToolKind::Function { strict },
		});
	}

	fn tool_choice(&mut self, raw: JsonRef<'_>) -> Option<ToolChoice> {
		if let Some(word) = text_of(raw) {
			return match word.as_ref() {
				"auto" => Some(ToolChoice::Auto),
				"required" => Some(ToolChoice::Required),
				"none" => Some(ToolChoice::None),
				other => {
					self.diagnostics.problem(format!("`tool_choice` `{other}`"));
					None
				},
			};
		}
		let fields = self.diagnostics.object("tool_choice", raw)?;
		let mut function = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"function" => function = Some(*value),
				other => self
					.diagnostics
					.problem(format!("field `tool_choice.{other}`")),
			}
		}
		let kind = fields.kind();
		if kind.as_deref() != Some("function") {
			let kind = kind.unwrap_or_default();
			self
				.diagnostics
				.problem(format!("`tool_choice` type `{kind}`"));
			return None;
		}
		let fields = function.and_then(|raw| self.diagnostics.object("tool_choice.function", raw));
		let mut name = None;
		for (field, value) in fields.iter().flat_map(|fields| &fields.0) {
			match field.as_ref() {
				"name" => {
					name = self
						.diagnostics
						.typed::<String>("tool_choice.function.name", value)
				},
				other => self
					.diagnostics
					.problem(format!("field `tool_choice.function.{other}`")),
			}
		}
		if name.is_none() {
			self
				.diagnostics
				.problem("`tool_choice` needs `function.name`");
		}
		name.map(ToolChoice::Tool)
	}

	fn messages(&mut self, messages: Read<'a, Vec<MessageRead<'a>>>) {
		let messages: Vec<_> = match messages {
			Read::Done(messages) => messages.into_iter().map(Read::Done).collect(),
			Read::Raw(raw) => {
				let Some(messages) = self.diagnostics.list("messages", raw) else {
					return;
				};
				messages.into_iter().map(Read::Raw).collect()
			},
		};
		for (at, message) in messages.into_iter().enumerate() {
			let path = format!("messages[{at}]");
			let (fields, content) = match message {
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
			let role = fields.0.iter().find(|(name, _)| name == "role");
			let role = role.and_then(|(_, raw)| text_of(raw));
			match role.as_deref() {
				Some("system" | "developer") => self.system_message(at, &path, &fields, content),
				Some("user") => self.user_message(at, &path, &fields, content),
				Some("assistant") => self.assistant_message(at, &path, &fields, content),
				Some("tool") => self.tool_message(at, &path, &fields, content),
				Some(other) => self.diagnostics.problem(format!("`{path}` role `{other}`")),
				None => self.diagnostics.problem(format!("`{path}` has no role")),
			}
		}
	}

	fn system_message(
		&mut self,
		at: usize,
		path: &str,
		fields: &Fields<'a>,
		mut content: Option<Read<'a, ContentRead<'a>>>,
	) {
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"role" => {},
				"content" => content = Some(Read::Raw(*value)),
				other => self.other_field(path, other, *value),
			}
		}
		let texts = content.and_then(|content| self.texts(&format!("{path}.content"), content));
		let Some(texts) = texts else {
			self.diagnostics.problem(format!("`{path}` has no content"));
			return;
		};
		if self.has_turn {
			// Stays as sent even when empty, so its place in the turn order is kept.
			let blocks = texts
				.into_iter()
				.map(|text| Content::Text(text.into()))
				.collect();
			self.push_message(at, Role::System, blocks);
		} else {
			let system = self.conversation.system.get_or_insert_with(Vec::new);
			system.extend(texts.into_iter().map(Text::from));
		}
	}

	fn user_message(
		&mut self,
		at: usize,
		path: &str,
		fields: &Fields<'a>,
		mut content: Option<Read<'a, ContentRead<'a>>>,
	) {
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"role" => {},
				"content" => content = Some(Read::Raw(*value)),
				other => self.other_field(path, other, *value),
			}
		}
		match content {
			Some(content) => {
				let content = self.user_content(&format!("{path}.content"), content);
				self.push(at, Role::User, content);
			},
			None => self.diagnostics.problem(format!("`{path}` has no content")),
		}
	}

	fn user_content(&mut self, path: &str, content: Read<'a, ContentRead<'a>>) -> Vec<Content<'a>> {
		let parts = match self.content(path, content) {
			Some(ContentRead::Text(text)) => return vec![Content::Text(text.into())],
			Some(ContentRead::Blocks(parts)) => parts,
			None => return Vec::new(),
		};
		let mut content = Vec::with_capacity(parts.len());
		for (at, part) in parts.into_iter().enumerate() {
			let path = format!("{path}[{at}]");
			let Some(fields) = self.part_fields(&path, part) else {
				continue;
			};
			match fields.kind().as_deref() {
				Some("image_url") => content.extend(self.image(&path, &fields)),
				_ => content.extend(
					self
						.text_part(&path, &fields)
						.map(|text| Content::Text(text.into())),
				),
			}
		}
		content
	}

	fn image(&mut self, path: &str, fields: &Fields<'a>) -> Option<Content<'a>> {
		let mut url = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"image_url" => {
					let path = format!("{path}.image_url");
					let Some(fields) = self.diagnostics.object(&path, value) else {
						continue;
					};
					for (name, value) in &fields.0 {
						match name.as_ref() {
							"url" => url = self.diagnostics.text(format_args!("{path}.url"), value),
							"detail" => self.diagnostics.drop_field("image detail"),
							other => self.diagnostics.problem(format!("field `{path}.{other}`")),
						}
					}
				},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let image = url.and_then(data_image);
		if image.is_none() {
			self.diagnostics.problem(format!(
				"`{path}` must be a base64 data URL of a supported image"
			));
		}
		image.map(Content::DataImage)
	}

	fn assistant_message(
		&mut self,
		at: usize,
		path: &str,
		fields: &Fields<'a>,
		mut content: Option<Read<'a, ContentRead<'a>>>,
	) {
		let mut calls = Vec::new();
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"role" => {},
				"content" => content = (!value.is_null()).then_some(Read::Raw(*value)),
				"tool_calls" => calls = self.tool_calls(&format!("{path}.tool_calls"), *value),
				// Clients that show reasoning may send its text back. The text isn't what a
				// provider replays, so it's dropped; replay state is in the call ids.
				"reasoning_content" | "reasoning" => {
					self.diagnostics.drop_field("assistant reasoning text")
				},
				other => self.other_field(path, other, *value),
			}
		}
		let texts = content
			.and_then(|content| self.texts(&format!("{path}.content"), content))
			.unwrap_or_default();
		let text: String = texts.iter().map(|text| text.as_ref()).collect();
		let mut content = Vec::with_capacity(texts.len() + calls.len());
		let mut calls = calls.into_iter();
		let first = calls.next();
		let mut first_call = None;
		if let Some(mut call) = first {
			self.push_first_call_content(at, &mut content, &text, &mut call);
			first_call = Some(call);
		} else if !text.is_empty() {
			content.push(Content::Text(text.clone().into()));
		}
		for mut call in first_call.into_iter().chain(calls) {
			if call
				.reasoning
				.iter()
				.any(|reasoning| reasoning.text_offset != text.len())
			{
				self.diagnostics.problem(format!(
					"`{path}.tool_calls` reasoning no longer matches assistant text"
				));
			}
			let call_at = content.len() + call.reasoning.len();
			Self::push_reasoning(at, &mut content, &mut call, call_at);
			self.open_calls.push((call.id.clone(), call.name.clone()));
			content.push(Content::ToolCall {
				id: call.id,
				name: call.name,
				namespace: None,
				input: call.input,
			});
		}
		self.push(at, Role::Assistant, content);
	}

	fn push_first_call_content(
		&mut self,
		message: usize,
		content: &mut Vec<Content<'a>>,
		text: &str,
		call: &mut Call<'a>,
	) {
		let valid = call.reasoning.iter().try_fold(0, |before, reasoning| {
			(reasoning.text_offset >= before
				&& reasoning.text_offset <= text.len()
				&& text.is_char_boundary(reasoning.text_offset))
			.then_some(reasoning.text_offset)
		});
		if valid.is_none() {
			self
				.diagnostics
				.problem("assistant reasoning no longer matches assistant text");
			if !text.is_empty() {
				content.push(Content::Text(text.to_string().into()));
			}
			return;
		}

		let mut cursor = 0;
		let mut text_blocks = 0;
		for reasoning in &call.reasoning {
			if reasoning.text_offset > cursor {
				text_blocks += 1;
				cursor = reasoning.text_offset;
			}
		}
		text_blocks += usize::from(cursor < text.len());
		let call_at = content.len() + call.reasoning.len() + text_blocks;
		cursor = 0;
		for reasoning in call.reasoning.drain(..) {
			if reasoning.text_offset > cursor {
				content.push(Content::Text(
					text[cursor..reasoning.text_offset].to_string().into(),
				));
			}
			Self::push_carrier(message, content, reasoning.carrier, call_at);
			cursor = reasoning.text_offset;
		}
		if cursor < text.len() {
			content.push(Content::Text(text[cursor..].to_string().into()));
		}
	}

	/// Each encoded item before its call, linked to it.
	fn push_reasoning(
		at: usize,
		content: &mut Vec<Content<'a>>,
		call: &mut Call<'a>,
		call_at: usize,
	) {
		for reasoning in call.reasoning.drain(..) {
			Self::push_carrier(at, content, reasoning.carrier, call_at);
		}
	}

	fn push_carrier(
		message: usize,
		content: &mut Vec<Content<'a>>,
		carrier: Carrier,
		call_at: usize,
	) {
		content.push(Content::Reasoning {
			carrier,
			next: Some(BlockId {
				message,
				block: call_at,
			}),
		});
	}

	fn tool_calls(&mut self, path: &str, raw: JsonRef<'a>) -> Vec<Call<'a>> {
		let Some(list) = self.diagnostics.list(path, raw) else {
			return Vec::new();
		};
		let mut calls = Vec::with_capacity(list.len());
		for (at, call) in list.into_iter().enumerate() {
			calls.extend(self.tool_call(&format!("{path}[{at}]"), call));
		}
		calls
	}

	fn tool_call(&mut self, path: &str, raw: JsonRef<'a>) -> Option<Call<'a>> {
		let fields = self.diagnostics.object(path, raw)?;
		let (mut id, mut function) = (None, None);
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"type" => {},
				"id" => {
					id = self
						.diagnostics
						.typed::<String>(&format!("{path}.id"), value)
				},
				"function" => function = Some(*value),
				// Some clients number the calls they echo, as in a streamed delta.
				"index" => {},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let kind = fields.kind();
		if kind.as_deref().is_some_and(|kind| kind != "function") {
			let kind = kind.unwrap_or_default();
			self
				.diagnostics
				.problem(format!("`{path}` call type `{kind}`"));
			return None;
		}
		let function_path = format!("{path}.function");
		let fields = function.and_then(|raw| self.diagnostics.object(&function_path, raw));
		let (mut name, mut arguments) = (None, None);
		for (field, value) in fields.iter().flat_map(|fields| &fields.0) {
			let at = format!("{function_path}.{field}");
			match field.as_ref() {
				"name" => name = self.diagnostics.typed::<String>(&at, value),
				"arguments" => arguments = self.diagnostics.text(&at, value),
				_ => self.diagnostics.problem(format!("field `{at}`")),
			}
		}
		let (Some(id), Some(name), Some(arguments)) = (id, name, arguments) else {
			self.diagnostics.problem(format!(
				"`{path}` needs `id`, `function.name` and `function.arguments`"
			));
			return None;
		};
		// A call that takes no arguments may be sent back with none.
		let arguments = if arguments.is_empty() {
			Cow::Borrowed("{}")
		} else {
			arguments
		};
		let Some(input) = object_text(arguments) else {
			self.diagnostics.problem(format!(
				"`{path}.function.arguments` must be a valid object"
			));
			return None;
		};
		let (call, reasoning) = self.call_id(&id);
		Some(Call {
			id: call,
			name,
			input,
			reasoning,
		})
	}

	/// The provider's call id, and the reasoning encoded into the id the client has.
	fn call_id(&mut self, id: &str) -> (String, Vec<PositionedCarrier>) {
		let (call, carrier) = split_call_id(id);
		let Some(carrier) = carrier else {
			return (call.to_string(), Vec::new());
		};
		let upstream = self.upstream;
		let items = match open_call_id(upstream.scope, call, carrier) {
			Ok(items) => items,
			// Left for the renderer to refuse, unless a stage removes it.
			Err(error) => {
				return (
					call.to_string(),
					vec![PositionedCarrier {
						text_offset: 0,
						carrier: Carrier::invalid(error),
					}],
				);
			},
		};
		let mut reasoning = Vec::with_capacity(items.len());
		for item in items {
			// Text in Chat history is joined into one string, and Gemini can't take a signature
			// for part of it.
			if upstream.protocol == UpstreamProtocol::Gemini && signs_text(&item.item) {
				self.diagnostics.drop_field("reasoning signatures on text");
				continue;
			}
			reasoning.push(PositionedCarrier {
				text_offset: item.text_offset,
				carrier: Carrier::decoded(item.item),
			});
		}
		(call.to_string(), reasoning)
	}

	fn tool_message(
		&mut self,
		at: usize,
		path: &str,
		fields: &Fields<'a>,
		mut content: Option<Read<'a, ContentRead<'a>>>,
	) {
		let (mut call, mut name) = (None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
				"role" => {},
				"tool_call_id" => {
					call = self
						.diagnostics
						.typed::<String>(&format!("{path}.tool_call_id"), value);
				},
				"content" => content = Some(Read::Raw(*value)),
				// Older clients name the function a result is for, which its call already says.
				"name" => {
					name = self
						.diagnostics
						.typed::<String>(&format!("{path}.name"), value)
				},
				other => self.other_field(path, other, *value),
			}
		}
		let texts = content.and_then(|content| self.texts(&format!("{path}.content"), content));
		let (Some(call), Some(texts)) = (call, texts) else {
			self
				.diagnostics
				.problem(format!("`{path}` needs `tool_call_id` and `content`"));
			return;
		};
		let (call, _) = split_call_id(&call);
		match self.open_calls.iter().position(|(open, _)| open == call) {
			Some(open) => {
				let (_, called) = self.open_calls.remove(open);
				match name {
					Some(name) if name == called => self.diagnostics.drop_field("tool result name"),
					Some(_) => self
						.diagnostics
						.problem(format!("`{path}.name` isn't the name of its call")),
					None => {},
				}
			},
			None => self
				.diagnostics
				.problem(format!("`{path}` answers no open tool call")),
		}
		let output = match <[_; 1]>::try_from(texts) {
			Ok([text]) => ToolOutput::Text(text.into()),
			Err(texts) => ToolOutput::Mixed(
				texts
					.into_iter()
					.map(|text| ToolPart::Text(text.into()))
					.collect(),
			),
		};
		let result = Content::ToolResult {
			call_id: call.to_string(),
			failed: None,
			output,
		};
		self.push_message(at, Role::User, vec![result]);
		self.has_turn = true;
	}

	/// A string, or a list of text parts.
	fn texts(&mut self, path: &str, content: Read<'a, ContentRead<'a>>) -> Option<Vec<Cow<'a, str>>> {
		let parts = match self.content(path, content)? {
			ContentRead::Text(text) => return Some(vec![text]),
			ContentRead::Blocks(parts) => parts,
		};
		let mut texts = Vec::with_capacity(parts.len());
		for (at, part) in parts.into_iter().enumerate() {
			let path = format!("{path}[{at}]");
			if let Some(fields) = self.part_fields(&path, part) {
				texts.extend(self.text_part(&path, &fields));
			}
		}
		Some(texts)
	}

	fn content(&mut self, path: &str, content: Read<'a, ContentRead<'a>>) -> Option<ContentRead<'a>> {
		match content {
			Read::Done(content) => Some(content),
			Read::Raw(raw) => match text_of(raw) {
				Some(text) => Some(ContentRead::Text(text)),
				None => {
					let parts = self.diagnostics.list(path, raw)?;
					Some(ContentRead::Blocks(
						parts.into_iter().map(Read::Raw).collect(),
					))
				},
			},
		}
	}

	fn part_fields(&mut self, path: &str, part: Read<'a, Fields<'a>>) -> Option<Fields<'a>> {
		match part {
			Read::Raw(raw) => self.diagnostics.object(path, raw),
			Read::Done(fields) => (!self.diagnostics.reject_duplicates(path, &fields)).then_some(fields),
		}
	}

	fn text_part(&mut self, path: &str, fields: &Fields<'a>) -> Option<Cow<'a, str>> {
		match fields.kind().as_deref() {
			Some("text") => {
				let mut text = None;
				for (name, value) in &fields.0 {
					match name.as_ref() {
						"type" => {},
						"text" => text = self.diagnostics.text(format_args!("{path}.text"), value),
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
					.problem(format!("`{path}` part type `{other}`"));
				None
			},
			None => {
				self.diagnostics.problem(format!("`{path}` has no type"));
				None
			},
		}
	}

	/// A message field this client may send as `null`, or must not send at all.
	fn other_field(&mut self, path: &str, name: &str, value: JsonRef<'_>) {
		let empty = value.is_null();
		match name {
			"refusal" | "audio" | "function_call" if empty => {},
			other => self.diagnostics.problem(format!("field `{path}.{other}`")),
		}
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

pub(super) struct BodyRead<'a> {
	pub(super) other_fields: Fields<'a>,
	pub(super) messages: Option<Vec<MessageRead<'a>>>,
}

pub(super) struct MessageRead<'a> {
	pub(super) other_fields: Fields<'a>,
	/// `None` when left out, or `null` in an assistant message.
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
				let (other_fields, content) = read_fields::<_, Option<ContentRead<'de>>>(map, "content")?;
				// An assistant that only calls tools sends `null`. Elsewhere it's a problem,
				// reported where it appears.
				let assistant = other_fields
					.0
					.iter()
					.any(|(name, raw)| name == "role" && raw.text().as_deref() == Some("assistant"));
				if content.as_ref().is_some_and(Option::is_none) && !assistant {
					return Err(M::Error::custom(
						"null content outside an assistant message",
					));
				}
				Ok(MessageRead {
					other_fields,
					content: content.flatten(),
				})
			}
		}

		deserializer.deserialize_map(MessageVisitor)
	}
}

/// A Chat Completions request the provider gets unconverted, read for the settings the caller
/// records.
pub fn read<'a>(fields: &Fields<'a>, reading: &mut Reading<'a>) {
	let mut limit = None;
	for (name, raw) in &fields.0 {
		match name.as_ref() {
			// The newer name wins when a client sends both.
			"max_completion_tokens" => limit = Reading::limit(*raw).or(limit),
			"max_tokens" => limit = limit.or_else(|| Reading::limit(*raw)),
			"temperature" => reading.settings().temperature = Some(*raw),
			"top_p" => reading.settings().top_p = Some(*raw),
			"stream" => reading.stream("stream", *raw),
			"reasoning_effort" => {
				if let Some(effort) = text_of(raw) {
					reading.effort(&effort);
				}
			},
			"stream_options" => {
				let include_usage = raw
					.fields()
					.ok()
					.and_then(|options| field(&options, "include_usage"))
					.and_then(|raw| raw.read().ok());
				reading.settings().include_usage = include_usage.unwrap_or(false);
			},
			_ => {},
		}
	}
	reading.settings().max_tokens = limit;
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
