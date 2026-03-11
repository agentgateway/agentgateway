use std::collections::HashMap;

use cel::{Context, Program, Value, context::DefaultVariableResolver};
use divan::{Bencher, black_box};

use crate::insert_all;

fn make_ctx() -> Context {
	let mut ctx = Context::default();
	insert_all(&mut ctx);
	ctx
}

/// Build an Owned MapValue with `n` string keys (as would come from JSON parsing).
fn owned_map(n: usize) -> Value<'static> {
	let map: HashMap<String, Value<'static>> = (0..n)
		.map(|i| {
			let key = if i % 2 == 0 {
				format!("x_key{i}")
			} else {
				format!("key{i}")
			};
			(key, Value::Int(i as i64))
		})
		.collect();
	Value::Map(map.into())
}

/// Build a CEL map literal expression with `n` string keys (produces Borrow).
fn borrow_map_expr(n: usize) -> String {
	let pairs: Vec<String> = (0..n)
		.map(|i| {
			if i % 2 == 0 {
				format!(r#""x_key{i}": {i}"#)
			} else {
				format!(r#""key{i}": {i}"#)
			}
		})
		.collect();
	format!("{{{}}}", pairs.join(", "))
}

#[divan::bench(args = [2, 5, 10, 20, 50])]
fn filter_keys(b: Bencher, n: usize) {
	let ctx = make_ctx();
	let expr = format!(
		r#"{}.filterKeys(k, !k.startsWith("x_"))"#,
		borrow_map_expr(n)
	);
	let prog = Program::compile(&expr).unwrap();
	b.bench(|| {
		Value::resolve(black_box(prog.expression()), &ctx, &DefaultVariableResolver)
			.unwrap()
			.as_static()
	});
}

/// Exercises MapValue::iter() on Owned maps (Arc-backed HashMap).
#[divan::bench(args = [2, 5, 10, 20, 50])]
fn map_eq_owned(b: Bencher, n: usize) {
	let ctx = make_ctx();
	let map = owned_map(n);
	let prog = Program::compile("m == m").unwrap();
	b.with_inputs(|| map.clone()).bench_refs(|map| {
		let resolver = cel::context::SingleVarResolver::new(&DefaultVariableResolver, "m", map.clone());
		Value::resolve(black_box(prog.expression()), &ctx, &resolver)
			.unwrap()
			.as_static()
	});
}
