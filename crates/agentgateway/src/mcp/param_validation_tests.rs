//! Unit tests for SEP-2243 `x-mcp-header` annotation validity.

use serde_json::{Value, json};

use super::*;

fn schema(props: Value) -> serde_json::Map<String, Value> {
	json!({"type": "object", "properties": props})
		.as_object()
		.unwrap()
		.clone()
}

#[test]
fn accepts_valid_top_level_annotations() {
	assert!(
		validate_x_mcp_header_annotations(&schema(json!({
			"region": {"type": "string", "x-mcp-header": "Region"},
			"count": {"type": "integer", "x-mcp-header": "Count"},
			"enabled": {"type": "boolean", "x-mcp-header": "Enabled"},
			"plain": {"type": "string"}
		})))
		.is_ok()
	);
}

#[test]
fn accepts_schema_without_annotations() {
	assert!(validate_x_mcp_header_annotations(&schema(json!({"plain": {"type": "string"}}))).is_ok());
}

#[test]
fn rejects_nested_annotation() {
	// SEP-2243 prose allows nesting, but the conformance edge-case table and suite reject it. The
	// suite is the behavior gate, so nested means invalid.
	let nested = schema(json!({
		"outer": {"type": "object", "properties": {"inner": {"type": "string", "x-mcp-header": "Inner"}}}
	}));
	assert!(validate_x_mcp_header_annotations(&nested).is_err());
}

#[test]
fn rejects_annotation_outside_top_level_properties() {
	let raw = |v: Value| v.as_object().unwrap().clone();
	// A root-level `$defs`/`allOf` annotation is nested even though no top-level property carries it,
	// including when `properties` is absent entirely (the early-return path).
	for case in [
		json!({"type": "object", "$defs": {"r": {"type": "string", "x-mcp-header": "R"}}}),
		json!({"type": "object", "allOf": [{"properties": {"r": {"x-mcp-header": "R"}}}]}),
		json!({"type": "object", "properties": {"ok": {"type": "string"}}, "$defs": {"r": {"x-mcp-header": "R"}}}),
	] {
		assert!(
			validate_x_mcp_header_annotations(&raw(case.clone())).is_err(),
			"expected Err for {case}"
		);
	}
}

#[test]
fn allows_unrelated_root_level_keywords() {
	// The nested scan must not trip on benign sibling keywords that carry no annotation.
	let raw = json!({
		"type": "object",
		"properties": {"region": {"type": "string", "x-mcp-header": "Region"}},
		"$defs": {"other": {"type": "string"}}
	})
	.as_object()
	.unwrap()
	.clone();
	assert!(validate_x_mcp_header_annotations(&raw).is_ok());
}

#[test]
fn rejects_invalid_annotations() {
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
			validate_x_mcp_header_annotations(&schema(bad.clone())).is_err(),
			"expected Err for {bad}"
		);
	}
}
