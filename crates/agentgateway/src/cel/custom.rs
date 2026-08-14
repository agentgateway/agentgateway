use std::collections::{HashMap, HashSet};

use cel::context::VariableResolver;
use cel::extractors::Function;
use cel::objects::ListValue;
use cel::{Context, ExecutionError, FunctionContext, Program, ResolveResult, Value};
use flagset::FlagSet;

use super::{Attributes, Error, ROOT_CONTEXT, attributes_for};

#[derive(Clone, Debug)]
struct Definition {
	// True for receiver-style functions such as `"x".matchesFoo()`.
	receiver: bool,
	name: String,
	// Fixed parameters. Variadic arguments are tracked separately so they can
	// be exposed as one CEL list.
	params: Vec<String>,
	variadic: Option<String>,
	// The CEL expression inside the custom function block.
	body: String,
	// Function references found in `body`; filtered to custom functions when
	// building the call graph.
	calls: Vec<String>,
	// Direct attributes referenced by `body`, before custom-call propagation.
	attributes: FlagSet<Attributes>,
}

struct Registered {
	receiver: bool,
	params: Vec<String>,
	variadic: Option<String>,
	// Compiled body evaluated with a resolver that overlays parameters on the
	// caller's CEL variables.
	program: Program,
}

#[derive(Default)]
pub(super) struct Registry {
	// Transitive attributes for each custom function, used when compiling
	// expressions that call them.
	attributes: HashMap<String, FlagSet<Attributes>>,
}

pub fn register(definitions: &str) -> Result<(), Error> {
	if definitions.trim().is_empty() {
		return Ok(());
	}
	// The CEL context is global so built-in and custom functions share one
	// function table. Register custom functions before any expression can
	// initialize that table without them.
	if ROOT_CONTEXT.get().is_some() {
		return Err(Error::Variable(
			"custom CEL functions must be registered before CEL is used".to_string(),
		));
	}

	let parsed = parse_all(definitions)?;
	validate_unique_names(&parsed)?;
	let calls = transitive_custom_calls(&parsed)?;
	let attributes = transitive_attributes(&parsed, &calls);

	let mut ctx = Context::default();
	agent_celx::insert_all(&mut ctx);
	reject_builtin_collisions(&parsed, &ctx)?;
	let mut registry = Registry::default();
	for definition in parsed {
		let function = register_function(&definition)?;
		ctx.add_function_direct(&definition.name, function);
		registry
			.attributes
			.insert(definition.name.clone(), attributes[&definition.name]);
	}
	ROOT_CONTEXT
		.set(super::RootContext {
			context: ctx,
			registry,
		})
		.map_err(|_| {
			Error::Variable("custom CEL functions must be registered before CEL is used".to_string())
		})?;
	Ok(())
}

fn validate_unique_names(definitions: &[Definition]) -> Result<(), Error> {
	let mut names = HashSet::new();
	for definition in definitions {
		if !names.insert(definition.name.as_str()) {
			return Err(Error::Variable(format!(
				"custom CEL function {} is defined more than once",
				definition.name
			)));
		}
	}
	Ok(())
}

fn reject_builtin_collisions(definitions: &[Definition], ctx: &Context) -> Result<(), Error> {
	for definition in definitions {
		if ctx.functions.contains_key(&definition.name) {
			return Err(Error::Variable(format!(
				"custom CEL function {} conflicts with an existing CEL function",
				definition.name
			)));
		}
	}
	Ok(())
}

fn transitive_custom_calls(
	definitions: &[Definition],
) -> Result<HashMap<String, HashSet<String>>, Error> {
	// Track only calls between custom functions. The resulting closure is used
	// both to reject recursion and to propagate snapshot attributes.
	let names = definitions
		.iter()
		.map(|definition| definition.name.as_str())
		.collect::<HashSet<_>>();
	let mut calls = definitions
		.iter()
		.map(|definition| {
			(
				definition.name.clone(),
				definition
					.calls
					.iter()
					.filter(|call| names.contains(call.as_str()))
					.cloned()
					.collect::<HashSet<_>>(),
			)
		})
		.collect::<HashMap<_, _>>();

	loop {
		let mut changed = false;
		for definition in definitions {
			let mut next = calls.get(&definition.name).cloned().unwrap_or_default();
			for call in definition
				.calls
				.iter()
				.filter(|call| names.contains(call.as_str()))
			{
				if let Some(called) = calls.get(call) {
					next.extend(called.iter().cloned());
				}
			}
			if next.contains(&definition.name) {
				return Err(Error::Variable(format!(
					"custom CEL function recursion is not supported: {}",
					definition.name
				)));
			}
			if calls.get(&definition.name) != Some(&next) {
				calls.insert(definition.name.clone(), next);
				changed = true;
			}
		}
		if !changed {
			break;
		}
	}

	Ok(calls)
}

fn transitive_attributes(
	definitions: &[Definition],
	calls: &HashMap<String, HashSet<String>>,
) -> HashMap<String, FlagSet<Attributes>> {
	// Attribute collection drives request/response snapshotting. Custom
	// functions need the attributes of every custom function they may call,
	// including forward references and multi-hop chains.
	let mut attributes = definitions
		.iter()
		.map(|definition| (definition.name.clone(), definition.attributes))
		.collect::<HashMap<_, _>>();
	for definition in definitions {
		let mut next = attributes
			.get(&definition.name)
			.copied()
			.unwrap_or_default();
		if let Some(called_functions) = calls.get(&definition.name) {
			for called_function in called_functions {
				if let Some(called) = attributes.get(called_function) {
					next |= *called;
				}
			}
		}
		attributes.insert(definition.name.clone(), next);
	}
	attributes
}

pub fn attributes_for_functions<'a>(
	functions: impl Iterator<Item = &'a str>,
) -> FlagSet<Attributes> {
	let Some(root) = ROOT_CONTEXT.get() else {
		return FlagSet::default();
	};
	functions.fold(FlagSet::default(), |mut acc, function| {
		if let Some(function_attrs) = root.registry.attributes.get(function) {
			acc |= *function_attrs;
		}
		acc
	})
}

fn register_function(definition: &Definition) -> Result<Function, Error> {
	let program = Program::compile_with_optimizer(&definition.body, agent_celx::DefaultOptimizer)?;
	// Custom functions live in the global CEL context for the rest of the
	// process. Leaking this small metadata object gives the callback the same
	// lifetime as the context and avoids a registry lookup on every call.
	let function: &'static Registered = Box::leak(Box::new(Registered {
		receiver: definition.receiver,
		params: definition.params.clone(),
		variadic: definition.variadic.clone(),
		program,
	}));
	let closure = funnel(move |ftx: &mut FunctionContext| custom_function(ftx, function));
	Ok(cel::extractors::IntoFunction::into_function(closure))
}

fn funnel<CL>(f: CL) -> CL
where
	CL: for<'b, 'a, 'rf> Fn(&'b mut FunctionContext<'a, 'rf>) -> ResolveResult<'a>,
{
	f
}

fn custom_function<'a, 'rf>(
	ftx: &mut FunctionContext<'a, 'rf>,
	function: &'static Registered,
) -> ResolveResult<'a> {
	let args = ftx.value_iter().collect::<Result<Vec<_>, _>>()?;
	let required_arg_count = function.params.len();
	if function.variadic.is_none() && args.len() != required_arg_count {
		return Err(ExecutionError::invalid_argument_count(
			required_arg_count,
			args.len(),
		));
	}
	if function.variadic.is_some() && args.len() < required_arg_count {
		return Err(ExecutionError::invalid_argument_count(
			required_arg_count,
			args.len(),
		));
	}
	let this = if function.receiver {
		Some(ftx.this_unmaterialized()?)
	} else {
		None
	};
	// Custom function parameters shadow request variables, while everything
	// else falls through to the caller's resolver.
	let resolver = CustomResolver {
		base: ftx.vars(),
		params: &function.params,
		args: &args,
		variadic: function.variadic.as_deref(),
		this,
	};
	function
		.program
		.execute_with(ftx.ptx, &resolver)
		.map(|value| value.always_materialize_owned())
}

struct CustomResolver<'a, 'rf> {
	base: &'rf dyn VariableResolver<'a>,
	params: &'rf [String],
	args: &'rf [Value<'a>],
	variadic: Option<&'rf str>,
	this: Option<Value<'a>>,
}

impl<'a> VariableResolver<'a> for CustomResolver<'a, '_> {
	fn resolve(&self, expr: &str) -> Option<Value<'a>> {
		if expr == "this" {
			return self.this.clone();
		}
		if let Some((idx, _)) = self
			.params
			.iter()
			.enumerate()
			.find(|(_, param)| param.as_str() == expr)
		{
			return self.args.get(idx).cloned();
		}
		// Variadic arguments are exposed as a CEL list of the remaining
		// arguments after the fixed parameters.
		if self.variadic == Some(expr) {
			let rest = self.args[self.params.len()..].to_vec();
			return Some(Value::List(ListValue::PartiallyOwned(rest.into())));
		}
		self.base.resolve(expr)
	}

	fn variables(&self) -> Option<Value<'a>> {
		self.base.variables()
	}

	fn resolve_member(&self, expr: &str, member: &str) -> Option<Value<'a>> {
		self.base.resolve_member(expr, member)
	}

	fn resolve_direct(&self, field: &cel::common::ast::OptimizedExpr) -> Option<Option<Value<'a>>> {
		self.base.resolve_direct(field)
	}
}

fn parse_all(input: &str) -> Result<Vec<Definition>, Error> {
	let mut definitions = Vec::new();
	let mut rest = input;
	while !rest.trim().is_empty() {
		let trimmed = rest.trim_start();
		let skipped = rest.len() - trimmed.len();
		rest = &rest[skipped..];
		if rest.starts_with('#') {
			if let Some(next) = rest.find('\n') {
				rest = &rest[next + 1..];
				continue;
			}
			break;
		}
		// Supported forms are:
		// * `name(args) { cel-expression }`
		// * `this.name(args) { cel-expression }`.
		// The final parameter may be variadic, written as `rest...`.
		// The CEL body may contain nested braces or quoted braces.
		let open = rest
			.find('{')
			.ok_or_else(|| Error::Variable("custom function missing body".to_string()))?;
		let close = find_body_close(rest, open)?;
		let header = rest[..open].trim();
		let body = rest[open + 1..close].trim();
		definitions.push(parse_definition(header, body)?);
		rest = &rest[close + 1..];
	}
	Ok(definitions)
}

fn find_body_close(input: &str, open: usize) -> Result<usize, Error> {
	let mut depth = 1usize;
	let mut quote = None;
	let mut escaped = false;
	for (idx, ch) in input[open + 1..].char_indices() {
		let idx = open + 1 + idx;
		if let Some(q) = quote {
			if escaped {
				escaped = false;
			} else if ch == '\\' {
				escaped = true;
			} else if ch == q {
				quote = None;
			}
			continue;
		}
		match ch {
			'\'' | '"' => quote = Some(ch),
			'{' => depth += 1,
			'}' => {
				depth -= 1;
				if depth == 0 {
					return Ok(idx);
				}
			},
			_ => {},
		}
	}
	Err(Error::Variable(
		"custom function body missing closing '}'".to_string(),
	))
}

fn parse_definition(header: &str, body: &str) -> Result<Definition, Error> {
	let paren = header
		.find('(')
		.ok_or_else(|| Error::Variable(format!("custom function missing parameter list: {header}")))?;
	let end = header
		.rfind(')')
		.ok_or_else(|| Error::Variable(format!("custom function missing ')': {header}")))?;
	if end != header.len() - 1 {
		return Err(Error::Variable(format!(
			"unexpected custom function header: {header}"
		)));
	}

	let receiver_and_name = header[..paren].trim();
	let (receiver, name) = match receiver_and_name.strip_prefix("this.") {
		Some(name) => (true, name),
		None => (false, receiver_and_name),
	};
	validate_ident(name)?;

	let args = header[paren + 1..end].trim();
	let mut params = Vec::new();
	let mut variadic = None;
	if !args.is_empty() {
		for raw in args.split(',') {
			let arg = raw.trim();
			if let Some(variadic_name) = arg.strip_suffix("...") {
				if variadic.is_some() {
					return Err(Error::Variable(format!(
						"custom function {name} has multiple variadic parameters"
					)));
				}
				let variadic_name = variadic_name.trim();
				validate_ident(variadic_name)?;
				variadic = Some(variadic_name.to_string());
			} else {
				if variadic.is_some() {
					return Err(Error::Variable(format!(
						"custom function {name} has parameters after variadic parameter"
					)));
				}
				validate_ident(arg)?;
				params.push(arg.to_string());
			}
		}
	}

	let program = Program::compile_with_optimizer(body, agent_celx::DefaultOptimizer)?;
	let calls = program
		.references()
		.functions()
		.into_iter()
		.map(str::to_string)
		.collect::<Vec<_>>();
	let mut attributes = attributes_for(program.expression());
	if calls.iter().any(|call| call == "variables") {
		attributes |= FlagSet::full();
	}

	Ok(Definition {
		receiver,
		name: name.to_string(),
		params,
		variadic,
		body: body.to_string(),
		calls,
		attributes,
	})
}

fn validate_ident(ident: &str) -> Result<(), Error> {
	if ident == "this" {
		return Err(Error::Variable(
			"custom function identifier cannot be 'this'".to_string(),
		));
	}
	let mut chars = ident.chars();
	let Some(first) = chars.next() else {
		return Err(Error::Variable(
			"custom function identifier is empty".to_string(),
		));
	};
	if !(first == '_' || first.is_ascii_alphabetic()) {
		return Err(Error::Variable(format!(
			"invalid custom function identifier: {ident}"
		)));
	}
	if !chars.all(|c| c == '_' || c.is_ascii_alphanumeric()) {
		return Err(Error::Variable(format!(
			"invalid custom function identifier: {ident}"
		)));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assert_err_contains(input: &str, expected: &str) {
		let err = parse_all(input).expect_err("parse should fail");
		assert!(
			err.to_string().contains(expected),
			"error {err:?} did not contain {expected:?}"
		);
	}

	fn assert_recursion_err_contains(input: &str, expected: &str) {
		let defs = parse_all(input).expect("parse should pass");
		let err = transitive_custom_calls(&defs).expect_err("recursion should fail");
		assert!(
			err.to_string().contains(expected),
			"error {err:?} did not contain {expected:?}"
		);
	}

	#[test]
	fn parses_basic_definitions() {
		let defs = parse_all(
			r#"
# comment
myFn() { "hi" }
this.add(a, rest...) { this + a + rest[0] }
"#,
		)
		.unwrap();
		assert_eq!(defs.len(), 2);
		assert_eq!(defs[0].name, "myFn");
		assert!(!defs[0].receiver);
		assert_eq!(defs[1].name, "add");
		assert!(defs[1].receiver);
		assert_eq!(defs[1].params, vec!["a"]);
		assert_eq!(defs[1].variadic.as_deref(), Some("rest"));
	}

	#[test]
	fn parses_whitespace_comments_and_multiple_blocks() {
		let defs = parse_all(
			r#"

  # leading comment with whitespace
  _with_underscores(a1, _b2) { a1 + _b2 }

  # comment between definitions
  this.receiverFn() { this }

  trailingVariadic(first, rest...) { first + rest.size() }
"#,
		)
		.unwrap();

		assert_eq!(defs.len(), 3);
		assert_eq!(defs[0].name, "_with_underscores");
		assert_eq!(defs[0].params, vec!["a1", "_b2"]);
		assert_eq!(defs[0].variadic, None);
		assert!(!defs[0].receiver);

		assert_eq!(defs[1].name, "receiverFn");
		assert!(defs[1].receiver);
		assert!(defs[1].params.is_empty());

		assert_eq!(defs[2].name, "trailingVariadic");
		assert_eq!(defs[2].params, vec!["first"]);
		assert_eq!(defs[2].variadic.as_deref(), Some("rest"));
	}

	#[test]
	fn parses_empty_and_comment_only_input() {
		assert!(parse_all("").unwrap().is_empty());
		assert!(parse_all("   \n\t  ").unwrap().is_empty());
		assert!(parse_all("  # only a comment").unwrap().is_empty());
	}

	#[test]
	fn parses_bodies_with_quoted_and_nested_braces() {
		let defs = parse_all(
			r#"
lt() { '}' }
obj() { {"end": "}"} }
"#,
		)
		.unwrap();

		assert_eq!(defs.len(), 2);
		assert_eq!(defs[0].name, "lt");
		assert_eq!(defs[0].body, "'}'");
		assert_eq!(defs[1].name, "obj");
		assert_eq!(defs[1].body, r#"{"end": "}"}"#);
	}

	#[test]
	fn includes_transitive_attributes_from_custom_function_calls() {
		let defs = parse_all(
			r#"
authenticated() {
  jwt != null || apiKey != null || nested()
}
nested() { request.headers["auth"] == "true" }
allVariables() { variables() }
"#,
		)
		.unwrap();

		let calls = transitive_custom_calls(&defs).unwrap();
		let attrs = transitive_attributes(&defs, &calls);
		let authenticated = attrs["authenticated"];
		assert!(authenticated.contains(Attributes::Jwt));
		assert!(authenticated.contains(Attributes::ApiKey));
		assert!(authenticated.contains(Attributes::Request));
		assert_eq!(attrs["allVariables"], FlagSet::full());
	}

	#[test]
	fn rejects_recursive_custom_functions() {
		assert_recursion_err_contains(
			"foo(a) { a == 10 ? 100 : foo(a + 1) }",
			"custom CEL function recursion is not supported: foo",
		);
		assert_recursion_err_contains(
			r#"
foo(a) { bar(a) }
bar(a) { foo(a) }
"#,
			"custom CEL function recursion is not supported",
		);
	}

	#[test]
	fn rejects_missing_function_parts() {
		assert_err_contains("myFn { true }", "missing parameter list");
		assert_err_contains("myFn( { true }", "missing ')'");
		assert_err_contains("myFn() true", "missing body");
		assert_err_contains("myFn() { true", "missing closing '}'");
		assert_err_contains("myFn() extra { true }", "unexpected custom function header");
	}

	#[test]
	fn rejects_invalid_identifiers() {
		assert_err_contains("1bad() { true }", "invalid custom function identifier");
		assert_err_contains("bad-name() { true }", "invalid custom function identifier");
		assert_err_contains("this.() { true }", "custom function identifier is empty");
		assert_err_contains(
			"this() { true }",
			"custom function identifier cannot be 'this'",
		);
		assert_err_contains("myFn(1bad) { true }", "invalid custom function identifier");
		assert_err_contains(
			"myFn(bad-name) { true }",
			"invalid custom function identifier",
		);
		assert_err_contains(
			"myFn(this) { true }",
			"custom function identifier cannot be 'this'",
		);
		assert_err_contains(
			"myFn(this...) { true }",
			"custom function identifier cannot be 'this'",
		);
	}

	#[test]
	fn rejects_duplicate_and_builtin_function_names() {
		let defs = parse_all(
			r#"
myFn() { true }
myFn(a) { a }
"#,
		)
		.unwrap();
		let err = validate_unique_names(&defs).unwrap_err();
		assert!(err.to_string().contains("is defined more than once"));

		let defs = parse_all("size() { 1 }").unwrap();
		let mut ctx = Context::default();
		agent_celx::insert_all(&mut ctx);
		let err = reject_builtin_collisions(&defs, &ctx).unwrap_err();
		assert!(
			err
				.to_string()
				.contains("conflicts with an existing CEL function")
		);
	}

	#[test]
	fn rejects_invalid_variadic_parameters() {
		assert_err_contains(
			"myFn(rest..., another...) { true }",
			"has multiple variadic parameters",
		);
		assert_err_contains(
			"myFn(rest..., after) { true }",
			"has parameters after variadic parameter",
		);
		assert_err_contains("myFn(...) { true }", "custom function identifier is empty");
	}

	#[test]
	fn rejects_invalid_cel_body() {
		assert!(parse_all("myFn() { 1 + }").is_err());
	}
}
