use std::str::FromStr;
use std::sync::Arc;

use ::http::request::Parts;
use ::http::uri::{Authority, PathAndQuery};
use ::http::{HeaderValue, header};
use agent_core::prelude::Strng;
use agent_core::strng;
use agent_hbone::server::RequestParts;
use axum_extra::headers::authorization::Bearer;
use headers::{ContentEncoding, HeaderMapExt};
pub use policy::Policy;
use rand::Rng;
use serde::de::DeserializeOwned;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use crate::http::auth::{AwsAuth, BackendAuth};
use crate::http::jwt::Claims;
use crate::http::{Body, Request, Response};
use crate::llm::types::{RequestType, ResponseType};
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputFormat {
	Completions,
	Messages,
	Responses,
	Embeddings,
	Realtime,
	CountTokens,
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
		}
	}
}

#[derive(Default, Clone, Debug, Serialize)]
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
					backend_auth: Some(BackendAuth::Gcp {}),
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
						Self::set_path_and_query(uri, anthropic::DEFAULT_PATH)?;
					}
					uri.authority = Some(Authority::from_static(anthropic::DEFAULT_HOST_STR));
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Gemini(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					if override_path {
						Self::set_path_and_query(uri, gemini::DEFAULT_PATH)?;
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
		match (original_format, self) {
			(InputFormat::Completions, _) => {
				// All providers support completions input
			},
			(InputFormat::Messages, AIProvider::Anthropic(_)) => {
				// Anthropic supports messages input
			},
			(InputFormat::Messages, AIProvider::Bedrock(_)) => {
				// Bedrock supports messages input (Anthropic passthrough)
			},
			(InputFormat::Responses, AIProvider::OpenAI(_)) => {
				// OpenAI supports responses input
			},
			(InputFormat::Responses, AIProvider::Bedrock(_)) => {
				// Bedrock supports responses input via translation
			},
			(InputFormat::CountTokens, AIProvider::Bedrock(_)) => {
				// Bedrock supports count_tokens input via translation
			},
			(InputFormat::Embeddings, AIProvider::OpenAI(_) | AIProvider::AzureOpenAI(_)) => {
				// passthrough
			},
			(m, p) => {
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

		let llm_info = req.to_llm_request(self.provider(), tokenize)?;
		if let Some(log) = log {
			let needs_prompt = log.cel.cel_context.with_llm_request(&llm_info);
			if needs_prompt {
				log.cel.cel_context.with_llm_prompt(req.get_messages())
			}
		}

		let request_model = llm_info.request_model.as_str();
		let new_request = if original_format == InputFormat::CountTokens {
			// Currently only bedrock is supported so no problems here.
			req.to_bedrock_token_count(parts.headers())?
		} else {
			match self {
				AIProvider::Vertex(provider) if provider.is_anthropic_model(Some(request_model)) => {
					let body = req.to_anthropic()?;
					provider.prepare_anthropic_request_body(body)?
				},
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::Vertex(_)
				| AIProvider::AzureOpenAI(_) => req.to_openai()?,
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
		if req.streaming {
			return self
				.process_streaming(req, rate_limit, log, include_completion_in_log, resp)
				.await;
		}
		// Buffer the body
		let buffer_limit = http::response_buffer_limit(&resp);
		let (mut parts, body) = resp.into_parts();
		let ce = parts.headers.typed_get::<ContentEncoding>();
		let Ok((encoding, bytes)) =
			http::compression::to_bytes_with_decompression(body, ce, buffer_limit).await
		else {
			return Err(AIError::ResponseTooLarge);
		};

		// count_tokens has simplified response handling (just format translation)
		if req.input_format == InputFormat::CountTokens {
			// Currently only bedrock is supported so we have no match needed here
			let (bytes, count) =
				conversion::bedrock::from_anthropic_token_count::translate_response(bytes.clone())?;

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
		// embeddings has simplified response handling (currently nothing; no translation needed)
		if req.input_format == InputFormat::Embeddings {
			let resp = Response::from_parts(parts, bytes.into());
			let llm_resp = LLMResponse::default();
			let llm_info = LLMInfo::new(req, llm_resp);
			log.store(Some(llm_info));
			return Ok(resp);
		}

		let (llm_resp, body) = if !parts.status.is_success() {
			let body = self.process_error(&req, &bytes)?;
			(LLMResponse::default(), body)
		} else {
			let mut resp = self.process_success(&req, &bytes)?;

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
		match (self, req.input_format) {
			// Completions with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Vertex(_),
				InputFormat::Completions,
			) => Ok(Box::new(
				serde_json::from_slice::<types::completions::Response>(bytes)
					.map_err(AIError::ResponseParsing)?,
			)),
			// Responses with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Vertex(_),
				InputFormat::Responses,
			) => Ok(Box::new(
				serde_json::from_slice::<types::responses::Response>(bytes)
					.map_err(AIError::ResponseParsing)?,
			)),
			// Anthropic messages: passthrough
			(AIProvider::Anthropic(_), InputFormat::Messages) => Ok(Box::new(
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

		Ok(match (self, input_format) {
			// Completions with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Vertex(_),
				InputFormat::Completions,
			) => conversion::completions::passthrough_stream(
				log,
				include_completion_in_log,
				rate_limit,
				resp,
			),
			// Responses with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Gemini(_)
				| AIProvider::AzureOpenAI(_)
				| AIProvider::Vertex(_),
				InputFormat::Responses,
			) => resp.map(|b| conversion::responses::passthrough_stream(b, buffer, log)),
			// Anthropic messages: passthrough
			(AIProvider::Anthropic(_), InputFormat::Messages) => {
				resp.map(|b| conversion::messages::passthrough_stream(b, buffer, log))
			},
			// Supported paths with conversion...
			(AIProvider::Anthropic(_), InputFormat::Completions) => {
				resp.map(|b| conversion::messages::from_completions::translate_stream(b, buffer, log))
			},
			(AIProvider::Bedrock(_), InputFormat::Completions) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_completions::translate_stream(b, buffer, log, &model, &msg)
				})
			},
			(AIProvider::Bedrock(_), InputFormat::Messages) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_messages::translate_stream(b, buffer, log, &model, &msg)
				})
			},
			(AIProvider::Bedrock(_), InputFormat::Responses) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_responses::translate_stream(b, buffer, log, &model, &msg)
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
		req: Request,
	) -> Result<(Parts, T), AIError> {
		// Buffer the body, max 2mb
		let (parts, body) = req.into_parts();
		let Ok(bytes) = axum::body::to_bytes(body, 2_097_152).await else {
			return Err(AIError::RequestTooLarge);
		};
		let mut req: T = if let Some(p) = policies {
			p.unmarshal_request(&bytes)?
		} else {
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?
		};

		if let Some(provider_model) = &self.override_model() {
			*req.model() = Some(provider_model.to_string());
		} else if req.model().is_none() {
			return Err(AIError::MissingField("model not specified".into()));
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
				InputFormat::Completions | InputFormat::Responses,
			) => {
				// Passthrough; nothing needed
				Ok(bytes.clone())
			},
			(AIProvider::Anthropic(_), InputFormat::Messages) => {
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
			(_, _) => Err(AIError::UnsupportedConversion(strng::literal!(
				"this provider and format is not supported"
			))),
		}
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
