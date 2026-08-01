use agentgateway::http::compression::{Compression, RequestDecompression};
use agentgateway::test_helpers::extauthmock;
use agentgateway::test_helpers::proxymock::RequestDump;
use agentgateway::types::agent::{
	ListenerTarget, PolicyPhase, PolicyTarget, TargetedPolicy, TrafficPolicy,
};
use headers::{ContentEncoding, HeaderMapExt};

use crate::common::prelude::*;

struct DenyAuthorization;

#[async_trait::async_trait]
impl extauthmock::Handler for DenyAuthorization {
	async fn check(
		&mut self,
		_request: &agentgateway::http::ext_authz::proto::CheckRequest,
	) -> Result<agentgateway::http::ext_authz::proto::CheckResponse, tonic::Status> {
		extauthmock::deny_response(
			agentgateway::http::ext_authz::proto::StatusCode::Forbidden,
			"external authorization failed",
		)
	}
}

#[tokio::test]
async fn response_compression_uses_downstream_negotiation() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"compression": {
				"responseCompression": {
					"preferredAlgorithms": ["brotli", "deflate", "gzip", "zstd"],
				},
			},
		}))
		.await;

	let response = send_request_headers(
		io,
		Method::GET,
		"http://lo/p",
		&[("accept-encoding", "zstd, gzip, br")],
	)
	.await;
	assert_eq!(response.headers()[header::CONTENT_ENCODING], "zstd");
	assert!(
		response.headers()[header::VARY]
			.to_str()
			.unwrap()
			.eq_ignore_ascii_case("Accept-Encoding")
	);
	assert!(!response.headers().contains_key(header::CONTENT_LENGTH));

	let (parts, body) = response.into_parts();
	let encoding = parts.headers.typed_get::<ContentEncoding>();
	let (body, _) =
		agentgateway::http::compression::decompress_body(body, encoding.as_ref()).unwrap();
	let decoded = body.collect().await.unwrap().to_bytes();
	let request: RequestDump = serde_json::from_slice(&decoded).unwrap();
	assert_eq!(request.uri.path(), "/p");
}

#[tokio::test]
async fn request_decompression_forwards_decoded_body() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"compression": {
				"requestDecompression": {
					"acceptedAlgorithms": ["deflate"],
				},
			},
		}))
		.await;

	let original = b"a compressed request body";
	let encoded = agentgateway::http::compression::encode_body(original, "deflate")
		.await
		.unwrap();
	let response = RequestBuilder::new(Method::POST, "http://lo/p")
		.header(header::CONTENT_ENCODING.as_str(), "deflate")
		.body(Body::from(encoded))
		.send(io)
		.await
		.unwrap();

	let body = response.collect().await.unwrap().to_bytes();
	let request: RequestDump = serde_json::from_slice(&body).unwrap();
	assert_eq!(request.body, original.as_slice());
	assert!(!request.headers.contains_key(header::CONTENT_ENCODING));
	assert!(!request.headers.contains_key(header::CONTENT_LENGTH));
}

#[tokio::test]
async fn request_body_policies_see_decompressed_body() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"compression": {
					"requestDecompression": {},
				},
				"directResponse": {
					"bodyExpression": "request.body",
					"status": 200,
				},
			},
		}))
		.await;

	let original = b"decompressed CEL request body";
	let encoded = agentgateway::http::compression::encode_body(original, "gzip")
		.await
		.unwrap();
	let response = RequestBuilder::new(Method::POST, "http://lo/p")
		.header(header::CONTENT_ENCODING.as_str(), "gzip")
		.body(Body::from(encoded))
		.send(io)
		.await
		.unwrap();

	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(read_body!(response).as_ref(), original);
}

#[tokio::test]
async fn request_decompression_is_lazy_before_authorization() {
	let (_mock, mut bind, io) = basic_setup().await;
	let authz = extauthmock::ExtAuthMock::new(|| DenyAuthorization)
		.spawn()
		.await;
	bind
		.attach_route(json!({
			"policies": {
				"compression": {
					"requestDecompression": {},
				},
				"extAuthz": {
					"host": authz.address,
				},
			},
		}))
		.await;

	let response = RequestBuilder::new(Method::POST, "http://lo/p")
		.header(header::CONTENT_ENCODING.as_str(), "gzip")
		.body(Body::from("not a gzip stream"))
		.send(io)
		.await
		.unwrap();

	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	assert_eq!(
		read_body!(response).as_ref(),
		b"external authorization failed"
	);
}

#[tokio::test]
async fn body_dependent_rate_limit_is_lazy_before_authorization() {
	let (_mock, mut bind, io) = basic_setup().await;
	let authz = extauthmock::ExtAuthMock::new(|| DenyAuthorization)
		.spawn()
		.await;
	bind
		.attach_route(json!({
			"policies": {
				"compression": {
					"requestDecompression": {},
				},
				"extAuthz": {
					"host": authz.address,
				},
				"remoteRateLimit": {
					"domain": "test",
					"host": "127.0.0.1:1",
					"descriptors": [{
						"entries": [{
							"key": "body",
							"value": "string(request.body)",
						}],
					}],
				},
			},
		}))
		.await;

	let pending_body = Body::from_stream(futures_util::stream::pending::<
		Result<bytes::Bytes, Infallible>,
	>());
	let response = tokio::time::timeout(
		Duration::from_secs(1),
		RequestBuilder::new(Method::POST, "http://lo/p")
			.header(header::CONTENT_ENCODING.as_str(), "gzip")
			.body(pending_body)
			.send(io),
	)
	.await
	.expect("authorization should reject without consuming the request body")
	.unwrap();

	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	assert_eq!(
		read_body!(response).as_ref(),
		b"external authorization failed"
	);
}

#[tokio::test]
async fn compression_inherits_as_a_single_policy() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind.with_policy(TargetedPolicy {
		key: "gateway-compression".into(),
		name: None,
		target: PolicyTarget::Gateway(ListenerTarget {
			gateway_name: "default".into(),
			gateway_namespace: "default".into(),
			listener_name: None,
			port: None,
		}),
		inheritance: Default::default(),
		policy: (
			TrafficPolicy::Compression(Compression {
				response_compression: None,
				request_decompression: Some(RequestDecompression::default()),
			}),
			PolicyPhase::Route,
		)
			.into(),
	});
	bind
		.attach_route_policy(json!({
			"compression": {
				"responseCompression": {},
			},
		}))
		.await;

	let original = b"request remains compressed";
	let encoded = agentgateway::http::compression::encode_body(original, "gzip")
		.await
		.unwrap();
	let response = RequestBuilder::new(Method::POST, "http://lo/p")
		.header(header::ACCEPT_ENCODING.as_str(), "gzip")
		.header(header::CONTENT_ENCODING.as_str(), "gzip")
		.body(Body::from(encoded.clone()))
		.send(io)
		.await
		.unwrap();

	assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
	let (parts, body) = response.into_parts();
	let encoding = parts.headers.typed_get::<ContentEncoding>();
	let (body, _) =
		agentgateway::http::compression::decompress_body(body, encoding.as_ref()).unwrap();
	let decoded = body.collect().await.unwrap().to_bytes();
	let request: RequestDump = serde_json::from_slice(&decoded).unwrap();
	assert_eq!(request.body, encoded);
	assert_eq!(request.headers[header::CONTENT_ENCODING], "gzip");
}
