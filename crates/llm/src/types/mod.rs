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
	fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse;
	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice>;
	fn set_webhook_choices(
		&mut self,
		resp: Vec<crate::webhook::ResponseChoice>,
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
	/// Message role, such as "system", "user", or "assistant".
	pub role: Strng,
	/// Message text content.
	pub content: Strng,
}

pub fn serialize_str<T: Serialize>(value: &T) -> Option<Strng> {
	serde_json::to_value(value).ok()?.as_str().map(Into::into)
}
