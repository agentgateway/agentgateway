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
	xfm.apply_request(&mut req, &ctx.build().unwrap()).unwrap();
	assert_eq!(req.headers().get("x-insert").unwrap(), "hello Bar");
}
