use std::collections::HashSet;

use http::{HeaderValue, Method};
use serde_json::json;

use super::*;
use crate::http::Body;

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
fn test_permissive() {
	let exec_serde = full_example_executor();
	let exec = exec_serde.as_executor();
	let assert_compile_failure = |expr: Expression| {
		assert!(
			exec
				.eval(&expr)
				.expect_err("must be an error")
				.to_string()
				.contains("could not be compiled")
		);
	};
	let (valid, err) = Expression::new_permissive("1 + 1");
	assert!(err.is_none());
	assert_eq!(2, exec.eval(&valid).unwrap().json().unwrap());

	assert_compile_failure(Expression::new_permissive("1 +").0);

	assert_compile_failure(Expression::new_permissive("'").0);

	assert_compile_failure(Expression::new_permissive("\"h").0);

	assert_compile_failure(Expression::new_permissive(r#"" || true || "#).0);
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
fn list_in() {
	let expr = "'san' in source.subjectAltNames";
	assert_eq!(json!(true), eval(expr).unwrap());
	let expr = "'not-san' in source.subjectAltNames";
	assert_eq!(json!(false), eval(expr).unwrap());
}

fn request_with_header_modes() -> crate::http::Request {
	let mut req = ::http::Request::builder()
		.method(Method::GET)
		.uri("http://example.com")
		.header("single", "z")
		.header("multi", "a,b")
		.body(Body::empty())
		.unwrap();
	req.headers_mut().append("multi", "c".parse().unwrap());
	let mut authorization = HeaderValue::from_static("Bearer token");
	authorization.set_sensitive(true);
	req.headers_mut().insert("authorization", authorization);
	req
}

mod headers {
	use cel::Value;

	use crate::cel::tests::{eval_request, request_with_header_modes};

	#[test]
	fn lookup_default() {
		assert_eq!(
			Value::Bool(true),
			eval_request(
				r#"request.headers.multi == ['a,b', 'c']"#,
				request_with_header_modes()
			)
			.unwrap()
		);
		assert_eq!(
			Value::Bool(true),
			eval_request(
				r#"request.headers.single == 'z'"#,
				request_with_header_modes()
			)
			.unwrap()
		);
	}

	#[test]
	fn redacted() {
		assert_eq!(
			"Bearer token",
			eval_request(
				r#"request.headers.authorization"#,
				request_with_header_modes()
			)
			.unwrap()
			.as_str()
			.unwrap()
			.as_ref()
		);
		assert_eq!(
			"<redacted>",
			eval_request(
				r#"request.headers.redacted().authorization"#,
				request_with_header_modes()
			)
			.unwrap()
			.as_str()
			.unwrap()
			.as_ref()
		);
	}

	#[test]
	fn join() {
		let req = request_with_header_modes();
		assert_eq!(
			Value::Bool(true),
			eval_request(r#"request.headers.join().multi == "a,b,c""#, req).unwrap()
		);
	}

	#[test]
	fn raw() {
		let req = request_with_header_modes();
		assert_eq!(
			Value::Bool(true),
			eval_request(r#"request.headers.raw().multi == ['a,b','c']"#, req,).unwrap()
		);
	}

	#[test]
	fn split() {
		let req = request_with_header_modes();
		assert_eq!(
			Value::Bool(true),
			eval_request(r#"request.headers.split().multi == ['a','b','c']"#, req,).unwrap()
		);
	}

	#[test]
	fn chained() {
		let req = request_with_header_modes();
		assert_eq!(
				Value::Bool(true),
				eval_request(
					r#"size(request.headers.redacted().raw()["authorization"]) == 1 && request.headers.redacted().raw()["authorization"][0] == "<redacted>""#,
					req,
				)
				.unwrap()
			);
	}

	#[test]
	fn last_mode_wins() {
		let req = request_with_header_modes();
		assert_eq!(
				Value::Bool(true),
				eval_request(
					r#"request.headers.raw().join().multi == "a,b,c" && request.headers.join().split().multi == ['a','b','c']"#,
					req,
				)
				.unwrap()
			);
	}

	#[test]
	fn cookie() {
		let req = || {
			::http::Request::builder()
				.method(http::Method::GET)
				.uri("http://example.com")
				.header("cookie", "session=abc; theme=light")
				.header("cookie", "session=def")
				.body(crate::http::Body::empty())
				.unwrap()
		};
		assert_eq!(
			"abc",
			eval_request(r#"request.headers.cookie("session")"#, req())
				.unwrap()
				.as_str()
				.unwrap()
				.as_ref()
		);
		assert_eq!(
			"light",
			eval_request(r#"request.headers.cookie("theme")"#, req())
				.unwrap()
				.as_str()
				.unwrap()
				.as_ref()
		);
	}

	#[test]
	fn cookie_missing() {
		let req = ::http::Request::builder()
			.method(http::Method::GET)
			.uri("http://example.com")
			.header("cookie", "session=abc")
			.body(crate::http::Body::empty())
			.unwrap();
		let err = eval_request(r#"request.headers.cookie("theme")"#, req).unwrap_err();
		assert!(err.to_string().contains("No such key: theme"));
	}
}

mod query_accessors {
	use cel::Value;
	use http::Method;

	use crate::cel::tests::eval_request;
	use crate::http::Body;

	fn request() -> crate::http::Request {
		::http::Request::builder()
			.method(Method::GET)
			.uri("http://example.com/api/test?foo=bar&foo=baz&zap=zip")
			.body(Body::empty())
			.unwrap()
	}

	#[test]
	fn path_stays_string_compatible() {
		assert_eq!(
			Value::Bool(true),
			eval_request(r#"request.path == "/api/test""#, request()).unwrap()
		);
	}

	#[test]
	fn query_reads_from_path_and_uri() {
		assert_eq!(
			Value::Bool(true),
			eval_request(
				r#"request.pathAndQuery.query("foo") == ["bar", "baz"] && request.uri.query("zap") == ["zip"]"#,
				request()
			)
			.unwrap()
		);
	}

	#[test]
	fn missing_query_is_no_such_key() {
		let err = eval_request(r#"request.pathAndQuery.query("missing")"#, request()).unwrap_err();
		assert!(err.to_string().contains("No such key: missing"), "{err}");
	}

	#[test]
	fn add_and_set_query_return_new_values() {
		assert_eq!(
			Value::Bool(true),
			eval_request(
				r#"request.pathAndQuery == "/api/test?foo=bar&foo=baz&zap=zip" &&
request.pathAndQuery.addQuery("foo", "qux") == "/api/test?foo=bar&foo=baz&zap=zip&foo=qux" &&
request.pathAndQuery.setQuery("foo", "qux") == "/api/test?zap=zip&foo=qux" &&
request.uri.setQuery("foo", "qux") == "http://example.com/api/test?zap=zip&foo=qux""#,
				request()
			)
			.unwrap()
		);
	}
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
	test(r#"a["b"].c"#, &["a"]);
	test(r#"a["b"]["c"]"#, &["a"]);
	test(r#"a.b[0]"#, &["a.b"]);
	test(r#"a.b[0].c"#, &["a.b"]);
	test(r#"a[b.c]"#, &["a", "b.c"]);
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
fn test_struct() {
	let expr = r#"foo{}"#;
	eval(expr).expect_err("expected an error");
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

#[test]
fn unset_values() {
	let req = || {
		::http::Request::builder()
			.method(Method::GET)
			.uri("http://example.com")
			.header("x-example", "value")
			.body(Body::empty())
			.unwrap()
	};
	assert_eq!(Value::Null, eval_request("jwt", req()).unwrap());
	assert_eq!(
		Value::Bool(true),
		eval_request("jwt == null", req()).unwrap()
	);
	// This is just invalid syntax
	assert!(eval_request("has(jwt)", req()).is_err());
	assert_eq!(
		Value::Bool(false),
		eval_request("has(jwt.sub)", req()).unwrap()
	);
}
