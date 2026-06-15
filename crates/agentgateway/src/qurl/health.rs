//! Health check endpoints and monitoring for qURL integration

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;

use axum::{Router, Json, response::IntoResponse, routing::get};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::qurl::{MultiRegionQurlClient, QurlResolutionCache, CircuitBreaker};

/// Health check response for qURL integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QurlHealthResponse {
    pub status: HealthStatus,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub version: String,
    pub regions: Vec<RegionHealthInfo>,
    pub cache: CacheHealthInfo,
    pub circuit_breakers: Vec<CircuitBreakerHealthInfo>,
}

/// Overall health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Per-region health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionHealthInfo {
    pub region_id: String,
    pub api_url: String,
    pub status: HealthStatus,
    pub last_check: chrono::DateTime<chrono::Utc>,
    pub latency_ms: Option<u64>,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
}

/// Cache health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheHealthInfo {
    pub l1_size: usize,
    pub l1_capacity: usize,
    pub l1_hit_rate: f64,
    pub l2_available: bool,
    pub l2_status: Option<String>,
}

/// Circuit breaker health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerHealthInfo {
    pub name: String,
    pub state: String,
    pub failure_count: usize,
    pub success_count: usize,
    pub request_count: usize,
}

/// Health check component trait
pub trait HealthCheckable: Send + Sync {
    fn name(&self) -> &str;
    fn check_health(&self) -> ComponentHealth;
}

/// Component health result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub details: HashMap<String, serde_json::Value>,
}

/// qURL health check handler
pub struct QurlHealthHandler {
    multi_region_client: Option<Arc<MultiRegionQurlClient>>,
    cache: Option<Arc<QurlResolutionCache>>,
    circuit_breakers: HashMap<String, Arc<CircuitBreaker>>,
    version: String,
}

impl QurlHealthHandler {
    /// Create new health handler
    pub fn new(version: String) -> Self {
        Self {
            multi_region_client: None,
            cache: None,
            circuit_breakers: HashMap::new(),
            version,
        }
    }

    /// Set multi-region client
    pub fn with_multi_region_client(mut self, client: Arc<MultiRegionQurlClient>) -> Self {
        self.multi_region_client = Some(client);
        self
    }

    /// Set cache
    pub fn with_cache(mut self, cache: Arc<QurlResolutionCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Add circuit breaker
    pub fn add_circuit_breaker(mut self, name: String, cb: Arc<CircuitBreaker>) -> Self {
        self.circuit_breakers.insert(name, cb);
        self
    }

    /// Perform comprehensive health check
    pub async fn check_health(&self) -> QurlHealthResponse {
        let mut regions = Vec::new();
        let mut overall_status = HealthStatus::Healthy;

        // Check multi-region client
        if let Some(client) = &self.multi_region_client {
            for region_status in client.region_statuses() {
                let region_health = match region_status.health {
                    crate::qurl::RegionHealth::Healthy => HealthStatus::Healthy,
                    crate::qurl::RegionHealth::Degraded => HealthStatus::Degraded,
                    crate::qurl::RegionHealth::Unhealthy => HealthStatus::Unhealthy,
                    crate::qurl::RegionHealth::Unknown => HealthStatus::Degraded,
                };

                if region_health == HealthStatus::Unhealthy {
                    overall_status = HealthStatus::Unhealthy;
                } else if region_health == HealthStatus::Degraded && overall_status == HealthStatus::Healthy {
                    overall_status = HealthStatus::Degraded;
                }

                regions.push(RegionHealthInfo {
                    region_id: region_status.region_id,
                    api_url: "".to_string(), // Would need to store this
                    status: region_health,
                    last_check: region_status.last_check,
                    latency_ms: region_status.latency_ms,
                    consecutive_failures: region_status.consecutive_failures,
                    consecutive_successes: region_status.consecutive_successes,
                });
            }
        }

        // Check cache
        let cache_info = if let Some(cache) = &self.cache {
            let stats = cache.stats();
            let hit_rate = if stats.hits + stats.misses > 0 {
                stats.hits as f64 / (stats.hits + stats.misses) as f64
            } else {
                0.0
            };

            CacheHealthInfo {
                l1_size: stats.size,
                l1_capacity: stats.capacity,
                l1_hit_rate: hit_rate,
                l2_available: false, // Would need to check if L2 is configured
                l2_status: None,
            }
        } else {
            CacheHealthInfo {
                l1_size: 0,
                l1_capacity: 0,
                l1_hit_rate: 0.0,
                l2_available: false,
                l2_status: None,
            }
        };

        // Check circuit breakers
        let mut circuit_breakers = Vec::new();
        for (name, cb) in &self.circuit_breakers {
            let stats = cb.stats();
            let cb_status = match stats.state {
                crate::qurl::CircuitState::Closed => HealthStatus::Healthy,
                crate::qurl::CircuitState::HalfOpen => HealthStatus::Degraded,
                crate::qurl::CircuitState::Open => HealthStatus::Unhealthy,
            };

            if cb_status == HealthStatus::Unhealthy {
                overall_status = HealthStatus::Unhealthy;
            } else if cb_status == HealthStatus::Degraded && overall_status == HealthStatus::Healthy {
                overall_status = HealthStatus::Degraded;
            }

            circuit_breakers.push(CircuitBreakerHealthInfo {
                name: name.clone(),
                state: format!("{:?}", stats.state),
                failure_count: stats.failure_count,
                success_count: stats.success_count,
                request_count: stats.request_count,
            });
        }

        QurlHealthResponse {
            status: overall_status,
            timestamp: chrono::Utc::now(),
            version: self.version.clone(),
            regions,
            cache: cache_info,
            circuit_breakers,
        }
    }
}

/// Create health check router
pub fn health_router(handler: Arc<QurlHealthHandler>) -> Router {
    Router::new()
        .route("/health", get(health_endpoint))
        .route("/health/live", get(liveness_endpoint))
        .route("/health/ready", get(readiness_endpoint))
        .with_state(handler)
}

/// Main health endpoint
async fn health_endpoint(
    axum::extract::State(handler): axum::extract::State<Arc<QurlHealthHandler>>,
) -> impl IntoResponse {
    let health = handler.check_health().await;
    let status_code = match health.status {
        HealthStatus::Healthy => axum::http::StatusCode::OK,
        HealthStatus::Degraded => axum::http::StatusCode::OK, // Still serving traffic
        HealthStatus::Unhealthy => axum::http::StatusCode::SERVICE_UNAVAILABLE,
    };
    (status_code, Json(health))
}

/// Liveness probe - always returns OK if process is running
async fn liveness_endpoint() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "alive",
        "timestamp": chrono::Utc::now()
    }))
}

/// Readiness probe - returns OK only if ready to serve traffic
async fn readiness_endpoint(
    axum::extract::State(handler): axum::extract::State<Arc<QurlHealthHandler>>,
) -> impl IntoResponse {
    let health = handler.check_health().await;
    let ready = !matches!(health.status, HealthStatus::Unhealthy);
    let status_code = if ready {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (status_code, Json(serde_json::json!({
        "ready": ready,
        "status": health.status,
        "timestamp": chrono::Utc::now()
    })))
}

/// Health check component for qURL client
pub struct QurlClientHealthCheck {
    client: Arc<MultiRegionQurlClient>,
}

impl QurlClientHealthCheck {
    pub fn new(client: Arc<MultiRegionQurlClient>) -> Self {
        Self { client }
    }
}

impl HealthCheckable for QurlClientHealthCheck {
    fn name(&self) -> &str {
        "qurl-client"
    }

    fn check_health(&self) -> ComponentHealth {
        let regions = self.client.region_statuses();
        let mut healthy_count = 0;
        let mut degraded_count = 0;
        let mut unhealthy_count = 0;

        for region in &regions {
            match region.health {
                crate::qurl::RegionHealth::Healthy => healthy_count += 1,
                crate::qurl::RegionHealth::Degraded => degraded_count += 1,
                crate::qurl::RegionHealth::Unhealthy => unhealthy_count += 1,
                crate::qurl::RegionHealth::Unknown => degraded_count += 1,
            }
        }

        let (status, message) = if unhealthy_count > 0 {
            (HealthStatus::Unhealthy, Some(format!("{} region(s) unhealthy", unhealthy_count)))
        } else if degraded_count > 0 {
            (HealthStatus::Degraded, Some(format!("{} region(s) degraded", degraded_count)))
        } else {
            (HealthStatus::Healthy, Some("All regions healthy".to_string()))
        };

        ComponentHealth {
            name: self.name().to_string(),
            status,
            message,
            details: HashMap::from([
                ("healthy_regions".to_string(), serde_json::json!(healthy_count)),
                ("degraded_regions".to_string(), serde_json::json!(degraded_count)),
                ("unhealthy_regions".to_string(), serde_json::json!(unhealthy_count)),
            ]),
        }
    }
}

/// Health check component for cache
pub struct CacheHealthCheck {
    cache: Arc<QurlResolutionCache>,
}

impl CacheHealthCheck {
    pub fn new(cache: Arc<QurlResolutionCache>) -> Self {
        Self { cache }
    }
}

impl HealthCheckable for CacheHealthCheck {
    fn name(&self) -> &str {
        "qurl-cache"
    }

    fn check_health(&self) -> ComponentHealth {
        let stats = self.cache.stats();
        let hit_rate = if stats.hits + stats.misses > 0 {
            stats.hits as f64 / (stats.hits + stats.misses) as f64
        } else {
            1.0 // No requests yet, consider healthy
        };

        let utilization = stats.size as f64 / stats.capacity.max(1) as f64;

        let (status, message) = if hit_rate < 0.1 && stats.hits + stats.misses > 100 {
            (HealthStatus::Degraded, Some(format!("Low cache hit rate: {:.1}%", hit_rate * 100.0)))
        } else if utilization > 0.95 {
            (HealthStatus::Degraded, Some(format!("Cache near capacity: {:.1}%", utilization * 100.0)))
        } else {
            (HealthStatus::Healthy, Some("Cache operating normally".to_string()))
        };

        ComponentHealth {
            name: self.name().to_string(),
            status,
            message,
            details: HashMap::from([
                ("size".to_string(), serde_json::json!(stats.size)),
                ("capacity".to_string(), serde_json::json!(stats.capacity)),
                ("hit_rate".to_string(), serde_json::json!(hit_rate)),
                ("hits".to_string(), serde_json::json!(stats.hits)),
                ("misses".to_string(), serde_json::json!(stats.misses)),
                ("evictions".to_string(), serde_json::json!(stats.evictions)),
                ("expired".to_string(), serde_json::json!(stats.expired)),
            ]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_serialization() {
        let status = HealthStatus::Healthy;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"healthy\"");

        let status = HealthStatus::Degraded;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"degraded\"");
    }

    #[test]
    fn test_component_health() {
        let health = ComponentHealth {
            name: "test".to_string(),
            status: HealthStatus::Healthy,
            message: Some("OK".to_string()),
            details: HashMap::new(),
        };
        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("healthy"));
    }
}