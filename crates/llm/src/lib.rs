use std::sync::Arc;
use std::time::{Duration, Instant};

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
pub mod model_catalog;
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

/// A model id is interpolated into a single-segment slot of an upstream path
/// (`.../models/{model}:generateContent`), so a `/` in one is either a resource-style name
/// (`models/x`, `tunedModels/x`, a Bedrock inference-profile ARN) or an attempt to choose the
/// upstream path. Deciding that here keeps extraction and every path builder on the same answer.
pub mod model_path {
	/// One path segment: not a separator, not a dot segment, and nothing that changes meaning when
	/// the URL we build is parsed again upstream.
	pub fn is_safe_segment(segment: &str) -> bool {
		!segment.is_empty()
			&& segment != "."
			&& segment != ".."
			&& !segment.contains([
				'/', '\\', '%', '?', '#', '<', '>', '"', '`', '{', '}', '|', '^',
			]) && !segment.chars().any(|c| c.is_control() || c.is_whitespace())
	}

	pub fn is_safe_resource_name(model: &str) -> bool {
		!model.is_empty() && model.split('/').all(is_safe_segment)
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn safe_names_are_accepted() {
			for model in [
				"gemini-2.5-flash",
				"gemini@001",
				"claude-3-5-sonnet-20241022-v2:0",
				"models/gemini-2.5-flash",
				"tunedModels/abc",
				"publishers/google/models/gemini-2.5-flash",
				"arn:aws:bedrock:us-east-1:1234:application-inference-profile/my-profile",
			] {
				assert!(is_safe_resource_name(model), "{model}");
			}
		}

		#[test]
		fn unsafe_names_are_rejected() {
			for model in [
				"",
				" ",
				"..",
				".",
				"gemini-2.5-flash/../../locations/global/endpoints/openapi/chat/completions",
				"gemini-2.5-flash/..",
				"/gemini-2.5-flash",
				"gemini-2.5-flash/",
				"gemini//flash",
				"gemini-2.5-flash%2F..",
				"gemini\\..\\..",
				"gemini 2.5 flash",
				"gemini\n",
				// A query or fragment would re-shape the path we build, dropping the `:method` suffix.
				"gemini-2.5-flash?alt=sse",
				"gemini-2.5-flash#frag",
				"gemini-2.5-flash<x",
			] {
				assert!(!is_safe_resource_name(model), "{model}");
			}
		}
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
	/// Gemini models/{model}:generateContent and models/{model}:streamGenerateContent
	GenerateContent,
	/// Gemini models/{model}:countTokens
	GeminiCountTokens,
	/// Cohere /v2/rerank (document reranking)
	Rerank,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub enum InputFormat {
	Completions,
	Messages,
	Responses,
	Embeddings,
	Realtime,
	/// Anthropic-shaped /v1/messages/count_tokens body
	CountTokens,
	Detect,
	Rerank,
	/// Native Gemini generateContent body
	Gemini,
	/// Native Gemini countTokens body
	GeminiCountTokens,
}

impl InputFormat {
	pub fn is_chat(&self) -> bool {
		matches!(
			self,
			InputFormat::Completions
				| InputFormat::Messages
				| InputFormat::Responses
				| InputFormat::Gemini
		)
	}

	pub fn supports_prompt_guard(&self) -> bool {
		match self {
			InputFormat::Completions => true,
			InputFormat::Messages => true,
			InputFormat::Responses => true,
			InputFormat::Gemini => true,
			InputFormat::Realtime => false,
			InputFormat::Embeddings => false,
			InputFormat::CountTokens => false,
			InputFormat::GeminiCountTokens => false,
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

impl ChatFormat {
	pub fn tag(&self) -> &'static str {
		use crate::model_catalog::tags;
		match self {
			ChatFormat::OpenAICompletions => tags::OPENAI_COMPLETIONS,
			ChatFormat::OpenAIResponses => tags::OPENAI_RESPONSES,
			ChatFormat::AnthropicMessages => tags::ANTHROPIC_MESSAGES,
			ChatFormat::BedrockConverse => tags::BEDROCK_CONVERSE,
			ChatFormat::VertexGemini => tags::VERTEX_GEMINI,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LLMRequest {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_tokens: Option<u64>,
	pub input_format: InputFormat,
	pub cache_convention: CacheTokenConvention,
	pub request_model: Strng,
	pub provider: Strng,
	pub streaming: bool,
	pub params: LLMRequestParams,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub prompt: Option<Arc<Vec<SimpleChatCompletionMessage>>>,
	#[serde(skip)]
	pub provider_state: Option<ProviderState>,
}

#[derive(Debug, Clone)]
pub enum ProviderState {
	Bedrock {
		tool_names: Arc<conversion::bedrock::BedrockToolNameMap>,
	},
	VertexGemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub enum CacheTokenConvention {
	#[default]
	InputIncludesCache,
	InputExcludesCache,
}

impl CacheTokenConvention {
	pub fn pending() -> Self {
		Self::InputIncludesCache
	}

	/// Normalize a provider-reported input token count so it always includes
	/// tokens read from and written to cache.
	pub fn include_cache_tokens(
		self,
		input_tokens: u64,
		cached_input_tokens: Option<u64>,
		cache_creation_input_tokens: Option<u64>,
	) -> u64 {
		match self {
			Self::InputIncludesCache => input_tokens,
			Self::InputExcludesCache => input_tokens
				.saturating_add(cached_input_tokens.unwrap_or_default())
				.saturating_add(cache_creation_input_tokens.unwrap_or_default()),
		}
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

	/// Return a cache-inclusive input token count with consistent semantics across providers.
	/// Falls back to the request-side tokenizer when the response has no usage count.
	pub fn normalized_input_tokens(&self) -> Option<u64> {
		self
			.response
			.input_tokens
			.map(|input_tokens| {
				self.request.cache_convention.include_cache_tokens(
					input_tokens,
					self.response.cached_input_tokens,
					self.response.cache_creation_input_tokens,
				)
			})
			.or(self.request.input_tokens)
	}
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LLMResponse {
	/// Provider-reported input tokens. Whether this includes cache tokens is described by the
	/// corresponding request's [`CacheTokenConvention`].
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
	/// Timestamp of the most recently observed output token, used to compute
	/// inter_chunk_latencies. Not the same as first_token once more than one token has arrived.
	#[serde(skip)]
	pub last_token_at: Option<Instant>,
	/// Bucketed summary of gaps between consecutive output chunks, replayed into
	/// the inter-chunk-latency histogram at request completion.
	#[serde(skip)]
	pub inter_chunk_latencies: TokenGapSummary,
}

/// Upper bounds for bucketing inter-chunk gaps before they're replayed into the
/// Prometheus histogram at request completion. MUST stay numerically identical
/// to agentgateway::telemetry::metrics::OUTPUT_TOKEN_BUCKET — this crate can't
/// import that private constant, so the two arrays are kept in sync by hand.
const INTER_CHUNK_LATENCY_BUCKETS: [f64; 14] = [
	0.001, 0.01, 0.025, 0.05, 0.075, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.75, 1.0, 2.5,
];

/// Fixed-size bucketed summary of gaps between consecutive streamed output
/// chunks. Avoids buffering one Duration per chunk (unbounded for very long
/// streaming responses) while still letting the exact per-bucket counts and
/// sum be replayed into the real histogram once labels are known, at request
/// completion.
#[derive(Debug, Clone)]
pub struct TokenGapSummary {
	// One (count, sum_of_values) pair per INTER_CHUNK_LATENCY_BUCKETS entry,
	// plus one trailing overflow bucket for values above the last boundary.
	buckets: [(u64, f64); INTER_CHUNK_LATENCY_BUCKETS.len() + 1],
}

impl Default for TokenGapSummary {
	fn default() -> Self {
		Self {
			buckets: [(0, 0.0); INTER_CHUNK_LATENCY_BUCKETS.len() + 1],
		}
	}
}

impl TokenGapSummary {
	pub fn record(&mut self, gap: Duration) {
		let v = gap.as_secs_f64();
		let idx = INTER_CHUNK_LATENCY_BUCKETS
			.iter()
			.position(|&upper| v <= upper)
			.unwrap_or(INTER_CHUNK_LATENCY_BUCKETS.len());
		let (count, sum) = &mut self.buckets[idx];
		*count += 1;
		*sum += v;
	}

	pub fn is_empty(&self) -> bool {
		self.buckets.iter().all(|(c, _)| *c == 0)
	}

	/// (count, mean_value) pairs in bucket order, for replaying into a
	/// Prometheus histogram at request completion.
	pub fn iter(&self) -> impl Iterator<Item = (u64, f64)> + '_ {
		self
			.buckets
			.iter()
			.map(|&(count, sum)| (count, if count == 0 { 0.0 } else { sum / count as f64 }))
	}
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
	ContentScope, OutputMessage, OutputMessagePart, RequestType, ResponseType,
	SimpleChatCompletionMessage, ToolCall,
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
	#[error("failed to decode response: {0}")]
	ResponseDecoding(axum_core::Error),
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

#[cfg(test)]
mod token_gap_summary_tests {
	use super::*;

	#[test]
	fn empty_summary_reports_empty() {
		assert!(TokenGapSummary::default().is_empty());
	}

	#[test]
	fn record_buckets_and_sums_correctly() {
		let mut summary = TokenGapSummary::default();
		// Falls in the first bucket (upper bound 0.001).
		summary.record(Duration::from_micros(500));
		// Falls in the last defined bucket (upper bound 2.5).
		summary.record(Duration::from_millis(2000));
		summary.record(Duration::from_millis(2400));
		// Exceeds every defined bucket; goes into the overflow bucket.
		summary.record(Duration::from_secs(10));

		assert!(!summary.is_empty());

		let buckets: Vec<(u64, f64)> = summary.iter().collect();
		assert_eq!(buckets.len(), INTER_CHUNK_LATENCY_BUCKETS.len() + 1);

		// First bucket: one observation of 0.0005s.
		assert_eq!(buckets[0], (1, 0.0005));

		// Last defined bucket (index 13, upper bound 2.5): two observations,
		// mean should be their average.
		let last_defined = buckets[INTER_CHUNK_LATENCY_BUCKETS.len() - 1];
		assert_eq!(last_defined.0, 2);
		assert!((last_defined.1 - 2.2).abs() < 1e-9);

		// Overflow bucket: one observation of 10s.
		let overflow = buckets[INTER_CHUNK_LATENCY_BUCKETS.len()];
		assert_eq!(overflow, (1, 10.0));

		// All other buckets remain untouched.
		for (i, &(count, mean)) in buckets.iter().enumerate() {
			if i == 0 || i == INTER_CHUNK_LATENCY_BUCKETS.len() - 1 || i == INTER_CHUNK_LATENCY_BUCKETS.len() {
				continue;
			}
			assert_eq!(count, 0);
			assert_eq!(mean, 0.0);
		}
	}
}
