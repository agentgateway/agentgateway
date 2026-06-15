//! Circuit breaker implementation for qURL API calls
//!
//! Provides resilience against cascading failures when qURL API is unavailable.

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use parking_lot::RwLock;
use tracing::{debug, warn};

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation, requests pass through
    Closed,
    /// Failure threshold exceeded, requests fail fast
    Open,
    /// Testing if service recovered, limited requests allowed
    HalfOpen,
}

/// Configuration for circuit breaker behavior
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening circuit
    pub failure_threshold: usize,
    /// Number of successes in half-open before closing
    pub success_threshold: usize,
    /// Time to wait before transitioning to half-open
    pub timeout: Duration,
    /// Minimum requests needed before evaluating failure rate
    pub minimum_requests: usize,
    /// Failure rate threshold (0.0-1.0) to open circuit
    pub failure_rate_threshold: f64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
            minimum_requests: 10,
            failure_rate_threshold: 0.5,
        }
    }
}

/// Circuit breaker for protecting upstream service calls
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: RwLock<CircuitState>,
    failure_count: AtomicUsize,
    success_count: AtomicUsize,
    request_count: AtomicUsize,
    last_failure_time: RwLock<Option<Instant>>,
    last_state_change: RwLock<Instant>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with default config
    pub fn new() -> Self {
        Self::with_config(CircuitBreakerConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: RwLock::new(CircuitState::Closed),
            failure_count: AtomicUsize::new(0),
            success_count: AtomicUsize::new(0),
            request_count: AtomicUsize::new(0),
            last_failure_time: RwLock::new(None),
            last_state_change: RwLock::new(Instant::now()),
        }
    }

    /// Execute an operation with circuit breaker protection
    pub async fn call<F, T, E>(&self, operation: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Result<T, E>,
    {
        // Check if we can proceed
        self.check_state()?;

        self.request_count.fetch_add(1, Ordering::Relaxed);

        match operation() {
            Ok(result) => {
                self.on_success();
                Ok(result)
            }
            Err(err) => {
                self.on_failure();
                Err(CircuitBreakerError::OperationFailed(err))
            }
        }
    }

    /// Check current state and transition if needed
    fn check_state(&self) -> Result<(), CircuitBreakerError<()>> {
        let mut state = self.state.write();

        match *state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open => {
                // Check if timeout has elapsed
                let last_change = *self.last_state_change.read();
                if last_change.elapsed() >= self.config.timeout {
                    debug!("Circuit breaker transitioning to half-open");
                    *state = CircuitState::HalfOpen;
                    *self.last_state_change.write() = Instant::now();
                    self.success_count.store(0, Ordering::Relaxed);
                    Ok(())
                } else {
                    Err(CircuitBreakerError::CircuitOpen)
                }
            }
            CircuitState::HalfOpen => {
                // Allow limited requests through
                Ok(())
            }
        }
    }

    /// Record a successful call
    fn on_success(&self) {
        let failures = self.failure_count.load(Ordering::Relaxed);
        let requests = self.request_count.load(Ordering::Relaxed);

        let mut state = self.state.write();

        match *state {
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::Relaxed) + 1;
                if successes >= self.config.success_threshold {
                    debug!("Circuit breaker closing after {} successes", successes);
                    *state = CircuitState::Closed;
                    *self.last_state_change.write() = Instant::now();
                    self.failure_count.store(0, Ordering::Relaxed);
                    self.request_count.store(0, Ordering::Relaxed);
                }
            }
            CircuitState::Closed => {
                // Reset failure count on success in closed state
                if failures > 0 {
                    self.failure_count.store(0, Ordering::Relaxed);
                }
            }
            CircuitState::Open => {
                // Should not happen due to check_state
            }
        }
    }

    /// Record a failed call
    fn on_failure(&self) {
        let failures = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        let requests = self.request_count.load(Ordering::Relaxed);

        *self.last_failure_time.write() = Some(Instant::now());

        let mut state = self.state.write();

        match *state {
            CircuitState::Closed => {
                // Check if we should open
                if requests >= self.config.minimum_requests {
                    let failure_rate = failures as f64 / requests as f64;
                    if failure_rate >= self.config.failure_rate_threshold
                        || failures >= self.config.failure_threshold
                    {
                        warn!(
                            "Circuit breaker opening: failures={}, requests={}, rate={:.2}",
                            failures, requests, failure_rate
                        );
                        *state = CircuitState::Open;
                        *self.last_state_change.write() = Instant::now();
                    }
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open reopens the circuit
                warn!("Circuit breaker reopening after half-open failure");
                *state = CircuitState::Open;
                *self.last_state_change.write() = Instant::now();
                self.success_count.store(0, Ordering::Relaxed);
            }
            CircuitState::Open => {
                // Already open
            }
        }
    }

    /// Get current circuit state
    pub fn state(&self) -> CircuitState {
        *self.state.read()
    }

    /// Get statistics
    pub fn stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            state: self.state(),
            failure_count: self.failure_count.load(Ordering::Relaxed),
            success_count: self.success_count.load(Ordering::Relaxed),
            request_count: self.request_count.load(Ordering::Relaxed),
        }
    }

    /// Force reset to closed state (for testing/admin)
    pub fn reset(&self) {
        *self.state.write() = CircuitState::Closed;
        self.failure_count.store(0, Ordering::Relaxed);
        self.success_count.store(0, Ordering::Relaxed);
        self.request_count.store(0, Ordering::Relaxed);
        *self.last_state_change.write() = Instant::now();
    }
}

/// Circuit breaker statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct CircuitBreakerStats {
    pub state: CircuitState,
    pub failure_count: usize,
    pub success_count: usize,
    pub request_count: usize,
}

/// Errors from circuit breaker
#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError<E> {
    #[error("Circuit breaker is open")]
    CircuitOpen,
    #[error("Operation failed: {0}")]
    OperationFailed(E),
}

impl<E> From<CircuitBreakerError<E>> for crate::QurlError
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(err: CircuitBreakerError<E>) -> Self {
        match err {
            CircuitBreakerError::CircuitOpen => crate::QurlError::ApiError("circuit breaker open".to_string()),
            CircuitBreakerError::OperationFailed(e) => crate::QurlError::RequestFailed(e.into()),
        }
    }
}

/// Circuit breaker per qURL API endpoint
pub type QurlCircuitBreaker = Arc<CircuitBreaker>;

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_circuit_breaker_closes_after_success() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            timeout: Duration::from_millis(100),
            minimum_requests: 1,
            failure_rate_threshold: 0.5,
        });

        // Initial state is closed
        assert_eq!(cb.state(), CircuitState::Closed);

        // Fail enough to open
        for _ in 0..3 {
            let _ = cb.call(|| Err::<(), &str>("fail")).await;
        }
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for timeout
        sleep(Duration::from_millis(150)).await;

        // Should be half-open now
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // Two successes should close it
        let _ = cb.call(|| Ok(())).await;
        let _ = cb.call(|| Ok(())).await;
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_reopens_on_half_open_failure() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 3,
            timeout: Duration::from_millis(100),
            minimum_requests: 1,
            failure_rate_threshold: 0.5,
        });

        // Open the circuit
        let _ = cb.call(|| Err::<(), &str>("fail")).await;
        let _ = cb.call(|| Err::<(), &str>("fail")).await;
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait and go half-open
        sleep(Duration::from_millis(150)).await;
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // Fail in half-open -> reopen
        let _ = cb.call(|| Err::<(), &str>("fail")).await;
        assert_eq!(cb.state(), CircuitState::Open);
    }
}