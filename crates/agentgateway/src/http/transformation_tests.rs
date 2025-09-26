use http::HeaderName;

use crate::http::transformation::Transformation;

fn build<const N: usize>(items: [(&str, &str); N]) -> Transformation {
	let hm = items
		.iter()
		.map(|(k, v)| (HeaderName::try_from(*k).unwrap(), v.to_string()))
		.collect();
	super::build(hm).unwrap()
}

#[test]
fn test_transformation() {
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/")
		.header("X-Custom-Foo", "Bar")
		.body(crate::http::Body::empty())
		.unwrap();
	let xfm = build([("x-insert", r#"hello {{ request_header("x-custom-foo") }}"#)]);
	let mut ctx = xfm.ctx();
	ctx.with_request(&req, "".to_string());
	xfm.apply(&mut req, ctx);
	assert_eq!(req.headers().get("x-insert").unwrap(), "hello Bar");
}
