use std::sync::Arc;
use std::time::Duration;

use crate::cel::Expression;
use crate::store::HasExpressions;
use crate::*;

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "DelayPolicy"))]
pub struct Policy {
	/// Artificial latency injected before the request is forwarded to the backend.
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub duration: Duration,
	/// Probability the delay is injected. This should evaluate to either a float between 0.0-1.0
	/// or true/false, and may be a CEL expression (e.g. `request.headers["x-chaos"] == "1"`).
	/// This defaults to 'true'.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		deserialize_with = "de_probability"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<StringBoolFloat>"))]
	pub probability: Option<Arc<Expression>>,
}

impl Policy {
	/// Rolls the probability gate for this request.
	pub fn should_inject(&self, req: &crate::http::Request) -> bool {
		match &self.probability {
			Some(p) => cel::Executor::new_request(req).eval_rng(p),
			None => true,
		}
	}
}

impl HasExpressions for Policy {
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.probability.iter().map(|e| e.as_ref())
	}
}

fn de_probability<'de, D>(deserializer: D) -> Result<Option<Arc<Expression>>, D::Error>
where
	D: Deserializer<'de>,
{
	Option::<StringBoolFloat>::deserialize(deserializer)?
		.map(|raw| {
			Expression::new_strict(&raw.0)
				.map(Arc::new)
				.map_err(serde::de::Error::custom)
		})
		.transpose()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_probability_forms() {
		let pol: Policy = serde_json::from_value(serde_json::json!({
			"duration": "2s",
			"probability": 0.1,
		}))
		.expect("numeric probability");
		assert_eq!(pol.duration, Duration::from_secs(2));
		assert!(pol.probability.is_some());

		let pol: Policy = serde_json::from_value(serde_json::json!({
			"duration": "500ms",
			"probability": "request.headers[\"x-chaos\"] == \"1\"",
		}))
		.expect("CEL probability");
		assert!(pol.probability.is_some());

		let pol: Policy =
			serde_json::from_value(serde_json::json!({"duration": "1s"})).expect("no probability");
		assert!(pol.probability.is_none());

		serde_json::from_value::<Policy>(serde_json::json!({
			"duration": "1s",
			"probability": "not a ( valid expression",
		}))
		.expect_err("invalid CEL must be rejected at config load");
	}
}
