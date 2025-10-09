use crate::cel::ContextBuilder;
use agent_core::strng;
use itertools::Itertools;

use super::*;

fn build<const N: usize>(items: [(&str, &str); N]) -> Transformation {
	let c = super::LocalTransformationConfig {
		request: Some(super::LocalTransform {
			add: items
				.iter()
				.map(|(k, v)| (strng::new(k), strng::new(v)))
				.collect_vec(),
			..Default::default()
		}),
		response: None,
	};
	Transformation::try_from(c).unwrap()
}

#[test]
fn test_transformation() {
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/")
		.header("X-Custom-Foo", "Bar")
		.body(crate::http::Body::empty())
		.unwrap();
	let xfm = build([("x-insert", r#""hello " + request.headers["x-custom-foo"]"#)]);
	let mut ctx = ContextBuilder::new();
	for e in xfm.expressions() {
		ctx.register_expression(e)
	}
	ctx.with_request(&req, "".to_string());
	xfm.apply_request(&mut req, &ctx.build().unwrap());
	assert_eq!(req.headers().get("x-insert").unwrap(), "hello Bar");
}

#[tokio::test]
async fn test_transformation_body() {
	let req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/")
		.body(crate::http::Body::empty())
		.unwrap();
	let c = super::LocalTransformationConfig {
		request: None,
		response: Some(super::LocalTransform {
			body: Some("\"hello\" + request.method".into()),
			..Default::default()
		}),
	};
	let xfm = Transformation::try_from(c).unwrap();
	let mut ctx = ContextBuilder::new();
	for e in xfm.expressions() {
		ctx.register_expression(e)
	}
	ctx.with_request(&req, "".to_string());
	let mut resp = ::http::Response::builder()
		.status(200)
		.body(crate::http::Body::empty())
		.unwrap();
	xfm.apply_response(&mut resp, &ctx.build().unwrap());
	let b = http::read_body_with_limit(resp.into_body(), 1000)
		.await
		.unwrap();
	assert_eq!(b.as_ref(), b"helloGET");
}

#[test]
fn test_transformation_pseudoheader() {
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/")
		.header("X-Custom-Foo", "Bar")
		.body(crate::http::Body::empty())
		.unwrap();
	let xfm = build([
		(
			":method",
			r#"request.headers["x-custom-foo"] == "Bar" ? "POST" : request.method"#,
		),
		(":path", r#""/" + request.uri.split("://")[0]"#),
		(":authority", r#""example.com""#),
	]);
	let mut ctx = ContextBuilder::new();
	for e in xfm.expressions() {
		ctx.register_expression(e)
	}
	ctx.with_request(&req, "".to_string());
	xfm.apply_request(&mut req, &ctx.build().unwrap());
	assert_eq!(req.method().as_str(), "POST");
	assert_eq!(req.uri().to_string().as_str(), "https://example.com/https");
}
