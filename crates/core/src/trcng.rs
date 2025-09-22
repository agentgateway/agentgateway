use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::baggage::BaggageExt;
use opentelemetry::global::{self, BoxedTracer};
use opentelemetry::propagation::TextMapCompositePropagator;
use opentelemetry::trace::{Span, SpanBuilder, Tracer as _};
use opentelemetry::{Context, KeyValue};
use opentelemetry_http::{HeaderExtractor, HeaderInjector};
use opentelemetry_otlp::{ExporterBuildError, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanProcessor};
use serde::{Deserialize, Serialize};
use tracing::info;

pub fn get_tracer() -> &'static BoxedTracer {
	static TRACER: OnceLock<BoxedTracer> = OnceLock::new();
	TRACER.get_or_init(|| global::tracer("agentgateway"))
}

pub trait Claim {
	fn get_claim(&self, key: &str) -> Option<&str>;
}

// start_span starts a span that takes into account custom attributes
pub fn start_span(span_name: impl Into<Cow<'static, str>>, context: &impl Claim) -> SpanBuilder {
	start_span_with_attributes(span_name, context, Default::default())
}
pub fn start_span_with_attributes(
	span_name: impl Into<Cow<'static, str>>,
	context: &impl Claim,
	mut attrs: Vec<KeyValue>,
) -> SpanBuilder {
	let mut base = get_tracer().span_builder(span_name);
	if let Some(tag_rules) = get_tag_rules() {
		for (k, v) in tag_rules {
			let v = if let Some((_, lookup)) = v.split_once("@") {
				context.get_claim(lookup).unwrap_or("unknown").to_string()
			} else {
				// Insert directly
				v
			};
			attrs.push(KeyValue::new(k, v))
		}
		base = base.with_attributes(attrs);
	};
	base
}

// Utility function to extract the context from the incoming request headers
pub fn extract_context_from_request(req: &http::HeaderMap) -> Context {
	global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(req)))
}

pub fn add_context_to_request(req: &mut http::HeaderMap, ctx: &Context) {
	global::get_text_map_propagator(|propagator| {
		propagator.inject_context(ctx, &mut HeaderInjector(req))
	});
	req.insert("baggage", "is_synthetic=true".parse().unwrap());
}

static TAG_RULES: OnceLock<HashMap<String, String>> = OnceLock::new();
fn get_tag_rules() -> Option<HashMap<String, String>> {
	TAG_RULES.get().cloned()
}
fn set_tag_rules(rules: HashMap<String, String>) {
	_ = TAG_RULES.get_or_init(|| rules)
}
fn get_resource() -> Resource {
	static RESOURCE: OnceLock<Resource> = OnceLock::new();
	RESOURCE
		.get_or_init(|| {
			Resource::builder()
				.with_service_name("agentgateway")
				.build()
		})
		.clone()
}

/// A custom span processor that enriches spans with baggage attributes. Baggage
/// information is not added automatically without this processor.
#[derive(Debug)]
struct EnrichWithBaggageSpanProcessor;
impl SpanProcessor for EnrichWithBaggageSpanProcessor {
	fn force_flush(&self) -> OTelSdkResult {
		Ok(())
	}

	fn shutdown(&self) -> OTelSdkResult {
		Ok(())
	}

	fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &Context) {
		for (kk, vv) in cx.baggage().iter() {
			span.set_attribute(KeyValue::new(kk.clone(), vv.0.clone()));
		}
	}

	fn on_end(&self, _span: opentelemetry_sdk::trace::SpanData) {}

	fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
		Ok(())
	}
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
	pub tracer: Tracer,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Tracer {
	#[serde(rename = "otlp")]
	Otlp { endpoint: Option<String> },
}

pub fn init_tracer(config: Config) -> Result<(), ExporterBuildError> {
	let Tracer::Otlp { endpoint } = config.tracer;
	let Some(endpoint) = endpoint else {
		return Ok(());
	};
	let baggage_propagator = BaggagePropagator::new();
	let trace_context_propagator = TraceContextPropagator::new();
	let composite_propagator = TextMapCompositePropagator::new(vec![
		Box::new(baggage_propagator),
		Box::new(trace_context_propagator),
	]);

	info!(endpoint, "initializing tracer");
	global::set_text_map_propagator(composite_propagator);
	let exporter = SpanExporter::builder()
		.with_tonic()
		.with_endpoint(endpoint)
		.build()?;

	let provider = SdkTracerProvider::builder()
		.with_span_processor(EnrichWithBaggageSpanProcessor)
		.with_resource(get_resource())
		.with_batch_exporter(exporter)
		.build();

	global::set_tracer_provider(provider.clone());
	// Usage of global is pretty bad here, but since we do it with provider it makes sense for this too.
	set_tag_rules(config.tags);
	Ok(())
}
