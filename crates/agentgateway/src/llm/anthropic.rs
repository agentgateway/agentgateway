use agent_core::prelude::Strng;
use agent_core::strng;
use async_openai::types::{
	ChatCompletionRequestToolMessageContent, ChatCompletionRequestToolMessageContentPart,
	FinishReason, ReasoningEffort,
};
use bytes::Bytes;
use chrono;
use itertools::Itertools;

use crate::http::{Body, Response};

use crate::llm::types::ResponseType;
use crate::llm::{AIError, InputFormat, LLMInfo};
use crate::telemetry::log::AsyncLog;
use crate::{parse, *};

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
			InputFormat::Responses => {
				unreachable!("Responses format should not be routed to Anthropic provider")
			},
			InputFormat::CountTokens => {
				unreachable!("CountTokens should be handled by process_count_tokens_response")
			},
		}
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
						ContentBlockDelta::CitationsDelta { .. } => {},
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
