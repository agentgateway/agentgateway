use agent_core::strng;
use agent_core::strng::Strng;
use bytes::Bytes;
use std::collections::HashMap;

use super::universal;
use crate::llm::AIError;
use crate::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub region: Option<Strng>,
	pub project_id: Strng,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub model_aliases: HashMap<Strng, Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("vertex");
}

impl Provider {
	pub async fn process_request(
		&self,
		mut req: universal::Request,
	) -> Result<universal::Request, AIError> {
		// Apply model alias resolution (request model takes precedence over provider default)
		if let Some(model) = req.model.as_deref().or(self.model.as_deref()) {
			if let Some(resolved) = crate::llm::resolve_model_alias(&self.model_aliases, model) {
				req.model = Some(resolved.to_string());
			} else {
				req.model = Some(model.to_string());
			}
		} else {
			return Err(AIError::MissingField("model not specified".into()));
		}
		// Gemini compat mode is the same!
		Ok(req)
	}
	pub async fn process_response(&self, bytes: &Bytes) -> Result<universal::Response, AIError> {
		let resp =
			serde_json::from_slice::<universal::Response>(bytes).map_err(AIError::ResponseParsing)?;
		Ok(resp)
	}
	pub async fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp = serde_json::from_slice::<universal::ChatCompletionErrorResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
		Ok(resp)
	}
	pub fn get_path_for_model(&self) -> Strng {
		strng::format!(
			"/v1beta1/projects/{}/locations/{}/endpoints/openapi/chat/completions",
			self.project_id,
			self.region.as_ref().unwrap_or(&strng::literal!("global"))
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
