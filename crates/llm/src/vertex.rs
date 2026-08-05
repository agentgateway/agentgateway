use agent_core::strng;
use agent_core::strng::Strng;
use serde_json::{Map, Value};

use crate::{AIError, RouteType, apply};

const ANTHROPIC_VERSION: &str = "vertex-2023-10-16";

/// Host for the Discovery Engine ranking endpoint used by rerank. Distinct from the `aiplatform`
/// host used for chat/embeddings; defined once so the request authority/Host header and the actual
/// TCP/TLS connection target stay in sync.
pub const DISCOVERY_ENGINE_HOST: Strng = strng::literal!("discoveryengine.googleapis.com");

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "VertexProvider"))]
pub struct Provider {
	/// Model ID to send to Vertex AI, overriding the model in the client request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	/// Vertex AI region. Special values: `global` uses the global endpoint, while `us` and `eu`
	/// use restricted multi-region endpoints. Other values are treated as regional locations.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub region: Option<Strng>,
	/// Google Cloud project ID for Vertex AI.
	pub project_id: Strng,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("gcp.vertex_ai");
}

pub fn prepare_anthropic_message_body(body: Vec<u8>) -> Result<Vec<u8>, AIError> {
	prepare_anthropic_body(body, |b| {
		b.remove("model");
	})
}

impl Provider {
	fn configured_model<'a>(&'a self, request_model: Option<&'a str>) -> Option<&'a str> {
		self.model.as_deref().or(request_model)
	}

	pub fn is_anthropic_model(&self, request_model: Option<&str>) -> bool {
		self.anthropic_model(request_model).is_some()
	}

	pub fn is_gemini_model(&self, request_model: Option<&str>) -> bool {
		self.gemini_model(request_model).is_some()
	}

	pub fn prepare_anthropic_message_body(&self, body: Vec<u8>) -> Result<Vec<u8>, AIError> {
		prepare_anthropic_message_body(body)
	}

	pub fn prepare_anthropic_count_tokens_body(&self, body: Vec<u8>) -> Result<Vec<u8>, AIError> {
		prepare_anthropic_body(body, |b| {
			if let Some(Value::String(model)) = b.get("model") {
				let normalized = self
					.configured_model(Some(model))
					.map(|s| s.to_string())
					.unwrap_or_else(|| model.clone());
				b.insert("model".to_string(), Value::String(normalized));
			}
		})
	}
}

/// Shared pipeline for Vertex Anthropic requests: parse, inject version,
/// apply caller-specific model handling, strip unsupported fields, serialize.
fn prepare_anthropic_body(
	body: Vec<u8>,
	apply: impl FnOnce(&mut Map<String, Value>),
) -> Result<Vec<u8>, AIError> {
	let mut body: Map<String, Value> =
		serde_json::from_slice(&body).map_err(AIError::RequestParsing)?;
	body.insert(
		"anthropic_version".to_string(),
		Value::String(ANTHROPIC_VERSION.to_string()),
	);
	apply(&mut body);
	remove_unsupported_vertex_fields(&mut body);
	serde_json::to_vec(&body).map_err(AIError::RequestMarshal)
}

impl Provider {
	pub fn get_path_for_model(
		&self,
		route: RouteType,
		request_model: Option<&str>,
		streaming: bool,
		native_gemini: bool,
	) -> Strng {
		let location = self
			.region
			.clone()
			.unwrap_or_else(|| strng::literal!("global"));

		match (
			route,
			self.anthropic_model(request_model),
			self.gemini_model(request_model),
		) {
			(RouteType::AnthropicTokenCount, _, _) => {
				strng::format!(
					"/v1/projects/{}/locations/{}/publishers/anthropic/models/count-tokens:rawPredict",
					self.project_id,
					location
				)
			},
			(RouteType::Rerank, _, _) => {
				strng::format!(
					"/v1/projects/{}/locations/{}/rankingConfigs/default_ranking_config:rank",
					self.project_id,
					location
				)
			},
			(RouteType::Embeddings, _, _) => {
				let model = self.configured_model(request_model).unwrap_or_default();
				strng::format!(
					"/v1/projects/{}/locations/{}/publishers/google/models/{}:predict",
					self.project_id,
					location,
					model
				)
			},
			(_, Some(model), _) => {
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

			// `?alt=sse` is required on the streaming endpoint; without it Vertex returns a
			// JSON array rather than an SSE stream.
			(RouteType::Completions, None, Some(model)) if native_gemini => {
				let method = if streaming {
					"streamGenerateContent?alt=sse"
				} else {
					"generateContent"
				};
				strng::format!(
					"/v1/projects/{}/locations/{}/publishers/google/models/{}:{}",
					self.project_id,
					location,
					model,
					method
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

	pub fn get_host(&self, route_type: RouteType) -> Strng {
		// Rerank is served by the Discovery Engine ranking endpoint, not the aiplatform host. Deciding
		// it here keeps the request authority (Host header) and the TCP/TLS connection target in sync.
		if route_type == RouteType::Rerank {
			return DISCOVERY_ENGINE_HOST;
		}
		match &self.region {
			None => strng::literal!("aiplatform.googleapis.com"),
			Some(region) if region == "global" => strng::literal!("aiplatform.googleapis.com"),
			Some(region) if region == "us" || region == "eu" => {
				strng::format!("aiplatform.{region}.rep.googleapis.com")
			},
			Some(region) => strng::format!("{region}-aiplatform.googleapis.com"),
		}
	}

	fn gemini_model<'a>(&'a self, request_model: Option<&'a str>) -> Option<Strng> {
		let model = self.configured_model(request_model)?;

		let stripped: &str = model
			.split_once("publishers/google/models/")
			.map(|(_, m)| m)
			.or_else(|| model.strip_prefix("models/"))
			.or_else(|| model.strip_prefix("google/"))
			.unwrap_or(model);

		// Embedding models can share the gemini- prefix (e.g. gemini-embedding-001) but
		// route via the Embeddings arm, not :generateContent.
		const EMBEDDING_PREFIXES: &[&str] = &[
			"text-embedding-",
			"gemini-embedding-",
			"text-multilingual-embedding-",
			"textembedding-",
			"multimodalembedding",
		];
		if EMBEDDING_PREFIXES.iter().any(|p| stripped.starts_with(p)) {
			return None;
		}

		if stripped.starts_with("gemini-") || stripped.starts_with("gemini@") {
			Some(strng::new(stripped))
		} else {
			None
		}
	}

	fn anthropic_model<'a>(&'a self, request_model: Option<&'a str>) -> Option<Strng> {
		let model = self.configured_model(request_model)?;

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
	// output_format is the deprecated predecessor of output_config.format; strip it.
	// output_config is intentionally preserved: Vertex supports it for structured outputs
	// (format.json_schema) and for adaptive thinking depth control (effort).
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
	#[case::raw_flash(None, Some("gemini-2.5-flash"), Some("gemini-2.5-flash"))]
	#[case::raw_pro(None, Some("gemini-3-pro"), Some("gemini-3-pro"))]
	#[case::at_separator(None, Some("gemini@001"), Some("gemini@001"))]
	#[case::strip_publishers_prefix(
		Some("publishers/google/models/gemini-2.5-pro"),
		None,
		Some("gemini-2.5-pro")
	)]
	#[case::strip_models_prefix(None, Some("models/gemini-2.5-flash"), Some("gemini-2.5-flash"))]
	#[case::strip_google_prefix(None, Some("google/gemini-2.5-flash"), Some("gemini-2.5-flash"))]
	#[case::claude_rejected(None, Some("claude-sonnet-4-5"), None)]
	#[case::gpt_rejected(None, Some("gpt-4o"), None)]
	#[case::text_embedding_excluded(None, Some("text-embedding-005"), None)]
	#[case::gemini_embedding_excluded(None, Some("gemini-embedding-001"), None)]
	#[case::multilingual_embedding_excluded(None, Some("text-multilingual-embedding-002"), None)]
	#[case::textembedding_legacy_excluded(None, Some("textembedding-gecko@003"), None)]
	#[case::multimodal_embedding_excluded(None, Some("multimodalembedding@001"), None)]
	#[case::embedding_under_models_prefix(None, Some("models/gemini-embedding-001"), None)]
	#[case::embedding_under_publishers_prefix(
		None,
		Some("publishers/google/models/text-embedding-005"),
		None
	)]
	#[case::provider_model_precedence(
		Some("gemini-2.5-flash"),
		Some("claude-sonnet-4-5"),
		Some("gemini-2.5-flash")
	)]
	#[case::no_model_anywhere(None, None, None)]
	fn test_gemini_model_normalization(
		#[case] provider: Option<&str>,
		#[case] req: Option<&str>,
		#[case] expected: Option<&str>,
	) {
		let p = Provider {
			project_id: strng::new("test-project"),
			model: provider.map(strng::new),
			region: None,
		};
		let actual = p.gemini_model(req).map(|m| m.to_string());
		assert_eq!(actual.as_deref(), expected);
	}

	#[test]
	fn test_is_gemini_model_consistency_with_optional() {
		let p = Provider {
			project_id: strng::new("test-project"),
			model: None,
			region: None,
		};
		assert!(p.is_gemini_model(Some("gemini-2.5-flash")));
		assert!(!p.is_gemini_model(Some("claude-sonnet-4-5")));
		assert!(!p.is_gemini_model(Some("gemini-embedding-001")));
		assert!(!p.is_gemini_model(None));
	}

	#[test]
	fn test_gemini_and_anthropic_heuristics_are_disjoint() {
		let p = Provider {
			project_id: strng::new("test-project"),
			model: None,
			region: None,
		};
		for m in [
			"gemini-2.5-flash",
			"gemini-3-pro",
			"gemini@001",
			"claude-sonnet-4-5",
			"claude-haiku-4-5-20251001",
		] {
			let g = p.is_gemini_model(Some(m));
			let a = p.is_anthropic_model(Some(m));
			assert!(
				!(g && a),
				"{m} matched both Gemini and Anthropic heuristics"
			);
		}
	}

	#[rstest::rstest]
	#[case::flash(
		None,
		Some("gemini-2.5-flash"),
		false,
		"/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
	)]
	#[case::flash_streaming(
		None,
		Some("gemini-2.5-flash"),
		true,
		"/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
	)]
	#[case::pro_regional(
		Some("us-central1"),
		Some("gemini-3-pro"),
		false,
		"/v1/projects/p/locations/us-central1/publishers/google/models/gemini-3-pro:generateContent"
	)]
	#[case::path_prefix_normalized(
		None,
		Some("publishers/google/models/gemini-2.5-flash"),
		false,
		"/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
	)]
	#[case::models_prefix_normalized(
		None,
		Some("models/gemini-2.5-flash"),
		false,
		"/v1/projects/p/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
	)]
	fn test_get_path_for_gemini_native(
		#[case] region: Option<&str>,
		#[case] req_model: Option<&str>,
		#[case] streaming: bool,
		#[case] expected: &str,
	) {
		let p = Provider {
			project_id: strng::new("p"),
			model: None,
			region: region.map(strng::new),
		};
		let got = p.get_path_for_model(RouteType::Completions, req_model, streaming, true);
		assert_eq!(got.as_str(), expected);
	}

	#[rstest::rstest]
	#[case::non_streaming(false)]
	#[case::streaming(true)]
	fn test_gemini_compat_translation_uses_compat_shim(#[case] streaming: bool) {
		let p = Provider {
			project_id: strng::new("p"),
			model: None,
			region: None,
		};
		let got = p.get_path_for_model(
			RouteType::Completions,
			Some("gemini-2.5-flash"),
			streaming,
			false,
		);
		assert_eq!(
			got.as_str(),
			"/v1/projects/p/locations/global/endpoints/openapi/chat/completions",
			"non-native Gemini translations use the compat shim"
		);
	}

	#[rstest::rstest]
	#[case::claude_still_routes_anthropic(
		Some("claude-sonnet-4-5"),
		false,
		"/v1/projects/p/locations/global/publishers/anthropic/models/claude-sonnet-4-5:rawPredict"
	)]
	#[case::claude_streaming_anthropic(
		Some("claude-sonnet-4-5"),
		true,
		"/v1/projects/p/locations/global/publishers/anthropic/models/claude-sonnet-4-5:streamRawPredict"
	)]
	#[case::non_gemini_falls_to_compat(
		Some("gpt-4o"),
		false,
		"/v1/projects/p/locations/global/endpoints/openapi/chat/completions"
	)]
	fn test_get_path_non_gemini_unchanged(
		#[case] req_model: Option<&str>,
		#[case] streaming: bool,
		#[case] expected: &str,
	) {
		let p = Provider {
			project_id: strng::new("p"),
			model: None,
			region: None,
		};
		let got = p.get_path_for_model(RouteType::Completions, req_model, streaming, false);
		assert_eq!(got.as_str(), expected);
	}

	#[test]
	fn test_embedding_route_takes_precedence_over_gemini_arm() {
		let p = Provider {
			project_id: strng::new("p"),
			model: None,
			region: None,
		};
		let path = p.get_path_for_model(
			RouteType::Embeddings,
			Some("gemini-embedding-001"),
			false,
			false,
		);
		assert!(
			path.as_str().ends_with(":predict"),
			"expected :predict, got {path}"
		);
		assert!(
			!path.as_str().contains(":generateContent"),
			"embedding route must not produce :generateContent, got {path}"
		);
	}

	#[rstest::rstest]
	#[case::no_region(None, "aiplatform.googleapis.com")]
	#[case::global_region(Some("global"), "aiplatform.googleapis.com")]
	#[case::multi_region(Some("us"), "aiplatform.us.rep.googleapis.com")]
	#[case::regional(Some("us-central1"), "us-central1-aiplatform.googleapis.com")]
	fn test_get_host(#[case] region: Option<&str>, #[case] expected: &str) {
		let p = Provider {
			project_id: strng::new("test-project"),
			model: None,
			region: region.map(strng::new),
		};
		assert_eq!(p.get_host(RouteType::Completions).as_str(), expected);
	}

	#[test]
	fn test_output_format_removed_output_config_preserved() {
		let mut body: Map<String, Value> = serde_json::from_value(serde_json::json!({
			"model": "claude-sonnet-4-5-20251001",
			"output_config": {
				"format": {
					"type": "json_schema",
					"schema": {
						"type": "object",
						"properties": {"answer": {"type": "integer"}},
						"required": ["answer"],
						"additionalProperties": false
					}
				}
			},
			"output_format": "markdown",
			"messages": [{"role": "user", "content": "hello"}]
		}))
		.unwrap();
		remove_unsupported_vertex_fields(&mut body);
		assert!(body.contains_key("output_config"));
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

		// output_format removed; output_config preserved for structured outputs.
		assert!(body.contains_key("output_config"));
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
