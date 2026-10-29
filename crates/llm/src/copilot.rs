use agent_core::strng;
use agent_core::strng::Strng;
use http::{HeaderMap, HeaderValue, header};

use crate::{ChatFormat, RouteType, apply};

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "CopilotProvider"))]
pub struct Provider {
	/// Model ID to send to GitHub Copilot, overriding the model in the client request.
	#[serde(default, rename = "model", skip_serializing_if = "Option::is_none")]
	pub model_override: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("copilot");
}

impl Provider {
	pub fn is_anthropic_model(request_model: &str) -> bool {
		request_model.to_ascii_lowercase().starts_with("claude-")
	}
	pub fn set_required_headers(headers: &mut HeaderMap) {
		headers.insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static("application/json"),
		);
		headers.insert(
			"editor-version",
			HeaderValue::from_static(concat!("agentgateway/", env!("CARGO_PKG_VERSION"))),
		);
		headers.insert(
			"openai-intent",
			HeaderValue::from_static("conversation-agent"),
		);
		headers.insert(
			"x-github-api-version",
			HeaderValue::from_static("2025-10-01"),
		);
		headers.insert("x-initiator", HeaderValue::from_static("agent"));
		headers.insert(
			"x-interaction-type",
			HeaderValue::from_static("conversation-agent"),
		);
	}
	pub fn supported_formats_for_model(
		request_model: &str,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Vec<ChatFormat> {
		let m = request_model;
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
			m if m.starts_with("claude-") => {
				// Copilot supports Completions even for Anthropic
				// This is enabled so we can do Responses --> Completions [--> Anthropic, within copilot, presumably].
				// If we add native Responses --> Anthropic we should drop this
				vec![ChatFormat::AnthropicMessages, ChatFormat::OpenAICompletions]
			},
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
			m if m.starts_with("microsoft-decision-") => vec![],
			_ => vec![ChatFormat::OpenAICompletions],
		}
	}
}

pub const DEFAULT_HOST_STR: &str = "api.githubcopilot.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub fn path_suffix(route: RouteType) -> &'static str {
	match route {
		RouteType::Embeddings => "/embeddings",
		RouteType::Messages => "/v1/messages",
		RouteType::Models => "/models",
		RouteType::Rerank => "/rerank",
		RouteType::Responses => "/responses",
		RouteType::SystemOne => "/v1/systemone",
		_ => "/chat/completions",
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model_catalog::{Catalog, TestCatalog, tags};

	#[test]
	fn catalog_format_tags_are_case_insensitive() {
		let cat = TestCatalog::new([("grok-2", &[tags::OPENAI_COMPLETIONS][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model("Grok-2", catalog),
			vec![ChatFormat::OpenAICompletions]
		);
	}

	#[test]
	fn catalog_format_tags_override_builtins() {
		// grok-* defaults to Responses; a catalog tag forces Completions instead.
		let cat = TestCatalog::new([("grok-2", &[tags::OPENAI_COMPLETIONS][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model("grok-2", catalog),
			vec![ChatFormat::OpenAICompletions]
		);
	}

	#[test]
	fn decision_model_has_no_chat_formats_and_uses_systemone_path() {
		assert!(Provider::supported_formats_for_model("microsoft-decision-1", None).is_empty());
		assert_eq!(path_suffix(RouteType::SystemOne), "/v1/systemone");
	}

	#[test]
	fn untagged_model_falls_back_to_builtins() {
		let cat = TestCatalog::new([("grok-2", &[][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model("grok-2", catalog),
			vec![ChatFormat::OpenAIResponses]
		);
	}
}
