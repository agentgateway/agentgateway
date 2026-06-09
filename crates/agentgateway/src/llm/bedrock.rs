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
	pub fn feature_model_id<'a>(&'a self, request_model: Option<&'a str>) -> Option<&'a str> {
		match self.model.as_deref() {
			Some(model) if is_inference_profile_model_id(model) => request_model.or(Some(model)),
			Some(model) => Some(model),
			None => request_model,
		}
	}

	pub fn is_anthropic_model(&self, request_model: Option<&str>) -> bool {
		let model = self
			.feature_model_id(request_model)
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
			_ if streaming => strng::format!("/model/{model}/converse-stream"),
			_ => strng::format!("/model/{model}/converse"),
		}
	}

	pub fn get_host(&self) -> Strng {
		strng::format!("bedrock-runtime.{}.amazonaws.com", self.region)
	}
}

pub fn is_inference_profile_model_id(model_id: &str) -> bool {
	let model_id = model_id.to_ascii_lowercase();
	model_id.contains(":application-inference-profile/")
		|| model_id.contains(":inference-profile/")
		|| model_id.starts_with("application-inference-profile/")
		|| model_id.starts_with("inference-profile/")
}

#[cfg(test)]
mod tests {
	use agent_core::strng;

	use super::*;

	fn provider(model: &str) -> Provider {
		Provider {
			model: Some(strng::new(model)),
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
			source_credentials_cache: Default::default(),
			assume_role_cache: Default::default(),
		}
	}

	#[test]
	fn anthropic_detection_uses_request_model_for_inference_profiles() {
		let provider =
			provider("arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/my-profile");

		assert!(provider.is_anthropic_model(Some("anthropic.claude-3-5-sonnet-20241022-v2:0")));
	}

	#[test]
	fn anthropic_detection_keeps_explicit_model_override_for_plain_models() {
		let provider = provider("amazon.titan-text-express-v1");

		assert!(!provider.is_anthropic_model(Some("anthropic.claude-3-5-sonnet-20241022-v2:0")));
	}
}
