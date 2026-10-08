//! In-process test harness for policies.
//!
//! Enable the `testing` feature in policy unit tests to run policies with real CEL
//! evaluation and a recording mock backend. `request` and `response` CEL variables
//! are derived from the mock request and response; other variables come from
//! [`PolicyTest::vars`]. Extra fields passed under `request` or `response` are merged in,
//! with the mock message's fields taking precedence:
//!
//! ```no_run
//! # use agent_policy::RequestPolicy;
//! # use serde::de::DeserializeOwned;
//! # async fn test_policy<P>()
//! # where P: RequestPolicy + DeserializeOwned {
//! use agent_policy::testing::{PolicyTest, TestRequest, TestResponse};
//!
//! let mut test = PolicyTest::<P>::new(serde_json::json!({ "condition": "jwt.sub == \"me\"" }))
//!     .vars(serde_json::json!({ "jwt": { "sub": "me" } }));
//! let response = test.request(TestRequest::get("/hello")).await;
//! assert!(!response.should_short_circuit());
//! test.response(TestResponse::default()).await;
//! # }
//! ```

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use agent_http::{
	Body, HeaderMap, HeaderName, HeaderValue, Method, PolicyResponse, Request, Response, StatusCode,
	Uri,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::{
	BackendDispatcher, BackendReferenceClient, Error, Expression, OutboundCallSubtype, PolicyCel,
	PolicyContext, RequestPolicy,
};

/// Runs one policy through its request and response phases against a mock host.
///
/// Phase failures panic, since this is only used in tests.
pub struct PolicyTest<P: RequestPolicy> {
	policy: P,
	cel: JsonCel,
	backend: RecordingBackend,
	request: Option<Request>,
	response: Option<Response>,
	request_result: PolicyResponse,
	response_result: PolicyResponse,
	response_state: Option<P::ResponseState>,
}

impl<P: RequestPolicy + DeserializeOwned> PolicyTest<P> {
	/// Deserializes the policy from its JSON configuration.
	pub fn new(config: serde_json::Value) -> Self {
		let policy = serde_json::from_value(config)
			.unwrap_or_else(|err| panic!("policy config failed to deserialize: {err}"));
		Self::from_policy(policy)
	}
}

impl<P: RequestPolicy> PolicyTest<P> {
	pub fn from_policy(policy: P) -> Self {
		agent_core::telemetry::testing::setup_test_logging();
		Self {
			policy,
			cel: JsonCel::new(serde_json::json!({})).expect("empty CEL variables must be valid"),
			backend: RecordingBackend::default(),
			request: None,
			response: None,
			request_result: PolicyResponse::default(),
			response_result: PolicyResponse::default(),
			response_state: None,
		}
	}

	/// Exposes the top-level fields of a JSON object as CEL variables.
	pub fn vars(mut self, variables: serde_json::Value) -> Self {
		self.cel =
			JsonCel::new(variables).unwrap_or_else(|err| panic!("invalid test CEL variables: {err}"));
		self
	}

	/// Runs the request phase, retaining any response state for [`Self::response`].
	pub async fn request(&mut self, request: TestRequest) -> &PolicyResponse {
		let mut request = request.into_request();
		let context = PolicyContext::new(&self.cel).with_backend(self.backend.dispatcher());
		let action = self
			.policy
			.apply(context, &mut request)
			.await
			.unwrap_or_else(|err| panic!("request phase failed: {err}"));
		self.request = Some(request);
		self.response_state = action.response_state;
		self.request_result = action.response;
		&self.request_result
	}

	/// Runs the response phase with the state returned by [`Self::request`].
	pub async fn response(&mut self, response: TestResponse) -> &PolicyResponse {
		let state = self
			.response_state
			.take()
			.expect("request phase did not return response state");
		let mut response = response.into_response();
		let context = PolicyContext::new(&self.cel).with_backend(self.backend.dispatcher());
		self.response_result = self
			.policy
			.apply_response(context, state, &mut response)
			.await
			.unwrap_or_else(|err| panic!("response phase failed: {err}"));
		self.response = Some(response);
		&self.response_result
	}

	pub fn request_headers(&self) -> &HeaderMap {
		self
			.request
			.as_ref()
			.expect("request phase has not run")
			.headers()
	}

	pub fn response_headers(&self) -> &HeaderMap {
		self
			.response
			.as_ref()
			.expect("response phase has not run")
			.headers()
	}

	pub fn backend_calls(&self) -> Vec<RecordedBackendCall> {
		self.backend.calls()
	}
}

/// Minimal backend configuration for policies that deserialize a backend target.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestBackend {
	pub backend: String,
}

/// Mock request supplied to [`PolicyTest::request`].
#[derive(Debug, Clone)]
pub struct TestRequest {
	method: Method,
	uri: Uri,
	headers: Vec<(HeaderName, HeaderValue)>,
	body: Vec<u8>,
}

impl Default for TestRequest {
	fn default() -> Self {
		Self::get("/")
	}
}

impl TestRequest {
	pub fn get(path: impl AsRef<str>) -> Self {
		Self::new(Method::GET, path)
	}

	pub fn new(method: Method, path: impl AsRef<str>) -> Self {
		Self {
			method,
			uri: path
				.as_ref()
				.parse()
				.unwrap_or_else(|err| panic!("invalid test request URI {:?}: {err}", path.as_ref())),
			headers: Vec::new(),
			body: Vec::new(),
		}
	}

	pub fn uri(mut self, uri: Uri) -> Self {
		self.uri = uri;
		self
	}

	pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
		self.headers.push((name, value));
		self
	}

	pub fn body(mut self, body: impl AsRef<[u8]>) -> Self {
		self.body = body.as_ref().to_vec();
		self
	}

	fn into_request(self) -> Request {
		let mut request = Request::new(Body::from(self.body));
		*request.method_mut() = self.method;
		*request.uri_mut() = self.uri;
		for (name, value) in self.headers {
			request.headers_mut().append(name, value);
		}
		request
	}
}

/// Mock response supplied to [`PolicyTest::response`].
#[derive(Debug, Clone)]
pub struct TestResponse {
	status: StatusCode,
	headers: Vec<(HeaderName, HeaderValue)>,
	body: Vec<u8>,
}

impl Default for TestResponse {
	fn default() -> Self {
		Self::new(StatusCode::OK)
	}
}

impl TestResponse {
	pub fn new(status: StatusCode) -> Self {
		Self {
			status,
			headers: Vec::new(),
			body: Vec::new(),
		}
	}

	pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
		self.headers.push((name, value));
		self
	}

	pub fn body(mut self, body: impl AsRef<[u8]>) -> Self {
		self.body = body.as_ref().to_vec();
		self
	}

	fn into_response(self) -> Response {
		let mut response = Response::new(Body::from(self.body));
		*response.status_mut() = self.status;
		for (name, value) in self.headers {
			response.headers_mut().append(name, value);
		}
		response
	}
}

/// CEL evaluator backed by top-level variables from a JSON object, plus `request` or
/// `response` derived from the message being evaluated.
struct JsonCel {
	context: cel::Context,
	variables: serde_json::Map<String, serde_json::Value>,
}

impl JsonCel {
	fn new(variables: serde_json::Value) -> Result<Self, Error> {
		let serde_json::Value::Object(variables) = variables else {
			return Err(Error::Variable(
				"test CEL variables must be a JSON object".to_owned(),
			));
		};
		let mut context = cel::Context::default();
		agent_celx::insert_all(&mut context);
		Ok(Self { context, variables })
	}

	fn eval(
		&self,
		expression: &Expression,
		message: (&str, serde_json::Value),
	) -> Result<cel::Value<'static>, Error> {
		let mut variables = self.variables.clone();
		// Fields from the mock message win; other user-supplied fields under the same name are kept.
		let (name, derived) = message;
		let entry = variables
			.entry(name)
			.or_insert_with(|| serde_json::Value::Object(Default::default()));
		match (entry, derived) {
			(serde_json::Value::Object(user), serde_json::Value::Object(derived)) => user.extend(derived),
			(entry, derived) => *entry = derived,
		}
		let mut resolver = cel::context::MapResolver::new();
		for (name, value) in &variables {
			let value =
				cel::to_value(value.clone()).map_err(|error| Error::Variable(error.to_string()))?;
			resolver.add_variable_from_value(name, value);
		}
		cel::Value::resolve(expression.ast(), &self.context, &resolver)
			.map(|value| value.as_static())
			.map_err(Error::from)
	}
}

impl PolicyCel for JsonCel {
	fn eval_request<'a>(
		&'a self,
		expression: &'a Expression,
		request: &'a Request,
	) -> Result<cel::Value<'a>, Error> {
		let request = serde_json::json!({
			"method": request.method().as_str(),
			"uri": request.uri().to_string(),
			"path": request.uri().path(),
			"headers": json_headers(request.headers()),
		});
		self.eval(expression, ("request", request))
	}

	fn eval_response<'a>(
		&'a self,
		expression: &'a Expression,
		response: &'a Response,
	) -> Result<cel::Value<'a>, Error> {
		let response = serde_json::json!({
			"code": response.status().as_u16(),
			"headers": json_headers(response.headers()),
		});
		self.eval(expression, ("response", response))
	}
}

fn json_headers(headers: &HeaderMap) -> serde_json::Map<String, serde_json::Value> {
	headers
		.keys()
		.map(|name| {
			let values = headers
				.get_all(name)
				.iter()
				.filter_map(|value| value.to_str().ok())
				.collect::<Vec<_>>();
			(name.to_string(), values.join(",").into())
		})
		.collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordedBackendCall {
	pub call: OutboundCallSubtype,
	pub method: Method,
	pub uri: Uri,
	pub headers: HeaderMap,
}

#[derive(Clone, Default)]
struct RecordingBackend {
	call: Option<OutboundCallSubtype>,
	calls: Arc<Mutex<Vec<RecordedBackendCall>>>,
}

impl RecordingBackend {
	fn dispatcher(&self) -> BackendDispatcher {
		BackendDispatcher::new(self.clone(), Arc::new(()), Arc::new(Vec::<()>::new()))
	}

	fn calls(&self) -> Vec<RecordedBackendCall> {
		self
			.calls
			.lock()
			.expect("recording backend lock poisoned")
			.clone()
	}
}

impl BackendReferenceClient<(), ()> for RecordingBackend {
	type Error = Infallible;

	fn with_policy_call(&self, call: OutboundCallSubtype) -> Self {
		Self {
			call: Some(call),
			calls: self.calls.clone(),
		}
	}

	async fn call_reference_with_policies(
		&self,
		req: Request,
		_backend_ref: &(),
		_policies: &[()],
	) -> Result<Response, Self::Error> {
		self
			.calls
			.lock()
			.expect("recording backend lock poisoned")
			.push(RecordedBackendCall {
				call: self.call.expect("policy call must be set before dispatch"),
				method: req.method().clone(),
				uri: req.uri().clone(),
				headers: req.headers().clone(),
			});
		Ok(Response::new(Body::empty()))
	}
}
