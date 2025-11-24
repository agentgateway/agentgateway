use cel::{Context, Program, Value};
use serde_json::json;

use crate::insert_all;

fn eval(expr: &str) -> anyhow::Result<Value> {
	let prog = Program::compile(expr)?;
	let mut c = Context::default();
	insert_all(&mut c);
	Ok(prog.execute(&c)?)
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

#[test]
fn ip() {
	let expr = r#"ip('192.168.0.1')"#;
	assert(json!("192.168.0.1"), expr);
	let expr = r#"ip('192.168.0.1.0')"#;
	assert_fails(expr);

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

fn assert(want: serde_json::Value, expr: &str) {
	assert_eq!(
		want,
		eval(expr).unwrap().json().unwrap(),
		"expression: {expr}"
	);
}

fn assert_fails(expr: &str) {
	assert!(eval(expr).is_err(), "expression: {expr}");
}
