use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Instant;

use agent_core::strng::Strng;
use bytes::Bytes;
use cel::Value;
use cel::common::ast::OptimizedExpr;
use cel::context::VariableResolver;
use cel::objects::BytesValue;
use cel::types::dynamic::DynamicType;
use http::{Extensions, HeaderMap, Method, Uri, Version};
use prometheus_client::encoding::EncodeLabelValue;
#[cfg(feature = "schema")]
pub use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::json;

use crate::cel::{Error, Expression, ROOT_CONTEXT};
use crate::http::ext_authz::ExtAuthzDynamicMetadata;
use crate::http::ext_proc::ExtProcDynamicMetadata;
use crate::http::{apikey, basicauth, jwt};
use crate::llm::{LLMInfo, LLMRequest};
use crate::mcp::{ResourceId, ResourceType};
use crate::serdes::schema;
use crate::transport::tls::TlsInfo;
use crate::{apply, llm};

#[derive(Debug, Default, cel::DynamicType)]
#[dynamic(rename_all = "camelCase")]
pub struct Executor<'a> {
	#[dynamic(skip_serializing_if = "Option::is_none")]
	pub request: Option<RequestRef<'a>>,

	#[dynamic(skip_serializing_if = "Option::is_none")]
	pub response: Option<ResponseRef<'a>>,

	#[dynamic(skip_serializing_if = "is_extension_or_direct_none")]
	pub source: ExtensionOrDirect<'a, SourceContext>,

	#[dynamic(skip_serializing_if = "is_extension_or_direct_none")]
	pub jwt: ExtensionOrDirect<'a, jwt::Claims>,

	#[dynamic(rename = "apiKey", skip_serializing_if = "is_extension_or_direct_none")]
	pub api_key: ExtensionOrDirect<'a, apikey::Claims>,

	#[dynamic(
		rename = "basicAuth",
		skip_serializing_if = "is_extension_or_direct_none"
	)]
	pub basic_auth: ExtensionOrDirect<'a, basicauth::Claims>,

	#[dynamic(skip_serializing_if = "is_extension_or_direct_none")]
	pub llm: ExtensionOrDirect<'a, LLMContext>,

	#[dynamic(rename = "llmRequest", skip_serializing_if = "Option::is_none")]
	pub llm_request: Option<&'a serde_json::Value>,

	#[dynamic(skip_serializing_if = "Option::is_none")]
	pub mcp: Option<&'a ResourceType>,

	#[dynamic(skip_serializing_if = "is_extension_or_direct_none")]
	pub backend: ExtensionOrDirect<'a, BackendContext>,

	#[dynamic(skip_serializing_if = "is_extension_or_direct_none")]
	pub extauthz: ExtensionOrDirect<'a, ExtAuthzDynamicMetadata>,

	#[dynamic(skip_serializing_if = "is_extension_or_direct_none")]
	pub extproc: ExtensionOrDirect<'a, ExtProcDynamicMetadata>,
}

fn is_extension_or_direct_none<T: Send + Sync + 'static>(e: &ExtensionOrDirect<T>) -> bool {
	e.deref().is_none()
}

#[apply(schema!)]
#[derive(cel::DynamicType)]
pub struct SourceContext {
	#[serde(default = "dummy_address")]
	/// The IP address of the downstream connection.
	pub address: IpAddr,
	#[serde(default)]
	/// The port of the downstream connection.
	pub port: u16,
	/// The (Istio SPIFFE) identity of the downstream connection, if available.
	#[serde(flatten, default, deserialize_with = "none_if_empty")]
	#[dynamic(flatten)]
	pub tls: Option<crate::transport::tls::TlsInfo>,
}
fn none_if_empty<'de, D>(deserializer: D) -> Result<Option<TlsInfo>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let tls = TlsInfo::deserialize(deserializer)?;
	Ok(if tls == TlsInfo::default() {
		None
	} else {
		Some(tls)
	})
}

fn dummy_address() -> IpAddr {
	IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

#[apply(schema!)]
#[derive(cel::DynamicType)]
pub struct BackendContext {
	/// The name of the backend being used. For example, `my-service` or `service/my-namespace/my-service:8080`.
	#[serde(default)]
	pub name: Strng,
	/// The type of backend. For example, `ai`, `mcp`, `static`, `dynamic`, or `service`.
	#[serde(rename = "type")]
	#[serde(default)]
	pub backend_type: BackendType,
	/// The protocol of backend. For example, `http`, `tcp`, `a2a`, `mcp`, or `llm`.
	#[serde(default)]
	pub protocol: BackendProtocol,
}

#[derive(
	Default, Copy, PartialEq, Eq, Hash, Debug, Clone, serde::Serialize, serde::Deserialize,
)]
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
	#[default]
	Unknown,
}

#[derive(
	Default,
	Copy,
	PartialEq,
	Eq,
	Hash,
	EncodeLabelValue,
	Debug,
	Clone,
	serde::Serialize,
	serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[allow(non_camel_case_types)]
#[derive(cel::DynamicType)]
pub enum BackendProtocol {
	#[default]
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
	pub fn new_mcp(req: Option<&'a RequestSnapshot>, mcp: &'a ResourceType) -> Self {
		let mut this = Self::new_empty();
		if let Some(req) = req {
			this.set_request_snapshot(req);
		}
		this.mcp = Some(mcp);
		this
	}
	pub fn new_llm(req: Option<&'a RequestSnapshot>, llm_body: &'a serde_json::Value) -> Self {
		let mut this = Self::new_empty();
		if let Some(req) = req {
			this.set_request_snapshot(req);
		}
		this.llm_request = Some(llm_body);
		this
	}
	pub fn new_logger(
		req: Option<&'a RequestSnapshot>,
		resp: Option<&'a ResponseSnapshot>,
		llm: Option<&'a LLMContext>,
		mcp: Option<&'a ResourceType>,
		end_time: Option<&'a str>,
	) -> Self {
		let mut this = Self::new_empty();
		if let Some(req) = req {
			this.set_request_snapshot(req);
		}
		if let Some(resp) = resp {
			this.set_response_snapshot(resp);
		}
		this.llm = ExtensionOrDirect::Direct(llm);
		this.mcp = mcp;
		if let Some(f) = this.request.as_mut() {
			f.end_time = end_time;
		}
		this
	}
	pub fn new_tcp_logger(
		source_context: Option<&'a SourceContext>,
		end_time: Option<&'a str>,
	) -> Self {
		let mut this = Self::new_empty();
		// For TCP connections, set the source context directly
		this.source = ExtensionOrDirect::Direct(source_context);
		if let Some(f) = this.request.as_mut() {
			f.end_time = end_time;
		}
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
		host: req.uri().authority().cloned(),
		scheme: req.uri().scheme().cloned(),
		version: req.version(),
		headers: req.headers().clone(),
		body: req.extensions_mut().remove::<BufferedBody>(),
		// This one we do not remove, as it's used downstream of the snapshot for auth in MCP case
		// TODO: structure this better
		jwt: req.extensions_mut().get::<jwt::Claims>().cloned(),
		api_key: req.extensions_mut().remove::<apikey::Claims>(),
		basic_auth: req.extensions_mut().remove::<basicauth::Claims>(),
		backend: req.extensions_mut().remove::<BackendContext>(),
		source: req.extensions_mut().remove::<SourceContext>(),
		extauthz: req.extensions_mut().remove::<ExtAuthzDynamicMetadata>(),
		extproc: req.extensions_mut().remove::<ExtProcDynamicMetadata>(),
		llm: req.extensions_mut().remove::<LLMContext>(),
		start_time: req.extensions_mut().remove::<RequestStartTime>(),
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

#[derive(Debug, Clone)]
pub struct RequestSnapshot {
	/// The request's method
	pub method: http::Method,

	/// The request's URI
	pub path: http::Uri,

	pub host: Option<::http::uri::Authority>,

	pub scheme: Option<::http::uri::Scheme>,

	/// The request's version
	pub version: http::Version,

	// TODO: do not use header_map, which will make multi-headers a list
	/// The request's headers
	pub headers: http::HeaderMap,

	pub body: Option<BufferedBody>,

	pub jwt: Option<jwt::Claims>,

	pub api_key: Option<apikey::Claims>,

	pub basic_auth: Option<basicauth::Claims>,

	pub backend: Option<BackendContext>,

	pub source: Option<SourceContext>,

	pub start_time: Option<RequestStartTime>,

	pub extauthz: Option<ExtAuthzDynamicMetadata>,
	pub extproc: Option<ExtProcDynamicMetadata>,

	pub llm: Option<LLMContext>,
}

#[derive(Debug, Clone, Serialize, cel::DynamicType)]
#[serde(rename_all = "camelCase")]
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
	/// The hostname of the request. For example, `example.com`.
	#[serde(serialize_with = "crate::serde_authority_opt")]
	#[dynamic(with_value = "to_value_str_opt")]
	pub host: Option<&'a ::http::uri::Authority>,

	/// The scheme of the request. For example, `https`.
	#[serde(serialize_with = "crate::serde_scheme_opt")]
	#[dynamic(with_value = "to_value_str_opt")]
	pub scheme: Option<&'a ::http::uri::Scheme>,

	/// The request's version
	#[serde(with = "http_serde::version")]
	#[dynamic(with_value = "version_to_value")]
	pub version: http::Version,

	/// The request's headers
	#[serde(with = "http_serde::header_map")]
	pub headers: &'a http::HeaderMap,

	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub body: ExtensionOrDirect<'a, BufferedBody>,

	#[serde(skip_serializing_if = "is_extension_or_direct_none")]
	pub start_time: ExtensionOrDirect<'a, RequestStartTime>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub end_time: Option<&'a str>,
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
#[apply(schema!)]
pub struct RequestRefSerde {
	/// The HTTP method of the request. For example, `GET`
	#[serde(default, with = "http_serde::method")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub method: http::Method,

	/// The complete URI of the request. For example, `http://example.com/path`.
	#[serde(default, with = "http_serde::uri")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub uri: http::Uri,

	/// The hostname of the request. For example, `example.com`.
	#[serde(default, with = "http_serde::option::authority")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub host: Option<::http::uri::Authority>,

	/// The scheme of the request. For example, `https`.
	#[serde(default, with = "http_serde::option::scheme")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub scheme: Option<::http::uri::Scheme>,

	/// The path of the request URI. For example, `/path`.
	#[serde(default)]
	pub path: String,

	/// The version of the request. For example, `HTTP/1.1`.
	#[serde(default, with = "http_serde::version")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub version: http::Version,

	/// The headers of the request.
	#[serde(default, with = "http_serde::header_map")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	pub headers: http::HeaderMap,

	/// The body of the request. Warning: accessing the body will cause the body to be buffered.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<BufferedBody>,

	/// The time the request started
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub start_time: Option<RequestStartTime>,
	/// The time the request completed
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub end_time: Option<String>,
}

#[apply(schema!)]
pub struct ResponseRefSerde {
	/// The HTTP status code of the response.
	#[serde(default)]
	pub code: u16,

	/// The headers of the response.
	#[serde(default, with = "http_serde::header_map")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	pub headers: http::HeaderMap,

	/// The body of the response. Warning: accessing the body will cause the body to be buffered.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<BufferedBody>,
}

impl<'a> From<&'a RequestSnapshot> for RequestRef<'a> {
	fn from(value: &'a RequestSnapshot) -> Self {
		Self {
			method: &value.method,
			uri: &value.path,
			path: value.path.path(),
			host: value.host.as_ref(),
			scheme: value.scheme.as_ref(),
			version: value.version,
			headers: &value.headers,
			body: value.body.as_ref().into(),
			start_time: value.start_time.as_ref().into(),
			end_time: None,
		}
	}
}
impl<'a, B> From<&'a ::http::Request<B>> for RequestRef<'a> {
	fn from(req: &'a ::http::Request<B>) -> Self {
		Self {
			method: req.method(),
			uri: req.uri(),
			path: req.uri().path(),
			host: req.uri().authority(),
			scheme: req.uri().scheme(),
			version: req.version(),
			headers: req.headers(),
			body: req.extensions().into(),
			start_time: req.extensions().into(),
			// Only known in snapshot phase...
			end_time: None,
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

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BufferedBody(#[cfg_attr(feature = "schema", schemars(with = "String"))] pub Bytes);

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

#[apply(schema!)]
pub struct RequestStartTime(pub String);
impl DynamicType for RequestStartTime {
	fn auto_materialize(&self) -> bool {
		self.0.auto_materialize()
	}
	fn materialize(&self) -> Value<'_> {
		self.0.materialize()
	}
	fn field(&self, field: &str) -> Option<Value<'_>> {
		self.0.field(field)
	}
}
impl PartialEq for RequestRef<'_> {
	fn eq(&self, _: &Self) -> bool {
		// Currently do not allow comparisons
		false
	}
}

#[apply(schema!)]
#[derive(Eq, PartialEq, cel::DynamicType)]
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
	/// The number of tokens in the input/prompt read from cache (savings)
	#[dynamic(rename = "cachedInputTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cached_input_tokens: Option<u64>,
	/// Tokens written to cache (costs)
	/// Not present with OpenAI
	#[dynamic(rename = "cacheCreationInputTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_creation_input_tokens: Option<u64>,
	/// The number of tokens in the output/completion.
	#[dynamic(rename = "outputTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_tokens: Option<u64>,
	/// The number of reasoning tokens in the output/completion.
	#[dynamic(rename = "reasoningTokens")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reasoning_tokens: Option<u64>,
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
			reasoning_tokens: resp.reasoning_tokens,
			cached_input_tokens: resp.cached_input_tokens,
			cache_creation_input_tokens: resp.cache_creation_input_tokens,
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
			reasoning_tokens: None,
			cached_input_tokens: None,
			cache_creation_input_tokens: None,
		}
	}
}

fn to_value_str<'a, T: AsRef<str>>(c: &'a &'a T) -> Value<'a> {
	Value::String(c.as_ref().into())
}
fn to_value_str_opt<'a, T: AsRef<str>>(c: &'a Option<&'a T>) -> Value<'a> {
	match c {
		None => Value::Null,
		Some(c) => Value::String(c.as_ref().into()),
	}
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
/// JSON -> ExecutorSerde -> Executor<'_> -> CEL -> JSON should be consistent.
#[apply(schema!)]
#[derive(Default)]
pub struct ExecutorSerde {
	/// `request` contains attributes about the incoming HTTP request
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request: Option<RequestRefSerde>,

	/// `response` contains attributes about the HTTP response
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response: Option<ResponseRefSerde>,

	/// `jwt` contains the claims from a verified JWT token. This is only present if the JWT policy is enabled.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwt: Option<jwt::Claims>,

	/// `apiKey` contains the claims from a verified API Key. This is only present if the API Key policy is enabled.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub api_key: Option<apikey::Claims>,

	/// `basicAuth` contains the claims from a verified basic authentication Key. This is only present if the Basic authentication policy is enabled.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub basic_auth: Option<basicauth::Claims>,

	/// `llm` contains attributes about an LLM request or response. This is only present when using an `ai` backend.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub llm: Option<LLMContext>,

	/// `llm_request` contains the raw LLM request before processing. This is only present *during* LLM policies;
	/// policies occurring after the LLM policy, such as logs, will not have this field present even for LLM requests.
	#[serde(rename = "llmRequest", skip_serializing_if = "Option::is_none")]
	pub llm_request: Option<serde_json::Value>,

	/// `source` contains attributes about the source of the request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub source: Option<SourceContext>,

	/// `mcp` contains attributes about the MCP request.
	// This is only included for schema generation; see build_with_mcp.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub mcp: Option<ResourceType>,

	/// `backend` contains information about the backend being used.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub backend: Option<BackendContext>,

	/// `extauthz` contains dynamic metadata from ext_authz filters
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub extauthz: Option<ExtAuthzDynamicMetadata>,

	/// `extproc` contains dynamic metadata from ext_proc filters
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub extproc: Option<ExtProcDynamicMetadata>,
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
				host: req.host.as_ref(),
				scheme: req.scheme.as_ref(),
				version: req.version,
				headers: &req.headers,
				body: ExtensionOrDirect::Direct(req.body.as_ref()),
				start_time: ExtensionOrDirect::Direct(req.start_time.as_ref()),
				end_time: req.end_time.as_deref(),
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
		exec.llm_request = self.llm_request.as_ref();

		// Set all the ExtensionOrDirect fields
		exec.source = ExtensionOrDirect::Direct(self.source.as_ref());
		exec.jwt = ExtensionOrDirect::Direct(self.jwt.as_ref());
		exec.api_key = ExtensionOrDirect::Direct(self.api_key.as_ref());
		exec.basic_auth = ExtensionOrDirect::Direct(self.basic_auth.as_ref());
		exec.llm = ExtensionOrDirect::Direct(self.llm.as_ref());
		exec.backend = ExtensionOrDirect::Direct(self.backend.as_ref());
		exec.extauthz = ExtensionOrDirect::Direct(self.extauthz.as_ref());
		exec.extproc = ExtensionOrDirect::Direct(self.extproc.as_ref());
		exec.mcp = self.mcp.as_ref();

		exec
	}
}

pub fn full_example_executor() -> ExecutorSerde {
	let mut req_headers = HeaderMap::new();
	req_headers.insert("foo", "bar".parse().unwrap());
	req_headers.insert("user-agent", "example".parse().unwrap());
	req_headers.insert("accept", "application/json".parse().unwrap());
	let mut resp_headers = HeaderMap::new();
	resp_headers.insert("content-type", "application/json".parse().unwrap());

	ExecutorSerde {
		request: Some(RequestRefSerde {
			method: Method::GET,
			uri: "http://example.com/api/test".parse::<Uri>().unwrap(),
			host: Some("example.com".parse().unwrap()),
			scheme: Some(::http::uri::Scheme::HTTP),
			path: "/api/test".to_string(),
			version: Version::HTTP_11,
			headers: req_headers,
			body: Some(BufferedBody(Bytes::from(r#"{"model": "fast"}"#))),
			start_time: Some(RequestStartTime("2000-01-01T12:00:00Z".to_string())),
			end_time: Some("2000-01-01T12:00:01Z".to_string()),
		}),
		response: Some(ResponseRefSerde {
			code: 200,
			headers: resp_headers,
			body: Some(BufferedBody(Bytes::from(r#"{"ok": true}"#))),
		}),
		source: Some(SourceContext {
			address: "127.0.0.1".parse().unwrap(),
			port: 12345,
			tls: Some(TlsInfo {
				identity: None,
				subject_alt_names: vec![],
				issuer: Default::default(),
				subject: Default::default(),
				subject_cn: Some("cn".into()),
			}),
		}),
		jwt: Some(jwt::Claims {
			inner: serde_json::Map::from_iter(vec![
				("sub".to_string(), json!("test-user")),
				("iss".to_string(), json!("agentgateway.dev")),
				("exp".to_string(), json!(1900650294)),
			]),
			jwt: SecretString::new("fake.jwt.token".into()),
		}),
		api_key: Some(apikey::Claims {
			key: apikey::APIKey::new("test-api-key-id"),
			metadata: json!({"role": "admin"}),
		}),
		basic_auth: Some(basicauth::Claims {
			username: "alice".into(),
		}),
		llm_request: Some(json!({
			"model": "provider/model"
		})),
		llm: Some(LLMContext {
			streaming: false,
			request_model: "gpt-4".into(),
			response_model: Some("gpt-4-turbo".into()),
			provider: "fake-ai".into(),
			input_tokens: Some(100),
			cached_input_tokens: Some(20),
			cache_creation_input_tokens: Some(10),
			output_tokens: Some(50),
			reasoning_tokens: Some(30),
			total_tokens: Some(150),
			first_token: None,
			count_tokens: Some(10),

			prompt: None,
			completion: Some(vec!["Hello".to_string()]),
			params: llm::LLMRequestParams {
				temperature: Some(0.7),
				top_p: Some(1.0),
				frequency_penalty: Some(0.0),
				presence_penalty: Some(0.0),
				seed: Some(42),
				max_tokens: Some(1024),
				encoding_format: None,
				dimensions: None,
			},
		}),
		mcp: Some(ResourceType::Tool(ResourceId::new(
			"my-mcp-server".to_string(),
			"get_weather".to_string(),
		))),
		backend: Some(BackendContext {
			name: "my-backend".into(),
			backend_type: BackendType::Service,
			protocol: BackendProtocol::http,
		}),
		extauthz: Some(ExtAuthzDynamicMetadata::default()),
		extproc: Some(ExtProcDynamicMetadata::default()),
	}
}

#[cfg(test)]
#[path = "types_test.rs"]
mod types_test;
