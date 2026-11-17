//! The caller can pass the model's `ThinkingControls` from its catalog as
//! `RenderSettings::Gemini`. Without it, `ThinkingControls::by_model_name` guesses.

use std::borrow::Cow;
use std::collections::HashMap;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::ir::conversation::{
	Block, BlockId, Carrier, Content, Conversation, DataImage, Effort, ImageFields, Message,
	MessagesThinkingDisplay, MessagesThinkingMode, OutputEffort, OutputFormat, Role, Settings, Text,
	Tool, ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::{Fields, JsonRef, is_strict, text_of};
use crate::prompt::diagnostics::{Diagnostics, Unread};
use crate::prompt::{NOTHING_TO_SEND, TOOL_ERROR_MARKER};
use crate::protocols::gemini::schema::{SchemaRefs, SchemaWriter};
use crate::protocols::{
	CallKind, ToolRoute, check_sampling, check_tool_choice, custom_description, flat_name,
	output_format_schema, read_image, system_reminder, tool_declarations,
};

const MAX_TOOL_NAME: usize = 64;

#[cfg(test)]
pub(crate) fn render(
	conversation: &Conversation<'_>,
	unread: &Unread<'_>,
	diagnostics: Diagnostics,
) -> (String, Diagnostics) {
	let (body, _, diagnostics) = render_with_routes(
		conversation,
		unread,
		diagnostics,
		ThinkingControls::by_model_name(""),
	);
	(body, diagnostics)
}

pub(crate) fn render_with_routes(
	conversation: &Conversation<'_>,
	unread: &Unread<'_>,
	diagnostics: Diagnostics,
	thinking: ThinkingControls,
) -> (String, Vec<ToolRoute>, Diagnostics) {
	let mut renderer = Renderer {
		diagnostics,
		contents: Contents::default(),
		routes: Vec::new(),
		schema_refs: SchemaRefs::default(),
		calls: HashMap::new(),
		unanswered_calls: Vec::new(),
		leading_system: Vec::new(),
		reminders: Vec::new(),
	};
	for image in &unread.images {
		renderer.refuse_image(&image.path);
	}
	renderer.drop_settings(&conversation.settings);
	check_tool_choice(
		&conversation.settings,
		&conversation.tools,
		&mut renderer.diagnostics,
	);
	let tools = renderer.tools(&conversation.tools);
	let client_schema = conversation
		.settings
		.messages_input
		.output_config
		.and_then(|raw| renderer.client_format_schema(raw));
	// A format a stage set replaces the client's.
	let reply_format = match (&conversation.settings.output_format, client_schema) {
		(Some(OutputFormat::JsonSchema(schema)), _) => Some(ReplyFormat {
			schema: renderer.root_schema("text.format.schema", schema.as_ref()),
			unenforced: "text.format enforcement beside tools",
		}),
		(Some(OutputFormat::Json), _) => Some(ReplyFormat {
			schema: None,
			unenforced: "text.format enforcement beside tools",
		}),
		(None, Some(schema)) => Some(ReplyFormat {
			schema: renderer.root_schema("output_config.format.schema", schema),
			unenforced: "output_config.format enforcement beside tools",
		}),
		(None, None) => None,
	};
	renderer.messages(&conversation.messages);
	let mut system: Vec<&str> = conversation
		.system
		.iter()
		.flatten()
		.map(Text::as_str)
		.collect();
	system.append(&mut renderer.leading_system);
	let thinking = renderer.thinking(&conversation.settings, thinking);
	let mut reply = ReplySettings {
		json: false,
		schema: None,
		thinking,
	};
	let told_format;
	match &reply_format {
		// With tools, Gemini 2 refuses a JSON reply, and Gemini 3 keeps calling tools and
		// never gives a final reply. So the model is told the format instead, and nothing enforces it.
		Some(format) if tools.is_some() => {
			renderer.diagnostics.drop_field(format.unenforced);
			told_format = format_request(format.schema.as_deref());
			system.push(&told_format);
		},
		Some(format) => {
			reply.json = true;
			reply.schema = format.schema.as_deref();
		},
		None => {},
	}
	let body = renderer.write(&conversation.settings, &system, tools, &reply);
	(body, renderer.routes, renderer.diagnostics)
}

struct Renderer<'c> {
	diagnostics: Diagnostics,
	contents: Contents<'c>,
	routes: Vec<ToolRoute>,
	schema_refs: SchemaRefs,
	calls: HashMap<String, Call>,
	unanswered_calls: Vec<String>,
	leading_system: Vec<&'c str>,
	/// Later system messages, waiting for a place in the next user turn.
	reminders: Vec<&'c str>,
}

struct Call {
	/// Results are sent in call order.
	position: usize,
	name: String,
}

/// A carrier's Gemini part, read only to place it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedPart<'a> {
	#[serde(default)]
	text: Option<String>,
	#[serde(default)]
	thought: bool,
	#[serde(borrow)]
	thought_signature: Option<&'a RawValue>,
	#[serde(borrow)]
	function_call: Option<SignedCall<'a>>,
}

#[derive(Deserialize)]
struct SignedCall<'a> {
	#[serde(borrow)]
	name: Cow<'a, str>,
}

/// State a reasoning block leaves for the block it's linked to.
#[derive(Default)]
struct Pending<'c> {
	call: Option<PendingCall<'c>>,
	text: Option<PendingText<'c>>,
}

struct PendingCall<'c> {
	signature: &'c RawValue,
	name: String,
	for_block: Option<BlockId>,
}

struct PendingText<'c> {
	signed_part: &'c RawValue,
	visible_text: String,
	for_block: Option<BlockId>,
}

impl<'c> Renderer<'c> {
	/// Effort is always sent when the client asks: left alone, Gemini's thinking can
	/// use up a small limit before it replies, which is what a classifier call has.
	fn thinking(&mut self, settings: &Settings<'_>, controls: ThinkingControls) -> Thinking {
		let unread = settings
			.messages_input
			.output_config
			.is_some_and(output_config_has_unread);
		if settings.effort.is_none() && unread {
			self.diagnostics.drop_field("output_config");
		}
		let thinking = Thinking::for_controls(settings, controls);
		// A level, or Pro's smallest budget, can't turn thinking off.
		let still_thinks = match thinking.effort {
			ThinkingEffort::Level(_) => true,
			ThinkingEffort::Budget(budget) => budget > 0,
			ThinkingEffort::ProviderDefault => false,
		};
		if still_thinks && requested_effort(settings) == Some(Effort::Off) {
			self.diagnostics.drop_field(if settings.effort.is_some() {
				"reasoning.effort none"
			} else {
				"thinking.type disabled"
			});
		}
		thinking
	}

	/// Gemini has no place for these, and nothing in them changes what a reply means.
	fn drop_settings(&mut self, settings: &Settings<'_>) {
		if settings.parallel_tool_calls == Some(false) {
			self.diagnostics.problem("setting `parallel_tool_calls`");
		}
		check_sampling(settings, &mut self.diagnostics);
		if let Some(top_k) = settings.top_k {
			self.diagnostics.typed::<u64>("top_k", top_k);
		}
		let fields = [
			(
				settings.messages_input.context_management.is_some(),
				"context_management",
			),
			(settings.messages_input.metadata.is_some(), "metadata"),
			(
				settings
					.messages_input
					.thinking
					.as_ref()
					.is_some_and(|thinking| thinking.budget_tokens.is_some()),
				"thinking.budget_tokens",
			),
		];
		for (present, field) in fields {
			if present {
				self.diagnostics.drop_field(field);
			}
		}
	}

	fn write(
		&mut self,
		settings: &Settings<'_>,
		system: &[&str],
		tools: Option<Vec<wire::FunctionDeclaration<'_>>>,
		reply: &ReplySettings<'_>,
	) -> String {
		let tool_config = if tools.is_some() {
			tool_config(settings)
		} else {
			None
		};
		let request = wire::Request {
			contents: std::mem::take(&mut self.contents).entries,
			system_instruction: (!system.is_empty()).then(|| wire::SystemInstruction {
				parts: [wire::SystemText {
					text: joined(system),
				}],
			}),
			tools: tools.map(|function_declarations| {
				[wire::Tools {
					function_declarations,
				}]
			}),
			tool_config,
			generation_config: Self::generation_config(settings, reply),
		};
		serde_json::to_string(&request).unwrap_or_else(|error| {
			self
				.diagnostics
				.problem(format!("could not serialize Gemini request: {error}"));
			String::new()
		})
	}

	fn generation_config<'s>(
		settings: &'s Settings<'_>,
		reply: &ReplySettings<'s>,
	) -> wire::GenerationConfig<'s> {
		wire::GenerationConfig {
			max_output_tokens: settings.max_tokens,
			temperature: settings.temperature,
			top_p: settings.top_p,
			top_k: settings.top_k,
			stop_sequences: (!settings.stop_sequences.is_empty())
				.then_some(settings.stop_sequences.as_slice()),
			response_mime_type: reply.json.then_some(wire::MimeType::Json),
			response_schema: reply.schema.map(|schema| read_json(schema).into()),
			thinking_config: thinking_config(reply.thinking),
		}
	}

	fn tools(&mut self, tools: &'c [Tool<'_>]) -> Option<Vec<wire::FunctionDeclaration<'c>>> {
		if tools.is_empty() {
			return None;
		}
		let mut out = Vec::with_capacity(tools.len());
		for tool in tool_declarations(tools) {
			let Some(flat) = self.tool_name(tool) else {
				continue;
			};
			let description = match &tool.kind {
				ToolKind::Function { strict } => {
					if strict.is_some() {
						self.diagnostics.drop_field("tool strict");
					}
					tool.description.as_deref().map(Cow::Borrowed)
				},
				ToolKind::Custom { grammar } => Some(Cow::Owned(custom_description(
					tool,
					grammar.as_deref(),
					&mut self.diagnostics,
				))),
				ToolKind::ToolSearch => tool.description.as_deref().map(Cow::Borrowed),
			};
			let path = format!("{}.input_schema", tool.path);
			let Some(schema) = self.root_schema(&path, tool.schema.as_ref()) else {
				continue;
			};
			out.push(wire::FunctionDeclaration {
				name: flat,
				description,
				// A rewritten schema has no source to borrow from, so it is owned.
				parameters: match RawValue::from_string(schema) {
					Ok(schema) => schema,
					Err(error) => unreachable!("the schema writer writes JSON: {error}"),
				},
			});
		}
		Some(out)
	}

	fn tool_name(&mut self, tool: &Tool<'_>) -> Option<String> {
		let flat = flat_name(tool.namespace.as_deref(), &tool.name);
		let valid = !flat.is_empty()
			&& flat.len() <= MAX_TOOL_NAME
			&& flat
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte));
		if !valid {
			self
				.diagnostics
				.problem(format!("`{}` name `{flat}` doesn't fit Gemini", tool.path));
			return None;
		}
		if self.routes.iter().any(|route| route.flat == flat) {
			self
				.diagnostics
				.problem(format!("`{}` name `{flat}` collides", tool.path));
			return None;
		}
		self.routes.push(ToolRoute {
			flat: flat.clone(),
			namespace: tool.namespace.clone(),
			name: tool.name.clone(),
			kind: CallKind::from(&tool.kind),
		});
		Some(flat)
	}

	fn root_schema(&mut self, path: &str, raw: JsonRef<'_>) -> Option<String> {
		SchemaWriter {
			diagnostics: &mut self.diagnostics,
			schema_refs: &mut self.schema_refs,
		}
		.root_schema(path, raw)
	}

	fn client_format_schema<'s>(&mut self, output_config: JsonRef<'s>) -> Option<JsonRef<'s>> {
		let fields: Fields<'s> = output_config.fields().ok()?;
		let (_, format) = fields.0.into_iter().find(|(name, _)| name == "format")?;
		output_format_schema(format, &mut self.diagnostics)
	}

	fn messages(&mut self, messages: &'c [Message<'_>]) {
		for message in messages {
			if message.messages_output_config.is_some() {
				self.diagnostics.drop_field("output_config");
			}
			match message.role {
				Role::User => self.user_message(message),
				Role::Assistant => self.assistant_message(message),
				Role::System => self.system_message(message),
			}
		}
		self.push_reminders();
		if !self.unanswered_calls.is_empty() {
			let calls = self.unanswered_calls.join(", ");
			self
				.diagnostics
				.problem(format!("tool calls with no result: {calls}"));
		}
		if self.contents.is_empty() && self.diagnostics.problems.is_empty() {
			self.diagnostics.problem(NOTHING_TO_SEND);
		}
	}

	/// Only a leading message can be an instruction. Anything later would change
	/// `systemInstruction` every turn, and with it the prefix Gemini can reuse.
	fn system_message(&mut self, message: &'c Message<'_>) {
		for block in &message.blocks {
			let Content::Text(text) = &block.content else {
				continue;
			};
			if text.is_empty() {
				continue;
			}
			if self.contents.is_empty() {
				self.leading_system.push(text.as_str());
			} else {
				self.reminders.push(text.as_str());
			}
		}
	}

	fn push_reminders(&mut self) {
		for text in std::mem::take(&mut self.reminders) {
			self.contents.push(
				wire::Role::User,
				text_part(Cow::Owned(system_reminder(text))),
			);
		}
	}

	/// Results come first, in call order, in their own entry: Vertex matches them
	/// by position, and Gemini 3 refuses a result with anything beside it.
	fn user_message(&mut self, message: &'c Message<'_>) {
		let mut results = Vec::new();
		let mut parts = Vec::new();
		for block in &message.blocks {
			match &block.content {
				Content::Text(text) => parts.push(text_part(Cow::Borrowed(text.as_str()))),
				Content::ToolResult {
					call_id,
					failed,
					output,
				} => results.extend(self.tool_result(block.id, call_id, failed.unwrap_or(false), output)),
				Content::Image(fields) => {
					let image = self.image(&block.id.path(), fields);
					parts.extend(image.map(wire::Part::InlineData));
				},
				Content::DataImage(image) => parts.push(wire::Part::InlineData(data_image(image))),
				Content::ToolCall { .. } | Content::Reasoning { .. } => {},
			}
		}
		results.sort_by_key(|(position, _)| *position);
		self
			.contents
			.results(results.into_iter().map(|(_, part)| part).collect());
		self.push_reminders();
		for part in parts {
			self.contents.push(wire::Role::User, part);
		}
	}

	fn image(&mut self, path: &str, fields: &ImageFields<'_>) -> Option<wire::InlineData<'static>> {
		let image = read_image(path, fields, &mut self.diagnostics)?;
		Some(wire::InlineData {
			inline_data: wire::Blob {
				mime_type: Cow::Owned(image.media_type),
				data: image.data,
			},
		})
	}

	fn refuse_image(&mut self, path: &str) {
		self
			.diagnostics
			.problem(format!("`{path}` block type `image`"));
	}

	fn tool_result(
		&mut self,
		id: BlockId,
		call: &str,
		failed: bool,
		output: &'c ToolOutput<'_>,
	) -> Option<(usize, wire::Part<'c>)> {
		let path = id.path();
		let mut images = Vec::new();
		let mut texts = Vec::new();
		match output {
			ToolOutput::Text(text) => texts.push(text.as_str()),
			ToolOutput::Mixed(parts) => {
				for part in parts {
					match part {
						ToolPart::Image { index, fields } => {
							images.extend(self.image(&format!("{path}.content[{index}]"), fields));
						},
						ToolPart::DataImage(image) => images.push(data_image(image)),
						ToolPart::Text(text) => texts.push(text.as_str()),
					}
				}
			},
		}
		let Some(open) = self.unanswered_calls.iter().position(|id| id == call) else {
			self
				.diagnostics
				.problem(format!("`{path}` has no call waiting for it"));
			return None;
		};
		self.unanswered_calls.remove(open);
		let Call { position, name } = self.calls.get(call)?;
		let (position, name) = (*position, name.clone());
		let part = wire::Part::FunctionResponse {
			function_response: wire::FunctionResponse {
				name,
				response: tool_response(&texts, failed),
				parts: images,
			},
		};
		Some((position, part))
	}

	fn assistant_message(&mut self, message: &'c Message<'_>) {
		self.push_reminders();
		let mut pending = Pending::default();
		for block in &message.blocks {
			match &block.content {
				Content::Reasoning { carrier, next } => {
					self.reasoning(block.id, carrier, *next, &mut pending);
				},
				Content::Text(text) => self.assistant_text(block, text.as_str(), &mut pending),
				Content::ToolCall {
					id,
					name,
					namespace,
					input,
				} => {
					self.flush_text(&mut pending);
					let name = flat_name(namespace.as_deref(), name);
					self.tool_use(block.id, id, name, input.as_ref(), &mut pending);
				},
				Content::ToolResult { .. } | Content::Image(_) | Content::DataImage(_) => {},
			}
		}
		self.flush_text(&mut pending);
		if pending.call.is_some() {
			let path = format!("messages[{}].content", message.index);
			self
				.diagnostics
				.problem(format!("`{path}`: a call's signature has no call"));
		}
	}

	/// Text before a signed part keeps its place, and the signed part follows it.
	/// A changed answer can't keep a signature made for the original.
	fn assistant_text(&mut self, block: &Block<'_>, text: &'c str, pending: &mut Pending<'c>) {
		let Some(signed) = pending.text.take() else {
			self
				.contents
				.push(wire::Role::Model, text_part(Cow::Borrowed(text)));
			return;
		};
		let path = block.id.path();
		if signed.for_block != Some(block.id) {
			self
				.diagnostics
				.problem(format!("`{path}` doesn't match its signature"));
			return;
		}
		let Some(before) = text.strip_suffix(signed.visible_text.as_str()) else {
			self
				.diagnostics
				.problem(format!("`{path}` doesn't match its signature"));
			return;
		};
		if !before.is_empty() {
			self
				.contents
				.push(wire::Role::Model, text_part(Cow::Borrowed(before)));
		}
		self
			.contents
			.push(wire::Role::Model, wire::Part::Provider(signed.signed_part));
	}

	fn flush_text(&mut self, pending: &mut Pending<'c>) {
		if let Some(signed) = pending.text.take() {
			self
				.contents
				.push(wire::Role::Model, wire::Part::Provider(signed.signed_part));
		}
	}

	fn reasoning(
		&mut self,
		id: BlockId,
		carrier: &'c Carrier,
		next: Option<BlockId>,
		pending: &mut Pending<'c>,
	) {
		let path = id.path();
		let item = match carrier.opened() {
			Ok(item) => item,
			Err(error) => {
				self.diagnostics.problem(format!("`{path}`: {error}"));
				return;
			},
		};
		let item = std::str::from_utf8(item).ok();
		let part = item.and_then(|item| serde_json::from_str::<SignedPart<'_>>(item).ok());
		let (Some(item), Some(part)) = (item, part) else {
			self
				.diagnostics
				.problem(format!("`{path}`: carrier holds no provider part"));
			return;
		};
		self.flush_text(pending);
		let signature = part.thought_signature;
		if let Some(call) = part.function_call {
			let Some(signature) = signature else {
				self
					.diagnostics
					.problem(format!("`{path}`: a signed call has no signature"));
				return;
			};
			pending.call = Some(PendingCall {
				signature,
				name: call.name.into_owned(),
				for_block: next,
			});
		} else if part.thought {
			if signature.is_some() {
				let item = read_json(item);
				self
					.contents
					.push(wire::Role::Model, wire::Part::Provider(item));
			}
		} else {
			pending.text = Some(PendingText {
				signed_part: read_json(item),
				visible_text: part.text.unwrap_or_default(),
				for_block: next,
			});
		}
	}

	fn tool_use(
		&mut self,
		block: BlockId,
		id: &str,
		name: String,
		input: JsonRef<'c>,
		pending: &mut Pending<'c>,
	) {
		let path = block.path();
		if self.calls.contains_key(id) {
			self
				.diagnostics
				.problem(format!("`{path}` repeats a tool call id"));
			return;
		}
		let signature = pending.call.take();
		if signature
			.as_ref()
			.is_some_and(|signed| signed.name != name || signed.for_block != Some(block))
		{
			self
				.diagnostics
				.problem(format!("`{path}` doesn't match its signature"));
			return;
		}
		let part = wire::Part::FunctionCall {
			function_call: wire::FunctionCall {
				name: name.clone(),
				args: input,
			},
			thought_signature: signature.map(|signed| signed.signature),
		};
		self.contents.push(wire::Role::Model, part);
		self.unanswered_calls.push(id.to_string());
		let position = self.calls.len();
		self.calls.insert(id.to_string(), Call { position, name });
	}
}

fn tool_response<'a>(texts: &[&'a str], failed: bool) -> wire::ToolResponse<'a> {
	if failed {
		return wire::ToolResponse::Text {
			content: Cow::Owned(format!("{TOOL_ERROR_MARKER}{}", texts.join("\n"))),
		};
	}
	// Keep complete JSON blocks structured: embedding them in prose can make a model
	// copy JSON escapes as literal characters when constructing the next call.
	let output: Vec<_> = texts
		.iter()
		.map(|text| {
			let trimmed = text.trim();
			if matches!(trimmed.as_bytes().first(), Some(b'{' | b'['))
				&& is_strict(trimmed)
				&& let Ok(raw) = serde_json::from_str::<&RawValue>(trimmed)
			{
				wire::ToolValue::Json(raw)
			} else {
				wire::ToolValue::Text(text)
			}
		})
		.collect();
	if output
		.iter()
		.any(|part| matches!(part, wire::ToolValue::Json(_)))
	{
		wire::ToolResponse::Structured { output }
	} else {
		wire::ToolResponse::Text {
			content: match texts {
				[text] => Cow::Borrowed(text),
				_ => Cow::Owned(texts.join("\n")),
			},
		}
	}
}

fn format_request(schema: Option<&str>) -> String {
	let mut text = String::from(
		"When you give your final answer, after any tool calls, reply with only a JSON value",
	);
	match schema {
		Some(schema) => {
			text.push_str(" matching this schema:\n");
			text.push_str(schema);
		},
		None => text.push('.'),
	}
	text
}

struct ReplyFormat {
	schema: Option<String>,
	/// Recorded as dropped when tools mean the format can only be asked for.
	unenforced: &'static str,
}

/// What `generationConfig` needs beyond the client's settings.
struct ReplySettings<'a> {
	json: bool,
	schema: Option<&'a str>,
	thinking: Thinking,
}

const fn tool_config<'a>(settings: &'a Settings<'_>) -> Option<wire::ToolConfig<'a>> {
	let mode = match &settings.tool_choice {
		Some(ToolChoice::Auto) => wire::FunctionCallingMode::Auto,
		Some(ToolChoice::Required | ToolChoice::Tool(_)) => wire::FunctionCallingMode::Any,
		Some(ToolChoice::None) => wire::FunctionCallingMode::None,
		None => return None,
	};
	let allowed_function_names = match &settings.tool_choice {
		Some(ToolChoice::Tool(name)) => Some([name.as_str()]),
		_ => None,
	};
	Some(wire::ToolConfig {
		function_calling_config: wire::FunctionCallingConfig {
			mode,
			allowed_function_names,
		},
	})
}

/// `None` leaves `thinkingConfig` out.
const fn thinking_config(thinking: Thinking) -> Option<wire::ThinkingConfig> {
	let (thinking_budget, thinking_level) = match thinking.effort {
		ThinkingEffort::ProviderDefault if !thinking.include_thoughts => return None,
		ThinkingEffort::ProviderDefault => (None, None),
		ThinkingEffort::Budget(budget) => (Some(budget), None),
		ThinkingEffort::Level(level) => {
			let level = match level {
				ThinkingLevel::Minimal => wire::ThinkingLevel::Minimal,
				ThinkingLevel::Low => wire::ThinkingLevel::Low,
				ThinkingLevel::Medium => wire::ThinkingLevel::Medium,
				ThinkingLevel::High => wire::ThinkingLevel::High,
			};
			(None, Some(level))
		},
	};
	Some(wire::ThinkingConfig {
		include_thoughts: thinking.include_thoughts,
		thinking_budget,
		thinking_level,
	})
}

pub(crate) fn signs_text(item: &[u8]) -> bool {
	serde_json::from_slice::<SignedPart<'_>>(item)
		.is_ok_and(|part| !part.thought && part.function_call.is_none() && part.text.is_some())
}

fn joined<'a>(system: &[&'a str]) -> Cow<'a, str> {
	match system {
		[only] => Cow::Borrowed(only),
		_ => Cow::Owned(system.join("\n\n")),
	}
}

const fn text_part(text: Cow<'_, str>) -> wire::Part<'_> {
	wire::Part::Text { text }
}

fn data_image<'a>(image: &'a DataImage<'_>) -> wire::InlineData<'a> {
	wire::InlineData {
		inline_data: wire::Blob {
			mime_type: Cow::Borrowed(&image.media_type),
			data: Cow::Borrowed(&image.data),
		},
	}
}

/// Neighboring parts of one role share an entry, because Gemini wants roles to
/// alternate. Tool results keep an entry to themselves.
#[derive(Default)]
struct Contents<'a> {
	entries: Vec<wire::Content<'a>>,
	/// Whether the last entry takes more parts. A results entry doesn't.
	open: bool,
}

impl<'a> Contents<'a> {
	const fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	fn push(&mut self, role: wire::Role, part: wire::Part<'a>) {
		match self.entries.last_mut() {
			Some(last) if self.open && last.role == role => last.parts.push(part),
			_ => {
				self.entries.push(wire::Content {
					role,
					parts: vec![part],
				});
				self.open = true;
			},
		}
	}

	fn results(&mut self, parts: Vec<wire::Part<'a>>) {
		if parts.is_empty() {
			return;
		}
		self.entries.push(wire::Content {
			role: wire::Role::User,
			parts,
		});
		self.open = false;
	}
}

/// What goes in `thinkingConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Thinking {
	pub(crate) effort: ThinkingEffort,
	/// Asks for thought summaries in the reply.
	pub(crate) include_thoughts: bool,
}

/// How Gemini configures thinking for the selected model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThinkingEffort {
	/// Sends no level or budget, so the model's own default applies.
	ProviderDefault,
	Level(ThinkingLevel),
	/// In tokens. Zero turns thinking off on models that allow it.
	Budget(u32),
}

/// A Gemini thinking level, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThinkingLevel {
	Minimal,
	Low,
	Medium,
	High,
}

impl ThinkingLevel {
	const ALL: [Self; 4] = [Self::Minimal, Self::Low, Self::Medium, Self::High];
}

/// How a model's thinking can be set, as a model catalog records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingControls {
	/// The levels it takes. Never empty.
	Levels(ThinkingLevels),
	/// A token budget in a range. Some models can't stop thinking, so their smallest
	/// budget is the least they take.
	Budget { min: u32, max: u32, can_stop: bool },
}

/// A set of thinking levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThinkingLevels([bool; 4]);

impl ThinkingLevels {
	#[must_use]
	pub const fn with(mut self, level: ThinkingLevel) -> Self {
		self.0[level as usize] = true;
		self
	}

	const fn contains(self, level: ThinkingLevel) -> bool {
		self.0[level as usize]
	}

	#[must_use]
	pub fn is_empty(self) -> bool {
		!self.0.contains(&true)
	}

	/// The level nearest `wanted`, the higher of two as near.
	fn nearest(self, wanted: ThinkingLevel) -> ThinkingLevel {
		let distance = |level: ThinkingLevel| (level as i8 - wanted as i8).abs();
		ThinkingLevel::ALL
			.into_iter()
			.rev()
			.filter(|level| self.contains(*level))
			.min_by_key(|level| distance(*level))
			.unwrap_or_else(|| unreachable!("a model with thinking levels takes at least one"))
	}
}

impl ThinkingControls {
	/// A guess for callers without a catalog, which goes stale with aliases and new
	/// families. Gemini 2 takes a budget, Gemma 4 only turns thinking on and off, and
	/// anything else takes a level but `minimal`, which only some Flash models take.
	#[must_use]
	pub fn by_model_name(model: &str) -> Self {
		use ThinkingLevel::{High, Low, Medium, Minimal};
		if model.contains("gemini-2.") {
			let pro = model.contains("pro");
			return Self::Budget {
				min: if pro { 128 } else { 0 },
				max: if pro { 32768 } else { 24576 },
				can_stop: !pro,
			};
		}
		let levels: &[ThinkingLevel] = if model.contains("gemma-4") {
			&[Minimal, High]
		} else {
			&[Low, Medium, High]
		};
		Self::Levels(
			levels
				.iter()
				.fold(ThinkingLevels::default(), |levels, level| {
					levels.with(*level)
				}),
		)
	}
}

impl Thinking {
	/// The client's effort as the model takes it: the nearest level, or a budget in its
	/// range. A level can't stop thinking, so `Off` is the lowest one.
	pub fn for_controls(settings: &Settings<'_>, controls: ThinkingControls) -> Self {
		let effort = match (requested_effort(settings), controls) {
			(None, _) => ThinkingEffort::ProviderDefault,
			(Some(effort), ThinkingControls::Levels(levels)) => {
				ThinkingEffort::Level(levels.nearest(match effort {
					Effort::Off | Effort::Minimal => ThinkingLevel::Minimal,
					Effort::Low => ThinkingLevel::Low,
					Effort::Medium => ThinkingLevel::Medium,
					Effort::High | Effort::XHigh => ThinkingLevel::High,
				}))
			},
			(Some(effort), ThinkingControls::Budget { min, max, can_stop }) => {
				ThinkingEffort::Budget(match effort {
					Effort::Off if can_stop => 0,
					Effort::Off => min,
					Effort::Minimal | Effort::Low => 1024_u32.clamp(min, max),
					Effort::Medium => 2048_u32.clamp(min, max),
					Effort::High => 4096_u32.clamp(min, max),
					Effort::XHigh => 8192_u32.clamp(min, max),
				})
			},
		};
		Self {
			effort,
			include_thoughts: shows_thoughts(settings),
		}
	}
}

/// A Messages client says it in `output_config`, and `thinking: disabled` is a
/// request for none.
pub fn requested_effort(settings: &Settings<'_>) -> Option<Effort> {
	if settings.effort.is_some() {
		return settings.effort;
	}
	let disabled = settings
		.messages_input
		.thinking
		.as_ref()
		.is_some_and(|thinking| thinking.mode == Some(MessagesThinkingMode::Disabled));
	settings
		.messages_input
		.output_config
		.and_then(output_config_effort)
		.or_else(|| disabled.then_some(Effort::Off))
}

#[must_use]
pub fn shows_thoughts(settings: &Settings<'_>) -> bool {
	let shown = settings
		.messages_input
		.thinking
		.as_ref()
		.is_some_and(|thinking| {
			let on = matches!(
				thinking.mode,
				Some(MessagesThinkingMode::Adaptive | MessagesThinkingMode::Enabled)
			);
			on && thinking.display != Some(MessagesThinkingDisplay::Omitted)
		});
	settings.show_reasoning_summary || shown
}

pub(crate) fn output_config_effort(raw: JsonRef<'_>) -> Option<Effort> {
	let fields: Fields<'_> = raw.fields().ok()?;
	let (_, value) = fields.0.iter().find(|(name, _)| name == "effort")?;
	OutputEffort::read(&text_of(value)?).map(OutputEffort::effort)
}

/// The renderer reads `format`, so anything else is what a drop must record.
pub(crate) fn output_config_has_unread(raw: JsonRef<'_>) -> bool {
	let Ok(fields) = raw.fields() else {
		return true;
	};
	fields.0.iter().any(|(name, value)| match name.as_ref() {
		"format" => false,
		"effort" => text_of(value)
			.and_then(|word| OutputEffort::read(&word))
			.is_none(),
		_ => true,
	})
}

mod wire {
	use std::borrow::Cow;
	use std::num::NonZeroU64;

	use serde::Serialize;
	use serde_json::value::RawValue;

	use crate::json::JsonRef;

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct Request<'a> {
		pub(super) contents: Vec<Content<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) system_instruction: Option<SystemInstruction<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) tools: Option<[Tools<'a>; 1]>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) tool_config: Option<ToolConfig<'a>>,
		pub(super) generation_config: GenerationConfig<'a>,
	}

	#[derive(Serialize)]
	pub(super) struct Content<'a> {
		pub(super) role: Role,
		pub(super) parts: Vec<Part<'a>>,
	}

	#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum Role {
		User,
		Model,
	}

	#[derive(Serialize)]
	#[serde(untagged, rename_all_fields = "camelCase")]
	pub(super) enum Part<'a> {
		Text {
			text: Cow<'a, str>,
		},
		InlineData(InlineData<'a>),
		FunctionCall {
			function_call: FunctionCall<'a>,
			#[serde(skip_serializing_if = "Option::is_none")]
			thought_signature: Option<&'a RawValue>,
		},
		FunctionResponse {
			function_response: FunctionResponse<'a>,
		},
		/// A signed provider part, restored byte for byte from its carrier.
		Provider(&'a RawValue),
	}

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct InlineData<'a> {
		pub(super) inline_data: Blob<'a>,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct Blob<'a> {
		pub(super) mime_type: Cow<'a, str>,
		pub(super) data: Cow<'a, str>,
	}

	#[derive(Serialize)]
	pub(super) struct FunctionCall<'a> {
		pub(super) name: String,
		pub(super) args: JsonRef<'a>,
	}

	#[derive(Serialize)]
	pub(super) struct FunctionResponse<'a> {
		pub(super) name: String,
		pub(super) response: ToolResponse<'a>,
		/// Nested so each image stays with its call. Gemini 2.5 refuses this form.
		#[serde(skip_serializing_if = "Vec::is_empty")]
		pub(super) parts: Vec<InlineData<'a>>,
	}

	#[derive(Serialize)]
	#[serde(untagged)]
	pub(super) enum ToolResponse<'a> {
		Text { content: Cow<'a, str> },
		Structured { output: Vec<ToolValue<'a>> },
	}

	#[derive(Serialize)]
	#[serde(untagged)]
	pub(super) enum ToolValue<'a> {
		Text(&'a str),
		Json(&'a RawValue),
	}

	#[derive(Serialize)]
	pub(super) struct SystemInstruction<'a> {
		pub(super) parts: [SystemText<'a>; 1],
	}

	#[derive(Serialize)]
	pub(super) struct SystemText<'a> {
		pub(super) text: Cow<'a, str>,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct Tools<'a> {
		pub(super) function_declarations: Vec<FunctionDeclaration<'a>>,
	}

	#[derive(Serialize)]
	pub(super) struct FunctionDeclaration<'a> {
		pub(super) name: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) description: Option<Cow<'a, str>>,
		/// Rewritten for Gemini, so owned.
		pub(super) parameters: Box<RawValue>,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct ToolConfig<'a> {
		pub(super) function_calling_config: FunctionCallingConfig<'a>,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct FunctionCallingConfig<'a> {
		pub(super) mode: FunctionCallingMode,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) allowed_function_names: Option<[&'a str; 1]>,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
	pub(super) enum FunctionCallingMode {
		Auto,
		Any,
		None,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct GenerationConfig<'a> {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) max_output_tokens: Option<NonZeroU64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) temperature: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) top_p: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) top_k: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) stop_sequences: Option<&'a [String]>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) response_mime_type: Option<MimeType>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) response_schema: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) thinking_config: Option<ThinkingConfig>,
	}

	#[derive(Serialize, Clone, Copy)]
	pub(super) enum MimeType {
		#[serde(rename = "application/json")]
		Json,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "camelCase")]
	pub(super) struct ThinkingConfig {
		#[serde(skip_serializing_if = "std::ops::Not::not")]
		pub(super) include_thoughts: bool,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) thinking_budget: Option<u32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) thinking_level: Option<ThinkingLevel>,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum ThinkingLevel {
		Minimal,
		Low,
		Medium,
		High,
	}
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;

/// JSON written or already read here, passed on as it is.
fn read_json(json: &str) -> &RawValue {
	match serde_json::from_str(json) {
		Ok(json) => json,
		Err(error) => unreachable!("JSON written or already read here: {error}"),
	}
}
