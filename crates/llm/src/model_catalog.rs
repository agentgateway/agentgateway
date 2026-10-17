//! Read handle to the model catalog, passed explicitly so `agent_llm` need not depend on agentgateway.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::types::completions::typed::ReasoningEffort;

/// Well-known catalog tags: AWS Bedrock endpoint tags, plus format tags mirroring [`crate::ChatFormat::tag`].
pub mod tags {
	// AWS Bedrock endpoint tags.

	/// AWS Bedrock model is served on the Runtime (Converse/Invoke) endpoint.
	pub const RUNTIME: &str = "runtime";
	/// AWS Bedrock model is served on the Mantle endpoint.
	pub const MANTLE: &str = "mantle";

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
	fn model_has_tag(&self, model_id: &str, tag: &str) -> bool;
	fn get_model_tags(&self, model_id: &str) -> Option<Arc<BTreeSet<String>>>;
	/// `None` when the catalog doesn't know the model; empty when it advertises no effort levels
	/// this crate recognizes, which doesn't mean the model takes none
	fn reasoning_efforts(&self, provider: &str, model_id: &str) -> Option<Vec<ReasoningEffort>>;
}

/// Borrowed catalog handle threaded through the request path; `None` when no catalog is loaded.
pub type Catalog<'a> = Option<&'a dyn ModelCatalogHandle>;

/// Round up to a supported effort, capped at the highest available
pub(crate) fn nearest_effort(
	wanted: &ReasoningEffort,
	accepted: &[ReasoningEffort],
) -> Option<ReasoningEffort> {
	accepted
		.iter()
		.filter(|e| rank(e) >= rank(wanted))
		.min_by_key(|e| rank(e))
		.or_else(|| accepted.iter().max_by_key(|e| rank(e)))
		.cloned()
}

fn rank(effort: &ReasoningEffort) -> u8 {
	match effort {
		ReasoningEffort::None => 0,
		ReasoningEffort::Minimal => 1,
		ReasoningEffort::Low => 2,
		ReasoningEffort::Medium => 3,
		ReasoningEffort::High => 4,
		ReasoningEffort::Xhigh => 5,
		ReasoningEffort::Max => 6,
	}
}

/// Test double answering from a fixed `model -> tags` map.
#[cfg(test)]
pub(crate) struct TestCatalog {
	models: std::collections::HashMap<String, BTreeSet<String>>,
	efforts: std::collections::HashMap<(String, String), Vec<ReasoningEffort>>,
}

#[cfg(test)]
impl TestCatalog {
	pub(crate) fn new<'a, M: IntoIterator<Item = (&'a str, &'a [&'a str])>>(models: M) -> Self {
		Self {
			models: models
				.into_iter()
				.map(|(m, tags)| (m.to_string(), tags.iter().map(|s| s.to_string()).collect()))
				.collect(),
			efforts: Default::default(),
		}
	}

	pub(crate) fn with_efforts(
		mut self,
		provider: &str,
		model: &str,
		efforts: Vec<ReasoningEffort>,
	) -> Self {
		self
			.efforts
			.insert((provider.into(), model.into()), efforts);
		self
	}
}

#[cfg(test)]
impl ModelCatalogHandle for TestCatalog {
	fn model_has_tag(&self, model_id: &str, tag: &str) -> bool {
		self.models.get(model_id).is_some_and(|t| t.contains(tag))
	}

	fn get_model_tags(&self, model_id: &str) -> Option<Arc<BTreeSet<String>>> {
		self.models.get(model_id).map(|t| Arc::new(t.clone()))
	}

	fn reasoning_efforts(&self, provider: &str, model_id: &str) -> Option<Vec<ReasoningEffort>> {
		self
			.efforts
			.get(&(provider.into(), model_id.into()))
			.cloned()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[rstest::rstest]
	#[case::exact("low", &["low", "high"], Some("low"))]
	#[case::rounds_up("minimal", &["low", "high"], Some("low"))]
	#[case::rounds_up_across_a_gap("low", &["minimal", "high"], Some("high"))]
	#[case::unsorted("minimal", &["high", "low"], Some("low"))]
	#[case::falls_back_to_the_highest("max", &["medium", "low", "high"], Some("high"))]
	#[case::empty("low", &[], None)]
	fn nearest_effort_rounds_up(
		#[case] wanted: &str,
		#[case] accepted: &[&str],
		#[case] expected: Option<&str>,
	) {
		let effort = |e: &str| -> ReasoningEffort { serde_json::from_value(e.into()).unwrap() };
		let accepted: Vec<_> = accepted.iter().map(|e| effort(e)).collect();
		assert_eq!(
			nearest_effort(&effort(wanted), &accepted),
			expected.map(effort)
		);
	}
}
