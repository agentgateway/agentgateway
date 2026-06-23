use std::borrow::Cow;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::{Duration, Instant, SystemTime};

use agent_core::metrics::CustomField;
use agent_core::strng::{RichStrng, Strng};
use agent_core::telemetry::{
	OptionExt, OtelLogSink, ValueBag, current_connection_id, current_request_id, debug, display,
};
use agent_core::{Timestamp, strng};
use bytes::Buf;
use crossbeam::atomic::AtomicCell;
use frozen_collections::FzHashSet;
use http_body::{Body, Frame, SizeHint};
use indexmap::IndexMap;
use itertools::Itertools;
use opentelemetry::logs::{AnyValue, LogRecord as _, Logger, LoggerProvider as _, Severity};
use opentelemetry::trace::SpanKind;
use opentelemetry::{Key, KeyValue};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use rust_decimal::prelude::ToPrimitive;
use serde::de::DeserializeOwned;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use tracing::{Level, debug, trace};

use crate::cel::{ContextBuilder, Expression, LLMContext};
use crate::http::{Request, health};
use crate::llm::InputFormat;
use crate::llm::cost::{CostLookupStatus, ModelCatalog};
use crate::mcp::{MCPInfo, MCPOperation};
use crate::proxy::{ProxyResponseReason, dtrace};
use crate::telemetry::metrics::{
	CostCatalogLookupLabels, GenAILabels, GenAILabelsTokenUsage, HTTPLabels, MCPCall, Metrics,
	RouteIdentifier,
};
use crate::telemetry::trc::TraceParent;
use crate::telemetry::{log_store, trc};
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::{BackendInfo, BindKey, ListenerName, RouteName, Target};
use crate::types::loadbalancer::ActiveHandle;
use crate::{cel, llm, mcp};

fn u64_to_i64(value: Option<u64>) -> Option<i64> {
	value.map(|value| value.min(i64::MAX as u64) as i64)
}

fn u128_to_i64(value: u128) -> i64 {
	value.min(i64::MAX as u128) as i64
}

fn kv_to_json(kv: &[(&str, Option<ValueBag>)]) -> Value {
	let mut map = serde_json::Map::with_capacity(kv.len());
	for (key, value) in kv {
		if let Some(value) = value
			&& let Ok(value) = serde_json::to_value(value)
		{
			map.insert((*key).to_string(), value);
		}
	}
	Value::Object(map)
}

fn string_attribute(attributes: &Value, key: &str) -> Option<String> {
	attributes
		.get(key)
		.and_then(Value::as_str)
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(ToOwned::to_owned)
}

fn user_agent_name(req: Option<&cel::RequestSnapshot>) -> Option<String> {
	let value = req?
		.headers
		.get(::http::header::USER_AGENT)?
		.to_str()
		.ok()?
		.trim();
	if value.is_empty() {
		return None;
	}
	let end = value
		.find(|c: char| c == '/' || c.is_ascii_whitespace() || c == ';' || c == '(')
		.unwrap_or(value.len());
	let name = value[..end].trim();
	(!name.is_empty()).then(|| name.to_string())
}

fn api_key_name(req: Option<&cel::RequestSnapshot>) -> Option<String> {
	req?
		.api_key
		.as_ref()?
		.metadata
		.get("name")?
		.as_str()
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(ToOwned::to_owned)
}

/// AsyncLog is a wrapper around an item that can be atomically set.
/// The intent is to provide additional info to the log after we have lost the RequestLog reference,
/// generally for things that rely on the response body.
#[derive(Clone)]
pub struct AsyncLog<T>(Arc<AtomicCell<Option<T>>>);

impl<T: Clone> AsyncLog<T> {
	// load_clone is only a best-effort snapshot. It temporarily removes the value so it can clone it,
	// then stores the clone back. A concurrent store/non_atomic_mutate between those operations can be
	// lost when we restore `cur`, so callers must not rely on this as an atomic read.
	pub fn load_clone(&self) -> Option<T> {
		let cur = self.0.take();
		self.0.store(cur.clone());
		cur
	}
}

impl<T> AsyncLog<T> {
	// non_atomic_mutate is a racey method to modify the current value.
	// If there is no current value, a default is used.
	// This is NOT atomically safe; during the mutation, loads() on the item will be empty.
	// This is ok for our usage cases
	pub fn non_atomic_mutate(&self, f: impl FnOnce(&mut T)) {
		let Some(mut cur) = self.0.take() else {
			return;
		};
		f(&mut cur);
		self.0.store(Some(cur));
	}
}

impl<T> AsyncLog<T> {
	pub fn store(&self, v: Option<T>) {
		self.0.store(v)
	}
	pub fn take(&self) -> Option<T> {
		self.0.take()
	}
}

impl<T: Copy> AsyncLog<T> {
	pub fn load(&self) -> Option<T> {
		self.0.load()
	}
}

impl<T> Default for AsyncLog<T> {
	fn default() -> Self {
		AsyncLog(Arc::new(AtomicCell::new(None)))
	}
}

impl<T: Debug> Debug for AsyncLog<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AsyncLog").finish_non_exhaustive()
	}
}

#[derive(serde::Serialize, Debug, Default, Clone)]
pub struct MetricsConfig {
	pub metric_fields: MetricFields,
	pub excluded_metrics: FzHashSet<String>,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct Config {
	/// Deprecated: use frontendPolicies.accessLog
	pub filter: Option<Arc<cel::Expression>>,
	/// Deprecated: use frontendPolicies.accessLog
	pub fields: LoggingFields,
	/// Database-only request log fields.
	pub database_fields: LoggingFields,
	/// Level sets the level for logs
	pub level: String,
	/// Format sets the logging format (text or json)
	pub format: crate::LoggingFormat,
	/// Optional request log database sink.
	pub database: Option<crate::telemetry::log_store::Config>,
}

#[derive(serde::Serialize, Default, Clone, Debug)]
pub struct LoggingFields {
	pub remove: Arc<FzHashSet<String>>,
	pub add: Arc<OrderedStringMap<Arc<cel::Expression>>>,
}

#[derive(serde::Serialize, Default, Clone, Debug)]
pub struct MetricFields {
	pub add: Arc<OrderedStringMap<Arc<cel::Expression>>>,
}

#[derive(Clone, Debug)]
pub struct OrderedStringMap<V> {
	map: std::collections::HashMap<Box<str>, V>,
	order: Box<[Box<str>]>,
}

impl<V> OrderedStringMap<V> {}

impl<V> OrderedStringMap<V> {
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
	pub fn len(&self) -> usize {
		self.map.len()
	}
	pub fn contains_key(&self, k: &str) -> bool {
		self.map.contains_key(k)
	}
	pub fn values_unordered(&self) -> impl Iterator<Item = &V> {
		self.map.values()
	}
	pub fn iter(&self) -> impl Iterator<Item = (&Box<str>, &V)> {
		self
			.order
			.iter()
			.map(|k| (k, self.map.get(k).expect("key must be present")))
	}
}

impl<V> Default for OrderedStringMap<V> {
	fn default() -> Self {
		Self {
			map: Default::default(),
			order: Default::default(),
		}
	}
}

impl<V: Serialize> Serialize for OrderedStringMap<V> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let mut m = serializer.serialize_map(Some(self.len()))?;
		for (k, v) in self.iter() {
			m.serialize_entry(k.as_ref(), v)?;
		}
		m.end()
	}
}

impl<'de, V: DeserializeOwned> Deserialize<'de> for OrderedStringMap<V> {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let im = IndexMap::<String, V>::deserialize(deserializer)?;
		Ok(OrderedStringMap::from_iter(im))
	}
}

#[cfg(feature = "schema")]
impl<V: schemars::JsonSchema> schemars::JsonSchema for OrderedStringMap<V> {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		format!("OrderedStringMap_{}", V::schema_name()).into()
	}

	fn json_schema(schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
		<std::collections::BTreeMap<String, V>>::json_schema(schema_gen)
	}
}

impl<K, V> FromIterator<(K, V)> for OrderedStringMap<V>
where
	K: AsRef<str>,
{
	fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
		let items = iter.into_iter().collect_vec();
		let order: Box<[Box<str>]> = items.iter().map(|(k, _)| k.as_ref().into()).collect();
		let map: std::collections::HashMap<Box<str>, V> = items
			.into_iter()
			.map(|(k, v)| (k.as_ref().into(), v))
			.collect();
		Self { map, order }
	}
}

impl LoggingFields {
	pub fn has(&self, k: &str) -> bool {
		self.remove.contains(k) || self.add.contains_key(k)
	}
}

fn json_value_to_value_bag(v: &Value) -> ValueBag<'_> {
	// serde_json::Number::as_f64 also succeeds for integers; only convert numbers
	// that were actually stored as f64 so large integers stay exact.
	if let Value::Number(n) = v
		&& n.is_f64()
		&& let Some(f) = n.as_f64()
	{
		ValueBag::from_f64(f)
	} else {
		ValueBag::capture_serde1(v)
	}
}

fn original_model_from_metadata<'a>(
	req: Option<&'a cel::RequestSnapshot>,
	resp: Option<&'a cel::ResponseSnapshot>,
) -> Option<&'a str> {
	resp
		.and_then(|snapshot| snapshot.metadata.as_ref())
		.and_then(|metadata| metadata.0.get("agentgateway_user_model"))
		.or_else(|| {
			req
				.and_then(|snapshot| snapshot.metadata.as_ref())
				.and_then(|metadata| metadata.0.get("agentgateway_user_model"))
		})
		.and_then(Value::as_str)
}

#[derive(Debug, Default)]
pub struct TraceSampler {
	pub random_sampling: Option<Arc<cel::Expression>>,
	pub client_sampling: Option<Arc<cel::Expression>>,
}

impl TraceSampler {
	pub fn trace_sampled(&self, req: &Request, tp: Option<&TraceParent>) -> bool {
		let TraceSampler {
			random_sampling,
			client_sampling,
		} = &self;
		let expr = if tp.is_some() {
			let Some(cs) = client_sampling else {
				// If client_sampling is not set, default to include it
				return true;
			};
			cs
		} else {
			let Some(rs) = random_sampling else {
				// If random_sampling is not set, default to NOT include it
				return false;
			};
			rs
		};
		let exec = cel::Executor::new_request(req);
		exec.eval_rng(expr.as_ref())
	}
}

#[derive(Debug)]
pub struct CelLogging {
	pub cel_context: cel::ContextBuilder,
	pub filter: Option<Arc<cel::Expression>>,
	pub fields: LoggingFields,
	pub database_fields: LoggingFields,
	pub metric_fields: MetricFields,
}

pub struct CelLoggingExecutor<'a> {
	pub executor: cel::Executor<'a>,
	pub filter: &'a Option<Arc<cel::Expression>>,
	pub fields: &'a LoggingFields,
	pub database_fields: &'a LoggingFields,
	pub metric_fields: &'a MetricFields,
}

impl<'a> CelLoggingExecutor<'a> {
	fn eval_filter(&self) -> bool {
		match self.filter.as_deref() {
			Some(f) => self.executor.eval_bool(f),
			None => true,
		}
	}

	pub fn eval(
		&self,
		fields: &'a OrderedStringMap<Arc<Expression>>,
	) -> Vec<(Cow<str>, Option<Value>)> {
		self.eval_keep_empty(fields, false)
	}

	pub fn eval_keep_empty(
		&self,
		fields: &'a OrderedStringMap<Arc<Expression>>,
		keep_empty: bool,
	) -> Vec<(Cow<str>, Option<Value>)> {
		let mut raws = Vec::with_capacity(fields.len());
		for (k, v) in fields.iter() {
			let field = self.executor.eval(v.as_ref());
			if let Err(err) = &field {
				trace!(target: "cel", ?err, expression=?v, "expression failed");
			}
			if let Ok(cel::Value::Null) = &field {
				trace!(target: "cel",  expression=?v, "expression evaluated to null");
			}
			let celv = field.ok().filter(|v| !matches!(v, cel::Value::Null));

			// We return Option here to match the schema but don't bother adding None values since they
			// will be dropped anyways
			if let Some(celv) = celv {
				Self::resolve_value(&mut raws, Cow::Borrowed(k.as_ref()), &celv, false);
			} else if keep_empty {
				raws.push((Cow::Borrowed(k.as_ref()), None));
			}
		}
		raws
	}

	fn resolve_value(
		raws: &mut Vec<(Cow<'a, str>, Option<Value>)>,
		k: Cow<'a, str>,
		celv: &cel::Value,
		always_flatten: bool,
	) {
		match agent_celx::FlattenSignal::from_value(celv) {
			Some(agent_celx::FlattenSignal::List(li)) => {
				raws.reserve(li.len());
				for (idx, v) in li.as_ref().iter().enumerate() {
					Self::resolve_value(raws, Cow::Owned(format!("{k}.{idx}")), v, false);
				}
				return;
			},
			Some(agent_celx::FlattenSignal::ListRecursive(li)) => {
				raws.reserve(li.len());
				for (idx, v) in li.as_ref().iter().enumerate() {
					Self::resolve_value(raws, Cow::Owned(format!("{k}.{idx}")), v, true);
				}
				return;
			},
			Some(agent_celx::FlattenSignal::Map(m)) => {
				raws.reserve(m.len());
				for (mk, mv) in m.iter() {
					Self::resolve_value(raws, Cow::Owned(format!("{k}.{mk}")), mv, false);
				}
				return;
			},
			Some(agent_celx::FlattenSignal::MapRecursive(m)) => {
				raws.reserve(m.len());
				for (mk, mv) in m.iter() {
					Self::resolve_value(raws, Cow::Owned(format!("{k}.{mk}")), mv, true);
				}
				return;
			},
			None => {},
		}

		if always_flatten {
			match celv {
				cel::Value::List(li) => {
					raws.reserve(li.len());
					for (idx, v) in li.as_ref().iter().enumerate() {
						let nk = Cow::Owned(format!("{k}.{idx}"));
						Self::resolve_value(raws, nk, v, true);
					}
				},
				cel::Value::Map(m) => {
					raws.reserve(m.len());
					for (mk, mv) in m.iter() {
						let nk = Cow::Owned(format!("{k}.{mk}"));
						Self::resolve_value(raws, nk, mv, true);
					}
				},
				_ => raws.push((k, celv.json().ok())),
			}
		} else {
			raws.push((k, celv.json().ok()));
		}
	}

	fn eval_additions(&self) -> Vec<(Cow<str>, Option<Value>)> {
		self.eval(&self.fields.add)
	}

	fn eval_database_additions(&self) -> Vec<(Cow<str>, Option<Value>)> {
		self.eval(&self.database_fields.add)
	}
}

impl CelLogging {
	pub fn new(cfg: Config, metrics: MetricsConfig) -> Self {
		let mut cel_context = cel::ContextBuilder::new();
		if let Some(f) = &cfg.filter {
			cel_context.register_log_expression(f.as_ref());
		}
		for v in cfg.fields.add.values_unordered() {
			cel_context.register_log_expression(v.as_ref());
		}
		for v in cfg.database_fields.add.values_unordered() {
			cel_context.register_log_expression(v.as_ref());
		}
		for v in metrics.metric_fields.add.values_unordered() {
			cel_context.register_log_expression(v.as_ref());
		}
		if cfg.database.is_some() {
			cel_context.register_log_request();
		}

		Self {
			cel_context,
			filter: cfg.filter,
			fields: cfg.fields,
			database_fields: cfg.database_fields,
			metric_fields: metrics.metric_fields,
		}
	}

	pub fn register(&mut self, fields: &LoggingFields) {
		for v in fields.add.values_unordered() {
			self.cel_context.register_log_expression(v.as_ref());
		}
	}

	pub fn ctx(&mut self) -> &mut ContextBuilder {
		&mut self.cel_context
	}

	pub fn build<'a>(&'a self, inputs: CelLoggingBuildInputs<'a>) -> CelLoggingExecutor<'a> {
		let CelLogging {
			cel_context: _,
			filter,
			fields,
			database_fields,
			metric_fields,
		} = self;
		let executor = if inputs.req.is_none() && inputs.source_context.is_some() {
			// TCP case: use new_tcp_logger
			cel::Executor::new_tcp_logger(inputs.source_context, inputs.end_time)
		} else {
			// HTTP case: use new_logger
			cel::Executor::new_logger(
				inputs.req,
				inputs.resp,
				inputs.llm_response,
				inputs.mcp,
				Some(inputs.end_time),
				inputs.proxy,
			)
		};
		CelLoggingExecutor {
			executor,
			filter,
			fields,
			database_fields,
			metric_fields,
		}
	}
}

pub struct CelLoggingBuildInputs<'a> {
	pub req: Option<&'a cel::RequestSnapshot>,
	pub resp: Option<&'a cel::ResponseSnapshot>,
	pub llm_response: Option<&'a LLMContext>,
	pub mcp: Option<&'a MCPInfo>,
	pub end_time: &'a cel::RequestTime,
	pub proxy: Option<&'a cel::ProxyContext>,
	pub source_context: Option<&'a cel::SourceContext>,
}

#[derive(Debug)]
pub struct DropOnLog {
	log: Option<RequestLog>,
	debug_tracer: Option<dtrace::DebugTracer>,
}

impl DropOnLog {
	pub fn as_mut(&mut self) -> Option<&mut RequestLog> {
		self.log.as_mut()
	}
	pub fn as_ref(&self) -> Option<&RequestLog> {
		self.log.as_ref()
	}
	pub fn with(&mut self, f: impl FnOnce(&mut RequestLog)) {
		if let Some(l) = self.log.as_mut() {
			f(l)
		}
	}

	/// Computes (health, eviction_duration, restore_health) for finish_request.
	/// `unhealthy` should already be evaluated (preferably with the shared CEL executor when available).
	/// When no CEL expression is set, the default treats 5xx, connection failures, or non-zero
	/// gRPC status as unhealthy.
	#[cfg(test)]
	fn default_unhealthy(log: &RequestLog) -> bool {
		Self::default_unhealthy_for_status(log, log.status)
	}

	fn default_unhealthy_for_status(
		log: &RequestLog,
		status: Option<crate::http::StatusCode>,
	) -> bool {
		status.is_none_or(|s| s.is_server_error())
			|| log.grpc_status.load().is_some_and(|status| status != 0)
	}

	fn eviction_unhealthy(
		log: &RequestLog,
		status: Option<crate::http::StatusCode>,
		cel_exec: &CelLoggingExecutor<'_>,
	) -> bool {
		let default_unhealthy = Self::default_unhealthy_for_status(log, status);
		let Some(policy) = &log.health_policy else {
			return default_unhealthy;
		};
		let Some(expr) = &policy.unhealthy_expression else {
			return default_unhealthy;
		};
		cel_exec.executor.eval_bool(expr.as_ref())
	}

	/// Returns (health, eviction_duration, restore_health).
	fn eviction_decision(
		health_policy: &Option<health::Policy>,
		retry_backoff: Option<Duration>,
		retry_after: Option<Duration>,
		current_health: f64,
		consecutive_failure_count: u64,
		times_ejected: u64,
		unhealthy: bool,
	) -> (bool, Option<Duration>, Option<f64>) {
		let Some(policy) = health_policy else {
			let health = !unhealthy;
			return (health, None, None);
		};
		let fallback_duration = retry_after.or(retry_backoff);
		policy.eviction_decision(
			current_health,
			consecutive_failure_count,
			times_ejected,
			unhealthy,
			fallback_duration,
		)
	}

	fn add_llm_metrics(
		log: &RequestLog,
		route_identifier: &RouteIdentifier,
		duration: Duration,
		llm_response: Option<&LLMContext>,
		custom_metric_fields: &CustomField,
	) {
		if let Some(llm_response) = llm_response {
			let gen_ai_labels = Arc::new(GenAILabels {
				gen_ai_operation_name: strng::literal!("chat").into(),
				gen_ai_system: llm_response.provider.clone().into(),
				gen_ai_request_model: llm_response.request_model.clone().into(),
				gen_ai_response_model: llm_response.response_model.clone().into(),
				custom: custom_metric_fields.clone(),
				route: route_identifier.clone(),
			});
			if let Some(status) = llm_response.cost_status {
				log
					.metrics
					.cost_catalog_lookups
					.get_or_create(&CostCatalogLookupLabels {
						status,
						common: gen_ai_labels.clone().into(),
					})
					.inc();
				// Pair the lookup metric with a debug line so an operator can see *why* a
				// request wasn't priced (e.g. response model "gpt-5.5-2026-04-23" vs catalog
				// "gpt-5.5") without scraping metrics.
				match status {
					CostLookupStatus::Missing => debug!(
						provider = %llm_response.provider,
						request_model = %llm_response.request_model,
						response_model = ?llm_response.response_model,
						"no model cost: model not found in catalog"
					),
					CostLookupStatus::Unpriced => debug!(
						provider = %llm_response.provider,
						request_model = %llm_response.request_model,
						response_model = ?llm_response.response_model,
						"no model cost: model found but has no rates"
					),
					CostLookupStatus::Exact | CostLookupStatus::NoCatalog => {},
				}
			}
			if let Some(cost) = llm_response
				.cost
				.as_ref()
				.and_then(|cost| cost.total().to_f64())
			{
				log
					.metrics
					.gen_ai_cost
					.get_or_create(&gen_ai_labels)
					.inc_by(cost);
			}
			if let Some(it) = llm_response.input_tokens {
				log
					.metrics
					.gen_ai_token_usage
					.get_or_create(&GenAILabelsTokenUsage {
						gen_ai_token_type: strng::literal!("input").into(),
						common: gen_ai_labels.clone().into(),
					})
					.observe(it as f64)
			}
			if let Some(ot) = llm_response.output_tokens {
				log
					.metrics
					.gen_ai_token_usage
					.get_or_create(&GenAILabelsTokenUsage {
						gen_ai_token_type: strng::literal!("output").into(),
						common: gen_ai_labels.clone().into(),
					})
					.observe(ot as f64)
			}
			if let Some(crt) = llm_response.cached_input_tokens {
				log
					.metrics
					.gen_ai_token_usage
					.get_or_create(&GenAILabelsTokenUsage {
						gen_ai_token_type: strng::literal!("input_cache_read").into(),
						common: gen_ai_labels.clone().into(),
					})
					.observe(crt as f64)
			}
			if let Some(cwt) = llm_response.cache_creation_input_tokens {
				log
					.metrics
					.gen_ai_token_usage
					.get_or_create(&GenAILabelsTokenUsage {
						gen_ai_token_type: strng::literal!("input_cache_write").into(),
						common: gen_ai_labels.clone().into(),
					})
					.observe(cwt as f64)
			}
			log
				.metrics
				.gen_ai_request_duration
				.get_or_create(&gen_ai_labels)
				.observe(duration.as_secs_f64());
			if let Some(ttft) = llm_response
				.time_to_first_token
				.and_then(|duration| duration.0.to_std().ok())
			{
				log
					.metrics
					.gen_ai_time_to_first_token
					.get_or_create(&gen_ai_labels)
					.observe(ttft.as_secs_f64());
			}
			if let Some(time_per_output_token) = llm_response
				.time_per_output_token
				.and_then(|duration| duration.0.to_std().ok())
			{
				log
					.metrics
					.gen_ai_time_per_output_token
					.get_or_create(&gen_ai_labels)
					.observe(time_per_output_token.as_secs_f64());
			}
		}
	}
}

impl From<RequestLog> for DropOnLog {
	fn from(log: RequestLog) -> Self {
		Self {
			log: Some(log),
			debug_tracer: dtrace::DebugTracer::active(),
		}
	}
}

fn proxy_context(log: &RequestLog) -> cel::ProxyContext {
	cel::ProxyContext {
		bind: log.bind_name.clone(),
		gateway: log
			.listener_name
			.as_ref()
			.map(|l| cel::ProxyGatewayContext {
				namespace: l.gateway_namespace.clone(),
				name: l.gateway_name.clone(),
			}),
		listener: log
			.listener_name
			.as_ref()
			.map(|l| cel::ProxyListenerContext {
				name: l.listener_name.clone(),
			}),
		route: log.route_name.as_ref().map(|r| cel::ProxyRouteContext {
			namespace: r.namespace.clone(),
			name: r.name.clone(),
			kind: r.kind.clone(),
			rule: r.rule_name.clone(),
		}),
		request_processing_duration: log
			.request_processing_duration
			.and_then(cel::CelDuration::from_std),
		upstream_duration: log.upstream_duration.and_then(cel::CelDuration::from_std),
		response_processing_duration: log
			.response_processing_duration
			.and_then(cel::CelDuration::from_std),
	}
}

impl RequestLog {
	pub fn new(
		cel: CelLogging,
		metrics: Arc<Metrics>,
		model_catalog: Arc<ModelCatalog>,
		start: Timestamp,
		tcp_info: TCPConnectionInfo,
	) -> Self {
		RequestLog {
			cel,
			metrics,
			model_catalog,
			start,
			request_processing_start: Instant::now(),
			request_processing_duration: None,
			upstream_duration: None,
			response_processing_start: None,
			response_processing_duration: None,
			connection_id: current_connection_id(),
			request_id: current_request_id(),
			tcp_info,
			tls_info: None,
			tracer: None,
			trace_spans: Arc::new(Mutex::new(Default::default())),
			otel_logger: None,
			endpoint: None,
			bind_name: None,
			listener_name: None,
			route_name: None,
			backend_info: None,
			backend_protocol: None,
			host: None,
			method: None,
			path: None,
			path_match: None,
			version: None,
			status: None,
			reason: None,
			retry_after: None,
			health_policy: None,
			retry_backoff: None,
			jwt_sub: None,
			retry_attempt: None,
			error: None,
			grpc_status: Default::default(),
			mcp_status: Default::default(),
			incoming_span: None,
			outgoing_span: None,
			llm_request: None,
			llm_response: Default::default(),
			a2a_method: None,
			inference_pool: None,
			request_handle: None,
			request_snapshot: None,
			response_snapshot: None,
			source_context: None,
			response_bytes: 0,
		}
	}

	pub fn span_writer(&self) -> SpanWriter {
		let inner = self.span_writer_inner();
		SpanWriter { inner }
	}
	fn span_writer_inner(&self) -> Option<SpanWriterInner> {
		// Early return if there is no tracer enabled at all
		self.tracer.as_ref()?;
		let tp = self.outgoing_span.clone()?;
		if !tp.is_sampled() {
			return None;
		}

		Some(SpanWriterInner {
			parent: tp,
			inner: self.trace_spans.clone(),
		})
	}

	fn finish_request_handle(
		&self,
		rh: ActiveHandle,
		end_time: Timestamp,
		cel_exec: &CelLoggingExecutor<'_>,
	) {
		self.finish_request_handle_with_attempt(rh, end_time, self.status, self.retry_after, cel_exec);
	}

	fn finish_request_handle_with_attempt(
		&self,
		rh: ActiveHandle,
		end_time: Timestamp,
		status: Option<crate::http::StatusCode>,
		retry_after: Option<Duration>,
		cel_exec: &CelLoggingExecutor<'_>,
	) {
		let unhealthy = DropOnLog::eviction_unhealthy(self, status, cel_exec);
		let (health, eviction_duration, restore_health) = DropOnLog::eviction_decision(
			&self.health_policy,
			self.retry_backoff,
			retry_after,
			rh.health_score(),
			rh.consecutive_failures(),
			rh.times_ejected(),
			unhealthy,
		);
		rh.finish_request(
			health,
			end_time.duration_since(&self.start),
			eviction_duration,
			restore_health,
		);
	}

	pub(crate) fn finalize_request_handle_for_attempt(
		&mut self,
		end_time: Timestamp,
		status: Option<crate::http::StatusCode>,
		retry_after: Option<Duration>,
		response_snapshot: Option<&cel::ResponseSnapshot>,
		llm_response: Option<&LLMContext>,
		mcp: Option<&MCPInfo>,
	) {
		let cel_end_time = cel::RequestTime(end_time.as_datetime());
		let proxy_timing = proxy_context(self);
		let cel_exec = self.cel.build(CelLoggingBuildInputs {
			req: self.request_snapshot.as_deref(),
			resp: response_snapshot,
			llm_response,
			mcp: mcp.filter(|m| !m.is_empty()),
			end_time: &cel_end_time,
			source_context: self.source_context.as_ref(),
			proxy: Some(&proxy_timing),
		});
		let Some(rh) = self.request_handle.take() else {
			return;
		};
		self.finish_request_handle_with_attempt(rh, end_time, status, retry_after, &cel_exec);
	}
}

#[derive(Debug)]
pub struct RequestLog {
	pub cel: CelLogging,
	pub metrics: Arc<Metrics>,
	pub model_catalog: Arc<ModelCatalog>,
	pub start: Timestamp,
	pub request_processing_start: Instant,
	pub request_processing_duration: Option<Duration>,
	pub upstream_duration: Option<Duration>,
	pub response_processing_start: Option<Instant>,
	pub response_processing_duration: Option<Duration>,
	pub connection_id: Option<u64>,
	pub request_id: Option<u64>,
	pub tcp_info: TCPConnectionInfo,

	// Set only for TLS traffic
	pub tls_info: Option<TLSConnectionInfo>,

	// Set only if the trace is sampled
	pub tracer: Option<std::sync::Arc<trc::Tracer>>,
	/// Additional spans created during the request (e.g. upstream call spans).
	/// These are flushed on drop when tracing is enabled.
	pub trace_spans: Arc<Mutex<Vec<BufferedSpan>>>,

	// Set only if OTLP logging is configured
	pub otel_logger: Option<std::sync::Arc<OtelAccessLogger>>,

	pub endpoint: Option<Target>,

	pub bind_name: Option<BindKey>,
	pub listener_name: Option<ListenerName>,
	pub route_name: Option<RouteName>,
	pub backend_info: Option<BackendInfo>,
	pub backend_protocol: Option<cel::BackendProtocol>,

	pub host: Option<String>,
	pub method: Option<::http::Method>,
	pub path: Option<String>,
	pub path_match: Option<Strng>,
	pub version: Option<::http::Version>,
	pub status: Option<crate::http::StatusCode>,
	pub reason: Option<ProxyResponseReason>,
	pub retry_after: Option<Duration>,

	/// Health policy for backend (e.g. AI provider) failover. Set from route policies when request_handle is used.
	pub health_policy: Option<health::Policy>,
	/// Retry backoff from route policy; used as fallback eviction duration when health_policy has no explicit duration.
	pub retry_backoff: Option<Duration>,

	pub jwt_sub: Option<String>,

	pub retry_attempt: Option<u8>,
	pub error: Option<String>,

	pub grpc_status: AsyncLog<u8>,
	pub mcp_status: AsyncLog<mcp::MCPInfo>,

	pub incoming_span: Option<trc::TraceParent>,
	pub outgoing_span: Option<trc::TraceParent>,

	pub llm_request: Option<llm::LLMRequest>,
	pub llm_response: AsyncLog<llm::LLMInfo>,

	pub a2a_method: Option<Strng>,

	pub inference_pool: Option<SocketAddr>,

	pub request_handle: Option<ActiveHandle>,
	pub request_snapshot: Option<Arc<cel::RequestSnapshot>>,
	pub response_snapshot: Option<cel::ResponseSnapshot>,
	/// Source context for TCP connections (where we don't have an HTTP request)
	pub source_context: Option<cel::SourceContext>,

	pub response_bytes: u64,
}

impl Drop for DropOnLog {
	fn drop(&mut self) {
		if let Some(debug_tracer) = &self.debug_tracer {
			debug_tracer.request_completed();
		} else {
			dtrace::trace(|t| t.request_completed());
		}
		let debug_tracer = self.debug_tracer.clone();
		dtrace::with_trace(debug_tracer, || {
			let Some(mut log) = self.log.take() else {
				return;
			};

			let route_identifier = RouteIdentifier {
				bind: (&log.bind_name).into(),
				gateway: log
					.listener_name
					.as_ref()
					.map(|l| l.as_gateway_name())
					.into(),
				listener: log.listener_name.as_ref().map(|l| &l.listener_name).into(),
				route: log.route_name.as_ref().map(|l| l.as_route_name()).into(),
				route_rule: log
					.route_name
					.as_ref()
					.and_then(|l| l.rule_name.as_ref())
					.into(),
			};

			let is_tcp = matches!(&log.backend_protocol, &Some(cel::BackendProtocol::tcp));

			let mut http_labels = HTTPLabels {
				backend: log
					.backend_info
					.as_ref()
					.map(|info| info.backend_name.clone())
					.into(),
				protocol: log.backend_protocol.into(),
				route: route_identifier.clone(),
				method: log.method.clone().into(),
				status: log.status.as_ref().map(|s| s.as_u16()).into(),
				reason: log.reason.into(),
				custom: CustomField::default(),
			};

			// Always run request_handle/finish_request first so LLM provider eviction (failover) runs
			// even when logging/tracing/metrics are disabled.
			let end_time = Timestamp::now();
			let duration = end_time.duration_since(&log.start);
			let enable_trace = log.tracer.is_some();

			let mut llm_response: Option<LLMContext> = log
				.llm_response
				.take()
				.map(|llm_info| LLMContext::from_llm_info(llm_info, Some(log.model_catalog.as_ref())));
			if let Some(llm_response) = llm_response.as_mut() {
				llm_response.set_token_timing(log.start.as_instant(), end_time.as_instant());
			}

			let mcp = log.mcp_status.take();
			let request_handle = log.request_handle.take();
			let cel_end_time = cel::RequestTime(end_time.as_datetime());
			// The response snapshot is captured before the response body is drained, so
			// trailer-only grpc-status values are learned later by LogBody. Copy the final
			// value back into the snapshot before evaluating access-log CEL fields.
			if let Some(grpc_status) = log.grpc_status.load()
				&& let Some(resp) = log.response_snapshot.as_mut()
			{
				resp.grpc_status = Some(grpc_status);
			}
			let proxy_timing = proxy_context(&log);
			if let Some(resp) = log.response_snapshot.as_mut() {
				resp.proxy = Some(proxy_timing.clone());
			}
			let cel_exec = log.cel.build(CelLoggingBuildInputs {
				req: log.request_snapshot.as_deref(),
				resp: log.response_snapshot.as_ref(),
				llm_response: llm_response.as_ref(),
				mcp: mcp.as_ref().filter(|m| !m.is_empty()),
				end_time: &cel_end_time,
				proxy: Some(&proxy_timing),
				source_context: log.source_context.as_ref(),
			});
			if let Some(rh) = request_handle {
				log.finish_request_handle(rh, end_time, &cel_exec);
			}

			let custom_metric_fields = CustomField::new(
				// For metrics, keep empty values which will become 'unknown'
				cel_exec
					.eval_keep_empty(&cel_exec.metric_fields.add, true)
					.into_iter()
					.map(|(k, v)| {
						(
							strng::new(k),
							v.and_then(|v| match v {
								Value::String(s) => Some(strng::new(s)),
								_ => None,
							}),
						)
					}),
			);
			http_labels.custom = custom_metric_fields.clone();
			if !is_tcp {
				log.metrics.requests.get_or_create(&http_labels).inc();
			}
			if log.response_bytes > 0 {
				log
					.metrics
					.response_bytes
					.get_or_create(&http_labels)
					.inc_by(log.response_bytes);
			}
			// Record HTTP request duration for all requests
			log
				.metrics
				.request_duration
				.get_or_create(&http_labels)
				.observe(duration.as_secs_f64());

			if let Some(retry_count) = log.retry_attempt {
				log
					.metrics
					.retries
					.get_or_create(&http_labels)
					.inc_by(retry_count as u64);
			}
			if !is_tcp {
				let labels = http_labels.into();
				if let Some(duration) = log.request_processing_duration {
					log
						.metrics
						.request_processing_duration
						.get_or_create(&labels)
						.observe(duration.as_secs_f64());
				}
				if let Some(duration) = log.response_processing_duration {
					log
						.metrics
						.response_processing_duration
						.get_or_create(&labels)
						.observe(duration.as_secs_f64());
				}
			}

			Self::add_llm_metrics(
				&log,
				&route_identifier,
				duration,
				llm_response.as_ref(),
				&custom_metric_fields,
			);
			if let Some(mcp) = &mcp
				&& mcp.method_name.is_some()
			{
				// Check mcp.method_name is set, so we don't count things like GET and DELETE
				log
					.metrics
					.mcp_requests
					.get_or_create(&MCPCall {
						method: mcp.method_name.as_ref().map(RichStrng::from).into(),
						resource_type: mcp.resource_type().into(),
						server: mcp.target_name().map(RichStrng::from).into(),
						resource: mcp.resource_name().map(RichStrng::from).into(),

						route: route_identifier.clone(),
						custom: custom_metric_fields.clone(),
					})
					.inc();
			}

			let maybe_enable_log = agent_core::telemetry::enabled("request", &Level::INFO);
			// For now we only enable this log for LLM requests to keep cost/performance appropriate.
			let log_store_enabled = log_store::enabled() && llm_response.is_some();
			if !maybe_enable_log && !enable_trace && !log_store_enabled {
				return;
			}

			let dur = format!("{}ms", duration.as_millis());
			let grpc = log.grpc_status.load();

			let input_tokens = llm_response.as_ref().and_then(|l| l.input_tokens);
			let cost = llm_response.as_ref().and_then(|l| l.cost.as_ref());
			let usage_cost_total = cost.map(|b| b.total().to_string());
			let trace_cost_fields = if enable_trace {
				cost.map(|b| {
					[
						("agw.ai.usage.cost.input", b.input.to_string()),
						("agw.ai.usage.cost.output", b.output.to_string()),
						("agw.ai.usage.cost.cache_read", b.cache_read.to_string()),
						("agw.ai.usage.cost.cache_write", b.cache_write.to_string()),
						("agw.ai.usage.cost.reasoning", b.reasoning.to_string()),
						("agw.ai.usage.cost.input_audio", b.input_audio.to_string()),
						("agw.ai.usage.cost.output_audio", b.output_audio.to_string()),
					]
				})
			} else {
				None
			};

			let trace_id = log.outgoing_span.as_ref().map(|id| id.trace_id());
			let span_id = log.outgoing_span.as_ref().map(|id| id.span_id());
			let fields = cel_exec.fields;
			let reason = log.reason.and_then(|r| match r {
				ProxyResponseReason::Upstream => None,
				_ => Some(r),
			});
			let mcp_target = mcp
				.as_ref()
				.and_then(|m| m.target_name())
				.map(str::to_owned);
			let mcp_resource_type = mcp.as_ref().and_then(|m| m.resource_type());
			let mcp_resource_uri = mcp.as_ref().and_then(|m| {
				if matches!(m.resource_type(), Some(MCPOperation::Resource)) {
					m.resource_name().map(str::to_owned)
				} else {
					None
				}
			});
			let mcp_tool_name = mcp.as_ref().and_then(|m| {
				if matches!(m.resource_type(), Some(MCPOperation::Tool)) {
					m.resource_name().map(str::to_owned)
				} else {
					None
				}
			});
			let mcp_prompt_name = mcp.as_ref().and_then(|m| {
				if matches!(m.resource_type(), Some(MCPOperation::Prompt)) {
					m.resource_name().map(str::to_owned)
				} else {
					None
				}
			});

			let emit_ids = agent_core::telemetry::enabled("request", &Level::DEBUG);
			let mut kv = vec![
				(
					"connection.id",
					emit_ids
						.then_some(log.connection_id)
						.flatten()
						.map(Into::into),
				),
				(
					"request.id",
					emit_ids.then_some(log.request_id).flatten().map(Into::into),
				),
				("gateway", route_identifier.gateway.as_deref().map(display)),
				(
					"listener",
					route_identifier.listener.as_deref().map(display),
				),
				(
					"route_rule",
					route_identifier.route_rule.as_deref().map(display),
				),
				("route", route_identifier.route.as_deref().map(display)),
				("endpoint", log.endpoint.display()),
				("src.addr", Some(display(&log.tcp_info.peer_addr))),
				("http.method", log.method.display()),
				("http.host", log.host.display()),
				("http.path", log.path.display()),
				// TODO: incoming vs outgoing
				("http.version", log.version.as_ref().map(debug)),
				(
					"http.status",
					log.status.as_ref().map(|s| s.as_u16().into()),
				),
				("grpc.status", grpc.map(Into::into)),
				(
					"tls.sni",
					if log.host.is_none() {
						log.tls_info.as_ref().and_then(|s| s.server_name.display())
					} else {
						None
					},
				),
				("trace.id", trace_id.display()),
				("span.id", span_id.display()),
				("jwt.sub", log.jwt_sub.display()),
				("protocol", log.backend_protocol.as_ref().map(debug)),
				("a2a.method", log.a2a_method.display()),
				(
					"mcp.method.name",
					mcp
						.as_ref()
						.and_then(|m| m.method_name.as_ref())
						.map(display),
				),
				("mcp.target", mcp_target.as_ref().map(display)),
				("mcp.resource.type", mcp_resource_type.as_ref().map(display)),
				("mcp.resource.uri", mcp_resource_uri.as_ref().map(display)),
				("gen_ai.tool.name", mcp_tool_name.as_ref().map(display)),
				("gen_ai.prompt.name", mcp_prompt_name.as_ref().map(display)),
				(
					"mcp.session.id",
					mcp
						.as_ref()
						.and_then(|m| m.session_id.as_ref())
						.map(display),
				),
				(
					"inferencepool.selected_endpoint",
					log.inference_pool.display(),
				),
				// OpenTelemetry Gen AI Semantic Conventions v1.40.0
				(
					"gen_ai.operation.name",
					log.llm_request.as_ref().map(|r| {
						if r.input_format == InputFormat::Embeddings {
							"embeddings".into()
						} else {
							"chat".into()
						}
					}),
				),
				(
					"gen_ai.provider.name",
					log.llm_request.as_ref().map(|l| display(&l.provider)),
				),
				(
					"gen_ai.request.model",
					log.llm_request.as_ref().map(|l| display(&l.request_model)),
				),
				(
					"gen_ai.response.model",
					llm_response
						.as_ref()
						.and_then(|l| l.response_model.display()),
				),
				("gen_ai.usage.input_tokens", input_tokens.map(Into::into)),
				(
					"gen_ai.usage.cache_creation.input_tokens",
					llm_response
						.as_ref()
						.and_then(|l| l.cache_creation_input_tokens)
						.map(Into::into),
				),
				(
					"gen_ai.usage.cache_read.input_tokens",
					llm_response
						.as_ref()
						.and_then(|l| l.cached_input_tokens)
						.map(Into::into),
				),
				(
					"gen_ai.usage.output_tokens",
					llm_response
						.as_ref()
						.and_then(|l| l.output_tokens)
						.map(Into::into),
				),
				(
					"agw.ai.usage.cost.total",
					usage_cost_total.as_deref().map(Into::into),
				),
				// Not part of official semconv
				(
					"gen_ai.usage.output_image_tokens",
					llm_response
						.as_ref()
						.and_then(|l| l.output_image_tokens)
						.map(Into::into),
				),
				// Not part of official semconv
				(
					"gen_ai.usage.output_audio_tokens",
					llm_response
						.as_ref()
						.and_then(|l| l.output_audio_tokens)
						.map(Into::into),
				),
				(
					"gen_ai.request.temperature",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.temperature)
						.map(Into::into),
				),
				(
					"gen_ai.embeddings.dimension.count",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.dimensions)
						.map(Into::into),
				),
				(
					"gen_ai.request.encoding_formats",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.encoding_format.display()),
				),
				(
					"gen_ai.request.top_p",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.top_p)
						.map(Into::into),
				),
				(
					"gen_ai.request.max_tokens",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.max_tokens)
						.map(|v| (v as i64).into()),
				),
				(
					"gen_ai.request.frequency_penalty",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.frequency_penalty)
						.map(Into::into),
				),
				(
					"gen_ai.request.presence_penalty",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.presence_penalty)
						.map(Into::into),
				),
				(
					"gen_ai.request.seed",
					log
						.llm_request
						.as_ref()
						.and_then(|l| l.params.seed)
						.map(Into::into),
				),
				("retry.attempt", log.retry_attempt.display()),
				("error", log.error.quoted()),
				("reason", reason.display()),
				("duration", Some(dur.as_str().into())),
			];

			let mut extra_kv_capacity = trace_cost_fields.as_ref().map_or(0, |fields| fields.len());
			if maybe_enable_log || log_store_enabled {
				extra_kv_capacity += fields.add.len();
			}
			kv.reserve_exact(extra_kv_capacity);

			if enable_trace && let Some(t) = &log.tracer {
				let base_len = kv.len();
				if let Some(trace_cost_fields) = &trace_cost_fields {
					kv.extend(
						trace_cost_fields
							.iter()
							.map(|(key, value)| (*key, Some(value.as_str().into()))),
					);
				}
				t.send(&log, &end_time, &cel_exec, kv.as_slice());
				kv.truncate(base_len);
				// Flush any buffered spans created during request processing.
				// Does best effort, if the lock is poisoned, skip flushing.
				if log.outgoing_span.as_ref().is_some_and(|s| s.is_sampled())
					&& trc::should_export_span(t.filter.as_deref(), &cel_exec.executor)
					&& let Ok(mut spans) = log.trace_spans.lock()
				{
					for buffered_span in spans.drain(..) {
						t.processor.emit(buffered_span.into_span_data());
					}
				}
			};
			if maybe_enable_log || log_store_enabled {
				let passes_log_filter = cel_exec.eval_filter();
				if !passes_log_filter {
					return;
				}
				kv.reserve(fields.add.len());
				for (k, v) in &mut kv {
					// Remove filtered lines, or things we are about to add
					if fields.has(k) {
						*v = None;
					}
				}
				// To avoid lifetime issues need to store the expression before we give it to ValueBag reference.
				// TODO: we could allow log() to take a list of borrows and then a list of OwnedValueBag
				let raws = cel_exec.eval_additions();
				for (k, v) in &raws {
					// Preserve JSON numbers as numeric ValueBags so the core logger can control
					// JSON number formatting instead of serializing serde_json::Number directly.
					let eval = v.as_ref().map(json_value_to_value_bag);
					kv.push((k, eval));
				}

				if maybe_enable_log {
					agent_core::telemetry::log("info", "request", &kv);

					if let Some(otel) = &log.otel_logger {
						otel.emit("info", "request", &kv);
					}
				}

				if log_store_enabled {
					let original_model = original_model_from_metadata(
						log.request_snapshot.as_deref(),
						log.response_snapshot.as_ref(),
					)
					.map(str::to_owned);

					let mut db_kv = kv.clone();
					let db_raws = cel_exec.eval_database_additions();
					let default_db_raws = [
						(
							Cow::Borrowed("user_agent.name"),
							user_agent_name(log.request_snapshot.as_deref()).map(Value::String),
						),
						(
							Cow::Borrowed("agw.ai.original_model"),
							original_model.clone().map(Value::String),
						),
						(
							Cow::Borrowed("agw.api_key.name"),
							api_key_name(log.request_snapshot.as_deref()).map(Value::String),
						),
					];
					db_kv.reserve(db_raws.len() + default_db_raws.len());
					for (k, v) in db_raws.iter().chain(default_db_raws.iter()) {
						let eval = v.as_ref().map(json_value_to_value_bag);
						db_kv.push((k, eval));
					}
					if let Some(cost) = cost {
						// Default log only puts totals; we want all of them.
						let cost_raws = [
							("agw.ai.usage.cost.total", cost.total()),
							("agw.ai.usage.cost.input", cost.input),
							("agw.ai.usage.cost.output", cost.output),
							("agw.ai.usage.cost.cacheRead", cost.cache_read),
							("agw.ai.usage.cost.cacheWrite", cost.cache_write),
							("agw.ai.usage.cost.reasoning", cost.reasoning),
							("agw.ai.usage.cost.inputAudio", cost.input_audio),
							("agw.ai.usage.cost.outputAudio", cost.output_audio),
						];
						db_kv.reserve(cost_raws.len());
						for (k, v) in &cost_raws {
							let eval = v.to_f64().map(ValueBag::from_f64);
							db_kv.push((*k, eval));
						}
					}
					let attributes_json = kv_to_json(&db_kv);
					let agentgateway_user = string_attribute(&attributes_json, "agentgateway.user");
					let agentgateway_group = string_attribute(&attributes_json, "agentgateway.group");
					let user_agent_name = string_attribute(&attributes_json, "user_agent.name");
					let payload = llm_response.as_ref().and_then(|info| {
						let request_prompt_json = info
							.prompt
							.as_ref()
							.and_then(|prompt| serde_json::to_value(prompt.as_ref()).ok());
						let response_completion_json = info
							.completion
							.as_ref()
							.and_then(|completion| serde_json::to_value(completion).ok());
						(request_prompt_json.is_some() || response_completion_json.is_some()).then_some(
							log_store::StoredRequestLogPayload {
								request_prompt_json,
								response_completion_json,
							},
						)
					});
					let has_payload = payload.is_some();
					let total_tokens = llm_response.as_ref().and_then(|llm| {
						llm
							.total_tokens
							.or_else(|| Some(llm.input_tokens? + llm.output_tokens?))
					});
					log_store::emit(log_store::StoredRequestLog {
						id: uuid::Uuid::new_v4().to_string(),
						started_at: log.start.as_datetime().with_timezone(&chrono::Utc),
						completed_at: end_time.as_datetime().with_timezone(&chrono::Utc),
						duration_ms: u128_to_i64(duration.as_millis()),
						trace_id: trace_id.map(|id| id.to_string()),
						span_id: span_id.map(|id| id.to_string()),
						http_status: log.status.as_ref().map(|s| i64::from(s.as_u16())),
						error: log.error.clone(),
						gen_ai_operation_name: log.llm_request.as_ref().map(|request| {
							if request.input_format == InputFormat::Embeddings {
								"embeddings".to_string()
							} else {
								"chat".to_string()
							}
						}),
						gen_ai_provider_name: log
							.llm_request
							.as_ref()
							.map(|request| request.provider.to_string()),
						gen_ai_request_model: log
							.llm_request
							.as_ref()
							.map(|request| request.request_model.to_string()),
						gen_ai_response_model: llm_response
							.as_ref()
							.and_then(|llm| llm.response_model.as_ref().map(ToString::to_string)),
						input_tokens: u64_to_i64(input_tokens),
						output_tokens: u64_to_i64(llm_response.as_ref().and_then(|llm| llm.output_tokens)),
						total_tokens: u64_to_i64(total_tokens),
						cost: cost.and_then(|cost| cost.total().to_f64()),
						agentgateway_user,
						agentgateway_group,
						user_agent_name,
						has_payload,
						attributes_json,
						payload,
					});
				}
			}
		});
	}
}

pin_project_lite::pin_project! {
		/// A data stream created from a [`Body`].
		#[derive(Debug)]
		pub struct LogBody<B> {
				#[pin]
				body: B,
				log: DropOnLog,
		}
}

impl<B> LogBody<B> {
	/// Create a new `LogBody`
	pub fn new(body: B, log: DropOnLog) -> Self {
		Self { body, log }
	}
}

impl<B: Body + Debug> Body for LogBody<B>
where
	B::Data: Debug,
{
	type Data = B::Data;
	type Error = B::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let this = self.project();
		let result = ready!(this.body.poll_frame(cx));
		match result {
			Some(Ok(frame)) => {
				if let Some(trailer) = frame.trailers_ref()
					&& let Some(grpc) = this.log.as_mut().map(|log| log.grpc_status.clone())
				{
					crate::proxy::httpproxy::maybe_set_grpc_status(&grpc, trailer);
				}
				if let Some(log) = this.log.as_mut()
					&& let Some(data) = frame.data_ref()
				{
					// Count the bytes in this data frame
					log.response_bytes = log.response_bytes.saturating_add(data.remaining() as u64);
				}
				Poll::Ready(Some(Ok(frame)))
			},
			res => Poll::Ready(res),
		}
	}

	fn is_end_stream(&self) -> bool {
		self.body.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		self.body.size_hint()
	}
}

pub struct OtelAccessLogger {
	provider: SdkLoggerProvider,
	logger: opentelemetry_sdk::logs::SdkLogger,
}

impl std::fmt::Debug for OtelAccessLogger {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("OtelAccessLogger").finish()
	}
}

fn to_any_value(v: &ValueBag) -> AnyValue {
	if let Some(b) = v.to_str() {
		AnyValue::String(b.to_string().into())
	} else if let Some(b) = v.to_i64() {
		AnyValue::Int(b)
	} else if let Some(b) = v.to_f64() {
		AnyValue::Double(b)
	} else if let Some(b) = v.to_bool() {
		AnyValue::Boolean(b)
	} else {
		AnyValue::String(v.to_string().into())
	}
}

/// Policy-aware OTLP gRPC log exporter that routes via `GrpcReferenceChannel`, ensuring
/// backend policies are looked up and applied by `PolicyClient::call_reference`.
#[derive(Clone)]
struct PolicyGrpcLogExporter {
	tonic_client:
		opentelemetry_proto::tonic::collector::logs::v1::logs_service_client::LogsServiceClient<
			crate::http::ext_proc::GrpcReferenceChannel,
		>,
	is_shutdown: Arc<bool>,
	resource: Resource,
	runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for PolicyGrpcLogExporter {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PolicyGrpcLogExporter").finish()
	}
}

impl PolicyGrpcLogExporter {
	fn new(
		inputs: Arc<crate::ProxyInputs>,
		target: Arc<crate::types::agent::SimpleBackendReference>,
		policies: Vec<crate::types::agent::BackendTrafficPolicy>,
		runtime: tokio::runtime::Handle,
	) -> Self {
		use crate::http::ext_proc::GrpcReferenceChannel;
		let channel = GrpcReferenceChannel {
			target,
			policies: Arc::new(policies),
			client: crate::proxy::httpproxy::PolicyClient::new(inputs),
		};
		let tonic_client =
			opentelemetry_proto::tonic::collector::logs::v1::logs_service_client::LogsServiceClient::new(
				channel,
			);
		Self {
			tonic_client,
			is_shutdown: Arc::new(false),
			resource: Resource::builder().build(),
			runtime,
		}
	}
}

impl opentelemetry_sdk::logs::LogExporter for PolicyGrpcLogExporter {
	fn export(
		&self,
		batch: opentelemetry_sdk::logs::LogBatch<'_>,
	) -> impl std::future::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send {
		use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
		use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};

		let is_shutdown = self.is_shutdown.clone();
		let mut client = self.tonic_client.clone();
		let resource: opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema =
			(&self.resource).into();
		let resource_logs = group_logs_by_resource_and_scope(&batch, &resource);
		let handle = self.runtime.clone();

		async move {
			if *is_shutdown {
				return Err(OTelSdkError::AlreadyShutdown);
			}
			let req =
				opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest { resource_logs };
			// Drop tonic Response inside the spawned task so guard is released on the Tokio runtime, not on
			// the BatchProcessor OS thread which has no Tokio context.
			handle
				.spawn(async move {
					client
						.export(req)
						.await
						.map(|_| ())
						.map_err(|e| e.message().to_string())
				})
				.await
				.map_err(|e| OTelSdkError::InternalFailure(e.to_string()))?
				.map_err(OTelSdkError::InternalFailure) as OTelSdkResult
		}
	}

	fn shutdown(&self) -> opentelemetry_sdk::error::OTelSdkResult {
		Ok(())
	}

	fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
		self.resource = resource.clone();
	}
}

fn build_resource(defaults: Option<&trc::GlobalResourceDefaults>) -> Resource {
	let mut resource_builder = Resource::builder();
	if let Some(d) = defaults {
		for kv in &d.attrs {
			resource_builder = resource_builder.with_attribute(kv.clone());
		}
	}
	resource_builder = resource_builder.with_service_name(
		defaults
			.and_then(|d| d.service_name.clone())
			.unwrap_or_else(|| "agentgateway".to_string()),
	);
	resource_builder = resource_builder.with_attribute(KeyValue::new(
		"service.version",
		agent_core::version::BuildInfo::new().version,
	));
	resource_builder.build()
}

impl OtelAccessLogger {
	pub fn new(
		policy_client: crate::proxy::httpproxy::PolicyClient,
		backend_ref: crate::types::agent::SimpleBackendReference,
		policies: Vec<crate::types::agent::BackendTrafficPolicy>,
		protocol: crate::types::agent::TracingProtocol,
		path: String,
	) -> anyhow::Result<Self> {
		let defaults = trc::global_resource_defaults();
		let resource = build_resource(defaults);

		let exporter_runtime = policy_client
			.inputs
			.cfg
			.admin_runtime_handle
			.clone()
			.unwrap_or_else(tokio::runtime::Handle::current);

		let provider = if protocol == crate::types::agent::TracingProtocol::Grpc {
			let exporter = PolicyGrpcLogExporter::new(
				policy_client.inputs.clone(),
				Arc::new(backend_ref),
				policies,
				exporter_runtime,
			);
			SdkLoggerProvider::builder()
				.with_resource(resource)
				.with_batch_exporter(exporter)
				.build()
		} else {
			let http_client = trc::PolicyOtelHttpClient {
				policy_client,
				backend_ref,
				policies,
				runtime: exporter_runtime,
			};
			let exporter = opentelemetry_otlp::LogExporter::builder()
				.with_http()
				.with_http_client(http_client)
				.with_endpoint(path)
				.build()?;
			SdkLoggerProvider::builder()
				.with_resource(resource)
				.with_batch_exporter(exporter)
				.build()
		};

		let logger = provider.logger("agentgateway.access");

		Ok(Self { provider, logger })
	}

	pub fn shutdown(&self) {
		let _ = self.provider.shutdown();
	}
}

impl OtelLogSink for OtelAccessLogger {
	fn emit<'v>(&self, level: &str, target: &str, kv: &[(&str, Option<ValueBag<'v>>)]) {
		let severity = match level {
			"error" => Severity::Error,
			"warn" => Severity::Warn,
			"info" => Severity::Info,
			"debug" => Severity::Debug,
			"trace" => Severity::Trace,
			_ => Severity::Info,
		};
		let severity_text: &'static str = match level {
			"error" => "ERROR",
			"warn" => "WARN",
			"info" => "INFO",
			"debug" => "DEBUG",
			"trace" => "TRACE",
			_ => "INFO",
		};

		let mut record = self.logger.create_log_record();
		record.set_severity_number(severity);
		record.set_severity_text(severity_text);
		record.set_target(target.to_string());

		let mut trace_id_val: Option<u128> = None;
		let mut span_id_val: Option<u64> = None;

		for &(k, ref v) in kv {
			let Some(v) = v else { continue };

			match k {
				"trace.id" => {
					if let Some(s) = v.to_str()
						&& let Ok(id) = u128::from_str_radix(&s, 16)
					{
						trace_id_val = Some(id);
					}
					record.add_attribute(Key::new(k.to_string()), to_any_value(v));
				},
				"span.id" => {
					if let Some(s) = v.to_str()
						&& let Ok(id) = u64::from_str_radix(&s, 16)
					{
						span_id_val = Some(id);
					}
					record.add_attribute(Key::new(k.to_string()), to_any_value(v));
				},
				_ => {
					record.add_attribute(Key::new(k.to_string()), to_any_value(v));
				},
			}
		}

		if let Some(tid) = trace_id_val {
			record.set_trace_context(
				opentelemetry::trace::TraceId::from(tid),
				span_id_val
					.map(opentelemetry::trace::SpanId::from)
					.unwrap_or(opentelemetry::trace::SpanId::INVALID),
				None,
			);
		}

		self.logger.emit(record);
	}

	fn shutdown(&self) {
		let _ = self.provider.shutdown();
	}
}

// SpanWriter is a construct that can start otel spans
#[derive(Debug, Default, Clone)]
pub struct SpanWriter {
	inner: Option<SpanWriterInner>,
}

impl SpanWriter {
	pub fn start(&self, name: impl Into<Cow<'static, str>>) -> SpanWriteOnDrop {
		match &self.inner {
			Some(i) => i.start(name),
			None => SpanWriteOnDrop::default(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct SpanWriterInner {
	parent: trc::TraceParent,
	inner: Arc<Mutex<Vec<BufferedSpan>>>,
}

impl SpanWriterInner {
	pub fn start(&self, name: impl Into<Cow<'static, str>>) -> SpanWriteOnDrop {
		// Create a unique child span ID for this recorded span.
		let child = self.parent.new_span();

		SpanWriteOnDrop {
			name: Some(name.into()),
			start_time: Some(SystemTime::now()),
			inner: self.inner.clone(),
			parent: Some(self.parent.clone()),
			span: Some(child),
		}
	}
}

#[derive(Default)]
pub struct SpanWriteOnDrop {
	name: Option<Cow<'static, str>>,
	start_time: Option<SystemTime>,
	inner: Arc<Mutex<Vec<BufferedSpan>>>,
	parent: Option<trc::TraceParent>,
	span: Option<trc::TraceParent>,
}
impl SpanWriteOnDrop {
	pub fn rename_span(&mut self, name: impl Into<Cow<'static, str>>) {
		if self.parent.is_some() {
			self.name = Some(name.into());
		}
	}
}
impl Drop for SpanWriteOnDrop {
	fn drop(&mut self) {
		let (Some(name), Some(parent), Some(span)) =
			(self.name.take(), self.parent.take(), self.span.take())
		else {
			return;
		};
		let end_time = SystemTime::now();

		// Store for later flush when the request log is finalized.
		if let Ok(mut spans) = self.inner.lock() {
			spans.push(BufferedSpan {
				name,
				span_kind: SpanKind::Server,
				start_time: self.start_time.unwrap_or(end_time),
				end_time,
				attributes: Vec::new(),
				parent,
				span,
			});
		}
	}
}

#[derive(Debug)]
pub struct BufferedSpan {
	name: Cow<'static, str>,
	span_kind: SpanKind,
	start_time: SystemTime,
	end_time: SystemTime,
	attributes: Vec<KeyValue>,
	parent: trc::TraceParent,
	span: trc::TraceParent,
}

impl BufferedSpan {
	fn into_span_data(self) -> opentelemetry_sdk::trace::SpanData {
		trc::trace_span_data(
			self.name,
			self.span_kind,
			&self.span,
			Some(&self.parent),
			self.start_time,
			self.end_time,
			self.attributes,
		)
	}
}

#[cfg(test)]
mod tests {
	use std::future::ready;
	use std::net::SocketAddr;
	use std::sync::{Arc, Mutex};
	use std::time::Instant;

	use opentelemetry::trace::SpanKind;
	use opentelemetry_sdk::error::OTelSdkResult;
	use opentelemetry_sdk::trace::{SimpleSpanProcessor, SpanData, SpanExporter};
	use prometheus_client::registry::Registry;

	use super::*;
	use crate::telemetry::metrics::Metrics;
	use crate::telemetry::trc;
	use crate::transport::stream::TCPConnectionInfo;

	#[derive(Clone, Debug, Default)]
	struct RecordingSpanExporter {
		spans: Arc<Mutex<Vec<SpanData>>>,
	}

	impl RecordingSpanExporter {
		fn finished_spans(&self) -> Vec<SpanData> {
			self.spans.lock().unwrap().clone()
		}
	}

	impl SpanExporter for RecordingSpanExporter {
		fn export(
			&self,
			batch: Vec<SpanData>,
		) -> impl std::future::Future<Output = OTelSdkResult> + Send {
			self.spans.lock().unwrap().extend(batch);
			ready(Ok(()))
		}
	}

	fn test_tracer() -> (Arc<trc::Tracer>, RecordingSpanExporter) {
		let exporter = RecordingSpanExporter::default();
		let processor = trc::SharedSpanProcessor::new(SimpleSpanProcessor::new(exporter.clone()));
		let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
			.with_span_processor(processor.clone())
			.build();
		(
			Arc::new(trc::Tracer {
				provider,
				processor,
				fields: Arc::new(LoggingFields::default()),
				filter: None,
			}),
			exporter,
		)
	}

	fn test_request_log() -> RequestLog {
		let cel = CelLogging {
			cel_context: crate::cel::ContextBuilder::new(),
			filter: None,
			fields: LoggingFields::default(),
			metric_fields: MetricFields::default(),
			database_fields: LoggingFields::default(),
		};
		let mut registry = Registry::default();
		let metrics = Arc::new(Metrics::new(&mut registry, Default::default()));
		RequestLog::new(
			cel,
			metrics,
			ModelCatalog::empty(),
			Timestamp::now(),
			TCPConnectionInfo {
				peer_addr: "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
				local_addr: "127.0.0.1:8080".parse::<SocketAddr>().unwrap(),
				start: Instant::now(),
				raw_peer_addr: None,
			},
		)
	}

	#[test]
	fn default_health_treats_non_zero_grpc_status_as_unhealthy() {
		let mut log = test_request_log();
		log.status = Some(http::StatusCode::OK);
		log.grpc_status.store(Some(13));
		assert!(DropOnLog::default_unhealthy(&log));

		log.grpc_status.store(Some(0));
		assert!(!DropOnLog::default_unhealthy(&log));
	}

	#[test]
	fn span_writer_flushes_recorded_spans_as_children_of_request_span() {
		let (tracer, exporter) = test_tracer();
		let mut request = test_request_log();
		request.tracer = Some(tracer.clone());

		let mut outgoing = trc::TraceParent::new();
		outgoing.flags = 1;
		request.outgoing_span = Some(outgoing.clone());

		{
			let _span = request.span_writer().start("buffered child span");
		}

		drop(DropOnLog::from(request));
		let _ = tracer.provider.force_flush();

		let spans = exporter.finished_spans();
		assert_eq!(spans.len(), 2);

		let child = spans
			.iter()
			.find(|span| span.name.as_ref() == "buffered child span")
			.expect("buffered span should be exported");
		assert_eq!(child.span_kind, SpanKind::Server);
		assert_eq!(child.parent_span_id, outgoing.span_id.into());
		assert_eq!(child.span_context.trace_id(), outgoing.trace_id.into());
		assert!(child.parent_span_is_remote);
	}

	#[test]
	fn span_writer_noops_for_unsampled_outgoing_span() {
		let (tracer, exporter) = test_tracer();
		let mut request = test_request_log();
		request.tracer = Some(tracer.clone());

		let mut outgoing = trc::TraceParent::new();
		outgoing.flags = 0;
		request.outgoing_span = Some(outgoing);

		{
			let mut span = request.span_writer().start("buffered child span");
			span.rename_span("renamed child span");
		}

		drop(DropOnLog::from(request));
		let _ = tracer.provider.force_flush();

		assert!(exporter.finished_spans().is_empty());
	}

	#[tokio::test]
	async fn llm_cost_breakdown_span_attributes() {
		let catalog_file = tempfile::NamedTempFile::new().unwrap();
		fs_err::write(
			catalog_file.path(),
			r#"{"providers":{"openai":{"models":{"my-model":{"rates":{"input":"1","output":"2"}}}}}}"#,
		)
		.unwrap();
		let catalog = ModelCatalog::new(vec![crate::ModelCatalogSource::File {
			file: catalog_file.path().to_path_buf(),
		}])
		.unwrap();
		let request = llm::LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Completions,
			native_format: None,
			cache_convention: llm::CacheTokenConvention::InputIncludesCache,
			request_model: strng::literal!("my-model"),
			provider: strng::literal!("openai"),
			streaming: false,
			params: llm::LLMRequestParams::default(),
			prompt: None,
		};
		let response = llm::LLMResponse {
			input_tokens: Some(1_000_000),
			output_tokens: Some(0),
			..Default::default()
		};
		for _ in 0..20 {
			let projection = catalog.project(&llm::LLMInfo::new(request.clone(), response.clone()));
			if projection.cost.is_some() {
				break;
			}
			tokio::time::sleep(Duration::from_millis(25)).await;
		}

		let (tracer, exporter) = test_tracer();
		let mut log = test_request_log();
		log.model_catalog = catalog;
		log.tracer = Some(tracer.clone());
		let mut outgoing = trc::TraceParent::new();
		outgoing.flags = 1;
		log.outgoing_span = Some(outgoing);
		log.llm_request = Some(request.clone());
		log
			.llm_response
			.store(Some(llm::LLMInfo::new(request, response)));

		drop(DropOnLog::from(log));
		let _ = tracer.provider.force_flush();

		let spans = exporter.finished_spans();
		let span = spans
			.iter()
			.find(|span| span.name.as_ref() == "unknown")
			.expect("request span should be exported");
		let has = |key: &str| span.attributes.iter().any(|attr| attr.key.as_str() == key);
		for expected in [
			"agw.ai.usage.cost.total", // lossless decimal total (also on the structured log)
			"agw.ai.usage.cost.input", // a span-only exact breakdown component
		] {
			assert!(has(expected), "expected {expected} span attribute");
		}
		assert!(
			span
				.attributes
				.iter()
				.all(|attr| attr.key.as_str() != "gen_ai.usage.cost"),
			"cost must not be emitted under the GenAI semantic convention namespace"
		);
		assert!(
			span
				.attributes
				.iter()
				.all(|attr| attr.key.as_str() != "agw.usage.cost"),
			"cost should use the AGW AI usage namespace"
		);
	}
}
