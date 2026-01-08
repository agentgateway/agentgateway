// Portions of this code are heavily inspired from https://github.com/Kuadrant/wasm-shim/
// Under Apache 2.0 license (https://github.com/Kuadrant/wasm-shim/blob/main/LICENSE)

use std::collections::HashSet;
use std::fmt::{Debug, Display, Formatter};
use std::net::IpAddr;
use std::sync::Arc;

use agent_core::strng::Strng;
use bytes::Bytes;
pub use cel::Value;
use cel::{Context, ExecutionError, ParseError, ParseErrors, Program};
use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelValue;
use serde::{Deserialize, Serialize, Serializer};
use tracing::log::debug;

use crate::http::{apikey, basicauth, jwt};
use crate::llm;
use crate::llm::{LLMInfo, LLMRequest};
use crate::serdes::*;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::BackendInfo;

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("execution: {0}")]
	Resolve(#[from] ExecutionError),
	#[error("parse: {0}")]
	Parse(#[from] ParseError),
	#[error("parse: {0}")]
	Parses(#[from] ParseErrors),
	#[error("variable: {0}")]
	Variable(String),
	#[error("failed to convert to json")]
	JsonConvert,
}

impl From<Box<dyn std::error::Error>> for Error {
	fn from(value: Box<dyn std::error::Error>) -> Self {
		Self::Variable(value.to_string())
	}
}

pub const SOURCE_ATTRIBUTE: &str = "source";
pub const REQUEST_ATTRIBUTE: &str = "request";
pub const REQUEST_BODY_ATTRIBUTE: &str = "request.body";
pub const LLM_ATTRIBUTE: &str = "llm";
pub const LLM_PROMPT_ATTRIBUTE: &str = "llm.prompt";
pub const LLM_COMPLETION_ATTRIBUTE: &str = "llm.completion";
pub const BACKEND_ATTRIBUTE: &str = "backend";
pub const RESPONSE_ATTRIBUTE: &str = "response";
pub const RESPONSE_BODY_ATTRIBUTE: &str = "response.body";
pub const JWT_ATTRIBUTE: &str = "jwt";
pub const API_KEY_ATTRIBUTE: &str = "apiKey";
pub const BASIC_AUTH_ATTRIBUTE: &str = "basicAuth";
pub const MCP_ATTRIBUTE: &str = "mcp";
pub const EXTAUTHZ_ATTRIBUTE: &str = "extauthz";
pub const EXTPROC_ATTRIBUTE: &str = "extproc";
pub const ALL_ATTRIBUTES: &[&str] = &[
	SOURCE_ATTRIBUTE,
	REQUEST_ATTRIBUTE,
	REQUEST_BODY_ATTRIBUTE,
	LLM_ATTRIBUTE,
	LLM_PROMPT_ATTRIBUTE,
	LLM_COMPLETION_ATTRIBUTE,
	BACKEND_ATTRIBUTE,
	RESPONSE_ATTRIBUTE,
	RESPONSE_BODY_ATTRIBUTE,
	JWT_ATTRIBUTE,
	API_KEY_ATTRIBUTE,
	BASIC_AUTH_ATTRIBUTE,
	MCP_ATTRIBUTE,
	EXTAUTHZ_ATTRIBUTE,
	EXTPROC_ATTRIBUTE,
];

pub struct Expression {
	attributes: HashSet<String>,
	expression: Program,
	original_expression: String,
}

impl Serialize for Expression {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&self.original_expression)
	}
}

impl<'de> Deserialize<'de> for Expression {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let e = String::deserialize(deserializer)?;
		// For local configs, we treat CEL as strict parsing
		crate::cel::Expression::new_strict(&e).map_err(|e| serde::de::Error::custom(e.to_string()))
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Expression {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"Expression".into()
	}

	fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({ "type": "string" })
	}
}

impl Debug for Expression {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Expression")
			.field("expression", &self.original_expression)
			.finish()
	}
}

fn root_context() -> Arc<Context<'static>> {
	let mut ctx = Context::default();
	agent_celx::insert_all(&mut ctx);
	Arc::new(ctx)
}

static ROOT_CONTEXT: Lazy<Arc<Context<'static>>> = Lazy::new(root_context);

#[derive(Debug)]
pub struct ContextBuilder {
	pub attributes: HashSet<String>,
	pub context: ExpressionContext,
}

impl Default for ContextBuilder {
	fn default() -> Self {
		Self::new()
	}
}

impl ContextBuilder {
	/// expensive_clone is just a normal clone but more clear that its NOT a cheap ref count
	pub fn expensive_clone(&self) -> Self {
		Self {
			attributes: self.attributes.clone(),
			context: self.context.clone(),
		}
	}
	pub fn new() -> Self {
		Self {
			attributes: Default::default(),
			context: Default::default(),
		}
	}
	/// register_expression registers the given expressions attributes as required attributes.
	/// Callers MUST call this for each expression they wish to call with the context if they want correct results.
	pub fn register_expression(&mut self, expression: &Expression) {
		self
			.attributes
			.extend(expression.attributes.iter().cloned());
	}
	pub fn with_request_body(&mut self, body: Bytes) {
		let Some(r) = &mut self.context.request else {
			return;
		};
		r.body = Some(body);
	}
	pub fn with_response_body(&mut self, body: Bytes) {
		let Some(r) = &mut self.context.response else {
			return;
		};
		r.body = Some(body);
	}
	pub fn with_request(&mut self, req: &crate::http::Request, start_time: String) -> bool {
		if !self.attributes.contains(REQUEST_ATTRIBUTE) {
			return false;
		}
		if let Some(r) = self.context.request.as_ref() {
			return r.body.is_none() && self.attributes.contains(REQUEST_BODY_ATTRIBUTE);
		}
		self.context.request = Some(RequestContext {
			method: req.method().clone(),
			// TODO: split headers and the rest?
			headers: req.headers().clone(),
			uri: req.uri().clone(),
			host: req.uri().authority().cloned(),
			scheme: req.uri().scheme().cloned(),
			path: req.uri().path().to_string(),
			body: None,
			end_time: None,
			start_time,
		});
		self.attributes.contains(REQUEST_BODY_ATTRIBUTE)
	}

	pub fn with_response(&mut self, resp: &crate::http::Response) -> bool {
		if !self.attributes.contains(RESPONSE_ATTRIBUTE) {
			return false;
		}
		self.context.response = Some(ResponseContext {
			code: resp.status(),
			headers: resp.headers().clone(),
			body: None,
		});
		self.attributes.contains(RESPONSE_BODY_ATTRIBUTE)
	}

	pub fn with_jwt(&mut self, info: &jwt::Claims) {
		if !self.attributes.contains(JWT_ATTRIBUTE) {
			return;
		}
		self.context.jwt = Some(info.clone())
	}

	pub fn with_api_key(&mut self, info: &apikey::Claims) {
		if !self.attributes.contains(API_KEY_ATTRIBUTE) {
			return;
		}
		self.context.api_key = Some(info.clone())
	}

	pub fn with_basic_auth(&mut self, info: &basicauth::Claims) {
		if !self.attributes.contains(BASIC_AUTH_ATTRIBUTE) {
			return;
		}
		self.context.basic_auth = Some(info.clone())
	}

	// returns true if there were any changes made
	pub fn with_extauthz(&mut self, req: &crate::http::Request) -> bool {
		if !self.attributes.contains(EXTAUTHZ_ATTRIBUTE) {
			return false;
		}

		// Extract dynamic metadata from ext_authz if present
		if let Some(ext_authz_metadata) = req
			.extensions()
			.get::<Arc<crate::http::ext_authz::ExtAuthzDynamicMetadata>>()
		{
			// Direct access to extauthz.field - metadata is already stored flat
			if !ext_authz_metadata.metadata.is_empty() {
				if let Some(existing) = &mut self.context.extauthz {
					for (k, v) in ext_authz_metadata.metadata.iter() {
						existing.insert(k.clone(), v.clone());
					}
				} else {
					self.context.extauthz = Some(ext_authz_metadata.metadata.clone());
				}
			}
			true
		} else {
			false
		}
	}

	/// Add ext_proc dynamic metadata to the context
	/// Returns true if there were any changes made
	pub fn with_extproc(&mut self, req: &crate::http::Request) -> bool {
		if !self.attributes.contains(EXTPROC_ATTRIBUTE) {
			return false;
		}

		// Extract dynamic metadata from ext_proc if present
		if let Some(ext_proc_metadata) = req
			.extensions()
			.get::<Arc<crate::http::ext_proc::ExtProcDynamicMetadata>>()
		{
			// Direct access to extproc.field - metadata is already stored flat
			if !ext_proc_metadata.metadata.is_empty() {
				if let Some(existing) = &mut self.context.extproc {
					for (k, v) in ext_proc_metadata.metadata.iter() {
						existing.insert(k.clone(), v.clone());
					}
				} else {
					self.context.extproc = Some(ext_proc_metadata.metadata.clone());
				}
			}
			true
		} else {
			false
		}
	}

	pub fn with_source(&mut self, tcp: &TCPConnectionInfo, tls: Option<&TLSConnectionInfo>) {
		if !self.attributes.contains(SOURCE_ATTRIBUTE) {
			return;
		}
		if self.context.source.is_some() {
			return;
		}
		self.context.source = Some(SourceContext {
			address: tcp.peer_addr.ip(),
			port: tcp.peer_addr.port(),
			tls: tls.and_then(|t| t.src_identity.clone()),
		})
	}

	pub fn with_llm_request(&mut self, info: &LLMRequest) -> bool {
		if !self.attributes.contains(LLM_ATTRIBUTE) {
			return false;
		}

		self.context.llm = Some(LLMContext {
			streaming: info.streaming,
			request_model: info.request_model.clone(),
			provider: info.provider.clone(),
			input_tokens: info.input_tokens,
			params: info.params.clone(),

			count_tokens: None,
			response_model: None,
			output_tokens: None,
			total_tokens: None,
			prompt: None,
			completion: None,
		});
		self.attributes.contains(LLM_PROMPT_ATTRIBUTE)
	}

	pub fn with_llm_prompt(&mut self, msg: Vec<llm::SimpleChatCompletionMessage>) {
		let Some(r) = &mut self.context.llm else {
			return;
		};
		r.prompt = Some(msg);
	}

	pub fn with_backend(&mut self, backend_info: &BackendInfo, backend_protocol: BackendProtocol) {
		if !self.attributes.contains(BACKEND_ATTRIBUTE) {
			return;
		}
		self.context.backend = Some(BackendContext {
			name: backend_info.backend_name.clone(),
			backend_type: backend_info.backend_type,
			protocol: backend_protocol,
		});
	}

	pub fn with_llm_response(&mut self, info: &LLMInfo) {
		if !self.attributes.contains(LLM_ATTRIBUTE) {
			return;
		}
		let resp = &info.response;
		if let Some(o) = self.context.llm.as_mut() {
			o.output_tokens = resp.output_tokens;
			o.count_tokens = resp.count_tokens;
			o.total_tokens = resp.total_tokens;
			if let Some(pt) = resp.input_tokens {
				// Better info, override
				o.input_tokens = Some(pt);
			}
			o.response_model = resp.provider_model.clone();
			// Not always set
			o.completion = resp.completion.clone();
		}
	}

	pub fn with_request_completion(&mut self, end_time: String) {
		if let Some(r) = self.context.request.as_mut() {
			r.end_time = Some(end_time);
		}
	}

	pub fn needs_llm_completion(&self) -> bool {
		self.attributes.contains(LLM_COMPLETION_ATTRIBUTE)
	}

	pub fn build_with_mcp(
		&self,
		mcp: Option<&crate::mcp::ResourceType>,
	) -> Result<Executor<'static>, Error> {
		let mut ctx: Context<'static> = ROOT_CONTEXT.new_inner_scope();

		let ExpressionContext {
			request,
			response,
			jwt,
			api_key,
			basic_auth,
			llm,
			source,
			mcp: _,
			backend,
			extauthz,
			extproc,
		} = &self.context;

		ctx.add_variable_from_value(REQUEST_ATTRIBUTE, opt_to_value(request)?);
		ctx.add_variable_from_value(RESPONSE_ATTRIBUTE, opt_to_value(response)?);
		ctx.add_variable_from_value(JWT_ATTRIBUTE, opt_to_value(jwt)?);
		ctx.add_variable_from_value(BASIC_AUTH_ATTRIBUTE, opt_to_value(basic_auth)?);
		ctx.add_variable_from_value(API_KEY_ATTRIBUTE, opt_to_value(api_key)?);
		ctx.add_variable_from_value(MCP_ATTRIBUTE, opt_to_value(&mcp)?);
		ctx.add_variable_from_value(BACKEND_ATTRIBUTE, opt_to_value(backend)?);
		ctx.add_variable_from_value(LLM_ATTRIBUTE, opt_to_value(llm)?);
		ctx.add_variable_from_value(SOURCE_ATTRIBUTE, opt_to_value(source)?);
		ctx.add_variable_from_value(EXTAUTHZ_ATTRIBUTE, opt_to_value(extauthz)?);
		ctx.add_variable_from_value(EXTPROC_ATTRIBUTE, opt_to_value(extproc)?);

		Ok(Executor { ctx })
	}

	pub fn build(&self) -> Result<Executor<'static>, Error> {
		self.build_with_mcp(None)
	}
}

impl Executor<'_> {
	pub fn eval(&self, expr: &Expression) -> Result<Value, Error> {
		match expr.expression.execute(&self.ctx) {
			Ok(v) => Ok(v),
			Err(e) => {
				tracing::trace!("failed to evaluate expression: {}", e);
				Err(e.into())
			},
		}
	}
	pub fn eval_bool(&self, expr: &Expression) -> bool {
		match self.eval(expr) {
			Ok(Value::Bool(b)) => b,
			_ => false,
		}
	}
}

pub fn value_as_bytes(v: &Value) -> Option<&[u8]> {
	match v {
		Value::String(b) => Some(b.as_bytes()),
		Value::Bytes(b) => Some(b.as_slice()),
		_ => None,
	}
}

pub fn value_as_int(v: &Value) -> Option<i64> {
	match v {
		Value::Int(b) => Some(*b),
		Value::UInt(b) => Some(i64::try_from(*b).ok()?),
		_ => None,
	}
}

pub fn value_as_string(v: &Value) -> Option<String> {
	match v {
		Value::String(v) => Some(v.to_string()),
		Value::Bool(v) => Some(v.to_string()),
		Value::Int(v) => Some(v.to_string()),
		Value::UInt(v) => Some(v.to_string()),
		Value::Bytes(v) => {
			use base64::Engine;
			Some(base64::prelude::BASE64_STANDARD.encode(v.as_ref()))
		},
		_ => None,
	}
}

pub fn value_as_header_value(v: &Value) -> Option<http::HeaderValue> {
	match v {
		Value::String(v) => Some(http::HeaderValue::from_str(v.as_str()).ok()?),
		Value::Bool(v) => Some(http::HeaderValue::from_str(&v.to_string()).ok()?),
		Value::Int(v) => Some(http::HeaderValue::from_str(&v.to_string()).ok()?),
		Value::UInt(v) => Some(http::HeaderValue::from_str(&v.to_string()).ok()?),
		Value::Bytes(v) => {
			use base64::Engine;
			let b = base64::prelude::BASE64_STANDARD.encode(v.as_ref());
			Some(http::HeaderValue::from_str(&b).ok()?)
		},
		_ => None,
	}
}

pub struct Executor<'a> {
	ctx: Context<'a>,
}
impl Expression {
	/// new_permissive compiles the expression. If the expression cannot be compiled, its instead replaced
	/// with an expression that always fails to evaluate
	pub fn new_permissive(original_expression: impl Into<String>) -> Self {
		let expr = original_expression.into();
		match Self::new_strict(&expr) {
			Ok(ok) => ok,
			Err(err) => {
				debug!("ignoring failed expression: {}", err);
				Self {
					attributes: Default::default(),
					expression: Self::new_strict("fail('the expression could not be compiled')")
						.expect("must be valid")
						.expression,
					original_expression: expr,
				}
			},
		}
	}
	/// new_strict compiles the expression, and returns an error if its invalid.
	pub fn new_strict(original_expression: impl Into<String>) -> Result<Self, Error> {
		let original_expression = original_expression.into();
		let expression = Program::compile(&original_expression)?;

		let mut props: Vec<Vec<&str>> = Vec::with_capacity(5);
		properties(
			&expression.expression().expr,
			&mut props,
			&mut Vec::default(),
		);

		let include_all = expression.references().functions().contains(&"variables");
		// For now we only look at the first level. We could be more precise
		let mut attributes: HashSet<String> = props
			.into_iter()
			.flat_map(|tokens| match tokens.as_slice() {
				["request", "body", ..] => vec![
					REQUEST_ATTRIBUTE.to_string(),
					REQUEST_BODY_ATTRIBUTE.to_string(),
				],
				["response", "body", ..] => vec![
					RESPONSE_ATTRIBUTE.to_string(),
					RESPONSE_BODY_ATTRIBUTE.to_string(),
				],
				["llm", "prompt", ..] => vec![LLM_ATTRIBUTE.to_string(), LLM_PROMPT_ATTRIBUTE.to_string()],
				["llm", "completion", ..] => vec![
					LLM_ATTRIBUTE.to_string(),
					LLM_COMPLETION_ATTRIBUTE.to_string(),
				],
				["extauthz", ..] => vec![EXTAUTHZ_ATTRIBUTE.to_string()],
				["extproc", ..] => vec![EXTPROC_ATTRIBUTE.to_string()],
				[first, ..] => vec![first.to_string()],
				_ => Vec::default(),
			})
			.collect();
		if include_all {
			ALL_ATTRIBUTES.iter().for_each(|attr| {
				attributes.insert(attr.to_string());
			});
		}

		Ok(Self {
			attributes,
			expression,
			original_expression,
		})
	}
}

#[derive(Default)]
#[apply(schema_ser!)]
pub struct ExpressionContext {
	/// `request` contains attributes about the incoming HTTP request
	pub request: Option<RequestContext>,
	/// `response` contains attributes about the HTTP response
	pub response: Option<ResponseContext>,
	/// `jwt` contains the claims from a verified JWT token. This is only present if the JWT policy is enabled.
	pub jwt: Option<jwt::Claims>,
	/// `apiKey` contains the claims from a verified API Key. This is only present if the API Key policy is enabled.
	pub api_key: Option<apikey::Claims>,
	/// `basicAuth` contains the claims from a verified basic authentication Key. This is only present if the Basic authentication policy is enabled.
	pub basic_auth: Option<basicauth::Claims>,
	/// `llm` contains attributes about an LLM request or response. This is only present when using an `ai` backend.
	pub llm: Option<LLMContext>,
	/// `source` contains attributes about the source of the request.
	pub source: Option<SourceContext>,
	/// `mcp` contains attributes about the MCP request.
	// This is only included for schema generation; see build_with_mcp.
	pub mcp: Option<crate::mcp::ResourceType>,
	/// `backend` contains information about the backend being used.
	pub backend: Option<BackendContext>,
	/// `extauthz` contains dynamic metadata from ext_authz filters
	pub extauthz: Option<std::collections::HashMap<String, serde_json::Value>>,
	/// `extproc` contains dynamic metadata from ext_proc filters
	pub extproc: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[apply(schema_ser!)]
pub struct RequestContext {
	#[serde(with = "http_serde::method")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	/// The HTTP method of the request. For example, `GET`
	pub method: ::http::Method,

	#[serde(with = "http_serde::uri")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	/// The complete URI of the request. For example, `http://example.com/path`.
	pub uri: ::http::Uri,

	#[serde(with = "http_serde::option::authority")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub host: Option<::http::uri::Authority>,

	#[serde(with = "http_serde::option::scheme")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub scheme: Option<::http::uri::Scheme>,

	/// The path of the request URI. For example, `/path`.
	pub path: String,

	#[serde(with = "http_serde::header_map")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	/// The headers of the request.
	pub headers: ::http::HeaderMap,

	/// The body of the request. Warning: accessing the body will cause the body to be buffered.
	pub body: Option<Bytes>,

	/// The (pre-rendered) time the request started
	pub start_time: String,

	/// The (pre-rendered) time the request completed
	pub end_time: Option<String>,
}

#[apply(schema_ser!)]
pub struct ResponseContext {
	#[serde(with = "http_serde::status_code")]
	#[cfg_attr(feature = "schema", schemars(with = "u16"))]
	/// The HTTP status code of the response.
	pub code: ::http::StatusCode,

	#[serde(with = "http_serde::header_map")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	/// The headers of the request.
	pub headers: ::http::HeaderMap,

	/// The body of the response. Warning: accessing the body will cause the body to be buffered.
	pub body: Option<Bytes>,
}

#[apply(schema_ser!)]
pub struct SourceContext {
	/// The IP address of the downstream connection.
	address: IpAddr,
	/// The port of the downstream connection.
	port: u16,
	/// The (Istio SPIFFE) identity of the downstream connection, if available.
	#[serde(flatten)]
	tls: Option<crate::transport::tls::TlsInfo>,
}

#[apply(schema_ser!)]
pub struct IdentityContext {
	/// The trust domain of the identity.
	trust_domain: Strng,
	/// The namespace of the identity.
	namespace: Strng,
	/// The service account of the identity.
	service_account: Strng,
}

#[apply(schema_ser!)]
pub struct BackendContext {
	/// The name of the backend being used. For example, `my-service` or `service/my-namespace/my-service:8080`.
	pub name: Strng,
	/// The type of backend. For example, `ai`, `mcp`, `static`, `dynamic`, or `service`.
	#[serde(rename = "type")]
	pub backend_type: BackendType,
	/// The protocol of backend. For example, `http`, `tcp`, `a2a`, `mcp`, or `llm`.
	pub protocol: BackendProtocol,
}

#[derive(Copy, PartialEq, Eq, Hash, Debug, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum BackendType {
	AI,
	MCP,
	Static,
	Dynamic,
	Service,
	Unknown,
}

#[derive(Copy, PartialEq, Eq, Hash, EncodeLabelValue, Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[allow(non_camel_case_types)]
pub enum BackendProtocol {
	http,
	tcp,
	a2a,
	mcp,
	llm,
}

#[apply(schema_ser!)]
pub struct LLMContext {
	/// Whether the LLM response is streamed.
	streaming: bool,
	/// The model requested for the LLM request. This may differ from the actual model used.
	request_model: Strng,
	/// The model that actually served the LLM response.
	#[serde(skip_serializing_if = "Option::is_none")]
	response_model: Option<Strng>,
	/// The provider of the LLM.
	provider: Strng,
	/// The number of tokens in the input/prompt.
	#[serde(skip_serializing_if = "Option::is_none")]
	input_tokens: Option<u64>,
	/// The number of tokens in the output/completion.
	#[serde(skip_serializing_if = "Option::is_none")]
	output_tokens: Option<u64>,
	/// The total number of tokens for the request.
	#[serde(skip_serializing_if = "Option::is_none")]
	total_tokens: Option<u64>,
	/// The number of tokens in the request, when using the token counting endpoint
	/// These are not counted as 'input tokens' since they do not consume input tokens.
	#[serde(skip_serializing_if = "Option::is_none")]
	count_tokens: Option<u64>,
	/// The prompt sent to the LLM. Warning: accessing this has some performance impacts for large prompts.
	#[serde(skip_serializing_if = "Option::is_none")]
	prompt: Option<Vec<llm::SimpleChatCompletionMessage>>,
	/// The completion from the LLM. Warning: accessing this has some performance impacts for large responses.
	#[serde(skip_serializing_if = "Option::is_none")]
	completion: Option<Vec<String>>,
	/// The parameters for the LLM request.
	params: llm::LLMRequestParams,
}

fn properties<'e>(
	exp: &'e cel::common::ast::Expr,
	all: &mut Vec<Vec<&'e str>>,
	path: &mut Vec<&'e str>,
) {
	use cel::common::ast::Expr::*;
	match exp {
		Unspecified => {},
		Call(call) => {
			if let Some(t) = &call.target {
				properties(&t.expr, all, path)
			}
			for arg in &call.args {
				properties(&arg.expr, all, path)
			}
		},
		Select(e) => {
			path.insert(0, e.field.as_str());
			properties(&e.operand.expr, all, path);
		},
		Comprehension(call) => {
			properties(&call.iter_range.expr, all, path);
			{
				let v = &call.iter_var;
				if !v.starts_with("@") {
					path.insert(0, v.as_str());
					all.push(path.clone());
					path.clear();
				}
			}
			properties(&call.loop_step.expr, all, path);
		},
		List(e) => {
			for elem in &e.elements {
				properties(&elem.expr, all, path);
			}
		},
		Map(v) => {
			for entry in &v.entries {
				match &entry.expr {
					cel::common::ast::EntryExpr::StructField(field) => {
						properties(&field.value.expr, all, path);
					},
					cel::common::ast::EntryExpr::MapEntry(map_entry) => {
						properties(&map_entry.value.expr, all, path);
					},
				}
			}
		},
		Struct(v) => {
			for entry in &v.entries {
				match &entry.expr {
					cel::common::ast::EntryExpr::StructField(field) => {
						properties(&field.value.expr, all, path);
					},
					cel::common::ast::EntryExpr::MapEntry(map_entry) => {
						properties(&map_entry.value.expr, all, path);
					},
				}
			}
		},
		Literal(_) => {},
		// Inline(_) => {},
		Ident(v) => {
			if !v.starts_with("@") {
				path.insert(0, v.as_str());
				all.push(path.clone());
				path.clear();
			}
		},
	}
}

pub struct Attribute {
	path: Path,
}

impl Debug for Attribute {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "Attribute {{ {:?} }}", self.path)
	}
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Path {
	tokens: Vec<String>,
}

impl Display for Path {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{}",
			self
				.tokens
				.iter()
				.map(|t| t.replace('.', "\\."))
				.collect::<Vec<String>>()
				.join(".")
		)
	}
}

impl Debug for Path {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "path: {:?}", self.tokens)
	}
}

impl From<&str> for Path {
	fn from(value: &str) -> Self {
		let mut token = String::new();
		let mut tokens: Vec<String> = Vec::new();
		let mut chars = value.chars();
		while let Some(ch) = chars.next() {
			match ch {
				'.' => {
					tokens.push(token);
					token = String::new();
				},
				'\\' => {
					if let Some(next) = chars.next() {
						token.push(next);
					}
				},
				_ => token.push(ch),
			}
		}
		tokens.push(token);

		Self { tokens }
	}
}

impl Path {
	pub fn new<T: Into<String>>(tokens: Vec<T>) -> Self {
		Self {
			tokens: tokens.into_iter().map(|i| i.into()).collect(),
		}
	}
	pub fn tokens(&self) -> Vec<&str> {
		self.tokens.iter().map(String::as_str).collect()
	}
}

fn opt_to_value<S: Serialize>(v: &Option<S>) -> Result<Value, Error> {
	Ok(v.as_ref().map(to_value).transpose()?.unwrap_or(Value::Null))
}

fn to_value(v: impl Serialize) -> Result<Value, Error> {
	cel::to_value(v).map_err(|e| Error::Variable(e.to_string()))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(any(test, feature = "internal_benches"))]
#[path = "benches.rs"]
mod benches;
