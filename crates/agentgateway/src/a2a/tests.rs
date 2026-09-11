use bytes::Bytes;
use http::Uri;
use http_body::Frame;
use http_body_util::StreamBody;
use serde_json::json;

use super::*;
use crate::http::{self, Method, header};
use crate::transport::BufferLimit;
use crate::types::agent::A2aPolicy;

#[test]
fn test_build_agent_path() {
	let test_cases = vec![
		// Test stripping /.well-known/agent.json
		(
			"https://example.com/.well-known/agent.json",
			"/.well-known/agent.json",
			"https://example.com",
		),
		(
			"https://example.com/api/.well-known/agent.json",
			"/.well-known/agent.json",
			"https://example.com/api",
		),
		(
			"http://localhost:8080/service/.well-known/agent.json",
			"/.well-known/agent.json",
			"http://localhost:8080/service",
		),
		// Test stripping /.well-known/agent-card.json
		(
			"https://example.com/.well-known/agent-card.json",
			"/.well-known/agent-card.json",
			"https://example.com",
		),
		(
			"https://example.com/api/.well-known/agent-card.json",
			"/.well-known/agent-card.json",
			"https://example.com/api",
		),
		(
			"http://localhost:8080/service/.well-known/agent-card.json",
			"/.well-known/agent-card.json",
			"http://localhost:8080/service",
		),
		(
			"https://example.com:443/.well-known/agent.json",
			"/.well-known/agent.json",
			"https://example.com:443",
		),
		(
			"http://example.com:80/.well-known/agent-card.json",
			"/.well-known/agent-card.json",
			"http://example.com:80",
		),
		// Test stripping custom agent card paths (issue #3018)
		(
			"https://solutions.com/api/ai/config/agents/agent-orchestrator/agent.json",
			"/agent.json",
			"https://solutions.com/api/ai/config/agents/agent-orchestrator",
		),
		(
			"https://elasticsearch.com/s/_ops/api/agent_builder/a2a/test_v1.json",
			"/test_v1.json",
			"https://elasticsearch.com/s/_ops/api/agent_builder/a2a",
		),
	];

	for (input_url, suffix, expected_output) in test_cases {
		let uri: Uri = input_url.parse().expect("Failed to parse URI");
		let result = build_agent_path(uri, suffix);
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

	let pol = A2aPolicy {
		agent_card_path: None,
	};
	let ty = classify_request(&pol, &mut req).await;

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

	let pol = A2aPolicy {
		agent_card_path: None,
	};
	let ty = classify_request(&pol, &mut req).await;

	match ty {
		RequestType::AgentCard(uri, _, _, _) => assert_eq!(uri, original),
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

	let pol = A2aPolicy {
		agent_card_path: None,
	};
	let ty = classify_request(&pol, &mut req).await;

	match ty {
		RequestType::AgentCard(uri, _, _, _) => assert_eq!(uri, original),
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

	let pol = A2aPolicy {
		agent_card_path: None,
	};
	let ty = classify_request(&pol, &mut req).await;

	match ty {
		RequestType::AgentCard(uri, _, _, _) => {
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

	let pol = A2aPolicy {
		agent_card_path: None,
	};
	let ty = classify_request(&pol, &mut req).await;

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

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

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

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

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

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

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

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"https://example.com/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

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

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"https://example.com/api/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

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
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"https://example.com/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
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

#[tokio::test]
async fn test_apply_to_response_records_success_call_telemetry() {
	let payload = json!({
		"jsonrpc": "2.0",
		"id": "1",
		"result": {
			"kind": "task",
			"contextId": "ctx",
			"status": { "state": "completed" }
		}
	});
	let raw = serde_json::to_vec(&payload).unwrap();
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(raw.clone()))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("tasks/send")),
		&mut resp,
	)
	.await
	.unwrap()
	.expect("success response should produce telemetry");

	assert_eq!(info.outcome, ResponseOutcome::Success);
	assert_eq!(info.error_code, None);
	assert_eq!(info.result_kind.as_ref().map(|s| s.as_str()), Some("task"));
	assert_eq!(
		info.task_state.as_ref().map(|s| s.as_str()),
		Some("completed")
	);
	assert_eq!(info.context_id.as_ref().map(|s| s.as_str()), Some("ctx"));
	assert_eq!(http::read_resp_body(resp).await.unwrap(), raw);
}

#[tokio::test]
async fn test_apply_to_response_records_error_call_telemetry() {
	let payload = json!({
		"jsonrpc": "2.0",
		"id": "1",
		"error": {
			"code": -32602,
			"message": "Invalid params"
		}
	});
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(serde_json::to_vec(&payload).unwrap()))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("tasks/send")),
		&mut resp,
	)
	.await
	.unwrap()
	.expect("error response should produce telemetry");

	assert_eq!(info.outcome, ResponseOutcome::Error);
	assert_eq!(info.error_code, Some(-32602));
	assert_eq!(info.result_kind, None);
	assert_eq!(info.task_state, None);
}

#[tokio::test]
async fn test_apply_to_response_records_unknown_size_json_call_telemetry() {
	let frames = futures_util::stream::iter([
		Ok::<_, crate::http::Error>(Frame::data(Bytes::from_static(
			b"{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"result\":{",
		))),
		Ok::<_, crate::http::Error>(Frame::data(Bytes::from_static(
			b"\"kind\":\"task\",\"status\":{\"state\":\"working\"}}}",
		))),
	]);
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::new(StreamBody::new(frames)))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("tasks/send")),
		&mut resp,
	)
	.await
	.unwrap()
	.expect("finite JSON without a size hint should produce telemetry");

	assert_eq!(info.outcome, ResponseOutcome::Success);
	assert_eq!(info.result_kind.as_ref().map(|s| s.as_str()), Some("task"));
	assert_eq!(
		info.task_state.as_ref().map(|s| s.as_str()),
		Some("working")
	);
}

#[tokio::test]
async fn test_apply_to_response_records_unknown_call_telemetry() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(r#"{"jsonrpc":"2.0","id":"1"}"#))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("tasks/send")),
		&mut resp,
	)
	.await
	.unwrap()
	.expect("parseable response should produce telemetry");

	assert_eq!(info.outcome, ResponseOutcome::Unknown);
	assert_eq!(info.error_code, None);
	assert_eq!(info.result_kind, None);
	assert_eq!(info.task_state, None);
}

#[tokio::test]
async fn test_apply_to_response_skips_invalid_json_call_telemetry() {
	let raw = Bytes::from_static(b"{\"jsonrpc\":\"2.0\"");
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(raw.clone()))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("tasks/send")),
		&mut resp,
	)
	.await
	.unwrap();

	assert!(info.is_none());
	assert_eq!(
		http::read_body_with_limit(resp.into_body(), raw.len())
			.await
			.unwrap(),
		raw
	);
}

#[tokio::test]
async fn test_apply_to_response_skips_non_json_call_telemetry() {
	let raw = Bytes::from_static(b"ok");
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "text/plain")
		.body(http::Body::from(raw.clone()))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("tasks/send")),
		&mut resp,
	)
	.await
	.unwrap();

	assert!(info.is_none());
	assert_eq!(
		http::read_body_with_limit(resp.into_body(), raw.len())
			.await
			.unwrap(),
		raw
	);
}

#[tokio::test]
async fn test_apply_to_response_skips_partial_call_telemetry() {
	let raw = Bytes::from_static(b"{\"jsonrpc\":\"2.0\",\"result\":{\"kind\":\"task\"}}");
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(raw.clone()))
		.unwrap();
	resp.extensions_mut().insert(BufferLimit::new(4));

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("tasks/send")),
		&mut resp,
	)
	.await
	.unwrap();

	assert!(info.is_none());
	assert_eq!(
		http::read_body_with_limit(resp.into_body(), raw.len())
			.await
			.unwrap(),
		raw
	);
}

#[tokio::test]
async fn test_apply_to_response_rewrites_url_with_path_rewrite() {
	// Regression test for issue #2981: when a URL rewrite changes the gateway
	// path prefix (e.g., /a2a/tick -> /a2a/tock), the interface URL should be
	// anchored at the *gateway* path, not naively appended with the backend path.
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "TOCK Scheduler Agent",
				"supportedInterfaces": [
					{ "protocolBinding": "JSONRPC", "url": "http://localhost:8080/a2a/tock/", "protocolVersion": "1.0" }
				],
			}))
			.unwrap(),
		))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			// Original (gateway) URL where the client requested the agent card.
			"http://localhost:4001/a2a/tick/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			// Backend (rewritten) request path — the URL rewrite policy translated
			// /a2a/tick -> /a2a/tock before sending to the backend.
			"/a2a/tock/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	// The interface URL should be anchored at the gateway path (/a2a/tick),
	// NOT have the backend path (/a2a/tock) appended.
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"http://localhost:4001/a2a/tick/"
	);
}

#[tokio::test]
async fn test_apply_to_response_preserves_subpath_with_path_rewrite() {
	// When the backend interface URL has a sub-path beyond the agent card
	// location, that sub-path should be preserved after rewriting.
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "example",
				"supportedInterfaces": [
					{ "protocolBinding": "JSONRPC", "url": "http://localhost:8080/a2a/tock/jsonrpc/" }
				],
			}))
			.unwrap(),
		))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"http://localhost:4001/a2a/tick/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/a2a/tock/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	// The /jsonrpc/ sub-path should be preserved, anchored at the gateway path.
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"http://localhost:4001/a2a/tick/jsonrpc/"
	);
}

#[tokio::test]
async fn test_apply_to_response_avoids_partial_path_segment_match() {
	// Regression test for edge case: when backend has multiple interfaces with
	// similar path prefixes (e.g., /weather and /weather-v2), we should only
	// strip complete path segments, not partial matches.
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "Weather Agent",
				"supportedInterfaces": [
					{ "protocolBinding": "JSONRPC", "url": "http://backend/internal/weather/jsonrpc" },
					{ "protocolBinding": "JSONRPC", "url": "http://backend/internal/weather-v2/jsonrpc" }
				],
			}))
			.unwrap(),
		))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"http://gateway/public/weather/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/internal/weather/.well-known/agent-card.json".to_string(),
			None,
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();
	assert!(info.is_none());

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	// First interface: /internal/weather/jsonrpc -> /public/weather/jsonrpc
	// (correct: complete path segment match)
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"http://gateway/public/weather/jsonrpc"
	);
	// Second interface: /internal/weather-v2/jsonrpc should NOT be stripped to
	// -v2/jsonrpc because /internal/weather is not a complete path segment prefix
	// of /internal/weather-v2. The gateway_base is /public/weather, so the result
	// should be /public/weather/internal/weather-v2/jsonrpc (no stripping occurred).
	assert_eq!(
		json["supportedInterfaces"][1]["url"],
		"http://gateway/public/weather/internal/weather-v2/jsonrpc"
	);
}

#[tokio::test]
async fn test_apply_to_response_uses_prefix_rewrite_context() {
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "example",
				"supportedInterfaces": [
					{ "protocolBinding": "JSONRPC", "url": "https://upstream.example/svc/agent" }
				],
			}))
			.unwrap(),
		))
		.unwrap();

	apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::AgentCard(
			"http://gateway.example/gw/svc/agent/.well-known/agent-card.json"
				.parse()
				.unwrap(),
			"/svc/agent/.well-known/agent-card.json".to_string(),
			Some(crate::http::filters::AppliedUrlRewrite {
				path: Some(crate::types::agent::PathRedirect::Prefix("/".into())),
				path_match: crate::types::agent::PathMatch::PathPrefix("/gw".into()),
			}),
			"/.well-known/agent-card.json".to_string(),
		),
		&mut resp,
	)
	.await
	.unwrap();

	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"http://gateway.example/gw/svc/agent"
	);
}

#[test]
fn test_strip_complete_path_prefix_normalizes_trailing_slashes() {
	assert_eq!(
		strip_complete_path_prefix("/svc/agent", "/svc/"),
		Some("/agent")
	);
	assert_eq!(join_path_prefix("/gw/", "/svc/agent"), "/gw/svc/agent");
}

#[test]
fn test_replace_path_changes_only_the_path() {
	assert_eq!(
		replace_path("https://gateway.example/?redirect=/", "/gw/svc/agent"),
		"https://gateway.example/gw/svc/agent?redirect=/"
	);
}

#[tokio::test]
async fn test_apply_to_response_records_v1_nested_task_telemetry() {
	// A2A v1.0 shape: the `oneof payload` puts the Task under `result.task`, with no
	// `kind` discriminator.
	let payload = json!({
		"jsonrpc": "2.0",
		"id": "1",
		"result": {
			"task": {
				"id": "abc",
				"contextId": "ctx",
				"status": { "state": "completed" }
			}
		}
	});
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(serde_json::to_vec(&payload).unwrap()))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("SendMessage")),
		&mut resp,
	)
	.await
	.unwrap()
	.expect("v1.0 nested task response should produce telemetry");

	assert_eq!(info.outcome, ResponseOutcome::Success);
	assert_eq!(info.result_kind.as_ref().map(|s| s.as_str()), Some("task"));
	assert_eq!(
		info.task_state.as_ref().map(|s| s.as_str()),
		Some("completed")
	);
	assert_eq!(info.context_id.as_ref().map(|s| s.as_str()), Some("ctx"));
}

// Test cases for issue #3018: non-standard agent card paths
// These tests verify the behavior when agent card is at non-standard paths

#[tokio::test]
async fn test_classify_request_non_standard_agent_card_path_with_custom_config() {
	// Regression test for issue #3018: non-standard agents that host agent card
	// at a custom path should be recognized when agent_card_path is configured.
	//
	// Example: https://solutions.com/api/ai/config/agents/agent-orchestrator/agent.json
	let original: Uri = "https://example.com/api/ai/config/agents/agent-orchestrator/agent.json"
		.parse()
		.unwrap();
	let mut req = ::http::Request::builder()
		.method(Method::GET)
		.uri("http://backend.internal/api/ai/config/agents/agent-orchestrator/agent.json")
		.body(http::Body::empty())
		.unwrap();
	req
		.extensions_mut()
		.insert(crate::http::filters::OriginalUrl(original.clone()));

	// Configure custom agent card path
	let pol = A2aPolicy {
		agent_card_path: Some(agent_core::strng::Strng::from("/agent.json")),
	};
	let ty = classify_request(&pol, &mut req).await;

	// With custom path configured, this should now return AgentCard
	match ty {
		RequestType::AgentCard(uri, _, _, suffix) => {
			assert_eq!(uri, original);
			assert_eq!(suffix, "/agent.json");
		},
		other => {
			panic!("expected agent card request for non-standard path with custom config, got {other:?}")
		},
	}
}

#[tokio::test]
async fn test_classify_request_non_standard_agent_card_path_elasticsearch_style() {
	// Regression test for issue #3018: Elasticsearch-style non-standard agent card paths
	//
	// Example: https://elasticsearch.com/s/_ops/api/agent_builder/a2a/test_v1.json
	let original: Uri = "https://example.com/s/_ops/api/agent_builder/a2a/test_v1.json"
		.parse()
		.unwrap();
	let mut req = ::http::Request::builder()
		.method(Method::GET)
		.uri("http://backend.internal/s/_ops/api/agent_builder/a2a/test_v1.json")
		.body(http::Body::empty())
		.unwrap();
	req
		.extensions_mut()
		.insert(crate::http::filters::OriginalUrl(original.clone()));

	// Configure custom agent card path for Elasticsearch-style
	let pol = A2aPolicy {
		agent_card_path: Some(agent_core::strng::Strng::from("/test_v1.json")),
	};
	let ty = classify_request(&pol, &mut req).await;

	// With custom path configured, this should now return AgentCard
	match ty {
		RequestType::AgentCard(uri, _, _, suffix) => {
			assert_eq!(uri, original);
			assert_eq!(suffix, "/test_v1.json");
		},
		other => {
			panic!("expected agent card request for custom JSON path with custom config, got {other:?}")
		},
	}
}

#[tokio::test]
async fn test_classify_request_without_custom_config_still_works_for_standard_paths() {
	// Verify that standard paths still work without custom configuration
	let original: Uri = "https://example.com/.well-known/agent-card.json"
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

	// No custom path configured
	let pol = A2aPolicy {
		agent_card_path: None,
	};
	let ty = classify_request(&pol, &mut req).await;

	match ty {
		RequestType::AgentCard(uri, _, _, suffix) => {
			assert_eq!(uri, original);
			assert_eq!(suffix, "/.well-known/agent-card.json");
		},
		other => panic!("expected agent card request for standard path, got {other:?}"),
	}
}

#[tokio::test]
async fn test_apply_to_response_non_standard_agent_card_url_rewrite() {
	// Regression test for issue #3018: URL rewrite should work for non-standard agent cards
	// This test verifies that when a non-standard agent card is recognized,
	// the URL rewriting works correctly.
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "Elasticsearch Agent",
				"url": "https://elasticsearch.com/s/_ops/api/agent_builder/a2a/_test_v1",
			}))
			.unwrap(),
		))
		.unwrap();

	// This simulates a non-standard agent card path being recognized with custom suffix
	let result = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: Some(agent_core::strng::Strng::from("/_test_v1.json")),
		}),
		RequestType::AgentCard(
			"https://gateway.example.com/a2a/_test_v1.json"
				.parse()
				.unwrap(),
			"/s/_ops/api/agent_builder/a2a/_test_v1.json".to_string(),
			None,
			"/_test_v1.json".to_string(), // Custom suffix
		),
		&mut resp,
	)
	.await;

	// If the agent card is recognized, URL rewriting should work
	assert!(result.is_ok());
	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	// The URL should be rewritten to the gateway path (with suffix stripped)
	assert_eq!(json["url"], "https://gateway.example.com/a2a");
}

#[tokio::test]
async fn test_apply_to_response_non_standard_agent_card_with_supported_interfaces() {
	// Regression test for issue #3018: URL rewrite for non-standard agent cards
	// with supportedInterfaces (A2A v1.0 format)
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(
			serde_json::to_vec(&json!({
				"name": "Solutions Agent",
				"supportedInterfaces": [
					// Interface URL shares common path with the agent card path
					{ "protocolBinding": "JSONRPC", "url": "https://solutions.com/api/ai/config/agents/agent-orchestrator/a2a/" }
				],
			}))
			.unwrap(),
		))
		.unwrap();

	let result = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: Some(agent_core::strng::Strng::from("/agent.json")),
		}),
		RequestType::AgentCard(
			"https://gateway.example.com/a2a/agents/agent-orchestrator/agent.json"
				.parse()
				.unwrap(),
			"/api/ai/config/agents/agent-orchestrator/agent.json".to_string(),
			None,
			"/agent.json".to_string(), // Custom suffix
		),
		&mut resp,
	)
	.await;

	assert!(result.is_ok());
	let body = http::read_resp_body(resp).await.unwrap();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	// The interface URL should be rewritten to the gateway path
	// The common prefix /api/ai/config/agents/agent-orchestrator is stripped,
	// leaving /a2a/ which is appended to the gateway base
	assert_eq!(
		json["supportedInterfaces"][0]["url"],
		"https://gateway.example.com/a2a/agents/agent-orchestrator/a2a/"
	);
}

#[tokio::test]
async fn test_apply_to_response_records_v1_nested_message_telemetry() {
	// A2A v1.0 shape: the other `oneof payload` arm puts a Message under
	// `result.message`. A Message carries `contextId` but has no status.
	let payload = json!({
		"jsonrpc": "2.0",
		"id": "1",
		"result": {
			"message": {
				"messageId": "msg-1",
				"contextId": "ctx",
				"role": "ROLE_AGENT"
			}
		}
	});
	let mut resp = ::http::Response::builder()
		.header(header::CONTENT_TYPE, "application/json")
		.body(http::Body::from(serde_json::to_vec(&payload).unwrap()))
		.unwrap();

	let info = apply_to_response(
		Some(&A2aPolicy {
			agent_card_path: None,
		}),
		RequestType::Call(Strng::from("SendMessage")),
		&mut resp,
	)
	.await
	.unwrap()
	.expect("v1.0 nested message response should produce telemetry");

	assert_eq!(info.outcome, ResponseOutcome::Success);
	assert_eq!(
		info.result_kind.as_ref().map(|s| s.as_str()),
		Some("message")
	);
	assert_eq!(info.task_state, None);
	assert_eq!(info.context_id.as_ref().map(|s| s.as_str()), Some("ctx"));
}
