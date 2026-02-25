use std::str::FromStr;
use std::sync::Arc;

use ::http::request::Parts;
use ::http::uri::{Authority, PathAndQuery};
use ::http::{HeaderValue, header};
use agent_core::prelude::Strng;
use agent_core::strng;
use axum_extra::headers::authorization::Bearer;
use bytes::{Buf, BytesMut};
use headers::{ContentEncoding, HeaderMapExt};
pub use policy::Policy;
use rand::RngExt;
use serde::de::DeserializeOwned;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};
use tokio_util::codec::Decoder;

use crate::http::auth::{AwsAuth, BackendAuth, GcpAuth};
use crate::http::jwt::Claims;
use crate::http::{Body, Request, Response};
pub use crate::llm::types::{RequestType, ResponseType};
use crate::proxy::httpproxy::PolicyClient;
use crate::store::{BackendPolicies, LLMResponsePolicies};
use crate::telemetry::log::{AsyncLog, RequestLog};
use crate::types::agent::{BackendPolicy, Target};
use crate::types::loadbalancer::{ActiveHandle, EndpointWithInfo};
use crate::*;

pub mod anthropic;
pub mod azureopenai;
pub mod bedrock;
pub mod gemini;
pub mod openai;
pub mod vertex;

mod conversion;
pub mod policy;
mod types;

pub use types::SimpleChatCompletionMessage;

#[cfg(test)]
mod tests;

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
				let (_, EndpointWithInfo { endpoint, info }) =
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
	pub host_override: Option<Target>,
	pub path_override: Option<Strng>,
	/// Whether to tokenize on the request flow. This enables us to do more accurate rate limits,
	/// since we know (part of) the cost of the request upfront.
	/// This comes with the cost of an expensive operation.
	#[serde(default)]
	pub tokenize: bool,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendPolicy>,
}

impl NamedAIProvider {
	pub fn use_default_policies(&self) -> bool {
		self.host_override.is_none()
	}
}

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
	/// Send the request to the upstream LLM provider as-is
	ParsedPassthrough,
	/// OpenAI /responses
	Responses,
	/// OpenAI /embeddings
	Embeddings,
	/// OpenAI /realtime (websockets)
	Realtime,
	/// Anthropic /v1/messages/count_tokens
	AnthropicTokenCount,
}

#[apply(schema!)]
pub enum AIProvider {
	OpenAI(openai::Provider),
	Gemini(gemini::Provider),
	Vertex(vertex::Provider),
	Anthropic(anthropic::Provider),
	Bedrock(bedrock::Provider),
	AzureOpenAI(azureopenai::Provider),
}

#[apply(schema!)]
pub enum LocalModelAIProvider {
	OpenAI,
	Gemini,
	Vertex,
	Anthropic,
	Bedrock,
	AzureOpenAI,
}

trait Provider {
	const NAME: Strng;
}

#[derive(Debug, Clone)]
pub struct LLMRequest {
	/// Input tokens derived by tokenizing the request. Not always enabled
	pub input_tokens: Option<u64>,
	pub input_format: InputFormat,
	pub request_model: Strng,
	pub provider: Strng,
	pub streaming: bool,
	pub params: LLMRequestParams,
	pub prompt: Option<Arc<Vec<SimpleChatCompletionMessage>>>,
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
}

impl InputFormat {
	pub fn supports_prompt_guard(&self) -> bool {
		match self {
			InputFormat::Completions => true,
			InputFormat::Messages => true,
			InputFormat::Responses => true,
			InputFormat::Realtime => false,
			InputFormat::Embeddings => false,
			InputFormat::CountTokens => false,
			InputFormat::Detect => false,
		}
	}
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, ::cel::DynamicType)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
	// Embeddings
	#[serde(skip_serializing_if = "Option::is_none")]
	pub encoding_format: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub dimensions: Option<u64>,
}
impl PartialEq for LLMRequestParams {
	fn eq(&self, _: &Self) -> bool {
		// ignore for now since we have f64
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
	fn new(req: LLMRequest, resp: LLMResponse) -> Self {
		Self {
			request: req,
			response: resp,
		}
	}
	pub fn input_tokens(&self) -> Option<u64> {
		self.response.input_tokens.or(self.request.input_tokens)
	}
}

#[derive(Debug, Clone, Default)]
pub struct LLMResponse {
	pub input_tokens: Option<u64>,
	/// count_tokens contains the number of tokens in the request, when using the token counting endpoint
	/// These are not counted as 'input tokens' since they do not consume input tokens.
	pub count_tokens: Option<u64>,
	pub output_tokens: Option<u64>,
	pub total_tokens: Option<u64>,
	pub reasoning_tokens: Option<u64>,
	pub cache_creation_input_tokens: Option<u64>,
	pub cached_input_tokens: Option<u64>,

	pub provider_model: Option<Strng>,
	pub completion: Option<Vec<String>>,

	// Time to get the first token. Only used for streaming.
	pub first_token: Option<Instant>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum RequestResult {
	Success(Request, LLMRequest),
	Rejected(Response),
}

impl AIProvider {
	pub fn provider(&self) -> Strng {
		match self {
			AIProvider::OpenAI(_p) => openai::Provider::NAME,
			AIProvider::Anthropic(_p) => anthropic::Provider::NAME,
			AIProvider::Gemini(_p) => gemini::Provider::NAME,
			AIProvider::Vertex(_p) => vertex::Provider::NAME,
			AIProvider::Bedrock(_p) => bedrock::Provider::NAME,
			AIProvider::AzureOpenAI(_p) => azureopenai::Provider::NAME,
		}
	}
	pub fn override_model(&self) -> Option<Strng> {
		match self {
			AIProvider::OpenAI(p) => p.model.clone(),
			AIProvider::Anthropic(p) => p.model.clone(),
			AIProvider::Gemini(p) => p.model.clone(),
			AIProvider::Vertex(p) => p.model.clone(),
			AIProvider::Bedrock(p) => p.model.clone(),
			AIProvider::AzureOpenAI(p) => p.model.clone(),
		}
	}
	pub fn default_connector(&self) -> (Target, BackendPolicies) {
		let btls = BackendPolicies {
			backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
			// We will use original request for now
			..Default::default()
		};
		match self {
			AIProvider::OpenAI(_) => (Target::Hostname(openai::DEFAULT_HOST, 443), btls),
			AIProvider::Gemini(_) => (Target::Hostname(gemini::DEFAULT_HOST, 443), btls),
			AIProvider::Vertex(p) => {
				let bp = BackendPolicies {
					backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
					backend_auth: Some(BackendAuth::Gcp(GcpAuth::default())),
					..Default::default()
				};
				(Target::Hostname(p.get_host(), 443), bp)
			},
			AIProvider::Anthropic(_) => (Target::Hostname(anthropic::DEFAULT_HOST, 443), btls),
			AIProvider::Bedrock(p) => {
				let bp = BackendPolicies {
					backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
					backend_auth: Some(BackendAuth::Aws(AwsAuth::Implicit {})),
					..Default::default()
				};
				(Target::Hostname(p.get_host(), 443), bp)
			},
			AIProvider::AzureOpenAI(p) => (Target::Hostname(p.get_host(), 443), btls),
		}
	}

	pub fn setup_request(
		&self,
		req: &mut Request,
		route_type: RouteType,
		llm_request: Option<&LLMRequest>,
		apply_host_path_defaults: bool,
	) -> anyhow::Result<()> {
		if apply_host_path_defaults {
			self.set_host_path_defaults(req, route_type, llm_request)?;
		}
		self.set_required_fields(req)?;
		Ok(())
	}

	fn set_path_and_query(uri: &mut http::uri::Parts, path: &'static str) -> anyhow::Result<()> {
		let query = uri.path_and_query.as_ref().and_then(|p| p.query());
		if let Some(query) = query {
			uri.path_and_query = Some(PathAndQuery::from_maybe_shared(format!(
				"{}?{}",
				path, query
			))?);
		} else {
			uri.path_and_query = Some(PathAndQuery::from_static(path));
		};
		Ok(())
	}

	pub fn set_host_path_defaults(
		&self,
		req: &mut Request,
		route_type: RouteType,
		llm_request: Option<&LLMRequest>,
	) -> anyhow::Result<()> {
		let override_path = route_type != RouteType::Passthrough;
		match self {
			AIProvider::OpenAI(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if override_path {
						Self::set_path_and_query(uri, openai::path(route_type))?;
					}
					uri.authority = Some(Authority::from_static(openai::DEFAULT_HOST_STR));
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Anthropic(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if override_path {
						Self::set_path_and_query(uri, anthropic::path(route_type))?;
					}
					uri.authority = Some(Authority::from_static(anthropic::DEFAULT_HOST_STR));
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Gemini(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if override_path {
						Self::set_path_and_query(uri, gemini::path(route_type))?;
					}
					uri.authority = Some(Authority::from_static(gemini::DEFAULT_HOST_STR));
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Vertex(provider) => {
				let request_model = llm_request.map(|l| l.request_model.as_str());
				let streaming = llm_request.map(|l| l.streaming).unwrap_or(false);
				let path = provider.get_path_for_model(route_type, request_model, streaming);
				http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						uri.path_and_query = Some(PathAndQuery::from_str(&path)?);
						uri.authority = Some(Authority::from_str(&provider.get_host())?);
						Ok(())
					})?;
					Ok(())
				})
			},
			AIProvider::Bedrock(provider) => {
				http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						if override_path && let Some(l) = llm_request {
							let path =
								provider.get_path_for_route(route_type, l.streaming, l.request_model.as_str());
							uri.path_and_query = Some(PathAndQuery::from_str(&path)?);
						}
						uri.authority = Some(Authority::from_str(&provider.get_host())?);
						Ok(())
					})?;
					// Store the region in request extensions so AWS signing can use it
					req.extensions.insert(bedrock::AwsRegion {
						region: provider.region.as_str().to_string(),
					});
					Ok(())
				})
			},
			AIProvider::AzureOpenAI(provider) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if override_path && let Some(l) = llm_request {
						let path = provider.get_path_for_model(route_type, l.request_model.as_str());
						uri.path_and_query = Some(PathAndQuery::from_str(&path)?);
					}
					uri.authority = Some(Authority::from_str(&provider.get_host())?);
					Ok(())
				})?;
				Ok(())
			}),
		}
	}

	pub fn set_required_fields(&self, req: &mut Request) -> anyhow::Result<()> {
		match self {
			AIProvider::Anthropic(_) => {
				http::modify_req(req, |req| {
					if let Some(authz) = req.headers.typed_get::<headers::Authorization<Bearer>>() {
						// Move bearer token in anthropic header
						req.headers.remove(http::header::AUTHORIZATION);
						let mut api_key = HeaderValue::from_str(authz.token())?;
						api_key.set_sensitive(true);
						req.headers.insert("x-api-key", api_key);
						// https://docs.anthropic.com/en/api/versioning
						req
							.headers
							.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
					};
					Ok(())
				})
			},
			_ => Ok(()),
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
			.read_body_and_default_model::<types::completions::Request>(policies, req)
			.await?;

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
		self
			.process_request(
				backend_info,
				policies,
				InputFormat::Completions,
				req,
				parts,
				tokenize,
				log,
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
		let (parts, req) = self
			.read_body_and_default_model::<types::messages::Request>(policies, req)
			.await?;

		self
			.process_request(
				backend_info,
				policies,
				InputFormat::Messages,
				req,
				parts,
				tokenize,
				log,
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
		let (parts, req) = self
			.read_body_and_default_model::<types::embeddings::Request>(policies, req)
			.await?;

		self
			.process_request(
				backend_info,
				policies,
				InputFormat::Embeddings,
				req,
				parts,
				tokenize,
				log,
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
		let (mut parts, req) = self
			.read_body_and_default_model::<types::responses::Request>(policies, req)
			.await?;

		// Strip client-specific headers that cause AWS signature mismatches for Bedrock
		if matches!(self, AIProvider::Bedrock(_)) {
			parts.headers.remove("conversation_id");
			parts.headers.remove("session_id");
		}

		self
			.process_request(
				backend_info,
				policies,
				InputFormat::Responses,
				req,
				parts,
				tokenize,
				log,
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
		let (parts, req) = self
			.read_body_and_default_model::<types::count_tokens::Request>(policies, req)
			.await?;

		self
			.process_request(
				backend_info,
				policies,
				InputFormat::CountTokens,
				req,
				parts,
				false,
				log,
			)
			.await
	}

	pub async fn process_passthrough_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (parts, req) = self
			.read_body_and_default_model_allow_unknown::<types::detect::Request>(policies, req)
			.await?;

		self
			.process_request(
				backend_info,
				policies,
				InputFormat::Detect,
				req,
				parts,
				false,
				log,
			)
			.await
	}

	#[allow(clippy::too_many_arguments)]
	async fn process_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		original_format: InputFormat,
		mut req: impl RequestType,
		mut parts: ::http::request::Parts,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		match (self, original_format) {
			(_, InputFormat::Detect) => {
				// All providers support detect; this is a passthrough!
			},
			(_, InputFormat::Completions) => {
				// All providers support completions input
			},
			(
				AIProvider::OpenAI(_) | AIProvider::AzureOpenAI(_) | AIProvider::Bedrock(_),
				InputFormat::Responses,
			) => {
				// OpenAI supports responses input (Bedrock supports responses input via translation)
			},
			(
				AIProvider::Anthropic(_) | AIProvider::Bedrock(_) | AIProvider::Vertex(_),
				InputFormat::Messages,
			) => {
				// Anthropic supports messages input (Bedrock & Vertex support assuming serving Anthropic models)
			},
			(
				AIProvider::Anthropic(_) | AIProvider::Bedrock(_) | AIProvider::Vertex(_),
				InputFormat::CountTokens,
			) => {
				// Anthropic supports count_tokens natively (Bedrock & Vertex assumes its serving Anthropic models)
			},
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Bedrock(_)
				| AIProvider::Vertex(_),
				InputFormat::Embeddings,
			) => {
				// OpenAI compatible, Gemini, Bedrock, or Vertex
			},
			(p, m) => {
				// Messages with OpenAI compatible: currently only supports translating the request
				return Err(AIError::UnsupportedConversion(strng::format!(
					"{m:?} from provider {}",
					p.provider()
				)));
			},
		}
		if let Some(p) = policies {
			// Apply model alias resolution
			if let Some(model) = req.model()
				&& let Some(aliased) = p.resolve_model_alias(model.as_str())
			{
				*model = aliased.to_string();
			}
			p.apply_prompt_enrichment(&mut req);

			if original_format.supports_prompt_guard() {
				let http_headers = &parts.headers;
				let claims = parts.extensions.get::<Claims>().cloned();
				if let Some(dr) = p
					.apply_prompt_guard(backend_info, &mut req, http_headers, claims)
					.await
					.map_err(|e| {
						warn!("failed to call prompt guard webhook: {e}");
						AIError::PromptWebhookError
					})? {
					return Ok(RequestResult::Rejected(dr));
				}
			}
		}

		let mut llm_info = req.to_llm_request(self.provider(), tokenize)?;
		if let Some(log) = log
			&& log.cel.cel_context.needs_llm_prompt()
			&& original_format.supports_prompt_guard()
		{
			llm_info.prompt = Some(req.get_messages().into());
		}
		parts.extensions.insert(llm_info.clone());

		let request_model = llm_info.request_model.as_str();
		let new_request = if original_format == InputFormat::CountTokens {
			match self {
				AIProvider::Anthropic(_) => req.to_anthropic()?,
				AIProvider::Bedrock(_) => req.to_bedrock_token_count(&parts.headers)?,
				AIProvider::Vertex(provider) => {
					let body = req.to_anthropic()?;
					provider.prepare_anthropic_count_tokens_body(body)?
				},
				_ => {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"count_tokens not supported for this provider"
					)));
				},
			}
		} else {
			match self {
				AIProvider::Vertex(provider) if provider.is_anthropic_model(Some(request_model)) => {
					let body = req.to_anthropic()?;
					provider.prepare_anthropic_message_body(body)?
				},
				AIProvider::OpenAI(_) | AIProvider::Gemini(_) | AIProvider::AzureOpenAI(_) => {
					req.to_openai()?
				},
				AIProvider::Vertex(p) => req.to_vertex(p)?,
				AIProvider::Anthropic(_) => req.to_anthropic()?,
				AIProvider::Bedrock(p) => req.to_bedrock(
					p,
					Some(&parts.headers),
					policies.and_then(|p| p.prompt_caching.as_ref()),
				)?,
			}
		};

		parts.headers.remove(header::CONTENT_LENGTH);
		let req = Request::from_parts(parts, Body::from(new_request));
		Ok(RequestResult::Success(req, llm_info))
	}

	pub async fn process_response(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		log: AsyncLog<llm::LLMInfo>,
		include_completion_in_log: bool,
		resp: Response,
	) -> Result<Response, AIError> {
		tracing::error!("howardjohn: process success... {req:?}");
		if req.streaming {
			return self
				.process_streaming(req, rate_limit, log, include_completion_in_log, resp)
				.await;
		}

		// Buffer the body
		let buffer_limit = http::response_buffer_limit(&resp);
		let (mut parts, body) = resp.into_parts();
		let ce = parts.headers.typed_get::<ContentEncoding>();
		let (encoding, bytes) =
			http::compression::to_bytes_with_decompression(body, ce.as_ref(), buffer_limit)
				.await
				.map_err(|e| map_compression_error(e, &parts.headers))?;

		// count_tokens has simplified response handling (just format translation)
		if req.input_format == InputFormat::CountTokens {
			let (bytes, count) = match self {
				AIProvider::Anthropic(_) | AIProvider::Vertex(_) | AIProvider::Bedrock(_) => {
					types::count_tokens::Response::translate_response(bytes)?
				},
				_ => {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"count_tokens response not supported for this provider"
					)));
				},
			};

			parts.headers.remove(header::CONTENT_LENGTH);
			let resp = Response::from_parts(parts, bytes.into());
			let llm_resp = LLMResponse {
				count_tokens: Some(count),
				..Default::default()
			};
			let llm_info = LLMInfo::new(req, llm_resp);
			log.store(Some(llm_info));
			return Ok(resp);
		}

		// embeddings has simplified response handling
		if req.input_format == InputFormat::Embeddings {
			if !parts.status.is_success() {
				let body = self.process_error(&req, &bytes)?;
				parts.headers.remove(header::CONTENT_LENGTH);
				let resp = Response::from_parts(parts, body.into());
				let llm_info = LLMInfo::new(req, LLMResponse::default());
				log.store(Some(llm_info));
				return Ok(resp);
			}

			let (llm_resp, bytes) = match self {
				AIProvider::Bedrock(_) => {
					let translated = conversion::bedrock::from_embeddings::translate_response(
						&bytes,
						&parts.headers,
						&req.request_model,
					)?;
					let llm_resp = translated.to_llm_response(false);
					let body = translated.serialize().map_err(AIError::ResponseParsing)?;
					(llm_resp, Bytes::from(body))
				},
				AIProvider::Vertex(p) if !p.is_anthropic_model(Some(&req.request_model)) => {
					let translated =
						conversion::vertex::from_embeddings::translate_response(&bytes, &req.request_model)?;
					let llm_resp = translated.to_llm_response(false);
					let body = translated.serialize().map_err(AIError::ResponseParsing)?;
					(llm_resp, Bytes::from(body))
				},
				_ => (LLMResponse::default(), bytes),
			};

			parts.headers.remove(header::CONTENT_LENGTH);
			let resp = Response::from_parts(parts, bytes.into());
			let llm_info = LLMInfo::new(req, llm_resp);
			log.store(Some(llm_info));
			return Ok(resp);
		}

		let (llm_resp, body) = if !parts.status.is_success() {
			let body = self.process_error(&req, &bytes)?;
			(LLMResponse::default(), body)
		} else {
			tracing::error!("howardjohn: process success...");
			let mut resp = self.process_success(&req, &bytes)?;
			tracing::error!("howardjohn: process success!");

			// Apply response prompt guard
			if let Some(dr) = Policy::apply_response_prompt_guard(
				&client,
				resp.as_mut(),
				&parts.headers,
				&rate_limit.prompt_guard,
			)
			.await
			.map_err(|e| {
				warn!("failed to apply response prompt guard: {e}");
				AIError::PromptWebhookError
			})? {
				return Ok(dr);
			}

			let llm_resp = resp.to_llm_response(include_completion_in_log);
			let body = resp.serialize().map_err(AIError::ResponseParsing)?;
			(llm_resp, Bytes::copy_from_slice(&body))
		};

		let body = if let Some(encoding) = encoding {
			Body::from(
				http::compression::encode_body(&body, encoding)
					.await
					.map_err(AIError::Encoding)?,
			)
		} else {
			Body::from(body)
		};
		parts.headers.remove(header::CONTENT_LENGTH);
		let resp = Response::from_parts(parts, body);

		let llm_info = LLMInfo::new(req, llm_resp);
		// In the initial request, we subtracted the approximate request tokens.
		// Now we should have the real request tokens and the response tokens
		amend_tokens(rate_limit, &llm_info);
		log.store(Some(llm_info));
		Ok(resp)
	}

	fn process_success(
		&self,
		req: &LLMRequest,
		bytes: &Bytes,
	) -> Result<Box<dyn ResponseType>, AIError> {
		match (self, dbg!(req.input_format)) {
			(_, InputFormat::Detect) => Ok(Box::new(
				serde_json::from_slice::<types::detect::Response>(bytes).map_err(|e| {
					warn!(
						error = %e,
						body = %String::from_utf8_lossy(bytes),
						"failed to parse completions response"
					);
					AIError::ResponseParsing(e)
				})?,
			)),
			// Completions with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_) | AIProvider::Gemini(_) | AIProvider::AzureOpenAI(_),
				InputFormat::Completions,
			) => Ok(Box::new(
				serde_json::from_slice::<types::completions::Response>(bytes).map_err(|e| {
					warn!(
						error = %e,
						body = %String::from_utf8_lossy(bytes),
						"failed to parse completions response"
					);
					AIError::ResponseParsing(e)
				})?,
			)),
			// Responses with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_) | AIProvider::Gemini(_) | AIProvider::AzureOpenAI(_),
				InputFormat::Responses,
			) => Ok(Box::new(
				serde_json::from_slice::<types::responses::Response>(bytes).map_err(|e| {
					warn!(
						error = %e,
						body = %String::from_utf8_lossy(bytes),
						"failed to parse responses response"
					);
					AIError::ResponseParsing(e)
				})?,
			)),
			// Anthropic messages: passthrough
			(AIProvider::Anthropic(_) | AIProvider::Vertex(_), InputFormat::Messages) => Ok(Box::new(
				serde_json::from_slice::<types::messages::Response>(bytes)
					.map_err(AIError::ResponseParsing)?,
			)),
			// Supported paths with conversion...
			(AIProvider::Anthropic(_), InputFormat::Completions) => {
				conversion::messages::from_completions::translate_response(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Completions) => {
				conversion::bedrock::from_completions::translate_response(bytes, &req.request_model)
			},
			(AIProvider::Bedrock(_), InputFormat::Messages) => {
				conversion::bedrock::from_messages::translate_response(bytes, &req.request_model)
			},
			(AIProvider::Bedrock(_), InputFormat::Responses) => {
				conversion::bedrock::from_responses::translate_response(bytes, &req.request_model)
			},
			(AIProvider::Vertex(p), InputFormat::Completions) => {
				if p.is_anthropic_model(Some(&req.request_model)) {
					conversion::messages::from_completions::translate_response(bytes)
				} else {
					Ok(Box::new(
						serde_json::from_slice::<types::completions::Response>(bytes)
							.map_err(AIError::ResponseParsing)?,
					))
				}
			},
			(_, InputFormat::Messages) => Err(AIError::UnsupportedConversion(strng::literal!(
				"this provider does not support Messages"
			))),
			(_, InputFormat::Responses) => Err(AIError::UnsupportedConversion(strng::literal!(
				"this provider does not support Responses"
			))),
			(_, InputFormat::Realtime) => Err(AIError::UnsupportedConversion(strng::literal!(
				"realtime does not use this codepath"
			))),
			(_, InputFormat::CountTokens) => {
				unreachable!("CountTokens should be handled by process_count_tokens_response")
			},
			(_, InputFormat::Embeddings) => {
				unreachable!("Embeddings should be handled by process_embeddings_response")
			},
		}
	}

	pub async fn process_streaming(
		&self,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		log: AsyncLog<llm::LLMInfo>,
		include_completion_in_log: bool,
		resp: Response,
	) -> Result<Response, AIError> {
		let model = req.request_model.clone();
		let input_format = req.input_format;
		// Store an empty response, as we stream in info we will parse into it
		let llmresp = llm::LLMInfo {
			request: req,
			response: LLMResponse::default(),
		};
		log.store(Some(llmresp));
		let buffer = http::response_buffer_limit(&resp);

		// Decompress before the SSE parser, which expects plaintext chunks.
		let (mut parts, body) = resp.into_parts();
		let ce = parts.headers.typed_get::<ContentEncoding>();
		let (body, decompressed_encoding) = http::compression::decompress_body(body, ce.as_ref())
			.map_err(|e| map_compression_error(e, &parts.headers))?;

		// Strip encoding headers after successful decompression
		if decompressed_encoding.is_some() {
			parts.headers.remove(header::CONTENT_ENCODING);
			parts.headers.remove(header::CONTENT_LENGTH);
			parts.headers.remove(header::TRANSFER_ENCODING);
		}
		let resp = Response::from_parts(parts, body);

		Ok(match (self, input_format) {
			// Completions with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Vertex(_),
				InputFormat::Completions,
			) => conversion::completions::passthrough_stream(
				AmendOnDrop::new(log, rate_limit),
				include_completion_in_log,
				resp,
			),
			(_, InputFormat::Detect) => {
				types::detect::passthrough_stream(AmendOnDrop::new(log, rate_limit), resp)
			},
			// Responses with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Vertex(_),
				InputFormat::Responses,
			) => resp.map(|b| {
				conversion::responses::passthrough_stream(b, buffer, AmendOnDrop::new(log, rate_limit))
			}),
			// Anthropic messages: passthrough
			(AIProvider::Anthropic(_) | AIProvider::Vertex(_), InputFormat::Messages) => resp.map(|b| {
				conversion::messages::passthrough_stream(b, buffer, AmendOnDrop::new(log, rate_limit))
			}),
			// Supported paths with conversion...
			(AIProvider::Anthropic(_), InputFormat::Completions) => resp.map(|b| {
				conversion::messages::from_completions::translate_stream(
					b,
					buffer,
					AmendOnDrop::new(log, rate_limit),
				)
			}),
			(AIProvider::Bedrock(_), InputFormat::Completions) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_completions::translate_stream(
						b,
						buffer,
						AmendOnDrop::new(log, rate_limit),
						&model,
						&msg,
					)
				})
			},
			(AIProvider::Bedrock(_), InputFormat::Messages) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_messages::translate_stream(
						b,
						buffer,
						AmendOnDrop::new(log, rate_limit),
						&model,
						&msg,
					)
				})
			},
			(AIProvider::Bedrock(_), InputFormat::Responses) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_responses::translate_stream(
						b,
						buffer,
						AmendOnDrop::new(log, rate_limit),
						&model,
						&msg,
					)
				})
			},
			(_, InputFormat::Messages) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"this provider does not support Messages for streaming"
				)));
			},
			(_, InputFormat::Realtime) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"realtime does not use streaming codepath"
				)));
			},
			(AIProvider::Anthropic(_), InputFormat::Responses) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"this provider does not support Responses for streaming"
				)));
			},
			(_, InputFormat::CountTokens) => {
				unreachable!("CountTokens should be handled by process_count_tokens_response")
			},
			(_, InputFormat::Embeddings) => {
				unreachable!("Embeddings should be handled by process_embeddings_response")
			},
		})
	}

	async fn read_body_and_default_model<T: RequestType + DeserializeOwned>(
		&self,
		policies: Option<&Policy>,
		hreq: Request,
	) -> Result<(Parts, T), AIError> {
		let (p, mut r) = self
			.read_body_and_default_model_allow_unknown::<T>(policies, hreq)
			.await?;
		if r.model().is_none() {
			return Err(AIError::MissingField("model not specified".into()));
		}
		Ok((p, r))
	}

	async fn read_body_and_default_model_allow_unknown<T: RequestType + DeserializeOwned>(
		&self,
		policies: Option<&Policy>,
		hreq: Request,
	) -> Result<(Parts, T), AIError> {
		let buffer = http::buffer_limit(&hreq);
		let (parts, body) = hreq.into_parts();
		let Ok(bytes) = http::read_body_with_limit(body, buffer).await else {
			return Err(AIError::RequestTooLarge);
		};
		let mut req: T = if let Some(p) = policies {
			p.unmarshal_request(&bytes)?
		} else {
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?
		};

		if let Some(provider_model) = &self.override_model() {
			*req.model() = Some(provider_model.to_string());
		}
		Ok((parts, req))
	}

	fn process_error(&self, req: &LLMRequest, bytes: &Bytes) -> Result<Bytes, AIError> {
		match (self, req.input_format) {
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Vertex(_),
				InputFormat::Completions | InputFormat::Responses | InputFormat::Embeddings,
			) => {
				// Passthrough; nothing needed
				Ok(bytes.clone())
			},
			(AIProvider::Anthropic(_) | AIProvider::Vertex(_), InputFormat::Messages) => {
				// Passthrough; nothing needed
				Ok(bytes.clone())
			},
			(AIProvider::Anthropic(_), InputFormat::Completions) => {
				conversion::messages::from_completions::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Completions) => {
				conversion::bedrock::from_completions::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Messages) => {
				conversion::bedrock::from_messages::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Responses) => {
				conversion::bedrock::from_responses::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Embeddings) => {
				conversion::bedrock::from_embeddings::translate_error(bytes)
			},
			(_, _) => Err(AIError::UnsupportedConversion(strng::literal!(
				"this provider and format is not supported"
			))),
		}
	}
}

#[derive(Default)]
struct GuessedUsage {
	input_tokens: Option<u64>,
	output_tokens: Option<u64>,
	total_tokens: Option<u64>,
	reasoning_tokens: Option<u64>,
	cache_creation_input_tokens: Option<u64>,
	cached_input_tokens: Option<u64>,
}

fn guess_passthrough_input_format(path: &str) -> InputFormat {
	if path.contains("/embeddings") {
		InputFormat::Embeddings
	} else if path.contains("/messages/count_tokens") {
		InputFormat::CountTokens
	} else if path.contains("/responses") {
		InputFormat::Responses
	} else if path.contains("/messages") {
		InputFormat::Messages
	} else {
		InputFormat::Completions
	}
}

fn guess_passthrough_request_model(
	provider: &AIProvider,
	parts: &Parts,
	body_json: Option<&serde_json::Value>,
) -> Strng {
	if let Some(body_json) = body_json
		&& let Some(model) = first_str_at_paths(
			body_json,
			&[
				&["model"],
				&["model_id"],
				&["modelId"],
				&["request", "model"],
				&["modelSpec", "name"],
			],
		) {
		return model;
	}

	if let Ok(url) = http::as_url(&parts.uri)
		&& let Some((_, model)) = url.query_pairs().find(|(k, _)| k == "model")
	{
		return strng::new(model);
	}

	provider.override_model().unwrap_or_default()
}

fn guess_passthrough_streaming(parts: &Parts, body_json: Option<&serde_json::Value>) -> bool {
	if let Some(body_json) = body_json
		&& let Some(streaming) = first_bool_at_paths(body_json, &[&["stream"], &["streaming"]])
	{
		return streaming;
	}

	if let Ok(url) = http::as_url(&parts.uri)
		&& let Some((_, stream)) = url.query_pairs().find(|(k, _)| k == "stream")
	{
		return stream.eq_ignore_ascii_case("true") || stream == "1";
	}

	false
}

fn guess_passthrough_request_params(body_json: Option<&serde_json::Value>) -> LLMRequestParams {
	let Some(body_json) = body_json else {
		return LLMRequestParams::default();
	};

	LLMRequestParams {
		temperature: first_f64_at_paths(body_json, &[&["temperature"]]),
		top_p: first_f64_at_paths(body_json, &[&["top_p"]]),
		frequency_penalty: first_f64_at_paths(body_json, &[&["frequency_penalty"]]),
		presence_penalty: first_f64_at_paths(body_json, &[&["presence_penalty"]]),
		seed: first_i64_at_paths(body_json, &[&["seed"]]),
		max_tokens: first_u64_at_paths(body_json, &[&["max_tokens"], &["max_output_tokens"]]),
		encoding_format: first_str_at_paths(body_json, &[&["encoding_format"]]),
		dimensions: first_u64_at_paths(body_json, &[&["dimensions"]]),
	}
}

fn apply_passthrough_response_guess(llm_response: &mut LLMResponse, value: &serde_json::Value) {
	if let Some(model) = first_str_at_paths(
		value,
		&[
			&["model"],
			&["message", "model"],
			&["response", "model"],
			&["result", "model"],
			&["model_id"],
			&["modelId"],
		],
	) {
		llm_response.provider_model = Some(model);
	}

	let guessed_usage = GuessedUsage {
		input_tokens: first_u64_at_paths(
			value,
			&[
				&["usage", "input_tokens"],
				&["usage", "prompt_tokens"],
				&["usage", "inputTokenCount"],
				&["message", "usage", "input_tokens"],
				&["response", "usage", "input_tokens"],
				&["response", "usage", "prompt_tokens"],
				&["input_tokens"],
				&["prompt_tokens"],
			],
		),
		output_tokens: first_u64_at_paths(
			value,
			&[
				&["usage", "output_tokens"],
				&["usage", "completion_tokens"],
				&["usage", "outputTokenCount"],
				&["message", "usage", "output_tokens"],
				&["response", "usage", "output_tokens"],
				&["response", "usage", "completion_tokens"],
				&["output_tokens"],
				&["completion_tokens"],
			],
		),
		total_tokens: first_u64_at_paths(
			value,
			&[
				&["usage", "total_tokens"],
				&["usage", "totalTokenCount"],
				&["response", "usage", "total_tokens"],
				&["total_tokens"],
			],
		),
		reasoning_tokens: first_u64_at_paths(
			value,
			&[
				&["usage", "completion_tokens_details", "reasoning_tokens"],
				&["usage", "output_tokens_details", "reasoning_tokens"],
				&[
					"response",
					"usage",
					"completion_tokens_details",
					"reasoning_tokens",
				],
				&[
					"response",
					"usage",
					"output_tokens_details",
					"reasoning_tokens",
				],
			],
		),
		cache_creation_input_tokens: first_u64_at_paths(
			value,
			&[
				&["usage", "cache_creation_input_tokens"],
				&["usage", "cache_write_input_tokens"],
				&["message", "usage", "cache_creation_input_tokens"],
				&["response", "usage", "cache_creation_input_tokens"],
				&["response", "usage", "cache_write_input_tokens"],
			],
		),
		cached_input_tokens: first_u64_at_paths(
			value,
			&[
				&["usage", "prompt_tokens_details", "cached_tokens"],
				&["usage", "input_tokens_details", "cached_tokens"],
				&["usage", "cache_read_input_tokens"],
				&["message", "usage", "cache_read_input_tokens"],
				&["response", "usage", "input_tokens_details", "cached_tokens"],
				&["response", "usage", "cache_read_input_tokens"],
			],
		),
	};

	if let Some(input_tokens) = guessed_usage.input_tokens {
		llm_response.input_tokens = Some(input_tokens);
	}
	if let Some(output_tokens) = guessed_usage.output_tokens {
		llm_response.output_tokens = Some(output_tokens);
	}
	if let Some(total_tokens) = guessed_usage.total_tokens {
		llm_response.total_tokens = Some(total_tokens);
	}
	if let Some(reasoning_tokens) = guessed_usage.reasoning_tokens {
		llm_response.reasoning_tokens = Some(reasoning_tokens);
	}
	if let Some(cache_creation_input_tokens) = guessed_usage.cache_creation_input_tokens {
		llm_response.cache_creation_input_tokens = Some(cache_creation_input_tokens);
	}
	if let Some(cached_input_tokens) = guessed_usage.cached_input_tokens {
		llm_response.cached_input_tokens = Some(cached_input_tokens);
	}

	if llm_response.total_tokens.is_none()
		&& let (Some(input_tokens), Some(output_tokens)) =
			(llm_response.input_tokens, llm_response.output_tokens)
	{
		llm_response.total_tokens = Some(input_tokens + output_tokens);
	}
}

fn value_at_path<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
	let mut current = value;
	for seg in path {
		current = current.get(*seg)?;
	}
	Some(current)
}

fn first_str_at_paths(value: &serde_json::Value, paths: &[&[&str]]) -> Option<Strng> {
	paths.iter().find_map(|path| {
		value_at_path(value, path)
			.and_then(serde_json::Value::as_str)
			.map(strng::new)
	})
}

fn first_bool_at_paths(value: &serde_json::Value, paths: &[&[&str]]) -> Option<bool> {
	paths
		.iter()
		.find_map(|path| value_at_path(value, path))
		.and_then(|v| {
			v.as_bool().or_else(|| {
				v.as_str().and_then(|s| {
					if s.eq_ignore_ascii_case("true") || s == "1" {
						Some(true)
					} else if s.eq_ignore_ascii_case("false") || s == "0" {
						Some(false)
					} else {
						None
					}
				})
			})
		})
}

fn first_u64_at_paths(value: &serde_json::Value, paths: &[&[&str]]) -> Option<u64> {
	paths
		.iter()
		.find_map(|path| value_at_path(value, path))
		.and_then(|v| {
			v.as_u64()
				.or_else(|| {
					v.as_i64()
						.and_then(|i| if i >= 0 { Some(i as u64) } else { None })
				})
				.or_else(|| {
					v.as_f64().and_then(|f| {
						if f.is_finite() && f >= 0.0 {
							Some(f as u64)
						} else {
							None
						}
					})
				})
				.or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
		})
}

fn first_f64_at_paths(value: &serde_json::Value, paths: &[&[&str]]) -> Option<f64> {
	paths
		.iter()
		.find_map(|path| value_at_path(value, path))
		.and_then(|v| {
			v.as_f64()
				.or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
		})
}

fn first_i64_at_paths(value: &serde_json::Value, paths: &[&[&str]]) -> Option<i64> {
	paths
		.iter()
		.find_map(|path| value_at_path(value, path))
		.and_then(|v| {
			v.as_i64()
				.or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
				.or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
		})
}

fn is_json_content_type(resp: &Response) -> bool {
	resp
		.headers()
		.get(header::CONTENT_TYPE)
		.and_then(|v| v.to_str().ok())
		.map(|v| {
			let ct = v.to_ascii_lowercase();
			ct.contains("application/json") || ct.contains("+json")
		})
		.unwrap_or(false)
}

fn is_sse_content_type(resp: &Response) -> bool {
	resp
		.headers()
		.get(header::CONTENT_TYPE)
		.and_then(|v| v.to_str().ok())
		.map(|v| v.to_ascii_lowercase().contains("text/event-stream"))
		.unwrap_or(false)
}

struct JsonValueDecoder {
	limit: usize,
	buffer: BytesMut,
	overflowed: bool,
	emitted: bool,
}

impl JsonValueDecoder {
	fn new(limit: usize) -> Self {
		Self {
			limit,
			buffer: BytesMut::new(),
			overflowed: false,
			emitted: false,
		}
	}
}

impl Decoder for JsonValueDecoder {
	type Item = Option<serde_json::Value>;
	type Error = std::io::Error;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		if src.is_empty() {
			return Ok(None);
		}

		if !self.overflowed {
			let remaining = self.limit.saturating_sub(self.buffer.len());
			if src.len() <= remaining {
				self.buffer.extend_from_slice(src);
			} else {
				self.buffer.extend_from_slice(&src[..remaining]);
				self.overflowed = true;
			}
		}

		src.advance(src.len());
		Ok(None)
	}

	fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		let _ = self.decode(src)?;
		if self.emitted {
			return Ok(None);
		}
		self.emitted = true;

		if self.overflowed {
			return Ok(Some(None));
		}

		Ok(Some(
			serde_json::from_slice::<serde_json::Value>(&self.buffer).ok(),
		))
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

fn num_tokens_from_messages(
	model: &str,
	messages: &[SimpleChatCompletionMessage],
) -> Result<u64, AIError> {
	// NOTE: This estimator only accounts for textual content in normalized messages.
	// Non-text items in Responses inputs (e.g., tool calls, images, files) are ignored here.
	// Use provider token counting endpoints if you need precise totals for those cases.
	let tokenizer = get_tokenizer(model).unwrap_or(Tokenizer::Cl100kBase);
	if tokenizer != Tokenizer::Cl100kBase && tokenizer != Tokenizer::O200kBase {
		// Chat completion is only supported chat models
		return Err(AIError::UnsupportedModel);
	}
	let bpe = get_bpe_from_tokenizer(tokenizer);

	let tokens_per_message = 3;

	let mut num_tokens: u64 = 0;
	for message in messages {
		num_tokens += tokens_per_message;
		// Role is always 1 token
		num_tokens += 1;
		num_tokens += bpe
			.encode_with_special_tokens(message.content.as_str())
			.len() as u64;
	}
	num_tokens += 3; // every reply is primed with <|start|>assistant<|message|>
	Ok(num_tokens)
}

fn num_tokens_from_anthropic_messages(
	model: &str,
	messages: &[types::messages::RequestMessage],
) -> Result<u64, AIError> {
	let tokenizer = get_tokenizer(model).unwrap_or(Tokenizer::Cl100kBase);
	if tokenizer != Tokenizer::Cl100kBase && tokenizer != Tokenizer::O200kBase {
		// Chat completion is only supported chat models
		return Err(AIError::UnsupportedModel);
	}
	let bpe = get_bpe_from_tokenizer(tokenizer);

	let tokens_per_message = 3;

	let mut num_tokens: u64 = 0;
	for message in messages {
		num_tokens += tokens_per_message;
		// Role is always 1 token
		num_tokens += 1;
		if let Some(t) = message.message_text() {
			num_tokens += bpe
				.encode_with_special_tokens(
					// We filter non-text previously
					t,
				)
				.len() as u64;
		}
	}
	num_tokens += 3; // every reply is primed with <|start|>assistant<|message|>
	Ok(num_tokens)
}

/// Tokenizers take about 200ms to load and are lazy loaded. This loads them on demand, outside the
/// request path
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
	#[error("unsupported conversion to {0}")]
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
	#[error("failed to marshal response: {0}")]
	ResponseMarshal(serde_json::Error),
	#[error("unsupported content encoding: {0}")]
	UnsupportedEncoding(Strng),
	#[error("failed to encode response: {0}")]
	Encoding(axum_core::Error),
	#[error("error computing tokens")]
	JoinError(#[from] tokio::task::JoinError),
}

fn amend_tokens(rate_limit: store::LLMResponsePolicies, llm_resp: &LLMInfo) {
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
		rrl.amend_tokens(tokens_to_remove)
	}
}

pub struct AmendOnDrop {
	log: AsyncLog<llm::LLMInfo>,
	pol: Option<LLMResponsePolicies>,
}

impl AmendOnDrop {
	pub fn new(log: AsyncLog<llm::LLMInfo>, pol: LLMResponsePolicies) -> Self {
		Self {
			log,
			pol: Some(pol),
		}
	}
	pub fn non_atomic_mutate(&self, f: impl FnOnce(&mut llm::LLMInfo)) {
		self.log.non_atomic_mutate(f);
	}
	pub fn report_rate_limit(&mut self) {
		if let Some(pol) = self.pol.take() {
			self.log.non_atomic_mutate(|r| amend_tokens(pol, r));
		}
	}
}

impl Drop for AmendOnDrop {
	fn drop(&mut self) {
		self.report_rate_limit();
	}
}
