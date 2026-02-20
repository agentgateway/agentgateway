// Portions of this code are heavily inspired from https://github.com/Kuadrant/wasm-shim/
// Under Apache 2.0 license (https://github.com/Kuadrant/wasm-shim/blob/main/LICENSE)

use std::fmt::{Debug, Formatter};
use std::sync::Arc;

pub use cel::Value;
pub use cel::types::dynamic::DynamicType;
use cel::{Context, ExecutionError, ParseError, ParseErrors, Program};
use flagset::FlagSet;
pub use helpers::*;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize, Serializer};
use tracing::log::debug;
pub use types::*;

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

fn root_context() -> Arc<Context> {
	let mut ctx = Context::default();
	agent_celx::insert_all(&mut ctx);
	Arc::new(ctx)
}

static ROOT_CONTEXT: Lazy<Arc<Context>> = Lazy::new(root_context);

flagset::flags! {
	enum Attributes: u16 {
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
	fn any_has(&self, attr: impl Into<FlagSet<Attributes>>) -> bool {
		let x = attr.into();
		self.request_attributes.contains(x)
			|| self.response_attributes.contains(x)
			|| self.logging_attributes.contains(x)
	}
	pub fn maybe_snapshot_response(
		&self,
		res: &mut crate::http::Response,
	) -> Option<ResponseSnapshot> {
		if self.any_has(Attributes::Response) {
			Some(types::snapshot_response(res))
		} else {
			None
		}
	}
	pub fn maybe_snapshot_request(&self, res: &mut crate::http::Request) -> Option<RequestSnapshot> {
		if self.any_has(Attributes::Source)
			|| self.any_has(Attributes::Request)
			|| self.any_has(Attributes::Llm)
			|| self.any_has(Attributes::Backend)
			|| self.any_has(Attributes::Jwt)
			|| self.any_has(Attributes::ApiKey)
			|| self.any_has(Attributes::BasicAuth)
			|| self.any_has(Attributes::Extauthz)
			|| self.any_has(Attributes::Extproc)
		{
			// TODO: support partial snapshots based on what is requested
			Some(types::snapshot_request(res))
		} else {
			None
		}
	}
	pub async fn maybe_buffer_request_body(&self, req: &mut crate::http::Request) {
		if self.any_has(Attributes::RequestBody) {
			if req.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			let Ok(body) = crate::http::inspect_body(req).await else {
				return;
			};
			req.extensions_mut().insert(BufferedBody(body));
		}
	}
	pub async fn maybe_buffer_response_body(&self, resp: &mut crate::http::Response) {
		if self.any_has(Attributes::ResponseBody) {
			if resp.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			let Ok(body) = crate::http::inspect_response_body(resp).await else {
				return;
			};
			resp.extensions_mut().insert(BufferedBody(body));
		}
	}

	pub fn needs_llm_prompt(&self) -> bool {
		self.any_has(Attributes::LlmPrompt)
	}
	pub fn needs_llm_completion(&self) -> bool {
		self.any_has(Attributes::LlmCompletion)
	}
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
					expression: Self::new_strict(format!(
						"fail(\"the expression {:?} could not be compiled\")",
						&expr
					))
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
		let expression =
			Program::compile_with_optimizer(&original_expression, agent_celx::DefaultOptimizer)?;

		let mut props: Vec<Vec<&str>> = Vec::with_capacity(5);
		properties::properties(
			&expression.expression().expr,
			&mut props,
			&mut Vec::default(),
		);

		let include_all = expression.references().functions().contains(&"variables");

		// For now we only look at the first level. We could be more precise
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
				_ => {},
			}
		}

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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(any(test, feature = "internal_benches"))]
#[path = "benches.rs"]
mod benches;
mod properties;
