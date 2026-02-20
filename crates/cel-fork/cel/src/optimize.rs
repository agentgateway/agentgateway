use std::sync::Arc;

use http::HeaderName;

use crate::common::ast::{
	CallExpr, ComprehensionExpr, EntryExpr, Expr, IdedEntryExpr, ListExpr, MapEntryExpr, MapExpr,
	OptimizedExpr, SelectExpr, operators,
};
use crate::objects::{ListValue, MapValue};
use crate::parser::Expression;
use crate::{IdedExpr, Value};

fn is_lit(e: &Expr) -> bool {
	matches!(e, Expr::Literal(_) | Expr::Inline(_))
}

fn as_value(e: IdedExpr) -> Value<'static> {
	assert!(is_lit(&e.expr));
	match e.expr {
		Expr::Literal(l) => Value::from(l),
		Expr::Inline(l) => l,
		_ => unreachable!(),
	}
}

/// An Optimizer is an input to the optimization process that defines user-specific optimizations.
/// While the default universal optimizations apply automatically, Optimizer allows specialized optimizations.
pub trait Optimizer {
	/// optimize will be called on each node *after* default optimizations, such as inlining, are done.
	/// Because this is called for each node, rather than just the top level expression, traversing the AST
	/// is not necessary.
	fn optimize(&self, _expr: &Expr) -> Option<Expr> {
		None
	}
}

struct DefaultOptimizer;
impl Optimizer for DefaultOptimizer {}

pub struct Optimize {
	optimizer: Box<dyn Optimizer>,
}

impl Optimize {
	pub fn new() -> Self {
		Self {
			optimizer: Box::new(DefaultOptimizer),
		}
	}
	pub fn new_with_optimizer<T: Optimizer + 'static>(t: T) -> Self {
		Self {
			optimizer: Box::new(t),
		}
	}

	pub fn optimize(&self, expr: Expression) -> Expression {
		let id = expr.id;
		let with_id = |expr: Expr| Expression { id, expr };
		let res = match expr.expr {
			Expr::Call(c) => {
				let target = c.target.map(|t| Box::new(self.optimize(*t)));
				let args = c
					.args
					.into_iter()
					.map(|a| self.optimize(a))
					.collect::<Vec<_>>();
				let call = CallExpr {
					target,
					args,
					func_name: c.func_name,
				};
				let expr = Expr::Call(call);
				let res = self.optimizer.optimize(&expr).unwrap_or(expr);
				with_id(res)
			},
			Expr::Comprehension(c) => {
				let expr = Expr::Comprehension(Box::new(ComprehensionExpr {
					iter_range: self.optimize(c.iter_range),
					iter_var: c.iter_var,
					iter_var2: c.iter_var2,
					accu_var: c.accu_var,
					accu_init: self.optimize(c.accu_init),
					loop_cond: self.optimize(c.loop_cond),
					loop_step: self.optimize(c.loop_step),
					result: self.optimize(c.result),
				}));
				with_id(self.optimizer.optimize(&expr).unwrap_or(expr))
			},
			Expr::Select(s) => {
				let expr = Expr::Select(SelectExpr {
					operand: Box::new(self.optimize(*s.operand)),
					field: s.field,
					test: s.test,
				});
				with_id(self.optimizer.optimize(&expr).unwrap_or(expr))
			},
			Expr::Struct(_) => {
				todo!()
			},
			Expr::List(v) => {
				let nl: Vec<IdedExpr> = v.elements.into_iter().map(|a| self.optimize(a)).collect();
				let expr = if nl.iter().all(|nl| is_lit(&nl.expr)) {
					Expr::Inline(Value::List(ListValue::Owned(
						nl.into_iter().map(as_value).collect(),
					)))
				} else {
					Expr::List(ListExpr {
						elements: nl,
						optional_indices: v.optional_indices,
					})
				};
				with_id(self.optimizer.optimize(&expr).unwrap_or(expr))
			},
			Expr::Map(m) => {
				let ne: Vec<IdedEntryExpr> = m
					.entries
					.into_iter()
					.map(|e| match e.expr {
						EntryExpr::MapEntry(me) => {
							let value = self.optimize(me.value);
							let key = self.optimize(me.key);
							let ne = MapEntryExpr {
								value,
								key,
								optional: me.optional,
							};
							IdedEntryExpr {
								id: e.id,
								expr: EntryExpr::MapEntry(ne),
							}
						},
						_ => unreachable!(),
					})
					.collect();
				let expr = if ne.iter().all(|nl| match &nl.expr {
					EntryExpr::MapEntry(me) => is_lit(&me.key.expr) && is_lit(&me.value.expr),
					_ => unreachable!(),
				}) {
					let r = ne
						.iter()
						.map(|e| match &e.expr {
							EntryExpr::MapEntry(me) => Ok((
								as_value(me.key.clone()).try_into()?,
								as_value(me.value.clone()),
							)),
							_ => unreachable!(),
						})
						.collect::<Result<_, Value<'static>>>();
					match r {
						Ok(v) => Expr::Inline(Value::Map(MapValue::Owned(Arc::new(v)))),
						Err(_) => Expr::Map(MapExpr { entries: ne }),
					}
				} else {
					Expr::Map(MapExpr { entries: ne })
				};
				with_id(self.optimizer.optimize(&expr).unwrap_or(expr))
			},
			Expr::Literal(value) => {
				let expr = Expr::Inline(Value::from(value));
				with_id(self.optimizer.optimize(&expr).unwrap_or(expr))
			},
			expr => with_id(self.optimizer.optimize(&expr).unwrap_or(expr)),
		};

		// Specialize `request.header[value]`
		if let Expr::Call(call) = &res.expr
			&& call.args.len() == 2
			&& call.func_name == operators::INDEX
			&& let Expr::Select(se) = &call.args[0].expr
			&& let Expr::Inline(Value::String(field)) = &call.args[1].expr
			&& !se.test
			&& let Expr::Ident(base) = &se.operand.expr
			&& (base == "request" || base == "response")
			&& se.field == "headers"
			&& let Ok(header) = HeaderName::from_bytes(field.as_bytes())
		{
			let request = base == "request";
			return with_id(Expr::Optimized {
				original: Box::new(res),
				optimized: OptimizedExpr::HeaderLookup { request, header },
			});
		}

		res
	}
}

#[cfg(test)]
mod test {
	use serde::{Serialize, Serializer};

	use crate::common::ast::{CallExpr, Expr};

	use crate::objects::{Opaque, OpaqueValue};
	use crate::{Context, ExecutionError, FunctionContext, IdedExpr, Program, ResolveResult, Value};

	pub struct RegexOptimizer;
	impl RegexOptimizer {
		fn specialize_call(&self, c: &CallExpr) -> Option<Expr> {
			fn expr_as_value(e: IdedExpr) -> Option<Value<'static>> {
				match e.expr {
					Expr::Literal(l) => Some(Value::from(l)),
					Expr::Inline(l) => Some(l),
					_ => None,
				}
			}
			match c.func_name.as_str() {
				"matches" if c.args.len() == 1 && c.target.is_some() => {
					let t = c.target.clone()?;
					let arg = c.args.first()?.clone();
					let id = arg.id;
					let Value::String(arg) = expr_as_value(arg)? else {
						return None;
					};

					// TODO: translate regex compile failures into inlined failures
					let opaque = Value::Object(OpaqueValue::new(PrecompileRegex(
						regex::Regex::new(&arg).ok()?,
					)));
					let id_expr = IdedExpr {
						id,
						expr: Expr::Inline(opaque),
					};
					// We invert this to be 'regex.precompiled_matches(string)'
					// instead of 'string.matches(regex)'
					Some(Expr::Call(CallExpr {
						func_name: "precompiled_matches".to_string(),
						target: Some(Box::new(id_expr)),
						args: vec![*t],
					}))
				},
				_ => None,
			}
		}
	}

	impl crate::Optimizer for RegexOptimizer {
		fn optimize(&self, c: &Expr) -> Option<Expr> {
			match c {
				Expr::Call(c) => self.specialize_call(c),
				_ => None,
			}
		}
	}

	#[derive(Debug, Clone)]
	struct PrecompileRegex(regex::Regex);

	impl PartialEq for PrecompileRegex {
		fn eq(&self, other: &Self) -> bool {
			self.0.as_str() == other.0.as_str()
		}
	}
	impl Eq for PrecompileRegex {}

	impl Opaque for PrecompileRegex {
		fn type_name(&self) -> &'static str {
			"precompiled_regex"
		}
	}

	impl Serialize for PrecompileRegex {
		fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
		where
			S: Serializer,
		{
			serializer.serialize_str(self.0.as_str())
		}
	}

	impl PrecompileRegex {
		pub fn precompiled_matches<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
			let this: Value = ftx.this()?;
			let val: std::sync::Arc<str> = ftx.arg(0)?;
			let Value::Object(obj) = this else {
				return Err(ExecutionError::UnexpectedType {
					got: this.type_of().as_str(),
					want: "precompiled_regex",
				});
			};
			let Some(rgx) = obj.downcast_ref::<Self>() else {
				return Err(ExecutionError::UnexpectedType {
					got: obj.type_name(),
					want: "precompiled_regex",
				});
			};
			Ok(Value::Bool(rgx.0.is_match(&val)))
		}
	}

	#[test]
	fn test_optimize_function() {
		let mut context = Context::default();
		context.add_function("precompiled_matches", PrecompileRegex::precompiled_matches);

		let program = Program::compile("'foo'.matches('fo.')")
			.unwrap()
			.optimized_with(RegexOptimizer);
		let value = program.execute(&context);
		assert_eq!(value, Ok(Value::Bool(true)));
		let Expr::Call(CallExpr {
			func_name,
			target: Some(t),
			..
		}) = &program.expression.expr
		else {
			panic!("expected optimization, got {program:?}");
		};
		assert_eq!(func_name.as_str(), "precompiled_matches");
		assert!(matches!(t.expr, Expr::Inline(Value::Object(_))));
	}
}
