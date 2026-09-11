use std::collections::BTreeMap;

use agent_core::strng;
use itertools::Itertools;

use super::{ModerationProvider, ModerationVerdict};
use crate::*;

/// The OpenAI moderations API.
pub struct OpenAI;

impl ModerationProvider for OpenAI {
	fn resource_name(&self) -> Strng {
		strng::literal!("_openai-moderation")
	}

	fn default_host(&self) -> Strng {
		strng::literal!("api.openai.com")
	}

	fn default_model(&self) -> Strng {
		strng::literal!("omni-moderation-latest")
	}

	fn build_request(
		&self,
		model: &str,
		messages: &[crate::llm::SimpleChatCompletionMessage],
	) -> anyhow::Result<(Strng, Vec<u8>)> {
		// The moderations API takes plain strings, so roles are dropped here rather than
		// upstream: providers that judge a conversation need them.
		let content = messages.iter().map(|m| m.content.as_str()).collect_vec();
		let body = serde_json::to_vec(&serde_json::json!({
			"input": content,
			"model": model,
		}))?;
		Ok((strng::literal!("/v1/moderations"), body))
	}

	fn parse(&self, body: &[u8]) -> anyhow::Result<Vec<ModerationVerdict>> {
		let resp: async_openai::types::moderations::CreateModerationResponse =
			serde_json::from_slice(body)?;
		resp
			.results
			.iter()
			.map(|r| {
				Ok(ModerationVerdict {
					// Kept even though the categories usually say the same thing: `Categories` is
					// a closed struct, so a category OpenAI adds later is dropped here while
					// `flagged` still reports it.
					provider_flagged: Some(r.flagged),
					categories: category_map(&r.categories, serde_json::Value::as_bool)?,
					scores: category_map(&r.category_scores, serde_json::Value::as_f64)?,
				})
			})
			.collect()
	}
}

/// Turn a provider's category struct into a map, keeping only the entries that read as the
/// expected type.
fn category_map<T: serde::Serialize, V>(
	categories: &T,
	pick: impl Fn(&serde_json::Value) -> Option<V>,
) -> anyhow::Result<BTreeMap<Strng, V>> {
	let serde_json::Value::Object(map) = serde_json::to_value(categories)? else {
		anyhow::bail!("moderation categories were not a JSON object");
	};
	Ok(
		map
			.iter()
			.filter_map(|(name, value)| pick(value).map(|value| (strng::new(name), value)))
			.collect(),
	)
}
