use agent_core::prelude::Strng;
use agent_core::strng;
use serde::{Deserialize, Serialize};

use crate::llm::types::RequestType;
use crate::llm::{AIError, InputFormat, LLMRequest, SimpleChatCompletionMessage, conversion};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Request {
	pub model: Option<String>,
	#[serde(flatten)]
	pub rest: serde_json::Map<String, serde_json::Value>,
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {
		// TODO: this would help since we can then count the pre-pending
	}

	fn append_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {
		// TODO: this would help since we can then count the appending
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

	fn to_bedrock_token_count(&self, headers: &::http::HeaderMap) -> Result<Vec<u8>, AIError> {
		conversion::bedrock::from_anthropic_token_count::translate(self, headers)
	}
}
