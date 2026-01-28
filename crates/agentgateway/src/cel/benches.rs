use std::collections::HashSet;

use divan::Bencher;
use http::Method;
use http_body_util::BodyExt;
use serde_json::json;

use crate::cel::{BufferedBody, Expression};
use crate::http::{Body, Request, jwt};

// Test case structure with name for benchmark identification
struct TestCase {
	name: &'static str,
	expression: &'static str,
	request_builder: fn() -> crate::http::Request,
	expected: serde_json::Value,
}

// keep in sync with test_cases
const TEST_CASE_NAMES: &[&str] = &["simple_access", "header", "bbr", "jwt", "cidr", "regex"];

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
			expected: json!("GET"),
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
			expected: json!("test-value"),
		},
		TestCase {
			name: "bbr",
			// expression: r#"jsonField(request.body, "model")"#,
			expression: r#"json(request.body).model"#,
			request_builder: || {
				with_body(
					::http::Request::builder()
						.method(Method::POST)
						.uri("http://example.com")
						.header("content-type", "application/json")
						.body(Body::from(
							include_bytes!("../llm/tests/request_full.json").to_vec(),
						))
						.unwrap(),
				)
			},
			expected: json!("gpt-4-turbo-preview"),
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
			expected: json!(true),
		},
		TestCase {
			name: "jwt",
			expression: r#"jwt.sub"#,
			request_builder: || {
				let mut req = ::http::Request::builder()
					.method(Method::GET)
					.uri("http://example.com")
					.body(Body::empty())
					.unwrap();
				let serde_json::Value::Object(claims) = json!({
					"aud": "test.agentgateway.dev",
					"exp": 1900650294,
					"field1": "value1",
					"iat": 1751558060,
					"iss": "agentgateway.dev",
					"jti": "e651668259e05a973a0b408395daec420852ac92f98717e3759429db1cac8762",
					"list": [
						"apple",
						"banana"
					],
					"nbf": 1751558060,
					"nested": {
						"key": "value"
					},
					"sub": "test-user"
				}) else {
					unreachable!()
				};
				req.extensions_mut().insert(jwt::Claims {
					inner: claims,
					jwt: Default::default(),
				});
				req
			},
			expected: json!("test-user"),
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
			expected: json!(true),
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
fn test_benchmark_cases_ref() {
	let tc: HashSet<&str> = test_cases().into_iter().map(|t| t.name).collect();
	let tn = HashSet::from_iter(TEST_CASE_NAMES.iter().cloned());
	assert_eq!(tc, tn, "missing test cases");
	for tc in test_cases() {
		let expr = Expression::new_strict(tc.expression)
			.unwrap_or_else(|e| panic!("Failed to compile expression '{}': {}", tc.expression, e));

		let req = (tc.request_builder)();
		let exec = crate::cel::Executor::new_request(&req);
		let result = exec
			.eval(&expr)
			.unwrap_or_else(|e| panic!("Failed to eval expression '{}': {}", tc.expression, e));

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
fn test_benchmark_cases_snapshot() {
	let tc: HashSet<&str> = test_cases().into_iter().map(|t| t.name).collect();
	let tn = HashSet::from_iter(TEST_CASE_NAMES.iter().cloned());
	assert_eq!(tc, tn, "missing test cases");
	for tc in test_cases() {
		let expr = Expression::new_strict(tc.expression)
			.unwrap_or_else(|e| panic!("Failed to compile expression '{}': {}", tc.expression, e));
		let mut req = (tc.request_builder)();
		let ss = crate::cel::snapshot_request(&mut req);
		let exec = crate::cel::Executor::new_logger(Some(&ss), None, None, None, None);
		let result = exec
			.eval(&expr)
			.unwrap_or_else(|e| panic!("Failed to eval expression '{}': {}", tc.expression, e));

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

fn with_body(req: crate::http::Request) -> crate::http::Request {
	let rt = &tokio::runtime::Runtime::new().unwrap();
	let (mut head, body) = req.into_parts();
	let b = rt.block_on(async move { body.collect().await.unwrap().to_bytes() });
	head.extensions.insert(BufferedBody(b));
	Request::from_parts(head, Body::empty())
}

#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_execute_snapshot(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	// Pre-compile and build context
	let expr = Expression::new_strict(tc.expression).unwrap();
	let mut req = (tc.request_builder)();
	let ss = crate::cel::snapshot_request(&mut req);
	let exec = crate::cel::Executor::new_logger(Some(&ss), None, None, None, None);

	b.bench(|| {
		let _ = divan::black_box(exec.eval(&expr).unwrap());
	});
}

#[divan::bench(args = TEST_CASE_NAMES)]
fn bench_execute_ref(b: Bencher, case_name: &str) {
	let tc = get_test_case(case_name);
	// Pre-compile and build context
	let expr = Expression::new_strict(tc.expression).unwrap();
	let req = (tc.request_builder)();
	let exec = crate::cel::Executor::new_request(&req);

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

	use crate::cel::Expression;
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

		b.bench(|| {
			divan::black_box(map.get("request").unwrap().get("method").unwrap());
		});
	}

	#[divan::bench]
	fn bench_lookup(b: Bencher) {
		let expr = Arc::new(Expression::new_strict(r#"request.method"#).unwrap());
		let req = ::http::Request::builder()
			.method(Method::GET)
			.header("x-example", "value")
			.body(Body::empty())
			.unwrap();
		let exec = crate::cel::Executor::new_request(&req);

		b.bench(|| {
			exec.eval(&expr).unwrap();
		});
	}
}
