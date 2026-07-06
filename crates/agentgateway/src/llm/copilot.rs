use agent_core::strng;
use agent_core::strng::Strng;

use crate::llm::{ChatFormat, RouteType};
use crate::*;

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "CopilotProvider"))]
pub struct Provider {
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
	pub(super) fn supported_formats_for_model(request_model: Option<&str>) -> Vec<ChatFormat> {
		let Some(m) = request_model else {
			// If we have no model not much we can do...
			return vec![ChatFormat::OpenAICompletions];
		};
		// Truth table from `curl https://api.githubcopilot.com/models -H "Authorization: Bearer ghu_..." | '.data[] | {id,supported_endpoints}'`
		match m {
			m if m.starts_with("claude-") => {
				// Copilot supports Completions even for Anthropic
				// This is enabled so we can do Responses --> Completions [--> Anthropic, within copilot, presumably].
				// If we add native Responses --> Anthropic we should drop this
				vec![ChatFormat::AnthropicMessages, ChatFormat::OpenAICompletions]
			},
			m if m.starts_with("mai-") => {
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
