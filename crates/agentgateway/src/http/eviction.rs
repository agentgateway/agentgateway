//! Configurable backend eviction (outlier detection) policy.
//!
//! When a response is considered unhealthy (by CEL or default 5xx), the backend can be
//! evicted for a configurable duration. Optional health threshold and health-on-unevict
//! support multi-request and recovery behavior.

use std::sync::Arc;
use std::time::Duration;

use crate::cel::Expression;
use crate::{serde_dur_option, *};

/// Policy for when and how long to evict a backend (e.g. AI provider) on unhealthy responses.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
	/// CEL expression evaluated per response; `true` means this response is unhealthy (evict).
	/// When absent, default is to treat 5xx (and missing response) as unhealthy.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub unhealthy_expression: Option<Arc<Expression>>,

	/// How long to evict the backend. When absent, falls back to `Retry-After` header (e.g. 429)
	/// or retry policy backoff, then a default (e.g. 30s).
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	pub eviction_duration: Option<Duration>,

	/// Evict only when endpoint health (EWMA) is below this threshold (0.0–1.0).
	/// When absent, eviction is driven only by the per-response unhealthy signal.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub health_threshold: Option<f64>,

	/// Health score to set when the endpoint is unevicted (e.g. 0.2 to give it a chance to recover).
	/// When absent, health is left unchanged on unevict.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub health_on_unevict: Option<f64>,
}

/// Local/config eviction policy with CEL as string; converted to Policy by compiling the expression.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalPolicy {
	/// CEL expression; `true` means unhealthy (evict). E.g. `response.status >= 500`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub unhealthy_expression: Option<String>,
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub eviction_duration: Option<Duration>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub health_threshold: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub health_on_unevict: Option<f64>,
}

impl TryFrom<LocalPolicy> for Policy {
	type Error = crate::cel::Error;
	fn try_from(local: LocalPolicy) -> Result<Self, Self::Error> {
		let unhealthy_expression = match local.unhealthy_expression {
			Some(s) if !s.trim().is_empty() => Some(Arc::new(Expression::new_strict(&s)?)),
			_ => None,
		};
		Ok(Policy {
			unhealthy_expression,
			eviction_duration: local.eviction_duration,
			health_threshold: local.health_threshold,
			health_on_unevict: local.health_on_unevict,
		})
	}
}
