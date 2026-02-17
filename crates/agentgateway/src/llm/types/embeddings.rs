use agent_core::prelude::Strng;
use agent_core::strng;
use serde::{Deserialize, Serialize};

use crate::json;
use crate::llm::types::RequestType;
use crate::llm::{AIError, InputFormat, LLMRequest, LLMRequestParams, SimpleChatCompletionMessage};

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	pub object: String,
	pub model: String,
	pub data: Vec<Embedding>,
	pub usage: Usage,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Embedding {
	pub index: u32,
	pub object: String,
	pub embedding: Vec<f32>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Usage {
	pub prompt_tokens: u32,
	pub total_tokens: u32,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	pub model: Option<String>,
	pub input: serde_json::Value,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub user: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub encoding_format: Option<typed::EncodingFormat>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub dimensions: Option<u32>,

	// Everything else - passthrough
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl TryInto<typed::Request> for &Request {
	type Error = AIError;

	fn try_into(self) -> Result<typed::Request, Self::Error> {
		json::convert::<_, typed::Request>(self).map_err(AIError::RequestMarshal)
	}
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
				encoding_format: self.encoding_format.as_ref().map(|f| match f {
					typed::EncodingFormat::Base64 => strng::literal!("base64"),
					typed::EncodingFormat::Float => strng::literal!("float"),
				}),
				dimensions: self.dimensions.map(|d| d as u64),
			},
			prompt: Default::default(),
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

	fn to_bedrock(
		&self,
		provider: &crate::llm::bedrock::Provider,
		_headers: Option<&::http::HeaderMap>,
		_prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		crate::llm::conversion::bedrock::from_embeddings::translate(self, provider)
	}

	fn to_vertex(&self, _provider: &crate::llm::vertex::Provider) -> Result<Vec<u8>, AIError> {
		crate::llm::conversion::vertex::from_embeddings::translate(self)
	}
}

impl crate::llm::types::ResponseType for Response {
	fn to_llm_response(&self, _include_completion_in_log: bool) -> crate::llm::LLMResponse {
		crate::llm::LLMResponse {
			input_tokens: Some(self.usage.prompt_tokens as u64),
			total_tokens: Some(self.usage.total_tokens as u64),
			..Default::default()
		}
	}

	fn to_webhook_choices(&self) -> Vec<crate::llm::policy::webhook::ResponseChoice> {
		vec![]
	}

	fn set_webhook_choices(
		&mut self,
		_resp: Vec<crate::llm::policy::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		Ok(())
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(self)
	}
}

/// 'typed' provides a strictly-typed internal representation of the OpenAI embeddings API.
/// This is used as a normalization bridge for non-OpenAI providers (e.g. Bedrock, Vertex).
/// These providers are converted into these typed structs first to ensure validity,
/// and then converted back to the top-level passthrough-preserving structs for the client.
pub mod typed {
	use serde::{Deserialize, Serialize};

	#[derive(Debug, Serialize, Default, Clone, Copy, PartialEq, Deserialize)]
	#[serde(rename_all = "lowercase")]
	pub enum EncodingFormat {
		#[default]
		Float,
		Base64,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Request {
		pub model: String,
		pub input: EmbeddingInput,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub user: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub encoding_format: Option<EncodingFormat>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub dimensions: Option<u32>,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	#[serde(untagged)]
	pub enum EmbeddingInput {
		String(String),
		Array(Vec<String>),
	}

	impl EmbeddingInput {
		pub fn as_strings(&self) -> Vec<String> {
			match self {
				EmbeddingInput::String(s) => vec![s.clone()],
				EmbeddingInput::Array(arr) => arr.clone(),
			}
		}

		pub fn first(&self) -> Option<&str> {
			match self {
				EmbeddingInput::String(s) => Some(s),
				EmbeddingInput::Array(arr) => arr.first().map(|s| s.as_str()),
			}
		}
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Response {
		pub object: String,
		pub model: String,
		pub data: Vec<Embedding>,
		pub usage: Usage,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Embedding {
		pub index: u32,
		pub object: String,
		pub embedding: Vec<f32>,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Usage {
		pub prompt_tokens: u32,
		pub total_tokens: u32,
	}
}
