use agent_core::strng;
use agent_core::strng::Strng;
use anyhow::anyhow;
use bytes::Bytes;
use serde_json::Value;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use super::{LLMResponse, Provider as LLMProvider, universal};
use crate::http::{Body, Request, Response};
use crate::llm::universal::{ChatCompletionRequest, ChatCompletionStreamOptions};
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
	const NAME: Strng = strng::literal!("openai");
}
pub const DEFAULT_HOST_STR: &str = "api.openai.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);
pub const DEFAULT_PATH: &str = "/v1/chat/completions";

impl Provider {
	pub async fn process_request(
		&self,
		mut req: universal::ChatCompletionRequest,
	) -> Result<universal::ChatCompletionRequest, AIError> {
		if let Some(model) = &self.model {
			req.model = model.to_string();
		}
		// If a user doesn't request usage, we will not get token information which we need
		// We always set it.
		// TODO?: this may impact the user, if they make assumptions about the stream NOT including usage.
		// Notably, this adds a final SSE event.
		// We could actually go remove that on the response, but it would mean we cannot do passthrough-parsing,
		// so unless we have a compelling use case for it, for now we keep it.
		if req.stream.unwrap_or_default() && req.stream_options.is_none() {
			req.stream_options = Some(ChatCompletionStreamOptions {
				include_usage: true,
			});
		}
		// This is openai already...
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
