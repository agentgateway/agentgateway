use std::collections::HashMap;
use std::ops::Sub;
use std::sync::Arc;
use std::time::SystemTime;

use agent_core::telemetry::ValueBag;
use http::Version;
use itertools::Itertools;
use once_cell::sync::OnceCell;
use opentelemetry::trace::{Span, SpanContext, SpanKind, TraceState, Tracer as _, TracerProvider};
use opentelemetry::{Key, KeyValue, TraceFlags};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
pub use traceparent::TraceParent;

use crate::cel;
use crate::telemetry::log::{CelLoggingExecutor, LoggingFields, RequestLog};
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
}

mod semconv {
	use opentelemetry::Key;

	pub static PROTOCOL_VERSION: Key = Key::from_static_str("network.protocol.version");
	pub static URL_SCHEME: Key = Key::from_static_str("url.scheme");
}

impl Tracer {
	pub fn new(cfg: &Config) -> anyhow::Result<Option<Tracer>> {
		let Some(ep) = &cfg.endpoint else {
			return Ok(None);
		};
		// Apply global defaults (gateway-derived if initialized)
		let defaults = GLOBAL_RESOURCE_DEFAULTS.get();
		let result = opentelemetry_sdk::trace::SdkTracerProvider::builder()
			.with_resource({
				let mut rb = Resource::builder()
					.with_service_name(
						defaults
							.and_then(|d| d.service_name.clone())
							.unwrap_or_else(|| "agentgateway".to_string()),
					)
					.with_attribute(KeyValue::new(
						"service.version",
						agent_core::version::BuildInfo::new().version,
					));
				if let Some(d) = defaults {
					for kv in &d.attrs {
						rb = rb.with_attribute(kv.clone());
					}
				}
				rb.build()
			})
			// TODO: this should be integrated with PolicyClient
			.with_batch_exporter(if cfg.protocol == Protocol::Grpc {
				// TODO: otel is using an old tonic version that mismatches with the one we have
				// let metadata = MetadataMap::from_headers(HeaderMap::from_iter(
				// 	cfg
				// 		.headers
				// 		.clone()
				// 		.into_iter()
				// 		.map(|(k, v)| Ok((HeaderName::try_from(k)?, HeaderValue::try_from(v)?)))
				// 		.collect::<Result<_, _>>()?
				// 		.iter(),
				// ));
				opentelemetry_otlp::SpanExporter::builder()
					.with_tonic()
					.with_endpoint(ep)
					// .with_metadata(metadata)
					.build()?
			} else {
				opentelemetry_otlp::SpanExporter::builder()
					.with_http()
					// For HTTP, we add the suffix ourselves
					.with_endpoint(format!("{}/v1/traces", ep.strip_suffix("/").unwrap_or(ep)))
					.with_headers(cfg.headers.clone())
					.build()?
			})
			.build();
		let tracer = result.tracer("agentgateway");
		Ok(Some(Tracer {
			tracer: Arc::new(tracer),
			provider: result,
			fields: Arc::new(cfg.fields.clone()),
		}))
	}

	/// Create a tracer from dynamic TracingConfig (policy-driven)
	/// This is used for per-listener tracing configurations
	pub fn create_tracer_from_config(
		config: &TracingConfig,
		fields: Arc<LoggingFields>,
	) -> anyhow::Result<Tracer> {
		let defaults = GLOBAL_RESOURCE_DEFAULTS.get();
		// Extract endpoint from backend reference
		let endpoint = match &config.provider_backend {
			SimpleBackendReference::Service { name, port } => {
				// Construct endpoint from service reference
				let scheme = if config.insecure { "http" } else { "https" };
				format!("{}://{}:{}", scheme, name.hostname, port)
			},
			SimpleBackendReference::InlineBackend(target) => {
				let scheme = if config.insecure { "http" } else { "https" };
				format!("{}://{}", scheme, target)
			},
			SimpleBackendReference::Backend(backend_name) => {
				// For backend names, we'll need to resolve them
				backend_name.to_string()
			},
			SimpleBackendReference::Invalid => {
				anyhow::bail!("Invalid backend reference for tracing provider");
			},
		};

		// Build the tracer provider with resources from config
		// Evaluate resource CEL expressions (note: resources should typically be static)
		let mut resource_builder = Resource::builder();
		if let Some(d) = defaults {
			for kv in &d.attrs {
				resource_builder = resource_builder.with_attribute(kv.clone());
			}
		}

		// Add default service version
		resource_builder = resource_builder.with_attribute(KeyValue::new(
			"service.version",
			agent_core::version::BuildInfo::new().version,
		));

		// Add resources from config
		// Note: Resources in OpenTelemetry are static service descriptors
		// Evaluate CEL expressions with empty context for static resource values
		let executor = cel::ContextBuilder::new().build()?;
		// Prefer tracer name from service.name resource if provided
		let mut tracer_name: Option<String> = None;
		for resource_attr in &config.resources {
			// Evaluate the CEL expression to get the static resource value
			if let Ok(value) = executor.eval(&resource_attr.value) {
				use opentelemetry::Value;
				let otel_value = match value {
					cel::Value::String(s) => {
						if resource_attr.name == "service.name" && tracer_name.is_none() {
							tracer_name = Some(s.to_string());
						}
						Value::String(s.to_string().into())
					},
					cel::Value::Int(i) => Value::I64(i),
					cel::Value::UInt(u) => Value::I64(u as i64),
					cel::Value::Float(f) => Value::F64(f),
					cel::Value::Bool(b) => Value::Bool(b),
					_ => Value::String(format!("{:?}", value).into()),
				};
				resource_builder =
					resource_builder.with_attribute(KeyValue::new(resource_attr.name.clone(), otel_value));
			}
		}

		// If no explicit service.name provided, fall back to defaults from proxy metadata
		let tracer_name = tracer_name
			.or_else(|| defaults.and_then(|d| d.service_name.clone()))
			.unwrap_or_else(|| "agentgateway".to_string());
		resource_builder = resource_builder.with_service_name(tracer_name.clone());

		let result = opentelemetry_sdk::trace::SdkTracerProvider::builder()
			.with_resource(resource_builder.build())
			.with_batch_exporter({
				// Default to gRPC for now. Note: Using a backend name here will not work without PolicyClient.
				opentelemetry_otlp::SpanExporter::builder()
					.with_tonic()
					.with_endpoint(&endpoint)
					.build()?
			})
			.build();

		let tracer = result.tracer(tracer_name);

		Ok(Tracer {
			tracer: Arc::new(tracer),
			provider: result,
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
	) {
		let mut attributes = attrs
			.iter()
			.filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))
			.map(|(k, v)| KeyValue::new(Key::new(k.to_string()), to_otel(v)))
			.collect_vec();
		let out_span = request.outgoing_span.as_ref().unwrap();
		if !out_span.is_sampled() {
			return;
		}
		let end = SystemTime::now();
		let elapsed = request.tcp_info.start.elapsed();

		// For now we only accept HTTP(?)
		attributes.push(KeyValue::new(semconv::URL_SCHEME.clone(), "http"));
		// Otel spec has a special format here
		match &request.version {
			Some(Version::HTTP_11) => {
				attributes.push(KeyValue::new(semconv::PROTOCOL_VERSION.clone(), "1.1"));
			},
			Some(Version::HTTP_2) => {
				attributes.push(KeyValue::new(semconv::PROTOCOL_VERSION.clone(), "2"));
			},
			_ => {},
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

		let span_name = span_name.unwrap_or_else(|| match (&request.method, &request.path_match) {
			(Some(method), Some(path_match)) => {
				format!("{method} {path_match}")
			},
			_ => "unknown".to_string(),
		});

		let out_span = request.outgoing_span.as_ref().unwrap();
		let mut sb = self
			.tracer
			.span_builder(span_name)
			.with_start_time(end.sub(elapsed))
			.with_end_time(SystemTime::now())
			.with_kind(SpanKind::Server)
			.with_attributes(attributes)
			.with_trace_id(out_span.trace_id.into())
			.with_span_id(out_span.span_id.into());

		if let Some(in_span) = &request.incoming_span {
			let parent = SpanContext::new(
				in_span.trace_id.into(),
				in_span.span_id.into(),
				TraceFlags::new(in_span.flags),
				true,
				TraceState::default(),
			);
			sb = sb.with_links(vec![opentelemetry::trace::Link::new(
				parent.clone(),
				vec![],
				0,
			)]);
		}
		sb.start(self.tracer.as_ref()).end()
	}
}

fn to_otel(v: &ValueBag) -> opentelemetry::Value {
	if let Some(b) = v.to_str() {
		opentelemetry::Value::String(b.to_string().into())
	} else if let Some(b) = v.to_i64() {
		opentelemetry::Value::I64(b)
	} else if let Some(b) = v.to_f64() {
		opentelemetry::Value::F64(b)
	} else {
		opentelemetry::Value::String(v.to_string().into())
	}
}

#[derive(Clone)]
struct PolicyOtelHttpClient {
	policy_client: crate::proxy::httpproxy::PolicyClient,
	backend_ref: SimpleBackendReference,
}

impl std::fmt::Debug for PolicyOtelHttpClient {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PolicyOtelHttpClient").finish()
	}
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for PolicyOtelHttpClient {
	async fn send_bytes(
		&self,
		request: http::Request<bytes::Bytes>,
	) -> Result<http::Response<bytes::Bytes>, Box<dyn std::error::Error + Send + Sync + 'static>> {
		let client = self.policy_client.clone();
		let backend_ref = self.backend_ref.clone();

		let (mut head, body_bytes) = request.into_parts();
		let mut uri_parts = head.uri.into_parts();
		uri_parts.scheme = None;
		uri_parts.authority = None;
		head.uri = http::Uri::from_parts(uri_parts)
			.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
		let req = crate::http::Request::from_parts(head, crate::http::Body::from(body_bytes));

		let resp = client
			.call_reference(req, &backend_ref)
			.await
			.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

		use http_body_util::BodyExt as _;
		let (parts, body) = resp.into_parts();
		let collected = body
			.collect()
			.await
			.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
		let bytes = collected.to_bytes();
		Ok(http::Response::from_parts(parts, bytes))
	}
}

impl Tracer {
	pub fn create_tracer_from_config_with_client(
		config: &TracingConfig,
		fields: Arc<LoggingFields>,
		policy_client: crate::proxy::httpproxy::PolicyClient,
	) -> anyhow::Result<Tracer> {
		let defaults = GLOBAL_RESOURCE_DEFAULTS.get();
		let mut resource_builder = Resource::builder();
		if let Some(d) = defaults {
			for kv in &d.attrs {
				resource_builder = resource_builder.with_attribute(kv.clone());
			}
		}
		resource_builder = resource_builder.with_attribute(KeyValue::new(
			"service.version",
			agent_core::version::BuildInfo::new().version,
		));
		let executor = cel::ContextBuilder::new().build()?;
		let mut tracer_name: Option<String> = None;
		for resource_attr in &config.resources {
			if let Ok(value) = executor.eval(&resource_attr.value) {
				use opentelemetry::Value;
				let otel_value = match value {
					cel::Value::String(s) => {
						if resource_attr.name == "service.name" && tracer_name.is_none() {
							tracer_name = Some(s.to_string());
						}
						Value::String(s.to_string().into())
					},
					cel::Value::Int(i) => Value::I64(i),
					cel::Value::UInt(u) => Value::I64(u as i64),
					cel::Value::Float(f) => Value::F64(f),
					cel::Value::Bool(b) => Value::Bool(b),
					_ => Value::String(format!("{:?}", value).into()),
				};
				resource_builder =
					resource_builder.with_attribute(KeyValue::new(resource_attr.name.clone(), otel_value));
			}
		}
		let tracer_name = tracer_name
			.or_else(|| defaults.and_then(|d| d.service_name.clone()))
			.unwrap_or_else(|| "agentgateway".to_string());
		resource_builder = resource_builder.with_service_name(tracer_name.clone());

		let http_client = PolicyOtelHttpClient {
			policy_client,
			backend_ref: config.provider_backend.clone(),
		};
		let endpoint_path = "http://agentgateway.invalid/v1/traces";
		let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
			.with_resource(resource_builder.build())
			.with_batch_exporter(
				opentelemetry_otlp::SpanExporter::builder()
					.with_http()
					.with_http_client(http_client)
					.with_endpoint(endpoint_path)
					.build()?,
			)
			.build();
		let tracer = provider.tracer(tracer_name);
		Ok(Tracer {
			tracer: Arc::new(tracer),
			provider,
			fields,
		})
	}
}

#[derive(Clone, Debug)]
struct GlobalResourceDefaults {
	service_name: Option<String>,
	attrs: Vec<KeyValue>,
}

static GLOBAL_RESOURCE_DEFAULTS: OnceCell<GlobalResourceDefaults> = OnceCell::new();

/// Initialize defaults using gateway name/namespace from config
pub fn set_resource_defaults_from_config(cfg: &crate::Config) {
	let pm = &cfg.proxy_metadata;
	let mut attrs: Vec<KeyValue> = vec![
		KeyValue::new("k8s.pod.name", pm.pod_name.clone()),
		KeyValue::new("k8s.namespace.name", pm.pod_namespace.clone()),
		KeyValue::new("k8s.node.name", pm.node_name.clone()),
		KeyValue::new("k8s.pod.ip", pm.instance_ip.clone()),
		KeyValue::new("service.instance.id", pm.node_id.clone()),
	];
	if let Some(host) = cfg.self_addr.as_deref()
		&& !host.is_empty()
	{
		attrs.push(KeyValue::new("host.name", host.to_string()));
	}
	// Use gateway name/namespace as authoritative service identity
	let service_name = cfg.xds.gateway.to_string();
	let service_namespace = cfg.xds.namespace.to_string();
	attrs.push(KeyValue::new("service.namespace", service_namespace));

	let _ = GLOBAL_RESOURCE_DEFAULTS.set(GlobalResourceDefaults {
		service_name: Some(service_name),
		attrs,
	});
}

mod traceparent {
	use std::fmt;

	use rand::Rng;

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
			let hv = hyper::header::HeaderValue::from_bytes(format!("{self:?}").as_bytes()).unwrap();
			req.headers_mut().insert(TRACEPARENT_HEADER, hv);
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
			write!(
				f,
				"{:02x}-{:032x}-{:016x}-{:02x}",
				self.version, self.trace_id, self.span_id, self.flags
			)
		}
	}

	impl fmt::Display for TraceParent {
		fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
			write!(f, "{:032x}", self.trace_id,)
		}
	}

	impl TryFrom<&str> for TraceParent {
		type Error = anyhow::Error;

		fn try_from(value: &str) -> Result<Self, Self::Error> {
			if value.len() != 55 {
				anyhow::bail!("traceparent malformed length was {}", value.len())
			}

			let segs: Vec<&str> = value.split('-').collect();

			Ok(Self {
				version: u8::from_str_radix(segs[0], 16)?,
				trace_id: u128::from_str_radix(segs[1], 16)?,
				span_id: u64::from_str_radix(segs[2], 16)?,
				flags: u8::from_str_radix(segs[3], 16)?,
			})
		}
	}
}
