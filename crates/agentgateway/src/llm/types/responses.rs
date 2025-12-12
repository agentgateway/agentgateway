use async_openai::types::responses::{
	Content, ContentType, Input, InputContent, InputItem, InputMessage, OutputContent, OutputText,
	Role,
};
use serde::{Deserialize, Serialize};

use super::*;
use crate::llm::{
	AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, RequestType, ResponseType,
	conversion,
};

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	// Required field for prompt enrichment/guards
	pub input: Input,

	// Fields we actually read for routing/telemetry
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_output_tokens: Option<u32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub stream: Option<bool>,

	// Everything else (tools, reasoning, etc.) - passthrough
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

impl From<SimpleChatCompletionMessage> for InputItem {
	fn from(msg: SimpleChatCompletionMessage) -> Self {
		let role = match msg.role.as_str() {
			"assistant" => Role::Assistant,
			"system" => Role::System,
			"developer" => Role::Developer,
			_ => Role::User,
		};

		InputItem::Message(InputMessage {
			kind: Default::default(),
			role,
			content: InputContent::TextInput(msg.content.to_string()),
		})
	}
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		if prompts.is_empty() {
			return;
		}

		// Convert prepend prompts to InputItems
		let prepend_items: Vec<InputItem> = prompts.into_iter().map(Into::into).collect();

		// Convert existing input to Items format if needed
		let mut items = match &self.input {
			Input::Text(text) => {
				vec![InputItem::Message(InputMessage {
					kind: Default::default(),
					role: Role::User,
					content: InputContent::TextInput(text.clone()),
				})]
			},
			Input::Items(existing_items) => existing_items.clone(),
		};

		// Prepend the new prompts
		items.splice(0..0, prepend_items);

		self.input = Input::Items(items);
	}

	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		let input_tokens = if tokenize {
			let tokens = crate::llm::num_tokens_from_responses_input(&model, &self.input)?;
			Some(tokens)
		} else {
			None
		};

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

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		match &self.input {
			Input::Text(text) => {
				vec![SimpleChatCompletionMessage {
					role: strng::literal!("user"),
					content: strng::new(text),
				}]
			},
			Input::Items(items) => items
				.iter()
				.filter_map(|item| match item {
					InputItem::Message(msg) => {
						let content = match &msg.content {
							InputContent::TextInput(text) => strng::new(text),
							InputContent::InputItemContentList(parts) => {
								// Extract text from all content parts
								let text = parts
									.iter()
									.filter_map(|part| match part {
										ContentType::InputText(input_text) => Some(input_text.text.as_str()),
										_ => None,
									})
									.collect::<Vec<_>>()
									.join("\n");
								strng::new(&text)
							},
						};

						let role = match msg.role {
							Role::User => strng::literal!("user"),
							Role::Assistant => strng::literal!("assistant"),
							Role::System => strng::literal!("system"),
							Role::Developer => strng::literal!("developer"),
						};

						Some(SimpleChatCompletionMessage { role, content })
					},
					_ => None,
				})
				.collect(),
		}
	}

	fn set_messages(&mut self, messages: Vec<SimpleChatCompletionMessage>) {
		self.input = Input::Items(messages.into_iter().map(Into::into).collect());
	}

	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		// Passthrough - just serialize
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}

	fn to_bedrock(
		&self,
		provider: &crate::llm::bedrock::Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		conversion::bedrock::from_responses::translate(self, provider, headers, prompt_caching)
	}
}

impl ResponseType for Response {
	fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
		LLMResponse {
			input_tokens: self.usage.as_ref().map(|u| u.input_tokens),
			output_tokens: self.usage.as_ref().map(|u| u.output_tokens),
			count_tokens: None,
			total_tokens: self
				.usage
				.as_ref()
				.map(|u| u.input_tokens + u.output_tokens),
			provider_model: Some(strng::new(&self.model)),
			completion: if include_completion_in_log {
				Some(
					self
						.output
						.iter()
						.filter_map(|o| match o {
							OutputContent::Message(msg) => Some(msg),
							_ => None,
						})
						.flat_map(|msg| {
							msg.content.iter().filter_map(|c| match c {
								Content::OutputText(t) => Some(t.text.clone()),
								_ => None,
							})
						})
						.collect(),
				)
			} else {
				None
			},
			first_token: Default::default(),
		}
	}

	fn to_webhook_choices(&self) -> Vec<crate::llm::policy::webhook::ResponseChoice> {
		self
			.output
			.iter()
			.filter_map(|o| match o {
				OutputContent::Message(msg) => {
					// Extract text from message content
					let content = msg
						.content
						.iter()
						.filter_map(|c| match c {
							Content::OutputText(t) => Some(t.text.clone()),
							_ => None,
						})
						.collect::<Vec<_>>()
						.join("\n");

					Some(crate::llm::policy::webhook::ResponseChoice {
						message: crate::llm::policy::webhook::Message {
							role: "assistant".into(),
							content: content.into(),
						},
					})
				},
				_ => None, // Ignore non-message outputs (tool calls, reasoning, etc.)
			})
			.collect()
	}

	fn set_webhook_choices(
		&mut self,
		choices: Vec<crate::llm::policy::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		// Filter only Message outputs (ignore tool calls, reasoning, etc.)
		let message_outputs: Vec<_> = self
			.output
			.iter_mut()
			.filter_map(|o| match o {
				OutputContent::Message(msg) => Some(msg),
				_ => None,
			})
			.collect();

		if message_outputs.len() != choices.len() {
			anyhow::bail!("webhook response message count mismatch");
		}

		for (msg, wh) in message_outputs.into_iter().zip(choices.into_iter()) {
			// Replace message content with webhook's modified content
			msg.content = vec![Content::OutputText(OutputText {
				text: wh.message.content.to_string(),
				annotations: vec![],
			})];
		}
		Ok(())
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self)
	}
}

pub mod typed {
	// Re-export async-openai Responses API types for cleaner usage
	pub use async_openai::types::responses::{
		Content, CreateResponse, FunctionCall, OutputContent, OutputMessage, OutputStatus, OutputText,
		Role,
	};
}
