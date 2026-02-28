use std::borrow::Cow;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::{Duration, Instant};

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
	GenAILabels, GenAILabelsTokenUsage, HTTPLabels, MCPCall, Metrics, RouteIdentifier,
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
			let gen_ai_labels = Arc::new(GenAILabels {
				gen_ai_operation_name: strng::literal!("chat").into(),
				gen_ai_system: llm_response.provider.clone().into(),
				gen_ai_request_model: llm_response.request_model.clone().into(),
				gen_ai_response_model: llm_response.response_model.clone().into(),
				custom: custom_metric_fields.clone(),
				route: route_identifier.clone(),
			});
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
		// TODO!
		// log
		// 	.cel
		// 	.cel_context
		// 	.with_request_completion(agent_core::telemetry::render_current_time());
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
			let resource = ResourceId::new(m.target_name.as_deref()?, m.resource_name.as_deref()?);
			match m.resource {
				Some(MCPOperation::Prompt) => Some(ResourceType::Prompt(resource)),
				Some(MCPOperation::Tool) => Some(ResourceType::Tool(resource)),
				Some(MCPOperation::Resource) => Some(ResourceType::Resource(resource)),
				Some(MCPOperation::Task) => Some(ResourceType::Task(resource)),
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
		}

		let enable_logs = maybe_enable_log && cel_exec.eval_filter();
		if !enable_logs && !enable_trace {
			return;
		}

		let dur = format!("{}ms", duration.as_millis());
		let grpc = log.grpc_status.load();

		let input_tokens = llm_response.as_ref().and_then(|l| l.input_tokens);

		let trace_id = log.outgoing_span.as_ref().map(|id| id.trace_id());
		let span_id = log.outgoing_span.as_ref().map(|id| id.span_id());

		let fields = cel_exec.fields;
		let reason = log.reason.and_then(|r| match r {
			ProxyResponseReason::Upstream => None,
			_ => Some(r),
		});

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
				"mcp.method",
				mcp
					.as_ref()
					.and_then(|m| m.method_name.as_ref())
					.map(display),
			),
			(
				"mcp.target",
				mcp
					.as_ref()
					.and_then(|m| m.target_name.as_ref())
					.map(display),
			),
			(
				"mcp.resource.type",
				mcp.as_ref().and_then(|m| m.resource.as_ref()).map(display),
			),
			(
				"mcp.resource.name",
				mcp
					.as_ref()
					.and_then(|m| m.resource_name.as_ref())
					.map(display),
			),
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
			// OpenTelemetry Gen AI Semantic Conventions v1.37.0
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
				"gen_ai.usage.output_tokens",
				llm_response
					.as_ref()
					.and_then(|l| l.output_tokens)
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
		if enable_trace && let Some(t) = &log.tracer {
			t.send(&log, &cel_exec, kv.as_slice())
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
