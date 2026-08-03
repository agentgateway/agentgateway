use agent_core::strng;
use agent_core::strng::Strng;

use crate::{ChatFormat, RouteType, apply};

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
	use serde_json::{Value, json};

	use super::prepare_responses_request;
	use crate::types::responses;

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
}
