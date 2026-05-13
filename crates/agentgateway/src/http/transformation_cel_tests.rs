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

#[tokio::test]
async fn test_transformation_form_urlencoded_body_merge() {
	let mut req = ::http::Request::builder()
		.method("POST")
		.uri("https://gateway.example.com/oauth/devicecode")
		.header("content-type", "application/x-www-form-urlencoded")
		.header("content-length", "0")
		.body(crate::http::Body::empty())
		.unwrap();
	req
		.extensions_mut()
		.insert(crate::cel::BufferedBody(bytes::Bytes::new()));

	let c = super::LocalTransformationConfig {
		request: Some(super::LocalTransform {
			body: Some(
				r#"
request.path == "/oauth/devicecode" ?
	form.encode(form.decode(request.body).merge({
		"client_id": "app-id",
		"scope": "openid profile api://app-id/access_as_user"
	})) :
request.path == "/oauth/token" ?
	form.encode(form.decode(request.body).merge({"client_id": "app-id"})) :
request.body
"#
				.into(),
			),
			..Default::default()
		}),
		response: None,
	};
	let xfm = Transformation::try_from_local_config(c, true).unwrap();

	xfm.apply_request(&mut req);

	assert!(req.headers().get(::http::header::CONTENT_LENGTH).is_none());
	let body = crate::http::read_body_with_limit(req.into_body(), 1000)
		.await
		.unwrap();
	let fields = url::form_urlencoded::parse(body.as_ref())
		.into_owned()
		.collect::<std::collections::HashMap<_, _>>();
	assert_eq!(fields.get("client_id").unwrap(), "app-id");
	assert_eq!(
		fields.get("scope").unwrap(),
		"openid profile api://app-id/access_as_user"
	);

	let mut req = ::http::Request::builder()
		.method("POST")
		.uri("https://gateway.example.com/oauth/token")
		.header("content-type", "application/x-www-form-urlencoded")
		.header("content-length", "0")
		.body(crate::http::Body::empty())
		.unwrap();
	req
		.extensions_mut()
		.insert(crate::cel::BufferedBody(bytes::Bytes::from_static(
			b"grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=abc",
		)));

	xfm.apply_request(&mut req);

	let body = crate::http::read_body_with_limit(req.into_body(), 1000)
		.await
		.unwrap();
	let fields = url::form_urlencoded::parse(body.as_ref())
		.into_owned()
		.collect::<std::collections::HashMap<_, _>>();
	assert_eq!(fields.get("client_id").unwrap(), "app-id");
	assert_eq!(fields.get("device_code").unwrap(), "abc");
	assert_eq!(
		fields.get("grant_type").unwrap(),
		"urn:ietf:params:oauth:grant-type:device_code"
	);
	assert!(!fields.contains_key("scope"));
}

#[tokio::test]
async fn test_transformation_response_json_body_rewrite() {
	let mut req = ::http::Request::builder()
		.method("POST")
		.uri("https://gateway.example.com/oauth/devicecode")
		.body(crate::http::Body::empty())
		.unwrap();
	let c = super::LocalTransformationConfig {
		request: None,
		response: Some(super::LocalTransform {
			body: Some(
				r#"
json(response.body).with(body,
	body.merge({
		"verification_uri": "https://gateway.example.com/oauth/verify",
		"verification_uri_complete": "https://gateway.example.com/oauth/verify?user_code=" + body.user_code
	})
)
	"#
					.into(),
				),
			..Default::default()
		}),
	};
	let xfm = Transformation::try_from_local_config(c, true).unwrap();
	let mut resp = ::http::Response::builder()
		.status(200)
		.header("content-type", "application/json")
		.body(crate::http::Body::from(
			r#"{"verification_uri":"https://login.microsoft.com/device","verification_uri_complete":"https://login.microsoft.com/device?user_code=ABCDEFGH","user_code":"ABCDEFGH"}"#,
		))
		.unwrap();
	resp.extensions_mut().insert(crate::cel::BufferedBody(
		bytes::Bytes::from_static(
			br#"{"verification_uri":"https://login.microsoft.com/device","verification_uri_complete":"https://login.microsoft.com/device?user_code=ABCDEFGH","user_code":"ABCDEFGH"}"#,
		),
	));

	let snap = cel::snapshot_request(&mut req, true);
	xfm.apply_response(&mut resp, Some(&snap));
	let body = crate::http::read_body_with_limit(resp.into_body(), 1000)
		.await
		.unwrap();
	let rewritten: serde_json::Value = serde_json::from_slice(body.as_ref()).unwrap();
	assert_eq!(
		rewritten["verification_uri"],
		"https://gateway.example.com/oauth/verify"
	);
	assert_eq!(
		rewritten["verification_uri_complete"],
		"https://gateway.example.com/oauth/verify?user_code=ABCDEFGH"
	);
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

#[test]
fn test_response_transformation_metadata_available_to_headers() {
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://www.rust-lang.org/example")
		.body(crate::http::Body::empty())
		.unwrap();
	let mut resp = ::http::Response::builder()
		.status(200)
		.body(crate::http::Body::empty())
		.unwrap();
	let c = super::LocalTransformationConfig {
		request: Some(super::LocalTransform {
			metadata: vec![
				("requestVal".into(), r#""from-request""#.into()),
				("shared".into(), r#""request""#.into()),
			],
			..Default::default()
		}),
		response: Some(super::LocalTransform {
			metadata: vec![
				("staticVal".into(), r#""hello-world""#.into()),
				("copied".into(), "metadata.requestVal".into()),
				("shared".into(), r#""response""#.into()),
			],
			set: vec![
				("x-static".into(), "metadata.staticVal".into()),
				("x-copied".into(), "metadata.copied".into()),
				("x-shared".into(), "metadata.shared".into()),
				("x-inline-static".into(), r#""hello-world""#.into()),
			],
			..Default::default()
		}),
	};
	let xfm = Transformation::try_from_local_config(c, true).unwrap();
	xfm.apply_request(&mut req);
	let snap = cel::snapshot_request(&mut req, true);

	xfm.apply_response(&mut resp, Some(&snap));

	assert_eq!(resp.headers().get("x-static").unwrap(), "hello-world");
	assert_eq!(resp.headers().get("x-copied").unwrap(), "from-request");
	assert_eq!(resp.headers().get("x-shared").unwrap(), "response");
	assert_eq!(
		resp.headers().get("x-inline-static").unwrap(),
		"hello-world"
	);

	let md = resp
		.extensions()
		.get::<TransformationMetadata>()
		.expect("metadata extension should be present");
	assert_eq!(
		md.0.get("requestVal").unwrap(),
		&serde_json::Value::String("from-request".to_string())
	);
	assert_eq!(
		md.0.get("staticVal").unwrap(),
		&serde_json::Value::String("hello-world".to_string())
	);
	assert_eq!(
		md.0.get("copied").unwrap(),
		&serde_json::Value::String("from-request".to_string())
	);
	assert_eq!(
		md.0.get("shared").unwrap(),
		&serde_json::Value::String("response".to_string())
	);

	let log_expr = cel::Expression::new_strict(
		r#"metadata.requestVal + "," + metadata.staticVal + "," + metadata.shared"#,
	)
	.unwrap();
	let mut log_context = cel::ContextBuilder::new();
	log_context.register_log_expression(&log_expr);
	let resp_snapshot = log_context
		.maybe_snapshot_response(&mut resp)
		.expect("metadata log expressions should snapshot response metadata");
	let log_exec = cel::Executor::new_logger(Some(&snap), Some(&resp_snapshot), None, None, None);
	let log_value = log_exec.eval(&log_expr).unwrap();
	assert_eq!(
		log_value,
		cel::Value::String("from-request,hello-world,response".into())
	);
}
