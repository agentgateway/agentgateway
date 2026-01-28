//! Vertex AI Provider
//!
//! Documentation for Claude on Vertex AI:
//! - https://platform.claude.com/docs/en/build-with-claude/claude-on-vertex-ai
//! - https://docs.cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/use-claude
//! - https://docs.cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/count-tokens

use agent_core::strng;
use agent_core::strng::Strng;
use serde_json::{Map, Value};

use crate::llm::{AIError, RouteType};
use crate::*;

#[cfg(test)]
#[path = "vertex_tests.rs"]
mod tests;

const ANTHROPIC_VERSION: &str = "vertex-2023-10-16";

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub region: Option<Strng>,
	pub project_id: Strng,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("gcp.vertex_ai");
}

impl Provider {
	fn configured_model<'a>(&'a self, request_model: Option<&'a str>) -> Option<&'a str> {
		self.model.as_deref().or(request_model)
	}

	fn get_anthropic_model_id<'a>(&'a self, request_model: Option<&'a str>) -> Option<Strng> {
		let model = self.configured_model(request_model)?;
		let model = model
			.strip_prefix("publishers/anthropic/models/")
			.or_else(|| model.strip_prefix("anthropic/"))
			.unwrap_or(model);
		Some(strng::new(model))
	}

	pub fn is_anthropic_model(&self, request_model: Option<&str>) -> bool {
		self
			.get_anthropic_model_id(request_model)
			.map(|m| m.contains("claude-"))
			.unwrap_or(false)
	}

	pub fn prepare_anthropic_request_body(&self, body: Vec<u8>) -> Result<Vec<u8>, AIError> {
		let mut map: Map<String, Value> =
			serde_json::from_slice(&body).map_err(AIError::RequestMarshal)?;
		map.insert(
			"anthropic_version".to_string(),
			Value::String(ANTHROPIC_VERSION.to_string()),
		);
		map.remove("model");
		serde_json::to_vec(&map).map_err(AIError::RequestMarshal)
	}

	pub fn get_path_for_model(
		&self,
		route: RouteType,
		request_model: Option<&str>,
		streaming: bool,
	) -> Strng {
		let location = self
			.region
			.clone()
			.unwrap_or_else(|| strng::literal!("global"));

		let raw_model = self.configured_model(request_model);
		let is_explicit_anthropic = raw_model
			.map(|m| m.starts_with("anthropic/") || m.starts_with("publishers/anthropic/models/"))
			.unwrap_or(false);

		let model = self.get_anthropic_model_id(request_model);

		let use_anthropic_endpoint = match (route, &model) {
			(RouteType::AnthropicTokenCount, _) => true,
			(_, Some(m)) => is_explicit_anthropic || m.contains("claude-"),
			_ => false,
		};

		if use_anthropic_endpoint && let Some(model) = model {
			let suffix = match route {
				RouteType::AnthropicTokenCount => "countTokens",
				_ if streaming => "streamRawPredict",
				_ => "rawPredict",
			};
			return strng::format!(
				"/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:{}",
				self.project_id,
				location,
				model,
				suffix
			);
		}
		let t = if route == RouteType::Embeddings {
			strng::literal!("embeddings")
		} else {
			strng::literal!("chat/completions")
		};
		strng::format!(
			"/v1/projects/{}/locations/{}/endpoints/openapi/{t}",
			self.project_id,
			location
		)
	}

	pub fn get_host(&self) -> Strng {
		match &self.region {
			None => {
				strng::literal!("aiplatform.googleapis.com")
			},
			Some(region) => {
				strng::format!("{region}-aiplatform.googleapis.com")
			},
		}
	}
}
