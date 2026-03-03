use std::sync::Arc;

use super::format::{precompile_format, precompile_parse};
use cel::common::ast::{CallExpr, Expr, SelectExpr};
use cel::objects::{OpaqueValue, StringValue};
use cel::{Context, ExecutionError, FunctionContext, IdedExpr, ResolveResult, Value};
use serde::Serialize;

pub fn insert_all(ctx: &mut Context) {
	ctx.add_function("precompiled_matches", PrecompileRegex::precompiled_matches)
}

pub struct DefaultOptimizer;
impl DefaultOptimizer {
	fn fail_call(id: u64, msg: &str) -> Expr {
		Expr::Call(CallExpr {
			func_name: "fail".to_string(),
			target: None,
			args: vec![IdedExpr {
				id,
				expr: Expr::Inline(Value::String(StringValue::Owned(Arc::from(msg)))),
			}],
		})
	}

	fn static_string(expr: &IdedExpr) -> Option<Arc<str>> {
		let Value::String(v) = expr_as_value(expr.clone())? else {
			return None;
		};
		Some(v.as_owned())
	}

	fn specialize_member(&self, c: &SelectExpr) -> Option<Expr> {
		let SelectExpr {
			operand,
			field,
			test,
		} = c;
		if *test {
			return None;
		}
		match &operand.expr {
			// json(data).field -> jsonField(data, "field")
			Expr::Call(c) if c.func_name == "json" && c.target.is_none() && c.args.len() == 1 => {
				Some(Expr::Call(CallExpr {
					func_name: "jsonField".to_string(),
					target: None,
					args: vec![
						c.args[0].clone(),
						IdedExpr {
							id: operand.id,
							expr: Expr::Inline(Value::String(StringValue::Owned(Arc::from(field.as_str())))),
						},
					],
				}))
			},
			_ => None,
		}
	}
	fn specialize_call(&self, c: &CallExpr) -> Option<Expr> {
		match c.func_name.as_str() {
			"format" if c.target.is_some() => {
				let target = c.target.as_ref()?;
				let Some(format_literal) = Self::static_string(target) else {
					return Some(Self::fail_call(
						target.id,
						"String.format requires a static string receiver",
					));
				};
				let compiled = match precompile_format(&format_literal) {
					Ok(compiled) => compiled,
					Err(err) => {
						return Some(Self::fail_call(target.id, &format!("String.format: {err}")));
					},
				};
				let precompiled_target = IdedExpr {
					id: target.id,
					expr: Expr::Inline(Value::Object(OpaqueValue::new(compiled))),
				};
				Some(Expr::Call(CallExpr {
					func_name: "precompiled_format".to_string(),
					target: Some(Box::new(precompiled_target)),
					args: c.args.clone(),
				}))
			},
			"parse" if c.target.is_some() && c.args.len() == 2 => {
				let pattern = c.args.first()?;
				let Some(pattern_literal) = Self::static_string(pattern) else {
					return Some(Self::fail_call(
						pattern.id,
						"String.parse requires a static string pattern",
					));
				};
				let compiled = match precompile_parse(&pattern_literal) {
					Ok(compiled) => compiled,
					Err(err) => {
						return Some(Self::fail_call(pattern.id, &format!("String.parse: {err}")));
					},
				};
				let input = c.target.clone()?;
				let expr = c.args.get(1)?.clone();
				let precompiled_target = IdedExpr {
					id: pattern.id,
					expr: Expr::Inline(Value::Object(OpaqueValue::new(compiled))),
				};
				Some(Expr::Call(CallExpr {
					func_name: "precompiled_parse".to_string(),
					target: Some(Box::new(precompiled_target)),
					args: vec![*input, expr],
				}))
			},
			"cidr" if c.args.len() == 1 && c.target.is_none() => {
				let arg = c.args.first()?.clone();
				let Value::String(arg) = expr_as_value(arg)? else {
					return None;
				};
				let parsed = super::cidr::Cidr::new(&arg)?;
				Some(Expr::Inline(Value::Object(OpaqueValue::new(parsed))))
			},
			"ip" if c.args.len() == 1 && c.target.is_none() => {
				let arg = c.args.first()?.clone();
				let Value::String(arg) = expr_as_value(arg)? else {
					return None;
				};
				let parsed = super::cidr::IP::new(&arg)?;
				Some(Expr::Inline(Value::Object(OpaqueValue::new(parsed))))
			},
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
impl cel::Optimizer for DefaultOptimizer {
	fn optimize(&self, expr: &Expr) -> Option<Expr> {
		match expr {
			Expr::Select(s) => self.specialize_member(s),
			Expr::Call(c) => self.specialize_call(c),
			_ => None,
		}
	}
}

fn expr_as_value(e: IdedExpr) -> Option<Value<'static>> {
	match e.expr {
		Expr::Literal(l) => Some(Value::from(l)),
		Expr::Inline(l) => Some(l),
		_ => None,
	}
}

#[derive(Debug, Serialize)]
struct PrecompileRegex(#[serde(with = "serde_regex")] regex::Regex);
crate::impl_opaque!(PrecompileRegex, "precompiled_regex");
impl PartialEq for PrecompileRegex {
	fn eq(&self, other: &Self) -> bool {
		self.0.as_str() == other.0.as_str()
	}
}
impl Eq for PrecompileRegex {}

impl PrecompileRegex {
	crate::impl_functions! {{}, {}}
	pub fn precompiled_matches<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
		let this: Value = ftx.this()?;
		let val: Arc<str> = ftx.arg(0)?;
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
