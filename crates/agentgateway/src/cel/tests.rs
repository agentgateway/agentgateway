use std::collections::HashSet;

use http::Method;

use super::*;
use crate::http::Body;

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
