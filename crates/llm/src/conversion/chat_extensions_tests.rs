use serde_json::{Value, json};

use crate::conversion::{messages, vertex_gemini};
use crate::model_catalog::{TestCatalog, tags};
use crate::types::completions::Request;

fn request(fields: Value) -> Request {
	let mut value = json!({"model": "gemini-2.5-pro", "messages": [{"role": "user", "content": "Hello"}], "max_tokens": 6400});
	value
		.as_object_mut()
		.unwrap()
		.extend(fields.as_object().unwrap().clone());
	serde_json::from_value(value).unwrap()
}

fn messages_body(req: &Request) -> Result<Value, crate::AIError> {
	let bytes = messages::from_completions::translate(req, None)?;
	Ok(serde_json::from_slice(&bytes).unwrap())
}

fn gemini_body(req: &Request) -> Result<Value, crate::AIError> {
	let bytes = vertex_gemini::from_completions::translate(req, None)?;
	Ok(serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn bare_schema_matches_wrapped_schema_on_both_providers() {
	let schema =
		json!({"type": "object", "properties": {"value": {"type": "string"}}, "required": ["value"]});
	let bare = request(json!({"json_schema": schema}));
	let wrapped = request(json!({"json_schema": {"name": "response", "schema": schema}}));
	assert_eq!(
		messages_body(&bare).unwrap(),
		messages_body(&wrapped).unwrap()
	);
	assert_eq!(gemini_body(&bare).unwrap(), gemini_body(&wrapped).unwrap());
}

#[test]
fn conflicting_schema_fields_are_rejected_by_both_providers() {
	let req =
		request(json!({"json_schema": {"type": "object"}, "response_format": {"type": "json_object"}}));
	assert!(
		messages_body(&req)
			.unwrap_err()
			.to_string()
			.contains("must agree")
	);
	assert!(
		gemini_body(&req)
			.unwrap_err()
			.to_string()
			.contains("must agree")
	);
}

#[test]
fn identical_schema_fields_are_accepted() {
	let alias = json!({"name": "person", "strict": true, "schema": {"type": "object"}});
	let req = request(
		json!({"json_schema": alias, "response_format": {"type": "json_schema", "json_schema": alias}}),
	);
	assert!(messages_body(&req).is_ok());
	assert!(gemini_body(&req).is_ok());
}

#[rstest::rstest]
#[case(json!("invalid"))]
#[case(json!({"schema": []}))]
fn invalid_schemas_are_rejected(#[case] schema: Value) {
	let req = request(json!({"json_schema": schema}));
	assert!(messages_body(&req).is_err());
	assert!(gemini_body(&req).is_err());
}

#[test]
fn adaptive_messages_use_standard_reasoning_effort() {
	let catalog = TestCatalog::new([("adaptive-model", &[tags::ADAPTIVE_THINKING][..])]);
	let req = request(json!({"model": "adaptive-model", "reasoning_effort": "high"}));
	let bytes = messages::from_completions::translate(&req, Some(&catalog)).unwrap();
	let body: Value = serde_json::from_slice(&bytes).unwrap();
	assert_eq!(body["thinking"]["type"], "adaptive");
	assert_eq!(body["output_config"]["effort"], "high");
}

#[test]
fn gemini_three_uses_standard_reasoning_effort() {
	let req = request(json!({"model": "gemini-3.1-pro", "reasoning_effort": "high"}));
	assert_eq!(
		gemini_body(&req).unwrap()["generationConfig"]["thinkingConfig"],
		json!({"thinkingLevel": "high", "includeThoughts": true})
	);
}

#[test]
fn nonstandard_reasoning_does_not_override_standard_effort() {
	let standard = request(json!({"reasoning_effort": "low"}));
	let extended = request(json!({
			"reasoning_effort": "low",
			"reasoning": {"effort": "high", "max_tokens": 2048, "exclude": true}
	}));
	assert_eq!(
		messages_body(&standard).unwrap(),
		messages_body(&extended).unwrap()
	);
	assert_eq!(
		gemini_body(&standard).unwrap(),
		gemini_body(&extended).unwrap()
	);
}

#[test]
fn passthrough_serialization_preserves_schema_and_standard_effort() {
	let fields = json!({"reasoning_effort": "high", "json_schema": {"type": "object"}});
	let wire = serde_json::to_value(request(fields.clone())).unwrap();
	assert_eq!(wire["reasoning_effort"], fields["reasoning_effort"]);
	assert_eq!(wire["json_schema"], fields["json_schema"]);
}
