use crate::common::prelude::*;

#[tokio::test]
async fn direct_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"responseHeaderModifier": {
					"add": {
						"x-filter": "x-filter-val"
					},
				},
				"directResponse": {
					"body": "hello",
					"status": 422,
				},
				"transformations": {
					"response": {
						"add": {
							"x-xfm": "'x-xfm-val'",
						},
					},
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 422);
	// Each type of response modifier should still run even though its a direct response
	assert_eq!(res.hdr("x-filter"), "x-filter-val");
	assert_eq!(res.hdr("x-xfm"), "x-xfm-val");
	assert_eq!(read_body!(res).as_ref(), b"hello");
}

#[tokio::test]
async fn direct_response_expression() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"directResponse": {
					"bodyExpression": "'hello ' + request.path",
					"headers": {
						"x-direct": "'direct-' + request.headers['x-test']",
					},
					"status": 422,
				},
			},
		}))
		.await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/p",
		&[("x-test", "value")],
	)
	.await;
	assert_eq!(res.status(), 422);
	assert_eq!(res.hdr("x-direct"), "direct-value");
	assert_eq!(read_body!(res).as_ref(), b"hello /p");
}

#[tokio::test]
async fn direct_response_expression_can_read_request_body() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"directResponse": {
					"bodyExpression": "request.body",
					"status": 200,
				},
			},
		}))
		.await;

	let res = send_request_body(io.clone(), Method::POST, "http://lo/p", b"echo me").await;
	assert_eq!(res.status(), 200);
	assert_eq!(read_body!(res).as_ref(), b"echo me");
}

#[tokio::test]
async fn conditional_direct_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"directResponse": {
					"conditional": [
						{
							"condition": "request.path == '/a'",
							"body": "hello a",
							"status": 200,
						},
						{
							"body": "hello fallback",
							"status": 202,
						},
					]
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/a").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo/b").await;
	assert_eq!(res.status(), 202);
}
