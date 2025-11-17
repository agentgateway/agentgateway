use std::collections::HashMap;
use std::fs::File;
use std::io::Write;

use super::*;
use crate::http::Body;
use divan::Bencher;
use http::Method;
use http_body_util::BodyExt;

fn eval_request(expr: &str, req: crate::http::Request) -> Result<Value, Error> {
	let mut cb = ContextBuilder::new();
	let exp = Expression::new(expr)?;
	cb.register_expression(&exp);
	cb.with_request(&req, "".to_string());
	let exec = cb.build()?;
	exec.eval(&exp)
}

// Test case structure with name for benchmark identification
struct TestCase {
	name: &'static str,
	expression: &'static str,
	request_builder: fn() -> crate::http::Request,
	expected: serde_json::Value,
}

// keep in sync with test_cases
const TEST_CASE_NAMES: &[&str] = &["case=simple_access", "case=header", "case=bbr"];

// Comprehensive test cases to be used across multiple tests
fn test_cases() -> Vec<TestCase> {
	vec![
		TestCase {
			name: "simple_access",
			expression: r#"request.method"#,
			request_builder: || {
				::http::Request::builder()
					.method(Method::GET)
					.uri("http://example.com")
					.body(Body::empty())
					.unwrap()
			},
			expected: serde_json::json!("GET"),
		},
		TestCase {
			name: "header",
			expression: r#"request.headers["x-custom"]"#,
			request_builder: || {
				::http::Request::builder()
					.method(Method::GET)
					.uri("http://example.com")
					.header("x-custom", "test-value")
					.body(Body::empty())
					.unwrap()
			},
			expected: serde_json::json!("test-value"),
		},
		TestCase {
			name: "bbr",
			expression: r#"json(request.body).model"#,
			request_builder: || {
				::http::Request::builder()
					.method(Method::POST)
					.uri("http://example.com")
					.header("content-type", "application/json")
					.body(Body::from(
						include_bytes!("../llm/tests/request_full.json").to_vec(),
					))
					.unwrap()
			},
			expected: serde_json::json!(true),
		},
	]
}

// Helper to lookup a test case by name
fn get_test_case(name: &str) -> TestCase {
	let name = name.strip_prefix("case=").unwrap();
	test_cases()
		.into_iter()
		.find(|tc| tc.name == name)
		.unwrap_or_else(|| panic!("Test case '{}' not found", name))
}

// Comprehensive test that validates the full compile -> build -> eval flow
#[test]
fn test_comprehensive_cel_flow() {
	let tc: HashSet<&str> = test_cases().into_iter().map(|t| t.name).collect();
	let tn = HashSet::from_iter(TEST_CASE_NAMES.iter().cloned());
	assert_eq!(tc, tn, "missing test cases");
	for tc in test_cases() {
		// Phase 1: Compile - parse the expression
		let expr = Expression::new(tc.expression)
			.unwrap_or_else(|e| panic!("Failed to compile expression '{}': {}", tc.expression, e));

		// Phase 2: Build - set up context with request
		let req = (tc.request_builder)();
		let mut cb = ContextBuilder::new();
		cb.register_expression(&expr);
		cb.with_request(&req, "".to_string());
		let exec = cb
			.build()
			.unwrap_or_else(|e| panic!("Failed to build context for '{}': {}", tc.expression, e));

		// Phase 3: Execute - evaluate the expression
		let result = exec
			.eval(&expr)
			.unwrap_or_else(|e| panic!("Failed to eval expression '{}': {}", tc.expression, e));

		// Assert result matches expected
		let result_json = result.json().unwrap_or_else(|e| {
			panic!(
				"Failed to convert result to JSON for '{}': {}",
				tc.expression, e
			)
		});
		assert_eq!(
			tc.expected, result_json,
			"Expression '{}' produced unexpected result",
			tc.expression
		);
	}
}

#[test]
fn test_eval() {
	let expr = Arc::new(Expression::new(r#"request.method"#).unwrap());
	let req = ::http::Request::builder()
		.method(Method::GET)
		.header("x-example", "value")
		.body(Body::empty())
		.unwrap();
	let mut cb = ContextBuilder::new();
	cb.register_expression(&expr);
	cb.with_request(&req, "".to_string());
	let exec = cb.build().unwrap();

	exec.eval(&expr).unwrap();
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

#[divan::bench]
fn bench_native(b: Bencher) {
	let req = ::http::Request::builder()
		.method(Method::GET)
		.header("x-example", "value")
		.body(http_body_util::Empty::<Bytes>::new())
		.unwrap();
	b.bench(|| {
		divan::black_box(req.method());
	});
}

#[divan::bench]
#[cfg(target_family = "unix")]
fn bench_native_map(b: Bencher) {
	let map = HashMap::from([(
		"request".to_string(),
		HashMap::from([("method".to_string(), "GET".to_string())]),
	)]);

	with_profiling("native", || {
		b.bench(|| {
			divan::black_box(map.get("request").unwrap().get("method").unwrap());
		});
	})
}

#[macro_export]
macro_rules! function {
	() => {{
		fn f() {}
		fn type_name_of<T>(_: T) -> &'static str {
			std::any::type_name::<T>()
		}
		let name = type_name_of(f);
		let name = &name[..name.len() - 3].to_string();
		name.strip_suffix("::with_profiling").unwrap().to_string()
	}};
}

#[cfg(target_family = "unix")]
fn with_profiling(name: &str, f: impl FnOnce()) {
	use pprof::protos::Message;
	let guard = pprof::ProfilerGuardBuilder::default()
		.frequency(1000)
		// .blocklist(&["libc", "libgcc", "pthread", "vdso"])
		.build()
		.unwrap();

	f();

	let report = guard.report().build().unwrap();
	let profile = report.pprof().unwrap();

	let body = profile.write_to_bytes().unwrap();
	File::create(format!("/tmp/pprof-{}::{name}", function!()))
		.unwrap()
		.write_all(&body)
		.unwrap()
}

#[divan::bench]
#[cfg(target_family = "unix")]
fn bench_lookup(b: Bencher) {
	let expr = Arc::new(Expression::new(r#"request.method"#).unwrap());
	let req = ::http::Request::builder()
		.method(Method::GET)
		.header("x-example", "value")
		.body(Body::empty())
		.unwrap();
	let mut cb = ContextBuilder::new();
	cb.register_expression(&expr);
	cb.with_request(&req, "".to_string());
	let exec = cb.build().unwrap();

	with_profiling("lookup", || {
		b.bench(|| {
			exec.eval(&expr).unwrap();
		});
	})
}

#[divan::bench]
fn bench_with_response(b: Bencher) {
	let expr = Arc::new(
		Expression::new(r#"response.status == 200 && response.headers["x-example"] == "value""#)
			.unwrap(),
	);
	b.with_inputs(|| {
		::http::Response::builder()
			.status(200)
			.header("x-example", "value")
			.body(Body::empty())
			.unwrap()
	})
	.bench_refs(|r| {
		let mut cb = ContextBuilder::new();
		cb.register_expression(&expr);
		cb.with_response(r);
		let exec = cb.build()?;
		exec.eval(&expr)
	});
}

#[divan::bench]
#[cfg(target_family = "unix")]
fn benchmark_register_build(b: Bencher) {
	let expr = Arc::new(Expression::new(r#"1 + 2 == 3"#).unwrap());
	with_profiling("full", || {
		b.with_inputs(|| {
			::http::Response::builder()
				.status(200)
				.header("x-example", "value")
				.body(Body::empty())
				.unwrap()
		})
		.bench_refs(|r| {
			let mut cb = ContextBuilder::new();
			cb.register_expression(&expr);
			cb.with_response(r);
			let exec = cb.build()?;
			exec.eval(&expr)
		});
	});
}

#[test]
fn test_properties() {
	let test = |e: &str, want: &[&str]| {
		let p = Program::compile(e).unwrap();
		let mut props = Vec::with_capacity(5);
		properties(&p.expression().expr, &mut props, &mut Vec::default());
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

// ============================================================================
// Comprehensive Benchmarks
// ============================================================================

// Benchmark: Compile phase - Expression::new() for each test case
#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_compile(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	b.bench(|| {
		let _ = divan::black_box(Expression::new(tc.expression).unwrap());
	});
}

// Benchmark: Build phase - ContextBuilder::build() for each test case
#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_build(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	// Pre-compile expression
	let expr = Expression::new(tc.expression).unwrap();
	let req = (tc.request_builder)();
	let mut cb = ContextBuilder::new();
	cb.register_expression(&expr);
	if cb.with_request(&req, "".to_string()) {
		let rt = &tokio::runtime::Runtime::new().unwrap();
		let b = rt.block_on(async move { req.into_body().collect().await.unwrap().to_bytes() });
		cb.with_request_body(b);
	}

	b.bench_local(|| {
		let _ = divan::black_box(cb.build().unwrap());
	});
}

// Benchmark: Execute phase - exec.eval() for each test case
#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_execute(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	// Pre-compile and build context
	let expr = Expression::new(tc.expression).unwrap();
	let req = (tc.request_builder)();
	let mut cb = ContextBuilder::new();
	cb.register_expression(&expr);
	if cb.with_request(&req, "".to_string()) {
		let rt = &tokio::runtime::Runtime::new().unwrap();
		let b = rt.block_on(async move { req.into_body().collect().await.unwrap().to_bytes() });
		cb.with_request_body(b);
	}

	let exec = cb.build().unwrap();

	b.bench(|| {
		let _ = divan::black_box(exec.eval(&expr).unwrap());
	});
}
