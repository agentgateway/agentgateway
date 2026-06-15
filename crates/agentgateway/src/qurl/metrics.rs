//! Prometheus metrics for qURL integration

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, Ordering};

use lazy_static::lazy_static;
use prometheus::{
    Counter, CounterVec, Gauge, GaugeVec, Histogram, HistogramVec, HistogramOpts, Opts,
    Registry, register_counter, register_counter_vec, register_gauge, register_gauge_vec,
    register_histogram, register_histogram_vec,
};
use tracing::{debug, warn};

lazy_static! {
    /// Global metrics registry
    static ref METRICS_REGISTRY: Registry = Registry::new();
}

/// Initialize metrics (call once at startup)
pub fn init_metrics() -> Result<(), prometheus::Error> {
    // Register all metrics
    register_qurl_resolve_total()?;
    register_qurl_resolve_duration()?;
    register_qurl_resolve_errors()?;
    register_qurl_cache_hits()?;
    register_qurl_cache_misses()?;
    register_qurl_cache_size()?;
    register_qurl_cache_evictions()?;
    register_qurl_circuit_breaker_state()?;
    register_qurl_nhp_knock_total()?;
    register_qurl_nhp_knock_duration()?;
    register_qurl_active_requests()?;
    register_qurl_region_health()?;
    register_qurl_region_latency()?;
    Ok(())
}

// ===== qURL Resolution Metrics =====

lazy_static! {
    /// Total number of qURL resolve attempts
    static ref QURL_RESOLVE_TOTAL: CounterVec = register_counter_vec!(
        "qurl_resolve_total",
        "Total number of qURL token resolution attempts",
        &["status", "region"]
    ).unwrap();

    /// Duration of qURL resolve calls
    static ref QURL_RESOLVE_DURATION: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "qurl_resolve_duration_seconds",
            "Duration of qURL token resolution in seconds"
        ).buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["region", "status"]
    ).unwrap();

    /// Errors during qURL resolution
    static ref QURL_RESOLVE_ERRORS: CounterVec = register_counter_vec!(
        "qurl_resolve_errors_total",
        "Total number of qURL resolution errors by type",
        &["error_type", "region"]
    ).unwrap();
}

/// Record a qURL resolution attempt
pub fn record_qurl_resolve(region: &str, success: bool, duration: Duration) {
    let status = if success { "success" } else { "error" };
    QURL_RESOLVE_TOTAL.with_label_values(&[status, region]).inc();
    QURL_RESOLVE_DURATION.with_label_values(&[region, status]).observe(duration.as_secs_f64());
}

/// Record a qURL resolution error
pub fn record_qurl_resolve_error(region: &str, error_type: &str) {
    QURL_RESOLVE_ERRORS.with_label_values(&[error_type, region]).inc();
}

// ===== Cache Metrics =====

lazy_static! {
    /// Cache hits
    static ref QURL_CACHE_HITS: CounterVec = register_counter_vec!(
        "qurl_cache_hits_total",
        "Total number of cache hits",
        &["tier"]
    ).unwrap();

    /// Cache misses
    static ref QURL_CACHE_MISSES: CounterVec = register_counter_vec!(
        "qurl_cache_misses_total",
        "Total number of cache misses",
        &["tier"]
    ).unwrap();

    /// Current cache size
    static ref QURL_CACHE_SIZE: GaugeVec = register_gauge_vec!(
        "qurl_cache_size",
        "Current number of entries in cache",
        &["tier"]
    ).unwrap();

    /// Cache evictions
    static ref QURL_CACHE_EVICTIONS: CounterVec = register_counter_vec!(
        "qurl_cache_evictions_total",
        "Total number of cache evictions",
        &["tier", "reason"]
    ).unwrap();
}

/// Record cache hit
pub fn record_cache_hit(tier: &str) {
    QURL_CACHE_HITS.with_label_values(&[tier]).inc();
}

/// Record cache miss
pub fn record_cache_miss(tier: &str) {
    QURL_CACHE_MISSES.with_label_values(&[tier]).inc();
}

/// Update cache size
pub fn update_cache_size(tier: &str, size: usize) {
    QURL_CACHE_SIZE.with_label_values(&[tier]).set(size as f64);
}

/// Record cache eviction
pub fn record_cache_eviction(tier: &str, reason: &str) {
    QURL_CACHE_EVICTIONS.with_label_values(&[tier, reason]).inc();
}

// ===== Circuit Breaker Metrics =====

lazy_static! {
    /// Circuit breaker state (0=closed, 1=half-open, 2=open)
    static ref QURL_CIRCUIT_BREAKER_STATE: GaugeVec = register_gauge_vec!(
        "qurl_circuit_breaker_state",
        "Circuit breaker state (0=closed, 1=half_open, 2=open)",
        &["name"]
    ).unwrap();

    /// Circuit breaker failure count
    static ref QURL_CIRCUIT_BREAKER_FAILURES: CounterVec = register_counter_vec!(
        "qurl_circuit_breaker_failures_total",
        "Total failures recorded by circuit breaker",
        &["name"]
    ).unwrap();

    /// Circuit breaker success count
    static ref QURL_CIRCUIT_BREAKER_SUCCESSES: CounterVec = register_counter_vec!(
        "qurl_circuit_breaker_successes_total",
        "Total successes recorded by circuit breaker",
        &["name"]
    ).unwrap();
}

/// Update circuit breaker state metric
pub fn update_circuit_breaker_state(name: &str, state: crate::qurl::CircuitState) {
    let state_value = match state {
        crate::qurl::CircuitState::Closed => 0.0,
        crate::qurl::CircuitState::HalfOpen => 1.0,
        crate::qurl::CircuitState::Open => 2.0,
    };
    QURL_CIRCUIT_BREAKER_STATE.with_label_values(&[name]).set(state_value);
}

/// Record circuit breaker failure
pub fn record_circuit_breaker_failure(name: &str) {
    QURL_CIRCUIT_BREAKER_FAILURES.with_label_values(&[name]).inc();
}

/// Record circuit breaker success
pub fn record_circuit_breaker_success(name: &str) {
    QURL_CIRCUIT_BREAKER_SUCCESSES.with_label_values(&[name]).inc();
}

// ===== NHP Knock Metrics =====

lazy_static! {
    /// NHP knock attempts
    static ref QURL_NHP_KNOCK_TOTAL: CounterVec = register_counter_vec!(
        "qurl_nhp_knock_total",
        "Total number of NHP knock attempts",
        &["status", "region"]
    ).unwrap();

    /// NHP knock duration
    static ref QURL_NHP_KNOCK_DURATION: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "qurl_nhp_knock_duration_seconds",
            "Duration of NHP knock in seconds"
        ).buckets(vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["status", "region"]
    ).unwrap();
}

/// Record NHP knock attempt
pub fn record_nhp_knock(region: &str, success: bool, duration: Duration) {
    let status = if success { "success" } else { "failed" };
    QURL_NHP_KNOCK_TOTAL.with_label_values(&[status, region]).inc();
    QURL_NHP_KNOCK_DURATION.with_label_values(&[status, region]).observe(duration.as_secs_f64());
}

/// Record NHP knock timeout
pub fn record_nhp_knock_timeout(region: &str, duration: Duration) {
    QURL_NHP_KNOCK_TOTAL.with_label_values(&["timeout", region]).inc();
    QURL_NHP_KNOCK_DURATION.with_label_values(&["timeout", region]).observe(duration.as_secs_f64());
}

// ===== Active Requests Metrics =====

lazy_static! {
    /// Currently active qURL resolution requests
    static ref QURL_ACTIVE_REQUESTS: GaugeVec = register_gauge_vec!(
        "qurl_active_requests",
        "Number of currently active qURL resolution requests",
        &["region"]
    ).unwrap();
}

/// Increment active requests
pub fn increment_active_requests(region: &str) {
    QURL_ACTIVE_REQUESTS.with_label_values(&[region]).inc();
}

/// Decrement active requests
pub fn decrement_active_requests(region: &str) {
    QURL_ACTIVE_REQUESTS.with_label_values(&[region]).dec();
}

// ===== Region Health Metrics =====

lazy_static! {
    /// Region health status (0=healthy, 1=degraded, 2=unhealthy, 3=unknown)
    static ref QURL_REGION_HEALTH: GaugeVec = register_gauge_vec!(
        "qurl_region_health",
        "Region health status (0=healthy, 1=degraded, 2=unhealthy, 3=unknown)",
        &["region_id"]
    ).unwrap();

    /// Region latency
    static ref QURL_REGION_LATENCY: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "qurl_region_latency_seconds",
            "Region request latency in seconds"
        ).buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
        &["region_id"]
    ).unwrap();

    /// Region consecutive failures
    static ref QURL_REGION_FAILURES: GaugeVec = register_gauge_vec!(
        "qurl_region_consecutive_failures",
        "Number of consecutive failures for region",
        &["region_id"]
    ).unwrap();
}

/// Update region health metric
pub fn update_region_health(region_id: &str, health: crate::qurl::RegionHealth) {
    let health_value = match health {
        crate::qurl::RegionHealth::Healthy => 0.0,
        crate::qurl::RegionHealth::Degraded => 1.0,
        crate::qurl::RegionHealth::Unhealthy => 2.0,
        crate::qurl::RegionHealth::Unknown => 3.0,
    };
    QURL_REGION_HEALTH.with_label_values(&[region_id]).set(health_value);
}

/// Record region latency
pub fn record_region_latency(region_id: &str, duration: Duration) {
    QURL_REGION_LATENCY.with_label_values(&[region_id]).observe(duration.as_secs_f64());
}

/// Update region consecutive failures
pub fn update_region_failures(region_id: &str, failures: u32) {
    QURL_REGION_FAILURES.with_label_values(&[region_id]).set(failures as f64);
}

/// Metrics handler for exposing /metrics endpoint
pub struct MetricsHandler {
    registry: Registry,
}

impl MetricsHandler {
    pub fn new() -> Self {
        Self {
            registry: METRICS_REGISTRY.clone(),
        }
    }

    /// Gather all metrics as Prometheus text format
    pub fn gather(&self) -> Result<String, prometheus::Error> {
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        let encoder = prometheus::TextEncoder::new();
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer).unwrap())
    }
}

impl Default for MetricsHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Create metrics router for axum
pub fn metrics_router() -> axum::Router {
    let handler = Arc::new(MetricsHandler::new());
    axum::Router::new()
        .route("/metrics", get(metrics_endpoint))
        .with_state(handler)
}

async fn metrics_endpoint(
    axum::extract::State(handler): axum::extract::State<Arc<MetricsHandler>>,
) -> impl axum::response::IntoResponse {
    match handler.gather() {
        Ok(metrics) => (
            axum::http::StatusCode::OK,
            [("Content-Type", "text/plain; version=0.0.4")],
            metrics,
        ),
        Err(e) => {
            warn!("Failed to gather metrics: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                [("Content-Type", "text/plain")],
                format!("Failed to gather metrics: {}", e),
            )
        }
    }
}

/// Middleware to track active requests
pub struct ActiveRequestTracker {
    region: String,
}

impl ActiveRequestTracker {
    pub fn new(region: String) -> Self {
        increment_active_requests(&region);
        Self { region }
    }
}

impl Drop for ActiveRequestTracker {
    fn drop(&mut self) {
        decrement_active_requests(&self.region);
    }
}

/// Helper for timing operations
pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn start() -> Self {
        Self { start: Instant::now() }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn elapsed_secs_f64(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qurl::RegionHealth;

    #[test]
    fn test_metrics_initialization() {
        init_metrics().unwrap();
    }

    #[test]
    fn test_record_resolve() {
        record_qurl_resolve("primary", true, Duration::from_millis(100));
        record_qurl_resolve("primary", false, Duration::from_millis(200));
        record_qurl_resolve_error("primary", "timeout");
    }

    #[test]
    fn test_cache_metrics() {
        record_cache_hit("l1");
        record_cache_miss("l1");
        record_cache_hit("l2");
        update_cache_size("l1", 100);
        record_cache_eviction("l1", "lru");
    }

    #[test]
    fn test_circuit_breaker_metrics() {
        update_circuit_breaker_state("qurl-api", crate::qurl::CircuitState::Closed);
        update_circuit_breaker_state("qurl-api", crate::qurl::CircuitState::Open);
        record_circuit_breaker_failure("qurl-api");
        record_circuit_breaker_success("qurl-api");
    }

    #[test]
    fn test_nhp_knock_metrics() {
        record_nhp_knock("primary", true, Duration::from_millis(50));
        record_nhp_knock("primary", false, Duration::from_millis(5000));
        record_nhp_knock_timeout("primary", Duration::from_secs(10));
    }

    #[test]
    fn test_region_metrics() {
        update_region_health("primary", RegionHealth::Healthy);
        update_region_health("secondary", RegionHealth::Degraded);
        record_region_latency("primary", Duration::from_millis(100));
        update_region_failures("primary", 5);
    }

    #[test]
    fn test_active_request_tracker() {
        let _tracker = ActiveRequestTracker::new("primary".to_string());
        // On drop, counter decrements
    }

    #[test]
    fn test_timer() {
        let timer = Timer::start();
        std::thread::sleep(Duration::from_millis(10));
        assert!(timer.elapsed() >= Duration::from_millis(10));
    }
}