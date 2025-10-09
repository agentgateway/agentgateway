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
	let expr = r#""hello".base64Encode()"#;
	assert(json!("aGVsbG8="), expr);
	let expr = r#"string("hello".base64Encode().base64Decode())"#;
	assert(json!("hello"), expr);
}

#[test]
fn map_values() {
	let expr = r#"{"a": 1, "b": 2}.mapValues(v, v * 2)"#;
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

#[test]
fn regex_replace() {
	let expr = r#""/path/1/id/499c81c2/bar".regexReplace("/path/([0-9]+?)/id/([0-9a-z]{8})/bar", "/path/{n}/id/{id}/bar")"#;
	assert(json!("/path/{n}/id/{id}/bar"), expr);
	let expr = r#""blah id=1234 bar".regexReplace("id=(.+?) ", "[$1] ")"#;
	assert(json!("blah [1234] bar"), expr);
	let expr = r#""/id/1234/data".regexReplace("/id/[0-9]*/", "/id/{id}/")"#;
	assert(json!("/id/{id}/data"), expr);
	let expr = r#""ab".regexReplace("a" + "b", "12")"#;
	assert(json!("12"), expr);
}

#[test]
fn merge_maps() {
	let expr = r#"{"a":2}.merge({"b":3})"#;
	assert(json!({"a":2, "b":3}), expr);
	let expr = r#"{"a":2}.merge({"a":3})"#;
	assert(json!({"a":3}), expr);
}

fn assert(want: serde_json::Value, expr: &str) {
	assert_eq!(
		want,
		eval(expr).unwrap().json().unwrap(),
		"expression: {expr}"
	);
}
