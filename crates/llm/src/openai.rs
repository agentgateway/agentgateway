use agent_core::strng;
use agent_core::strng::Strng;

use crate::{RouteType, apply};

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "OpenAIProvider"))]
pub struct Provider {
	/// Model ID to send to OpenAI, overriding the model in the client request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	/// Configuration for running OpenAI inline moderation on request input and generated output.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub moderation: Option<ModerationParam>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("openai");
}

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "OpenAIModeration"))]
pub struct ModerationParam {
	/// The moderation model to use. Defaults to `omni-moderation-latest`.
	#[serde(default = "default_moderation_model")]
	pub model: Strng,
	/// Policies to apply to request input and generated output.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policy: Option<ModerationPolicyParam>,
}

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "OpenAIModerationPolicy"))]
pub struct ModerationPolicyParam {
	/// Policy for request input moderation.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub input: Option<ModerationConfigParam>,
	/// Policy for generated output moderation.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub output: Option<ModerationConfigParam>,
}

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "OpenAIModerationConfig"))]
pub struct ModerationConfigParam {
	pub mode: ModerationMode,
}

#[apply(schema_enum!)]
#[cfg_attr(feature = "schema", schemars(rename = "OpenAIModerationMode"))]
pub enum ModerationMode {
	Score,
	Block,
}

pub const DEFAULT_MODERATION_MODEL: Strng = strng::literal!("omni-moderation-latest");

fn default_moderation_model() -> Strng {
	DEFAULT_MODERATION_MODEL
}

pub const DEFAULT_HOST_STR: &str = "api.openai.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub const DEFAULT_BASE_PATH: &str = "/v1";

pub fn path_suffix(route: RouteType) -> &'static str {
	match route {
		RouteType::Responses => "/responses",
		RouteType::Embeddings => "/embeddings",
		RouteType::Rerank => "/rerank",
		RouteType::Realtime => "/realtime",
		// All others get translated down to completions
		_ => "/chat/completions",
	}
}
