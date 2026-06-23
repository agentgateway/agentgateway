use std::str::FromStr;
use std::sync::Arc;

use ::http::request::Parts;
use ::http::uri::{Authority, PathAndQuery};
use ::http::{HeaderMap, HeaderName, HeaderValue, header};
use agent_core::prelude::Strng;
use agent_core::strng;
use axum_extra::headers::authorization::Bearer;
use headers::{ContentEncoding, HeaderMapExt};
pub use policy::Policy;
use rand::RngExt;
use serde::de::DeserializeOwned;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use crate::http::auth::{AppliedBackendAuthLocation, AwsAuth, AzureAuth, BackendAuth, GcpAuth};
use crate::http::jwt::Claims;
use crate::http::{Body, Request, Response};
pub use crate::llm::types::{RequestType, ResponseType};
use crate::proxy::httpproxy::PolicyClient;
use crate::store::{BackendPolicies, LLMResponsePolicies};
use crate::telemetry::log::{AsyncLog, RequestLog};
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference, Target};
use crate::types::loadbalancer::{ActiveHandle, EndpointWithInfo};
use crate::*;

pub mod anthropic;
pub mod azure;
pub mod bedrock;
pub mod copilot;
pub mod custom;
pub mod gemini;
pub mod model_router;
pub mod openai;
pub mod vertex;

mod conversion;
pub mod cost;
pub mod policy;
mod types;

use policy::streaming_guardrails::GuardedSseBody;
pub use types::SimpleChatCompletionMessage;

use crate::cel::{Executor, LLMContext, RequestSnapshot};
use crate::proxy::dtrace;
use crate::store;

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

#[apply(schema!)]
pub enum AIProvider {
	OpenAI(openai::Provider),
	Gemini(gemini::Provider),
	Vertex(vertex::Provider),
	Anthropic(anthropic::Provider),
	Bedrock(bedrock::Provider),
	Azure(azure::Provider),
	Copilot(copilot::Provider),
	Custom(custom::Provider),
}

#[apply(schema!)]
pub enum LocalModelAIProvider {
	OpenAI,
	Gemini,
	Vertex,
	Anthropic,
	Bedrock,
	Azure,
	Copilot,
	Custom(custom::Provider),
}

trait Provider {
	const NAME: Strng;
}

#[derive(Debug, Clone)]
pub struct LLMRequest {
	/// Input tokens derived by tokenizing the request. Not always enabled
	pub input_tokens: Option<u64>,
	pub input_format: InputFormat,
	pub native_format: Option<custom::ProviderFormat>,
	pub cache_convention: CacheTokenConvention,
	pub request_model: Strng,
	pub provider: Strng,
	pub streaming: bool,
	pub params: LLMRequestParams,
	pub prompt: Option<Arc<Vec<SimpleChatCompletionMessage>>>,
}

/// Whether an upstream's reported `input_tokens` already includes cached tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheTokenConvention {
	/// OpenAI-style: cached tokens are a subset of `input_tokens`
	#[default]
	InputIncludesCache,
	/// Anthropic-style: `input_tokens` is already fresh
	InputExcludesCache,
}

impl CacheTokenConvention {
	/// Placeholder used while request conversion lacks provider/native-format
	/// context. `process_request` replaces this with the classified convention.
	pub(crate) fn pending() -> Self {
		Self::InputIncludesCache
	}
}

/// Classify how the upstream reports cached tokens, from the source wire format the
/// gateway is about to parse — not the provider name, which can carry another
/// provider's native semantics (e.g. Vertex serving Anthropic models).
fn cache_convention_for(
	provider: &AIProvider,
	native_format: Option<custom::ProviderFormat>,
	request_model: &str,
) -> CacheTokenConvention {
	use CacheTokenConvention::*;
	use custom::ProviderFormat::{AnthropicTokenCount, Messages};
	match provider {
		AIProvider::Anthropic(_) | AIProvider::Bedrock(_) => InputExcludesCache,
		AIProvider::Vertex(p) if p.is_anthropic_model(Some(request_model)) => InputExcludesCache,
		AIProvider::Custom(_) => match native_format {
			Some(Messages | AnthropicTokenCount) => InputExcludesCache,
			// TODO(mk): Detect/passthrough mode has native_format = None. Need to confirm if there is an issue with classification
			_ => InputIncludesCache,
		},
		_ => InputIncludesCache, // openai, azure, copilot, gemini, vertex non-anthropic
	}
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

	fn provider_format(&self) -> Option<custom::ProviderFormat> {
		match self {
			InputFormat::Completions => Some(custom::ProviderFormat::Completions),
			InputFormat::Messages => Some(custom::ProviderFormat::Messages),
			InputFormat::Responses => Some(custom::ProviderFormat::Responses),
			InputFormat::Embeddings => Some(custom::ProviderFormat::Embeddings),
			InputFormat::Realtime => Some(custom::ProviderFormat::Realtime),
			InputFormat::CountTokens => Some(custom::ProviderFormat::AnthropicTokenCount),
			InputFormat::Detect => None,
			InputFormat::Rerank => Some(custom::ProviderFormat::Rerank),
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct LLMResponse {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_image_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_text_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_audio_tokens: Option<u64>,
	/// count_tokens contains the number of tokens in the request, when using the token counting endpoint
	/// These are not counted as 'input tokens' since they do not consume input tokens.
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
			AIProvider::Azure(_p) => azure::Provider::NAME,
			AIProvider::Copilot(_p) => copilot::Provider::NAME,
			AIProvider::Custom(p) => p
				.provider_override
				.clone()
				.unwrap_or(custom::Provider::NAME),
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
				backend_auth: Some(BackendAuth::Copilot),
				..btls
			},
			AIProvider::Vertex(_) => BackendPolicies {
				backend_auth: Some(BackendAuth::Gcp(GcpAuth::default())),
				..btls
			},
			AIProvider::Bedrock(p) => BackendPolicies {
				backend_auth: Some(BackendAuth::Aws(AwsAuth::Implicit {
					service_name: None,
					assume_role: None,
					source_credentials_cache: p.source_credentials_cache.clone(),
					assume_role_cache: p.assume_role_cache.clone(),
				})),
				..btls
			},
			AIProvider::Azure(p) => BackendPolicies {
				backend_auth: Some(BackendAuth::Azure(AzureAuth::Implicit {
					cached_cred: p.cached_cred.clone(),
				})),
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
		self.set_required_fields(req)?;
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
				http::modify_req(req, |req| {
					http::modify_uri(req, |uri| {
						let path = provider.get_path_for_model(route_type, request_model, streaming);
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
					let Some(native_format) = llm_request.and_then(|l| l.native_format) else {
						return Ok(());
					};
					if let Some(path) = provider.path_for(native_format) {
						Self::set_path_and_query(uri, path)?;
						return Ok(());
					}
					let native_route_type = native_format.route_type();
					let path = match native_route_type {
						RouteType::Messages | RouteType::AnthropicTokenCount => format!(
							"{}{}",
							path_prefix.map_or(anthropic::DEFAULT_BASE_PATH, |prefix| {
								prefix.trim_end_matches('/')
							}),
							anthropic::path_suffix(native_route_type)
						),
						_ => format!(
							"{}{}",
							path_prefix.map_or(openai::DEFAULT_BASE_PATH, |prefix| {
								prefix.trim_end_matches('/')
							}),
							openai::path_suffix(native_route_type)
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

	pub fn set_required_fields(&self, req: &mut Request) -> anyhow::Result<()> {
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
		if matches!(self, AIProvider::OpenAI(_)) {
			req.normalize_openai_token_limit();
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
			.read_body_and_default_model::<types::messages::Request>(policies, req, log)
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
			.read_body_and_default_model::<types::embeddings::Request>(policies, req, log)
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

	pub async fn process_rerank_request(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		tokenize: bool,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		let (parts, req) = self
			.read_body_and_default_model::<types::rerank::Request>(policies, req, log)
			.await?;

		self
			.process_request(
				backend_info,
				policies,
				InputFormat::Rerank,
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
			.read_body_and_default_model::<types::responses::Request>(policies, req, log)
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
			.read_body_and_default_model::<types::count_tokens::Request>(policies, req, log)
			.await?;

		// Some Anthropic-compatible clients (e.g. Claude Code) always call
		// `/v1/messages/count_tokens`. For providers/models without a native
		// count-tokens endpoint, we must still answer this route, so we fall
		// back to local token estimation using the normalized messages payload.
		let use_local = match self {
			AIProvider::Anthropic(_) => false,
			AIProvider::Bedrock(p) => !p.is_anthropic_model(req.model.as_deref()),
			AIProvider::Vertex(p) => !p.is_anthropic_model(req.model.as_deref()),
			AIProvider::Custom(p) => !p.supports(custom::ProviderFormat::AnthropicTokenCount),
			_ => true,
		};
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
			if let Some(p) = policies {
				p.unmarshal_request(&bytes, log)
			} else {
				serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)
			}
			.unwrap_or_else(|_| types::detect::Request::new_raw(bytes))
		} else {
			types::detect::Request::new_raw(bytes)
		};

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
		let native_format = match self {
			AIProvider::Custom(_) if original_format == InputFormat::Detect => None,
			AIProvider::Custom(p) => p
				.native_format_for(original_format)
				.ok_or_else(|| {
					AIError::UnsupportedConversion(strng::format!(
						"from {original_format:?} to provider {}",
						self.provider()
					))
				})?
				.into(),
			_ => original_format.provider_format(),
		};
		match (self, original_format) {
			(_, InputFormat::Detect) => {
				// All providers support detect; this is a passthrough!
			},
			(AIProvider::Custom(_), _) => {},
			(_, InputFormat::Completions) => {
				// All providers support completions input
			},
			(
				AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Azure(_)
				| AIProvider::Bedrock(_)
				| AIProvider::Gemini(_),
				InputFormat::Responses,
			) => {
				// OpenAI supports responses input (Bedrock & Gemini support responses input via translation)
			},
			(
				AIProvider::Anthropic(_)
				| AIProvider::Bedrock(_)
				| AIProvider::Vertex(_)
				| AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Gemini(_)
				| AIProvider::Azure(_),
				InputFormat::Messages,
			) => {
				// Anthropic supports messages input (Bedrock & Vertex support assuming serving Anthropic models)
				// OpenAI/Gemini/Azure support messages via translation to chat completions
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
				| AIProvider::Azure(_)
				| AIProvider::Bedrock(_)
				| AIProvider::Vertex(_),
				InputFormat::Embeddings,
			) => {
				// OpenAI compatible, Gemini, Bedrock, or Vertex
			},
			(
				AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Azure(_)
				| AIProvider::Bedrock(_)
				| AIProvider::Vertex(_),
				InputFormat::Rerank,
			) => {
				// OpenAI-compatible providers (OpenAI/Copilot/Azure-Foundry) pass the
				// Cohere-shaped request through unchanged; Bedrock & Vertex translate.
				// Custom is already covered above. These OpenAI-compatible surfaces only
				// actually serve rerank when pointed (e.g. via hostOverride or Azure Foundry)
				// at a Cohere-compatible backend; otherwise the upstream returns its own 404.
				// Anthropic and Gemini have no rerank endpoint and fall through to the
				// unsupported arm below.
			},
			(p, m) => {
				// Unsupported provider/format combination.
				return Err(AIError::UnsupportedConversion(strng::format!(
					"from {m:?} to provider {}",
					p.provider()
				)));
			},
		}
		if let Some(p) = policies {
			// Apply model alias resolution
			if req.supports_model()
				&& let Some(model) = req.model()
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
		if original_format == InputFormat::Detect {
			types::detect::amend_request_info(&mut llm_info, parts.uri.path());
		}
		llm_info.native_format = native_format;
		llm_info.cache_convention = cache_convention_for(self, native_format, &llm_info.request_model);
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
				AIProvider::Custom(_) => req.to_anthropic()?,
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
				AIProvider::Custom(_) => match (original_format, native_format) {
					(_, None) => req.to_openai()?,
					(InputFormat::Completions, Some(custom::ProviderFormat::Completions))
					| (InputFormat::Embeddings, Some(custom::ProviderFormat::Embeddings))
					| (InputFormat::Rerank, Some(custom::ProviderFormat::Rerank))
					| (InputFormat::Messages, Some(custom::ProviderFormat::Completions))
					| (InputFormat::Responses, Some(custom::ProviderFormat::Responses)) => req.to_openai()?,
					(InputFormat::Completions, Some(custom::ProviderFormat::Messages))
					| (InputFormat::Messages, Some(custom::ProviderFormat::Messages)) => req.to_anthropic()?,
					(InputFormat::Responses, Some(custom::ProviderFormat::Completions)) => {
						req.to_openai_chat_completions()?
					},
					(InputFormat::CountTokens | InputFormat::Realtime, _) => {
						return Err(AIError::UnsupportedConversion(strng::literal!(
							"this request format does not use this codepath"
						)));
					},
					(_, Some(unsupported)) => {
						return Err(AIError::UnsupportedConversion(strng::format!(
							"unsupported custom native format {unsupported:?}"
						)));
					},
				},
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_) => req.to_openai()?,
				AIProvider::Vertex(p) => {
					if p.is_anthropic_model(Some(request_model)) {
						let body = req.to_anthropic()?;
						p.prepare_anthropic_message_body(body)?
					} else {
						req.to_vertex(p)?
					}
				},
				AIProvider::Gemini(_) => {
					if original_format == InputFormat::Responses {
						req.to_openai_chat_completions()?
					} else {
						req.to_openai()?
					}
				},
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

	#[allow(clippy::too_many_arguments)]
	pub async fn process_response(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		log: AsyncLog<llm::LLMInfo>,
		include_completion_in_log: bool,
		model_catalog: Option<&Arc<cost::ModelCatalog>>,
		resp: Response,
	) -> Result<Response, AIError> {
		// Non-success responses are plain JSON, not event-stream data.
		// Only enter the streaming path for successful responses; errors
		// fall through to the buffered path where process_error translates them.
		if req.streaming && resp.status().is_success() {
			return self
				.process_streaming(
					client,
					req,
					rate_limit,
					req_snapshot,
					log,
					include_completion_in_log,
					model_catalog.cloned(),
					resp,
				)
				.await;
		}
		let model_catalog = model_catalog.map(Arc::as_ref);

		// Buffer the body
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
		// Also strip encoding headers now that the body is decompressed; the chat
		// completions path re-adds Content-Encoding when it re-compresses.
		if encoding.is_some() {
			parts
				.extensions
				.insert(crate::cel::BufferedBody(bytes.clone()));
			parts.headers.remove(header::CONTENT_ENCODING);
			parts.headers.remove(header::TRANSFER_ENCODING);
		}

		// count_tokens has simplified response handling (just format translation)
		if req.input_format == InputFormat::CountTokens {
			let (bytes, count) = match self {
				AIProvider::Anthropic(_) | AIProvider::Vertex(_) | AIProvider::Bedrock(_) => {
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
			let llm_resp = LLMResponse {
				count_tokens: Some(count),
				..Default::default()
			};
			return Ok(Self::finalize_response(
				parts,
				bytes.into(),
				req,
				llm_resp,
				model_catalog,
				&log,
			));
		}

		// embeddings has simplified response handling
		if req.input_format == InputFormat::Embeddings {
			parts.headers.remove(header::CONTENT_LENGTH);
			if !parts.status.is_success() {
				let body = self.process_error(&req, parts.status, &bytes)?;
				return Ok(Self::finalize_response(
					parts,
					body.into(),
					req,
					LLMResponse::default(),
					model_catalog,
					&log,
				));
			}
			let (llm_resp, bytes) = self.process_embeddings_response(&req, &parts.headers, bytes)?;
			return Ok(Self::finalize_response(
				parts,
				bytes.into(),
				req,
				llm_resp,
				model_catalog,
				&log,
			));
		}

		// rerank has simplified response handling (like embeddings)
		if req.input_format == InputFormat::Rerank {
			parts.headers.remove(header::CONTENT_LENGTH);
			if !parts.status.is_success() {
				let body = self.process_error(&req, parts.status, &bytes)?;
				return Ok(Self::finalize_response(
					parts,
					body.into(),
					req,
					LLMResponse::default(),
					model_catalog,
					&log,
				));
			}
			let (llm_resp, bytes) = self.process_rerank_response(bytes)?;
			return Ok(Self::finalize_response(
				parts,
				bytes.into(),
				req,
				llm_resp,
				model_catalog,
				&log,
			));
		}

		let (llm_resp, body) = if !parts.status.is_success() {
			let body = self.process_error(&req, parts.status, &bytes)?;
			(LLMResponse::default(), body)
		} else {
			let mut resp = self.process_success(&req, &bytes)?;
			let prompt_guard_headers =
				response_prompt_guard_headers(&parts.headers, rate_limit.request_traceparent.as_ref());

			// Apply response prompt guard
			if let Some(dr) = Policy::apply_response_prompt_guard(
				&client,
				resp.as_mut(),
				&prompt_guard_headers,
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
				let llm_resp = translated.to_llm_response(false);
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			AIProvider::Vertex(p) if !p.is_anthropic_model(Some(&req.request_model)) => {
				let translated =
					conversion::vertex::from_embeddings::translate_response(&bytes, &req.request_model)?;
				let llm_resp = translated.to_llm_response(false);
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			_ => {
				let resp: types::embeddings::Response =
					serde_json::from_slice(&bytes).map_err(logged_response_parsing(&bytes))?;
				Ok((resp.to_llm_response(false), bytes))
			},
		}
	}

	fn process_rerank_response(&self, bytes: Bytes) -> Result<(LLMResponse, Bytes), AIError> {
		match self {
			AIProvider::Bedrock(_) => {
				let translated = conversion::bedrock::from_rerank::translate_response(&bytes)?;
				let llm_resp = translated.to_llm_response(false);
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			AIProvider::Vertex(_) => {
				let translated = conversion::vertex::from_rerank::translate_response(&bytes)?;
				let llm_resp = translated.to_llm_response(false);
				let body = translated.serialize().map_err(AIError::ResponseParsing)?;
				Ok((llm_resp, Bytes::from(body)))
			},
			_ => {
				let resp =
					types::rerank::parse_response_lenient(&bytes).map_err(logged_response_parsing(&bytes))?;
				Ok((resp.to_llm_response(false), bytes))
			},
		}
	}

	fn parse_completions_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		Ok(Box::new(
			serde_json::from_slice::<types::completions::Response>(bytes).map_err(|e| {
				warn!(
					error = %e,
					body = %String::from_utf8_lossy(bytes),
					"failed to parse completions response"
				);
				AIError::ResponseParsing(e)
			})?,
		))
	}

	fn parse_responses_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		Ok(Box::new(
			serde_json::from_slice::<types::responses::Response>(bytes).map_err(|e| {
				warn!(
					error = %e,
					body = %String::from_utf8_lossy(bytes),
					"failed to parse responses response"
				);
				AIError::ResponseParsing(e)
			})?,
		))
	}

	fn parse_messages_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		Ok(Box::new(
			serde_json::from_slice::<types::messages::Response>(bytes)
				.map_err(AIError::ResponseParsing)?,
		))
	}

	fn process_custom_success(
		req: &LLMRequest,
		bytes: &Bytes,
	) -> Result<Box<dyn ResponseType>, AIError> {
		match (req.input_format, req.native_format) {
			(InputFormat::Completions, Some(custom::ProviderFormat::Completions)) => {
				Self::parse_completions_response(bytes)
			},
			(InputFormat::Completions, Some(custom::ProviderFormat::Messages)) => {
				conversion::messages::from_completions::translate_response(bytes)
			},
			(InputFormat::Messages, Some(custom::ProviderFormat::Messages)) => {
				Self::parse_messages_response(bytes)
			},
			(InputFormat::Messages, Some(custom::ProviderFormat::Completions)) => {
				conversion::completions::from_messages::translate_response(bytes)
			},
			(InputFormat::Responses, Some(custom::ProviderFormat::Responses)) => {
				Self::parse_responses_response(bytes)
			},
			(InputFormat::Responses, Some(custom::ProviderFormat::Completions)) => {
				conversion::openai_compat::to_responses::translate_response(bytes, &req.request_model)
			},
			(InputFormat::Detect, None) => Ok(Box::new(
				serde_json::from_slice::<types::detect::Response>(bytes)
					.unwrap_or_else(|_| types::detect::Response::new_raw(bytes.clone())),
			)),
			(input, native) => Err(AIError::UnsupportedConversion(strng::format!(
				"custom provider cannot translate {native:?} response to {input:?}"
			))),
		}
	}

	fn process_success(
		&self,
		req: &LLMRequest,
		bytes: &Bytes,
	) -> Result<Box<dyn ResponseType>, AIError> {
		match (self, req.input_format) {
			(_, InputFormat::Detect) => Ok(Box::new(
				serde_json::from_slice::<types::detect::Response>(bytes)
					.unwrap_or_else(|_| types::detect::Response::new_raw(bytes.clone())),
			)),
			(AIProvider::Custom(_), _) => Self::process_custom_success(req, bytes),
			// Completions with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Gemini(_)
				| AIProvider::Azure(_),
				InputFormat::Completions,
			) => Self::parse_completions_response(bytes),
			// Responses with OpenAI/Azure: just passthrough
			(
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_),
				InputFormat::Responses,
			) => Self::parse_responses_response(bytes),
			// Vertex messages: passthrough only for Anthropic models, otherwise translate from completions
			(AIProvider::Vertex(p), InputFormat::Messages) => {
				if p.is_anthropic_model(Some(&req.request_model)) {
					Ok(Box::new(
						serde_json::from_slice::<types::messages::Response>(bytes)
							.map_err(logged_response_parsing(bytes))?,
					))
				} else {
					conversion::completions::from_messages::translate_response(bytes)
				}
			},
			// Anthropic messages: passthrough
			(AIProvider::Anthropic(_), InputFormat::Messages) => Self::parse_messages_response(bytes),
			// OpenAI/Gemini/Azure messages: translate from chat completions
			(
				AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Gemini(_)
				| AIProvider::Azure(_),
				InputFormat::Messages,
			) => conversion::completions::from_messages::translate_response(bytes),
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
							.map_err(logged_response_parsing(bytes))?,
					))
				}
			},
			(AIProvider::Gemini(_), InputFormat::Responses) => {
				conversion::openai_compat::to_responses::translate_response(bytes, &req.request_model)
			},
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
			(_, InputFormat::Rerank) => {
				unreachable!("Rerank should be handled by process_rerank_response")
			},
		}
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn process_streaming(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		response_policies: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		log: AsyncLog<llm::LLMInfo>,
		include_completion_in_log: bool,
		model_catalog: Option<Arc<cost::ModelCatalog>>,
		resp: Response,
	) -> Result<Response, AIError> {
		let is_vertex_anthropic = match self {
			AIProvider::Vertex(p) => p.is_anthropic_model(Some(&req.request_model)),
			_ => false,
		};
		let model = req.request_model.clone();
		let input_format = req.input_format;
		let native_format = req.native_format;
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
			temp_guard.begin_streaming_response_guard(&client, &prompt_guard_headers)
		} else {
			vec![]
		};

		let logger = AmendOnDrop::new(log, response_policies, req_snapshot, model_catalog);
		let stream_format = match self {
			AIProvider::Bedrock(_) => "awsEventStream",
			_ => "sseJson",
		};
		crate::proxy::dtrace::trace(|trace| {
			trace.llm_streaming_translation(
				self.provider().to_string(),
				format!("{input_format:?}"),
				native_format.map(|f| format!("{f:?}")),
				stream_format.to_string(),
			)
		});
		let translated = match (self, input_format, native_format) {
			(
				AIProvider::Custom(_),
				InputFormat::Completions,
				Some(custom::ProviderFormat::Completions),
			) => conversion::completions::passthrough_stream(logger, include_completion_in_log, resp),
			(AIProvider::Custom(_), InputFormat::Completions, Some(custom::ProviderFormat::Messages)) => {
				resp.map(|b| conversion::messages::from_completions::translate_stream(b, buffer, logger))
			},
			(AIProvider::Custom(_), InputFormat::Messages, Some(custom::ProviderFormat::Messages)) => {
				resp.map(|b| {
					conversion::messages::passthrough_stream(b, buffer, logger, include_completion_in_log)
				})
			},
			(AIProvider::Custom(_), InputFormat::Messages, Some(custom::ProviderFormat::Completions)) => {
				resp.map(|b| conversion::completions::from_messages::translate_stream(b, buffer, logger))
			},
			(AIProvider::Custom(_), InputFormat::Responses, Some(custom::ProviderFormat::Responses)) => {
				resp.map(|b| {
					conversion::responses::passthrough_stream(b, buffer, logger, include_completion_in_log)
				})
			},
			(
				AIProvider::Custom(_),
				InputFormat::Responses,
				Some(custom::ProviderFormat::Completions),
			) => {
				resp.map(|b| conversion::openai_compat::to_responses::translate_stream(b, buffer, logger))
			},
			// Completions with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Gemini(_)
				| AIProvider::Azure(_),
				InputFormat::Completions,
				_,
			) => conversion::completions::passthrough_stream(logger, include_completion_in_log, resp),
			// Vertex completions: passthrough for OpenAI-compatible models, translate for Anthropic models
			(AIProvider::Vertex(_), InputFormat::Completions, _) if is_vertex_anthropic => {
				resp.map(|b| conversion::messages::from_completions::translate_stream(b, buffer, logger))
			},
			(AIProvider::Vertex(_), InputFormat::Completions, _) => {
				conversion::completions::passthrough_stream(logger, include_completion_in_log, resp)
			},
			(AIProvider::Bedrock(_), InputFormat::Detect, _) => {
				types::detect::passthrough_aws_stream(logger, resp)
			},
			(_, InputFormat::Detect, _) => types::detect::passthrough_stream(logger, resp),
			// Responses with OpenAI: just passthrough
			(
				AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Azure(_)
				| AIProvider::Vertex(_),
				InputFormat::Responses,
				_,
			) => resp.map(|b| {
				conversion::responses::passthrough_stream(b, buffer, logger, include_completion_in_log)
			}),
			(AIProvider::Gemini(_), InputFormat::Responses, _) => {
				resp.map(|b| conversion::openai_compat::to_responses::translate_stream(b, buffer, logger))
			},
			// Vertex messages: passthrough only for Anthropic models, otherwise translate from completions
			(AIProvider::Vertex(_), InputFormat::Messages, _) if is_vertex_anthropic => resp.map(|b| {
				conversion::messages::passthrough_stream(b, buffer, logger, include_completion_in_log)
			}),
			(AIProvider::Vertex(_), InputFormat::Messages, _) => {
				resp.map(|b| conversion::completions::from_messages::translate_stream(b, buffer, logger))
			},
			// Anthropic messages: passthrough
			(AIProvider::Anthropic(_), InputFormat::Messages, _) => resp.map(|b| {
				conversion::messages::passthrough_stream(b, buffer, logger, include_completion_in_log)
			}),
			// OpenAI/Gemini/Azure messages: translate from chat completions
			(
				AIProvider::OpenAI(_)
				| AIProvider::Copilot(_)
				| AIProvider::Gemini(_)
				| AIProvider::Azure(_),
				InputFormat::Messages,
				_,
			) => resp.map(|b| conversion::completions::from_messages::translate_stream(b, buffer, logger)),
			// Supported paths with conversion...
			(AIProvider::Anthropic(_), InputFormat::Completions, _) => {
				resp.map(|b| conversion::messages::from_completions::translate_stream(b, buffer, logger))
			},
			(AIProvider::Bedrock(_), InputFormat::Completions, _) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_completions::translate_stream(b, buffer, logger, &model, &msg)
				})
			},
			(AIProvider::Bedrock(_), InputFormat::Messages, _) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_messages::translate_stream(
						b,
						buffer,
						logger,
						&model,
						&msg,
						include_completion_in_log,
					)
				})
			},
			(AIProvider::Bedrock(_), InputFormat::Responses, _) => {
				let msg = conversion::bedrock::message_id(&resp);
				resp.map(move |b| {
					conversion::bedrock::from_responses::translate_stream(b, buffer, logger, &model, &msg)
				})
			},
			(_, InputFormat::Realtime, _) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"realtime does not use streaming codepath"
				)));
			},
			(_, InputFormat::Responses, _) => {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"this provider does not support Responses for streaming"
				)));
			},
			(_, InputFormat::CountTokens, _) => {
				unreachable!("CountTokens should be handled by process_count_tokens_response")
			},
			(_, InputFormat::Embeddings, _) => {
				unreachable!("Embeddings should be handled by process_embeddings_response")
			},
			(_, InputFormat::Rerank, _) => {
				unreachable!("Rerank should be handled by process_rerank_response")
			},
			(AIProvider::Custom(_), input, native) => {
				return Err(AIError::UnsupportedConversion(strng::format!(
					"custom provider cannot translate {native:?} stream to {input:?}"
				)));
			},
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
		let (parts, body) = hreq.into_parts();
		let Ok(bytes) = http::read_body_with_limit(body, buffer).await else {
			return Err(AIError::RequestTooLarge);
		};
		let mut req: T = if let Some(p) = policies {
			p.unmarshal_request(&bytes, log)?
		} else {
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?
		};

		if let Some(provider_model) = &self.override_model() {
			*req.model() = Some(provider_model.to_string());
		} else if req.model().is_none() {
			if let Some(path_model) = types::detect::extract_model_from_path(parts.uri.path()) {
				*req.model() = Some(path_model.to_string());
			} else {
				return Err(AIError::MissingField("model not specified".into()));
			}
		}
		Ok((parts, req))
	}

	fn process_error(
		&self,
		req: &LLMRequest,
		status: ::http::StatusCode,
		bytes: &Bytes,
	) -> Result<Bytes, AIError> {
		match (self, req.input_format, req.native_format) {
			(
				AIProvider::Custom(_),
				InputFormat::Completions,
				Some(custom::ProviderFormat::Completions),
			)
			| (
				AIProvider::Custom(_),
				InputFormat::Responses,
				Some(custom::ProviderFormat::Completions | custom::ProviderFormat::Responses),
			)
			| (
				AIProvider::Custom(_),
				InputFormat::Embeddings,
				Some(custom::ProviderFormat::Embeddings),
			) => Ok(bytes.clone()),
			(AIProvider::Custom(_), InputFormat::Completions, Some(custom::ProviderFormat::Messages)) => {
				conversion::messages::from_completions::translate_error(bytes)
			},
			(AIProvider::Custom(_), InputFormat::Messages, Some(custom::ProviderFormat::Completions)) => {
				conversion::completions::from_messages::translate_error(bytes, status)
			},
			(AIProvider::Custom(_), InputFormat::Messages, Some(custom::ProviderFormat::Messages)) => {
				Ok(bytes.clone())
			},
			(
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_),
				InputFormat::Completions | InputFormat::Responses | InputFormat::Embeddings,
				_,
			) => {
				// Passthrough; nothing needed
				Ok(bytes.clone())
			},
			(AIProvider::Gemini(_), InputFormat::Completions, _) => {
				conversion::completions::translate_google_error(bytes)
			},
			(AIProvider::Gemini(_), InputFormat::Responses, _) => {
				conversion::gemini::from_responses::translate_error(bytes)
			},
			(AIProvider::Gemini(_), InputFormat::Embeddings, _) => {
				// Passthrough; Gemini embeddings endpoint already returns OpenAI-compatible errors.
				Ok(bytes.clone())
			},
			(AIProvider::Vertex(p), InputFormat::Completions, _) => {
				if p.is_anthropic_model(Some(&req.request_model)) {
					Ok(bytes.clone())
				} else {
					conversion::completions::translate_google_error(bytes)
				}
			},
			(AIProvider::Vertex(_), InputFormat::Embeddings, _) => {
				// Passthrough; Vertex embeddings endpoint already returns OpenAI-compatible errors.
				Ok(bytes.clone())
			},
			(
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_),
				InputFormat::Messages,
				_,
			) => conversion::completions::from_messages::translate_error(bytes, status),
			(AIProvider::Gemini(_), InputFormat::Messages, _) => {
				conversion::messages::translate_google_error(bytes)
			},
			(AIProvider::Vertex(p), InputFormat::Messages, _) => {
				if p.is_anthropic_model(Some(&req.request_model)) {
					Ok(bytes.clone())
				} else {
					conversion::messages::translate_google_error(bytes)
				}
			},
			(AIProvider::Anthropic(_), InputFormat::Messages, _) => {
				conversion::messages::translate_anthropic_error(bytes, status)
			},
			(_, InputFormat::Detect, _) => {
				// Passthrough; nothing needed
				Ok(bytes.clone())
			},
			(AIProvider::Anthropic(_), InputFormat::Completions, _) => {
				conversion::messages::from_completions::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Completions, _) => {
				conversion::bedrock::from_completions::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Messages, _) => {
				conversion::bedrock::from_messages::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Responses, _) => {
				conversion::bedrock::from_responses::translate_error(bytes)
			},
			(AIProvider::Bedrock(_), InputFormat::Embeddings, _) => {
				conversion::bedrock::from_embeddings::translate_error(bytes)
			},
			(AIProvider::Custom(_), InputFormat::Rerank, Some(custom::ProviderFormat::Rerank)) => {
				Ok(bytes.clone())
			},
			(
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_),
				InputFormat::Rerank,
				_,
			) => Ok(bytes.clone()),
			(AIProvider::Bedrock(_), InputFormat::Rerank, _) => {
				conversion::bedrock::from_rerank::translate_error(bytes)
			},
			(AIProvider::Vertex(_), InputFormat::Rerank, _) => {
				conversion::vertex::from_rerank::translate_error(bytes)
			},
			(_, _, _) => Err(AIError::UnsupportedConversion(strng::literal!(
				"this provider and format is not supported"
			))),
		}
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

pub(crate) fn logged_response_parsing(
	bytes: &[u8],
) -> impl FnOnce(serde_json::Error) -> AIError + '_ {
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
	pub fn report_rate_limit(&mut self) {
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
}

impl Drop for AmendOnDrop {
	fn drop(&mut self) {
		self.report_rate_limit();
	}
}
