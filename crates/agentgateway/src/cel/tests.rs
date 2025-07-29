use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use agent_core::strng;
use divan::Bencher;
use http::Method;
use serde_json::json;

use super::*;
use crate::http::Body;
use crate::store::Stores;
use crate::types::agent::{Listener, ListenerProtocol, PathMatch, Route, RouteMatch, RouteSet};

fn eval_request(expr: &str, req: crate::http::Request) -> Result<Value, Error> {
	let mut cb = ContextBuilder::new();
	let exp = Expression::new(expr)?;
	cb.register_expression(&exp);
	cb.with_request(&req);
	let exec = cb.build()?;
	exec.eval(&exp)
}

fn eval(expr: &str) -> Result<Value, Error> {
	let mut cb = ContextBuilder::new();
	let exp = Expression::new(expr)?;
	cb.register_expression(&exp);
	let exec = cb.build()?;
	exec.eval(&exp)
}

#[test]
fn test_eval() {
	let expr = Arc::new(Expression::new(r#"request.method"#).unwrap());
	let ctx = root_context();
	let req = ::http::Request::builder()
		.method(Method::GET)
		.header("x-example", "value")
		.body(Body::empty())
		.unwrap();
	let mut cb = ContextBuilder::new();
	cb.register_expression(&expr);
	cb.with_request(&req);
	let exec = cb.build().unwrap();

	exec.eval(&expr);
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
fn fn_with() {
	let expr = r#"[1,2].with(a, a + a)"#;
	assert_eq!(json!([1, 2, 1, 2]), eval(expr).unwrap().json().unwrap());
}

#[test]
fn fn_json() {
	let expr = r#"json('{"hi":1}').hi"#;
	assert_eq!(json!(1), eval(expr).unwrap().json().unwrap());
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
fn bench_native_map(b: Bencher) {
	let req = ::http::Request::builder()
		.method(Method::GET)
		.header("x-example", "value")
		.body(http_body_util::Empty::<Bytes>::new())
		.unwrap();
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

	let mut body = profile.write_to_bytes().unwrap();
	File::create(format!("/tmp/pprof-{}::{name}", function!()))
		.unwrap()
		.write_all(&body)
		.unwrap()
}

#[divan::bench]
fn bench_lookup(b: Bencher) {
	let expr = Arc::new(Expression::new(r#"request.method"#).unwrap());
	let ctx = root_context();
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
			exec.eval(&expr);
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
fn bench(b: Bencher) {
	let expr = Arc::new(Expression::new(r#"1 + 2 == 3"#).unwrap());
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
