use agent_core::strng;
use agent_core::strng::Strng;
use serde_json::{Map, Value};

use crate::llm::{AIError, RouteType};
use crate::*;

const ANTHROPIC_VERSION: &str = "vertex-2023-10-16";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
			.unwrap_or_else(|| strng::literal!("global"));
		if let Some(model) = self.anthropic_model(request_model) {
			return strng::format!(
				"/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:{}",
				self.project_id,
				location,
				model,
				if streaming {
					"streamRawPredict"
				} else {
					"rawPredict"
				}
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
			Some(region) if region != "global" => {
				strng::format!("{region}-aiplatform.googleapis.com")
			},
			_ => {
				strng::literal!("aiplatform.googleapis.com")
			},
		}
	}

	pub async fn process_streaming(
		&self,
		log: crate::telemetry::log::AsyncLog<super::LLMInfo>,
		resp: crate::http::Response,
		model: &str,
		input_format: super::InputFormat,
	) -> crate::http::Response {
		let buffer = crate::http::response_buffer_limit(&resp);
		
		// Check if this is an Anthropic model - if so, use Anthropic streaming logic
		if self.is_anthropic_model(Some(model)) {
			match input_format {
				super::InputFormat::Completions => {
					resp.map(|b| super::conversion::messages::from_completions::translate_stream(b, buffer, log))
				},
				super::InputFormat::Messages => {
					resp.map(|b| super::conversion::messages::passthrough_stream(b, buffer, log))
				},
				super::InputFormat::Responses | super::InputFormat::CountTokens => {
					resp // For other input formats, just pass through
				},
			}
		} else {
			// For standard Vertex AI models, use default OpenAI-compatible streaming
			resp.map(|b| {
				super::parse::sse::json_passthrough::<super::types::completions::typed::StreamResponse>(b, buffer, move |f| {
					match f {
						Some(Ok(f)) => {
							log.non_atomic_mutate(|r| {
								if r.response.provider_model.is_none() {
									r.response.provider_model = Some(strng::new(&f.model));
								}
								if let Some(u) = f.usage {
									r.response.input_tokens = Some(u.prompt_tokens as u64);
									r.response.output_tokens = Some(u.completion_tokens as u64);
									r.response.total_tokens = Some(u.total_tokens as u64);
								}
							});
						},
						_ => {}
					}
				})
			})
		}
	}
}
