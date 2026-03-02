use std::sync::Arc;

use ::http::HeaderMap;

use super::*;
use crate::cel;
use crate::http::ext_authz::ExtAuthzDynamicMetadata;
use crate::http::localratelimit::RateLimitType;
use crate::http::remoteratelimit::{
	Descriptor, DescriptorEntry, DescriptorSet, FailureMode, RemoteRateLimit,
};
use crate::mcp::{ResourceId, ResourceType};
use crate::types::agent::SimpleBackendReference;

/// Helper: build a `McpRemoteRateLimit` with the given descriptor entries.
fn make_rate_limit(descriptor_entries: Vec<DescriptorEntry>) -> McpRemoteRateLimit {
	make_rate_limit_with_failure_mode(descriptor_entries, FailureMode::default())
}

fn make_rate_limit_with_failure_mode(
	descriptor_entries: Vec<DescriptorEntry>,
	failure_mode: FailureMode,
) -> McpRemoteRateLimit {
	McpRemoteRateLimit(RemoteRateLimit {
		domain: "test-domain".to_string(),
		target: Arc::new(SimpleBackendReference::Invalid),
		descriptors: Arc::new(DescriptorSet(descriptor_entries)),
		timeout: None,
		failure_mode,
	})
}

/// Helper: build a `DescriptorEntry` from a list of (key, cel_expression) pairs.
fn make_descriptor_entry(entries: Vec<(&str, &str)>, limit_type: RateLimitType) -> DescriptorEntry {
	let descriptors: Vec<Descriptor> = entries
		.into_iter()
		.map(|(key, expr)| {
			Descriptor(
				key.to_string(),
				cel::Expression::new_strict(expr).expect("valid CEL expression"),
			)
		})
		.collect();
	DescriptorEntry {
		entries: Arc::new(descriptors),
		limit_type,
	}
}

// --- McpRateLimitError tests ---

#[test]
fn mcp_rate_limit_error_display() {
	let err = McpRateLimitError::default();
	assert_eq!(err.to_string(), "mcp rate limit exceeded");
}

#[test]
fn mcp_rate_limit_error_default_has_empty_headers() {
	let err = McpRateLimitError::default();
	assert!(err.response_headers.is_empty());
}

#[test]
fn mcp_rate_limit_error_carries_headers() {
	let mut headers = HeaderMap::new();
	headers.insert("x-ratelimit-limit", "100".parse().unwrap());
	headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
	let err = McpRateLimitError {
		response_headers: headers,
		service_error: false,
	};
	assert_eq!(err.response_headers.len(), 2);
	assert_eq!(
		err.response_headers.get("x-ratelimit-limit").unwrap(),
		"100"
	);
	assert_eq!(
		err.response_headers.get("x-ratelimit-remaining").unwrap(),
		"0"
	);
}

#[test]
fn mcp_rate_limit_error_default_is_not_service_error() {
	let err = McpRateLimitError::default();
	assert!(!err.service_error);
}

#[test]
fn mcp_rate_limit_error_service_error_flag() {
	let err = McpRateLimitError {
		response_headers: HeaderMap::new(),
		service_error: true,
	};
	assert!(err.service_error);
	assert!(err.response_headers.is_empty());
}

// --- build_request_from_snapshot tests ---

fn make_snapshot(headers: Vec<(&str, &str)>) -> cel::RequestSnapshot {
	let mut header_map = ::http::HeaderMap::new();
	for (k, v) in headers {
		header_map.insert(
			::http::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
			::http::HeaderValue::from_str(v).unwrap(),
		);
	}
	cel::RequestSnapshot {
		method: ::http::Method::POST,
		path: "/mcp".parse().unwrap(),
		host: None,
		scheme: None,
		version: ::http::Version::HTTP_11,
		headers: header_map,
		body: None,
		jwt: None,
		api_key: None,
		basic_auth: None,
		backend: None,
		source: None,
		start_time: None,
		extauthz: None,
		extproc: None,
		llm: None,
	}
}

#[test]
fn build_request_from_snapshot_all_descriptors_evaluate() {
	let entry = make_descriptor_entry(
		vec![("user", r#""test-user""#), ("tool", r#""echo""#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);

	let snapshot = make_snapshot(vec![]);
	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected Some when all literal descriptors evaluate via snapshot"
	);
	let request = result.unwrap();
	assert_eq!(request.descriptors.len(), 1);
	assert_eq!(request.descriptors[0].entries.len(), 2);
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "test-user");
	assert_eq!(request.descriptors[0].entries[1].key, "tool");
	assert_eq!(request.descriptors[0].entries[1].value, "echo");
}

#[test]
fn build_request_from_snapshot_header_descriptor_evaluates() {
	let entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-client-id"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);

	let snapshot = make_snapshot(vec![("x-client-id", "my-client")]);
	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(result.is_some());
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].value, "my-client");
}

#[test]
fn build_request_from_snapshot_missing_header_returns_none() {
	let entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-missing-header"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);

	let snapshot = make_snapshot(vec![]);
	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when descriptor references missing header in snapshot"
	);
}

#[test]
fn build_request_from_snapshot_no_matching_type_returns_none() {
	let entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Tokens);
	let rl = make_rate_limit(vec![entry]);

	let snapshot = make_snapshot(vec![]);
	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when no candidates match the requested type via snapshot"
	);
}

#[test]
fn build_request_from_snapshot_cost_propagated() {
	let entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Tokens);
	let rl = make_rate_limit(vec![entry]);

	let snapshot = make_snapshot(vec![]);
	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Tokens, Some(42));
	assert!(result.is_some());
	assert_eq!(result.unwrap().descriptors[0].hits_addend, Some(42));
}

#[test]
fn build_request_from_snapshot_partial_descriptor_failure_sends_successful_only() {
	let good_entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Requests);
	let bad_entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-missing"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![good_entry, bad_entry]);

	let snapshot = make_snapshot(vec![]);
	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(result.is_some());
	let request = result.unwrap();
	assert_eq!(request.descriptors.len(), 1);
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "test-user");
}

#[test]
fn build_request_from_snapshot_all_descriptors_fail_returns_none() {
	let bad_entry1 = make_descriptor_entry(
		vec![("a", r#"request.headers["x-missing-1"]"#)],
		RateLimitType::Requests,
	);
	let bad_entry2 = make_descriptor_entry(
		vec![("b", r#"request.headers["x-missing-2"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![bad_entry1, bad_entry2]);

	let snapshot = make_snapshot(vec![]);
	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when all descriptor entries fail via snapshot"
	);
}

// --- MCP resource type tests ---

fn make_tool_resource(target: &str, name: &str) -> ResourceType {
	ResourceType::Tool(ResourceId::new(target.to_string(), name.to_string()))
}

fn make_prompt_resource(target: &str, name: &str) -> ResourceType {
	ResourceType::Prompt(ResourceId::new(target.to_string(), name.to_string()))
}

fn make_resource_resource(target: &str, name: &str) -> ResourceType {
	ResourceType::Resource(ResourceId::new(target.to_string(), name.to_string()))
}

#[test]
fn build_request_from_snapshot_mcp_tool_name_resolves() {
	let entry = make_descriptor_entry(vec![("tool", "mcp.tool.name")], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![]);
	let resource = make_tool_resource("my-service", "echo");

	let result =
		rl.build_request_from_snapshot(&snapshot, Some(&resource), RateLimitType::Requests, None);
	assert!(result.is_some(), "expected mcp.tool.name to resolve");
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].key, "tool");
	assert_eq!(request.descriptors[0].entries[0].value, "echo");
}

#[test]
fn build_request_from_snapshot_mcp_tool_target_resolves() {
	let entry = make_descriptor_entry(vec![("target", "mcp.tool.target")], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![]);
	let resource = make_tool_resource("my-service", "echo");

	let result =
		rl.build_request_from_snapshot(&snapshot, Some(&resource), RateLimitType::Requests, None);
	assert!(result.is_some(), "expected mcp.tool.target to resolve");
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].value, "my-service");
}

#[test]
fn build_request_from_snapshot_mcp_prompt_name_resolves() {
	let entry = make_descriptor_entry(vec![("prompt", "mcp.prompt.name")], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![]);
	let resource = make_prompt_resource("my-service", "greeting");

	let result =
		rl.build_request_from_snapshot(&snapshot, Some(&resource), RateLimitType::Requests, None);
	assert!(result.is_some(), "expected mcp.prompt.name to resolve");
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].value, "greeting");
}

#[test]
fn build_request_from_snapshot_mcp_resource_name_resolves() {
	let entry = make_descriptor_entry(
		vec![("resource", "mcp.resource.name")],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![]);
	let resource = make_resource_resource("my-service", "file:///data.txt");

	let result =
		rl.build_request_from_snapshot(&snapshot, Some(&resource), RateLimitType::Requests, None);
	assert!(result.is_some(), "expected mcp.resource.name to resolve");
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].value, "file:///data.txt");
}

#[test]
fn build_request_from_snapshot_mcp_var_fails_without_resource() {
	let entry = make_descriptor_entry(vec![("tool", "mcp.tool.name")], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![]);

	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when mcp.tool.name is used without a ResourceType"
	);
}

#[test]
fn build_request_from_snapshot_mcp_wrong_resource_type_fails() {
	let entry = make_descriptor_entry(vec![("tool", "mcp.tool.name")], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![]);
	let resource = make_prompt_resource("my-service", "greeting");

	let result =
		rl.build_request_from_snapshot(&snapshot, Some(&resource), RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when mcp.tool.name is used with a Prompt resource"
	);
}

#[test]
fn build_request_from_snapshot_mcp_and_snapshot_vars_together() {
	let entry = make_descriptor_entry(
		vec![
			("tool", "mcp.tool.name"),
			("client", r#"request.headers["x-client-id"]"#),
		],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![("x-client-id", "agent-123")]);
	let resource = make_tool_resource("my-service", "echo");

	let result =
		rl.build_request_from_snapshot(&snapshot, Some(&resource), RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected both mcp and request vars to resolve"
	);
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].key, "tool");
	assert_eq!(request.descriptors[0].entries[0].value, "echo");
	assert_eq!(request.descriptors[0].entries[1].key, "client");
	assert_eq!(request.descriptors[0].entries[1].value, "agent-123");
}

// --- extauthz tests ---

fn make_extauthz(entries: Vec<(&str, serde_json::Value)>) -> ExtAuthzDynamicMetadata {
	let map: serde_json::Map<String, serde_json::Value> = entries
		.into_iter()
		.map(|(k, v)| (k.to_string(), v))
		.collect();
	serde_json::from_value(serde_json::Value::Object(map)).expect("valid ExtAuthzDynamicMetadata")
}

fn make_snapshot_with_extauthz(
	headers: Vec<(&str, &str)>,
	extauthz: ExtAuthzDynamicMetadata,
) -> cel::RequestSnapshot {
	let mut snapshot = make_snapshot(headers);
	snapshot.extauthz = Some(extauthz);
	snapshot
}

#[test]
fn build_request_from_snapshot_extauthz_string_resolves() {
	let entry = make_descriptor_entry(
		vec![("user", r#"extauthz.user_id"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);
	let extauthz = make_extauthz(vec![("user_id", serde_json::Value::String("alice".into()))]);
	let snapshot = make_snapshot_with_extauthz(vec![], extauthz);

	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(result.is_some(), "expected extauthz.user_id to resolve");
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "alice");
}

#[test]
fn build_request_from_snapshot_extauthz_missing_key_fails() {
	let entry = make_descriptor_entry(
		vec![("user", "extauthz.nonexistent")],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);
	let extauthz = make_extauthz(vec![("user_id", serde_json::Value::String("alice".into()))]);
	let snapshot = make_snapshot_with_extauthz(vec![], extauthz);

	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when extauthz key does not exist"
	);
}

#[test]
fn build_request_from_snapshot_extauthz_without_metadata_fails() {
	let entry = make_descriptor_entry(vec![("user", "extauthz.user_id")], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);
	let snapshot = make_snapshot(vec![]);

	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when extauthz is not present in snapshot"
	);
}

#[test]
fn build_request_from_snapshot_extauthz_and_header_together() {
	let entry = make_descriptor_entry(
		vec![
			("user", "extauthz.user_id"),
			("client", r#"request.headers["x-client-id"]"#),
		],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);
	let extauthz = make_extauthz(vec![("user_id", serde_json::Value::String("bob".into()))]);
	let snapshot = make_snapshot_with_extauthz(vec![("x-client-id", "agent-456")], extauthz);

	let result = rl.build_request_from_snapshot(&snapshot, None, RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected both extauthz and header to resolve"
	);
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "bob");
	assert_eq!(request.descriptors[0].entries[1].key, "client");
	assert_eq!(request.descriptors[0].entries[1].value, "agent-456");
}

#[test]
fn build_request_from_snapshot_extauthz_and_mcp_together() {
	let entry = make_descriptor_entry(
		vec![("user", "extauthz.user_id"), ("tool", "mcp.tool.name")],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);
	let extauthz = make_extauthz(vec![("user_id", serde_json::Value::String("carol".into()))]);
	let snapshot = make_snapshot_with_extauthz(vec![], extauthz);
	let resource = make_tool_resource("my-service", "search");

	let result =
		rl.build_request_from_snapshot(&snapshot, Some(&resource), RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected both extauthz and mcp vars to resolve"
	);
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "carol");
	assert_eq!(request.descriptors[0].entries[1].key, "tool");
	assert_eq!(request.descriptors[0].entries[1].value, "search");
}

// --- handle_service_error tests ---

#[test]
fn handle_service_error_fail_open_returns_ok() {
	let rl = make_rate_limit_with_failure_mode(vec![], FailureMode::FailOpen);
	let result = rl.handle_service_error();
	assert!(
		result.is_ok(),
		"failOpen should allow request through when service is unreachable"
	);
	assert!(result.unwrap().is_empty());
}

#[test]
fn handle_service_error_fail_closed_returns_err_with_service_error_flag() {
	let rl = make_rate_limit_with_failure_mode(vec![], FailureMode::FailClosed);
	let result = rl.handle_service_error();
	assert!(
		result.is_err(),
		"failClosed should deny request when service is unreachable"
	);
	let err = result.unwrap_err();
	assert!(
		err.service_error,
		"service_error flag should be set so handler maps to 500, not 429"
	);
	assert!(
		err.response_headers.is_empty(),
		"no response headers expected from unreachable service"
	);
}
