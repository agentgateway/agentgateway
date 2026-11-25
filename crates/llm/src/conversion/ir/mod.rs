//! Owned request and reply models for typed-first translation.
//!
//! A reader decides what each wire part means; a writer decides how to encode it. Both record
//! what they leave out in one [`Drops`]. Streamed replies do not use these models yet.

use std::collections::{BTreeMap, HashMap};
use std::mem::DropGuard;

use serde_json::Value;

pub mod messages;
pub mod responses;

/// A chat request, as the client asked for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
	pub model: String,
	pub max_output_tokens: usize,
	/// The leading system prompt; later system turns stay in `messages`.
	pub instructions: Vec<TextPart>,
	pub messages: Vec<Message>,
	pub temperature: Option<f32>,
	pub top_p: Option<f32>,
	pub top_k: Option<usize>,
	pub stop_sequences: Vec<String>,
	pub stream: bool,
	pub reasoning: Reasoning,
	pub output_format: Option<OutputFormat>,
	pub metadata: BTreeMap<String, String>,
	pub tools: Vec<Tool>,
	pub tool_choice: Option<ToolChoice>,
	/// `None` keeps the destination's default.
	pub parallel_tool_calls: Option<bool>,
	/// Request-wide automatic cache mark.
	pub cache: Option<CacheMark>,
}

/// One turn of the conversation history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
	// TODO(mk): history is grouped by role, as Messages sends it. If a Responses reader needs
	// synthetic messages or loses item identity, switch to ordered items for messages, calls and results.
	pub role: Role,
	/// Indexes match the source content array.
	pub parts: Vec<Part>,
}

/// Who wrote a message in the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
	User,
	Assistant,
	/// A system turn inside the history, not the leading instructions.
	System,
}

/// One piece of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
	Text(TextPart),
	Image {
		source: Source,
		cache: Option<CacheMark>,
	},
	Document(Document),
	Reasoning {
		text: String,
		signature: String,
	},
	RedactedReasoning {
		data: String,
	},
	ToolCall {
		id: String,
		name: String,
		arguments: Value,
		cache: Option<CacheMark>,
	},
	ToolResult {
		call_id: String,
		content: Vec<ToolResultPart>,
		is_error: Option<bool>,
		cache: Option<CacheMark>,
	},
	/// Keeps the index of a part the reader skipped and already recorded as dropped.
	Unread,
}

/// Text with optional citations and cache mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPart {
	pub text: String,
	pub citations: Option<Value>,
	pub cache: Option<CacheMark>,
}

/// A document given to the model, such as a PDF or plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
	pub source: Source,
	pub title: Option<String>,
	pub context: Option<String>,
	pub citations: Option<Value>,
	pub cache: Option<CacheMark>,
}

/// One piece of a tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResultPart {
	Text(TextPart),
	/// A reference to a deferred tool definition, by name.
	ToolReference {
		tool_name: String,
		cache: Option<CacheMark>,
	},
	Image {
		source: Source,
		cache: Option<CacheMark>,
	},
	Document(Document),
	Unread,
}

/// Where the bytes of an image or document come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
	Base64 {
		media_type: String,
		data: String,
	},
	Url(String),
	/// A provider file ID.
	File(String),
	/// Plain text carried as a document.
	Text(String),
	/// A source type the reader does not know.
	Other(String),
	/// A known type missing a required field, with the error to give if it is sent.
	Incomplete(&'static str),
}

/// Reasoning controls as the client gave them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reasoning {
	pub mode: Option<ReasoningMode>,
	/// Never derived from a budget.
	pub effort: Option<Effort>,
	/// How the client wants reasoning shown, such as `summarized`.
	pub display: Option<String>,
}

/// Whether and how the model may reason before it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningMode {
	Off,
	/// The model decides how much to reason.
	Adaptive,
	Budget {
		tokens: u64,
		/// For destinations that take an effort instead of a budget.
		fallback_effort: Effort,
	},
}

/// How hard the model should reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
	Low,
	Medium,
	High,
	Xhigh,
	Max,
}

/// A required shape for the model's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
	JsonSchema { schema: Value },
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tool {
	Function {
		name: String,
		description: Option<String>,
		parameters: Value,
		strict: Option<bool>,
		cache: Option<CacheMark>,
	},
	/// A tool the source provider runs itself, such as Anthropic web search.
	Server {
		kind: String,
		name: String,
		cache: Option<CacheMark>,
		config: HashMap<String, Value>,
	},
}

/// Whether and which tool the model must call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolChoice {
	Auto,
	/// The model must call some tool.
	Required,
	None,
	Tool(String),
}

/// A prompt cache breakpoint on a part, tool or the whole request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CacheMark {
	pub ttl: Option<String>,
}

/// A finished, non-streamed model reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
	pub id: String,
	pub model: String,
	pub parts: Vec<ReplyPart>,
	pub stop_reason: StopReason,
	/// `None` when the provider reported no usage.
	pub usage: Option<Usage>,
	pub service_tier: Option<String>,
}

/// One piece of a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyPart {
	Text {
		text: String,
		citations: Vec<Citation>,
	},
	Refusal(String),
	ToolCall {
		id: String,
		name: String,
		arguments: Value,
	},
}

/// A web page the reply text cites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
	pub url: String,
	pub title: String,
}

/// Why the model stopped. Tool-call and refusal stops are left to the writer, which sees the parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
	Completed,
	/// The output reached a token limit.
	Length,
	ContentFilter,
	ContextWindow,
}

/// Token counts for one reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
	/// All input tokens, including those read from or written to the cache.
	pub input_tokens: u64,
	pub output_tokens: u64,
	/// `None` when the provider did not say, which is not the same as zero.
	pub cache_read_tokens: Option<u64>,
	pub cache_write_tokens: Option<u64>,
}

/// The fields and parts one translation left out, kept only while `recording`.
#[derive(Debug)]
pub struct Drops {
	recording: bool,
	list: Vec<Dropped>,
}

impl Drops {
	/// When not `recording`, marks are ignored and their paths are never built.
	pub const fn new(recording: bool) -> Self {
		Self {
			recording,
			list: Vec::new(),
		}
	}

	/// Records only when debug logging is on, and logs the drops under `message` when the guard
	/// goes out of scope, including when the translation fails.
	pub fn logged(message: &'static str) -> DropGuard<Self, impl FnOnce(Self)> {
		DropGuard::new(
			Self::new(tracing::enabled!(tracing::Level::DEBUG)),
			move |drops| {
				if !drops.list.is_empty() {
					tracing::debug!(dropped = ?drops.list, "{message}");
				}
			},
		)
	}

	/// Notes a drop at the path `path` builds.
	pub fn record(&mut self, reason: DropReason, path: impl FnOnce() -> String) {
		if self.recording {
			self.list.push(Dropped {
				path: path(),
				reason,
			});
		}
	}
}

/// A supplied field or part that did not reach the destination. Holds no content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
	path: String,
	reason: DropReason,
}

/// Why a field or part did not reach the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
	/// The source adapter does not model this field or part.
	Unread,
	/// The model holds it, but the destination cannot express it.
	Unsupported,
}

#[cfg(test)]
mod tests;
