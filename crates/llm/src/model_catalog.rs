//! Read handle to the model catalog, passed explicitly so `agent_llm` need not depend on agentgateway.

use std::collections::BTreeSet;
use std::sync::Arc;

/// Well-known chat-format tags, mirroring [`crate::ChatFormat::tag`].
pub mod tags {
	pub const OPENAI_COMPLETIONS: &str = "openai_completions";
	pub const OPENAI_RESPONSES: &str = "openai_responses";
	pub const ANTHROPIC_MESSAGES: &str = "anthropic_messages";
	pub const BEDROCK_CONVERSE: &str = "bedrock_converse";
	pub const VERTEX_GEMINI: &str = "vertex_gemini";
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
