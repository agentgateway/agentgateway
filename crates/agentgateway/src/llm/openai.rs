use agent_core::strng;
use agent_core::strng::Strng;

use crate::llm::RouteType;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("openai");
}
pub const DEFAULT_HOST_STR: &str = "api.openai.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub fn path(route: RouteType) -> &'static str {
	match route {
		// For Responses we forward to the responses endpoint
		RouteType::Responses => "/v1/responses",
		// For Embeddings we forward to the embeddings endpoint
		RouteType::Embeddings => "/v1/embeddings",
		RouteType::Realtime => "/v1/realtime",
		// All others get translated down to completions
		_ => "/v1/chat/completions",
	}
}
