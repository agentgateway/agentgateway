use assert_matches::assert_matches;
use cel::types::dynamic::DynamicValue;
use cel::{Context, Program, Value, context};
use serde_json::json;

use crate::insert_all;

fn eval(expr: &str) -> anyhow::Result<Value<'static>> {
	eval_with_optimizations_check(expr, true)
}
fn eval_with_optimizations_check(expr: &str, check: bool) -> anyhow::Result<Value<'static>> {
	let prog = Program::compile(expr)?;
	let optimized = Program::compile_with_optimizer(expr, crate::DefaultOptimizer)?;
	let mut c = Context::default();
	insert_all(&mut c);

	let vars = Vars {
		foo: "hello",
		bar: "world",
	};
	let resolver = context::SingleVarResolver::new(
		&context::DefaultVariableResolver,
		"vars",
		Value::Dynamic(DynamicValue::new(&vars)),
	);

	let a = Value::resolve(prog.expression(), &c, &resolver)?.as_static();
	let b = Value::resolve(optimized.expression(), &c, &resolver)?.as_static();
	if check {
		assert_eq!(a, b, "optimizations changed behavior ({expr})");
	}

	Ok(a)
}
fn eval_non_static(expr: &str, f: impl FnOnce(Value<'_>)) -> anyhow::Result<()> {
	let expr = expr.to_string();
	let prog = Program::compile_with_optimizer(&expr, crate::DefaultOptimizer)?;
	let mut c = Context::default();
	insert_all(&mut c);

	let vars = Vars {
		foo: "hello",
		bar: "world",
	};
	let resolver = context::SingleVarResolver::new(
		&context::DefaultVariableResolver,
		"vars",
		Value::Dynamic(DynamicValue::new(&vars)),
	);

	let a = Value::resolve(prog.expression(), &c, &resolver)?;
	f(a);
	Ok(())
}

#[test]
fn with() {
	let expr = r#"[1,2].with(a, a + a)"#;
	assert(json!([1, 2, 1, 2]), expr);

	// with() should not materialize
	eval_non_static("vars.with(v, v)", |r| {
		assert_matches!(r, Value::Dynamic(_));
	})
	.unwrap();
}

#[test]
fn json() {
	let expr = r#"json('{"hi":1}').hi"#;
	assert(json!(1), expr);
	let expr = r#"json('{"hi":1}').unknown"#;
	assert_fails(expr);
}

#[test]
fn json_field() {
	let expr = r#"jsonField('{"hi":1}', "hi")"#;
	assert(json!(1), expr);
	let expr = r#"jsonField('{"hi":1}', "unknown")"#;
	assert_fails(expr);
}

#[test]
fn random() {
	let expr = r#"int(random() * 10.0)"#;
	let v = eval_with_optimizations_check(expr, false)
		.unwrap()
		.json()
		.unwrap()
		.as_i64()
		.unwrap();
	assert!((0..=10).contains(&v));
}

#[test]
fn base64() {
	let expr = r#"base64.encode('hello')"#;
	assert(json!("aGVsbG8="), expr);
	let expr = r#"string(base64.decode(base64.encode("hello")))"#;
	assert(json!("hello"), expr);
}

#[test]
fn map_values() {
	let expr = r#"{"a": 1, "b": 2}.mapValues(v, v * 2)"#;
	assert(json!({"a": 2, "b": 4}), expr);
	let expr = r#"vars.mapValues(v, v +  ' hi')"#;
	assert(json!({"bar": "world hi", "foo": "hello hi"}), expr);
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

	// default() should not materialize
	eval_non_static("default(a.b, vars)", |r| {
		assert_matches!(r, Value::Dynamic(_));
	})
	.unwrap();
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

#[test]
fn ip() {
	let expr = r#"ip('192.168.0.1')"#;
	assert(json!("192.168.0.1"), expr);
	let expr = r#"ip('192.168.0.1.0')"#;
	assert_fails(expr);

	let expr = r#"ip("192.168.0.1").family()"#;
	assert(json!(4), expr);

	let expr = r#"isIP('192.168.0.1')"#;
	assert(json!(true), expr);
	let expr = r#"isIP('192.168.0.1.0')"#;
	assert(json!(false), expr);

	// let expr = r#"ip.isCanonical("127.0.0.1")"#;
	// assert(json!(true), expr);
	//
	// let expr = r#"ip.isCanonical("127.0.0.1.0")"#;
	// assert_fails(expr);

	let expr = r#"ip("192.168.0.1").family()"#;
	assert(json!(4), expr);

	let expr = r#"ip("0.0.0.0").isUnspecified()"#;
	assert(json!(true), expr);
	let expr = r#"ip("127.0.0.1").isUnspecified()"#;
	assert(json!(false), expr);

	let expr = r#"ip("127.0.0.1").isLoopback()"#;
	assert(json!(true), expr);
	let expr = r#"ip("1.2.3.4").isLoopback()"#;
	assert(json!(false), expr);

	let expr = r#"ip("224.0.0.1").isLinkLocalMulticast()"#;
	assert(json!(true), expr);
	let expr = r#"ip("224.0.1.1").isLinkLocalMulticast()"#;
	assert(json!(false), expr);

	let expr = r#"ip("169.254.169.254").isLinkLocalUnicast()"#;
	assert(json!(true), expr);

	let expr = r#"ip("192.168.0.1").isLinkLocalUnicast()"#;
	assert(json!(false), expr);

	let expr = r#"ip("192.168.0.1").isGlobalUnicast()"#;
	assert(json!(true), expr);

	let expr = r#"ip("255.255.255.255").isGlobalUnicast()"#;
	assert(json!(false), expr);

	// IPv6 tests
	let expr = r#"ip("2001:db8::68")"#;
	assert(json!("2001:db8::68"), expr);

	let expr = r#"ip("2001:db8:::68")"#;
	assert_fails(expr);

	let expr = r#"isIP("2001:db8::68")"#;
	assert(json!(true), expr);

	let expr = r#"isIP("2001:db8:::68")"#;
	assert(json!(false), expr);

	// let expr = r#"ip.isCanonical("2001:db8::68")"#;
	// assert(json!(true), expr);
	//
	// let expr = r#"ip.isCanonical("2001:DB8::68")"#;
	// assert(json!(false), expr);
	//
	// let expr = r#"ip.isCanonical("2001:db8:::68")"#;
	// assert_fails(expr);

	let expr = r#"ip("2001:db8::68").family()"#;
	assert(json!(6), expr);

	let expr = r#"ip("::").isUnspecified()"#;
	assert(json!(true), expr);

	let expr = r#"ip("::1").isUnspecified()"#;
	assert(json!(false), expr);

	let expr = r#"ip("::1").isLoopback()"#;
	assert(json!(true), expr);

	let expr = r#"ip("2001:db8::abcd").isLoopback()"#;
	assert(json!(false), expr);

	let expr = r#"ip("ff02::1").isLinkLocalMulticast()"#;
	assert(json!(true), expr);

	let expr = r#"ip("fd00::1").isLinkLocalMulticast()"#;
	assert(json!(false), expr);

	let expr = r#"ip("fe80::1").isLinkLocalUnicast()"#;
	assert(json!(true), expr);

	let expr = r#"ip("fd80::1").isLinkLocalUnicast()"#;
	assert(json!(false), expr);

	let expr = r#"ip("2001:db8::abcd").isGlobalUnicast()"#;
	assert(json!(true), expr);

	let expr = r#"ip("ff00::1").isGlobalUnicast()"#;
	assert(json!(false), expr);

	// Type conversion test. TODO
	// let expr = r#"string(ip("192.168.0.1"))"#;
	// assert(json!("192.168.0.1"), expr);

	let expr = r#"isIP(cidr("192.168.0.0/24"))"#;
	assert_fails(expr);
}

#[test]
fn cidr() {
	let expr = r#"cidr('127.0.0.1/8')"#;
	assert(json!("127.0.0.1/8"), expr);

	let expr = r#"cidr('127.0.0.1/8').containsIP(ip('127.0.0.1'))"#;
	assert(json!(true), expr);
	let expr = r#"cidr('127.0.0.1/8').containsIP(ip('128.0.0.1'))"#;
	assert(json!(false), expr);

	let expr = r#"cidr('127.0.0.1/8').containsCIDR(cidr('128.0.0.1/32'))"#;
	assert(json!(false), expr);
	let expr = r#"cidr('127.0.0.1/8').containsCIDR(cidr('127.0.0.1/27'))"#;
	assert(json!(true), expr);
	let expr = r#"cidr('127.0.0.1/8').containsCIDR(cidr('127.0.0.1/32'))"#;
	assert(json!(true), expr);

	let expr = r#"cidr('127.0.0.0/8').masked()"#;
	assert(json!("127.0.0.0/8"), expr);
	let expr = r#"cidr('127.0.7.1/8').masked()"#;
	assert(json!("127.0.0.0/8"), expr);

	let expr = r#"cidr('127.0.7.1/8').prefixLength()"#;
	assert(json!(8), expr);
	let expr = r#"cidr('::1/128').prefixLength()"#;
	assert(json!(128), expr);

	let expr = r#"cidr('127.0.0.1/8').containsIP('127.0.0.1')"#;
	assert(json!(true), expr);
}

#[test]
fn uuid() {
	// Test that uuid() returns a string
	let expr = r#"uuid()"#;
	let result = eval_with_optimizations_check(expr, false)
		.unwrap()
		.json()
		.unwrap();
	assert!(result.is_string(), "uuid() should return a string");
	// Test that it's formatted like a UUID (8-4-4-4-12 hex digits)
	let uuid_str = result.as_str().unwrap();
	assert_eq!(uuid_str.len(), 36, "UUID should be 36 characters long");
	assert_eq!(uuid_str.chars().nth(8).unwrap(), '-');
	assert_eq!(uuid_str.chars().nth(13).unwrap(), '-');
	assert_eq!(uuid_str.chars().nth(18).unwrap(), '-');
	assert_eq!(uuid_str.chars().nth(23).unwrap(), '-');
	// Test that it conforms to UUID version 4 format specifications
	// The version field (at index 14, the 15th character) should be '4'
	assert_eq!(
		uuid_str.chars().nth(14).unwrap(),
		'4',
		"UUID version field should be '4'"
	);
	// The variant field (at index 19, i.e., the 20th character) should be one of '8', '9', 'a', or 'b'
	let variant_char = uuid_str.chars().nth(19).unwrap();
	assert!(
		['8', '9', 'a', 'b'].contains(&variant_char),
		"UUID variant field should be '8', '9', 'a', or 'b', got '{}'",
		variant_char
	);
	// Test that multiple calls return different UUIDs
	let result2 = eval_with_optimizations_check(expr, false)
		.unwrap()
		.json()
		.unwrap();
	assert_ne!(
		result, result2,
		"Multiple uuid() calls should return different values"
	);
}
fn assert(want: serde_json::Value, expr: &str) {
	assert_eq!(
		want,
		eval(expr)
			.unwrap_or_else(|_| panic!("{expr}"))
			.json()
			.unwrap(),
		"expression: {expr}"
	);
}

fn assert_fails(expr: &str) {
	assert!(eval(expr).is_err(), "expression: {expr}");
}

#[derive(Debug, Clone, cel::DynamicType)]
struct Vars<'a> {
	foo: &'a str,
	bar: &'a str,
}
