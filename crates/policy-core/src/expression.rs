//! CEL expressions and the host-provided evaluation interface.

use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::OnceLock;

use cel::{ExecutionError, ParseError, ParseErrors, Program};
use flagset::FlagSet;
use serde::{Deserialize, Serialize, Serializer};
use tracing::debug;

flagset::flags! {
	/// Gateway data required to evaluate a CEL expression.
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

		Extauthz,
		Extproc,
		Metadata,
		Proxy,
	}
}

static CUSTOM_FUNCTION_ATTRIBUTES: OnceLock<HashMap<String, FlagSet<Attributes>>> = OnceLock::new();

/// Error produced while compiling or evaluating a policy CEL expression.
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

/// A compiled CEL expression retained alongside its original source.
///
/// Policies own expressions, while the host implements [`PolicyCel`] to supply
/// request and response data without exposing gateway-specific snapshot types.
pub struct Expression {
	attributes: FlagSet<Attributes>,
	expression: Program,
	pub original_expression: String,
}

impl Expression {
	/// Returns the gateway data required to evaluate this expression.
	pub fn attributes(&self) -> FlagSet<Attributes> {
		self.attributes
	}

	/// Returns the compiled CEL syntax tree.
	pub fn ast(&self) -> &cel::IdedExpr {
		self.expression.expression()
	}

	pub fn needs_llm_request(&self) -> bool {
		self.attributes.contains(Attributes::LlmRequest)
	}

	/// Compiles an expression, replacing invalid input with an expression that
	/// fails when evaluated.
	///
	/// The suppressed compilation error is returned alongside the expression.
	pub fn new_permissive(original_expression: impl Into<String>) -> (Self, Option<Error>) {
		let expr = original_expression.into();
		match Self::new_strict(&expr) {
			Ok(ok) => (ok, None),
			Err(err) => {
				debug!("ignoring failed expression: {}", err);
				let fail_message =
					serde_json::to_string(&format!("the expression {expr:?} could not be compiled"))
						.expect("string serialization must succeed");
				let fallback = Self::new_strict(format!("fail({fail_message})")).expect("must be valid");
				(
					Self {
						attributes: FlagSet::default(),
						expression: fallback.expression,
						original_expression: expr,
					},
					Some(err),
				)
			},
		}
	}

	/// Compiles an expression, returning an error when the source is invalid.
	pub fn new_strict(original_expression: impl Into<String>) -> Result<Self, Error> {
		let original_expression = original_expression.into();
		let expression =
			Program::compile_with_optimizer(&original_expression, agent_celx::DefaultOptimizer)?;
		let attributes = expression_attributes(&expression);
		Ok(Self {
			attributes,
			expression,
			original_expression,
		})
	}
}

/// Installs attributes referenced transitively by gateway-defined CEL functions.
///
/// Custom functions must be registered before policy expressions are compiled.
pub fn install_custom_function_attributes(
	attributes: HashMap<String, FlagSet<Attributes>>,
) -> Result<(), Error> {
	CUSTOM_FUNCTION_ATTRIBUTES
		.set(attributes)
		.map_err(|_| Error::Variable("custom CEL function attributes are already installed".to_owned()))
}

fn expression_attributes(expression: &Program) -> FlagSet<Attributes> {
	let mut attributes = attributes_for_ast(expression.expression());
	let references = expression.references();
	if references.has_function("variables") {
		attributes |= FlagSet::full();
	}
	if let Some(custom) = CUSTOM_FUNCTION_ATTRIBUTES.get() {
		for function in references.functions() {
			if let Some(function_attributes) = custom.get(function) {
				attributes |= *function_attributes;
			}
		}
	}
	attributes
}

/// Determines direct gateway data references in a CEL syntax tree.
///
/// This is public for custom-function registration. Normal policy code should
/// read the attributes stored on [`Expression`] instead.
#[doc(hidden)]
pub fn attributes_for_ast(expression: &cel::IdedExpr) -> FlagSet<Attributes> {
	let mut properties = Vec::with_capacity(5);
	collect_properties(&expression.expr, &mut properties, &mut Vec::new());

	let mut attributes = FlagSet::default();
	for tokens in properties {
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

fn collect_properties<'expression>(
	expression: &'expression cel::common::ast::Expr,
	all: &mut Vec<Vec<&'expression str>>,
	path: &mut Vec<&'expression str>,
) {
	use cel::common::ast::Expr::*;
	match expression {
		Unspecified | Literal(_) | Inline(_) => {},
		Optimized { original, .. } => collect_properties(&original.expr, all, path),
		Call(call) => {
			path.clear();
			if let Some(target) = &call.target {
				collect_properties(&target.expr, all, path);
			}
			for argument in &call.args {
				collect_properties(&argument.expr, all, path);
			}
		},
		Select(select) => {
			path.insert(0, select.field.as_str());
			collect_properties(&select.operand.expr, all, path);
		},
		Comprehension(comprehension) => {
			collect_properties(&comprehension.iter_range.expr, all, path);
			if !comprehension.iter_var.starts_with('@') {
				path.insert(0, comprehension.iter_var.as_str());
				all.push(path.clone());
				path.clear();
			}
			collect_properties(&comprehension.loop_step.expr, all, path);
		},
		List(list) => {
			for element in &list.elements {
				collect_properties(&element.expr, all, path);
			}
		},
		Map(map) => {
			for entry in &map.entries {
				match &entry.expr {
					cel::common::ast::EntryExpr::StructField(field) => {
						collect_properties(&field.value.expr, all, path);
					},
					cel::common::ast::EntryExpr::MapEntry(entry) => {
						collect_properties(&entry.value.expr, all, path);
					},
				}
			}
		},
		Struct(value) => {
			for entry in &value.entries {
				match &entry.expr {
					cel::common::ast::EntryExpr::StructField(field) => {
						collect_properties(&field.value.expr, all, path);
					},
					cel::common::ast::EntryExpr::MapEntry(entry) => {
						collect_properties(&entry.value.expr, all, path);
					},
				}
			}
		},
		Ident(identifier) => {
			if !identifier.starts_with('@') {
				path.insert(0, identifier.as_str());
				all.push(path.clone());
				path.clear();
			}
		},
	}
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
		let expression = String::deserialize(deserializer)?;
		Expression::new_strict(&expression).map_err(|error| serde::de::Error::custom(error.to_string()))
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

	fn eval_request_response<'a>(
		&'a self,
		expression: &'a Expression,
		_req: &'a agent_http::Request,
		resp: &'a agent_http::Response,
	) -> Result<cel::Value<'a>, Error> {
		self.eval_response(expression, resp)
	}
}

#[cfg(test)]
mod tests {
	use std::collections::HashSet;

	use super::*;

	#[test]
	fn stores_attributes_when_expression_is_compiled() {
		let expression = Expression::new_strict(
			r#"request.body.foo == "bar" && llmRequest.model == "gpt" && jwt.sub != """#,
		)
		.unwrap();

		assert!(expression.attributes().contains(Attributes::Request));
		assert!(expression.attributes().contains(Attributes::RequestBody));
		assert!(expression.attributes().contains(Attributes::LlmRequest));
		assert!(expression.attributes().contains(Attributes::Jwt));
	}

	#[test]
	fn variables_function_requires_all_attributes() {
		let expression = Expression::new_strict("variables()").unwrap();

		assert_eq!(expression.attributes(), FlagSet::full());
	}

	#[test]
	fn collects_expression_properties() {
		let test = |source: &str, expected: &[&str]| {
			let program = Program::compile(source).unwrap();
			let mut properties = Vec::with_capacity(5);
			collect_properties(
				&program.expression().expr,
				&mut properties,
				&mut Vec::default(),
			);
			let expected = HashSet::from_iter(expected.iter().map(|property| property.to_string()));
			let actual = properties
				.into_iter()
				.map(|property| property.join("."))
				.collect::<HashSet<_>>();
			assert_eq!(expected, actual, "expression: {source}");
		};

		test(r#"foo.bar.baz"#, &["foo.bar.baz"]);
		test(r#"foo["bar"]"#, &["foo"]);
		test(r#"foo.baz["bar"]"#, &["foo.baz"]);
		test(r#"foo.with(x, x.body)"#, &["foo", "x", "x.body"]);
		test(r#"foo.map(x, x.body)"#, &["foo", "x", "x.body"]);
		test(r#"foo.bar.map(x, x.body)"#, &["foo.bar", "x", "x.body"]);
		test(r#"fn(bar.baz)"#, &["bar.baz"]);
		test(r#"{"key":val, "listkey":[a.b]}"#, &["val", "a.b"]);
		test(r#"a? b: c"#, &["a", "b", "c"]);
		test(r#"a || b"#, &["a", "b"]);
		test(r#"!a.b"#, &["a.b"]);
		test(r#"a.b < c"#, &["a.b", "c"]);
		test(r#"a.b + c + 2"#, &["a.b", "c"]);
		test(r#"a["b"].c"#, &["a"]);
		test(r#"a["b"]["c"]"#, &["a"]);
		test(r#"a.b[0]"#, &["a.b"]);
		test(r#"a.b[0].c"#, &["a.b"]);
		test(r#"a[b.c]"#, &["a", "b.c"]);
		test(r#"{"a":"b"}.a"#, &[]);
		test(r#"extauthz.user_id"#, &["extauthz.user_id"]);
		test(r#"extauthz.role == "admin""#, &["extauthz.role"]);
	}
}
