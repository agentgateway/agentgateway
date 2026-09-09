//! Read handle to the model catalog, passed explicitly so `agent_llm` need not depend on agentgateway.

use std::collections::BTreeSet;
use std::sync::Arc;

/// Well-known chat-format tags, mirroring [`crate::ChatFormat::tag`].
pub mod tags {
	// Request formats.

	/// Accepts Anthropic's native Messages request format.
	pub const ANTHROPIC_MESSAGES: &str = "anthropic_messages";
	/// Accepts Amazon Bedrock's Converse request format.
	pub const BEDROCK_CONVERSE: &str = "bedrock_converse";
	/// Accepts OpenAI's Chat Completions request format.
	pub const OPENAI_COMPLETIONS: &str = "openai_completions";
	/// Accepts OpenAI's Responses request format.
	pub const OPENAI_RESPONSES: &str = "openai_responses";
	/// Accepts Google's native Gemini request format.
	pub const VERTEX_GEMINI: &str = "vertex_gemini";

	// Anthropic thinking capabilities.

	/// Supports Anthropic adaptive thinking with an effort level.
	pub const ADAPTIVE_THINKING: &str = "adaptive_thinking";
	/// Supports Anthropic legacy thinking with an explicit token budget.
	pub const LEGACY_THINKING: &str = "legacy_thinking";
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AnthropicThinkingCapabilities {
	pub adaptive: bool,
	pub legacy: bool,
}

pub(crate) fn anthropic_thinking_capabilities(
	model: &str,
	catalog: Catalog<'_>,
) -> AnthropicThinkingCapabilities {
	let tags = catalog.and_then(|catalog| catalog.get_model_tags(model));
	AnthropicThinkingCapabilities {
		adaptive: tags
			.as_ref()
			.is_some_and(|tags| tags.contains(tags::ADAPTIVE_THINKING)),
		legacy: tags
			.as_ref()
			.is_some_and(|tags| tags.contains(tags::LEGACY_THINKING)),
	}
}

/// Read handle to the model catalog; implemented in agentgateway on the cost `ModelCatalog`.
pub trait ModelCatalogHandle: Send + Sync {
	fn get_model_tags(&self, model_id: &str) -> Option<Arc<BTreeSet<String>>>;
}

/// Borrowed catalog handle threaded through the request path; `None` when no catalog is loaded.
pub type Catalog<'a> = Option<&'a dyn ModelCatalogHandle>;

/// Test double answering from a fixed `model -> tags` map.
#[cfg(test)]
pub(crate) struct TestCatalog {
	models: std::collections::HashMap<String, BTreeSet<String>>,
}

#[cfg(test)]
impl TestCatalog {
	pub(crate) fn new<'a, M: IntoIterator<Item = (&'a str, &'a [&'a str])>>(models: M) -> Self {
		Self {
			models: models
				.into_iter()
				.map(|(m, tags)| (m.to_string(), tags.iter().map(|s| s.to_string()).collect()))
				.collect(),
		}
	}
}

#[cfg(test)]
impl ModelCatalogHandle for TestCatalog {
	fn get_model_tags(&self, model_id: &str) -> Option<Arc<BTreeSet<String>>> {
		self.models.get(model_id).map(|t| Arc::new(t.clone()))
	}
}
