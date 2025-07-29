use agent_core::strng;
use agent_core::strng::Strng;
use anyhow::anyhow;
use bytes::Bytes;
use serde_json::Value;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use super::{LLMResponse, Provider as LLMProvider, universal};
use crate::http::{Body, Request, Response};
use crate::llm::bedrock::translate_error;
use crate::llm::bedrock::types::ConverseErrorResponse;
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
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("gemini");
}
pub const DEFAULT_HOST_STR: &str = "generativelanguage.googleapis.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);
pub const DEFAULT_PATH: &str = "/v1beta/openai/chat/completions";

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
}
