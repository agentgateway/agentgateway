use std::str::FromStr;

use ::http::uri::{Authority, PathAndQuery};
use ::http::{HeaderValue, StatusCode, header};
use agent_core::prelude::Strng;
use agent_core::strng;
use axum_extra::headers::authorization::Bearer;
use headers::{ContentEncoding, HeaderMapExt};
use itertools::Itertools;
pub use policy::Policy;
use rand::Rng;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use crate::http::auth::{AwsAuth, BackendAuth};
use crate::http::jwt::Claims;
use crate::http::{Body, Request, Response};
use crate::llm::universal::{ChatCompletionError, ChatCompletionErrorResponse};
use crate::store::{BackendPolicies, LLMResponsePolicies};
use crate::telemetry::log::{AsyncLog, RequestLog};
use crate::types::agent::Target;
use crate::types::loadbalancer::{ActiveHandle, EndpointWithInfo};
use crate::{client, *};

pub mod anthropic;
pub mod bedrock;
pub mod gemini;
pub mod openai;
mod pii;
pub mod policy;
#[cfg(test)]
mod tests;
pub mod universal;
pub mod vertex;

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

#[apply(schema!)]
pub struct NamedAIProvider {
	pub name: Strng,
	pub provider: AIProvider,
	pub host_override: Option<Target>,
	/// Whether to tokenize on the request flow. This enables us to do more accurate rate limits,
	/// since we know (part of) the cost of the request upfront.
	/// This comes with the cost of an expensive operation.
	#[serde(default)]
	pub tokenize: bool,
}

#[apply(schema!)]
pub enum AIProvider {
	OpenAI(openai::Provider),
	Gemini(gemini::Provider),
	Vertex(vertex::Provider),
	Anthropic(anthropic::Provider),
	Bedrock(bedrock::Provider),
}

trait Provider {
	const NAME: Strng;
}

#[derive(Debug, Clone)]
pub struct LLMRequest {
	/// Input tokens derived by tokenizing the request. Not always enabled
	pub input_tokens: Option<u64>,
	pub request_model: Strng,
	pub provider: Strng,
	pub streaming: bool,
	pub params: llm::LLMRequestParams,
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
}

#[derive(Debug, Clone)]
pub struct LLMResponse {
	pub request: LLMRequest,
	pub input_tokens_from_response: Option<u64>,
	pub output_tokens: Option<u64>,
	pub total_tokens: Option<u64>,
	pub provider_model: Option<Strng>,
	pub completion: Option<Vec<String>>,
	// Time to get the first token. Only used for streaming.
	pub first_token: Option<Instant>,
}

impl LLMResponse {
	pub fn input_tokens(&self) -> Option<u64> {
		self
			.input_tokens_from_response
			.or(self.request.input_tokens)
	}
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
		}
	}
	pub fn default_connector(&self) -> (Target, BackendPolicies) {
		let btls = BackendPolicies {
			backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
			// We will use original request for now
			backend_auth: None,
			a2a: None,
			inference_routing: None,
			llm_provider: None,
		};
		match self {
			AIProvider::OpenAI(_) => (Target::Hostname(openai::DEFAULT_HOST, 443), btls),
			AIProvider::Gemini(_) => (Target::Hostname(gemini::DEFAULT_HOST, 443), btls),
			AIProvider::Vertex(p) => {
				let bp = BackendPolicies {
					backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
					backend_auth: Some(BackendAuth::Gcp {}),
					a2a: None,
					inference_routing: None,
					llm_provider: None,
				};
				(Target::Hostname(p.get_host(), 443), bp)
			},
			AIProvider::Anthropic(_) => (Target::Hostname(anthropic::DEFAULT_HOST, 443), btls),
			AIProvider::Bedrock(p) => {
				let bp = BackendPolicies {
					backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
					backend_auth: Some(BackendAuth::Aws(AwsAuth::Implicit {})),
					a2a: None,
					inference_routing: None,
					llm_provider: None,
				};
				(Target::Hostname(p.get_host(), 443), bp)
			},
		}
	}
	pub fn setup_request(&self, req: &mut Request, llm_request: &LLMRequest) -> anyhow::Result<()> {
		match self {
			AIProvider::OpenAI(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					uri.path_and_query = Some(PathAndQuery::from_static(openai::DEFAULT_PATH));
					uri.authority = Some(Authority::from_static(openai::DEFAULT_HOST_STR));
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Anthropic(_) => {
				http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						uri.path_and_query = Some(PathAndQuery::from_static(anthropic::DEFAULT_PATH));
						uri.authority = Some(Authority::from_static(anthropic::DEFAULT_HOST_STR));
						Ok(())
					})?;
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
			AIProvider::Gemini(_) => http::modify_req(req, |req| {
				http::modify_uri(req, |uri| {
					uri.path_and_query = Some(PathAndQuery::from_static(gemini::DEFAULT_PATH));
					uri.authority = Some(Authority::from_static(gemini::DEFAULT_HOST_STR));
					Ok(())
				})?;
				Ok(())
			}),
			AIProvider::Vertex(provider) => {
				let path = provider.get_path_for_model();
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
				// For Bedrock, use a default model path - the actual model will be specified in the request body
				let path =
					provider.get_path_for_model(llm_request.streaming, llm_request.request_model.as_str());
				http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						uri.path_and_query = Some(PathAndQuery::from_str(&path)?);
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
		}
	}

	pub async fn process_request(
		&self,
		client: client::Client,
		policies: Option<&Policy>,
		req: Request,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		// Buffer the body, max 2mb
		let (mut parts, body) = req.into_parts();
		let Ok(bytes) = axum::body::to_bytes(body, 2_097_152).await else {
			return Err(AIError::RequestTooLarge);
		};
		let mut req: universal::Request = if let Some(p) = policies {
			p.unmarshal_request(&bytes)?
		} else {
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?
		};

		if let Some(p) = policies {
			p.apply_prompt_enrichment(&mut req);
			let http_headers = &parts.headers;
			let claims = parts.extensions.get::<Claims>().cloned();
			if let Some(dr) = p
				.apply_prompt_guard(client, &mut req, http_headers, claims)
				.await
				.map_err(|e| {
					warn!("failed to call prompt guard webhook: {e}");
					AIError::PromptWebhookError
				})? {
				return Ok(RequestResult::Rejected(dr));
			}
		}
		let llm_info = self.to_llm_request(&req, tokenize).await?;
		if let Some(log) = log {
			let needs_prompt = log.cel.cel_context.with_llm_request(&llm_info);
			if needs_prompt {
				log
					.cel
					.cel_context
					.with_llm_prompt(req.messages.iter().map(Into::into).collect_vec())
			}
		}

		// If a user doesn't request usage, we will not get token information which we need
		// We always set it.
		// TODO?: this may impact the user, if they make assumptions about the stream NOT including usage.
		// Notably, this adds a final SSE event.
		// We could actually go remove that on the response, but it would mean we cannot do passthrough-parsing,
		// so unless we have a compelling use case for it, for now we keep it.
		if req.stream.unwrap_or_default() && req.stream_options.is_none() {
			req.stream_options = Some(universal::StreamOptions {
				include_usage: true,
			});
		}
		let resp_json = match self {
			AIProvider::OpenAI(p) => serde_json::to_vec(&p.process_request(req).await?),
			AIProvider::Gemini(p) => serde_json::to_vec(&p.process_request(req).await?),
			AIProvider::Vertex(p) => serde_json::to_vec(&p.process_request(req).await?),
			AIProvider::Anthropic(p) => serde_json::to_vec(&p.process_request(req).await?),
			AIProvider::Bedrock(p) => serde_json::to_vec(&p.process_request(req).await?),
		};
		let body = resp_json.map_err(AIError::RequestMarshal)?;
		let resp = Body::from(body);
		parts.headers.remove(header::CONTENT_LENGTH);
		let req = Request::from_parts(parts, resp);
		Ok(RequestResult::Success(req, llm_info))
	}

	pub async fn process_response(
		&self,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		log: AsyncLog<llm::LLMResponse>,
		include_completion_in_log: bool,
		resp: Response,
	) -> Result<Response, AIError> {
		if req.streaming {
			return self
				.process_streaming(req, rate_limit, log, include_completion_in_log, resp)
				.await;
		}
		// Buffer the body, max 2mb
		let (mut parts, body) = resp.into_parts();
		let ce = parts.headers.typed_get::<ContentEncoding>();
		let Ok((encoding, bytes)) =
			http::compression::to_bytes_with_decompression(body, ce, 2_097_152).await
		else {
			return Err(AIError::RequestTooLarge);
		};
		// 3 cases: success, error properly handled, and unexpected error we need to synthesize
		let openai_response = self
			.process_response_status(&req, parts.status, &bytes)
			.await
			.unwrap_or_else(|err| {
				Err(ChatCompletionErrorResponse {
					event_id: None,
					error: ChatCompletionError {
						// Assume its due to the request being invalid, though we don't really know for sure
						r#type: "invalid_request_error".to_string(),
						message: format!(
							"failed to process response body ({err}): {}",
							std::str::from_utf8(&bytes).unwrap_or("invalid utf8")
						),
						param: None,
						code: None,
						event_id: None,
					},
				})
			});
		let (llm_resp, body) = match openai_response {
			Ok(mut success) => {
				let llm_resp = LLMResponse {
					request: req,
					input_tokens_from_response: success.usage.as_ref().map(|u| u.prompt_tokens as u64),
					output_tokens: success.usage.as_ref().map(|u| u.completion_tokens as u64),
					total_tokens: success.usage.as_ref().map(|u| u.total_tokens as u64),
					provider_model: Some(strng::new(&success.model)),
					completion: if include_completion_in_log {
						Some(
							success
								.choices
								.iter()
								.flat_map(|c| c.message.content.clone())
								.collect_vec(),
						)
					} else {
						None
					},
					first_token: Default::default(),
				};
				// Apply response prompt guard
				if let Some(dr) =
					Policy::apply_response_prompt_guard(&mut success, &rate_limit.prompt_guard)
						.await
						.map_err(|e| {
							warn!("failed to apply response prompt guard: {e}");
							AIError::PromptWebhookError
						})? {
					return Ok(dr);
				}

				let body = serde_json::to_vec(&success).map_err(AIError::ResponseMarshal)?;
				(llm_resp, body)
			},
			Err(err) => {
				let llm_resp = LLMResponse {
					request: req,
					input_tokens_from_response: None,
					output_tokens: None,
					total_tokens: None,
					provider_model: None,
					completion: None,
					first_token: None,
				};
				let body = serde_json::to_vec(&err).map_err(AIError::ResponseMarshal)?;
				(llm_resp, body)
			},
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

		// In the initial request, we subtracted the approximate request tokens.
		// Now we should have the real request tokens and the response tokens
		amend_tokens(rate_limit, &llm_resp);
		log.store(Some(llm_resp));
		Ok(resp)
	}

	async fn process_response_status(
		&self,
		req: &LLMRequest,
		status: StatusCode,
		bytes: &Bytes,
	) -> Result<Result<universal::Response, ChatCompletionErrorResponse>, AIError> {
		if status.is_success() {
			let openai_response = match self {
				AIProvider::OpenAI(p) => p.process_response(bytes).await?,
				AIProvider::Gemini(p) => p.process_response(bytes).await?,
				AIProvider::Vertex(p) => p.process_response(bytes).await?,
				AIProvider::Anthropic(p) => p.process_response(bytes).await?,
				AIProvider::Bedrock(p) => {
					p.process_response(req.request_model.as_str(), bytes)
						.await?
				},
			};
			Ok(Ok(openai_response))
		} else {
			let openai_response = match self {
				AIProvider::OpenAI(p) => p.process_error(bytes).await?,
				AIProvider::Gemini(p) => p.process_error(bytes).await?,
				AIProvider::Vertex(p) => p.process_error(bytes).await?,
				AIProvider::Anthropic(p) => p.process_error(bytes).await?,
				AIProvider::Bedrock(p) => p.process_error(bytes).await?,
			};
			Ok(Err(openai_response))
		}
	}

	pub async fn process_streaming(
		&self,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		log: AsyncLog<llm::LLMResponse>,
		include_completion_in_log: bool,
		resp: Response,
	) -> Result<Response, AIError> {
		let model = req.request_model.clone();
		// Store an empty response, as we stream in info we will parse into it
		let llmresp = llm::LLMResponse {
			request: req,
			input_tokens_from_response: Default::default(),
			output_tokens: Default::default(),
			total_tokens: Default::default(),
			provider_model: Default::default(),
			completion: Default::default(),
			first_token: Default::default(),
		};
		log.store(Some(llmresp));
		let resp = match self {
			AIProvider::Anthropic(p) => p.process_streaming(log, resp).await,
			AIProvider::Bedrock(p) => p.process_streaming(log, resp, model.as_str()).await,
			_ => {
				self
					.default_process_streaming(log, include_completion_in_log, rate_limit, resp)
					.await
			},
		};
		Ok(resp)
	}

	async fn default_process_streaming(
		&self,
		log: AsyncLog<llm::LLMResponse>,
		include_completion_in_log: bool,
		rate_limit: LLMResponsePolicies,
		resp: Response,
	) -> Response {
		let mut completion = if include_completion_in_log {
			Some(String::new())
		} else {
			None
		};
		resp.map(|b| {
			let mut seen_provider = false;
			let mut saw_token = false;
			let mut rate_limit = Some(rate_limit);
			parse::sse::json_passthrough::<universal::StreamResponse>(b, move |f| {
				match f {
					Some(Ok(f)) => {
						if let Some(c) = completion.as_mut()
							&& let Some(delta) = f.choices.first().and_then(|c| c.delta.content.as_deref())
						{
							c.push_str(delta);
						}
						if !saw_token {
							saw_token = true;
							log.non_atomic_mutate(|r| {
								r.first_token = Some(Instant::now());
							});
						}
						if !seen_provider {
							seen_provider = true;
							log.non_atomic_mutate(|r| r.provider_model = Some(strng::new(&f.model)));
						}
						if let Some(u) = f.usage {
							log.non_atomic_mutate(|r| {
								r.input_tokens_from_response = Some(u.prompt_tokens as u64);
								r.output_tokens = Some(u.completion_tokens as u64);
								r.total_tokens = Some(u.total_tokens as u64);
								if let Some(c) = completion.take() {
									r.completion = Some(vec![c]);
								}

								if let Some(rl) = rate_limit.take() {
									amend_tokens(rl, r);
								}
							});
						}
					},
					Some(Err(e)) => {
						debug!("failed to parse streaming response: {e}");
					},
					None => {
						// We are done, try to set completion if we haven't already
						// This is useful in case we never see "usage"
						log.non_atomic_mutate(|r| {
							if let Some(c) = completion.take() {
								r.completion = Some(vec![c]);
							}
						});
					},
				}
			})
		})
	}

	pub async fn to_llm_request(
		&self,
		req: &universal::Request,
		tokenize: bool,
	) -> Result<LLMRequest, AIError> {
		let input_tokens = if tokenize {
			// TODO: avoid clone, we need it for spawn_blocking though
			let msg = req.clone().messages.clone();
			let model = req.clone().model.clone();
			let tokens = tokio::task::spawn_blocking(move || {
				let res = num_tokens_from_messages(&model, &msg)?;
				Ok::<_, AIError>(res)
			})
			.await??;
			Some(tokens)
		} else {
			None
		};
		// Pass the original body through
		let llm = LLMRequest {
			input_tokens,
			request_model: req.model.as_str().into(),
			provider: self.provider(),
			streaming: req.stream.unwrap_or_default(),
			params: LLMRequestParams {
				temperature: req.temperature.map(Into::into),
				top_p: req.top_p.map(Into::into),
				frequency_penalty: req.frequency_penalty.map(Into::into),
				presence_penalty: req.presence_penalty.map(Into::into),
				seed: req.seed,
				max_tokens: universal::max_tokens_option(req),
			},
		};
		Ok(llm)
	}
}

// TODO: do we always want to spend cost of tokenizing, or just allow skipping and using the response?
fn num_tokens_from_messages(
	model: &str,
	messages: &[universal::RequestMessage],
) -> Result<u64, AIError> {
	let tokenizer = get_tokenizer(model).unwrap_or(Tokenizer::Cl100kBase);
	if tokenizer != Tokenizer::Cl100kBase && tokenizer != Tokenizer::O200kBase {
		// Chat completion is only supported chat models
		return Err(AIError::UnsupportedModel);
	}
	let bpe = get_bpe_from_tokenizer(tokenizer);

	let (tokens_per_message, tokens_per_name) = (3, 1);

	let mut num_tokens: u64 = 0;
	for message in messages {
		num_tokens += tokens_per_message;
		// Role is always 1 token
		num_tokens += 1;
		// num_tokens += bpe
		// .encode_with_special_tokens(
		// 	message.role
		// )
		// .len() as u64;
		if let Some(t) = universal::message_text(message) {
			num_tokens += bpe
				.encode_with_special_tokens(
					// We filter non-text previously
					t,
				)
				.len() as u64;
		}
		if let Some(name) = universal::message_name(message) {
			num_tokens += bpe.encode_with_special_tokens(name).len() as u64;
			num_tokens += tokens_per_name;
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
	#[error("request was too large")]
	RequestTooLarge,
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

fn amend_tokens(rate_limit: store::LLMResponsePolicies, llm_resp: &LLMResponse) {
	let input_mismatch = match (
		llm_resp.request.input_tokens,
		llm_resp.input_tokens_from_response,
	) {
		// Already counted 'req'
		(Some(req), Some(resp)) => (resp as i64) - (req as i64),
		// No request or response count... this is probably an issue.
		(_, None) => 0,
		// No request counted, so count the full response
		(_, Some(resp)) => resp as i64,
	};
	let response = llm_resp.output_tokens.unwrap_or_default();
	let tokens_to_remove = input_mismatch + (response as i64);

	for lrl in &rate_limit.local_rate_limit {
		lrl.amend_tokens(tokens_to_remove)
	}
	if let Some(rrl) = rate_limit.remote_rate_limit {
		rrl.amend_tokens(tokens_to_remove)
	}
}

#[apply(schema!)]
pub struct SimpleChatCompletionMessage {
	pub role: Strng,
	pub content: Strng,
}

impl From<&universal::RequestMessage> for SimpleChatCompletionMessage {
	fn from(msg: &universal::RequestMessage) -> Self {
		let role = universal::message_role(msg);
		let content = universal::message_text(msg).unwrap_or_default();
		Self {
			role: role.into(),
			content: content.into(),
		}
	}
}
