use std::sync::Arc;

use super::*;
use crate::cel;
use crate::http::localratelimit::RateLimitType;

/// Helper: build a `RemoteRateLimit` with the given descriptor entries.
fn make_rate_limit(descriptor_entries: Vec<DescriptorEntry>) -> RemoteRateLimit {
	RemoteRateLimit {
		domain: "test-domain".to_string(),
		target: Arc::new(SimpleBackendReference::Invalid),
		descriptors: Arc::new(DescriptorSet(descriptor_entries)),
		timeout: None,
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
/// `build_request` should return `None` (fail-fast on first failure).
#[test]
fn build_request_second_descriptor_fails_returns_none() {
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
		result.is_none(),
		"expected None when any descriptor entry fails evaluation"
	);
}

/// When the first descriptor fails, `build_request` should return `None`
/// without evaluating the second.
#[test]
fn build_request_first_descriptor_fails_returns_none() {
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
		result.is_none(),
		"expected None when first descriptor fails"
	);
}

/// When no descriptors match the requested `limit_type`,
/// `build_request` returns `Some` with an empty descriptors list.
/// (Callers guard against this before calling `build_request`.)
#[test]
fn build_request_no_matching_type_returns_some_empty() {
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
		result.is_some(),
		"expected Some with empty descriptors when no candidates match"
	);
	assert!(result.unwrap().descriptors.is_empty());
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
