use agent_core::prelude::Strng;
use agent_core::strng;
use bytes::Bytes;
use chrono;
use itertools::Itertools;
use serde::Serialize;
use serde_json::Value;

use crate::http::Response;
use crate::llm::anthropic::types::{
	ContentBlockDelta, MessagesErrorResponse, MessagesRequest, MessagesResponse, MessagesStreamEvent,
};
use crate::llm::universal::{ChatCompletionChoiceStream, ChatCompletionRequest, Usage};
use crate::llm::{AIError, LLMRequest, LLMResponse, universal};
use crate::telemetry::log::AsyncLog;
use crate::{llm, parse, *};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("anthropic");
}
pub const DEFAULT_HOST_STR: &str = "api.anthropic.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);
pub const DEFAULT_PATH: &str = "/v1/messages";

impl Provider {
	pub async fn process_request(
		&self,
		mut req: universal::ChatCompletionRequest,
	) -> Result<MessagesRequest, AIError> {
		if let Some(model) = &self.model {
			req.model = model.to_string();
		}
		let anthropic_message = translate_request(req);
		Ok(anthropic_message)
	}
	pub async fn process_response(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionResponse, AIError> {
		let resp =
			serde_json::from_slice::<MessagesResponse>(bytes).map_err(AIError::ResponseParsing)?;
		let openai = translate_response(resp);
		Ok(openai)
	}

	pub async fn process_streaming(&self, log: AsyncLog<LLMResponse>, resp: Response) -> Response {
		resp.map(|b| {
			let mut message_id = None;
			let mut model = String::new();
			let mut created = chrono::Utc::now().timestamp();
			let mut current_content = String::new();
			let mut finish_reason = None;
			let mut input_tokens = 0;
			// https://docs.anthropic.com/en/docs/build-with-claude/streaming
			parse::sse::json_transform::<MessagesStreamEvent, universal::ChatCompletionStreamResponse>(
				b,
				move |f| {
					let mk = |choices: Vec<ChatCompletionChoiceStream>, usage: Option<Usage>| {
						Some(universal::ChatCompletionStreamResponse {
							id: message_id.clone(),
							model: model.clone(),
							object: "chat.completion.chunk".to_string(),
							system_fingerprint: None,
							created,
							choices,
							usage,
						})
					};
					// ignore errors... what else can we do?
					let f = f.ok()?;

					// Extract info we need
					match f {
						MessagesStreamEvent::MessageStart { message } => {
							message_id = Some(message.id);
							model = message.model.clone();
							input_tokens = message.usage.input_tokens;
							log.non_atomic_mutate(|r| {
								r.output_tokens = Some(message.usage.output_tokens as u64);
								r.input_tokens_from_response = Some(message.usage.input_tokens as u64);
								r.provider_model = Some(strng::new(&message.model))
							});
							// no need to respond with anything yet
							None
						},

						MessagesStreamEvent::ContentBlockStart { .. } => {
							// There is never(?) any content here
							None
						},
						MessagesStreamEvent::ContentBlockDelta { delta, .. } => {
							let ContentBlockDelta::TextDelta { text } = delta;
							let choice = universal::ChatCompletionChoiceStream {
								index: 0,
								delta: universal::ChatCompletionMessageForResponseDelta {
									role: None,
									content: Some(text),
									refusal: None,
									name: None,
									tool_calls: None,
								},
								finish_reason: None,
							};
							mk(vec![choice], None)
						},
						MessagesStreamEvent::MessageDelta { usage, delta } => {
							finish_reason = delta.stop_reason.map(|reason| match reason {
								types::StopReason::EndTurn => universal::FinishReason::stop,
								types::StopReason::MaxTokens => universal::FinishReason::length,
								types::StopReason::StopSequence => universal::FinishReason::stop,
							});
							log.non_atomic_mutate(|r| {
								r.output_tokens = Some(usage.output_tokens as u64);
								if let Some(inp) = r.input_tokens_from_response {
									r.total_tokens = Some(inp + usage.output_tokens as u64)
								}
							});
							mk(
								vec![],
								Some(universal::Usage {
									prompt_tokens: usage.output_tokens as i32,
									completion_tokens: input_tokens as i32,
									total_tokens: (input_tokens + usage.output_tokens) as i32,
								}),
							)
						},
						MessagesStreamEvent::ContentBlockStop { .. } => None,
						MessagesStreamEvent::MessageStop => None,
						MessagesStreamEvent::Ping => None,
					}
				},
			)
		})
	}

	pub async fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp =
			serde_json::from_slice::<MessagesErrorResponse>(bytes).map_err(AIError::ResponseParsing)?;
		translate_error(resp)
	}
}

pub(super) fn translate_error(
	resp: MessagesErrorResponse,
) -> Result<universal::ChatCompletionErrorResponse, AIError> {
	Ok(universal::ChatCompletionErrorResponse {
		event_id: None,
		error: universal::ChatCompletionError {
			r#type: "invalid_request_error".to_string(),
			message: resp.error.message,
			param: None,
			code: None,
			event_id: None,
		},
	})
}

pub(super) fn translate_response(resp: MessagesResponse) -> universal::ChatCompletionResponse {
	// Convert Anthropic content blocks to OpenAI message content
	let choices = resp
		.content
		.iter()
		.filter_map(|block| {
			let text = match block {
				types::ContentBlock::Text { text } => Some(text.clone()),
				types::ContentBlock::Image { .. } => return None, // Skip images in response for now
			};
			let message = universal::ChatCompletionMessageForResponse {
				role: universal::MessageRole::assistant,
				content: text,
				reasoning_content: None,
				name: None,
				tool_calls: None,
			};
			let finish_reason = resp.stop_reason.map(|reason| match reason {
				types::StopReason::EndTurn => universal::FinishReason::stop,
				types::StopReason::MaxTokens => universal::FinishReason::length,
				types::StopReason::StopSequence => universal::FinishReason::stop,
			});
			// Only one choice for anthropic
			let choice = universal::ChatCompletionChoice {
				index: 0,
				message,
				finish_reason,
				finish_details: None,
			};
			Some(choice)
		})
		.collect::<Vec<_>>();

	// Convert usage from Anthropic format to OpenAI format
	let usage = universal::Usage {
		prompt_tokens: resp.usage.input_tokens as i32,
		completion_tokens: resp.usage.output_tokens as i32,
		total_tokens: (resp.usage.input_tokens + resp.usage.output_tokens) as i32,
	};

	universal::ChatCompletionResponse {
		id: Some(resp.id),
		object: "chat.completion".to_string(),
		// No date in anthropic response so just call it "now"
		created: chrono::Utc::now().timestamp(),
		model: resp.model,
		choices,
		usage,
		system_fingerprint: None,
	}
}

pub(super) fn translate_request(req: ChatCompletionRequest) -> types::MessagesRequest {
	// Anthropic has all system prompts in a single field. Join them
	let system = req
		.messages
		.iter()
		.filter_map(|msg| {
			if msg.role == universal::MessageRole::system {
				match &msg.content {
					universal::Content::Text(text) => Some(text.clone()),
					_ => None, // Skip non-text system messages
				}
			} else {
				None
			}
		})
		.collect::<Vec<String>>()
		.join("\n");

	// Convert messages to Anthropic format
	let messages = req
		.messages
		.iter()
		.filter(|msg| msg.role != universal::MessageRole::system)
		.map(|msg| {
			let role = match msg.role {
				universal::MessageRole::user => types::Role::User,
				universal::MessageRole::assistant => types::Role::Assistant,
				_ => types::Role::User, // Default to user for other roles
			};

			let content = match &msg.content {
				universal::Content::Text(text) => {
					vec![types::ContentBlock::Text { text: text.clone() }]
				},
				universal::Content::ImageUrl(urls) => {
					urls
						.iter()
						.map(|img_url| {
							if let Some(url) = &img_url.image_url {
								types::ContentBlock::Image {
									source: url.url.clone(),
									media_type: "image/jpeg".to_string(), // Default to JPEG
									data: "".to_string(),                 // Base64 data would go here if using base64
								}
							} else {
								types::ContentBlock::Text {
									text: img_url.text.clone().unwrap_or_default(),
								}
							}
						})
						.collect()
				},
			};

			types::Message { role, content }
		})
		.collect();

	types::MessagesRequest {
		messages,
		system,
		model: req.model,
		max_tokens: req.max_tokens.unwrap_or(4096) as usize,
		stop_sequences: req.stop.unwrap_or_default(),
		stream: req.stream.unwrap_or(false),
		temperature: req.temperature,
		top_p: req.top_p,
		top_k: None, // OpenAI doesn't have top_k
	}
}

pub(super) mod types {
	use serde::{Deserialize, Serialize};

	use crate::serdes::is_default;

	#[derive(Copy, Clone, Deserialize, Serialize, Debug, PartialEq, Eq, Default)]
	#[serde(rename_all = "snake_case")]
	pub enum Role {
		#[default]
		User,
		Assistant,
	}

	#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum ContentBlock {
		Text {
			text: String,
		},
		Image {
			source: String,
			media_type: String,
			data: String,
		},
	}

	#[derive(Clone, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub struct Message {
		pub role: Role,
		pub content: Vec<ContentBlock>,
	}

	#[derive(Clone, Serialize, Default, Debug, PartialEq)]
	pub struct MessagesRequest {
		/// The User/Assistent prompts.
		pub messages: Vec<Message>,
		/// The System prompt.
		#[serde(skip_serializing_if = "String::is_empty")]
		pub system: String,
		/// The model to use.
		pub model: String,
		/// The maximum number of tokens to generate before stopping.
		pub max_tokens: usize,
		/// The stop sequences to use.
		#[serde(skip_serializing_if = "Vec::is_empty")]
		pub stop_sequences: Vec<String>,
		/// Whether to incrementally stream the response.
		#[serde(default, skip_serializing_if = "is_default")]
		pub stream: bool,
		/// Amount of randomness injected into the response.
		///
		/// Defaults to 1.0. Ranges from 0.0 to 1.0. Use temperature closer to 0.0 for analytical /
		/// multiple choice, and closer to 1.0 for creative and generative tasks. Note that even
		/// with temperature of 0.0, the results will not be fully deterministic.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub temperature: Option<f64>,
		/// Use nucleus sampling.
		///
		/// In nucleus sampling, we compute the cumulative distribution over all the options for each
		/// subsequent token in decreasing probability order and cut it off once it reaches a particular
		/// probability specified by top_p. You should either alter temperature or top_p, but not both.
		/// Recommended for advanced use cases only. You usually only need to use temperature.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f64>,
		/// Only sample from the top K options for each subsequent token.
		/// Used to remove "long tail" low probability responses. Learn more technical details here.
		/// Recommended for advanced use cases only. You usually only need to use temperature.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_k: Option<usize>,
	}

	/// Response body for the Messages API.
	#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
	pub struct MessagesResponse {
		/// Unique object identifier.
		/// The format and length of IDs may change over time.
		pub id: String,
		/// Object type.
		/// For Messages, this is always "message".
		pub r#type: String,
		/// Conversational role of the generated message.
		/// This will always be "assistant".
		pub role: Role,
		/// Content generated by the model.
		/// This is an array of content blocks, each of which has a type that determines its shape.
		/// Currently, the only type in responses is "text".
		///
		/// Example:
		/// `[{"type": "text", "text": "Hi, I'm Claude."}]`
		///
		/// If the request input messages ended with an assistant turn, then the response content
		/// will continue directly from that last turn. You can use this to constrain the model's
		/// output.
		///
		/// For example, if the input messages were:
		/// `[ {"role": "user", "content": "What's the Greek name for Sun? (A) Sol (B) Helios (C) Sun"},
		///    {"role": "assistant", "content": "The best answer is ("} ]`
		///
		/// Then the response content might be:
		/// `[{"type": "text", "text": "B)"}]`
		pub content: Vec<ContentBlock>,
		/// The model that handled the request.
		pub model: String,
		/// The reason that we stopped.
		/// This may be one the following values:
		/// - "end_turn": the model reached a natural stopping point
		/// - "max_tokens": we exceeded the requested max_tokens or the model's maximum
		/// - "stop_sequence": one of your provided custom stop_sequences was generated
		///
		/// Note that these values are different than those in /v1/complete, where end_turn and
		/// stop_sequence were not differentiated.
		///
		/// In non-streaming mode this value is always non-null. In streaming mode, it is null
		/// in the message_start event and non-null otherwise.
		pub stop_reason: Option<StopReason>,
		/// Which custom stop sequence was generated, if any.
		/// This value will be a non-null string if one of your custom stop sequences was generated.
		pub stop_sequence: Option<String>,
		/// Billing and rate-limit usage.
		/// Anthropic's API bills and rate-limits by token counts, as tokens represent the underlying
		/// cost to our systems.
		///
		/// Under the hood, the API transforms requests into a format suitable for the model. The
		/// model's output then goes through a parsing stage before becoming an API response. As a
		/// result, the token counts in usage will not match one-to-one with the exact visible
		/// content of an API request or response.
		///
		/// For example, output_tokens will be non-zero, even for an empty string response from Claude.
		pub usage: Usage,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum MessagesStreamEvent {
		MessageStart {
			message: MessagesResponse,
		},
		ContentBlockStart {
			index: usize,
			content_block: ContentBlock,
		},
		ContentBlockDelta {
			index: usize,
			delta: ContentBlockDelta,
		},
		ContentBlockStop {
			index: usize,
		},
		MessageDelta {
			delta: MessageDelta,
			usage: MessageDeltaUsage,
		},
		MessageStop,
		Ping,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum ContentBlockDelta {
		TextDelta { text: String },
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	pub struct MessageDeltaUsage {
		pub output_tokens: usize,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	pub struct MessageDelta {
		/// The reason that we stopped.
		/// This may be one the following values:
		/// - "end_turn": the model reached a natural stopping point
		/// - "max_tokens": we exceeded the requested max_tokens or the model's maximum
		/// - "stop_sequence": one of your provided custom stop_sequences was generated
		///
		/// Note that these values are different than those in /v1/complete, where end_turn and
		/// stop_sequence were not differentiated.
		///
		/// In non-streaming mode this value is always non-null. In streaming mode, it is null
		/// in the message_start event and non-null otherwise.
		pub stop_reason: Option<StopReason>,
		/// Which custom stop sequence was generated, if any.
		/// This value will be a non-null string if one of your custom stop sequences was generated.
		pub stop_sequence: Option<String>,
	}

	/// Response body for the Messages API.
	#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
	pub struct MessagesErrorResponse {
		pub r#type: String,
		pub error: MessagesError,
	}

	#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
	pub struct MessagesError {
		pub r#type: String,
		pub message: String,
	}

	/// Reason for stopping the response generation.
	#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub enum StopReason {
		/// The model reached a natural stopping point.
		EndTurn,
		/// The requested max_tokens or the model's maximum was exceeded.
		MaxTokens,
		/// One of the provided custom stop_sequences was generated.
		StopSequence,
	}

	/// Billing and rate-limit usage.
	#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
	pub struct Usage {
		/// The number of input tokens which were used.
		pub input_tokens: usize,

		/// The number of output tokens which were used.
		pub output_tokens: usize,
	}
}
