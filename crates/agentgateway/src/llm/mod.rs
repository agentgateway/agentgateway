use std::str::FromStr;

use ::http::uri::{Authority, PathAndQuery};
use ::http::{HeaderValue, StatusCode, header};
use agent_core::prelude::Strng;
use agent_core::strng;
use axum_extra::headers::authorization::Bearer;
use headers::{Header, HeaderMapExt};
use itertools::Itertools;
pub use policy::Policy;
use serde_json::Value;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use crate::http::auth::BackendAuth;
use crate::http::backendtls::BackendTLS;
use crate::http::localratelimit::RateLimit;
use crate::http::{Body, Request, Response};
use crate::llm::universal::{
	ChatCompletionError, ChatCompletionErrorResponse, ChatCompletionResponse, Content, MessageRole,
	ToolCall,
};
use crate::proxy::ProxyError;
use crate::store::BackendPolicies;
use crate::telemetry::log::{AsyncLog, RequestLog};
use crate::types::agent::{BackendName, Target};
use crate::{client, *};

pub mod anthropic;
pub mod bedrock;
pub mod gemini;
pub mod openai;
mod pii;
mod policy;
#[cfg(test)]
mod tests;
pub mod vertex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AIBackend {
	pub provider: AIProvider,
	pub host_override: Option<Target>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
	pub input_tokens: u64,
	pub request_model: Strng,
	pub provider: Strng,
	pub streaming: bool,
}

#[derive(Debug, Clone)]
pub struct LLMResponse {
	pub request: LLMRequest,
	pub input_tokens_from_response: Option<u64>,
	pub output_tokens: Option<u64>,
	pub total_tokens: Option<u64>,
	pub provider_model: Option<Strng>,
	pub completion: Option<Vec<String>>,
}

#[derive(Debug)]
pub enum RequestResult {
	Success(Request, LLMRequest),
	Rejected(Response),
}

impl AIProvider {
	pub fn provider(&self) -> Strng {
		match self {
			AIProvider::OpenAI(p) => openai::Provider::NAME,
			AIProvider::Anthropic(p) => anthropic::Provider::NAME,
			AIProvider::Gemini(p) => gemini::Provider::NAME,
			AIProvider::Vertex(p) => vertex::Provider::NAME,
			AIProvider::Bedrock(p) => bedrock::Provider::NAME,
		}
	}
	pub fn default_connector(&self) -> (Target, BackendPolicies) {
		let btls = BackendPolicies {
			backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
			// We will use original request for now
			backend_auth: None,
			a2a: None,
			llm: None,
			inference_routing: None,
			llm_provider: Some((self.clone(), true)),
		};
		match self {
			AIProvider::OpenAI(_) => (Target::Hostname(openai::DEFAULT_HOST, 443), btls),
			AIProvider::Gemini(_) => (Target::Hostname(gemini::DEFAULT_HOST, 443), btls),
			AIProvider::Vertex(p) => {
				let bp = BackendPolicies {
					backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
					backend_auth: Some(BackendAuth::Gcp {}),
					a2a: None,
					llm: None,
					inference_routing: None,
					llm_provider: Some((self.clone(), true)),
				};
				(Target::Hostname(p.get_host(), 443), bp)
			},
			AIProvider::Anthropic(_) => (Target::Hostname(anthropic::DEFAULT_HOST, 443), btls),
			AIProvider::Bedrock(p) => {
				let bp = BackendPolicies {
					backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
					backend_auth: Some(BackendAuth::Aws {}),
					a2a: None,
					llm: None,
					inference_routing: None,
					llm_provider: Some((self.clone(), true)),
				};
				(Target::Hostname(p.get_host(), 443), bp)
			},
		}
	}
	pub fn setup_request(&self, req: &mut Request) -> anyhow::Result<()> {
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
		}
	}

	pub async fn process_request(
		&self,
		client: client::Client,
		policies: Option<&Policy>,
		req: Request,
		mut log: &mut Option<&mut RequestLog>,
	) -> (Result<RequestResult, AIError>) {
		// Buffer the body, max 2mb
		let (mut parts, body) = req.into_parts();
		let Ok(bytes) = axum::body::to_bytes(body, 2_097_152).await else {
			return Err(AIError::RequestTooLarge);
		};
		let mut req: universal::ChatCompletionRequest =
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?;
		if req
			.messages
			.iter()
			.any(|m| !matches!(m.content, universal::Content::Text(_)))
		{
			return Err(AIError::UnsupportedContent);
		};
		if let Some(p) = policies {
			let http_headers = &parts.headers;
			if let Some(dr) = p.apply(client, &mut req, http_headers).await.map_err(|e| {
				warn!("failed to call prompt guard webhook: {e}");
				AIError::PromptWebhookError
			})? {
				return Ok(RequestResult::Rejected(dr));
			}
		}
		let llm_info = self.to_llm_request(&req).await?;
		if let Some(log) = log {
			let needs_prompt = log.cel.cel_context.with_llm_request(&llm_info);
			if needs_prompt {
				log
					.cel
					.cel_context
					.with_llm_prompt(req.messages.iter().map(Into::into).collect_vec())
			}
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
		rate_limit: Vec<http::localratelimit::RateLimit>,
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
		let Ok(bytes) = axum::body::to_bytes(body, 2_097_152).await else {
			return Err(AIError::RequestTooLarge);
		};
		// 3 cases: success, error properly handled, and unexpected error we need to synthesize
		let openai_response = self
			.process_response_status(parts.status, &bytes)
			.await
			.unwrap_or_else(|err| {
				Err(ChatCompletionErrorResponse {
					event_id: None,
					error: ChatCompletionError {
						// Assume its due to the request being invalid, though we don't really know for sure
						r#type: "invalid_request_error".to_string(),
						message: format!(
							"failed to process response body: {}",
							std::str::from_utf8(&bytes).unwrap_or("invalid utf8")
						),
						param: None,
						code: None,
						event_id: None,
					},
				})
			});
		let (llm_resp, body) = match openai_response {
			Ok(success) => {
				let llm_resp = LLMResponse {
					request: req,
					input_tokens_from_response: Some(success.usage.prompt_tokens as u64),
					output_tokens: Some(success.usage.completion_tokens as u64),
					total_tokens: Some(success.usage.total_tokens as u64),
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
				};
				let body = Body::from(serde_json::to_vec(&success).map_err(AIError::ResponseMarshal)?);
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
				};
				let body = Body::from(serde_json::to_vec(&err).map_err(AIError::ResponseMarshal)?);
				(llm_resp, body)
			},
		};
		parts.headers.remove(header::CONTENT_LENGTH);
		let resp = Response::from_parts(parts, body);

		// In the initial request, we subtracted the approximate request tokens.
		// Now we should have the real request tokens and the response tokens
		amend_tokens(&rate_limit, &llm_resp);
		log.store(Some(llm_resp));
		Ok(resp)
	}

	async fn process_response_status(
		&self,
		status: StatusCode,
		bytes: &Bytes,
	) -> Result<Result<ChatCompletionResponse, ChatCompletionErrorResponse>, AIError> {
		if status.is_success() {
			let openai_response = match self {
				AIProvider::OpenAI(p) => p.process_response(bytes).await?,
				AIProvider::Gemini(p) => p.process_response(bytes).await?,
				AIProvider::Vertex(p) => p.process_response(bytes).await?,
				AIProvider::Anthropic(p) => p.process_response(bytes).await?,
				AIProvider::Bedrock(p) => p.process_response(bytes).await?,
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
		rate_limit: Vec<http::localratelimit::RateLimit>,
		log: AsyncLog<llm::LLMResponse>,
		include_completion_in_log: bool,
		resp: Response,
	) -> Result<Response, AIError> {
		// Store an empty response, as we stream in info we will parse into it
		let mut llmresp = llm::LLMResponse {
			request: req,
			input_tokens_from_response: Default::default(),
			output_tokens: Default::default(),
			total_tokens: Default::default(),
			provider_model: Default::default(),
			completion: Default::default(),
		};
		log.store(Some(llmresp));
		let resp = match self {
			AIProvider::Anthropic(p) => p.process_streaming(log, resp).await,
			AIProvider::Bedrock(p) => return Err(AIError::StreamingUnsupported),
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
		rate_limit: Vec<http::localratelimit::RateLimit>,
		resp: Response,
	) -> Response {
		let mut completion = if include_completion_in_log {
			Some(String::new())
		} else {
			None
		};
		resp.map(|b| {
			let mut seen_provider = false;
			parse::sse::json_passthrough::<universal::ChatCompletionStreamResponse>(b, move |f| {
				match f {
					Some(Ok(f)) => {
						if let Some(c) = completion.as_mut() {
							if let Some(delta) = f.choices.first().and_then(|c| c.delta.content.as_deref()) {
								c.push_str(delta);
							}
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

								amend_tokens(rate_limit.as_slice(), r);
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
		req: &universal::ChatCompletionRequest,
	) -> Result<LLMRequest, AIError> {
		let req2 = req.clone(); // TODO: avoid clone, we need it for spawn_blocking though
		let tokens = tokio::task::spawn_blocking(move || {
			let res = num_tokens_from_messages(&req2.model, &req2.messages)?;
			Ok::<_, AIError>(res)
		})
		.await??;
		// Pass the original body through
		let llm = LLMRequest {
			input_tokens: tokens,
			request_model: req.model.as_str().into(),
			provider: self.provider(),
			streaming: req.stream.unwrap_or_default(),
		};
		Ok(llm)
	}
}

// TODO: do we always want to spend cost of tokenizing, or just allow skipping and using the response?
fn num_tokens_from_messages(
	model: &str,
	messages: &[universal::ChatCompletionMessage],
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
		num_tokens += bpe
			.encode_with_special_tokens(
				// We filter non-text previously
				message.content.must_as_text(),
			)
			.len() as u64;
		if let Some(name) = message.name.as_ref() {
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
	#[error("error computing tokens")]
	JoinError(#[from] tokio::task::JoinError),
}

fn amend_tokens(rate_limit: &[RateLimit], llm_resp: &LLMResponse) {
	for lrl in rate_limit {
		let base = llm_resp.request.input_tokens;
		let input_mismatch = llm_resp
			.input_tokens_from_response
			.map(|real| (real as i64) - (base as i64))
			.unwrap_or_default();
		let response = llm_resp.output_tokens.unwrap_or_default();
		let tokens_to_remove = input_mismatch + (response as i64);
		lrl.amend_tokens(tokens_to_remove)
	}
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct SimpleChatCompletionMessage {
	pub role: Strng,
	pub content: Strng,
}

impl From<&universal::ChatCompletionMessage> for SimpleChatCompletionMessage {
	fn from(msg: &universal::ChatCompletionMessage) -> Self {
		Self {
			role: match msg.role {
				MessageRole::user => strng::literal!("user"),
				MessageRole::system => strng::literal!("system"),
				MessageRole::assistant => strng::literal!("assistant"),
				MessageRole::function => strng::literal!("function"),
				MessageRole::tool => strng::literal!("tool"),
			},
			content: match &msg.content {
				Content::Text(t) => t.into(),
				Content::ImageUrl(t) => {
					strng::literal!("image")
				},
			},
		}
	}
}

mod universal {
	use std::collections::HashMap;
	use std::fmt;

	use serde::de::{self, MapAccess, SeqAccess, Visitor};
	use serde::ser::SerializeMap;
	use serde::{Deserialize, Deserializer, Serialize, Serializer};
	use serde_json::Value;

	#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub enum ToolChoiceType {
		None,
		Auto,
		Required,
		ToolChoice { tool: Tool },
	}

	#[derive(Debug, Serialize, Deserialize, Clone)]
	pub struct ChatCompletionRequest {
		pub model: String,
		pub messages: Vec<ChatCompletionMessage>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub temperature: Option<f64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub n: Option<i64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub response_format: Option<Value>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stream: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stream_options: Option<ChatCompletionStreamOptions>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stop: Option<Vec<String>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub max_tokens: Option<i64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub presence_penalty: Option<f64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub frequency_penalty: Option<f64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub logit_bias: Option<HashMap<String, i32>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub user: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub seed: Option<i64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tools: Option<Vec<Tool>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub parallel_tool_calls: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[serde(serialize_with = "serialize_tool_choice")]
		pub tool_choice: Option<ToolChoiceType>,
	}

	#[derive(Debug, Serialize, Deserialize, Clone)]
	pub struct ChatCompletionStreamOptions {
		pub include_usage: bool,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	#[allow(non_camel_case_types)]
	pub enum MessageRole {
		user,
		system,
		assistant,
		function,
		tool,
	}

	#[derive(Debug, Clone, PartialEq, Eq)]
	pub enum Content {
		Text(String),
		ImageUrl(Vec<ImageUrl>),
	}

	impl Content {
		pub fn must_as_text(&self) -> &str {
			match self {
				Content::Text(txt) => txt,
				Content::ImageUrl(_) => {
					panic!("expected text")
				},
			}
		}
	}

	impl serde::Serialize for Content {
		fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
		where
			S: serde::Serializer,
		{
			match *self {
				Content::Text(ref text) => {
					if text.is_empty() {
						serializer.serialize_none()
					} else {
						serializer.serialize_str(text)
					}
				},
				Content::ImageUrl(ref image_url) => image_url.serialize(serializer),
			}
		}
	}

	impl<'de> Deserialize<'de> for Content {
		fn deserialize<D>(deserializer: D) -> Result<Content, D::Error>
		where
			D: Deserializer<'de>,
		{
			struct ContentVisitor;

			impl<'de> Visitor<'de> for ContentVisitor {
				type Value = Content;

				fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
					formatter.write_str("a valid content type")
				}

				fn visit_str<E>(self, value: &str) -> Result<Content, E>
				where
					E: de::Error,
				{
					Ok(Content::Text(value.to_string()))
				}

				fn visit_seq<A>(self, seq: A) -> Result<Content, A::Error>
				where
					A: SeqAccess<'de>,
				{
					let image_urls: Vec<ImageUrl> =
						Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
					Ok(Content::ImageUrl(image_urls))
				}

				fn visit_map<M>(self, map: M) -> Result<Content, M::Error>
				where
					M: MapAccess<'de>,
				{
					let image_urls: Vec<ImageUrl> =
						Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?;
					Ok(Content::ImageUrl(image_urls))
				}

				fn visit_none<E>(self) -> Result<Self::Value, E>
				where
					E: de::Error,
				{
					Ok(Content::Text(String::new()))
				}

				fn visit_unit<E>(self) -> Result<Self::Value, E>
				where
					E: de::Error,
				{
					Ok(Content::Text(String::new()))
				}
			}

			deserializer.deserialize_any(ContentVisitor)
		}
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	#[allow(non_camel_case_types)]
	pub enum ContentType {
		text,
		image_url,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	#[allow(non_camel_case_types)]
	pub struct ImageUrlType {
		pub url: String,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	#[allow(non_camel_case_types)]
	pub struct ImageUrl {
		pub r#type: ContentType,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub text: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub image_url: Option<ImageUrlType>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ChatCompletionMessage {
		pub role: MessageRole,
		pub content: Content,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub name: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_calls: Option<Vec<ToolCall>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_call_id: Option<String>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ChatCompletionMessageForResponse {
		pub role: MessageRole,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub content: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub reasoning_content: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub name: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_calls: Option<Vec<ToolCall>>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ChatCompletionMessageForResponseDelta {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub role: Option<MessageRole>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub content: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub refusal: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub name: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_calls: Option<Vec<ToolCall>>,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionChoice {
		pub index: i64,
		pub message: ChatCompletionMessageForResponse,
		pub finish_reason: Option<FinishReason>,
		pub finish_details: Option<FinishDetails>,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionChoiceStream {
		pub index: i64,
		pub delta: ChatCompletionMessageForResponseDelta,
		pub finish_reason: Option<FinishReason>,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionResponse {
		pub id: Option<String>,
		pub object: String,
		pub created: i64,
		pub model: String,
		pub choices: Vec<ChatCompletionChoice>,
		pub usage: Usage,
		pub system_fingerprint: Option<String>,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionStreamResponse {
		pub id: Option<String>,
		pub object: String,
		pub created: i64,
		pub model: String,
		pub choices: Vec<ChatCompletionChoiceStream>,
		pub usage: Option<Usage>,
		pub system_fingerprint: Option<String>,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionErrorResponse {
		pub event_id: Option<String>,
		pub error: ChatCompletionError,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionError {
		pub r#type: String,
		pub message: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub param: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub code: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub event_id: Option<String>,
	}

	#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
	#[allow(non_camel_case_types)]
	pub enum FinishReason {
		stop,
		length,
		content_filter,
		tool_calls,
		null,
	}

	#[derive(Debug, Deserialize, Serialize)]
	#[allow(non_camel_case_types)]
	pub struct FinishDetails {
		pub r#type: FinishReason,
		pub stop: String,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ToolCall {
		pub id: String,
		pub r#type: String,
		pub function: ToolCallFunction,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ToolCallFunction {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub name: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub arguments: Option<String>,
	}

	fn serialize_tool_choice<S>(
		value: &Option<ToolChoiceType>,
		serializer: S,
	) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match value {
			Some(ToolChoiceType::None) => serializer.serialize_str("none"),
			Some(ToolChoiceType::Auto) => serializer.serialize_str("auto"),
			Some(ToolChoiceType::Required) => serializer.serialize_str("required"),
			Some(ToolChoiceType::ToolChoice { tool }) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("type", &tool.r#type)?;
				map.serialize_entry("function", &tool.function)?;
				map.end()
			},
			None => serializer.serialize_none(),
		}
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	pub struct Tool {
		pub r#type: ToolType,
		pub function: Function,
	}

	#[derive(Debug, Deserialize, Serialize, Copy, Clone, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub enum ToolType {
		Function,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct Usage {
		pub prompt_tokens: i32,
		pub completion_tokens: i32,
		pub total_tokens: i32,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	pub struct Function {
		pub name: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub description: Option<String>,
		pub parameters: FunctionParameters,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	pub struct FunctionParameters {
		#[serde(rename = "type")]
		pub schema_type: JSONSchemaType,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub properties: Option<HashMap<String, Box<JSONSchemaDefine>>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub required: Option<Vec<String>>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
	#[serde(rename_all = "lowercase")]
	pub enum JSONSchemaType {
		Object,
		Number,
		String,
		Array,
		Null,
		Boolean,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
	pub struct JSONSchemaDefine {
		#[serde(rename = "type")]
		pub schema_type: Option<JSONSchemaType>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub description: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub enum_values: Option<Vec<String>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub properties: Option<HashMap<String, Box<JSONSchemaDefine>>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub required: Option<Vec<String>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub items: Option<Box<JSONSchemaDefine>>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	#[serde(tag = "type")]
	#[serde(rename_all = "snake_case")]
	pub enum Tools {
		CodeInterpreter,
		FileSearch(ToolsFileSearch),
		Function(ToolsFunction),
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ToolsFileSearch {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub file_search: Option<ToolsFileSearchObject>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ToolsFunction {
		pub function: Function,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct ToolsFileSearchObject {
		pub max_num_results: Option<u8>,
		pub ranking_options: Option<FileSearchRankingOptions>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct FileSearchRankingOptions {
		pub ranker: Option<FileSearchRanker>,
		pub score_threshold: Option<f32>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub enum FileSearchRanker {
		#[serde(rename = "auto")]
		Auto,
		#[serde(rename = "default_2024_08_21")]
		Default2024_08_21,
	}
}
