use agent_core::prelude::Strng;
use agent_core::strng;
use async_openai::types::{
	ChatCompletionRequestToolMessageContent, ChatCompletionRequestToolMessageContentPart,
	FinishReason, ReasoningEffort,
};
use bytes::Bytes;
use chrono;

use crate::http::{Body, Response};
use crate::llm::anthropic::types::{
	ContentBlock, ContentBlockDelta, MessagesErrorResponse, MessagesRequest, MessagesResponse,
	MessagesStreamEvent, StopReason, ThinkingInput, ToolResultContent, ToolResultContentPart,
};
use crate::llm::universal::{RequestSystemMessage, RequestVendorExtensions, ResponseType};
use crate::llm::{AIError, InputFormat, LLMInfo, universal};
use crate::telemetry::log::AsyncLog;
use crate::{parse, *};
use itertools::Itertools;

#[apply(schema!)]
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
	pub async fn process_streaming(
		&self,
		log: AsyncLog<LLMInfo>,
		resp: Response,
		input_format: InputFormat,
	) -> Response {
		let buffer = http::response_buffer_limit(&resp);
		match input_format {
			InputFormat::Completions => resp.map(|b| translate_stream(b, buffer, log)),
			InputFormat::Messages => resp.map(|b| passthrough_stream(b, buffer, log)),
		}
	}

	pub fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp =
			serde_json::from_slice::<MessagesErrorResponse>(bytes).map_err(AIError::ResponseParsing)?;
		translate_error(resp)
	}
}

pub fn process_response(
	bytes: &Bytes,
	input_format: InputFormat,
) -> Result<Box<dyn ResponseType>, AIError> {
	match input_format {
		InputFormat::Completions => {
			let resp =
				serde_json::from_slice::<MessagesResponse>(bytes).map_err(AIError::ResponseParsing)?;
			let openai = translate_response(resp);
			let passthrough = json::convert::<_, universal::passthrough::Response>(&openai)
				.map_err(AIError::ResponseParsing)?;
			Ok(Box::new(passthrough))
		},
		InputFormat::Messages => {
			let resp =
				serde_json::from_slice::<passthrough::Response>(bytes).map_err(AIError::ResponseParsing)?;

			Ok(Box::new(resp))
		},
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
			types::ContentBlock::Text(types::ContentTextBlock { text, .. }) => {
				content = Some(text.clone())
			},
			ContentBlock::ToolUse {
				id, name, input, ..
			}
			| ContentBlock::ServerToolUse {
				id, name, input, ..
			} => {
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

			// not currently supported
			types::ContentBlock::Image { .. } => continue,
			ContentBlock::Document(_) => continue,
			ContentBlock::SearchResult(_) => continue,
			ContentBlock::Unknown => continue,
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
					vec![types::ContentBlock::Text(types::ContentTextBlock {
						text: s.to_string(),
						citations: None,
						cache_control: None,
					})]
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
	let thinking = if let Some(budget) = req.vendor_extensions.thinking_budget_tokens {
		Some(types::ThinkingInput::Enabled {
			budget_tokens: budget,
		})
	} else {
		match &req.reasoning_effort {
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
		}
	};
	types::MessagesRequest {
		messages,
		system: if system.is_empty() {
			None
		} else {
			Some(system)
		},
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

pub(super) fn translate_stream(b: Body, buffer_limit: usize, log: AsyncLog<LLMInfo>) -> Body {
	let mut message_id = None;
	let mut model = String::new();
	let created = chrono::Utc::now().timestamp() as u32;
	// let mut finish_reason = None;
	let mut input_tokens = 0;
	let mut saw_token = false;
	// https://docs.anthropic.com/en/docs/build-with-claude/streaming
	parse::sse::json_transform::<MessagesStreamEvent, universal::StreamResponse>(
		b,
		buffer_limit,
		move |f| {
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
						r.response.output_tokens = Some(message.usage.output_tokens as u64);
						r.response.input_tokens = Some(message.usage.input_tokens as u64);
						r.response.provider_model = Some(strng::new(&message.model))
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
							r.response.first_token = Some(Instant::now());
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
						r.response.output_tokens = Some(usage.output_tokens as u64);
						if let Some(inp) = r.response.input_tokens {
							r.response.total_tokens = Some(inp + usage.output_tokens as u64)
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
		},
	)
}

pub(super) fn passthrough_stream(b: Body, buffer_limit: usize, log: AsyncLog<LLMInfo>) -> Body {
	let mut saw_token = false;
	// https://docs.anthropic.com/en/docs/build-with-claude/streaming
	parse::sse::json_passthrough::<MessagesStreamEvent>(b, buffer_limit, move |f| {
		// ignore errors... what else can we do?
		let Some(Ok(f)) = f else { return };

		// Extract info we need
		match f {
			MessagesStreamEvent::MessageStart { message } => {
				log.non_atomic_mutate(|r| {
					r.response.output_tokens = Some(message.usage.output_tokens as u64);
					r.response.input_tokens = Some(message.usage.input_tokens as u64);
					r.response.provider_model = Some(strng::new(&message.model))
				});
			},
			MessagesStreamEvent::ContentBlockDelta { .. } => {
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}
			},
			MessagesStreamEvent::MessageDelta { usage, delta: _ } => {
				log.non_atomic_mutate(|r| {
					r.response.output_tokens = Some(usage.output_tokens as u64);
					if let Some(inp) = r.response.input_tokens {
						r.response.total_tokens = Some(inp + usage.output_tokens as u64)
					}
				});
			},
			MessagesStreamEvent::ContentBlockStart { .. }
			| MessagesStreamEvent::ContentBlockStop { .. }
			| MessagesStreamEvent::MessageStop
			| MessagesStreamEvent::Ping => {},
		}
	})
}

pub(super) fn translate_anthropic_response(_req: universal::Response) -> types::MessagesResponse {
	// TODO: implement this
	types::MessagesResponse {
		id: "".to_string(),
		r#type: "".to_string(),
		role: Default::default(),
		content: vec![],
		model: "".to_string(),
		stop_reason: None,
		stop_sequence: None,
		usage: types::Usage {
			input_tokens: 0,
			output_tokens: 0,
		},
	}
}
pub(super) fn translate_anthropic_request(req: types::MessagesRequest) -> universal::Request {
	let types::MessagesRequest {
		messages,
		system,
		model,
		max_tokens,
		stop_sequences,
		stream,
		temperature,
		top_p,
		top_k,
		tools,
		tool_choice,
		metadata,
		thinking,
	} = req;
	let mut msgs: Vec<universal::RequestMessage> = Vec::new();

	// Handle the system prompt
	if let Some(system) = system {
		msgs.push(universal::RequestMessage::System(
			RequestSystemMessage::from(system),
		));
	}

	// Convert messages from Anthropic to universal format
	for msg in messages {
		match msg.role {
			types::Role::User => {
				let mut user_text = String::new();
				for block in msg.content {
					match block {
						types::ContentBlock::Text(types::ContentTextBlock { text, .. }) => {
							if !user_text.is_empty() {
								user_text.push('\n');
							}
							user_text.push_str(&text);
						},
						types::ContentBlock::ToolResult {
							tool_use_id,
							content,
							..
						} => {
							msgs.push(
								universal::RequestToolMessage {
									tool_call_id: tool_use_id,
									content: match content {
										ToolResultContent::Text(t) => t.into(),
										ToolResultContent::Array(parts) => {
											ChatCompletionRequestToolMessageContent::Array(
												parts
													.into_iter()
													.filter_map(|p| match p {
														ToolResultContentPart::Text(types::ContentTextBlock {
															text, ..
														}) => Some(ChatCompletionRequestToolMessageContentPart::Text(
															text.into(),
														)),
														// Other types are not supported
														_ => None,
													})
													.collect_vec(),
											)
										},
									},
								}
								.into(),
							);
						},
						// Image content is not directly supported in universal::Message::User in this form.
						// This would require a different content format not represented here.
						types::ContentBlock::Image { .. } => {}, // Image content is not directly supported in universal::Message::User in this form.
						// This would require a different content format not represented here.
						// ToolUse blocks are expected from assistants, not users.
						types::ContentBlock::ServerToolUse { .. } | types::ContentBlock::ToolUse { .. } => {}, // ToolUse blocks are expected from assistants, not users.

						// Other content block types are not expected from the user in a request.
						_ => {},
					}
				}
				if !user_text.is_empty() {
					msgs.push(
						universal::RequestUserMessage {
							content: user_text.into(),
							name: None,
						}
						.into(),
					);
				}
			},
			types::Role::Assistant => {
				let mut assistant_text = None;
				let mut tool_calls = Vec::new();
				for block in msg.content {
					match block {
						types::ContentBlock::Text(types::ContentTextBlock { text, .. }) => {
							assistant_text = Some(text);
						},
						types::ContentBlock::ToolUse {
							id, name, input, ..
						} => {
							tool_calls.push(universal::MessageToolCall {
								id,
								r#type: universal::ToolType::Function,
								function: universal::FunctionCall {
									name,
									// It's assumed that the input is a JSON object that can be stringified.
									arguments: serde_json::to_string(&input).unwrap_or_default(),
								},
							});
						},
						ContentBlock::Thinking { .. } => {
							// TODO
						},
						ContentBlock::RedactedThinking { .. } => {
							// TODO
						},
						// Other content block types are not expected from the assistant in a request.
						_ => {},
					}
				}
				if assistant_text.is_some() || !tool_calls.is_empty() {
					msgs.push(
						universal::RequestAssistantMessage {
							content: assistant_text.map(Into::into),
							tool_calls: if tool_calls.is_empty() {
								None
							} else {
								Some(tool_calls)
							},
							..Default::default()
						}
						.into(),
					);
				}
			},
		}
	}

	let tools = tools
		.into_iter()
		.flat_map(|tools| tools.into_iter())
		.map(|tool| universal::Tool {
			r#type: universal::ToolType::Function,
			function: universal::FunctionObject {
				name: tool.name,
				description: tool.description,
				parameters: Some(tool.input_schema),
				strict: None,
			},
		})
		.collect_vec();
	let tool_choice = tool_choice.map(|choice| match choice {
		types::ToolChoice::Auto => universal::ToolChoiceOption::Auto,
		types::ToolChoice::Any => universal::ToolChoiceOption::Required,
		types::ToolChoice::Tool { name } => {
			universal::ToolChoiceOption::Named(universal::NamedToolChoice {
				r#type: universal::ToolType::Function,
				function: universal::FunctionName { name },
			})
		},
		types::ToolChoice::None => universal::ToolChoiceOption::None,
	});

	universal::Request {
		model: Some(model),
		messages: msgs,
		stream: Some(stream),
		temperature,
		top_p,
		max_completion_tokens: Some(max_tokens as u32),
		stop: if stop_sequences.is_empty() {
			None
		} else {
			Some(universal::Stop::StringArray(stop_sequences))
		},
		tools: if tools.is_empty() { None } else { Some(tools) },
		tool_choice,
		parallel_tool_calls: None,
		user: metadata.and_then(|m| m.fields.get("user_id").cloned()),

		vendor_extensions: RequestVendorExtensions {
			top_k,
			thinking_budget_tokens: thinking.and_then(|t| match t {
				ThinkingInput::Enabled { budget_tokens } => Some(budget_tokens),
				ThinkingInput::Disabled { .. } => None,
			}),
		},

		// The following OpenAI fields are not supported by Anthropic and are set to None:
		frequency_penalty: None,
		logit_bias: None,
		logprobs: None,
		top_logprobs: None,
		n: None,
		modalities: None,
		prediction: None,
		audio: None,
		presence_penalty: None,
		response_format: None,
		seed: None,
		#[allow(deprecated)]
		function_call: None,
		#[allow(deprecated)]
		functions: None,
		metadata: None,
		#[allow(deprecated)]
		max_tokens: None,
		service_tier: None,
		web_search_options: None,
		stream_options: None,
		store: None,
		reasoning_effort: None,
	}
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
	use crate::serdes::is_default;
	use serde::{Deserialize, Deserializer, Serialize};
	use serde_json::Value;

	#[derive(Copy, Clone, Deserialize, Serialize, Debug, PartialEq, Eq, Default)]
	#[serde(rename_all = "snake_case")]
	pub enum Role {
		#[default]
		User,
		Assistant,
	}
	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentTextBlock {
		pub text: String,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub citations: Option<Value>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentImageBlock {
		pub source: Value,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentSearchResultBlock {
		pub content: Vec<Value>,
		pub source: String,
		pub title: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentDocumentBlock {
		pub source: Value,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub citations: Option<Value>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub context: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub title: Option<String>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum ContentBlock {
		Text(ContentTextBlock),
		Image(ContentImageBlock),
		Document(ContentDocumentBlock),
		SearchResult(ContentSearchResultBlock),
		Thinking {
			thinking: String,
			signature: String,
		},
		RedactedThinking {
			data: String,
		},
		/// Tool use content
		ToolUse {
			id: String,
			name: String,
			input: serde_json::Value,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
		/// Tool result content
		ToolResult {
			tool_use_id: String,
			content: ToolResultContent,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
			#[serde(skip_serializing_if = "Option::is_none")]
			is_error: Option<bool>,
		},
		ServerToolUse {
			id: String,
			name: String,
			input: serde_json::Value,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
		// There are LOTs of possible values; since we don't support them all, just allow them without failing
		#[serde(other)]
		Unknown,
	}

	#[derive(Debug, Serialize, Deserialize, Clone)]
	#[serde(untagged)]
	pub enum ToolResultContent {
		/// The text contents of the tool message.
		Text(String),
		/// An array of content parts with a defined type. For tool messages, only type `text` is supported.
		Array(Vec<ToolResultContentPart>),
	}

	#[derive(Debug, Serialize, Deserialize, Clone)]
	#[serde(tag = "type")]
	pub enum ToolResultContentPart {
		Text(ContentTextBlock),
		Image(ContentImageBlock),
		Document(ContentDocumentBlock),
		SearchResult(ContentSearchResultBlock),
	}

	#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum CacheControlEphemeral {
		Ephemeral {
			#[serde(default)]
			#[serde(skip_serializing_if = "Option::is_none")]
			ttl: Option<String>,
		},
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct Message {
		pub role: Role,
		#[serde(deserialize_with = "deserialize_content")]
		pub content: Vec<ContentBlock>,
	}

	// Custom deserializer that handles both string and array formats
	fn deserialize_content<'de, D>(deserializer: D) -> Result<Vec<ContentBlock>, D::Error>
	where
		D: Deserializer<'de>,
	{
		use serde::de::Error;
		use serde_json::Value;

		let value = Value::deserialize(deserializer)?;

		match value {
			// If it's a string, wrap it in a Text content block
			Value::String(text) => Ok(vec![ContentBlock::Text(ContentTextBlock {
				text,
				citations: None,
				cache_control: None,
			})]),
			// If it's an array, deserialize normally
			Value::Array(_) => Vec::<ContentBlock>::deserialize(value).map_err(D::Error::custom),
			// Reject other types
			_ => Err(D::Error::custom(
				"content must be either a string or an array",
			)),
		}
	}

	#[derive(Deserialize, Serialize, Default, Debug)]
	pub struct MessagesRequest {
		/// The User/Assistent prompts.
		pub messages: Vec<Message>,
		/// The System prompt.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub system: Option<String>,
		/// The model to use.
		pub model: String,
		/// The maximum number of tokens to generate before stopping.
		pub max_tokens: usize,
		/// The stop sequences to use.
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
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
	#[derive(Debug, Serialize, Deserialize, Clone)]
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

	#[derive(Clone, Serialize, Deserialize, Debug)]
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
pub mod passthrough {
	use crate::llm::policy::webhook::{Message, ResponseChoice};
	use crate::llm::universal::{RequestType, ResponseType};
	use crate::llm::{
		AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, SimpleChatCompletionMessage,
		anthropic, num_tokens_from_anthropic_messages,
	};
	use crate::{json, llm};
	use agent_core::prelude::Strng;
	use agent_core::strng;
	use itertools::Itertools;
	use serde::{Deserialize, Serialize};

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Request {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub model: Option<String>,
		pub messages: Vec<RequestMessage>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub temperature: Option<f32>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stream: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub max_tokens: Option<u64>,

		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct RequestMessage {
		pub role: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub content: Option<RequestContent>,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	impl RequestMessage {
		pub fn message_text(&self) -> Option<&str> {
			self.content.as_ref().and_then(|c| match c {
				RequestContent::Text(t) => Some(t.as_str()),
				// TODO?
				RequestContent::Array(_) => None,
			})
		}
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	#[serde(untagged)]
	pub enum RequestContent {
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

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Response {
		pub model: String,
		pub usage: Usage,
		pub content: Vec<Content>,
		#[serde(flatten, default)]
		pub rest: serde_json::Value,
	}

	#[derive(Debug, Deserialize, Clone, Serialize)]
	pub struct Content {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub text: Option<String>,
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
		fn prepend_prompts(&mut self, prompts: Vec<llm::SimpleChatCompletionMessage>) {
			self
				.messages
				.splice(..0, prompts.into_iter().map(convert_message));
		}

		fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
			let model = strng::new(self.model.as_deref().unwrap_or_default());
			let input_tokens = if tokenize {
				let tokens = num_tokens_from_anthropic_messages(&model, &self.messages)?;
				Some(tokens)
			} else {
				None
			};
			// Pass the original body through
			let llm = LLMRequest {
				input_tokens,
				input_format: InputFormat::Messages,
				request_model: model,
				provider,
				streaming: self.stream.unwrap_or_default(),
				params: LLMRequestParams {
					temperature: self.temperature.map(Into::into),
					top_p: self.top_p.map(Into::into),
					frequency_penalty: None,
					presence_penalty: None,
					seed: None,
					max_tokens: self.max_tokens,
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
							RequestContent::Text(t) => Some(strng::new(t)),
							// TODO?
							RequestContent::Array(_) => None,
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

		fn to_openai(&self) -> Result<Vec<u8>, AIError> {
			let typed =
				json::convert::<_, anthropic::MessagesRequest>(self).map_err(AIError::RequestMarshal)?;
			let xlated = anthropic::translate_anthropic_request(typed);
			serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
		}

		fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
			serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
		}
	}

	fn convert_message(r: SimpleChatCompletionMessage) -> RequestMessage {
		RequestMessage {
			role: r.role.to_string(),
			content: Some(RequestContent::Text(r.content.to_string())),
			rest: Default::default(),
		}
	}

	impl ResponseType for Response {
		fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
			LLMResponse {
				input_tokens: Some(self.usage.input_tokens),
				output_tokens: Some(self.usage.output_tokens),
				total_tokens: Some(self.usage.output_tokens + self.usage.input_tokens),
				provider_model: Some(strng::new(&self.model)),
				completion: if include_completion_in_log {
					Some(
						self
							.content
							.iter()
							.flat_map(|c| c.text.clone())
							.collect_vec(),
					)
				} else {
					None
				},
				first_token: Default::default(),
			}
		}

		fn set_webhook_choices(&mut self, choices: Vec<ResponseChoice>) -> anyhow::Result<()> {
			if self.content.len() != choices.len() {
				anyhow::bail!("webhook response message count mismatch");
			}
			for (m, wh) in self.content.iter_mut().zip(choices.into_iter()) {
				m.text = Some(wh.message.content.to_string());
			}
			Ok(())
		}

		fn to_webhook_choices(&self) -> Vec<ResponseChoice> {
			self
				.content
				.iter()
				.map(|c| {
					let content = c.text.clone().unwrap_or_default();
					ResponseChoice {
						message: Message {
							role: "assistant".into(),
							content: content.into(),
						},
					}
				})
				.collect()
		}

		fn serialize(&self) -> serde_json::Result<Vec<u8>> {
			serde_json::to_vec(&self)
		}
	}
}
