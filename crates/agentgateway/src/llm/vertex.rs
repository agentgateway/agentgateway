use agent_core::strng;
use agent_core::strng::Strng;
use serde_json::{Map, Value};

use crate::llm::{AIError, RouteType};
use crate::*;

const ANTHROPIC_VERSION: &str = "vertex-2023-10-16";

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub region: Option<Strng>,
	pub project_id: Strng,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("gcp.vertex_ai");
}

impl Provider {
	fn configured_model<'a>(&'a self, request_model: Option<&'a str>) -> Option<&'a str> {
		self.model.as_deref().or(request_model)
	}

	pub fn is_anthropic_model(&self, request_model: Option<&str>) -> bool {
		self.anthropic_model(request_model).is_some()
	}

	pub fn prepare_anthropic_message_body(&self, body: Vec<u8>) -> Result<Vec<u8>, AIError> {
		self.prepare_anthropic_body(body, |b| {
			b.remove("model");
		})
	}

	pub fn prepare_anthropic_count_tokens_body(&self, body: Vec<u8>) -> Result<Vec<u8>, AIError> {
		self.prepare_anthropic_body(body, |b| {
			if let Some(Value::String(model)) = b.get("model") {
				let normalized = self
					.configured_model(Some(model))
					.map(|s| s.to_string())
					.unwrap_or_else(|| model.clone());
				b.insert("model".to_string(), Value::String(normalized));
			}
		})
	}

	/// Shared pipeline for Vertex Anthropic requests: parse, inject version,
	/// apply caller-specific model handling, strip unsupported fields, serialize.
	fn prepare_anthropic_body(
		&self,
		body: Vec<u8>,
		apply: impl FnOnce(&mut Map<String, Value>),
	) -> Result<Vec<u8>, AIError> {
		let mut body: Map<String, Value> =
			serde_json::from_slice(&body).map_err(AIError::RequestMarshal)?;
		body.insert(
			"anthropic_version".to_string(),
			Value::String(ANTHROPIC_VERSION.to_string()),
		);
		apply(&mut body);
		remove_unsupported_vertex_fields(&mut body);
		serde_json::to_vec(&body).map_err(AIError::RequestMarshal)
	}

	pub fn get_path_for_model(
		&self,
		route: RouteType,
		request_model: Option<&str>,
		streaming: bool,
	) -> Strng {
		let location = self
			.region
			.clone()
			.unwrap_or_else(|| strng::literal!("global"));

		match (route, self.anthropic_model(request_model)) {
			(RouteType::AnthropicTokenCount, _) => {
				strng::format!(
					"/v1/projects/{}/locations/{}/publishers/anthropic/models/count-tokens:rawPredict",
					self.project_id,
					location
				)
			},
			(RouteType::Embeddings, _) => {
				let model = self.configured_model(request_model).unwrap_or_default();
				strng::format!(
					"/v1/projects/{}/locations/{}/publishers/google/models/{}:predict",
					self.project_id,
					location,
					model
				)
			},
			(_, Some(model)) => {
				strng::format!(
					"/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:{}",
					self.project_id,
					location,
					model,
					if streaming {
						"streamRawPredict"
					} else {
						"rawPredict"
					}
				)
			},
			_ => {
				strng::format!(
					"/v1/projects/{}/locations/{}/endpoints/openapi/chat/completions",
					self.project_id,
					location
				)
			},
		}
	}

	pub fn get_host(&self, _request_model: Option<&str>) -> Strng {
		match &self.region {
			None => strng::literal!("aiplatform.googleapis.com"),
			Some(region) if region == "global" => strng::literal!("aiplatform.googleapis.com"),
			Some(region) => strng::format!("{region}-aiplatform.googleapis.com"),
		}
	}

	fn anthropic_model<'a>(&'a self, request_model: Option<&'a str>) -> Option<Strng> {
		let model = self.configured_model(request_model)?;

		// Strip known prefixes
		let model: &str = model
			.split_once("publishers/anthropic/models/")
			.map(|(_, m)| m)
			.or_else(|| model.strip_prefix("anthropic/"))
			.or_else(|| {
				if model.starts_with("claude-") {
					Some(model)
				} else {
					None
				}
			})?;

		// Replace -YYYYMMDD with @YYYYMMDD
		if model.len() > 8 && model.as_bytes()[model.len() - 9] == b'-' {
			let (base, date) = model.split_at(model.len() - 8);
			if date.chars().all(|c| c.is_ascii_digit()) {
				Some(strng::new(format!("{}@{}", &base[..base.len() - 1], date)))
			} else {
				Some(strng::new(model))
			}
		} else {
			Some(strng::new(model))
		}
	}
}

fn remove_unsupported_vertex_fields(body: &mut Map<String, Value>) {
	body.remove("output_config");
	body.remove("output_format");
	// Vertex supports cache_control but not the "scope" child from the prompt-caching-scope beta.
	for value in body.values_mut() {
		remove_nested_field(value, "cache_control", "scope");
	}
}

fn remove_nested_field(value: &mut Value, key: &str, child: &str) {
	match value {
		Value::Object(map) => {
			if let Some(Value::Object(nested)) = map.get_mut(key) {
				nested.remove(child);
			}
			for v in map.values_mut() {
				remove_nested_field(v, key, child);
			}
		},
		Value::Array(arr) => {
			for v in arr {
				remove_nested_field(v, key, child);
			}
		},
		_ => {},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[rstest::rstest]
	#[case::strip_publishers_prefix(
		Some("publishers/anthropic/models/claude-sonnet-4-5-20251001"),
		None,
		Some("claude-sonnet-4-5@20251001")
	)]
	#[case::strip_anthropic_prefix(
		Some("anthropic/claude-haiku-4-5-20251001"),
		None,
		Some("claude-haiku-4-5@20251001")
	)]
	#[case::raw_claude_prefix(None, Some("claude-opus-3-20240229"), Some("claude-opus-3@20240229"))]
	#[case::no_date_suffix(None, Some("claude-opus-4-6"), Some("claude-opus-4-6"))]
	#[case::legacy_model(
		None,
		Some("claude-3-5-sonnet-20241022"),
		Some("claude-3-5-sonnet@20241022")
	)]
	#[case::non_digit_date_suffix(
		None,
		Some("claude-haiku-4-5-2025abcd"),
		Some("claude-haiku-4-5-2025abcd")
	)]
	#[case::non_anthropic_model(None, Some("text-embedding-004"), None)]
	#[case::provider_model_precedence(
		Some("anthropic/claude-haiku-4-5-20251001"),
		Some("anthropic/claude-sonnet-4-5-20251001"),
		Some("claude-haiku-4-5@20251001")
	)]
	fn test_anthropic_model_normalization(
		#[case] provider: Option<&str>,
		#[case] req: Option<&str>,
		#[case] expected: Option<&str>,
	) {
		let p = Provider {
			project_id: strng::new("test-project"),
			model: provider.map(strng::new),
			region: None,
		};
		let actual = p.anthropic_model(req).map(|m| m.to_string());
		assert_eq!(actual.as_deref(), expected);
	}

	#[rstest::rstest]
	#[case::no_region(None, "aiplatform.googleapis.com")]
	#[case::global_region(Some("global"), "aiplatform.googleapis.com")]
	#[case::regional(Some("us-central1"), "us-central1-aiplatform.googleapis.com")]
	fn test_get_host(#[case] region: Option<&str>, #[case] expected: &str) {
		let p = Provider {
			project_id: strng::new("test-project"),
			model: None,
			region: region.map(strng::new),
		};
		assert_eq!(p.get_host(None).as_str(), expected);
	}

	#[test]
	fn test_remove_top_level_output_fields() {
		let mut body: Map<String, Value> = serde_json::from_value(serde_json::json!({
			"model": "claude-sonnet-4-5-20251001",
			"output_config": {"format": "json"},
			"output_format": "markdown",
			"messages": [{"role": "user", "content": "hello"}]
		}))
		.unwrap();
		remove_unsupported_vertex_fields(&mut body);
		assert!(!body.contains_key("output_config"));
		assert!(!body.contains_key("output_format"));
		assert!(body.contains_key("model"));
		assert!(body.contains_key("messages"));
	}

	#[test]
	fn test_output_fields_preserved_when_nested() {
		let mut body: Map<String, Value> = serde_json::from_value(serde_json::json!({
			"messages": [{
				"role": "user",
				"content": "hello",
				"output_config": {"format": "json"},
				"output_format": "markdown"
			}]
		}))
		.unwrap();
		remove_unsupported_vertex_fields(&mut body);
		let msg = body["messages"][0].as_object().unwrap();
		assert!(msg.contains_key("output_config"));
		assert!(msg.contains_key("output_format"));
	}

	#[test]
	fn test_cache_control_scope_removed_recursively() {
		let mut body: Map<String, Value> = serde_json::from_value(serde_json::json!({
			"system": [{
				"type": "text",
				"text": "You are helpful.",
				"cache_control": {"type": "ephemeral", "scope": "turn"}
			}],
			"messages": [{
				"role": "user",
				"content": [{
					"type": "text",
					"text": "hello",
					"cache_control": {"type": "ephemeral", "scope": "session"}
				}]
			}]
		}))
		.unwrap();
		remove_unsupported_vertex_fields(&mut body);
		let sys_cc = body["system"][0]["cache_control"].as_object().unwrap();
		assert_eq!(sys_cc.get("type").unwrap(), "ephemeral");
		assert!(!sys_cc.contains_key("scope"));
		let msg_cc = body["messages"][0]["content"][0]["cache_control"]
			.as_object()
			.unwrap();
		assert_eq!(msg_cc.get("type").unwrap(), "ephemeral");
		assert!(!msg_cc.contains_key("scope"));
	}

	#[test]
	fn test_cache_control_without_scope_untouched() {
		let mut body: Map<String, Value> = serde_json::from_value(serde_json::json!({
			"messages": [{
				"role": "user",
				"content": [{
					"type": "text",
					"text": "hello",
					"cache_control": {"type": "ephemeral"}
				}]
			}]
		}))
		.unwrap();
		let expected = body.clone();
		remove_unsupported_vertex_fields(&mut body);
		assert_eq!(body, expected);
	}

	#[test]
	fn test_cache_control_non_object_untouched() {
		let mut body: Map<String, Value> = serde_json::from_value(serde_json::json!({
			"messages": [{
				"role": "user",
				"content": [{
					"type": "text",
					"text": "hello",
					"cache_control": "enabled"
				}]
			}]
		}))
		.unwrap();
		let expected = body.clone();
		remove_unsupported_vertex_fields(&mut body);
		assert_eq!(body, expected);
	}

	#[test]
	fn test_realistic_anthropic_messages_body() {
		let mut body: Map<String, Value> = serde_json::from_value(serde_json::json!({
			"model": "claude-sonnet-4-5-20251001",
			"max_tokens": 1024,
			"output_config": {"format": "json"},
			"output_format": "markdown",
			"system": [{
				"type": "text",
				"text": "You are a helpful assistant.",
				"cache_control": {"type": "ephemeral", "scope": "turn"}
			}],
			"messages": [
				{
					"role": "user",
					"content": [
						{
							"type": "text",
							"text": "What is 2+2?",
							"cache_control": {"type": "ephemeral", "scope": "session"}
						},
						{
							"type": "image",
							"source": {"type": "base64", "data": "abc"},
							"cache_control": {"type": "ephemeral"}
						}
					]
				},
				{
					"role": "assistant",
					"content": [{"type": "text", "text": "4"}]
				}
			]
		}))
		.unwrap();
		remove_unsupported_vertex_fields(&mut body);

		// Top-level fields removed
		assert!(!body.contains_key("output_config"));
		assert!(!body.contains_key("output_format"));
		// Preserved fields
		assert_eq!(body["max_tokens"], 1024);
		assert_eq!(body["model"], "claude-sonnet-4-5-20251001");

		// System cache_control: scope removed, type kept
		let sys_cc = body["system"][0]["cache_control"].as_object().unwrap();
		assert_eq!(sys_cc.len(), 1);
		assert_eq!(sys_cc["type"], "ephemeral");

		// First user content block: scope removed
		let user_cc = body["messages"][0]["content"][0]["cache_control"]
			.as_object()
			.unwrap();
		assert_eq!(user_cc.len(), 1);
		assert_eq!(user_cc["type"], "ephemeral");

		// Second user content block: no scope, so unchanged (still has type)
		let img_cc = body["messages"][0]["content"][1]["cache_control"]
			.as_object()
			.unwrap();
		assert_eq!(img_cc.len(), 1);
		assert_eq!(img_cc["type"], "ephemeral");

		// Assistant content untouched (no cache_control)
		assert!(
			body["messages"][1]["content"][0]
				.get("cache_control")
				.is_none()
		);
	}
}
