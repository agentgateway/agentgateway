use std::sync::Arc;
use std::time::Duration;

use crate::cel::{Executor, Expression, Value};
use crate::http::filters::RequestDeadline;
use crate::*;

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "DelayPolicy"))]
pub struct Policy {
	/// Artificial latency injected before the request is forwarded to the backend. Either a duration
	/// string such as `2s`, or a CEL expression evaluated against the request that returns a duration
	/// (e.g. `duration("500ms")`) or a number interpreted as milliseconds (e.g.
	/// `random() < 0.1 ? 500 : 0` for probabilistic delay, or `int(random() * 500)` for jitter). A
	/// non-positive result injects no delay.
	#[serde(deserialize_with = "crate::cel::de_duration_or_expression")]
	pub duration: Arc<Expression>,
}

impl Policy {
	/// Evaluates the duration expression against the request, returning the delay to inject, or `None`
	/// when the result is non-positive (or neither a duration nor a number).
	fn eval_duration(&self, req: &crate::http::Request) -> Option<Duration> {
		let exec = Executor::new_request(req);
		let ms = match exec.eval(&self.duration).ok()? {
			// A duration value is used directly; a number is interpreted as milliseconds.
			Value::Duration(d) => return d.to_std().ok().filter(|d| !d.is_zero()),
			Value::Int(v) => v as f64,
			Value::UInt(v) => v as f64,
			Value::Float(v) => v,
			_ => return None,
		};
		(ms > 0.0).then(|| Duration::from_secs_f64(ms / 1000.0))
	}
}

impl crate::store::RequestPolicyTrait for Policy {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		let Some(duration) = self.eval_duration(req) else {
			return Ok(Default::default());
		};
		let sleep = tokio::time::sleep(duration);
		match req.extensions().get::<RequestDeadline>() {
			// delay is counted against request timeout, mimics real latency
			Some(RequestDeadline(deadline)) => {
				tokio::time::timeout_at(tokio::time::Instant::from_std(*deadline), sleep)
					.await
					.map_err(|_| crate::proxy::ProxyError::RequestTimeout)?;
			},
			None => sleep.await,
		}
		Ok(Default::default())
	}

	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::once(self.duration.as_ref())
	}
}
