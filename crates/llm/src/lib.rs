use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use agent_core::prelude::Strng;
use aws_credential_types::Credentials;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};
use tracing::warn;

pub use agent_core::serdes;
pub use agent_core::serdes::{JsonSchema, apply, attribute_alias, define_schema_aliases};

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
pub mod types;
pub mod vertex;

#[cfg(test)]
mod golden_tests;

pub mod llm {
	pub use crate::*;
}

pub trait Provider {
	const NAME: Strng;
}

pub mod http {
	pub type Error = axum_core::Error;
	pub type Body = axum_core::body::Body;
	pub type Request = ::http::Request<Body>;
	pub type Response = ::http::Response<Body>;

	pub use ::http::{HeaderMap, HeaderName, HeaderValue, header};

	pub mod x_headers {
		pub const TRACEPARENT: http::HeaderName = http::HeaderName::from_static("traceparent");
		pub const X_AMZN_REQUESTID: http::HeaderName =
			http::HeaderName::from_static("x-amzn-requestid");
	}

	pub mod auth {
		pub mod aws {
			pub use crate::auth::{AwsAssumeRoleCache, AwsCredentialsCache};
		}

		pub mod azure {
			pub use crate::auth::AzureCredentialCache;
		}
	}

	pub fn buffer_limit(_req: &Request) -> usize {
		2_097_152
	}

	pub fn response_buffer_limit(_resp: &Response) -> usize {
		2_097_152
	}

	pub async fn read_body_with_limit(body: Body, limit: usize) -> Result<bytes::Bytes, Error> {
		axum::body::to_bytes(body, limit).await
	}
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

pub mod policy {
	pub use crate::PromptCachingConfig;

	pub mod webhook {
		use serde::{Deserialize, Serialize};

		pub type Message = crate::SimpleChatCompletionMessage;

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct GuardrailsPromptRequest {
			/// body contains the object which is a list of the Message JSON objects from the prompts in the request
			pub body: PromptMessages,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct GuardrailsPromptResponse {
			/// action is the action to be taken based on the request.
			/// The following actions are available on the response:
			/// - PassAction: No action is required.
			/// - MaskAction: Mask the response body.
			/// - RejectAction: Reject the request.
			pub action: RequestAction,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct GuardrailsResponseRequest {
			/// body contains the object with a list of Choice that contains the response content from the LLM.
			pub body: ResponseChoices,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct GuardrailsResponseResponse {
			/// action is the action to be taken based on the request.
			/// The following actions are available on the response:
			/// - PassAction: No action is required.
			/// - MaskAction: Mask the response body.
			/// - RejectAction: Reject the response.
			pub action: ResponseAction,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct PromptMessages {
			/// List of prompt messages including role and content.
			pub messages: Vec<Message>,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct ResponseChoice {
			/// message contains the role and text content of the response from the LLM model.
			pub message: Message,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct ResponseChoices {
			/// list of possible independent responses from the LLM
			pub choices: Vec<ResponseChoice>,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct PassAction {
			/// reason is a human readable string that explains the reason for the action.
			#[serde(skip_serializing_if = "Option::is_none")]
			pub reason: Option<String>,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct MaskAction {
			/// body contains the modified messages that masked out some of the original contents.
			/// When used in a GuardrailPromptResponse, this should be PromptMessages.
			/// When used in GuardrailResponseResponse, this should be ResponseChoices
			pub body: MaskActionBody,
			/// reason is a human readable string that explains the reason for the action.
			#[serde(skip_serializing_if = "Option::is_none")]
			pub reason: Option<String>,
		}

		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(rename_all = "snake_case")]
		pub struct RejectAction {
			/// body is the rejection message that will be used for HTTP error response body.
			pub body: String,
			/// status_code is the HTTP status code to be returned in the HTTP error response.
			pub status_code: u16,
			/// reason is a human readable string that explains the reason for the action.
			#[serde(skip_serializing_if = "Option::is_none")]
			pub reason: Option<String>,
		}

		/// Enum for actions available in prompt responses
		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(untagged, rename_all = "snake_case")]
		pub enum RequestAction {
			Mask(MaskAction),
			Reject(RejectAction),
			Pass(PassAction),
		}

		/// Enum for actions available in response responses
		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(untagged, rename_all = "snake_case")]
		pub enum ResponseAction {
			Mask(MaskAction),
			Reject(RejectAction),
			Pass(PassAction),
		}

		/// Enum for MaskAction body that can be either PromptMessages or ResponseChoices
		#[derive(Debug, Clone, Serialize, Deserialize)]
		#[serde(untagged)]
		pub enum MaskActionBody {
			PromptMessages(PromptMessages),
			ResponseChoices(ResponseChoices),
		}
	}
}

pub mod auth {
	use super::*;

	#[derive(Default, Clone)]
	pub struct AwsCredentialsCache(pub Arc<tokio::sync::Mutex<Option<Credentials>>>);

	impl std::fmt::Debug for AwsCredentialsCache {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("AwsCredentialsCache")
		}
	}

	#[derive(Debug, Clone, PartialEq, Eq, Hash)]
	pub struct AssumeRoleCacheKey {
		pub role_arn: String,
		pub resolved_sts_region: String,
		pub session_name: Option<String>,
		/// Pre-sorted (key, value) pairs so the cache key is stable regardless of tag order.
		pub tags: Arc<[(String, String)]>,
	}

	#[derive(Default, Clone)]
	pub struct AwsAssumeRoleCache(
		pub Arc<tokio::sync::Mutex<HashMap<AssumeRoleCacheKey, Credentials>>>,
	);

	impl std::fmt::Debug for AwsAssumeRoleCache {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("AwsAssumeRoleCache")
		}
	}

	#[derive(Default, Clone)]
	pub struct AzureCredentialCache(
		pub Arc<tokio::sync::OnceCell<Arc<dyn azure_core::credentials::TokenCredential>>>,
	);

	impl std::fmt::Debug for AzureCredentialCache {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("AzureCredentialCache")
		}
	}
}

#[apply(schema!)]
#[derive(Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteType {
	Completions,
	Messages,
	Models,
	Passthrough,
	Detect,
	Responses,
	Embeddings,
	Realtime,
	AnthropicTokenCount,
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
	#[serde(skip)]
	pub first_token: Option<Instant>,
}

#[derive(Clone)]
pub struct AsyncLog<T>(Arc<Mutex<Option<T>>>);

impl<T> AsyncLog<T> {
	pub fn non_atomic_mutate(&self, f: impl FnOnce(&mut T)) {
		let mut lock = self.0.lock().expect("async log mutex poisoned");
		if let Some(cur) = lock.as_mut() {
			f(cur);
		}
	}

	pub fn store(&self, v: Option<T>) {
		*self.0.lock().expect("async log mutex poisoned") = v;
	}

	pub fn take(&self) -> Option<T> {
		self.0.lock().expect("async log mutex poisoned").take()
	}
}

impl<T> Default for AsyncLog<T> {
	fn default() -> Self {
		Self(Arc::new(Mutex::new(None)))
	}
}

#[derive(Clone)]
pub struct AmendOnDrop {
	mutate: Arc<dyn Fn(&mut dyn FnMut(&mut LLMInfo)) + Send + Sync>,
	report: Arc<dyn Fn() + Send + Sync>,
}

impl AmendOnDrop {
	pub fn new(log: AsyncLog<LLMInfo>) -> Self {
		Self {
			mutate: Arc::new(move |f| log.non_atomic_mutate(f)),
			report: Arc::new(|| {}),
		}
	}

	pub fn from_callbacks(
		mutate: impl Fn(&mut dyn FnMut(&mut LLMInfo)) + Send + Sync + 'static,
		report: impl Fn() + Send + Sync + 'static,
	) -> Self {
		Self {
			mutate: Arc::new(mutate),
			report: Arc::new(report),
		}
	}

	pub fn non_atomic_mutate(&self, mut f: impl FnMut(&mut LLMInfo)) {
		(self.mutate)(&mut f);
	}

	pub fn report_rate_limit(&mut self) {
		(self.report)();
	}
}

impl Default for AmendOnDrop {
	fn default() -> Self {
		Self::new(AsyncLog::default())
	}
}

pub use types::{RequestType, ResponseType, SimpleChatCompletionMessage};

pub fn num_tokens_from_messages(
	model: &str,
	messages: &[SimpleChatCompletionMessage],
) -> Result<u64, AIError> {
	let tokenizer = get_tokenizer(model).unwrap_or(Tokenizer::Cl100kBase);
	if tokenizer != Tokenizer::Cl100kBase && tokenizer != Tokenizer::O200kBase {
		return Err(AIError::UnsupportedModel);
	}
	let bpe = get_bpe_from_tokenizer(tokenizer);
	let tokens_per_message = 3;

	let mut num_tokens: u64 = 0;
	for message in messages {
		num_tokens += tokens_per_message;
		num_tokens += 1;
		num_tokens += bpe
			.encode_with_special_tokens(message.content.as_str())
			.len() as u64;
	}
	num_tokens += 3;
	Ok(num_tokens)
}

pub fn preload_tokenizers() {
	let _ = tiktoken_rs::cl100k_base_singleton();
	let _ = tiktoken_rs::o200k_base_singleton();
}

pub fn get_bpe_from_tokenizer<'a>(tokenizer: Tokenizer) -> &'a CoreBPE {
	match tokenizer {
		Tokenizer::O200kHarmony => tiktoken_rs::o200k_harmony_singleton(),
		Tokenizer::O200kBase => tiktoken_rs::o200k_base_singleton(),
		Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
		Tokenizer::R50kBase => tiktoken_rs::r50k_base_singleton(),
		Tokenizer::P50kBase => tiktoken_rs::r50k_base_singleton(),
		Tokenizer::P50kEdit => tiktoken_rs::r50k_base_singleton(),
		Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
	}
}

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
	#[serde(rename = "cacheSystem")]
	pub cache_system: bool,
	#[serde(rename = "cacheMessages")]
	pub cache_messages: bool,
	#[serde(rename = "cacheTools")]
	pub cache_tools: bool,
	#[serde(rename = "minTokens")]
	pub min_tokens: Option<usize>,
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
