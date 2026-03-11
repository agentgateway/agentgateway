use std::collections::HashSet;

use super::*;
use crate::http::Body;
use http::Method;
use serde_json::json;

fn eval(expr: &str) -> Result<serde_json::Value, Error> {
	let exec_serde = full_example_executor();
	let exec = exec_serde.as_executor();
	let exp = Expression::new_strict(expr)?;
	exec
		.eval(&exp)?
		.json()
		.map_err(|e| Error::Variable(format!("{e}")))
}

fn eval_request(expr: &str, req: crate::http::Request) -> Result<Value, Error> {
	let mut cb = ContextBuilder::new();
	let exp = Expression::new_strict(expr)?;
	cb.register_expression(&exp);
	let exec = crate::cel::Executor::new_request(&req);
	Ok(exec.eval(&exp)?.as_static())
}

#[test]
fn test_eval() {
	let req = ::http::Request::builder()
		.method(Method::GET)
		.header("x-example", "value")
		.body(Body::empty())
		.unwrap();
	eval_request("request.method", req).unwrap();
}

#[test]
fn expression() {
	let expr = r#"request.method == "GET" && request.headers["x-example"] == "value""#;
	let req = ::http::Request::builder()
		.method(Method::GET)
		.uri("http://example.com")
		.header("x-example", "value")
		.body(Body::empty())
		.unwrap();
	assert_eq!(Value::Bool(true), eval_request(expr, req).unwrap());
}

#[test]
fn test_properties() {
	let test = |e: &str, want: &[&str]| {
		let p = Program::compile(e).unwrap();
		let mut props = Vec::with_capacity(5);
		crate::cel::properties::properties(&p.expression().expr, &mut props, &mut Vec::default());
		let want = HashSet::from_iter(want.iter().map(|s| s.to_string()));
		let got = props
			.into_iter()
			.map(|p| p.join("."))
			.collect::<HashSet<_>>();
		assert_eq!(want, got, "expression: {e}");
	};

	test(r#"foo.bar.baz"#, &["foo.bar.baz"]);
	test(r#"foo["bar"]"#, &["foo"]);
	test(r#"foo.baz["bar"]"#, &["foo.baz"]);
	// This is not quite right but maybe good enough.
	test(r#"foo.with(x, x.body)"#, &["foo", "x", "x.body"]);
	test(r#"foo.map(x, x.body)"#, &["foo", "x", "x.body"]);
	test(r#"foo.bar.map(x, x.body)"#, &["foo.bar", "x", "x.body"]);

	test(r#"fn(bar.baz)"#, &["bar.baz"]);
	test(r#"{"key":val, "listkey":[a.b]}"#, &["val", "a.b"]);
	test(r#"{"key":val, "listkey":[a.b]}"#, &["val", "a.b"]);
	test(r#"a? b: c"#, &["a", "b", "c"]);
	test(r#"a || b"#, &["a", "b"]);
	test(r#"!a.b"#, &["a.b"]);
	test(r#"a.b < c"#, &["a.b", "c"]);
	test(r#"a.b + c + 2"#, &["a.b", "c"]);
	// This is not right! Should just be 'a' probably
	test(r#"a["b"].c"#, &["a.c"]);
	test(r#"a.b[0]"#, &["a.b"]);
	test(r#"{"a":"b"}.a"#, &[]);
	// Test extauthz namespace recognition
	test(r#"extauthz.user_id"#, &["extauthz.user_id"]);
	test(r#"extauthz.role == "admin""#, &["extauthz.role"]);
}

#[test]
fn map() {
	let expr = r#"request.headers.map(v, v)"#;
	let v = eval(expr).unwrap();
	let v = v.as_array().unwrap();
	assert!(v.contains(&json!("user-agent")), "{v:?}");
}

#[test]
fn map_filter_dynamic_bool() {
	let expr = r#"[1, 2].map(x, llm.streaming, x + 1)"#;
	assert_eq!(json!([]), eval(expr).unwrap());
}

#[test]
fn dynamic_bool_in_logical_ops() {
	assert_eq!(json!(false), eval(r#"false || llm.streaming"#).unwrap());
	assert_eq!(json!(false), eval(r#"true && llm.streaming"#).unwrap());
}

#[test]
fn dynamic_index_key() {
	let expr = r#"{"bar": 1}[request.headers["foo"]]"#;
	assert_eq!(json!(1), eval(expr).unwrap());
}

#[test]
fn has_on_dynamic_map() {
	assert_eq!(json!(true), eval(r#"has(request.headers.foo)"#).unwrap());
}
