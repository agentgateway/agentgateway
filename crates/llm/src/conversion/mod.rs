pub mod bedrock;
pub mod completions;
pub mod gemini;
pub mod messages;
pub mod openai_compat;
pub mod responses;
pub mod vertex;
pub mod vertex_gemini;

use crate::AIError;

/// Translate an OpenAI `tool_calls[].function.arguments` string into an Anthropic
/// `tool_use.input` value.
///
/// A tool call with no arguments is conventionally encoded as the empty string, which is not
/// valid JSON. Anthropic requires `tool_use.input` to be an object, so that becomes `{}`.
///
/// Everything else is parsed and passed through unchanged, including values that are malformed
/// or not an object (this would avoid changing silently the upstream response).
pub(crate) fn tool_arguments_to_input(arguments: &str) -> serde_json::Value {
	if arguments.is_empty() {
		return serde_json::json!({});
	}
	serde_json::from_str::<serde_json::Value>(arguments)
		.unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()))
}

/// Remove the internal Vertex Gemini thought-signature suffix from tool-call identifiers before
/// a request is sent to another provider. Native Gemini requests must keep the suffix so the
/// provider can recover the signature, while OpenAI-compatible, Anthropic, and Bedrock wire
/// formats have no such convention and may reject the resulting overlong identifier.
pub fn strip_vertex_thought_signatures(mut body: Vec<u8>) -> Result<Vec<u8>, AIError> {
	let mut value: serde_json::Value =
		serde_json::from_slice(&body).map_err(AIError::RequestMarshal)?;
	strip_thought_signature_fields(&mut value);
	body = serde_json::to_vec(&value).map_err(AIError::RequestMarshal)?;
	Ok(body)
}

fn strip_thought_signature_fields(value: &mut serde_json::Value) {
	let serde_json::Value::Object(object) = value else {
		if let serde_json::Value::Array(values) = value {
			for value in values {
				strip_thought_signature_fields(value);
			}
		}
		return;
	};

	let item_type = object
		.get("type")
		.and_then(serde_json::Value::as_str)
		.map(str::to_owned);
	if (item_type.as_deref() == Some("function") && object.contains_key("function"))
		|| item_type.as_deref() == Some("tool_use")
	{
		strip_string_field(object, "id");
	}
	if matches!(
		item_type.as_deref(),
		Some("function_call" | "function_call_output")
	) {
		strip_string_field(object, "call_id");
	}
	if object.get("role").and_then(serde_json::Value::as_str) == Some("tool") {
		strip_string_field(object, "tool_call_id");
	}
	if let Some(serde_json::Value::Array(tool_calls)) = object.get_mut("tool_calls") {
		for tool_call in tool_calls {
			if let serde_json::Value::Object(tool_call) = tool_call {
				strip_string_field(tool_call, "id");
			}
		}
	}
	strip_string_field(object, "tool_use_id");
	strip_string_field(object, "toolUseId");

	for value in object.values_mut() {
		strip_thought_signature_fields(value);
	}
}

fn strip_string_field(object: &mut serde_json::Map<String, serde_json::Value>, field: &str) {
	let Some(value) = object.get(field).and_then(serde_json::Value::as_str) else {
		return;
	};
	let stripped = vertex_gemini::strip_thought_signature(value).to_owned();
	object.insert(field.to_string(), serde_json::Value::String(stripped));
}
#[cfg(test)]
mod rerank_tests;

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::{strip_vertex_thought_signatures, tool_arguments_to_input};

	#[test]
	fn empty_arguments_become_an_empty_object() {
		assert_eq!(tool_arguments_to_input(""), json!({}));
	}

	#[test]
	fn everything_else_passes_through() {
		// Valid JSON is forwarded as parsed, whatever shape it has.
		assert_eq!(tool_arguments_to_input("{}"), json!({}));
		assert_eq!(
			tool_arguments_to_input("{\"a\":1,\"b\":[2,null]}"),
			json!({"a": 1, "b": [2, null]})
		);
		assert_eq!(tool_arguments_to_input("[]"), json!([]));
		// Non-object JSON is forwarded as parsed; malformed JSON is kept verbatim as a string instead of being degraded to `{}`
		assert_eq!(tool_arguments_to_input("null"), json!(null));
		assert_eq!(tool_arguments_to_input("5"), json!(5));
		assert_eq!(
			tool_arguments_to_input("{\"location\": \"Par"),
			json!("{\"location\": \"Par")
		);
		assert_eq!(tool_arguments_to_input("  "), json!("  "));
	}

	#[test]
	fn strips_vertex_thought_signatures_only_from_tool_identifiers() {
		let body = serde_json::to_vec(&json!({
			"messages": [
				{
					"role": "assistant",
					"tool_calls": [{
						"id": "call-1__thought__signature",
						"function": {"name": "lookup", "arguments": "{}"}
					}]
				},
				{"role": "tool", "tool_call_id": "call-1__thought__signature"}
			],
			"input": [
				{"type": "function_call", "call_id": "call-1__thought__signature"},
				{"type": "function_call_output", "call_id": "call-1__thought__signature"}
			],
			"content": [{"type": "tool_use", "id": "call-1__thought__signature"}],
			"metadata": {"id": "call-1__thought__signature"}
		}))
		.unwrap();
		let body = strip_vertex_thought_signatures(body).unwrap();
		let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call-1");
		assert_eq!(body["messages"][1]["tool_call_id"], "call-1");
		assert_eq!(body["input"][0]["call_id"], "call-1");
		assert_eq!(body["input"][1]["call_id"], "call-1");
		assert_eq!(body["content"][0]["id"], "call-1");
		assert_eq!(body["metadata"]["id"], "call-1__thought__signature");
	}
}
