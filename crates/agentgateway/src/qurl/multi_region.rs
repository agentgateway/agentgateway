//! Multi-region qURL API client with failover and health checks

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use url::Url;

use crate::qurl::{QurlClient, QurlProviderConfig, ResolveRequest, ResolveResponse, QurlError};

/// Region configuration for qURL API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QurlRegionConfig {
    /// Region identifier
    pub region_id: String,
    /// qURL API base URL for this region
    pub api_url: String,
    /// API key for this region (optional, fallback to primary)
    pub api_key: Option<String>,
    /// Priority (lower = higher priority)
    pub priority: u32,
    /// Health check endpoint (relative to api_url)
    pub health_endpoint: String,
    /// Health check interval
    pub health_interval: Duration,
    /// Timeout for requests to this region
    pub timeout: Duration,
    /// Whether this region is enabled
    pub enabled: bool,
}

impl Default for QurlRegionConfig {
    fn default() -> Self {
        Self {
            region_id: "primary".to_string(),
            api_url: "https://api.layerv.ai".to_string(),
            api_key: None,
            priority: 100,
            health_endpoint: "/v1/health".to_string(),
            health_interval: Duration::from_secs(30),
            timeout: Duration::from_secs(10),
            enabled: true,
        }
    }
}

/// Health status of a region
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegionHealth {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// Region health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionStatus {
    pub region_id: String,
    pub health: RegionHealth,
    pub last_check: chrono::DateTime<chrono::Utc>,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub latency_ms: Option<u64>,
}

/// Multi-region qURL client with automatic failover
pub struct MultiRegionQurlClient {
    regions: Arc<RwLock<Vec<RegionState>>>,
    primary_config: QurlProviderConfig,
    http_client: Client,
    current_region: Arc<RwLock<Option<String>>>,
    failover_enabled: bool,
    max_retries_per_region: u32,
    retry_delay: Duration,
}

struct RegionState {
    config: QurlRegionConfig,
    client: QurlClient,
    status: RegionStatus,
    health_check_handle: Option<tokio::task::JoinHandle<()>>,
}

impl MultiRegionQurlClient {
    /// Create new multi-region client
    pub fn new(
        primary_config: QurlProviderConfig,
        regions: Vec<QurlRegionConfig>,
        failover_enabled: bool,
    ) -> Result<Self, QurlError> {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| QurlError::ConfigError(format!("Failed to create HTTP client: {}", e)))?;

        // Validate we have at least one region
        if regions.is_empty() {
            return Err(QurlError::ConfigError("At least one region required".to_string()));
        }

        // Sort regions by priority
        let mut sorted_regions = regions;
        sorted_regions.sort_by_key(|r| r.priority);

        let mut region_states = Vec::new();
        for region_config in sorted_regions {
            if !region_config.enabled {
                continue;
            }

            // Create region-specific config
            let region_client_config = QurlProviderConfig {
                api_url: region_config.api_url.clone(),
                api_key: region_config.api_key.clone().or_else(|| primary_config.api_key.clone())
                    .ok_or_else(|| QurlError::ConfigError("API key required".to_string()))?,
                resource_id: primary_config.resource_id.clone(),
                token: primary_config.token.clone(),
                nhp_agent_id: primary_config.nhp_agent_id.clone(),
                cache_ttl: primary_config.cache_ttl,
            };

            let client = QurlClient::new(region_client_config)?;

            let status = RegionStatus {
                region_id: region_config.region_id.clone(),
                health: RegionHealth::Unknown,
                last_check: chrono::Utc::now(),
                consecutive_failures: 0,
                consecutive_successes: 0,
                latency_ms: None,
            };

            region_states.push(RegionState {
                config: region_config,
                client,
                status,
                health_check_handle: None,
            });
        }

        if region_states.is_empty() {
            return Err(QurlError::ConfigError("No enabled regions".to_string()));
        }

        // Set current region to highest priority
        let current_region = Some(region_states[0].config.region_id.clone());

        Ok(Self {
            regions: Arc::new(RwLock::new(region_states)),
            primary_config,
            http_client,
            current_region: Arc::new(RwLock::new(current_region)),
            failover_enabled,
            max_retries_per_region: 2,
            retry_delay: Duration::from_millis(100),
        })
    }

    /// Resolve a qURL token with automatic failover
    pub async fn resolve(&self, request: ResolveRequest) -> Result<ResolveResponse, QurlError> {
        let regions = self.get_healthy_regions().await;

        if regions.is_empty() {
            // No healthy regions, try all as last resort
            warn!("No healthy regions available, attempting all regions");
            return self.resolve_with_fallback(request).await;
        }

        // Try primary healthy region first
        for region_id in regions {
            if let Some(result) = self.try_region(&region_id, &request).await {
                return Ok(result);
            }
        }

        // If all healthy regions failed and failover is enabled, try unhealthy ones
        if self.failover_enabled {
            return self.resolve_with_fallback(request).await;
        }

        Err(QurlError::ApiError("All healthy regions failed".to_string()))
    }

    /// Try resolving with a specific region
    async fn try_region(&self, region_id: &str, request: &ResolveRequest) -> Option<ResolveResponse> {
        let regions = self.regions.read();
        let region_state = regions.iter().find(|r| r.config.region_id == region_id)?;

        let start = Instant::now();
        let result = region_state.client.resolve(request.clone()).await;
        let latency = start.elapsed();

        // Update health based on result
        drop(regions);
        self.update_region_health(region_id, result.is_ok(), latency).await;

        match result {
            Ok(response) => Some(response),
            Err(e) => {
                debug!(region = %region_id, "Region resolve failed: {}", e);
                None
            }
        }
    }

    /// Resolve trying all regions as fallback
    async fn resolve_with_fallback(&self, request: ResolveRequest) -> Result<ResolveResponse, QurlError> {
        let region_ids: Vec<String> = {
            let regions = self.regions.read();
            regions.iter().map(|r| r.config.region_id.clone()).collect()
        };

        let mut last_error = None;

        for region_id in region_ids {
            for attempt in 0..=self.max_retries_per_region {
                if attempt > 0 {
                    tokio::time::sleep(self.retry_delay * attempt).await;
                }

                let regions = self.regions.read();
                if let Some(region_state) = regions.iter().find(|r| r.config.region_id == region_id) {
                    let start = Instant::now();
                    let result = region_state.client.resolve(request.clone()).await;
                    let latency = start.elapsed();

                    drop(regions);
                    self.update_region_health(&region_id, result.is_ok(), latency).await;

                    match result {
                        Ok(response) => {
                            info!(region = %region_id, attempt, "Fallback resolve succeeded");
                            return Ok(response);
                        }
                        Err(e) => {
                            last_error = Some(e);
                            debug!(region = %region_id, attempt, "Fallback attempt failed");
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| QurlError::ApiError("All regions failed".to_string())))
    }

    /// Get list of healthy region IDs (sorted by priority)
    async fn get_healthy_regions(&self) -> Vec<String> {
        let regions = self.regions.read();
        regions
            .iter()
            .filter(|r| matches!(r.status.health, RegionHealth::Healthy | RegionHealth::Degraded))
            .map(|r| r.config.region_id.clone())
            .collect()
    }

    /// Update region health based on request outcome
    async fn update_region_health(&self, region_id: &str, success: bool, latency: Duration) {
        let mut regions = self.regions.write();
        if let Some(region_state) = regions.iter_mut().find(|r| r.config.region_id == region_id) {
            let status = &mut region_state.status;
            status.last_check = chrono::Utc::now();
            status.latency_ms = Some(latency.as_millis() as u64);

            if success {
                status.consecutive_successes += 1;
                status.consecutive_failures = 0;
                if status.consecutive_successes >= 2 {
                    status.health = RegionHealth::Healthy;
                } else if status.health == RegionHealth::Unhealthy {
                    status.health = RegionHealth::Degraded;
                }
            } else {
                status.consecutive_failures += 1;
                status.consecutive_successes = 0;
                if status.consecutive_failures >= 3 {
                    status.health = RegionHealth::Unhealthy;
                } else if status.health == RegionHealth::Healthy {
                    status.health = RegionHealth::Degraded;
                }
            }

            debug!(
                region = %region_id,
                health = ?status.health,
                failures = status.consecutive_failures,
                successes = status.consecutive_successes,
                latency_ms = status.latency_ms,
                "Region health updated"
            );
        }
    }

    /// Start health check tasks for all regions
    pub fn start_health_checks(&self) {
        let regions = self.regions.clone();
        let http_client = self.http_client.clone();

        for region_state in regions.read().iter() {
            let config = region_state.config.clone();
            let http_client = http_client.clone();
            let regions = regions.clone();

            let handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(config.health_interval);
                loop {
                    interval.tick().await;
                    Self::perform_health_check(&config, &http_client, &regions).await;
                }
            });

            // Store handle (would need mutable access to region_state)
            // For now, we just spawn and let it run
            let _ = handle;
        }
    }

    /// Perform health check on a region
    async fn perform_health_check(
        config: &QurlRegionConfig,
        http_client: &Client,
        regions: &Arc<RwLock<Vec<RegionState>>>,
    ) {
        let url = format!("{}{}", config.api_url, config.health_endpoint);
        let start = Instant::now();

        let result = http_client
            .get(&url)
            .timeout(config.timeout)
            .send()
            .await;

        let latency = start.elapsed();

        let (success, status_code) = match result {
            Ok(resp) => (resp.status().is_success(), resp.status().as_u16()),
            Err(_) => (false, 0),
        };

        let mut regions_guard = regions.write();
        if let Some(region_state) = regions_guard.iter_mut().find(|r| r.config.region_id == config.region_id) {
            let status = &mut region_state.status;
            status.last_check = chrono::Utc::now();
            status.latency_ms = Some(latency.as_millis() as u64);

            if success {
                status.consecutive_successes += 1;
                status.consecutive_failures = 0;
                if status.consecutive_successes >= 2 {
                    status.health = RegionHealth::Healthy;
                } else if status.health == RegionHealth::Unhealthy {
                    status.health = RegionHealth::Degraded;
                }
            } else {
                status.consecutive_failures += 1;
                status.consecutive_successes = 0;
                if status.consecutive_failures >= 3 {
                    status.health = RegionHealth::Unhealthy;
                } else if status.health == RegionHealth::Healthy {
                    status.health = RegionHealth::Degraded;
                }
            }

            debug!(
                region = %config.region_id,
                health = ?status.health,
                status_code,
                latency_ms = status.latency_ms,
                "Health check completed"
            );
        }
    }

    /// Get current region statuses
    pub fn region_statuses(&self) -> Vec<RegionStatus> {
        self.regions.read().iter().map(|r| r.status.clone()).collect()
    }

    /// Get current primary region
    pub fn current_region(&self) -> Option<String> {
        self.current_region.read().clone()
    }

    /// Manually trigger failover to next healthy region
    pub async fn failover(&self) -> Result<String, QurlError> {
        let healthy = self.get_healthy_regions().await;
        let current = self.current_region.read().clone();

        let next = healthy.iter().find(|r| {
            if let Some(ref curr) = current {
                r != curr
            } else {
                true
            }
        });

        if let Some(next_region) = next {
            *self.current_region.write() = Some(next_region.clone());
            info!("Manual failover to region: {}", next_region);
            Ok(next_region.clone())
        } else {
            Err(QurlError::ApiError("No healthy region to failover to".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_config_default() {
        let config = QurlRegionConfig::default();
        assert_eq!(config.region_id, "primary");
        assert_eq!(config.priority, 100);
        assert!(config.enabled);
    }

    #[test]
    fn test_region_health_ordering() {
        use RegionHealth::*;
        assert!(Healthy > Degraded);
        assert!(Degraded > Unhealthy);
        assert!(Unhealthy > Unknown);
    }
}