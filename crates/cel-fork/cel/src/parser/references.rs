use std::collections::HashSet;

use crate::common::ast::{Expr, IdedExpr};
use crate::objects::standard_type;

/// A collection of all the references that an expression makes to variables and functions.
pub struct ExpressionReferences<'expr> {
	variables: HashSet<&'expr str>,
	functions: HashSet<&'expr str>,
}

/// One function call site: the name invoked and the arity it was invoked at.
///
/// `arity` counts a method-style receiver, so `x.contains(y)` and
/// `contains(x, y)` both report 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallSignature<'expr> {
	pub name: &'expr str,
	pub arity: usize,
	pub method_style: bool,
}

impl ExpressionReferences<'_> {
	/// Returns true if the expression references the provided variable name.
	///
	/// # Example
	/// ```rust
	/// # use cel::parser::Parser;
	/// let expression = Parser::new().parse("foo.bar == true").unwrap();
	/// let references = expression.references();
	/// assert!(references.has_variable("foo"));
	/// ```
	pub fn has_variable(&self, name: impl AsRef<str>) -> bool {
		self.variables.contains(name.as_ref())
	}

	/// Returns true if the expression references the provided function name.
	///
	/// # Example
	/// ```rust
	/// # use cel::parser::Parser;
	/// let expression = Parser::new().parse("size(foo) > 0").unwrap();
	/// let references = expression.references();
	/// assert!(references.has_function("size"));
	/// ```
	pub fn has_function(&self, name: impl AsRef<str>) -> bool {
		self.functions.contains(name.as_ref())
	}

	/// Returns a list of all variables referenced in the expression.
	///
	/// # Example
	/// ```rust
	/// # use cel::parser::Parser;
	/// let expression = Parser::new().parse("foo.bar == true").unwrap();
	/// let references = expression.references();
	/// assert_eq!(vec!["foo"], references.variables());
	/// ```
	pub fn variables(&self) -> Vec<&str> {
		self.variables.iter().copied().collect()
	}

	/// Returns a list of all functions referenced in the expression.
	///
	/// # Example
	/// ```rust
	/// # use cel::parser::Parser;
	/// let expression = Parser::new().parse("size(foo) > 0").unwrap();
	/// let references = expression.references();
	/// assert!(references.functions().contains(&"_>_"));
	/// assert!(references.functions().contains(&"size"));
	/// ```
	pub fn functions(&self) -> Vec<&str> {
		self.functions.iter().copied().collect()
	}
}

impl IdedExpr {
	/// Visits every sub-expression, parents before children.
	///
	/// # Example
	/// ```rust
	/// # use cel::parser::Parser;
	/// let expression = Parser::new().parse("1 + 2").unwrap();
	/// let mut count = 0;
	/// expression.walk(&mut |_| count += 1);
	/// assert_eq!(count, 3);
	/// ```
	pub fn walk<'expr>(&'expr self, visit: &mut impl FnMut(&'expr IdedExpr)) {
		visit(self);
		match &self.expr {
			Expr::Optimized { original, .. } => original.walk(visit),
			Expr::Unspecified | Expr::Ident(_) | Expr::Literal(_) | Expr::Inline(_) => {},
			Expr::Call(call) => {
				if let Some(target) = &call.target {
					target.walk(visit);
				}
				for arg in &call.args {
					arg.walk(visit);
				}
			},
			Expr::Comprehension(comp) => {
				comp.iter_range.walk(visit);
				comp.accu_init.walk(visit);
				comp.loop_cond.walk(visit);
				comp.loop_step.walk(visit);
				comp.result.walk(visit);
			},
			Expr::List(list) => {
				for elem in &list.elements {
					elem.walk(visit);
				}
			},
			Expr::Map(map) => walk_entries(&map.entries, visit),
			Expr::Struct(struct_expr) => walk_entries(&struct_expr.entries, visit),
			Expr::Select(select) => select.operand.walk(visit),
		}
	}

	/// Returns a set of all variables and functions referenced in the expression.
	///
	/// # Example
	/// ```rust
	/// # use cel::parser::Parser;
	/// let expression = Parser::new().parse("foo && size(foo) > 0").unwrap();
	/// let references = expression.references();
	///
	/// assert!(references.has_variable("foo"));
	/// assert!(references.has_function("size"));
	/// ```
	pub fn references(&self) -> ExpressionReferences<'_> {
		let mut variables = HashSet::new();
		let mut functions = HashSet::new();
		self.walk(&mut |e| match &e.expr {
			Expr::Call(call) => {
				functions.insert(call.func_name.as_str());
			},
			Expr::Ident(name) => {
				// todo! Might want to make this "smarter" (are we in a comprehension?) and better encode these in const
				if !name.starts_with('@') && standard_type(name).is_none() {
					variables.insert(name.as_str());
				}
			},
			Expr::Optimized { .. }
			| Expr::Unspecified
			| Expr::Literal(_)
			| Expr::Inline(_)
			| Expr::Comprehension(_)
			| Expr::List(_)
			| Expr::Map(_)
			| Expr::Struct(_)
			| Expr::Select(_) => {},
		});
		ExpressionReferences {
			variables,
			functions,
		}
	}

	/// Returns every function call in the expression, with the arity it was
	/// called at.
	///
	/// # Example
	/// ```rust
	/// # use cel::parser::Parser;
	/// let expression = Parser::new().parse("'ab'.contains('a')").unwrap();
	/// let calls = expression.call_signatures();
	///
	/// let contains = calls.iter().find(|c| c.name == "contains").unwrap();
	/// assert_eq!(contains.arity, 2);
	/// assert!(contains.method_style);
	/// ```
	pub fn call_signatures(&self) -> Vec<CallSignature<'_>> {
		let mut calls = Vec::new();
		self.walk(&mut |e| {
			if let Expr::Call(call) = &e.expr {
				calls.push(CallSignature {
					name: &call.func_name,
					arity: call.args.len() + usize::from(call.target.is_some()),
					method_style: call.target.is_some(),
				});
			}
		});
		calls
	}
}

fn walk_entries<'expr>(
	entries: &'expr [crate::common::ast::IdedEntryExpr],
	visit: &mut impl FnMut(&'expr IdedExpr),
) {
	for entry in entries {
		match &entry.expr {
			crate::common::ast::EntryExpr::StructField(field) => field.value.walk(visit),
			crate::common::ast::EntryExpr::MapEntry(map_entry) => {
				map_entry.key.walk(visit);
				map_entry.value.walk(visit);
			},
		}
	}
}
