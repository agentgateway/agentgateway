pub mod bedrock;
pub mod completions;
pub mod count_tokens;
pub mod detect;
pub mod embeddings;
pub mod messages;
pub mod rerank;
pub mod responses;
pub mod vertex;
pub mod vertex_gemini;

use agent_core::prelude::Strng;
use agent_core::strng;
use serde::Serialize;

use crate::{AIError, LLMRequest, LLMResponse, apply};

pub enum ChatRequest<'a> {
	Completions(&'a completions::Request),
	Messages(&'a messages::Request),
	Responses(&'a responses::Request),
}

/// ResponseType is an abstraction over provider/endpoint specific response formats that enables
/// uniform policy enforcement and observability
pub trait ResponseType: Send + Sync {
	fn to_llm_response(&self, log_content: crate::LogContentFields) -> LLMResponse;
	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice>;
	fn set_webhook_choices(
		&mut self,
		resp: Vec<crate::webhook::ResponseChoice>,
	) -> anyhow::Result<()>;
	fn serialize(&self) -> serde_json::Result<Vec<u8>>;
	fn visit_text_mut(&mut self, f: &mut dyn FnMut(&mut String));
}

/// RequestType is an abstraction over provider/endpoint specific request formats that enables
/// uniform policy enforcement and observability
pub trait RequestType: Send + Sync {
	fn supports_model(&self) -> bool {
		true
	}
	fn model(&mut self) -> &mut Option<String>;
	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>);
	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>);
	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError>;
	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage>;
	fn set_messages(&mut self, messages: Vec<SimpleChatCompletionMessage>);
	fn to_value(&self) -> serde_json::Result<serde_json::Value>;
	fn visit_text_mut(&mut self, f: &mut dyn FnMut(&mut String));
}

/// Scan each maximal run of consecutive text parts as one `sep`-joined string, so guard patterns
/// spanning adjacent text blocks still match. Parts where `text_of` returns `None` (images, tool
/// blocks) are never scanned and act as run boundaries.
///
/// If the guard edits the joined text, the run collapses into its last part (where
/// `cache_control` conventionally lives), which receives the edited text. Untouched runs keep
/// their original parts byte-identical, and single-part runs are scanned in place with no copying.
pub(crate) fn scan_text_runs<T>(
	parts: &mut Vec<T>,
	sep: &str,
	mut text_of: impl FnMut(&mut T) -> Option<&mut String>,
	f: &mut dyn FnMut(&mut String),
) {
	fn flush<T>(
		out: &mut Vec<T>,
		run: &mut Vec<T>,
		sep: &str,
		text_of: &mut impl FnMut(&mut T) -> Option<&mut String>,
		f: &mut dyn FnMut(&mut String),
	) {
		if run.len() == 1 {
			let mut part = run.pop().expect("run has one part");
			f(text_of(&mut part).expect("run parts have text"));
			out.push(part);
			return;
		}
		let mut joined = String::new();
		for part in run.iter_mut() {
			if !joined.is_empty() {
				joined.push_str(sep);
			}
			joined.push_str(text_of(part).expect("run parts have text"));
		}
		let original = joined.clone();
		f(&mut joined);
		if joined == original {
			out.append(run);
			return;
		}
		let mut last = run.pop().expect("run has parts");
		run.clear();
		*text_of(&mut last).expect("run parts have text") = joined;
		out.push(last);
	}

	let mut out: Vec<T> = Vec::with_capacity(parts.len());
	let mut run: Vec<T> = Vec::new();
	for mut part in parts.drain(..) {
		if text_of(&mut part).is_some() {
			run.push(part);
		} else {
			if !run.is_empty() {
				flush(&mut out, &mut run, sep, &mut text_of, f);
			}
			out.push(part);
		}
	}
	if !run.is_empty() {
		flush(&mut out, &mut run, sep, &mut text_of, f);
	}
	*parts = out;
}

/// SimpleChatCompletionMessage is a simplified chat message
#[apply(schema!)]
#[derive(Eq, PartialEq, cel::DynamicType)]
pub struct SimpleChatCompletionMessage {
	/// Message role, such as "system", "user", or "assistant".
	pub role: Strng,
	/// Message text content.
	pub content: Strng,
}

/// ToolCall represents a single tool/function invocation surfaced for observability.
#[apply(schema!)]
#[derive(cel::DynamicType)]
pub struct ToolCall {
	pub id: Strng,
	pub name: Strng,
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::Value"))]
	pub arguments: serde_json::Value,
}

/// A single content part within an output message, per the GenAI semantic conventions.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputMessagePart {
	ToolCall {
		id: Strng,
		name: Strng,
		arguments: serde_json::Value,
	},
}

/// A structured output message for the `gen_ai.output.messages` semantic convention attribute.
#[derive(Debug, Clone, Serialize)]
pub struct OutputMessage {
	pub role: Strng,
	pub content: Vec<OutputMessagePart>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub finish_reason: Option<Strng>,
}

impl OutputMessage {
	pub fn tool_calls(&self) -> Vec<ToolCall> {
		self
			.content
			.iter()
			.map(|p| match p {
				OutputMessagePart::ToolCall {
					id,
					name,
					arguments,
				} => ToolCall {
					id: id.clone(),
					name: name.clone(),
					arguments: arguments.clone(),
				},
			})
			.collect()
	}
}

pub fn serialize_str<T: Serialize>(value: &T) -> Option<Strng> {
	serde_json::to_value(value).ok()?.as_str().map(Into::into)
}
