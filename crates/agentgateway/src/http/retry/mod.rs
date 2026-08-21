mod body;

use std::num::NonZeroU8;
use std::sync::Arc;

use agent_core::serdes::BackoffSpec;

pub use body::ReplayBody;

use crate::cel::Expression;
use crate::store::HasExpressions;
use crate::*;

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "RetryPolicy"))]
pub struct Policy {
	/// Total number of attempts, including the original request.
	#[serde(default = "default_attempts")]
	pub attempts: NonZeroU8,
	/// Delay between retry attempts.
	///
	/// Can be either:
	/// - A single duration string (e.g. `"5s"`) — the same delay is used for every retry.
	/// - A list of duration strings (e.g. `["2s", "4s", "8s", "16s", "32s"]`) —
	///   the delay cycles through the list on successive retries, restarting
	///   from the beginning when the list is exhausted.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "agent_core::serdes::serde_backoff_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub backoff: Option<BackoffSpec>,
	/// HTTP response status codes that should be retried.
	#[serde(serialize_with = "ser_display_iter", deserialize_with = "de_codes")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<std::num::NonZeroU16>"))]
	pub codes: Box<[http::StatusCode]>,
	/// CEL expression evaluated against the request before any attempt; when `false`,
	/// retries are disabled (only the initial attempt is made), e.g. `request.method == "GET"`.
	/// Retrying requires buffering the request body in memory for replay, so this lets us skip
	/// that cost when the request is known to be non-retriable (e.g. streaming or websockets).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub precondition: Option<Arc<Expression>>,
	/// CEL expression evaluated against each response to decide whether to retry. A response
	/// is retried when its status code is in `codes` *or* this expression evaluates to `true`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub condition: Option<Arc<Expression>>,
}

impl HasExpressions for Policy {
	/// Exposes the precondition/condition expressions so the proxy snapshots the
	/// request/response attributes they reference.
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self
			.precondition
			.iter()
			.chain(self.condition.iter())
			.map(|e| e.as_ref())
	}
}

pub fn de_codes<'de: 'a, 'a, D>(deserializer: D) -> Result<Box<[http::StatusCode]>, D::Error>
where
	D: Deserializer<'de>,
{
	let raw = Vec::<u16>::deserialize(deserializer)?;
	let boxed = raw
		.into_iter()
		.map(|c| http::StatusCode::from_u16(c).map_err(serde::de::Error::custom))
		.collect::<Result<Vec<_>, _>>()?;
	Ok(boxed.into_boxed_slice())
}
fn default_attempts() -> NonZeroU8 {
	NonZeroU8::new(1).unwrap()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_pre_and_post_conditions() {
		let pol: Policy = serde_json::from_value(serde_json::json!({
			"attempts": 3,
			"codes": [503],
			"precondition": "request.method == \"GET\"",
			"condition": "response.headers[\"x-req-failed\"] != \"\"",
		}))
		.unwrap();
		assert_eq!(pol.attempts.get(), 3);
		assert_eq!(
			pol.precondition.as_ref().unwrap().original_expression,
			"request.method == \"GET\""
		);
		assert_eq!(
			pol.condition.as_ref().unwrap().original_expression,
			"response.headers[\"x-req-failed\"] != \"\""
		);
	}

	#[test]
	fn conditions_default_to_none() {
		let pol: Policy = serde_json::from_value(serde_json::json!({
			"attempts": 1,
			"codes": [500],
		}))
		.unwrap();
		assert!(pol.precondition.is_none());
		assert!(pol.condition.is_none());
	}

	#[test]
	fn expressions_exposes_both_conditions() {
		let pol: Policy = serde_json::from_value(serde_json::json!({
			"attempts": 2,
			"codes": [],
			"precondition": "request.method == \"GET\"",
			"condition": "response.code == 200",
		}))
		.unwrap();
		assert_eq!(pol.expressions().count(), 2);
	}

	#[test]
	fn parses_fixed_backoff() {
		let pol: Policy = serde_json::from_value(serde_json::json!({
			"attempts": 3,
			"backoff": "5s",
			"codes": [503],
		}))
		.unwrap();
		assert_eq!(pol.attempts.get(), 3);
		let backoff = pol.backoff.expect("backoff should be set");
		assert_eq!(
			backoff.delay_for_attempt(0),
			std::time::Duration::from_secs(5)
		);
		assert_eq!(
			backoff.delay_for_attempt(1),
			std::time::Duration::from_secs(5)
		);
		assert_eq!(
			backoff.delay_for_attempt(10),
			std::time::Duration::from_secs(5)
		);
	}

	#[test]
	fn parses_sequence_backoff() {
		let pol: Policy = serde_json::from_value(serde_json::json!({
			"attempts": 10,
			"backoff": ["2s", "4s", "8s", "16s", "32s"],
			"codes": [429, 503],
		}))
		.unwrap();
		assert_eq!(pol.attempts.get(), 10);
		let backoff = pol.backoff.expect("backoff should be set");
		// Verify the cycling pattern
		assert_eq!(backoff.delay_for_attempt(0), std::time::Duration::from_secs(2));
		assert_eq!(backoff.delay_for_attempt(1), std::time::Duration::from_secs(4));
		assert_eq!(backoff.delay_for_attempt(2), std::time::Duration::from_secs(8));
		assert_eq!(backoff.delay_for_attempt(3), std::time::Duration::from_secs(16));
		assert_eq!(backoff.delay_for_attempt(4), std::time::Duration::from_secs(32));
		// Cycle restarts
		assert_eq!(backoff.delay_for_attempt(5), std::time::Duration::from_secs(2));
		assert_eq!(backoff.delay_for_attempt(6), std::time::Duration::from_secs(4));
	}

	#[test]
	fn no_backoff_defaults_to_none() {
		let pol: Policy = serde_json::from_value(serde_json::json!({
			"attempts": 3,
			"codes": [503],
		}))
		.unwrap();
		assert!(pol.backoff.is_none());
	}
}
