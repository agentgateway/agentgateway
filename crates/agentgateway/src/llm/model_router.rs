use std::sync::Arc;

use agent_core::prelude::Strng;
use agent_core::strng;
use anyhow::Context;
use bytes::Bytes;
use rand::seq::IndexedRandom;
use serde_json::Value;

use crate::http::transformation_cel::TransformationMetadata;
use crate::http::{self, Request, Response};
use crate::types::agent::{
	BackendReference, BackendTrafficPolicy, HeaderMatch, HeaderValueMatch, RouteBackendReference,
	TrafficPolicy,
};
use crate::{apply, cel, schema_enum};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoute {
	pub name: String,
	pub visibility: ModelVisibility,
	pub header_matches: Vec<Vec<HeaderMatch>>,
	pub backend_key: Strng,
	pub route_policies: Vec<TrafficPolicy>,
	pub backend_policies: Vec<BackendTrafficPolicy>,
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualModelRoute {
	pub name: String,
	pub route_policies: Vec<TrafficPolicy>,
	pub routing: VirtualModelRouting,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VirtualModelRouting {
	Weighted(Vec<WeightedTarget>),
	Failover { backend_key: Strng },
	Conditional(Vec<ConditionalTarget>),
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightedTarget {
	pub model: String,
	pub weight: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalTarget {
	pub model: String,
	pub when: Option<Arc<cel::Expression>>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouter {
	models: Vec<ModelRoute>,
	virtual_models: Vec<VirtualModelRoute>,
	created: u64,
}

#[derive(Debug, Clone)]
pub struct ResolvedBackend {
	pub backend: RouteBackendReference,
	pub route_policies: Vec<TrafficPolicy>,
}

pub enum ResolveResult {
	DirectResponse(Response),
	Backend(ResolvedBackend),
}

impl ModelRouter {
	pub fn new(
		models: Vec<ModelRoute>,
		virtual_models: Vec<VirtualModelRoute>,
		created: u64,
	) -> Self {
		Self {
			models,
			virtual_models,
			created,
		}
	}

	pub async fn resolve(&self, req: &mut Request) -> anyhow::Result<ResolveResult> {
		if is_model_list_request(req) {
			return Ok(ResolveResult::DirectResponse(
				self.model_list_response(req)?,
			));
		}
		let requested_model = match requested_model(req).await {
			Ok(model) => model,
			Err(err) => {
				tracing::debug!(%err, "failed to read LLM request model");
				return Ok(ResolveResult::DirectResponse(model_not_found_response()));
			},
		};
		req
			.extensions_mut()
			.get_or_insert_with(TransformationMetadata::default)
			.0
			.insert(
				"agentgateway_user_model".to_string(),
				Value::String(requested_model.clone()),
			);
		if let Some(virtual_model) = self
			.virtual_models
			.iter()
			.find(|model| model.name == requested_model)
		{
			return Ok(match self.resolve_virtual_model(virtual_model, req).await {
				Ok(backend) => ResolveResult::Backend(backend),
				Err(err) => {
					tracing::debug!(%err, "failed to resolve LLM virtual model");
					ResolveResult::DirectResponse(model_not_found_response())
				},
			});
		}

		Ok(
			match self.resolve_concrete_model(&requested_model, false, req) {
				Ok(route) => ResolveResult::Backend(route),
				Err(err) => {
					tracing::debug!(%err, "failed to resolve LLM model");
					ResolveResult::DirectResponse(model_not_found_response())
				},
			},
		)
	}

	fn model_list_response(&self, req: &Request) -> anyhow::Result<Response> {
		let data = self
			.models
			.iter()
			.filter(|model| model.visibility == ModelVisibility::Public)
			.filter(|model| model_authorized(model, req))
			.map(|model| model_list_entry(&model.name, self.created))
			.chain(
				self
					.virtual_models
					.iter()
					.map(|model| model_list_entry(&model.name, self.created)),
			)
			.collect::<Vec<_>>();
		let body = serde_json::to_vec(&serde_json::json!({
			"data": data,
			"object": "list",
		}))?;
		Ok(
			::http::Response::builder()
				.status(::http::StatusCode::OK)
				.header(::http::header::CONTENT_TYPE, "application/json")
				.body(http::Body::from(body))?,
		)
	}

	async fn resolve_virtual_model(
		&self,
		virtual_model: &VirtualModelRoute,
		req: &mut Request,
	) -> anyhow::Result<ResolvedBackend> {
		let target = match &virtual_model.routing {
			VirtualModelRouting::Weighted(targets) => targets
				.choose_weighted(&mut rand::rng(), |target| target.weight)
				.map(|target| target.model.clone())
				.context("virtual model has no valid weighted targets")?,
			VirtualModelRouting::Failover { backend_key } => {
				return Ok(ResolvedBackend {
					backend: RouteBackendReference {
						weight: 1,
						target: BackendReference::Backend(strng::format!("/{}", backend_key)).into(),
						inline_policies: vec![],
					},
					route_policies: virtual_model.route_policies.clone(),
				});
			},
			VirtualModelRouting::Conditional(targets) => {
				let exec = cel::Executor::new_request(req);
				targets
					.iter()
					.find(|target| {
						target
							.when
							.as_ref()
							.map(|expr| exec.eval_bool(expr))
							.unwrap_or(true)
					})
					.map(|target| target.model.clone())
					.context("virtual model did not match any conditional target")?
			},
		};
		rewrite_body_model(req, &target).await?;
		self
			.resolve_concrete_model(&target, true, req)
			.with_context(|| format!("failed to resolve LLM virtual model target {target}"))
	}

	fn resolve_concrete_model(
		&self,
		requested_model: &str,
		allow_internal: bool,
		req: &Request,
	) -> anyhow::Result<ResolvedBackend> {
		let model = self
			.models
			.iter()
			.find(|model| {
				(allow_internal || model.visibility == ModelVisibility::Public)
					&& model_name_matches(&model.name, requested_model)
					&& header_matches(&model.header_matches, req)
			})
			.with_context(|| format!("model not found: {requested_model}"))?;
		Ok(ResolvedBackend {
			backend: RouteBackendReference {
				weight: 1,
				target: BackendReference::Backend(strng::format!("/{}", model.backend_key)).into(),
				inline_policies: model.backend_policies.clone(),
			},
			route_policies: model.route_policies.clone(),
		})
	}
}

fn model_not_found_response() -> Response {
	::http::Response::builder()
		.status(::http::StatusCode::NOT_FOUND)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			r#"{"error":{"message":"Model not found","type":"invalid_request_error","code":"model_not_found"}}"#,
		))
		.expect("static LLM model not found response is valid")
}

fn model_authorized(model: &ModelRoute, req: &Request) -> bool {
	let rules = model
		.route_policies
		.iter()
		.filter_map(|policy| match policy {
			TrafficPolicy::Authorization(authorization) => Some(authorization.0.clone()),
			_ => None,
		})
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

fn model_list_entry(id: &str, created: u64) -> serde_json::Value {
	serde_json::json!({
		"id": id,
		"object": "model",
		"created": created,
		"owned_by": "openai",
	})
}

fn is_model_list_request(req: &Request) -> bool {
	let path = req.uri().path().trim_end_matches('/');
	path == "/v1/models"
		|| path
			.strip_prefix("/v1/models")
			.is_some_and(|suffix| suffix.starts_with('/'))
		|| path == "/models"
		|| path
			.strip_prefix("/models")
			.is_some_and(|suffix| suffix.starts_with('/'))
}

fn header_matches(matches: &[Vec<HeaderMatch>], req: &Request) -> bool {
	if matches.is_empty() {
		return true;
	}
	matches.iter().any(|headers| headers_match(headers, req))
}

fn headers_match(headers: &[HeaderMatch], req: &Request) -> bool {
	for HeaderMatch { name, value } in headers {
		let Some(have) = http::get_pseudo_or_header_value(name, req) else {
			return false;
		};
		match value {
			HeaderValueMatch::Exact(want) => {
				if have.as_ref() != *want {
					return false;
				}
			},
			HeaderValueMatch::Regex(want) => {
				let Some(have_str) = have.to_str().ok() else {
					return false;
				};
				let Some(m) = want.find(have_str) else {
					return false;
				};
				if !(m.start() == 0 && m.end() == have_str.len()) {
					return false;
				}
			},
			HeaderValueMatch::Invalid => return false,
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

async fn requested_model(req: &mut Request) -> anyhow::Result<String> {
	let path = req.uri().path();
	if path.ends_with(":streamRawPredict") || path.ends_with(":rawPredict") {
		return path
			.rsplit_once("/publishers/anthropic/models/")
			.and_then(|(_, rest)| rest.split_once(':'))
			.map(|(model, _)| model.to_string())
			.context("missing vertex anthropic model in request path");
	}
	if path.ends_with("/invoke-with-response-stream") || path.ends_with("/invoke") {
		return path
			.rsplit_once("/model/")
			.and_then(|(_, rest)| rest.split_once("/invoke"))
			.map(|(model, _)| model.to_string())
			.context("missing bedrock model in request path");
	}

	let body = body_bytes(req).await?;
	let body: Value = serde_json::from_slice(&body).context("failed to parse LLM request body")?;
	body
		.get("model")
		.and_then(Value::as_str)
		.map(ToString::to_string)
		.context("LLM request body is missing string field 'model'")
}

async fn rewrite_body_model(req: &mut Request, target: &str) -> anyhow::Result<()> {
	let body = body_bytes(req).await?;
	if body.is_empty() {
		return Ok(());
	}
	let Ok(mut json) = serde_json::from_slice::<Value>(&body) else {
		return Ok(());
	};
	let Some(obj) = json.as_object_mut() else {
		return Ok(());
	};
	obj.insert("model".to_string(), Value::String(target.to_string()));
	let body = serde_json::to_vec(&json)?;
	*req.body_mut() = http::Body::from(body);
	req.headers_mut().remove(::http::header::CONTENT_LENGTH);
	req.extensions_mut().remove::<cel::BufferedBody>();
	Ok(())
}

async fn body_bytes(req: &mut Request) -> anyhow::Result<Bytes> {
	if let Some(body) = req.extensions().get::<cel::BufferedBody>() {
		return Ok(body.0.clone());
	}
	let body = http::inspect_body(req).await?;
	req.extensions_mut().insert(cel::BufferedBody(body.clone()));
	Ok(body)
}
