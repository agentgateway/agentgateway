use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::typed::{
	EasyInputContent, EasyInputMessage, InputContent, InputItem, InputMessage, InputRole,
	InputTextContent, OutputItem, OutputMessageContent as Content, OutputTextContent as OutputText,
	Role,
};
use super::*;
use crate::llm::{
	AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, RequestType, ResponseType,
	conversion,
};

/// Raw Responses API input — preserves the wire format for passthrough fidelity.
/// Typed deserialization would reject unknown item shapes (e.g. assistant history).
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum RequestInput {
	Text(String),
	Items(Vec<RawInputItem>),
}

#[derive(Debug, Deserialize, Clone, Serialize, PartialEq)]
#[serde(transparent)]
pub struct RawInputItem(Value);

impl RawInputItem {
	fn from_typed(item: InputItem) -> Self {
		Self(serde_json::to_value(item).expect("responses input item should serialize"))
	}

	fn from_user_text(text: String) -> Self {
		Self::from_typed(InputItem::from(InputMessage {
			content: vec![InputContent::InputText(InputTextContent { text })],
			role: InputRole::User,
			status: None,
		}))
	}

	fn from_simple_message(msg: SimpleChatCompletionMessage) -> Self {
		Self::from_typed(InputItem::from(msg))
	}

	fn as_simple_message(&self) -> Option<SimpleChatCompletionMessage> {
		let role = self.0.get("role")?.as_str()?;
		let role = match role {
			"user" => strng::literal!("user"),
			"assistant" => strng::literal!("assistant"),
			"system" => strng::literal!("system"),
			"developer" => strng::literal!("developer"),
			_ => return None,
		};

		let content = match self.0.get("content")? {
			Value::String(text) => strng::new(text),
			Value::Array(parts) => {
				let text = parts
					.iter()
					.filter_map(|part| {
						let part_type = part.get("type")?.as_str()?;
						match part_type {
							"input_text" | "output_text" => part.get("text")?.as_str(),
							_ => None,
						}
					})
					.collect::<Vec<_>>()
					.join("\n");
				strng::new(&text)
			},
			_ => return None,
		};

		Some(SimpleChatCompletionMessage { role, content })
	}
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	// Required field for prompt enrichment/guards
	pub input: RequestInput,

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

	#[serde(skip_serializing_if = "Option::is_none")]
	pub vendor_extensions: Option<RequestVendorExtensions>,

	// Everything else (tools, reasoning, etc.) - passthrough
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct RequestVendorExtensions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub thinking_budget_tokens: Option<u64>,

	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	pub id: String,
	pub status: String,
	pub output: Vec<OutputItem>,
	pub model: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub service_tier: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<Usage>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Usage {
	pub input_tokens: u64,
	pub output_tokens: u64,
	/// Breakdown of tokens used in a completion.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_tokens_details: Option<UsageInputDetails>,
	/// Breakdown of tokens used in the prompt.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_tokens_details: Option<UsageOutputDetails>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct UsageOutputDetails {
	pub reasoning_tokens: Option<u64>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct UsageInputDetails {
	pub cached_tokens: Option<u64>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

pub struct ResponseBuilder {
	response_id: String,
	model: String,
	created_at: u64,
}

impl ResponseBuilder {
	pub fn new(response_id: impl Into<String>, model: impl Into<String>) -> Self {
		Self {
			response_id: response_id.into(),
			model: model.into(),
			created_at: chrono::Utc::now().timestamp() as u64,
		}
	}

	pub fn response(
		&self,
		status: typed::Status,
		usage: Option<typed::ResponseUsage>,
		error: Option<typed::ErrorObject>,
		incomplete_details: Option<typed::IncompleteDetails>,
	) -> typed::Response {
		typed::Response {
			background: None,
			billing: None,
			conversation: None,
			created_at: self.created_at,
			completed_at: None,
			error,
			id: self.response_id.clone(),
			incomplete_details,
			instructions: None,
			max_output_tokens: None,
			metadata: None,
			model: self.model.clone(),
			object: "response".to_string(),
			output: Vec::new(),
			parallel_tool_calls: None,
			previous_response_id: None,
			prompt: None,
			prompt_cache_key: None,
			prompt_cache_retention: None,
			reasoning: None,
			safety_identifier: None,
			service_tier: None,
			status,
			temperature: None,
			text: None,
			tool_choice: None,
			tools: None,
			top_logprobs: None,
			top_p: None,
			truncation: None,
			usage,
		}
	}

	pub fn created_event(&self, sequence_number: u64) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseCreated(typed::ResponseCreatedEvent {
			sequence_number,
			response: self.response(typed::Status::InProgress, None, None, None),
		})
	}

	pub fn completed_event(
		&self,
		sequence_number: u64,
		usage: Option<typed::ResponseUsage>,
	) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseCompleted(typed::ResponseCompletedEvent {
			sequence_number,
			response: self.response(typed::Status::Completed, usage, None, None),
		})
	}

	pub fn incomplete_event(
		&self,
		sequence_number: u64,
		usage: Option<typed::ResponseUsage>,
		incomplete_details: typed::IncompleteDetails,
	) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseIncomplete(typed::ResponseIncompleteEvent {
			sequence_number,
			response: self.response(
				typed::Status::Incomplete,
				usage,
				None,
				Some(incomplete_details),
			),
		})
	}

	pub fn failed_event(
		&self,
		sequence_number: u64,
		usage: Option<typed::ResponseUsage>,
		error: typed::ErrorObject,
	) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseFailed(typed::ResponseFailedEvent {
			sequence_number,
			response: self.response(typed::Status::Failed, usage, Some(error), None),
		})
	}
}

impl From<SimpleChatCompletionMessage> for InputItem {
	fn from(msg: SimpleChatCompletionMessage) -> Self {
		match msg.role.as_str() {
			"assistant" => InputItem::EasyMessage(EasyInputMessage {
				r#type: Default::default(),
				role: Role::Assistant,
				content: EasyInputContent::Text(msg.content.to_string()),
				phase: None,
			}),
			"system" => InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: msg.content.to_string(),
				})],
				role: InputRole::System,
				status: None,
			}),
			"developer" => InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: msg.content.to_string(),
				})],
				role: InputRole::Developer,
				status: None,
			}),
			_ => InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: msg.content.to_string(),
				})],
				role: InputRole::User,
				status: None,
			}),
		}
	}
}

impl Request {
	fn take_input_as_items(&mut self) -> Vec<RawInputItem> {
		match std::mem::replace(&mut self.input, RequestInput::Items(Vec::new())) {
			RequestInput::Text(text) => vec![RawInputItem::from_user_text(text)],
			RequestInput::Items(items) => items,
		}
	}
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		let mut items = self.take_input_as_items();
		let prepend_items: Vec<RawInputItem> = prompts
			.into_iter()
			.map(RawInputItem::from_simple_message)
			.collect();
		items.splice(0..0, prepend_items);
		self.input = RequestInput::Items(items);
	}

	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		let mut items = self.take_input_as_items();
		items.extend(prompts.into_iter().map(RawInputItem::from_simple_message));
		self.input = RequestInput::Items(items);
	}

	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		let input_tokens = if tokenize {
			let messages = self.get_messages();
			let tokens = crate::llm::num_tokens_from_messages(&model, &messages)?;
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
				encoding_format: None,
				dimensions: None,
			},
			prompt: Default::default(),
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		match &self.input {
			RequestInput::Text(text) => {
				vec![SimpleChatCompletionMessage {
					role: strng::literal!("user"),
					content: strng::new(text),
				}]
			},
			RequestInput::Items(items) => items
				.iter()
				.filter_map(RawInputItem::as_simple_message)
				.collect(),
		}
	}

	fn set_messages(&mut self, messages: Vec<SimpleChatCompletionMessage>) {
		self.input = RequestInput::Items(
			messages
				.into_iter()
				.map(RawInputItem::from_simple_message)
				.collect(),
		);
	}

	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		// Passthrough - just serialize
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}

	fn to_openai_chat_completions(&self) -> Result<Vec<u8>, AIError> {
		conversion::openai_compat::from_responses::translate(self)
	}

	fn to_bedrock(
		&self,
		provider: &crate::llm::bedrock::Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		conversion::bedrock::from_responses::translate(self, provider, headers, prompt_caching)
	}

	fn to_vertex(&self, _provider: &crate::llm::vertex::Provider) -> Result<Vec<u8>, AIError> {
		self.to_openai()
	}
}

impl ResponseType for Response {
	fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
		LLMResponse {
			input_tokens: self.usage.as_ref().map(|u| u.input_tokens),
			input_image_tokens: None,
			input_text_tokens: None,
			input_audio_tokens: None,
			output_tokens: self.usage.as_ref().map(|u| u.output_tokens),
			// Note: responses supports image generation, but it does not report image generation as tokens.
			// Instead there is a cost based on the image parameters (https://platform.openai.com/docs/guides/image-generation#calculating-costs)
			// which we do not currently emit.
			output_image_tokens: None,
			output_text_tokens: None,
			output_audio_tokens: None,
			count_tokens: None,
			total_tokens: self
				.usage
				.as_ref()
				.map(|u| u.input_tokens + u.output_tokens),
			reasoning_tokens: self.usage.as_ref().and_then(|u| {
				u.output_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
			}),
			cached_input_tokens: self.usage.as_ref().and_then(|u| {
				u.input_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
			}),
			cache_creation_input_tokens: None,
			service_tier: self.service_tier.as_deref().map(Into::into),
			provider_model: Some(strng::new(&self.model)),
			completion: if include_completion_in_log {
				Some(
					self
						.output
						.iter()
						.filter_map(|o| match o {
							OutputItem::Message(msg) => Some(msg),
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
				OutputItem::Message(msg) => {
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
				OutputItem::Message(msg) => Some(msg),
				_ => None,
			})
			.collect();

		if message_outputs.len() != choices.len() {
			anyhow::bail!("webhook response message count mismatch");
		}

		for (msg, wh) in message_outputs.into_iter().zip(choices.into_iter()) {
			// Replace message content with webhook's modified content
			msg.content = vec![Content::OutputText(OutputText {
				annotations: vec![],
				logprobs: None,
				text: wh.message.content.to_string(),
			})];
		}
		Ok(())
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self)
	}
}

pub mod typed {
	use async_openai::types::responses as openai_responses;
	// Re-export async-openai Responses API types for cleaner usage
	pub use async_openai::types::responses::{
		AssistantRole, CreateResponse, CustomToolCallOutput, CustomToolCallOutputOutput,
		EasyInputContent, EasyInputMessage, ErrorObject, FunctionCallOutput, FunctionToolCall,
		IncompleteDetails, InputContent, InputItem, InputMessage, InputParam, InputRole,
		InputTextContent, InputTokenDetails, Item, MessageItem, OutputContent, OutputItem,
		OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent, OutputTokenDetails,
		ReasoningEffort, Response, ResponseCompletedEvent, ResponseContentPartAddedEvent,
		ResponseContentPartDoneEvent, ResponseCreatedEvent, ResponseErrorEvent, ResponseFailedEvent,
		ResponseFunctionCallArgumentsDeltaEvent, ResponseFunctionCallArgumentsDoneEvent,
		ResponseIncompleteEvent, ResponseOutputItemAddedEvent, ResponseOutputItemDoneEvent,
		ResponseTextDeltaEvent, ResponseTextParam, ResponseUsage, Role, Status,
		TextResponseFormatConfiguration, Tool, ToolChoiceFunction, ToolChoiceOptions, ToolChoiceParam,
	};
	use serde::{Deserialize, Serialize};

	/// Event types for streaming responses from the Responses API (minimal strict subset).
	#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
	#[allow(clippy::enum_variant_names)]
	#[serde(tag = "type")]
	pub enum ResponseStreamEvent {
		/// An event that is emitted when a response is created.
		#[serde(rename = "response.created")]
		ResponseCreated(openai_responses::ResponseCreatedEvent),
		/// Emitted when a new output item is added.
		#[serde(rename = "response.output_item.added")]
		ResponseOutputItemAdded(openai_responses::ResponseOutputItemAddedEvent),
		/// Emitted when a new content part is added.
		#[serde(rename = "response.content_part.added")]
		ResponseContentPartAdded(openai_responses::ResponseContentPartAddedEvent),
		/// Emitted when there is an additional text delta.
		#[serde(rename = "response.output_text.delta")]
		ResponseOutputTextDelta(openai_responses::ResponseTextDeltaEvent),
		/// Emitted when there is a partial function-call arguments delta.
		#[serde(rename = "response.function_call_arguments.delta")]
		ResponseFunctionCallArgumentsDelta(openai_responses::ResponseFunctionCallArgumentsDeltaEvent),
		/// Emitted when function-call arguments are finalized.
		#[serde(rename = "response.function_call_arguments.done")]
		ResponseFunctionCallArgumentsDone(openai_responses::ResponseFunctionCallArgumentsDoneEvent),
		/// Emitted when a content part is done.
		#[serde(rename = "response.content_part.done")]
		ResponseContentPartDone(openai_responses::ResponseContentPartDoneEvent),
		/// Emitted when an output item is marked done.
		#[serde(rename = "response.output_item.done")]
		ResponseOutputItemDone(openai_responses::ResponseOutputItemDoneEvent),
		/// Emitted when the model response is complete.
		#[serde(rename = "response.completed")]
		ResponseCompleted(openai_responses::ResponseCompletedEvent),
		/// An event that is emitted when a response finishes as incomplete.
		#[serde(rename = "response.incomplete")]
		ResponseIncomplete(openai_responses::ResponseIncompleteEvent),
		/// An event that is emitted when a response fails.
		#[serde(rename = "response.failed")]
		ResponseFailed(openai_responses::ResponseFailedEvent),
		/// Emitted when an error occurs.
		#[serde(rename = "error")]
		ResponseError(openai_responses::ResponseErrorEvent),
	}
}
