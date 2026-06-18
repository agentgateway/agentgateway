// Portions of this code are heavily inspired from https://github.com/Kuadrant/wasm-shim/
// Under Apache 2.0 license (https://github.com/Kuadrant/wasm-shim/blob/main/LICENSE)

use std::fmt::{Debug, Formatter};
use std::sync::OnceLock;

pub use cel::Value;
pub use cel::types::dynamic::DynamicType;
use cel::{Context, ExecutionError, ParseError, ParseErrors, Program};
use flagset::FlagSet;
pub use helpers::*;
use serde::{Deserialize, Serialize, Serializer};
use tracing::log::debug;
pub use types::*;

mod custom;
mod helpers;
mod types;

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

pub struct Expression {
	attributes: FlagSet<Attributes>,
	expression: Program,
	pub original_expression: String,
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

struct RootContext {
	context: Context,
	registry: custom::Registry,
}

static ROOT_CONTEXT: OnceLock<RootContext> = OnceLock::new();

fn context() -> &'static Context {
	&ROOT_CONTEXT
		.get_or_init(|| {
			let mut ctx = Context::default();
			agent_celx::insert_all(&mut ctx);
			RootContext {
				context: ctx,
				registry: custom::Registry::default(),
			}
		})
		.context
}

pub fn register_custom_functions(definitions: &str) -> Result<(), Error> {
	custom::register(definitions)
}

flagset::flags! {
	enum Attributes: u32 {
		Source,

		Request,
		RequestBody,

		Response,
		ResponseBody,

		Llm,
		LlmPrompt,
		LlmCompletion,

		Backend,

		Jwt,
		ApiKey,
		BasicAuth,

		Mcp,

		Extauthz,
		Extproc,
		Metadata,
		Proxy,
	}
}

#[derive(Debug)]
pub struct ContextBuilder {
	// Attributes used during the request phase: before we
	request_attributes: FlagSet<Attributes>,
	response_attributes: FlagSet<Attributes>,
	logging_attributes: FlagSet<Attributes>,
}

impl Default for ContextBuilder {
	fn default() -> Self {
		Self::new()
	}
}

impl ContextBuilder {
	pub fn new() -> Self {
		Self {
			request_attributes: Default::default(),
			response_attributes: Default::default(),
			logging_attributes: Default::default(),
		}
	}
	/// register_expression registers the given expressions attributes as required attributes.
	/// Callers MUST call this for each expression they wish to call with the context if they want correct results.
	pub fn register_expression(&mut self, expression: &Expression) {
		// TODO: different types
		self.request_attributes |= expression.attributes
	}
	/// register_log_expression registers the given expressions attributes as required attributes.
	/// This should only be used for "log" expressions. Log expressions are ones that run after the complete
	/// request and response (including the body) are complete. I.e. if its not executed during DropOnLog,
	/// its probably not the correct usage.
	/// The benefit of this compared to register_expression is that we can do more optimal processing of
	/// bodies, as we know they will complete before we need them, so we can lazily observe the body instead
	/// of proactively buffering.
	pub fn register_log_expression(&mut self, expression: &Expression) {
		self.logging_attributes |= expression.attributes
	}
	pub fn register_log_request(&mut self) {
		self.logging_attributes |= Attributes::Request;
	}
	fn any_has(&self, attr: impl Into<FlagSet<Attributes>>) -> bool {
		let x = attr.into();
		self.request_attributes.contains(x)
			|| self.response_attributes.contains(x)
			|| self.logging_attributes.contains(x)
	}
	fn before_log_has(&self, attr: impl Into<FlagSet<Attributes>>) -> bool {
		let x = attr.into();
		self.request_attributes.contains(x) || self.response_attributes.contains(x)
	}
	fn log_only_has(&self, attr: impl Into<FlagSet<Attributes>>) -> bool {
		let x = attr.into();
		self.logging_attributes.contains(x) && !self.before_log_has(x)
	}
	pub fn maybe_snapshot_response(
		&self,
		res: &mut crate::http::Response,
	) -> Option<ResponseSnapshot> {
		if self.any_has(Attributes::Response)
			|| self.any_has(Attributes::Metadata)
			|| self.any_has(Attributes::Proxy)
		{
			Some(types::snapshot_response(res))
		} else {
			None
		}
	}
	pub fn maybe_snapshot_request(
		&self,
		res: &mut crate::http::Request,
		clear: bool,
	) -> Option<RequestSnapshot> {
		if self.any_has(Attributes::Source)
			|| self.any_has(Attributes::Request)
			|| self.any_has(Attributes::Llm)
			|| self.any_has(Attributes::Proxy)
			|| self.any_has(Attributes::Backend)
			|| self.any_has(Attributes::Jwt)
			|| self.any_has(Attributes::ApiKey)
			|| self.any_has(Attributes::BasicAuth)
			|| self.any_has(Attributes::Extauthz)
			|| self.any_has(Attributes::Extproc)
			|| self.any_has(Attributes::Metadata)
		{
			// TODO: support partial snapshots based on what is requested
			Some(types::snapshot_request(res, clear))
		} else {
			None
		}
	}
	pub async fn maybe_buffer_request_body(&self, req: &mut crate::http::Request) {
		if self.before_log_has(Attributes::RequestBody) {
			if req.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			let Ok(body) = crate::http::inspect_body(req).await else {
				return;
			};
			req.extensions_mut().insert(BufferedBody(body));
		} else if self.log_only_has(Attributes::RequestBody) {
			if req.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			if req
				.extensions()
				.get::<crate::http::RecordedBodyHandle>()
				.is_some()
			{
				return;
			}
			let body = std::mem::replace(req.body_mut(), crate::http::Body::empty());
			let limit = crate::http::buffer_limit(req);
			let (body, handle) = crate::http::RecordedBody::new_with_limit(body, limit);
			*req.body_mut() = crate::http::Body::new(body);
			req.extensions_mut().insert(handle);
		}
	}
	pub async fn maybe_buffer_response_body(&self, resp: &mut crate::http::Response) {
		if self.before_log_has(Attributes::ResponseBody) {
			if resp.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			let Ok(body) = crate::http::inspect_response_body(resp).await else {
				return;
			};
			resp.extensions_mut().insert(BufferedBody(body));
		} else if self.log_only_has(Attributes::ResponseBody) {
			if resp.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			if resp
				.extensions()
				.get::<crate::http::RecordedBodyHandle>()
				.is_some()
			{
				return;
			}
			let body = std::mem::replace(resp.body_mut(), crate::http::Body::empty());
			let limit = crate::http::response_buffer_limit(resp);
			let (body, handle) = crate::http::RecordedBody::new_with_limit(body, limit);
			*resp.body_mut() = crate::http::Body::new(body);
			resp.extensions_mut().insert(handle);
		}
	}

	pub fn needs_llm(&self) -> bool {
		self.any_has(Attributes::Llm)
	}

	pub fn needs_llm_prompt(&self) -> bool {
		self.any_has(Attributes::LlmPrompt)
	}
	pub fn needs_llm_completion(&self) -> bool {
		self.any_has(Attributes::LlmCompletion)
	}
}

impl Expression {
	pub fn ast(&self) -> &cel::IdedExpr {
		self.expression.expression()
	}

	/// new_permissive compiles the expression. If the expression cannot be compiled, its instead replaced
	/// with an expression that always fails to evaluate. The returned error is the compilation error
	/// from the original expression, if one was suppressed.
	pub fn new_permissive(original_expression: impl Into<String>) -> (Self, Option<Error>) {
		let expr = original_expression.into();
		match Self::new_strict(&expr) {
			Ok(ok) => (ok, None),
			Err(err) => {
				debug!("ignoring failed expression: {}", err);
				let fail_message =
					serde_json::to_string(&format!("the expression {expr:?} could not be compiled"))
						.expect("string serialization must succeed");
				(
					Self {
						attributes: Default::default(),
						expression: Self::new_strict(format!("fail({fail_message})"))
							.expect("must be valid")
							.expression,
						original_expression: expr,
					},
					Some(err),
				)
			},
		}
	}
	/// new_strict compiles the expression, and returns an error if its invalid.
	pub fn new_strict(original_expression: impl Into<String>) -> Result<Self, Error> {
		let original_expression = original_expression.into();
		let expression =
			Program::compile_with_optimizer(&original_expression, agent_celx::DefaultOptimizer)?;

		let mut attributes = attributes_for(expression.expression());

		let include_all = expression.references().functions().contains(&"variables");
		attributes |= custom::attributes_for_functions(expression.references().functions().into_iter());

		if include_all {
			attributes |= FlagSet::full();
		}

		Ok(Self {
			attributes,
			expression,
			original_expression,
		})
	}
}

fn attributes_for(expression: &cel::IdedExpr) -> FlagSet<Attributes> {
	let mut props: Vec<Vec<&str>> = Vec::with_capacity(5);
	properties::properties(&expression.expr, &mut props, &mut Vec::default());

	// For now we only look at the first level. We could be more precise.
	let mut attributes: FlagSet<Attributes> = FlagSet::default();
	for tokens in props {
		match tokens.as_slice() {
			["request", "body", ..] => {
				attributes |= Attributes::Request | Attributes::RequestBody;
			},
			["request", ..] => {
				attributes |= Attributes::Request;
			},
			["response", "body", ..] => {
				attributes |= Attributes::Response | Attributes::ResponseBody;
			},
			["response", ..] => {
				attributes |= Attributes::Response;
			},
			["llm", "prompt", ..] => {
				attributes |= Attributes::Llm | Attributes::LlmPrompt;
			},
			["llm", "completion", ..] => {
				attributes |= Attributes::Llm | Attributes::LlmCompletion;
			},
			["llm", ..] => {
				attributes |= Attributes::Llm;
			},
			["source", ..] => {
				attributes |= Attributes::Source;
			},
			["backend", ..] => {
				attributes |= Attributes::Backend;
			},
			["jwt", ..] => {
				attributes |= Attributes::Jwt;
			},
			["apiKey", ..] => {
				attributes |= Attributes::ApiKey;
			},
			["basicAuth", ..] => {
				attributes |= Attributes::BasicAuth;
			},
			["mcp", ..] => {
				attributes |= Attributes::Mcp;
			},
			["extauthz", ..] => {
				attributes |= Attributes::Extauthz;
			},
			["extproc", ..] => {
				attributes |= Attributes::Extproc;
			},
			["metadata", ..] => {
				attributes |= Attributes::Metadata;
			},
			["proxy", ..] => {
				attributes |= Attributes::Proxy;
			},
			_ => {},
		}
	}
	attributes
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(any(test, feature = "internal_benches"))]
#[path = "benches.rs"]
mod benches;
mod properties;
mod query;
