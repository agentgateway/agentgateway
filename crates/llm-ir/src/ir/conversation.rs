//! Protocol-neutral request model between client parsing and provider rendering.
//!
//! A [`Conversation`] contains [`Settings`], tools, and ordered messages. Messages
//! contain stable [`BlockId`] values and typed [`Content`]. Edits change neutral
//! fields; client-specific fields are retained only for validation or removal.

use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU64;

use crate::carrier::CarrierError;
use crate::json::{Fields, Json, JsonRef};

/// A protocol-neutral request ready for edits or provider rendering.
pub struct Conversation<'a> {
	pub settings: Settings<'a>,
	/// `None` if the client sent none, which is not the same as an empty list.
	pub system: Option<Vec<Text<'a>>>,
	pub tools: Vec<Tool<'a>>,
	pub messages: Vec<Message<'a>>,
	/// Where a Messages client asked the prompt cache to end, with `cache_control`.
	pub cache_breakpoints: Vec<CacheBreakpoint>,
}

/// A breakpoint inside a tool result belongs to the whole result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheBreakpoint {
	/// Its place in `Conversation::system`.
	System(usize),
	Block(BlockId),
}

/// Request settings retained until the target renderer accepts or refuses them.
#[derive(Default)]
pub struct Settings<'a> {
	pub max_tokens: Option<NonZeroU64>,
	/// Not yet read as a number: a target that refuses the field reports only that.
	pub temperature: Option<JsonRef<'a>>,
	pub top_p: Option<JsonRef<'a>>,
	pub top_k: Option<JsonRef<'a>>,
	pub stop_sequences: Vec<String>,
	pub stream: Option<bool>,
	/// The client asked for usage at the end of a stream, in a format that sends it only
	/// when asked.
	pub include_usage: bool,
	pub tool_choice: Option<ToolChoice>,
	pub parallel_tool_calls: Option<bool>,
	/// An explicit edit overrides client effort, including per-turn effort and
	/// disabled thinking. Other client thinking options are still validated.
	pub effort: Option<Effort>,
	pub output_format: Option<OutputFormat<'a>>,
	pub show_reasoning_summary: bool,
	pub keep_earlier_reasoning: bool,
	pub messages_input: MessagesInputSettings<'a>,
}

/// Messages-only input retained for target-specific validation.
#[derive(Default)]
pub struct MessagesInputSettings<'a> {
	pub thinking: Option<MessagesThinking<'a>>,
	pub output_config: Option<JsonRef<'a>>,
	pub context_management: Option<JsonRef<'a>>,
	pub metadata: Option<JsonRef<'a>>,
}

/// How the model may choose a declared tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolChoice {
	Auto,
	Required,
	None,
	Tool(String),
}

/// Protocol-neutral reasoning effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
	Off,
	Minimal,
	Low,
	Medium,
	High,
	XHigh,
}

impl Effort {
	/// The effort a word names, as Chat Completions and Responses spell it.
	#[must_use]
	pub fn read(word: &str) -> Option<Self> {
		match word {
			"none" => Some(Self::Off),
			"minimal" => Some(Self::Minimal),
			"low" => Some(Self::Low),
			"medium" => Some(Self::Medium),
			"high" => Some(Self::High),
			"xhigh" => Some(Self::XHigh),
			_ => None,
		}
	}
}

/// A word a Messages client can put in `output_config.effort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputEffort {
	Low,
	Medium,
	High,
	XHigh,
	Max,
}

impl OutputEffort {
	#[must_use]
	pub fn read(word: &str) -> Option<Self> {
		match word {
			"low" => Some(Self::Low),
			"medium" => Some(Self::Medium),
			"high" => Some(Self::High),
			"xhigh" => Some(Self::XHigh),
			"max" => Some(Self::Max),
			_ => None,
		}
	}

	#[must_use]
	pub const fn effort(self) -> Effort {
		match self {
			Self::Low => Effort::Low,
			Self::Medium => Effort::Medium,
			Self::High => Effort::High,
			Self::XHigh | Self::Max => Effort::XHigh,
		}
	}
}

/// The requested structured output shape.
pub enum OutputFormat<'a> {
	Json,
	JsonSchema(Json<'a>),
}

/// Messages thinking fields retained for target-specific rendering.
pub struct MessagesThinking<'a> {
	pub mode: Option<MessagesThinkingMode>,
	pub display: Option<MessagesThinkingDisplay>,
	pub budget_tokens: Option<JsonRef<'a>>,
}

/// How a Messages request enables thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagesThinkingMode {
	Adaptive,
	Enabled,
	Disabled,
}

/// Whether a Messages client wants a visible reasoning summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagesThinkingDisplay {
	Summarized,
	Omitted,
}

/// A tool declaration in the protocol-neutral request.
pub struct Tool<'a> {
	/// Where the client declared it, for reporting.
	pub path: String,
	pub name: String,
	pub namespace: Option<String>,
	pub description: Option<String>,
	/// Unchecked: each target decides which schemas it accepts. Owned when a parser
	/// had to rewrite the client's schema into JSON Schema.
	pub schema: Json<'a>,
	pub kind: ToolKind,
}

/// The input and result shape of a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolKind {
	Function {
		strict: Option<bool>,
	},
	/// Takes free text, sent as the one `input` field of `schema`.
	Custom {
		grammar: Option<String>,
	},
	/// Codex's search for tools it hasn't sent yet. Codex performs the search and
	/// replies with the tools it found, which then join the list.
	ToolSearch,
}

/// A message role understood by every supported protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
	User,
	Assistant,
	System,
}

/// One ordered request message.
pub struct Message<'a> {
	/// Its place in the client's list, for reporting.
	pub index: usize,
	pub role: Role,
	pub blocks: Vec<Block<'a>>,
	/// Raw client configuration. Edits may read or clear it, but should use
	/// `Settings::effort` to change effort.
	pub messages_output_config: Option<JsonRef<'a>>,
}

/// A block's place in the client's request. It doesn't change when other blocks do,
/// so a change can name what it touched and a replay link still works when blocks move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockId {
	pub message: usize,
	pub block: usize,
}

impl BlockId {
	#[must_use]
	pub fn path(&self) -> String {
		format!("messages[{}].content[{}]", self.message, self.block)
	}
}

/// One content block with its stable request location.
pub struct Block<'a> {
	pub id: BlockId,
	pub content: Content<'a>,
}

/// The protocol-neutral content of one message block.
///
/// Text and JSON borrow from the request until an edit replaces them.
pub enum Content<'a> {
	Text(Text<'a>),
	Image(ImageFields<'a>),
	DataImage(DataImage<'a>),
	ToolCall {
		id: String,
		name: String,
		namespace: Option<String>,
		input: Json<'a>,
	},
	ToolResult {
		call_id: String,
		failed: Option<bool>,
		output: ToolOutput<'a>,
	},
	Reasoning {
		carrier: Carrier,
		/// The block this replay state belongs with, if any.
		next: Option<BlockId>,
	},
}

/// An unread client image block. Edits may inspect or remove the block; use
/// `DataImage` to add or replace an image.
pub struct ImageFields<'a>(pub(crate) Fields<'a>);

/// A protocol-neutral base64 image. Parsers validate client images;
/// edits must supply a supported media type and valid base64 data.
pub struct DataImage<'a> {
	pub media_type: String,
	pub data: Cow<'a, str>,
}

/// A tool result as text or an ordered mix of text and images.
pub enum ToolOutput<'a> {
	Text(Text<'a>),
	Mixed(Vec<ToolPart<'a>>),
}

/// One item in a mixed tool result.
pub enum ToolPart<'a> {
	Text(Text<'a>),
	Image {
		/// Its place in the client's list, for reporting.
		index: usize,
		fields: ImageFields<'a>,
	},
	DataImage(DataImage<'a>),
}

/// Provider replay state carried through the neutral request.
///
/// Edits may remove invalid state but cannot read or render it.
pub struct Carrier(CarrierState);

enum CarrierState {
	Decoded(Vec<u8>),
	Invalid(CarrierError),
}

impl Carrier {
	pub(crate) const fn decoded(state: Vec<u8>) -> Self {
		Self(CarrierState::Decoded(state))
	}

	pub(crate) const fn invalid(error: CarrierError) -> Self {
		Self(CarrierState::Invalid(error))
	}

	pub(crate) fn opened(&self) -> Result<&[u8], &CarrierError> {
		match &self.0 {
			CarrierState::Decoded(state) => Ok(state),
			CarrierState::Invalid(error) => Err(error),
		}
	}
}

/// Request text that borrows the input until an edit replaces it.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Text<'a>(Cow<'a, str>);

impl<'a> Text<'a> {
	/// Returns a borrowed view of the text.
	#[must_use]
	pub fn read(&self) -> Cow<'_, str> {
		Cow::Borrowed(&self.0)
	}

	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	/// Its length in bytes, decoded.
	#[must_use]
	pub fn len(&self) -> usize {
		self.0.len()
	}

	pub(crate) fn as_str(&self) -> &str {
		&self.0
	}
}

impl From<String> for Text<'_> {
	fn from(text: String) -> Self {
		Self(Cow::Owned(text))
	}
}

impl<'a> From<&'a str> for Text<'a> {
	fn from(text: &'a str) -> Self {
		Self(Cow::Borrowed(text))
	}
}

impl<'a> From<Cow<'a, str>> for Text<'a> {
	fn from(text: Cow<'a, str>) -> Self {
		Self(text)
	}
}

impl fmt::Debug for Text<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt::Debug::fmt(&*self.0, f)
	}
}

/// Where a text is in the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPlace {
	/// Its place in `Conversation::system`.
	System(usize),
	/// A text block, in a message with this role.
	Message(Role, BlockId),
	/// A tool result's text. `part` is its place among a mixed result's parts.
	ToolResult { block: BlockId, part: Option<usize> },
	/// A string value in a tool call's arguments, the `string`th in the order sent.
	/// Object keys aren't text.
	ToolCall { block: BlockId, string: usize },
}

impl<'a> Conversation<'a> {
	/// Every text the provider will see, in request order. Tool arguments are decoded
	/// only as their string values are read.
	pub fn texts(&self) -> impl Iterator<Item = (TextPlace, Text<'_>)> {
		let system = self
			.system
			.iter()
			.flatten()
			.enumerate()
			.map(|(at, text)| (TextPlace::System(at), text.borrowed()));
		let messages = self.messages.iter().flat_map(|message| {
			message
				.blocks
				.iter()
				.flat_map(move |block| block_texts(message.role, block))
		});
		system.chain(messages)
	}

	/// Calls `edit` with every text, in request order.
	pub fn edit_texts(&mut self, mut edit: impl FnMut(TextMut<'_, '_>)) {
		for (at, text) in self.system.iter_mut().flatten().enumerate() {
			visit(&mut edit, TextPlace::System(at), text);
		}
		for message in &mut self.messages {
			for block in &mut message.blocks {
				match &mut block.content {
					Content::Text(text) => {
						let place = TextPlace::Message(message.role, block.id);
						visit(&mut edit, place, text);
					},
					Content::ToolCall { input, .. } => {
						if let Some(edited) = edit_arguments(&mut edit, block.id, input.as_ref()) {
							*input = edited;
						}
					},
					Content::ToolResult {
						output: ToolOutput::Text(text),
						..
					} => {
						let place = TextPlace::ToolResult {
							block: block.id,
							part: None,
						};
						visit(&mut edit, place, text);
					},
					Content::ToolResult {
						output: ToolOutput::Mixed(parts),
						..
					} => {
						for (at, part) in parts.iter_mut().enumerate() {
							if let ToolPart::Text(text) = part {
								let place = TextPlace::ToolResult {
									block: block.id,
									part: Some(at),
								};
								visit(&mut edit, place, text);
							}
						}
					},
					_ => {},
				}
			}
		}
	}
}

/// One text, handed to `Conversation::edit_texts`.
pub struct TextMut<'e, 't> {
	place: TextPlace,
	text: &'e mut Text<'t>,
	replaced: &'e mut bool,
}

impl<'t> TextMut<'_, 't> {
	#[must_use]
	pub const fn place(&self) -> TextPlace {
		self.place
	}

	#[must_use]
	pub fn read(&self) -> Cow<'_, str> {
		self.text.read()
	}

	pub fn replace(&mut self, text: impl Into<Text<'t>>) {
		*self.text = text.into();
		*self.replaced = true;
	}
}

impl Text<'_> {
	fn borrowed(&self) -> Text<'_> {
		Text(Cow::Borrowed(&self.0))
	}
}

fn block_texts<'e>(
	role: Role,
	block: &'e Block<'_>,
) -> impl Iterator<Item = (TextPlace, Text<'e>)> {
	let id = block.id;
	let text = match &block.content {
		Content::Text(text) => Some((TextPlace::Message(role, id), text.borrowed())),
		Content::ToolResult {
			output: ToolOutput::Text(text),
			..
		} => Some((
			TextPlace::ToolResult {
				block: id,
				part: None,
			},
			text.borrowed(),
		)),
		_ => None,
	};
	let arguments = match &block.content {
		Content::ToolCall { input, .. } => Some(input.as_ref()),
		_ => None,
	};
	let parts = match &block.content {
		Content::ToolResult {
			output: ToolOutput::Mixed(parts),
			..
		} => Some(parts),
		_ => None,
	};
	text
		.into_iter()
		.chain(arguments.into_iter().flat_map(move |json| {
			json
				.strings()
				.enumerate()
				.map(move |(string, text)| (TextPlace::ToolCall { block: id, string }, Text(text)))
		}))
		.chain(parts.into_iter().flat_map(move |parts| {
			parts
				.iter()
				.enumerate()
				.filter_map(move |(at, part)| match part {
					ToolPart::Text(text) => Some((
						TextPlace::ToolResult {
							block: id,
							part: Some(at),
						},
						text.borrowed(),
					)),
					_ => None,
				})
		}))
}

/// Returns whether `edit` replaced the text.
fn visit(edit: &mut impl FnMut(TextMut<'_, '_>), place: TextPlace, text: &mut Text<'_>) -> bool {
	let mut replaced = false;
	edit(TextMut {
		place,
		text,
		replaced: &mut replaced,
	});
	replaced
}

/// Rewrites only the strings `edit` replaced. `None` when nothing was replaced.
fn edit_arguments(
	edit: &mut impl FnMut(TextMut<'_, '_>),
	block: BlockId,
	json: JsonRef<'_>,
) -> Option<Json<'static>> {
	json.replace_strings(|string, text| {
		let mut text = Text(text);
		let place = TextPlace::ToolCall { block, string };
		visit(edit, place, &mut text).then(|| text.0.into_owned())
	})
}
