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
	pub fn supported_formats_for_model(request_model: Option<&str>) -> Vec<ChatFormat> {
		if Self::is_anthropic_model(request_model) {
			return vec![ChatFormat::AnthropicMessages];
		}
		let Some(m) = request_model else {
			// If we have no model not much we can do...
			return vec![ChatFormat::OpenAICompletions];
		};
		let normalized_model = m.to_ascii_lowercase();
		// Truth table from `curl https://api.githubcopilot.com/models -H "Authorization: Bearer ghu_..." | '.data[] | {id,supported_endpoints}'`
		match normalized_model.as_str() {
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

/// Applies confirmed Copilot compatibility rules to a Responses request before conversion.
pub fn prepare_responses_request(
	request: &crate::types::responses::Request,
) -> crate::types::responses::Request {
	let mut request = request.clone();
	let automatic_tool_choice = request
		.rest
		.get("tool_choice")
		.is_none_or(|choice| choice.as_str() == Some("auto"));
	if automatic_tool_choice
		&& let Some(tools) = request
			.rest
			.get_mut("tools")
			.and_then(serde_json::Value::as_array_mut)
	{
		let cache_only_search = serde_json::json!({
			"type": "web_search",
			"external_web_access": false,
		});
		tools.retain(|tool| tool != &cache_only_search);
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

	use super::{prepare_messages_body, prepare_messages_headers, prepare_responses_request};
	use crate::types::responses;

	const CLAUDE_CODE_BETA_HEADER: &str =
		"claude-code-20250219,advisor-tool-2026-03-01,effort-2025-11-24";

	fn request(tools: Value, tool_choice: Option<Value>) -> responses::Request {
		let mut request = json!({
			"input": "work",
			"model": "claude-sonnet-5",
			"tools": tools,
		});
		if let Some(tool_choice) = tool_choice {
			request["tool_choice"] = tool_choice;
		}
		serde_json::from_value(request).expect("valid Responses request")
	}

	fn tools(request: &responses::Request) -> &Vec<Value> {
		request.rest["tools"]
			.as_array()
			.expect("Responses tools array")
	}

	#[test]
	fn policy_removes_only_cache_only_hosted_search_for_automatic_choice() {
		for tool_choice in [None, Some(json!("auto"))] {
			let request = request(
				json!([
					{"type":"function","name":"web_search","parameters":{"type":"object"}},
					{"type":"web_search","external_web_access":false},
					{"type":"function","name":"weather","parameters":{"type":"object"}}
				]),
				tool_choice,
			);
			let prepared = prepare_responses_request(&request);

			assert_eq!(
				tools(&request).len(),
				3,
				"the caller request must not be mutated"
			);
			assert_eq!(
				tools(&prepared),
				&json!([
					{"type":"function","name":"web_search","parameters":{"type":"object"}},
					{"type":"function","name":"weather","parameters":{"type":"object"}}
				])
				.as_array()
				.expect("expected tools")
				.clone()
			);
		}
	}

	#[test]
	fn policy_preserves_hosted_search_outside_the_safe_case() {
		let cases = [
			(
				json!({"type":"web_search","external_web_access":true}),
				None,
			),
			(json!({"type":"web_search"}), None),
			(
				json!({"type":"web_search","external_web_access":false,"unexpected":true}),
				None,
			),
			(
				json!({"type":"web_search","external_web_access":"false"}),
				None,
			),
			(
				json!({"type":"web_search","external_web_access":false}),
				Some(json!({"type":"web_search"})),
			),
			(
				json!({"type":"web_search","external_web_access":false}),
				Some(json!("required")),
			),
			(
				json!({"type":"web_search","external_web_access":false}),
				Some(json!("none")),
			),
		];

		for (tool, tool_choice) in cases {
			let request = request(json!([tool]), tool_choice);
			let prepared = prepare_responses_request(&request);
			assert_eq!(tools(&prepared), tools(&request));
		}
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
}
