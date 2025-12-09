use std::collections::{HashMap, HashSet};
use std::time::Instant;

use agent_core::prelude::Strng;
use agent_core::strng;
use async_openai::types::{ChatCompletionMessageToolCallChunk, FunctionCallStream};
use bytes::Bytes;
use chrono;
use itertools::Itertools;
use rand::Rng;
use tracing::trace;

use crate::http::{Body, Response};

use crate::llm::openai::responses;
use crate::llm::{AIError, LLMInfo};
use crate::telemetry::log::AsyncLog;
use crate::*;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // Optional: model override for Bedrock API path
	pub region: Strng, // Required: AWS region
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("aws.bedrock");
}

impl Provider {
	pub async fn process_streaming(
		&self,
		log: AsyncLog<LLMInfo>,
		resp: Response,
		model: &str,
		input_format: crate::llm::InputFormat,
	) -> Response {
		let model = self.model.as_deref().unwrap_or(model).to_string();

		// Bedrock doesn't return an ID, so get one from the request... if we can
		let message_id = resp
			.headers()
			.get(http::x_headers::X_AMZN_REQUESTID)
			.and_then(|s| s.to_str().ok().map(|s| s.to_owned()))
			.unwrap_or_else(|| format!("{:016x}", rand::rng().random::<u64>()));

		match input_format {
			crate::llm::InputFormat::Completions => resp.map(|b| {
				translate_stream_to_completions(b, log.clone(), model.clone(), message_id.clone())
			}),
			crate::llm::InputFormat::Messages => {
				resp.map(|body| translate_stream_to_messages(body, log, model, message_id))
			},
			crate::llm::InputFormat::Responses => {
				resp.map(|body| translate_stream_to_responses(body, log, model, message_id))
			},
			crate::llm::InputFormat::CountTokens => {
				unreachable!("CountTokens should be handled by process_count_tokens_response")
			},
		}
	}

	pub fn get_path_for_route(
		&self,
		route_type: super::RouteType,
		streaming: bool,
		model: &str,
	) -> Strng {
		let model = self.model.as_deref().unwrap_or(model);
		match route_type {
			super::RouteType::AnthropicTokenCount => strng::format!("/model/{model}/count-tokens"),
			_ if streaming => strng::format!("/model/{model}/converse-stream"),
			_ => strng::format!("/model/{model}/converse"),
		}
	}

	pub fn get_host(&self) -> Strng {
		strng::format!("bedrock-runtime.{}.amazonaws.com", self.region)
	}
}

pub(super) fn translate_count_tokens_request(
	req: anthropic::CountTokensRequest,
	anthropic_version: &str,
) -> Result<types::CountTokensRequest, AIError> {
	use base64::Engine;

	let mut body = req.rest;

	// AWS Bedrock's count-tokens endpoint wraps InvokeModel, which requires a valid
	// Anthropic Messages API request. The `max_tokens` parameter is required by Anthropic's API.
	// We set it to 1 (the minimum valid value) since token counting doesn't generate output.
	body
		.entry("max_tokens")
		.or_insert(serde_json::Value::Number(1.into()));
	body
		.entry("anthropic_version")
		.or_insert(serde_json::Value::String(anthropic_version.into()));

	let body_json = serde_json::to_vec(&body).map_err(AIError::RequestMarshal)?;
	let body_b64 = base64::engine::general_purpose::STANDARD.encode(&body_json);

	Ok(types::CountTokensRequest {
		input: types::CountTokensInputInvokeModel {
			invoke_model: types::InvokeModelBody { body: body_b64 },
		},
	})
}

pub(super) fn process_count_tokens_request(
	count_req: anthropic::CountTokensRequest,
	anthropic_version: &str,
) -> Result<Vec<u8>, AIError> {
	let bedrock_req = translate_count_tokens_request(count_req, anthropic_version)?;
	serde_json::to_vec(&bedrock_req).map_err(AIError::RequestMarshal)
}

pub fn translate_count_tokens_response(bedrock_bytes: &[u8]) -> Result<Vec<u8>, AIError> {
	let resp: types::CountTokensResponse =
		serde_json::from_slice(bedrock_bytes).map_err(AIError::ResponseParsing)?;
	serde_json::to_vec(&resp).map_err(AIError::ResponseMarshal)
}

pub async fn process_count_tokens_response(
	resp: crate::http::Response,
) -> Result<crate::http::Response, anyhow::Error> {
	use crate::http;

	let lim = http::response_buffer_limit(&resp);
	let (parts, body) = resp.into_parts();
	let bytes = http::read_body_with_limit(body, lim).await?;

	if parts.status.is_success() {
		let response_bytes = translate_count_tokens_response(&bytes)
			.map_err(|e| anyhow::anyhow!("Failed to translate count_tokens response: {}", e))?;

		let mut parts = parts;
		parts.headers.remove(http::header::CONTENT_LENGTH);

		Ok(crate::http::Response::from_parts(
			parts,
			response_bytes.into(),
		))
	} else {
		Ok(crate::http::Response::from_parts(parts, bytes.into()))
	}
}

fn translate_stop_reason(resp: &StopReason) -> universal::FinishReason {
	match resp {
		StopReason::EndTurn => universal::FinishReason::Stop,
		StopReason::MaxTokens => universal::FinishReason::Length,
		StopReason::StopSequence => universal::FinishReason::Stop,
		StopReason::ContentFiltered => universal::FinishReason::ContentFilter,
		StopReason::GuardrailIntervened => universal::FinishReason::ContentFilter,
		StopReason::ToolUse => universal::FinishReason::ToolCalls,
		StopReason::ModelContextWindowExceeded => universal::FinishReason::Length,
	}
}

pub(super) fn translate_stream_to_completions(
	b: Body,
	log: AsyncLog<LLMInfo>,
	model: String,
	message_id: String,
) -> Body {
	// This is static for all chunks!
	let created = chrono::Utc::now().timestamp() as u32;
	let mut saw_token = false;
	// Track tool call JSON buffers by content block index
	let mut tool_calls: HashMap<i32, String> = HashMap::new();

	parse::aws_sse::transform(b, move |f| {
		let res = types::ConverseStreamOutput::deserialize(f).ok()?;
		let mk = |choices: Vec<universal::ChatChoiceStream>, usage: Option<universal::Usage>| {
			Some(universal::StreamResponse {
				id: message_id.clone(),
				model: model.clone(),
				object: "chat.completion.chunk".to_string(),
				system_fingerprint: None,
				service_tier: None,
				created,
				choices,
				usage,
			})
		};

		match res {
			types::ConverseStreamOutput::ContentBlockStart(start) => {
				// Track tool call starts for streaming
				if let Some(types::ContentBlockStart::ToolUse(tu)) = start.start {
					tool_calls.insert(start.content_block_index, String::new());
					// Emit the start of a tool call
					let d = universal::StreamResponseDelta {
						tool_calls: Some(vec![ChatCompletionMessageToolCallChunk {
							index: start.content_block_index as u32,
							id: Some(tu.tool_use_id),
							r#type: Some(universal::ToolType::Function),
							function: Some(FunctionCallStream {
								name: Some(tu.name),
								arguments: None,
							}),
						}]),
						..Default::default()
					};
					let choice = universal::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta: d,
						finish_reason: None,
					};
					mk(vec![choice], None)
				} else {
					// Text/reasoning starts don't need events in Universal format
					None
				}
			},
			types::ConverseStreamOutput::ContentBlockDelta(d) => {
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}

				let delta = d.delta.map(|delta| {
					let mut dr = universal::StreamResponseDelta::default();
					match delta {
						types::ContentBlockDelta::ReasoningContent(
							types::ReasoningContentBlockDelta::Text(t),
						) => {
							dr.reasoning_content = Some(t);
						},
						types::ContentBlockDelta::ReasoningContent(
							types::ReasoningContentBlockDelta::RedactedContent(_),
						) => {
							dr.reasoning_content = Some("[REDACTED]".to_string());
						},
						types::ContentBlockDelta::ReasoningContent(_) => {},
						types::ContentBlockDelta::Text(t) => {
							dr.content = Some(t);
						},
						types::ContentBlockDelta::ToolUse(tu) => {
							// Accumulate tool call JSON and emit deltas
							if let Some(json_buffer) = tool_calls.get_mut(&d.content_block_index) {
								json_buffer.push_str(&tu.input);
								dr.tool_calls = Some(vec![ChatCompletionMessageToolCallChunk {
									index: d.content_block_index as u32,
									id: None, // Only sent in the first chunk
									r#type: None,
									function: Some(FunctionCallStream {
										name: None,
										arguments: Some(tu.input),
									}),
								}]);
							}
						},
					};
					dr
				});

				if let Some(delta) = delta {
					let choice = universal::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta,
						finish_reason: None,
					};
					mk(vec![choice], None)
				} else {
					None
				}
			},
			types::ConverseStreamOutput::ContentBlockStop(stop) => {
				// Clean up tool call tracking for this content block
				tool_calls.remove(&stop.content_block_index);
				None
			},
			types::ConverseStreamOutput::MessageStart(start) => {
				// Just send a blob with the role
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta {
						role: Some(match start.role {
							types::Role::Assistant => universal::Role::Assistant,
							types::Role::User => universal::Role::User,
						}),
						..Default::default()
					},
					finish_reason: None,
				};
				mk(vec![choice], None)
			},
			types::ConverseStreamOutput::MessageStop(stop) => {
				let finish_reason = Some(translate_stop_reason(&stop.stop_reason));

				// Just send a blob with the finish reason
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta::default(),
					finish_reason,
				};
				mk(vec![choice], None)
			},
			types::ConverseStreamOutput::Metadata(metadata) => {
				if let Some(usage) = metadata.usage {
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
					});

					mk(
						vec![],
						Some(universal::Usage {
							prompt_tokens: usage.input_tokens as u32,
							completion_tokens: usage.output_tokens as u32,
							total_tokens: usage.total_tokens as u32,
							prompt_tokens_details: None,
							completion_tokens_details: None,
						}),
					)
				} else {
					None
				}
			},
		}
	})
}

/// Translate Bedrock streaming events to Anthropic Messages SSE format
///
/// This function converts Bedrock's binary event stream to Anthropic's SSE format
/// using typed MessagesStreamEvent structs for compile-time safety.
///
/// Note: Some events are synthesized when Bedrock doesn't emit ContentBlockStart
/// events for text/thinking content.
pub(super) fn translate_stream_to_messages(
	b: Body,
	log: AsyncLog<LLMInfo>,
	model: String,
	_message_id: String,
) -> Body {
	let mut saw_token = false;
	let mut seen_blocks: HashSet<i32> = HashSet::new();
	let mut pending_stop_reason: Option<types::StopReason> = None;
	let mut pending_usage: Option<types::TokenUsage> = None;

	parse::aws_sse::transform_multi(b, move |aws_event| {
		let event = match types::ConverseStreamOutput::deserialize(aws_event) {
			Ok(e) => e,
			Err(e) => {
				tracing::error!(error = %e, "failed to deserialize bedrock stream event");
				return vec![(
					"error",
					serde_json::json!({
						"type": "error",
						"error": {
							"type": "api_error",
							"message": "Stream processing error"
						}
					}),
				)];
			},
		};

		match event {
			types::ConverseStreamOutput::MessageStart(_start) => {
				let event = anthropic::MessagesStreamEvent::MessageStart {
					message: anthropic::MessagesResponse {
						id: generate_anthropic_message_id(),
						r#type: "message".to_string(),
						role: anthropic::Role::Assistant,
						content: vec![],
						model: model.clone(),
						stop_reason: None,
						stop_sequence: None,
						usage: anthropic::Usage {
							input_tokens: 0,
							output_tokens: 0,
							cache_creation_input_tokens: None,
							cache_read_input_tokens: None,
						},
					},
				};
				let (event_name, event_data) = event.into_sse_tuple();
				vec![(event_name, serde_json::to_value(event_data).unwrap())]
			},
			types::ConverseStreamOutput::ContentBlockStart(start) => {
				seen_blocks.insert(start.content_block_index);
				let content_block = match start.start {
					Some(types::ContentBlockStart::ToolUse(s)) => anthropic::ContentBlock::ToolUse {
						id: s.tool_use_id,
						name: s.name,
						input: serde_json::json!({}),
						cache_control: None,
					},
					Some(types::ContentBlockStart::ReasoningContent) => anthropic::ContentBlock::Thinking {
						thinking: String::new(),
						signature: String::new(),
					},
					_ => anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
						text: String::new(),
						citations: None,
						cache_control: None,
					}),
				};

				let event = anthropic::MessagesStreamEvent::ContentBlockStart {
					index: start.content_block_index as usize,
					content_block,
				};
				let (event_name, event_data) = event.into_sse_tuple();
				vec![(event_name, serde_json::to_value(event_data).unwrap())]
			},
			types::ConverseStreamOutput::ContentBlockDelta(delta) => {
				let mut out = Vec::new();

				// Synthesize ContentStart for first text/thinking delta on this index
				let first_for_index = !seen_blocks.contains(&delta.content_block_index);
				if first_for_index {
					seen_blocks.insert(delta.content_block_index);

					if let Some(ref d) = delta.delta {
						let content_block = match d {
							types::ContentBlockDelta::Text(_) => {
								Some(anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
									text: String::new(),
									citations: None,
									cache_control: None,
								}))
							},
							types::ContentBlockDelta::ReasoningContent(_) => {
								Some(anthropic::ContentBlock::Thinking {
									thinking: String::new(),
									signature: String::new(),
								})
							},
							types::ContentBlockDelta::ToolUse(_) => None,
						};

						if let Some(cb) = content_block {
							let event = anthropic::MessagesStreamEvent::ContentBlockStart {
								index: delta.content_block_index as usize,
								content_block: cb,
							};
							let (event_name, event_data) = event.into_sse_tuple();
							out.push((event_name, serde_json::to_value(event_data).unwrap()));
						}
					}
				}

				if let Some(d) = delta.delta {
					if !saw_token {
						saw_token = true;
						log.non_atomic_mutate(|r| {
							r.response.first_token = Some(Instant::now());
						});
					}

					let anthropic_delta = match d {
						types::ContentBlockDelta::Text(text) => {
							anthropic::ContentBlockDelta::TextDelta { text }
						},
						types::ContentBlockDelta::ReasoningContent(rc) => match rc {
							types::ReasoningContentBlockDelta::Text(t) => {
								anthropic::ContentBlockDelta::ThinkingDelta { thinking: t }
							},
							types::ReasoningContentBlockDelta::Signature(sig) => {
								anthropic::ContentBlockDelta::SignatureDelta { signature: sig }
							},
							types::ReasoningContentBlockDelta::RedactedContent(_) => {
								anthropic::ContentBlockDelta::ThinkingDelta {
									thinking: "[REDACTED]".to_string(),
								}
							},
							types::ReasoningContentBlockDelta::Unknown => {
								anthropic::ContentBlockDelta::ThinkingDelta {
									thinking: String::new(),
								}
							},
						},
						types::ContentBlockDelta::ToolUse(tu) => anthropic::ContentBlockDelta::InputJsonDelta {
							partial_json: tu.input,
						},
					};

					let event = anthropic::MessagesStreamEvent::ContentBlockDelta {
						index: delta.content_block_index as usize,
						delta: anthropic_delta,
					};
					let (event_name, event_data) = event.into_sse_tuple();
					out.push((event_name, serde_json::to_value(event_data).unwrap()));
				}

				out
			},
			types::ConverseStreamOutput::ContentBlockStop(stop) => {
				seen_blocks.remove(&stop.content_block_index);
				let event = anthropic::MessagesStreamEvent::ContentBlockStop {
					index: stop.content_block_index as usize,
				};
				let (event_name, event_data) = event.into_sse_tuple();
				vec![(event_name, serde_json::to_value(event_data).unwrap())]
			},
			types::ConverseStreamOutput::MessageStop(stop) => {
				pending_stop_reason = Some(stop.stop_reason);
				vec![]
			},
			types::ConverseStreamOutput::Metadata(meta) => {
				if let Some(usage) = meta.usage {
					pending_usage = Some(usage);
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
					});
				}

				let mut out = Vec::new();
				let stop = pending_stop_reason.take();
				let usage = pending_usage.take();

				if let (Some(stop_reason), Some(usage_data)) = (stop, usage) {
					let event = anthropic::MessagesStreamEvent::MessageDelta {
						delta: anthropic::MessageDelta {
							stop_reason: Some(translate_stop_reason_to_anthropic(stop_reason)),
							stop_sequence: None,
						},
						usage: to_anthropic_message_delta_usage(usage_data),
					};
					let (event_name, event_data) = event.into_sse_tuple();
					out.push((event_name, serde_json::to_value(event_data).unwrap()));
				}

				let event = anthropic::MessagesStreamEvent::MessageStop;
				let (event_name, event_data) = event.into_sse_tuple();
				out.push((event_name, serde_json::to_value(event_data).unwrap()));

				out
			},
		}
	})
}

pub(super) fn translate_stream_to_responses(
	b: Body,
	log: AsyncLog<LLMInfo>,
	model: String,
	_message_id: String,
) -> Body {
	let mut saw_token = false;
	let mut pending_stop_reason: Option<types::StopReason> = None;
	let mut pending_usage: Option<types::TokenUsage> = None;
	let mut seen_blocks: HashSet<i32> = HashSet::new();

	// Track tool calls for streaming: (index -> (item_id, name, json_buffer))
	let mut tool_calls: HashMap<i32, (String, String, String)> = HashMap::new();

	// Track sequence numbers and item IDs
	let mut sequence_number: u64 = 0;
	let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());

	// Track message item ID for text content
	let message_item_id = format!("msg_{:016x}", rand::rng().random::<u64>());

	parse::aws_sse::transform_multi(b, move |aws_event| {
		tracing::debug!("Raw AWS event - headers: {:?}", aws_event.headers);
		if let Ok(body_str) = std::str::from_utf8(&aws_event.body) {
			tracing::debug!("AWS event body: {}", body_str);
		}

		let event = match types::ConverseStreamOutput::deserialize(aws_event) {
			Ok(e) => e,
			Err(e) => {
				tracing::error!(error = %e, "failed to deserialize bedrock stream event");
				return vec![(
					"error",
					serde_json::json!({
						"type": "error",
						"error": {
							"message": "Stream processing error"
						}
					}),
				)];
			},
		};

		match event {
			types::ConverseStreamOutput::MessageStart(_start) => {
				let mut events = Vec::new();

				sequence_number += 1;
				let created_event = serde_json::json!({
					"type": "response.created",
					"sequence_number": sequence_number,
					"response": {
						"id": response_id.clone(),
						"object": "response",
						"model": model.clone(),
						"created_at": chrono::Utc::now().timestamp() as u64,
						"status": "in_progress"
					}
				});
				events.push(("event", created_event));

				sequence_number += 1;
				let item_added_event = serde_json::json!({
					"type": "response.output_item.added",
					"sequence_number": sequence_number,
					"output_index": 0,
					"item": {
						"type": "message",
						"id": message_item_id.clone(),
						"role": "assistant",
						"status": "in_progress",
						"content": []
					}
				});
				events.push(("event", item_added_event));

				events
			},
			types::ConverseStreamOutput::ContentBlockStart(start) => {
				seen_blocks.insert(start.content_block_index);

				match start.start {
					Some(types::ContentBlockStart::ToolUse(tu)) => {
						let tool_call_item_id = format!("call_{:016x}", rand::rng().random::<u64>());
						tool_calls.insert(
							start.content_block_index,
							(tool_call_item_id.clone(), tu.name.clone(), String::new()),
						);

						sequence_number += 1;
						let item_added_event = serde_json::json!({
							"type": "response.output_item.added",
							"sequence_number": sequence_number,
							"output_index": start.content_block_index as u32,
							"item": {
								"type": "function_call",
								"id": tool_call_item_id,
								"call_id": tool_call_item_id,
								"name": tu.name,
								"arguments": "",
								"status": "in_progress"
							}
						});

						vec![("event", item_added_event)]
					},
					Some(types::ContentBlockStart::Text) => {
						sequence_number += 1;
						let part_added_event = serde_json::json!({
							"type": "response.content_part.added",
							"sequence_number": sequence_number,
							"item_id": message_item_id.clone(),
							"output_index": start.content_block_index as u32,
							"content_index": 0,
							"part": {
								"type": "text",
								"text": ""
							}
						});

						vec![("event", part_added_event)]
					},
					_ => {
						sequence_number += 1;
						let part_added_event = serde_json::json!({
							"type": "response.content_part.added",
							"sequence_number": sequence_number,
							"item_id": message_item_id.clone(),
							"output_index": start.content_block_index as u32,
							"content_index": 0,
							"part": {
								"type": "text",
								"text": ""
							}
						});

						vec![("event", part_added_event)]
					},
				}
			},
			types::ConverseStreamOutput::ContentBlockDelta(delta) => {
				let mut out = Vec::new();

				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}

				if let Some(d) = delta.delta {
					match d {
						types::ContentBlockDelta::Text(text) => {
							sequence_number += 1;
							let delta_event = serde_json::json!({
								"type": "response.output_text.delta",
								"sequence_number": sequence_number,
								"item_id": message_item_id.clone(),
								"output_index": delta.content_block_index as u32,
								"content_index": 0,
								"delta": text
							});
							out.push(("event", delta_event));
						},
						types::ContentBlockDelta::ReasoningContent(rc) => match rc {
							types::ReasoningContentBlockDelta::Text(t) => {
								sequence_number += 1;
								let delta_event = serde_json::json!({
									"type": "response.output_text.delta",
									"sequence_number": sequence_number,
									"item_id": message_item_id.clone(),
									"output_index": delta.content_block_index as u32,
									"content_index": 0,
									"delta": t
								});
								out.push(("event", delta_event));
							},
							types::ReasoningContentBlockDelta::RedactedContent(_) => {
								sequence_number += 1;
								let delta_event = serde_json::json!({
									"type": "response.output_text.delta",
									"sequence_number": sequence_number,
									"item_id": message_item_id.clone(),
									"output_index": delta.content_block_index as u32,
									"content_index": 0,
									"delta": "[REDACTED]"
								});
								out.push(("event", delta_event));
							},
							_ => {},
						},
						types::ContentBlockDelta::ToolUse(tu) => {
							if let Some((item_id, _name, buffer)) = tool_calls.get_mut(&delta.content_block_index)
							{
								buffer.push_str(&tu.input);

								sequence_number += 1;
								let delta_event = serde_json::json!({
									"type": "response.function_call_arguments.delta",
									"sequence_number": sequence_number,
									"item_id": item_id.clone(),
									"output_index": delta.content_block_index as u32,
									"delta": tu.input
								});
								out.push(("event", delta_event));
							}
						},
					}
				}

				out
			},
			types::ConverseStreamOutput::ContentBlockStop(stop) => {
				let mut events = Vec::new();

				if let Some((item_id, name, buffer)) = tool_calls.remove(&stop.content_block_index) {
					sequence_number += 1;
					let args_done_event = serde_json::json!({
						"type": "response.function_call_arguments.done",
						"sequence_number": sequence_number,
						"item_id": item_id.clone(),
						"output_index": stop.content_block_index as u32,
						"name": name.clone(),
						"arguments": buffer.clone()
					});
					events.push(("event", args_done_event));

					sequence_number += 1;
					let item_done_event = serde_json::json!({
						"type": "response.output_item.done",
						"sequence_number": sequence_number,
						"output_index": stop.content_block_index as u32,
						"item": {
							"type": "function_call",
							"id": item_id.clone(),
							"call_id": item_id,
							"name": name,
							"arguments": buffer,
							"status": "completed"
						}
					});
					events.push(("event", item_done_event));
				} else if seen_blocks.remove(&stop.content_block_index) {
					sequence_number += 1;
					let part_done_event = serde_json::json!({
						"type": "response.content_part.done",
						"sequence_number": sequence_number,
						"item_id": message_item_id.clone(),
						"output_index": stop.content_block_index as u32,
						"content_index": 0,
						"part": {
							"type": "text"
						}
					});
					events.push(("event", part_done_event));
				}

				events
			},
			types::ConverseStreamOutput::MessageStop(stop) => {
				pending_stop_reason = Some(stop.stop_reason);
				vec![]
			},
			types::ConverseStreamOutput::Metadata(meta) => {
				if let Some(usage) = meta.usage {
					pending_usage = Some(usage);
					log.non_atomic_mutate(|r| {
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
					});
				}

				let mut out = Vec::new();

				sequence_number += 1;
				let message_done_event = serde_json::json!({
					"type": "response.output_item.done",
					"sequence_number": sequence_number,
					"output_index": 0,
					"item": {
						"type": "message",
						"id": message_item_id.clone(),
						"role": "assistant",
						"status": "completed"
					}
				});
				out.push(("event", message_done_event));

				let stop = pending_stop_reason.take();
				let usage_data = pending_usage.take();

				let usage_obj = usage_data.map(|u| {
					serde_json::json!({
						"input_tokens": u.input_tokens as u32,
						"output_tokens": u.output_tokens as u32,
						"total_tokens": (u.input_tokens + u.output_tokens) as u32,
						"input_tokens_details": {
							"cached_tokens": u.cache_read_input_tokens.unwrap_or(0) as u32
						},
						"output_tokens_details": {
							"reasoning_tokens": 0
						}
					})
				});

				sequence_number += 1;
				let done_event = match stop {
					Some(StopReason::EndTurn) | Some(StopReason::StopSequence) | None => {
						serde_json::json!({
							"type": "response.completed",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "completed",
								"usage": usage_obj
							}
						})
					},
					Some(StopReason::MaxTokens) | Some(StopReason::ModelContextWindowExceeded) => {
						serde_json::json!({
							"type": "response.incomplete",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "incomplete",
								"usage": usage_obj,
								"incomplete_details": {
									"reason": "max_tokens"
								}
							}
						})
					},
					Some(StopReason::ContentFiltered) | Some(StopReason::GuardrailIntervened) => {
						serde_json::json!({
							"type": "response.failed",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "failed",
								"usage": usage_obj,
								"error": {
									"code": "content_filter",
									"message": "Content filtered by guardrails"
								}
							}
						})
					},
					Some(StopReason::ToolUse) => {
						serde_json::json!({
							"type": "response.completed",
							"sequence_number": sequence_number,
							"response": {
								"id": response_id.clone(),
								"object": "response",
								"model": model.clone(),
								"created_at": chrono::Utc::now().timestamp() as u64,
								"status": "completed",
								"usage": usage_obj
							}
						})
					},
				};

				out.push(("event", done_event));
				out
			},
		}
	})
}

fn generate_anthropic_message_id() -> String {
	let timestamp = chrono::Utc::now().timestamp_millis();
	let random: u32 = rand::random();
	format!("msg_{:x}{:08x}", timestamp, random)
}

fn translate_stop_reason_to_anthropic(stop_reason: StopReason) -> anthropic::StopReason {
	match stop_reason {
		StopReason::EndTurn => anthropic::StopReason::EndTurn,
		StopReason::MaxTokens => anthropic::StopReason::MaxTokens,
		StopReason::ModelContextWindowExceeded => anthropic::StopReason::ModelContextWindowExceeded,
		StopReason::StopSequence => anthropic::StopReason::StopSequence,
		StopReason::ToolUse => anthropic::StopReason::ToolUse,
		StopReason::ContentFiltered | StopReason::GuardrailIntervened => anthropic::StopReason::Refusal,
	}
}

fn to_anthropic_message_delta_usage(
	usage: types::TokenUsage,
) -> crate::llm::anthropic::types::MessageDeltaUsage {
	crate::llm::anthropic::types::MessageDeltaUsage {
		input_tokens: usage.input_tokens,
		output_tokens: usage.output_tokens,
		cache_creation_input_tokens: usage.cache_write_input_tokens,
		cache_read_input_tokens: usage.cache_read_input_tokens,
	}
}

#[cfg(test)]
mod tests {
	use ::http::HeaderMap;
	use serde_json::json;

	use super::*;

	#[test]
	fn test_metadata_from_header() {
		let provider = Provider {
			model: None,
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

		// Simulate transformation CEL setting x-bedrock-metadata header
		let mut headers = HeaderMap::new();
		headers.insert(
			"x-bedrock-metadata",
			r#"{"user_id": "user123", "department": "engineering"}"#
				.parse()
				.unwrap(),
		);

		let req = anthropic::MessagesRequest {
			model: "anthropic.claude-3-sonnet".to_string(),
			messages: vec![anthropic::Message {
				role: anthropic::Role::User,
				content: vec![anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				})],
			}],
			max_tokens: 100,
			metadata: None,
			system: None,
			stop_sequences: vec![],
			stream: false,
			temperature: None,
			top_k: None,
			top_p: None,
			tools: None,
			tool_choice: None,
			thinking: None,
		};

		let out = translate_request_messages(req, &provider, Some(&headers)).unwrap();
		let metadata = out.request_metadata.unwrap();

		assert_eq!(metadata.get("user_id"), Some(&"user123".to_string()));
		assert_eq!(metadata.get("department"), Some(&"engineering".to_string()));
	}

	#[test]
	fn test_translate_request_messages_maps_top_k_from_typed() {
		let provider = Provider {
			model: Some(strng::new("anthropic.claude-3")),
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

		let req = anthropic::MessagesRequest {
			model: "anthropic.claude-3".to_string(),
			messages: vec![anthropic::Message {
				role: anthropic::Role::User,
				content: vec![anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
					text: "hello".to_string(),
					citations: None,
					cache_control: None,
				})],
			}],
			system: None,
			max_tokens: 256,
			stop_sequences: vec![],
			stream: false,
			temperature: Some(0.7),
			top_p: Some(0.9),
			top_k: Some(7),
			tools: None,
			tool_choice: None,
			metadata: None,
			thinking: None,
		};

		let out = translate_request_messages(req, &provider, None).unwrap();
		let inf = out.inference_config.unwrap();
		assert_eq!(inf.top_k, Some(7));
	}

	#[test]
	fn test_extract_beta_headers_variants() {
		let headers = HeaderMap::new();
		assert!(extract_beta_headers(&headers).unwrap().is_none());

		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			"prompt-caching-2024-07-31".parse().unwrap(),
		);
		assert_eq!(
			extract_beta_headers(&headers).unwrap().unwrap(),
			vec![json!("prompt-caching-2024-07-31")]
		);

		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			"cache-control-2024-08-15,computer-use-2024-10-22"
				.parse()
				.unwrap(),
		);
		assert_eq!(
			extract_beta_headers(&headers).unwrap().unwrap(),
			vec![
				json!("cache-control-2024-08-15"),
				json!("computer-use-2024-10-22"),
			]
		);

		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			" cache-control-2024-08-15 , computer-use-2024-10-22 "
				.parse()
				.unwrap(),
		);
		assert_eq!(
			extract_beta_headers(&headers).unwrap().unwrap(),
			vec![
				json!("cache-control-2024-08-15"),
				json!("computer-use-2024-10-22"),
			]
		);

		let mut headers = HeaderMap::new();
		headers.append(
			"anthropic-beta",
			"cache-control-2024-08-15".parse().unwrap(),
		);
		headers.append("anthropic-beta", "computer-use-2024-10-22".parse().unwrap());
		let mut beta_features = extract_beta_headers(&headers)
			.unwrap()
			.unwrap()
			.into_iter()
			.map(|v| v.as_str().unwrap().to_string())
			.collect::<Vec<_>>();
		beta_features.sort();
		assert_eq!(
			beta_features,
			vec![
				"cache-control-2024-08-15".to_string(),
				"computer-use-2024-10-22".to_string(),
			]
		);
	}
}
