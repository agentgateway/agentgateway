use agent_core::prelude::Strng;
use agent_core::strng;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::llm::types::{RequestType, messages};
use crate::llm::{AIError, InputFormat, LLMRequest, SimpleChatCompletionMessage, conversion};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Request {
	pub messages: Vec<messages::RequestMessage>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<messages::TextBlock>,
	#[serde(flatten)]
	pub rest: serde_json::Map<String, serde_json::Value>,
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		messages::prepend_prompts_helper(&mut self.messages, &mut self.system, prompts);
	}

	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		messages::append_prompts_helper(&mut self.messages, &mut self.system, prompts);
	}

	fn to_llm_request(&self, provider: Strng, _tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		Ok(LLMRequest {
			// We never tokenize these, so always empty
			input_tokens: None,
			input_format: InputFormat::CountTokens,
			request_model: model,
			provider,
			streaming: false,
			params: Default::default(),
			prompt: Default::default(),
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		unimplemented!(
			"get_messages is used for prompt guard; prompt guard is disable for token counting."
		)
	}

	fn set_messages(&mut self, _messages: Vec<SimpleChatCompletionMessage>) {
		unimplemented!(
			"set_messages is used for prompt guard; prompt guard is disable for token counting."
		)
	}

	fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}

	fn to_bedrock_token_count(&self, headers: &::http::HeaderMap) -> Result<Vec<u8>, AIError> {
		conversion::bedrock::from_anthropic_token_count::translate(self, headers)
	}

	fn to_vertex(&self, provider: &crate::llm::vertex::Provider) -> Result<Vec<u8>, AIError> {
		let body = self.to_anthropic()?;
		provider.prepare_anthropic_count_tokens_body(body)
	}
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Response {
	#[serde(alias = "inputTokens")]
	pub input_tokens: u64,
}

impl Response {
	pub fn translate_response(bytes: Bytes) -> Result<(Bytes, u64), AIError> {
		let resp: Self = serde_json::from_slice(&bytes).map_err(AIError::ResponseParsing)?;
		Ok((bytes, resp.input_tokens))
	}
}
