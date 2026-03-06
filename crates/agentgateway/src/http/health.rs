//! Configurable backend health / eviction (outlier detection) policy.
//!
//! When a response is considered unhealthy (by CEL or default 5xx), the backend can be
//! evicted for a configurable duration. If no health policy is configured, no eviction
//! is applied. Optional health threshold and health-on-unevict support multi-request and
//! recovery behavior.

use std::sync::Arc;
use std::time::Duration;

use crate::cel::Expression;
use crate::{serde_dur_option, *};

/// Eviction sub-policy: how long to remove a backend from the active set after an unhealthy response.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Eviction {
	/// How long to evict the backend. When absent, falls back to `Retry-After` header (e.g. 429)
	/// or retry policy backoff, then a default (e.g. 30s).
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	pub duration: Option<Duration>,

	/// Number of consecutive failures required before evicting the backend.
	/// When absent or zero, a single unhealthy response can trigger eviction.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub threshold: Option<i32>,

	/// When true, an evicted backend that returns a healthy response is immediately marked healthy.
	#[serde(default)]
	pub health_on_return: bool,
}

/// Health policy: determines when a backend is unhealthy and how to evict it.
///
/// Maps to the proto `Health` message containing an `unhealthy_condition` CEL expression
/// and an optional `Eviction` sub-message with the eviction duration.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
	/// CEL expression evaluated per response; `true` means this response is unhealthy (evict).
	/// When absent, default is to treat 5xx (and missing response) as unhealthy, but only
	/// when a health policy is configured.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub unhealthy_expression: Option<Arc<Expression>>,

	/// Eviction settings (duration). When absent, falls back to defaults.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub eviction: Option<Eviction>,

	/// Evict only when endpoint health (EWMA) is below this threshold (0.0–1.0).
	/// When absent, eviction is driven only by the per-response unhealthy signal.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub health_threshold: Option<f64>,

	/// Health score to set when the endpoint is unevicted (e.g. 0.2 to give it a chance to recover).
	/// When absent, health is left unchanged on unevict.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub health_on_unevict: Option<f64>,
}

impl Policy {
	/// Returns the configured eviction duration, if any.
	pub fn eviction_duration(&self) -> Option<Duration> {
		self.eviction.as_ref().and_then(|e| e.duration)
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
	pub threshold: Option<i32>,
	#[serde(default)]
	pub health_on_return: bool,
}

/// Local/config health policy with CEL as string; converted to Policy by compiling the expression.
/// Mirrors the proto `Health` message structure.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalHealthPolicy {
	/// CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub unhealthy_expression: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub eviction: Option<LocalEviction>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub health_threshold: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub health_on_unevict: Option<f64>,
}

impl TryFrom<LocalHealthPolicy> for Policy {
	type Error = crate::cel::Error;
	fn try_from(local: LocalHealthPolicy) -> Result<Self, Self::Error> {
		let validate_score = |field: &str, value: Option<f64>| -> Result<(), crate::cel::Error> {
			if let Some(v) = value
				&& !(0.0..=1.0).contains(&v)
			{
				return Err(crate::cel::Error::Variable(format!(
					"health.{field} must be between 0.0 and 1.0"
				)));
			}
			Ok(())
		};
		validate_score("healthThreshold", local.health_threshold)?;
		validate_score("healthOnUnevict", local.health_on_unevict)?;

		let unhealthy_expression = match local.unhealthy_expression {
			Some(s) if !s.trim().is_empty() => Some(Arc::new(Expression::new_strict(&s)?)),
			_ => None,
		};
		let eviction = local.eviction.map(|e| Eviction {
			duration: e.duration,
			threshold: e.threshold,
			health_on_return: e.health_on_return,
		});
		Ok(Policy {
			unhealthy_expression,
			eviction,
			health_threshold: local.health_threshold,
			health_on_unevict: local.health_on_unevict,
		})
	}
}
