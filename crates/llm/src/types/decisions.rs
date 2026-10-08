use agent_core::prelude::Strng;
use agent_core::strng;
use serde::{Deserialize, Serialize};

use crate::types::{ContentScope, RequestType, responses};
use crate::{AIError, InputFormat, LLMRequest, LLMRequestParams, SimpleChatCompletionMessage};

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<responses::Usage>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl RequestType for Request {
	fn input_format() -> InputFormat {
		InputFormat::Decisions
	}
	fn body_is_json(&self) -> bool {
		true
	}
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn to_value(&self) -> serde_json::Result<serde_json::Value> {
		serde_json::to_value(self)
	}

	fn prepend_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {}

	fn append_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {}

	fn to_llm_request(&self, provider: Strng, _tokenize: bool) -> Result<LLMRequest, AIError> {
		Ok(LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Decisions,
			cache_convention: crate::CacheTokenConvention::pending(),
			request_model: strng::new(self.model.as_deref().unwrap_or_default()),
			provider,
			streaming: false,
			params: LLMRequestParams::default(),
			prompt: Default::default(),
			provider_state: None,
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		unimplemented!("get_messages is used for prompt guard; prompt guard is disabled for decisions.")
	}

	fn set_messages(&mut self, _messages: Vec<SimpleChatCompletionMessage>) {
		unimplemented!("set_messages is used for prompt guard; prompt guard is disabled for decisions.")
	}

	fn visit_text_mut(&mut self, _f: &mut dyn FnMut(ContentScope, &mut String)) {
		unimplemented!(
			"visit_text_mut is used for prompt guard; prompt guard is disabled for decisions."
		)
	}
}

impl crate::types::ResponseType for Response {
	fn to_llm_response(&self, _log_content: crate::LogContentFields) -> crate::LLMResponse {
		let usage = self.usage.as_ref();
		crate::LLMResponse {
			input_tokens: usage.map(|u| u.input_tokens),
			output_tokens: usage.map(|u| u.output_tokens),
			total_tokens: usage.map(|u| u.total_tokens.unwrap_or(u.input_tokens + u.output_tokens)),
			reasoning_tokens: usage
				.and_then(|u| u.output_tokens_details.as_ref())
				.and_then(|d| d.reasoning_tokens),
			cached_input_tokens: usage
				.and_then(|u| u.input_tokens_details.as_ref())
				.and_then(|d| d.cached_tokens),
			cache_creation_input_tokens: usage
				.and_then(|u| u.input_tokens_details.as_ref())
				.and_then(|d| d.cache_write_tokens),
			provider_model: self.model.as_deref().map(strng::new),
			..Default::default()
		}
	}

	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice> {
		vec![]
	}

	fn set_webhook_choices(
		&mut self,
		_resp: Vec<crate::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		Ok(())
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(self)
	}

	fn visit_text_mut(&mut self, _f: &mut dyn FnMut(crate::types::ResponseText, &mut String)) {}
}
