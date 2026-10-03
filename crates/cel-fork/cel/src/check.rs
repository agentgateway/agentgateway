//! Static call-site checking against a [`Context`]'s declared metadata.
//!
//! [`Program::check`](crate::Program::check) validates every function call in
//! an expression against the [`FunctionMeta`] declared at registration,
//! reporting calls that cannot succeed (or almost certainly misbehave) without
//! executing anything. This is arity and name-resolution checking only — no
//! type checking of any kind.
//!
//! Checking is deliberately conservative: a call is only diagnosed when the
//! environment has declared enough for the diagnosis to be trustworthy.
//! Method-style calls are unchecked unless the environment has declared its
//! opaque method surface ([`Context::declare_opaque_methods`]), functions
//! registered without metadata are unchecked, and qualified functions
//! (`optional.of`, `math.ceil`, ...) are unchecked. Consequently an empty
//! result means "nothing provably wrong", not "valid".

use std::fmt;

use crate::common::ast::IdedExpr;
use crate::context::{Context, FunctionMeta, ReceiverStyle};
use crate::parser::CallSignature;

/// How the checker resolved the name at a diagnosed call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOrigin {
	/// A parser-level comprehension macro (`has`, `all`, `exists`,
	/// `exists_one`/`existsOne`, `map`, `filter`). These expand at parse time
	/// only when receiver style and arity already match; a mismatched use
	/// falls through to an ordinary call of a name that is never registered,
	/// so it fails at runtime even in cases (like surplus arguments) where a
	/// registered function would tolerate the call.
	Macro,
	/// A function registered in the [`Context`].
	Function,
}

/// What is wrong with a diagnosed call site.
///
/// Argument counts here are expressed as the user wrote them: for a
/// method-style call `x.f(a, b)` the count is 2, and declared arity ranges
/// are translated into the same terms before comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
	/// The name resolves to nothing callable in this environment.
	UnknownFunction,
	/// Fewer arguments than the declared minimum; the call fails at runtime.
	TooFewArguments { min: usize },
	/// More arguments than the declared maximum.
	TooManyArguments { max: usize },
	/// The function must be called method-style (`x.f(...)`) but was called
	/// bare.
	ReceiverMissing,
	/// The function never reads a receiver, but was called method-style.
	ReceiverForbidden,
}

/// One statically-detected problem with a call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
	/// The function name as written in the expression.
	pub function: String,
	/// The number of arguments as written (a method-style receiver is not
	/// counted).
	pub args: usize,
	/// Whether the call was written method-style (`x.f(...)`).
	pub method_style: bool,
	pub origin: CallOrigin,
	pub kind: DiagnosticKind,
}

impl fmt::Display for Diagnostic {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let name = &self.function;
		let described = match self.origin {
			CallOrigin::Macro => format!("the '{name}' macro"),
			CallOrigin::Function => format!("'{name}'"),
		};
		match &self.kind {
			DiagnosticKind::UnknownFunction if self.method_style => {
				write!(f, "unknown method '{name}'")
			},
			DiagnosticKind::UnknownFunction => write!(f, "unknown function '{name}'"),
			DiagnosticKind::TooFewArguments { min } => write!(
				f,
				"{described} requires at least {} but {} provided",
				plural(*min, "argument"),
				count_were(self.args),
			),
			DiagnosticKind::TooManyArguments { max: 0 } => write!(
				f,
				"{described} accepts no arguments but {} provided",
				count_were(self.args),
			),
			DiagnosticKind::TooManyArguments { max } => write!(
				f,
				"{described} accepts at most {} but {} provided",
				plural(*max, "argument"),
				count_were(self.args),
			),
			DiagnosticKind::ReceiverMissing => {
				write!(f, "{described} must be called as a method: `x.{name}(...)`")
			},
			DiagnosticKind::ReceiverForbidden => {
				write!(f, "{described} cannot be called as a method: `{name}(...)`")
			},
		}
	}
}

fn plural(n: usize, noun: &str) -> String {
	if n == 1 {
		format!("{n} {noun}")
	} else {
		format!("{n} {noun}s")
	}
}

fn count_were(n: usize) -> String {
	if n == 1 {
		"1 was".to_string()
	} else {
		format!("{n} were")
	}
}

/// Operators are represented as call nodes (`a == b` is `_==_(a, b)`) but are
/// dispatched inline by the interpreter and never registered in a [`Context`],
/// so the checker must ignore them. Their synthesized names (`_==_`, `!_`,
/// `@in`, ...) are never valid CEL identifiers, unlike every registrable
/// function name, so that property rather than a name list identifies them.
fn is_operator(name: &str) -> bool {
	name.starts_with('@') || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl IdedExpr {
	/// Checks every call site against the context's declared metadata. See
	/// [`Program::check`](crate::Program::check).
	pub fn check(&self, ctx: &Context) -> Vec<Diagnostic> {
		let mut diagnostics = Vec::new();
		for sig in self.call_signatures() {
			check_call(&sig, ctx, &mut diagnostics);
		}
		diagnostics
	}
}

fn check_call(sig: &CallSignature, ctx: &Context, out: &mut Vec<Diagnostic>) {
	if is_operator(sig.name) {
		return;
	}

	if sig.method_style {
		// At runtime, opaque/dynamic values get first refusal on method-style
		// calls before dispatch falls back to the global function table. An
		// intercepted call obeys the opaque method's shape, not the global
		// function's, so checking is only sound once the environment has
		// declared which names can be intercepted — and never for those names.
		let Some(declared) = ctx.opaque_methods() else {
			return;
		};
		if declared.contains(sig.name) {
			return;
		}
	}

	// Mirror runtime dispatch: the global function table governs any call that
	// reaches it, including mismatched macro uses that fell through the parser,
	// so a registered name wins over the macro table.
	if ctx.functions.contains_key(sig.name) {
		if let Some(meta) = ctx.function_meta(sig.name) {
			check_against(sig, meta, CallOrigin::Function, out);
		}
		return;
	}

	if let Some((_, shape, _)) = crate::parser::macros::MACROS
		.iter()
		.find(|(name, _, _)| *name == sig.name)
	{
		check_against(sig, *shape, CallOrigin::Macro, out);
		return;
	}

	// A call whose target is a bare identifier may be a qualified function
	// (`optional.of(x)`, `math.ceil(x)`). The call signature does not carry
	// the target's identity, so any name that exists as a qualified function
	// is left unchecked rather than misreported.
	if sig.method_style
		&& ctx
			.qualified_functions
			.keys()
			.any(|(_, name)| name == sig.name)
	{
		return;
	}

	// A bare call has no opaque fallback, so an unknown name always fails at
	// runtime. A method-style call only reaches this point when the declared
	// opaque surface ruled out interception.
	out.push(diagnostic(
		sig,
		CallOrigin::Function,
		DiagnosticKind::UnknownFunction,
	));
}

fn check_against(
	sig: &CallSignature,
	meta: FunctionMeta,
	origin: CallOrigin,
	out: &mut Vec<Diagnostic>,
) {
	match meta.receiver {
		ReceiverStyle::Required if !sig.method_style => {
			out.push(diagnostic(sig, origin, DiagnosticKind::ReceiverMissing));
			return;
		},
		ReceiverStyle::Forbidden if sig.method_style => {
			out.push(diagnostic(sig, origin, DiagnosticKind::ReceiverForbidden));
		},
		_ => {},
	}

	// A Forbidden function's declared range already excludes the receiver.
	let written = written_args(sig);
	let (min, max) = if sig.method_style && meta.receiver != ReceiverStyle::Forbidden {
		(
			meta.min_args.saturating_sub(1),
			meta.max_args.map(|m| m.saturating_sub(1)),
		)
	} else {
		(meta.min_args, meta.max_args)
	};
	if written < min {
		out.push(diagnostic(
			sig,
			origin,
			DiagnosticKind::TooFewArguments { min },
		));
	} else if let Some(max) = max
		&& written > max
	{
		out.push(diagnostic(
			sig,
			origin,
			DiagnosticKind::TooManyArguments { max },
		));
	}
}

/// The argument count as the user wrote it: a method-style receiver is not
/// counted.
fn written_args(sig: &CallSignature) -> usize {
	sig.arity - usize::from(sig.method_style)
}

fn diagnostic(sig: &CallSignature, origin: CallOrigin, kind: DiagnosticKind) -> Diagnostic {
	Diagnostic {
		function: sig.name.to_string(),
		args: written_args(sig),
		method_style: sig.method_style,
		origin,
		kind,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Program;

	fn check(expr: &str, ctx: &Context) -> Vec<Diagnostic> {
		Program::compile_unoptimized(expr).unwrap().check(ctx)
	}

	/// A context whose opaque surface is declared complete-and-empty, enabling
	/// full checking of method-style calls.
	fn checked_ctx() -> Context {
		let mut ctx = Context::default();
		ctx.declare_opaque_methods(std::iter::empty::<String>());
		ctx
	}

	#[test]
	fn operator_dense_expression_is_clean() {
		let ctx = checked_ctx();
		assert_eq!(check("a == b && c > d || e in f ? g : h", &ctx), vec![]);
		assert_eq!(check("!a && -b < c && d[0] != e % f", &ctx), vec![]);
	}

	#[test]
	fn well_formed_macros_are_clean() {
		let ctx = checked_ctx();
		assert_eq!(check("has(a.b)", &ctx), vec![]);
		assert_eq!(check("list.map(x, x * 2)", &ctx), vec![]);
		assert_eq!(check("list.map(x, x > 1, x * 2)", &ctx), vec![]);
		assert_eq!(check("list.filter(x, x > 1)", &ctx), vec![]);
		assert_eq!(check("list.exists(x, x > 1)", &ctx), vec![]);
		assert_eq!(check("list.exists_one(x, x > 1)", &ctx), vec![]);
		assert_eq!(check("list.existsOne(x, x > 1)", &ctx), vec![]);
		assert_eq!(check("list.all(x, x > 1)", &ctx), vec![]);
	}

	#[test]
	fn wrong_arity_macro_uses_are_diagnosed() {
		let ctx = checked_ctx();

		let diags = check("list.map(x)", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].origin, CallOrigin::Macro);
		assert_eq!(diags[0].kind, DiagnosticKind::TooFewArguments { min: 2 });
		assert_eq!(
			diags[0].to_string(),
			"the 'map' macro requires at least 2 arguments but 1 was provided"
		);

		let diags = check("has(a.b, c)", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::TooManyArguments { max: 1 });
		assert_eq!(
			diags[0].to_string(),
			"the 'has' macro accepts at most 1 argument but 2 were provided"
		);

		let diags = check("list.filter(x)", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::TooFewArguments { min: 2 });

		let diags = check("map(a, b)", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::ReceiverMissing);
		assert_eq!(
			diags[0].to_string(),
			"the 'map' macro must be called as a method: `x.map(...)`"
		);

		let diags = check("x.has(y)", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::ReceiverForbidden);
		assert_eq!(
			diags[0].to_string(),
			"the 'has' macro cannot be called as a method: `has(...)`"
		);
	}

	#[test]
	fn unknown_bare_call_is_diagnosed_even_without_opaque_surface() {
		let ctx = Context::default();
		let diags = check("contians('a', 'b')", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::UnknownFunction);
		assert_eq!(diags[0].to_string(), "unknown function 'contians'");
	}

	#[test]
	fn method_calls_are_unchecked_until_surface_is_declared() {
		let ctx = Context::default();
		assert_eq!(check("secret.unredacted()", &ctx), vec![]);
		assert_eq!(check("x.contians('a')", &ctx), vec![]);
		// Even a mismatched macro: its fallen-through call could be intercepted.
		assert_eq!(check("list.map(x)", &ctx), vec![]);
	}

	#[test]
	fn declared_opaque_methods_suppress_only_their_names() {
		let mut ctx = Context::default();
		ctx.declare_opaque_methods(["unredacted"]);
		assert_eq!(check("secret.unredacted()", &ctx), vec![]);
		let diags = check("secret.unredactd()", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::UnknownFunction);
		assert_eq!(diags[0].to_string(), "unknown method 'unredactd'");
	}

	#[test]
	fn opaque_methods_shadow_global_functions() {
		let mut ctx = checked_ctx();
		assert_eq!(
			check("x.size(y)", &ctx),
			vec![Diagnostic {
				function: "size".to_string(),
				args: 1,
				method_style: true,
				origin: CallOrigin::Function,
				kind: DiagnosticKind::TooManyArguments { max: 0 },
			}]
		);
		assert_eq!(
			check("x.size(y)", &ctx)[0].to_string(),
			"'size' accepts no arguments but 1 was provided"
		);
		ctx.declare_opaque_methods(["size"]);
		assert_eq!(check("x.size(y)", &ctx), vec![]);
	}

	#[test]
	fn either_style_arity_is_checked_in_both_styles() {
		let ctx = checked_ctx();
		assert_eq!(check("size(a)", &ctx), vec![]);
		assert_eq!(check("a.size()", &ctx), vec![]);

		let diags = check("size(a, b)", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::TooManyArguments { max: 1 });
		assert_eq!(
			diags[0].to_string(),
			"'size' accepts at most 1 argument but 2 were provided"
		);

		let diags = check("size()", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::TooFewArguments { min: 1 });

		let diags = check("a.contains()", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::TooFewArguments { min: 1 });
		assert_eq!(
			diags[0].to_string(),
			"'contains' requires at least 1 argument but 0 were provided"
		);
	}

	#[test]
	fn forbidden_receiver_is_diagnosed_but_arity_still_checked() {
		let ctx = checked_ctx();
		let diags = check("x.duration('1h')", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::ReceiverForbidden);
		assert_eq!(
			diags[0].to_string(),
			"'duration' cannot be called as a method: `duration(...)`"
		);

		let diags = check("duration('1h', '2h')", &ctx);
		assert_eq!(diags.len(), 1);
		assert_eq!(diags[0].kind, DiagnosticKind::TooManyArguments { max: 1 });

		assert_eq!(check("max(a, b, c, d, e)", &ctx), vec![]);
		assert_eq!(check("max()", &ctx), vec![]);
	}

	#[test]
	fn qualified_functions_are_unchecked() {
		let ctx = checked_ctx();
		assert_eq!(check("optional.of(x)", &ctx), vec![]);
		assert_eq!(check("optional.none()", &ctx), vec![]);
		// Even at arities it would reject: name alone cannot tell the two apart.
		assert_eq!(check("optional.of(x, y)", &ctx), vec![]);
	}

	#[test]
	fn functions_without_metadata_are_unchecked() {
		let mut ctx = checked_ctx();
		ctx.add_function("mystery", crate::functions::size);
		assert_eq!(check("mystery()", &ctx), vec![]);
		assert_eq!(check("mystery(a, b, c)", &ctx), vec![]);
	}

	/// Drift guard: metadata that declares a minimum arity must agree with the
	/// implementation. Every default function whose declared shape rules out a
	/// bare zero-argument call must actually fail one at runtime, and the two
	/// variadic ones that declare min 0 must not.
	#[test]
	fn declared_minimums_match_runtime_behavior() {
		let ctx = Context::default();
		let names: Vec<String> = ctx.functions.keys().cloned().collect();
		for name in names {
			let Some(meta) = ctx.function_meta(&name) else {
				continue;
			};
			if meta.receiver == ReceiverStyle::Required {
				continue;
			}
			let Ok(program) = Program::compile(&format!("{name}()")) else {
				continue;
			};
			let result = program.execute(&ctx);
			if meta.min_args > 0 {
				assert!(
					result.is_err(),
					"{name}() succeeded at runtime but metadata declares min_args {}",
					meta.min_args
				);
			} else {
				assert!(
					result.is_ok(),
					"{name}() failed at runtime ({result:?}) but metadata declares min_args 0",
				);
			}
		}
	}
}
