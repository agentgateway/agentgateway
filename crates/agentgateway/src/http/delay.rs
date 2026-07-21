use std::time::Duration;

use crate::http::filters::RequestDeadline;
use crate::*;

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "DelayPolicy"))]
pub struct Policy {
	/// Artificial latency injected before the request is forwarded to the backend.
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub duration: Duration,
}

impl crate::store::RequestPolicyTrait for Policy {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		let sleep = tokio::time::sleep(self.duration);
		match req.extensions().get::<RequestDeadline>() {
			// delay is counted against request timeout, mimicks real latency
			Some(RequestDeadline(deadline)) => {
				tokio::time::timeout_at(tokio::time::Instant::from_std(*deadline), sleep)
					.await
					.map_err(|_| crate::proxy::ProxyError::RequestTimeout)?;
			},
			None => sleep.await,
		}
		Ok(Default::default())
	}
}
