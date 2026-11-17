//! `RenderSettings::ChatCompletions` maps the request's reasoning effort to
//! the model's accepted levels after edits. Missing facts use model-name defaults.

use std::borrow::Cow;
use std::collections::HashSet;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::carrier::IdentifierPseudonymizer;
use crate::ir::conversation::{
	BlockId, Carrier, Content, Conversation, DataImage, Effort, ImageFields, Message, OutputEffort,
	OutputFormat, Role, Settings, Text, Tool, ToolChoice, ToolKind, ToolOutput, ToolPart,
};
use crate::json::{JsonRef, text_of};
use crate::prompt::diagnostics::{Diagnostics, Unread};
use crate::prompt::{NOTHING_TO_SEND, ResolvedCapabilities, TOOL_ERROR_MARKER};
use crate::protocols::reasoning::ReasoningControls;
use crate::protocols::{
	CallKind, ToolRoute, check_sampling, check_tool_choice, custom_description, flat_name,
	read_image, short_identifier, tool_declarations,
};

const MAX_TOOL_NAME: usize = 64;
/// OpenAI's name for a format that came with no name of its own.
const FORMAT_NAME: &str = "structured_output";
pub(crate) struct Options<'a> {
	pub(crate) identifier_pseudonymizer: &'a dyn IdentifierPseudonymizer,
	pub(crate) model: &'a str,
	pub(crate) reasoning: ReasoningControls,
	pub(crate) capabilities: ResolvedCapabilities,
}

pub(crate) fn render(
	conversation: &Conversation<'_>,
	unread: &Unread<'_>,
	diagnostics: Diagnostics,
	options: Options<'_>,
) -> (String, Vec<ToolRoute>, Diagnostics) {
	let mut renderer = Renderer {
		options,
		diagnostics,
		messages: Vec::new(),
		has_turn: false,
		routes: Vec::new(),
		seen_calls: HashSet::new(),
		unanswered_calls: Vec::new(),
	};
	for image in &unread.images {
		renderer.refuse_image(&image.path);
	}
	let body = match renderer.request(conversation) {
		Ok(body) => body,
		Err(error) => {
			renderer.diagnostics.problem(format!(
				"could not serialize Chat Completions request: {error}"
			));
			String::new()
		},
	};
	(body, renderer.routes, renderer.diagnostics)
}

struct Renderer<'a, 'c> {
	options: Options<'c>,
	diagnostics: Diagnostics,
	messages: Vec<wire::Message<'a>>,
	/// A provider refuses a request that's only system messages.
	has_turn: bool,
	routes: Vec<ToolRoute>,
	seen_calls: HashSet<&'a str>,
	unanswered_calls: Vec<&'a str>,
}

/// A carrier's reasoning fields, read only to put them back.
#[derive(Deserialize)]
struct Reasoning<'a> {
	#[serde(borrow)]
	reasoning_content: Option<&'a RawValue>,
	#[serde(borrow)]
	reasoning: Option<&'a RawValue>,
	#[serde(borrow)]
	reasoning_signature: Option<&'a RawValue>,
}

/// A user text stands alone as a string, but becomes a typed part beside images.
enum Part<'a> {
	Text(&'a str),
	Rendered(wire::UserPart<'a>),
}

#[derive(Default)]
struct OutputConfig<'a> {
	effort: Option<Effort>,
	format: Option<wire::ResponseFormat<'a>>,
}

struct RequestSettings<'a> {
	stream: Option<bool>,
	stream_options: Option<wire::StreamOptions>,
	stop: Option<&'a [String]>,
	reasoning_effort: Option<wire::ReasoningEffort>,
	response_format: Option<wire::ResponseFormat<'a>>,
	user: Option<String>,
}

#[derive(Default)]
struct ReasoningFields<'a> {
	reasoning_content: Option<Cow<'a, str>>,
	reasoning: Option<Cow<'a, str>>,
	reasoning_signature: Option<&'a RawValue>,
}

impl<'a> Renderer<'a, '_> {
	fn request(&mut self, conversation: &'a Conversation<'_>) -> serde_json::Result<String> {
		let settings = &conversation.settings;
		self.system(conversation.system.iter().flatten().map(Text::as_str));
		self.messages(&conversation.messages);
		self.drop_settings(settings);
		check_tool_choice(settings, &conversation.tools, &mut self.diagnostics);
		let tools = self.tools(&conversation.tools);
		let config = self.output_config(settings);
		let fields = self.settings(settings, config, !conversation.tools.is_empty());
		let has_tools = !conversation.tools.is_empty();
		let request = wire::Request {
			model: self.options.model.to_string(),
			messages: std::mem::take(&mut self.messages),
			tools: has_tools.then_some(tools),
			tool_choice: has_tools
				.then(|| tool_choice(settings.tool_choice.as_ref()))
				.flatten(),
			parallel_tool_calls: has_tools.then_some(settings.parallel_tool_calls).flatten(),
			stream: fields.stream,
			stream_options: fields.stream_options,
			max_completion_tokens: settings.max_tokens,
			temperature: settings.temperature,
			top_p: settings.top_p,
			stop: fields.stop,
			reasoning_effort: fields.reasoning_effort,
			response_format: fields.response_format,
			user: fields.user,
		};
		serde_json::to_string(&request)
	}

	fn system(&mut self, texts: impl Iterator<Item = &'a str>) {
		let texts: Vec<&str> = texts.filter(|text| !text.is_empty()).collect();
		if let Some(content) = joined(&texts, "\n\n") {
			self.messages.push(wire::Message::System { content });
		}
	}

	fn messages(&mut self, messages: &'a [Message<'_>]) {
		for message in messages {
			if message.messages_output_config.is_some() {
				self.diagnostics.drop_field("output_config");
			}
			match message.role {
				Role::User => self.user_message(message),
				Role::Assistant => self.assistant_message(message),
				Role::System => {
					self.check_answered();
					let texts = message
						.blocks
						.iter()
						.filter_map(|block| match &block.content {
							Content::Text(text) => Some(text.as_str()),
							_ => None,
						});
					self.system(texts);
				},
			}
		}
		self.check_answered();
		if !self.has_turn && self.diagnostics.problems.is_empty() {
			self.diagnostics.problem(NOTHING_TO_SEND);
		}
	}

	/// OpenAI wants every call to have its result before anything else follows it.
	fn check_answered(&mut self) {
		if !self.unanswered_calls.is_empty() {
			let calls = std::mem::take(&mut self.unanswered_calls).join(", ");
			self
				.diagnostics
				.problem(format!("tool calls with no result: {calls}"));
		}
	}

	/// Results come first, each a message of its own. A `tool` message contains only
	/// text, so a result's images go in the user message after them.
	fn user_message(&mut self, message: &'a Message<'_>) {
		let mut parts = Vec::new();
		let mut images = Vec::new();
		for block in &message.blocks {
			match &block.content {
				Content::Text(text) => parts.push(Part::Text(text.as_str())),
				Content::ToolResult {
					call_id,
					failed,
					output,
				} => {
					self.tool_result(
						block.id,
						call_id,
						failed.unwrap_or(false),
						output,
						&mut images,
					);
				},
				Content::Image(fields) => {
					parts.extend(self.image(&block.id.path(), fields).map(Part::Rendered));
				},
				Content::DataImage(image) => parts.push(Part::Rendered(data_image_part(image))),
				Content::ToolCall { .. } | Content::Reasoning { .. } => {},
			}
		}
		parts.extend(images.into_iter().map(Part::Rendered));
		if parts.is_empty() {
			return;
		}
		self.check_answered();
		self.has_turn = true;
		let content = match parts.as_slice() {
			[Part::Text(text)] => wire::UserContent::Text(Cow::Borrowed(text)),
			_ => wire::UserContent::Parts(
				parts
					.into_iter()
					.map(|part| match part {
						Part::Text(text) => wire::UserPart::Text {
							text: Cow::Borrowed(text),
						},
						Part::Rendered(part) => part,
					})
					.collect(),
			),
		};
		self.messages.push(wire::Message::User { content });
	}

	fn image(&mut self, path: &str, fields: &ImageFields<'_>) -> Option<wire::UserPart<'a>> {
		let image = read_image(path, fields, &mut self.diagnostics)?;
		Some(image_part(&image.media_type, &image.data))
	}

	fn refuse_image(&mut self, path: &str) {
		self
			.diagnostics
			.problem(format!("`{path}` block type `image`"));
	}

	fn tool_result(
		&mut self,
		id: BlockId,
		call: &'a str,
		failed: bool,
		output: &'a ToolOutput<'_>,
		images: &mut Vec<wire::UserPart<'a>>,
	) {
		let path = id.path();
		let Some(open) = self.unanswered_calls.iter().position(|id| *id == call) else {
			self
				.diagnostics
				.problem(format!("`{path}` has no call waiting for it"));
			return;
		};
		self.unanswered_calls.remove(open);
		let mut texts = Vec::new();
		match output {
			ToolOutput::Text(output) => texts.push(output.as_str()),
			ToolOutput::Mixed(parts) => {
				for part in parts {
					match part {
						ToolPart::Image { index, fields } => {
							images.extend(self.image(&format!("{path}.content[{index}]"), fields));
						},
						ToolPart::DataImage(image) => images.push(data_image_part(image)),
						ToolPart::Text(text) => texts.push(text.as_str()),
					}
				}
			},
		}
		self.has_turn = true;
		self.messages.push(wire::Message::Tool {
			tool_call_id: call,
			content: joined_result(&texts, failed),
		});
	}

	fn assistant_message(&mut self, message: &'a Message<'_>) {
		self.check_answered();
		let mut texts = Vec::new();
		let mut calls = Vec::new();
		let mut reasoning = Vec::new();
		for block in &message.blocks {
			match &block.content {
				Content::Text(text) => texts.push(text.as_str()),
				Content::Reasoning { carrier, .. } => {
					self.reasoning(block.id, carrier, &mut reasoning);
				},
				Content::ToolCall {
					id,
					name,
					namespace,
					input,
				} => {
					let name = flat_name(namespace.as_deref(), name);
					calls.extend(self.call(block.id, id, &name, input.as_ref()));
				},
				Content::ToolResult { .. } | Content::Image(_) | Content::DataImage(_) => {},
			}
		}
		if texts.is_empty() && calls.is_empty() && reasoning.is_empty() {
			return;
		}
		self.has_turn = true;
		let content = joined(&texts, "");
		let reasoning = self.reasoning_fields(&reasoning);
		self.messages.push(wire::Message::Assistant {
			content,
			reasoning_content: reasoning.reasoning_content,
			reasoning: reasoning.reasoning,
			reasoning_signature: reasoning.reasoning_signature,
			tool_calls: calls,
		});
	}

	fn reasoning(&mut self, id: BlockId, carrier: &'a Carrier, out: &mut Vec<&'a [u8]>) {
		match carrier.opened() {
			Ok(item) => out.push(item),
			Err(error) => {
				self
					.diagnostics
					.problem(format!("`{}`: {error}", id.path()));
			},
		}
	}

	/// A turn has one reasoning text. A signature vouches for one part, so it stays
	/// only if there was just one.
	fn reasoning_fields(&mut self, items: &[&'a [u8]]) -> ReasoningFields<'a> {
		let mut field = None;
		let mut texts = Vec::new();
		let mut signature = None;
		for item in items {
			let parsed = serde_json::from_slice::<Reasoning<'_>>(item);
			let Ok(parsed) = parsed else {
				self.diagnostics.problem("carrier contains no reasoning");
				return ReasoningFields::default();
			};
			let (name, text) = match (parsed.reasoning_content, parsed.reasoning) {
				(Some(text), _) => ("reasoning_content", text),
				(None, Some(text)) => ("reasoning", text),
				(None, None) => {
					self.diagnostics.problem("carrier contains no reasoning");
					return ReasoningFields::default();
				},
			};
			field.get_or_insert(name);
			texts.extend(text_of(text).map(|text| text.into_owned()));
			signature = parsed.reasoning_signature;
		}
		let Some(field) = field else {
			return ReasoningFields::default();
		};
		let text = Some(Cow::Owned(texts.join("\n\n")));
		let mut fields = ReasoningFields {
			reasoning_signature: signature.filter(|_| items.len() == 1),
			..ReasoningFields::default()
		};
		if field == "reasoning_content" {
			fields.reasoning_content = text;
		} else {
			fields.reasoning = text;
		}
		fields
	}

	fn call(
		&mut self,
		block: BlockId,
		id: &'a str,
		name: &str,
		input: JsonRef<'a>,
	) -> Option<wire::ToolCall<'a>> {
		if self.seen_calls.contains(id) {
			self
				.diagnostics
				.problem(format!("`{}` repeats a tool call id", block.path()));
			return None;
		}
		self.seen_calls.insert(id);
		self.unanswered_calls.push(id);
		Some(wire::ToolCall {
			id,
			kind: wire::FunctionType::Function,
			function: wire::CalledFunction {
				name: name.to_string(),
				arguments: input.encoded(),
			},
		})
	}

	fn drop_settings(&mut self, settings: &Settings<'_>) {
		check_sampling(settings, &mut self.diagnostics);
		if settings.top_k.is_some() {
			self.diagnostics.problem("setting `top_k`");
		}
		let thinking = settings.messages_input.thinking.as_ref();
		let fields = [
			(
				settings.messages_input.context_management.is_some(),
				"context_management",
			),
			(settings.show_reasoning_summary, "reasoning.summary"),
			(settings.keep_earlier_reasoning, "reasoning.context"),
			(
				thinking.is_some_and(|thinking| thinking.budget_tokens.is_some()),
				"thinking.budget_tokens",
			),
		];
		for (present, field) in fields {
			if present {
				self.diagnostics.drop_field(field);
			}
		}
	}

	fn tools(&mut self, tools: &'a [Tool<'_>]) -> Vec<wire::Tool<'a>> {
		let mut rendered = Vec::with_capacity(tools.len());
		for tool in tool_declarations(tools) {
			rendered.extend(self.tool(tool));
		}
		rendered
	}

	fn tool(&mut self, tool: &'a Tool<'_>) -> Option<wire::Tool<'a>> {
		let flat = self.tool_name(tool)?;
		let (strict, description) = match &tool.kind {
			ToolKind::Function { strict } => (*strict, tool.description.clone()),
			ToolKind::Custom { grammar } => {
				let description = custom_description(tool, grammar.as_deref(), &mut self.diagnostics);
				(None, Some(description))
			},
			ToolKind::ToolSearch => (None, tool.description.clone()),
		};
		Some(wire::Tool {
			kind: wire::FunctionType::Function,
			function: wire::FunctionTool {
				name: flat,
				description: description.filter(|text| !text.is_empty()),
				parameters: tool.schema.as_ref(),
				strict,
			},
		})
	}

	fn tool_name(&mut self, tool: &Tool<'_>) -> Option<String> {
		let flat = flat_name(tool.namespace.as_deref(), &tool.name);
		let valid = !flat.is_empty()
			&& flat.len() <= MAX_TOOL_NAME
			&& flat
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte));
		if !valid {
			self
				.diagnostics
				.problem(format!("`{}` name `{flat}` doesn't fit", tool.path));
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

	/// A Messages client says effort and format in `output_config`.
	fn output_config(&mut self, settings: &'a Settings<'_>) -> OutputConfig<'a> {
		let mut config = OutputConfig::default();
		let Some(raw) = settings.messages_input.output_config else {
			return config;
		};
		let Some(fields) = self.diagnostics.object("output_config", raw) else {
			return config;
		};
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"effort" => config.effort = self.effort_word(*value),
				"format" => config.format = self.format(*value),
				other => self
					.diagnostics
					.problem(format!("field `output_config.{other}`")),
			}
		}
		config
	}

	fn effort_word(&mut self, raw: JsonRef<'_>) -> Option<Effort> {
		match text_of(raw).as_deref().and_then(OutputEffort::read) {
			Some(effort) => Some(effort.effort()),
			None => {
				self.diagnostics.problem("`output_config.effort`");
				None
			},
		}
	}

	fn format(&mut self, raw: JsonRef<'a>) -> Option<wire::ResponseFormat<'a>> {
		let fields = self.diagnostics.object("output_config.format", raw)?;
		let schema = fields.0.iter().find(|(name, _)| name == "schema");
		match (fields.kind().as_deref(), schema) {
			(Some("json_schema"), Some((_, schema))) => Some(Self::schema_format(*schema)),
			_ => {
				self.diagnostics.problem("`output_config.format`");
				None
			},
		}
	}

	fn settings(
		&mut self,
		settings: &'a Settings<'_>,
		config: OutputConfig<'a>,
		has_tools: bool,
	) -> RequestSettings<'a> {
		let stream = (settings.stream == Some(true)).then_some(true);
		let stream_options = stream.map(|_| wire::StreamOptions {
			include_usage: true,
		});
		let stop = self.stop_sequences(&settings.stop_sequences);
		let reasoning_effort = self.effort(settings.effort.or(config.effort), has_tools);
		let response_format = match &settings.output_format {
			Some(OutputFormat::Json) => Some(wire::ResponseFormat::JsonObject),
			Some(OutputFormat::JsonSchema(schema)) => Some(Self::schema_format(schema.as_ref())),
			None => config.format,
		};
		let user = settings
			.messages_input
			.metadata
			.and_then(|raw| self.metadata(raw));
		RequestSettings {
			stream,
			stream_options,
			stop,
			reasoning_effort,
			response_format,
			user,
		}
	}

	const fn stop_sequences(&self, sequences: &'a [String]) -> Option<&'a [String]> {
		if sequences.is_empty() {
			return None;
		}
		if !self.options.capabilities.native_stop_sequences {
			return None;
		}
		Some(sequences)
	}

	/// The request's effort mapped onto the reasoning controls, model defaults and
	/// what the model allows beside tools.
	fn effort(
		&mut self,
		requested: Option<Effort>,
		has_tools: bool,
	) -> Option<wire::ReasoningEffort> {
		let chosen = ReasoningEffort::for_controls(
			requested,
			has_tools,
			self.options.capabilities,
			self.options.reasoning,
		);
		let constrained_by_tools = has_tools && !self.options.capabilities.reasoning_with_tools;
		match chosen {
			ReasoningEffort::Omit if requested.is_some() => self.diagnostics.drop_field("effort"),
			ReasoningEffort::Send(Effort::Off)
				if constrained_by_tools && requested != Some(Effort::Off) =>
			{
				self.diagnostics.drop_field("effort beside tools");
			},
			_ => {},
		}
		let ReasoningEffort::Send(effort) = chosen else {
			return None;
		};
		Some(match effort {
			Effort::Off => wire::ReasoningEffort::None,
			Effort::Minimal => wire::ReasoningEffort::Minimal,
			Effort::Low => wire::ReasoningEffort::Low,
			Effort::Medium => wire::ReasoningEffort::Medium,
			Effort::High => wire::ReasoningEffort::High,
			Effort::XHigh => wire::ReasoningEffort::Xhigh,
		})
	}

	/// Other metadata needs `store: true` on OpenAI, which this never sets.
	fn metadata(&mut self, raw: JsonRef<'_>) -> Option<String> {
		let fields = self.diagnostics.object("metadata", raw)?;
		let mut user = None;
		for (name, value) in &fields.0 {
			match name.as_ref() {
				"user_id" => {
					user = self
						.diagnostics
						.typed::<String>("metadata.user_id", value)
						.map(|user| short_identifier(self.options.identifier_pseudonymizer, user));
				},
				other => self
					.diagnostics
					.problem(format!("field `metadata.{other}`")),
			}
		}
		user
	}

	const fn schema_format(schema: JsonRef<'a>) -> wire::ResponseFormat<'a> {
		wire::ResponseFormat::JsonSchema {
			json_schema: wire::JsonSchema {
				name: FORMAT_NAME,
				schema,
			},
		}
	}
}

fn tool_choice(choice: Option<&ToolChoice>) -> Option<wire::ToolChoice> {
	match choice {
		Some(ToolChoice::Auto) => Some(wire::ToolChoice::Mode(wire::ToolChoiceMode::Auto)),
		Some(ToolChoice::Required) => Some(wire::ToolChoice::Mode(wire::ToolChoiceMode::Required)),
		Some(ToolChoice::None) => Some(wire::ToolChoice::Mode(wire::ToolChoiceMode::None)),
		Some(ToolChoice::Tool(name)) => Some(wire::ToolChoice::Function {
			kind: wire::FunctionType::Function,
			function: wire::ChosenFunction { name: name.clone() },
		}),
		None => None,
	}
}

fn data_image_part(image: &DataImage<'_>) -> wire::UserPart<'static> {
	image_part(&image.media_type, &image.data)
}

fn image_part(media_type: &str, data: &str) -> wire::UserPart<'static> {
	wire::UserPart::ImageUrl {
		image_url: wire::ImageUrl {
			url: format!("data:{media_type};base64,{data}"),
		},
	}
}

fn joined<'a>(texts: &[&'a str], separator: &str) -> Option<Cow<'a, str>> {
	match texts {
		[] => None,
		[only] => Some(Cow::Borrowed(only)),
		_ => Some(Cow::Owned(texts.join(separator))),
	}
}

fn joined_result<'a>(texts: &[&'a str], failed: bool) -> Cow<'a, str> {
	if !failed {
		return joined(texts, "\n").unwrap_or_default();
	}
	let mut output = String::from(TOOL_ERROR_MARKER);
	if !texts.is_empty() {
		output.push_str(&texts.join("\n"));
	}
	Cow::Owned(output)
}

/// Whether and how Chat Completions sends reasoning effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReasoningEffort {
	Omit,
	/// `Off` is sent as `none`.
	Send(Effort),
}

impl ReasoningEffort {
	fn for_controls(
		requested: Option<Effort>,
		has_tools: bool,
		capabilities: ResolvedCapabilities,
		controls: ReasoningControls,
	) -> Self {
		let Some(effort) = requested else {
			return Self::Omit;
		};
		let Some(effort) = controls.effort(effort, capabilities.default_reasoning_effort) else {
			return Self::Omit;
		};
		if has_tools && !capabilities.reasoning_with_tools {
			return if controls.accepts(Effort::Off) {
				Self::Send(Effort::Off)
			} else {
				Self::Omit
			};
		}
		Self::Send(effort)
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
		pub(super) model: String,
		pub(super) messages: Vec<Message<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) tools: Option<Vec<Tool<'a>>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) tool_choice: Option<ToolChoice>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) parallel_tool_calls: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) stream: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) stream_options: Option<StreamOptions>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) max_completion_tokens: Option<NonZeroU64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) temperature: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) top_p: Option<JsonRef<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) stop: Option<&'a [String]>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) reasoning_effort: Option<ReasoningEffort>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) response_format: Option<ResponseFormat<'a>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) user: Option<String>,
	}

	#[derive(Serialize)]
	#[serde(tag = "role", rename_all = "snake_case")]
	pub(super) enum Message<'a> {
		System {
			content: Cow<'a, str>,
		},
		User {
			content: UserContent<'a>,
		},
		Tool {
			tool_call_id: &'a str,
			content: Cow<'a, str>,
		},
		Assistant {
			content: Option<Cow<'a, str>>,
			#[serde(skip_serializing_if = "Option::is_none")]
			reasoning_content: Option<Cow<'a, str>>,
			#[serde(skip_serializing_if = "Option::is_none")]
			reasoning: Option<Cow<'a, str>>,
			#[serde(skip_serializing_if = "Option::is_none")]
			reasoning_signature: Option<&'a RawValue>,
			#[serde(skip_serializing_if = "Vec::is_empty")]
			tool_calls: Vec<ToolCall<'a>>,
		},
	}

	#[derive(Serialize)]
	#[serde(untagged)]
	pub(super) enum UserContent<'a> {
		Text(Cow<'a, str>),
		Parts(Vec<UserPart<'a>>),
	}

	#[derive(Serialize)]
	#[serde(tag = "type", rename_all = "snake_case")]
	pub(super) enum UserPart<'a> {
		Text { text: Cow<'a, str> },
		ImageUrl { image_url: ImageUrl },
	}

	#[derive(Serialize)]
	pub(super) struct ImageUrl {
		pub(super) url: String,
	}

	#[derive(Serialize)]
	pub(super) struct ToolCall<'a> {
		pub(super) id: &'a str,
		#[serde(rename = "type")]
		pub(super) kind: FunctionType,
		pub(super) function: CalledFunction<'a>,
	}

	#[derive(Serialize)]
	pub(super) struct CalledFunction<'a> {
		pub(super) name: String,
		pub(super) arguments: Cow<'a, str>,
	}

	#[derive(Serialize)]
	pub(super) struct Tool<'a> {
		#[serde(rename = "type")]
		pub(super) kind: FunctionType,
		pub(super) function: FunctionTool<'a>,
	}

	#[derive(Serialize)]
	pub(super) struct FunctionTool<'a> {
		pub(super) name: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) description: Option<String>,
		pub(super) parameters: JsonRef<'a>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub(super) strict: Option<bool>,
	}

	#[derive(Serialize, Clone, Copy)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum FunctionType {
		Function,
	}

	#[derive(Serialize)]
	#[serde(untagged)]
	pub(super) enum ToolChoice {
		Mode(ToolChoiceMode),
		Function {
			#[serde(rename = "type")]
			kind: FunctionType,
			function: ChosenFunction,
		},
	}

	#[derive(Serialize)]
	pub(super) struct ChosenFunction {
		pub(super) name: String,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum ToolChoiceMode {
		Auto,
		Required,
		None,
	}

	#[derive(Serialize)]
	pub(super) struct StreamOptions {
		pub(super) include_usage: bool,
	}

	#[derive(Serialize)]
	#[serde(rename_all = "snake_case")]
	pub(super) enum ReasoningEffort {
		None,
		Minimal,
		Low,
		Medium,
		High,
		Xhigh,
	}

	#[derive(Serialize)]
	#[serde(tag = "type", rename_all = "snake_case")]
	pub(super) enum ResponseFormat<'a> {
		JsonObject,
		JsonSchema { json_schema: JsonSchema<'a> },
	}

	#[derive(Serialize)]
	pub(super) struct JsonSchema<'a> {
		pub(super) name: &'static str,
		pub(super) schema: JsonRef<'a>,
	}
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
