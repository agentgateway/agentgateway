use std::collections::HashMap;
use std::fs::File;
use std::io::Write;

use divan::Bencher;
use http::Method;

use super::*;
use crate::http::Body;

fn eval_request(expr: &str, req: crate::http::Request) -> Result<Value, Error> {
	let mut cb = ContextBuilder::new();
	let exp = Expression::new(expr)?;
	cb.register_expression(&exp);
	cb.with_request(&req);
	let exec = cb.build()?;
	exec.eval(&exp)
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
	cb.with_request(&req);
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
	cb.with_request(&req);
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
}
