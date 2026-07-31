use agent_http::{HeaderValue, PolicyResponse, Request, header};
use agent_policy::{BoxError, PolicyContext, RequestPolicy};

#[derive(Debug, Default)]
pub struct Simple;

impl RequestPolicy for Simple {
	async fn apply(
		&self,
		_ctx: PolicyContext<'_>,
		req: &mut Request,
	) -> Result<PolicyResponse, BoxError> {
		req.headers_mut().insert(
			header::HeaderName::from_static("x-hello-world"),
			HeaderValue::from_static("hello"),
		);
		Ok(PolicyResponse::default())
	}
}

#[cfg(test)]
mod tests {
	use agent_policy::testing::{TestEnvironment, TestRequest};

	use super::*;

	#[tokio::test]
	async fn inserts_hello_world_header() {
		let mut env = TestEnvironment::default();

		let _ = env
			.run_request(&Simple, TestRequest::get("/hello"))
			.await
			.unwrap();

		assert_eq!(
			env.request_headers()["x-hello-world"],
			HeaderValue::from_static("hello")
		);
	}
}
