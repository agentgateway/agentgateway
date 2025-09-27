use std::collections::HashMap;

use agent_core::strng;
use agent_core::strng::Strng;
use bytes::Bytes;

use super::universal;
use crate::llm::AIError;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub model_aliases: HashMap<Strng, Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("openai");
}
pub const DEFAULT_HOST_STR: &str = "api.openai.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);
pub const DEFAULT_PATH: &str = "/v1/chat/completions";

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
		// This is openai already...
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
}
