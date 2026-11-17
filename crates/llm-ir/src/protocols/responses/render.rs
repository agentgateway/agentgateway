use std::borrow::Cow;
use std::collections::HashSet;

use super::wire;
use crate::carrier::IdentifierPseudonymizer;
use crate::ir::conversation::{
	BlockId, CacheBreakpoint, Carrier, Content, Conversation, DataImage, Effort, ImageFields,
	Message, MessagesThinking, MessagesThinkingDisplay, MessagesThinkingMode, OutputEffort,
	OutputFormat, Role, Settings, Text, Tool, ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::{Fields, JsonRef, is_strict_object};
use crate::prompt::diagnostics::{Diagnostics, Unread, UnreadConfig};
use crate::prompt::{NOTHING_TO_SEND, ResolvedCapabilities, TOOL_ERROR_MARKER};
use crate::protocols::reasoning::ReasoningControls;
use crate::protocols::{
	check_sampling, check_tool_choice, output_format_schema, provider_item, read_image,
	refuse_special_tool, short_identifier,
};

pub struct Options<'a> {
	pub(crate) identifier_pseudonymizer: &'a dyn IdentifierPseudonymizer,
	pub(crate) model: &'a str,
	pub(crate) safety_identifier: bool,
	pub(crate) reasoning_context: bool,
	pub(crate) reasoning: ReasoningControls,
	pub(crate) capabilities: ResolvedCapabilities,
	/// Empty when the resolved capabilities exclude explicit cache breakpoints.
	pub(crate) cache_breakpoints: &'a [CacheBreakpoint],
}

pub fn render<'a>(
	conversation: &'a Conversation<'_>,
	unread: &'a Unread<'_>,
	diagnostics: Diagnostics,
	options: Options<'_>,
) -> (String, Diagnostics) {
	let mut renderer = Renderer {
		options,
		diagnostics,
		input: Items::default(),
		unanswered_calls: Vec::new(),
		seen_calls: HashSet::new(),
	};
	let body = match renderer.request(conversation, unread) {
		Ok(body) => body,
		Err(error) => {
			renderer
				.diagnostics
				.problem(format!("could not serialize Responses request: {error}"));
			String::new()
		},
	};
	(body, renderer.diagnostics)
}

struct Renderer<'a, 'c> {
	options: Options<'c>,
	diagnostics: Diagnostics,
	input: Items<'a>,
	unanswered_calls: Vec<&'a str>,
	seen_calls: HashSet<&'a str>,
}

#[derive(Default)]
struct Target<'r> {
	tools: Option<Vec<wire::FunctionTool<'r>>>,
	effort: Option<Effort>,
	/// From a Messages client's `output_config.format`, already checked.
	schema: Option<JsonRef<'r>>,
	adaptive_thinking: bool,
	thinking_off: bool,
	reasoning_context: bool,
	safety_identifier: Option<String>,
}

impl<'a> Renderer<'a, '_> {
	fn request(
		&mut self,
		conversation: &'a Conversation<'_>,
		unread: &'a Unread<'_>,
	) -> serde_json::Result<String> {
		let settings = &conversation.settings;
		// `system[]` joins into `instructions`. System messages inside the
		// conversation stay in position, as input items.
		let instructions = self.instructions(conversation.system.as_deref());
		let mut target = Target {
			tools: self.tools(&conversation.tools),
			..Target::default()
		};
		check_sampling(settings, &mut self.diagnostics);
		// Responses has no `stop`; `ReplyPlan::emulated_stop_sequences` applies it on replies.
		if settings.top_k.is_some() {
			self.diagnostics.drop_field("top_k");
		}
		if settings.keep_earlier_reasoning {
			self.diagnostics.problem("setting `keep_earlier_reasoning`");
		}
		check_tool_choice(settings, &conversation.tools, &mut self.diagnostics);
		if let Some(thinking) = &settings.messages_input.thinking {
			self.thinking(thinking, &mut target);
		}
		if let Some(raw) = settings.messages_input.output_config {
			self.output_config(raw, &mut target);
		}
		if let Some(raw) = settings.messages_input.context_management {
			self.context_management(raw, &mut target);
		}
		if let Some(raw) = settings.messages_input.metadata {
			self.metadata(raw, &mut target);
		}
		for image in &unread.images {
			self.image(&image.path, &image.fields);
		}
		let turn_effort = self.messages(&conversation.messages, unread);
		let empty = conversation
			.messages
			.iter()
			.all(|message| message.blocks.is_empty());
		if empty && self.diagnostics.problems.is_empty() {
			self.diagnostics.problem(NOTHING_TO_SEND);
		}
		target.effort = turn_effort.or(target.effort);
		if target.thinking_off {
			target.effort = Some(Effort::Off);
		}
		if let Some(effort) = settings.effort {
			target.effort = Some(effort);
		}
		if target.adaptive_thinking && target.effort.is_none() {
			self
				.diagnostics
				.problem("adaptive `thinking` without an effort");
		}
		if let Some(requested) = target.effort {
			target.effort = self.options.reasoning.effort(
				requested,
				self.options.capabilities.default_reasoning_effort,
			);
			if target.effort.is_none() {
				self.diagnostics.drop_field("reasoning effort");
			}
		}
		self.write(settings, target, instructions)
	}

	fn instructions(&mut self, system: Option<&'a [Text<'_>]>) -> Option<Cow<'a, str>> {
		// `instructions` has no parts to mark, so a marked system prompt is a message.
		let marked_system = self
			.options
			.cache_breakpoints
			.iter()
			.any(|at| matches!(at, CacheBreakpoint::System(_)));
		if let Some(system) = system
			&& marked_system
		{
			for (at, text) in system.iter().enumerate() {
				let mark = self.marked(CacheBreakpoint::System(at));
				self.input.text(Role::System, text.as_str(), mark);
			}
			self.input.close();
			None
		} else {
			system.map(joined_instructions)
		}
	}

	fn marked(&self, at: CacheBreakpoint) -> bool {
		self.options.cache_breakpoints.contains(&at)
	}

	fn write(
		&mut self,
		settings: &'a Settings<'_>,
		target: Target<'a>,
		instructions: Option<Cow<'a, str>>,
	) -> serde_json::Result<String> {
		// A format a stage set replaces the client's.
		let text = match (&settings.output_format, target.schema) {
			(Some(OutputFormat::Json), _) => Some(wire::TextConfig {
				format: wire::TextFormat::JsonObject,
			}),
			(Some(OutputFormat::JsonSchema(schema)), _) if is_strict_object(schema) => {
				Some(Self::json_schema(schema.as_ref()))
			},
			(Some(OutputFormat::JsonSchema(_)), _) => {
				self
					.diagnostics
					.problem("`text.format.schema` must be a valid object");
				None
			},
			(None, Some(schema)) => Some(Self::json_schema(schema)),
			(None, None) => None,
		};
		let has_tools = target.tools.is_some();
		let (tool_choice, parallel_tool_calls) = if has_tools {
			(
				tool_choice(settings.tool_choice.as_ref()),
				settings.parallel_tool_calls,
			)
		} else {
			(None, None)
		};
		let reasoning =
			(target.effort.is_some() || target.reasoning_context).then_some(wire::Reasoning {
				effort: target.effort.map(wire_effort),
				context: target
					.reasoning_context
					.then_some(wire::ReasoningContext::AllTurns),
			});
		let request = wire::Request {
			model: self.options.model,
			instructions,
			input: std::mem::take(&mut self.input.items),
			tools: target.tools,
			tool_choice,
			parallel_tool_calls,
			max_output_tokens: settings.max_tokens,
			temperature: settings.temperature,
			top_p: settings.top_p,
			text,
			reasoning,
			stream: settings.stream,
			store: false,
			include: [wire::Include::ReasoningEncryptedContent],
			safety_identifier: target.safety_identifier.as_deref(),
		};
		serde_json::to_string(&request)
	}

	fn tools(&mut self, tools: &'a [Tool<'_>]) -> Option<Vec<wire::FunctionTool<'a>>> {
		if tools.is_empty() {
			return None;
		}
		let mut output = Vec::with_capacity(tools.len());
		for tool in tools {
			let strict = match tool.kind {
				ToolKind::Function { strict } if tool.namespace.is_none() => strict.unwrap_or(false),
				_ => {
					refuse_special_tool(tool, &mut self.diagnostics);
					false
				},
			};
			// Left out, OpenAI may normalize the schema and change its meaning.
			output.push(wire::FunctionTool {
				kind: wire::FunctionType::Function,
				name: &tool.name,
				description: tool.description.as_deref(),
				parameters: tool.schema.as_ref(),
				strict,
			});
		}
		Some(output)
	}

	/// Leaving out `reasoning.summary` keeps replies free of plaintext reasoning, so
	/// only `omitted` can be met.
	fn thinking(&mut self, thinking: &MessagesThinking<'_>, target: &mut Target<'_>) {
		match thinking.mode {
			Some(MessagesThinkingMode::Adaptive) => target.adaptive_thinking = true,
			Some(MessagesThinkingMode::Enabled) => self.diagnostics.problem("`thinking.type` `enabled`"),
			Some(MessagesThinkingMode::Disabled) => target.thinking_off = true,
			None => {},
		}
		if thinking.display == Some(MessagesThinkingDisplay::Summarized) {
			self.diagnostics.problem("`thinking.display` `summarized`");
		}
		if thinking.budget_tokens.is_some() {
			self.diagnostics.problem("field `thinking.budget_tokens`");
		}
	}

	/// Claude Code asks for effort and structured output here.
	fn output_config<'r>(&mut self, raw: JsonRef<'r>, target: &mut Target<'r>) {
		let Some(fields) = self.diagnostics.object("output_config", raw) else {
			return;
		};
		for (name, value) in fields.0 {
			match name.as_ref() {
				"effort" => target.effort = self.effort_word("output_config.effort", value),
				"format" => target.schema = output_format_schema(value, &mut self.diagnostics),
				other => self
					.diagnostics
					.problem(format!("field `output_config.{other}`")),
			}
		}
	}

	fn effort(&mut self, path: &str, raw: JsonRef<'_>) -> Option<Effort> {
		let fields = self.diagnostics.object(path, raw)?;
		let mut effort = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"effort" => effort = self.effort_word(&format!("{path}.effort"), *value),
				other => self.diagnostics.problem(format!("field `{path}.{other}`")),
			}
		}
		effort
	}

	fn effort_word(&mut self, path: &str, raw: JsonRef<'_>) -> Option<Effort> {
		let word = self.diagnostics.typed::<String>(path, raw)?;
		match OutputEffort::read(&word) {
			// Other targets send `max` as their top level. Nothing has decided that here.
			Some(OutputEffort::Max) | None => {
				self.diagnostics.problem(format!("`{path}` `{word}`"));
				None
			},
			Some(OutputEffort::Low) => Some(Effort::Low),
			Some(OutputEffort::Medium) => Some(Effort::Medium),
			Some(OutputEffort::High) => Some(Effort::High),
			Some(OutputEffort::XHigh) => Some(Effort::XHigh),
		}
	}

	fn context_management(&mut self, raw: JsonRef<'_>, target: &mut Target<'_>) {
		if !is_clear_thinking_keep_all(raw) {
			self.diagnostics.problem("`context_management` shape");
		} else if self.options.reasoning_context {
			target.reasoning_context = true;
		} else {
			self
				.diagnostics
				.problem("`context_management` on a model that doesn't accept it");
		}
	}

	fn metadata(&mut self, raw: JsonRef<'_>, target: &mut Target<'_>) {
		let Some(fields) = self.diagnostics.object("metadata", raw) else {
			return;
		};
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"user_id" => {
					let Some(user) = self.diagnostics.typed::<String>("metadata.user_id", value) else {
						continue;
					};
					if self.options.safety_identifier {
						target.safety_identifier = Some(short_identifier(
							self.options.identifier_pseudonymizer,
							user,
						));
					} else {
						self.diagnostics.drop_field("metadata.user_id");
					}
				},
				other => self
					.diagnostics
					.problem(format!("field `metadata.{other}`")),
			}
		}
	}

	/// Returns the effort of the last turn that set one.
	fn messages(&mut self, messages: &'a [Message<'_>], unread: &'a Unread<'_>) -> Option<Effort> {
		let mut turn_effort = None;
		let mut configs = unread.output_configs.iter().peekable();
		for message in messages {
			while let Some(config) = configs.next_if(|config| config.index < message.index) {
				self.unread_config(config, &mut turn_effort);
			}
			let path = format!("messages[{}]", message.index);
			let effort = message
				.messages_output_config
				.and_then(|raw| self.effort(&format!("{path}.output_config"), raw));
			let new_input = match message.role {
				Role::User => {
					let new_input = self.user_message(message);
					new_input || unread.refused_text.contains(&message.index)
				},
				Role::Assistant => {
					self.assistant_message(message);
					false
				},
				Role::System => {
					self.system_message(message);
					false
				},
			};
			// An effort belongs to its turn, and a new user input starts a turn.
			if new_input {
				turn_effort = None;
			}
			turn_effort = effort.or(turn_effort);
		}
		for config in configs {
			self.unread_config(config, &mut turn_effort);
		}
		self.input.close();
		if !self.unanswered_calls.is_empty() {
			let calls = self.unanswered_calls.join(", ");
			self
				.diagnostics
				.problem(format!("tool calls with no result: {calls}"));
		}
		turn_effort
	}

	fn unread_config(&mut self, config: &UnreadConfig<'_>, turn_effort: &mut Option<Effort>) {
		let path = format!("messages[{}].output_config", config.index);
		let effort = self.effort(&path, config.raw);
		if config.in_turn {
			*turn_effort = effort.or_else(|| turn_effort.take());
		}
	}

	fn system_message(&mut self, message: &'a Message<'_>) {
		for block in &message.blocks {
			if let Content::Text(text) = &block.content {
				let mark = self.marked(CacheBreakpoint::Block(block.id));
				self.input.text(Role::System, text.as_str(), mark);
			}
		}
		self.input.close();
	}

	/// Returns whether the message contains new input, not only tool results.
	fn user_message(&mut self, message: &'a Message<'_>) -> bool {
		let mut new_input = false;
		for block in &message.blocks {
			let mark = self.marked(CacheBreakpoint::Block(block.id));
			match &block.content {
				Content::Text(text) => {
					new_input = true;
					self.input.text(Role::User, text.as_str(), mark);
				},
				Content::Image(fields) => {
					new_input = true;
					if let Some(url) = self.image(&block.id.path(), fields) {
						self.input.image(Role::User, Cow::Owned(url), mark);
					}
				},
				Content::DataImage(image) => {
					new_input = true;
					self
						.input
						.image(Role::User, Cow::Owned(image_url(image)), mark);
				},
				Content::ToolResult {
					call_id,
					failed,
					output,
				} => {
					self.input.close();
					self.tool_result(block.id, call_id, *failed, output);
				},
				Content::ToolCall { .. } | Content::Reasoning { .. } => {},
			}
		}
		self.input.close();
		new_input
	}

	/// An image as a data URL, the only form both APIs share.
	fn image(&mut self, path: &str, fields: &ImageFields<'_>) -> Option<String> {
		let image = read_image(path, fields, &mut self.diagnostics)?;
		Some(image_url(&image))
	}

	fn tool_result(
		&mut self,
		id: BlockId,
		call: &'a str,
		failed: Option<bool>,
		output: &'a ToolOutput<'_>,
	) {
		let path = id.path();
		if failed == Some(false) {
			self.diagnostics.drop_field("tool_result.is_error");
		}
		match self.unanswered_calls.iter().position(|open| *open == call) {
			Some(at) => {
				self.unanswered_calls.remove(at);
			},
			None => self
				.diagnostics
				.problem(format!("`{path}` answers no open tool call")),
		}
		let failed = failed == Some(true);
		let mark = self.marked(CacheBreakpoint::Block(id));
		let output = match output {
			ToolOutput::Text(text) => {
				let text = if failed {
					Cow::Owned(format!("{TOOL_ERROR_MARKER}{}", text.as_str()))
				} else {
					Cow::Borrowed(text.as_str())
				};
				// Plain text has no part to mark.
				if mark {
					wire::ToolOutput::Parts(vec![wire_part(Role::User, text, true)])
				} else {
					wire::ToolOutput::Text(text)
				}
			},
			ToolOutput::Mixed(parts) => {
				wire::ToolOutput::Parts(self.mixed_output(&path, failed, parts, mark))
			},
		};
		self.input.push(wire::InputItem::Known(
			wire::KnownInputItem::FunctionCallOutput {
				call_id: call,
				output,
			},
		));
	}

	/// A marked result is marked on its last part.
	fn mixed_output(
		&mut self,
		path: &str,
		failed: bool,
		parts: &'a [ToolPart<'_>],
		mark: bool,
	) -> Vec<wire::Part<'a>> {
		let mut output = Vec::with_capacity(parts.len() + usize::from(failed));
		if failed {
			output.push(wire_part(
				Role::User,
				Cow::Borrowed(TOOL_ERROR_MARKER),
				false,
			));
		}
		for part in parts {
			match part {
				ToolPart::Text(text) => {
					output.push(wire_part(Role::User, Cow::Borrowed(text.as_str()), false))
				},
				ToolPart::Image { index, fields } => {
					if let Some(url) = self.image(&format!("{path}.content[{index}]"), fields) {
						output.push(wire_image(Cow::Owned(url), false));
					}
				},
				ToolPart::DataImage(image) => {
					output.push(wire_image(Cow::Owned(image_url(image)), false));
				},
			}
		}
		if mark {
			if let Some(last) = output.last_mut() {
				last.mark();
			} else {
				output.push(wire_part(Role::User, Cow::Borrowed(""), true));
			}
		}
		output
	}

	fn assistant_message(&mut self, message: &'a Message<'_>) {
		for block in &message.blocks {
			match &block.content {
				Content::Reasoning { carrier, .. } => {
					self.input.close();
					self.reasoning(block.id, carrier);
				},
				Content::Text(text) => {
					let mark = self.marked(CacheBreakpoint::Block(block.id));
					self.input.text(Role::Assistant, text.as_str(), mark);
				},
				Content::ToolCall {
					id, name, input, ..
				} => {
					self.input.close();
					self.tool_use(block.id, id, name, input.as_ref());
				},
				Content::Image(_) | Content::DataImage(_) | Content::ToolResult { .. } => {},
			}
		}
		self.input.close();
	}

	/// Restores the provider item byte for byte. Only a provider-completed item
	/// was ever encoded, so it keeps its position even with nothing after it.
	fn reasoning(&mut self, id: BlockId, carrier: &'a Carrier) {
		let path = id.path();
		let item = match carrier.opened() {
			Ok(item) => item,
			Err(error) => {
				self.diagnostics.problem(format!("`{path}`: {error}"));
				return;
			},
		};
		match provider_item(item) {
			Some(item) => self.input.push(wire::InputItem::Provider(item)),
			None => self
				.diagnostics
				.problem(format!("`{path}`: carrier contains no provider item")),
		}
	}

	fn tool_use(&mut self, block: BlockId, id: &'a str, name: &'a str, input: JsonRef<'a>) {
		if self.seen_calls.contains(id) {
			self
				.diagnostics
				.problem(format!("`{}` repeats a tool call id", block.path()));
			return;
		}
		self
			.input
			.push(wire::InputItem::Known(wire::KnownInputItem::FunctionCall {
				call_id: id,
				name,
				arguments: input.encoded(),
			}));
		self.seen_calls.insert(id);
		self.unanswered_calls.push(id);
	}

	const fn json_schema(schema: JsonRef<'a>) -> wire::TextConfig<'a> {
		wire::TextConfig {
			format: wire::TextFormat::JsonSchema {
				name: "structured_output",
				schema,
			},
		}
	}
}

fn image_url(image: &DataImage<'_>) -> String {
	format!("data:{};base64,{}", image.media_type, image.data)
}

fn joined_instructions<'a>(system: &'a [Text<'_>]) -> Cow<'a, str> {
	match system {
		[] => Cow::Borrowed(""),
		[only] => Cow::Borrowed(only.as_str()),
		_ => {
			let capacity = system.iter().map(|text| text.len()).sum::<usize>() + (system.len() - 1) * 2;
			let mut joined = String::with_capacity(capacity);
			for (at, text) in system.iter().enumerate() {
				if at > 0 {
					joined.push_str("\n\n");
				}
				joined.push_str(text.as_str());
			}
			Cow::Owned(joined)
		},
	}
}

fn tool_choice(choice: Option<&ToolChoice>) -> Option<wire::ToolChoice<'_>> {
	match choice {
		Some(ToolChoice::Auto) => Some(wire::ToolChoice::Mode(wire::ToolChoiceMode::Auto)),
		Some(ToolChoice::Required) => Some(wire::ToolChoice::Mode(wire::ToolChoiceMode::Required)),
		Some(ToolChoice::None) => Some(wire::ToolChoice::Mode(wire::ToolChoiceMode::None)),
		Some(ToolChoice::Tool(name)) => Some(wire::ToolChoice::Function {
			kind: wire::FunctionType::Function,
			name,
		}),
		None => None,
	}
}

#[derive(Default)]
struct Items<'a> {
	items: Vec<wire::InputItem<'a>>,
	message_role: Option<wire::Role>,
	message_parts: Vec<wire::Part<'a>>,
}

impl<'a> Items<'a> {
	fn text(&mut self, role: Role, text: &'a str, mark: bool) {
		self.open(role);
		self
			.message_parts
			.push(wire_part(role, Cow::Borrowed(text), mark));
	}

	fn image(&mut self, role: Role, url: Cow<'a, str>, mark: bool) {
		self.open(role);
		self.message_parts.push(wire_image(url, mark));
	}

	fn open(&mut self, role: Role) {
		let role = wire_role(role);
		if self.message_role != Some(role) {
			self.close();
			self.message_role = Some(role);
		}
	}

	fn push(&mut self, item: wire::InputItem<'a>) {
		self.close();
		self.items.push(item);
	}

	fn close(&mut self) {
		if let Some(role) = self.message_role.take() {
			self
				.items
				.push(wire::InputItem::Known(wire::KnownInputItem::Message {
					role,
					content: std::mem::take(&mut self.message_parts),
				}));
		}
	}
}

const fn wire_effort(effort: Effort) -> wire::ReasoningEffort {
	match effort {
		Effort::Off => wire::ReasoningEffort::None,
		Effort::Minimal => wire::ReasoningEffort::Minimal,
		Effort::Low => wire::ReasoningEffort::Low,
		Effort::Medium => wire::ReasoningEffort::Medium,
		Effort::High => wire::ReasoningEffort::High,
		Effort::XHigh => wire::ReasoningEffort::Xhigh,
	}
}

const fn wire_role(role: Role) -> wire::Role {
	match role {
		Role::User => wire::Role::User,
		Role::Assistant => wire::Role::Assistant,
		Role::System => wire::Role::System,
	}
}

fn wire_part<'a>(role: Role, text: Cow<'a, str>, mark: bool) -> wire::Part<'a> {
	let prompt_cache_breakpoint = mark.then_some(wire::PromptCacheBreakpoint::EXPLICIT);
	match role {
		Role::Assistant => wire::Part::OutputText {
			text,
			prompt_cache_breakpoint,
		},
		Role::User | Role::System => wire::Part::InputText {
			text,
			prompt_cache_breakpoint,
		},
	}
}

fn wire_image(url: Cow<'_, str>, mark: bool) -> wire::Part<'_> {
	wire::Part::InputImage {
		image_url: url,
		prompt_cache_breakpoint: mark.then_some(wire::PromptCacheBreakpoint::EXPLICIT),
	}
}

/// The only shape Claude Code was seen to send: one edit, keeping everything.
fn is_clear_thinking_keep_all(raw: JsonRef<'_>) -> bool {
	let string = |raw: JsonRef<'_>, want: &str| raw.read::<String>().is_ok_and(|text| text == want);
	let Ok(Fields(top)) = raw.fields() else {
		return false;
	};
	let [(name, edits)] = top.as_slice() else {
		return false;
	};
	let Ok(edits) = edits.list().and_then(|items| {
		items
			.into_iter()
			.map(JsonRef::fields)
			.collect::<Result<Vec<_>, _>>()
	}) else {
		return false;
	};
	let [edit] = edits.as_slice() else {
		return false;
	};
	name == "edits"
		&& edit.0.len() == 2
		&& edit
			.0
			.iter()
			.any(|(name, value)| name == "type" && string(*value, "clear_thinking_20251015"))
		&& edit
			.0
			.iter()
			.any(|(name, value)| name == "keep" && string(*value, "all"))
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
