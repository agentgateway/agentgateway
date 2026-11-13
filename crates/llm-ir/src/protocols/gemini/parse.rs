//! A Gemini `generateContent` request from a client. The model and whether to stream
//! come from the URL, so the caller passes them; a `model` in the body is dropped.
//! Field names may be in snake case, as Google's API reads both.
//!
//! Reads turns, parts and text in one pass when possible. Otherwise, reads fields
//! separately to report errors; failure of the single-pass read is not a refusal.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use super::{Signature, read_signature};
use crate::carrier::{self, StateScope};
use crate::ir::conversation::{
	Block, BlockId, Carrier, Content, Conversation, DataImage, Effort, Message, OutputFormat, Role,
	Settings, Text, Tool, ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::input::RequestInput;
use crate::json::{Fields, Json, JsonRef, Name, is_strict_object, text_of};
use crate::prompt::diagnostics::{Diagnostics, TokenLimit};
use crate::prompt::passthrough::Reading;
use crate::protocols::{Read, is_image, read_fields_unless};

const EMPTY_SCHEMA: &str = r#"{"type":"object","properties":{}}"#;

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
		diagnostics: Diagnostics::default(),
		open_calls: Vec::new(),
		conversation: Conversation {
			settings: Settings::default(),
			system: None,
			tools: Vec::new(),
			messages: Vec::new(),
			cache_breakpoints: Vec::new(),
		},
	};
	let has_limit = parser.request(body, one_pass);
	let settings = &mut parser.conversation.settings;
	settings.max_tokens = settings.max_tokens.or(default_max_tokens);
	if needs_limit && settings.max_tokens.is_none() && !has_limit {
		parser
			.diagnostics
			.problem("no `generationConfig.maxOutputTokens`, and no default for this model");
	}
	Parsed {
		conversation: parser.conversation,
		diagnostics: parser.diagnostics,
	}
}

struct Parser<'a, 'c> {
	scope: StateScope<'c>,
	diagnostics: Diagnostics,
	open_calls: Vec<OpenCall>,
	conversation: Conversation<'a>,
}

/// Gemini's calls and results may have no ID. Such a result belongs to the oldest open
/// call of its name.
struct OpenCall {
	id: String,
	name: String,
	given_id: bool,
}

/// The output format, read from fields that may come in any order.
#[derive(Default)]
struct ReplyFormat<'a> {
	mime_type: Option<String>,
	schema: Option<Json<'a>>,
}

impl<'a> Parser<'a, '_> {
	/// Whether the client set an output limit, even one that was refused.
	fn request(&mut self, body: RequestInput<'a>, one_pass: bool) -> bool {
		let (fields, mut contents) = match one_pass.then(|| body.read_raw::<BodyRead<'a>>()).flatten() {
			Some(BodyRead { fields, contents }) => (fields, contents.map(Read::Done)),
			None => {
				let fields = body.fields();
				let Some(fields) = fields else {
					self.diagnostics.problem("body is not a JSON object");
					return true;
				};
				(fields, None)
			},
		};
		let Some(fields) = self.named("request", fields) else {
			return true;
		};
		let (mut tool_config, mut has_limit) = (None, false);
		for (name, raw) in &fields.0 {
			match name.as_ref() {
				"contents" => contents = Some(Read::Raw(*raw)),
				"systemInstruction" => self.system_instruction(*raw),
				"tools" => self.tools(*raw),
				"toolConfig" => tool_config = Some(*raw),
				"generationConfig" => has_limit = self.generation_config(*raw),
				"safetySettings" => self.diagnostics.drop_field("safetySettings"),
				"labels" => self.diagnostics.drop_field("labels"),
				"model" => {
					self.diagnostics.typed::<String>("model", raw);
					self.diagnostics.drop_field("model in the body");
				},
				other => self.diagnostics.problem(format!("field `{other}`")),
			}
		}
		if let Some(raw) = tool_config {
			self.tool_config(raw);
		}
		match contents {
			Some(contents) => self.contents(contents),
			None => self.diagnostics.problem("missing `contents`"),
		}
		has_limit
	}

	/// Snake-case names become camel case, and a name given both ways is a repeat.
	fn named(&mut self, path: impl fmt::Display, mut fields: Fields<'a>) -> Option<Fields<'a>> {
		for (name, _) in &mut fields.0 {
			if let Cow::Owned(camel) = camel_case(name) {
				*name = Cow::Owned(camel);
			}
		}
		(!self.diagnostics.reject_duplicates(path, &fields)).then_some(fields)
	}

	fn object(&mut self, path: impl fmt::Display, raw: JsonRef<'a>) -> Option<Fields<'a>> {
		let Ok(fields) = raw.fields() else {
			self
				.diagnostics
				.problem(format!("`{path}` must be an object"));
			return None;
		};
		self.named(path, fields)
	}

	fn system_instruction(&mut self, raw: JsonRef<'a>) {
		let Some(fields) = self.object("systemInstruction", raw) else {
			return;
		};
		let mut texts = Vec::new();
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"role" => {
					self
						.diagnostics
						.typed::<String>("systemInstruction.role", value);
				},
				"parts" => {
					let Some(parts) = self.diagnostics.list("systemInstruction.parts", value) else {
						continue;
					};
					for (at, part) in parts.into_iter().enumerate() {
						let path = format!("systemInstruction.parts[{at}]");
						texts.extend(self.system_part(&path, part));
					}
				},
				other => self
					.diagnostics
					.problem(format!("field `systemInstruction.{other}`")),
			}
		}
		self.conversation.system = Some(texts.into_iter().map(Text::from).collect());
	}

	fn system_part(&mut self, path: &str, raw: JsonRef<'a>) -> Option<Cow<'a, str>> {
		let fields = self.object(path, raw)?;
		let mut text = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"text" => text = self.diagnostics.text(format_args!("{path}.text"), value),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		if text.is_none() {
			self.diagnostics.problem(format!("`{path}` has no text"));
		}
		text
	}

	fn tools(&mut self, raw: JsonRef<'a>) {
		let Some(tools) = self.diagnostics.list("tools", raw) else {
			return;
		};
		for (at, tool) in tools.into_iter().enumerate() {
			let path = format!("tools[{at}]");
			let Some(fields) = self.object(&path, tool) else {
				continue;
			};
			for (name, value) in &fields.0 {
				match name.as_ref() {
					"functionDeclarations" => {
						let path = format!("{path}.functionDeclarations");
						let Some(declarations) = self.diagnostics.list(&path, value) else {
							continue;
						};
						for (at, declaration) in declarations.into_iter().enumerate() {
							self.function_declaration(&format!("{path}[{at}]"), declaration);
						}
					},
					// Google's own tools, such as `googleSearch` and `codeExecution`.
					other => self.diagnostics.problem(format!("`{path}.{other}` tool")),
				}
			}
		}
	}

	fn function_declaration(&mut self, path: &str, raw: JsonRef<'a>) {
		let Some(fields) = self.object(path, raw) else {
			return;
		};
		let (mut name, mut description, mut schema) = (None, None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
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
				"parameters" if schema.is_none() => {
					schema = self
						.schema(&format!("{path}.parameters"), *value)
						.map(Json::from);
				},
				"parametersJsonSchema" if schema.is_none() => {
					if is_strict_object(value) {
						schema = Some(Json::from(*value));
					} else {
						self.diagnostics.problem(format!(
							"`{path}.parametersJsonSchema` must be a valid object"
						));
					}
				},
				"parameters" | "parametersJsonSchema" => self.diagnostics.problem(format!(
					"`{path}` has both `parameters` and `parametersJsonSchema`"
				)),
				"response" | "responseJsonSchema" => {
					self.diagnostics.drop_field("function response schema");
				},
				"behavior" => {
					let behavior = self
						.diagnostics
						.typed::<String>(&format!("{path}.behavior"), value);
					if !matches!(behavior.as_deref(), None | Some("BLOCKING" | "UNSPECIFIED")) {
						self.diagnostics.problem(format!(
							"`{path}.behavior` `{}`",
							behavior.unwrap_or_default()
						));
					}
				},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let Some(name) = name else {
			self.diagnostics.problem(format!("`{path}` has no name"));
			return;
		};
		let schema = match schema {
			Some(schema) => schema,
			None => {
				let Ok(empty) = RawValue::from_string(EMPTY_SCHEMA.to_string()) else {
					return;
				};
				Json::from(empty)
			},
		};
		self.conversation.tools.push(Tool {
			path: path.to_string(),
			name,
			namespace: None,
			description,
			schema,
			kind: ToolKind::Function { strict: None },
		});
	}

	fn tool_config(&mut self, raw: JsonRef<'a>) {
		let Some(fields) = self.object("toolConfig", raw) else {
			return;
		};
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"functionCallingConfig" => self.function_calling(*value),
				other => self
					.diagnostics
					.problem(format!("field `toolConfig.{other}`")),
			}
		}
	}

	fn function_calling(&mut self, raw: JsonRef<'a>) {
		const PATH: &str = "toolConfig.functionCallingConfig";
		let Some(fields) = self.object(PATH, raw) else {
			return;
		};
		let (mut mode, mut names) = (None, None);
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"mode" => {
					mode = self
						.diagnostics
						.typed::<String>(&format!("{PATH}.mode"), value)
				},
				"allowedFunctionNames" => {
					names = self
						.diagnostics
						.typed::<Vec<String>>(&format!("{PATH}.allowedFunctionNames"), value);
				},
				other => self.diagnostics.problem(format!("field `{PATH}.{other}`")),
			}
		}
		let mode = mode.map(|mode| mode.to_ascii_uppercase());
		let names = names.unwrap_or_default();
		let choice = match (mode.as_deref(), names.as_slice()) {
			(None | Some("AUTO" | "MODE_UNSPECIFIED"), []) => ToolChoice::Auto,
			(Some("NONE"), []) => ToolChoice::None,
			(Some("ANY"), []) => ToolChoice::Required,
			(Some("ANY"), [name]) => ToolChoice::Tool(name.clone()),
			(Some("ANY"), _) => {
				self.diagnostics.problem(format!(
					"`{PATH}.allowedFunctionNames` with more than one name"
				));
				return;
			},
			(Some("AUTO" | "MODE_UNSPECIFIED" | "NONE") | None, _) => {
				self
					.diagnostics
					.problem(format!("`{PATH}.allowedFunctionNames` without mode `ANY`"));
				return;
			},
			(Some(other), _) => {
				self.diagnostics.problem(format!("`{PATH}.mode` `{other}`"));
				return;
			},
		};
		self.conversation.settings.tool_choice = Some(choice);
	}

	/// Whether the client set an output limit, even one that was refused.
	fn generation_config(&mut self, raw: JsonRef<'a>) -> bool {
		const PATH: &str = "generationConfig";
		let Some(fields) = self.object(PATH, raw) else {
			return false;
		};
		let mut has_limit = false;
		let mut format = ReplyFormat::default();
		for (name, value) in &fields.0 {
			let path = format!("{PATH}.{name}");
			let settings = &mut self.conversation.settings;
			match name.as_ref() {
				"temperature" => settings.temperature = Some(*value),
				"topP" => settings.top_p = Some(*value),
				"topK" => settings.top_k = Some(*value),
				"maxOutputTokens" => {
					has_limit = true;
					if let TokenLimit::Positive(limit) = self.diagnostics.token_limit(&path, value) {
						self.conversation.settings.max_tokens = Some(limit);
					}
				},
				"stopSequences" => {
					if let Some(stops) = self.diagnostics.typed::<Vec<String>>(&path, value) {
						self.conversation.settings.stop_sequences = stops;
					}
				},
				"candidateCount" => match self.diagnostics.typed::<u64>(&path, value) {
					Some(1) | None => {},
					Some(_) => self.diagnostics.problem(format!("`{path}` other than 1")),
				},
				"responseModalities" => {
					let modalities = self.diagnostics.typed::<Vec<String>>(&path, value);
					if !modalities.is_none_or(|modalities| {
						modalities
							.iter()
							.all(|modality| modality.eq_ignore_ascii_case("text"))
					}) {
						self
							.diagnostics
							.problem(format!("`{path}` other than text"));
					}
				},
				"responseMimeType" => format.mime_type = self.diagnostics.typed(&path, value),
				"responseSchema" | "responseJsonSchema" if format.schema.is_some() => {
					self.diagnostics.problem(format!(
						"`{PATH}` has both `responseSchema` and `responseJsonSchema`"
					));
				},
				"responseSchema" => format.schema = self.schema(&path, *value).map(Json::from),
				"responseJsonSchema" => {
					if is_strict_object(value) {
						format.schema = Some(Json::from(*value));
					} else {
						self
							.diagnostics
							.problem(format!("`{path}` must be a valid object"));
					}
				},
				"thinkingConfig" => self.thinking(*value),
				"mediaResolution" => self
					.diagnostics
					.drop_field("generationConfig.mediaResolution"),
				other => self.diagnostics.problem(format!("field `{PATH}.{other}`")),
			}
		}
		self.reply_format(format);
		has_limit
	}

	fn reply_format(&mut self, format: ReplyFormat<'a>) {
		let output_format = match (format.mime_type.as_deref(), format.schema) {
			(None | Some("text/plain"), None) => None,
			(Some("application/json"), None) => Some(OutputFormat::Json),
			(Some("application/json"), Some(schema)) => Some(OutputFormat::JsonSchema(schema)),
			(None | Some("text/plain"), Some(_)) => {
				self
					.diagnostics
					.problem("a response schema needs `responseMimeType` `application/json`");
				None
			},
			(Some(other), _) => {
				self
					.diagnostics
					.problem(format!("`generationConfig.responseMimeType` `{other}`"));
				None
			},
		};
		self.conversation.settings.output_format = output_format;
	}

	fn thinking(&mut self, raw: JsonRef<'a>) {
		const PATH: &str = "generationConfig.thinkingConfig";
		let Some(fields) = self.object(PATH, raw) else {
			return;
		};
		let (mut level, mut budget) = (None, None);
		for (name, value) in &fields.0 {
			let path = format!("{PATH}.{name}");
			match name.as_ref() {
				"includeThoughts" => {
					self.conversation.settings.show_reasoning_summary =
						self.diagnostics.typed::<bool>(&path, value) == Some(true);
				},
				"thinkingLevel" => level = self.diagnostics.typed::<String>(&path, value),
				"thinkingBudget" => budget = self.diagnostics.typed::<i64>(&path, value),
				other => self.diagnostics.problem(format!("field `{PATH}.{other}`")),
			}
		}
		let effort = match (level, budget) {
			(Some(_), Some(_)) => {
				self
					.diagnostics
					.problem(format!("`{PATH}` has both a level and a budget"));
				return;
			},
			(Some(level), None) => match level.to_ascii_uppercase().as_str() {
				"MINIMAL" => Effort::Minimal,
				"LOW" => Effort::Low,
				"MEDIUM" => Effort::Medium,
				"HIGH" => Effort::High,
				"THINKING_LEVEL_UNSPECIFIED" => return,
				_ => {
					self
						.diagnostics
						.problem(format!("`{PATH}.thinkingLevel` `{level}`"));
					return;
				},
			},
			(None, Some(0)) => Effort::Off,
			// Budgets are read back as the effort Gemini 2 gets for them.
			(None, Some(budget)) => {
				self.diagnostics.drop_field("thinkingConfig.thinkingBudget");
				match budget {
					-1 => Effort::Medium,
					1..=1024 => Effort::Low,
					1025..=2048 => Effort::Medium,
					2049..=4096 => Effort::High,
					4097.. => Effort::XHigh,
					_ => {
						self
							.diagnostics
							.problem(format!("`{PATH}.thinkingBudget` `{budget}`"));
						return;
					},
				}
			},
			(None, None) => return,
		};
		self.conversation.settings.effort = Some(effort);
	}

	fn contents(&mut self, contents: Read<'a, List<'a, TurnRead<'a>>>) {
		let turns = match contents {
			Read::Done(List(turns)) => turns,
			Read::Raw(raw) => match self.diagnostics.list("contents", raw) {
				Some(turns) => turns.into_iter().map(Read::Raw).collect(),
				None => return,
			},
		};
		for (at, content) in turns.into_iter().enumerate() {
			let path = Place {
				turn: at,
				part: None,
			};
			let (fields, mut parts) = match content {
				Read::Done(TurnRead { fields, parts }) => {
					(self.named(path, fields), parts.map(|List(parts)| parts))
				},
				Read::Raw(raw) => (self.object(path, raw), None),
			};
			let Some(fields) = fields else {
				continue;
			};
			let mut role = Role::User;
			for (name, value) in &fields.0 {
				match name.as_ref() {
					"role" => match value.text().as_deref() {
						Some("user") => role = Role::User,
						Some("model") => role = Role::Assistant,
						_ => match self
							.diagnostics
							.typed::<String>(&format!("{path}.role"), value)
							.as_deref()
						{
							Some("user") | None => role = Role::User,
							Some("model") => role = Role::Assistant,
							Some(other) => self.diagnostics.problem(format!("`{path}` role `{other}`")),
						},
					},
					"parts" => {
						parts = self
							.diagnostics
							.list(format_args!("{path}.parts"), value)
							.map(|parts| parts.into_iter().map(Read::Raw).collect());
					},
					other => self.diagnostics.problem(format!("field `{path}.{other}`")),
				}
			}
			let Some(parts) = parts else {
				self.diagnostics.problem(format!("`{path}` has no parts"));
				continue;
			};
			let mut blocks = Vec::new();
			for (part_at, part) in parts.into_iter().enumerate() {
				let path = Place {
					turn: at,
					part: Some(part_at),
				};
				let id = BlockId {
					message: at,
					block: part_at,
				};
				self.part(path, id, role, part, &mut blocks);
			}
			if !blocks.is_empty() {
				self.conversation.messages.push(turn(at, role, blocks));
			}
		}
	}

	/// Replay state goes just before the part that contained it.
	fn part(
		&mut self,
		path: Place,
		id: BlockId,
		role: Role,
		part: Read<'a, PartRead<'a>>,
		blocks: &mut Vec<Content<'a>>,
	) {
		let (fields, text, function_response) = match part {
			Read::Done(PartRead {
				fields,
				text,
				function_response,
			}) => (self.named(path, fields), text, function_response),
			Read::Raw(raw) => (self.object(path, raw), None, None),
		};
		let Some(fields) = fields else {
			return;
		};
		// Data read with the part has no other data beside it.
		let mut data = match (text, function_response) {
			(Some(text), _) => Some(("text", PartData::Text(text))),
			(None, Some(read)) => Some(("functionResponse", PartData::FunctionResponse(read))),
			(None, None) => None,
		};
		let (mut thought, mut signature) = (false, None);
		let mut refused_data = false;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"text" | "inlineData" | "functionCall" | "functionResponse" if data.is_none() => {
					data = Some((name.as_ref(), PartData::Raw(*value)));
				},
				"text" | "inlineData" | "functionCall" | "functionResponse" => {
					self
						.diagnostics
						.problem(format!("`{path}` has more than one kind of data"));
					return;
				},
				"thought" => {
					thought = self
						.diagnostics
						.typed::<bool>(&format!("{path}.thought"), value)
						== Some(true);
				},
				"thoughtSignature" => {
					signature = self
						.diagnostics
						.typed::<String>(&format!("{path}.thoughtSignature"), value);
				},
				"fileData" => {
					refused_data = true;
					self.diagnostics.problem(format!("`{path}` file reference"));
				},
				"executableCode" | "codeExecutionResult" => {
					refused_data = true;
					self.diagnostics.problem(format!("`{path}` code execution"));
				},
				"mediaResolution" => self.diagnostics.drop_field("part mediaResolution"),
				// Likely a kind of data this core doesn't know.
				other => {
					refused_data = true;
					self.diagnostics.problem(format!("field `{path}.{other}`"));
				},
			}
		}
		if role == Role::User && (thought || signature.is_some()) {
			self
				.diagnostics
				.problem(format!("`{path}` is a thought in a user turn"));
			return;
		}
		if let Some(signature) = signature {
			self.signature(path, &signature, blocks);
		}
		let Some((kind, value)) = data else {
			if !refused_data {
				self.diagnostics.problem(format!("`{path}` has no data"));
			}
			return;
		};
		let value = match value {
			PartData::Text(text) => {
				if thought {
					self.diagnostics.drop_field("thought text");
					return;
				}
				blocks.extend((!text.is_empty()).then(|| Content::Text(text.into())));
				return;
			},
			value => value,
		};
		let content = match (kind, role) {
			("text", _) if thought => {
				self
					.diagnostics
					.text(format_args!("{path}.text"), value.raw());
				self.diagnostics.drop_field("thought text");
				None
			},
			(_, _) if thought => {
				self
					.diagnostics
					.problem(format!("`{path}` is a thought with no text"));
				None
			},
			("text", _) => self
				.diagnostics
				.text(format_args!("{path}.text"), value.raw())
				.filter(|text| !text.is_empty())
				.map(|text| Content::Text(text.into())),
			("inlineData", Role::User) => self.image(&format!("{path}.inlineData"), value.raw()),
			("functionCall", Role::Assistant) => {
				self.function_call(&format!("{path}.functionCall"), id, value.raw())
			},
			("functionResponse", Role::User) => {
				let read = match value {
					PartData::FunctionResponse(read) => Read::Done(read),
					value => Read::Raw(value.raw()),
				};
				self.function_response(&format!("{path}.functionResponse"), read)
			},
			(kind, _) => {
				let turn = if role == Role::User { "user" } else { "model" };
				self
					.diagnostics
					.problem(format!("`{path}.{kind}` doesn't belong in a {turn} turn"));
				None
			},
		};
		blocks.extend(content);
	}

	fn signature(&mut self, path: Place, text: &str, blocks: &mut Vec<Content<'a>>) {
		match read_signature(text) {
			Signature::Carriers(carriers) => {
				for text in carriers {
					let carrier = match carrier::open(
						self.scope,
						carrier::CarrierPurpose::GeminiThoughtSignature,
						&text,
					) {
						Ok(item) => Carrier::decoded(item),
						Err(error) => Carrier::invalid(error),
					};
					blocks.push(Content::Reasoning {
						carrier,
						next: None,
					});
				}
			},
			Signature::Placeholder => self.diagnostics.drop_field("placeholder thought signature"),
			Signature::Foreign => self.diagnostics.problem(format!(
				"`{path}.thoughtSignature` was not written by this gateway"
			)),
		}
	}

	fn image(&mut self, path: &str, raw: JsonRef<'a>) -> Option<Content<'a>> {
		self.data_image(path, raw).map(Content::DataImage)
	}

	fn data_image(&mut self, path: &str, raw: JsonRef<'a>) -> Option<DataImage<'a>> {
		let fields = self.object(path, raw)?;
		let (mut media_type, mut data) = (None, None);
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"mimeType" => {
					media_type = self
						.diagnostics
						.typed::<String>(&format!("{path}.mimeType"), value)
				},
				"data" => data = self.diagnostics.text(format_args!("{path}.data"), value),
				"displayName" => self.diagnostics.drop_field("inlineData.displayName"),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		match (media_type, data) {
			(Some(media_type), Some(data)) if is_image(&media_type, &data) => {
				Some(DataImage { media_type, data })
			},
			_ => {
				self
					.diagnostics
					.problem(format!("`{path}` is not a supported base64 image"));
				None
			},
		}
	}

	fn function_call(&mut self, path: &str, at: BlockId, raw: JsonRef<'a>) -> Option<Content<'a>> {
		let fields = self.object(path, raw)?;
		let (mut id, mut name, mut arguments) = (None, None, None);
		for (field, value) in &fields.0 {
			match field.as_ref() {
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
				"args" => arguments = Some(*value),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let Some(name) = name else {
			self.diagnostics.problem(format!("`{path}` has no name"));
			return None;
		};
		let input = match arguments {
			Some(arguments) if is_strict_object(arguments) => Json::from(arguments),
			Some(_) => {
				self
					.diagnostics
					.problem(format!("`{path}.args` must be a valid object"));
				return None;
			},
			None => RawValue::from_string("{}".to_string()).ok()?.into(),
		};
		let given_id = id.is_some();
		let id = id.unwrap_or_else(|| format!("gemini_call_{}_{}", at.message, at.block));
		self.open_calls.push(OpenCall {
			id: id.clone(),
			name: name.clone(),
			given_id,
		});
		Some(Content::ToolCall {
			id,
			name,
			namespace: None,
			input,
		})
	}

	fn function_response(
		&mut self,
		path: &str,
		read: Read<'a, FunctionResponseRead<'a>>,
	) -> Option<Content<'a>> {
		let (fields, mut response) = match read {
			Read::Done(FunctionResponseRead { fields, response }) => {
				(self.named(path, fields)?, response)
			},
			Read::Raw(raw) => (self.object(path, raw)?, None),
		};
		let (mut id, mut name, mut parts) = (None, None, Vec::new());
		for (field, value) in &fields.0 {
			match field.as_ref() {
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
				"response" => response = Some(*value),
				"parts" => {
					let Some(list) = self.diagnostics.list(format_args!("{path}.parts"), value) else {
						continue;
					};
					for (at, part) in list.into_iter().enumerate() {
						parts.extend(self.response_part(&format!("{path}.parts[{at}]"), part));
					}
				},
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		let Some(name) = name else {
			self.diagnostics.problem(format!("`{path}` has no name"));
			return None;
		};
		let Some(call_id) = self.answer(id.as_deref(), &name) else {
			self
				.diagnostics
				.problem(format!("`{path}` answers no open function call"));
			return None;
		};
		let (failed, text) = match response {
			Some(response) => self.response(&format!("{path}.response"), response)?,
			None => (None, Cow::Borrowed("")),
		};
		let output = if parts.is_empty() {
			ToolOutput::Text(text.into())
		} else {
			let mut all = vec![ToolPart::Text(text.into())];
			all.extend(parts);
			ToolOutput::Mixed(all)
		};
		Some(Content::ToolResult {
			call_id,
			failed,
			output,
		})
	}

	/// The open call a response belongs to: the one with its ID, or else the oldest call
	/// of its name that had no ID of its own.
	fn answer(&mut self, id: Option<&str>, name: &str) -> Option<String> {
		let by_id = id.and_then(|id| self.open_calls.iter().position(|call| call.id == id));
		let at = by_id.or_else(|| {
			self
				.open_calls
				.iter()
				.position(|call| call.name == name && (id.is_none() || !call.given_id))
		})?;
		Some(self.open_calls.remove(at).id)
	}

	/// Gemini CLI replies with `{"output": text}` or `{"error": text}`; Google's
	/// examples use `content`. Any other response goes on as its JSON text.
	fn response(&mut self, path: &str, raw: JsonRef<'a>) -> Option<(Option<bool>, Cow<'a, str>)> {
		// The usual single text is decoded as it is read; anything else is read field by field.
		if let JsonRef::Raw(raw) = raw
			&& let Ok(ResponseText { failed, text }) = serde_json::from_str(raw.get())
		{
			return Some((failed, text));
		}
		let Ok(fields) = raw.fields() else {
			self
				.diagnostics
				.problem(format!("`{path}` must be an object"));
			return None;
		};
		if let [(name, value)] = fields.0.as_slice() {
			let text = text_of(value).unwrap_or_else(|| value.encoded());
			match name.as_ref() {
				"output" | "content" if text_of(value).is_some() => return Some((None, text)),
				"error" => return Some((Some(true), text)),
				_ => {},
			}
		}
		Some((None, raw.encoded()))
	}

	fn response_part(&mut self, path: &str, raw: JsonRef<'a>) -> Option<ToolPart<'a>> {
		let fields = self.object(path, raw)?;
		let mut image = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"inlineData" => image = self.data_image(&format!("{path}.inlineData"), *value),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		image.map(ToolPart::DataImage)
	}

	/// Gemini's schema is OpenAPI's subset of JSON Schema, with upper-case types and
	/// `nullable`. It is rewritten as JSON Schema.
	fn schema(&mut self, path: &str, raw: JsonRef<'a>) -> Option<Box<RawValue>> {
		let value = self.schema_value(path, raw)?;
		serde_json::value::to_raw_value(&value).ok()
	}

	fn schema_value(&mut self, path: &str, raw: JsonRef<'a>) -> Option<Value> {
		let fields = self.object(path, raw)?;
		let mut out = Map::new();
		let mut nullable = false;
		for (name, value) in &fields.0 {
			let at = format!("{path}.{name}");
			match name.as_ref() {
				"type" => {
					let kind = self.diagnostics.typed::<String>(&at, value)?;
					let kind = kind.to_ascii_lowercase();
					if !matches!(
						kind.as_str(),
						"string" | "number" | "integer" | "boolean" | "array" | "object" | "null"
					) {
						self.diagnostics.problem(format!("`{at}` `{kind}`"));
						return None;
					}
					out.insert(name.to_string(), Value::String(kind));
				},
				"nullable" => nullable = self.diagnostics.typed::<bool>(&at, value)?,
				"format" | "title" | "description" | "pattern" | "enum" | "required" | "default"
				| "minimum" | "maximum" => {
					out.insert(
						name.to_string(),
						self.diagnostics.typed::<Value>(&at, value)?,
					);
				},
				"minItems" | "maxItems" | "minLength" | "maxLength" | "minProperties" | "maxProperties" => {
					// Protobuf's JSON writes 64-bit numbers as strings.
					let count = match self.diagnostics.typed::<Value>(&at, value)? {
						Value::String(text) => text.parse::<u64>().ok(),
						number => number.as_u64(),
					};
					let Some(count) = count else {
						self.diagnostics.problem(format!("`{at}` must be a count"));
						return None;
					};
					out.insert(name.to_string(), Value::from(count));
				},
				"properties" => {
					// Property names are the client's own, so they keep their spelling.
					let properties = self.diagnostics.object(&at, value)?;
					let mut schemas = Map::new();
					for (property, schema) in &properties.0 {
						let schema = self.schema_value(&format!("{at}.{property}"), *schema)?;
						schemas.insert(property.to_string(), schema);
					}
					out.insert(name.to_string(), Value::Object(schemas));
				},
				"items" => {
					out.insert(name.to_string(), self.schema_value(&at, *value)?);
				},
				"anyOf" => {
					let list = self.diagnostics.list(&at, value)?;
					let mut schemas = Vec::with_capacity(list.len());
					for (index, schema) in list.into_iter().enumerate() {
						schemas.push(self.schema_value(&format!("{at}[{index}]"), schema)?);
					}
					out.insert(name.to_string(), Value::Array(schemas));
				},
				"propertyOrdering" => self.diagnostics.drop_field("schema propertyOrdering"),
				"example" => self.diagnostics.drop_field("schema example"),
				other => {
					self
						.diagnostics
						.problem(format!("`{path}` schema keyword `{other}`"));
					return None;
				},
			}
		}
		if nullable && let Some(kind) = out.remove("type") {
			out.insert("type".into(), Value::Array(vec![kind, "null".into()]));
		}
		Some(Value::Object(out))
	}
}

/// Function results go first in their turn, as some targets require, and replay
/// state is linked to the block that follows it.
fn turn<'a>(at: usize, role: Role, contents: Vec<Content<'a>>) -> Message<'a> {
	let mut blocks: Vec<Block<'a>> = contents
		.into_iter()
		.enumerate()
		.map(|(block, content)| Block {
			id: BlockId { message: at, block },
			content,
		})
		.collect();
	blocks.sort_by_key(|block| !matches!(block.content, Content::ToolResult { .. }));
	for index in 0..blocks.len() {
		if !matches!(blocks[index].content, Content::Reasoning { .. }) {
			continue;
		}
		let target = blocks[index + 1..]
			.iter()
			.find(|block| !matches!(block.content, Content::Reasoning { .. }))
			.map(|block| block.id);
		if let Content::Reasoning { next, .. } = &mut blocks[index].content {
			*next = target;
		}
	}
	Message {
		index: at,
		role,
		blocks,
		messages_output_config: None,
	}
}

/// Where a turn or one of its parts is, written out only when a problem names it.
#[derive(Clone, Copy)]
struct Place {
	turn: usize,
	part: Option<usize>,
}

impl fmt::Display for Place {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "contents[{}]", self.turn)?;
		match self.part {
			Some(part) => write!(f, ".parts[{part}]"),
			None => Ok(()),
		}
	}
}

pub(super) struct BodyRead<'a> {
	pub(super) fields: Fields<'a>,
	pub(super) contents: Option<List<'a, TurnRead<'a>>>,
}

pub(super) struct TurnRead<'a> {
	pub(super) fields: Fields<'a>,
	pub(super) parts: Option<List<'a, PartRead<'a>>>,
}

/// A part, with its text or function response read if it has one.
pub(super) struct PartRead<'a> {
	pub(super) fields: Fields<'a>,
	pub(super) text: Option<Cow<'a, str>>,
	pub(super) function_response: Option<FunctionResponseRead<'a>>,
}

/// A function response read with its part, so a tool's output isn't scanned once to capture
/// it and again to read it. `response` is read for its text later, or sent on as it is.
pub(super) struct FunctionResponseRead<'a> {
	pub(super) fields: Fields<'a>,
	pub(super) response: Option<JsonRef<'a>>,
}

/// A part's data as far as the part read it.
enum PartData<'a> {
	Text(Cow<'a, str>),
	FunctionResponse(FunctionResponseRead<'a>),
	Raw(JsonRef<'a>),
}

impl<'a> PartData<'a> {
	fn raw(self) -> JsonRef<'a> {
		match self {
			Self::Raw(raw) => raw,
			Self::Text(_) | Self::FunctionResponse(_) => {
				unreachable!("text and function responses are read where the part is")
			},
		}
	}
}

/// A function response that is only `output`, `content` or `error` text, as Gemini CLI and
/// Google's examples send it.
struct ResponseText<'a> {
	failed: Option<bool>,
	text: Cow<'a, str>,
}

/// Items read in the same pass, so a list read level by level fits in its place.
pub(super) struct List<'a, T>(pub(super) Vec<Read<'a, T>>);

#[cfg(test)]
pub(super) fn read_body(text: &str) -> Option<BodyRead<'_>> {
	serde_json::from_str(text).ok()
}

/// One map's fields, with the value of `nested` read as `T`. `nested` in snake case, or
/// repeated in either case, is left to the level-by-level read to report.
fn read_fields<'de, M, T>(map: M, nested: &str) -> Result<(Fields<'de>, Option<T>), M::Error>
where
	M: MapAccess<'de>,
	T: Deserialize<'de>,
{
	read_fields_unless(map, nested, |name| camel_case(name) == nested)
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
				let (fields, contents) = read_fields(map, "contents")?;
				Ok(BodyRead { fields, contents })
			}
		}

		deserializer.deserialize_map(BodyVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for TurnRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct TurnVisitor;

		impl<'de> Visitor<'de> for TurnVisitor {
			type Value = TurnRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a turn object")
			}

			fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
				let (fields, parts) = read_fields(map, "parts")?;
				Ok(TurnRead { fields, parts })
			}
		}

		deserializer.deserialize_map(TurnVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for PartRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct PartVisitor;

		impl<'de> Visitor<'de> for PartVisitor {
			type Value = PartRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a part object")
			}

			fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
				let (mut fields, mut text, mut function_response) = (Vec::new(), None, None);
				while let Some(Name(name)) = map.next_key()? {
					match name.as_ref() {
						"text" if text.is_none() => text = Some(map.next_value::<Name<'de>>()?.0),
						"functionResponse" if function_response.is_none() => {
							function_response = Some(map.next_value::<FunctionResponseRead<'de>>()?);
						},
						"text" | "functionResponse" => return Err(M::Error::custom("repeated field")),
						_ if matches!(camel_case(&name).as_ref(), "text" | "functionResponse") => {
							return Err(M::Error::custom("nested field in snake case"));
						},
						_ => fields.push((name, map.next_value::<JsonRef<'de>>()?)),
					}
				}
				// Which kind of data comes first decides which problems are reported.
				let other_data = fields
					.iter()
					.any(|(name, _)| matches!(camel_case(name).as_ref(), "inlineData" | "functionCall"));
				let read = usize::from(text.is_some()) + usize::from(function_response.is_some());
				if read > 1 || (read == 1 && other_data) {
					return Err(M::Error::custom("more than one kind of data"));
				}
				Ok(PartRead {
					fields: Fields(fields),
					text,
					function_response,
				})
			}
		}

		deserializer.deserialize_map(PartVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for FunctionResponseRead<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct FunctionResponseVisitor;

		impl<'de> Visitor<'de> for FunctionResponseVisitor {
			type Value = FunctionResponseRead<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a function response object")
			}

			fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
				let (fields, response) = read_fields(map, "response")?;
				Ok(FunctionResponseRead { fields, response })
			}
		}

		deserializer.deserialize_map(FunctionResponseVisitor)
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for ResponseText<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct ResponseTextVisitor;

		impl<'de> Visitor<'de> for ResponseTextVisitor {
			type Value = ResponseText<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a response with one text")
			}

			fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
				let Some(Name(name)) = map.next_key()? else {
					return Err(M::Error::custom("no text"));
				};
				let failed = match name.as_ref() {
					"output" | "content" => None,
					"error" => Some(true),
					_ => return Err(M::Error::custom("not a text response")),
				};
				let Name(text) = map.next_value()?;
				if map.next_key::<Name<'de>>()?.is_some() {
					return Err(M::Error::custom("more than one field"));
				}
				Ok(ResponseText { failed, text })
			}
		}

		deserializer.deserialize_map(ResponseTextVisitor)
	}
}

impl<'de: 'a, 'a, T: Deserialize<'de>> Deserialize<'de> for List<'a, T> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct ListVisitor<T>(PhantomData<T>);

		impl<'de, T: Deserialize<'de>> Visitor<'de> for ListVisitor<T> {
			type Value = List<'de, T>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a list")
			}

			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
				let mut items = Vec::new();
				while let Some(item) = seq.next_element::<T>()? {
					items.push(Read::Done(item));
				}
				Ok(List(items))
			}
		}

		deserializer.deserialize_seq(ListVisitor(PhantomData))
	}
}

/// A Gemini request the provider gets unconverted, read for the settings the caller records. The
/// API takes each field name in camelCase or snake_case, so both are read.
pub fn read<'a>(fields: &Fields<'a>, reading: &mut Reading<'a>) {
	for (name, raw) in &fields.0 {
		if camel_case(name) == "generationConfig" {
			generation_config(*raw, reading);
		}
	}
}

fn generation_config<'a>(raw: JsonRef<'a>, reading: &mut Reading<'a>) {
	let Ok(config) = raw.fields() else {
		return;
	};
	for (name, raw) in &config.0 {
		match camel_case(name).as_ref() {
			"maxOutputTokens" => reading.settings().max_tokens = Reading::limit(*raw),
			"temperature" => reading.settings().temperature = Some(*raw),
			"topP" => reading.settings().top_p = Some(*raw),
			"topK" => reading.settings().top_k = Some(*raw),
			"thinkingConfig" => {
				let level = raw.fields().ok().and_then(|thinking| {
					thinking
						.0
						.iter()
						.find(|(name, _)| camel_case(name) == "thinkingLevel")
						.and_then(|(_, raw)| raw.read::<String>().ok())
				});
				if let Some(level) = level {
					reading.effort(&level.to_ascii_lowercase());
				}
			},
			_ => {},
		}
	}
}

/// A snake_case name in camelCase. A leading underscore is kept.
pub(super) fn camel_case(name: &str) -> Cow<'_, str> {
	if !name.contains('_') {
		return Cow::Borrowed(name);
	}
	let mut out = String::with_capacity(name.len());
	let mut upper = false;
	for char in name.chars() {
		if char == '_' && !out.is_empty() {
			upper = true;
		} else if upper {
			out.push(char.to_ascii_uppercase());
			upper = false;
		} else {
			out.push(char);
		}
	}
	Cow::Owned(out)
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
