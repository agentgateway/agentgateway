pub(super) fn properties<'e>(
	exp: &'e cel::common::ast::Expr,
	all: &mut Vec<Vec<&'e str>>,
	path: &mut Vec<&'e str>,
) {
	use cel::common::ast::Expr::*;
	match exp {
		Unspecified => {},
		Optimized { original, .. } => properties(&original.expr, all, path),
		Call(call) => {
			if let Some(t) = &call.target {
				properties(&t.expr, all, path)
			}
			for arg in &call.args {
				properties(&arg.expr, all, path)
			}
		},
		Select(e) => {
			path.insert(0, e.field.as_str());
			properties(&e.operand.expr, all, path);
		},
		Comprehension(call) => {
			properties(&call.iter_range.expr, all, path);
			{
				let v = &call.iter_var;
				if !v.starts_with("@") {
					path.insert(0, v.as_str());
					all.push(path.clone());
					path.clear();
				}
			}
			properties(&call.loop_step.expr, all, path);
		},
		List(e) => {
			for elem in &e.elements {
				properties(&elem.expr, all, path);
			}
		},
		Map(v) => {
			for entry in &v.entries {
				match &entry.expr {
					cel::common::ast::EntryExpr::StructField(field) => {
						properties(&field.value.expr, all, path);
					},
					cel::common::ast::EntryExpr::MapEntry(map_entry) => {
						properties(&map_entry.value.expr, all, path);
					},
				}
			}
		},
		Struct(v) => {
			for entry in &v.entries {
				match &entry.expr {
					cel::common::ast::EntryExpr::StructField(field) => {
						properties(&field.value.expr, all, path);
					},
					cel::common::ast::EntryExpr::MapEntry(map_entry) => {
						properties(&map_entry.value.expr, all, path);
					},
				}
			}
		},
		Literal(_) => {},
		Inline(_) => {},
		Ident(v) => {
			if !v.starts_with("@") {
				path.insert(0, v.as_str());
				all.push(path.clone());
				path.clear();
			}
		},
	}
}
