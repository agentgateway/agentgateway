use std::borrow::Cow;
use std::sync::Arc;

use agent_core::{metrics, strng};
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use openapiv3::ReferenceOr;
use prometheus_client::registry::Registry;
use rmcp::model::Tool;
use rstest::rstest;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::client::Client;
use crate::proxy::httpproxy::PolicyClient;
use crate::serdes::FileInlineOrRemote;
use crate::store::{BackendPolicies, Stores};
use crate::types::agent::{Backend, ResourceName, SimpleBackend, Target};
use crate::types::local::{
	LocalBackend, LocalMcpBackend, LocalMcpTarget, LocalMcpTargetSpec, McpBackendHost,
	McpStatefulMode,
};
use crate::{BackendConfig, ProxyInputs, client, mcp};

// Helper to create a handler and mock server for tests
async fn setup() -> (MockServer, Handler) {
	let server = MockServer::start().await;
	let host = server.uri();
	let parsed = reqwest::Url::parse(&host).unwrap();
	let config = crate::config::parse_config("{}".to_string(), None).unwrap();
	let encoder = config.session_encoder.clone();
	let stores = Stores::with_ipv6_enabled(config.ipv6_enabled);
	let client = Client::new(
		&client::Config {
			resolver_cfg: ResolverConfig::default(),
			resolver_opts: ResolverOpts::default(),
		},
		None,
		BackendConfig::default(),
		None,
	);
	let pi = Arc::new(ProxyInputs {
		cfg: Arc::new(config),
		stores: stores.clone(),
		tracer: None,
		metrics: Arc::new(crate::metrics::Metrics::new(
			metrics::sub_registry(&mut Registry::default()),
			Default::default(),
		)),
		upstream: client.clone(),
		ca: None,

		mcp_state: mcp::router::App::new(stores.clone(), encoder),
	});

	let client = PolicyClient { inputs: pi.clone() };
	// Define a sample tool for testing
	let test_tool_get = Tool {
		name: Cow::Borrowed("get_user"),
		description: Some(Cow::Borrowed("Get user details")), // Added description
		icons: None,
		title: None,
		meta: None,
		execution: None,
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
		allowed_headers: HashSet::from(["X-Request-ID".to_string()]),
	};

	let test_tool_post = Tool {
		name: Cow::Borrowed("create_user"),
		description: Some(Cow::Borrowed("Create a new user")),
		icons: None,
		title: None,
		meta: None,
		execution: None,
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
		allowed_headers: HashSet::from(["X-API-Key".to_string()]),
	};

	let backend = SimpleBackend::Opaque(
		ResourceName::new(strng::literal!("dummy"), "".into()),
		Target::Hostname(
			parsed.host().unwrap().to_string().into(),
			parsed.port().unwrap_or(8080),
		),
	);
	let upstream_client = super::super::McpHttpClient::new(
		client,
		backend,
		BackendPolicies::default(),
		false,
		"test-target".to_string(),
	);
	let handler = Handler::new(
		upstream_client,
		vec![
			(test_tool_get, upstream_call_get),
			(test_tool_post, upstream_call_post),
		],
		"".to_string(),
	);

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
			&IncomingRequestContext::empty(),
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
			&IncomingRequestContext::empty(),
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
			&IncomingRequestContext::empty(),
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
			&IncomingRequestContext::empty(),
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
			&IncomingRequestContext::empty(),
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
			&IncomingRequestContext::empty(),
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
			&IncomingRequestContext::empty(),
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
	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": user_id })))
		.mount(&server)
		.await;

	// Intentionally provide a non-string header value
	let args = json!({
			"path": { "user_id": user_id },
			"header": { "X-Request-ID": 12345 }
	});

	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&IncomingRequestContext::empty(),
		)
		.await;
	assert!(result.is_ok());
	assert_eq!(result.unwrap(), json!({ "id": user_id }));
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
			&IncomingRequestContext::empty(),
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
			&IncomingRequestContext::empty(),
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

#[tokio::test]
async fn test_call_tool_with_compressed_response() {
	let (server, handler) = setup().await;

	let user_id = "compressed-user";
	let expected_response = json!({ "id": user_id, "name": "Compressed User", "data": "This is a longer response that benefits from compression" });

	// Encode the response body with gzip
	let response_json = serde_json::to_vec(&expected_response).unwrap();
	let compressed_body = crate::http::compression::encode_body(&response_json, "gzip")
		.await
		.unwrap();

	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.respond_with(
			ResponseTemplate::new(200)
				.insert_header("Content-Encoding", "gzip")
				.set_body_bytes(compressed_body),
		)
		.mount(&server)
		.await;

	let args = json!({ "path": { "user_id": user_id } });
	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&IncomingRequestContext::empty(),
		)
		.await;

	assert!(result.is_ok());
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_call_tool_response_wrapping() {
	let (server, handler) = setup().await;

	let test_cases = [
		(false, Value::Null),
		(
			false,
			json!({ "id": "123", "name": "Test User", "email": "test@example.com" }),
		),
		(
			true,
			json!([ { "id": 1, "name": "1" }, { "id": 2, "name": "2" }, { "id": 3, "name": "3" }]),
		),
		(true, json!("plain text response")),
		(true, json!(42)),
		(true, json!(true)),
	];

	for (i, (wrapped, response)) in test_cases.iter().enumerate() {
		let user_id = format!("{}", i);
		Mock::given(method("GET"))
			.and(path(format!("/users/{}", user_id)))
			.respond_with(ResponseTemplate::new(200).set_body_json(response))
			.expect(1)
			.mount(&server)
			.await;

		let args = json!({ "path": { "user_id": user_id } });
		let result = handler
			.call_tool(
				"get_user",
				Some(args.as_object().unwrap().clone()),
				&IncomingRequestContext::empty(),
			)
			.await;
		assert!(result.is_ok());

		// Spec requires an object https://modelcontextprotocol.io/specification/2025-06-18/schema#calltoolresult
		let expected = if *wrapped {
			json!({ "data": response })
		} else {
			response.clone()
		};
		assert_eq!(result.unwrap(), expected);
	}
}
#[tokio::test]
async fn test_normalize_url_path_empty_prefix() {
	// Test the fix for double slash issue when prefix is empty (host/port config)
	let result = super::normalize_url_path("", "/mqtt/healthcheck");
	assert_eq!(result, "/mqtt/healthcheck");
}

#[tokio::test]
async fn test_normalize_url_path_with_prefix() {
	// Test with a prefix that has trailing slash
	let result = super::normalize_url_path("/api/v3/", "/pet");
	assert_eq!(result, "/api/v3/pet");
}

#[tokio::test]
async fn test_normalize_url_path_prefix_no_trailing_slash() {
	// Test with a prefix without trailing slash
	let result = super::normalize_url_path("/api/v3", "/pet");
	assert_eq!(result, "/api/v3/pet");
}

#[tokio::test]
async fn test_normalize_url_path_path_without_leading_slash() {
	// Test with path that doesn't start with slash
	let result = super::normalize_url_path("/api/v3", "pet");
	assert_eq!(result, "/api/v3/pet");
}

#[rstest]
#[case::empty_prefix("", "/mqtt/healthcheck", "/mqtt/healthcheck")]
#[case::with_prefix("/api/v3/", "/pet", "/api/v3/pet")]
#[case::prefix_no_trailing_slash("/api/v3", "/pet", "/api/v3/pet")]
#[case::without_leading_slash("/api/v3", "pet", "/api/v3/pet")]
#[case::empty_prefix_path_without_slash("", "pet", "/pet")]
fn test_normalize_url_path(#[case] prefix: &str, #[case] path: &str, #[case] expected: &str) {
	let result = super::normalize_url_path(prefix, path);
	assert_eq!(result, expected);
}

#[rstest]
#[case::empty_string(json!({"verbose": ""}), vec![("verbose", "")])]
#[case::string_value(json!({"verbose": "true"}), vec![("verbose", "true")])]
#[case::boolean_true(json!({"verbose": true}), vec![("verbose", "true")])]
#[case::boolean_false(json!({"verbose": false}), vec![("verbose", "false")])]
#[case::integer_value(json!({"verbose": "123"}), vec![("verbose", "123")])]
#[case::special_chars(json!({"verbose": "hello world"}), vec![("verbose", "hello world")])]
#[case::array_values(json!({"verbose": ["a", "b", "c"]}), vec![("verbose", "a"), ("verbose", "b"), ("verbose", "c")])]
#[case::ampersand_in_value(json!({"verbose": "foo&admin=true"}), vec![("verbose", "foo&admin=true")])]
#[case::equals_in_value(json!({"verbose": "foo=bar"}), vec![("verbose", "foo=bar")])]
#[case::question_mark_in_value(json!({"verbose": "foo?bar"}), vec![("verbose", "foo?bar")])]
#[case::combined_injection(json!({"verbose": "x&evil=1&admin=true"}), vec![("verbose", "x&evil=1&admin=true")])]
#[tokio::test]
async fn test_query_param_types(
	#[case] query_args: serde_json::Value,
	#[case] expected_params: Vec<(&str, &str)>,
) {
	let (server, handler) = setup().await;

	let user_id = "test-user";

	let mut mock = Mock::given(method("GET")).and(path(format!("/users/{user_id}")));
	for (key, value) in &expected_params {
		mock = mock.and(query_param(*key, *value));
	}
	mock
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": user_id })))
		.expect(1)
		.mount(&server)
		.await;

	let args = json!({
		"path": { "user_id": user_id },
		"query": query_args
	});

	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&IncomingRequestContext::empty(),
		)
		.await;

	assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
	assert_eq!(result.unwrap(), json!({ "id": user_id }));
}

#[rstest]
#[case::simple_id("123", "/users/123")]
#[case::numeric_id("456", "/users/456")]
#[case::spaces("user name", "/users/user%20name")]
#[case::unicode("user\u{00e9}", "/users/user%C3%A9")]
#[case::path_traversal("../admin", "/users/..%2Fadmin")]
#[case::embedded_slashes("user-1/o/er-1001", "/users/user-1%2Fo%2Fer-1001")]
#[case::query_injection("123?admin=true", "/users/123%3Fadmin%3Dtrue")]
#[case::query_with_ampersand("123?a=1&b=2", "/users/123%3Fa%3D1%26b%3D2")]
#[case::hash_fragment("user#section", "/users/user%23section")]
#[case::ampersand_in_path("user&admin=true", "/users/user%26admin%3Dtrue")]
#[tokio::test]
async fn test_path_param_encoding(#[case] user_id: &str, #[case] expected_path: &str) {
	let (server, handler) = setup().await;

	Mock::given(method("GET"))
		.and(path(expected_path))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": user_id })))
		.expect(1)
		.mount(&server)
		.await;

	let args = json!({ "path": { "user_id": user_id } });

	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&IncomingRequestContext::empty(),
		)
		.await;

	assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
	assert_eq!(result.unwrap(), json!({ "id": user_id }));
}

#[tokio::test]
async fn test_schema_defined_headers_work() {
	let (server, handler) = setup().await;

	let user_id = "custom-header-test";
	let expected_response = json!({ "id": user_id });

	// Only X-Request-ID is defined in the schema for get_user tool
	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.and(header("X-Request-ID", "my-request-123"))
		.respond_with(ResponseTemplate::new(200).set_body_json(&expected_response))
		.expect(1)
		.mount(&server)
		.await;

	let args = json!({
		"path": { "user_id": user_id },
		"header": {
			"X-Request-ID": "my-request-123"
		}
	});

	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&IncomingRequestContext::empty(),
		)
		.await;

	assert!(
		result.is_ok(),
		"Schema-defined headers should work: {:?}",
		result.err()
	);
	assert_eq!(result.unwrap(), expected_response);
}

// Custom matcher to verify a header is NOT present
struct HeaderNotPresent {
	header_name: String,
}

impl HeaderNotPresent {
	fn new(header_name: impl Into<String>) -> Self {
		Self {
			header_name: header_name.into(),
		}
	}
}

impl Match for HeaderNotPresent {
	fn matches(&self, request: &Request) -> bool {
		!request.headers.contains_key(self.header_name.as_str())
	}
}

#[tokio::test]
async fn test_blocked_headers_are_ignored() {
	let (server, handler) = setup().await;

	let request_body = json!({ "name": "Test User", "email": "test@example.com" });
	let expected_response = json!({ "id": "new-user", "name": "Test User" });

	Mock::given(method("POST"))
		.and(path("/users"))
		.and(header("content-length", "47")) // length of request_body
		.and(header("content-type", "application/json"))
		.and(HeaderNotPresent::new("transfer-encoding"))
		.and(body_json(&request_body))
		.respond_with(ResponseTemplate::new(201).set_body_json(&expected_response))
		.expect(1)
		.mount(&server)
		.await;

	let args = json!({
		"body": request_body,
		"header": {
			"content-length": "999999999",
			"content-type": "text/plain",
			"transfer-encoding": "chunked",
			"host": "evil.com"
		}
	});

	let result = handler
		.call_tool(
			"create_user",
			Some(args.as_object().unwrap().clone()),
			&IncomingRequestContext::empty(),
		)
		.await;

	// The request should succeed with the correct headers (blocked headers ignored)
	assert!(result.is_ok(), "Request should succeed: {:?}", result.err());
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_headers_not_in_schema_are_ignored() {
	let (server, handler) = setup().await;

	let user_id = "schema-header-test";
	let expected_response = json!({ "id": user_id });

	// Only expect X-Request-ID (defined in schema), NOT X-Malicious-Header
	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.and(header("X-Request-ID", "valid-request"))
		.and(HeaderNotPresent::new("X-Malicious-Header"))
		.respond_with(ResponseTemplate::new(200).set_body_json(&expected_response))
		.expect(1)
		.mount(&server)
		.await;

	let args = json!({
		"path": { "user_id": user_id },
		"header": {
			"X-Request-ID": "valid-request",
			"X-Malicious-Header": "should-be-ignored"
		}
	});

	let result = handler
		.call_tool(
			"get_user",
			Some(args.as_object().unwrap().clone()),
			&IncomingRequestContext::empty(),
		)
		.await;

	assert!(
		result.is_ok(),
		"Request should succeed with schema-defined headers: {:?}",
		result.err()
	);
	assert_eq!(result.unwrap(), expected_response);
}

#[tokio::test]
async fn test_call_tool_structured_content_fallback() {
	// Test that CallToolResult has both content and structured_content populated
	// This verifies our backwards compatibility fix for langchain-mcp-adapter
	let (server, handler) = setup().await;

	let user_id = "test123";
	let expected_response = json!({ "id": user_id, "name": "Test User", "status": "active" });

	Mock::given(method("GET"))
		.and(path(format!("/users/{user_id}")))
		.respond_with(ResponseTemplate::new(200).set_body_json(&expected_response))
		.mount(&server)
		.await;

	// Test the MCP message handling directly
	use rmcp::model::*;

	let request = JsonRpcRequest {
		jsonrpc: JsonRpcVersion2_0,
		id: RequestId::String("test-123".into()),
		request: ClientRequest::CallToolRequest(CallToolRequest {
			method: CallToolRequestMethod,
			params: CallToolRequestParams {
				meta: None,
				task: None,
				name: "get_user".into(),
				arguments: Some(
					json!({ "path": { "user_id": user_id } })
						.as_object()
						.unwrap()
						.clone(),
				),
			},
			extensions: Extensions::default(),
		}),
	};

	let result = handler
		.send_message(request, &IncomingRequestContext::empty())
		.await;
	assert!(result.is_ok(), "send_message should succeed");

	let messages = result.unwrap();

	// Convert Messages to Vec to inspect CallToolResult
	use futures_util::StreamExt;
	let mut message_stream = messages;
	let message_result = message_stream
		.next()
		.await
		.expect("Should receive at least one message");

	// Handle the Result<JsonRpcMessage, ClientError> wrapper
	let server_msg = message_result.expect("Message processing should succeed");

	let JsonRpcMessage::Response(response) = server_msg else {
		panic!("Should receive a Response message, got: {:?}", server_msg);
	};

	let ServerResult::CallToolResult(call_result) = &response.result else {
		panic!(
			"Response should contain CallToolResult, got: {:?}",
			response.result
		);
	};

	// Test 1: content field should NOT be empty (our backwards compatibility fix)
	assert!(
		!call_result.content.is_empty(),
		"content field should not be empty after our fix"
	);
	assert_eq!(
		call_result.content.len(),
		1,
		"content should have exactly one item"
	);

	// Test 2: structured_content should contain the original JSON
	assert!(
		call_result.structured_content.is_some(),
		"structured_content should be populated"
	);
	let structured_content = call_result.structured_content.as_ref().unwrap();
	assert_eq!(
		*structured_content, expected_response,
		"structured_content should contain original API response"
	);

	// Test 3: content[0] should contain serialized JSON as text
	let content_item = call_result
		.content
		.first()
		.expect("content should have at least one item");

	let RawContent::Text(text_content) = &content_item.raw else {
		panic!(
			"content[0] should be Text content type, got: {:?}",
			content_item.raw
		);
	};

	let serialized_json = &text_content.text;
	let parsed_from_content: serde_json::Value =
		serde_json::from_str(serialized_json).expect("content should contain valid JSON string");
	assert_eq!(
		parsed_from_content, expected_response,
		"content should contain serialized version of API response"
	);

	// Test 4: Both fields should represent the same data
	assert_eq!(
		parsed_from_content, *structured_content,
		"content and structured_content should represent the same data"
	);
}

#[tokio::test]
async fn test_openapi_from_url() {
	// Test creating LocalMcpTargetSpec::OpenAPI with URL schema and converting to runtime backend
	let server = MockServer::start().await;

	// Mock OpenAPI schema response
	let openapi_json = json!({
		"openapi": "3.0.0",
		"info": {
			"title": "User API",
			"version": "1.0.0"
		},
		"paths": {
			"/users/{user_id}": {
				"get": {
					"summary": "Get user details",
					"parameters": [
						{
							"name": "user_id",
							"in": "path",
							"required": true,
							"schema": {
								"type": "string"
							}
						}
					],
					"responses": {
						"200": {
							"description": "User details",
							"content": {
								"application/json": {
									"schema": {
										"type": "object",
										"properties": {
											"id": {"type": "string"},
											"name": {"type": "string"}
										}
									}
								}
							}
						}
					}
				}
			},
			"/users": {
				"post": {
					"summary": "Create a new user",
					"requestBody": {
						"required": true,
						"content": {
							"application/json": {
								"schema": {
									"type": "object",
									"properties": {
										"name": {"type": "string"},
										"email": {"type": "string"}
									},
									"required": ["name", "email"]
								}
							}
						}
					},
					"responses": {
						"201": {
							"description": "User created",
							"content": {
								"application/json": {
									"schema": {
										"type": "object",
										"properties": {
											"id": {"type": "string"},
											"name": {"type": "string"},
											"email": {"type": "string"}
										}
									}
								}
							}
						}
					}
				}
			}
		}
	});

	Mock::given(method("GET"))
		.and(path("/openapi.json"))
		.respond_with(ResponseTemplate::new(200).set_body_json(&openapi_json))
		.mount(&server)
		.await;

	// Create client
	let client = Client::new(
		&client::Config {
			resolver_cfg: ResolverConfig::default(),
			resolver_opts: ResolverOpts::default(),
		},
		None,
		BackendConfig::default(),
		None,
	);

	// Create LocalMcpTargetSpec::OpenAPI with remote URL schema
	let schema_url = format!("{}/openapi.json", server.uri());

	// Use serde_json to create McpBackendHost since fields are private
	let backend_json = json!({
		"host": "https://api.users.com"
	});
	let backend: McpBackendHost = serde_json::from_value(backend_json).unwrap();

	let local_target_spec = LocalMcpTargetSpec::OpenAPI {
		backend,
		schema: FileInlineOrRemote::Remote {
			url: schema_url.parse().unwrap(),
		},
	};

	// Create a LocalBackend::MCP to test the full conversion pipeline
	let local_backend = LocalBackend::MCP(LocalMcpBackend {
		targets: vec![Arc::new(LocalMcpTarget {
			name: "users-api".into(),
			spec: local_target_spec,
			policies: None,
		})],
		stateful_mode: McpStatefulMode::Stateful,
		prefix_mode: None,
		allow_degraded: false,
	});

	// Convert to runtime backends
	let backend_name = ResourceName::new("test-users".into(), "".into());
	let result = local_backend.as_backends(backend_name, client).await;

	// Verify the conversion succeeded
	assert!(
		result.is_ok(),
		"Should successfully convert LocalBackend::MCP with remote OpenAPI schema"
	);
	let backends = result.unwrap();

	// Should have at least one backend
	assert!(!backends.is_empty(), "Should have at least one backend");

	// Find the MCP backend
	let mcp_backend = backends
		.iter()
		.find(|b| matches!(b.backend, Backend::MCP(_, _)));
	assert!(mcp_backend.is_some(), "Should contain an MCP backend");

	if let Some(backend_with_policies) = mcp_backend
		&& let Backend::MCP(_, mcp_backend) = &backend_with_policies.backend
	{
		assert_eq!(mcp_backend.targets.len(), 1);

		// Verify the target was converted to OpenAPI
		let target = &mcp_backend.targets[0];
		assert_eq!(target.name.as_str(), "users-api");

		// Verify it's an OpenAPI target spec with the fetched schema
		if let crate::types::agent::McpTargetSpec::OpenAPI(openapi_target) = &target.spec {
			let schema = &openapi_target.schema;
			assert_eq!(schema.openapi, "3.0.0");
			assert_eq!(schema.info.title, "User API");
			assert_eq!(schema.info.version, "1.0.0");

			// Check if paths contains the expected paths
			let has_users_path = schema.paths.paths.contains_key("/users");
			assert!(has_users_path, "Schema should contain /users path");

			let has_users_id_path = schema.paths.paths.contains_key("/users/{user_id}");
			assert!(
				has_users_id_path,
				"Schema should contain /users/{{user_id}} path"
			);

			// Verify the path details were preserved
			if let Some(path_item_ref) = schema.paths.paths.get("/users/{user_id}") {
				match path_item_ref {
					ReferenceOr::Item(path_item) => {
						if let Some(get_op) = &path_item.get {
							assert_eq!(get_op.summary.as_deref(), Some("Get user details"));
						}
					},
					ReferenceOr::Reference { reference: _ } => {
						panic!("Expected path item, got reference");
					},
				}
			}

			if let Some(path_item_ref) = schema.paths.paths.get("/users") {
				match path_item_ref {
					ReferenceOr::Item(path_item) => {
						if let Some(post_op) = &path_item.post {
							assert_eq!(post_op.summary.as_deref(), Some("Create a new user"));
						}
					},
					ReferenceOr::Reference { reference: _ } => {
						panic!("Expected path item, got reference");
					},
				}
			}
		} else {
			panic!("Expected OpenAPI target spec, got {:?}", target.spec);
		}
	}
}
