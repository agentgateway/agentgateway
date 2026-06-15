//! Distributed tracing integration for qURL resolution

use std::sync::Arc;
use std::time::Instant;

use opentelemetry::{global, KeyValue, trace::{Span, SpanKind, TraceContextExt, Tracer}};
use opentelemetry::trace::{FutureExt, TracedFuture};
use opentelemetry_sdk::trace as sdktrace;
use tracing::{debug, error, info, Instrument, Span as TracingSpan};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::qurl::{QurlError, ResolveRequest, ResolveResponse};

/// Tracing configuration
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// Service name for traces
    pub service_name: String,
    /// Enable tracing
    pub enabled: bool,
    /// Sample rate (0.0 - 1.0)
    pub sample_rate: f64,
    /// OTLP endpoint
    pub otlp_endpoint: Option<String>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            service_name: "agentgateway-qurl".to_string(),
            enabled: true,
            sample_rate: 1.0,
            otlp_endpoint: None,
        }
    }
}

/// Initialize OpenTelemetry tracing
pub fn init_tracing(config: TracingConfig) -> Result<opentelemetry_sdk::trace::TracerProvider, Box<dyn std::error::Error + Send + Sync>> {
    if !config.enabled {
        return Ok(opentelemetry_sdk::trace::TracerProvider::default());
    }

    let mut exporter_builder = opentelemetry_otlp::new_exporter()
        .tonic();

    if let Some(endpoint) = config.otlp_endpoint {
        exporter_builder = exporter_builder.with_endpoint(endpoint);
    }

    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter_builder.build_trace_exporter()?)
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_attributes([KeyValue::new("service.name", config.service_name)])
                .build()
        )
        .with_sampler(sdktrace::Sampler::TraceIdRatioBased(config.sample_rate))
        .build();

    global::set_tracer_provider(tracer_provider.clone());

    Ok(tracer_provider)
}

/// Create a tracer for qURL operations
pub fn qurl_tracer() -> opentelemetry_sdk::trace::Tracer {
    global::tracer("agentgateway::qurl")
}

/// Attributes for qURL spans
pub mod attributes {
    pub const QURL_TOKEN_ID: &str = "qurl.token_id";
    pub const QURL_RESOURCE_ID: &str = "qurl.resource_id";
    pub const QURL_RESOLVED_URL: &str = "qurl.resolved_url";
    pub const QURL_ACCESS_GRANT_EXPIRES_IN: &str = "qurl.access_grant_expires_in";
    pub const QURL_GRANTED_SRC_IP: &str = "qurl.granted_src_ip";
    pub const QURL_NHP_KNOCK_STATUS: &str = "qurl.nhp_knock_status";
    pub const QURL_NHP_KNOCK_DURATION_MS: &str = "qurl.nhp_knock_duration_ms";
    pub const QURL_REGION: &str = "qurl.region";
    pub const QURL_CACHE_HIT: &str = "qurl.cache_hit";
    pub const QURL_CACHE_TIER: &str = "qurl.cache_tier";
    pub const QURL_CIRCUIT_BREAKER_STATE: &str = "qurl.circuit_breaker_state";
}

/// Span builder for qURL resolution
pub struct QurlResolveSpan {
    span: opentelemetry_sdk::trace::Span,
    tracer: opentelemetry_sdk::trace::Tracer,
}

impl QurlResolveSpan {
    /// Start a new qURL resolution span
    pub fn start(token_id: Option<&str>, resource_id: Option<&str>, region: &str) -> Self {
        if !crate::qurl::tracing::is_tracing_enabled() {
            return Self::noop();
        }

        let tracer = qurl_tracer();
        let mut span = tracer
            .span_builder("qurl.resolve")
            .with_kind(SpanKind::Client)
            .with_attributes(vec![
                KeyValue::new(attributes::QURL_REGION, region.to_string()),
            ])
            .start(&tracer);

        if let Some(token_id) = token_id {
            span.add_attribute(KeyValue::new(attributes::QURL_TOKEN_ID, token_id.to_string()));
        }
        if let Some(resource_id) = resource_id {
            span.add_attribute(KeyValue::new(attributes::QURL_RESOURCE_ID, resource_id.to_string()));
        }

        Self { span, tracer }
    }

    /// Create a no-op span for when tracing is disabled
    fn noop() -> Self {
        // Use a no-op tracer
        let tracer = opentelemetry_sdk::trace::Tracer::noop();
        let span = tracer.span_builder("noop").start(&tracer);
        Self { span, tracer }
    }

    /// Mark resolution as successful
    pub fn set_success(&mut self, response: &ResolveResponse) {
        if let Some(grant) = &response.access_grant {
            self.span.add_attribute(KeyValue::new(
                attributes::QURL_RESOLVED_URL,
                response.target_url.clone(),
            ));
            self.span.add_attribute(KeyValue::new(
                attributes::QURL_ACCESS_GRANT_EXPIRES_IN,
                grant.expires_in as i64,
            ));
            if let Some(src_ip) = &grant.src_ip {
                self.span.add_attribute(KeyValue::new(
                    attributes::QURL_GRANTED_SRC_IP,
                    src_ip.to_string(),
                ));
            }
        }
        self.span.add_attribute(KeyValue::new("qurl.success", true));
    }

    /// Mark resolution as failed
    pub fn set_error(&mut self, error: &QurlError) {
        self.span.add_attribute(KeyValue::new("qurl.success", false));
        self.span.add_attribute(KeyValue::new("qurl.error", error.to_string()));
        self.span.record_error(opentelemetry::trace::Error::new(error.to_string()));
    }

    /// Record cache hit/miss
    pub fn set_cache_hit(&mut self, hit: bool, tier: &str) {
        self.span.add_attribute(KeyValue::new(attributes::QURL_CACHE_HIT, hit));
        self.span.add_attribute(KeyValue::new(attributes::QURL_CACHE_TIER, tier.to_string()));
    }

    /// Record NHP knock
    pub fn record_nhp_knock(&mut self, success: bool, duration_ms: u64) {
        self.span.add_attribute(KeyValue::new(
            attributes::QURL_NHP_KNOCK_STATUS,
            if success { "success" } else { "failed" }
        ));
        self.span.add_attribute(KeyValue::new(
            attributes::QURL_NHP_KNOCK_DURATION_MS,
            duration_ms as i64,
        ));
    }

    /// Record circuit breaker state
    pub fn record_circuit_breaker(&mut self, state: &str) {
        self.span.add_attribute(KeyValue::new(attributes::QURL_CIRCUIT_BREAKER_STATE, state));
    }

    /// End the span
    pub fn end(self) {
        self.span.end();
    }
}

/// Trace a qURL resolution operation
pub async fn trace_resolve<F, Fut>(
    token_id: Option<&str>,
    resource_id: Option<&str>,
    region: &str,
    operation: F,
) -> Result<ResolveResponse, QurlError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<ResolveResponse, QurlError>>,
{
    if !is_tracing_enabled() {
        return operation().await;
    }

    let tracer = qurl_tracer();
    let span = tracer
        .span_builder("qurl.resolve")
        .with_kind(SpanKind::Client)
        .with_attributes(vec![
            KeyValue::new(attributes::QURL_REGION, region.to_string()),
        ])
        .start(&tracer);

    if let Some(token_id) = token_id {
        span.add_attribute(KeyValue::new(attributes::QURL_TOKEN_ID, token_id.to_string()));
    }
    if let Some(resource_id) = resource_id {
        span.add_attribute(KeyValue::new(attributes::QURL_RESOURCE_ID, resource_id.to_string()));
    }

    let cx = opentelemetry::Context::current_with_span(span);
    let result = operation()
        .with_context(cx.clone())
        .await;

    // Get the span back from context to add attributes
    if let Some(span) = cx.span() {
        match &result {
            Ok(response) => {
                if let Some(grant) = &response.access_grant {
                    span.add_attribute(KeyValue::new(attributes::QURL_RESOLVED_URL, response.target_url.clone()));
                    span.add_attribute(KeyValue::new(attributes::QURL_ACCESS_GRANT_EXPIRES_IN, grant.expires_in as i64));
                    if let Some(src_ip) = &grant.src_ip {
                        span.add_attribute(KeyValue::new(attributes::QURL_GRANTED_SRC_IP, src_ip.to_string()));
                    }
                }
                span.add_attribute(KeyValue::new("qurl.success", true));
            }
            Err(error) => {
                span.add_attribute(KeyValue::new("qurl.success", false));
                span.add_attribute(KeyValue::new("qurl.error", error.to_string()));
                span.record_error(opentelemetry::trace::Error::new(error.to_string()));
            }
        }
        span.end();
    }

    result
}

/// Trace a cache operation
pub async fn trace_cache_operation<F, Fut, T>(
    operation_name: &str,
    key: &str,
    tier: &str,
    operation: F,
) -> Result<T, QurlError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, QurlError>>,
{
    if !is_tracing_enabled() {
        return operation().await;
    }

    let tracer = qurl_tracer();
    let span = tracer
        .span_builder(format!("qurl.cache.{}", operation_name))
        .with_kind(SpanKind::Internal)
        .with_attributes(vec![
            KeyValue::new("qurl.cache.key", key.to_string()),
            KeyValue::new(attributes::QURL_CACHE_TIER, tier.to_string()),
        ])
        .start(&tracer);

    let cx = opentelemetry::Context::current_with_span(span);
    let result = operation()
        .with_context(cx.clone())
        .await;

    if let Some(span) = cx.span() {
        let hit = result.is_ok();
        span.add_attribute(KeyValue::new(attributes::QURL_CACHE_HIT, hit));
        if let Err(ref e) = result {
            span.add_attribute(KeyValue::new("qurl.error", e.to_string()));
            span.record_error(opentelemetry::trace::Error::new(e.to_string()));
        }
        span.end();
    }

    result
}

/// Trace an NHP knock operation
pub async fn trace_nhp_knock<F, Fut, T>(
    region: &str,
    operation: F,
) -> Result<T, QurlError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, QurlError>>,
{
    if !is_tracing_enabled() {
        return operation().await;
    }

    let tracer = qurl_tracer();
    let span = tracer
        .span_builder("qurl.nhp_knock")
        .with_kind(SpanKind::Client)
        .with_attributes(vec![
            KeyValue::new(attributes::QURL_REGION, region.to_string()),
        ])
        .start(&tracer);

    let start = Instant::now();
    let cx = opentelemetry::Context::current_with_span(span);
    let result = operation()
        .with_context(cx.clone())
        .await;
    let duration_ms = start.elapsed().as_millis() as u64;

    if let Some(span) = cx.span() {
        let success = result.is_ok();
        span.add_attribute(KeyValue::new(attributes::QURL_NHP_KNOCK_STATUS, if success { "success" } else { "failed" }));
        span.add_attribute(KeyValue::new(attributes::QURL_NHP_KNOCK_DURATION_MS, duration_ms as i64));
        if let Err(ref e) = result {
            span.add_attribute(KeyValue::new("qurl.error", e.to_string()));
            span.record_error(opentelemetry::trace::Error::new(e.to_string()));
        }
        span.end();
    }

    result
}

/// Check if tracing is enabled (placeholder - replace with actual config check)
fn is_tracing_enabled() -> bool {
    // In real implementation, check global config
    true
}

/// Extract trace context from HTTP headers for downstream requests
pub fn inject_trace_context(headers: &mut reqwest::header::HeaderMap) {
    let cx = opentelemetry::Context::current();
    let propagator = opentelemetry::global::text_map_propagator();
    propagator.inject_context(&cx, &mut HeaderInjector(headers));
}

/// Header injector for trace context propagation
struct HeaderInjector<'a>(&'a mut reqwest::header::HeaderMap);

impl<'a> opentelemetry::propagation::Injector for HeaderInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            self.0.insert(name, value);
        }
    }
}

/// Extract trace context from incoming request headers
pub fn extract_trace_context(headers: &reqwest::header::HeaderMap) -> opentelemetry::Context {
    let propagator = opentelemetry::global::text_map_propagator();
    propagator.extract(&HeaderExtractor(headers))
}

/// Header extractor for trace context
struct HeaderExtractor<'a>(&'a reqwest::header::HeaderMap);

impl<'a> opentelemetry::propagation::Extractor for HeaderExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_config_default() {
        let config = TracingConfig::default();
        assert_eq!(config.service_name, "agentgateway-qurl");
        assert!(config.enabled);
        assert_eq!(config.sample_rate, 1.0);
    }

    #[test]
    fn test_header_injector() {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut injector = HeaderInjector(&mut headers);
        injector.set("traceparent", "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_string());
        assert!(headers.contains_key("traceparent"));
    }
}