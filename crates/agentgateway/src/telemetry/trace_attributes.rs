//! Discovers the attributes on the request span by running synthetic requests through the real
//! logging path and recording what the tracer exports. Used by `cargo xtask tracing` to generate
//! `schema/tracing.md`, so the list cannot drift from what the proxy actually emits.

use std::collections::BTreeMap;
use std::future::ready;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_core::{Timestamp, strng};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SimpleSpanProcessor, SpanData, SpanExporter};
use prometheus_client::registry::Registry;

use super::log::{CelLogging, DropOnLog, LoggingFields, MetricFields, RequestLog};
use super::metrics::Metrics;
use super::trc;
use crate::http::substrate::ateattr::{ResumeDisposition, RouteOutcome};
use crate::llm::catalog::ModelCatalog;
use crate::mcp::MCPInfo;
use crate::proxy::ProxyResponseReason;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::transport::tls::{IstioIdentity, TlsInfo};
use crate::types::agent::{ListenerName, RouteName, Target};
use crate::{a2a, cel, llm};

/// The kind of request that sets an attribute. Attributes are grouped by the first kind that
/// emits them, in declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RequestKind {
	Common,
	Llm,
	Mcp,
	A2a,
	Substrate,
}

impl RequestKind {
	pub fn title(self) -> &'static str {
		match self {
			RequestKind::Common => "Common",
			RequestKind::Llm => "LLM",
			RequestKind::Mcp => "MCP",
			RequestKind::A2a => "A2A",
			RequestKind::Substrate => "Substrate",
		}
	}
}

#[derive(Debug, Clone)]
pub struct SpanAttribute {
	pub name: String,
	pub kind: RequestKind,
	/// Set when the attribute is only emitted under a condition the synthetic requests do not
	/// exercise.
	pub condition: Option<&'static str>,
}

/// Attributes that the synthetic requests cannot reach, with the condition that enables them.
const CONDITIONAL: &[(&str, RequestKind, &str)] = &[
	(
		"connection.id",
		RequestKind::Common,
		"Debug logging is enabled for the `request` target.",
	),
	(
		"request.id",
		RequestKind::Common,
		"Debug logging is enabled for the `request` target.",
	),
];

#[derive(Clone, Debug, Default)]
struct RecordingSpanExporter {
	spans: Arc<Mutex<Vec<SpanData>>>,
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

fn tracer() -> (Arc<trc::Tracer>, RecordingSpanExporter) {
	let exporter = RecordingSpanExporter::default();
	let processor = trc::SharedSpanProcessor::new(SimpleSpanProcessor::new(exporter.clone()));
	let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
		.with_span_processor(processor.clone())
		.build();
	(
		Arc::new(trc::Tracer {
			provider: super::NonBlockingDrop::new(provider),
			processor,
			fields: Arc::new(LoggingFields::default()),
			filter: None,
		}),
		exporter,
	)
}

fn base_log(catalog: Arc<ModelCatalog>) -> RequestLog {
	let cel = CelLogging {
		cel_context: cel::ContextBuilder::new(),
		filter: None,
		fields: LoggingFields::default(),
		otlp_filter: None,
		otlp_fields: LoggingFields::default(),
		metric_fields: MetricFields::default(),
		standard_attributes: LoggingFields::default(),
		database_fields: LoggingFields::default(),
	};
	let mut registry = Registry::default();
	let metrics = Arc::new(Metrics::new(
		&mut registry,
		Default::default(),
		Default::default(),
	));
	let mut log = RequestLog::new(
		cel,
		metrics,
		catalog,
		Timestamp::now(),
		TCPConnectionInfo {
			peer_addr: "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
			local_addr: "127.0.0.1:8080".parse::<SocketAddr>().unwrap(),
			start: Instant::now(),
			raw_peer_addr: None,
		},
	);
	log.listener_name = Some(ListenerName::default());
	log.route_name = Some(RouteName {
		name: strng::literal!("route"),
		namespace: strng::literal!("default"),
		rule_name: Some(strng::literal!("rule")),
		kind: None,
	});
	log.endpoint = Some(Target::Hostname(strng::literal!("backend.example"), 8080));
	log.status = Some(crate::http::StatusCode::OK);
	log
}

fn full_http(log: &mut RequestLog) {
	log.host = Some("example.com:8443".to_string());
	log.server_port = Some(8443);
	log.scheme = Some(::http::uri::Scheme::HTTPS);
	log.method = Some(::http::Method::POST);
	log.path = Some("/path?query=1".to_string());
	log.version = Some(::http::Version::HTTP_11);
	log.status = Some(crate::http::StatusCode::INTERNAL_SERVER_ERROR);
	log.reason = Some(ProxyResponseReason::DirectResponse);
	log.error = Some("error".to_string());
	log.jwt_sub = Some("subject".to_string());
	log.retry_attempt = Some(1);
	log.inference_pool = Some(Target::Hostname(strng::literal!("pool.example"), 8000));
	log.backend_protocol = Some(cel::BackendProtocol::http);
	log.tls_info = Some(TLSConnectionInfo {
		src_identity: Some(TlsInfo {
			identity: Some(IstioIdentity::new(
				strng::literal!("cluster.local"),
				strng::literal!("ns"),
				strng::literal!("sa"),
			)),
			..Default::default()
		}),
		server_name: Some("example.com".to_string()),
		negotiated_alpn: None,
	});
	log.grpc_status.store(Some(2));
}

/// `tls.sni` is only set when the request has no HTTP host.
fn tls_only(log: &mut RequestLog) {
	log.host = None;
	log.tls_info = Some(TLSConnectionInfo {
		src_identity: None,
		server_name: Some("example.com".to_string()),
		negotiated_alpn: None,
	});
}

fn llm_request() -> llm::LLMRequest {
	llm::LLMRequest {
		input_tokens: Some(10),
		input_format: llm::InputFormat::Completions,
		cache_convention: llm::CacheTokenConvention::InputIncludesCache,
		request_model: strng::literal!("model"),
		original_model: None,
		provider: strng::literal!("openai"),
		streaming: true,
		params: llm::LLMRequestParams {
			temperature: Some(0.5),
			top_p: Some(0.5),
			frequency_penalty: Some(0.5),
			presence_penalty: Some(0.5),
			seed: Some(1),
			max_tokens: Some(100),
			encoding_format: Some(strng::literal!("float")),
			dimensions: Some(8),
			..Default::default()
		},
		prompt: None,
		provider_state: None,
	}
}

fn llm_response() -> llm::LLMResponse {
	let now = Instant::now();
	llm::LLMResponse {
		input_tokens: Some(10),
		input_audio_tokens: Some(1),
		output_tokens: Some(10),
		output_image_tokens: Some(1),
		output_audio_tokens: Some(1),
		audio_seconds: Some(1.0),
		reasoning_tokens: Some(1),
		cache_creation_input_tokens: Some(1),
		cached_input_tokens: Some(1),
		pages: Some(1),
		provider_model: Some(strng::literal!("model")),
		response_id: Some(strng::literal!("id")),
		finish_reasons: Some(vec![strng::literal!("stop")]),
		first_token: Some(now),
		last_token_at: Some(now),
		..Default::default()
	}
}

fn llm(log: &mut RequestLog) {
	let request = llm_request();
	log.backend_protocol = Some(cel::BackendProtocol::llm);
	log.llm_request = Some(request.clone());
	log.original_model = Some("alias".to_string());
	log
		.llm_response
		.store(Some(llm::LLMInfo::new(request, llm_response())));
	log.guardrails.store(Some(vec![cel::GuardrailInfo {
		phase: strng::literal!("request"),
		guard: strng::literal!("regex"),
		action: Default::default(),
		detail: Default::default(),
	}]));
}

fn mcp(log: &mut RequestLog, set: impl FnOnce(&mut MCPInfo)) {
	log.backend_protocol = Some(cel::BackendProtocol::mcp);
	let mut mcp = MCPInfo {
		method_name: Some(strng::literal!("tools/call")),
		session_id: Some("session".to_string()),
		..Default::default()
	};
	set(&mut mcp);
	mcp.capture_error(&rmcp::ErrorData::invalid_params("error", None));
	log.mcp_status.store(Some(mcp));
}

fn a2a(log: &mut RequestLog) {
	log.backend_protocol = Some(cel::BackendProtocol::a2a);
	log.a2a_method = Some(strng::literal!("message/send"));
	log.a2a_response = Some(a2a::ResponseInfo {
		outcome: a2a::ResponseOutcome::Error,
		error_code: Some(1),
		result_kind: Some(strng::literal!("task")),
		task_state: Some(strng::literal!("completed")),
		context_id: Some(strng::literal!("context")),
	});
}

fn substrate(log: &mut RequestLog) {
	log.ate_actor_name = Some("actor".to_string());
	log.ate_actor_uid = Some("uid".to_string());
	log.ate_atespace = Some("space".to_string());
	log.ate_router_resume = Some(ResumeDisposition::Triggered);
	log.ate_router_route_duration = Some(Duration::from_millis(1));
	log.ate_router_outcome = Some(RouteOutcome::Ok);
}

/// A catalog with prices for the synthetic LLM model, so cost attributes are emitted.
async fn priced_catalog() -> anyhow::Result<Arc<ModelCatalog>> {
	let dir = std::env::temp_dir().join(format!("agw-trace-attributes-{}", std::process::id()));
	fs_err::create_dir_all(&dir)?;
	let file = dir.join("catalog.json");
	fs_err::write(
		&file,
		r#"{"providers":{"openai":{"models":{"model":{"rates":{"input":"1","output":"2","cacheRead":"1","cacheWrite":"1","reasoning":"1","inputAudio":"1","outputAudio":"1","perPage":"1","perAudioMinute":"1"}}}}}}"#,
	)?;
	let catalog = ModelCatalog::new(vec![crate::ModelCatalogSource::File { file }]).await?;
	let info = llm::LLMInfo::new(llm_request(), llm_response());
	for _ in 0..40 {
		if catalog.project(&info).cost.is_some() {
			return Ok(catalog);
		}
		tokio::time::sleep(Duration::from_millis(25)).await;
	}
	anyhow::bail!("model catalog did not load")
}

type Scenario = (RequestKind, Box<dyn Fn(&mut RequestLog)>);

fn export(log: RequestLog) -> Vec<String> {
	let (tracer, exporter) = tracer();
	let mut log = log;
	log.tracer = Some(tracer.clone());
	let mut outgoing = trc::TraceParent::new();
	outgoing.flags = 1;
	log.outgoing_span = Some(outgoing);
	drop(DropOnLog::from(log));
	let _ = tracer.provider.force_flush();
	let spans = exporter.spans.lock().unwrap();
	spans
		.iter()
		.flat_map(|span| span.attributes.iter().map(|kv| kv.key.as_str().to_string()))
		.collect()
}

/// Returns every attribute the request span can carry, sorted by kind and then name.
pub async fn span_attributes() -> anyhow::Result<Vec<SpanAttribute>> {
	let catalog = priced_catalog().await?;
	let mut found: BTreeMap<String, RequestKind> = BTreeMap::new();
	let mut record = |kind: RequestKind, names: Vec<String>| {
		for name in names {
			found.entry(name).or_insert(kind);
		}
	};

	let scenarios: Vec<Scenario> = vec![
		(RequestKind::Common, Box::new(full_http)),
		(RequestKind::Common, Box::new(tls_only)),
		(RequestKind::Llm, Box::new(llm)),
		(
			RequestKind::Mcp,
			Box::new(|log| mcp(log, |m| m.set_tool("target".into(), "tool".into()))),
		),
		(
			RequestKind::Mcp,
			Box::new(|log| mcp(log, |m| m.set_prompt("target".into(), "prompt".into()))),
		),
		(
			RequestKind::Mcp,
			Box::new(|log| mcp(log, |m| m.set_resource("target".into(), "resource".into()))),
		),
		(RequestKind::A2a, Box::new(a2a)),
		(RequestKind::Substrate, Box::new(substrate)),
	];
	for (kind, setup) in &scenarios {
		let mut log = base_log(catalog.clone());
		setup(&mut log);
		record(*kind, export(log));
	}

	// Standard attributes are CEL expressions that only appear when they return a value, which
	// depends on how the caller authenticated. List them from the default configuration.
	let standard = crate::config::standard_attributes(None)?;
	for (name, _) in standard.add.iter() {
		found.entry(name.to_string()).or_insert(RequestKind::Common);
	}

	let mut out: Vec<SpanAttribute> = found
		.into_iter()
		.map(|(name, kind)| SpanAttribute {
			name,
			kind,
			condition: None,
		})
		.collect();
	for (name, kind, condition) in CONDITIONAL {
		if !out.iter().any(|a| a.name == *name) {
			out.push(SpanAttribute {
				name: name.to_string(),
				kind: *kind,
				condition: Some(condition),
			});
		}
	}
	out.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
	Ok(out)
}
