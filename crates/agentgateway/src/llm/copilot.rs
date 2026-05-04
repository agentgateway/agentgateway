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
	const NAME: Strng = strng::literal!("copilot");
}

pub const DEFAULT_HOST_STR: &str = "api.githubcopilot.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub fn path_suffix(route: RouteType) -> &'static str {
	match route {
		RouteType::Responses => "/responses",
		RouteType::Embeddings => "/embeddings",
		RouteType::Models => "/models",
		_ => "/chat/completions",
	}
}
