use agent_core::strng;
use agent_core::strng::Strng;
use http::{HeaderMap, HeaderValue};

use crate::{AIError, ChatFormat, RouteType, apply};

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "CopilotProvider"))]
pub struct Provider {
	/// Model ID to send to GitHub Copilot, overriding the model in the client request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("copilot");
}

impl Provider {
	pub fn is_anthropic_model(request_model: Option<&str>) -> bool {
		request_model.is_some_and(|model| model.to_ascii_lowercase().starts_with("claude-"))
	}
	pub fn supported_formats_for_model(
		request_model: Option<&str>,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Vec<ChatFormat> {
		let Some(m) = request_model else {
			// If we have no model not much we can do...
			return vec![ChatFormat::OpenAICompletions];
		};
		let normalized_model = m.to_ascii_lowercase();
		// TODO: also support endpoint parsing from copilot models and add a tool to grab specific setups in agctl
		if let Some(tags) = catalog.and_then(|c| c.get_model_tags(&normalized_model)) {
			let formats: Vec<ChatFormat> = [
				ChatFormat::OpenAICompletions,
				ChatFormat::OpenAIResponses,
				ChatFormat::AnthropicMessages,
				ChatFormat::BedrockConverse,
				ChatFormat::VertexGemini,
			]
			.into_iter()
			.filter(|f| tags.contains(f.tag()))
			.collect();
			if !formats.is_empty() {
				tracing::debug!(model = %m, ?formats, "copilot formats from modelcatalog tags");
				return formats;
			}
		}
		// Truth table from `curl https://api.githubcopilot.com/models -H "Authorization: Bearer ghu_..." | '.data[] | {id,supported_endpoints}'`
		match normalized_model.as_str() {
			m if m.starts_with("claude-") => vec![ChatFormat::AnthropicMessages],
			m if m.starts_with("grok-") || m.starts_with("mai-") => {
				vec![ChatFormat::OpenAIResponses]
			},
			m if m.starts_with("gemini-") => {
				vec![ChatFormat::OpenAICompletions]
			},
			m if m.starts_with("gpt-3") || m.starts_with("gpt-4") => {
				vec![ChatFormat::OpenAICompletions]
			},
			"gpt-5.4" | "gpt-5-mini" => {
				vec![ChatFormat::OpenAICompletions, ChatFormat::OpenAIResponses]
			},
			m if m.starts_with("gpt-") => {
				vec![ChatFormat::OpenAIResponses]
			},
			_ => vec![ChatFormat::OpenAICompletions],
		}
	}
}

pub const DEFAULT_HOST_STR: &str = "api.githubcopilot.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

const UNSUPPORTED_BETA_HEADER: &str = "advisor-tool-2026-03-01";

/// Applies Copilot's Messages header policy without changing supported Anthropic beta entries.
pub fn prepare_messages_headers(headers: &mut HeaderMap) {
	headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
	let mut kept = Vec::new();
	for value in headers.get_all("anthropic-beta") {
		let Ok(header_str) = value.to_str() else {
			continue;
		};
		for feature in header_str.split(',') {
			let trimmed = feature.trim();
			if !trimmed.is_empty() && trimmed != UNSUPPORTED_BETA_HEADER {
				kept.push(trimmed.to_string());
			}
		}
	}
	headers.remove("anthropic-beta");
	if !kept.is_empty()
		&& let Ok(value) = HeaderValue::from_str(&kept.join(","))
	{
		headers.insert("anthropic-beta", value);
	}
}

/// Removes Messages fields that Copilot's Anthropic-compatible endpoint rejects.
pub fn prepare_messages_body(body: Vec<u8>) -> Result<Vec<u8>, AIError> {
	let mut body: serde_json::Map<String, serde_json::Value> =
		serde_json::from_slice(&body).map_err(AIError::RequestParsing)?;
	body.remove("context_management");
	serde_json::to_vec(&body).map_err(AIError::RequestMarshal)
}

/// Drops the cache-only hosted-search declaration that Copilot treats as a no-op.
pub fn prepare_responses_request(
	mut request: crate::types::responses::Request,
) -> crate::types::responses::Request {
	let automatic_tool_choice = request
		.rest
		.get("tool_choice")
		.is_none_or(|choice| choice.as_str() == Some("auto"));
	let removed_all_tools = if automatic_tool_choice
		&& let Some(tools) = request
			.rest
			.get_mut("tools")
			.and_then(serde_json::Value::as_array_mut)
	{
		let had_tools = !tools.is_empty();
		let cache_only_search = serde_json::json!({
			"type": "web_search",
			"external_web_access": false,
		});
		tools.retain(|tool| tool != &cache_only_search);
		had_tools && tools.is_empty()
	} else {
		false
	};
	if removed_all_tools && let Some(rest) = request.rest.as_object_mut() {
		rest.remove("tool_choice");
	}
	request
}

pub fn path_suffix(route: RouteType) -> &'static str {
	match route {
		RouteType::Messages => "/v1/messages",
		RouteType::Responses => "/responses",
		RouteType::Embeddings => "/embeddings",
		RouteType::Rerank => "/rerank",
		RouteType::Models => "/models",
		_ => "/chat/completions",
	}
}

#[cfg(test)]
mod tests {
	use http::{HeaderMap, HeaderValue};
	use serde_json::{Value, json};

	use super::*;
	use crate::model_catalog::{Catalog, TestCatalog, tags};

	#[test]
	fn catalog_format_tags_override_builtins() {
		// grok-* defaults to Responses; a catalog tag forces Completions instead.
		let cat = TestCatalog::new([("grok-2", &[tags::OPENAI_COMPLETIONS][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model(Some("grok-2"), catalog),
			vec![ChatFormat::OpenAICompletions]
		);
	}

	#[test]
	fn catalog_format_tags_are_case_insensitive() {
		let cat = TestCatalog::new([("grok-2", &[tags::OPENAI_COMPLETIONS][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model(Some("Grok-2"), catalog),
			vec![ChatFormat::OpenAICompletions]
		);
	}

	#[test]
	fn untagged_model_falls_back_to_builtins() {
		let cat = TestCatalog::new([("grok-2", &[][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model(Some("grok-2"), catalog),
			vec![ChatFormat::OpenAIResponses]
		);
	}

	const CLAUDE_CODE_BETA_HEADER: &str =
		"claude-code-20250219,advisor-tool-2026-03-01,effort-2025-11-24";

	fn responses_request(
		tools: Value,
		tool_choice: Option<Value>,
	) -> crate::types::responses::Request {
		let mut request = json!({"input": "say hi", "tools": tools});
		if let Some(tool_choice) = tool_choice {
			request["tool_choice"] = tool_choice;
		}
		serde_json::from_value(request).expect("Responses request")
	}

	fn response_tools(request: &crate::types::responses::Request) -> &[Value] {
		request.rest["tools"]
			.as_array()
			.expect("Responses tools array")
	}

	#[test]
	fn messages_header_policy_sets_version_and_filters_only_unsupported_entries() {
		let mut headers = HeaderMap::new();
		headers.append(
			"anthropic-beta",
			HeaderValue::from_static("advisor-tool-2026-03-01"),
		);
		headers.append(
			"anthropic-beta",
			HeaderValue::from_static(CLAUDE_CODE_BETA_HEADER),
		);

		prepare_messages_headers(&mut headers);

		assert_eq!(headers["anthropic-version"], "2023-06-01");
		assert_eq!(
			headers["anthropic-beta"],
			"claude-code-20250219,effort-2025-11-24"
		);
	}

	#[test]
	fn messages_header_policy_removes_beta_header_when_nothing_survives() {
		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			HeaderValue::from_static("advisor-tool-2026-03-01"),
		);

		prepare_messages_headers(&mut headers);

		assert!(!headers.contains_key("anthropic-beta"));
	}

	#[test]
	fn messages_body_policy_removes_only_context_management() {
		let body = serde_json::to_vec(&json!({
			"model": "claude-sonnet-5",
			"messages": [{"role": "user", "content": "hi"}],
			"context_management": {"edits": [{"type": "clear_tool_uses_20250919"}]},
			"some_future_anthropic_field": "preserved"
		}))
		.expect("Messages body");

		let prepared: Value =
			serde_json::from_slice(&prepare_messages_body(body).expect("Copilot Messages body policy"))
				.expect("prepared Messages body");

		assert!(prepared.get("context_management").is_none());
		assert_eq!(prepared["some_future_anthropic_field"], "preserved");
	}

	#[test]
	fn responses_policy_removes_orphaned_automatic_tool_choice() {
		let request = responses_request(
			json!([{"type": "web_search", "external_web_access": false}]),
			Some(json!("auto")),
		);

		let prepared = prepare_responses_request(request);

		assert!(response_tools(&prepared).is_empty());
		assert!(prepared.rest.get("tool_choice").is_none());
	}

	#[test]
	fn responses_policy_removes_only_cache_only_search_for_automatic_choice() {
		for tool_choice in [None, Some(json!("auto"))] {
			let request = responses_request(
				json!([
					{"type": "function", "name": "web_search", "parameters": {"type": "object"}},
					{"type": "web_search", "external_web_access": false},
					{"type": "function", "name": "weather", "parameters": {"type": "object"}}
				]),
				tool_choice.clone(),
			);

			let prepared = prepare_responses_request(request);

			assert_eq!(
				response_tools(&prepared),
				json!([
					{"type": "function", "name": "web_search", "parameters": {"type": "object"}},
					{"type": "function", "name": "weather", "parameters": {"type": "object"}}
				])
				.as_array()
				.expect("expected tools")
			);
			assert_eq!(prepared.rest.get("tool_choice"), tool_choice.as_ref());
		}
	}

	#[test]
	fn responses_policy_preserves_tools_outside_the_safe_case() {
		let cases = [
			(
				json!({"type": "web_search", "external_web_access": true}),
				None,
			),
			(json!({"type": "web_search"}), None),
			(
				json!({"type": "web_search", "external_web_access": false, "unexpected": true}),
				None,
			),
			(
				json!({"type": "web_search", "external_web_access": "false"}),
				None,
			),
			(
				json!({"type": "file_search", "vector_store_ids": ["vs_1"]}),
				None,
			),
			(
				json!({"type": "web_search", "external_web_access": false}),
				Some(json!({"type": "web_search"})),
			),
			(
				json!({"type": "web_search", "external_web_access": false}),
				Some(json!("required")),
			),
			(
				json!({"type": "web_search", "external_web_access": false}),
				Some(json!("none")),
			),
		];

		for (tool, tool_choice) in cases {
			let request = responses_request(json!([tool]), tool_choice);
			let expected = response_tools(&request).to_vec();

			let prepared = prepare_responses_request(request);

			assert_eq!(response_tools(&prepared), expected);
		}
	}
}
