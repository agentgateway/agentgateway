//! Optional GCF (Graph Compact Format) encoding for MCP tool responses.
//!
//! When enabled, text content in CallToolResult responses is re-encoded
//! from JSON to GCF, reducing token usage for LLM consumers.
//! See <https://gcformat.com>

use rmcp::model::{Annotated, Content, RawContent, RawTextContent, ServerJsonRpcMessage, ServerResult};

/// Re-encode JSON text content in CallToolResult responses as GCF.
///
/// Only transforms text content that successfully parses as JSON.
/// Non-JSON text, images, audio, and other content types pass through unchanged.
pub fn encode_tool_result(message: ServerJsonRpcMessage) -> ServerJsonRpcMessage {
	match message {
		ServerJsonRpcMessage::Response(mut response) => {
			if let ServerResult::CallToolResult(ref mut result) = response.result {
				result.content = result
					.content
					.drain(..)
					.map(encode_content)
					.collect();
			}
			ServerJsonRpcMessage::Response(response)
		},
		other => other,
	}
}

fn encode_content(content: Content) -> Content {
	match &content.raw {
		RawContent::Text(text) => {
			// Only re-encode if the text is valid JSON
			match serde_json::from_str::<serde_json::Value>(&text.text) {
				Ok(value) => {
					let gcf_text = gcf::encode_generic(&value);
					Annotated {
						raw: RawContent::Text(RawTextContent {
							text: gcf_text,
							meta: text.meta.clone(),
						}),
						annotations: content.annotations,
					}
				},
				// Not JSON, pass through unchanged
				Err(_) => content,
			}
		},
		// Non-text content passes through unchanged
		_ => content,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rmcp::model::{CallToolResult, Content, RequestId};
	use serde_json::json;

	#[test]
	fn encodes_json_text_as_gcf() {
		let result = CallToolResult::success(vec![Content::text(
			serde_json::to_string(&json!({"name": "test", "value": 42})).unwrap(),
		)]);
		let message = ServerJsonRpcMessage::response(
			ServerResult::CallToolResult(result),
			RequestId::Number(1),
		);

		let encoded = encode_tool_result(message);
		if let ServerJsonRpcMessage::Response(resp) = encoded {
			if let ServerResult::CallToolResult(result) = resp.result {
				let text = result.content[0].raw.as_text().unwrap();
				assert!(text.text.starts_with("GCF profile=generic"));
				assert!(text.text.contains("name=test"));
				assert!(text.text.contains("value=42"));
			} else {
				panic!("expected CallToolResult");
			}
		} else {
			panic!("expected Response");
		}
	}

	#[test]
	fn passes_through_non_json_text() {
		let result = CallToolResult::success(vec![Content::text("hello world")]);
		let message = ServerJsonRpcMessage::response(
			ServerResult::CallToolResult(result),
			RequestId::Number(1),
		);

		let encoded = encode_tool_result(message);
		if let ServerJsonRpcMessage::Response(resp) = encoded {
			if let ServerResult::CallToolResult(result) = resp.result {
				let text = result.content[0].raw.as_text().unwrap();
				assert_eq!(text.text, "hello world");
			} else {
				panic!("expected CallToolResult");
			}
		} else {
			panic!("expected Response");
		}
	}

	#[test]
	fn passes_through_non_tool_results() {
		let message = ServerJsonRpcMessage::error(
			rmcp::ErrorData::internal_error("test error", None),
			RequestId::Number(1),
		);
		let encoded = encode_tool_result(message.clone());
		// Error messages should pass through unchanged
		assert!(matches!(encoded, ServerJsonRpcMessage::Error(_)));
	}
}
