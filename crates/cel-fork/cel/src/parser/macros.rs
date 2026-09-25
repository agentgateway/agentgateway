use crate::common::ast::{CallExpr, ComprehensionExpr, Expr, IdedExpr, ListExpr, operators};
use crate::common::value::CelVal::{Boolean, Int};
use crate::context::{FunctionMeta, ReceiverStyle};
use crate::parser::{MacroExprHelper, ParseError};

pub type MacroExpander = fn(
	helper: &mut MacroExprHelper,
	target: Option<IdedExpr>,
	args: Vec<IdedExpr>,
) -> Result<IdedExpr, ParseError>;

/// The comprehension macros and the call shapes they expand at. A call
/// matching a shape is expanded at parse time and never appears as a call
/// node, so the checker treats a surviving call node with one of these names
/// as a mismatched use. Arity counts include the receiver, per
/// [`FunctionMeta`]'s convention.
pub(crate) const MACROS: &[(&str, FunctionMeta, MacroExpander)] = &[
	(operators::HAS, FunctionMeta::global(1), has_macro_expander),
	(
		operators::EXISTS,
		FunctionMeta::method(3),
		exists_macro_expander,
	),
	(operators::ALL, FunctionMeta::method(3), all_macro_expander),
	(
		operators::EXISTS_ONE,
		FunctionMeta::method(3),
		exists_one_macro_expander,
	),
	(
		"existsOne",
		FunctionMeta::method(3),
		exists_one_macro_expander,
	),
	(
		operators::MAP,
		FunctionMeta::method(3).up_to(4),
		map_macro_expander,
	),
	(
		operators::FILTER,
		FunctionMeta::method(3),
		filter_macro_expander,
	),
];

pub fn find_expander(
	func_name: &str,
	target: Option<&IdedExpr>,
	args: &[IdedExpr],
) -> Option<MacroExpander> {
	let (_, shape, expander) = MACROS.iter().find(|(name, _, _)| *name == func_name)?;
	let receiver_ok = match shape.receiver {
		ReceiverStyle::Required => target.is_some(),
		ReceiverStyle::Forbidden => target.is_none(),
		ReceiverStyle::Either => true,
	};
	let arity = args.len() + usize::from(target.is_some());
	(receiver_ok && arity >= shape.min_args && shape.max_args.is_none_or(|max| arity <= max))
		.then_some(*expander)
}

fn has_macro_expander(
	helper: &mut MacroExprHelper,
	target: Option<IdedExpr>,
	mut args: Vec<IdedExpr>,
) -> Result<IdedExpr, ParseError> {
	if target.is_some() {
		unreachable!("Got a target when expecting `None`!")
	}
	if args.len() != 1 {
		unreachable!("Expected a single arg!")
	}

	let ided_expr = args.remove(0);
	match ided_expr.expr {
		Expr::Select(mut select) => {
			select.test = true;
			Ok(helper.next_expr(Expr::Select(select)))
		},
		_ => Err(ParseError {
			source: None,
			pos: helper.pos_for(ided_expr.id).unwrap_or_default(),
			msg: "invalid argument to has() macro".to_string(),
			expr_id: 0,
			source_info: None,
		}),
	}
}

fn exists_macro_expander(
	helper: &mut MacroExprHelper,
	target: Option<IdedExpr>,
	mut args: Vec<IdedExpr>,
) -> Result<IdedExpr, ParseError> {
	if target.is_none() {
		unreachable!("Expected a target, but got `None`!")
	}
	if args.len() != 2 {
		unreachable!("Expected two args!")
	}

	let mut arguments = vec![args.remove(1)];
	let v = extract_ident(args.remove(0), helper)?;

	let init = helper.next_expr(Expr::Literal(Boolean(false)));
	let result_binding = "@result".to_string();
	let accu_ident = helper.next_expr(Expr::Ident(result_binding.clone()));
	let arg = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::LOGICAL_NOT.to_string(),
		target: None,
		args: vec![accu_ident],
	}));
	let condition = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::NOT_STRICTLY_FALSE.to_string(),
		target: None,
		args: vec![arg],
	}));

	arguments.insert(0, helper.next_expr(Expr::Ident(result_binding.clone())));
	let step = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::LOGICAL_OR.to_string(),
		target: None,
		args: arguments,
	}));

	let result = helper.next_expr(Expr::Ident(result_binding.clone()));

	Ok(
		helper.next_expr(Expr::Comprehension(Box::new(ComprehensionExpr {
			iter_range: target.unwrap(),
			iter_var: v,
			iter_var2: None,
			accu_var: result_binding,
			accu_init: init,
			loop_cond: condition,
			loop_step: step,
			result,
		}))),
	)
}
fn all_macro_expander(
	helper: &mut MacroExprHelper,
	target: Option<IdedExpr>,
	mut args: Vec<IdedExpr>,
) -> Result<IdedExpr, ParseError> {
	if target.is_none() {
		unreachable!("Expected a target, but got `None`!")
	}
	if args.len() != 2 {
		unreachable!("Expected two args!")
	}

	let mut arguments = vec![args.remove(1)];
	let v = extract_ident(args.remove(0), helper)?;

	let init = helper.next_expr(Expr::Literal(Boolean(true)));
	let result_binding = "@result".to_string();
	let accu_ident = helper.next_expr(Expr::Ident(result_binding.clone()));
	let condition = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::NOT_STRICTLY_FALSE.to_string(),
		target: None,
		args: vec![accu_ident],
	}));

	arguments.insert(0, helper.next_expr(Expr::Ident(result_binding.clone())));
	let step = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::LOGICAL_AND.to_string(),
		target: None,
		args: arguments,
	}));

	let result = helper.next_expr(Expr::Ident(result_binding.clone()));

	Ok(
		helper.next_expr(Expr::Comprehension(Box::new(ComprehensionExpr {
			iter_range: target.unwrap(),
			iter_var: v,
			iter_var2: None,
			accu_var: result_binding,
			accu_init: init,
			loop_cond: condition,
			loop_step: step,
			result,
		}))),
	)
}

fn exists_one_macro_expander(
	helper: &mut MacroExprHelper,
	target: Option<IdedExpr>,
	mut args: Vec<IdedExpr>,
) -> Result<IdedExpr, ParseError> {
	if target.is_none() {
		unreachable!("Expected a target, but got `None`!")
	}
	if args.len() != 2 {
		unreachable!("Expected two args!")
	}

	let mut arguments = vec![args.remove(1)];
	let v = extract_ident(args.remove(0), helper)?;

	let init = helper.next_expr(Expr::Literal(Int(0)));
	let result_binding = "@result".to_string();
	let condition = helper.next_expr(Expr::Literal(Boolean(true)));

	let args = vec![
		helper.next_expr(Expr::Ident(result_binding.clone())),
		helper.next_expr(Expr::Literal(Int(1))),
	];
	arguments.push(helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::ADD.to_string(),
		target: None,
		args,
	})));
	arguments.push(helper.next_expr(Expr::Ident(result_binding.clone())));

	let step = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::CONDITIONAL.to_string(),
		target: None,
		args: arguments,
	}));

	let accu = helper.next_expr(Expr::Ident(result_binding.clone()));
	let one = helper.next_expr(Expr::Literal(Int(1)));
	let result = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::EQUALS.to_string(),
		target: None,
		args: vec![accu, one],
	}));

	Ok(
		helper.next_expr(Expr::Comprehension(Box::new(ComprehensionExpr {
			iter_range: target.unwrap(),
			iter_var: v,
			iter_var2: None,
			accu_var: result_binding,
			accu_init: init,
			loop_cond: condition,
			loop_step: step,
			result,
		}))),
	)
}

fn map_macro_expander(
	helper: &mut MacroExprHelper,
	target: Option<IdedExpr>,
	mut args: Vec<IdedExpr>,
) -> Result<IdedExpr, ParseError> {
	if target.is_none() {
		unreachable!("Expected a target, but got `None`!")
	}
	if args.len() != 2 && args.len() != 3 {
		unreachable!("Expected two or three args!")
	}

	let func = args.pop().unwrap();
	let v = extract_ident(args.remove(0), helper)?;

	let init = helper.next_expr(Expr::List(ListExpr::new(Vec::default())));
	let result_binding = "@result".to_string();
	let condition = helper.next_expr(Expr::Literal(Boolean(true)));

	let filter = args.pop();

	let args = vec![
		helper.next_expr(Expr::Ident(result_binding.clone())),
		helper.next_expr(Expr::List(ListExpr::new(vec![func]))),
	];
	let step = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::ADD.to_string(),
		target: None,
		args,
	}));

	let step = match filter {
		Some(filter) => {
			let accu = helper.next_expr(Expr::Ident(result_binding.clone()));
			helper.next_expr(Expr::Call(CallExpr {
				func_name: operators::CONDITIONAL.to_string(),
				target: None,
				args: vec![filter, step, accu],
			}))
		},
		None => step,
	};

	let result = helper.next_expr(Expr::Ident(result_binding.clone()));

	Ok(
		helper.next_expr(Expr::Comprehension(Box::new(ComprehensionExpr {
			iter_range: target.unwrap(),
			iter_var: v,
			iter_var2: None,
			accu_var: result_binding,
			accu_init: init,
			loop_cond: condition,
			loop_step: step,
			result,
		}))),
	)
}

fn filter_macro_expander(
	helper: &mut MacroExprHelper,
	target: Option<IdedExpr>,
	mut args: Vec<IdedExpr>,
) -> Result<IdedExpr, ParseError> {
	if target.is_none() {
		unreachable!("Expected a target, but got `None`!")
	}
	if args.len() != 2 {
		unreachable!("Expected two args!")
	}

	let var = args.remove(0);
	let v = extract_ident(var.clone(), helper)?;
	let filter = args.pop().unwrap();

	let init = helper.next_expr(Expr::List(ListExpr::new(Vec::default())));
	let result_binding = "@result".to_string();
	let condition = helper.next_expr(Expr::Literal(Boolean(true)));

	let args = vec![
		helper.next_expr(Expr::Ident(result_binding.clone())),
		helper.next_expr(Expr::List(ListExpr::new(vec![var]))),
	];
	let step = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::ADD.to_string(),
		target: None,
		args,
	}));

	let accu = helper.next_expr(Expr::Ident(result_binding.clone()));
	let step = helper.next_expr(Expr::Call(CallExpr {
		func_name: operators::CONDITIONAL.to_string(),
		target: None,
		args: vec![filter, step, accu],
	}));

	let result = helper.next_expr(Expr::Ident(result_binding.clone()));

	Ok(
		helper.next_expr(Expr::Comprehension(Box::new(ComprehensionExpr {
			iter_range: target.unwrap(),
			iter_var: v,
			iter_var2: None,
			accu_var: result_binding,
			accu_init: init,
			loop_cond: condition,
			loop_step: step,
			result,
		}))),
	)
}

fn extract_ident(expr: IdedExpr, helper: &mut MacroExprHelper) -> Result<String, ParseError> {
	match expr.expr {
		Expr::Ident(ident) => Ok(ident),
		_ => Err(ParseError {
			source: None,
			pos: helper.pos_for(expr.id).unwrap_or_default(),
			msg: "argument must be a simple name".to_string(),
			expr_id: 0,
			source_info: None,
		}),
	}
}
