use std::sync::Arc;

use super::*;
use crate::cel;
use crate::http::jwt;
use crate::http::localratelimit::RateLimitType;

/// Helper: build a `RemoteRateLimit` with the given descriptor entries.
fn make_rate_limit(descriptor_entries: Vec<DescriptorEntry>) -> RemoteRateLimit {
	RemoteRateLimit {
		domain: "test-domain".to_string(),
		target: Arc::new(SimpleBackendReference::Invalid),
		descriptors: Arc::new(DescriptorSet(descriptor_entries)),
		timeout: None,
		failure_mode: FailureMode::default(),
	}
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

// --- build_request tests ---

/// When all descriptor CEL expressions evaluate successfully against the request,
/// `build_request` should return `Some` with all descriptors populated.
#[test]
fn build_request_all_descriptors_evaluate_returns_some() {
	let entry = make_descriptor_entry(
		vec![("user", r#""test-user""#), ("tool", r#""echo""#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected Some when all descriptors evaluate"
	);
	let request = result.unwrap();
	assert_eq!(request.descriptors.len(), 1);
	assert_eq!(request.descriptors[0].entries.len(), 2);
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "test-user");
	assert_eq!(request.descriptors[0].entries[1].key, "tool");
	assert_eq!(request.descriptors[0].entries[1].value, "echo");
}

/// When a descriptor references a request header that exists,
/// it should evaluate successfully.
#[test]
fn build_request_header_descriptor_evaluates() {
	let entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-client-id"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.header("x-client-id", "my-client")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(result.is_some());
	let request = result.unwrap();
	assert_eq!(request.descriptors[0].entries[0].value, "my-client");
}

/// When a descriptor references a request header that does NOT exist,
/// evaluation should fail and `build_request` should return `None`.
#[test]
fn build_request_missing_header_returns_none() {
	let entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-missing-header"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);

	// Request without the expected header
	let req = ::http::Request::builder()
		.method(::http::Method::DELETE)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when descriptor evaluation fails"
	);
}

/// When there are multiple descriptor entries and the second one fails,
/// `build_request` should drop the failed descriptor and return `Some`
/// with only the successful one (matching Envoy's per-descriptor semantics).
#[test]
fn build_request_second_descriptor_fails_sends_successful_only() {
	let good_entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Requests);
	let bad_entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-missing"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![good_entry, bad_entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected Some with the successful descriptor when only one fails"
	);
	let request = result.unwrap();
	assert_eq!(
		request.descriptors.len(),
		1,
		"only the successful descriptor should be sent"
	);
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "test-user");
}

/// When the first descriptor fails but the second succeeds,
/// `build_request` should drop the failed one and return `Some`
/// with only the successful descriptor.
#[test]
fn build_request_first_descriptor_fails_sends_successful_only() {
	let bad_entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-missing"]"#)],
		RateLimitType::Requests,
	);
	let good_entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Requests);
	let rl = make_rate_limit(vec![bad_entry, good_entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected Some with the successful descriptor when only the first fails"
	);
	let request = result.unwrap();
	assert_eq!(
		request.descriptors.len(),
		1,
		"only the successful descriptor should be sent"
	);
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "test-user");
}

/// When no descriptors match the requested `limit_type`,
/// `build_request` returns `None` since there is nothing to send.
/// (Callers also guard against this before calling `build_request`.)
#[test]
fn build_request_no_matching_type_returns_none() {
	// Configure only Token-type descriptors
	let entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Tokens);
	let rl = make_rate_limit(vec![entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	// Ask for Requests type -- no candidates
	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when no candidates match the requested type"
	);
}

/// The `cost` parameter should be propagated to `hits_addend` on each descriptor.
#[test]
fn build_request_cost_propagated_to_hits_addend() {
	let entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Tokens);
	let rl = make_rate_limit(vec![entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Tokens, Some(42));
	assert!(result.is_some());
	assert_eq!(result.unwrap().descriptors[0].hits_addend, Some(42));
}

/// Simulates the DELETE disconnect scenario: a DELETE request with no body
/// and a descriptor that references a header not present on the request.
#[test]
fn build_request_delete_disconnect_skips_ratelimit() {
	let entry = make_descriptor_entry(
		vec![
			("user", r#"request.headers["x-user-id"]"#),
			("tool", r#"request.headers["x-tool"]"#),
		],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![entry]);

	// DELETE request with no custom headers (typical session disconnect)
	let req = ::http::Request::builder()
		.method(::http::Method::DELETE)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None for DELETE disconnect with missing descriptor headers"
	);
}

/// When multiple descriptor entries all evaluate successfully,
/// all of them should appear in the returned request.
#[test]
fn build_request_multiple_entries_all_succeed() {
	let entry1 = make_descriptor_entry(vec![("user", r#""alice""#)], RateLimitType::Requests);
	let entry2 = make_descriptor_entry(vec![("tool", r#""echo""#)], RateLimitType::Requests);
	let entry3 = make_descriptor_entry(vec![("env", r#""prod""#)], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry1, entry2, entry3]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(result.is_some());
	let request = result.unwrap();
	assert_eq!(request.descriptors.len(), 3);
	assert_eq!(request.descriptors[0].entries[0].value, "alice");
	assert_eq!(request.descriptors[1].entries[0].value, "echo");
	assert_eq!(request.descriptors[2].entries[0].value, "prod");
}

/// The Tokens limit type follows the same behavior: when a descriptor
/// fails to evaluate, `build_request` returns `None`.
#[test]
fn build_request_tokens_type_missing_header_returns_none() {
	let entry = make_descriptor_entry(
		vec![("client", r#"request.headers["x-client-id"]"#)],
		RateLimitType::Tokens,
	);
	let rl = make_rate_limit(vec![entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Tokens, Some(100));
	assert!(
		result.is_none(),
		"expected None for Tokens type when descriptor fails"
	);
}

/// The Tokens limit type returns `Some` when all descriptors evaluate.
#[test]
fn build_request_tokens_type_all_succeed() {
	let entry = make_descriptor_entry(vec![("user", r#""test-user""#)], RateLimitType::Tokens);
	let rl = make_rate_limit(vec![entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Tokens, Some(50));
	assert!(result.is_some());
	let request = result.unwrap();
	assert_eq!(request.descriptors.len(), 1);
	assert_eq!(request.descriptors[0].entries[0].value, "test-user");
	assert_eq!(request.descriptors[0].hits_addend, Some(50));
}

/// When ALL descriptor entries fail evaluation, `build_request` returns `None`
/// since there is nothing to send to the rate-limit service.
#[test]
fn build_request_all_descriptors_fail_returns_none() {
	let bad_entry1 = make_descriptor_entry(
		vec![("client", r#"request.headers["x-missing-1"]"#)],
		RateLimitType::Requests,
	);
	let bad_entry2 = make_descriptor_entry(
		vec![("user", r#"request.headers["x-missing-2"]"#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![bad_entry1, bad_entry2]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when all descriptor entries fail evaluation"
	);
}

// --- Multiple descriptors (multiple `- entries:` blocks) with multi-entry keys ---

/// Two descriptors each with multiple entries, all evaluate successfully.
/// Both descriptors should appear in the gRPC request.
///
/// Config equivalent:
/// ```yaml
/// descriptors:
///   - entries:
///       - key: path
///         value: '"literal-path"'
///       - key: remote_address
///         value: 'request.headers["x-forwarded-for"]'
///     type: requests
///   - entries:
///       - key: method
///         value: '"POST"'
///       - key: user
///         value: 'request.headers["x-user-id"]'
///     type: requests
/// ```
#[test]
fn build_request_two_descriptors_multi_entry_all_succeed() {
	let desc1 = make_descriptor_entry(
		vec![
			("path", r#""literal-path""#),
			("remote_address", r#"request.headers["x-forwarded-for"]"#),
		],
		RateLimitType::Requests,
	);
	let desc2 = make_descriptor_entry(
		vec![
			("method", r#""POST""#),
			("user", r#"request.headers["x-user-id"]"#),
		],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![desc1, desc2]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.header("x-forwarded-for", "10.0.0.1")
		.header("x-user-id", "alice")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(result.is_some());
	let request = result.unwrap();
	assert_eq!(request.descriptors.len(), 2);
	// First descriptor: path + remote_address
	assert_eq!(request.descriptors[0].entries.len(), 2);
	assert_eq!(request.descriptors[0].entries[0].key, "path");
	assert_eq!(request.descriptors[0].entries[0].value, "literal-path");
	assert_eq!(request.descriptors[0].entries[1].key, "remote_address");
	assert_eq!(request.descriptors[0].entries[1].value, "10.0.0.1");
	// Second descriptor: method + user
	assert_eq!(request.descriptors[1].entries.len(), 2);
	assert_eq!(request.descriptors[1].entries[0].key, "method");
	assert_eq!(request.descriptors[1].entries[0].value, "POST");
	assert_eq!(request.descriptors[1].entries[1].key, "user");
	assert_eq!(request.descriptors[1].entries[1].value, "alice");
}

/// Two descriptors with multiple entries each. The first descriptor has a
/// missing header causing it to fail; the second succeeds.
/// Only the second descriptor should be sent (Envoy per-descriptor drop).
#[test]
fn build_request_two_descriptors_first_partially_fails_sends_second() {
	// First descriptor: "path" succeeds but "remote_address" references a missing header
	let desc1 = make_descriptor_entry(
		vec![
			("path", r#""literal-path""#),
			("remote_address", r#"request.headers["x-forwarded-for"]"#),
		],
		RateLimitType::Requests,
	);
	// Second descriptor: both entries are literals, always succeed
	let desc2 = make_descriptor_entry(
		vec![("method", r#""POST""#), ("env", r#""prod""#)],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![desc1, desc2]);

	// Request WITHOUT x-forwarded-for → first descriptor fails
	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected Some — second descriptor should still be sent"
	);
	let request = result.unwrap();
	assert_eq!(
		request.descriptors.len(),
		1,
		"only the second descriptor should be sent"
	);
	assert_eq!(request.descriptors[0].entries[0].key, "method");
	assert_eq!(request.descriptors[0].entries[0].value, "POST");
	assert_eq!(request.descriptors[0].entries[1].key, "env");
	assert_eq!(request.descriptors[0].entries[1].value, "prod");
}

/// Two descriptors with multiple entries each. Both have at least one entry
/// referencing a missing header, so both fail. `build_request` returns `None`.
#[test]
fn build_request_two_descriptors_both_partially_fail_returns_none() {
	let desc1 = make_descriptor_entry(
		vec![
			("path", r#""literal-path""#),
			("remote_address", r#"request.headers["x-forwarded-for"]"#),
		],
		RateLimitType::Requests,
	);
	let desc2 = make_descriptor_entry(
		vec![
			("method", r#""POST""#),
			("user", r#"request.headers["x-user-id"]"#),
		],
		RateLimitType::Requests,
	);
	let rl = make_rate_limit(vec![desc1, desc2]);

	// Request without either header → both descriptors fail
	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when all descriptors have at least one failing entry"
	);
}

/// When a CEL expression evaluates successfully but returns a non-string value
/// (e.g., a map), `value_as_string` returns None, causing the descriptor to fail
/// and `build_request` to return `None`.
#[test]
fn build_request_non_string_cel_result_returns_none() {
	// `{"a": "b"}` evaluates to a map, which is not convertible to a string
	let entry = make_descriptor_entry(vec![("data", r#"{"a": "b"}"#)], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);

	let req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_none(),
		"expected None when CEL result is not string-convertible"
	);
}

// --- FailureMode tests ---

#[test]
fn failure_mode_defaults_to_fail_closed() {
	let mode = FailureMode::default();
	assert_eq!(mode, FailureMode::FailClosed);
}

#[test]
fn failure_mode_serde_roundtrip() {
	// Test failOpen
	let json = serde_json::to_string(&FailureMode::FailOpen).unwrap();
	assert_eq!(json, r#""failOpen""#);
	let deserialized: FailureMode = serde_json::from_str(&json).unwrap();
	assert_eq!(deserialized, FailureMode::FailOpen);

	// Test failClosed
	let json = serde_json::to_string(&FailureMode::FailClosed).unwrap();
	assert_eq!(json, r#""failClosed""#);
	let deserialized: FailureMode = serde_json::from_str(&json).unwrap();
	assert_eq!(deserialized, FailureMode::FailClosed);
}

#[test]
fn failure_mode_accepts_pascal_case_alias() {
	// Test FailOpen (PascalCase alias for compatibility)
	let deserialized: FailureMode = serde_json::from_str(r#""FailOpen""#).unwrap();
	assert_eq!(deserialized, FailureMode::FailOpen);

	// Test FailClosed (PascalCase alias for compatibility)
	let deserialized: FailureMode = serde_json::from_str(r#""FailClosed""#).unwrap();
	assert_eq!(deserialized, FailureMode::FailClosed);

	// Serialization still uses camelCase (not the alias)
	let json = serde_json::to_string(&FailureMode::FailOpen).unwrap();
	assert_eq!(json, r#""failOpen""#);
}

// --- apply tests ---

#[test]
fn apply_ok_response_passes_through() {
	use crate::http::tests_common::request_for_uri;

	let mut req = request_for_uri("http://example.com/test");
	let response = proto::RateLimitResponse {
		overall_code: proto::rate_limit_response::Code::Ok as i32,
		statuses: vec![],
		response_headers_to_add: vec![],
		request_headers_to_add: vec![proto::HeaderValue {
			key: "x-ratelimit-remaining".to_string(),
			value: "99".to_string(),
			raw_value: vec![],
		}],
		raw_body: vec![],
		dynamic_metadata: None,
		quota: None,
	};
	let result = RemoteRateLimit::apply(&mut req, response).unwrap();
	// Should not have a direct response (request is allowed)
	assert!(result.direct_response.is_none());
	// Request header should have been added
	assert_eq!(req.headers().get("x-ratelimit-remaining").unwrap(), "99");
}

#[test]
fn apply_over_limit_response_returns_429() {
	use crate::http::tests_common::request_for_uri;
	use ::http::StatusCode;

	let mut req = request_for_uri("http://example.com/test");
	let response = proto::RateLimitResponse {
		overall_code: proto::rate_limit_response::Code::OverLimit as i32,
		statuses: vec![],
		response_headers_to_add: vec![proto::HeaderValue {
			key: "retry-after".to_string(),
			value: "60".to_string(),
			raw_value: vec![],
		}],
		request_headers_to_add: vec![],
		raw_body: b"rate limit exceeded".to_vec(),
		dynamic_metadata: None,
		quota: None,
	};
	let result = RemoteRateLimit::apply(&mut req, response).unwrap();
	// Should have a direct response with 429
	let direct = result.direct_response.unwrap();
	assert_eq!(direct.status(), StatusCode::TOO_MANY_REQUESTS);
	assert_eq!(direct.headers().get("retry-after").unwrap(), "60");
}

// --- ProxyError mapping tests ---

#[test]
fn rate_limit_failed_maps_to_500() {
	use ::http::StatusCode;

	let err = ProxyError::RateLimitFailed;
	let response = err.into_response();
	assert_eq!(
		response.status(),
		StatusCode::INTERNAL_SERVER_ERROR,
		"RateLimitFailed should map to 500, not 429"
	);
}

#[test]
fn rate_limit_exceeded_maps_to_429() {
	use ::http::StatusCode;

	let err = ProxyError::RateLimitExceeded {
		limit: 10,
		remaining: 0,
		reset_seconds: 60,
	};
	let response = err.into_response();
	assert_eq!(
		response.status(),
		StatusCode::TOO_MANY_REQUESTS,
		"RateLimitExceeded should map to 429"
	);
}

// --- Config deserialization tests ---

#[test]
fn config_with_failure_mode_deserializes() {
	let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: failOpen
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
	let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
	assert_eq!(rrl.failure_mode, FailureMode::FailOpen);
	assert_eq!(rrl.domain, "test");
}

#[test]
fn config_with_fail_closed_deserializes() {
	let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: failClosed
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
	let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
	assert_eq!(rrl.failure_mode, FailureMode::FailClosed);
}

#[test]
fn config_with_pascal_case_aliases_deserializes() {
	// Test FailOpen (PascalCase alias)
	let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: FailOpen
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
	let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
	assert_eq!(rrl.failure_mode, FailureMode::FailOpen);

	// Test FailClosed (PascalCase alias)
	let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: FailClosed
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
	let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
	assert_eq!(rrl.failure_mode, FailureMode::FailClosed);
}

#[test]
fn config_without_failure_mode_defaults_to_fail_closed() {
	let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
	let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
	assert_eq!(
		rrl.failure_mode,
		FailureMode::FailClosed,
		"Missing failureMode should default to failClosed"
	);
}

/// When a descriptor uses a CEL expression that returns a Dynamic value (e.g. `jwt.sub`),
/// we materialize it before string conversion so the descriptor is populated.
/// This covers the always_materialize_owned() path in eval_descriptor.
#[test]
fn build_request_jwt_sub_descriptor_evaluates_with_materialization() {
	let entry = make_descriptor_entry(vec![("user", "jwt.sub")], RateLimitType::Requests);
	let rl = make_rate_limit(vec![entry]);

	let mut req = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri("http://example.com/mcp")
		.body(crate::http::Body::empty())
		.unwrap();
	let serde_json::Value::Object(claims) = serde_json::json!({
		"iss": "https://example.com",
		"sub": "rate-limit-user",
		"exp": 9999999999_i64,
	}) else {
		unreachable!()
	};
	req.extensions_mut().insert(jwt::Claims {
		inner: claims,
		jwt: Default::default(),
	});

	let result = rl.build_request(&req, RateLimitType::Requests, None);
	assert!(
		result.is_some(),
		"expected Some when jwt.sub evaluates (with materialization) to a string"
	);
	let request = result.unwrap();
	assert_eq!(request.descriptors.len(), 1);
	assert_eq!(request.descriptors[0].entries.len(), 1);
	assert_eq!(request.descriptors[0].entries[0].key, "user");
	assert_eq!(request.descriptors[0].entries[0].value, "rate-limit-user");
}
