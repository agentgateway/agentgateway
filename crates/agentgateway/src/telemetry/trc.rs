use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Sub;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use agent_core::telemetry::ValueBag;
use http::Version;
use itertools::Itertools;
use once_cell::sync::OnceCell;
use opentelemetry::trace::{
	Span, SpanContext, SpanKind, Status, TraceContextExt, TraceState, Tracer as _, TracerProvider,
};
use opentelemetry::{Context, InstrumentationScope, Key, KeyValue, TraceFlags};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
pub use traceparent::TraceParent;

use crate::cel;
use crate::telemetry::log::{CelLoggingExecutor, LoggingFields, RequestLog};
use crate::telemetry::operation_spans::request_root_span_name;
use crate::types::agent::{SimpleBackendReference, TracingConfig};

#[derive(Clone, Debug)]
pub struct Tracer {
	pub tracer: Arc<opentelemetry_sdk::trace::SdkTracer>,
	pub provider: SdkTracerProvider,
	pub fields: Arc<LoggingFields>,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Copy, Eq, PartialEq, Clone, Debug)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(crate::JsonSchema))]
pub enum Protocol {
	#[default]
	Grpc,
	Http,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct Config {
	pub endpoint: Option<String>,
	pub headers: HashMap<String, String>,
	pub protocol: Protocol,
	pub fields: LoggingFields,
	pub random_sampling: Option<Arc<cel::Expression>>,
	pub client_sampling: Option<Arc<cel::Expression>>,
	pub path: String,
}

mod semconv {
	use opentelemetry::Key;

	pub static GATEWAY_LEGACY_SPAN_NAME: Key = Key::from_static_str("gateway.legacy_span_name");
	pub static GATEWAY_SPAN_SCHEMA_VERSION: Key = Key::from_static_str("gateway.span_schema_version");
	pub static PROTOCOL_VERSION: Key = Key::from_static_str("network.protocol.version");
	pub static URL_SCHEME: Key = Key::from_static_str("url.scheme");
}

const OTEL_SCHEMA_URL: &str = "https://opentelemetry.io/schemas/1.40.0";

fn append_path(endpoint: &str, path: &str) -> String {
	let path = path.trim_start_matches('/');
	format!("{}/{}", endpoint.trim_end_matches('/'), path)
}

fn normalize_otel_path(path: &str) -> Cow<'_, str> {
	if path.starts_with('/') {
		Cow::Borrowed(path)
	} else {
		Cow::Owned(format!("/{path}"))
	}
}

fn join_base_and_cfg_path(base_path: &str, cfg_path: &str) -> String {
	let cfg_path = normalize_otel_path(cfg_path);
	if base_path.is_empty() || base_path == "/" {
		return cfg_path.into_owned();
	}

	let base_path = normalize_otel_path(base_path);
	let base_path = base_path.as_ref().trim_end_matches('/');
	if base_path.ends_with(cfg_path.as_ref()) {
		base_path.to_string()
	} else {
		append_path(base_path, cfg_path.as_ref())
	}
}

fn scheme_for_request(request: &RequestLog) -> Option<&'static str> {
	if matches!(request.backend_protocol, Some(cel::BackendProtocol::tcp)) {
		return None;
	}
	if request.version.is_some() || request.backend_protocol.is_some() {
		return Some(if request.tls_info.is_some() {
			"https"
		} else {
			"http"
		});
	}
	None
}

fn network_protocol_version_for_request(version: Option<Version>) -> Option<&'static str> {
	match version {
		Some(Version::HTTP_11) => Some("1.1"),
		Some(Version::HTTP_2) => Some("2"),
		_ => None,
	}
}

fn legacy_request_span_name(request: &RequestLog) -> Option<String> {
	let method = request.method.as_ref()?.as_str();
	let path = request.path_match.as_deref().or(request.path.as_deref())?;
	Some(format!("{method} {path}"))
}

fn tracer_scope(name: String) -> InstrumentationScope {
	InstrumentationScope::builder(name)
		.with_schema_url(OTEL_SCHEMA_URL)
		.build()
}

fn status_for_request(request: &RequestLog) -> Option<Status> {
	if request.error.is_some() {
		return Some(Status::error("request_error"));
	}
	let status = request.status?;
	if status.is_server_error() {
		let description = match status.as_u16() {
			500 => "http 500",
			502 => "http 502",
			503 => "http 503",
			504 => "http 504",
			_ => "http error",
		};
		return Some(Status::error(description));
	}
	None
}

impl Tracer {
	pub fn create_tracer_from_config_with_client(
		config: &TracingConfig,
		fields: Arc<LoggingFields>,
		policy_client: crate::proxy::httpproxy::PolicyClient,
	) -> anyhow::Result<Tracer> {
		// Important: this may be called from the dataplane runtime (policy lazy init),
		// but we want exporter tasks/spans to run on the admin runtime when available.
		let exporter_runtime = policy_client
			.inputs
			.cfg
			.admin_runtime_handle
			.clone()
			.unwrap_or_else(tokio::runtime::Handle::current);

		let defaults = GLOBAL_RESOURCE_DEFAULTS.get();
		let mut resource_builder =
			Resource::builder().with_schema_url(Vec::<KeyValue>::new(), OTEL_SCHEMA_URL);
		if let Some(d) = defaults {
			for kv in &d.attrs {
				resource_builder = resource_builder.with_attribute(kv.clone());
			}
		}
		resource_builder = resource_builder.with_attribute(KeyValue::new(
			"service.version",
			agent_core::version::BuildInfo::new().version,
		));
		let exec = cel::Executor::new_empty();
		let mut tracer_name: Option<String> = None;
		for (name, expr) in config.resources.iter() {
			let name: &str = name.as_ref();
			if let Ok(value) = exec.eval(expr.as_ref()) {
				use opentelemetry::Value;
				let otel_value = match value {
					cel::Value::String(s) => {
						if name == "service.name" && tracer_name.is_none() {
							tracer_name = Some(s.to_string());
						}
						Value::String(s.to_string().into())
					},
					cel::Value::Int(i) => Value::I64(i),
					cel::Value::UInt(u) => Value::I64(u as i64),
					cel::Value::Float(f) => Value::F64(f),
					cel::Value::Bool(b) => Value::Bool(b),
					_ => {
						let json_str = value
							.json()
							.ok()
							.and_then(|j| serde_json::to_string(&j).ok())
							.unwrap_or_else(|| format!("{:?}", value));
						Value::String(json_str.into())
					},
				};
				resource_builder =
					resource_builder.with_attribute(KeyValue::new(name.to_string(), otel_value));
			}
		}
		let tracer_name = tracer_name
			.or_else(|| defaults.and_then(|d| d.service_name.clone()))
			.unwrap_or_else(|| "agentgateway".to_string());
		resource_builder = resource_builder.with_service_name(tracer_name.clone());

		// Build once and reuse in the provider
		let resource = resource_builder.build();

		// Choose exporter based on per-policy protocol:
		// - gRPC when protocol is "grpc"
		// - otherwise HTTP (fall back to gRPC if no HTTP path is available)
		let provider = if config.protocol == crate::types::agent::TracingProtocol::Grpc {
			// Use gRPC exporter that routes via PolicyClient/GrpcReferenceChannel
			let exporter = PolicyGrpcSpanExporter::new(
				policy_client.inputs.clone(),
				Arc::new(config.provider_backend.clone()),
				exporter_runtime.clone(),
			);
			opentelemetry_sdk::trace::SdkTracerProvider::builder()
				.with_resource(resource.clone())
				.with_batch_exporter(exporter)
				.build()
		} else {
			// Use HTTP exporter via PolicyClient by default.
			// Resolve the OTLP/HTTP path from global defaults; if not set, use the per-policy path (default "/v1/traces").
			let endpoint_path = GLOBAL_RESOURCE_DEFAULTS
				.get()
				.and_then(|d| d.otlp_http_path.clone())
				.unwrap_or_else(|| {
					let p = config.path.clone();
					if p.starts_with('/') {
						p
					} else {
						format!("/{}", p)
					}
				});
			let http_client = PolicyOtelHttpClient {
				policy_client,
				backend_ref: config.provider_backend.clone(),
				runtime: exporter_runtime,
			};
			let exporter = opentelemetry_otlp::SpanExporter::builder()
				.with_http()
				.with_http_client(http_client)
				.with_endpoint(endpoint_path)
				.build()?;
			opentelemetry_sdk::trace::SdkTracerProvider::builder()
				.with_resource(resource.clone())
				.with_batch_exporter(exporter)
				.build()
		};
		let tracer = provider.tracer_with_scope(tracer_scope(tracer_name));
		Ok(Tracer {
			tracer: Arc::new(tracer),
			provider,
			fields,
		})
	}

	pub fn shutdown(&self) {
		let _ = self.provider.shutdown();
	}

	pub fn send<'v>(
		&self,
		request: &RequestLog,
		cel_exec: &CelLoggingExecutor,
		attrs: &[(&str, Option<ValueBag<'v>>)],
		extra_attrs: &[KeyValue],
		suppressed_keys: &[&str],
	) {
		let Some(out_span) = request.outgoing_span.as_ref() else {
			return;
		};
		if !out_span.is_sampled() {
			return;
		}
		let mut attributes = attrs
			.iter()
			.filter(|(k, _)| !self.fields.has(k))
			.filter(|(k, _)| !suppressed_keys.contains(k))
			.filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))
			.map(|(k, v)| KeyValue::new(Key::new(k.to_string()), to_otel(v)))
			.collect_vec();
		attributes.extend(extra_attrs.iter().cloned());
		let end = SystemTime::now();
		let elapsed = request.tcp_info.start.elapsed();

		let mut has_url_scheme = false;
		let mut has_protocol_version = false;
		let mut has_legacy_span_name = false;
		let mut has_span_schema_version = false;
		for kv in &attributes {
			let key = kv.key.as_str();
			if key == semconv::URL_SCHEME.as_str() {
				has_url_scheme = true;
			} else if key == semconv::PROTOCOL_VERSION.as_str() {
				has_protocol_version = true;
			} else if key == semconv::GATEWAY_LEGACY_SPAN_NAME.as_str() {
				has_legacy_span_name = true;
			} else if key == semconv::GATEWAY_SPAN_SCHEMA_VERSION.as_str() {
				has_span_schema_version = true;
			}
			if has_url_scheme && has_protocol_version && has_legacy_span_name && has_span_schema_version {
				break;
			}
		}
		if !has_url_scheme && let Some(scheme) = scheme_for_request(request) {
			attributes.push(KeyValue::new(semconv::URL_SCHEME.clone(), scheme));
		}
		if !has_protocol_version
			&& let Some(version) = network_protocol_version_for_request(request.version)
		{
			attributes.push(KeyValue::new(semconv::PROTOCOL_VERSION.clone(), version));
		}
		if !has_legacy_span_name && let Some(legacy_name) = legacy_request_span_name(request) {
			attributes.push(KeyValue::new(
				semconv::GATEWAY_LEGACY_SPAN_NAME.clone(),
				legacy_name,
			));
		}
		if !has_span_schema_version {
			attributes.push(KeyValue::new(
				semconv::GATEWAY_SPAN_SCHEMA_VERSION.clone(),
				2_i64,
			));
		}

		attributes.reserve(self.fields.add.len());

		// To avoid lifetime issues need to store the expression before we give it to ValueBag reference.
		// TODO: we could allow log() to take a list of borrows and then a list of OwnedValueBag
		let raws = cel_exec.eval(&self.fields.add);
		let mut span_name = None;
		for (k, v) in raws {
			if k == "span.name"
				&& let Some(serde_json::Value::String(s)) = v
			{
				span_name = Some(s);
			} else if let Some(eval) = v.as_ref().map(ValueBag::capture_serde1) {
				attributes.push(KeyValue::new(Key::new(k.to_string()), to_otel(&eval)));
			}
		}

		let span_name =
			span_name.unwrap_or_else(|| request_root_span_name(request.path_match.as_deref()));
		let mut sb = self
			.tracer
			.span_builder(span_name)
			.with_start_time(end.sub(elapsed))
			.with_end_time(SystemTime::now())
			.with_kind(SpanKind::Server)
			.with_attributes(attributes)
			.with_trace_id(out_span.trace_id.into())
			.with_span_id(out_span.span_id.into());
		if let Some(status) = status_for_request(request) {
			sb = sb.with_status(status);
		}

		if let Some(in_span) = &request.incoming_span {
			let parent_ctx = remote_parent_context(in_span);
			sb.start_with_context(self.tracer.as_ref(), &parent_ctx)
				.end()
		} else {
			sb.start(self.tracer.as_ref()).end()
		}
	}
}

pub(crate) fn remote_parent_context(parent: &TraceParent) -> Context {
	let sc = SpanContext::new(
		parent.trace_id.into(),
		parent.span_id.into(),
		TraceFlags::new(parent.flags),
		true,
		TraceState::default(),
	);
	Context::new().with_remote_span_context(sc)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashMap;
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};
	use std::sync::{Arc, Mutex};
	use std::time::{Duration, Instant};

	use frozen_collections::FzHashSet;
	use opentelemetry::trace::TraceContextExt;
	use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
		TraceService, TraceServiceServer,
	};
	use opentelemetry_proto::tonic::collector::trace::v1::{
		ExportTraceServiceRequest, ExportTraceServiceResponse,
	};
	use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
	use prometheus_client::registry::Registry;
	use tokio::sync::mpsc;
	use tonic::{Request, Response, Status};

	#[test]
	fn remote_parent_context_uses_traceparent_values() {
		let tp = TraceParent {
			version: 0,
			trace_id: 0x1234,
			span_id: 0xabcd,
			flags: 0x01,
		};
		let ctx = remote_parent_context(&tp);
		let span = ctx.span();
		let sc = span.span_context();
		assert_eq!(u128::from_be_bytes(sc.trace_id().to_bytes()), tp.trace_id);
		assert_eq!(u64::from_be_bytes(sc.span_id().to_bytes()), tp.span_id);
		assert!(sc.is_remote());
		assert!(sc.is_sampled());
	}

	#[test]
	fn traceparent_try_from_rejects_bad_segment_count_without_panicking() {
		let malformed = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-";
		assert!(TraceParent::try_from(malformed).is_err());
	}

	#[test]
	fn traceparent_try_from_accepts_valid_header() {
		let value = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
		let parsed = TraceParent::try_from(value).expect("valid traceparent");
		assert_eq!(parsed.version, 0);
		assert_eq!(parsed.trace_id, 0x4bf92f3577b34da6a3ce929d0e0e4736);
		assert_eq!(parsed.span_id, 0x00f067aa0ba902b7);
		assert_eq!(parsed.flags, 0x01);
	}

	#[test]
	fn append_path_trims_boundary_slashes() {
		let joined = append_path("http://collector:4318/", "/v1/traces");
		assert_eq!(joined, "http://collector:4318/v1/traces");
	}

	#[test]
	fn normalize_otel_path_enforces_leading_slash() {
		assert_eq!(normalize_otel_path("v1/traces"), "/v1/traces");
		assert_eq!(normalize_otel_path("/v1/traces"), "/v1/traces");
	}

	#[test]
	fn join_base_and_cfg_path_avoids_duplicate_suffix() {
		let joined = join_base_and_cfg_path("/otlp/v1/traces", "v1/traces");
		assert_eq!(joined, "/otlp/v1/traces");
	}

	#[test]
	fn join_base_and_cfg_path_joins_root_or_base_paths() {
		assert_eq!(join_base_and_cfg_path("/", "v1/traces"), "/v1/traces");
		assert_eq!(
			join_base_and_cfg_path("/collector", "/v1/traces"),
			"/collector/v1/traces"
		);
	}

	#[derive(Debug)]
	struct MockTraceService {
		tx: Mutex<mpsc::Sender<ExportTraceServiceRequest>>,
	}

	#[tonic::async_trait]
	impl TraceService for MockTraceService {
		async fn export(
			&self,
			request: Request<ExportTraceServiceRequest>,
		) -> Result<Response<ExportTraceServiceResponse>, Status> {
			self
				.tx
				.lock()
				.expect("trace export sender mutex poisoned")
				.try_send(request.into_inner())
				.map_err(|e| Status::internal(format!("failed to capture export request: {e}")))?;
			Ok(Response::new(ExportTraceServiceResponse {
				partial_success: None,
			}))
		}
	}

	async fn start_mock_trace_collector() -> (SocketAddr, mpsc::Receiver<ExportTraceServiceRequest>) {
		let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
			.await
			.expect("bind mock trace collector");
		let addr = listener.local_addr().expect("mock collector local addr");

		let (tx, rx) = mpsc::channel(4);
		let service = MockTraceService { tx: Mutex::new(tx) };
		tokio::spawn(async move {
			tonic::transport::Server::builder()
				.add_service(TraceServiceServer::new(service))
				.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
				.await
				.expect("mock trace collector server failed");
		});

		(addr, rx)
	}

	fn make_min_req_log() -> crate::telemetry::log::RequestLog {
		let log_cfg = crate::telemetry::log::Config {
			filter: None,
			fields: crate::telemetry::log::LoggingFields::default(),
			metric_fields: Arc::new(crate::telemetry::log::MetricFields::default()),
			excluded_metrics: FzHashSet::default(),
			level: "info".to_string(),
			format: crate::LoggingFormat::Text,
		};
		let tracing_cfg = Config {
			endpoint: None,
			headers: HashMap::new(),
			protocol: Protocol::Grpc,
			fields: crate::telemetry::log::LoggingFields::default(),
			random_sampling: None,
			client_sampling: None,
			path: "/v1/traces".to_string(),
		};
		let cel = crate::telemetry::log::CelLogging::new(log_cfg, tracing_cfg);
		let mut prom = Registry::default();
		let metrics = Arc::new(crate::telemetry::metrics::Metrics::new(
			&mut prom,
			FzHashSet::default(),
		));
		let start = Instant::now();
		let tcp_info = crate::transport::stream::TCPConnectionInfo {
			peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345),
			local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080),
			start,
			raw_peer_addr: None,
		};
		crate::telemetry::log::RequestLog::new(cel, metrics, start, tcp_info)
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn otlp_grpc_wire_export_emits_server_span_and_resource_attrs() {
		let (addr, mut rx) = start_mock_trace_collector().await;

		let exporter = opentelemetry_otlp::SpanExporter::builder()
			.with_tonic()
			.with_endpoint(format!("http://{addr}"))
			.build()
			.expect("otlp tonic exporter init");
		let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
			.with_resource(
				Resource::builder()
					.with_service_name("agentgateway")
					.with_schema_url(Vec::<KeyValue>::new(), OTEL_SCHEMA_URL)
					.with_attribute(KeyValue::new(
						"service.version",
						agent_core::version::BuildInfo::new().version,
					))
					.build(),
			)
			.with_batch_exporter(exporter)
			.build();
		let tracer = Tracer {
			tracer: Arc::new(provider.tracer_with_scope(tracer_scope("agentgateway".to_string()))),
			provider,
			fields: Arc::new(crate::telemetry::log::LoggingFields::default()),
		};

		let mut req_log = make_min_req_log();
		let outgoing = TraceParent {
			version: 0,
			trace_id: 0xabcddcbaabcddcbaabcddcbaabcddcba,
			span_id: 0x0102030405060708,
			flags: 0x01,
		};
		req_log.method = Some(http::Method::POST);
		req_log.path = Some("/v1/chat/completions".to_string());
		req_log.outgoing_span = Some(outgoing.clone());
		let cel_exec = req_log
			.cel
			.build(None, None, None, None, None, None)
			.expect("cel executor");

		tracer.send(&req_log, &cel_exec, &[], &[], &[]);
		tracer.shutdown();

		let export_req = tokio::time::timeout(Duration::from_secs(5), rx.recv())
			.await
			.expect("collector timeout")
			.expect("collector channel closed unexpectedly");

		let service_name = export_req
			.resource_spans
			.iter()
			.filter_map(|rs| rs.resource.as_ref())
			.flat_map(|r| r.attributes.iter())
			.find_map(|kv| {
				if kv.key != "service.name" {
					return None;
				}
				match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
					Some(AnyValue::StringValue(s)) if !s.is_empty() => Some(s.as_str()),
					_ => None,
				}
			});
		assert!(
			service_name.is_some(),
			"exported resource should include non-empty service.name"
		);

		let spans = export_req
			.resource_spans
			.iter()
			.flat_map(|rs| rs.scope_spans.iter())
			.flat_map(|ss| ss.spans.iter())
			.collect_vec();
		assert_eq!(spans.len(), 1, "expected exactly one exported span");

		let ingress = spans[0];
		assert_eq!(
			ingress.kind,
			opentelemetry_proto::tonic::trace::v1::span::SpanKind::Server as i32,
			"ingress span must be Server on the wire"
		);
		let wire_trace_id = u128::from_be_bytes(
			ingress
				.trace_id
				.as_slice()
				.try_into()
				.expect("trace id should be 16 bytes"),
		);
		let wire_span_id = u64::from_be_bytes(
			ingress
				.span_id
				.as_slice()
				.try_into()
				.expect("span id should be 8 bytes"),
		);
		assert_eq!(wire_trace_id, outgoing.trace_id);
		assert_eq!(wire_span_id, outgoing.span_id);
		let legacy_span_name = ingress.attributes.iter().find_map(|kv| {
			if kv.key != "gateway.legacy_span_name" {
				return None;
			}
			match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
				Some(AnyValue::StringValue(v)) => Some(v.as_str()),
				_ => None,
			}
		});
		assert_eq!(legacy_span_name, Some("POST /v1/chat/completions"));
		let span_schema_version = ingress.attributes.iter().find_map(|kv| {
			if kv.key != "gateway.span_schema_version" {
				return None;
			}
			match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
				Some(AnyValue::IntValue(v)) => Some(*v),
				_ => None,
			}
		});
		assert_eq!(span_schema_version, Some(2));
	}
}

/// Policy-aware OTLP gRPC exporter that routes via `GrpcReferenceChannel`, ensuring
/// backend policies are looked up and applied by `PolicyClient::call_reference`.
/// For now we implement SpanExporter ourslves for grpc until https://github.com/open-telemetry/opentelemetry-rust/issues/3147 is addressed.
#[derive(Clone)]
struct PolicyGrpcSpanExporter {
	tonic_client:
		opentelemetry_proto::tonic::collector::trace::v1::trace_service_client::TraceServiceClient<
			crate::http::ext_proc::GrpcReferenceChannel,
		>,
	is_shutdown: Arc<AtomicBool>,
	resource: Resource,
	runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for PolicyGrpcSpanExporter {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PolicyGrpcSpanExporter").finish()
	}
}

impl PolicyGrpcSpanExporter {
	fn new(
		inputs: Arc<crate::ProxyInputs>,
		target: Arc<SimpleBackendReference>,
		runtime: tokio::runtime::Handle,
	) -> Self {
		use crate::http::ext_proc::GrpcReferenceChannel;
		let channel = GrpcReferenceChannel {
			target,
			client: crate::proxy::httpproxy::PolicyClient { inputs },
			timeout: None,
			span_writer: None,
		};
		let tonic_client = opentelemetry_proto::tonic::collector::trace::v1::trace_service_client::TraceServiceClient::new(
			channel,
		);
		Self {
			tonic_client,
			is_shutdown: Arc::new(AtomicBool::new(false)),
			resource: Resource::builder().build(),
			runtime,
		}
	}
}

#[async_trait::async_trait]
impl opentelemetry_sdk::trace::SpanExporter for PolicyGrpcSpanExporter {
	fn export(
		&self,
		batch: Vec<opentelemetry_sdk::trace::SpanData>,
	) -> impl futures_util::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send {
		use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
		let is_shutdown = self.is_shutdown.clone();
		let mut client = self.tonic_client.clone();
		let resource = self.resource.clone();
		let handle = self.runtime.clone();
		async move {
			if is_shutdown.load(Ordering::Acquire) {
				return Err(OTelSdkError::AlreadyShutdown);
			}
			// Reuse OTLP transform to convert SDK spans to ResourceSpans
			let resource_spans = from_span_data(&resource, batch);
			let req = opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest {
				resource_spans,
			};
			// Ensure export runs on the application's Tokio runtime
			handle
				.spawn(async move { client.export(req).await })
				.await
				.map_err(|e| OTelSdkError::InternalFailure(e.to_string()))?
				.map(|_| ())
				.map_err(|e| OTelSdkError::InternalFailure(e.to_string())) as OTelSdkResult
		}
	}

	fn shutdown(&mut self) -> opentelemetry_sdk::error::OTelSdkResult {
		self.is_shutdown.store(true, Ordering::Release);
		Ok(())
	}

	fn set_resource(&mut self, res: &opentelemetry_sdk::Resource) {
		self.resource = res.clone();
	}
}

fn to_otel(v: &ValueBag) -> opentelemetry::Value {
	if let Some(b) = v.to_str() {
		opentelemetry::Value::String(b.to_string().into())
	} else if let Some(b) = v.to_i64() {
		opentelemetry::Value::I64(b)
	} else if let Some(b) = v.to_f64() {
		opentelemetry::Value::F64(b)
	} else if let Some(b) = v.to_bool() {
		opentelemetry::Value::Bool(b)
	} else {
		opentelemetry::Value::String(v.to_string().into())
	}
}

#[derive(Clone, Debug)]
struct PolicyOtelHttpClient {
	policy_client: crate::proxy::httpproxy::PolicyClient,
	backend_ref: SimpleBackendReference,
	runtime: tokio::runtime::Handle,
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for PolicyOtelHttpClient {
	async fn send_bytes(
		&self,
		request: http::Request<bytes::Bytes>,
	) -> Result<http::Response<bytes::Bytes>, Box<dyn std::error::Error + Send + Sync + 'static>> {
		let client = self.policy_client.clone();
		let backend_ref = self.backend_ref.clone();
		let handle = self.runtime.clone();

		let (mut head, body_bytes) = request.into_parts();
		let mut uri_parts = head.uri.into_parts();
		uri_parts.scheme = None;
		uri_parts.authority = None;
		head.uri = http::Uri::from_parts(uri_parts).map_err(Box::new)?;
		let req = crate::http::Request::from_parts(head, crate::http::Body::from(body_bytes));

		let resp = handle
			.spawn(async move {
				client
					.call_reference(req, &backend_ref)
					.await
					.map_err(Box::new)
			})
			.await
			.map_err(Box::new)??;

		use http_body_util::BodyExt as _;
		let (parts, body) = resp.into_parts();
		let collected = body.collect().await.map_err(Box::new)?;
		let bytes = collected.to_bytes();
		Ok(http::Response::from_parts(parts, bytes))
	}
}

#[derive(Clone, Debug)]
struct GlobalResourceDefaults {
	service_name: Option<String>,
	attrs: Vec<KeyValue>,
	// If set, the OTLP/HTTP path (e.g., "/v1/traces") derived from cfg.tracing.endpoint or per-policy TracingConfig.path
	otlp_http_path: Option<String>,
}

static GLOBAL_RESOURCE_DEFAULTS: OnceCell<GlobalResourceDefaults> = OnceCell::new();

/// Build a tonic ResourceSpans payload from SDK SpanData.
/// Unblock exports for our custom exporter until https://github.com/open-telemetry/opentelemetry-rust/issues/3147 is addressed.
fn from_span_data(
	resource: &opentelemetry_sdk::Resource,
	spans: Vec<opentelemetry_sdk::trace::SpanData>,
) -> Vec<opentelemetry_proto::tonic::trace::v1::ResourceSpans> {
	let opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema {
		attributes,
		schema_url,
	} = resource.into();
	// Group spans by their instrumentation scope
	let mut scope_map: HashMap<
		opentelemetry::InstrumentationScope,
		Vec<opentelemetry_sdk::trace::SpanData>,
	> = HashMap::new();
	for span in spans {
		scope_map
			.entry(span.instrumentation_scope.clone())
			.or_default()
			.push(span);
	}

	// Convert the grouped spans into ScopeSpans
	let scope_spans = scope_map
		.into_iter()
		.map(
			|(instrumentation, span_records)| opentelemetry_proto::tonic::trace::v1::ScopeSpans {
				scope: Some((&instrumentation, None).into()),
				schema_url: instrumentation
					.schema_url()
					.map(ToOwned::to_owned)
					.unwrap_or_default(),
				spans: span_records.into_iter().map(Into::into).collect(),
			},
		)
		.collect();
	let resource_schema_url = schema_url.unwrap_or_default();
	vec![opentelemetry_proto::tonic::trace::v1::ResourceSpans {
		resource: Some(opentelemetry_proto::tonic::resource::v1::Resource {
			attributes: attributes.0,
			dropped_attributes_count: 0,
			entity_refs: vec![],
		}),
		schema_url: resource_schema_url,
		scope_spans,
	}]
}

/// Initialize defaults using gateway name/namespace from config
pub fn set_resource_defaults_from_config(cfg: &crate::Config) {
	let pm = &cfg.proxy_metadata;
	let mut attrs: Vec<KeyValue> = Vec::new();
	let mut push_if_present = |k: &'static str, v: &str| {
		if !v.is_empty() {
			attrs.push(KeyValue::new(k, v.to_string()));
		}
	};

	push_if_present("k8s.pod.name", pm.pod_name.as_str());
	push_if_present("k8s.namespace.name", pm.pod_namespace.as_str());
	push_if_present("k8s.node.name", pm.node_name.as_str());
	// `INSTANCE_IP` defaults to "1.1.1.1" when unset, avoid exporting placeholder values.
	if !pm.instance_ip.is_empty() && pm.instance_ip != "1.1.1.1" {
		attrs.push(KeyValue::new("k8s.pod.ip", pm.instance_ip.clone()));
	}
	// `node_id` is derived from pod name/namespace, only set if we have those set
	if !pm.node_id.is_empty() && !pm.pod_name.is_empty() && !pm.pod_namespace.is_empty() {
		attrs.push(KeyValue::new("service.instance.id", pm.node_id.clone()));
	}
	if let Some(host) = cfg.self_addr.as_deref()
		&& !host.is_empty()
	{
		attrs.push(KeyValue::new("host.name", host.to_string()));
	}
	// Use gateway name/namespace as authoritative service identity
	let service_name = cfg.xds.gateway.to_string();
	let service_namespace = cfg.xds.namespace.to_string();
	attrs.push(KeyValue::new("service.namespace", service_namespace));

	// Derive OTLP/HTTP path from cfg.tracing.endpoint if provided and protocol is HTTP.
	// We only need the path component; the actual authority is resolved via backend policies.
	let mut otlp_http_path: Option<String> = None;
	if let Some(ep) = cfg.tracing.endpoint.as_deref()
		&& cfg.tracing.protocol == Protocol::Http
	{
		// Try to parse as a URI to extract the path component
		if let Ok(uri) = http::Uri::try_from(ep) {
			let path = join_base_and_cfg_path(uri.path(), cfg.tracing.path.as_str());
			otlp_http_path = Some(path);
		} else {
			// Fallback to default if parsing fails
			otlp_http_path = Some("/v1/traces".to_string());
		}
	}

	if GLOBAL_RESOURCE_DEFAULTS
		.set(GlobalResourceDefaults {
			service_name: Some(service_name),
			attrs,
			otlp_http_path,
		})
		.is_err()
	{
		tracing::warn!("set_resource_defaults_from_config called more than once; ignoring");
	}
}

mod traceparent {
	use std::fmt;

	use rand::RngExt;

	use crate::http::Request;

	/// Represents a traceparent, as defined by https://www.w3.org/TR/trace-context/
	#[derive(Clone, Eq, PartialEq)]
	pub struct TraceParent {
		pub version: u8,
		pub trace_id: u128,
		pub span_id: u64,
		pub flags: u8,
	}

	pub const TRACEPARENT_HEADER: &str = "traceparent";

	impl Default for TraceParent {
		fn default() -> Self {
			Self::new()
		}
	}

	impl TraceParent {
		pub fn new() -> Self {
			let mut rng = rand::rng();
			Self {
				version: 0,
				trace_id: rng.random(),
				span_id: rng.random(),
				flags: 0,
			}
		}
		pub fn insert_header(&self, req: &mut Request) {
			if let Ok(hv) = hyper::header::HeaderValue::from_str(&self.to_string()) {
				req.headers_mut().insert(TRACEPARENT_HEADER, hv);
			}
		}
		pub fn from_request(req: &Request) -> Option<Self> {
			req
				.headers()
				.get(TRACEPARENT_HEADER)
				.and_then(|b| b.to_str().ok())
				.and_then(|b| TraceParent::try_from(b).ok())
		}
		pub fn new_span(&self) -> Self {
			let mut rng = rand::rng();
			let mut cpy: TraceParent = self.clone();
			cpy.span_id = rng.random();
			cpy
		}
		pub fn trace_id(&self) -> String {
			format!("{:032x}", self.trace_id)
		}
		pub fn span_id(&self) -> String {
			format!("{:016x}", self.span_id)
		}
		pub fn is_sampled(&self) -> bool {
			(self.flags & 0x01) == 0x01
		}
	}

	impl fmt::Debug for TraceParent {
		fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
			fmt::Display::fmt(self, f)
		}
	}

	impl fmt::Display for TraceParent {
		fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
			write!(
				f,
				"{:02x}-{:032x}-{:016x}-{:02x}",
				self.version, self.trace_id, self.span_id, self.flags
			)
		}
	}

	impl TryFrom<&str> for TraceParent {
		type Error = anyhow::Error;

		fn try_from(value: &str) -> Result<Self, Self::Error> {
			if value.len() != 55 {
				anyhow::bail!("traceparent malformed length was {}", value.len())
			}

			let mut segs = value.split('-');
			let (Some(version), Some(trace_id), Some(span_id), Some(flags), None) = (
				segs.next(),
				segs.next(),
				segs.next(),
				segs.next(),
				segs.next(),
			) else {
				anyhow::bail!("traceparent malformed segment count");
			};

			Ok(Self {
				version: u8::from_str_radix(version, 16)?,
				trace_id: u128::from_str_radix(trace_id, 16)?,
				span_id: u64::from_str_radix(span_id, 16)?,
				flags: u8::from_str_radix(flags, 16)?,
			})
		}
	}
}
