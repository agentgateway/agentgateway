use agent_core::prelude::Strng;
use agent_core::strng;
use async_openai::types::{FinishReason, ReasoningEffort};
use bytes::Bytes;
use chrono;

use crate::http::{Body, Response};
use crate::llm::anthropic::types::{
	ContentBlock, ContentBlockDelta, MessagesErrorResponse, MessagesRequest, MessagesResponse,
	MessagesStreamEvent, StopReason,
};
use crate::llm::{AIError, LLMResponse, universal};
use crate::telemetry::log::AsyncLog;
use crate::{parse, *};
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
		mut req: universal::Request,
	) -> Result<MessagesRequest, AIError> {
		if let Some(provider_model) = &self.model {
			req.model = Some(provider_model.to_string());
		} else if req.model.is_none() {
			return Err(AIError::MissingField("model not specified".into()));
		}
		let anthropic_message = translate_request(req);
		Ok(anthropic_message)
	}
	pub async fn process_response(&self, bytes: &Bytes) -> Result<universal::Response, AIError> {
		let resp =
			serde_json::from_slice::<MessagesResponse>(bytes).map_err(AIError::ResponseParsing)?;
		let openai = translate_response(resp);
		Ok(openai)
	}

	pub async fn process_streaming(&self, log: AsyncLog<LLMResponse>, resp: Response) -> Response {
		resp.map(|b| translate_stream(b, log))
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

pub(super) fn translate_response(resp: MessagesResponse) -> universal::Response {
	// Convert Anthropic content blocks to OpenAI message content
	let mut tool_calls: Vec<universal::MessageToolCall> = Vec::new();
	let mut content = None;
	let mut reasoning_content = None;
	for block in resp.content {
		match block {
			types::ContentBlock::Text { text } => content = Some(text.clone()),
			types::ContentBlock::Image { .. } => continue, // Skip images in response for now
			ContentBlock::ToolUse { id, name, input } => {
				let Some(args) = serde_json::to_string(&input).ok() else {
					continue;
				};
				tool_calls.push(universal::MessageToolCall {
					id: id.clone(),
					r#type: universal::ToolType::Function,
					function: universal::FunctionCall {
						name: name.clone(),
						arguments: args,
					},
				});
			},
			ContentBlock::ToolResult { .. } => {
				// Should be on the request path, not the response path
				continue;
			},
			// For now we ignore Redacted and signature think through a better approach as this may be needed
			ContentBlock::Thinking { thinking, .. } => {
				reasoning_content = Some(thinking);
			},
			ContentBlock::RedactedThinking { .. } => {},
		}
	}
	let message = universal::ResponseMessage {
		role: universal::Role::Assistant,
		content,
		tool_calls: if tool_calls.is_empty() {
			None
		} else {
			Some(tool_calls)
		},
		#[allow(deprecated)]
		function_call: None,
		refusal: None,
		audio: None,
		reasoning_content,
		extra: None,
	};
	let finish_reason = resp.stop_reason.as_ref().map(translate_stop_reason);
	// Only one choice for anthropic
	let choice = universal::ChatChoice {
		index: 0,
		message,
		finish_reason,
		logprobs: None,
	};

	let choices = vec![choice];
	// Convert usage from Anthropic format to OpenAI format
	let usage = universal::Usage {
		prompt_tokens: resp.usage.input_tokens as u32,
		completion_tokens: resp.usage.output_tokens as u32,
		total_tokens: (resp.usage.input_tokens + resp.usage.output_tokens) as u32,
		prompt_tokens_details: None,
		completion_tokens_details: None,
	};

	universal::Response {
		id: resp.id,
		object: "chat.completion".to_string(),
		// No date in anthropic response so just call it "now"
		created: chrono::Utc::now().timestamp() as u32,
		model: resp.model,
		choices,
		usage: Some(usage),
		service_tier: None,
		system_fingerprint: None,
	}
}

pub(super) fn translate_request(req: universal::Request) -> types::MessagesRequest {
	let max_tokens = universal::max_tokens(&req);
	let stop_sequences = universal::stop_sequence(&req);
	// Anthropic has all system prompts in a single field. Join them
	let system = req
		.messages
		.iter()
		.filter_map(|msg| {
			if universal::message_role(msg) == universal::SYSTEM_ROLE {
				universal::message_text(msg).map(|s| s.to_string())
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
		.filter(|msg| universal::message_role(msg) != universal::SYSTEM_ROLE)
		.filter_map(|msg| {
			let role = match universal::message_role(msg) {
				universal::ASSISTANT_ROLE => types::Role::Assistant,
				// Default to user for other roles
				_ => types::Role::User,
			};

			universal::message_text(msg)
				.map(|s| {
					vec![types::ContentBlock::Text {
						text: s.to_string(),
					}]
				})
				.map(|content| types::Message { role, content })
		})
		.collect();

	let tools = if let Some(tools) = req.tools {
		let mapped_tools: Vec<_> = tools
			.iter()
			.map(|tool| types::Tool {
				name: tool.function.name.clone(),
				description: tool.function.description.clone(),
				input_schema: tool.function.parameters.clone().unwrap_or_default(),
			})
			.collect();
		Some(mapped_tools)
	} else {
		None
	};
	let metadata = req.user.map(|user| types::Metadata {
		fields: HashMap::from([("user_id".to_string(), user)]),
	});

	let tool_choice = match req.tool_choice {
		Some(universal::ToolChoiceOption::Named(universal::NamedToolChoice {
			r#type: _,
			function,
		})) => Some(types::ToolChoice::Tool {
			name: function.name,
		}),
		Some(universal::ToolChoiceOption::Auto) => Some(types::ToolChoice::Auto),
		Some(universal::ToolChoiceOption::Required) => Some(types::ToolChoice::Any),
		Some(universal::ToolChoiceOption::None) => Some(types::ToolChoice::None),
		None => None,
	};
	let thinking = match &req.reasoning_effort {
		// Arbitrary constants come from LiteLLM defaults.
		// OpenRouter uses percentages which may be more appropriate though (https://openrouter.ai/docs/use-cases/reasoning-tokens#reasoning-effort-level)
		Some(ReasoningEffort::Low) => Some(types::ThinkingInput::Enabled {
			budget_tokens: 1024,
		}),
		Some(ReasoningEffort::Medium) => Some(types::ThinkingInput::Enabled {
			budget_tokens: 2048,
		}),
		Some(ReasoningEffort::High) => Some(types::ThinkingInput::Enabled {
			budget_tokens: 4096,
		}),
		None => None,
	};
	types::MessagesRequest {
		messages,
		system,
		model: req.model.unwrap_or_default(),
		max_tokens,
		stop_sequences,
		stream: req.stream.unwrap_or(false),
		temperature: req.temperature,
		top_p: req.top_p,
		top_k: None, // OpenAI doesn't have top_k
		tools,
		tool_choice,
		metadata,
		thinking,
	}
}

pub(super) fn translate_stream(b: Body, log: AsyncLog<LLMResponse>) -> Body {
	let mut message_id = None;
	let mut model = String::new();
	let created = chrono::Utc::now().timestamp() as u32;
	// let mut finish_reason = None;
	let mut input_tokens = 0;
	let mut saw_token = false;
	// https://docs.anthropic.com/en/docs/build-with-claude/streaming
	parse::sse::json_transform::<MessagesStreamEvent, universal::StreamResponse>(b, move |f| {
		let mk = |choices: Vec<universal::ChatChoiceStream>, usage: Option<universal::Usage>| {
			Some(universal::StreamResponse {
				id: message_id.clone().unwrap_or_else(|| "unknown".to_string()),
				model: model.clone(),
				object: "chat.completion.chunk".to_string(),
				system_fingerprint: None,
				service_tier: None,
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
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.first_token = Some(Instant::now());
					});
				}
				let mut dr = universal::StreamResponseDelta::default();
				match delta {
					ContentBlockDelta::TextDelta { text } => {
						dr.content = Some(text);
					},
					ContentBlockDelta::ThinkingDelta { thinking } => dr.reasoning_content = Some(thinking),
					// TODO
					ContentBlockDelta::InputJsonDelta { .. } => {},
					ContentBlockDelta::SignatureDelta { .. } => {},
				};
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: dr,
					finish_reason: None,
				};
				mk(vec![choice], None)
			},
			MessagesStreamEvent::MessageDelta { usage, delta: _ } => {
				// TODO
				// finish_reason = delta.stop_reason.as_ref().map(translate_stop_reason);
				log.non_atomic_mutate(|r| {
					r.output_tokens = Some(usage.output_tokens as u64);
					if let Some(inp) = r.input_tokens_from_response {
						r.total_tokens = Some(inp + usage.output_tokens as u64)
					}
				});
				mk(
					vec![],
					Some(universal::Usage {
						prompt_tokens: input_tokens as u32,
						completion_tokens: usage.output_tokens as u32,

						total_tokens: (input_tokens + usage.output_tokens) as u32,

						prompt_tokens_details: None,
						completion_tokens_details: None,
					}),
				)
			},
			MessagesStreamEvent::ContentBlockStop { .. } => None,
			MessagesStreamEvent::MessageStop => None,
			MessagesStreamEvent::Ping => None,
		}
	})
}

fn translate_stop_reason(resp: &types::StopReason) -> FinishReason {
	match resp {
		StopReason::EndTurn => universal::FinishReason::Stop,
		StopReason::MaxTokens => universal::FinishReason::Length,
		StopReason::StopSequence => universal::FinishReason::Stop,
		StopReason::ToolUse => universal::FinishReason::ToolCalls,
		StopReason::Refusal => universal::FinishReason::ContentFilter,
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
		/// Tool use content
		#[serde(rename = "tool_use")]
		ToolUse {
			id: String,
			name: String,
			input: serde_json::Value,
		},
		/// Tool result content
		#[serde(rename = "tool_result")]
		ToolResult {
			tool_use_id: String,
			content: String,
		},
		Thinking {
			thinking: String,
			signature: String,
		},
		#[serde(rename = "redacted_thinking")]
		RedactedThinking {
			data: String,
		},
	}

	#[derive(Clone, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub struct Message {
		pub role: Role,
		pub content: Vec<ContentBlock>,
	}

	#[derive(Serialize, Default, Debug)]
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
		pub temperature: Option<f32>,
		/// Use nucleus sampling.
		///
		/// In nucleus sampling, we compute the cumulative distribution over all the options for each
		/// subsequent token in decreasing probability order and cut it off once it reaches a particular
		/// probability specified by top_p. You should either alter temperature or top_p, but not both.
		/// Recommended for advanced use cases only. You usually only need to use temperature.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f32>,
		/// Only sample from the top K options for each subsequent token.
		/// Used to remove "long tail" low probability responses. Learn more technical details here.
		/// Recommended for advanced use cases only. You usually only need to use temperature.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_k: Option<usize>,
		/// Tools that the model may use
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tools: Option<Vec<Tool>>,
		/// How the model should use tools
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_choice: Option<ToolChoice>,
		/// Request metadata
		#[serde(skip_serializing_if = "Option::is_none")]
		pub metadata: Option<Metadata>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub thinking: Option<ThinkingInput>,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum ThinkingInput {
		Enabled { budget_tokens: u64 },
		Disabled {},
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
	#[allow(clippy::enum_variant_names)]
	pub enum ContentBlockDelta {
		TextDelta { text: String },
		InputJsonDelta { partial_json: String },
		ThinkingDelta { thinking: String },
		SignatureDelta { signature: String },
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
		ToolUse,
		Refusal,
	}

	/// Billing and rate-limit usage.
	#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
	pub struct Usage {
		/// The number of input tokens which were used.
		pub input_tokens: usize,

		/// The number of output tokens which were used.
		pub output_tokens: usize,
	}

	/// Tool definition
	#[derive(Debug, Serialize, Deserialize)]
	pub struct Tool {
		/// Name of the tool
		pub name: String,
		/// Description of the tool
		#[serde(skip_serializing_if = "Option::is_none")]
		pub description: Option<String>,
		/// JSON schema for tool input
		pub input_schema: serde_json::Value,
	}

	/// Tool choice configuration
	#[derive(Debug, Serialize, Deserialize)]
	#[serde(tag = "type")]
	pub enum ToolChoice {
		/// Let model choose whether to use tools
		#[serde(rename = "auto")]
		Auto,
		/// Model must use one of the provided tools
		#[serde(rename = "any")]
		Any,
		/// Model must use a specific tool
		#[serde(rename = "tool")]
		Tool { name: String },
		/// Model must not use any tools
		#[serde(rename = "none")]
		None,
	}

	/// Message metadata
	#[derive(Debug, Serialize, Deserialize, Default)]
	pub struct Metadata {
		/// Custom metadata fields
		#[serde(flatten)]
		pub fields: std::collections::HashMap<String, String>,
	}
}
