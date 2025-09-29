use agent_core::strng;
use agent_core::strng::Strng;
use bytes::Bytes;

use super::universal;
use crate::llm::AIError;
use crate::*;

#[apply(schema!)]
pub struct Provider {}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("openai");
}
pub const DEFAULT_HOST_STR: &str = "api.openai.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);
pub const DEFAULT_PATH: &str = "/v1/chat/completions";

impl Provider {
	pub async fn process_request(
		&self,
		req: universal::Request,
	) -> Result<universal::Request, AIError> {
		if req.model.is_none() {
			return Err(AIError::MissingField("model not specified".into()));
		}
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
