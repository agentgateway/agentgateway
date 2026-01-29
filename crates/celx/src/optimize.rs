use std::sync::Arc;

use cel::common::ast::{CallExpr, Expr, SelectExpr};
use cel::objects::{OpaqueValue, StringValue};
use cel::{Context, ExecutionError, FunctionContext, IdedExpr, ResolveResult, Value};
use serde::Serialize;

pub fn insert_all(ctx: &mut Context) {
	ctx.add_function("precompiled_matches", PrecompileRegex::precompiled_matches)
}

pub struct DefaultOptimizer;
impl DefaultOptimizer {
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
