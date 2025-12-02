use std::collections::HashSet;
use std::fs::File;
use std::io::Write;

use divan::Bencher;
use http::Method;
use http_body_util::BodyExt;

use crate::cel::{ContextBuilder, Expression};
use crate::function;
use crate::http::{Body, Request};

// Test case structure with name for benchmark identification
struct TestCase {
	name: &'static str,
	expression: &'static str,
	request_builder: fn() -> crate::http::Request,
	expected: serde_json::Value,
}

// keep in sync with test_cases
const TEST_CASE_NAMES: &[&str] = &["simple_access", "header", "bbr", "cidr", "regex"];

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
			expected: serde_json::json!("gpt-4-turbo-preview"),
		},
		TestCase {
			name: "cidr",
			expression: r#"cidr("127.0.0.1/8").containsIP(request.headers['x-forwarded-for'])"#,
			request_builder: || {
				::http::Request::builder()
					.method(Method::POST)
					.uri("http://example.com")
					.header("content-type", "application/json")
					.header("x-forwarded-for", "127.0.0.1")
					.body(Body::empty())
					.unwrap()
			},
			expected: serde_json::json!(true),
		},
		TestCase {
			name: "regex",
			expression: r#"request.path.matches('^/user/([^/]+)/view$')"#,
			request_builder: || {
				::http::Request::builder()
					.method(Method::POST)
					.uri("http://example.com/user/1234/view")
					.body(Body::empty())
					.unwrap()
			},
			expected: serde_json::json!(true),
		},
	]
}

// Helper to lookup a test case by name
fn get_test_case(name: &str) -> TestCase {
	test_cases()
		.into_iter()
		.find(|tc| tc.name == name)
		.unwrap_or_else(|| panic!("Test case '{}' not found", name))
}

// validates the full compile -> build -> eval flow
#[test]
fn test_benchmark_cases() {
	let tc: HashSet<&str> = test_cases().into_iter().map(|t| t.name).collect();
	let tn = HashSet::from_iter(TEST_CASE_NAMES.iter().cloned());
	assert_eq!(tc, tn, "missing test cases");
	for tc in test_cases() {
		// Phase 1: Compile - parse the expression
		let expr = Expression::new_strict(tc.expression)
			.unwrap_or_else(|e| panic!("Failed to compile expression '{}': {}", tc.expression, e));

		// Phase 2: Build - set up context with request
		let req = (tc.request_builder)();
		let cb = setup_context(&expr, req);
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

// Benchmark: Compile phase - Expression::new() for each test case
#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_compile(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	b.bench(|| {
		let _ = divan::black_box(Expression::new_strict(tc.expression).unwrap());
	});
}

// Benchmark: Build phase - ContextBuilder::build() for each test case
#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_build(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	// Pre-compile expression
	let expr = Expression::new_strict(tc.expression).unwrap();
	let req = (tc.request_builder)();
	let cb = setup_context(&expr, req);

	b.bench_local(|| {
		let _ = divan::black_box(cb.build().unwrap());
	});
}

fn setup_context(expr: &Expression, req: Request) -> ContextBuilder {
	let mut cb = ContextBuilder::new();
	cb.register_expression(expr);
	if cb.with_request(&req, "".to_string()) {
		let rt = &tokio::runtime::Runtime::new().unwrap();
		let b = rt.block_on(async move { req.into_body().collect().await.unwrap().to_bytes() });
		cb.with_request_body(b);
	}
	cb
}

// Benchmark: Execute phase - exec.eval() for each test case
#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_execute(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	// Pre-compile and build context
	let expr = Expression::new_strict(tc.expression).unwrap();
	let req = (tc.request_builder)();
	let cb = setup_context(&expr, req);

	let exec = cb.build().unwrap();

	b.bench(|| {
		let _ = divan::black_box(exec.eval(&expr).unwrap());
	});
}

// lookup compares different ways to do field access in CEL
mod lookup {
	use std::collections::HashMap;
	use std::sync::Arc;

	use bytes::Bytes;
	use divan::Bencher;
	use http::Method;

	use crate::cel::{ContextBuilder, Expression};
	use crate::http::Body;

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
	fn bench_match(b: Bencher) {
		let k1 = divan::black_box("request");
		let k2 = divan::black_box("method");
		b.bench(|| {
			divan::black_box(match k1 {
				"request" => match k2 {
					"method" => Some(Method::GET),
					_ => None,
				},
				"a" => None,
				"b" => None,
				"c" => None,
				"z" => None,
				_ => None,
			})
		});
	}

	#[divan::bench]
	fn bench_native_map(b: Bencher) {
		let map = HashMap::from([(
			"request".to_string(),
			HashMap::from([("method".to_string(), "GET".to_string())]),
		)]);

		super::with_profiling("native", || {
			b.bench(|| {
				divan::black_box(map.get("request").unwrap().get("method").unwrap());
			});
		})
	}

	#[divan::bench]
	fn bench_lookup(b: Bencher) {
		let expr = Arc::new(Expression::new_strict(r#"request.method"#).unwrap());
		let req = ::http::Request::builder()
			.method(Method::GET)
			.header("x-example", "value")
			.body(Body::empty())
			.unwrap();
		let mut cb = ContextBuilder::new();
		cb.register_expression(&expr);
		cb.with_request(&req, "".to_string());
		let exec = cb.build().unwrap();

		super::with_profiling("lookup", || {
			b.bench(|| {
				exec.eval(&expr).unwrap();
			});
		})
	}
}

#[cfg(not(target_family = "unix"))]
pub fn with_profiling(name: &str, f: impl FnOnce()) {
	f();
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
pub fn with_profiling(name: &str, f: impl FnOnce()) {
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
