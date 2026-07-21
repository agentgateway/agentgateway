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
}

impl HasExpressions for Policy {
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::empty()
	}
}
