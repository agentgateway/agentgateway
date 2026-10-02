use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use agent_core::strng;
use bytes::Bytes;
use futures_util::stream;
use headers::{ContentEncoding, HeaderMapExt};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use rand::seq::IndexedRandom;
use serde_json::Value;

use crate::http::transformation_cel::TransformationMetadata;
use crate::http::{self, Request, Response};
use crate::types::agent::{
	Authorization, BackendTrafficPolicy, HeaderMatch, RouteBackendReference,
};
use crate::{apply, cel, llm, schema_enum, schema_ser_schema};

#[apply(schema_ser_schema!)]
pub struct ModelRoute {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub id: Option<String>,
	pub name: String,
	pub created: u64,
	pub visibility: ModelVisibility,
	pub header_matches: Vec<Vec<HeaderMatch>>,
	pub backend: RouteBackendReference,
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::Value"))]
	pub policies: ModelRoutePolicies,
	#[cfg_attr(feature = "schema", schemars(with = "Vec<serde_json::Value>"))]
	pub backend_policies: Vec<BackendTrafficPolicy>,
}

#[apply(schema_ser_schema!)]
pub struct ModelRoutePolicies {
	pub llm: Arc<llm::Policy>,
	pub authorization: Option<Authorization>,
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum ModelVisibility {
	/// Public models can be requested directly by clients and are included in the model list.
	#[default]
	Public,
	/// Internal models can be targeted by virtual models but cannot be requested directly.
	Internal,
}

impl ModelVisibility {
	pub fn is_public(&self) -> bool {
		matches!(self, Self::Public)
	}
}

pub fn default_route_types() -> Arc<llm::Policy> {
	Arc::new(llm::Policy {
		routes: [
			(strng::new("/v1/models"), llm::RouteType::Models),
			(
				strng::new("/v1/chat/completions"),
				llm::RouteType::Completions,
			),
			(strng::new("/v1/messages"), llm::RouteType::Messages),
			(
				strng::new("/v1/messages/count_tokens"),
				llm::RouteType::AnthropicTokenCount,
			),
			(strng::new(":rawPredict"), llm::RouteType::Messages),
			(strng::new(":streamRawPredict"), llm::RouteType::Messages),
			(
				strng::new(":generateContent"),
				llm::RouteType::GenerateContent,
			),
			(
				strng::new(":streamGenerateContent"),
				llm::RouteType::GenerateContent,
			),
			(
				strng::new(":countTokens"),
				llm::RouteType::GeminiCountTokens,
			),
			(strng::new("/v1/responses"), llm::RouteType::Responses),
			(strng::new("/v1/images/generations"), llm::RouteType::Detect),
			(strng::new("/v1/images/edits"), llm::RouteType::Detect),
			(strng::new("/v1/images/variations"), llm::RouteType::Detect),
			(strng::new("/v1/responses/compact"), llm::RouteType::Detect),
			(strng::new("/v1/embeddings"), llm::RouteType::Embeddings),
			(strng::new("/v1/rerank"), llm::RouteType::Rerank),
			(strng::new("/v2/rerank"), llm::RouteType::Rerank),
			(strng::new("*"), llm::RouteType::Passthrough),
		]
		.into_iter()
		.collect(),
		..Default::default()
	})
}

#[apply(schema_ser_schema!)]
pub struct VirtualModelRoute {
	pub name: String,
	pub created: u64,
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::Value"))]
	pub llm_policy: Arc<llm::Policy>,
	pub routing: VirtualModelRouting,
}

#[apply(schema_ser_schema!)]
pub enum VirtualModelRouting {
	Weighted(Vec<WeightedTarget>),
	Failover { backend: RouteBackendReference },
	Conditional(Vec<ConditionalTarget>),
}

#[apply(schema_ser_schema!)]
pub struct WeightedTarget {
	pub model: String,
	pub weight: usize,
}

#[apply(schema_ser_schema!)]
pub struct ConditionalTarget {
	pub model: String,
	pub when: Option<Arc<cel::Expression>>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouter {
	models: Vec<ModelRoute>,
	virtual_models: Vec<VirtualModelRoute>,
	#[serde(skip)]
	codex_subscription_auth: Option<Arc<dyn CodexSubscriptionAuth>>,
	#[serde(skip)]
	codex_wildcards: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct ResolvedBackend {
	pub backend: RouteBackendReference,
	pub llm_policy: Arc<llm::Policy>,
}

pub enum ResolveResult {
	DirectResponse(Response),
	ModelList(ModelList),
	Backend(ResolvedBackend),
}

/// Static entries are rendered locally; public wildcard routes can contribute provider catalogs
/// without creating separate HTTP API routes.
pub struct ModelList {
	pub static_models: Vec<serde_json::Value>,
	pub dynamic_backends: Vec<DynamicModelBackend>,
}

pub struct DynamicModelBackend {
	pub resolved: ResolvedBackend,
	/// Codex's catalog is required to form a complete model list. Other provider catalogs are
	/// best-effort so one unavailable provider does not hide models discovered from others.
	pub required: bool,
}

/// The model name reserved for the gateway-owned Codex subscription authorization flow.
/// It is deliberately not a route or catalog entry, so it can never reach a Codex upstream.
pub const CODEX_SUBSCRIPTION_AUTH_MODEL: &str = "codex-subscription-auth";

/// A sanitized result from a Codex OAuth device authorization action.
///
/// Credential management owns the device session and token persistence. The router only renders
/// these public instructions, keeping access and refresh token material out of the response path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexSubscriptionAuthState {
	Pending {
		verification_url: String,
		user_code: String,
		expires_in_seconds: u64,
	},
	Authorized,
	Expired,
	Denied,
	Unavailable,
}

pub type CodexSubscriptionAuthFuture<'a> =
	Pin<Box<dyn Future<Output = CodexSubscriptionAuthState> + Send + 'a>>;

/// Boundary implemented by the future OAuth credential manager.
///
/// Each control request starts a device authorization session or polls an existing pending one.
/// The returned state contains only values intended for the client; implementations must never
/// return OAuth credentials through this interface.
pub trait CodexSubscriptionAuth: Send + Sync {
	fn start_or_poll(&self) -> CodexSubscriptionAuthFuture<'_>;
}

impl CodexSubscriptionAuth for crate::llm::codex_oauth::Manager {
	fn start_or_poll(&self) -> CodexSubscriptionAuthFuture<'_> {
		Box::pin(async move {
			match self.start_or_poll().await {
				Ok(crate::llm::codex_oauth::AuthorizationState::Pending {
					verification_uri,
					user_code,
					expires_at,
					..
				}) => CodexSubscriptionAuthState::Pending {
					verification_url: verification_uri,
					user_code,
					expires_in_seconds: expires_at
						.duration_since(std::time::SystemTime::now())
						.unwrap_or_default()
						.as_secs(),
				},
				Ok(crate::llm::codex_oauth::AuthorizationState::Authorized) => {
					CodexSubscriptionAuthState::Authorized
				},
				Ok(crate::llm::codex_oauth::AuthorizationState::Expired)
				| Err(crate::llm::codex_oauth::OAuthError::Expired) => CodexSubscriptionAuthState::Expired,
				Ok(crate::llm::codex_oauth::AuthorizationState::Denied)
				| Err(crate::llm::codex_oauth::OAuthError::Denied) => CodexSubscriptionAuthState::Denied,
				Ok(crate::llm::codex_oauth::AuthorizationState::Failed) => {
					tracing::warn!("Codex OAuth device authorization failed");
					CodexSubscriptionAuthState::Unavailable
				},
				Err(err) => {
					tracing::warn!(%err, "Codex OAuth authorization is unavailable");
					CodexSubscriptionAuthState::Unavailable
				},
			}
		})
	}
}

impl std::fmt::Debug for ModelRouter {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("ModelRouter")
			.field("models", &self.models)
			.field("virtual_models", &self.virtual_models)
			.field(
				"has_codex_subscription_auth",
				&self.codex_subscription_auth.is_some(),
			)
			.finish()
	}
}

type RouterResult<T> = Result<T, Box<Response>>;

struct RequestedModel {
	model: String,
	location: RequestedModelLocation,
}

enum RequestedModelLocation {
	Body(Value),
	Multipart,
	Path,
}

impl RequestedModelLocation {
	fn llm_request(&self) -> Option<&Value> {
		match self {
			Self::Body(body) => Some(body),
			Self::Multipart | Self::Path => None,
		}
	}
}

impl ModelRouter {
	pub fn new(models: Vec<ModelRoute>, virtual_models: Vec<VirtualModelRoute>) -> Self {
		Self {
			models,
			virtual_models,
			codex_subscription_auth: None,
			codex_wildcards: vec![],
		}
	}

	pub fn with_codex_subscription_auth(
		mut self,
		codex_subscription_auth: Arc<dyn CodexSubscriptionAuth>,
	) -> Self {
		self.codex_subscription_auth = Some(codex_subscription_auth);
		self
	}

	pub fn with_codex_namespaces(
		mut self,
		is_codex: impl Fn(&RouteBackendReference) -> bool,
	) -> Self {
		self.codex_wildcards = self
			.models
			.iter()
			.enumerate()
			.filter(|(_, model)| {
				matches!(
					model.name.as_str(),
					"*" | llm::codex_subscription::MODEL_PATTERN
				) && is_codex(&model.backend)
			})
			.map(|(index, _)| index)
			.collect();
		self
	}

	pub async fn resolve(&self, req: &mut Request) -> ResolveResult {
		if is_model_list_request(req) {
			let static_models = self.static_model_list(req);
			let dynamic_backends = self.dynamic_model_backends(req);
			if !dynamic_backends.is_empty() {
				return ResolveResult::ModelList(ModelList {
					static_models,
					dynamic_backends,
				});
			}
			return ResolveResult::DirectResponse(Self::model_list_response(static_models));
		}
		let requested_model = match requested_model(req).await {
			Ok(requested_model) => requested_model,
			Err(resp) => return ResolveResult::DirectResponse(*resp),
		};
		if requested_model.model == CODEX_SUBSCRIPTION_AUTH_MODEL
			&& is_codex_subscription_auth_request(req)
		{
			let chat_completions = req.uri().path().trim_end_matches('/') == "/v1/chat/completions";
			return ResolveResult::DirectResponse(
				self
					.codex_subscription_auth_response(chat_completions, requested_model.location)
					.await,
			);
		}
		if !api_key_model_authorized(req, &requested_model.model) {
			return ResolveResult::DirectResponse(api_key_model_authorization_denied_response());
		}
		req
			.extensions_mut()
			.get_or_insert_with(TransformationMetadata::default)
			.0
			.insert(
				"agentgateway_user_model".to_string(),
				Value::String(requested_model.model.clone()),
			);
		if let Some(virtual_model) = self
			.virtual_models
			.iter()
			.find(|model| model.name == requested_model.model)
		{
			return self
				.resolve_virtual_model(virtual_model, req, requested_model.location)
				.await;
		}
		tracing::trace!(
			requested_model = %requested_model.model,
			virtual_model_count = self.virtual_models.len(),
			"unable to find declared virtual model; trying concrete model routes",
		);

		match self.resolve_concrete_model(&requested_model.model, false, req) {
			Ok(Some(route)) => ResolveResult::Backend(route),
			Ok(None) => ResolveResult::DirectResponse(model_not_found_response()),
			Err(()) => ResolveResult::DirectResponse(model_authorization_denied_response()),
		}
	}

	async fn codex_subscription_auth_response(
		&self,
		chat_completions: bool,
		location: RequestedModelLocation,
	) -> Response {
		let Some(body) = location.llm_request() else {
			return llm_error_response(
				::http::StatusCode::BAD_REQUEST,
				"Codex subscription authentication requires a JSON request body",
				"codex_subscription_auth_invalid_request",
			);
		};
		codex_subscription_auth_control_response(
			self.codex_subscription_auth.as_deref(),
			chat_completions,
			body.get("stream").and_then(Value::as_bool).unwrap_or(false),
		)
		.await
	}

	fn static_model_list(&self, req: &Request) -> Vec<serde_json::Value> {
		self
			.models
			.iter()
			.enumerate()
			.filter(|(index, _)| !self.codex_wildcards.contains(index))
			.map(|(_, model)| model)
			.filter(|model| model.visibility == ModelVisibility::Public)
			// A wildcard Codex route is an inference dispatch rule, not a model record.
			.filter(|model| !model.name.contains('*'))
			.filter(|model| model_authorized(model, req))
			.flat_map(|model| {
				api_key_discoverable_models(req, &model.name)
					.map(|name| model_list_entry(name, model.created))
			})
			.chain(
				self
					.virtual_models
					.iter()
					.filter(|model| api_key_model_authorized(req, &model.name))
					.map(|model| model_list_entry(&model.name, model.created)),
			)
			.collect()
	}

	fn dynamic_model_backends(&self, req: &Request) -> Vec<DynamicModelBackend> {
		self
			.models
			.iter()
			.enumerate()
			.filter(|(_, model)| model.name.contains('*'))
			.filter(|(_, model)| {
				model.visibility == ModelVisibility::Public && model_authorized(model, req)
			})
			.map(|(index, model)| DynamicModelBackend {
				resolved: ResolvedBackend {
					backend: model.backend.clone(),
					llm_policy: model.policies.llm.clone(),
				},
				required: self.codex_wildcards.contains(&index),
			})
			.collect()
	}

	fn model_list_response(data: Vec<serde_json::Value>) -> Response {
		let body = serde_json::json!({
			"data": data,
			"object": "list",
		})
		.to_string();
		::http::Response::builder()
			.status(::http::StatusCode::OK)
			.header(::http::header::CONTENT_TYPE, "application/json")
			.body(http::Body::from(body))
			.expect("LLM model list response is valid")
	}

	async fn resolve_virtual_model(
		&self,
		virtual_model: &VirtualModelRoute,
		req: &mut Request,
		location: RequestedModelLocation,
	) -> ResolveResult {
		let target = match &virtual_model.routing {
			VirtualModelRouting::Weighted(targets) => {
				match targets.choose_weighted(&mut rand::rng(), |target| target.weight) {
					Ok(target) => target.model.clone(),
					Err(err) => {
						tracing::debug!(%err, "failed to select weighted virtual model target");
						return ResolveResult::DirectResponse(llm_error_response(
							::http::StatusCode::NOT_FOUND,
							&format!("Virtual model {} could not be resolved", virtual_model.name),
							"virtual_model_not_resolved",
						));
					},
				}
			},
			VirtualModelRouting::Failover { backend } => {
				return ResolveResult::Backend(ResolvedBackend {
					backend: backend.clone(),
					llm_policy: virtual_model.llm_policy.clone(),
				});
			},
			VirtualModelRouting::Conditional(targets) => {
				let exec = match location.llm_request() {
					Some(llm_request) => cel::Executor::new_llm_request(req, llm_request),
					None => cel::Executor::new_request(req),
				};
				match targets.iter().find(|target| {
					target
						.when
						.as_ref()
						.map(|expr| exec.eval_bool(expr))
						.unwrap_or(true)
				}) {
					Some(target) => target.model.clone(),
					None => {
						return ResolveResult::DirectResponse(llm_error_response(
							::http::StatusCode::BAD_REQUEST,
							&format!(
								"Virtual model {} did not match any conditional target",
								virtual_model.name
							),
							"virtual_model_no_matching_target",
						));
					},
				}
			},
		};
		if let Err(resp) = rewrite_request_model(req, location, &target) {
			return ResolveResult::DirectResponse(*resp);
		}
		match self.resolve_concrete_model(&target, true, req) {
			Ok(Some(route)) => ResolveResult::Backend(route),
			Ok(None) => {
				tracing::debug!(
					virtual_model = %virtual_model.name,
					target_model = %target,
					"virtual model selected target with no declared concrete model",
				);
				ResolveResult::DirectResponse(llm_error_response(
					::http::StatusCode::NOT_FOUND,
					&format!(
						"Virtual model {} selected target {target}, but no matching model was found",
						virtual_model.name
					),
					"virtual_model_target_not_found",
				))
			},
			Err(()) => ResolveResult::DirectResponse(model_authorization_denied_response()),
		}
	}

	fn resolve_concrete_model(
		&self,
		requested_model: &str,
		allow_internal: bool,
		req: &Request,
	) -> Result<Option<ResolvedBackend>, ()> {
		// `models` can store things like `provider/*`. The concrete `requested_model` will be like `provider/real-model`.
		let matches = |index: usize, model: &ModelRoute| {
			(!self.codex_wildcards.contains(&index)
				|| llm::codex_subscription::upstream_model(requested_model).is_some())
				&& model_name_matches(&model.name, requested_model)
				&& header_matches(&model.header_matches, req)
		};
		let Some((_, model)) = self
			.models
			.iter()
			.enumerate()
			.filter(|(index, model)| matches(*index, model))
			.min_by_key(|(index, model)| {
				let wildcard = model.name.contains('*');
				let codex_dispatch = self.codex_wildcards.contains(index)
					&& llm::codex_subscription::upstream_model(requested_model).is_some();
				let rank = if !wildcard {
					0
				} else if codex_dispatch {
					2
				} else if model.name == "*" {
					4
				} else {
					1
				};
				let specificity = model.name.trim_end_matches('*').len();
				(rank, std::cmp::Reverse(specificity), *index)
			})
		else {
			return Ok(None);
		};
		if !allow_internal && model.visibility != ModelVisibility::Public {
			return Ok(None);
		}
		if !model_authorized(model, req) {
			return Err(());
		}
		Ok(Some(ResolvedBackend {
			backend: model.backend.clone(),
			llm_policy: model.policies.llm.clone(),
		}))
	}
}

fn model_not_found_response() -> Response {
	llm_error_response(
		::http::StatusCode::NOT_FOUND,
		"Model not found",
		"model_not_found",
	)
}

fn model_authorization_denied_response() -> Response {
	llm_error_response(
		::http::StatusCode::FORBIDDEN,
		"Model authorization denied",
		"model_authorization_denied",
	)
}

fn api_key_model_authorization_denied_response() -> Response {
	llm_error_response(
		::http::StatusCode::FORBIDDEN,
		"Model is not allowed for this API key",
		"model_not_allowed",
	)
}

fn request_body_too_large_response() -> Response {
	llm_error_response(
		::http::StatusCode::PAYLOAD_TOO_LARGE,
		"LLM request body exceeded the buffer limit",
		"request_body_too_large",
	)
}

fn llm_error_response(status: ::http::StatusCode, message: &str, code: &str) -> Response {
	::http::Response::builder()
		.status(status)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::json!({
				"error": {
					"message": message,
					"type": "invalid_request_error",
					"code": code,
				}
			})
			.to_string(),
		))
		.expect("LLM error response is valid")
}

fn is_codex_subscription_auth_request(req: &Request) -> bool {
	req.method() == ::http::Method::POST
		&& matches!(
			req.uri().path().trim_end_matches('/'),
			"/v1/chat/completions" | "/v1/responses"
		)
}

/// Renders the reserved Codex subscription authorization control response.
///
/// This accepts only the OAuth manager boundary and request properties needed to select the
/// public response shape, so direct Codex backends can use the same state without exposing
/// credentials or entering catalog admission.
pub async fn codex_subscription_auth_control_response(
	auth: Option<&dyn CodexSubscriptionAuth>,
	chat_completions: bool,
	streaming: bool,
) -> Response {
	if streaming {
		return llm_error_response(
			::http::StatusCode::BAD_REQUEST,
			"Streaming is not supported by the codex-subscription-auth control model",
			"codex_subscription_auth_streaming_unsupported",
		);
	}
	let state = match auth {
		Some(auth) => auth.start_or_poll().await,
		None => CodexSubscriptionAuthState::Unavailable,
	};
	let instruction = codex_subscription_auth_instruction(state);
	if chat_completions {
		chat_completion_control_response(&instruction)
	} else {
		responses_control_response(&instruction)
	}
}

fn codex_subscription_auth_instruction(state: CodexSubscriptionAuthState) -> String {
	let instruction = match state {
		CodexSubscriptionAuthState::Pending {
			verification_url,
			user_code,
			expires_in_seconds,
		} => serde_json::json!({
			"status": "pending",
			"verification_url": verification_url,
			"user_code": user_code,
			"expires_in_seconds": expires_in_seconds,
		}),
		CodexSubscriptionAuthState::Authorized => serde_json::json!({
			"status": "authorized",
			"message": "Codex subscription authorization is complete.",
		}),
		CodexSubscriptionAuthState::Expired => serde_json::json!({
			"status": "expired",
			"message": "Codex subscription authorization expired. Call this model again to start a new authorization session.",
		}),
		CodexSubscriptionAuthState::Denied => serde_json::json!({
			"status": "denied",
			"message": "Codex subscription authorization was denied. Call this model again to start a new authorization session.",
		}),
		CodexSubscriptionAuthState::Unavailable => serde_json::json!({
			"status": "unavailable",
			"message": "Codex subscription authorization is not configured on this gateway.",
		}),
	};
	serde_json::to_string(&instruction).expect("Codex auth instruction is serializable")
}

fn control_response_created_at() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

fn chat_completion_control_response(instruction: &str) -> Response {
	let body = serde_json::json!({
		"id": "codex-subscription-auth",
		"object": "chat.completion",
		"created": control_response_created_at(),
		"model": CODEX_SUBSCRIPTION_AUTH_MODEL,
		"choices": [{
			"index": 0,
			"message": {"role": "assistant", "content": instruction},
			"finish_reason": "stop",
		}],
	});
	::http::Response::builder()
		.status(::http::StatusCode::OK)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(body.to_string()))
		.expect("Codex auth Chat Completions response is valid")
}

fn responses_control_response(instruction: &str) -> Response {
	let body = serde_json::json!({
		"id": "resp_codex_subscription_auth",
		"object": "response",
		"created_at": control_response_created_at(),
		"status": "completed",
		"model": CODEX_SUBSCRIPTION_AUTH_MODEL,
		"output": [{
			"id": "msg_codex_subscription_auth",
			"type": "message",
			"status": "completed",
			"role": "assistant",
			"content": [{
				"type": "output_text",
				"text": instruction,
				"annotations": [],
			}],
		}],
		"output_text": instruction,
	});
	::http::Response::builder()
		.status(::http::StatusCode::OK)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(body.to_string()))
		.expect("Codex auth Responses response is valid")
}

fn model_authorized(model: &ModelRoute, req: &Request) -> bool {
	let rules = model
		.policies
		.authorization
		.iter()
		.map(|authorization| authorization.0.clone())
		.collect::<Vec<_>>();
	if rules.is_empty() {
		return true;
	}
	crate::http::authorization::HTTPAuthorizationSet::new(
		crate::http::authorization::RuleSets::from_arcs(rules),
	)
	.apply(req)
	.is_ok()
}

fn api_key_model_authorized(req: &Request, model: &str) -> bool {
	let Some(policy) = req
		.extensions()
		.get::<crate::http::apikey::ModelAccessPolicy>()
	else {
		return true;
	};
	let allowed = policy.allows(model);
	if !allowed {
		tracing::debug!(model, "requested model is not allowed for API key");
	}
	allowed
}

fn api_key_discoverable_models<'a>(
	req: &'a Request,
	configured_model: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
	crate::http::apikey::discoverable_models(
		req
			.extensions()
			.get::<crate::http::apikey::ModelAccessPolicy>(),
		configured_model,
	)
}

fn model_list_entry(id: &str, created: u64) -> serde_json::Value {
	serde_json::json!({
		"id": id,
		"object": "model",
		"created": created,
		// TODO: this matches some other gateways but seems odd. Should we use the real provide here?
		"owned_by": "openai",
	})
}

fn is_model_list_request(req: &Request) -> bool {
	if req.method() != ::http::Method::GET {
		return false;
	}
	let path = req.uri().path().trim_end_matches('/');
	path == "/v1/models" || path == "/models"
}

fn header_matches(matches: &[Vec<HeaderMatch>], req: &Request) -> bool {
	if matches.is_empty() {
		return true;
	}
	matches.iter().any(|headers| headers_match(headers, req))
}

fn headers_match(headers: &[HeaderMatch], req: &Request) -> bool {
	for HeaderMatch { name, value } in headers {
		if !http::request_header_matches(name, value, req) {
			return false;
		}
	}
	true
}

fn model_name_matches(pattern: &str, model: &str) -> bool {
	if pattern == "*" {
		return true;
	}
	if let Some(prefix) = pattern.strip_suffix('*') {
		return model.starts_with(prefix);
	}
	if let Some(suffix) = pattern.strip_prefix('*') {
		return model.ends_with(suffix);
	}
	pattern == model
}

async fn requested_model(req: &mut Request) -> RouterResult<RequestedModel> {
	let path = req.uri().path();
	if let Some(model) = crate::llm::types::detect::extract_model_from_path(path) {
		return Ok(RequestedModel {
			model: model.to_string(),
			location: RequestedModelLocation::Path,
		});
	}

	let body = body_bytes(req).await?;
	if let Some(boundary) = multipart_boundary(req) {
		let model = multipart_model(&body, &boundary).await?;
		return Ok(RequestedModel {
			model,
			location: RequestedModelLocation::Multipart,
		});
	}
	let body: Value = serde_json::from_slice(&body).map_err(|err| {
		tracing::debug!(%err, "failed to parse LLM request body");
		Box::new(llm_error_response(
			::http::StatusCode::BAD_REQUEST,
			"LLM request body must be valid JSON",
			"invalid_request_body",
		))
	})?;
	let model = body
		.get("model")
		.and_then(Value::as_str)
		.map(ToString::to_string)
		.ok_or_else(|| {
			Box::new(llm_error_response(
				::http::StatusCode::BAD_REQUEST,
				"LLM request body is missing string field 'model'",
				"missing_model",
			))
		})?;
	Ok(RequestedModel {
		model,
		location: RequestedModelLocation::Body(body),
	})
}

fn rewrite_request_model(
	req: &mut Request,
	location: RequestedModelLocation,
	target: &str,
) -> RouterResult<()> {
	match location {
		RequestedModelLocation::Body(body) => rewrite_body_model(req, body, target),
		RequestedModelLocation::Path => rewrite_uri_model(req, target),
		// TODO: Rewrite multipart model fields for virtual model routing.
		RequestedModelLocation::Multipart => Ok(()),
	}
}

pub(crate) async fn normalize_codex_model(req: &mut Request) -> RouterResult<()> {
	let mut requested = requested_model(req).await?;
	if requested.model == CODEX_SUBSCRIPTION_AUTH_MODEL {
		return Ok(());
	}
	let authorized_name = req
		.extensions()
		.get::<TransformationMetadata>()
		.and_then(|metadata| metadata.0.get("agentgateway_user_model"))
		.and_then(Value::as_str)
		.unwrap_or(&requested.model);
	if !api_key_model_authorized(req, authorized_name) {
		return Err(Box::new(api_key_model_authorization_denied_response()));
	}
	let Some(slug) = llm::codex_subscription::upstream_model(&requested.model) else {
		return Err(Box::new(llm_error_response(
			::http::StatusCode::BAD_REQUEST,
			"Codex models require an openai/ prefix and a model ID",
			"model_not_found",
		)));
	};
	req
		.extensions_mut()
		.get_or_insert_with(TransformationMetadata::default)
		.0
		.entry("agentgateway_user_model".to_string())
		.or_insert_with(|| Value::String(requested.model.clone()));
	if let RequestedModelLocation::Body(Value::Object(body)) = &mut requested.location {
		// The Codex subscription backend rejects this OpenAI Responses parameter.
		body.remove("max_output_tokens");
	}
	rewrite_request_model(req, requested.location, slug)
}

fn rewrite_body_model(req: &mut Request, mut body: Value, target: &str) -> RouterResult<()> {
	let Some(obj) = body.as_object_mut() else {
		return Ok(());
	};
	obj.insert("model".to_string(), Value::String(target.to_string()));
	let body = serde_json::to_vec(&body).map_err(|err| {
		tracing::debug!(%err, "failed to serialize rewritten LLM request body");
		Box::new(llm_error_response(
			::http::StatusCode::BAD_REQUEST,
			"Failed to rewrite LLM request body model",
			"request_body_rewrite_failed",
		))
	})?;
	*req.body_mut() = http::Body::from(body);
	req.headers_mut().remove(::http::header::CONTENT_LENGTH);
	req.extensions_mut().remove::<cel::BufferedBody>();
	Ok(())
}

fn rewrite_uri_model(req: &mut Request, target: &str) -> RouterResult<()> {
	let Some(path_and_query) = req.uri().path_and_query() else {
		return Ok(());
	};
	let Some(path) = rewrite_path_model(path_and_query.path(), target) else {
		return Ok(());
	};
	let path_and_query = if let Some(query) = path_and_query.query() {
		format!("{path}?{query}")
	} else {
		path
	};
	let path_and_query = path_and_query.parse().map_err(|err| {
		tracing::debug!(%err, "failed to rewrite LLM request URI model");
		Box::new(llm_error_response(
			::http::StatusCode::BAD_REQUEST,
			"Failed to rewrite LLM request URI model",
			"request_uri_rewrite_failed",
		))
	})?;
	let mut parts = req.uri().clone().into_parts();
	parts.path_and_query = Some(path_and_query);
	*req.uri_mut() = ::http::Uri::from_parts(parts).map_err(|err| {
		tracing::debug!(%err, "failed to rebuild LLM request URI");
		Box::new(llm_error_response(
			::http::StatusCode::BAD_REQUEST,
			"Failed to rewrite LLM request URI model",
			"request_uri_rewrite_failed",
		))
	})?;
	Ok(())
}

fn rewrite_path_model(path: &str, target: &str) -> Option<String> {
	if path.ends_with(":streamRawPredict") || path.ends_with(":rawPredict") {
		return rewrite_publishers_path_model(path, target);
	}
	if path.ends_with(":generateContent")
		|| path.ends_with(":streamGenerateContent")
		|| path.ends_with(":countTokens")
	{
		if path.contains("/publishers/") {
			return rewrite_publishers_path_model(path, target);
		}
		// Gemini API: /v1beta/models/{model}:{suffix}
		let (prefix, rest) = path.split_once("/models/")?;
		let (_, suffix) = rest.split_once(':')?;
		return Some(format!(
			"{prefix}/models/{}:{suffix}",
			encode_model_path_segment(target)
		));
	}
	for suffix in [
		"/invoke-with-response-stream",
		"/invoke",
		"/converse-stream",
		"/converse",
	] {
		if let Some(before_suffix) = path.strip_suffix(suffix)
			&& let Some((prefix, _)) = before_suffix.split_once("/model/")
		{
			return Some(format!(
				"{prefix}/model/{}{suffix}",
				encode_model_path_segment(target)
			));
		}
	}
	None
}

fn rewrite_publishers_path_model(path: &str, target: &str) -> Option<String> {
	// Vertex: .../publishers/{publisher}/models/{model}:{suffix}
	// Preserve the publisher from the path; only rewrite the model id. Matching only
	// `publishers/anthropic` incorrectly dropped virtual-model rewrites for other publishers.
	let (prefix, rest) = path.split_once("/publishers/")?;
	let (publisher, after_publisher) = rest.split_once("/models/")?;
	if publisher.is_empty() {
		return None;
	}
	let (_, suffix) = after_publisher.split_once(':')?;
	Some(format!(
		"{prefix}/publishers/{publisher}/models/{}:{suffix}",
		encode_model_path_segment(target)
	))
}

fn encode_model_path_segment(model: &str) -> String {
	const MODEL_SEGMENT: &AsciiSet = &CONTROLS.add(b'/').add(b'%');
	utf8_percent_encode(model, MODEL_SEGMENT).to_string()
}

fn multipart_boundary(req: &Request) -> Option<String> {
	req
		.headers()
		.get(::http::header::CONTENT_TYPE)
		.and_then(|content_type| content_type.to_str().ok())
		.and_then(|content_type| multer::parse_boundary(content_type).ok())
}

async fn multipart_model(body: &Bytes, boundary: &str) -> RouterResult<String> {
	let stream = stream::once(std::future::ready(Ok::<Bytes, multer::Error>(body.clone())));
	let mut multipart = multer::Multipart::new(stream, boundary);
	while let Some(field) = multipart.next_field().await.map_err(|err| {
		tracing::debug!(%err, "failed to parse LLM multipart request body");
		Box::new(llm_error_response(
			::http::StatusCode::BAD_REQUEST,
			"LLM multipart request body must be valid multipart/form-data",
			"invalid_request_body",
		))
	})? {
		if field.name() == Some("model") {
			return field.text().await.map_err(|err| {
				tracing::debug!(%err, "failed to parse LLM multipart model field");
				Box::new(llm_error_response(
					::http::StatusCode::BAD_REQUEST,
					"LLM multipart request body has invalid string field 'model'",
					"invalid_model",
				))
			});
		}
	}
	Err(Box::new(llm_error_response(
		::http::StatusCode::BAD_REQUEST,
		"LLM multipart request body is missing string field 'model'",
		"missing_model",
	)))
}

async fn body_bytes(req: &mut Request) -> RouterResult<Bytes> {
	let limit = http::buffer_limit(req);
	let content_encoding = req.headers().typed_get::<ContentEncoding>();
	if content_encoding.is_some() {
		let body = if let Some(body) = req.extensions().get::<cel::BufferedBody>() {
			http::Body::from(
				body
					.bytes()
					.cloned()
					.ok_or_else(|| Box::new(request_body_too_large_response()))?,
			)
		} else {
			std::mem::take(req.body_mut())
		};
		let (encoding, body) =
			http::compression::to_bytes_with_decompression(body, content_encoding.as_ref(), limit)
				.await
				.map_err(|err| match err {
					http::compression::Error::LimitExceeded => Box::new(request_body_too_large_response()),
					err => {
						tracing::debug!(%err, "failed to decode LLM request body");
						Box::new(llm_error_response(
							::http::StatusCode::BAD_REQUEST,
							"Failed to decode LLM request body",
							"request_body_decode_failed",
						))
					},
				})?;
		*req.body_mut() = http::Body::from(body.clone());
		if encoding.is_some() {
			req.headers_mut().remove(::http::header::CONTENT_ENCODING);
			req.headers_mut().remove(::http::header::CONTENT_LENGTH);
			req.headers_mut().remove(::http::header::TRANSFER_ENCODING);
		}
		req
			.extensions_mut()
			.insert(cel::BufferedBody::complete(body.clone()));
		return Ok(body);
	}
	if let Some(body) = req.extensions().get::<cel::BufferedBody>() {
		return body
			.bytes()
			.cloned()
			.ok_or_else(|| Box::new(request_body_too_large_response()));
	}
	let inspection = http::inspect_body_with_limit(req.body_mut(), limit)
		.await
		.map_err(|err| {
			tracing::debug!(%err, "failed to read LLM request body");
			Box::new(llm_error_response(
				::http::StatusCode::BAD_REQUEST,
				"Failed to read LLM request body",
				"request_body_read_failed",
			))
		})?;
	let body = match inspection {
		http::BodyInspection::Complete(body) => body,
		http::BodyInspection::Partial(_) => {
			return Err(Box::new(request_body_too_large_response()));
		},
	};
	req
		.extensions_mut()
		.insert(cel::BufferedBody::complete(body.clone()));
	Ok(body)
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::*;
	use crate::transport::BufferLimit;
	use crate::types::agent::RouteBackendTarget;

	#[tokio::test]
	async fn normalize_codex_model_removes_only_output_limit() {
		for limit in [None, Some(Value::Null), Some(serde_json::json!(4096))] {
			let mut body = serde_json::json!({
				"model": "openai/gpt-test",
				"input": "hello",
				"instructions": "Be concise",
				"stream": false,
				"store": true,
				"reasoning": {"effort": "low"},
				"metadata": {"max_output_tokens": "preserve nested fields"}
			});
			let mut expected = body.clone();
			expected["model"] = serde_json::json!("gpt-test");
			if let Some(limit) = limit {
				body["max_output_tokens"] = limit;
			}
			let bytes = Bytes::from(serde_json::to_vec(&body).unwrap());
			let mut req = ::http::Request::builder()
				.method(::http::Method::POST)
				.uri("/v1/responses")
				.header(::http::header::CONTENT_LENGTH, bytes.len())
				.body(http::Body::from(bytes.clone()))
				.unwrap();
			req
				.extensions_mut()
				.insert(cel::BufferedBody::complete(bytes));

			normalize_codex_model(&mut req).await.unwrap();

			assert!(!req.headers().contains_key(::http::header::CONTENT_LENGTH));
			assert!(req.extensions().get::<cel::BufferedBody>().is_none());
			assert_eq!(
				req.extensions().get::<TransformationMetadata>().unwrap().0["agentgateway_user_model"],
				"openai/gpt-test"
			);
			let bytes = http::read_body_with_limit(req.into_body(), 4096)
				.await
				.unwrap();
			assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
		}
	}

	#[tokio::test]
	async fn generic_model_rewrite_preserves_output_limit() {
		let body = serde_json::json!({
			"model": "virtual-model",
			"input": "hello",
			"max_output_tokens": 4096
		});
		let mut req = ::http::Request::builder()
			.method(::http::Method::POST)
			.uri("/v1/responses")
			.body(http::Body::from(body.to_string()))
			.unwrap();
		let requested = requested_model(&mut req).await.unwrap();
		rewrite_request_model(&mut req, requested.location, "gpt-test").unwrap();

		let mut expected = body;
		expected["model"] = serde_json::json!("gpt-test");
		let bytes = http::read_body_with_limit(req.into_body(), 4096)
			.await
			.unwrap();
		assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
	}

	#[derive(Debug)]
	struct TestCodexSubscriptionAuth(CodexSubscriptionAuthState);

	impl CodexSubscriptionAuth for TestCodexSubscriptionAuth {
		fn start_or_poll(&self) -> CodexSubscriptionAuthFuture<'_> {
			Box::pin(std::future::ready(self.0.clone()))
		}
	}

	#[derive(Default)]
	struct CountingCodexSubscriptionAuth(AtomicUsize);

	impl CodexSubscriptionAuth for CountingCodexSubscriptionAuth {
		fn start_or_poll(&self) -> CodexSubscriptionAuthFuture<'_> {
			self.0.fetch_add(1, Ordering::Relaxed);
			Box::pin(std::future::ready(CodexSubscriptionAuthState::Unavailable))
		}
	}

	struct PendingThenDeniedCodexEndpoint;

	#[async_trait::async_trait]
	impl crate::llm::codex_oauth::TokenEndpoint for PendingThenDeniedCodexEndpoint {
		async fn begin_device_authorization(
			&self,
			_request: crate::llm::codex_oauth::DeviceAuthorizationRequest,
		) -> Result<
			crate::llm::codex_oauth::DeviceAuthorizationResponse,
			crate::llm::codex_oauth::OAuthError,
		> {
			Ok(crate::llm::codex_oauth::DeviceAuthorizationResponse {
				verification_uri: "https://auth.example.test/device".to_string(),
				verification_uri_complete: Some("https://auth.example.test/device?code=secret".to_string()),
				user_code: "USER-CODE".to_string(),
				device_code: secrecy::SecretString::from("device-code"),
				expires_in: std::time::Duration::from_secs(60),
				interval: std::time::Duration::from_secs(1),
			})
		}

		async fn poll_device_authorization(
			&self,
			_device_code: &secrecy::SecretString,
			_user_code: &str,
		) -> Result<crate::llm::codex_oauth::DevicePoll, crate::llm::codex_oauth::OAuthError> {
			Ok(crate::llm::codex_oauth::DevicePoll::Denied)
		}

		async fn exchange_authorization_code(
			&self,
			_authorization_code: &secrecy::SecretString,
			_pkce_verifier: &secrecy::SecretString,
		) -> Result<crate::llm::codex_oauth::TokenResponse, crate::llm::codex_oauth::OAuthError> {
			Err(crate::llm::codex_oauth::OAuthError::InvalidResponse)
		}

		async fn refresh(
			&self,
			_refresh_token: &secrecy::SecretString,
		) -> Result<crate::llm::codex_oauth::TokenResponse, crate::llm::codex_oauth::OAuthError> {
			Err(crate::llm::codex_oauth::OAuthError::CredentialUnavailable)
		}
	}

	async fn response_json(response: Response) -> Value {
		let body = http::read_body_with_limit(response.into_body(), 16 * 1024)
			.await
			.expect("response body");
		serde_json::from_slice(&body).expect("JSON response")
	}

	#[tokio::test]
	async fn codex_subscription_auth_returns_chat_completions_control_response() {
		let router = ModelRouter::new(vec![], vec![]).with_codex_subscription_auth(Arc::new(
			TestCodexSubscriptionAuth(CodexSubscriptionAuthState::Pending {
				verification_url: "https://auth.example.test/device?flow=codex".to_string(),
				user_code: "ABCD-EFGH".to_string(),
				expires_in_seconds: 600,
			}),
		));
		let mut req = ::http::Request::builder()
			.method(::http::Method::POST)
			.uri("http://example.com/v1/chat/completions")
			.body(http::Body::from(
				r#"{"model":"codex-subscription-auth","messages":[]}"#,
			))
			.expect("valid request");

		let ResolveResult::DirectResponse(response) = router.resolve(&mut req).await else {
			panic!("control model must not resolve a backend");
		};
		assert_eq!(response.status(), ::http::StatusCode::OK);
		let response = response_json(response).await;
		assert_eq!(response["object"], "chat.completion");
		assert_eq!(response["model"], CODEX_SUBSCRIPTION_AUTH_MODEL);
		assert_eq!(response["choices"][0]["message"]["role"], "assistant");
		let instruction: Value = serde_json::from_str(
			response["choices"][0]["message"]["content"]
				.as_str()
				.expect("assistant content"),
		)
		.expect("JSON-safe instruction");
		assert_eq!(instruction["status"], "pending");
		assert_eq!(instruction["user_code"], "ABCD-EFGH");
		assert!(instruction.get("access_token").is_none());
		assert!(instruction.get("refresh_token").is_none());
	}

	#[tokio::test]
	async fn codex_subscription_auth_returns_responses_control_response() {
		let router = ModelRouter::new(vec![], vec![]).with_codex_subscription_auth(Arc::new(
			TestCodexSubscriptionAuth(CodexSubscriptionAuthState::Authorized),
		));
		let mut req = ::http::Request::builder()
			.method(::http::Method::POST)
			.uri("http://example.com/v1/responses")
			.body(http::Body::from(
				r#"{"model":"codex-subscription-auth","input":"authorize"}"#,
			))
			.expect("valid request");

		let ResolveResult::DirectResponse(response) = router.resolve(&mut req).await else {
			panic!("control model must not resolve a backend");
		};
		let response = response_json(response).await;
		assert_eq!(response["object"], "response");
		assert_eq!(response["status"], "completed");
		assert_eq!(response["output"][0]["role"], "assistant");
		let instruction: Value = serde_json::from_str(
			response["output"][0]["content"][0]["text"]
				.as_str()
				.expect("assistant output text"),
		)
		.expect("JSON-safe instruction");
		assert_eq!(instruction["status"], "authorized");
	}

	#[tokio::test]
	async fn codex_oauth_manager_adapter_starts_and_polls_device_authorization() {
		let manager = crate::llm::codex_oauth::Manager::new(
			Arc::new(PendingThenDeniedCodexEndpoint),
			Arc::new(crate::llm::codex_oauth::MemoryCredentialStore::default()),
		);

		assert!(matches!(
			CodexSubscriptionAuth::start_or_poll(&manager).await,
			CodexSubscriptionAuthState::Pending {
				ref verification_url,
				ref user_code,
				expires_in_seconds,
			} if verification_url == "https://auth.example.test/device"
				&& user_code == "USER-CODE"
				&& expires_in_seconds > 0
		));
		assert_eq!(
			CodexSubscriptionAuth::start_or_poll(&manager).await,
			CodexSubscriptionAuthState::Denied
		);
	}

	#[tokio::test]
	async fn codex_subscription_auth_rejects_streaming_without_calling_hook() {
		let router = ModelRouter::new(vec![], vec![]);
		let mut req = ::http::Request::builder()
			.method(::http::Method::POST)
			.uri("http://example.com/v1/chat/completions")
			.body(http::Body::from(
				r#"{"model":"codex-subscription-auth","messages":[],"stream":true}"#,
			))
			.expect("valid request");

		let ResolveResult::DirectResponse(response) = router.resolve(&mut req).await else {
			panic!("streaming control model must not resolve a backend");
		};
		assert_eq!(response.status(), ::http::StatusCode::BAD_REQUEST);
		let response = response_json(response).await;
		assert_eq!(
			response["error"]["code"],
			"codex_subscription_auth_streaming_unsupported"
		);
	}

	#[tokio::test]
	async fn normal_models_keep_existing_resolution_behavior() {
		let auth = Arc::new(CountingCodexSubscriptionAuth::default());
		let router = ModelRouter::new(vec![], vec![]).with_codex_subscription_auth(auth.clone());
		let mut req = ::http::Request::builder()
			.method(::http::Method::POST)
			.uri("http://example.com/v1/chat/completions")
			.body(http::Body::from(
				r#"{"model":"normal-model","messages":[]}"#,
			))
			.expect("valid request");

		let ResolveResult::DirectResponse(response) = router.resolve(&mut req).await else {
			panic!("unknown normal model must remain a direct not-found response");
		};
		assert_eq!(response.status(), ::http::StatusCode::NOT_FOUND);
		let response = response_json(response).await;
		assert_eq!(response["error"]["code"], "model_not_found");
		assert_eq!(auth.0.load(Ordering::Relaxed), 0);
	}

	#[tokio::test]
	async fn model_list_collects_all_public_wildcard_catalog_routes() {
		let model = |name: &str| ModelRoute {
			id: None,
			name: name.to_string(),
			created: 0,
			visibility: ModelVisibility::Public,
			header_matches: vec![],
			backend: RouteBackendReference {
				weight: 1,
				target: RouteBackendTarget::Invalid,
				inline_policies: vec![],
			},
			policies: ModelRoutePolicies {
				llm: default_route_types(),
				authorization: None,
			},
			backend_policies: vec![],
		};
		let router = ModelRouter::new(
			vec![model("provider/*"), model("static-model"), model("*")],
			vec![],
		);
		let mut req = ::http::Request::builder()
			.method(::http::Method::GET)
			.uri("/v1/models")
			.body(http::Body::empty())
			.expect("valid model-list request");

		let ResolveResult::ModelList(list) = router.resolve(&mut req).await else {
			panic!("wildcard routes must contribute dynamic catalogs");
		};
		assert_eq!(list.dynamic_backends.len(), 2);
		assert_eq!(
			list
				.static_models
				.iter()
				.map(|model| model["id"].as_str().unwrap())
				.collect::<Vec<_>>(),
			["static-model"]
		);
		assert!(list.dynamic_backends.iter().all(|source| !source.required));
	}

	#[test]
	fn exact_and_codex_routes_precede_a_generic_catalog_fallback() {
		let model = |name: &str, backend: &str| ModelRoute {
			id: None,
			name: name.to_string(),
			created: 0,
			visibility: ModelVisibility::Public,
			header_matches: vec![],
			backend: RouteBackendReference {
				weight: 1,
				target: RouteBackendTarget::Backend(strng::new(backend)),
				inline_policies: vec![],
			},
			policies: ModelRoutePolicies {
				llm: default_route_types(),
				authorization: None,
			},
			backend_policies: vec![],
		};
		let codex_backend = "codex";
		let router = ModelRouter::new(
			vec![
				model("*", "tii"),
				model("moonshotai/Kimi-K3", "kimi"),
				model(llm::codex_subscription::MODEL_PATTERN, codex_backend),
			],
			vec![],
		)
		.with_codex_namespaces(|backend| {
			matches!(&backend.target, RouteBackendTarget::Backend(key) if key.as_str() == codex_backend)
		});
		let req = ::http::Request::builder()
			.uri("/v1/responses")
			.body(http::Body::empty())
			.expect("valid request");

		for (name, expected_backend) in [
			("moonshotai/Kimi-K3", "kimi"),
			("openai/gpt-6-sol", "codex"),
			("other-model", "tii"),
		] {
			let resolved = router
				.resolve_concrete_model(name, false, &req)
				.unwrap()
				.expect("a model route exists");
			assert!(matches!(
				resolved.backend.target,
				RouteBackendTarget::Backend(ref key) if key.as_str() == expected_backend
			));
		}

		let mut hidden = model("internal-model", "hidden");
		hidden.visibility = ModelVisibility::Internal;
		let router = ModelRouter::new(vec![model("*", "tii"), hidden], vec![]);
		assert!(
			router
				.resolve_concrete_model("internal-model", false, &req)
				.unwrap()
				.is_none()
		);
	}

	#[tokio::test]
	async fn conditional_virtual_model_can_use_llm_request() {
		let model = |name: &str| ModelRoute {
			id: None,
			name: name.to_string(),
			created: 0,
			visibility: ModelVisibility::Internal,
			header_matches: vec![],
			backend: RouteBackendReference {
				weight: 1,
				target: RouteBackendTarget::Invalid,
				inline_policies: vec![],
			},
			policies: ModelRoutePolicies {
				llm: default_route_types(),
				authorization: None,
			},
			backend_policies: vec![],
		};
		let router = ModelRouter::new(
			vec![model("economy-model"), model("premium-model")],
			vec![VirtualModelRoute {
				name: "smart-model".to_string(),
				created: 0,
				llm_policy: default_route_types(),
				routing: VirtualModelRouting::Conditional(vec![
					ConditionalTarget {
						model: "economy-model".to_string(),
						when: Some(Arc::new(
							cel::Expression::new_strict("llmRequest.max_tokens <= 1024")
								.expect("valid CEL expression"),
						)),
					},
					ConditionalTarget {
						model: "premium-model".to_string(),
						when: None,
					},
				]),
			}],
		);
		let mut req = ::http::Request::builder()
			.uri("http://example.com/v1/chat/completions")
			.body(http::Body::from(
				r#"{"model":"smart-model","max_tokens":256}"#,
			))
			.expect("valid request");

		assert!(matches!(
			router.resolve(&mut req).await,
			ResolveResult::Backend(_)
		));
		let body = http::read_body_with_limit(req.into_body(), 1024)
			.await
			.expect("rewritten request body");
		let body: Value = serde_json::from_slice(&body).expect("valid JSON request body");
		assert_eq!(body["model"], "economy-model");
	}

	#[test]
	fn concrete_model_authorization_filters_requests() {
		let authorization = Authorization(Arc::new(crate::http::authorization::RuleSet::new(
			crate::http::authorization::PolicySet::new(
				vec![Arc::new(
					cel::Expression::new_strict("request.headers['x-model-access'] == 'allowed'".to_string())
						.expect("valid CEL expression"),
				)],
				vec![],
				vec![],
			),
		)));
		let model = ModelRoute {
			id: None,
			name: "gpt-5-mini".to_string(),
			created: 0,
			visibility: ModelVisibility::Public,
			header_matches: vec![],
			backend: RouteBackendReference {
				weight: 1,
				target: RouteBackendTarget::Invalid,
				inline_policies: vec![],
			},
			policies: ModelRoutePolicies {
				llm: default_route_types(),
				authorization: Some(authorization),
			},
			backend_policies: vec![],
		};

		let allowed = ::http::Request::builder()
			.uri("http://example.com/v1/chat/completions")
			.header("x-model-access", "allowed")
			.body(http::Body::empty())
			.expect("valid request");
		let denied = ::http::Request::builder()
			.uri("http://example.com/v1/chat/completions")
			.body(http::Body::empty())
			.expect("valid request");

		assert!(model_authorized(&model, &allowed));
		assert!(!model_authorized(&model, &denied));
	}

	#[test]
	fn rewrite_path_model_rewrites_bedrock_converse_and_preserves_suffix() {
		assert_eq!(
			rewrite_path_model(
				"/model/anthropic.claude-3-5-sonnet-20241022-v2:0/converse",
				"anthropic.claude-3-haiku-20240307-v1:0",
			)
			.as_deref(),
			Some("/model/anthropic.claude-3-haiku-20240307-v1:0/converse")
		);
	}

	#[test]
	fn rewrite_path_model_rewrites_bedrock_invoke_and_encodes_slashes() {
		assert_eq!(
			rewrite_path_model(
				"/model/virtual/invoke-with-response-stream",
				"arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/my-profile",
			)
			.as_deref(),
			Some(
				"/model/arn:aws:bedrock:us-east-1:123456789012:application-inference-profile%2Fmy-profile/invoke-with-response-stream"
			)
		);
	}

	#[test]
	fn rewrite_path_model_rewrites_vertex_raw_predict() {
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/us/publishers/anthropic/models/virtual:rawPredict",
				"claude-sonnet",
			)
			.as_deref(),
			Some("/v1/projects/p/locations/us/publishers/anthropic/models/claude-sonnet:rawPredict")
		);
	}

	#[test]
	fn rewrite_path_model_rewrites_vertex_raw_predict_for_non_anthropic_publishers() {
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/us/publishers/google/models/virtual:rawPredict",
				"gemini-2.0-flash",
			)
			.as_deref(),
			Some("/v1/projects/p/locations/us/publishers/google/models/gemini-2.0-flash:rawPredict")
		);
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/us/publishers/meta/models/virtual:streamRawPredict",
				"llama-3.1-70b",
			)
			.as_deref(),
			Some("/v1/projects/p/locations/us/publishers/meta/models/llama-3.1-70b:streamRawPredict")
		);
	}

	#[test]
	fn rewrite_path_model_rewrites_vertex_gemini_paths() {
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/global/publishers/google/models/virtual:generateContent",
				"gemini-2.5-flash",
			)
			.as_deref(),
			Some(
				"/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
			)
		);
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/global/publishers/google/models/virtual:streamGenerateContent",
				"gemini-2.5-flash",
			)
			.as_deref(),
			Some(
				"/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent"
			)
		);
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/global/publishers/google/models/virtual:countTokens",
				"gemini-2.5-flash",
			)
			.as_deref(),
			Some("/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:countTokens")
		);
	}

	#[test]
	fn rewrite_path_model_rewrites_bare_gemini_api_paths() {
		assert_eq!(
			rewrite_path_model("/v1beta/models/virtual:generateContent", "gemini-2.5-pro").as_deref(),
			Some("/v1beta/models/gemini-2.5-pro:generateContent")
		);
		assert_eq!(
			rewrite_path_model(
				"/v1beta/models/virtual:streamGenerateContent",
				"gemini-2.5-pro"
			)
			.as_deref(),
			Some("/v1beta/models/gemini-2.5-pro:streamGenerateContent")
		);
		assert_eq!(
			rewrite_path_model("/v1beta/models/virtual:countTokens", "gemini-2.5-pro").as_deref(),
			Some("/v1beta/models/gemini-2.5-pro:countTokens")
		);
	}

	#[test]
	fn rewrite_path_model_encodes_slashes_in_gemini_targets() {
		// Vertex tuned/global endpoints are addressed by resource name, which must stay in a single
		// path segment.
		assert_eq!(
			rewrite_path_model("/v1beta/models/virtual:generateContent", "tunedModels/abc").as_deref(),
			Some("/v1beta/models/tunedModels%2Fabc:generateContent")
		);
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/global/publishers/google/models/virtual:generateContent",
				"tunedModels/abc",
			)
			.as_deref(),
			Some(
				"/v1/projects/p/locations/global/publishers/google/models/tunedModels%2Fabc:generateContent"
			)
		);
	}

	#[test]
	fn rewrite_path_model_ignores_gemini_shaped_paths_it_cannot_parse() {
		// No `/models/` segment, and a publisher path missing its publisher: rewriting would
		// fabricate a path, so both must no-op and leave the client's URI alone.
		assert_eq!(
			rewrite_path_model(
				"/v1beta/tunedModels/virtual:generateContent",
				"gemini-2.5-flash"
			),
			None
		);
		assert_eq!(
			rewrite_path_model(
				"/v1/projects/p/locations/global/publishers//models/virtual:countTokens",
				"gemini-2.5-flash",
			),
			None
		);
		assert_eq!(
			rewrite_path_model("/v1beta/models/virtual:embedContent", "gemini-2.5-flash"),
			None
		);
	}

	#[test]
	fn rewrite_uri_model_preserves_alt_sse_on_gemini_streams() {
		// The streaming route is only SSE because of `?alt=sse`; a virtual-model rewrite that
		// dropped it would flip the upstream to the JSON-array variant.
		let mut req = ::http::Request::builder()
			.uri("http://example.com/v1beta/models/virtual:streamGenerateContent?alt=sse&key=abc")
			.body(http::Body::empty())
			.unwrap();
		rewrite_uri_model(&mut req, "gemini-2.5-flash").expect("URI rewrites");
		assert_eq!(
			req.uri().to_string(),
			"http://example.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=abc"
		);
	}

	#[test]
	fn rewrite_uri_model_preserves_query() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/model/virtual/converse?trace=true")
			.body(http::Body::empty())
			.unwrap();
		rewrite_uri_model(&mut req, "real/model").expect("URI rewrites");
		assert_eq!(
			req.uri().to_string(),
			"http://example.com/model/real%2Fmodel/converse?trace=true"
		);
	}

	#[tokio::test]
	async fn body_bytes_rejects_json_body_over_buffer_limit() {
		let request_body = br#"{"model":"real-model","messages":[{"role":"user","content":"this part is over the limit"}]}"#;
		let mut req = ::http::Request::builder()
			.uri("http://example.com/v1/chat/completions")
			.body(http::Body::from(request_body.as_slice()))
			.unwrap();
		req.extensions_mut().insert(BufferLimit(24));

		let resp = *body_bytes(&mut req)
			.await
			.expect_err("over-limit body should fail");
		assert_eq!(resp.status(), ::http::StatusCode::PAYLOAD_TOO_LARGE);
		let error_body = http::read_body_with_limit(resp.into_body(), 1024)
			.await
			.expect("error body");
		let error: Value = serde_json::from_slice(&error_body).expect("error JSON");
		assert_eq!(error["error"]["code"], "request_body_too_large");

		let restored = http::read_body_with_limit(req.into_body(), 1024)
			.await
			.expect("restored request body");
		assert_eq!(restored, Bytes::from_static(request_body));
	}

	#[tokio::test]
	async fn requested_model_decodes_gzip_body() {
		let body = br#"{"model":"claude-opus-4-8","messages":[]}"#;
		let compressed = http::compression::encode_body(body, "gzip")
			.await
			.expect("gzip encode");
		let mut req = ::http::Request::builder()
			.uri("http://example.com/v1/messages")
			.header(::http::header::CONTENT_ENCODING, "gzip")
			.header(::http::header::CONTENT_LENGTH, compressed.len())
			.body(http::Body::from(compressed))
			.unwrap();

		let requested = requested_model(&mut req)
			.await
			.expect("gzip request body should decode");
		assert_eq!(requested.model, "claude-opus-4-8");
		assert!(!req.headers().contains_key(::http::header::CONTENT_ENCODING));
		assert!(!req.headers().contains_key(::http::header::CONTENT_LENGTH));
		assert_eq!(
			http::read_body_with_limit(req.into_body(), 1024)
				.await
				.expect("decompressed request body"),
			Bytes::from_static(body)
		);
	}

	#[tokio::test]
	async fn requested_model_reads_gemini_paths_without_touching_the_body() {
		// The Gemini body carries no `model`, so the router has to take it from the path — and must
		// leave the body untouched, since it is what reaches the upstream verbatim.
		let body = br#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;
		for uri in [
			"http://example.com/v1beta/models/gemini-2.5-flash:generateContent",
			"http://example.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
			"http://example.com/v1beta/models/gemini-2.5-flash:countTokens",
			"http://example.com/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:generateContent",
		] {
			let mut req = ::http::Request::builder()
				.uri(uri)
				.body(http::Body::from(body.as_slice()))
				.unwrap();

			let requested = requested_model(&mut req)
				.await
				.expect("the model rides the Gemini path");
			assert_eq!(requested.model, "gemini-2.5-flash", "{uri}");
			assert!(matches!(requested.location, RequestedModelLocation::Path));
			assert_eq!(
				http::read_body_with_limit(req.into_body(), 1024)
					.await
					.expect("request body"),
				Bytes::from_static(body),
				"{uri}"
			);
		}
	}

	#[test]
	fn default_routes_resolve_gemini_suffixes() {
		let policy = default_route_types();
		assert_eq!(
			policy.resolve_route("/v1beta/models/gemini-2.5-flash:generateContent"),
			llm::RouteType::GenerateContent
		);
		assert_eq!(
			policy.resolve_route("/v1beta/models/gemini-2.5-flash:streamGenerateContent"),
			llm::RouteType::GenerateContent
		);
		assert_eq!(
			policy.resolve_route("/v1beta/models/gemini-2.5-flash:countTokens"),
			llm::RouteType::GeminiCountTokens
		);
		assert_eq!(
			policy.resolve_route(
				"/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-pro:generateContent"
			),
			llm::RouteType::GenerateContent
		);
	}

	#[test]
	fn default_routes_resolve_gemini_stream_ignoring_query() {
		// The dispatcher matches on `uri.path()`, so the `?alt=sse` the Gemini SDKs append to the
		// streaming endpoint never reaches the suffix matcher.
		let uri: ::http::Uri = "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
			.parse()
			.expect("valid uri");
		assert_eq!(
			default_route_types().resolve_route(uri.path()),
			llm::RouteType::GenerateContent
		);
	}

	#[test]
	fn stream_generate_content_does_not_match_generate_content() {
		// `:generateContent` is not a suffix of `:streamGenerateContent`, so the two entries are
		// independent even before longest-suffix-first ordering applies. Point them at different
		// route types so a mis-resolution would be visible.
		let policy = llm::Policy {
			routes: [
				(strng::new(":generateContent"), llm::RouteType::Passthrough),
				(
					strng::new(":streamGenerateContent"),
					llm::RouteType::GenerateContent,
				),
			]
			.into_iter()
			.collect(),
			..Default::default()
		};
		assert_eq!(
			policy.resolve_route("/v1beta/models/gemini-2.5-flash:streamGenerateContent"),
			llm::RouteType::GenerateContent
		);
		assert_eq!(
			policy.resolve_route("/v1beta/models/gemini-2.5-flash:generateContent"),
			llm::RouteType::Passthrough
		);
	}

	#[test]
	fn default_routes_preserve_existing_suffixes() {
		let policy = default_route_types();
		assert_eq!(policy.resolve_route("/v1/models"), llm::RouteType::Models);
		assert_eq!(
			policy.resolve_route("/v1/projects/p/locations/us/publishers/anthropic/models/m:rawPredict"),
			llm::RouteType::Messages
		);
		assert_eq!(
			policy.resolve_route(
				"/v1/projects/p/locations/us/publishers/anthropic/models/m:streamRawPredict"
			),
			llm::RouteType::Messages
		);
		assert_eq!(
			policy.resolve_route("/v1/messages"),
			llm::RouteType::Messages
		);
		assert_eq!(
			policy.resolve_route("/v1/chat/completions"),
			llm::RouteType::Completions
		);
		assert_eq!(
			policy.resolve_route("/v1/anything/else"),
			llm::RouteType::Passthrough
		);
	}

	#[test]
	fn codex_wildcards_require_public_namespace() {
		let wildcard = ModelRoute {
			id: None,
			name: "*".into(),
			created: 0,
			visibility: ModelVisibility::Public,
			header_matches: vec![],
			backend: RouteBackendReference {
				weight: 1,
				target: RouteBackendTarget::Invalid,
				inline_policies: vec![],
			},
			policies: ModelRoutePolicies {
				llm: default_route_types(),
				authorization: None,
			},
			backend_policies: vec![],
		};
		let req = ::http::Request::builder()
			.uri("/v1/responses")
			.body(http::Body::empty())
			.unwrap();
		let generic = ModelRouter::new(vec![wildcard.clone()], vec![]);
		assert!(
			generic
				.resolve_concrete_model("other/model", false, &req)
				.unwrap()
				.is_some()
		);
		let router = generic.with_codex_namespaces(|_| true);
		for model in ["gpt-test", "other/gpt-test", "openai/"] {
			assert!(
				router
					.resolve_concrete_model(model, false, &req)
					.unwrap()
					.is_none()
			);
		}
		assert!(
			router
				.resolve_concrete_model("openai/gpt-test", false, &req)
				.unwrap()
				.is_some()
		);
		let mut exact = wildcard.clone();
		exact.name = "Qwen/model".into();
		exact.backend.weight = 7;
		let router = ModelRouter::new(vec![exact, wildcard], vec![]).with_codex_namespaces(|_| true);
		assert_eq!(
			router
				.resolve_concrete_model("Qwen/model", false, &req)
				.unwrap()
				.unwrap()
				.backend
				.weight,
			7
		);
	}
}
