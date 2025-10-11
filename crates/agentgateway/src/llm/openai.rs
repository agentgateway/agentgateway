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
		mut req: universal::passthrough::Request,
	) -> Result<universal::passthrough::Request, AIError> {
		if let Some(provider_model) = &self.model {
			req.model = Some(provider_model.to_string());
		} else if req.model.is_none() {
			return Err(AIError::MissingField("model not specified".into()));
		}
		// This is openai already...
		Ok(req)
	}
	pub fn process_response(
		&self,
		bytes: &Bytes,
	) -> Result<universal::passthrough::Response, AIError> {
		let resp = serde_json::from_slice::<universal::passthrough::Response>(bytes)
			.map_err(AIError::ResponseParsing)?;
		Ok(resp)
	}
	pub fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp = serde_json::from_slice::<universal::ChatCompletionErrorResponse>(bytes)
			.map_err(AIError::ResponseParsing)?;
		Ok(resp)
	}
}

pub mod responses {
	use async_openai::types::responses::{
		Input, OutputContent, ReasoningConfig, ServiceTier, TextConfig, ToolChoice, ToolDefinition,
		Truncation,
	};
	use bytes::Bytes;

	use crate::llm::universal::{RequestType, ResponseType};
	use crate::llm::{AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse};

	pub mod passthrough {
		use super::*;
		use serde::{Deserialize, Serialize};

		#[derive(Debug, Deserialize, Clone, Serialize)]
		pub struct Request {
			pub input: Input,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub model: Option<String>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub instructions: Option<String>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub max_output_tokens: Option<u32>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub previous_response_id: Option<String>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub temperature: Option<f32>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub top_p: Option<f32>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub stream: Option<bool>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub tools: Option<Vec<ToolDefinition>>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub tool_choice: Option<ToolChoice>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub parallel_tool_calls: Option<bool>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub user: Option<String>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub metadata: Option<std::collections::HashMap<String, String>>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub reasoning: Option<ReasoningConfig>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub text: Option<TextConfig>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub store: Option<bool>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub service_tier: Option<ServiceTier>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub top_logprobs: Option<u32>,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub truncation: Option<Truncation>,

			#[serde(flatten, default)]
			pub rest: serde_json::Value,
		}

		#[derive(Debug, Deserialize, Clone, Serialize)]
		pub struct Response {
			pub id: String,
			pub status: String,
			pub output: Vec<OutputContent>,
			pub model: String,
			#[serde(skip_serializing_if = "Option::is_none")]
			pub usage: Option<Usage>,
			#[serde(flatten, default)]
			pub rest: serde_json::Value,
		}

		#[derive(Debug, Deserialize, Clone, Serialize)]
		pub struct Usage {
			pub input_tokens: u64,
			pub output_tokens: u64,
			#[serde(flatten, default)]
			pub rest: serde_json::Value,
		}

		impl RequestType for Request {
			fn model(&mut self) -> Option<&mut String> {
				self.model.as_mut()
			}

			fn prepend_prompts(&mut self, _prompts: Vec<crate::llm::SimpleChatCompletionMessage>) {
				// TODO
			}

			fn to_llm_request(
				&self,
				provider: agent_core::strng::Strng,
				_tokenize: bool,
			) -> Result<LLMRequest, AIError> {
				let model = agent_core::strng::new(self.model.as_deref().unwrap_or_default());
				// TODO: Implement tokenization for responses format
				let input_tokens = None;

				Ok(LLMRequest {
					input_tokens,
					input_format: InputFormat::Responses,
					request_model: model,
					provider,
					streaming: self.stream.unwrap_or_default(),
					params: LLMRequestParams {
						temperature: self.temperature.map(Into::into),
						top_p: self.top_p.map(Into::into),
						frequency_penalty: None,
						presence_penalty: None,
						seed: None,
						max_tokens: self.max_output_tokens.map(Into::into),
					},
				})
			}

			fn get_messages(&self) -> Vec<crate::llm::SimpleChatCompletionMessage> {
				// TODO
				vec![]
			}

			fn set_messages(&mut self, _messages: Vec<crate::llm::SimpleChatCompletionMessage>) {
				// TODO
			}

			fn to_openai(&self) -> Result<Vec<u8>, AIError> {
				// Passthrough - just serialize
				serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
			}
		}

		impl ResponseType for Response {
			fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
				LLMResponse {
					input_tokens: self.usage.as_ref().map(|u| u.input_tokens),
					output_tokens: self.usage.as_ref().map(|u| u.output_tokens),
					total_tokens: self.usage.as_ref().map(|u| u.input_tokens + u.output_tokens),
					provider_model: Some(agent_core::strng::new(&self.model)),
					completion: if include_completion_in_log {
						Some(
							self.output
								.iter()
								.filter_map(|o| match o {
									OutputContent::Message(msg) => msg
										.content
										.iter()
										.filter_map(|c| match c {
											async_openai::types::responses::Content::OutputText(t) => {
												Some(t.text.clone())
											}
											_ => None,
										})
										.collect::<Vec<_>>()
										.first()
										.cloned(),
									_ => None,
								})
								.collect(),
						)
					} else {
						None
					},
					first_token: Default::default(),
				}
			}

			fn set_webhook_choices(
				&mut self,
				_choices: Vec<crate::llm::policy::webhook::ResponseChoice>,
			) -> anyhow::Result<()> {
				// TODO
				Ok(())
			}

			fn to_webhook_choices(&self) -> Vec<crate::llm::policy::webhook::ResponseChoice> {
				// TODO
				vec![]
			}

			fn serialize(&self) -> serde_json::Result<Vec<u8>> {
				serde_json::to_vec(&self)
			}
		}
	}

	pub fn process_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<passthrough::Response>(bytes)
			.map_err(AIError::ResponseParsing)?;
		Ok(Box::new(resp))
	}

	pub async fn process_streaming(
		_log: crate::telemetry::log::AsyncLog<crate::llm::LLMInfo>,
		resp: crate::http::Response,
	) -> crate::http::Response {
		// TODO: extract telemetry from ResponseEvent when available in async-openai fork
		resp
	}
}
