pub mod bedrock;
pub mod completions;
pub mod count_tokens;
pub mod detect;
pub mod embeddings;
pub mod messages;
pub mod rerank;
pub mod responses;
pub mod vertex;

use agent_core::prelude::Strng;
use agent_core::strng;
use serde::Serialize;

use crate::apply;
use crate::llm::{AIError, LLMRequest, LLMResponse};
use crate::serdes::schema;

pub enum ChatRequest<'a> {
	Completions(&'a completions::Request),
	Messages(&'a messages::Request),
	Responses(&'a responses::Request),
}

/// ResponseType is an abstraction over provider/endpoint specific response formats that enables
/// uniform policy enforcement and observability
pub trait ResponseType: Send + Sync {
	fn to_llm_response(&self, log_content: crate::llm::LogContentFields) -> LLMResponse;
	fn to_webhook_choices(&self) -> Vec<crate::llm::policy::webhook::ResponseChoice>;
	fn set_webhook_choices(
		&mut self,
		resp: Vec<crate::llm::policy::webhook::ResponseChoice>,
	) -> anyhow::Result<()>;
	fn serialize(&self) -> serde_json::Result<Vec<u8>>;
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
}

/// SimpleChatCompletionMessage is a simplified chat message
#[apply(schema!)]
#[derive(Eq, PartialEq, cel::DynamicType)]
pub struct SimpleChatCompletionMessage {
	pub role: Strng,
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
	Text { text: String },
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
			.filter_map(|p| match p {
				OutputMessagePart::ToolCall {
					id,
					name,
					arguments,
				} => Some(ToolCall {
					id: id.clone(),
					name: name.clone(),
					arguments: arguments.clone(),
				}),
				_ => None,
			})
			.collect()
	}
}

pub fn serialize_str<T: Serialize>(value: &T) -> Option<Strng> {
	serde_json::to_value(value).ok()?.as_str().map(Into::into)
}
