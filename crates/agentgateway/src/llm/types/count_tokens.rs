use agent_core::prelude::Strng;
use agent_core::strng;
use serde::{Deserialize, Serialize};

use crate::llm::types::{RequestType, ResponseType};
use crate::llm::{
	AIError, InputFormat, LLMRequest, LLMResponse, SimpleChatCompletionMessage, conversion,
};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Request {
	pub model: Option<String>,
	#[serde(flatten)]
	pub rest: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	pub input_tokens: u64,
}

impl ResponseType for Response {
	fn to_llm_response(&self, _include_completion_in_log: bool) -> LLMResponse {
		LLMResponse {
			count_tokens: Some(self.input_tokens),
			..Default::default()
		}
	}

	fn set_webhook_choices(
		&mut self,
		_choices: Vec<crate::llm::policy::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		Ok(())
	}

	fn to_webhook_choices(&self) -> Vec<crate::llm::policy::webhook::ResponseChoice> {
		vec![]
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self)
	}
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

	fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}
}
