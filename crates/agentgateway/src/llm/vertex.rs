use agent_core::strng;
use agent_core::strng::Strng;
use serde_json::{Map, Value};

use crate::llm::{AIError, RouteType};
use crate::*;

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

	fn anthropic_model<'a>(&'a self, request_model: Option<&'a str>) -> Option<Strng> {
		let model = self.configured_model(request_model)?;
		model
			.strip_prefix("publishers/anthropic/models/")
			.or_else(|| model.strip_prefix("anthropic/"))
			.map(strng::new)
	}

	pub fn is_anthropic_model(&self, request_model: Option<&str>) -> bool {
		self.anthropic_model(request_model).is_some()
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
			.unwrap_or_else(|| strng::literal!("us-central1"));
		if let Some(model) = self.anthropic_model(request_model) {
			return match route {
				RouteType::AnthropicTokenCount => strng::format!(
					"/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:countTokens",
					self.project_id,
					location,
					model
				),
				_ => strng::format!(
					"/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:{}",
					self.project_id,
					location,
					model,
					if streaming {
						"streamRawPredict"
					} else {
						"rawPredict"
					}
				),
			};
		}

		if route == RouteType::Embeddings {
			let model = self.configured_model(request_model).unwrap_or_default();
			return strng::format!(
				"/v1/projects/{}/locations/{}/publishers/google/models/{}:predict",
				self.project_id,
				location,
				model
			);
		}

		strng::format!(
			"/v1/projects/{}/locations/{}/endpoints/openapi/chat/completions",
			self.project_id,
			location
		)
	}

	pub fn get_host(&self, _request_model: Option<&str>) -> Strng {
		match &self.region {
			None => {
				strng::literal!("us-central1-aiplatform.googleapis.com")
			},
			Some(region) => {
				strng::format!("{region}-aiplatform.googleapis.com")
			},
		}
	}
}
