use agent_core::prelude::Strng;
use agent_core::strng;
use serde::{Deserialize, Serialize};

use crate::llm::types::RequestType;
use crate::llm::{AIError, InputFormat, LLMRequest, LLMRequestParams, SimpleChatCompletionMessage};

#[derive(Debug, Serialize, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncodingFormat {
	#[default]
	Float,
	Base64,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	pub model: Option<String>,
	// pub input: EmbeddingInput,
	// pub user: Option<String>,
	pub encoding_format: Option<EncodingFormat>,
	pub dimensions: Option<u64>,

	// Everything else - passthrough
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {
		// Ignored
	}

	fn append_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {
		// Ignored
	}

	fn to_llm_request(&self, provider: Strng, _tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		Ok(LLMRequest {
			// We never tokenize these, so always empty
			input_tokens: None,
			input_format: InputFormat::Embeddings,
			request_model: model,
			provider,
			streaming: false,
			params: LLMRequestParams {
				temperature: None,
				top_p: None,
				frequency_penalty: None,
				presence_penalty: None,
				seed: None,
				max_tokens: None,
				encoding_format: self.encoding_format.map(|f| match f {
					EncodingFormat::Base64 => strng::literal!("base64"),
					EncodingFormat::Float => strng::literal!("float"),
				}),
				dimensions: self.dimensions,
			},
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		unimplemented!("get_messages is used for prompt guard; prompt guard is disable for embeddings.")
	}

	fn set_messages(&mut self, _messages: Vec<SimpleChatCompletionMessage>) {
		unimplemented!("set_messages is used for prompt guard; prompt guard is disable for embeddings.")
	}

	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}
}
