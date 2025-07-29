use agent_core::strng;
use agent_core::strng::Strng;
use anyhow::anyhow;
use bytes::Bytes;
use serde_json::Value;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use super::{LLMResponse, Provider as LLMProvider, universal};
use crate::http::{Body, Request, Response};
use crate::llm::universal::ChatCompletionRequest;
use crate::llm::{AIError, AIProvider, LLMRequest};
use crate::proxy::ProxyError;
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
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("vertex");
}

impl Provider {
	pub async fn process_request(
		&self,
		mut req: universal::ChatCompletionRequest,
	) -> Result<universal::ChatCompletionRequest, AIError> {
		if let Some(model) = &self.model {
			req.model = model.to_string();
		}
		// Gemini compat mode is the same!
		Ok(req)
	}
	pub async fn process_response(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionResponse, AIError> {
		let resp = serde_json::from_slice::<universal::ChatCompletionResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
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
