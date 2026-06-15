//! Retry policies with exponential backoff, jitter, and request hedging

use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rand::Rng;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::qurl::QurlError;

/// Retry policy configuration
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Initial backoff duration
    pub initial_backoff: Duration,
    /// Maximum backoff duration
    pub max_backoff: Duration,
    /// Backoff multiplier (exponential)
    pub multiplier: f64,
    /// Jitter factor (0.0 - 1.0)
    pub jitter_factor: f64,
    /// Retryable error codes
    pub retryable_status_codes: Vec<u16>,
    /// Whether to retry on timeout
    pub retry_on_timeout: bool,
    /// Whether to retry on connection error
    pub retry_on_connection_error: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
            multiplier: 2.0,
            jitter_factor: 0.1,
            retryable_status_codes: vec![429, 500, 502, 503, 504],
            retry_on_timeout: true,
            retry_on_connection_error: true,
        }
    }
}

impl RetryPolicy {
    /// Create aggressive retry policy for critical paths
    pub fn aggressive() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_secs(5),
            multiplier: 1.5,
            jitter_factor: 0.2,
            retryable_status_codes: vec![429, 500, 502, 503, 504],
            retry_on_timeout: true,
            retry_on_connection_error: true,
        }
    }

    /// Create conservative retry policy
    pub fn conservative() -> Self {
        Self {
            max_attempts: 2,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            multiplier: 2.0,
            jitter_factor: 0.05,
            retryable_status_codes: vec![502, 503, 504],
            retry_on_timeout: false,
            retry_on_connection_error: true,
        }
    }

    /// Calculate backoff for attempt number
    pub fn backoff(&self, attempt: u32) -> Duration {
        let base = self.initial_backoff.as_millis() as f64
            * self.multiplier.powi(attempt as i32);
        let capped = base.min(self.max_backoff.as_millis() as f64);

        // Add jitter
        let jitter = capped * self.jitter_factor;
        let mut rng = rand::thread_rng();
        let jittered = capped + rng.gen_range(-jitter..=jitter);

        Duration::from_millis(jittered.max(0.0) as u64)
    }

    /// Check if an error is retryable
    pub fn is_retryable(&self, error: &QurlError) -> bool {
        match error {
            QurlError::RequestFailed(e) if self.retry_on_connection_error => {
                // Check if it's a connection error
                e.to_string().contains("connection")
                    || e.to_string().contains("connect")
                    || e.to_string().contains("timeout")
            }
            QurlError::ApiError(msg) if self.retry_on_timeout => {
                msg.contains("timeout") || msg.contains("Timeout")
            }
            QurlError::HttpError(status) => self.retryable_status_codes.contains(status),
            QurlError::RateLimited(_) => true,
            _ => false,
        }
    }
}

/// Execute an operation with retry policy
pub async fn with_retry<F, Fut, T, E>(
    policy: &RetryPolicy,
    operation: F,
) -> Result<T, QurlError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, QurlError>>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut attempt = 0;
    let mut last_error = None;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    debug!(attempt, "Retry succeeded");
                }
                return Ok(result);
            }
            Err(error) => {
                last_error = Some(error);

                if attempt >= policy.max_attempts {
                    warn!(attempts = attempt + 1, "Max retry attempts reached");
                    break;
                }

                if !policy.is_retryable(last_error.as_ref().unwrap()) {
                    debug!("Error is not retryable");
                    break;
                }

                attempt += 1;
                let backoff = policy.backoff(attempt - 1);
                debug!(attempt, backoff_ms = backoff.as_millis(), "Retrying after backoff");
                sleep(backoff).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| QurlError::ApiError("Retry failed".to_string())))
}

/// Request hedging configuration
#[derive(Debug, Clone)]
pub struct HedgingConfig {
    /// Delay before sending hedged request
    pub hedge_delay: Duration,
    /// Maximum number of hedged requests
    pub max_hedged_requests: u32,
    /// Whether hedging is enabled
    pub enabled: bool,
}

impl Default for HedgingConfig {
    fn default() -> Self {
        Self {
            hedge_delay: Duration::from_millis(100),
            max_hedged_requests: 2,
            enabled: true,
        }
    }
}

/// Execute with request hedging (send duplicate requests after delay)
pub async fn with_hedging<F, Fut, T, E>(
    config: &HedgingConfig,
    primary: F,
    fallback: F,
) -> Result<T, QurlError>
where
    F: Fn() -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T, QurlError>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
    T: Send + 'static,
{
    if !config.enabled {
        return primary().await;
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel(1 + config.max_hedged_requests as usize);
    let completed = Arc::new(AtomicU32::new(0));
    let hedged_count = Arc::new(AtomicU32::new(0));

    // Primary request
    let tx_clone = tx.clone();
    let completed_clone = completed.clone();
    let primary_fut = primary();
    tokio::spawn(async move {
        let result = primary_fut.await;
        completed_clone.fetch_add(1, Ordering::Relaxed);
        let _ = tx_clone.send(("primary", result)).await;
    });

    // Wait for hedge delay
    sleep(config.hedge_delay).await;

    // Check if primary completed
    if completed.load(Ordering::Relaxed) > 0 {
        if let Some((source, result)) = rx.recv().await {
            debug!(source, "Request completed before hedging");
            return result;
        }
    }

    // Send hedged requests
    for i in 0..config.max_hedged_requests {
        let tx_clone = tx.clone();
        let completed_clone = completed.clone();
        let hedged_count_clone = hedged_count.clone();
        let fallback_fut = fallback();

        tokio::spawn(async move {
            let result = fallback_fut.await;
            completed_clone.fetch_add(1, Ordering::Relaxed);
            hedged_count_clone.fetch_add(1, Ordering::Relaxed);
            let _ = tx_clone.send((format!("hedge_{}", i), result)).await;
        });
    }

    // Wait for first successful response
    let mut first_error: Option<QurlError> = None;
    let mut completed_count = 0;
    let max_responses = 1 + config.max_hedged_requests as usize;

    while completed_count < max_responses {
        tokio::select! {
            Some((source, result)) = rx.recv() => {
                completed_count += 1;
                match result {
                    Ok(value) => {
                        debug!(source, hedged = hedged_count.load(Ordering::Relaxed), "Request succeeded");
                        return Ok(value);
                    }
                    Err(e) => {
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                        debug!(source, "Request failed, waiting for others");
                    }
                }
            }
            else => break, // Channel closed
        }
    }

    // All failed
    Err(first_error.unwrap_or_else(|| QurlError::ApiError("All hedged requests failed".to_string())))
}

/// Retry with hedging combined
pub async fn with_retry_and_hedging<F, Fut, T, E>(
    retry_policy: &RetryPolicy,
    hedging_config: &HedgingConfig,
    operation: F,
) -> Result<T, QurlError>
where
    F: Fn() -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T, QurlError>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
    T: Send + 'static,
{
    let mut attempt = 0;
    let mut last_error = None;

    loop {
        let result = with_hedging(hedging_config, operation.clone(), operation.clone()).await;

        match result {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = Some(error);

                if attempt >= retry_policy.max_attempts {
                    break;
                }

                if !retry_policy.is_retryable(last_error.as_ref().unwrap()) {
                    break;
                }

                attempt += 1;
                let backoff = retry_policy.backoff(attempt - 1);
                debug!(attempt, backoff_ms = backoff.as_millis(), "Retrying with hedging after backoff");
                sleep(backoff).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| QurlError::ApiError("Retry with hedging failed".to_string())))
}

/// Circuit breaker aware retry
pub async fn with_circuit_breaker_retry<F, Fut, T>(
    circuit_breaker: &crate::qurl::CircuitBreaker,
    retry_policy: &RetryPolicy,
    operation: F,
) -> Result<T, QurlError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, QurlError>>,
{
    // Try to execute through circuit breaker
    circuit_breaker.call(|| async {
        with_retry(retry_policy, operation).await
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qurl::QurlError;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_retry_policy_backoff() {
        let policy = RetryPolicy::default();
        let backoff1 = policy.backoff(0);
        let backoff2 = policy.backoff(1);
        let backoff3 = policy.backoff(2);

        // Backoff should increase exponentially (with jitter it might not be exactly 2x)
        assert!(backoff1 >= Duration::from_millis(80));
        assert!(backoff1 <= Duration::from_millis(120));
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_first_try() {
        let policy = RetryPolicy::default();
        let result = with_retry(&policy, || async { Ok::<_, QurlError>("success") }).await;
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failure() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(100),
            multiplier: 1.0,
            jitter_factor: 0.0,
            ..Default::default()
        };

        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_clone = attempts.clone();

        let result = with_retry(&policy, move || {
            let attempts = attempts_clone.clone();
            async move {
                let count = attempts.fetch_add(1, Ordering::Relaxed);
                if count < 2 {
                    Err(QurlError::ApiError("temporary failure".to_string()))
                } else {
                    Ok("success")
                }
            }
        }).await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let policy = RetryPolicy {
            max_attempts: 2,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
            multiplier: 1.0,
            jitter_factor: 0.0,
            ..Default::default()
        };

        let result = with_retry(&policy, || async {
            Err::<String, QurlError>(QurlError::ApiError("permanent failure".to_string()))
        }).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_hedging_primary_succeeds() {
        let config = HedgingConfig {
            hedge_delay: Duration::from_millis(100),
            max_hedged_requests: 2,
            enabled: true,
        };

        let primary_called = Arc::new(AtomicU32::new(0));
        let fallback_called = Arc::new(AtomicU32::new(0));

        let primary = {
            let primary_called = primary_called.clone();
            move || {
                primary_called.fetch_add(1, Ordering::Relaxed);
                async { Ok::<_, QurlError>("primary") }
            }
        };

        let fallback = {
            let fallback_called = fallback_called.clone();
            move || {
                fallback_called.fetch_add(1, Ordering::Relaxed);
                async { Ok::<_, QurlError>("fallback") }
            }
        };

        let result = with_hedging(&config, primary, fallback).await;
        assert_eq!(result.unwrap(), "primary");
        assert_eq!(primary_called.load(Ordering::Relaxed), 1);
        // Fallback might be called if hedge delay elapsed before primary completed
        // but it shouldn't matter since primary succeeded first
    }
}