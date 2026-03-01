use std::borrow::Cow;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::{Duration, Instant, SystemTime};

use agent_core::metrics::CustomField;
use agent_core::strng;
use agent_core::strng::{RichStrng, Strng};
use agent_core::telemetry::{OptionExt, ValueBag, debug, display};
use bytes::Buf;
use crossbeam::atomic::AtomicCell;
use frozen_collections::{FzHashSet, FzStringMap};
use http_body::{Body, Frame, SizeHint};
use indexmap::IndexMap;
use itertools::Itertools;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, SpanBuilder, SpanKind, Tracer as _};
use serde::de::DeserializeOwned;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use tracing::{Level, trace};

use crate::cel::{ContextBuilder, Expression, LLMContext};
use crate::http::Request;
use crate::llm::InputFormat;
use crate::mcp::{MCPOperation, ResourceId, ResourceType};
use crate::proxy::ProxyResponseReason;
use crate::telemetry::metrics::{
	GenAILabels, GenAILabelsTokenUsage, HTTPLabels, MCPCall, MCPServerOperationDurationLabels,
	Metrics, RouteIdentifier,
};
use crate::telemetry::trc;
use crate::telemetry::trc::TraceParent;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::{BackendInfo, BindKey, ListenerName, RouteName, Target};
use crate::types::loadbalancer::ActiveHandle;
use crate::{cel, llm, mcp};

/// AsyncLog is a wrapper around an item that can be atomically set.
/// The intent is to provide additional info to the log after we have lost the RequestLog reference,
/// generally for things that rely on the response body.
#[derive(Clone)]
pub struct AsyncLog<T>(Arc<AtomicCell<Option<T>>>);

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

#[derive(serde::Serialize, Debug, Clone)]
pub struct Config {
	pub filter: Option<Arc<cel::Expression>>,
	pub fields: LoggingFields,
	pub metric_fields: Arc<MetricFields>,
	pub excluded_metrics: FzHashSet<String>,
	pub level: String,
	pub format: crate::LoggingFormat,
}

#[derive(serde::Serialize, Default, Clone, Debug)]
pub struct LoggingFields {
	pub remove: Arc<FzHashSet<String>>,
	pub add: Arc<OrderedStringMap<Arc<cel::Expression>>>,
}

#[derive(serde::Serialize, Default, Clone, Debug)]
pub struct MetricFields {
	pub add: OrderedStringMap<Arc<cel::Expression>>,
}

#[derive(Clone, Debug)]
pub struct OrderedStringMap<V> {
	map: FzStringMap<Box<str>, V>,
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
		let map: FzStringMap<Box<str>, V> = items.into_iter().collect();
		Self { map, order }
	}
}

impl LoggingFields {
	pub fn has(&self, k: &str) -> bool {
		self.remove.contains(k) || self.add.contains_key(k)
	}
}

#[derive(Debug)]
pub struct TraceSampler {
	pub random_sampling: Option<Arc<cel::Expression>>,
	pub client_sampling: Option<Arc<cel::Expression>>,
}

#[derive(Debug)]
pub struct CelLogging {
	pub cel_context: cel::ContextBuilder,
	pub filter: Option<Arc<cel::Expression>>,
	pub fields: LoggingFields,
	pub metric_fields: Arc<MetricFields>,
	pub tracing_sampler: TraceSampler,
}

pub struct CelLoggingExecutor<'a> {
	pub executor: cel::Executor<'a>,
	pub filter: &'a Option<Arc<cel::Expression>>,
	pub fields: &'a LoggingFields,
	pub metric_fields: &'a Arc<MetricFields>,
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
		// Fast path: keep borrowed key when no flattening is required.
		if !always_flatten && agent_celx::FlattenSignal::from_value(celv).is_none() {
			raws.push((k, celv.json().ok()));
			return;
		}

		let mut key = k.into_owned();
		Self::resolve_value_with_owned_key(raws, &mut key, celv, always_flatten);
	}

	fn with_key_suffix<T: std::fmt::Display, F: FnOnce(&mut String)>(
		key: &mut String,
		suffix: T,
		f: F,
	) {
		let base_len = key.len();
		use std::fmt::Write as _;
		if write!(key, ".{suffix}").is_ok() {
			f(key);
		}
		key.truncate(base_len);
	}

	fn resolve_value_with_owned_key(
		raws: &mut Vec<(Cow<'a, str>, Option<Value>)>,
		key: &mut String,
		celv: &cel::Value,
		always_flatten: bool,
	) {
		match agent_celx::FlattenSignal::from_value(celv) {
			Some(agent_celx::FlattenSignal::List(li)) => {
				raws.reserve(li.len());
				for (idx, v) in li.as_ref().iter().enumerate() {
					Self::with_key_suffix(key, idx, |key| {
						Self::resolve_value_with_owned_key(raws, key, v, false);
					});
				}
				return;
			},
			Some(agent_celx::FlattenSignal::ListRecursive(li)) => {
				raws.reserve(li.len());
				for (idx, v) in li.as_ref().iter().enumerate() {
					Self::with_key_suffix(key, idx, |key| {
						Self::resolve_value_with_owned_key(raws, key, v, true);
					});
				}
				return;
			},
			Some(agent_celx::FlattenSignal::Map(m)) => {
				raws.reserve(m.len());
				for (mk, mv) in m.iter() {
					Self::with_key_suffix(key, mk, |key| {
						Self::resolve_value_with_owned_key(raws, key, mv, false);
					});
				}
				return;
			},
			Some(agent_celx::FlattenSignal::MapRecursive(m)) => {
				raws.reserve(m.len());
				for (mk, mv) in m.iter() {
					Self::with_key_suffix(key, mk, |key| {
						Self::resolve_value_with_owned_key(raws, key, mv, true);
					});
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
						Self::with_key_suffix(key, idx, |key| {
							Self::resolve_value_with_owned_key(raws, key, v, true);
						});
					}
				},
				cel::Value::Map(m) => {
					raws.reserve(m.len());
					for (mk, mv) in m.iter() {
						Self::with_key_suffix(key, mk, |key| {
							Self::resolve_value_with_owned_key(raws, key, mv, true);
						});
					}
				},
				_ => raws.push((Cow::Owned(key.clone()), celv.json().ok())),
			}
		} else {
			raws.push((Cow::Owned(key.clone()), celv.json().ok()));
		}
	}

	fn eval_additions(&self) -> Vec<(Cow<str>, Option<Value>)> {
		self.eval(&self.fields.add)
	}
}

impl CelLogging {
	pub fn new(cfg: Config, tracing_config: trc::Config) -> Self {
		let mut cel_context = cel::ContextBuilder::new();
		if let Some(f) = &cfg.filter {
			cel_context.register_expression(f.as_ref());
		}
		for v in cfg.fields.add.values_unordered() {
			cel_context.register_expression(v.as_ref());
		}
		for v in cfg.metric_fields.add.values_unordered() {
			cel_context.register_expression(v.as_ref());
		}

		Self {
			cel_context,
			filter: cfg.filter,
			fields: cfg.fields,
			metric_fields: cfg.metric_fields,
			tracing_sampler: TraceSampler {
				random_sampling: tracing_config.random_sampling,
				client_sampling: tracing_config.client_sampling,
			},
		}
	}

	pub fn register(&mut self, fields: &LoggingFields) {
		for v in fields.add.values_unordered() {
			self.cel_context.register_expression(v.as_ref());
		}
	}

	pub fn ctx(&mut self) -> &mut ContextBuilder {
		&mut self.cel_context
	}

	pub fn build<'a>(
		&'a self,
		req: Option<&'a cel::RequestSnapshot>,
		resp: Option<&'a cel::ResponseSnapshot>,
		llm_response: Option<&'a LLMContext>,
		mcp: Option<&'a ResourceType>,
		end_time: Option<&'a str>,
		source_context: Option<&'a cel::SourceContext>,
	) -> Result<CelLoggingExecutor<'a>, cel::Error> {
		let CelLogging {
			cel_context: _,
			filter,
			fields,
			metric_fields,
			tracing_sampler: _,
		} = self;
		let executor = if req.is_none() && source_context.is_some() {
			// TCP case: use new_tcp_logger
			cel::Executor::new_tcp_logger(source_context, end_time)
		} else {
			// HTTP case: use new_logger
			cel::Executor::new_logger(req, resp, llm_response, mcp, end_time)
		};
		Ok(CelLoggingExecutor {
			executor,
			filter,
			fields,
			metric_fields,
		})
	}
}

#[derive(Debug)]
pub struct DropOnLog {
	log: Option<RequestLog>,
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

	fn add_llm_metrics(
		log: &RequestLog,
		route_identifier: &RouteIdentifier,
		end_time: Instant,
		duration: Duration,
		llm_response: Option<&LLMContext>,
		custom_metric_fields: &CustomField,
	) {
		if let Some(llm_response) = llm_response {
			let provider = normalize_gen_ai_provider(llm_response.provider.as_str());
			let operation_name = log
				.llm_request
				.as_ref()
				.map(|r| gen_ai_operation_name(r.input_format))
				.unwrap_or("chat");
			let gen_ai_labels = Arc::new(GenAILabels {
				gen_ai_operation_name: strng::new(operation_name).into(),
				gen_ai_system: strng::new(provider.as_ref()).into(),
				gen_ai_request_model: llm_response.request_model.clone().into(),
				gen_ai_response_model: llm_response.response_model.clone().into(),
				custom: custom_metric_fields.clone(),
				route: route_identifier.clone(),
			});
			let input_tokens = llm_response
				.input_tokens
				.or(llm_response.count_tokens)
				.or_else(|| log.llm_request.as_ref().and_then(|r| r.input_tokens));
			if let Some(it) = input_tokens {
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
			if let Some(tt) = llm_response.total_tokens.or(input_tokens) {
				log
					.metrics
					.gen_ai_token_usage
					.get_or_create(&GenAILabelsTokenUsage {
						gen_ai_token_type: strng::literal!("total").into(),
						common: gen_ai_labels.clone().into(),
					})
					.observe(tt as f64)
			}
			log
				.metrics
				.gen_ai_request_duration
				.get_or_create(&gen_ai_labels)
				.observe(duration.as_secs_f64());
			if let Some(ft) = llm_response.first_token {
				let ttft = ft - log.start;
				// Duration from start of request to first token
				// This is the start of when WE got the request, but it should probably be when we SENT the upstream.
				log
					.metrics
					.gen_ai_time_to_first_token
					.get_or_create(&gen_ai_labels)
					.observe(ttft.as_secs_f64());

				if let Some(ot) = llm_response.output_tokens {
					let first_to_last = end_time - ft;
					let throughput = first_to_last.as_secs_f64() / (ot as f64);
					log
						.metrics
						.gen_ai_time_per_output_token
						.get_or_create(&gen_ai_labels)
						.observe(throughput);
				}
			}
		}
	}
}

pub(crate) fn gen_ai_operation_name(input_format: InputFormat) -> &'static str {
	match input_format {
		InputFormat::Completions => "chat",
		InputFormat::Embeddings => "embeddings",
		InputFormat::CountTokens => "token_count",
		InputFormat::Messages | InputFormat::Responses | InputFormat::Realtime => "chat",
	}
}

pub(crate) fn mcp_gen_ai_operation_name(method_name: Option<&str>) -> Option<&'static str> {
	match method_name {
		Some("tools/call") => Some("execute_tool"),
		Some("prompts/get") => Some("chat"),
		Some("resources/read") => Some("retrieval"),
		Some("resources/list") => Some("retrieval"),
		Some("resources/templates/list") => Some("retrieval"),
		_ => None,
	}
}

pub(crate) fn normalize_gen_ai_provider(provider: &str) -> Cow<'_, str> {
	if provider.eq_ignore_ascii_case("bedrock") {
		Cow::Borrowed("aws.bedrock")
	} else if provider.eq_ignore_ascii_case("vertex") {
		Cow::Borrowed("gcp.vertex_ai")
	} else if provider.eq_ignore_ascii_case("gemini") {
		Cow::Borrowed("gcp.gemini")
	} else if provider.eq_ignore_ascii_case("azureopenai")
		|| provider.eq_ignore_ascii_case("azure_openai")
		|| provider.eq_ignore_ascii_case("azure-openai")
	{
		Cow::Borrowed("azure.ai.openai")
	} else {
		Cow::Borrowed(provider)
	}
}

fn gen_ai_server_address_port(log: &RequestLog) -> (Option<String>, Option<i64>) {
	match log.endpoint.as_ref() {
		Some(Target::Address(addr)) => (Some(addr.ip().to_string()), Some(i64::from(addr.port()))),
		Some(Target::Hostname(host, port)) => (Some(host.to_string()), Some(i64::from(*port))),
		Some(Target::UnixSocket(path)) => (path.to_str().map(str::to_string), None),
		None => (None, None),
	}
}

impl From<RequestLog> for DropOnLog {
	fn from(log: RequestLog) -> Self {
		Self { log: Some(log) }
	}
}

impl RequestLog {
	pub fn new(
		cel: CelLogging,
		metrics: Arc<Metrics>,
		start: Instant,
		tcp_info: TCPConnectionInfo,
	) -> Self {
		RequestLog {
			cel,
			metrics,
			start,
			tcp_info,
			tls_info: None,
			tracer: None,
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
}

#[derive(Debug)]
pub struct RequestLog {
	pub cel: CelLogging,
	pub metrics: Arc<Metrics>,
	pub start: Instant,
	pub tcp_info: TCPConnectionInfo,

	// Set only for TLS traffic
	pub tls_info: Option<TLSConnectionInfo>,

	// Set only if the trace is sampled
	pub tracer: Option<std::sync::Arc<trc::Tracer>>,

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

	pub jwt_sub: Option<String>,

	pub retry_attempt: Option<u8>,
	pub error: Option<String>,

	pub grpc_status: AsyncLog<u8>,
	pub mcp_status: AsyncLog<mcp::MCPInfo>,

	pub incoming_span: Option<trc::TraceParent>,
	pub outgoing_span: Option<trc::TraceParent>,

	pub llm_request: Option<llm::LLMRequest>,
	pub llm_response: AsyncLog<llm::LLMInfo>,

	pub a2a_method: Option<&'static str>,

	pub inference_pool: Option<SocketAddr>,

	pub request_handle: Option<ActiveHandle>,
	pub request_snapshot: Option<cel::RequestSnapshot>,
	pub response_snapshot: Option<cel::ResponseSnapshot>,
	/// Source context for TCP connections (where we don't have an HTTP request)
	pub source_context: Option<cel::SourceContext>,

	pub response_bytes: u64,
}

impl RequestLog {
	pub fn trace_sampled(&self, req: &Request, tp: Option<&TraceParent>) -> bool {
		let TraceSampler {
			random_sampling,
			client_sampling,
		} = &self.cel.tracing_sampler;
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

	pub fn span_writer(&self) -> Option<SpanWriter> {
		let parent = self.outgoing_span.clone()?;
		if !parent.is_sampled() {
			return None;
		}
		let tracer = self.tracer.clone()?;
		Some(SpanWriter { parent, tracer })
	}
}

#[derive(Clone, Debug)]
pub struct SpanWriter {
	parent: trc::TraceParent,
	tracer: Arc<trc::Tracer>,
}

impl SpanWriter {
	#[cfg(test)]
	pub(crate) fn for_test(parent: trc::TraceParent, tracer: Arc<trc::Tracer>) -> Self {
		Self { parent, tracer }
	}

	pub fn write(
		&self,
		name: impl Into<Cow<'static, str>>,
		f: impl FnOnce(SpanBuilder) -> SpanBuilder,
	) {
		let now = SystemTime::now();
		let mut sb = self
			.tracer
			.tracer
			.span_builder(name)
			.with_start_time(now)
			.with_end_time(now)
			.with_kind(SpanKind::Client);
		sb = f(sb);
		let parent_ctx = trc::remote_parent_context(&self.parent);
		sb.start_with_context(self.tracer.tracer.as_ref(), &parent_ctx)
			.end()
	}

	/// Start a CLIENT span that stays open until explicitly ended or dropped.
	///
	/// This is useful for operation spans that must parent multiple downstream
	/// child spans emitted over the lifetime of a request phase.
	pub fn start(
		&self,
		name: impl Into<Cow<'static, str>>,
		f: impl FnOnce(SpanBuilder) -> SpanBuilder,
	) -> StartedSpan {
		let now = SystemTime::now();
		let mut sb = self
			.tracer
			.tracer
			.span_builder(name)
			.with_start_time(now)
			.with_kind(SpanKind::Client);
		sb = f(sb);
		let parent_ctx = trc::remote_parent_context(&self.parent);
		let span = sb.start_with_context(self.tracer.tracer.as_ref(), &parent_ctx);
		let span_ctx = span.span_context();
		let traceparent = trc::TraceParent {
			version: 0,
			trace_id: u128::from_be_bytes(span_ctx.trace_id().to_bytes()),
			span_id: u64::from_be_bytes(span_ctx.span_id().to_bytes()),
			flags: span_ctx.trace_flags().to_u8(),
		};
		StartedSpan {
			span: Some(span),
			child_writer: self.with_parent(traceparent),
		}
	}

	/// Create a child traceparent under the request span carried by this writer.
	/// This is used to make downstream MCP calls children of an operation span.
	pub fn child_traceparent(&self) -> trc::TraceParent {
		self.parent.new_span()
	}

	/// Create a new writer that uses `parent` as its remote parent context.
	/// Useful when nested operation spans need their own child writer topology.
	pub fn with_parent(&self, parent: trc::TraceParent) -> SpanWriter {
		SpanWriter {
			parent,
			tracer: self.tracer.clone(),
		}
	}

	/// Create a child traceparent and a writer whose parent is that child.
	/// The returned writer can be passed down to make descendants children of
	/// the returned operation traceparent.
	pub fn child_writer(&self) -> (trc::TraceParent, SpanWriter) {
		let child = self.child_traceparent();
		(child.clone(), self.with_parent(child))
	}

	/// Write a span with an explicit trace/span ID while preserving the writer's parent.
	///
	/// Use this when an externally propagated `traceparent` must match this span's context.
	pub fn write_with_traceparent(
		&self,
		traceparent: &trc::TraceParent,
		name: impl Into<Cow<'static, str>>,
		f: impl FnOnce(SpanBuilder) -> SpanBuilder,
	) {
		let now = SystemTime::now();
		let mut sb = self
			.tracer
			.tracer
			.span_builder(name)
			.with_start_time(now)
			.with_end_time(now)
			.with_kind(SpanKind::Client)
			.with_trace_id(traceparent.trace_id.into())
			.with_span_id(traceparent.span_id.into());
		sb = f(sb);
		let parent_ctx = trc::remote_parent_context(&self.parent);
		sb.start_with_context(self.tracer.tracer.as_ref(), &parent_ctx)
			.end()
	}

	/// Truncate a string attribute value to at most `max_bytes` bytes (UTF-8 boundary safe).
	/// Returns the original string if it is within the limit or if `max_bytes` is `None`.
	pub fn truncate_attr(value: String, max_bytes: Option<usize>) -> String {
		let Some(limit) = max_bytes else {
			return value;
		};
		if value.len() <= limit {
			return value;
		}
		// Truncate at a valid UTF-8 char boundary.
		let mut end = limit;
		while !value.is_char_boundary(end) {
			end -= 1;
		}
		format!("{}…", &value[..end])
	}

	pub fn attrs(items: impl IntoIterator<Item = (&'static str, String)>) -> Vec<KeyValue> {
		items
			.into_iter()
			.map(|(k, v)| KeyValue::new(k, v))
			.collect_vec()
	}
}

pub struct StartedSpan {
	span: Option<opentelemetry_sdk::trace::Span>,
	child_writer: SpanWriter,
}

impl StartedSpan {
	pub fn child_writer(&self) -> SpanWriter {
		self.child_writer.clone()
	}

	pub fn traceparent(&self) -> &trc::TraceParent {
		&self.child_writer.parent
	}

	pub fn set_attributes(&mut self, attrs: impl IntoIterator<Item = KeyValue>) {
		if let Some(span) = self.span.as_mut() {
			span.set_attributes(attrs);
		}
	}

	pub fn add_event(&mut self, event: opentelemetry::trace::Event) {
		if let Some(span) = self.span.as_mut() {
			span.add_event_with_timestamp(event.name, event.timestamp, event.attributes);
		}
	}

	pub fn set_status(&mut self, status: opentelemetry::trace::Status) {
		if let Some(span) = self.span.as_mut() {
			span.set_status(status);
		}
	}

	pub fn end(&mut self) {
		if let Some(mut span) = self.span.take() {
			span.end();
		}
	}
}

impl Drop for StartedSpan {
	fn drop(&mut self) {
		self.end();
	}
}

impl Drop for DropOnLog {
	fn drop(&mut self) {
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

		let enable_custom_metrics = !log.cel.metric_fields.add.is_empty();

		let enable_trace = log.tracer.is_some();
		// We will later check it also matches a filter, but filter is slower
		let maybe_enable_log = agent_core::telemetry::enabled("request", &Level::INFO);
		if !maybe_enable_log && !enable_trace && !enable_custom_metrics {
			// Report our non-customized metrics
			if !is_tcp {
				log.metrics.requests.get_or_create(&http_labels).inc();
			}
			return;
		}

		let end_time = Instant::now();
		let duration = end_time - log.start;
		if let Some(rh) = log.request_handle.take() {
			let status = log
				.status
				.unwrap_or(crate::http::StatusCode::INTERNAL_SERVER_ERROR);
			let health = !status.is_server_error() && !status.is_client_error();
			rh.finish_request(health, duration, log.retry_after);
		}

		let llm_response = log.llm_response.take().map(Into::into);

		let mcp = log.mcp_status.take();
		let mcp_cel = mcp.as_ref().and_then(|m| {
			let resource = ResourceId::new(
				m.target_name.as_deref()?.to_string(),
				m.resource_name.as_deref()?.to_string(),
			);
			match m.resource {
				Some(MCPOperation::Prompt) => Some(ResourceType::Prompt(resource)),
				Some(MCPOperation::Tool) => Some(ResourceType::Tool(resource)),
				Some(MCPOperation::Resource) => Some(ResourceType::Resource(resource)),
				_ => None,
			}
		});
		let end_time_str = agent_core::telemetry::render_current_time();
		let Ok(cel_exec) = log.cel.build(
			log.request_snapshot.as_ref(),
			log.response_snapshot.as_ref(),
			llm_response.as_ref(),
			mcp_cel.as_ref(),
			Some(&end_time_str),
			log.source_context.as_ref(),
		) else {
			tracing::warn!("failed to build CEL context");
			return;
		};

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

		Self::add_llm_metrics(
			&log,
			&route_identifier,
			end_time,
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
					resource_type: mcp.resource.into(),
					server: mcp.target_name.as_ref().map(RichStrng::from).into(),
					resource: mcp.resource_name.as_ref().map(RichStrng::from).into(),

					route: route_identifier.clone(),
					custom: custom_metric_fields.clone(),
				})
				.inc();
			let operation_error_type =
				if log.error.is_some() || log.status.as_ref().is_some_and(|s| s.is_server_error()) {
					log
						.status
						.as_ref()
						.map(|s| s.as_u16().to_string())
						.or_else(|| log.error.clone())
				} else {
					None
				};
			log
				.metrics
				.mcp_server_operation_duration
				.get_or_create(&MCPServerOperationDurationLabels {
					mcp_method_name: mcp.method_name.as_ref().map(RichStrng::from).into(),
					error_type: operation_error_type.as_ref().map(RichStrng::from).into(),
					route: route_identifier.clone(),
					custom: custom_metric_fields.clone(),
				})
				.observe(duration.as_secs_f64());
		}

		let enable_logs = maybe_enable_log && cel_exec.eval_filter();
		if !enable_logs && !enable_trace {
			return;
		}

		let dur = format!("{}ms", duration.as_millis());
		let grpc = log.grpc_status.load();

		let input_tokens = llm_response
			.as_ref()
			.and_then(|l| l.input_tokens.or(l.count_tokens))
			.or_else(|| log.llm_request.as_ref().and_then(|r| r.input_tokens));
		let total_tokens = llm_response
			.as_ref()
			.and_then(|l| l.total_tokens.or(l.count_tokens))
			.or(input_tokens);
		let gen_ai_response_finish_reasons = llm_response
			.as_ref()
			.and_then(|l| l.finish_reasons.as_ref())
			.filter(|v| !v.is_empty())
			.map(|v| v.iter().map(|s| s.as_str()).join(","));
		let gen_ai_request_stop_sequences = log
			.llm_request
			.as_ref()
			.and_then(|l| l.params.stop_sequences.as_ref())
			.filter(|v| !v.is_empty())
			.map(|v| v.iter().map(|s| s.as_str()).join(","));
		let (gen_ai_server_address, gen_ai_server_port) = if log.llm_request.is_some() {
			gen_ai_server_address_port(&log)
		} else {
			(None, None)
		};
		let mcp_method_name = mcp.as_ref().and_then(|m| m.method_name.as_deref());
		let mcp_target_name = mcp.as_ref().and_then(|m| m.target_name.as_deref());
		let mcp_resource_name = mcp.as_ref().and_then(|m| m.resource_name.as_deref());
		let mcp_resource_type = mcp.as_ref().and_then(|m| m.resource);
		let mcp_protocol_version = mcp.as_ref().and_then(|m| m.protocol_version.as_deref());
		let jsonrpc_request_id = mcp.as_ref().and_then(|m| m.jsonrpc_request_id.as_deref());
		let mcp_gen_ai_operation = mcp_gen_ai_operation_name(mcp_method_name);
		let gen_ai_agent_name = log
			.a2a_method
			.filter(|method| *method != "unknown")
			.and_then(|_| {
				log
					.backend_info
					.as_ref()
					.map(|backend| &backend.backend_name)
			});
		let gen_ai_operation = log
			.llm_request
			.as_ref()
			.map(|r| gen_ai_operation_name(r.input_format))
			.or(mcp_gen_ai_operation)
			.or_else(|| {
				log
					.a2a_method
					.filter(|method| *method != "unknown")
					.map(|_| "invoke_agent")
			});
		let gen_ai_tool_name = if mcp_gen_ai_operation == Some("execute_tool") {
			mcp_resource_name
		} else {
			None
		};
		let gen_ai_prompt_name = if mcp_resource_type == Some(MCPOperation::Prompt) {
			mcp_resource_name
		} else {
			None
		};
		let gen_ai_data_source_id = if mcp_gen_ai_operation == Some("retrieval") {
			mcp_target_name
		} else {
			None
		};
		let gen_ai_provider = log
			.llm_request
			.as_ref()
			.map(|l| normalize_gen_ai_provider(l.provider.as_str()).into_owned());

		let trace_id = log.outgoing_span.as_ref().map(|id| id.trace_id());
		let span_id = log.outgoing_span.as_ref().map(|id| id.span_id());

		let fields = cel_exec.fields;
		let reason = log.reason.and_then(|r| match r {
			ProxyResponseReason::Upstream => None,
			_ => Some(r),
		});
		let error_type =
			if log.error.is_some() || log.status.as_ref().is_some_and(|s| s.as_u16() >= 400) {
				grpc
					.map(|s| s.to_string())
					.or_else(|| log.status.as_ref().map(|s| s.as_u16().to_string()))
					.or_else(|| reason.map(|r| format!("{r:?}")))
			} else {
				None
			};

		let mut kv = vec![
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
			("http.request.method", log.method.display()),
			("http.host", log.host.display()),
			(
				"server.address",
				gen_ai_server_address
					.as_deref()
					.map(Into::into)
					.or_else(|| log.host.display()),
			),
			("server.port", gen_ai_server_port.map(Into::into)),
			("http.path", log.path.display()),
			("url.path", log.path.display()),
			("http.version", log.version.as_ref().map(debug)),
			(
				"network.protocol.version",
				log
					.version
					.as_ref()
					.map(crate::http::version_str)
					.map(Into::into),
			),
			(
				"http.status",
				log.status.as_ref().map(|s| s.as_u16().into()),
			),
			(
				"http.response.status_code",
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
			("mcp.method", mcp_method_name.map(Into::into)),
			("mcp.method.name", mcp_method_name.map(Into::into)),
			("mcp.protocol.version", mcp_protocol_version.map(Into::into)),
			("jsonrpc.request.id", jsonrpc_request_id.map(Into::into)),
			("mcp.target", mcp_target_name.map(Into::into)),
			("mcp.resource.type", mcp_resource_type.as_ref().map(display)),
			("mcp.resource.name", mcp_resource_name.map(Into::into)),
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
			("gen_ai.operation.name", gen_ai_operation.map(Into::into)),
			("gen_ai.agent.name", gen_ai_agent_name.map(display)),
			// Phase 3: conversation identity — use mcp.session.id for MCP flows (source order 2).
			(
				"gen_ai.conversation.id",
				mcp
					.as_ref()
					.and_then(|m| m.session_id.as_ref())
					.map(display),
			),
			(
				"gen_ai.provider.name",
				gen_ai_provider.as_ref().map(display),
			),
			("gen_ai.tool.name", gen_ai_tool_name.map(Into::into)),
			("gen_ai.prompt.name", gen_ai_prompt_name.map(Into::into)),
			(
				"gen_ai.data_source.id",
				gen_ai_data_source_id.map(Into::into),
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
			(
				"gen_ai.response.id",
				llm_response.as_ref().and_then(|l| l.response_id.display()),
			),
			(
				"gen_ai.response.finish_reasons",
				gen_ai_response_finish_reasons.as_deref().map(Into::into),
			),
			("gen_ai.usage.input_tokens", input_tokens.map(Into::into)),
			(
				"gen_ai.usage.output_tokens",
				llm_response
					.as_ref()
					.and_then(|l| l.output_tokens)
					.map(Into::into),
			),
			("gen_ai.usage.total_tokens", total_tokens.map(Into::into)),
			(
				"gen_ai.usage.reasoning_tokens",
				llm_response
					.as_ref()
					.and_then(|l| l.reasoning_tokens)
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
				"gen_ai.usage.cache_creation.input_tokens",
				llm_response
					.as_ref()
					.and_then(|l| l.cache_creation_input_tokens)
					.map(Into::into),
			),
			(
				"gen_ai.usage.cached_input_tokens",
				llm_response
					.as_ref()
					.and_then(|l| l.cached_input_tokens)
					.map(Into::into),
			),
			(
				"gen_ai.usage.cache_creation_input_tokens",
				llm_response
					.as_ref()
					.and_then(|l| l.cache_creation_input_tokens)
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
				"gen_ai.request.top_k",
				log
					.llm_request
					.as_ref()
					.and_then(|l| l.params.top_k)
					.map(|v| (v as f64).into()),
			),
			(
				"gen_ai.request.choice.count",
				log
					.llm_request
					.as_ref()
					.and_then(|l| l.params.choice_count)
					.map(|v| (v as i64).into()),
			),
			(
				"gen_ai.request.stop_sequences",
				gen_ai_request_stop_sequences.as_deref().map(Into::into),
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
			("error.type", error_type.as_ref().map(display)),
			("error", log.error.quoted()),
			("reason", reason.display()),
			("duration", Some(dur.as_str().into())),
		];
		let mut trace_extra_attrs = Vec::new();
		let mut trace_suppressed_keys = Vec::new();
		if let Some(finish_reasons) = llm_response
			.as_ref()
			.and_then(|l| l.finish_reasons.as_ref())
			.filter(|v| !v.is_empty())
		{
			trace_suppressed_keys.push("gen_ai.response.finish_reasons");
			trace_extra_attrs.push(KeyValue::new(
				"gen_ai.response.finish_reasons",
				opentelemetry::Value::Array(opentelemetry::Array::String(
					finish_reasons
						.iter()
						.map(|s| s.to_string().into())
						.collect_vec(),
				)),
			));
		}
		if let Some(stop_sequences) = log
			.llm_request
			.as_ref()
			.and_then(|l| l.params.stop_sequences.as_ref())
			.filter(|v| !v.is_empty())
		{
			trace_suppressed_keys.push("gen_ai.request.stop_sequences");
			trace_extra_attrs.push(KeyValue::new(
				"gen_ai.request.stop_sequences",
				opentelemetry::Value::Array(opentelemetry::Array::String(
					stop_sequences
						.iter()
						.map(|s| s.to_string().into())
						.collect_vec(),
				)),
			));
		}
		if let Some(encoding_format) = log
			.llm_request
			.as_ref()
			.and_then(|l| l.params.encoding_format.as_ref())
		{
			trace_suppressed_keys.push("gen_ai.request.encoding_formats");
			trace_extra_attrs.push(KeyValue::new(
				"gen_ai.request.encoding_formats",
				opentelemetry::Value::Array(opentelemetry::Array::String(vec![
					encoding_format.to_string().into(),
				])),
			));
		}
		if enable_trace && let Some(t) = &log.tracer {
			t.send(
				&log,
				&cel_exec,
				kv.as_slice(),
				trace_extra_attrs.as_slice(),
				trace_suppressed_keys.as_slice(),
			)
		};
		if enable_logs {
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
				// TODO: convert directly instead of via json()
				let eval = v.as_ref().map(ValueBag::capture_serde1);
				kv.push((k, eval));
			}

			agent_core::telemetry::log("info", "request", &kv);
		}
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

#[cfg(test)]
mod tests {
	use super::*;
	use opentelemetry::trace::TracerProvider;
	use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
	use opentelemetry_sdk::trace::{SdkTracerProvider, SimpleSpanProcessor, SpanData};
	use std::sync::Mutex;
	use std::time::Duration;

	#[derive(Clone, Debug, Default)]
	struct CaptureExporter {
		spans: Arc<Mutex<Vec<SpanData>>>,
	}

	impl CaptureExporter {
		fn get_finished_spans(&self) -> Vec<SpanData> {
			self.spans.lock().expect("spans lock").clone()
		}
	}

	impl opentelemetry_sdk::trace::SpanExporter for CaptureExporter {
		fn export(
			&self,
			mut batch: Vec<SpanData>,
		) -> impl std::future::Future<Output = OTelSdkResult> + Send {
			let spans = self.spans.clone();
			async move {
				spans
					.lock()
					.map_err(|e| OTelSdkError::InternalFailure(e.to_string()))?
					.append(&mut batch);
				Ok(())
			}
		}

		fn shutdown_with_timeout(&mut self, _timeout: Duration) -> OTelSdkResult {
			Ok(())
		}
	}

	#[test]
	fn span_writer_emits_child_span_with_remote_parent() {
		let exporter = CaptureExporter::default();
		let provider = SdkTracerProvider::builder()
			.with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
			.build();
		let sdk_tracer = provider.tracer("telemetry-log-tests");

		let wrapped_tracer = Arc::new(trc::Tracer {
			tracer: Arc::new(sdk_tracer),
			provider,
			fields: Arc::new(LoggingFields::default()),
		});

		let parent = trc::TraceParent {
			version: 0,
			trace_id: 0x1122,
			span_id: 0x3344,
			flags: 0x01,
		};

		let writer = SpanWriter {
			parent,
			tracer: wrapped_tracer,
		};
		writer.write("upstream test", |sb| {
			sb.with_attributes(vec![
				KeyValue::new("network.protocol.name", "http"),
				KeyValue::new("url.path", "/mcp"),
			])
		});

		let spans = exporter.get_finished_spans();
		assert_eq!(spans.len(), 1);
		let span = &spans[0];
		assert_eq!(span.name.as_ref(), "upstream test");
		assert_eq!(
			u128::from_be_bytes(span.span_context.trace_id().to_bytes()),
			0x1122
		);
		assert_eq!(u64::from_be_bytes(span.parent_span_id.to_bytes()), 0x3344);
		assert!(span.parent_span_is_remote);
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "network.protocol.name" && kv.value.as_str() == "http")
		);
	}

	#[test]
	fn span_writer_can_emit_span_with_explicit_traceparent() {
		let exporter = CaptureExporter::default();
		let provider = SdkTracerProvider::builder()
			.with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
			.build();
		let sdk_tracer = provider.tracer("telemetry-log-tests-explicit");

		let wrapped_tracer = Arc::new(trc::Tracer {
			tracer: Arc::new(sdk_tracer),
			provider,
			fields: Arc::new(LoggingFields::default()),
		});

		let parent = trc::TraceParent {
			version: 0,
			trace_id: 0x7788,
			span_id: 0x99aa,
			flags: 0x01,
		};
		let writer = SpanWriter {
			parent: parent.clone(),
			tracer: wrapped_tracer,
		};
		let operation_tp = writer.child_traceparent();
		writer.write_with_traceparent(&operation_tp, "mcp tools/call", |sb| sb);

		let spans = exporter.get_finished_spans();
		assert_eq!(spans.len(), 1);
		let span = &spans[0];
		assert_eq!(
			u128::from_be_bytes(span.span_context.trace_id().to_bytes()),
			operation_tp.trace_id
		);
		assert_eq!(
			u64::from_be_bytes(span.span_context.span_id().to_bytes()),
			operation_tp.span_id
		);
		assert_eq!(
			u64::from_be_bytes(span.parent_span_id.to_bytes()),
			parent.span_id
		);
		assert!(span.parent_span_is_remote);
	}

	#[test]
	fn span_topology_mcp_fanout_and_retry() {
		use opentelemetry::trace::{Span as _, Tracer as _};

		let exporter = CaptureExporter::default();
		let provider = SdkTracerProvider::builder()
			.with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
			.build();
		let sdk_tracer = provider.tracer("telemetry-log-topology-tests");

		let wrapped_tracer = Arc::new(trc::Tracer {
			tracer: Arc::new(sdk_tracer),
			provider,
			fields: Arc::new(LoggingFields::default()),
		});

		let incoming = trc::TraceParent {
			version: 0,
			trace_id: 0xfeed,
			span_id: 0xaaa1,
			flags: 0x01,
		};
		let outgoing = trc::TraceParent {
			version: 0,
			trace_id: incoming.trace_id,
			span_id: 0xbbb2,
			flags: incoming.flags,
		};

		let request_parent = trc::remote_parent_context(&incoming);
		wrapped_tracer
			.tracer
			.span_builder("POST /mcp")
			.with_kind(SpanKind::Server)
			.with_trace_id(outgoing.trace_id.into())
			.with_span_id(outgoing.span_id.into())
			.start_with_context(wrapped_tracer.tracer.as_ref(), &request_parent)
			.end();

		let writer = SpanWriter {
			parent: outgoing.clone(),
			tracer: wrapped_tracer.clone(),
		};
		writer.write("upstream fanout mcp-a", |sb| {
			sb.with_attributes(vec![
				KeyValue::new("network.protocol.name", "http"),
				KeyValue::new("mcp.target", "mcp-a"),
			])
		});
		writer.write("upstream retry mcp-a", |sb| {
			sb.with_attributes(vec![
				KeyValue::new("network.protocol.name", "http"),
				KeyValue::new("mcp.target", "mcp-a"),
				KeyValue::new("gateway.retry_attempt", "1"),
			])
		});

		let spans = exporter.get_finished_spans();
		assert_eq!(spans.len(), 3);
		let request = spans
			.iter()
			.find(|s| s.name.as_ref() == "POST /mcp")
			.expect("request span");
		let fanout = spans
			.iter()
			.find(|s| s.name.as_ref() == "upstream fanout mcp-a")
			.expect("fanout child span");
		let retry = spans
			.iter()
			.find(|s| s.name.as_ref() == "upstream retry mcp-a")
			.expect("retry child span");

		let trace_id = u128::from_be_bytes(request.span_context.trace_id().to_bytes());
		assert_eq!(trace_id, incoming.trace_id);
		assert_eq!(
			trace_id,
			u128::from_be_bytes(fanout.span_context.trace_id().to_bytes())
		);
		assert_eq!(
			trace_id,
			u128::from_be_bytes(retry.span_context.trace_id().to_bytes())
		);

		assert_eq!(
			u64::from_be_bytes(request.parent_span_id.to_bytes()),
			incoming.span_id
		);
		assert!(request.parent_span_is_remote);
		assert_eq!(
			u64::from_be_bytes(fanout.parent_span_id.to_bytes()),
			outgoing.span_id
		);
		assert_eq!(
			u64::from_be_bytes(retry.parent_span_id.to_bytes()),
			outgoing.span_id
		);
		assert!(fanout.parent_span_is_remote);
		assert!(retry.parent_span_is_remote);
		assert_eq!(request.span_kind, SpanKind::Server);
		assert_eq!(fanout.span_kind, SpanKind::Client);
		assert_eq!(retry.span_kind, SpanKind::Client);

		assert!(
			fanout
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "mcp.target" && kv.value.as_str() == "mcp-a")
		);
		assert!(
			retry
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "gateway.retry_attempt" && kv.value.as_str() == "1")
		);
	}

	#[test]
	fn gen_ai_operation_name_maps_all_input_formats() {
		assert_eq!(gen_ai_operation_name(InputFormat::Completions), "chat");
		assert_eq!(gen_ai_operation_name(InputFormat::Embeddings), "embeddings");
		assert_eq!(
			gen_ai_operation_name(InputFormat::CountTokens),
			"token_count"
		);
		assert_eq!(gen_ai_operation_name(InputFormat::Messages), "chat");
		assert_eq!(gen_ai_operation_name(InputFormat::Responses), "chat");
		assert_eq!(gen_ai_operation_name(InputFormat::Realtime), "chat");
	}

	#[test]
	fn mcp_gen_ai_operation_name_maps_supported_methods() {
		assert_eq!(
			mcp_gen_ai_operation_name(Some("tools/call")),
			Some("execute_tool")
		);
		assert_eq!(mcp_gen_ai_operation_name(Some("prompts/get")), Some("chat"));
		assert_eq!(
			mcp_gen_ai_operation_name(Some("resources/read")),
			Some("retrieval")
		);
		assert_eq!(
			mcp_gen_ai_operation_name(Some("resources/list")),
			Some("retrieval")
		);
		assert_eq!(
			mcp_gen_ai_operation_name(Some("resources/templates/list")),
			Some("retrieval")
		);
		assert_eq!(mcp_gen_ai_operation_name(Some("unknown")), None);
		assert_eq!(mcp_gen_ai_operation_name(None), None);
	}

	#[test]
	fn normalize_gen_ai_provider_maps_aliases_and_preserves_other_values() {
		assert_eq!(normalize_gen_ai_provider("BEDROCK"), "aws.bedrock");
		assert_eq!(normalize_gen_ai_provider("vertex"), "gcp.vertex_ai");
		assert_eq!(normalize_gen_ai_provider("Gemini"), "gcp.gemini");
		assert_eq!(normalize_gen_ai_provider("azureopenai"), "azure.ai.openai");
		assert_eq!(normalize_gen_ai_provider("azure_openai"), "azure.ai.openai");
		assert_eq!(normalize_gen_ai_provider("azure-openai"), "azure.ai.openai");
		assert_eq!(normalize_gen_ai_provider("openai"), "openai");
	}

	// Phase 5: attribute truncation tests
	#[test]
	fn truncate_attr_returns_unchanged_when_within_limit() {
		let s = "hello".to_string();
		assert_eq!(SpanWriter::truncate_attr(s.clone(), Some(10)), s);
	}

	#[test]
	fn truncate_attr_returns_unchanged_when_no_limit() {
		let s = "x".repeat(10_000);
		assert_eq!(SpanWriter::truncate_attr(s.clone(), None).len(), 10_000);
	}

	#[test]
	fn truncate_attr_truncates_at_utf8_boundary() {
		// "café" is 5 bytes (c=1, a=1, f=1, é=2). Limit of 4 splits "é" — must back up to "caf".
		let s = "café world";
		let result = SpanWriter::truncate_attr(s.to_string(), Some(4));
		// Must end with the ellipsis suffix and not include the split multibyte char
		assert!(result.starts_with("caf"), "got: {result}");
		assert!(result.ends_with("\u{2026}"), "got: {result}"); // \u{2026} = "…"
	}

	#[test]
	fn truncate_attr_appends_ellipsis_on_truncation() {
		let s = "hello world";
		let result = SpanWriter::truncate_attr(s.to_string(), Some(5));
		assert!(
			result.ends_with("\u{2026}"),
			"expected ellipsis, got: {result}"
		);
		assert!(result.starts_with("hello"), "got: {result}");
	}
}
