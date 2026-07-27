use std::sync::Arc;
use std::time::Instant;

use agent_core::prelude::Strng;
pub use agent_core::serdes;
pub use agent_core::serdes::{JsonSchema, apply, attribute_alias, define_schema_aliases};
use tracing::warn;

define_schema_aliases!();

pub mod anthropic;
pub mod azure;
pub mod bedrock;
pub mod conversion;
pub mod copilot;
pub mod custom;
pub mod gemini;
pub mod openai;
pub mod parse;
pub mod tokenizer;
pub mod types;
pub mod vertex;

#[cfg(test)]
mod golden_tests;

pub trait Provider {
	const NAME: Strng;
}

pub mod json {
	use serde::Serialize;
	use serde::de::DeserializeOwned;
	use serde_json::Value;

	pub fn traverse<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
		if path.is_empty() {
			return Some(value);
		}
		path.iter().try_fold(value, |target, token| match target {
			Value::Object(map) => map.get(*token),
			Value::Array(list) => parse_index(token).and_then(|x| list.get(x)),
			_ => None,
		})
	}

	fn parse_index(s: &str) -> Option<usize> {
		if s.starts_with('+') || (s.starts_with('0') && s.len() != 1) {
			return None;
		}
		s.parse().ok()
	}

	pub fn convert<I: Serialize, O: DeserializeOwned>(input: &I) -> Result<O, serde_json::Error> {
		let v = serde_json::to_value(input)?;
		serde_json::from_value::<O>(v)
	}
}

pub mod webhook {
	use serde::{Deserialize, Serialize};

	pub type Message = crate::SimpleChatCompletionMessage;

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct ResponseChoice {
		/// message contains the role and text content of the response from the LLM model.
		pub message: Message,
	}
}

/// The HTTP endpoint class, such as `/v1/chat/completions` or `/v1/messages`.
///
/// This is used both for the client route we matched and for the upstream route
/// we finally send to. For chat, those can differ: a client Anthropic
/// `/v1/messages` request is `RouteType::Messages` and `InputFormat::Messages`,
/// but it may be translated and sent upstream as `RouteType::Completions`.
///
/// `RouteType` is about the HTTP endpoint. `InputFormat` is about the parsed
/// client payload and the response shape we owe back to that client. The main
/// difference is this type includes things like Detect and Passthrough.
#[apply(schema!)]
#[derive(Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteType {
	/// OpenAI /v1/chat/completions
	Completions,
	/// Anthropic /v1/messages
	Messages,
	/// OpenAI /v1/models
	Models,
	/// Send the request to the upstream LLM provider as-is
	Passthrough,
	/// Send the request to the upstream LLM provider as-is but attempt to extract information from it
	/// and apply a subset of policies (rate limit and telemetry; no guardrails).
	Detect,
	/// OpenAI /responses
	Responses,
	/// OpenAI /embeddings
	Embeddings,
	/// OpenAI /realtime (websockets)
	Realtime,
	/// Anthropic /v1/messages/count_tokens
	AnthropicTokenCount,
	/// Cohere /v2/rerank (document reranking)
	Rerank,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputFormat {
	Completions,
	Messages,
	Responses,
	Embeddings,
	Realtime,
	CountTokens,
	Detect,
	Rerank,
}

impl InputFormat {
	pub fn is_chat(&self) -> bool {
		matches!(
			self,
			InputFormat::Completions | InputFormat::Messages | InputFormat::Responses
		)
	}

	pub fn supports_prompt_guard(&self) -> bool {
		match self {
			InputFormat::Completions => true,
			InputFormat::Messages => true,
			InputFormat::Responses => true,
			InputFormat::Realtime => false,
			InputFormat::Embeddings => false,
			InputFormat::CountTokens => false,
			InputFormat::Detect => false,
			InputFormat::Rerank => false,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatFormat {
	OpenAICompletions,
	OpenAIResponses,
	AnthropicMessages,
	BedrockConverse,
	VertexGemini,
}

#[derive(Debug, Clone)]
pub struct LLMRequest {
	pub input_tokens: Option<u64>,
	pub input_format: InputFormat,
	pub cache_convention: CacheTokenConvention,
	pub request_model: Strng,
	pub provider: Strng,
	pub streaming: bool,
	pub params: LLMRequestParams,
	pub prompt: Option<Arc<Vec<SimpleChatCompletionMessage>>>,
	pub provider_state: Option<ProviderState>,
}

#[derive(Debug, Clone)]
pub enum ProviderState {
	Bedrock {
		tool_names: Arc<conversion::bedrock::BedrockToolNameMap>,
	},
	VertexGemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheTokenConvention {
	#[default]
	InputIncludesCache,
	InputExcludesCache,
}

impl CacheTokenConvention {
	pub fn pending() -> Self {
		Self::InputIncludesCache
	}
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize, ::cel::DynamicType)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LLMRequestParams {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub frequency_penalty: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub presence_penalty: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub seed: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub encoding_format: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub dimensions: Option<u64>,
}

impl PartialEq for LLMRequestParams {
	fn eq(&self, _: &Self) -> bool {
		false
	}
}

impl Eq for LLMRequestParams {}

#[derive(Debug, Clone)]
pub struct LLMInfo {
	pub request: LLMRequest,
	pub response: LLMResponse,
}

impl LLMInfo {
	pub fn new(req: LLMRequest, resp: LLMResponse) -> Self {
		Self {
			request: req,
			response: resp,
		}
	}

	pub fn input_tokens(&self) -> Option<u64> {
		self.response.input_tokens.or(self.request.input_tokens)
	}
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LLMResponse {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_image_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_text_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_audio_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub count_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_image_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_text_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_audio_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub total_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reasoning_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_creation_input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cached_input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub service_tier: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub provider_model: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub completion: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_messages: Option<Vec<types::OutputMessage>>,
	#[serde(skip)]
	pub first_token: Option<Instant>,
}

/// LogContentFields controls which response content is captured for observability.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogContentFields {
	/// Whether to capture the raw completion text.
	pub completion: bool,
	/// Whether to capture tool/function calls as structured output messages.
	pub tool_calls: bool,
}

pub trait StreamingUsageReporter: Send {
	fn update(&self, f: &mut dyn FnMut(&mut LLMInfo));
	fn report_usage(&mut self);
}

pub struct StreamingUsageGuard {
	reporter: Box<dyn StreamingUsageReporter>,
}

impl StreamingUsageGuard {
	pub fn new(reporter: Box<dyn StreamingUsageReporter>) -> Self {
		Self { reporter }
	}

	pub fn update(&self, mut f: impl FnMut(&mut LLMInfo)) {
		self.reporter.update(&mut f);
	}

	pub fn report_usage(&mut self) {
		self.reporter.report_usage();
	}
}

impl Default for StreamingUsageGuard {
	fn default() -> Self {
		struct NoopReporter;

		impl StreamingUsageReporter for NoopReporter {
			fn update(&self, _f: &mut dyn FnMut(&mut LLMInfo)) {}
			fn report_usage(&mut self) {}
		}

		Self::new(Box::new(NoopReporter))
	}
}

pub use types::{
	OutputMessage, OutputMessagePart, RequestType, ResponseType, SimpleChatCompletionMessage,
	ToolCall,
};

pub fn logged_response_parsing(bytes: &[u8]) -> impl FnOnce(serde_json::Error) -> AIError + '_ {
	|e| {
		const LOGGED_BODY_LIMIT: usize = 1024;
		let body = &bytes[..bytes.len().min(LOGGED_BODY_LIMIT)];
		warn!(
			error = %e,
			body = %String::from_utf8_lossy(body),
			"failed to parse response"
		);
		AIError::ResponseParsing(e)
	}
}

#[derive(thiserror::Error, Debug)]
pub enum AIError {
	#[error("missing field: {0}")]
	MissingField(Strng),
	#[error("model not found")]
	ModelNotFound,
	#[error("message not found")]
	MessageNotFound,
	#[error("response was missing fields")]
	IncompleteResponse,
	#[error("unknown model")]
	UnknownModel,
	#[error("todo: streaming is not currently supported for this provider")]
	StreamingUnsupported,
	#[error("unsupported model")]
	UnsupportedModel,
	#[error("unsupported content")]
	UnsupportedContent,
	#[error("unsupported conversion: {0}")]
	UnsupportedConversion(Strng),
	#[error("request was too large")]
	RequestTooLarge,
	#[error("response was too large")]
	ResponseTooLarge,
	#[error("prompt guard failed")]
	PromptWebhookError,
	#[error("failed to parse request: {0}")]
	RequestParsing(serde_json::Error),
	#[error("failed to marshal request: {0}")]
	RequestMarshal(serde_json::Error),
	#[error("failed to parse response: {0}")]
	ResponseParsing(serde_json::Error),
	#[error("invalid response: {0}")]
	InvalidResponse(Strng),
	#[error("failed to marshal response: {0}")]
	ResponseMarshal(serde_json::Error),
	#[error("unsupported content encoding: {0}")]
	UnsupportedEncoding(Strng),
	#[error("failed to encode response: {0}")]
	Encoding(axum_core::Error),
	#[error("error computing tokens")]
	JoinError(#[from] tokio::task::JoinError),
}

#[apply(schema!)]
#[serde(default)]
pub struct PromptCachingConfig {
	/// Add cache markers to system prompts when supported by the provider.
	#[serde(rename = "cacheSystem")]
	pub cache_system: bool,

	/// Add cache markers to chat messages when supported by the provider.
	#[serde(rename = "cacheMessages")]
	pub cache_messages: bool,

	/// Add cache markers to tool definitions when supported by the provider.
	#[serde(rename = "cacheTools")]
	pub cache_tools: bool,

	/// Minimum prompt size required before cache markers are added.
	#[serde(rename = "minTokens")]
	pub min_tokens: Option<usize>,

	/// Message offset used when choosing where to place cache markers.
	#[serde(rename = "cacheMessageOffset")]
	pub cache_message_offset: usize,
}

impl Default for PromptCachingConfig {
	fn default() -> Self {
		Self {
			cache_system: true,
			cache_messages: true,
			cache_tools: false,
			min_tokens: Some(1024),
			cache_message_offset: 0,
		}
	}
}
