use agent_http::{HeaderMap, Method, Uri};
use agent_policy::PolicyCall;
use agent_policy::testing::{RecordedBackendCall, TestBackend, TestEnvironment, TestRequest};
use serde_json::json;

use super::*;

type TestAdvanced = Advanced<TestBackend>;

#[tokio::test]
async fn evaluates_cel_traces_and_calls_backend() {
	let req = TestRequest::get("/hello");
	let result = TestEnvironment::run_with_vars::<TestAdvanced>(
		json!({
			"condition": "request.path == \"/hello\" && claims.role.lowerAscii() == \"admin\"",
			"backend": "authz",
		}),
		req,
		json!({
			"request": {
				"path": "/hello",
			},
			"claims": {
				"role": "ADMIN",
			},
		}),
	)
	.await
	.unwrap();

	assert_eq!(
		result.backend_calls(),
		&[RecordedBackendCall {
			call: PolicyCall::ExtAuthz,
			method: Method::POST,
			uri: Uri::from_static("/check"),
			headers: HeaderMap::new(),
		}]
	);
}

#[tokio::test]
async fn skips_when_disabled() {
	let req = TestRequest::get("/hello");
	let result = TestEnvironment::run::<TestAdvanced>(
		json!({
			"condition": "false",
			"backend": "authz",
		}),
		req,
	)
	.await
	.unwrap();

	assert_eq!(result.backend_calls(), &[],);
}
