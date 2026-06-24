use http::Uri;
use serde_json::json;

use super::*;
use crate::http::{self, Method, header};
use crate::types::agent::A2aPolicy;

#[test]
fn test_build_agent_path() {
	let test_cases = vec![
		// Test stripping /.well-known/agent.json
		(
			"https://example.com/.well-known/agent.json",
			"https://example.com",
		),
		(
			"https://example.com/api/.well-known/agent.json",
			"https://example.com/api",
		),
		(
			"http://localhost:8080/service/.well-known/agent.json",
			"http://localhost:8080/service",
		),
		// Test stripping /.well-known/agent-card.json
		(
			"https://example.com/.well-known/agent-card.json",
			"https://example.com",
		),
		(
			"https://example.com/api/.well-known/agent-card.json",
			"https://example.com/api",
		),
		(
			"http://localhost:8080/service/.well-known/agent-card.json",
			"http://localhost:8080/service",
		),
		(
			"https://example.com:443/.well-known/agent.json",
			"https://example.com:443",
		),
		(
			"http://example.com:80/.well-known/agent-card.json",
			"http://example.com:80",
		),
	];

	for (input_url, expected_output) in test_cases {
		let uri: Uri = input_url.parse().expect("Failed to parse URI");
		let result = build_agent_path(uri);
		assert_eq!(result, expected_output, "Failed for input: {input_url}");
	}
}

#[tokio::test]
async fn test_classify_request_extracts_method_and_preserves_body() {
	let payload = json!({
		"jsonrpc": "2.0",
		"id": "2",
		"method": "tasks/send",
		"params": { "id": "task-123" },
	});
	let body = serde_json::to_vec(&payload).unwrap();
	let mut req = ::http::Request::builder()
		.method(Method::POST)
		.uri("https://example.com/")
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(body.clone()))
		.unwrap();

	let ty = classify_request(&mut req).await;

	match ty {
		RequestType::Call(method) => assert_eq!(method.as_str(), "tasks/send"),
		other => panic!("expected call request, got {other:?}"),
	}
	assert_eq!(http::read_req_body(req).await.unwrap(), body);
}

#[tokio::test]
async fn test_classify_request_uses_original_url_for_agent_card() {
	let original: Uri = "https://example.com/api/.well-known/agent-card.json"
		.parse()
		.unwrap();
	let mut req = ::http::Request::builder()
		.method(Method::GET)
		.uri("http://backend.internal/.well-known/agent-card.json")
		.body(http::Body::empty())
		.unwrap();
	req
		.extensions_mut()
		.insert(crate::http::filters::OriginalUrl(original.clone()));

	let ty = classify_request(&mut req).await;

	match ty {
		RequestType::AgentCard(uri) => assert_eq!(uri, original),
		other => panic!("expected agent card request, got {other:?}"),
	}
}

#[tokio::test]
async fn test_classify_request_uses_original_url_for_agent_card_with_subpath() {
	let original: Uri = "https://example.com/api/.well-known/agent-card.json"
		.parse()
		.unwrap();
	let mut req = ::http::Request::builder()
		.method(Method::GET)
		.uri("http://backend.internal/sub/path/.well-known/agent-card.json")
		.body(http::Body::empty())
		.unwrap();
	req
		.extensions_mut()
		.insert(crate::http::filters::OriginalUrl(original.clone()));

	let ty = classify_request(&mut req).await;

	match ty {
		RequestType::AgentCard(uri) => assert_eq!(uri, original),
		other => panic!("expected agent card request, got {other:?}"),
	}
}

#[tokio::test]
async fn test_classify_request_uses_x_forwarded_proto_for_agent_card() {
	let original: Uri = "http://example.com/api/.well-known/agent-card.json"
		.parse()
		.unwrap();
	let mut req = ::http::Request::builder()
		.method(Method::GET)
		.uri("http://backend.internal/.well-known/agent-card.json")
		.header("x-forwarded-proto", "https")
		.body(http::Body::empty())
		.unwrap();
	req
		.extensions_mut()
		.insert(crate::http::filters::OriginalUrl(original));

	let ty = classify_request(&mut req).await;

	match ty {
		RequestType::AgentCard(uri) => {
			assert_eq!(
				uri,
				"https://example.com/api/.well-known/agent-card.json"
					.parse::<Uri>()
					.unwrap()
			)
		},
		other => panic!("expected agent card request, got {other:?}"),
	}
}

#[tokio::test]
async fn test_classify_request_returns_unknown_method_on_invalid_json() {
	let mut req = ::http::Request::builder()
		.method(Method::POST)
		.uri("https://example.com/")
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from("{\"jsonrpc\":\"2.0\""))
		.unwrap();

	let ty = classify_request(&mut req).await;

	match ty {
		RequestType::Call(method) => assert_eq!(method.as_str(), "unknown"),
		other => panic!("expected call request, got {other:?}"),
	}
}

#[tokio::test]
async fn test_apply_to_response_rewrites_agent_card_url() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "example",
				"url": "http://backend.internal/.well-known/agent-card.json",
			}))
			.unwrap(),
		))
		.unwrap();

	apply_to_response(
		Some(&A2aPolicy {}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
		),
		&mut resp,
	)
	.await
	.unwrap();

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(json["url"], "https://example.com/api");
}

#[tokio::test]
async fn test_apply_to_response_rewrites_v1_agent_card_single_interface() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "example",
				"supportedInterfaces": [
					{ "protocolBinding": "JSONRPC", "url": "http://backend.internal/a2a/jsonrpc/" }
				],
			}))
			.unwrap(),
		))
		.unwrap();

	apply_to_response(
		Some(&A2aPolicy {}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
		),
		&mut resp,
	)
	.await
	.unwrap();

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"https://example.com/api/a2a/jsonrpc/"
	);
}

#[tokio::test]
async fn test_apply_to_response_rewrites_v1_agent_card_multiple_interfaces() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "example",
				"supportedInterfaces": [
					{ "protocolBinding": "JSONRPC", "url": "http://backend.internal/a2a/jsonrpc/" },
					{ "protocolBinding": "GRPC", "url": "http://backend.internal/grpc/" },
				],
			}))
			.unwrap(),
		))
		.unwrap();

	apply_to_response(
		Some(&A2aPolicy {}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
		),
		&mut resp,
	)
	.await
	.unwrap();

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"https://example.com/api/a2a/jsonrpc/"
	);
	assert_eq!(
		json["supportedInterfaces"][1]["url"],
		"https://example.com/api/grpc/"
	);
}

#[tokio::test]
async fn test_apply_to_response_rewrites_v1_agent_card_root_path() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "example",
				"supportedInterfaces": [
					{ "protocolBinding": "JSONRPC", "url": "http://backend.internal/a2a/jsonrpc/" }
				],
			}))
			.unwrap(),
		))
		.unwrap();

	apply_to_response(
		Some(&A2aPolicy {}),
		RequestType::AgentCard(
			"https://example.com/.well-known/agent-card.json"
				.parse()
				.unwrap(),
		),
		&mut resp,
	)
	.await
	.unwrap();

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"https://example.com/a2a/jsonrpc/"
	);
}

#[tokio::test]
async fn test_apply_to_response_skips_interface_without_url() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "example",
				"supportedInterfaces": [
					{ "protocolBinding": "GRPC" },
					{ "protocolBinding": "JSONRPC", "url": "http://backend.internal/a2a/jsonrpc/" },
				],
			}))
			.unwrap(),
		))
		.unwrap();

	apply_to_response(
		Some(&A2aPolicy {}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
		),
		&mut resp,
	)
	.await
	.unwrap();

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	assert!(json["supportedInterfaces"][0].get("url").is_none());
	assert_eq!(
		json["supportedInterfaces"][1]["url"],
		"https://example.com/api/a2a/jsonrpc/"
	);
}

#[tokio::test]
async fn test_apply_to_response_errors_when_neither_url_field_present() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({ "name": "example" })).unwrap(),
		))
		.unwrap();

	let result = apply_to_response(
		Some(&A2aPolicy {}),
		RequestType::AgentCard(
			"https://example.com/.well-known/agent-card.json"
				.parse()
				.unwrap(),
		),
		&mut resp,
	)
	.await;

	assert!(result.is_err());
	assert!(
		result
			.unwrap_err()
			.to_string()
			.contains("agent card missing URL")
	);
}
