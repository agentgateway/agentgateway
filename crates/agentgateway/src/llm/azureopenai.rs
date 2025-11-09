use agent_core::strng;
use agent_core::strng::Strng;
use bytes::Bytes;

use super::universal;
use crate::llm::AIError;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // this is the Azure OpenAI model deployment name
	pub host: Strng, // required
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub api_version: Option<Strng>, // optional, defaults to "v1"
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("azure.openai");
}

impl Provider {
	pub fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp = serde_json::from_slice::<universal::ChatCompletionErrorResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
		Ok(resp)
	}
	pub fn get_path_for_model(&self, model: &str) -> Strng {
		let api_version = self.api_version();
		if api_version == "v1" {
			strng::format!("/openai/v1/chat/completions")
		} else if api_version == "preview" {
			// v1 preview API
			strng::format!("/openai/v1/chat/completions?api-version=preview")
		} else {
			let model = self.model.as_deref().unwrap_or(model);
			strng::format!(
				"/openai/deployments/{}/chat/completions?api-version={}",
				model,
				api_version
			)
		}
	}
	pub fn get_host(&self) -> Strng {
		self.host.clone()
	}

	fn api_version(&self) -> &str {
		self.api_version.as_deref().unwrap_or("v1")
	}
}
