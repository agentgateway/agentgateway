//! The client's reply as typed JSON: the response, its output items and stream events.

use std::borrow::Cow;
use std::num::NonZeroU64;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

use crate::ir::events::{DecodeError, JsonStr, Usage};
use crate::json::{JsonRef, string_value};

#[derive(Deserialize)]
pub(super) struct Wire<'a> {
	#[serde(rename = "type", borrow)]
	pub kind: Cow<'a, str>,
	pub output_index: Option<u32>,
	pub content_index: Option<u32>,
	#[serde(borrow)]
	pub delta: Option<&'a RawValue>,
	#[serde(borrow)]
	pub text: Option<&'a RawValue>,
	#[serde(borrow)]
	pub refusal: Option<&'a RawValue>,
	#[serde(borrow)]
	pub arguments: Option<&'a RawValue>,
	#[serde(borrow)]
	pub item: Option<&'a RawValue>,
	#[serde(borrow)]
	pub part: Option<&'a RawValue>,
	#[serde(borrow)]
	pub response: Option<&'a RawValue>,
	#[serde(borrow)]
	pub annotation: Option<WireAnnotation<'a>>,
	/// An `error` event's fields as documented.
	#[serde(borrow)]
	pub code: Option<Cow<'a, str>>,
	#[serde(borrow)]
	pub message: Option<Cow<'a, str>>,
	/// The same fields as OpenAI actually sends them, nested.
	#[serde(borrow)]
	pub error: Option<WireError<'a>>,
}

#[derive(Deserialize)]
pub(super) struct WireStart<'a> {
	#[serde(borrow)]
	pub id: Cow<'a, str>,
	#[serde(borrow)]
	pub model: Cow<'a, str>,
}

#[derive(Deserialize)]
pub(super) struct WireItem<'a> {
	#[serde(rename = "type", borrow)]
	pub kind: Cow<'a, str>,
	#[serde(borrow)]
	pub call_id: Option<Cow<'a, str>>,
	#[serde(borrow)]
	pub name: Option<Cow<'a, str>>,
	#[serde(borrow)]
	pub arguments: Option<&'a RawValue>,
	#[serde(borrow)]
	pub status: Option<Cow<'a, str>>,
	#[serde(borrow)]
	pub encrypted_content: Option<&'a RawValue>,
	#[serde(default, borrow)]
	pub content: Vec<WireContent<'a>>,
}

#[derive(Deserialize)]
pub(super) struct WireContent<'a> {
	#[serde(rename = "type", borrow)]
	pub kind: Cow<'a, str>,
	#[serde(borrow)]
	pub text: Option<&'a RawValue>,
	#[serde(borrow)]
	pub refusal: Option<&'a RawValue>,
	#[serde(default, borrow)]
	pub annotations: Vec<WireAnnotation<'a>>,
}

#[derive(Deserialize)]
pub(super) struct WireAnnotation<'a> {
	#[serde(rename = "type", borrow)]
	pub kind: Cow<'a, str>,
	#[serde(borrow)]
	pub url: Option<Cow<'a, str>>,
	#[serde(borrow)]
	pub title: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
pub(super) struct WirePart<'a> {
	#[serde(rename = "type", borrow)]
	pub kind: Cow<'a, str>,
}

#[derive(Deserialize)]
pub(super) struct WireTerminal<'a> {
	#[serde(default, borrow)]
	pub output: Vec<WireTerminalItem<'a>>,
	#[serde(borrow)]
	pub service_tier: Option<Cow<'a, str>>,
	pub usage: Option<WireUsage>,
	#[serde(borrow)]
	pub incomplete_details: Option<WireIncomplete<'a>>,
	#[serde(borrow)]
	pub error: Option<WireError<'a>>,
}

/// Only what the stream checks, so the rest of a large terminal event is skipped.
#[derive(Deserialize)]
pub(super) struct WireTerminalItem<'a> {
	#[serde(rename = "type", borrow)]
	pub kind: Cow<'a, str>,
	#[serde(borrow)]
	pub encrypted_content: Option<&'a RawValue>,
}

#[derive(Deserialize)]
pub(super) struct WireIncomplete<'a> {
	#[serde(borrow)]
	pub reason: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
pub(super) struct WireError<'a> {
	#[serde(borrow)]
	pub code: Option<Cow<'a, str>>,
	#[serde(borrow)]
	pub message: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
pub(super) struct WireUsage {
	pub input_tokens: Option<u64>,
	pub output_tokens: Option<u64>,
	pub total_tokens: Option<u64>,
	pub input_tokens_details: Option<WireInputDetails>,
	pub output_tokens_details: Option<WireOutputDetails>,
}

#[derive(Deserialize)]
pub(super) struct WireInputDetails {
	pub cached_tokens: Option<u64>,
	pub cache_write_tokens: Option<u64>,
	pub cache_write_tokens_details: Option<WireCacheWriteDetails>,
}

#[derive(Deserialize)]
pub(super) struct WireCacheWriteDetails {
	pub ttl_12h_tokens: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct WireOutputDetails {
	pub reasoning_tokens: Option<u64>,
}

impl From<WireUsage> for Usage {
	fn from(usage: WireUsage) -> Self {
		let input = usage.input_tokens_details;
		let output = usage.output_tokens_details;
		Self {
			input_tokens: usage.input_tokens,
			output_tokens: usage.output_tokens,
			total_tokens: usage.total_tokens,
			cached_tokens: input.as_ref().and_then(|details| details.cached_tokens),
			cache_write_tokens: input
				.as_ref()
				.and_then(|details| details.cache_write_tokens),
			cache_write_12h_tokens: input
				.as_ref()
				.and_then(|details| details.cache_write_tokens_details.as_ref())
				.and_then(|details| details.ttl_12h_tokens),
			reasoning_tokens: output.as_ref().and_then(|details| details.reasoning_tokens),
		}
	}
}

pub(super) fn string<'a>(
	raw: Option<&'a RawValue>,
	what: &'static str,
) -> Result<JsonStr<'a>, DecodeError> {
	JsonStr::read(raw.ok_or(DecodeError::WrongType(what))?, what)
}

pub(super) fn summary_text(item: &RawValue) -> Result<Box<RawValue>, DecodeError> {
	#[derive(Deserialize)]
	struct Reasoning<'a> {
		#[serde(default, borrow)]
		summary: Vec<Summary<'a>>,
	}
	#[derive(Deserialize)]
	struct Summary<'a> {
		#[serde(borrow)]
		text: Option<&'a RawValue>,
	}
	let parsed: Reasoning<'_> = serde_json::from_str(item.get())?;
	let mut joined = String::new();
	for part in parsed.summary {
		if let Some(raw) = part.text {
			joined.push_str(&JsonStr::read(raw, "summary")?.decode());
		}
	}
	Ok(string_value(&joined)?)
}

#[derive(Serialize)]
pub(super) struct Request<'a> {
	pub(super) model: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) instructions: Option<Cow<'a, str>>,
	pub(super) input: Vec<InputItem<'a>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) tools: Option<Vec<FunctionTool<'a>>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) tool_choice: Option<ToolChoice<'a>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) parallel_tool_calls: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) max_output_tokens: Option<NonZeroU64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) temperature: Option<JsonRef<'a>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) top_p: Option<JsonRef<'a>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) text: Option<TextConfig<'a>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) reasoning: Option<Reasoning>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) stream: Option<bool>,
	pub(super) store: bool,
	pub(super) include: [Include; 1],
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) safety_identifier: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum InputItem<'a> {
	Known(KnownInputItem<'a>),
	Provider(&'a RawValue),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum KnownInputItem<'a> {
	Message {
		role: Role,
		content: Vec<Part<'a>>,
	},
	FunctionCall {
		call_id: &'a str,
		name: &'a str,
		arguments: Cow<'a, str>,
	},
	FunctionCallOutput {
		call_id: &'a str,
		output: ToolOutput<'a>,
	},
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
	User,
	Assistant,
	System,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Part<'a> {
	InputText {
		text: Cow<'a, str>,
		#[serde(skip_serializing_if = "Option::is_none")]
		prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
	},
	OutputText {
		text: Cow<'a, str>,
		#[serde(skip_serializing_if = "Option::is_none")]
		prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
	},
	InputImage {
		image_url: Cow<'a, str>,
		#[serde(skip_serializing_if = "Option::is_none")]
		prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
	},
}

impl Part<'_> {
	pub(super) const fn mark(&mut self) {
		match self {
			Self::InputText {
				prompt_cache_breakpoint,
				..
			}
			| Self::OutputText {
				prompt_cache_breakpoint,
				..
			}
			| Self::InputImage {
				prompt_cache_breakpoint,
				..
			} => *prompt_cache_breakpoint = Some(PromptCacheBreakpoint::EXPLICIT),
		}
	}
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum ToolOutput<'a> {
	Text(Cow<'a, str>),
	Parts(Vec<Part<'a>>),
}

#[derive(Serialize)]
pub(super) struct FunctionTool<'a> {
	#[serde(rename = "type")]
	pub(super) kind: FunctionType,
	pub(super) name: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) description: Option<&'a str>,
	pub(super) parameters: JsonRef<'a>,
	pub(super) strict: bool,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(super) enum FunctionType {
	Function,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum ToolChoice<'a> {
	Mode(ToolChoiceMode),
	Function {
		#[serde(rename = "type")]
		kind: FunctionType,
		name: &'a str,
	},
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ToolChoiceMode {
	Auto,
	Required,
	None,
}

#[derive(Serialize)]
pub(super) struct TextConfig<'a> {
	pub(super) format: TextFormat<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum TextFormat<'a> {
	JsonObject,
	JsonSchema {
		name: &'static str,
		schema: JsonRef<'a>,
	},
}

#[derive(Serialize)]
pub(super) struct Reasoning {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) effort: Option<ReasoningEffort>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) context: Option<ReasoningContext>,
}

#[derive(Serialize, Clone, Copy)]
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
#[serde(rename_all = "snake_case")]
pub(super) enum ReasoningContext {
	AllTurns,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(super) enum Include {
	#[serde(rename = "reasoning.encrypted_content")]
	ReasoningEncryptedContent,
}

#[derive(Serialize, Clone, Copy)]
pub(super) struct PromptCacheBreakpoint {
	mode: PromptCacheMode,
}

impl PromptCacheBreakpoint {
	pub(super) const EXPLICIT: Self = Self {
		mode: PromptCacheMode::Explicit,
	};
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum PromptCacheMode {
	Explicit,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub(super) enum StreamEvent<'a> {
	#[serde(rename = "response.created")]
	Created {
		response: Response<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.completed")]
	Completed {
		response: Response<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.incomplete")]
	Incomplete {
		response: Response<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.failed")]
	Failed {
		response: Response<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.output_item.added")]
	ItemAdded {
		output_index: usize,
		item: Item<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.output_item.done")]
	ItemDone {
		output_index: usize,
		item: Item<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.content_part.added")]
	ContentPartAdded {
		item_id: &'a str,
		output_index: usize,
		content_index: usize,
		part: &'a OutputText<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.content_part.done")]
	ContentPartDone {
		item_id: &'a str,
		output_index: usize,
		content_index: usize,
		part: &'a OutputText<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.output_text.done")]
	TextDone {
		item_id: &'a str,
		output_index: usize,
		content_index: usize,
		text: &'a RawValue,
		sequence_number: u64,
	},
	#[serde(rename = "response.reasoning_summary_part.added")]
	SummaryPartAdded {
		item_id: &'a str,
		output_index: usize,
		summary_index: usize,
		part: SummaryText<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.reasoning_summary_part.done")]
	SummaryPartDone {
		item_id: &'a str,
		output_index: usize,
		summary_index: usize,
		part: SummaryText<'a>,
		sequence_number: u64,
	},
	#[serde(rename = "response.reasoning_summary_text.done")]
	SummaryDone {
		item_id: &'a str,
		output_index: usize,
		summary_index: usize,
		text: &'a RawValue,
		sequence_number: u64,
	},
}

impl StreamEvent<'_> {
	/// The SSE event name, which is the event's `type`. Deltas are written by
	/// `EventStream::emit_delta`.
	pub(super) const fn name(&self) -> &'static str {
		match self {
			Self::Created { .. } => "response.created",
			Self::Completed { .. } => "response.completed",
			Self::Incomplete { .. } => "response.incomplete",
			Self::Failed { .. } => "response.failed",
			Self::ItemAdded { .. } => "response.output_item.added",
			Self::ItemDone { .. } => "response.output_item.done",
			Self::ContentPartAdded { .. } => "response.content_part.added",
			Self::ContentPartDone { .. } => "response.content_part.done",
			Self::TextDone { .. } => "response.output_text.done",
			Self::SummaryPartAdded { .. } => "response.reasoning_summary_part.added",
			Self::SummaryPartDone { .. } => "response.reasoning_summary_part.done",
			Self::SummaryDone { .. } => "response.reasoning_summary_text.done",
		}
	}
}

#[derive(Serialize)]
pub(super) struct Response<'a> {
	pub(super) id: &'a str,
	pub(super) object: &'static str,
	pub(super) created_at: u64,
	pub(super) model: &'a str,
	pub(super) status: Status,
	pub(super) output: &'a [Box<RawValue>],
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) usage: Option<ResponseUsage>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) error: Option<ResponseError<'a>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) incomplete_details: Option<IncompleteDetails>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
	InProgress,
	Completed,
	Incomplete,
	Failed,
}

#[derive(Serialize)]
pub(super) struct ResponseUsage {
	pub(super) input_tokens: u64,
	pub(super) input_tokens_details: InputTokensDetails,
	pub(super) output_tokens: u64,
	pub(super) output_tokens_details: OutputTokensDetails,
	pub(super) total_tokens: u64,
}

#[derive(Serialize)]
pub(super) struct InputTokensDetails {
	pub(super) cached_tokens: u64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) cache_write_tokens: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct OutputTokensDetails {
	pub(super) reasoning_tokens: u64,
}

#[derive(Serialize)]
pub(super) struct ResponseError<'a> {
	pub(super) code: &'static str,
	pub(super) message: &'a str,
}

#[derive(Serialize)]
pub(super) struct IncompleteDetails {
	pub(super) reason: &'static str,
}

/// An item as it opens, or one already written.
#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Item<'a> {
	Open(OutputItem<'a>),
	Written(&'a RawValue),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum OutputItem<'a> {
	Message {
		id: &'a str,
		role: Role,
		status: ItemStatus,
		content: &'a [OutputText<'a>],
	},
	Reasoning {
		id: &'a str,
		summary: &'a [SummaryText<'a>],
		#[serde(skip_serializing_if = "Option::is_none")]
		encrypted_content: Option<&'a str>,
	},
	FunctionCall {
		id: &'a str,
		call_id: &'a str,
		name: &'a str,
		#[serde(skip_serializing_if = "Option::is_none")]
		namespace: Option<&'a str>,
		arguments: &'a RawValue,
		status: ItemStatus,
	},
	CustomToolCall {
		id: &'a str,
		call_id: &'a str,
		name: &'a str,
		input: &'a str,
		status: ItemStatus,
	},
	ToolSearchCall {
		id: &'a str,
		call_id: &'a str,
		execution: Execution,
		arguments: &'a RawValue,
		status: ItemStatus,
	},
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(super) enum ItemStatus {
	InProgress,
	Completed,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(super) enum Execution {
	Client,
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "output_text")]
pub(super) struct OutputText<'a> {
	pub(super) text: Text<'a>,
	pub(super) annotations: &'a [UrlCitation],
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "summary_text")]
pub(super) struct SummaryText<'a> {
	pub(super) text: Text<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "url_citation")]
pub(super) struct UrlCitation {
	pub(super) url: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(super) title: Option<String>,
}

/// A JSON string already escaped, or none yet.
#[derive(Clone, Copy)]
pub(super) enum Text<'a> {
	Escaped(&'a RawValue),
	Empty,
}

impl Serialize for Text<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Self::Escaped(raw) => raw.serialize(serializer),
			Self::Empty => serializer.serialize_str(""),
		}
	}
}

#[derive(Serialize)]
pub(super) struct ErrorBody<'a> {
	pub(super) error: ErrorDetail<'a>,
}

#[derive(Serialize)]
pub(super) struct ErrorDetail<'a> {
	#[serde(rename = "type")]
	pub(super) kind: &'a str,
	pub(super) code: &'a str,
	pub(super) message: Cow<'a, str>,
}
