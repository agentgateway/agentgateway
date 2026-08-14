use agent_core::prelude::Strng;
use agent_core::strng;

use crate::{RouteType, apply};

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "AnthropicProvider"))]
pub struct Provider {
	/// Model ID to send to Anthropic, overriding the model in the client request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("anthropic");
}
pub const DEFAULT_HOST_STR: &str = "api.anthropic.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat";

pub const DEFAULT_BASE_PATH: &str = "/v1";

pub fn path_suffix(route: RouteType) -> &'static str {
	match route {
		RouteType::AnthropicTokenCount => "/messages/count_tokens",
		_ => "/messages",
	}
}
