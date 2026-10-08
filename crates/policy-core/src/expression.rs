use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::OnceLock;

use cel::{ExecutionError, ParseError, ParseErrors, Program};
use flagset::FlagSet;
use serde::{Deserialize, Serialize, Serializer};
use tracing::debug;

use crate::properties;

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
		Expression::new_strict(&e).map_err(|e| serde::de::Error::custom(e.to_string()))
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

flagset::flags! {
	pub enum Attributes: u32 {
		Source,
		Destination,

		Request,
		RequestBody,

		Response,
		ResponseBody,

		Llm,
		LlmRequest,
		LlmPrompt,
		LlmCompletion,
		LlmToolCalls,

		Backend,

		Jwt,
		ApiKey,
		BasicAuth,

		Mcp,

		Guardrails,

		Extauthz,
		Extproc,
		Metadata,
		Proxy,
	}
}

impl Expression {
	pub fn attributes(&self) -> FlagSet<Attributes> {
		self.attributes
	}

	pub fn ast(&self) -> &cel::IdedExpr {
		self.expression.expression()
	}

	pub fn needs_llm_request(&self) -> bool {
		self.attributes.contains(Attributes::LlmRequest)
	}

	pub fn needs_llm(&self) -> bool {
		self.attributes.contains(Attributes::Llm)
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
		attributes |= attributes_for_functions(expression.references().functions().into_iter());

		if include_all {
			attributes |= FlagSet::full();
		}

		Ok(Self {
			attributes,
			expression,
			original_expression,
		})
	}

	/// new_unoptimized compiles the expression without optimizations, for comparing optimizer behavior in tests.
	#[doc(hidden)]
	pub fn new_unoptimized(original_expression: impl Into<String>) -> Result<Self, Error> {
		let original_expression = original_expression.into();
		let expression = Program::compile_unoptimized(&original_expression)?;
		let mut attributes = attributes_for(expression.expression());
		attributes |= attributes_for_functions(expression.references().functions().into_iter());
		if expression.references().functions().contains(&"variables") {
			attributes |= FlagSet::full();
		}
		Ok(Self {
			attributes,
			expression,
			original_expression,
		})
	}
}

pub fn attributes_for(expression: &cel::IdedExpr) -> FlagSet<Attributes> {
	let mut props: Vec<Vec<&str>> = Vec::with_capacity(5);
	properties::properties(&expression.expr, &mut props, &mut Vec::default());

	// For now we only look at the first level. We could be more precise.
	let mut attributes: FlagSet<Attributes> = FlagSet::default();
	for tokens in props {
		match tokens.as_slice() {
			["request", "body" | "bodyPrefix", ..] => {
				attributes |= Attributes::Request | Attributes::RequestBody;
			},
			["request", ..] => {
				attributes |= Attributes::Request;
			},
			["response", "body" | "bodyPrefix", ..] => {
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
			["llm", "toolCalls", ..] => {
				attributes |= Attributes::Llm | Attributes::LlmToolCalls;
			},
			["llm", ..] => {
				attributes |= Attributes::Llm;
			},
			["llmRequest", ..] => {
				attributes |= Attributes::LlmRequest;
			},
			["source", ..] => {
				attributes |= Attributes::Source;
			},
			["destination", ..] => {
				attributes |= Attributes::Destination;
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
			["guardrails", ..] => {
				attributes |= Attributes::Guardrails;
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

static CUSTOM_FUNCTION_ATTRIBUTES: OnceLock<HashMap<String, FlagSet<Attributes>>> = OnceLock::new();

/// Installs the transitive attributes of each custom function, used when compiling
/// expressions that call them. Custom functions must be registered before expressions are compiled.
pub fn install_custom_function_attributes(
	attributes: HashMap<String, FlagSet<Attributes>>,
) -> Result<(), Error> {
	CUSTOM_FUNCTION_ATTRIBUTES
		.set(attributes)
		.map_err(|_| Error::Variable("custom CEL function attributes are already installed".to_owned()))
}

fn attributes_for_functions<'a>(functions: impl Iterator<Item = &'a str>) -> FlagSet<Attributes> {
	let Some(registry) = CUSTOM_FUNCTION_ATTRIBUTES.get() else {
		return FlagSet::default();
	};
	functions.fold(FlagSet::default(), |mut acc, function| {
		if let Some(function_attrs) = registry.get(function) {
			acc |= *function_attrs;
		}
		acc
	})
}

/// Evaluates policy expressions against host-owned HTTP state.
///
/// Keeping this interface narrow allows policy crates to evaluate CEL without
/// depending on the gateway's request log or snapshot implementation.
pub trait PolicyCel: Send + Sync {
	fn eval_request<'a>(
		&'a self,
		expression: &'a Expression,
		req: &'a agent_http::Request,
	) -> Result<cel::Value<'a>, Error>;

	fn eval_response<'a>(
		&'a self,
		expression: &'a Expression,
		resp: &'a agent_http::Response,
	) -> Result<cel::Value<'a>, Error>;
}
