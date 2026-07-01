//! Unit tests for SEP-2243 `Mcp-Param-*` deep validation.

use rmcp::model::RequestId;
use rmcp::transport::common::mcp_headers::encode_header_value;
use serde_json::{Value, json};

use super::*;
use crate::mcp::Error;

fn headers(pairs: &[(&str, &str)]) -> ::http::HeaderMap {
	let mut h = ::http::HeaderMap::new();
	for (k, v) in pairs {
		h.insert(
			::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
			::http::HeaderValue::from_str(v).unwrap(),
		);
	}
	h
}

fn schema(props: Value) -> serde_json::Map<String, Value> {
	json!({"type": "object", "properties": props})
		.as_object()
		.unwrap()
		.clone()
}

fn args(pairs: &[(&str, Value)]) -> serde_json::Map<String, Value> {
	pairs
		.iter()
		.map(|(k, v)| (k.to_string(), v.clone()))
		.collect()
}

fn region_params() -> Vec<XMcpHeaderParam> {
	x_mcp_header_params(&schema(
		json!({"region": {"type": "string", "x-mcp-header": "Region"}}),
	))
	.unwrap()
}

// Error shape: Mcp-Param-* failures render as HEADER_MISMATCH, not INVALID_PARAMS.

#[test]
fn custom_param_errors_use_header_mismatch_code() {
	for err in [
		Error::InvalidRoutingHeader(Some(RequestId::Number(5)), HEADER_MCP_PARAM),
		Error::HeaderBodyMismatch(Some(RequestId::Number(6)), HEADER_MCP_PARAM),
	] {
		let body: Value = serde_json::from_str(&err.jsonrpc_error_body().unwrap()).unwrap();
		// HEADER_MISMATCH is an external wire code; pin the literal so an accidental remap fails.
		assert_eq!(body["error"]["code"], json!(-32020));
	}
}

// Resolver and annotation validation.

#[test]
fn x_mcp_header_params_resolves_top_level_annotations() {
	let params = x_mcp_header_params(&schema(json!({
		"region": {"type": "string", "x-mcp-header": "Region"},
		"count": {"type": "integer", "x-mcp-header": "Count"},
		"plain": {"type": "string"}
	})))
	.unwrap();
	assert_eq!(params.len(), 2);
	let region = params.iter().find(|p| p.param == "region").unwrap();
	assert_eq!(
		region.header,
		::http::HeaderName::from_static("mcp-param-region")
	);
	assert_eq!(region.ty, XMcpPrimitive::String);
	let count = params.iter().find(|p| p.param == "count").unwrap();
	assert_eq!(count.ty, XMcpPrimitive::Integer);
}

#[test]
fn x_mcp_header_params_empty_without_annotations() {
	assert!(
		x_mcp_header_params(&schema(json!({"plain": {"type": "string"}})))
			.unwrap()
			.is_empty()
	);
}

#[test]
fn x_mcp_header_params_rejects_nested_annotation() {
	// SEP-2243 prose line 169 allows nesting, but the conformance edge-case table
	// and suite reject it. The suite is the behavior gate, so nested means invalid.
	let nested = schema(json!({
		"outer": {"type": "object", "properties": {"inner": {"type": "string", "x-mcp-header": "Inner"}}}
	}));
	assert!(x_mcp_header_params(&nested).is_err());
}

#[test]
fn x_mcp_header_params_rejects_annotation_outside_top_level_properties() {
	let raw = |v: Value| v.as_object().unwrap().clone();
	// A root-level `$defs`/`allOf` annotation is nested even though no top-level property carries
	// it, including when `properties` is absent entirely (the early-return path).
	for case in [
		json!({"type": "object", "$defs": {"r": {"type": "string", "x-mcp-header": "R"}}}),
		json!({"type": "object", "allOf": [{"properties": {"r": {"x-mcp-header": "R"}}}]}),
		json!({"type": "object", "properties": {"ok": {"type": "string"}}, "$defs": {"r": {"x-mcp-header": "R"}}}),
	] {
		assert!(
			x_mcp_header_params(&raw(case.clone())).is_err(),
			"expected Err for {case}"
		);
	}
}

#[test]
fn x_mcp_header_params_allows_unrelated_root_level_keywords() {
	// The nested scan must not trip on benign sibling keywords that carry no annotation.
	let raw = json!({
		"type": "object",
		"properties": {"region": {"type": "string", "x-mcp-header": "Region"}},
		"$defs": {"other": {"type": "string"}}
	})
	.as_object()
	.unwrap()
	.clone();
	let params = x_mcp_header_params(&raw).unwrap();
	assert_eq!(params.len(), 1);
	assert_eq!(params[0].param, "region");
}

#[test]
fn x_mcp_header_params_rejects_invalid_annotations() {
	for bad in [
		json!({"p": {"type": "string", "x-mcp-header": ""}}),
		json!({"p": {"type": "string", "x-mcp-header": "has space"}}),
		json!({"p": {"type": "string", "x-mcp-header": "a:b"}}),
		json!({"p": {"type": "number", "x-mcp-header": "P"}}),
		json!({"p": {"type": "array", "x-mcp-header": "P"}}),
		json!({
			"a": {"type": "string", "x-mcp-header": "Dup"},
			"b": {"type": "string", "x-mcp-header": "DUP"}
		}),
	] {
		assert!(
			x_mcp_header_params(&schema(bad.clone())).is_err(),
			"expected Err for {bad}"
		);
	}
}

// Value decode and compare.

#[test]
fn validate_param_value_matches_by_type() {
	assert!(validate_param_value("us-west1", &json!("us-west1"), XMcpPrimitive::String).is_ok());
	assert_eq!(
		validate_param_value("other", &json!("us-west1"), XMcpPrimitive::String),
		Err(XMcpValueError::Mismatch)
	);
	// Integers compare numerically: 42 == "42" == "42.0".
	assert!(validate_param_value("42", &json!(42), XMcpPrimitive::Integer).is_ok());
	assert!(validate_param_value("42.0", &json!(42), XMcpPrimitive::Integer).is_ok());
	assert_eq!(
		validate_param_value("43", &json!(42), XMcpPrimitive::Integer),
		Err(XMcpValueError::Mismatch)
	);
	assert!(validate_param_value("true", &json!(true), XMcpPrimitive::Boolean).is_ok());
	assert_eq!(
		validate_param_value("True", &json!(true), XMcpPrimitive::Boolean),
		Err(XMcpValueError::Mismatch)
	);
	// Sentinel-encoded value round-trips.
	let encoded = encode_header_value("wéird");
	assert!(validate_param_value(&encoded, &json!("wéird"), XMcpPrimitive::String).is_ok());
	assert_eq!(
		validate_param_value("=?base64?@@@?=", &json!("x"), XMcpPrimitive::String),
		Err(XMcpValueError::Undecodable)
	);
}

#[test]
fn validate_param_value_rejects_out_of_range_integer() {
	let over = json!(9_007_199_254_740_992_i64); // 2^53
	assert_eq!(
		validate_param_value("9007199254740992", &over, XMcpPrimitive::Integer),
		Err(XMcpValueError::OutOfRange)
	);
	let max = json!(9_007_199_254_740_991_i64); // 2^53 - 1
	assert!(validate_param_value("9007199254740991", &max, XMcpPrimitive::Integer).is_ok());
}

#[test]
fn validate_param_value_integer_compares_exactly_not_via_float() {
	// SEP `42` == `42.0`: an all-zero fraction still matches numerically.
	assert!(validate_param_value("42.00", &json!(42), XMcpPrimitive::Integer).is_ok());
	// A non-integer header must not match, even when f64 would round it onto the
	// body. The routed value must equal the executed value.
	assert_eq!(
		validate_param_value("42.000000000000001", &json!(42), XMcpPrimitive::Integer),
		Err(XMcpValueError::Mismatch)
	);
	// Near 2^53 the f64 ULP is ~1, so a fractional header would round onto the integer body.
	assert_eq!(
		validate_param_value(
			"9007199254740991.4",
			&json!(9_007_199_254_740_991_i64),
			XMcpPrimitive::Integer
		),
		Err(XMcpValueError::Mismatch)
	);
}

#[test]
fn validate_param_value_integer_accepts_integral_float_body() {
	// serde_json stores `42.0` as f64, so `as_i64` alone misses it; the SEP compares integers
	// numerically (`42` == `42.0`), so an integral-float body must still match the header.
	assert!(validate_param_value("42", &json!(42.0), XMcpPrimitive::Integer).is_ok());
	assert!(validate_param_value("42.0", &json!(42.0), XMcpPrimitive::Integer).is_ok());
	// A non-integral float for an integer param is still rejected.
	assert_eq!(
		validate_param_value("42", &json!(42.5), XMcpPrimitive::Integer),
		Err(XMcpValueError::OutOfRange)
	);
	// An integral float past the JS-safe bound is rejected, like its i64 twin (2^53).
	assert_eq!(
		validate_param_value(
			"9007199254740992",
			&json!(9_007_199_254_740_992.0_f64),
			XMcpPrimitive::Integer
		),
		Err(XMcpValueError::OutOfRange)
	);
}

#[test]
fn validate_param_value_integer_rejects_sentinel_padded_value() {
	// A sentinel-wrapped value with interior whitespace decodes to `" 42 "`. The integer compare is
	// verbatim (the padding is part of the value, not HTTP OWS), so it must NOT match body 42.
	let padded = encode_header_value(" 42 ");
	assert!(
		padded.starts_with("=?base64?"),
		"test value must be sentinel-wrapped"
	);
	assert_eq!(
		validate_param_value(&padded, &json!(42), XMcpPrimitive::Integer),
		Err(XMcpValueError::Mismatch)
	);
}

// Inbound header validation against arguments.

#[test]
fn validate_custom_param_headers_accepts_match_and_trims_ows() {
	let params = region_params();
	let a = args(&[("region", json!("us-west1"))]);
	let id = Some(RequestId::Number(1));
	assert!(
		validate_custom_param_headers(
			&params,
			Some(&a),
			&headers(&[("mcp-param-region", "us-west1")]),
			&id
		)
		.is_ok()
	);
	// RFC 9110 OWS around the value is trimmed before comparison.
	assert!(
		validate_custom_param_headers(
			&params,
			Some(&a),
			&headers(&[("mcp-param-region", "  us-west1  ")]),
			&id
		)
		.is_ok()
	);
}

#[test]
fn validate_custom_param_headers_rejects_each_violation() {
	let params = region_params();
	let with_region = args(&[("region", json!("us-west1"))]);
	let no_region = args(&[]);
	let id = Some(RequestId::Number(1));
	// value mismatch
	assert!(matches!(
		validate_custom_param_headers(
			&params,
			Some(&with_region),
			&headers(&[("mcp-param-region", "us-east1")]),
			&id
		),
		Err(Error::HeaderBodyMismatch(..))
	));
	// unexpected Mcp-Param-* with no declared mapping (empty params)
	assert!(matches!(
		validate_custom_param_headers(
			&[],
			Some(&no_region),
			&headers(&[("mcp-param-region", "x")]),
			&id
		),
		Err(Error::InvalidRoutingHeader(..))
	));
	// header present for an absent argument
	assert!(matches!(
		validate_custom_param_headers(
			&params,
			Some(&no_region),
			&headers(&[("mcp-param-region", "us-west1")]),
			&id
		),
		Err(Error::HeaderBodyMismatch(..))
	));
}

#[test]
fn validate_custom_param_headers_allows_absent_header() {
	// Validate-if-present: a declared param whose value is in the body but whose routing header the
	// client did not send is accepted — there is no header to validate against.
	let params = region_params();
	let a = args(&[("region", json!("us-west1"))]);
	let id = Some(RequestId::Number(1));
	assert!(validate_custom_param_headers(&params, Some(&a), &headers(&[]), &id).is_ok());
}

#[test]
fn validate_custom_param_headers_rejects_duplicate_param_header() {
	let params = region_params();
	let a = args(&[("region", json!("us-west1"))]);
	let id = Some(RequestId::Number(1));
	// Two identical lines still reject: a repeated routing header is ambiguous, not "first wins".
	let mut h = ::http::HeaderMap::new();
	let name = ::http::HeaderName::from_static("mcp-param-region");
	h.append(&name, ::http::HeaderValue::from_static("us-west1"));
	h.append(&name, ::http::HeaderValue::from_static("us-west1"));
	assert!(matches!(
		validate_custom_param_headers(&params, Some(&a), &h, &id),
		Err(Error::InvalidRoutingHeader(..))
	));
}

#[test]
fn validate_tool_call_headers_resolves_then_validates() {
	let id = Some(RequestId::Number(1));
	let a = args(&[("region", json!("us-west1"))]);
	let valid = schema(json!({"region": {"type": "string", "x-mcp-header": "Region"}}));
	assert!(
		validate_tool_call_headers(
			&valid,
			Some(&a),
			&headers(&[("mcp-param-region", "us-west1")]),
			&id
		)
		.is_ok()
	);
	// A direct call to a tool with invalid `x-mcp-header` annotations is rejected.
	// The list path only filters; the call path must reject too.
	let invalid = schema(json!({"region": {"type": "number", "x-mcp-header": "Region"}}));
	assert!(matches!(
		validate_tool_call_headers(&invalid, Some(&a), &headers(&[]), &id),
		Err(Error::InvalidRoutingHeader(..))
	));
}

#[test]
fn is_mcp_param_header_matches_prefix_case_insensitively() {
	assert!(is_mcp_param_header(&::http::HeaderName::from_static(
		"mcp-param-region"
	)));
	// HeaderName normalizes to lowercase, so an uppercase wire name still matches.
	assert!(is_mcp_param_header(
		&::http::HeaderName::from_bytes(b"Mcp-Param-Region").unwrap()
	));
	assert!(!is_mcp_param_header(&::http::HeaderName::from_static(
		"mcp-method"
	)));
	// The bare prefix with no suffix is not a param header.
	assert!(!is_mcp_param_header(&::http::HeaderName::from_static(
		"mcp-param-"
	)));
}
