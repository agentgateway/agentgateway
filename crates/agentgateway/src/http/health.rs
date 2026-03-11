//! Configurable backend health / eviction (outlier detection) policy.
//!
//! When a response is considered unhealthy (by CEL or default 5xx), the backend can be
//! evicted for a configurable duration. If no health policy is configured, no eviction
//! is applied. Optional health/failure thresholds and recovery health support multi-request
//! and recovery behavior.

use std::sync::Arc;
use std::time::Duration;

use crate::cel::Expression;
use crate::{serde_dur_option, *};

/// Eviction sub-policy: how long to remove a backend from the active set after an unhealthy response.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Eviction {
	/// Base ejection time. When absent, falls back to `Retry-After` header (e.g. 429)
	/// or retry policy backoff, then a default (e.g. 3s).
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	pub duration: Option<Duration>,

	/// Health score to restore when a backend returns from eviction (e.g. 0.2 for gradual recovery).
	/// When absent, health is left unchanged on recovery.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub restore_health: Option<f64>,

	/// Number of consecutive unhealthy responses required before evicting the backend.
	/// When both this and `health_threshold` are set, eviction triggers when either condition is met.
	/// When neither is set, a single unhealthy response can trigger eviction.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub consecutive_failures: Option<i32>,

	/// Evict only when endpoint health (EWMA) is below this threshold (0.0–1.0).
	/// When both this and `consecutive_failures` are set, eviction triggers when either condition is met.
	/// When neither is set, a single unhealthy response triggers eviction.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub health_threshold: Option<f64>,
}

/// Health policy: determines when a backend is unhealthy and how to evict it.
///
/// Maps to the proto `Health` message containing an `unhealthy_condition` CEL expression
/// and an optional `Eviction` sub-message with eviction settings.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
	/// CEL expression evaluated per response; `true` means this response is unhealthy (evict).
	/// When absent, any 5xx response, or a connection failure, is treated as unhealthy.
	/// This default lowers the backend's health score but does not trigger eviction on its own.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub unhealthy_expression: Option<Arc<Expression>>,

	/// Eviction settings. When absent, falls back to defaults.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub eviction: Option<Eviction>,
}

const DEFAULT_EVICTION_SECS: u64 = 3;

impl Policy {
	/// Returns the configured base eviction duration, if any.
	pub fn eviction_duration(&self) -> Option<Duration> {
		self.eviction.as_ref().and_then(|e| e.duration)
	}

	/// Computes the eviction decision for a single request.
	///
	/// `current_health` and `consecutive_failure_count` reflect state **before** this request
	/// is recorded. `fallback_duration` is used when no explicit eviction duration is configured
	/// (e.g. from `Retry-After` headers or retry backoff).
	///
	/// Returns `(is_healthy, eviction_duration, restore_health)`.
	pub(crate) fn eviction_decision(
		&self,
		current_health: f64,
		consecutive_failure_count: u64,
		times_ejected: u64,
		unhealthy: bool,
		fallback_duration: Option<Duration>,
	) -> (bool, Option<Duration>, Option<f64>) {
		let health = !unhealthy;
		let ev = self.eviction.as_ref();
		let eviction_duration = if unhealthy {
			let base_duration = self
				.eviction_duration()
				.or(fallback_duration)
				.or(Some(Duration::from_secs(DEFAULT_EVICTION_SECS)));
			let health_threshold = ev.and_then(|e| e.health_threshold);
			let consecutive_failures = ev.and_then(|e| e.consecutive_failures);
			// +1 because the current failure hasn't been recorded yet.
			let failures_including_current = consecutive_failure_count + 1;
			let health_below = health_threshold.is_some_and(|t| current_health < t);
			let consecutive_exceeded = consecutive_failures
				.is_some_and(|count| count > 0 && failures_including_current >= count as u64);
			let below_threshold = if health_threshold.is_some() || consecutive_failures.is_some() {
				health_below || consecutive_exceeded
			} else {
				true
			};
			if below_threshold {
				// Multiplicative backoff: base_duration * (times_ejected + 1).
				// No cap -- if all endpoints are evicted the loadbalancer falls back
				// to returning evicted endpoints, which is better than an arbitrary
				// max that unevenly distributes load across equally-degraded backends.
				let multiplier = times_ejected.saturating_add(1);
				base_duration.map(|d| d.saturating_mul(multiplier as u32))
			} else {
				None
			}
		} else {
			None
		};
		(health, eviction_duration, ev.and_then(|e| e.restore_health))
	}
}

/// Local/config eviction sub-policy with duration as string; mirrors `Eviction`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalEviction {
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub duration: Option<Duration>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub restore_health: Option<f64>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub consecutive_failures: Option<i32>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub health_threshold: Option<f64>,
}

/// Local/config health policy with CEL as string; converted to Policy by compiling the expression.
/// Mirrors the proto `Health` message structure.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalHealthPolicy {
	/// CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.
	/// When unset, any 5xx or connection failure is treated as unhealthy.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub unhealthy_expression: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub eviction: Option<LocalEviction>,
}

impl TryFrom<LocalHealthPolicy> for Policy {
	type Error = crate::cel::Error;
	fn try_from(local: LocalHealthPolicy) -> Result<Self, Self::Error> {
		let eviction = match local.eviction {
			Some(e) => {
				let validate_score = |field: &str, value: Option<f64>| -> Result<(), crate::cel::Error> {
					if let Some(v) = value
						&& !(0.0..=1.0).contains(&v)
					{
						return Err(crate::cel::Error::Variable(format!(
							"health.eviction.{field} must be between 0.0 and 1.0"
						)));
					}
					Ok(())
				};
				validate_score("healthThreshold", e.health_threshold)?;
				validate_score("restoreHealth", e.restore_health)?;
				Some(Eviction {
					duration: e.duration,
					restore_health: e.restore_health,
					consecutive_failures: e.consecutive_failures,
					health_threshold: e.health_threshold,
				})
			},
			None => None,
		};

		let unhealthy_expression = match local.unhealthy_expression {
			Some(s) if !s.trim().is_empty() => Some(Arc::new(Expression::new_strict(&s)?)),
			_ => None,
		};
		Ok(Policy {
			unhealthy_expression,
			eviction,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn policy_with_threshold(threshold: f64) -> Policy {
		Policy {
			eviction: Some(Eviction {
				health_threshold: Some(threshold),
				..Default::default()
			}),
			..Default::default()
		}
	}

	fn policy_with_consecutive(count: i32) -> Policy {
		Policy {
			eviction: Some(Eviction {
				consecutive_failures: Some(count),
				..Default::default()
			}),
			..Default::default()
		}
	}

	fn policy_with_eviction_duration(secs: u64) -> Policy {
		Policy {
			eviction: Some(Eviction {
				duration: Some(Duration::from_secs(secs)),
				..Default::default()
			}),
			..Default::default()
		}
	}

	// --- healthy responses never trigger eviction ---

	#[test]
	fn healthy_response_no_eviction() {
		let policy = Policy::default();
		let (healthy, eviction, _) = policy.eviction_decision(1.0, 0, 0, false, None);
		assert!(healthy);
		assert!(eviction.is_none());
	}

	#[test]
	fn healthy_response_with_threshold_no_eviction() {
		let policy = policy_with_threshold(0.5);
		let (healthy, eviction, _) = policy.eviction_decision(0.1, 10, 5, false, None);
		assert!(healthy);
		assert!(eviction.is_none());
	}

	// --- no thresholds: any unhealthy triggers eviction ---

	#[test]
	fn unhealthy_no_thresholds_evicts() {
		let policy = policy_with_eviction_duration(10);
		let (healthy, eviction, _) = policy.eviction_decision(1.0, 0, 0, true, None);
		assert!(!healthy);
		assert_eq!(eviction, Some(Duration::from_secs(10)));
	}

	#[test]
	fn unhealthy_default_eviction_duration() {
		let policy = Policy::default();
		let (_, eviction, _) = policy.eviction_decision(1.0, 0, 0, true, None);
		assert_eq!(eviction, Some(Duration::from_secs(DEFAULT_EVICTION_SECS)));
	}

	// --- health_threshold only ---

	#[test]
	fn health_threshold_above_no_eviction() {
		let policy = policy_with_threshold(0.5);
		// current_health=0.7 > 0.5, should not evict
		let (_, eviction, _) = policy.eviction_decision(0.7, 0, 0, true, None);
		assert!(eviction.is_none());
	}

	#[test]
	fn health_threshold_at_boundary_no_eviction() {
		let policy = policy_with_threshold(0.5);
		// current_health=0.5 == 0.5, "is_some_and(|t| current_health < t)" is false
		let (_, eviction, _) = policy.eviction_decision(0.5, 0, 0, true, None);
		assert!(eviction.is_none());
	}

	#[test]
	fn health_threshold_below_evicts() {
		let policy = policy_with_threshold(0.5);
		// current_health=0.49 < 0.5
		let (_, eviction, _) = policy.eviction_decision(0.49, 0, 0, true, None);
		assert!(eviction.is_some());
	}

	// --- consecutive_failures only ---

	#[test]
	fn consecutive_failures_below_count_no_eviction() {
		let policy = policy_with_consecutive(3);
		// consecutive_failure_count=1, failures_including_current=2 < 3
		let (_, eviction, _) = policy.eviction_decision(1.0, 1, 0, true, None);
		assert!(eviction.is_none());
	}

	#[test]
	fn consecutive_failures_at_count_evicts() {
		let policy = policy_with_consecutive(3);
		// consecutive_failure_count=2, failures_including_current=3 >= 3
		let (_, eviction, _) = policy.eviction_decision(1.0, 2, 0, true, None);
		assert!(eviction.is_some());
	}

	#[test]
	fn consecutive_failures_above_count_evicts() {
		let policy = policy_with_consecutive(3);
		// consecutive_failure_count=5, failures_including_current=6 >= 3
		let (_, eviction, _) = policy.eviction_decision(1.0, 5, 0, true, None);
		assert!(eviction.is_some());
	}

	#[test]
	fn consecutive_failures_zero_threshold_never_triggers() {
		let policy = policy_with_consecutive(0);
		let (_, eviction, _) = policy.eviction_decision(1.0, 100, 0, true, None);
		assert!(eviction.is_none());
	}

	// --- both thresholds (OR logic) ---

	#[test]
	fn both_thresholds_health_below_triggers_eviction() {
		let policy = Policy {
			eviction: Some(Eviction {
				health_threshold: Some(0.5),
				consecutive_failures: Some(5),
				..Default::default()
			}),
			..Default::default()
		};
		// health 0.3 < 0.5, but consecutive_failures=0 (1 including current < 5)
		let (_, eviction, _) = policy.eviction_decision(0.3, 0, 0, true, None);
		assert!(eviction.is_some());
	}

	#[test]
	fn both_thresholds_consecutive_exceeded_triggers_eviction() {
		let policy = Policy {
			eviction: Some(Eviction {
				health_threshold: Some(0.5),
				consecutive_failures: Some(3),
				..Default::default()
			}),
			..Default::default()
		};
		// health 0.9 > 0.5, but consecutive=2 (3 including current >= 3)
		let (_, eviction, _) = policy.eviction_decision(0.9, 2, 0, true, None);
		assert!(eviction.is_some());
	}

	#[test]
	fn both_thresholds_neither_met_no_eviction() {
		let policy = Policy {
			eviction: Some(Eviction {
				health_threshold: Some(0.5),
				consecutive_failures: Some(5),
				..Default::default()
			}),
			..Default::default()
		};
		// health 0.7 > 0.5, consecutive=1 (2 including current < 5)
		let (_, eviction, _) = policy.eviction_decision(0.7, 1, 0, true, None);
		assert!(eviction.is_none());
	}

	// --- restore_health passthrough ---

	#[test]
	fn returns_restore_health() {
		let policy = Policy {
			eviction: Some(Eviction {
				restore_health: Some(0.5),
				..Default::default()
			}),
			..Default::default()
		};
		let (_, _, hon) = policy.eviction_decision(1.0, 0, 0, true, None);
		assert_eq!(hon, Some(0.5));
	}

	#[test]
	fn returns_restore_health_none_when_unset() {
		let policy = Policy::default();
		let (_, _, hon) = policy.eviction_decision(1.0, 0, 0, true, None);
		assert_eq!(hon, None);
	}

	// --- eviction duration computation ---

	#[test]
	fn explicit_eviction_duration_used() {
		let policy = policy_with_eviction_duration(60);
		let (_, eviction, _) = policy.eviction_decision(1.0, 0, 0, true, None);
		assert_eq!(eviction, Some(Duration::from_secs(60)));
	}

	#[test]
	fn fallback_duration_used_when_no_explicit() {
		let policy = Policy::default();
		let fallback = Some(Duration::from_secs(45));
		let (_, eviction, _) = policy.eviction_decision(1.0, 0, 0, true, fallback);
		assert_eq!(eviction, Some(Duration::from_secs(45)));
	}

	#[test]
	fn explicit_duration_preferred_over_fallback() {
		let policy = policy_with_eviction_duration(10);
		let fallback = Some(Duration::from_secs(45));
		let (_, eviction, _) = policy.eviction_decision(1.0, 0, 0, true, fallback);
		assert_eq!(eviction, Some(Duration::from_secs(10)));
	}

	#[test]
	fn multiplicative_backoff_with_times_ejected() {
		let policy = policy_with_eviction_duration(10);
		// times_ejected=2 → multiplier=3 → 10*3=30s
		let (_, eviction, _) = policy.eviction_decision(1.0, 0, 2, true, None);
		assert_eq!(eviction, Some(Duration::from_secs(30)));
	}

	#[test]
	fn eviction_duration_uncapped_backoff() {
		let policy = policy_with_eviction_duration(60);
		// times_ejected=4 → multiplier=5 → 60*5=300s (no cap)
		let (_, eviction, _) = policy.eviction_decision(1.0, 0, 4, true, None);
		assert_eq!(eviction, Some(Duration::from_secs(300)));
	}

	// --- EWMA health score simulation with restoreHealth ---

	#[test]
	fn ewma_simulation_three_failures_with_threshold() {
		let policy = Policy {
			eviction: Some(Eviction {
				duration: Some(Duration::from_secs(10)),
				health_threshold: Some(0.5),
				restore_health: Some(1.0),
				..Default::default()
			}),
			..Default::default()
		};

		const ALPHA: f64 = 0.3;

		// Simulate the EWMA progression: start at 1.0
		let mut health = 1.0;

		// Failure 1: health=1.0 > 0.5, no eviction
		let (_, eviction, _) = policy.eviction_decision(health, 0, 0, true, None);
		assert!(eviction.is_none(), "failure 1 should not evict");
		health = ALPHA * 0.0 + (1.0 - ALPHA) * health; // 0.7

		// Failure 2: health=0.7 > 0.5, no eviction
		let (_, eviction, _) = policy.eviction_decision(health, 1, 0, true, None);
		assert!(eviction.is_none(), "failure 2 should not evict");
		health = ALPHA * 0.0 + (1.0 - ALPHA) * health; // 0.49

		// Failure 3: health=0.49 < 0.5, eviction!
		let (_, eviction, hon) = policy.eviction_decision(health, 2, 0, true, None);
		assert_eq!(
			eviction,
			Some(Duration::from_secs(10)),
			"failure 3 should evict"
		);
		assert_eq!(hon, Some(1.0));
	}

	#[test]
	fn ewma_simulation_after_unevict_with_full_health() {
		let policy = Policy {
			eviction: Some(Eviction {
				duration: Some(Duration::from_secs(10)),
				health_threshold: Some(0.5),
				restore_health: Some(1.0),
				..Default::default()
			}),
			..Default::default()
		};

		const ALPHA: f64 = 0.3;

		// After uneviction with restoreHealth=1.0, health is reset to 1.0.
		// The endpoint gets a fresh start and needs 3 failures to re-evict.
		let mut health = 1.0;

		// Failure 1: 1.0 > 0.5
		let (_, eviction, _) = policy.eviction_decision(health, 0, 1, true, None);
		assert!(eviction.is_none());
		health = ALPHA * 0.0 + (1.0 - ALPHA) * health; // 0.7

		// Failure 2: 0.7 > 0.5
		let (_, eviction, _) = policy.eviction_decision(health, 1, 1, true, None);
		assert!(eviction.is_none());
		health = ALPHA * 0.0 + (1.0 - ALPHA) * health; // 0.49

		// Failure 3: 0.49 < 0.5, re-evicted
		let (_, eviction, _) = policy.eviction_decision(health, 2, 1, true, None);
		assert!(eviction.is_some(), "should re-evict after 3 failures");
		// Backoff: 10s * (times_ejected=1 + 1) = 20s
		assert_eq!(eviction, Some(Duration::from_secs(20)));
	}

	#[test]
	fn ewma_simulation_after_unevict_with_zero_health() {
		let policy = Policy {
			eviction: Some(Eviction {
				duration: Some(Duration::from_secs(10)),
				health_threshold: Some(0.5),
				restore_health: Some(0.0),
				..Default::default()
			}),
			..Default::default()
		};

		// After uneviction with restoreHealth=0.0, health is 0.0.
		// First failure: 0.0 < 0.5 → immediately re-evicted.
		let (_, eviction, _) = policy.eviction_decision(0.0, 0, 1, true, None);
		assert!(
			eviction.is_some(),
			"should immediately re-evict with health=0.0"
		);
	}

	#[test]
	fn consecutive_failures_not_reset_on_unevict() {
		let policy = Policy {
			eviction: Some(Eviction {
				duration: Some(Duration::from_secs(10)),
				consecutive_failures: Some(3),
				restore_health: Some(1.0),
				..Default::default()
			}),
			..Default::default()
		};

		// After uneviction, consecutive_failures counter is NOT reset (stays at 3).
		// Even with restoreHealth=1.0, the first failure after uneviction
		// sees failures_including_current=4 >= 3, so it's immediately re-evicted.
		let (_, eviction, _) = policy.eviction_decision(1.0, 3, 1, true, None);
		assert!(
			eviction.is_some(),
			"consecutive_failures=3 after uneviction → immediate re-eviction"
		);
	}
}
