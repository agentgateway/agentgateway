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
	Transformation::try_from_local_config(c, true).unwrap()
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
	xfm.apply_request(&mut req);
	assert_eq!(req.headers().get("x-insert").unwrap(), "hello Bar");
}

#[tokio::test]
async fn test_transformation_body() {
	let mut req = ::http::Request::builder()
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
	let xfm = Transformation::try_from_local_config(c, true).unwrap();

	let mut resp = ::http::Response::builder()
		.status(200)
		.body(crate::http::Body::empty())
		.unwrap();
	let snap = cel::snapshot_request(&mut req, true);
	xfm.apply_response(&mut resp, Some(&snap));
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
	xfm.apply_request(&mut req);
	assert_eq!(req.method().as_str(), "POST");
	assert_eq!(req.uri().to_string().as_str(), "https://example.com/https");
}

#[test]
fn test_transformation_host_header_lifts_to_authority() {
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/")
		.body(crate::http::Body::empty())
		.unwrap();
	let xfm = build([("host", r#""example.com:8443""#)]);
	xfm.apply_request(&mut req);
	assert_eq!(req.uri().to_string().as_str(), "https://example.com:8443/");
	assert!(req.headers().get(::http::header::HOST).is_none());
}

#[test]
fn test_transformation_namespaced_variable() {
	// `vars.greeting` is reused by two header expressions.
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/")
		.body(crate::http::Body::empty())
		.unwrap();
	let c = super::LocalTransformationConfig {
		request: Some(super::LocalTransform {
			set: vec![
				(strng::new("x-greeting"), strng::new("vars.greeting")),
				(
					strng::new("x-greeting-upper"),
					strng::new("vars.greeting + \"!\""),
				),
			],
			variables: vec![cel::CelVariableSpec {
				name: "greeting".into(),
				expression: r#""hello " + request.method"#.into(),
				alias: false,
			}],
			..Default::default()
		}),
		response: None,
	};
	let xfm = Transformation::try_from_local_config(c, true).unwrap();
	xfm.apply_request(&mut req);
	assert_eq!(req.headers().get("x-greeting").unwrap(), "hello GET");
	assert_eq!(req.headers().get("x-greeting-upper").unwrap(), "hello GET!");
}

#[test]
fn test_transformation_alias_variable() {
	// `alias=true` exposes the var bare; `vars.<name>` form should not match.
	let mut req = ::http::Request::builder()
		.method("POST")
		.uri("https://www.rust-lang.org/")
		.body(crate::http::Body::empty())
		.unwrap();
	let c = super::LocalTransformationConfig {
		request: Some(super::LocalTransform {
			set: vec![(strng::new("x-method"), strng::new("aliased_method"))],
			variables: vec![cel::CelVariableSpec {
				name: "aliased_method".into(),
				expression: "request.method".into(),
				alias: true,
			}],
			..Default::default()
		}),
		response: None,
	};
	let xfm = Transformation::try_from_local_config(c, true).unwrap();
	xfm.apply_request(&mut req);
	assert_eq!(req.headers().get("x-method").unwrap(), "POST");
}

#[test]
fn test_transformation_metadata() {
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/example")
		.body(crate::http::Body::empty())
		.unwrap();
	let c = super::LocalTransformationConfig {
		request: Some(super::LocalTransform {
			metadata: vec![
				("originalPath".into(), "request.path".into()),
				("isGet".into(), "request.method == 'GET'".into()),
			],
			..Default::default()
		}),
		response: None,
	};
	let xfm = Transformation::try_from_local_config(c, true).unwrap();
	xfm.apply_request(&mut req);
	let md = req
		.extensions()
		.get::<TransformationMetadata>()
		.expect("metadata extension should be present");
	assert_eq!(
		md.0.get("originalPath").unwrap(),
		&serde_json::Value::String("/example".to_string())
	);
	assert_eq!(md.0.get("isGet").unwrap(), &serde_json::Value::Bool(true));
}
