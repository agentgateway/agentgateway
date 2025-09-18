use cel::Value;
use serde_json::json;

use crate::cel::{ContextBuilder, Error, Expression};

fn eval(expr: &str) -> Result<Value, Error> {
	let mut cb = ContextBuilder::new();
	let exp = Expression::new(expr)?;
	cb.register_expression(&exp);
	let exec = cb.build()?;
	exec.eval(&exp)
}

#[test]
fn with() {
	let expr = r#"[1,2].with(a, a + a)"#;
	assert(json!([1, 2, 1, 2]), expr);
}

#[test]
fn json() {
	let expr = r#"json('{"hi":1}').hi"#;
	assert(json!(1), expr);
}

#[test]
fn random() {
	let expr = r#"int(random() * 10.0)"#;
	let v = eval(expr).unwrap().json().unwrap().as_i64().unwrap();
	assert!((0..=10).contains(&v));
}

#[test]
fn base64() {
	let expr = r#""hello".base64_encode()"#;
	assert(json!("aGVsbG8="), expr);
	let expr = r#"string("hello".base64_encode().base64_decode())"#;
	assert(json!("hello"), expr);
}

#[test]
fn map_values() {
	let expr = r#"{"a": 1, "b": 2}.map_values(v, v * 2)"#;
	assert(json!({"a": 2, "b": 4}), expr);
}

#[test]
fn default() {
	let expr = r#"default(a, "b")"#;
	assert(json!("b"), expr);
	let expr = r#"default({"a":1}["a"], 2)"#;
	assert(json!(1), expr);
	let expr = r#"default({"a":1}["b"], 2)"#;
	assert(json!(2), expr);
	let expr = r#"default(a.b, "b")"#;
	assert(json!("b"), expr);
}

fn assert(want: serde_json::Value, expr: &str) {
	assert_eq!(
		want,
		eval(expr).unwrap().json().unwrap(),
		"expression: {expr}"
	);
}
