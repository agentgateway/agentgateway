//! Wire DTOs for the Vertex native Gemini API (`:generateContent` /
//! `:streamGenerateContent`).
//!
//! References:
//! - <https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/models/inference>
//! - <https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/rest/v1/projects.locations.publishers.models/generateContent>
//! - <https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/rest/v1/projects.locations.publishers.models/streamGenerateContent>
//!
//! Deserialized types tolerate unknown fields (flattened into `rest`, no `deny_unknown_fields`)
//! so Google's additive changes don't break parsing.

use serde::{Deserialize, Serialize};

// ---------- Request ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentRequest {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub contents: Vec<Content>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub system_instruction: Option<Content>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub tools: Vec<Tool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tool_config: Option<ToolConfig>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub generation_config: Option<GenerationConfig>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub safety_settings: Vec<SafetySetting>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cached_content: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub labels: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Content {
	/// `user` or `model`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub role: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub parts: Vec<Part>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

// ---------- Part: untagged enum with required-discriminator variants ----------

/// Untagged enum: serde tries variants in order, so common shapes come first and
/// `Unknown` last. Each typed variant is keyed by a required discriminator field; an
/// object matching none deserializes as `Unknown`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
	Text(TextPart),
	FunctionCall(FunctionCallPart),
	FunctionResponse(FunctionResponsePart),
	InlineData(InlineDataPart),
	FileData(FileDataPart),
	ExecutableCode(ExecutableCodePart),
	CodeExecutionResult(CodeExecutionResultPart),
	Unknown(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPart {
	pub text: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thought: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thought_signature: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallPart {
	pub function_call: FunctionCall,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thought: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thought_signature: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionResponsePart {
	pub function_response: FunctionResponse,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineDataPart {
	pub inline_data: Blob,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDataPart {
	pub file_data: FileData,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableCodePart {
	pub executable_code: serde_json::Value,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeExecutionResultPart {
	pub code_execution_result: serde_json::Value,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

// ---------- Part inner types ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub id: Option<String>,
	#[serde(default)]
	pub args: serde_json::Value,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub id: Option<String>,
	#[serde(default)]
	pub response: serde_json::Value,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blob {
	// Field order matters: Vertex returns 400 if `mimeType` comes after `data`.
	pub mime_type: String,
	pub data: String,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileData {
	// Field order matters: `mimeType` must precede `fileUri` in serialised JSON.
	// `mime_type` is optional; Vertex resolves it server-side for Files API URLs.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub mime_type: Option<String>,
	pub file_uri: String,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

// ---------- Tools and tool config ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub function_declarations: Vec<FunctionDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub function_calling_config: Option<FunctionCallingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallingConfig {
	/// `AUTO` | `ANY` | `NONE`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub mode: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub allowed_function_names: Vec<String>,
}

// ---------- GenerationConfig and thinking ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub top_k: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub frequency_penalty: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub presence_penalty: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_output_tokens: Option<u32>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub stop_sequences: Vec<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub candidate_count: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub seed: Option<i64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_mime_type: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_schema: Option<serde_json::Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thinking_config: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingConfig {
	/// Gemini 3: `"minimal"` | `"low"` | `"medium"` | `"high"`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thinking_level: Option<String>,
	/// Gemini 2.5: integer token budget; `-1` means dynamic.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thinking_budget: Option<i32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub include_thoughts: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetySetting {
	pub category: String,
	pub threshold: String,
}

// ---------- Response ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub candidates: Vec<Candidate>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompt_feedback: Option<PromptFeedback>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub usage_metadata: Option<UsageMetadata>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_id: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model_version: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub content: Option<Content>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub finish_reason: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub index: Option<u32>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedback {
	/// Set when the prompt is blocked before any candidate is produced.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub block_reason: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompt_token_count: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub candidates_token_count: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub total_token_count: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cached_content_token_count: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thoughts_token_count: Option<u64>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	// ---------- Part disambiguation ----------

	#[test]
	fn part_text_matches_text_variant() {
		let p: Part = serde_json::from_value(json!({ "text": "hello" })).unwrap();
		assert!(matches!(p, Part::Text(_)));
	}

	#[test]
	fn part_text_with_thought_keeps_typed_variant() {
		let p: Part = serde_json::from_value(json!({ "text": "internal", "thought": true })).unwrap();
		let Part::Text(t) = p else {
			panic!("expected Text variant")
		};
		assert_eq!(t.text, "internal");
		assert_eq!(t.thought, Some(true));
	}

	#[test]
	fn part_function_call_matches_function_call_variant() {
		let p: Part = serde_json::from_value(json!({
			"functionCall": { "name": "get_weather", "args": { "city": "Berlin" } }
		}))
		.unwrap();
		let Part::FunctionCall(fc) = p else {
			panic!("expected FunctionCall variant")
		};
		assert_eq!(fc.function_call.name, "get_weather");
		assert_eq!(fc.function_call.args["city"], "Berlin");
	}

	#[test]
	fn part_function_response_matches_function_response_variant() {
		let p: Part = serde_json::from_value(json!({
			"functionResponse": { "name": "get_weather", "response": { "temp_c": 12 } }
		}))
		.unwrap();
		assert!(matches!(p, Part::FunctionResponse(_)));
	}

	#[test]
	fn part_inline_data_and_file_data() {
		let inline: Part = serde_json::from_value(
			json!({ "inlineData": { "mimeType": "image/png", "data": "iVBORw0KG..." } }),
		)
		.unwrap();
		assert!(matches!(inline, Part::InlineData(_)));

		let file: Part = serde_json::from_value(json!({
			"fileData": { "mimeType": "image/jpeg", "fileUri": "gs://bucket/k.jpg" }
		}))
		.unwrap();
		assert!(matches!(file, Part::FileData(_)));
	}

	#[test]
	fn part_unknown_catches_genuinely_new_shape() {
		let p: Part = serde_json::from_value(json!({ "videoMetadata": { "fps": 30 } })).unwrap();
		assert!(matches!(p, Part::Unknown(_)));
	}

	#[test]
	fn part_empty_object_falls_to_unknown() {
		let p: Part = serde_json::from_value(json!({})).unwrap();
		assert!(matches!(p, Part::Unknown(_)));
	}

	// ---------- Drift tolerance: rest flatten preserves additive fields ----------

	#[test]
	fn part_text_additive_field_round_trips_via_rest_not_unknown() {
		let raw = json!({ "text": "hi", "someFutureFlag": "x" });
		let p: Part = serde_json::from_value(raw.clone()).unwrap();
		let Part::Text(_) = &p else {
			panic!("additive field must not push variant to Unknown")
		};
		let out = serde_json::to_value(&p).unwrap();
		assert_eq!(out["text"], "hi");
		assert_eq!(out["someFutureFlag"], "x");
	}

	#[test]
	fn function_call_additive_field_round_trips() {
		let raw = json!({
			"functionCall": { "name": "f", "args": {}, "someNewSubField": 1 },
			"someNewPartLevelField": true
		});
		let p: Part = serde_json::from_value(raw).unwrap();
		let Part::FunctionCall(_) = &p else {
			panic!("additive part-level field must not flip variant to Unknown")
		};
		let out = serde_json::to_value(&p).unwrap();
		assert_eq!(out["functionCall"]["someNewSubField"], 1);
		assert_eq!(out["someNewPartLevelField"], true);
	}

	// ---------- thoughtSignature round-trips on Text and FunctionCall ----------

	#[test]
	fn thought_signature_on_text_part_round_trips() {
		let sig = "Co4CAdHtim/rWgX...truncated...";
		let p: Part = serde_json::from_value(
			json!({ "text": "thinking", "thought": true, "thoughtSignature": sig }),
		)
		.unwrap();
		let Part::Text(t) = &p else {
			panic!("expected Text variant");
		};
		assert_eq!(t.thought_signature.as_deref(), Some(sig));
		let out = serde_json::to_value(&p).unwrap();
		assert_eq!(out["thoughtSignature"], sig);
	}

	#[test]
	fn thought_signature_on_function_call_round_trips() {
		let sig = "AnotherSignature...";
		let p: Part = serde_json::from_value(json!({
			"functionCall": { "name": "f", "args": {} },
			"thoughtSignature": sig
		}))
		.unwrap();
		let Part::FunctionCall(fc) = &p else {
			panic!("expected FunctionCall variant");
		};
		assert_eq!(fc.thought_signature.as_deref(), Some(sig));
	}

	// ---------- Field-order regression: file_data and blob ----------

	#[test]
	fn file_data_field_order_mime_type_before_file_uri() {
		// Vertex rejects file_data with `fileUri` appearing before `mimeType`.
		let fd = FileDataPart {
			file_data: FileData {
				mime_type: Some("audio/mpeg".into()),
				file_uri: "gs://bucket/audio.mp3".into(),
				rest: serde_json::Value::Null,
			},
			rest: serde_json::Value::Null,
		};
		let s = serde_json::to_string(&fd).unwrap();
		let mime_pos = s.find("\"mimeType\"").expect("mimeType present");
		let uri_pos = s.find("\"fileUri\"").expect("fileUri present");
		assert!(
			mime_pos < uri_pos,
			"mimeType must serialize before fileUri, got: {s}"
		);
	}

	#[test]
	fn blob_field_order_mime_type_before_data() {
		let b = InlineDataPart {
			inline_data: Blob {
				mime_type: "image/png".into(),
				data: "iVBORw0KG...".into(),
				rest: serde_json::Value::Null,
			},
			rest: serde_json::Value::Null,
		};
		let s = serde_json::to_string(&b).unwrap();
		let mime_pos = s.find("\"mimeType\"").expect("mimeType present");
		let data_pos = s.find("\"data\"").expect("data present");
		assert!(
			mime_pos < data_pos,
			"mimeType must serialize before data, got: {s}"
		);
	}

	// ---------- Top-level request/response round-trip ----------

	#[test]
	fn empty_request_serializes_minimally() {
		let r = GenerateContentRequest::default();
		let s = serde_json::to_string(&r).unwrap();
		assert_eq!(s, "{}");
	}

	#[test]
	fn response_with_block_reason_parses() {
		let raw = json!({
			"promptFeedback": { "blockReason": "SAFETY" },
			"usageMetadata": { "promptTokenCount": 12, "totalTokenCount": 12 }
		});
		let resp: GenerateContentResponse = serde_json::from_value(raw).unwrap();
		assert_eq!(
			resp.prompt_feedback.unwrap().block_reason.as_deref(),
			Some("SAFETY")
		);
		assert!(resp.candidates.is_empty());
		assert_eq!(resp.usage_metadata.unwrap().prompt_token_count, Some(12));
	}

	#[test]
	fn response_with_thought_signature_on_candidate_text_part() {
		let raw = json!({
			"candidates": [{
				"content": {
					"role": "model",
					"parts": [
						{ "text": "reasoning", "thought": true, "thoughtSignature": "SIG" },
						{ "text": "answer" }
					]
				},
				"finishReason": "STOP"
			}],
			"usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 7, "totalTokenCount": 12 }
		});
		let resp: GenerateContentResponse = serde_json::from_value(raw).unwrap();
		assert_eq!(resp.candidates.len(), 1);
		let content = resp.candidates[0].content.as_ref().unwrap();
		assert_eq!(content.parts.len(), 2);
		let Part::Text(t0) = &content.parts[0] else {
			panic!("first part should be Text (thought)")
		};
		assert_eq!(t0.thought, Some(true));
		assert_eq!(t0.thought_signature.as_deref(), Some("SIG"));
		let Part::Text(t1) = &content.parts[1] else {
			panic!("second part should be Text (answer)")
		};
		assert!(t1.thought.is_none() || t1.thought == Some(false));
		assert_eq!(t1.thought_signature, None);
	}

	#[test]
	fn function_calling_config_round_trips() {
		let cfg = ToolConfig {
			function_calling_config: Some(FunctionCallingConfig {
				mode: Some("ANY".into()),
				allowed_function_names: vec!["get_weather".into()],
			}),
		};
		let s = serde_json::to_string(&cfg).unwrap();
		assert!(s.contains("\"functionCallingConfig\""));
		assert!(s.contains("\"allowedFunctionNames\""));
		assert!(s.contains("\"mode\":\"ANY\""));
	}

	#[test]
	fn thinking_config_round_trips() {
		let raw = json!({ "thinkingLevel": "high", "includeThoughts": true });
		let tc: ThinkingConfig = serde_json::from_value(raw).unwrap();
		assert_eq!(tc.thinking_level.as_deref(), Some("high"));
		assert_eq!(tc.include_thoughts, Some(true));

		let raw2 = json!({ "thinkingBudget": 2048 });
		let tc2: ThinkingConfig = serde_json::from_value(raw2).unwrap();
		assert_eq!(tc2.thinking_budget, Some(2048));
	}
}
