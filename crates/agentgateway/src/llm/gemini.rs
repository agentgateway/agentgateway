use agent_core::strng;
use agent_core::strng::Strng;

use crate::llm::RouteType;
use crate::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("gcp.gemini");
}
pub const DEFAULT_HOST_STR: &str = "generativelanguage.googleapis.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub fn path(route: RouteType) -> &'static str {
	match route {
		RouteType::Embeddings => "/v1beta/openai/embeddings",
		_ => "/v1beta/openai/chat/completions",
	}
}
