use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::http::header::CONTENT_TYPE;
use ::http::{HeaderMap, HeaderValue, Method};
use anyhow::{Context, bail};
use bytes::Bytes;
use itertools::Itertools;
use quick_cache::sync::Cache;
use serde_json::Value as JsonValue;

use crate::cel::{Expression, Value};
use crate::http::ext_authz::{CacheConfig, CacheKey, effective_cache_entries};
use crate::http::filters::BackendRequestTimeout;
use crate::http::{HeaderOrPseudo, HeaderOrPseudoValue, Request, RequestOrResponse};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::SimpleBackendReferenceWithPoliciesAndPath;
use crate::*;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Selects a virtual model's target by calling an external HTTP service.
#[apply(schema!)]
pub struct VirtualModelCallout {
	/// Service that selects the model, and the backend policies used when connecting to it.
	/// A `host` URL may include the request path, such as `https://router.example.com/v1/route`.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPoliciesAndPath,
	/// Headers to set on the callout request, computed from CEL expressions.
	/// Keys may be header names or the `:path`, `:method`, and `:authority` pseudo-headers.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde_as(as = "serde_with::Map<_, _>")]
	pub headers: Vec<(HeaderOrPseudo, Arc<Expression>)>,
	/// CEL expression that computes the callout request body.
	/// Strings and bytes are used directly; other values are JSON-encoded.
	/// If unset, the original request body is forwarded.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<Arc<Expression>>,
	/// CEL expressions that compute request payload fields from the callout response, overriding existing values.
	/// `callout.headers` holds the response headers and `callout.body` the decoded JSON response body.
	/// `model` is required and selects the target from `llm.models`.
	pub transformation: HashMap<String, Arc<Expression>>,
	/// Behavior when the callout fails, returns a non-2xx or non-JSON response, or selects an unknown model.
	/// Defaults to `failClosed`.
	#[serde(default)]
	pub failure_mode: VirtualModelCalloutFailureMode,
	/// Reuse callout responses using CEL expressions as the cache key.
	/// On a cache hit, `transformation` is evaluated against the cached response.
	/// Keying on a session identifier makes routing sticky for that session.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cache: Option<CacheConfig>,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache_store: Option<Arc<Cache<CacheKey, CachedCallout>>>,
}

#[apply(schema!)]
#[derive(Default)]
pub enum VirtualModelCalloutFailureMode {
	/// Reject the request.
	#[default]
	FailClosed,
	/// Route to this model, which is resolved against llm.models, without applying `transformation`.
	Fallback(String),
}

#[derive(Clone, Debug)]
struct CachedCallout {
	expires_at: Instant,
	response: Arc<JsonValue>,
}

#[derive(Debug)]
pub struct CalloutSelection {
	pub model: String,
	/// Request payload fields to set. `None` removes the field.
	pub fields: Vec<(String, Option<JsonValue>)>,
}

impl VirtualModelCallout {
	pub fn with_configured_cache_store(mut self) -> Self {
		self.cache_store = self
			.cache
			.as_ref()
			.map(|cache| Arc::new(Cache::new(effective_cache_entries(cache.max_entries))));
		self
	}

	pub async fn select(
		&self,
		client: &PolicyClient,
		req: &mut Request,
		llm_request: Option<&JsonValue>,
	) -> anyhow::Result<CalloutSelection> {
		let cache_key = self.cache_key(req, llm_request);
		let response = match cache_key.as_ref().and_then(|key| self.cached(key)) {
			Some(response) => response,
			None => {
				let callout_req = self.build_request(req, llm_request)?;
				let response = Arc::new(self.call(client, callout_req).await?);
				if let Some(key) = cache_key {
					self.insert_cache(key, req, llm_request, &response);
				}
				response
			},
		};
		self.transform(req, llm_request, &response)
	}

	fn cache_key(&self, req: &Request, llm_request: Option<&JsonValue>) -> Option<CacheKey> {
		let cache = self.cache.as_ref()?;
		if cache.key.is_empty() {
			return None;
		}
		CacheKey::evaluate(&executor(req, llm_request), &cache.key)
			.inspect_err(|index| tracing::debug!(index, "callout cache key evaluation failed"))
			.ok()
	}

	fn cached(&self, key: &CacheKey) -> Option<Arc<JsonValue>> {
		let store = self.cache_store.as_ref()?;
		let cached = store.get(key)?;
		let now = Instant::now();
		if cached.expires_at <= now {
			store.remove_if(key, |cached| cached.expires_at <= now);
			return None;
		}
		Some(cached.response)
	}

	fn insert_cache(
		&self,
		key: CacheKey,
		req: &Request,
		llm_request: Option<&JsonValue>,
		response: &Arc<JsonValue>,
	) {
		let (Some(cache), Some(store)) = (&self.cache, &self.cache_store) else {
			return;
		};
		let mut exec = executor(req, llm_request);
		exec.callout = Some(response);
		let Some(expires_at) = cache
			.evaluate_ttl(&exec)
			.and_then(|ttl| Instant::now().checked_add(ttl))
		else {
			tracing::debug!("skip caching callout response; invalid TTL");
			return;
		};
		store.insert(
			key,
			CachedCallout {
				expires_at,
				response: response.clone(),
			},
		);
	}

	async fn call(&self, client: &PolicyClient, callout_req: Request) -> anyhow::Result<JsonValue> {
		let resp = client
			.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Callout)
			.call_reference_with_policies(
				callout_req,
				&self.target.target.target,
				self.target.target.policies.as_slice(),
			)
			.await?;
		if !resp.status().is_success() {
			bail!("callout returned status {}", resp.status());
		}
		let headers = headers_json(resp.headers());
		let body: JsonValue = json::from_response_body(resp).await?;
		Ok(serde_json::json!({ "headers": headers, "body": body }))
	}

	fn build_request(
		&self,
		req: &Request,
		llm_request: Option<&JsonValue>,
	) -> anyhow::Result<Request> {
		let exec = executor(req, llm_request);
		let body = match &self.body {
			Some(expr) => cel::value_as_byte_or_json(exec.eval(expr)?)?,
			None => llm_request
				.map(serde_json::to_vec)
				.transpose()?
				.map(Bytes::from)
				.unwrap_or_default(),
		};
		let path = self.target.path.as_ref().map_or("/", |path| path.as_str());
		let mut callout_req = ::http::Request::builder()
			.method(Method::POST)
			.uri(path)
			.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
			.body(http::Body::from(body))?;
		for (k, expr) in &self.headers {
			let value = HeaderOrPseudoValue::from_cel_result(k, exec.eval(expr).ok());
			RequestOrResponse::Request(&mut callout_req).apply_header(
				k,
				value,
				http::HeaderMutationAction::OverwriteIfExistsOrAdd,
			);
		}
		// Can be overridden by a timeout on the target's backend policies.
		callout_req
			.extensions_mut()
			.insert(BackendRequestTimeout(DEFAULT_TIMEOUT));
		Ok(callout_req)
	}

	fn transform(
		&self,
		req: &Request,
		llm_request: Option<&JsonValue>,
		response: &JsonValue,
	) -> anyhow::Result<CalloutSelection> {
		let mut exec = executor(req, llm_request);
		exec.callout = Some(response);
		let mut model = None;
		let mut fields = Vec::with_capacity(self.transformation.len());
		for (k, expr) in &self.transformation {
			if k == "model" {
				let Value::String(selected) = exec.eval(expr)? else {
					bail!("callout model must be a string");
				};
				model = Some(selected.to_string());
			} else {
				fields.push((k.clone(), exec.eval(expr).ok().and_then(|v| v.json().ok())));
			}
		}
		Ok(CalloutSelection {
			model: model.context("callout transformation must set model")?,
			fields,
		})
	}
}

fn executor<'a>(req: &'a Request, llm_request: Option<&'a JsonValue>) -> cel::Executor<'a> {
	match llm_request {
		Some(llm_request) => cel::Executor::new_llm_request(req, llm_request),
		None => cel::Executor::new_request(req),
	}
}

fn headers_json(headers: &HeaderMap) -> JsonValue {
	headers
		.keys()
		.map(|name| {
			let value = headers
				.get_all(name)
				.iter()
				.filter_map(|v| v.to_str().ok())
				.join(",");
			(name.as_str().to_string(), JsonValue::String(value))
		})
		.collect::<serde_json::Map<_, _>>()
		.into()
}
