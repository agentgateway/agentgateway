use agent_core::strng;
use agent_core::strng::Strng;
use serde_json::{Map, Value};

use crate::http::HeaderMap;
use crate::llm::conversion::bedrock::helpers::extract_beta_headers;
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

	/// Beta features that are not supported on Vertex AI and should be filtered out.
	const UNSUPPORTED_BETA_FEATURES: &'static [&'static str] = &[
		"oauth-",       // OAuth features don't apply to Vertex (uses GCP auth instead)
		"claude-code-", // Claude Code specific features for direct Anthropic API
	];

	fn is_supported_beta_feature(feature: &str) -> bool {
		!Self::UNSUPPORTED_BETA_FEATURES
			.iter()
			.any(|prefix| feature.starts_with(prefix))
	}

	pub fn prepare_anthropic_request_body(
		&self,
		body: Vec<u8>,
		headers: &HeaderMap,
	) -> Result<Vec<u8>, AIError> {
		self.prepare_anthropic_request_body_internal(body, headers, true, false)
	}

	/// Prepare request body for count-tokens endpoint.
	/// Unlike messages, count-tokens needs the model in the body (not in the URL path).
	pub fn prepare_count_tokens_request_body(
		&self,
		body: Vec<u8>,
		headers: &HeaderMap,
	) -> Result<Vec<u8>, AIError> {
		// Keep model (remove_model=false), don't add max_tokens (add_max_tokens=false)
		self.prepare_anthropic_request_body_internal(body, headers, false, false)
	}

	fn prepare_anthropic_request_body_internal(
		&self,
		body: Vec<u8>,
		headers: &HeaderMap,
		remove_model: bool,
		add_max_tokens: bool,
	) -> Result<Vec<u8>, AIError> {
		let mut map: Map<String, Value> =
			serde_json::from_slice(&body).map_err(AIError::RequestMarshal)?;
		map.insert(
			"anthropic_version".to_string(),
			Value::String(ANTHROPIC_VERSION.to_string()),
		);
		if remove_model {
			map.remove("model");
		}
		if add_max_tokens {
			map.entry("max_tokens".to_string())
				.or_insert(Value::Number(1.into()));
		}

		// Extract anthropic-beta headers and add to body if present
		// Filter out beta features not supported on Vertex AI
		if let Some(beta_features) = extract_beta_headers(headers)? {
			let filtered: Vec<Value> = beta_features
				.into_iter()
				.filter(|v| {
					if let Value::String(s) = v {
						Self::is_supported_beta_feature(s)
					} else {
						true
					}
				})
				.collect();
			if !filtered.is_empty() {
				map.insert("anthropic_beta".to_string(), Value::Array(filtered));
			}
		}

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
			return match route {
				RouteType::AnthropicTokenCount => {
					// Vertex AI has a dedicated count-tokens endpoint.
					// The model is specified in the request body, not the path.
					// See: https://cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/count-tokens
					strng::format!(
						"/v1/projects/{}/locations/{}/publishers/anthropic/models/count-tokens:rawPredict",
						self.project_id,
						location,
					)
				}
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
			None => strng::literal!("aiplatform.googleapis.com"),
			// Global endpoint uses the same host as no region - just "aiplatform.googleapis.com"
			// See: https://github.com/anthropics/anthropic-sdk-typescript/issues/800
			Some(region) if region == "global" => strng::literal!("aiplatform.googleapis.com"),
			Some(region) => strng::format!("{region}-aiplatform.googleapis.com"),
		}
	}
}
