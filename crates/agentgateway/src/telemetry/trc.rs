use std::collections::HashMap;
use std::ops::Sub;
use std::sync::Arc;
use std::time::SystemTime;

use agent_core::telemetry::ValueBag;
use http::Version;
use itertools::Itertools;
use opentelemetry::trace::{Span, SpanContext, SpanKind, TraceState, Tracer as _, TracerProvider};
use opentelemetry::{Key, KeyValue, TraceFlags};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
pub use traceparent::TraceParent;

use crate::cel;
use crate::telemetry::log::{CelLoggingExecutor, LoggingFields, RequestLog};

#[derive(Clone, Debug)]
pub struct Tracer {
	pub tracer: Arc<opentelemetry_sdk::trace::SdkTracer>,
	pub provider: SdkTracerProvider,
	pub fields: LoggingFields,
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
		let result = opentelemetry_sdk::trace::SdkTracerProvider::builder()
			.with_resource(
				Resource::builder()
					.with_service_name("agentgateway")
					.with_attribute(KeyValue::new(
						"service.version",
						agent_core::version::BuildInfo::new().version,
					))
					.build(),
			)
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
			fields: cfg.fields.clone(),
		}))
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
