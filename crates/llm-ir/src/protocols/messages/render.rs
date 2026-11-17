//! Adjacent user or assistant messages join into one here, because Messages
//! wants roles to alternate.
//!
//! Tool names are flattened here and restored on replies through `ToolRoute`s. Cache
//! breakpoints go on the last system block and the last block of the last user
//! message, or where a [`PromptCaching`] policy puts them, after every stage has run, so
//! an edit doesn't move one.

use std::borrow::Cow;

use crate::ir::conversation::{
	Carrier, Content, Conversation, DataImage, Effort, OutputFormat, Role, Settings, Text, Tool,
	ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::is_strict_object;
use crate::prompt::NOTHING_TO_SEND;
use crate::prompt::diagnostics::Diagnostics;
use crate::protocols::{
	CallKind, ToolRoute, check_sampling, check_tool_choice, custom_description, flat_name,
	provider_item, read_image, system_reminder, tool_declarations,
};

const MAX_TOOL_NAME: usize = 128;
const CONTEXT_MANAGEMENT_BETA: &str = "context-management-2025-06-27";

pub struct Rendered {
	pub(crate) body: String,
	pub(crate) routes: Vec<ToolRoute>,
	pub(crate) beta: Option<&'static str>,
}

/// Where a request asks the provider to cache, as a gateway policy sets it in place of the
/// renderer's own marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptCaching {
	/// Marks the last system block, when the system prompt has at least `min_tokens`.
	pub system: bool,
	/// Marks the last block that isn't reasoning in the message `message_offset` before
	/// the second-to-last one.
	pub messages: bool,
	/// Marks the last tool.
	pub tools: bool,
	/// Minimum estimated system-prompt tokens for caching; `None` disables the threshold.
	/// Estimated as 1.3 tokens per whitespace-separated word, rounded down.
	pub min_tokens: Option<usize>,
	pub message_offset: usize,
}

pub fn render<'a>(
	conversation: &'a Conversation<'_>,
	mut diagnostics: Diagnostics,
	model: &'a str,
	caching: Option<PromptCaching>,
) -> (Rendered, Diagnostics) {
	let mut routes = Vec::new();
	let mut tools = tools(&conversation.tools, &mut routes, &mut diagnostics);
	let mut messages = messages(conversation, &mut diagnostics);
	match caching {
		None => mark_last_user_turn(&mut messages),
		Some(caching) => {
			if caching.messages {
				mark_earlier_turn(&mut messages, caching.message_offset);
			}
			if caching.tools
				&& let Some(tool) = tools.last_mut()
			{
				tool.cache_control = Some(wire::CacheControl::EPHEMERAL);
			}
		},
	}
	let settings = &conversation.settings;
	check_tool_choice(settings, &conversation.tools, &mut diagnostics);
	check_sampling(settings, &mut diagnostics);
	if let Some(top_k) = settings.top_k {
		diagnostics.typed::<u64>("top_k", top_k);
	}
	if let Some(effort) = settings.effort
		&& matches!(effort, Effort::Off | Effort::Minimal)
	{
		diagnostics.problem("`reasoning.effort` has no Messages form");
	}
	let format = match &settings.output_format {
		Some(OutputFormat::JsonSchema(schema)) if is_strict_object(schema) => Some(wire::Format {
			kind: wire::JsonSchemaType::JsonSchema,
			schema: schema.as_ref(),
		}),
		Some(OutputFormat::JsonSchema(_)) => {
			diagnostics.problem("`text.format.schema` must be a valid object");
			None
		},
		Some(OutputFormat::Json) => {
			diagnostics.problem("`text.format` `json_object` has no Messages form");
			None
		},
		None => None,
	};
	let system: Vec<wire::SystemBlock<'a>> = conversation
		.system
		.iter()
		.flatten()
		.map(Text::as_str)
		.filter(|text: &&str| !text.is_empty())
		.map(|text| wire::SystemBlock {
			kind: wire::TextType::Text,
			text,
			cache_control: None,
		})
		.collect();
	let effort = settings.effort.map(|effort| match effort {
		Effort::Off | Effort::Minimal | Effort::Low => wire::Effort::Low,
		Effort::Medium => wire::Effort::Medium,
		Effort::High => wire::Effort::High,
		Effort::XHigh => wire::Effort::Xhigh,
	});
	let has_tools = !tools.is_empty();
	let request = wire::Request {
		model,
		// Parsing or `prompt::after_edits` rejects a missing limit before this body is sent.
		max_tokens: settings.max_tokens,
		system: (!system.is_empty()).then(|| match caching {
			Some(caching) if !caches_system(caching, &system) => system,
			_ => mark_last_system_block(system),
		}),
		messages,
		tools: has_tools.then_some(tools),
		tool_choice: has_tools.then(|| tool_choice(settings)),
		top_k: settings.top_k,
		temperature: settings.temperature,
		top_p: settings.top_p,
		stop_sequences: (!settings.stop_sequences.is_empty())
			.then_some(settings.stop_sequences.as_slice()),
		thinking: effort.map(|_| thinking(settings)),
		output_config: (effort.is_some() || format.is_some())
			.then_some(wire::OutputConfig { effort, format }),
		context_management: settings
			.keep_earlier_reasoning
			.then_some(wire::ContextManagement {
				edits: [wire::ContextEdit {
					kind: wire::ContextEditType::ClearThinking,
					keep: wire::Keep::All,
				}],
			}),
		stream: settings.stream,
	};
	let body = serde_json::to_string(&request).unwrap_or_else(|error| {
		diagnostics.problem(format!("could not serialize Messages request: {error}"));
		String::new()
	});
	let beta = settings
		.keep_earlier_reasoning
		.then_some(CONTEXT_MANAGEMENT_BETA);
	(Rendered { body, routes, beta }, diagnostics)
}

fn tool_choice<'a>(settings: &'a Settings<'_>) -> wire::ToolChoice<'a> {
	let (kind, name) = match &settings.tool_choice {
		Some(ToolChoice::Auto) | None => (wire::ToolChoiceType::Auto, None),
		Some(ToolChoice::Required) => (wire::ToolChoiceType::Any, None),
		Some(ToolChoice::None) => (wire::ToolChoiceType::None, None),
		Some(ToolChoice::Tool(name)) => (wire::ToolChoiceType::Tool, Some(name.as_str())),
	};
	let disable_parallel_tool_use = (settings.parallel_tool_calls == Some(false)
		&& kind != wire::ToolChoiceType::None)
		.then_some(true);
	wire::ToolChoice {
		kind,
		name,
		disable_parallel_tool_use,
	}
}

const fn thinking(settings: &Settings<'_>) -> wire::Thinking {
	wire::Thinking {
		kind: wire::ThinkingType::Adaptive,
		display: if settings.show_reasoning_summary {
			wire::ThinkingDisplay::Summarized
		} else {
			wire::ThinkingDisplay::Omitted
		},
	}
}

fn tools<'a>(
	tools: &'a [Tool<'_>],
	routes: &mut Vec<ToolRoute>,
	diagnostics: &mut Diagnostics,
) -> Vec<wire::Tool<'a>> {
	let mut out = Vec::with_capacity(tools.len());
	for tool in tool_declarations(tools) {
		let flat = flat_name(tool.namespace.as_deref(), &tool.name);
		let valid = !flat.is_empty()
			&& flat.len() <= MAX_TOOL_NAME
			&& flat
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
		if !valid {
			diagnostics.problem(format!(
				"`{}` name `{flat}` doesn't fit Messages",
				tool.path
			));
			continue;
		}
		if routes.iter().any(|route| route.flat == flat) {
			diagnostics.problem(format!("`{}` name `{flat}` collides", tool.path));
			continue;
		}
		let (strict, description) = match &tool.kind {
			ToolKind::Function { strict } => (*strict, tool.description.as_deref().map(Cow::Borrowed)),
			ToolKind::Custom { grammar } => {
				let description = custom_description(tool, grammar.as_deref(), diagnostics);
				(None, Some(Cow::Owned(description)))
			},
			ToolKind::ToolSearch => (None, tool.description.as_deref().map(Cow::Borrowed)),
		};
		out.push(wire::Tool {
			name: flat.clone(),
			description: description.filter(|text| !text.is_empty()),
			input_schema: tool.schema.as_ref(),
			strict,
			cache_control: None,
		});
		routes.push(ToolRoute {
			flat,
			namespace: tool.namespace.clone(),
			name: tool.name.clone(),
			kind: CallKind::from(&tool.kind),
		});
	}
	out
}

/// Messages takes system text only in the top-level `system` field, so a later system
/// message goes in the user turn at its place, as a reminder. A reminder waits for the
/// tool results of the next user message, which Messages wants first in their turn. A
/// message a stage emptied is left out. Parsing checks tool calls, but a stage can undo
/// that.
fn messages<'a>(
	conversation: &'a Conversation<'_>,
	diagnostics: &mut Diagnostics,
) -> Vec<wire::Message<'a>> {
	let checks_edits = diagnostics.problems.is_empty();
	let mut out: Vec<wire::Message<'a>> = Vec::with_capacity(conversation.messages.len());
	let mut reminders = Vec::new();
	let mut open_calls: Vec<&str> = Vec::new();
	for message in &conversation.messages {
		if message.role == Role::System {
			reminders.extend(
				message
					.blocks
					.iter()
					.filter_map(|block| match &block.content {
						Content::Text(text) if !text.is_empty() => Some(reminder(text.as_str())),
						_ => None,
					}),
			);
			continue;
		}
		let path = format!("input[{}]", message.index);
		let calls_before = open_calls.len();
		let mut blocks = Vec::with_capacity(message.blocks.len());
		for block in &message.blocks {
			blocks.extend(render_block(&path, &block.content, diagnostics));
			match &block.content {
				Content::ToolCall { id, .. } => open_calls.push(id),
				Content::ToolResult { call_id, .. } => {
					match open_calls.iter().position(|open| open == call_id) {
						Some(open) => {
							open_calls.remove(open);
						},
						None if checks_edits => {
							diagnostics.problem(format!("`{path}` answers no open tool call"));
						},
						None => {},
					}
				},
				_ => {},
			}
		}
		if blocks.is_empty() {
			continue;
		}
		match message.role {
			Role::User => {
				let results = blocks
					.iter()
					.take_while(|block| block.is_tool_result())
					.count();
				blocks.splice(results..results, reminders.drain(..));
				push_turn(&mut out, wire::Role::User, blocks);
			},
			Role::Assistant => {
				if !reminders.is_empty() {
					push_turn(&mut out, wire::Role::User, std::mem::take(&mut reminders));
				}
				let new_turn = out.last().is_some_and(|last| last.role == wire::Role::User);
				if checks_edits && new_turn && calls_before > 0 {
					diagnostics.problem(format!("`{path}` follows a tool call with no result"));
				}
				push_turn(&mut out, wire::Role::Assistant, blocks);
			},
			Role::System => unreachable!("system messages are handled above"),
		}
	}
	if !reminders.is_empty() {
		push_turn(&mut out, wire::Role::User, reminders);
	}
	if checks_edits {
		if !open_calls.is_empty() {
			let calls = open_calls.join(", ");
			diagnostics.problem(format!("tool calls with no result: {calls}"));
		}
		if out.is_empty() {
			diagnostics.problem(NOTHING_TO_SEND);
		}
	}
	out
}

fn reminder(text: &str) -> wire::Block<'static> {
	wire::Block::Known(wire::KnownBlock::Text {
		text: Cow::Owned(system_reminder(text)),
		cache_control: None,
	})
}

fn push_turn<'a>(
	out: &mut Vec<wire::Message<'a>>,
	role: wire::Role,
	mut blocks: Vec<wire::Block<'a>>,
) {
	match out.last_mut() {
		Some(last) if last.role == role => last.content.append(&mut blocks),
		_ => out.push(wire::Message {
			role,
			content: blocks,
		}),
	}
}

fn render_block<'a>(
	path: &str,
	content: &'a Content<'_>,
	diagnostics: &mut Diagnostics,
) -> Option<wire::Block<'a>> {
	let block = match content {
		// Messages refuses an empty text block here, though not in a tool result.
		Content::Text(text) if text.is_empty() => return None,
		Content::Text(text) => wire::KnownBlock::Text {
			text: Cow::Borrowed(text.as_str()),
			cache_control: None,
		},
		Content::DataImage(image) => image_block(borrowed_image(image)),
		Content::Image(fields) => image_block(owned_image(read_image(path, fields, diagnostics)?)),
		Content::ToolCall {
			id,
			name,
			namespace,
			input,
		} => wire::KnownBlock::ToolUse {
			id,
			name: flat_name(namespace.as_deref(), name),
			input: input.as_ref(),
			cache_control: None,
		},
		Content::ToolResult {
			call_id,
			failed,
			output,
		} => wire::KnownBlock::ToolResult {
			tool_use_id: call_id,
			is_error: *failed,
			content: output_blocks(path, output, diagnostics),
			cache_control: None,
		},
		Content::Reasoning { carrier, .. } => return reasoning(path, carrier, diagnostics),
	};
	Some(wire::Block::Known(block))
}

fn output_blocks<'a>(
	path: &str,
	output: &'a ToolOutput<'_>,
	diagnostics: &mut Diagnostics,
) -> Vec<wire::ResultBlock<'a>> {
	match output {
		ToolOutput::Text(text) => vec![wire::ResultBlock::Text {
			text: text.as_str(),
		}],
		ToolOutput::Mixed(parts) => parts
			.iter()
			.filter_map(|part| match part {
				ToolPart::Text(text) => Some(wire::ResultBlock::Text {
					text: text.as_str(),
				}),
				ToolPart::DataImage(image) => Some(wire::ResultBlock::Image {
					source: borrowed_image(image),
				}),
				ToolPart::Image { index, fields } => {
					read_image(&format!("{path}.content[{index}]"), fields, diagnostics).map(|image| {
						wire::ResultBlock::Image {
							source: owned_image(image),
						}
					})
				},
			})
			.collect(),
	}
}

/// Restores the provider's thinking block byte for byte from its carrier.
fn reasoning<'a>(
	path: &str,
	carrier: &'a Carrier,
	diagnostics: &mut Diagnostics,
) -> Option<wire::Block<'a>> {
	let item = match carrier.opened() {
		Ok(item) => item,
		Err(error) => {
			diagnostics.problem(format!("`{path}`: {error}"));
			return None;
		},
	};
	match provider_item(item) {
		Some(item) => Some(wire::Block::Provider(item)),
		None => {
			diagnostics.problem(format!("`{path}`: carrier contains no provider item"));
			None
		},
	}
}

fn caches_system(caching: PromptCaching, system: &[wire::SystemBlock<'_>]) -> bool {
	let words = || {
		system
			.iter()
			.map(|block| block.text.split_whitespace().count())
			.sum::<usize>()
	};
	caching.system
		&& caching
			.min_tokens
			.is_none_or(|min| words() * 13 / 10 >= min)
}

fn mark_last_system_block(mut system: Vec<wire::SystemBlock<'_>>) -> Vec<wire::SystemBlock<'_>> {
	if let Some(last) = system.last_mut() {
		last.cache_control = Some(wire::CacheControl::EPHEMERAL);
	}
	system
}

/// Messages caches only where a request marks it, and a Responses client's
/// `prompt_cache_key` has no Messages field, so without a mark long sessions lose
/// caching.
fn mark_last_user_turn(messages: &mut [wire::Message<'_>]) {
	let last = messages
		.iter_mut()
		.rev()
		.find(|message| message.role == wire::Role::User)
		.and_then(|message| message.content.last_mut());
	if let Some(block) = last {
		block.mark();
	}
}

/// Marks the last non-reasoning block in the second-to-last message, or `offset`
/// messages earlier (clamped to the first message).
fn mark_earlier_turn(messages: &mut [wire::Message<'_>], offset: usize) {
	let Some(at) = messages.len().checked_sub(2) else {
		return;
	};
	let last = messages[at.saturating_sub(offset)]
		.content
		.iter_mut()
		.rev()
		.find(|block| matches!(block, wire::Block::Known(_)));
	if let Some(block) = last {
		block.mark();
	}
}

const fn image_block(source: wire::ImageSource<'_>) -> wire::KnownBlock<'_> {
	wire::KnownBlock::Image {
		source,
		cache_control: None,
	}
}

fn borrowed_image<'a>(image: &'a DataImage<'_>) -> wire::ImageSource<'a> {
	wire::ImageSource {
		kind: wire::Base64Type::Base64,
		media_type: Cow::Borrowed(&image.media_type),
		data: Cow::Borrowed(&image.data),
	}
}

fn owned_image(image: DataImage<'static>) -> wire::ImageSource<'static> {
	wire::ImageSource {
		kind: wire::Base64Type::Base64,
		media_type: Cow::Owned(image.media_type),
		data: image.data,
	}
}

mod wire {
	use std::borrow::Cow;
	use std::num::NonZeroU64;

	use serde::Serialize;
	use serde_json::value::RawValue;

	use crate::json::JsonRef;

	#[derive(Serialize)]
	pub(super) struct Request<'a> {
		pub(super) model: &'a str,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) max_tokens: Option<NonZeroU64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) system: Option<Vec<SystemBlock<'a>>>,
		pub(super) messages: Vec<Message<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) tools: Option<Vec<Tool<'a>>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) tool_choice: Option<ToolChoice<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) top_k: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) temperature: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) top_p: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) stop_sequences: Option<&'a [String]>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) thinking: Option<Thinking>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) output_config: Option<OutputConfig<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) context_management: Option<ContextManagement>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) stream: Option<bool>,
	}

	#[derive(Serialize)]
	pub(super) struct SystemBlock<'a> {
		#[serde(rename = "type")]
		pub(super) kind: TextType,
		pub(super) text: &'a str,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) cache_control: Option<CacheControl>,
	}

	#[derive(Serialize)]
	pub(super) struct Message<'a> {
		pub(super) role: Role,
		pub(super) content: Vec<Block<'a>>,
	}

	#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum Role {
		User,
		Assistant,
	}

	#[derive(Serialize)]
	#[serde(untagged)]
	pub(super) enum Block<'a> {
		Known(KnownBlock<'a>),
		/// A provider thinking block, restored byte for byte from its carrier.
		Provider(&'a RawValue),
	}

	impl Block<'_> {
		pub(super) const fn is_tool_result(&self) -> bool {
			matches!(self, Self::Known(KnownBlock::ToolResult { .. }))
		}

		pub(super) const fn mark(&mut self) {
			match self {
				Self::Known(block) => block.mark(),
				// Provider reasoning cannot be cache-marked. `prompt::after_edits` rejects it
				// in user turns.
				Self::Provider(_) => {},
			}
		}
	}

	#[derive(Serialize)]
	#[serde(tag = "type", rename_all = "snake_case")]
	pub(super) enum KnownBlock<'a> {
		Text {
			text: Cow<'a, str>,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControl>,
		},
		Image {
			source: ImageSource<'a>,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControl>,
		},
		ToolUse {
			id: &'a str,
			name: String,
			input: JsonRef<'a>,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControl>,
		},
		ToolResult {
			tool_use_id: &'a str,
			#[serde(skip_serializing_if = "Option::is_none")]
			is_error: Option<bool>,
			content: Vec<ResultBlock<'a>>,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControl>,
		},
	}

	impl KnownBlock<'_> {
		const fn mark(&mut self) {
			let (Self::Text { cache_control, .. }
			| Self::Image { cache_control, .. }
			| Self::ToolUse { cache_control, .. }
			| Self::ToolResult { cache_control, .. }) = self;
			*cache_control = Some(CacheControl::EPHEMERAL);
		}
	}

	#[derive(Serialize)]
	#[serde(tag = "type", rename_all = "snake_case")]
	pub(super) enum ResultBlock<'a> {
		Text { text: &'a str },
		Image { source: ImageSource<'a> },
	}

	#[derive(Serialize)]
	pub(super) struct ImageSource<'a> {
		#[serde(rename = "type")]
		pub(super) kind: Base64Type,
		pub(super) media_type: Cow<'a, str>,
		pub(super) data: Cow<'a, str>,
	}

	#[derive(Serialize)]
	pub(super) struct Tool<'a> {
		pub(super) name: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) description: Option<Cow<'a, str>>,
		pub(super) input_schema: JsonRef<'a>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) strict: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) cache_control: Option<CacheControl>,
	}

	#[derive(Serialize)]
	pub(super) struct ToolChoice<'a> {
		#[serde(rename = "type")]
		pub(super) kind: ToolChoiceType,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) name: Option<&'a str>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) disable_parallel_tool_use: Option<bool>,
	}

	#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum ToolChoiceType {
		Auto,
		Any,
		None,
		Tool,
	}

	#[derive(Serialize)]
	pub(super) struct Thinking {
		#[serde(rename = "type")]
		pub(super) kind: ThinkingType,
		pub(super) display: ThinkingDisplay,
	}

	#[derive(Serialize)]
	pub(super) struct OutputConfig<'a> {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) effort: Option<Effort>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) format: Option<Format<'a>>,
	}

	#[derive(Serialize)]
	pub(super) struct Format<'a> {
		#[serde(rename = "type")]
		pub(super) kind: JsonSchemaType,
		pub(super) schema: JsonRef<'a>,
	}

	#[derive(Serialize)]
	pub(super) struct ContextManagement {
		pub(super) edits: [ContextEdit; 1],
	}

	#[derive(Serialize)]
	pub(super) struct ContextEdit {
		#[serde(rename = "type")]
		pub(super) kind: ContextEditType,
		pub(super) keep: Keep,
	}

	#[derive(Serialize, Clone, Copy)]
	pub(super) struct CacheControl {
		#[serde(rename = "type")]
		kind: CacheType,
	}

	impl CacheControl {
		pub(super) const EPHEMERAL: Self = Self {
			kind: CacheType::Ephemeral,
		};
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	enum CacheType {
		Ephemeral,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum TextType {
		Text,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum Base64Type {
		Base64,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum ThinkingType {
		Adaptive,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum ThinkingDisplay {
		Summarized,
		Omitted,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum Effort {
		Low,
		Medium,
		High,
		Xhigh,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum JsonSchemaType {
		JsonSchema,
	}

	#[derive(Serialize, Clone, Copy)]
	pub(super) enum ContextEditType {
		#[serde(rename = "clear_thinking_20251015")]
		ClearThinking,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum Keep {
		All,
	}
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
