#![allow(deprecated)]
#![allow(deprecated_in_future)]

use std::collections::HashMap;

use agent_core::strng;
use agent_core::strng::Strng;
#[allow(deprecated)]
#[allow(deprecated_in_future)]
pub use async_openai::types::ChatCompletionFunctions;
use async_openai::types::{
	ChatChoiceLogprobs, ChatCompletionMessageToolCall, ChatCompletionMessageToolCallChunk,
	ChatCompletionResponseMessageAudio, CompletionUsage, FunctionCallStream, ServiceTierResponse,
};
pub use async_openai::types::{
	ChatCompletionAudio, ChatCompletionFunctionCall,
	ChatCompletionMessageToolCall as MessageToolCall, ChatCompletionModalities,
	ChatCompletionNamedToolChoice as NamedToolChoice,
	ChatCompletionRequestAssistantMessage as RequestAssistantMessage,
	ChatCompletionRequestAssistantMessageContent as RequestAssistantMessageContent,
	ChatCompletionRequestDeveloperMessage as RequestDeveloperMessage,
	ChatCompletionRequestDeveloperMessageContent as RequestDeveloperMessageContent,
	ChatCompletionRequestFunctionMessage as RequestFunctionMessage,
	ChatCompletionRequestMessage as RequestMessage,
	ChatCompletionRequestSystemMessage as RequestSystemMessage,
	ChatCompletionRequestSystemMessageContent as RequestSystemMessageContent,
	ChatCompletionRequestToolMessage as RequestToolMessage,
	ChatCompletionRequestToolMessageContent as RequestToolMessageContent,
	ChatCompletionRequestUserMessage as RequestUserMessage,
	ChatCompletionRequestUserMessageContent as RequestUserMessageContent,
	ChatCompletionStreamOptions as StreamOptions, ChatCompletionTool, ChatCompletionTool as Tool,
	ChatCompletionToolChoiceOption as ToolChoiceOption, ChatCompletionToolChoiceOption,
	ChatCompletionToolType as ToolType, CompletionUsage as Usage, CreateChatCompletionRequest,
	FinishReason, FunctionCall, FunctionName, FunctionObject, PredictionContent, ReasoningEffort,
	ResponseFormat, Role, ServiceTier, Stop, WebSearchOptions,
};
use serde::{Deserialize, Serialize};

use crate::llm;
use crate::llm::bedrock::Provider;
use crate::llm::{AIError, LLMRequest, LLMResponse};

pub mod passthrough {
	use agent_core::strng;
	use agent_core::strng::Strng;
	use bytes::Bytes;
	use itertools::Itertools;
	use serde::{Deserialize, Serialize};

	use crate::llm::bedrock::Provider;
	use crate::llm::policy::webhook::{Message, ResponseChoice};
	use crate::llm::universal::ResponseType;
	use crate::llm::{
		AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, SimpleChatCompletionMessage,
		anthropic, universal,
	};
	use crate::{json, llm};

	pub fn process_response(
		bytes: &Bytes,
		input_format: InputFormat,
	) -> Result<Box<dyn ResponseType>, AIError> {
		match input_format {
			InputFormat::Completions => {
				let resp = serde_json::from_slice::<universal::passthrough::Response>(bytes)
					.map_err(AIError::ResponseParsing)?;

				Ok(Box::new(resp))
			},
			InputFormat::Messages => {
				let resp =
					serde_json::from_slice::<universal::Response>(bytes).map_err(AIError::ResponseParsing)?;
				let anthropic = anthropic::translate_anthropic_response(resp);
				let passthrough = json::convert::<_, anthropic::passthrough::Response>(&anthropic)
					.map_err(AIError::ResponseParsing)?;
				Ok(Box::new(passthrough))
			},
			InputFormat::Responses => {
				unreachable!("Responses format should not be routed to Universal (OpenAI) provider")
			},
			InputFormat::CountTokens => {
				unreachable!("CountTokens should be handled by process_count_tokens_response")
			},
		}
	}

	#[derive(Clone, Debug, Serialize, Deserialize)]
	pub struct Request {
		pub messages: Vec<RequestMessage>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub model: Option<String>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub temperature: Option<f32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stream: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub frequency_penalty: Option<f32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub presence_penalty: Option<f32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub seed: Option<i64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stream_options: Option<StreamOptions>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub max_tokens: Option<u32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub max_completion_tokens: Option<u32>,

		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	/// Options for streaming response. Only set this when you set `stream: true`.
	#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
	pub struct StreamOptions {
		/// If set, an additional chunk will be streamed before the `data: [DONE]` message. The `usage` field on this chunk shows the token usage statistics for the entire request, and the `choices` field will always be an empty array. All other chunks will also include a `usage` field, but with a null value.
		pub include_usage: bool,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Response {
		pub model: String,
		pub usage: Option<Usage>,
		/// A list of chat completion choices. Can be more than one if `n` is greater than 1.
		pub choices: Vec<Choice>,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Choice {
		pub message: ResponseMessage,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
	pub struct ResponseMessage {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub content: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub role: Option<String>,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}
	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Usage {
		/// Number of tokens in the prompt.
		pub prompt_tokens: u32,
		/// Number of tokens in the generated completion.
		pub completion_tokens: u32,
		/// Total number of tokens used in the request (prompt + completion).
		pub total_tokens: u32,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	impl super::ResponseType for Response {
		fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
			LLMResponse {
				input_tokens: self.usage.as_ref().map(|u| u.prompt_tokens as u64),
				output_tokens: self.usage.as_ref().map(|u| u.completion_tokens as u64),
				total_tokens: self.usage.as_ref().map(|u| u.total_tokens as u64),
				provider_model: Some(strng::new(&self.model)),
				completion: if include_completion_in_log {
					Some(
						self
							.choices
							.iter()
							.flat_map(|c| c.message.content.clone())
							.collect_vec(),
					)
				} else {
					None
				},
				first_token: Default::default(),
			}
		}

		fn set_webhook_choices(&mut self, choices: Vec<ResponseChoice>) -> anyhow::Result<()> {
			if self.choices.len() != choices.len() {
				anyhow::bail!("webhook response message count mismatch");
			}
			for (m, wh) in self.choices.iter_mut().zip(choices.into_iter()) {
				m.message.content = Some(wh.message.content.to_string());
			}
			Ok(())
		}

		fn to_webhook_choices(&self) -> Vec<ResponseChoice> {
			self
				.choices
				.iter()
				.map(|c| {
					let role = c.message.role.clone().unwrap_or_default().into();
					let content = c.message.content.clone().unwrap_or_default().into();
					ResponseChoice {
						message: Message { role, content },
					}
				})
				.collect()
		}

		fn serialize(&self) -> serde_json::Result<Vec<u8>> {
			serde_json::to_vec(&self)
		}
	}

	impl super::RequestType for Request {
		fn model(&mut self) -> Option<&mut String> {
			self.model.as_mut()
		}
		fn prepend_prompts(&mut self, prompts: Vec<llm::SimpleChatCompletionMessage>) {
			self
				.messages
				.splice(..0, prompts.into_iter().map(convert_message));
		}

		fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
			let typed = json::convert::<_, universal::Request>(self).map_err(AIError::RequestMarshal)?;
			let xlated = anthropic::translate_request(typed);
			serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
		}

		fn to_bedrock(
			&self,
			provider: &Provider,
			headers: Option<&::http::HeaderMap>,
			prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
		) -> Result<Vec<u8>, AIError> {
			let typed = json::convert::<_, universal::Request>(self).map_err(AIError::RequestMarshal)?;
			let xlated =
				llm::bedrock::translate_request_completions(typed, provider, headers, prompt_caching);
			serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
		}

		fn to_openai(&self) -> Result<Vec<u8>, AIError> {
			serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
		}

		fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
			let model = strng::new(self.model.as_deref().unwrap_or_default());
			let input_tokens = if tokenize {
				let tokens = crate::llm::num_tokens_from_messages(&model, &self.messages)?;
				Some(tokens)
			} else {
				None
			};
			// Pass the original body through
			let llm = LLMRequest {
				input_tokens,
				input_format: InputFormat::Completions,
				request_model: model,
				provider,
				streaming: self.stream.unwrap_or_default(),
				params: LLMRequestParams {
					temperature: self.temperature.map(Into::into),
					top_p: self.top_p.map(Into::into),
					frequency_penalty: self.frequency_penalty.map(Into::into),
					presence_penalty: self.presence_penalty.map(Into::into),
					seed: self.seed,
					max_tokens: self
						.max_completion_tokens
						.or(self.max_tokens)
						.map(Into::into),
				},
			};
			Ok(llm)
		}

		fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
			self
				.messages
				.iter()
				.map(|m| {
					let content = m
						.content
						.as_ref()
						.and_then(|c| match c {
							Content::Text(t) => Some(strng::new(t)),
							// TODO?
							Content::Array(_) => None,
						})
						.unwrap_or_default();
					SimpleChatCompletionMessage {
						role: strng::new(&m.role),
						content,
					}
				})
				.collect()
		}

		fn set_messages(&mut self, messages: Vec<llm::SimpleChatCompletionMessage>) {
			self.messages = messages.into_iter().map(convert_message).collect();
		}
	}

	fn convert_message(r: SimpleChatCompletionMessage) -> RequestMessage {
		RequestMessage {
			role: r.role.to_string(),
			content: Some(Content::Text(r.content.to_string())),
			name: None,
			rest: Default::default(),
		}
	}
	#[derive(Clone, Debug, Serialize, Deserialize)]
	pub struct RequestMessage {
		pub role: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub name: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub content: Option<Content>,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	impl RequestMessage {
		pub fn message_text(&self) -> Option<&str> {
			self.content.as_ref().and_then(|c| match c {
				Content::Text(t) => Some(t.as_str()),
				// TODO?
				Content::Array(_) => None,
			})
		}
	}

	#[derive(Clone, Debug, Serialize, Deserialize)]
	#[serde(untagged)]
	pub enum Content {
		Text(String),
		Array(Vec<ContentPart>),
	}

	#[derive(Clone, Debug, Serialize, Deserialize)]
	pub struct ContentPart {
		pub r#type: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub text: Option<String>,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}
}
