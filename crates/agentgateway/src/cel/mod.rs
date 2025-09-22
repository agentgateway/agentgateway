// Portions of this code are heavily inspired from https://github.com/Kuadrant/wasm-shim/
// Under Apache 2.0 license (https://github.com/Kuadrant/wasm-shim/blob/main/LICENSE)

use std::collections::HashSet;
use std::fmt::{Debug, Display, Formatter};
use std::net::IpAddr;
use std::sync::Arc;

use agent_core::strng::Strng;
use bytes::Bytes;
pub use cel::Value;
use cel::objects::Key;
use cel::{Context, ExecutionError, ParseError, ParseErrors, Program};
pub use functions::{FLATTEN_LIST, FLATTEN_LIST_RECURSIVE, FLATTEN_MAP, FLATTEN_MAP_RECURSIVE};
use once_cell::sync::Lazy;
use serde::{Serialize, Serializer};

use crate::http::jwt::Claims;
use crate::llm;
use crate::llm::{LLMRequest, LLMResponse};
use crate::serdes::*;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::discovery::Identity;

mod functions;
mod strings;

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
pub const RESPONSE_ATTRIBUTE: &str = "response";
pub const JWT_ATTRIBUTE: &str = "jwt";
pub const MCP_ATTRIBUTE: &str = "mcp";
pub const EXTAUTHZ_ATTRIBUTE: &str = "extauthz";
pub const ALL_ATTRIBUTES: &[&str] = &[
	SOURCE_ATTRIBUTE,
	REQUEST_ATTRIBUTE,
	REQUEST_BODY_ATTRIBUTE,
	LLM_ATTRIBUTE,
	LLM_PROMPT_ATTRIBUTE,
	LLM_COMPLETION_ATTRIBUTE,
	RESPONSE_ATTRIBUTE,
	JWT_ATTRIBUTE,
	MCP_ATTRIBUTE,
	EXTAUTHZ_ATTRIBUTE,
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

impl Debug for Expression {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Expression")
			.field("expression", &self.original_expression)
			.finish()
	}
}

fn root_context() -> Arc<Context<'static>> {
	let mut ctx = Context::default();
	functions::insert_all(&mut ctx);
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
	pub fn with_request(&mut self, req: &crate::http::Request) -> bool {
		if !self.attributes.contains(REQUEST_ATTRIBUTE) {
			return false;
		}
		self.context.request = Some(RequestContext {
			method: req.method().clone(),
			// TODO: split headers and the rest?
			headers: req.headers().clone(),
			uri: req.uri().clone(),
			path: req.uri().path().to_string(),
			body: None,
		});
		self.attributes.contains(REQUEST_BODY_ATTRIBUTE)
	}
	pub fn with_response(&mut self, resp: &crate::http::Response) {
		if !self.attributes.contains(RESPONSE_ATTRIBUTE) {
			return;
		}
		self.context.response = Some(ResponseContext {
			code: resp.status(),
		})
	}

	pub fn with_jwt(&mut self, info: &Claims) {
		if !self.attributes.contains(JWT_ATTRIBUTE) {
			return;
		}
		self.context.jwt = Some(info.clone())
	}

	pub fn with_extauthz(&mut self, req: &crate::http::Request) {
		if !self.attributes.contains(EXTAUTHZ_ATTRIBUTE) {
			return;
		}

		// Extract dynamic metadata from ext_authz if present
		if let Some(ext_authz_metadata) = req
			.extensions()
			.get::<Arc<crate::http::ext_authz::ExtAuthzDynamicMetadata>>()
		{
			// Direct access to extauthz.field - metadata is already stored flat
			if !ext_authz_metadata.metadata.is_empty() {
				self.context.extauthz = Some(ext_authz_metadata.metadata.clone());
			}
		}
	}

	pub fn with_source(&mut self, tcp: &TCPConnectionInfo, tls: Option<&TLSConnectionInfo>) {
		if !self.attributes.contains(SOURCE_ATTRIBUTE) {
			return;
		}
		self.context.source = Some(SourceContext {
			address: tcp.peer_addr.ip(),
			port: tcp.peer_addr.port(),
			identity: tls.and_then(|t| t.src_identity.as_ref()).map(|m| match m {
				Identity::Spiffe {
					trust_domain,
					namespace,
					service_account,
				} => IdentityContext {
					trust_domain: trust_domain.clone(),
					namespace: namespace.clone(),
					service_account: service_account.clone(),
				},
			}),
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

	pub fn with_llm_response(&mut self, info: &LLMResponse) {
		if !self.attributes.contains(LLM_ATTRIBUTE) {
			return;
		}
		if let Some(o) = self.context.llm.as_mut() {
			o.output_tokens = info.output_tokens;
			o.total_tokens = info.total_tokens;
			if let Some(pt) = info.input_tokens_from_response {
				// Better info, override
				o.input_tokens = Some(pt);
			}
			o.response_model = info.provider_model.clone();
			// Not always set
			o.completion = info.completion.clone();
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
			llm,
			source,
			mcp: _,
			extauthz,
		} = &self.context;

		ctx.add_variable_from_value(REQUEST_ATTRIBUTE, opt_to_value(request)?);
		ctx.add_variable_from_value(RESPONSE_ATTRIBUTE, opt_to_value(response)?);
		ctx.add_variable_from_value(JWT_ATTRIBUTE, opt_to_value(jwt)?);
		ctx.add_variable_from_value(MCP_ATTRIBUTE, opt_to_value(&mcp)?);
		ctx.add_variable_from_value(LLM_ATTRIBUTE, opt_to_value(llm)?);
		ctx.add_variable_from_value(SOURCE_ATTRIBUTE, opt_to_value(source)?);
		ctx.add_variable_from_value(EXTAUTHZ_ATTRIBUTE, opt_to_value(extauthz)?);

		Ok(Executor { ctx })
	}

	pub fn build(&self) -> Result<Executor<'static>, Error> {
		self.build_with_mcp(None)
	}
}

impl Executor<'_> {
	pub fn eval(&self, expr: &Expression) -> Result<Value, Error> {
		Ok(expr.expression.execute(&self.ctx)?)
	}
	pub fn eval_bool(&self, expr: &Expression) -> bool {
		match self.eval(expr) {
			Ok(Value::Bool(b)) => b,
			_ => false,
		}
	}
}

pub struct Executor<'a> {
	ctx: Context<'a>,
}
impl Expression {
	pub fn new(original_expression: impl Into<String>) -> Result<Self, Error> {
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
				["llm", "prompt", ..] => vec![LLM_ATTRIBUTE.to_string(), LLM_PROMPT_ATTRIBUTE.to_string()],
				["llm", "completion", ..] => vec![
					LLM_ATTRIBUTE.to_string(),
					LLM_COMPLETION_ATTRIBUTE.to_string(),
				],
				["extauthz", ..] => vec![EXTAUTHZ_ATTRIBUTE.to_string()],
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
	pub jwt: Option<Claims>,
	/// `llm` contains attributes about an LLM request or response. This is only present when using an `ai` backend.
	pub llm: Option<LLMContext>,
	/// `source` contains attributes about the source of the request.
	pub source: Option<SourceContext>,
	/// `mcp` contains attributes about the MCP request.
	// This is only included for schema generation; see build_with_mcp.
	pub mcp: Option<crate::mcp::ResourceType>,
	/// `extauthz` contains dynamic metadata from ext_authz filters
	pub extauthz: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[apply(schema_ser!)]
pub struct RequestContext {
	#[serde(with = "http_serde::method")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	/// The HTTP method of the request.
	pub method: ::http::Method,

	#[serde(with = "http_serde::uri")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	/// The URI of the request.
	pub uri: ::http::Uri,

	/// The path of the request URI.
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
}

#[apply(schema_ser!)]
pub struct ResponseContext {
	#[serde(with = "http_serde::status_code")]
	#[cfg_attr(feature = "schema", schemars(with = "u16"))]
	/// The HTTP status code of the response.
	pub code: ::http::StatusCode,
}

#[apply(schema_ser!)]
pub struct SourceContext {
	/// The IP address of the downstream connection.
	address: IpAddr,
	/// The port of the downstream connection.
	port: u16,
	/// The (Istio SPIFFE) identity of the downstream connection, if available.
	identity: Option<IdentityContext>,
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

#[cfg(any(test, feature = "internal_benches"))]
#[path = "tests.rs"]
mod tests;
