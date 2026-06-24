use agent_core::prelude::Strng;
use agent_core::strng;

use crate::http::auth::aws::{AwsAssumeRoleCache, AwsCredentialsCache};
use crate::*;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename = "BedrockProvider"))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // Optional: model override for Bedrock API path
	pub region: Strng, // Required: AWS region
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,
	/// Per-provider AWS source credential cache, shared across requests via Arc.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub source_credentials_cache: AwsCredentialsCache,
	/// Per-provider AWS AssumeRole credential cache, shared across requests via Arc.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub assume_role_cache: AwsAssumeRoleCache,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("aws.bedrock");
}

impl Provider {
	pub fn is_anthropic_model(&self, request_model: Option<&str>) -> bool {
		let model = self
			.model
			.as_deref()
			.or(request_model)
			.unwrap_or_default()
			.to_ascii_lowercase();
		model.contains("anthropic.claude")
	}

	pub fn get_path_for_route(
		&self,
		route_type: super::RouteType,
		streaming: bool,
		model: &str,
	) -> Strng {
		let model = self.model.as_deref().unwrap_or(model);
		const MODEL_SEGMENT: &percent_encoding::AsciiSet =
			&percent_encoding::CONTROLS.add(b'/').add(b'%');
		let model = percent_encoding::utf8_percent_encode(model, MODEL_SEGMENT);
		match route_type {
			super::RouteType::AnthropicTokenCount => strng::format!("/model/{model}/count-tokens"),
			super::RouteType::Embeddings => strng::format!("/model/{model}/invoke"),
			// Rerank uses the agent-runtime Rerank action (model goes in the body as an ARN).
			super::RouteType::Rerank => strng::literal!("/rerank"),
			_ if streaming => strng::format!("/model/{model}/converse-stream"),
			_ => strng::format!("/model/{model}/converse"),
		}
	}

	pub fn get_host(&self, route_type: super::RouteType) -> Strng {
		match route_type {
			super::RouteType::Rerank => {
				strng::format!("bedrock-agent-runtime.{}.amazonaws.com", self.region)
			},
			_ => strng::format!("bedrock-runtime.{}.amazonaws.com", self.region),
		}
	}
}
