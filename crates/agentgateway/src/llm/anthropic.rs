use agent_core::prelude::Strng;
use agent_core::strng;

use crate::llm::RouteType;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("anthropic");
}
pub const DEFAULT_HOST_STR: &str = "api.anthropic.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub fn path(route: RouteType) -> &'static str {
	match route {
		RouteType::AnthropicTokenCount => "/v1/messages/count_tokens",
		_ => "/v1/messages",
	}
}
