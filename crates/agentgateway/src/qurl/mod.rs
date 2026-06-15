//! qURL client for agentgateway integration
//!
//! Provides secure, time-limited access to protected resources via qURL tokens
//! with automatic OpenNHP knock triggering.

pub mod advanced_cache;
pub mod circuit_breaker;
pub mod health;
pub mod metrics;
pub mod multi_region;
pub mod redis_cache;
pub mod retry;
pub mod tracing;

use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_core::prelude::Strng;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// Re-export key types
pub use advanced_cache::{CacheConfig, CacheEntry, CacheStats, TieredCache, QurlResolutionCache, L2Cache};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState, CircuitBreakerError, CircuitBreakerStats, QurlCircuitBreaker};
pub use health::{QurlHealthHandler, QurlHealthResponse, HealthStatus, RegionHealthInfo, CacheHealthInfo, CircuitBreakerHealthInfo, health_router, HealthCheckable, ComponentHealth, QurlClientHealthCheck, CacheHealthCheck};
pub use metrics::{init_metrics, record_qurl_resolve, record_qurl_resolve_error, record_cache_hit, record_cache_miss, update_cache_size, record_cache_eviction, update_circuit_breaker_state, record_circuit_breaker_failure, record_circuit_breaker_success, record_nhp_knock, record_nhp_knock_timeout, increment_active_requests, decrement_active_requests, update_region_health, record_region_latency, update_region_failures, MetricsHandler, metrics_router, ActiveRequestTracker, Timer};
pub use multi_region::{MultiRegionQurlClient, QurlRegionConfig, RegionHealth, RegionStatus};
pub use redis_cache::{RedisCacheConfig, RedisL2Cache, InMemoryL2Cache};
pub use retry::{RetryPolicy, HedgingConfig, with_retry, with_hedging, with_retry_and_hedging, with_circuit_breaker_retry};
pub use tracing::{TracingConfig, init_tracing, qurl_tracer, trace_resolve, trace_cache_operation, trace_nhp_knock, inject_trace_context, extract_trace_context, attributes};

/// qURL API client for resolving access tokens
#[derive(Clone)]
pub struct QurlClient {
    http: Client,
    api_url: String,
    api_key: SecretString,
    /// Cache of resolved URLs to avoid repeated qURL API calls
    cache: Arc<RwLock<QurlCache>>,
}

/// Cached resolved URL with expiry tracking
#[derive(Debug, Clone)]
struct CachedResolution {
    target_url: String,
    resource_id: Strng,
    expires_at: Instant,
    src_ip: std::net::IpAddr,
}

/// In-memory cache for qURL resolutions
#[derive(Default)]
struct QurlCache {
    /// Cache keyed by token (at_*) or resource_id (r_*)
    resolutions: std::collections::HashMap<String, CachedResolution>,
}

impl QurlClient {
    /// Create a new qURL client
    pub fn new(api_url: String, api_key: SecretString) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("failed to build qURL HTTP client")?;

        Ok(Self {
            http,
            api_url: api_url.trim_end_matches('/').to_string(),
            api_key,
            cache: Arc::new(RwLock::new(QurlCache::default())),
        })
    }

    /// Resolve a qURL access token (at_*) to get the target URL
    /// This triggers an NHP knock on the server side
    pub async fn resolve_token(&self, token: &str) -> Result<ResolveResponse> {
        let cache_key = format!("token:{}", token);

        // Check cache first
        if let Some(cached) = self.check_cache(&cache_key).await {
            debug!(token = %token, "qURL cache hit");
            return Ok(ResolveResponse {
                target_url: cached.target_url,
                resource_id: cached.resource_id.to_string(),
                access_grant: AccessGrant {
                    expires_in: Duration::from_secs(0), // cached, no fresh expiry
                    granted_at: Utc::now(),
                    src_ip: cached.src_ip,
                },
            });
        }

        info!(token = %token, "Resolving qURL token");
        let response = self.do_resolve_token(token).await?;

        // Cache the result
        self.cache_resolution(&cache_key, &response).await;

        Ok(response)
    }

    /// Resolve by resource ID (r_*) - gets the current valid token for the resource
    pub async fn resolve_by_resource(&self, resource_id: &str) -> Result<ResolveResponse> {
        let cache_key = format!("resource:{}", resource_id);

        if let Some(cached) = self.check_cache(&cache_key).await {
            debug!(resource_id = %resource_id, "qURL resource cache hit");
            return Ok(ResolveResponse {
                target_url: cached.target_url,
                resource_id: cached.resource_id.to_string(),
                access_grant: AccessGrant {
                    expires_in: Duration::from_secs(0),
                    granted_at: Utc::now(),
                    src_ip: cached.src_ip,
                },
            });
        }

        info!(resource_id = %resource_id, "Resolving qURL by resource ID");
        // For resource-based resolution, we'd typically need a token
        // This could be extended to support resource-level tokens
        Err(anyhow::anyhow!("resource-based resolution requires a token; use resolve_token with at_* token"))
    }

    /// Internal method to call qURL API
    async fn do_resolve_token(&self, token: &str) -> Result<ResolveResponse> {
        let url = format!("{}/v1/resolve", self.api_url);

        let request = ResolveRequest {
            access_token: token.to_string(),
        };

        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key.expose_secret()))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("qURL resolve request failed")?;

        let status = response.status();
        let body = response.text().await.context("failed to read qURL response")?;

        if !status.is_success() {
            return Err(self.handle_error(status, &body));
        }

        let envelope: QurlEnvelope<ResolveData> = serde_json::from_str(&body)
            .context("failed to parse qURL response")?;

        let data = envelope.data.context("qURL response missing data")?;

        Ok(ResolveResponse {
            target_url: data.target_url,
            resource_id: data.resource_id,
            access_grant: data.access_grant,
        })
    }

    /// Check cache for valid resolution
    async fn check_cache(&self, key: &str) -> Option<CachedResolution> {
        let cache = self.cache.read().await;
        cache.resolutions.get(key).and_then(|cached| {
            if Instant::now() < cached.expires_at {
                Some(cached.clone())
            } else {
                None
            }
        })
    }

    /// Cache a successful resolution
    async fn cache_resolution(&self, key: &str, response: &ResolveResponse) {
        // Parse expiry from access_grant
        let expires_at = Instant::now() + response.access_grant.expires_in;

        // Parse src_ip
        let src_ip = response.access_grant.src_ip;

        let cached = CachedResolution {
            target_url: response.target_url.clone(),
            resource_id: Strng::from(response.resource_id.as_str()),
            expires_at,
            src_ip,
        };

        let mut cache = self.cache.write().await;
        cache.resolutions.insert(key.to_string(), cached);
    }

    /// Handle qURL API errors
    fn handle_error(&self, status: StatusCode, body: &str) -> anyhow::Error {
        // Try to parse RFC 7807 problem details
        if let Ok(problem) = serde_json::from_str::<ProblemDetails>(body) {
            anyhow::anyhow!("qURL API error ({}): {} - {}", status, problem.title, problem.detail)
        } else {
            anyhow::anyhow!("qURL API error ({}): {}", status, body)
        }
    }

    /// Clear expired cache entries (can be called periodically)
    pub async fn cleanup_cache(&self) {
        let mut cache = self.cache.write().await;
        let now = Instant::now();
        cache.resolutions.retain(|_, v| v.expires_at > now);
    }
}

/// qURL API resolve request
#[derive(Serialize)]
struct ResolveRequest {
    access_token: String,
}

/// qURL API envelope response (all responses use this format)
#[derive(Deserialize)]
struct QurlEnvelope<T> {
    data: Option<T>,
    error: Option<ProblemDetails>,
    meta: Option<QurlMeta>,
}

/// qURL response metadata
#[derive(Deserialize)]
#[allow(dead_code)]
struct QurlMeta {
    request_id: String,
    #[serde(default)]
    page_size: usize,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}

/// RFC 7807 Problem Details
#[derive(Deserialize, Serialize, Debug)]
struct ProblemDetails {
    r#type: Option<String>,
    title: String,
    status: Option<u16>,
    detail: String,
    instance: Option<String>,
}

/// qURL resolve response data
#[derive(Deserialize)]
struct ResolveData {
    target_url: String,
    resource_id: String,
    access_grant: AccessGrant,
}

/// Resolved qURL response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveResponse {
    /// The actual target URL to connect to (after NHP knock grants access)
    pub target_url: String,
    /// The resource ID (r_*)
    pub resource_id: String,
    /// Access grant details
    pub access_grant: AccessGrant,
}

/// Access grant from qURL resolve
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessGrant {
    /// Seconds until the network access expires
    #[serde(with = "serde_duration")]
    pub expires_in: Duration,
    /// When the grant was issued
    pub granted_at: DateTime<Utc>,
    /// Source IP that was granted access (returned by qURL API)
    pub src_ip: std::net::IpAddr,
}

/// Serde helper for Duration
mod serde_duration {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

/// Configuration for qURL provider integration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QurlProviderConfig {
    /// Model name to use (overrides request model if set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// qURL API base URL (default: https://api.layerv.ai)
    #[serde(default = "default_api_url")]
    pub api_url: String,
    /// API key with qurl:resolve scope
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<SecretString>,
    /// qURL resource ID (r_*) - alternative to token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// qURL access token (at_*) - alternative to resource_id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// NHP agent ID for agent bootstrap
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nhp_agent_id: Option<String>,
    /// Custom formats to support (defaults to QurlNHP for completions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formats: Option<Vec<crate::llm::custom::ProviderFormatConfig>>,
    /// Cache TTL for resolved URLs (default: from access_grant.expires_in)
    #[serde(skip_serializing_if = "Option::is_none", with = "serde_option_duration")]
    pub cache_ttl: Option<Duration>,
}

fn default_api_url() -> String {
    "https://api.layerv.ai".to_string()
}

mod serde_option_duration {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(opt: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match opt {
            Some(d) => serializer.serialize_some(&d.as_secs()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt = Option::<u64>::deserialize(deserializer)?;
        Ok(opt.map(Duration::from_secs))
    }
}

/// Errors from qURL operations
#[derive(thiserror::Error, Debug)]
pub enum QurlError {
    #[error("qURL API request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("qURL API returned error: {0}")]
    ApiError(String),
    #[error("failed to parse qURL response: {0}")]
    ParseError(String),
    #[error("token expired or invalid")]
    TokenExpired,
    #[error("rate limited by qURL API: retry after {0} seconds")]
    RateLimited(u64),
}

impl From<QurlError> for anyhow::Error {
    fn from(err: QurlError) -> Self {
        anyhow::anyhow!(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qurl_client_creation() {
        let client = QurlClient::new(
            "https://api.layerv.ai".to_string(),
            SecretString::new("test-key".into()),
        );
        assert!(client.is_ok());
    }

    #[test]
    fn test_qurl_provider_config_defaults() {
        let config = QurlProviderConfig::default();
        assert_eq!(config.api_url, "https://api.layerv.ai");
        assert!(config.api_key.is_none());
        assert!(config.resource_id.is_none());
        assert!(config.token.is_none());
    }
}