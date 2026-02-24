use crate::json::traverse;
use crate::llm::bedrock::Provider;
use crate::llm::policy::PromptCachingConfig;
use crate::llm::policy::webhook::ResponseChoice;
use crate::llm::types::completions::{Choice, Usage};
use crate::llm::types::messages;
use crate::llm::{
	AIError, AmendOnDrop, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, RequestType,
	ResponseType, SimpleChatCompletionMessage, conversion, types,
};
use crate::{json, parse};
use agent_core::prelude::Strng;
use agent_core::strng;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;
use tracing::debug;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Request {
	blblblb: Option<String>,
	#[serde(flatten)]
	pub rest: serde_json::Value,
}

impl Request {
	pub fn lookup<'a, T>(&'a self, path: &[&str], f: impl Fn(&'a Value) -> Option<T>) -> Option<T> {
		crate::json::must_traverse(&self.rest, path, f).ok()
	}
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.blblblb
	}

	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		// Not supported
	}

	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		// Not supported
	}

	fn to_llm_request(&self, provider: Strng, _tokenize: bool) -> Result<LLMRequest, AIError> {
		Ok(LLMRequest {
			// We never tokenize these, so always empty
			input_tokens: None,
			input_format: InputFormat::Detect,
			request_model: self
				.lookup(&["model"], |v| v.as_str())
				.map(Into::into)
				.unwrap_or_default(),
			provider,
			streaming: self
				.lookup(&["temperature"], |v| v.as_bool())
				.unwrap_or_default(),
			params: LLMRequestParams {
				temperature: self.lookup(&["temperature"], |v| v.as_f64()),
				top_p: self.lookup(&["top_p"], |v| v.as_f64()),
				frequency_penalty: self.lookup(&["frequency_penalty"], |v| v.as_f64()),
				presence_penalty: self.lookup(&["presence_penalty"], |v| v.as_f64()),
				seed: self.lookup(&["seed"], |v| v.as_i64()),
				max_tokens: self.lookup(&["max_tokens"], |v| v.as_u64()),
				encoding_format: self
					.lookup(&["encoding_format"], |v| v.as_str())
					.map(Into::into),
				dimensions: self.lookup(&["dimensions"], |v| v.as_u64()),
			},
			prompt: Default::default(),
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		unimplemented!("get_messages is used for prompt guard; prompt guard is disable for detect.")
	}

	fn set_messages(&mut self, _messages: Vec<SimpleChatCompletionMessage>) {
		unimplemented!("set_messages is used for prompt guard; prompt guard is disable for detect.")
	}
	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		tracing::error!("howardjohn: DETECT");
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}
	fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
		self.to_openai()
	}

	fn to_bedrock(
		&self,
		_provider: &Provider,
		_headers: Option<&HeaderMap>,
		_prompt_caching: Option<&PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		self.to_openai()
	}
	fn to_bedrock_token_count(&self, headers: &::http::HeaderMap) -> Result<Vec<u8>, AIError> {
		self.to_openai()
	}
	fn to_vertex(&self, provider: &crate::llm::vertex::Provider) -> Result<Vec<u8>, AIError> {
		self.to_openai()
	}
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}
impl Response {
	pub fn lookup<'a, T>(&'a self, path: &[&str], f: impl Fn(&'a Value) -> Option<T>) -> Option<T> {
		crate::json::must_traverse(&self.rest, path, f).ok()
	}
}
impl ResponseType for Response {
	fn to_llm_response(&self, _include_completion_in_log: bool) -> LLMResponse {
		crate::llm::LLMResponse {
			input_tokens: self.lookup(&["usage", "input_tokens"], |v| v.as_u64()),
			count_tokens: None,
			output_tokens: self.lookup(&["usage", "output_tokens"], |v| v.as_u64()),
			total_tokens: None,
			reasoning_tokens: None,
			cache_creation_input_tokens: None,
			cached_input_tokens: None,
			provider_model: None,
			completion: None,
			first_token: None,
		}
	}

	fn to_webhook_choices(&self) -> Vec<ResponseChoice> {
		unimplemented!(
			"to_webhook_choices is used for prompt guard; prompt guard is disable for detect."
		)
	}

	fn set_webhook_choices(&mut self, resp: Vec<ResponseChoice>) -> anyhow::Result<()> {
		unimplemented!(
			"to_webhook_choices is used for prompt guard; prompt guard is disable for detect."
		)
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self)
	}
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct StreamResponse {
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}
pub fn passthrough_stream(
	mut log: AmendOnDrop,
	resp: crate::http::Response,
) -> crate::http::Response {
	let buffer_limit = crate::http::response_buffer_limit(&resp);
	resp.map(|b| {
		parse::sse::json_passthrough::<StreamResponse>(b, buffer_limit, move |f| match f {
			Some(Ok(f)) => {
				tracing::error!("howardjohn: parsed {f:?}");
			},
			Some(Err(e)) => {
				debug!("failed to parse streaming response: {e}");
			},
			None => {},
		})
	})
}
