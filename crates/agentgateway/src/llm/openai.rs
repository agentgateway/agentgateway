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
	use bytes::Bytes;

	use crate::llm::universal::{RequestType, ResponseType};
	use crate::llm::{AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse};

	// Re-export async-openai Responses API types for cleaner usage
	pub use async_openai::types::responses::{
		Content, ContentType, CreateResponse, FunctionCall, Input, InputContent, InputItem,
		InputMessage, OutputContent, OutputMessage, OutputStatus, OutputText, ResponseEvent, Role,
		ToolChoice, ToolChoiceMode, ToolDefinition,
	};

	pub mod passthrough {
		use super::*;
		use serde::{Deserialize, Serialize};

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

		impl RequestType for Request {
			fn model(&mut self) -> Option<&mut String> {
				self.model.as_mut()
			}

			fn prepend_prompts(&mut self, prompts: Vec<crate::llm::SimpleChatCompletionMessage>) {
				if prompts.is_empty() {
					return;
				}

				// Convert prepend prompts to InputItems
				let prepend_items: Vec<InputItem> = prompts
					.into_iter()
					.map(|msg| {
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
					})
					.collect();

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

			fn to_llm_request(
				&self,
				provider: agent_core::strng::Strng,
				tokenize: bool,
			) -> Result<LLMRequest, AIError> {
				let model = agent_core::strng::new(self.model.as_deref().unwrap_or_default());
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

			fn get_messages(&self) -> Vec<crate::llm::SimpleChatCompletionMessage> {
				match &self.input {
					Input::Text(text) => {
						vec![crate::llm::SimpleChatCompletionMessage {
							role: agent_core::strng::literal!("user"),
							content: agent_core::strng::new(text),
						}]
					},
					Input::Items(items) => items
						.iter()
						.filter_map(|item| match item {
							InputItem::Message(msg) => {
								let content = match &msg.content {
									InputContent::TextInput(text) => agent_core::strng::new(text),
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
										agent_core::strng::new(&text)
									},
								};

								let role = match msg.role {
									Role::User => agent_core::strng::literal!("user"),
									Role::Assistant => agent_core::strng::literal!("assistant"),
									Role::System => agent_core::strng::literal!("system"),
									Role::Developer => agent_core::strng::literal!("developer"),
								};

								Some(crate::llm::SimpleChatCompletionMessage { role, content })
							},
							_ => None,
						})
						.collect(),
				}
			}

			fn set_messages(&mut self, messages: Vec<crate::llm::SimpleChatCompletionMessage>) {
				let items: Vec<InputItem> = messages
					.into_iter()
					.map(|msg| {
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
					})
					.collect();

				self.input = Input::Items(items);
			}

			fn to_openai(&self) -> Result<Vec<u8>, AIError> {
				// Passthrough - just serialize
				serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
			}

			fn to_bedrock(
				&self,
				provider: &crate::llm::bedrock::Provider,
				headers: Option<&::http::HeaderMap>,
				prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
			) -> Result<Vec<u8>, AIError> {
				// Convert passthrough Request to typed CreateResponse (same pattern as Anthropic Messages)
				let typed =
					crate::json::convert::<_, CreateResponse>(self).map_err(AIError::RequestMarshal)?;
				let bedrock_request = crate::llm::bedrock::translate_request_responses(
					&typed,
					provider,
					headers,
					prompt_caching,
				)?;
				serde_json::to_vec(&bedrock_request).map_err(AIError::RequestMarshal)
			}
		}

		impl ResponseType for Response {
			fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
				LLMResponse {
					input_tokens: self.usage.as_ref().map(|u| u.input_tokens),
					output_tokens: self.usage.as_ref().map(|u| u.output_tokens),
					total_tokens: self
						.usage
						.as_ref()
						.map(|u| u.input_tokens + u.output_tokens),
					provider_model: Some(agent_core::strng::new(&self.model)),
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

			fn serialize(&self) -> serde_json::Result<Vec<u8>> {
				serde_json::to_vec(&self)
			}
		}
	}

	pub fn process_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp =
			serde_json::from_slice::<passthrough::Response>(bytes).map_err(AIError::ResponseParsing)?;
		Ok(Box::new(resp))
	}

	pub async fn process_streaming(
		log: crate::telemetry::log::AsyncLog<crate::llm::LLMInfo>,
		resp: crate::http::Response,
	) -> crate::http::Response {
		let buffer_limit = crate::http::response_buffer_limit(&resp);
		let mut saw_token = false;

		resp.map(|b| {
			crate::parse::sse::json_passthrough::<ResponseEvent>(b, buffer_limit, move |event| {
				let Some(Ok(event)) = event else {
					return;
				};

				match event {
					ResponseEvent::ResponseCreated(created) => {
						log.non_atomic_mutate(|r| {
							if let Some(model) = &created.response.model {
								r.response.provider_model = Some(agent_core::strng::new(model));
							}
							if let Some(usage) = &created.response.usage {
								r.response.input_tokens = Some(usage.input_tokens as u64);
								r.response.output_tokens = Some(usage.output_tokens as u64);
								r.response.total_tokens = Some(usage.total_tokens as u64);
							}
						});
					},
					ResponseEvent::ResponseOutputTextDelta(_) => {
						if !saw_token {
							saw_token = true;
							log.non_atomic_mutate(|r| {
								r.response.first_token = Some(std::time::Instant::now());
							});
						}
					},
					ResponseEvent::ResponseCompleted(completed) => {
						log.non_atomic_mutate(|r| {
							if let Some(model) = &completed.response.model {
								r.response.provider_model = Some(agent_core::strng::new(model));
							}
							if let Some(usage) = &completed.response.usage {
								r.response.input_tokens = Some(usage.input_tokens as u64);
								r.response.output_tokens = Some(usage.output_tokens as u64);
								r.response.total_tokens = Some(usage.total_tokens as u64);
							}
						});
					},
					_ => {
						// Ignore other events
					},
				}
			})
		})
	}
}
