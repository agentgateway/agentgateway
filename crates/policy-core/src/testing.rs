//! In-process test harness for policies.
//!
//! Enable the `testing` feature in policy unit tests to run policies with real CEL
//! evaluation over JSON variables and mock backend dispatch:
//!
//! ```no_run
//! # use agent_policy::{BoxError, RequestPolicy};
//! # use serde::de::DeserializeOwned;
//! # async fn test_policy<P>() -> Result<(), BoxError>
//! # where P: RequestPolicy + DeserializeOwned {
//! use agent_policy::testing::{TestEnvironment, TestRequest};
//!
//! let run = TestEnvironment::run_with_vars::<P>(
//!     serde_json::json!({}),
//!     TestRequest::get("/hello"),
//!     serde_json::json!({
//!         "request": { "path": "/hello" }
//!     }),
//! ).await?;
//! assert!(!run.policy_response.should_short_circuit());
//! # Ok(())
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
	BackendDispatcher, BackendPolicy, BackendReferenceClient, BoxError, Error, Expression,
	PolicyCall, PolicyCel, PolicyContext, RequestPolicy, ResponsePolicy,
};

/// Mock host used to run policy callbacks in unit tests.
pub struct TestEnvironment {
	cel: TestCel,
	backend: RecordingBackend,
	request: Option<Request>,
	response: Option<Response>,
}

impl Default for TestEnvironment {
	fn default() -> Self {
		Self::from_cel(TestCel::Json(JsonCel::default()))
	}
}

impl TestEnvironment {
	/// Creates a reusable environment with a fixed CEL result.
	pub fn new(cel: FixedCel) -> Self {
		Self::from_cel(TestCel::Fixed(cel))
	}

	/// Creates a reusable environment with JSON-backed CEL variables.
	pub fn with_vars(variables: serde_json::Value) -> Result<Self, Error> {
		Ok(Self::from_cel(TestCel::Json(JsonCel::new(variables)?)))
	}

	fn from_cel(cel: TestCel) -> Self {
		agent_core::telemetry::testing::setup_test_logging();
		Self {
			cel,
			backend: RecordingBackend::default(),
			request: None,
			response: None,
		}
	}

	/// Deserializes and runs a request policy with real CEL evaluation and no variables.
	pub async fn run<P>(config: serde_json::Value, request: TestRequest) -> Result<TestRun, BoxError>
	where
		P: RequestPolicy + DeserializeOwned,
	{
		Self::run_with_vars::<P>(config, request, serde_json::json!({})).await
	}

	/// Deserializes and runs a request policy with top-level JSON fields exposed as CEL variables.
	pub async fn run_with_vars<P>(
		config: serde_json::Value,
		request: TestRequest,
		variables: serde_json::Value,
	) -> Result<TestRun, BoxError>
	where
		P: RequestPolicy + DeserializeOwned,
	{
		Self::run_with_evaluator::<P>(config, request, TestCel::Json(JsonCel::new(variables)?)).await
	}

	/// Deserializes and runs a request policy with a caller-supplied CEL stub.
	pub async fn run_with_cel<P>(
		config: serde_json::Value,
		request: TestRequest,
		cel: FixedCel,
	) -> Result<TestRun, BoxError>
	where
		P: RequestPolicy + DeserializeOwned,
	{
		Self::run_with_evaluator::<P>(config, request, TestCel::Fixed(cel)).await
	}

	async fn run_with_evaluator<P>(
		config: serde_json::Value,
		request: TestRequest,
		cel: TestCel,
	) -> Result<TestRun, BoxError>
	where
		P: RequestPolicy + DeserializeOwned,
	{
		let policy: P = serde_json::from_value(config)?;
		let mut environment = Self::from_cel(cel);
		let policy_response = environment.run_request(&policy, request).await?;

		Ok(TestRun {
			policy_response,
			request: environment
				.request
				.take()
				.expect("run_request always records its request"),
			backend_calls: environment.backend.calls(),
		})
	}

	/// Runs a request policy with the supplied mock request.
	pub async fn run_request<P>(
		&mut self,
		policy: &P,
		request: TestRequest,
	) -> Result<PolicyResponse, BoxError>
	where
		P: RequestPolicy + ?Sized,
	{
		self.backend.clear();
		let mut request = request.into_request();
		let context = PolicyContext::new(&self.cel).with_backend(self.backend.dispatcher());
		let result = policy.apply(context, &mut request).await;
		self.request = Some(request);
		result
	}

	/// Runs a response policy with the supplied mock response.
	pub async fn run_response<P>(
		&mut self,
		policy: &P,
		response: TestResponse,
	) -> Result<PolicyResponse, BoxError>
	where
		P: ResponsePolicy + ?Sized,
	{
		self.backend.clear();
		let mut response = response.into_response();
		let context = PolicyContext::new(&self.cel).with_backend(self.backend.dispatcher());
		let result = policy.apply_response(context, &mut response).await;
		self.response = Some(response);
		result
	}

	/// Runs a backend policy with the supplied mock request.
	pub async fn run_backend<P>(
		&mut self,
		policy: &P,
		request: TestRequest,
	) -> Result<PolicyResponse, BoxError>
	where
		P: BackendPolicy + ?Sized,
	{
		self.backend.clear();
		let mut request = request.into_request();
		let context = PolicyContext::new(&self.cel).with_backend(self.backend.dispatcher());
		let result = policy.apply_backend(context, &mut request).await;
		self.request = Some(request);
		result
	}

	pub fn request(&self) -> Option<&Request> {
		self.request.as_ref()
	}

	pub fn response(&self) -> Option<&Response> {
		self.response.as_ref()
	}

	pub fn request_headers(&self) -> HeaderMap {
		self
			.request
			.as_ref()
			.map(|request| request.headers().clone())
			.unwrap_or_default()
	}

	pub fn response_headers(&self) -> HeaderMap {
		self
			.response
			.as_ref()
			.map(|response| response.headers().clone())
			.unwrap_or_default()
	}

	pub fn response_status(&self) -> Option<StatusCode> {
		self.response.as_ref().map(Response::status)
	}

	pub fn backend_calls(&self) -> Vec<RecordedBackendCall> {
		self.backend.calls()
	}
}

/// State captured by a one-shot [`TestEnvironment::run`].
pub struct TestRun {
	pub policy_response: PolicyResponse,
	request: Request,
	backend_calls: Vec<RecordedBackendCall>,
}

impl TestRun {
	pub fn request(&self) -> &Request {
		&self.request
	}

	pub fn request_headers(&self) -> &HeaderMap {
		self.request.headers()
	}

	pub fn backend_calls(&self) -> &[RecordedBackendCall] {
		&self.backend_calls
	}
}

/// Minimal backend configuration for policies that deserialize a backend target.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestBackend {
	pub backend: String,
}

/// Mock request supplied to [`TestEnvironment::run_request`] or
/// [`TestEnvironment::run_backend`].
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
				.unwrap_or_else(|_| Uri::from_static("/")),
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

/// Mock response supplied to [`TestEnvironment::run_response`].
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

pub struct FixedCel {
	value: cel::Value<'static>,
}

impl FixedCel {
	pub fn new(value: cel::Value<'static>) -> Self {
		Self { value }
	}

	pub fn bool(value: bool) -> Self {
		Self::new(cel::Value::Bool(value))
	}
}

/// CEL evaluator backed by top-level variables from a JSON object.
struct JsonCel {
	context: cel::Context,
	variables: Vec<(String, cel::Value<'static>)>,
}

impl Default for JsonCel {
	fn default() -> Self {
		Self::new(serde_json::json!({})).expect("empty CEL variables must be valid")
	}
}

impl JsonCel {
	pub fn new(variables: serde_json::Value) -> Result<Self, Error> {
		let serde_json::Value::Object(variables) = variables else {
			return Err(Error::Variable(
				"test CEL variables must be a JSON object".to_owned(),
			));
		};
		let variables = variables
			.into_iter()
			.map(|(name, value)| {
				cel::to_value(value)
					.map(|value| (name, value))
					.map_err(|error| Error::Variable(error.to_string()))
			})
			.collect::<Result<Vec<_>, _>>()?;
		let mut context = cel::Context::default();
		agent_celx::insert_all(&mut context);
		Ok(Self { context, variables })
	}

	fn eval(&self, expression: &Expression) -> Result<cel::Value<'static>, Error> {
		let mut variables = cel::context::MapResolver::new();
		for (name, value) in &self.variables {
			variables.add_variable_from_value(name, value.clone());
		}
		cel::Value::resolve(expression.ast(), &self.context, &variables)
			.map(|value| value.as_static())
			.map_err(Error::from)
	}
}

impl PolicyCel for JsonCel {
	fn eval_request<'a>(
		&'a self,
		expression: &'a Expression,
		_request: &'a Request,
	) -> Result<cel::Value<'a>, Error> {
		self.eval(expression)
	}

	fn eval_response<'a>(
		&'a self,
		expression: &'a Expression,
		_response: &'a Response,
	) -> Result<cel::Value<'a>, Error> {
		self.eval(expression)
	}
}

enum TestCel {
	Json(JsonCel),
	Fixed(FixedCel),
}

impl PolicyCel for TestCel {
	fn eval_request<'a>(
		&'a self,
		expression: &'a Expression,
		request: &'a Request,
	) -> Result<cel::Value<'a>, Error> {
		match self {
			Self::Json(cel) => cel.eval_request(expression, request),
			Self::Fixed(cel) => cel.eval_request(expression, request),
		}
	}

	fn eval_response<'a>(
		&'a self,
		expression: &'a Expression,
		response: &'a Response,
	) -> Result<cel::Value<'a>, Error> {
		match self {
			Self::Json(cel) => cel.eval_response(expression, response),
			Self::Fixed(cel) => cel.eval_response(expression, response),
		}
	}

	fn eval_request_response<'a>(
		&'a self,
		expression: &'a Expression,
		request: &'a Request,
		response: &'a Response,
	) -> Result<cel::Value<'a>, Error> {
		match self {
			Self::Json(cel) => cel.eval_request_response(expression, request, response),
			Self::Fixed(cel) => cel.eval_request_response(expression, request, response),
		}
	}
}

impl PolicyCel for FixedCel {
	fn eval_request<'a>(
		&'a self,
		_expression: &'a Expression,
		_request: &'a Request,
	) -> Result<cel::Value<'a>, Error> {
		Ok(self.value.clone())
	}

	fn eval_response<'a>(
		&'a self,
		_expression: &'a Expression,
		_response: &'a Response,
	) -> Result<cel::Value<'a>, Error> {
		Ok(self.value.clone())
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedBackendCall {
	pub call: PolicyCall,
	pub method: Method,
	pub uri: Uri,
	pub headers: HeaderMap,
}

#[derive(Clone, Default)]
pub struct RecordingBackend {
	call: Option<PolicyCall>,
	calls: Arc<Mutex<Vec<RecordedBackendCall>>>,
}

impl RecordingBackend {
	pub fn dispatcher(&self) -> BackendDispatcher {
		BackendDispatcher::new(self.clone(), Arc::new(()), Arc::new(Vec::<()>::new()))
	}

	pub fn calls(&self) -> Vec<RecordedBackendCall> {
		self
			.calls
			.lock()
			.expect("recording backend lock poisoned")
			.clone()
	}

	fn clear(&self) {
		self
			.calls
			.lock()
			.expect("recording backend lock poisoned")
			.clear();
	}
}

impl BackendReferenceClient<(), ()> for RecordingBackend {
	type Error = Infallible;

	fn with_policy_call(&self, call: PolicyCall) -> Self {
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
