use crate::llm::bedrock::Provider;
use crate::llm::policy::PromptCachingConfig;
use crate::llm::policy::webhook::ResponseChoice;
use crate::llm::{
	AIError, AmendOnDrop, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, RequestType,
	ResponseType, SimpleChatCompletionMessage,
};
use crate::{json, llm, parse};
use agent_core::prelude::Strng;
use bytes::Bytes;
use http::HeaderMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tracing::debug;

fn lookup<'a, T, const C: usize>(
	value: &'a Value,
	paths: [&[&str]; C],
	f: impl Fn(&'a Value) -> Option<T>,
) -> Option<T> {
	for path in paths {
		if let Some(s) = json::traverse(value, path).and_then(&f) {
			return Some(s);
		}
	}
	None
}

#[derive(Clone, Debug)]
pub enum Request {
	Raw(Bytes),
	Json(serde_json::Value),
}

impl<'de> Deserialize<'de> for Request {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let v = Value::deserialize(deserializer)?;
		Ok(Request::Json(v))
	}
}

impl Request {
	pub fn new_raw(body: Bytes) -> Self {
		Self::Raw(body)
	}
	pub fn lookup<'a, T, const C: usize>(
		&'a self,
		path: [&[&str]; C],
		f: impl Fn(&'a Value) -> Option<T>,
	) -> Option<T> {
		match &self {
			Self::Raw(_) => None,
			Self::Json(b) => lookup(b, path, f),
		}
	}
}

impl RequestType for Request {
	fn supports_model(&self) -> bool {
		false
	}
	fn model(&mut self) -> &mut Option<String> {
		unimplemented!("model is not available");
	}

	fn prepend_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {
		// Not supported
	}

	fn append_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {
		// Not supported
	}

	fn to_llm_request(&self, provider: Strng, _tokenize: bool) -> Result<LLMRequest, AIError> {
		Ok(LLMRequest {
			// We never tokenize these, so always empty
			input_tokens: None,
			input_format: InputFormat::Detect,
			request_model: self
				.lookup(lookups::MODEL, |v| v.as_str())
				.map(Into::into)
				.unwrap_or_default(),
			provider,
			streaming: self
				.lookup(lookups::STREAM, |v| v.as_bool())
				.unwrap_or_default(),
			params: LLMRequestParams {
				temperature: self.lookup(lookups::TEMPERATURE, |v| v.as_f64()),
				top_p: self.lookup(lookups::TOP_P, |v| v.as_f64()),
				frequency_penalty: self.lookup(lookups::FREQUENCY_PENALTY, |v| v.as_f64()),
				presence_penalty: self.lookup(lookups::PRESENCE_PENALTY, |v| v.as_f64()),
				seed: self.lookup(lookups::SEED, |v| v.as_i64()),
				max_tokens: self.lookup(lookups::MAX_TOKENS, |v| v.as_u64()),
				encoding_format: self
					.lookup(lookups::ENCODING_FORMAT, |v| v.as_str())
					.map(Into::into),
				dimensions: self.lookup(lookups::DIMENSIONS, |v| v.as_u64()),
			},
			prompt: Default::default(),
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		unimplemented!("get_messages is used for prompt guard; prompt guard is disabled for detect.")
	}

	fn set_messages(&mut self, _messages: Vec<SimpleChatCompletionMessage>) {
		unimplemented!("set_messages is used for prompt guard; prompt guard is disabled for detect.")
	}
	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		match self {
			Self::Raw(bytes) => Ok(bytes.to_vec()),
			Self::Json(v) => serde_json::to_vec(v).map_err(AIError::RequestMarshal),
		}
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
	fn to_bedrock_token_count(&self, _headers: &::http::HeaderMap) -> Result<Vec<u8>, AIError> {
		self.to_openai()
	}
	fn to_vertex(&self, _provider: &crate::llm::vertex::Provider) -> Result<Vec<u8>, AIError> {
		self.to_openai()
	}
}

#[derive(Debug, Clone)]
pub enum Response {
	Raw(Bytes),
	Json(serde_json::Value),
}
impl Response {
	pub fn new_raw(body: Bytes) -> Self {
		Self::Raw(body)
	}
	pub fn lookup<'a, T, const C: usize>(
		&'a self,
		path: [&[&str]; C],
		f: impl Fn(&'a Value) -> Option<T>,
	) -> Option<T> {
		match &self {
			Self::Raw(_) => None,
			Self::Json(b) => lookup(b, path, f),
		}
	}
}

mod lookups {
	pub const MODEL: [&[&str]; 1] = [&["model"]];
	pub const TEMPERATURE: [&[&str]; 1] = [&["temperature"]];
	pub const STREAM: [&[&str]; 1] = [&["stream"]];
	pub const TOP_P: [&[&str]; 1] = [&["top_p"]];
	pub const FREQUENCY_PENALTY: [&[&str]; 1] = [&["frequency_penalty"]];
	pub const PRESENCE_PENALTY: [&[&str]; 1] = [&["presence_penalty"]];
	pub const SEED: [&[&str]; 1] = [&["seed"]];
	pub const MAX_TOKENS: [&[&str]; 2] = [&["max_completion_tokens"], &["max_tokens"]];
	pub const ENCODING_FORMAT: [&[&str]; 1] = [&["encoding_format"]];
	pub const DIMENSIONS: [&[&str]; 1] = [&["dimensions"]];
	pub const USAGE_INPUT_TOKENS: [&[&str]; 3] = [
		&["usage", "input_tokens"],
		// Responses streaming
		&["response", "usage", "input_tokens"],
		&["usage", "prompt_tokens"],
	];
	pub const USAGE_OUTPUT_TOKENS: [&[&str]; 3] = [
		&["usage", "output_tokens"],
		// Responses streaming
		&["response", "usage", "output_tokens"],
		&["usage", "completion_tokens"],
	];
	pub const USAGE_TOTAL_TOKENS: [&[&str]; 1] = [&["usage", "total_tokens"]];
	pub const REASONING: [&[&str]; 3] = [
		// Responses
		&["usage", "output_tokens_details", "reasoning_tokens"],
		// Responses streaming
		&[
			"response",
			"usage",
			"output_tokens_details",
			"reasoning_tokens",
		],
		// Completions
		&["usage", "completion_tokens_details", "reasoning_tokens"],
	];
	pub const CACHE_CREATION_INPUT_TOKENS: [&[&str]; 1] = [&["usage", "cache_creation_input_tokens"]];
	pub const CACHED_INPUT_TOKENS: [&[&str]; 4] = [
		// Message
		&["usage", "cache_read_input_tokens"],
		// Responses
		&["usage", "input_tokens_details", "cached_tokens"],
		// Responses streaming
		&["response", "usage", "input_tokens_details", "cached_tokens"],
		// Completions
		&["usage", "prompt_tokens_details", "cached_tokens"],
	];
}

impl<'de> Deserialize<'de> for Response {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let v = Value::deserialize(deserializer)?;
		Ok(Response::Json(v))
	}
}

impl ResponseType for Response {
	fn to_llm_response(&self, _include_completion_in_log: bool) -> LLMResponse {
		let input_tokens = self.lookup(lookups::USAGE_INPUT_TOKENS, |v| v.as_u64());
		let output_tokens = self.lookup(lookups::USAGE_OUTPUT_TOKENS, |v| v.as_u64());
		let total_tokens = self.lookup(lookups::USAGE_TOTAL_TOKENS, |v| v.as_u64());
		crate::llm::LLMResponse {
			count_tokens: None, // We never tokenize these, so always empty
			input_tokens,
			output_tokens,
			total_tokens: total_tokens.or_else(|| Some(input_tokens? + output_tokens?)),
			reasoning_tokens: self.lookup(lookups::REASONING, |v| v.as_u64()),
			cache_creation_input_tokens: self
				.lookup(lookups::CACHE_CREATION_INPUT_TOKENS, |v| v.as_u64()),
			cached_input_tokens: self.lookup(lookups::CACHED_INPUT_TOKENS, |v| v.as_u64()),
			provider_model: self.lookup(lookups::MODEL, |v| v.as_str()).map(Into::into),
			completion: None,
			// TODO: we could probably derive this
			first_token: None,
		}
	}

	fn to_webhook_choices(&self) -> Vec<ResponseChoice> {
		unimplemented!(
			"to_webhook_choices is used for prompt guard; prompt guard is disabled for detect."
		)
	}

	fn set_webhook_choices(&mut self, _resp: Vec<ResponseChoice>) -> anyhow::Result<()> {
		unimplemented!(
			"set_webhook_choices is used for prompt guard; prompt guard is disabled for detect."
		)
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		match self {
			Self::Raw(bytes) => Ok(bytes.to_vec()),
			Self::Json(v) => Ok(serde_json::to_vec(v)?),
		}
	}
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct StreamResponse {
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl StreamResponse {
	fn set_if<'a, T: Copy, const C: usize>(
		&'a self,
		log: &AmendOnDrop,
		paths: [&[&str]; C],
		cvt: impl Fn(&'a Value) -> Option<T>,
		apply: impl Fn(&mut llm::LLMInfo, T),
	) -> Option<T> {
		if let Some(res) = lookup(&self.rest, paths, cvt) {
			log.non_atomic_mutate(|l| apply(l, res));
			Some(res)
		} else {
			None
		}
	}
}

pub fn passthrough_stream(
	mut log: AmendOnDrop,
	resp: crate::http::Response,
) -> crate::http::Response {
	let buffer_limit = crate::http::response_buffer_limit(&resp);
	resp.map(|b| {
		parse::sse::permissive_json_passthrough::<StreamResponse>(b, buffer_limit, move |f| match f {
			Some(Ok(f)) => {
				let input_tokens = f.set_if(
					&log,
					lookups::USAGE_INPUT_TOKENS,
					|v| v.as_u64(),
					|l, v| l.response.input_tokens = Some(v),
				);
				let output_tokens = f.set_if(
					&log,
					lookups::USAGE_OUTPUT_TOKENS,
					|v| v.as_u64(),
					|l, v| l.response.output_tokens = Some(v),
				);
				let total_tokens = f.set_if(
					&log,
					lookups::USAGE_TOTAL_TOKENS,
					|v| v.as_u64(),
					|l, v| l.response.total_tokens = Some(v),
				);
				let reasoning_tokens = f.set_if(
					&log,
					lookups::REASONING,
					|v| v.as_u64(),
					|l, v| l.response.reasoning_tokens = Some(v),
				);
				let cache_creation_input_tokens = f.set_if(
					&log,
					lookups::CACHE_CREATION_INPUT_TOKENS,
					|v| v.as_u64(),
					|l, v| l.response.cache_creation_input_tokens = Some(v),
				);
				let cached_input_tokens = f.set_if(
					&log,
					lookups::CACHED_INPUT_TOKENS,
					|v| v.as_u64(),
					|l, v| l.response.cached_input_tokens = Some(v),
				);
				let provider_model = f.set_if(
					&log,
					lookups::MODEL,
					|v| v.as_str(),
					|l, v| l.response.provider_model = Some(v.into()),
				);
				if total_tokens.is_none()
					&& let (Some(input), Some(output)) = (input_tokens, output_tokens)
				{
					log.non_atomic_mutate(|l| l.response.total_tokens = Some(input + output));
				}
				if input_tokens.is_some()
					|| output_tokens.is_some()
					|| total_tokens.is_some()
					|| reasoning_tokens.is_some()
					|| cache_creation_input_tokens.is_some()
					|| cached_input_tokens.is_some()
					|| provider_model.is_some()
				{
					log.report_rate_limit();
				}
			},
			Some(Err(e)) => {
				debug!("failed to parse streaming response: {e}");
			},
			None => {},
		})
	})
}
