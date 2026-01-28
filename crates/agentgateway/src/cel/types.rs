use std::fmt::Debug;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use agent_core::strng::Strng;
use bytes::Bytes;
use cel::Value;
use cel::common::ast::OptimizedExpr;
use cel::context::VariableResolver;
use cel::objects::BytesValue;
use cel::types::dynamic::DynamicType;
use http::Extensions;
use prometheus_client::encoding::EncodeLabelValue;
#[cfg(feature = "schema")]
pub use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};

use crate::cel::{Error, Expression, ROOT_CONTEXT};
use crate::http::ext_authz::ExtAuthzDynamicMetadata;
use crate::http::{apikey, basicauth, jwt};
use crate::llm::{LLMInfo, LLMRequest};
use crate::mcp::ResourceType;
use crate::serdes::schema;
use crate::{apply, llm};

#[derive(Debug, Default, cel::DynamicType, Serialize)]
pub struct Executor<'a> {
	/// `request` contains attributes about the incoming HTTP request
	#[serde(skip_serializing_if = "Option::is_none")]
	pub request: Option<RequestRef<'a>>,

	/// `response` contains attributes about the HTTP response
	#[serde(skip_serializing_if = "Option::is_none")]
	pub response: Option<ResponseRef<'a>>,

	/// `source` contains attributes about the source of the request.
	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub source: ExtensionOrDirect<'a, SourceContext>,

	/// `jwt` contains the claims from a verified JWT token. This is only present if the JWT policy is enabled.
	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub jwt: ExtensionOrDirect<'a, jwt::Claims>,

	/// `apiKey` contains the claims from a verified API Key. This is only present if the API Key policy is enabled.
	#[serde(rename = "apiKey", skip_serializing_if = "is_extension_or_direct_none")]
	pub api_key: ExtensionOrDirect<'a, apikey::Claims>,

	/// `basicAuth` contains the claims from a verified basic authentication Key. This is only present if the Basic authentication policy is enabled.
	#[serde(
		rename = "basicAuth",
		skip_serializing_if = "is_extension_or_direct_none"
	)]
	pub basic_auth: ExtensionOrDirect<'a, basicauth::Claims>,

	/// `llm` contains attributes about an LLM request or response. This is only present when using an `ai` backend.
	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub llm: ExtensionOrDirect<'a, LLMContext>,

	/// `mcp` contains attributes about the MCP request.
	// This is only included for schema generation; see build_with_mcp.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mcp: Option<&'a ResourceType>,

	/// `backend` contains information about the backend being used.
	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub backend: ExtensionOrDirect<'a, BackendContext>,
	/// `extauthz` contains dynamic metadata from ext_authz filters
	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub extauthz: ExtensionOrDirect<'a, ExtAuthzDynamicMetadata>,
	// `extproc` contains dynamic metadata from ext_proc filters
	// pub extproc: ExtensionOrDirect<'a, ExtProcDynamicMetadata>,
}

fn is_extension_or_direct_none<T: Send + Sync + 'static>(e: &ExtensionOrDirect<T>) -> bool {
	e.deref().is_none()
}

#[derive(Clone, Debug, Serialize, Deserialize, cel::DynamicType)]
pub struct SourceContext {
	/// The IP address of the downstream connection.
	pub address: IpAddr,
	/// The port of the downstream connection.
	pub port: u16,
	/// The (Istio SPIFFE) identity of the downstream connection, if available.
	#[serde(flatten, skip_deserializing)]
	#[dynamic(flatten)]
	pub tls: Option<crate::transport::tls::TlsInfo>,
}

#[apply(schema!)]
#[derive(cel::DynamicType)]
pub struct BackendContext {
	/// The name of the backend being used. For example, `my-service` or `service/my-namespace/my-service:8080`.
	pub name: Strng,
	/// The type of backend. For example, `ai`, `mcp`, `static`, `dynamic`, or `service`.
	#[dynamic(rename = "type")]
	#[serde(rename = "type")]
	pub backend_type: BackendType,
	/// The protocol of backend. For example, `http`, `tcp`, `a2a`, `mcp`, or `llm`.
	pub protocol: BackendProtocol,
}

#[derive(Copy, PartialEq, Eq, Hash, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(cel::DynamicType)]
pub enum BackendType {
	#[dynamic(rename = "ai")]
	AI,
	#[dynamic(rename = "mcp")]
	MCP,
	#[dynamic(rename = "static")]
	Static,
	#[dynamic(rename = "dynamic")]
	Dynamic,
	#[dynamic(rename = "service")]
	Service,
	#[dynamic(rename = "unknown")]
	Unknown,
}

#[derive(
	Copy, PartialEq, Eq, Hash, EncodeLabelValue, Debug, Clone, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[allow(non_camel_case_types)]
#[derive(cel::DynamicType)]
pub enum BackendProtocol {
	http,
	tcp,
	a2a,
	mcp,
	llm,
}

struct ExecutorResolver<'a> {
	executor: &'a Executor<'a>,
}
impl<'a> VariableResolver<'a> for ExecutorResolver<'a> {
	fn resolve(&self, variable: &str) -> Option<Value<'a>> {
		self.executor.field(variable)
	}
	// A bit annoying, but a nice speed up for us
	fn resolve_member(&self, expr: &str, member: &str) -> Option<Value<'a>> {
		match expr {
			"request" => self.executor.request.as_ref().and_then(|r| r.field(member)),
			"response" => self
				.executor
				.response
				.as_ref()
				.and_then(|r| r.field(member)),
			_ => None,
		}
	}
	fn resolve_direct(&self, field: &OptimizedExpr) -> Option<Option<Value<'a>>> {
		match field {
			// To avoid a conversion from a string key into a HeaderName, we have a hot path
			OptimizedExpr::HeaderLookup { request, header } if *request => Some(
				self
					.executor
					.request
					.as_ref()
					.and_then(|r| r.headers.get(header))
					.and_then(|h| h.to_str().ok())
					.map(|s| Value::String(s.into())),
			),
			// OptimizedExpr::HeaderLookup { request, header } if !*request => Some(
			// 	self
			// 		.executor
			// 		.response
			// 		.as_ref()
			// 		.and_then(|r| r.headers.get(header))
			// 		.and_then(|h| h.to_str().ok())
			// 		.map(|s| Value::String(s.into())),
			// ),
			_ => None,
		}
	}
}

impl<'a> Executor<'a> {
	fn set_request<B>(&mut self, req: &'a ::http::Request<B>) {
		self.request = Some(req.into());
		let ext = req.extensions();
		self.api_key = ExtensionOrDirect::Extension(ext);
		self.jwt = ExtensionOrDirect::Extension(ext);
		self.llm = ExtensionOrDirect::Extension(ext);
		self.basic_auth = ExtensionOrDirect::Extension(ext);
		self.extauthz = ExtensionOrDirect::Extension(ext);
		self.backend = ExtensionOrDirect::Extension(ext);
		self.source = ExtensionOrDirect::Extension(ext);
	}
	fn set_request_snapshot(&mut self, req: &'a RequestSnapshot) {
		self.request = Some(req.into());
		self.api_key = ExtensionOrDirect::Direct(req.api_key.as_ref());
		self.jwt = ExtensionOrDirect::Direct(req.jwt.as_ref());
		self.basic_auth = ExtensionOrDirect::Direct(req.basic_auth.as_ref());
		self.extauthz = ExtensionOrDirect::Direct(req.extauthz.as_ref());
		self.backend = ExtensionOrDirect::Direct(req.backend.as_ref());
		// self.extproc = ExtensionOrDirect::Direct(req.basic_auth.as_ref());
		self.llm = ExtensionOrDirect::Direct(req.llm.as_ref());
		self.source = ExtensionOrDirect::Direct(req.source.as_ref());
	}
	fn set_response(&mut self, resp: &'a crate::http::Response) {
		self.response = Some(resp.into());
	}
	fn set_response_snapshot(&mut self, resp: &'a ResponseSnapshot) {
		self.response = Some(resp.into());
	}
	pub fn new_empty() -> Self {
		Default::default()
	}
	pub fn new_mcp<B>(req: &'a ::http::Request<B>, mcp: &'a ResourceType) -> Self {
		let mut this = Self::new_empty();
		this.set_request(req);
		this.mcp = Some(mcp);
		this
	}
	pub fn new_logger(
		req: Option<&'a RequestSnapshot>,
		resp: Option<&'a ResponseSnapshot>,
		llm: Option<&'a LLMContext>,
	) -> Self {
		let mut this = Self::new_empty();
		if let Some(req) = req {
			this.set_request_snapshot(req);
		}
		if let Some(resp) = resp {
			this.set_response_snapshot(resp);
		}
		this.llm = ExtensionOrDirect::Direct(llm);
		// TODO: MCP
		// this.mcp = Some(McpContextRef::Ref(mcp));
		this
	}
	pub fn new_request(req: &'a crate::http::Request) -> Self {
		let mut this = Self::new_empty();
		this.set_request(req);
		this
	}
	pub fn new_request_and_response(
		req: &'a crate::http::Request,
		resp: &'a crate::http::Response,
	) -> Self {
		let mut this = Self::new_empty();
		this.set_request(req);
		this.set_response(resp);
		this
	}
	pub fn new_response(
		req: Option<&'a RequestSnapshot>,
		response: &'a crate::http::Response,
	) -> Self {
		let mut this = Self::new_empty();
		if let Some(req) = req {
			this.set_request_snapshot(req);
		}
		this.set_response(response);
		this
	}

	pub fn eval(&'a self, expr: &'a Expression) -> Result<Value<'a>, Error> {
		let resolver = ExecutorResolver { executor: self };
		match Value::resolve(
			expr.expression.expression(),
			ROOT_CONTEXT.as_ref(),
			&resolver,
		) {
			Ok(v) => Ok(v),
			Err(e) => {
				tracing::trace!("failed to evaluate expression: {}", e);
				Err(e.into())
			},
		}
	}
	pub fn eval_bool(&self, expr: &Expression) -> bool {
		self
			.eval(expr)
			.map(|v| v.as_bool().unwrap_or_default())
			.unwrap_or_default()
	}

	/// eval_rng evaluates a float (0.0-1.0) or a bool and evaluates to a bool. If a float is returned,
	/// it represents the likelihood true is returned.
	pub fn eval_rng(&self, expr: &Expression) -> bool {
		match self.eval(expr) {
			Ok(Value::Bool(b)) => b,
			Ok(Value::Float(f)) => {
				// Clamp this down to 0-1 rang; random_bool can panic
				let f = f.clamp(0.0, 1.0);
				rand::random_bool(f)
			},
			Ok(Value::Int(f)) => {
				// Clamp this down to 0-1 rang; random_bool can panic
				let f = f.clamp(0, 1);
				rand::random_bool(f as f64)
			},
			_ => false,
		}
	}
}

/// snapshot_request takes a request and returns a snapshot of its attributes.
/// EXTENSIONS ARE CLEARED. Do not use this if you still need the extensions later.
pub fn snapshot_request(req: &mut crate::http::Request) -> RequestSnapshot {
	RequestSnapshot {
		method: req.method().clone(),
		path: req.uri().clone(),
		version: req.version(),
		headers: req.headers().clone(),
		body: req.extensions_mut().remove::<BufferedBody>(),
		jwt: req.extensions_mut().remove::<jwt::Claims>(),
		api_key: req.extensions_mut().remove::<apikey::Claims>(),
		basic_auth: req.extensions_mut().remove::<basicauth::Claims>(),
		backend: req.extensions_mut().remove::<BackendContext>(),
		source: req.extensions_mut().remove::<SourceContext>(),
		extauthz: req.extensions_mut().remove::<ExtAuthzDynamicMetadata>(),
		llm: req.extensions_mut().remove::<LLMContext>(),
	}
}

/// snapshot_response takes a response and returns a snapshot of its attributes.
/// EXTENSIONS ARE CLEARED. Do not use this if you still need the extensions later.
pub fn snapshot_response(resp: &mut crate::http::Response) -> ResponseSnapshot {
	ResponseSnapshot {
		code: resp.status(),
		headers: resp.headers().clone(),
		body: resp.extensions_mut().remove::<BufferedBody>(),
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSnapshot {
	/// The request's method
	#[serde(with = "http_serde::method")]
	pub method: http::Method,

	/// The request's URI
	#[serde(with = "http_serde::uri", rename = "uri")]
	pub path: http::Uri,

	/// The request's version
	#[serde(with = "http_serde::version")]
	pub version: http::Version,

	// TODO: do not use header_map, which will make multi-headers a list
	#[serde(with = "http_serde::header_map")]
	/// The request's headers
	pub headers: http::HeaderMap,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<BufferedBody>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwt: Option<jwt::Claims>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub api_key: Option<apikey::Claims>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub basic_auth: Option<basicauth::Claims>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub backend: Option<BackendContext>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub source: Option<SourceContext>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub extauthz: Option<ExtAuthzDynamicMetadata>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub llm: Option<LLMContext>,
}

#[derive(Debug, Clone, Serialize, cel::DynamicType)]
pub struct RequestRef<'a> {
	/// The request's method
	#[serde(with = "http_serde::method")]
	#[dynamic(with_value = "to_value_str")]
	pub method: &'a http::Method,

	/// The request's URI
	#[serde(with = "http_serde::uri")]
	#[dynamic(with_value = "to_value_owned_string")]
	pub uri: &'a http::Uri,
	pub path: &'a str,

	/// The request's version
	#[serde(with = "http_serde::version")]
	#[dynamic(with_value = "version_to_value")]
	pub version: http::Version,

	/// The request's headers
	#[serde(with = "http_serde::header_map")]
	pub headers: &'a http::HeaderMap,

	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub body: ExtensionOrDirect<'a, BufferedBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseSnapshot {
	#[serde(with = "http_serde::status_code")]
	pub code: http::StatusCode,
	#[serde(with = "http_serde::header_map")]
	pub headers: http::HeaderMap,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<BufferedBody>,
}

#[derive(Debug, Clone, Serialize, cel::DynamicType)]
pub struct ResponseRef<'a> {
	/// The HTTP status code of the response.
	pub code: u16,

	/// The headers of the response.
	#[serde(with = "http_serde::header_map")]
	pub headers: &'a http::HeaderMap,

	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub body: ExtensionOrDirect<'a, BufferedBody>,
}

impl<'a> From<&'a ResponseSnapshot> for ResponseRef<'a> {
	fn from(value: &'a ResponseSnapshot) -> Self {
		Self {
			code: value.code.as_u16(),
			headers: &value.headers,
			body: value.body.as_ref().into(),
		}
	}
}

/// Owned version of RequestRef for JSON serialization/deserialization.
///
/// This struct contains owned HTTP request data that can be serialized to/from JSON.
/// It mirrors `RequestRef` but uses owned types (`http::Method`, `http::Uri`, `String`)
/// instead of references.
///
/// # JSON Representation
///
/// - `method`: HTTP method as a string (e.g., "GET", "POST")
/// - `uri`: Full URI as a string
/// - `path`: Path component extracted from URI
/// - `version`: HTTP version (e.g., "HTTP/1.1")
/// - `headers`: Header map
/// - `body`: Optional request body (base64-encoded string in JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRefSerde {
	/// The request's method
	#[serde(with = "http_serde::method")]
	pub method: http::Method,

	/// The request's URI
	#[serde(with = "http_serde::uri")]
	pub uri: http::Uri,

	/// The request's path (extracted from URI)
	pub path: String,

	/// The request's version
	#[serde(with = "http_serde::version")]
	pub version: http::Version,

	/// The request's headers
	#[serde(with = "http_serde::header_map")]
	pub headers: http::HeaderMap,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<BufferedBody>,
}

/// Owned version of ResponseRef for JSON serialization/deserialization.
///
/// This struct contains owned HTTP response data that can be serialized to/from JSON.
/// It mirrors `ResponseRef` but uses owned types.
///
/// # JSON Representation
///
/// - `code`: HTTP status code as a number (e.g., 200, 404)
/// - `headers`: Header map
/// - `body`: Optional response body (base64-encoded string in JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseRefSerde {
	/// The HTTP status code of the response.
	pub code: u16,

	/// The headers of the response.
	#[serde(with = "http_serde::header_map")]
	pub headers: http::HeaderMap,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<BufferedBody>,
}

impl<'a> From<&'a RequestSnapshot> for RequestRef<'a> {
	fn from(value: &'a RequestSnapshot) -> Self {
		Self {
			method: &value.method,
			uri: &value.path,
			path: value.path.path(),
			version: value.version,
			headers: &value.headers,
			body: value.body.as_ref().into(),
		}
	}
}
impl<'a, B> From<&'a ::http::Request<B>> for RequestRef<'a> {
	fn from(req: &'a ::http::Request<B>) -> Self {
		Self {
			method: req.method(),
			uri: req.uri(),
			path: req.uri().path(),
			version: req.version(),
			headers: req.headers(),
			body: req.extensions().into(),
		}
	}
}

impl<'a> From<&'a crate::http::Response> for ResponseRef<'a> {
	fn from(resp: &'a crate::http::Response) -> Self {
		Self {
			code: resp.status().as_u16(),
			headers: resp.headers(),
			body: resp.extensions().into(),
		}
	}
}

/// Buffered request or response body.
///
/// # Serialization
///
/// - **JSON**: Serialized as a base64-encoded string
/// - **CEL**: Materialized as `Value::Bytes` when accessed in CEL expressions
///
/// # Example
///
/// ```ignore
/// // JSON representation
/// {
///   "body": "SGVsbG8gV29ybGQ="  // base64-encoded "Hello World"
/// }
///
/// // In CEL, access as bytes:
/// // request.body  // → Value::Bytes(b"Hello World")
/// ```
#[derive(Debug, Clone)]
pub struct BufferedBody(pub Bytes);

impl Serialize for BufferedBody {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use base64::Engine;
		let encoded = base64::engine::general_purpose::STANDARD.encode(&self.0);
		serializer.serialize_str(&encoded)
	}
}

impl<'de> Deserialize<'de> for BufferedBody {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		use base64::Engine;
		let s = String::deserialize(deserializer)?;
		let bytes = base64::engine::general_purpose::STANDARD
			.decode(&s)
			.map_err(serde::de::Error::custom)?;
		Ok(BufferedBody(Bytes::from(bytes)))
	}
}

impl DynamicType for BufferedBody {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::Bytes(BytesValue::Bytes(self.0.clone()))
	}
}

impl PartialEq for RequestRef<'_> {
	fn eq(&self, _: &Self) -> bool {
		// Currently do not allow comparisons
		false
	}
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, cel::DynamicType)]
#[serde(rename_all = "camelCase")]
pub struct LLMContext {
	/// Whether the LLM response is streamed.
	pub streaming: bool,
	/// The model requested for the LLM request. This may differ from the actual model used.
	#[dynamic(rename = "requestModel")]
	pub request_model: Strng,
	/// The model that actually served the LLM response.
	#[dynamic(rename = "responseModel")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub response_model: Option<Strng>,
	/// The provider of the LLM.
	pub provider: Strng,
	/// The number of tokens in the input/prompt.
	#[dynamic(rename = "inputTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_tokens: Option<u64>,
	/// The number of tokens in the output/completion.
	#[dynamic(rename = "outputTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_tokens: Option<u64>,
	/// The total number of tokens for the request.
	#[dynamic(rename = "totalTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub total_tokens: Option<u64>,
	// For now, not exposed to CEL; only used to piggy-back this field for metrics.
	#[serde(skip)]
	#[dynamic(skip)]
	pub first_token: Option<Instant>,
	/// The number of tokens in the request, when using the token counting endpoint
	/// These are not counted as 'input tokens' since they do not consume input tokens.
	#[dynamic(rename = "countTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub count_tokens: Option<u64>,
	/// The prompt sent to the LLM. Warning: accessing this has some performance impacts for large prompts.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub prompt: Option<Arc<Vec<llm::SimpleChatCompletionMessage>>>,
	/// The completion from the LLM. Warning: accessing this has some performance impacts for large responses.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub completion: Option<Vec<String>>,
	/// The parameters for the LLM request.
	pub params: llm::LLMRequestParams,
}

impl From<llm::LLMInfo> for LLMContext {
	fn from(value: LLMInfo) -> Self {
		let resp = value.response;
		let mut base = LLMContext {
			output_tokens: resp.output_tokens,
			count_tokens: resp.count_tokens,
			total_tokens: resp.total_tokens,
			first_token: resp.first_token,
			response_model: resp.provider_model.clone(),
			// Not always set
			completion: resp.completion.clone(),
			..LLMContext::from(value.request)
		};

		if let Some(pt) = resp.input_tokens {
			// Better info, override
			base.input_tokens = Some(pt);
		}
		base
	}
}
impl From<llm::LLMRequest> for LLMContext {
	fn from(info: LLMRequest) -> Self {
		let LLMRequest {
			input_tokens,
			input_format: _, // Expose this?
			request_model,
			provider,
			streaming,
			params,
			prompt,
		} = info;
		LLMContext {
			streaming,
			request_model,
			provider,
			input_tokens,
			params,
			prompt,

			first_token: None,
			count_tokens: None,
			response_model: None,
			output_tokens: None,
			total_tokens: None,
			completion: None,
		}
	}
}

fn to_value_str<'a, T: AsRef<str>>(c: &'a &'a T) -> Value<'a> {
	Value::String(c.as_ref().into())
}
pub fn to_value_redacted<'a>(c: &'a SecretString) -> Value<'a> {
	Value::String(c.expose_secret().into())
}
fn version_to_value<'a>(c: &'a http::Version) -> Value<'a> {
	Value::String(crate::http::version_str(c).into())
}
fn to_value_owned_string<'a, T: ToString>(c: &'a &'a T) -> Value<'a> {
	Value::String(c.to_string().into())
}

/// Wrapper for values that can come from HTTP Extensions or direct references.
///
/// This enum is used in `Executor` to support two patterns:
/// - **Extension**: Value is looked up from `http::Extensions` at access time
/// - **Direct**: Value is a direct optional reference (used when building from snapshots)
///
/// # Serialization
///
/// When serialized, this type dereferences to the underlying value:
/// - If present: serializes the value of type `T`
/// - If absent: serializes as `null`
///
/// # Deserialization
///
/// This type does **not** support deserialization. Use `ExecutorSerde` with `Option<T>`
/// fields for deserialization, then convert to `Executor` using `as_executor()`.
#[derive(Debug, Clone)]
pub enum ExtensionOrDirect<'a, T> {
	Extension(&'a http::Extensions),
	Direct(Option<&'a T>),
}

impl<'a, T: Serialize + Send + Sync + 'static> Serialize for ExtensionOrDirect<'a, T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self.deref() {
			Some(v) => v.serialize(serializer),
			None => serializer.serialize_none(),
		}
	}
}

impl<'a, T> From<&'a http::Extensions> for ExtensionOrDirect<'a, T> {
	fn from(value: &'a Extensions) -> Self {
		Self::Extension(value)
	}
}
impl<'a, T> From<Option<&'a T>> for ExtensionOrDirect<'a, T> {
	fn from(value: Option<&'a T>) -> Self {
		Self::Direct(value)
	}
}

impl<T> Default for ExtensionOrDirect<'_, T> {
	fn default() -> Self {
		Self::Direct(None)
	}
}

impl<'a, T: Send + Sync + 'static> ExtensionOrDirect<'a, T> {
	fn deref(&self) -> Option<&'a T> {
		match self {
			ExtensionOrDirect::Extension(e) => e.get::<T>(),
			ExtensionOrDirect::Direct(t) => *t,
		}
	}
}

impl<'a, T> DynamicType for ExtensionOrDirect<'a, T>
where
	T: DynamicType + Debug + Send + Sync + 'static,
{
	fn auto_materialize(&self) -> bool {
		match self.deref() {
			Some(v) => v.auto_materialize(),
			None => true, // Null should auto-materialize
		}
	}
	fn materialize(&self) -> Value<'_> {
		match self.deref() {
			Some(t) => t.materialize(),
			None => Value::Null,
		}
	}

	fn field(&self, field: &str) -> Option<Value<'_>> {
		match self.deref() {
			Some(t) => t.field(field),
			None => None,
		}
	}
}

/// Owned version of Executor for JSON serialization/deserialization.
///
/// `ExecutorSerde` is a fully-owned representation that can be deserialized from JSON,
/// stored, and later converted to an `Executor<'_>` for use with CEL expressions.
///
/// # Usage
///
/// ```ignore
/// use agentgateway::cel::types::ExecutorSerde;
///
/// // Deserialize from JSON
/// let json = r#"{
///     "request": {
///         "method": "GET",
///         "uri": "https://example.com/api",
///         "path": "/api",
///         "version": "HTTP/1.1",
///         "headers": {}
///     }
/// }"#;
///
/// let snapshot: ExecutorSerde = serde_json::from_str(json).unwrap();
///
/// // Convert to Executor (borrows from snapshot)
/// let executor = snapshot.as_executor();
///
/// // Use with CEL expressions
/// // let result = executor.eval(&expression);
///
/// // The snapshot must outlive the executor
/// drop(executor);
/// drop(snapshot);
/// ```
///
/// # Lifetime Requirements
///
/// The `Executor<'_>` returned by `as_executor()` borrows from this `ExecutorSerde`.
/// The `ExecutorSerde` must remain alive for the entire lifetime of the `Executor`.
///
/// # JSON Format
///
/// - Request bodies are serialized as base64-encoded strings
/// - Field names use camelCase (e.g., `apiKey`, `basicAuth`)
/// - Optional fields are omitted when `None`
///
/// # Round-trip Behavior
///
/// JSON → ExecutorSerde → Executor<'_> → CEL → JSON should be consistent, with the
/// following notes:
/// - CEL's `variables()` may include explicit nulls for `None` fields
/// - JWT token secrets are not preserved on deserialization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutorSerde {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request: Option<RequestRefSerde>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response: Option<ResponseRefSerde>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub source: Option<SourceContext>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwt: Option<jwt::Claims>,

	#[serde(rename = "apiKey")]
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub api_key: Option<apikey::Claims>,

	#[serde(rename = "basicAuth")]
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub basic_auth: Option<basicauth::Claims>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub llm: Option<LLMContext>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub mcp: Option<ResourceType>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub backend: Option<BackendContext>,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub extauthz: Option<ExtAuthzDynamicMetadata>,
}

impl ExecutorSerde {
	/// Converts this owned representation into an `Executor` that borrows from it.
	///
	/// # Lifetime Requirements
	///
	/// The returned `Executor<'_>` borrows data from this `ExecutorSerde`. The
	/// `ExecutorSerde` **must outlive** the returned `Executor`.
	///
	/// # Example
	///
	/// ```ignore
	/// let snapshot: ExecutorSerde = serde_json::from_str(json_str)?;
	///
	/// // This is OK - snapshot outlives executor
	/// {
	///     let executor = snapshot.as_executor();
	///     let result = executor.eval(&expression)?;
	/// } // executor dropped here
	///
	/// // This is WRONG - would cause use-after-free
	/// // let executor = {
	/// //     let snapshot: ExecutorSerde = serde_json::from_str(json_str)?;
	/// //     snapshot.as_executor() // ERROR: returns reference to dropped value
	/// // };
	/// ```
	///
	/// # Returns
	///
	/// An `Executor<'_>` with all fields populated from this snapshot. Fields that
	/// are `None` in the snapshot will be absent in the executor.
	pub fn as_executor(&self) -> Executor<'_> {
		let mut exec = Executor::new_empty();

		// Set request if present
		if let Some(req) = &self.request {
			exec.request = Some(RequestRef {
				method: &req.method,
				uri: &req.uri,
				path: &req.path,
				version: req.version,
				headers: &req.headers,
				body: ExtensionOrDirect::Direct(req.body.as_ref()),
			});
		}

		// Set response if present
		if let Some(resp) = &self.response {
			exec.response = Some(ResponseRef {
				code: resp.code,
				headers: &resp.headers,
				body: ExtensionOrDirect::Direct(resp.body.as_ref()),
			});
		}

		// Set all the ExtensionOrDirect fields
		exec.source = ExtensionOrDirect::Direct(self.source.as_ref());
		exec.jwt = ExtensionOrDirect::Direct(self.jwt.as_ref());
		exec.api_key = ExtensionOrDirect::Direct(self.api_key.as_ref());
		exec.basic_auth = ExtensionOrDirect::Direct(self.basic_auth.as_ref());
		exec.llm = ExtensionOrDirect::Direct(self.llm.as_ref());
		exec.backend = ExtensionOrDirect::Direct(self.backend.as_ref());
		exec.extauthz = ExtensionOrDirect::Direct(self.extauthz.as_ref());
		exec.mcp = self.mcp.as_ref();

		exec
	}
}

#[cfg(test)]
#[path = "types_test.rs"]
mod types_test;
