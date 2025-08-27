use std::borrow::Cow;
use std::sync::Arc;

use agent_core::{drain, metrics, strng};
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use prometheus_client::registry::Registry;
use rmcp::model::Tool;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::client::Client;
use crate::store::Stores;
use crate::types::agent::Target;
use crate::{ProxyInputs, client, mcp};

// Helper to create a handler and mock server for tests
async fn setup() -> (MockServer, Handler) {
	let server = MockServer::start().await;
	let host = server.uri();
	let parsed = reqwest::Url::parse(&host).unwrap();
	let config = crate::config::parse_config("{}".to_string(), None).unwrap();
	let stores = Stores::new();
	let client = Client::new(
		&client::Config {
			resolver_cfg: ResolverConfig::default(),
			resolver_opts: ResolverOpts::default(),
		},
		None,
	);
	let (_drain_tx, drain_rx) = drain::new();
	let pi = Arc::new(ProxyInputs {
		cfg: Arc::new(config),
		stores: stores.clone(),
		tracer: None,
		metrics: Arc::new(crate::metrics::Metrics::new(metrics::sub_registry(
			&mut Registry::default(),
		))),
		upstream: client.clone(),
		ca: None,

		mcp_state: mcp::sse::App::new(
			stores.clone(),
			Arc::new(crate::mcp::relay::metrics::Metrics::new(
				&mut Registry::default(),
				None, // TODO custom tags
			)),
			drain_rx.clone(),
		),
	});

	let client = PolicyClient { inputs: pi };
	// Define a sample tool for testing
	let test_tool_get = Tool {
		name: Cow::Borrowed("get_user"),
		description: Some(Cow::Borrowed("Get user details")), // Added description
		input_schema: Arc::new(
			json!({ // Define a simple schema for testing
					"type": "object",
					"properties": {
							"path": {
									"type": "object",
									"properties": {
											"user_id": {"type": "string"}
									},
									"required": ["user_id"]
							},
							"query": {
									"type": "object",
									"properties": {
											"verbose": {"type": "string"}
									}
							},
							"header": {
									"type": "object",
									"properties": {
											"X-Request-ID": {"type": "string"}
									}
							}
					},
					"required": ["path"] // Only path is required for this tool
			})
			.as_object()
			.unwrap()
			.clone(),
		),
		annotations: None,
		output_schema: None,
	};
	let upstream_call_get = UpstreamOpenAPICall {
		method: "GET".to_string(),
		path: "/users/{user_id}".to_string(),
	};

	let test_tool_post = Tool {
		name: Cow::Borrowed("create_user"),
		description: Some(Cow::Borrowed("Create a new user")),
		input_schema: Arc::new(
			json!({
				"type": "object",
				"properties": {
					"body": {
						"type": "object",
						"properties": {
							"name": {"type": "string"},
							"email": {"type": "string"}
						},
						"required": ["name", "email"]
					},
					"query": {
						"type": "object",
						"properties": {
							"source": {"type": "string"}
						}
					},
					"header": {
						"type": "object",
						"properties": {
							"X-API-Key": {"type": "string"}
						}
					}
				},
				"required": ["body"]
			})
			.as_object()
			.unwrap()
			.clone(),
		),
		output_schema: None,
		annotations: None,
	};
	let upstream_call_post = UpstreamOpenAPICall {
		method: "POST".to_string(),
		path: "/users".to_string(),
	};

	let handler = Handler {
		prefix: "".to_string(),
		client,
		tools: vec![
			(test_tool_get, upstream_call_get),
			(test_tool_post, upstream_call_post),
		],
		default_policies: BackendPolicies::default(),
		backend: SimpleBackend::Opaque(
			strng::literal!("dummy"),
			Target::Hostname(
				parsed.host().unwrap().to_string().into(),
				parsed.port().unwrap_or(8080),
			),
		),
	};

	(server, handler)
}

#[tokio::test]
async fn test_call_tool_get_simple_success() {
	let (server, handler) = setup().await;

	let user_id = "123";
	let expected_response = json!({ "id": user_id, "name": "Test User" });

	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.respond_with(ResponseTemplate::new(200).set_body_json(&expected_response))
		.mount(&server)
		.await;

	let args = json!({ "path": { "user_id": user_id } });
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	assert!(result.is_ok());
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_call_tool_get_with_query() {
	let (server, handler) = setup().await;

	let user_id = "456";
	let verbose_flag = "true";
	let expected_response =
		json!({ "id": user_id, "name": "Test User", "details": "Verbose details" });

	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.and(query_param("verbose", verbose_flag))
		.respond_with(ResponseTemplate::new(200).set_body_json(&expected_response))
		.mount(&server)
		.await;

	let args = json!({ "path": { "user_id": user_id }, "query": { "verbose": verbose_flag } });
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	assert!(result.is_ok());
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_call_tool_get_with_header() {
	let (server, handler) = setup().await;

	let user_id = "789";
	let request_id = "req-abc";
	let expected_response = json!({ "id": user_id, "name": "Another User" });

	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.and(header("X-Request-ID", request_id))
		.respond_with(ResponseTemplate::new(200).set_body_json(&expected_response))
		.mount(&server)
		.await;

	let args = json!({ "path": { "user_id": user_id }, "header": { "X-Request-ID": request_id } });
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	assert!(result.is_ok());
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_call_tool_post_with_body() {
	let (server, handler) = setup().await;

	let request_body = json!({ "name": "New User", "email": "new@example.com" });
	let expected_response = json!({ "id": "xyz", "name": "New User", "email": "new@example.com" });

	Mock::given(method("POST"))
		.and(path("/users"))
		.and(body_json(&request_body))
		.respond_with(ResponseTemplate::new(201).set_body_json(&expected_response))
		.mount(&server)
		.await;

	let args = json!({ "body": request_body });
	let result = handler
		.call_tool(
			"create_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	assert!(result.is_ok());
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_call_tool_post_all_params() {
	let (server, handler) = setup().await;

	let request_body = json!({ "name": "Complete User", "email": "complete@example.com" });
	let api_key = "secret-key";
	let source = "test-suite";
	let expected_response = json!({ "id": "comp-123", "name": "Complete User" });

	Mock::given(method("POST"))
		.and(path("/users"))
		.and(query_param("source", source))
		.and(header("X-API-Key", api_key))
		.and(body_json(&request_body))
		.respond_with(ResponseTemplate::new(201).set_body_json(&expected_response))
		.mount(&server)
		.await;

	let args = json!({
			"body": request_body,
			"query": { "source": source },
			"header": { "X-API-Key": api_key }
	});
	let result = handler
		.call_tool(
			"create_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	assert!(result.is_ok());
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_call_tool_tool_not_found() {
	let (_server, handler) = setup().await; // Mock server not needed

	let args = json!({});
	let result = handler
		.call_tool(
			"nonexistent_tool",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	assert!(result.is_err());
	assert!(
		result
			.unwrap_err()
			.to_string()
			.contains("tool nonexistent_tool not found")
	);
}

#[tokio::test]
async fn test_call_tool_upstream_error() {
	let (server, handler) = setup().await;

	let user_id = "error-user";
	let error_response = json!({ "error": "User not found" });

	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.respond_with(ResponseTemplate::new(404).set_body_json(&error_response))
		.mount(&server)
		.await;

	let args = json!({ "path": { "user_id": user_id } });
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	assert!(result.is_err());
	let err = result.unwrap_err();
	assert!(err.to_string().contains("failed with status 404 Not Found"));
	assert!(err.to_string().contains(&error_response.to_string()));
}

#[tokio::test]
async fn test_call_tool_invalid_header_value() {
	let (server, handler) = setup().await;

	let user_id = "header-issue";
	// Mock is set up but won't be hit because header construction fails client-side
	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": user_id })))
		.mount(&server)
		.await;

	// Intentionally provide a non-string header value
	let args = json!({
			"path": { "user_id": user_id },
			"header": { "X-Request-ID": 12345 } // Invalid header value (not a string)
	});

	// We expect the call to succeed, but the invalid header should be skipped (and logged)
	// The mock doesn't expect the header, so if the request goes through without it, it passes.
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;
	assert!(result.is_ok()); // Check that the call still succeeds despite the bad header
	assert_eq!(result.unwrap(), json!({ "id": user_id }));
	// We can't easily assert the log message here, but manual inspection of logs would show the warning.
}

#[tokio::test]
async fn test_call_tool_invalid_query_param_value() {
	let (server, handler) = setup().await;

	let user_id = "query-issue";
	// Mock is set up but won't be hit with the invalid query param
	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		// IMPORTANT: We don't .and(query_param(...)) here because the invalid param is skipped
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": user_id })))
		.mount(&server)
		.await;

	// Intentionally provide a non-string query value
	let args = json!({
			"path": { "user_id": user_id },
			"query": { "verbose": true } // Invalid query value (not a string)
	});

	// We expect the call to succeed, but the invalid query param should be skipped (and logged)
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;
	assert!(result.is_ok());
	assert_eq!(result.unwrap(), json!({ "id": user_id }));
}

#[tokio::test]
async fn test_call_tool_invalid_path_param_value() {
	let (server, handler) = setup().await;

	let invalid_user_id = json!(12345); // Not a string
	// Mock is set up for the *literal* path, as substitution will fail
	Mock::given(method("GET"))
		.and(path("/users/{user_id}")) // Path doesn't get substituted
		.respond_with(
			ResponseTemplate::new(404) // Or whatever the server does with a literal {user_id}
				.set_body_string("Not Found - Literal Path"),
		)
		.mount(&server)
		.await;

	let args = json!({
			"path": { "user_id": invalid_user_id }
	});

	// The call might succeed at the HTTP level but might return an error from the server,
	// or potentially fail if the path is fundamentally invalid after non-substitution.
	// Here we assume the server returns 404 for the literal path.
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&HeaderMap::new(),
		)
		.await;

	// Depending on server behavior for the literal path, this might be Ok or Err.
	// If server returns 404 for the literal path:
	assert!(result.is_err());
	assert!(
		result
			.as_ref()
			.unwrap_err()
			.to_string()
			.contains("failed with status 404 Not Found"),
		"{}",
		result.unwrap_err().to_string()
	);

	// If the request *itself* failed before sending (e.g., invalid URL formed),
	// the error might be different.
}
