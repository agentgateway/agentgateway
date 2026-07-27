use std::str::FromStr;
use std::sync::Arc;

use ::http::request::Parts;
use ::http::uri::{Authority, PathAndQuery};
use ::http::{HeaderMap, HeaderName, HeaderValue, header};
use agent_core::prelude::Strng;
use agent_core::strng;
pub use agent_llm::tokenizer::{num_tokens_from_messages, preload_tokenizers};
pub use agent_llm::{
	AIError, CacheTokenConvention, ChatFormat, InputFormat, LLMInfo, LLMRequest, LLMRequestParams,
	LLMResponse, LogContentFields, PromptCachingConfig, Provider, ProviderState, RequestType,
	ResponseType, RouteType, SimpleChatCompletionMessage, anthropic, conversion, copilot, custom,
	gemini, logged_response_parsing, openai, types,
};
use axum_extra::headers::authorization::Bearer;
use headers::{ContentEncoding, HeaderMapExt};
pub use policy::Policy;
use rand::RngExt;
use serde::de::DeserializeOwned;

use crate::http::auth::{
	AppliedBackendAuthLocation, AwsAuth, AzureAuth, BackendAuth, BackendAuthKind, GcpAuth,
};
use crate::http::jwt::Claims;
use crate::http::{Body, Request, Response};
use crate::proxy::httpproxy::PolicyClient;
use crate::store::{BackendPolicies, LLMResponsePolicies};
use crate::telemetry::log::{AsyncLog, RequestLog};
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference, Target};
use crate::types::loadbalancer::{ActiveHandle, EndpointWithInfo};
use crate::*;
pub mod model_router;
pub use agent_llm::{azure, bedrock, vertex};

pub mod cost;
pub mod policy;

use policy::streaming_guardrails::GuardedSseBody;
pub use types::{OutputMessage, OutputMessagePart, ToolCall};

use crate::cel::{Executor, LLMContext, RequestSnapshot};
use crate::proxy::dtrace;
use crate::store;

pub const LOCAL_LISTENER_NAME: &str = "llm";

#[cfg(test)]
mod anthropic_tests;

#[cfg(test)]
mod tests;

fn normalize_sse_response_headers(mut resp: Response) -> Response {
	resp.headers_mut().insert(
		header::CONTENT_TYPE,
		HeaderValue::from_static("text/event-stream"),
	);
	resp.headers_mut().remove(header::CONTENT_LENGTH);
	resp
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AIBackend {
	pub providers: crate::types::loadbalancer::EndpointSet<NamedAIProvider>,
}

impl AIBackend {
	pub fn select_provider(&self) -> Option<(Arc<NamedAIProvider>, ActiveHandle)> {
		let iter = self.providers.iter();
		let index = iter.index();
		if index.is_empty() {
			return None;
		}
		// Intentionally allow `rand::seq::index::sample` so we can pick the same element twice
		// This avoids starvation where the worst endpoint gets 0 traffic
		let a = rand::rng().random_range(0..index.len());
		let b = rand::rng().random_range(0..index.len());
		let best = [a, b]
			.into_iter()
			.map(|idx| {
				let (_, EndpointWithInfo { endpoint, info, .. }) =
					index.get_index(idx).expect("index already checked");
				(endpoint.clone(), info)
			})
			.max_by(|(_, a), (_, b)| a.score().total_cmp(&b.score()));
		let (ep, ep_info) = best?;
		let handle = self.providers.start_request(ep.name.clone(), ep_info);
		Some((ep, handle))
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedAIProvider {
	pub name: Strng,
	pub provider: AIProvider,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub provider_backend: Option<SimpleBackendReference>,
	pub host_override: Option<Target>,
	pub path_override: Option<Strng>,
	pub path_prefix: Option<Strng>,
	/// Whether to tokenize on the request flow. This enables us to do more accurate rate limits,
	/// since we know (part of) the cost of the request upfront.
	/// This comes with the cost of an expensive operation.
	#[serde(default)]
	pub tokenize: bool,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendTrafficPolicy>,
}

#[apply(schema!)]
pub enum AIProvider {
	OpenAI(openai::Provider),
	Gemini(gemini::Provider),
	Vertex(vertex::Provider),
	Anthropic(anthropic::Provider),
	Bedrock(BedrockProvider),
	Azure(AzureProvider),
	Copilot(copilot::Provider),
	Custom(custom::Provider),
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockProvider {
	#[serde(flatten)]
	pub provider: bedrock::Provider,
	#[serde(skip)]
	pub source_credentials_cache: crate::http::auth::aws::AwsCredentialsCache,
	#[serde(skip)]
	pub assume_role_cache: crate::http::auth::aws::AwsAssumeRoleCache,
}

impl BedrockProvider {
	pub fn new(provider: bedrock::Provider) -> Self {
		Self {
			provider,
			source_credentials_cache: Default::default(),
			assume_role_cache: Default::default(),
		}
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for BedrockProvider {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		std::borrow::Cow::Borrowed("BedrockProvider")
	}

	fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
		<bedrock::Provider as schemars::JsonSchema>::json_schema(generator)
	}
}

impl std::ops::Deref for BedrockProvider {
	type Target = bedrock::Provider;

	fn deref(&self) -> &Self::Target {
		&self.provider
	}
}

impl std::ops::DerefMut for BedrockProvider {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.provider
	}
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureProvider {
	#[serde(flatten)]
	pub provider: azure::Provider,
	#[serde(skip)]
	pub cached_cred: crate::http::auth::azure::AzureCredentialCache,
}

impl AzureProvider {
	pub fn new(provider: azure::Provider) -> Self {
		Self {
			provider,
			cached_cred: Default::default(),
		}
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for AzureProvider {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		std::borrow::Cow::Borrowed("AzureProvider")
	}

	fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
		<azure::Provider as schemars::JsonSchema>::json_schema(generator)
	}
}

impl std::ops::Deref for AzureProvider {
	type Target = azure::Provider;

	fn deref(&self) -> &Self::Target {
		&self.provider
	}
}

impl std::ops::DerefMut for AzureProvider {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.provider
	}
}

impl AIProvider {
	pub fn bedrock(provider: bedrock::Provider) -> Self {
		Self::Bedrock(BedrockProvider::new(provider))
	}

	pub fn azure(provider: azure::Provider) -> Self {
		Self::Azure(AzureProvider::new(provider))
	}
}

/// Classify how the upstream reports cached tokens, from the source wire format the
/// gateway is about to parse — not the provider name, which can carry another
/// provider's native semantics (e.g. Vertex serving Anthropic models).
fn cache_convention_for(
	provider: &AIProvider,
	provider_format: Option<custom::ProviderFormat>,
	request_model: &str,
) -> CacheTokenConvention {
	use CacheTokenConvention::*;
	use custom::ProviderFormat::{AnthropicTokenCount, Messages};
	match provider {
		AIProvider::Anthropic(_) | AIProvider::Bedrock(_) => InputExcludesCache,
		AIProvider::Copilot(_) if copilot::Provider::is_anthropic_model(Some(request_model)) => {
			InputExcludesCache
		},
		AIProvider::Vertex(p) if p.is_anthropic_model(Some(request_model)) => InputExcludesCache,
		AIProvider::Custom(_) => match provider_format {
			Some(Messages | AnthropicTokenCount) => InputExcludesCache,
			_ => InputIncludesCache,
		},
		_ => InputIncludesCache, // openai, azure, gemini, copilot/vertex non-anthropic
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatErrorFormat {
	OpenAI,
	Google,
	Anthropic,
	Bedrock,
}

struct ChatTranslation {
	/// Client-facing chat request/response shape.
	input: InputFormat,
	/// Upstream chat wire format used for request, response, stream, and error translation.
	output: ChatFormat,
}

/// Result of translating a chat request before it is sent upstream.
///
/// `body` is the upstream request payload. `provider_state` carries any
/// conversion state needed to translate the later response/stream back to the
/// original `InputFormat`; most providers do not need it.
struct RenderedChatRequest {
	body: Vec<u8>,
	provider_state: Option<ProviderState>,
}

// Context provider to each request translation
struct ChatRequestContext<'a> {
	provider: &'a AIProvider,
	headers: &'a HeaderMap,
	prompt_caching: Option<&'a policy::PromptCachingConfig>,
}

// Context provider to each response translation
struct ChatResponseContext<'a> {
	model: &'a str,
	tool_name_map: Option<&'a conversion::bedrock::BedrockToolNameMap>,
}

// Context provider to each response translation (streaming)
struct ChatStreamContext {
	buffer_limit: usize,
	logger: agent_llm::StreamingUsageGuard,
	model: String,
	log_content: LogContentFields,
	tool_name_map: Option<conversion::bedrock::BedrockToolNameMap>,
}

/// Ordered chat conversion table.
///
/// For a client `InputFormat`, pick the first entry whose `ChatFormat` is
/// supported by the selected provider/model. Put cheaper or more native
/// translations before broader fallbacks.
const CHAT_TRANSLATIONS: &[ChatTranslation] = {
	const fn chat(input: InputFormat, output: ChatFormat) -> ChatTranslation {
		ChatTranslation { input, output }
	}
	&[
		// Direct passthrough
		chat(InputFormat::Responses, ChatFormat::OpenAIResponses),
		// Quirk: normally we prefer direct passthrough. However, for Gemini, we can do a better job of
		// the conversion than Google's OpenAI compatible endpoint, so we put this first. This will
		// only actually be used for Vertex + Gemini models.
		chat(InputFormat::Completions, ChatFormat::VertexGemini),
		chat(InputFormat::Completions, ChatFormat::OpenAICompletions),
		chat(InputFormat::Messages, ChatFormat::AnthropicMessages),
		// Missing: Bedrock --> Bedrock
		//
		// Completions
		chat(InputFormat::Completions, ChatFormat::AnthropicMessages),
		chat(InputFormat::Completions, ChatFormat::BedrockConverse),
		// Messages
		chat(InputFormat::Messages, ChatFormat::OpenAICompletions),
		chat(InputFormat::Messages, ChatFormat::BedrockConverse),
		// Missing: Messages --> Responses
		//
		// Responses
		chat(InputFormat::Responses, ChatFormat::OpenAICompletions),
		chat(InputFormat::Responses, ChatFormat::BedrockConverse),
		// Missing: Responses -> Messages
	]
};

fn render_openai_completions(req: types::ChatRequest<'_>) -> Result<Vec<u8>, AIError> {
	match req {
		types::ChatRequest::Completions(req) => {
			serde_json::to_vec(req).map_err(AIError::RequestMarshal)
		},
		types::ChatRequest::Messages(req) => conversion::completions::from_messages::translate(req),
		types::ChatRequest::Responses(req) => conversion::openai_compat::from_responses::translate(req),
	}
}

fn render_openai_responses(req: types::ChatRequest<'_>) -> Result<Vec<u8>, AIError> {
	match req {
		types::ChatRequest::Responses(req) => serde_json::to_vec(req).map_err(AIError::RequestMarshal),
		_ => Err(AIError::UnsupportedConversion(strng::literal!(
			"expected responses request"
		))),
	}
}

fn render_anthropic_messages(req: types::ChatRequest<'_>) -> Result<Vec<u8>, AIError> {
	match req {
		types::ChatRequest::Completions(req) => conversion::messages::from_completions::translate(req),
		types::ChatRequest::Messages(req) => serde_json::to_vec(req).map_err(AIError::RequestMarshal),
		types::ChatRequest::Responses(_) => Err(AIError::UnsupportedConversion(strng::literal!(
			"responses to messages"
		))),
	}
}

fn render_vertex_gemini(
	req: types::ChatRequest<'_>,
	ctx: &ChatRequestContext<'_>,
) -> Result<Vec<u8>, AIError> {
	let AIProvider::Vertex(provider) = ctx.provider else {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"expected vertex provider"
		)));
	};
	match req {
		types::ChatRequest::Completions(req) => {
			conversion::vertex_gemini::from_completions::translate(req, provider.model.as_deref())
		},
		_ => Err(AIError::UnsupportedConversion(strng::literal!(
			"vertex gemini only supports completions input"
		))),
	}
}

fn render_bedrock_converse(
	req: types::ChatRequest<'_>,
	ctx: &ChatRequestContext<'_>,
) -> Result<RenderedChatRequest, AIError> {
	let AIProvider::Bedrock(provider) = ctx.provider else {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"expected bedrock provider"
		)));
	};
	let bedrock = match req {
		types::ChatRequest::Completions(req) => conversion::bedrock::from_completions::translate(
			req,
			provider,
			Some(ctx.headers),
			ctx.prompt_caching,
		),
		types::ChatRequest::Messages(req) => {
			conversion::bedrock::from_messages::translate(req, provider, Some(ctx.headers))
		},
		types::ChatRequest::Responses(req) => conversion::bedrock::from_responses::translate(
			req,
			provider,
			Some(ctx.headers),
			ctx.prompt_caching,
		),
	}?;
	let provider_state = if bedrock.tool_name_map.is_empty() {
		None
	} else {
		Some(ProviderState::Bedrock {
			tool_names: Arc::new(bedrock.tool_name_map),
		})
	};
	Ok(RenderedChatRequest {
		body: bedrock.body,
		provider_state,
	})
}

impl ChatTranslation {
	fn provider_format(&self) -> custom::ProviderFormat {
		match self.output {
			ChatFormat::OpenAICompletions => custom::ProviderFormat::Completions,
			ChatFormat::OpenAIResponses => custom::ProviderFormat::Responses,
			ChatFormat::AnthropicMessages => custom::ProviderFormat::Messages,
			ChatFormat::BedrockConverse => match self.input {
				// Bedrock chat always renders to Converse. This format is only used for
				// shared bookkeeping (route type, cache convention, custom-style labels);
				// Bedrock path setup ignores these chat distinctions for Converse.
				InputFormat::Completions => custom::ProviderFormat::Completions,
				InputFormat::Messages => custom::ProviderFormat::Messages,
				InputFormat::Responses => custom::ProviderFormat::Responses,
				_ => unreachable!("chat translation selected for non-chat input"),
			},
			ChatFormat::VertexGemini => custom::ProviderFormat::Completions,
		}
	}

	fn render_request(
		&self,
		req: types::ChatRequest<'_>,
		ctx: &ChatRequestContext<'_>,
	) -> Result<RenderedChatRequest, AIError> {
		let body = match self.output {
			ChatFormat::OpenAICompletions => render_openai_completions(req),
			ChatFormat::OpenAIResponses => render_openai_responses(req),
			ChatFormat::AnthropicMessages if matches!(ctx.provider, AIProvider::Vertex(_)) => {
				vertex::prepare_anthropic_message_body(render_anthropic_messages(req)?)
			},
			ChatFormat::AnthropicMessages => render_anthropic_messages(req),
			ChatFormat::BedrockConverse => return render_bedrock_converse(req, ctx),
			ChatFormat::VertexGemini => {
				return Ok(RenderedChatRequest {
					body: render_vertex_gemini(req, ctx)?,
					provider_state: Some(ProviderState::VertexGemini),
				});
			},
		}?;
		Ok(RenderedChatRequest {
			body,
			provider_state: None,
		})
	}

	fn render_response(
		&self,
		bytes: &Bytes,
		ctx: &ChatResponseContext<'_>,
	) -> Result<Box<dyn ResponseType>, AIError> {
		match self.output {
			ChatFormat::OpenAICompletions => match self.input {
				InputFormat::Completions => {
					AIProvider::parse_response::<types::completions::Response>(bytes)
				},
				InputFormat::Messages => conversion::completions::from_messages::translate_response(bytes),
				InputFormat::Responses => {
					conversion::openai_compat::to_responses::translate_response(bytes, ctx.model)
				},
				_ => Err(AIError::UnsupportedConversion(strng::format!(
					"from {:?} to {:?}",
					self.output,
					self.input
				))),
			},
			ChatFormat::OpenAIResponses => match self.input {
				InputFormat::Responses => AIProvider::parse_response::<types::responses::Response>(bytes),
				_ => Err(AIError::UnsupportedConversion(strng::format!(
					"from {:?} to {:?}",
					self.output,
					self.input
				))),
			},
			ChatFormat::AnthropicMessages => match self.input {
				InputFormat::Messages => AIProvider::parse_response::<types::messages::Response>(bytes),
				InputFormat::Completions => {
					conversion::messages::from_completions::translate_response(bytes)
				},
				_ => Err(AIError::UnsupportedConversion(strng::format!(
					"from {:?} to {:?}",
					self.output,
					self.input
				))),
			},
			ChatFormat::BedrockConverse => match self.input {
				InputFormat::Completions => conversion::bedrock::from_completions::translate_response(
					bytes,
					ctx.model,
					ctx.tool_name_map,
				),
				InputFormat::Messages => conversion::bedrock::from_messages::translate_response(
					bytes,
					ctx.model,
					ctx.tool_name_map,
				),
				InputFormat::Responses => conversion::bedrock::from_responses::translate_response(
					bytes,
					ctx.model,
					ctx.tool_name_map,
				),
				_ => Err(AIError::UnsupportedConversion(strng::format!(
					"from {:?} to {:?}",
					self.output,
					self.input
				))),
			},
			ChatFormat::VertexGemini => match self.input {
				InputFormat::Completions => {
					conversion::vertex_gemini::to_completions::translate_response(bytes)
				},
				_ => Err(AIError::UnsupportedConversion(strng::format!(
					"from {:?} to {:?}",
					self.output,
					self.input
				))),
			},
		}
	}

	fn stream(&self, resp: Response, ctx: ChatStreamContext) -> Response {
		match self.output {
			ChatFormat::OpenAICompletions => match self.input {
				InputFormat::Completions => {
					conversion::completions::passthrough_stream(ctx.logger, ctx.log_content, resp)
				},
				InputFormat::Messages => resp.map(|b| {
					conversion::completions::from_messages::translate_stream(
						b,
						ctx.buffer_limit,
						ctx.logger,
						ctx.log_content,
					)
				}),
				InputFormat::Responses => resp.map(|b| {
					conversion::openai_compat::to_responses::translate_stream(b, ctx.buffer_limit, ctx.logger)
				}),
				_ => resp,
			},

			ChatFormat::OpenAIResponses => match self.input {
				InputFormat::Responses => resp.map(|b| {
					conversion::responses::passthrough_stream(
						b,
						ctx.buffer_limit,
						ctx.logger,
						ctx.log_content,
					)
				}),
				_ => resp,
			},

			ChatFormat::AnthropicMessages => match self.input {
				InputFormat::Messages => resp.map(|b| {
					conversion::messages::passthrough_stream(b, ctx.buffer_limit, ctx.logger, ctx.log_content)
				}),
				InputFormat::Completions => resp.map(|b| {
					conversion::messages::from_completions::translate_stream(b, ctx.buffer_limit, ctx.logger)
				}),
				_ => resp,
			},

			ChatFormat::BedrockConverse => match self.input {
				InputFormat::Completions => {
					let msg = conversion::bedrock::message_id(&resp);
					let tool_name_map = ctx.tool_name_map.clone();
					resp.map(move |b| {
						conversion::bedrock::from_completions::translate_stream(
							b,
							ctx.buffer_limit,
							ctx.logger,
							&ctx.model,
							&msg,
							tool_name_map,
						)
					})
				},
				InputFormat::Messages => {
					let msg = conversion::bedrock::message_id(&resp);
					let tool_name_map = ctx.tool_name_map.clone();
					resp.map(move |b| {
						conversion::bedrock::from_messages::translate_stream(
							b,
							ctx.buffer_limit,
							ctx.logger,
							&ctx.model,
							&msg,
							ctx.log_content,
							tool_name_map,
						)
					})
				},
				InputFormat::Responses => {
					let msg = conversion::bedrock::message_id(&resp);
					let tool_name_map = ctx.tool_name_map.clone();
					resp.map(move |b| {
						conversion::bedrock::from_responses::translate_stream(
							b,
							ctx.buffer_limit,
							ctx.logger,
							&ctx.model,
							&msg,
							tool_name_map,
						)
					})
				},
				_ => resp,
			},

			ChatFormat::VertexGemini => match self.input {
				InputFormat::Completions => resp.map(|b| {
					conversion::vertex_gemini::to_completions::translate_stream(
						b,
						ctx.buffer_limit,
						strng::new(&ctx.model),
						ctx.logger,
					)
				}),
				_ => resp,
			},
		}
	}

	fn error(
		&self,
		bytes: &Bytes,
		status: ::http::StatusCode,
		format: ChatErrorFormat,
	) -> Result<Bytes, AIError> {
		let unsupported = || {
			Err(AIError::UnsupportedConversion(strng::format!(
				"from {:?} error to {:?}",
				format,
				self.input
			)))
		};
		match self.output {
			ChatFormat::OpenAICompletions => match format {
				ChatErrorFormat::OpenAI => match self.input {
					InputFormat::Completions => Ok(bytes.clone()),
					InputFormat::Messages => {
						conversion::completions::from_messages::translate_error(bytes, status)
					},
					InputFormat::Responses => Ok(bytes.clone()),
					_ => unsupported(),
				},
				ChatErrorFormat::Google => match self.input {
					InputFormat::Messages => conversion::messages::translate_google_error(bytes),
					InputFormat::Responses => conversion::gemini::from_responses::translate_error(bytes),
					_ => conversion::completions::translate_google_error(bytes),
				},
				ChatErrorFormat::Anthropic => match self.input {
					InputFormat::Messages => {
						conversion::completions::from_messages::translate_error(bytes, status)
					},
					_ => unsupported(),
				},
				ChatErrorFormat::Bedrock => unsupported(),
			},

			ChatFormat::OpenAIResponses => match format {
				ChatErrorFormat::OpenAI => match self.input {
					InputFormat::Responses => Ok(bytes.clone()),
					_ => unsupported(),
				},
				_ => unsupported(),
			},

			ChatFormat::AnthropicMessages => match format {
				ChatErrorFormat::Anthropic => match self.input {
					InputFormat::Messages => conversion::messages::translate_anthropic_error(bytes, status),
					InputFormat::Completions => {
						conversion::messages::from_completions::translate_error(bytes)
					},
					_ => unsupported(),
				},
				ChatErrorFormat::OpenAI => match self.input {
					InputFormat::Messages => Ok(bytes.clone()),
					_ => unsupported(),
				},
				_ => unsupported(),
			},

			ChatFormat::BedrockConverse => match format {
				ChatErrorFormat::Bedrock => match self.input {
					InputFormat::Completions => conversion::bedrock::from_completions::translate_error(bytes),
					InputFormat::Messages => conversion::bedrock::from_messages::translate_error(bytes),
					InputFormat::Responses => conversion::bedrock::from_responses::translate_error(bytes),
					_ => unsupported(),
				},
				ChatErrorFormat::OpenAI => match self.input {
					InputFormat::Messages => Ok(bytes.clone()),
					_ => unsupported(),
				},
				_ => unsupported(),
			},

			ChatFormat::VertexGemini => match format {
				ChatErrorFormat::Google => conversion::completions::translate_google_error(bytes),
				_ => unsupported(),
			},
		}
	}
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum RequestResult {
	Success {
		request: Request,
		llm_request: LLMRequest,
		upstream_route_type: RouteType,
	},
	Rejected(Response),
	GuardrailRejected {
		response: Response,
		guardrail: &'static str,
	},
}

enum PreparedRequest {
	Ready(LLMRequest),
	GuardrailRejected {
		response: Response,
		guardrail: &'static str,
	},
}

struct BufferedResponse {
	parts: ::http::response::Parts,
	bytes: Bytes,
	encoding: Option<&'static str>,
}

impl AIProvider {
	pub fn provider(&self) -> Strng {
		match self {
			AIProvider::OpenAI(_p) => openai::Provider::NAME,
			AIProvider::Anthropic(_p) => anthropic::Provider::NAME,
			AIProvider::Gemini(_p) => gemini::Provider::NAME,
			AIProvider::Vertex(_p) => vertex::Provider::NAME,
			AIProvider::Bedrock(_p) => bedrock::Provider::NAME,
			AIProvider::Azure(_p) => azure::Provider::NAME,
			AIProvider::Copilot(_p) => copilot::Provider::NAME,
			AIProvider::Custom(p) => p
				.provider_override
				.clone()
				.unwrap_or(custom::Provider::NAME),
		}
	}
	fn default_base_path(&self) -> Option<&'static str> {
		match self {
			AIProvider::OpenAI(_) | AIProvider::Copilot(_) => Some(openai::DEFAULT_BASE_PATH),
			AIProvider::Anthropic(_) => Some(anthropic::DEFAULT_BASE_PATH),
			_ => None,
		}
	}

	pub fn override_model(&self) -> Option<Strng> {
		match self {
			AIProvider::OpenAI(p) => p.model.clone(),
			AIProvider::Anthropic(p) => p.model.clone(),
			AIProvider::Gemini(p) => p.model.clone(),
			AIProvider::Vertex(p) => p.model.clone(),
			AIProvider::Bedrock(p) => p.model.clone(),
			AIProvider::Azure(p) => p.model.clone(),
			AIProvider::Copilot(p) => p.model.clone(),
			AIProvider::Custom(p) => p.model.clone(),
		}
	}

	pub fn supported_formats(&self, request_model: Option<&str>) -> Vec<custom::ProviderFormat> {
		use custom::ProviderFormat::*;
		match self {
			AIProvider::OpenAI(_) => vec![Completions, Responses, Embeddings, Realtime, Rerank],
			AIProvider::Copilot(_) => {
				if copilot::Provider::is_anthropic_model(request_model) {
					vec![Messages]
				} else {
					vec![Completions, Responses, Rerank, Embeddings]
				}
			},
			AIProvider::Azure(p) => {
				let mut formats = vec![Completions, Responses, Embeddings, Rerank];
				if matches!(p.resource_type, azure::AzureResourceType::Foundry)
					&& p.is_anthropic_model(request_model)
				{
					formats.extend([Messages, AnthropicTokenCount]);
				}
				formats
			},
			AIProvider::Gemini(_) => vec![Completions, Embeddings],
			AIProvider::Anthropic(_) => vec![Messages, AnthropicTokenCount],
			AIProvider::Bedrock(p) => {
				let mut formats = vec![Completions, Messages, Responses, Embeddings, Rerank];
				if p.is_anthropic_model(request_model) {
					formats.push(AnthropicTokenCount);
				}
				formats
			},
			AIProvider::Vertex(p) => {
				let mut formats = if p.is_anthropic_model(request_model) {
					vec![Messages, AnthropicTokenCount]
				} else {
					vec![Completions]
				};
				formats.extend([Embeddings, Rerank]);
				formats
			},
			AIProvider::Custom(p) => p.formats.iter().map(|f| f.format).collect(),
		}
	}

	fn supported_chat_formats(&self, request_model: Option<&str>) -> Vec<ChatFormat> {
		match self {
			AIProvider::OpenAI(_) => {
				vec![ChatFormat::OpenAIResponses, ChatFormat::OpenAICompletions]
			},

			AIProvider::Copilot(_) => copilot::Provider::supported_formats_for_model(request_model),

			AIProvider::Azure(p)
				if matches!(p.resource_type, azure::AzureResourceType::Foundry)
					&& p.is_anthropic_model(request_model) =>
			{
				// Foundry Claude models require the Anthropic-native endpoint; the
				// OpenAI-compatible chat/responses endpoints return api_not_supported.
				vec![ChatFormat::AnthropicMessages]
			},
			AIProvider::Azure(_) => vec![ChatFormat::OpenAIResponses, ChatFormat::OpenAICompletions],

			AIProvider::Gemini(_) => vec![ChatFormat::OpenAICompletions],
			AIProvider::Anthropic(_) => vec![ChatFormat::AnthropicMessages],
			AIProvider::Bedrock(_) => vec![ChatFormat::BedrockConverse],

			AIProvider::Vertex(p) if p.is_anthropic_model(request_model) => {
				vec![ChatFormat::AnthropicMessages]
			},
			AIProvider::Vertex(p) if p.is_gemini_model(request_model) => {
				vec![ChatFormat::VertexGemini, ChatFormat::OpenAICompletions]
			},
			AIProvider::Vertex(_) => vec![ChatFormat::OpenAICompletions],

			AIProvider::Custom(p) => p
				.formats
				.iter()
				.filter_map(|f| match f.format {
					custom::ProviderFormat::Completions => Some(ChatFormat::OpenAICompletions),
					custom::ProviderFormat::Messages => Some(ChatFormat::AnthropicMessages),
					custom::ProviderFormat::Responses => Some(ChatFormat::OpenAIResponses),
					_ => None,
				})
				.collect(),
		}
	}

	fn chat_error_format(
		&self,
		translation: &ChatTranslation,
		request_model: Option<&str>,
	) -> ChatErrorFormat {
		match (self, translation.output) {
			(AIProvider::Gemini(_), ChatFormat::OpenAICompletions) => ChatErrorFormat::Google,
			(AIProvider::Vertex(p), ChatFormat::OpenAICompletions)
				if !p.is_anthropic_model(request_model) =>
			{
				ChatErrorFormat::Google
			},
			(_, ChatFormat::BedrockConverse) => ChatErrorFormat::Bedrock,
			(_, ChatFormat::AnthropicMessages) => ChatErrorFormat::Anthropic,
			(_, ChatFormat::OpenAICompletions | ChatFormat::OpenAIResponses) => ChatErrorFormat::OpenAI,
			(_, ChatFormat::VertexGemini) => ChatErrorFormat::Google,
		}
	}

	fn chat_translation(
		&self,
		input_format: InputFormat,
		request_model: Option<&str>,
	) -> Result<&'static ChatTranslation, AIError> {
		let supported = self.supported_chat_formats(request_model);
		CHAT_TRANSLATIONS
			.iter()
			.find(|translation| {
				translation.input == input_format && supported.contains(&translation.output)
			})
			.ok_or_else(|| {
				AIError::UnsupportedConversion(strng::format!(
					"from {input_format:?} to provider {} (supported: {supported:?})",
					self.provider()
				))
			})
	}

	pub fn supports_format(
		&self,
		format: custom::ProviderFormat,
		request_model: Option<&str>,
	) -> bool {
		self.supported_formats(request_model).contains(&format)
	}

	fn non_chat_provider_format_for(
		&self,
		input_format: InputFormat,
		request_model: Option<&str>,
	) -> Option<custom::ProviderFormat> {
		use custom::ProviderFormat::*;
		let format = match input_format {
			InputFormat::Embeddings => Embeddings,
			InputFormat::Realtime => Realtime,
			InputFormat::CountTokens => AnthropicTokenCount,
			InputFormat::Rerank => Rerank,
			InputFormat::Detect
			| InputFormat::Completions
			| InputFormat::Messages
			| InputFormat::Responses => return None,
		};
		self
			.supports_format(format, request_model)
			.then_some(format)
	}

	/// Default backend policies (TLS + auth) for connecting to the provider. Split from
	/// [`Self::default_connector_target`] so callers can compute effective policies, resolve the LLM
	/// route from them, and only then pick the connection target. Returns `None` for custom providers,
	/// which require an explicit host override or provider backend.
	pub fn default_connector_policies(&self) -> Option<BackendPolicies> {
		let btls = BackendPolicies {
			backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
			// We will use original request for now
			..Default::default()
		};
		Some(match self {
			AIProvider::OpenAI(_) | AIProvider::Gemini(_) | AIProvider::Anthropic(_) => btls,
			AIProvider::Copilot(_) => BackendPolicies {
				backend_auth: Some(BackendAuth::new(BackendAuthKind::Copilot)),
				..btls
			},
			AIProvider::Vertex(_) => BackendPolicies {
				backend_auth: Some(BackendAuth::new(BackendAuthKind::Gcp(GcpAuth::default()))),
				..btls
			},
			AIProvider::Bedrock(p) => BackendPolicies {
				backend_auth: Some(BackendAuth::new(BackendAuthKind::Aws(AwsAuth::Implicit {
					service_name: None,
					region: None,
					assume_role: None,
					source_credentials_cache: p.source_credentials_cache.clone(),
					assume_role_cache: p.assume_role_cache.clone(),
				}))),
				..btls
			},
			AIProvider::Azure(p) => BackendPolicies {
				backend_auth: Some(BackendAuth::new(BackendAuthKind::Azure(
					AzureAuth::Implicit {
						cached_cred: p.cached_cred.clone(),
					},
				))),
				..btls
			},
			AIProvider::Custom(_) => return None,
		})
	}

	/// Default connection target for the provider, for the given LLM route. Route-aware because some
	/// providers serve routes from different hosts (Bedrock rerank uses `bedrock-agent-runtime` and
	/// Vertex rerank uses `discoveryengine`, distinct from the chat/embeddings host). Returns `None`
	/// for custom providers, which require an explicit host override or provider backend.
	pub fn default_connector_target(&self, route_type: RouteType) -> Option<Target> {
		Some(match self {
			AIProvider::OpenAI(_) => Target::Hostname(openai::DEFAULT_HOST, 443),
			AIProvider::Copilot(_) => Target::Hostname(copilot::DEFAULT_HOST, 443),
			AIProvider::Gemini(_) => Target::Hostname(gemini::DEFAULT_HOST, 443),
			AIProvider::Anthropic(_) => Target::Hostname(anthropic::DEFAULT_HOST, 443),
			AIProvider::Vertex(p) => Target::Hostname(p.get_host(route_type), 443),
			AIProvider::Bedrock(p) => Target::Hostname(p.get_host(route_type), 443),
			AIProvider::Azure(p) => Target::Hostname(p.get_host(), 443),
			AIProvider::Custom(_) => return None,
		})
	}

	pub fn setup_request(
		&self,
		req: &mut Request,
		route_type: RouteType,
		llm_request: Option<&LLMRequest>,
		path_override: Option<&str>,
		path_prefix: Option<&str>,
		has_host_override: bool,
	) -> anyhow::Result<()> {
		if let Some(path_override) = path_override {
			http::modify_req_uri(req, |uri| {
				uri.path_and_query = Some(PathAndQuery::from_str(path_override)?);
				Ok(())
			})?;
		} else {
			self.set_default_path(req, route_type, llm_request, path_prefix, has_host_override)?;
		}
		if !has_host_override {
			self.set_default_authority(req, route_type)?;
		}
		self.set_required_fields(req, route_type, llm_request)?;
		Ok(())
	}

	fn set_path_and_query(uri: &mut http::uri::Parts, path: &str) -> anyhow::Result<()> {
		let query = uri.path_and_query.as_ref().and_then(|p| p.query());
		if let Some(query) = query {
			let separator = if path.contains('?') { "&" } else { "?" };
			uri.path_and_query = Some(PathAndQuery::from_maybe_shared(format!(
				"{}{}{}",
				path, separator, query
			))?);
		} else {
			uri.path_and_query = Some(PathAndQuery::try_from(path)?);
		};
		Ok(())
	}

	fn with_path_prefix(path: &str, path_prefix: Option<&str>) -> String {
		match path_prefix {
			Some(prefix) => format!("{}{}", prefix.trim_end_matches('/'), path),
			None => path.to_string(),
		}
	}

	pub fn set_default_path(
		&self,
		req: &mut Request,
		route_type: RouteType,
		llm_request: Option<&LLMRequest>,
		path_prefix: Option<&str>,
		has_host_override: bool,
	) -> anyhow::Result<()> {
		if matches!(route_type, RouteType::Passthrough | RouteType::Detect) {
			if let Some(prefix) = path_prefix {
				http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						let current = uri
							.path_and_query
							.as_ref()
							.map(|pq| pq.path())
							.unwrap_or("/");
						let new_path = match self.default_base_path() {
							// For providers with a default base path (e.g. /v1), strip it so
							// that pathPrefix replaces it — consistent with non-passthrough routes.
							// If the path doesn't start with the default base, still apply
							// pathPrefix so it is never silently dropped.
							Some(base) => {
								let rest = current
									.strip_prefix(base)
									.filter(|rest| rest.is_empty() || rest.starts_with('/'))
									.unwrap_or(current);
								format!("{}{}", prefix.trim_end_matches('/'), rest)
							},
							// For other providers, pathPrefix is prepended to the full path,
							// consistent with with_path_prefix used in their non-passthrough code.
							None => format!("{}{}", prefix.trim_end_matches('/'), current),
						};
						Self::set_path_and_query(uri, &new_path)?;
						Ok(())
					})
				})?;
			}
			return Ok(());
		}

		if has_host_override && path_prefix.is_none() && !matches!(self, AIProvider::Custom(_)) {
			return Ok(());
		}

		match self {
			AIProvider::OpenAI(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					let path = format!(
						"{}{}",
						path_prefix.map_or(openai::DEFAULT_BASE_PATH, |prefix| {
							prefix.trim_end_matches('/')
						}),
						openai::path_suffix(route_type)
					);
					Self::set_path_and_query(uri, &path)?;
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Copilot(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					let path = format!(
						"{}{}",
						path_prefix.map_or("", |prefix| prefix.trim_end_matches('/')),
						copilot::path_suffix(route_type)
					);
					Self::set_path_and_query(uri, &path)?;
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Anthropic(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					let path = format!(
						"{}{}",
						path_prefix.map_or(anthropic::DEFAULT_BASE_PATH, |prefix| {
							prefix.trim_end_matches('/')
						}),
						anthropic::path_suffix(route_type),
					);
					Self::set_path_and_query(uri, &path)?;
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Gemini(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					let path = Self::with_path_prefix(gemini::path(route_type), path_prefix);
					Self::set_path_and_query(uri, &path)?;
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Vertex(provider) => {
				let request_model = llm_request.map(|l| l.request_model.as_str());
				let streaming = llm_request.map(|l| l.streaming).unwrap_or(false);
				let native_gemini = llm_request
					.and_then(|l| l.provider_state.as_ref())
					.is_some_and(|s| matches!(s, ProviderState::VertexGemini));
				http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						let path =
							provider.get_path_for_model(route_type, request_model, streaming, native_gemini);
						let path = Self::with_path_prefix(&path, path_prefix);
						Self::set_path_and_query(uri, &path)?;
						Ok(())
					})?;
					Ok(())
				})
			},
			AIProvider::Bedrock(provider) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if let Some(l) = llm_request {
						let path =
							provider.get_path_for_route(route_type, l.streaming, l.request_model.as_str());
						let path = Self::with_path_prefix(&path, path_prefix);
						Self::set_path_and_query(uri, &path)?;
					}
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Azure(provider) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if let Some(l) = llm_request {
						let path = provider.get_path_for_model(route_type, l.request_model.as_str());
						let path = Self::with_path_prefix(&path, path_prefix);
						Self::set_path_and_query(uri, &path)?;
					}
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Custom(provider) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if let Some(path) = provider.path_for_route(route_type) {
						Self::set_path_and_query(uri, path)?;
						return Ok(());
					}
					let path = match route_type {
						RouteType::Messages | RouteType::AnthropicTokenCount => format!(
							"{}{}",
							path_prefix.map_or(anthropic::DEFAULT_BASE_PATH, |prefix| {
								prefix.trim_end_matches('/')
							}),
							anthropic::path_suffix(route_type)
						),
						_ => format!(
							"{}{}",
							path_prefix.map_or(openai::DEFAULT_BASE_PATH, |prefix| {
								prefix.trim_end_matches('/')
							}),
							openai::path_suffix(route_type)
						),
					};
					Self::set_path_and_query(uri, &path)?;
					Ok(())
				})?;
				Ok(())
			}),
		}
	}

	pub fn set_default_authority(
		&self,
		req: &mut Request,
		route_type: RouteType,
	) -> anyhow::Result<()> {
		let authority = match self {
			AIProvider::OpenAI(_) => Authority::from_static(openai::DEFAULT_HOST_STR),
			AIProvider::Copilot(_) => Authority::from_static(copilot::DEFAULT_HOST_STR),
			AIProvider::Anthropic(_) => Authority::from_static(anthropic::DEFAULT_HOST_STR),
			AIProvider::Gemini(_) => Authority::from_static(gemini::DEFAULT_HOST_STR),
			AIProvider::Vertex(provider) => Authority::from_str(&provider.get_host(route_type))?,
			AIProvider::Azure(provider) => Authority::from_str(&provider.get_host())?,
			AIProvider::Custom(_) => return Ok(()),
			AIProvider::Bedrock(provider) => {
				// Store the region in request extensions so AWS signing can use it.
				return http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						uri.authority = Some(Authority::from_str(&provider.get_host(route_type))?);
						Ok(())
					})?;
					req.extensions.insert(bedrock::AwsRegion {
						region: provider.region.as_str().to_string(),
					});
					Ok(())
				});
			},
		};
		http::modify_req(req, |req| {
			http::modify_uri(req, |uri| {
				uri.authority = Some(authority);
				Ok(())
			})?;
			Ok(())
		})
	}

	pub fn set_required_fields(
		&self,
		req: &mut Request,
		route_type: RouteType,
		llm_request: Option<&LLMRequest>,
	) -> anyhow::Result<()> {
		match self {
			AIProvider::Anthropic(_) => {
				http::modify_req(req, |req| {
					if let Some(authz) = req.headers.typed_get::<headers::Authorization<Bearer>>() {
						// Check whether the backend auth location was explicitly configured by the user.
						// When explicit, we must not rewrite it
						// (e.g. Databricks Anthropic Messages API requires Authorization: Bearer <jwt>).
						let explicit_authorization = req
							.extensions
							.get::<AppliedBackendAuthLocation>()
							.is_some_and(|auth| auth.explicit);

						if authz.token().starts_with(anthropic::OAUTH_TOKEN_PREFIX) || explicit_authorization {
							// OAuth tokens ("sk-ant-oat*") keep Authorization: Bearer; drop any x-api-key.
							// Explicitly configured Authorization auth also keeps the header as-is.
							req.headers.remove("x-api-key");
						} else {
							// All other tokens are moved to x-api-key (standard API key auth).
							req.headers.remove(http::header::AUTHORIZATION);
							let mut api_key = HeaderValue::from_str(authz.token())?;
							api_key.set_sensitive(true);
							req.headers.insert("x-api-key", api_key);
						}
					}
					// https://docs.anthropic.com/en/api/versioning
					req
						.headers
						.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
					Ok(())
				})
			},
			AIProvider::Azure(p) => {
				// Foundry's Anthropic-native endpoint requires the anthropic-version header,
				// but only for Claude models — GPT models use the OpenAI-compatible path.
				let model = llm_request.map(|r| r.request_model.as_str()).unwrap_or("");
				if matches!(p.resource_type, azure::AzureResourceType::Foundry)
					&& p.is_anthropic_model(Some(model))
					&& matches!(
						route_type,
						RouteType::Messages | RouteType::AnthropicTokenCount
					) {
					http::modify_req(req, |req| {
						req
							.headers
							.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
						Ok(())
					})
				} else {
					Ok(())
				}
			},
			_ => Ok(()),
		}
	}

	// Anthropic does not like CORS requests, but we are not really directly CORS since we are proxying requests.
	// Securing the requests and management of CORS is handled by the proxy so we just directly send.
	pub fn strip_browser_cors_headers(&self, req: &mut Request) {
		if !matches!(self, AIProvider::Anthropic(_)) {
			return;
		}

		let headers = req.headers_mut();
		headers.remove("origin");
		headers.remove("access-control-request-method");
		headers.remove("access-control-request-headers");

		let sec_fetch_headers: Vec<HeaderName> = headers
			.keys()
			.filter(|name| name.as_str().starts_with("sec-fetch-"))
			.cloned()
			.collect();
		for name in sec_fetch_headers {
			headers.remove(name);
		}
	}

	pub async fn process_completions_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (parts, mut req) = self
			.read_body_and_default_model::<types::completions::Request>(policies, req, log)
			.await?;
		self.apply_model_alias(policies, &mut req);

		// If a user doesn't request usage, we will not get token information which we need
		// We always set it.
		// TODO?: this may impact the user, if they make assumptions about the stream NOT including usage.
		// Notably, this adds a final SSE event.
		// We could actually go remove that on the response, but it would mean we cannot do passthrough-parsing,
		// so unless we have a compelling use case for it, for now we keep it.
		if req.stream.unwrap_or_default() && req.stream_options.is_none() {
			req.stream_options = Some(types::completions::StreamOptions {
				include_usage: true,
				rest: Default::default(),
			});
		}
		if matches!(
			self,
			AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_)
		) {
			req.normalize_openai_token_limit();
		}
		self
			.process_chat_request(
				backend_info,
				policies,
				InputFormat::Completions,
				req,
				parts,
				tokenize,
				log,
				|req| types::ChatRequest::Completions(req),
			)
			.await
	}

	pub async fn process_messages_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (parts, mut req) = self
			.read_body_and_default_model::<types::messages::Request>(policies, req, log)
			.await?;
		self.apply_model_alias(policies, &mut req);

		self
			.process_chat_request(
				backend_info,
				policies,
				InputFormat::Messages,
				req,
				parts,
				tokenize,
				log,
				|req| types::ChatRequest::Messages(req),
			)
			.await
	}

	pub async fn process_embeddings_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (parts, mut req) = self
			.read_body_and_default_model::<types::embeddings::Request>(policies, req, log)
			.await?;
		self.apply_model_alias(policies, &mut req);

		self
			.process_non_chat_request(
				backend_info,
				policies,
				InputFormat::Embeddings,
				req,
				parts,
				tokenize,
				log,
				|provider, req, _, _| provider.render_embeddings_request(req),
			)
			.await
	}

	pub async fn process_rerank_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (parts, mut req) = self
			.read_body_and_default_model::<types::rerank::Request>(policies, req, log)
			.await?;
		self.apply_model_alias(policies, &mut req);

		self
			.process_non_chat_request(
				backend_info,
				policies,
				InputFormat::Rerank,
				req,
				parts,
				tokenize,
				log,
				|provider, req, _, _| provider.render_rerank_request(req),
			)
			.await
	}

	pub async fn process_responses_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (mut parts, mut req) = self
			.read_body_and_default_model::<types::responses::Request>(policies, req, log)
			.await?;
		self.apply_model_alias(policies, &mut req);

		// Strip client-specific headers that cause AWS signature mismatches for Bedrock
		if matches!(self, AIProvider::Bedrock(_)) {
			parts.headers.remove("conversation_id");
			parts.headers.remove("session_id");
		}

		self
			.process_chat_request(
				backend_info,
				policies,
				InputFormat::Responses,
				req,
				parts,
				tokenize,
				log,
				|req| types::ChatRequest::Responses(req),
			)
			.await
	}

	pub async fn process_count_tokens_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		req: Request,
		policies: Option<&Policy>,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (parts, mut req) = self
			.read_body_and_default_model::<types::count_tokens::Request>(policies, req, log)
			.await?;
		self.apply_model_alias(policies, &mut req);

		// Some Anthropic-compatible clients (e.g. Claude Code) always call
		// `/v1/messages/count_tokens`. For providers/models without a native
		// count-tokens endpoint, we must still answer this route, so we fall
		// back to local token estimation using the normalized messages payload.
		let use_local = !self.supports_format(
			custom::ProviderFormat::AnthropicTokenCount,
			req.model.as_deref(),
		);
		if use_local {
			let messages = req.get_messages();
			let model = req.model.as_deref().unwrap_or_default();
			let count = num_tokens_from_messages(model, &messages)?;
			let body = serde_json::to_vec(&types::count_tokens::Response {
				input_tokens: count,
			})
			.map_err(AIError::ResponseMarshal)?;
			let resp = ::http::Response::builder()
				.status(::http::StatusCode::OK)
				.header(::http::header::CONTENT_TYPE, "application/json")
				.body(Body::from(body))
				.expect("failed to build count_tokens response");
			return Ok(RequestResult::Rejected(resp));
		}

		self
			.process_non_chat_request(
				backend_info,
				policies,
				InputFormat::CountTokens,
				req,
				parts,
				false,
				log,
				|provider, req, parts, request_model| {
					provider.render_count_tokens_request(req, &parts.headers, request_model)
				},
			)
			.await
	}

	pub async fn process_detect_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		hreq: Request,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		// We don't use read_body_and_default_model here because we need a lot of special logic
		// Unfortunately we buffer just due to how our interface works. Ideally we could not when
		// it is not even JSON
		let buffer = http::buffer_limit(&hreq);
		let is_json = hreq
			.headers()
			.typed_get::<headers::ContentType>()
			.map(|v| v == headers::ContentType::json())
			.unwrap_or_default();
		let (parts, body) = hreq.into_parts();
		let Ok(bytes) = http::read_body_with_limit(body, buffer).await else {
			return Err(AIError::RequestTooLarge);
		};

		let req = if is_json {
			if let Some(p) = policies
				&& p.has_request_body_mutations()
			{
				p.unmarshal_request(&bytes, log)
			} else {
				serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)
			}
			.unwrap_or_else(|_| types::detect::Request::new_raw(bytes))
		} else {
			types::detect::Request::new_raw(bytes)
		};

		self
			.process_non_chat_request(
				backend_info,
				policies,
				InputFormat::Detect,
				req,
				parts,
				false,
				log,
				|_, req, _, _| match req {
					types::detect::Request::Raw(bytes) => Ok(bytes.to_vec()),
					types::detect::Request::Json(value) => {
						serde_json::to_vec(value).map_err(AIError::RequestMarshal)
					},
				},
			)
			.await
	}

	fn render_count_tokens_request(
		&self,
		req: &types::count_tokens::Request,
		headers: &HeaderMap,
		request_model: &str,
	) -> Result<Vec<u8>, AIError> {
		match self {
			AIProvider::Anthropic(_) | AIProvider::Custom(_) => {
				serde_json::to_vec(req).map_err(AIError::RequestMarshal)
			},
			AIProvider::Bedrock(_) => {
				conversion::bedrock::from_anthropic_token_count::translate(req, headers)
			},
			AIProvider::Vertex(provider) => {
				let body = serde_json::to_vec(req).map_err(AIError::RequestMarshal)?;
				provider.prepare_anthropic_count_tokens_body(body)
			},
			AIProvider::Azure(p)
				if matches!(p.resource_type, azure::AzureResourceType::Foundry)
					&& p.is_anthropic_model(Some(request_model)) =>
			{
				serde_json::to_vec(req).map_err(AIError::RequestMarshal)
			},
			_ => Err(AIError::UnsupportedConversion(strng::literal!(
				"count_tokens not supported for this provider"
			))),
		}
	}

	fn render_embeddings_request(
		&self,
		req: &types::embeddings::Request,
	) -> Result<Vec<u8>, AIError> {
		match self {
			AIProvider::Custom(_)
			| AIProvider::OpenAI(_)
			| AIProvider::Copilot(_)
			| AIProvider::Azure(_)
			| AIProvider::Gemini(_)
			| AIProvider::Anthropic(_) => serde_json::to_vec(req).map_err(AIError::RequestMarshal),
			AIProvider::Vertex(_) => conversion::vertex::from_embeddings::translate(req),
			AIProvider::Bedrock(p) => conversion::bedrock::from_embeddings::translate(req, p),
		}
	}

	fn render_rerank_request(&self, req: &types::rerank::Request) -> Result<Vec<u8>, AIError> {
		match self {
			AIProvider::Custom(_)
			| AIProvider::OpenAI(_)
			| AIProvider::Copilot(_)
			| AIProvider::Azure(_)
			| AIProvider::Gemini(_)
			| AIProvider::Anthropic(_) => serde_json::to_vec(req).map_err(AIError::RequestMarshal),
			AIProvider::Vertex(p) => conversion::vertex::from_rerank::translate(req, p),
			AIProvider::Bedrock(p) => conversion::bedrock::from_rerank::translate(req, p),
		}
	}

	fn apply_model_alias(&self, policies: Option<&Policy>, req: &mut impl RequestType) {
		if let Some(p) = policies {
			// Apply model alias resolution
			if req.supports_model()
				&& let Some(model) = req.model()
				&& let Some(aliased) = p.resolve_model_alias(model.as_str())
			{
				*model = aliased.to_string();
			}
		}
	}

	#[allow(clippy::too_many_arguments)]
	async fn prepare_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		original_format: InputFormat,
		req: &mut impl RequestType,
		parts: &mut Parts,
		provider_format: Option<custom::ProviderFormat>,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<PreparedRequest, AIError> {
		if let Some(p) = policies {
			p.apply_prompt_enrichment(req);

			if original_format.supports_prompt_guard() {
				let http_headers = &parts.headers;
				let claims = parts.extensions.get::<Claims>().cloned();
				let original = log.as_ref().and_then(|l| l.request_snapshot.clone());
				if let Some((response, guardrail)) = p
					.apply_prompt_guard(backend_info, req, http_headers, claims, original.as_deref())
					.await
					.map_err(|e| {
						warn!("failed to call prompt guard webhook: {e}");
						AIError::PromptWebhookError
					})? {
					return Ok(PreparedRequest::GuardrailRejected {
						response,
						guardrail,
					});
				}
			}
		}

		let mut llm_info = req.to_llm_request(self.provider(), tokenize)?;
		if original_format == InputFormat::Detect {
			types::detect::amend_request_info(&mut llm_info, parts.uri.path());
		}
		llm_info.cache_convention =
			cache_convention_for(self, provider_format, &llm_info.request_model);
		if let Some(log) = log
			&& log.cel.cel_context.needs_llm_prompt()
			&& original_format.supports_prompt_guard()
		{
			llm_info.prompt = Some(req.get_messages().into());
		}

		Ok(PreparedRequest::Ready(llm_info))
	}

	#[allow(clippy::too_many_arguments)]
	async fn process_chat_request<T, F>(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		original_format: InputFormat,
		mut req: T,
		mut parts: Parts,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
		chat_request: F,
	) -> Result<RequestResult, AIError>
	where
		T: RequestType,
		F: for<'a> FnOnce(&'a T) -> types::ChatRequest<'a>,
	{
		let request_model = if req.supports_model() {
			req.model().as_deref().map(str::to_string)
		} else {
			None
		};
		let chat_translation = self.chat_translation(original_format, request_model.as_deref())?;
		let provider_format = chat_translation.provider_format();
		let prepared = self
			.prepare_request(
				backend_info,
				policies,
				original_format,
				&mut req,
				&mut parts,
				Some(provider_format),
				tokenize,
				log,
			)
			.await?;
		let mut llm_info = match prepared {
			PreparedRequest::Ready(llm_info) => llm_info,
			PreparedRequest::GuardrailRejected {
				response,
				guardrail,
			} => {
				return Ok(RequestResult::GuardrailRejected {
					response,
					guardrail,
				});
			},
		};

		let rendered = chat_translation.render_request(
			chat_request(&req),
			&ChatRequestContext {
				provider: self,
				headers: &parts.headers,
				prompt_caching: policies.and_then(|p| p.prompt_caching.as_ref()),
			},
		)?;
		llm_info.provider_state = rendered.provider_state;
		parts.headers.remove(header::CONTENT_LENGTH);
		let req = Request::from_parts(parts, Body::from(rendered.body));
		Ok(RequestResult::Success {
			request: req,
			llm_request: llm_info,
			upstream_route_type: provider_format.route_type(),
		})
	}

	#[allow(clippy::too_many_arguments)]
	async fn process_non_chat_request<T, F>(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		original_format: InputFormat,
		mut req: T,
		mut parts: Parts,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
		render: F,
	) -> Result<RequestResult, AIError>
	where
		T: RequestType,
		F: FnOnce(&AIProvider, &T, &Parts, &str) -> Result<Vec<u8>, AIError>,
	{
		let request_model = if req.supports_model() {
			req.model().as_deref().map(str::to_string)
		} else {
			None
		};
		let provider_format = if original_format == InputFormat::Detect {
			None
		} else {
			self
				.non_chat_provider_format_for(original_format, request_model.as_deref())
				.ok_or_else(|| {
					AIError::UnsupportedConversion(strng::format!(
						"from {original_format:?} to provider {}",
						self.provider()
					))
				})?
				.into()
		};
		let prepared = self
			.prepare_request(
				backend_info,
				policies,
				original_format,
				&mut req,
				&mut parts,
				provider_format,
				tokenize,
				log,
			)
			.await?;
		let llm_info = match prepared {
			PreparedRequest::Ready(llm_info) => llm_info,
			PreparedRequest::GuardrailRejected {
				response,
				guardrail,
			} => {
				return Ok(RequestResult::GuardrailRejected {
					response,
					guardrail,
				});
			},
		};
		let request_model = llm_info.request_model.as_str();
		let body = render(self, &req, &parts, request_model)?;
		parts.headers.remove(header::CONTENT_LENGTH);
		let req = Request::from_parts(parts, Body::from(body));
		Ok(RequestResult::Success {
			request: req,
			llm_request: llm_info,
			upstream_route_type: provider_format
				.map(custom::ProviderFormat::route_type)
				.unwrap_or(RouteType::Detect),
		})
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn process_response(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		log: AsyncLog<llm::LLMInfo>,
		log_content: LogContentFields,
		model_catalog: Option<&Arc<cost::ModelCatalog>>,
		resp: Response,
	) -> Result<Response, AIError> {
		// Non-success responses are plain JSON, not event-stream data.
		// Only enter the streaming path for successful responses; errors
		// fall through to the buffered path where process_error translates them.
		if req.streaming && resp.status().is_success() {
			return self.process_streaming(
				client,
				req,
				rate_limit,
				req_snapshot,
				log,
				log_content,
				model_catalog.cloned(),
				resp,
			);
		}
		let model_catalog = model_catalog.map(Arc::as_ref);

		let buffered = Self::buffer_response(resp).await?;

		match req.input_format {
			InputFormat::CountTokens => {
				self.process_count_tokens_response(req, buffered, model_catalog, &log)
			},
			InputFormat::Embeddings => {
				self.process_embeddings_buffered_response(req, buffered, model_catalog, &log)
			},
			InputFormat::Rerank => {
				self.process_rerank_buffered_response(req, buffered, model_catalog, &log)
			},
			_ => {
				self
					.process_chat_or_detect_buffered_response(
						client,
						req,
						rate_limit,
						req_snapshot,
						log,
						log_content,
						model_catalog,
						buffered,
					)
					.await
			},
		}
	}

	#[allow(clippy::too_many_arguments)]
	async fn process_chat_or_detect_buffered_response(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		log: AsyncLog<llm::LLMInfo>,
		log_content: LogContentFields,
		model_catalog: Option<&cost::ModelCatalog>,
		buffered: BufferedResponse,
	) -> Result<Response, AIError> {
		let BufferedResponse {
			mut parts,
			bytes,
			encoding,
		} = buffered;

		let (llm_resp, body) = if !parts.status.is_success() {
			let body = self.process_error(&req, parts.status, &bytes)?;
			(LLMResponse::default(), body)
		} else {
			let mut resp = self.translate_chat_or_detect_response(&req, &bytes)?;
			let prompt_guard_headers =
				response_prompt_guard_headers(&parts.headers, rate_limit.request_traceparent.as_ref());

			// Apply response prompt guard
			if let Some(dr) = Policy::apply_response_prompt_guard(
				&client,
				resp.as_mut(),
				&prompt_guard_headers,
				&rate_limit.prompt_guard,
				req_snapshot.as_deref(),
			)
			.await
			.map_err(|e| {
				warn!("failed to apply response prompt guard: {e}");
				AIError::PromptWebhookError
			})? {
				return Ok(dr);
			}

			let llm_resp = resp.to_llm_response(log_content);
			let body = resp.serialize().map_err(AIError::ResponseParsing)?;
			(llm_resp, Bytes::copy_from_slice(&body))
		};

		let body = if let Some(encoding) = encoding {
			parts
				.headers
				.insert(header::CONTENT_ENCODING, HeaderValue::from_static(encoding));
			Body::from(
				http::compression::encode_body(&body, encoding)
					.await
					.map_err(AIError::Encoding)?,
			)
		} else {
			Body::from(body)
		};
		parts.headers.remove(header::CONTENT_LENGTH);
		let llm_info = LLMInfo::new(req, llm_resp);
		parts
			.extensions
			.insert(crate::cel::LLMContext::from_llm_info(
				llm_info.clone(),
				model_catalog,
			));
		let resp = Response::from_parts(parts, body);

		if !rate_limit.local_rate_limit.is_empty() || rate_limit.remote_rate_limit.is_some() {
			let exec = cel::Executor::new_response(req_snapshot.as_deref(), &resp);
			// In the initial request, we subtracted the approximate request tokens.
			// Now we should have the real request tokens and the response tokens
			amend_tokens(rate_limit, &llm_info, exec);
		}
		log.store(Some(llm_info));
		Ok(resp)
	}

	async fn buffer_response(resp: Response) -> Result<BufferedResponse, AIError> {
		let buffer_limit = http::response_buffer_limit(&resp);
		let (mut parts, body) = resp.into_parts();
		let body = dtrace::TracingBody::maybe_wrap("llm raw response", body, buffer_limit);
		let ce = parts.headers.typed_get::<ContentEncoding>();
		let (encoding, bytes) =
			http::compression::to_bytes_with_decompression(body, ce.as_ref(), buffer_limit)
				.await
				.map_err(|e| map_compression_error(e, &parts.headers))?;

		// Snapshot decompressed bytes for CEL response.body access before re-compression,
		// so maybe_buffer_response_body can skip decompression entirely.
		if encoding.is_some() {
			parts
				.extensions
				.insert(crate::cel::BufferedBody::complete(bytes.clone()));
			parts.headers.remove(header::CONTENT_ENCODING);
			parts.headers.remove(header::TRANSFER_ENCODING);
		}

		Ok(BufferedResponse {
			parts,
			bytes,
			encoding,
		})
	}

	fn finalize_response(
		mut parts: ::http::response::Parts,
		body: Body,
		req: LLMRequest,
		llm_resp: LLMResponse,
		model_catalog: Option<&cost::ModelCatalog>,
		log: &AsyncLog<llm::LLMInfo>,
	) -> Response {
		let llm_info = LLMInfo::new(req, llm_resp);
		parts
			.extensions
			.insert(crate::cel::LLMContext::from_llm_info(
				llm_info.clone(),
				model_catalog,
			));
		log.store(Some(llm_info));
		Response::from_parts(parts, body)
	}

	fn process_count_tokens_response(
		&self,
		req: LLMRequest,
		buffered: BufferedResponse,
		model_catalog: Option<&cost::ModelCatalog>,
		log: &AsyncLog<llm::LLMInfo>,
	) -> Result<Response, AIError> {
		let BufferedResponse {
			mut parts, bytes, ..
		} = buffered;
		let (bytes, count) = match self {
			AIProvider::Anthropic(_) | AIProvider::Vertex(_) | AIProvider::Bedrock(_) => {
				types::count_tokens::Response::translate_response(bytes)?
			},
			AIProvider::Azure(p)
				if matches!(p.resource_type, azure::AzureResourceType::Foundry)
					&& p.is_anthropic_model(Some(&req.request_model)) =>
			{
				// Foundry returns the Anthropic-native count_tokens shape for Claude models.
				types::count_tokens::Response::translate_response(bytes)?
			},
			AIProvider::Custom(p) if p.supports(custom::ProviderFormat::AnthropicTokenCount) => {
				types::count_tokens::Response::translate_response(bytes)?
			},
			_ => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"count_tokens response not supported for this provider"
				)));
			},
		};

		parts.headers.remove(header::CONTENT_LENGTH);
		Ok(Self::finalize_response(
			parts,
			bytes.into(),
			req,
			LLMResponse {
				count_tokens: Some(count),
				..Default::default()
			},
			model_catalog,
			log,
		))
	}

	fn process_embeddings_buffered_response(
		&self,
		req: LLMRequest,
		buffered: BufferedResponse,
		model_catalog: Option<&cost::ModelCatalog>,
		log: &AsyncLog<llm::LLMInfo>,
	) -> Result<Response, AIError> {
		let BufferedResponse {
			mut parts, bytes, ..
		} = buffered;
		parts.headers.remove(header::CONTENT_LENGTH);
		if !parts.status.is_success() {
			let body = self.process_error(&req, parts.status, &bytes)?;
			return Ok(Self::finalize_response(
				parts,
				body.into(),
				req,
				LLMResponse::default(),
				model_catalog,
				log,
			));
		}
		let (llm_resp, bytes) = self.process_embeddings_response(&req, &parts.headers, bytes)?;
		Ok(Self::finalize_response(
			parts,
			bytes.into(),
			req,
			llm_resp,
			model_catalog,
			log,
		))
	}

	fn process_rerank_buffered_response(
		&self,
		req: LLMRequest,
		buffered: BufferedResponse,
		model_catalog: Option<&cost::ModelCatalog>,
		log: &AsyncLog<llm::LLMInfo>,
	) -> Result<Response, AIError> {
		let BufferedResponse {
			mut parts, bytes, ..
		} = buffered;
		parts.headers.remove(header::CONTENT_LENGTH);
		if !parts.status.is_success() {
			let body = self.process_error(&req, parts.status, &bytes)?;
			return Ok(Self::finalize_response(
				parts,
				body.into(),
				req,
				LLMResponse::default(),
				model_catalog,
				log,
			));
		}
		let (llm_resp, bytes) = self.process_rerank_response(bytes)?;
		Ok(Self::finalize_response(
			parts,
			bytes.into(),
			req,
			llm_resp,
			model_catalog,
			log,
		))
	}

	fn process_embeddings_response(
		&self,
		req: &LLMRequest,
		headers: &::http::HeaderMap,
		bytes: Bytes,
	) -> Result<(LLMResponse, Bytes), AIError> {
		match self {
			AIProvider::Bedrock(_) => {
				let translated = conversion::bedrock::from_embeddings::translate_response(
					&bytes,
					headers,
					&req.request_model,
				)?;
				let llm_resp = translated.to_llm_response(LogContentFields::default());
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			AIProvider::Vertex(p) if !p.is_anthropic_model(Some(&req.request_model)) => {
				let translated =
					conversion::vertex::from_embeddings::translate_response(&bytes, &req.request_model)?;
				let llm_resp = translated.to_llm_response(LogContentFields::default());
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			_ => {
				let resp: types::embeddings::Response =
					serde_json::from_slice(&bytes).map_err(logged_response_parsing(&bytes))?;
				Ok((resp.to_llm_response(LogContentFields::default()), bytes))
			},
		}
	}

	fn process_rerank_response(&self, bytes: Bytes) -> Result<(LLMResponse, Bytes), AIError> {
		match self {
			AIProvider::Bedrock(_) => {
				let translated = conversion::bedrock::from_rerank::translate_response(&bytes)?;
				let llm_resp = translated.to_llm_response(LogContentFields::default());
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			AIProvider::Vertex(_) => {
				let translated = conversion::vertex::from_rerank::translate_response(&bytes)?;
				let llm_resp = translated.to_llm_response(LogContentFields::default());
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			_ => {
				let resp =
					types::rerank::parse_response_lenient(&bytes).map_err(logged_response_parsing(&bytes))?;
				Ok((resp.to_llm_response(LogContentFields::default()), bytes))
			},
		}
	}

	fn parse_response<T>(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError>
	where
		T: ResponseType + DeserializeOwned + 'static,
	{
		Ok(Box::new(
			serde_json::from_slice::<T>(bytes).map_err(logged_response_parsing(bytes))?,
		))
	}

	fn translate_chat_or_detect_response(
		&self,
		req: &LLMRequest,
		bytes: &Bytes,
	) -> Result<Box<dyn ResponseType>, AIError> {
		if req.input_format == InputFormat::Detect {
			return Ok(Box::new(
				serde_json::from_slice::<types::detect::Response>(bytes)
					.unwrap_or_else(|_| types::detect::Response::new_raw(bytes.clone())),
			));
		}

		let translation = self.chat_translation(req.input_format, Some(&req.request_model))?;
		translation.render_response(
			bytes,
			&ChatResponseContext {
				model: &req.request_model,
				tool_name_map: bedrock_tool_name_map(req),
			},
		)
	}

	#[allow(clippy::too_many_arguments)]
	pub fn process_streaming(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		response_policies: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		log: AsyncLog<llm::LLMInfo>,
		log_content: LogContentFields,
		model_catalog: Option<Arc<cost::ModelCatalog>>,
		resp: Response,
	) -> Result<Response, AIError> {
		let model = req.request_model.clone();
		let input_format = req.input_format;
		let bedrock_tool_name_map = bedrock_tool_name_map(&req).cloned();
		let chat_translation = if input_format.is_chat() {
			Some(self.chat_translation(input_format, Some(&model))?)
		} else {
			None
		};
		// Store an empty response, as we stream in info we will parse into it
		let llmresp = llm::LLMInfo {
			request: req,
			response: LLMResponse::default(),
		};
		log.store(Some(llmresp));
		let buffer = http::response_buffer_limit(&resp);

		// Decompress before the SSE parser, which expects plaintext chunks.
		let (mut parts, body) = resp.into_parts();
		let body = dtrace::TracingBody::maybe_wrap("llm raw response", body, buffer);
		let ce = parts.headers.typed_get::<ContentEncoding>();
		let (body, decompressed_encoding) = http::compression::decompress_body(body, ce.as_ref())
			.map_err(|e| map_compression_error(e, &parts.headers))?;

		// Strip encoding headers after successful decompression
		if decompressed_encoding.is_some() {
			parts.headers.remove(header::CONTENT_ENCODING);
			parts.headers.remove(header::CONTENT_LENGTH);
			parts.headers.remove(header::TRANSFER_ENCODING);
		}

		// Build headers for guardrail evaluation (same as buffered path).
		let prompt_guard_headers = response_prompt_guard_headers(
			&parts.headers,
			response_policies.request_traceparent.as_ref(),
		);

		let resp = Response::from_parts(parts, body);
		let resp = if matches!(input_format, InputFormat::Detect) {
			resp
		} else {
			normalize_sse_response_headers(resp)
		};

		// Build evaluators before format translation so guardrails run against translated
		// SSE output, not raw upstream bytes. Applying them before translation silently
		// breaks Bedrock (AWS Event Stream is binary, not SSE) and any provider whose
		// wire format differs from SSE. Detect paths are raw pass-throughs; skip them.
		let evaluators = if response_policies.streaming_prompt_guard_enabled
			&& !response_policies.prompt_guard.is_empty()
			&& !matches!(input_format, InputFormat::Detect)
		{
			use policy::PromptGuard;
			let temp_guard = PromptGuard {
				streaming: policy::PromptGuardStreamingMode::Enabled,
				request: vec![],
				response: response_policies.prompt_guard.clone(),
			};
			temp_guard.begin_streaming_response_guard(
				&client,
				&prompt_guard_headers,
				req_snapshot.clone(),
			)
		} else {
			vec![]
		};

		let logger = AmendOnDrop::new(log, response_policies, req_snapshot, model_catalog).into_llm();
		let stream_format = match self {
			AIProvider::Bedrock(_) => "awsEventStream",
			_ => "sseJson",
		};
		crate::proxy::dtrace::trace(|trace| {
			trace.llm_streaming_translation(
				self.provider().to_string(),
				format!("{input_format:?}"),
				chat_translation
					.map(|translation| format!("{:?}", translation.provider_format().route_type())),
				stream_format.to_string(),
			)
		});
		let translated = if input_format.is_chat() {
			let translation = chat_translation.expect("chat translation was selected for chat input");
			translation.stream(
				resp,
				ChatStreamContext {
					buffer_limit: buffer,
					logger,
					model: model.to_string(),
					log_content,
					tool_name_map: bedrock_tool_name_map,
				},
			)
		} else {
			match (self, input_format) {
				(AIProvider::Bedrock(_), InputFormat::Detect) => {
					types::detect::passthrough_aws_stream(logger, resp)
				},
				(_, InputFormat::Detect) => types::detect::passthrough_stream(logger, resp),
				(_, InputFormat::Realtime) => {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"realtime does not use streaming codepath"
					)));
				},
				(_, _) => {
					return Err(AIError::UnsupportedConversion(strng::format!(
						"{input_format:?} does not use streaming response translation"
					)));
				},
			}
		};

		if !evaluators.is_empty() {
			// `logger` is owned by the translated body; pass None to avoid double-logging.
			return Ok(translated.map(|b| GuardedSseBody::new(b, evaluators, buffer, None)));
		}
		Ok(translated)
	}

	async fn read_body_and_default_model<T: RequestType + DeserializeOwned>(
		&self,
		policies: Option<&Policy>,
		hreq: Request,
		log: &mut Option<&mut RequestLog>,
	) -> Result<(Parts, T), AIError> {
		let buffer = http::buffer_limit(&hreq);
		let (mut parts, body) = hreq.into_parts();
		// Decode Content-Encoding (gzip/deflate/br/zstd) before parsing the body as
		// JSON. Clients such as the Claude Code harness gzip-compress request bodies
		// above a size threshold; without decoding, the reader would hand the
		// compressed bytes straight to serde_json and fail with a misleading
		// "LLM request body must be valid JSON" 400, even for tiny payloads. This
		// mirrors the response path, which already decompresses via the same helper.
		let ce = parts.headers.typed_get::<ContentEncoding>();
		let (encoding, bytes) =
			match http::compression::to_bytes_with_decompression(body, ce.as_ref(), buffer).await {
				Ok(v) => v,
				Err(http::compression::Error::LimitExceeded) => return Err(AIError::RequestTooLarge),
				Err(e) => return Err(map_compression_error(e, &parts.headers)),
			};
		// Strip encoding headers now that the body is plaintext so downstream
		// translation/marshalling and upstream forwarding see a consistent body.
		if encoding.is_some() {
			parts.headers.remove(header::CONTENT_ENCODING);
			parts.headers.remove(header::TRANSFER_ENCODING);
		}

		if self.override_model().is_none()
			&& types::detect::extract_model_from_path(parts.uri.path()).is_none()
			&& !policies.is_some_and(Policy::has_request_body_mutations)
		{
			let mut req: T = serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?;
			let model = req.model();
			if model.as_deref().is_none() {
				return Err(AIError::MissingField("model not specified".into()));
			}
			return Ok((parts, req));
		}

		let mut request: serde_json::Value =
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?;
		self.set_provider_request_model(&parts, &mut request)?;
		let mut request = if let Some(p) = policies {
			p.apply_request_body_mutations(request, log)?
		} else {
			request
		};
		self.finalize_request_model(&mut request)?;
		let req: T = serde_json::from_value(request).map_err(AIError::RequestParsing)?;

		Ok((parts, req))
	}

	fn set_provider_request_model(
		&self,
		parts: &Parts,
		req: &mut serde_json::Value,
	) -> Result<(), AIError> {
		let Some(obj) = req.as_object_mut() else {
			return Err(AIError::MissingField("request must be an object".into()));
		};
		if let Some(provider_model) = &self.override_model() {
			obj.insert(
				"model".to_string(),
				serde_json::Value::String(provider_model.to_string()),
			);
		} else if !matches!(obj.get("model"), Some(serde_json::Value::String(_)))
			&& let Some(path_model) = types::detect::extract_model_from_path(parts.uri.path())
		{
			obj.insert(
				"model".to_string(),
				serde_json::Value::String(path_model.to_string()),
			);
		}
		Ok(())
	}

	fn finalize_request_model(&self, req: &mut serde_json::Value) -> Result<(), AIError> {
		let Some(obj) = req.as_object_mut() else {
			return Err(AIError::MissingField("request must be an object".into()));
		};
		if obj
			.get("model")
			.and_then(serde_json::Value::as_str)
			.is_none()
		{
			return Err(AIError::MissingField("model not specified".into()));
		}
		Ok(())
	}

	fn process_error(
		&self,
		req: &LLMRequest,
		status: ::http::StatusCode,
		bytes: &Bytes,
	) -> Result<Bytes, AIError> {
		if req.input_format.is_chat() {
			let translation = self.chat_translation(req.input_format, Some(&req.request_model))?;
			return translation.error(
				bytes,
				status,
				self.chat_error_format(translation, Some(&req.request_model)),
			);
		}
		match (self, req.input_format) {
			(AIProvider::Custom(_), InputFormat::Embeddings) => Ok(bytes.clone()),
			(
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_),
				InputFormat::Embeddings,
			) => {
				// Passthrough; nothing needed
				Ok(bytes.clone())
			},
			(AIProvider::Gemini(_), InputFormat::Embeddings) => {
				// Passthrough; Gemini embeddings endpoint already returns OpenAI-compatible errors.
				Ok(bytes.clone())
			},
			(AIProvider::Vertex(_), InputFormat::Embeddings) => {
				// Passthrough; Vertex embeddings endpoint already returns OpenAI-compatible errors.
				Ok(bytes.clone())
			},
			(_, InputFormat::Detect) => {
				// Passthrough; nothing needed
				Ok(bytes.clone())
			},
			(AIProvider::Bedrock(_), InputFormat::Embeddings) => {
				conversion::bedrock::from_embeddings::translate_error(bytes)
			},
			(AIProvider::Custom(_), InputFormat::Rerank) => Ok(bytes.clone()),
			(
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_),
				InputFormat::Rerank,
			) => Ok(bytes.clone()),
			(AIProvider::Bedrock(_), InputFormat::Rerank) => {
				conversion::bedrock::from_rerank::translate_error(bytes)
			},
			(AIProvider::Vertex(_), InputFormat::Rerank) => {
				conversion::vertex::from_rerank::translate_error(bytes)
			},
			(_, InputFormat::Realtime) => Err(AIError::UnsupportedConversion(strng::literal!(
				"realtime does not use this codepath"
			))),
			(_, _) => Err(AIError::UnsupportedConversion(strng::literal!(
				"this provider and format is not supported"
			))),
		}
	}
}

fn bedrock_tool_name_map(req: &LLMRequest) -> Option<&conversion::bedrock::BedrockToolNameMap> {
	match &req.provider_state {
		Some(ProviderState::Bedrock { tool_names }) => Some(tool_names.as_ref()),
		_ => None,
	}
}

fn map_compression_error(e: http::compression::Error, headers: &::http::HeaderMap) -> AIError {
	match e {
		http::compression::Error::UnsupportedEncoding => AIError::UnsupportedEncoding(strng::new(
			headers
				.get(header::CONTENT_ENCODING)
				.and_then(|v| v.to_str().ok())
				.unwrap_or("unknown"),
		)),
		http::compression::Error::LimitExceeded => AIError::ResponseTooLarge,
		http::compression::Error::Io(e) => AIError::Encoding(axum_core::Error::new(e)),
		http::compression::Error::Body(e) => AIError::Encoding(e),
	}
}

fn response_prompt_guard_headers(
	response_headers: &HeaderMap,
	request_traceparent: Option<&HeaderValue>,
) -> HeaderMap {
	let mut headers = response_headers.clone();
	if let Some(traceparent) = request_traceparent {
		headers.insert(http::x_headers::TRACEPARENT, traceparent.clone());
	}
	headers
}

fn amend_tokens(rate_limit: store::LLMResponsePolicies, llm_resp: &LLMInfo, exec: Executor) {
	let input_mismatch = match (
		llm_resp.request.input_tokens,
		llm_resp.response.input_tokens,
	) {
		// Already counted 'req'
		(Some(req), Some(resp)) => (resp as i64) - (req as i64),
		// No request or response count... this is probably an issue.
		(_, None) => 0,
		// No request counted, so count the full response
		(_, Some(resp)) => resp as i64,
	};
	let response = llm_resp.response.output_tokens.unwrap_or_default();
	let tokens_to_remove = input_mismatch + (response as i64);

	for lrl in &rate_limit.local_rate_limit {
		lrl.amend_tokens(tokens_to_remove)
	}
	if let Some(rrl) = rate_limit.remote_rate_limit {
		rrl.amend_tokens(tokens_to_remove, &exec)
	}
}

pub struct AmendOnDrop {
	log: AsyncLog<llm::LLMInfo>,
	pol: Option<LLMResponsePolicies>,
	req: Option<Arc<RequestSnapshot>>,
	catalog: Option<Arc<cost::ModelCatalog>>,
}

impl AmendOnDrop {
	pub fn new(
		log: AsyncLog<llm::LLMInfo>,
		pol: LLMResponsePolicies,
		req: Option<Arc<RequestSnapshot>>,
		catalog: Option<Arc<cost::ModelCatalog>>,
	) -> Self {
		Self {
			log,
			pol: Some(pol),
			req,
			catalog,
		}
	}
	pub fn non_atomic_mutate(&self, f: impl FnOnce(&mut llm::LLMInfo)) {
		self.log.non_atomic_mutate(f);
	}
	pub fn report_usage(&mut self) {
		if let Some(pol) = self.pol.take()
			&& (!pol.local_rate_limit.is_empty() || pol.remote_rate_limit.is_some())
		{
			self.log.non_atomic_mutate(|r| {
				let ctx = LLMContext::from_llm_info(r.clone(), self.catalog.as_deref());
				let exec = cel::Executor::new_llm_rate_limit_streaming(self.req.as_deref(), &ctx);
				amend_tokens(pol, r, exec)
			});
		}
	}

	pub fn into_llm(self) -> agent_llm::StreamingUsageGuard {
		agent_llm::StreamingUsageGuard::new(Box::new(self))
	}
}

impl agent_llm::StreamingUsageReporter for AmendOnDrop {
	fn update(&self, f: &mut dyn FnMut(&mut llm::LLMInfo)) {
		self.log.non_atomic_mutate(|info| f(info));
	}

	fn report_usage(&mut self) {
		AmendOnDrop::report_usage(self);
	}
}

impl Drop for AmendOnDrop {
	fn drop(&mut self) {
		self.report_usage();
	}
}
