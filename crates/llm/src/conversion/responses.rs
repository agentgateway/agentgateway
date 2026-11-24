use std::collections::BTreeMap;
use std::time::Instant;

use agent_core::strng::{self, Strng};
use agent_http::Body;
use serde::Deserialize;

use crate::types::detect;
use crate::{OutputMessage, OutputMessagePart, StreamingUsageGuard, parse, types};

#[allow(clippy::large_enum_variant)] // The large variant is used 99% of the time so just always use it.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum StreamResponse {
	// Attempt to parse it properly
	Typed(types::responses::typed::ResponseStreamEvent),
	// Fallback to detect mode. This is useful if the provider breaks the spec (which happens with custom
	// providers but also OpenAI themselves, often)
	Raw(detect::StreamResponse),
}

pub fn passthrough_stream(
	b: Body,
	buffer_limit: usize,
	mut log: StreamingUsageGuard,
	log_content: crate::LogContentFields,
) -> Body {
	let mut saw_token = false;
	let mut last_token_at: Option<Instant> = None;
	let mut completion = log_content.completion.then(String::new);
	let mut tool_calls = log_content.tool_calls.then(BTreeMap::new);
	parse::sse::json_passthrough::<StreamResponse>(b, buffer_limit, move |event| {
		let Some(Ok(event)) = event else {
			// Stream ended ([DONE]): flush completion if not already set via ResponseCompleted
			if event.is_none() {
				log.update(|r| {
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
					if r.response.output_messages.is_none() {
						r.response.output_messages = take_output_messages(&mut tool_calls, None);
					}
				});
			}
			return;
		};
		let event = match event {
			StreamResponse::Typed(e) => e,
			StreamResponse::Raw(raw) => {
				detect::amend_from_stream_response(&mut log, &raw);
				return;
			},
		};
		match event {
			types::responses::typed::ResponseStreamEvent::ResponseCreated(created) => {
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&created.response.model));
					r.response.response_id = Some(strng::new(&created.response.id));
					r.response.service_tier = created
						.response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str);
					if let Some(usage) = &created.response.usage {
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
						r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
						r.response.cache_creation_input_tokens = usage
							.input_tokens_details
							.cache_write_tokens
							.map(|tokens| tokens as u64);
						r.response.cache_creation_12h_input_tokens = usage
							.input_tokens_details
							.cache_write_tokens_details
							.as_ref()
							.and_then(|d| d.ttl_12h_tokens)
							.map(|t| t as u64);
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
				});
			},
			types::responses::typed::ResponseStreamEvent::ResponseOutputTextDelta(ref delta) => {
				let now = Instant::now();
				if !saw_token {
					saw_token = true;
					last_token_at = Some(now);
					log.update(|r| {
						r.response.first_token = Some(now);
					});
				} else if let Some(prev) = last_token_at.replace(now) {
					let gap = now.duration_since(prev);
					log.update(|r| r.response.inter_chunk_latencies.record(gap));
				}
				if let Some(c) = completion.as_mut() {
					c.push_str(&delta.delta);
				}
			},
			types::responses::typed::ResponseStreamEvent::ResponseOutputItemDone(done) => {
				if let Some(tool_calls) = tool_calls.as_mut()
					&& let Some(part) = types::responses::output_item_tool_call_part(&done.item)
				{
					tool_calls.insert(done.output_index, part);
				}
			},
			types::responses::typed::ResponseStreamEvent::ResponseIncomplete(e) => {
				record_provider_response(&log, &e.response);
			},
			types::responses::typed::ResponseStreamEvent::ResponseFailed(e) => {
				record_provider_response(&log, &e.response);
			},
			types::responses::typed::ResponseStreamEvent::ResponseCompleted(completed) => {
				record_provider_response(&log, &completed.response);
				let finish_reason = types::serialize_str(&completed.response.status);
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&completed.response.model));
					r.response.service_tier = completed
						.response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str);
					if let Some(usage) = &completed.response.usage {
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
						r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
						r.response.cache_creation_input_tokens = usage
							.input_tokens_details
							.cache_write_tokens
							.map(|tokens| tokens as u64);
						r.response.cache_creation_12h_input_tokens = usage
							.input_tokens_details
							.cache_write_tokens_details
							.as_ref()
							.and_then(|d| d.ttl_12h_tokens)
							.map(|t| t as u64);
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
					r.response.output_messages = take_output_messages(&mut tool_calls, finish_reason.clone());
				});
			},
			_ => {},
		}
	})
}

pub(crate) fn record_provider_response(
	log: &StreamingUsageGuard,
	resp: &types::responses::typed::Response,
) {
	let reason = types::responses::typed_finish_reason(resp);
	log.update(|r| {
		r.response.response_id = Some(strng::new(&resp.id));
		if let Some(reason) = &reason {
			r.response.set_finish_reason(0, reason.clone());
		}
	});
}

fn take_output_messages(
	tool_calls: &mut Option<BTreeMap<u32, OutputMessagePart>>,
	finish_reason: Option<Strng>,
) -> Option<Vec<OutputMessage>> {
	let content: Vec<_> = std::mem::take(tool_calls.as_mut()?).into_values().collect();
	(!content.is_empty()).then(|| {
		vec![OutputMessage {
			role: strng::literal!("assistant"),
			content,
			finish_reason,
		}]
	})
}

pub mod from_messages {
	use std::collections::{BTreeMap, HashMap, HashSet};
	use std::time::Instant;

	use agent_core::strng;
	use agent_http::Body;
	use bytes::Bytes;
	use rand::RngExt;
	use serde_json::{Map, Value};
	use types::messages::typed as messages;
	use types::responses::typed as responses;

	use crate::conversion::ir;
	use crate::parse::sse::SseJsonEvent;
	use crate::types::ResponseType;
	use crate::{
		AIError, OutputMessagePart, StreamingUsageGuard, logged_response_parsing, parse, types,
	};

	pub fn translate(req: &types::messages::Request) -> Result<Vec<u8>, AIError> {
		let xlated = translate_request(req)?;
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub fn translate_request(
		req: &types::messages::Request,
	) -> Result<types::responses::Request, AIError> {
		let mut dropped = Vec::new();
		let request = ir::messages::parse_request(req, &mut dropped)?;
		let request = ir::responses::render_request(request, &mut dropped)?;
		if !dropped.is_empty() {
			let dropped = dropped
				.iter()
				.map(|dropped| (dropped.redacted_path(), dropped.reason()))
				.collect::<Vec<_>>();
			tracing::debug!(?dropped, "messages request fields not sent to responses");
		}
		Ok(request)
	}

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<responses::Response>(bytes)
			.map_err(logged_response_parsing(bytes))?;
		let provider_usage = resp.usage.as_ref().map(|u| super::super::ProviderUsage {
			input_tokens: u.input_tokens as u64,
			total_tokens: u.total_tokens as u64,
			reasoning_tokens: Some(u.output_tokens_details.reasoning_tokens as u64),
			cache_creation_12h_input_tokens: u
				.input_tokens_details
				.cache_write_tokens_details
				.as_ref()
				.and_then(|d| d.ttl_12h_tokens)
				.map(|t| t as u64),
			..Default::default()
		});
		let id = Some(strng::new(&resp.id));
		let finish_reasons = types::responses::typed_finish_reason(&resp).map(|r| vec![r]);
		let mut dropped = Vec::new();
		let reply = ir::responses::parse_reply(resp, &mut dropped)?;
		if !dropped.is_empty() {
			let dropped = dropped
				.iter()
				.map(|dropped| (dropped.redacted_path(), dropped.reason()))
				.collect::<Vec<_>>();
			tracing::debug!(?dropped, "responses reply fields not sent to messages");
		}
		Ok(Box::new(super::super::ProviderResponse {
			response: ir::messages::render_reply(reply),
			usage: provider_usage,
			id,
			finish_reasons,
		}))
	}

	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
	) -> Body {
		#[derive(Default)]
		struct ToolBlock {
			index: Option<usize>,
			id: Option<String>,
			name: Option<String>,
			emitted_arguments: bool,
			open: bool,
		}

		#[derive(Default)]
		struct StreamState {
			sent_message_start: bool,
			sent_message_stop: bool,
			failed: bool,
			last_token_at: Option<Instant>,
			next_block_index: usize,
			response_id: Option<String>,
			model: Option<String>,
			text_blocks: HashMap<(u32, u32), usize>,
			open_text_blocks: HashSet<(u32, u32)>,
			emitted_refusals: HashSet<(u32, u32)>,
			tool_blocks: HashMap<u32, ToolBlock>,
			pending_usage: Option<responses::ResponseUsage>,
			pending_stop_reason: Option<messages::StopReason>,
			saw_tool_call: bool,
			saw_refusal: bool,
		}

		fn push_event(
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			event: messages::MessagesStreamEvent,
		) {
			events.push(event.into_sse_tuple());
		}

		fn ensure_message_start(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			log: &StreamingUsageGuard,
		) {
			if state.sent_message_start {
				return;
			}
			state.sent_message_start = true;
			let id = state
				.response_id
				.clone()
				.unwrap_or_else(|| format!("msg_{:016x}", rand::rng().random::<u64>()));
			let model = state.model.clone().unwrap_or_default();
			push_event(
				events,
				messages::MessagesStreamEvent::MessageStart {
					message: messages::MessagesResponse::start(id, model.clone()),
				},
			);
			if !model.is_empty() {
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&model));
				});
			}
		}

		fn record_token(state: &mut StreamState, log: &StreamingUsageGuard) {
			let now = Instant::now();
			if let Some(prev) = state.last_token_at.replace(now) {
				log.update(|r| {
					r.response
						.inter_chunk_latencies
						.record(now.duration_since(prev))
				});
			} else {
				log.update(|r| r.response.first_token = Some(now));
			}
		}

		fn close_text_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			key: (u32, u32),
		) {
			if state.open_text_blocks.remove(&key)
				&& let Some(index) = state.text_blocks.get(&key)
			{
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStop { index: *index },
				);
			}
		}

		fn finish_refusal(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			log: &StreamingUsageGuard,
			completion: &mut Option<String>,
			key: (u32, u32),
			text: String,
		) {
			state.saw_refusal = true;
			state.pending_stop_reason = Some(messages::StopReason::Refusal);
			// Final events repeat the full refusal; only emit it when no delta was sent.
			if !text.is_empty() && state.emitted_refusals.insert(key) {
				ensure_message_start(state, events, log);
				let index = open_text_block(state, events, key);
				record_token(state, log);
				if let Some(c) = completion.as_mut() {
					c.push_str(&text);
				}
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockDelta {
						index,
						delta: messages::ContentBlockDelta::TextDelta { text },
					},
				);
			}
			close_text_block(state, events, key);
		}

		fn close_text_blocks_for_output(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			output_index: u32,
		) {
			let mut keys: Vec<_> = state
				.open_text_blocks
				.iter()
				.filter(|(out, _)| *out == output_index)
				.copied()
				.collect();
			keys.sort_by_key(|key| state.text_blocks.get(key).copied().unwrap_or_default());
			for key in keys {
				close_text_block(state, events, key);
			}
		}

		fn close_all_text_blocks(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) {
			let mut keys: Vec<_> = state.open_text_blocks.iter().copied().collect();
			keys.sort_by_key(|key| state.text_blocks.get(key).copied().unwrap_or_default());
			for key in keys {
				close_text_block(state, events, key);
			}
		}

		fn close_tool_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			output_index: u32,
		) {
			let Some(block) = state.tool_blocks.get_mut(&output_index) else {
				return;
			};
			if block.open {
				block.open = false;
				if let Some(index) = block.index {
					push_event(
						events,
						messages::MessagesStreamEvent::ContentBlockStop { index },
					);
				}
			}
		}

		fn close_all_tool_blocks(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) {
			let mut blocks: Vec<_> = state
				.tool_blocks
				.iter()
				.filter_map(|(output_index, block)| block.index.map(|idx| (*output_index, idx)))
				.collect();
			blocks.sort_by_key(|(_, index)| *index);
			for (output_index, _) in blocks {
				close_tool_block(state, events, output_index);
			}
		}

		fn open_text_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			key: (u32, u32),
		) -> usize {
			close_all_tool_blocks(state, events);
			let index = *state.text_blocks.entry(key).or_insert_with(|| {
				let index = state.next_block_index;
				state.next_block_index += 1;
				index
			});
			if state.open_text_blocks.insert(key) {
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStart {
						index,
						content_block: messages::ContentBlock::text(String::new()),
					},
				);
			}
			index
		}

		fn open_tool_block(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			output_index: u32,
			id: Option<String>,
			name: Option<String>,
		) -> usize {
			close_all_text_blocks(state, events);
			state.saw_tool_call = true;
			let mut start = None;
			let index = {
				let block = state.tool_blocks.entry(output_index).or_default();
				if block.index.is_none() {
					block.index = Some(state.next_block_index);
					state.next_block_index += 1;
				}
				if block.id.is_none() {
					block.id = id;
				}
				if block.name.as_ref().is_none_or(|name| name.is_empty()) {
					block.name = name;
				}
				let index = block.index.unwrap_or_default();
				if !block.open {
					block.open = true;
					start = Some((
						index,
						block
							.id
							.clone()
							.unwrap_or_else(|| format!("call_{output_index}")),
						block.name.clone().unwrap_or_default(),
					));
				}
				index
			};
			if let Some((index, id, name)) = start {
				push_event(
					events,
					messages::MessagesStreamEvent::ContentBlockStart {
						index,
						content_block: messages::ContentBlock::ToolUse {
							id,
							name,
							input: Value::Object(Map::new()),
							cache_control: None,
						},
					},
				);
			}
			index
		}

		fn update_usage_log(
			log: &StreamingUsageGuard,
			usage: &responses::ResponseUsage,
			service_tier: Option<String>,
		) {
			log.update(|r| {
				r.response.input_tokens = Some(usage.input_tokens as u64);
				r.response.output_tokens = Some(usage.output_tokens as u64);
				r.response.total_tokens = Some(usage.total_tokens as u64);
				r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
				r.response.cache_creation_input_tokens = usage
					.input_tokens_details
					.cache_write_tokens
					.map(|tokens| tokens as u64);
				r.response.cache_creation_12h_input_tokens = usage
					.input_tokens_details
					.cache_write_tokens_details
					.as_ref()
					.and_then(|d| d.ttl_12h_tokens)
					.map(|t| t as u64);
				r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
				if let Some(service_tier) = &service_tier {
					r.response.service_tier = Some(strng::new(service_tier));
				}
			});
		}

		fn record_response_metadata(
			state: &mut StreamState,
			log: &StreamingUsageGuard,
			response: &responses::Response,
		) {
			state.response_id = Some(response.id.clone());
			state.model = Some(response.model.clone());
			log.update(|r| {
				r.response.provider_model = Some(strng::new(&response.model));
				r.response.response_id = Some(strng::new(&response.id));
				r.response.service_tier = response
					.service_tier
					.as_ref()
					.and_then(types::serialize_str);
			});
			if let Some(usage) = &response.usage {
				state.pending_usage = Some(usage.clone());
				update_usage_log(
					log,
					usage,
					response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str)
						.map(|tier| tier.to_string()),
				);
			}
		}

		fn flush_message_end(
			state: &mut StreamState,
			events: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
			log: &StreamingUsageGuard,
			completion: &mut Option<String>,
			tool_calls: &mut Option<BTreeMap<u32, OutputMessagePart>>,
			force: bool,
		) {
			if state.sent_message_stop {
				return;
			}
			if !state.sent_message_start && !force {
				return;
			}
			ensure_message_start(state, events, log);

			let stop_reason = state.pending_stop_reason.take().unwrap_or({
				if state.saw_tool_call {
					messages::StopReason::ToolUse
				} else {
					messages::StopReason::EndTurn
				}
			});
			let usage = state.pending_usage.take();
			let message_usage = translate_delta_usage(usage.as_ref());

			close_all_text_blocks(state, events);
			close_all_tool_blocks(state, events);

			push_event(
				events,
				messages::MessagesStreamEvent::MessageDelta {
					delta: messages::MessageDelta {
						stop_reason: Some(stop_reason),
						stop_sequence: None,
					},
					usage: message_usage,
				},
			);
			push_event(events, messages::MessagesStreamEvent::MessageStop);
			state.sent_message_stop = true;

			let finish_reason = types::serialize_str(&stop_reason);
			log.update(|r| {
				if let Some(c) = completion.take() {
					r.response.completion = Some(vec![c]);
				}
				r.response.output_messages = super::take_output_messages(tool_calls, finish_reason.clone());
			});
		}

		let mut state = StreamState::default();
		let mut completion = log_content.completion.then(String::new);
		let mut tool_calls = log_content.tool_calls.then(BTreeMap::new);

		parse::sse::json_transform_multi::<
			responses::ResponseStreamEvent,
			messages::MessagesStreamEvent,
			_,
		>(b, buffer_limit, move |evt| {
			let mut events: Vec<(&'static str, messages::MessagesStreamEvent)> = Vec::new();
			if state.failed {
				return events;
			}
			match evt {
				SseJsonEvent::Eof | SseJsonEvent::Error => return events,
				SseJsonEvent::Done => {
					flush_message_end(
						&mut state,
						&mut events,
						&log,
						&mut completion,
						&mut tool_calls,
						true,
					);
				},
				SseJsonEvent::Data(Err(e)) => {
					tracing::warn!(
						"Failed to parse OpenAI Responses stream during messages translation: {}",
						e
					);
				},
				SseJsonEvent::Data(Ok(event)) => match event {
					responses::ResponseStreamEvent::ResponseCreated(created) => {
						record_response_metadata(&mut state, &log, &created.response);
						ensure_message_start(&mut state, &mut events, &log);
					},
					responses::ResponseStreamEvent::ResponseOutputItemAdded(added) => {
						ensure_message_start(&mut state, &mut events, &log);
						if let responses::OutputItem::FunctionCall(call) = added.item {
							open_tool_block(
								&mut state,
								&mut events,
								added.output_index,
								Some(call.call_id),
								Some(call.name),
							);
							record_token(&mut state, &log);
						}
					},
					responses::ResponseStreamEvent::ResponseContentPartAdded(added) => {
						ensure_message_start(&mut state, &mut events, &log);
						match added.part {
							responses::OutputContent::OutputText(_) => {
								open_text_block(
									&mut state,
									&mut events,
									(added.output_index, added.content_index),
								);
							},
							responses::OutputContent::Refusal(_) => {
								state.pending_stop_reason = Some(messages::StopReason::Refusal);
								open_text_block(
									&mut state,
									&mut events,
									(added.output_index, added.content_index),
								);
							},
							responses::OutputContent::ReasoningText(_) => {
								tracing::warn!(
									"Skipping unsupported Responses reasoning stream content during messages translation"
								);
							},
						}
					},
					responses::ResponseStreamEvent::ResponseOutputTextDelta(delta) => {
						ensure_message_start(&mut state, &mut events, &log);
						let index = open_text_block(
							&mut state,
							&mut events,
							(delta.output_index, delta.content_index),
						);
						record_token(&mut state, &log);
						if let Some(c) = completion.as_mut() {
							c.push_str(&delta.delta);
						}
						push_event(
							&mut events,
							messages::MessagesStreamEvent::ContentBlockDelta {
								index,
								delta: messages::ContentBlockDelta::TextDelta { text: delta.delta },
							},
						);
					},
					responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta) => {
						ensure_message_start(&mut state, &mut events, &log);
						let index = open_tool_block(
							&mut state,
							&mut events,
							delta.output_index,
							Some(delta.item_id.clone()),
							None,
						);
						state
							.tool_blocks
							.entry(delta.output_index)
							.or_default()
							.emitted_arguments = true;
						record_token(&mut state, &log);
						push_event(
							&mut events,
							messages::MessagesStreamEvent::ContentBlockDelta {
								index,
								delta: messages::ContentBlockDelta::InputJsonDelta {
									partial_json: delta.delta,
								},
							},
						);
					},
					responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(done) => {
						ensure_message_start(&mut state, &mut events, &log);
						let index = open_tool_block(
							&mut state,
							&mut events,
							done.output_index,
							Some(done.item_id.clone()),
							done.name.clone(),
						);
						let block = state.tool_blocks.entry(done.output_index).or_default();
						if !block.emitted_arguments && !done.arguments.is_empty() {
							block.emitted_arguments = true;
							record_token(&mut state, &log);
							push_event(
								&mut events,
								messages::MessagesStreamEvent::ContentBlockDelta {
									index,
									delta: messages::ContentBlockDelta::InputJsonDelta {
										partial_json: done.arguments,
									},
								},
							);
						}
					},
					responses::ResponseStreamEvent::ResponseRefusalDelta(delta) => {
						state.saw_refusal = true;
						state.pending_stop_reason = Some(messages::StopReason::Refusal);
						let key = (delta.output_index, delta.content_index);
						let index = open_text_block(&mut state, &mut events, key);
						if !delta.delta.is_empty() {
							state.emitted_refusals.insert(key);
							record_token(&mut state, &log);
							if let Some(c) = completion.as_mut() {
								c.push_str(&delta.delta);
							}
							push_event(
								&mut events,
								messages::MessagesStreamEvent::ContentBlockDelta {
									index,
									delta: messages::ContentBlockDelta::TextDelta { text: delta.delta },
								},
							);
						}
					},
					responses::ResponseStreamEvent::ResponseRefusalDone(done) => {
						finish_refusal(
							&mut state,
							&mut events,
							&log,
							&mut completion,
							(done.output_index, done.content_index),
							done.refusal,
						);
					},
					responses::ResponseStreamEvent::ResponseContentPartDone(done) => {
						let key = (done.output_index, done.content_index);
						if let responses::OutputContent::Refusal(refusal) = done.part {
							finish_refusal(
								&mut state,
								&mut events,
								&log,
								&mut completion,
								key,
								refusal.refusal,
							);
						} else {
							close_text_block(&mut state, &mut events, key);
						}
					},
					responses::ResponseStreamEvent::ResponseOutputItemDone(done) => match done.item {
						responses::OutputItem::FunctionCall(call) => {
							state.saw_tool_call = true;
							let index = open_tool_block(
								&mut state,
								&mut events,
								done.output_index,
								Some(call.call_id.clone()),
								Some(call.name.clone()),
							);
							let block = state.tool_blocks.entry(done.output_index).or_default();
							if !block.emitted_arguments && !call.arguments.is_empty() {
								block.emitted_arguments = true;
								push_event(
									&mut events,
									messages::MessagesStreamEvent::ContentBlockDelta {
										index,
										delta: messages::ContentBlockDelta::InputJsonDelta {
											partial_json: call.arguments.clone(),
										},
									},
								);
							}
							if let Some(tool_calls) = tool_calls.as_mut() {
								let arguments = serde_json::from_str(&call.arguments)
									.unwrap_or_else(|_| Value::Object(Map::new()));
								tool_calls.insert(
									done.output_index,
									OutputMessagePart::ToolCall {
										id: strng::new(&call.call_id),
										name: strng::new(&call.name),
										arguments,
									},
								);
							}
							close_tool_block(&mut state, &mut events, done.output_index);
						},
						responses::OutputItem::Message(_) => {
							close_text_blocks_for_output(&mut state, &mut events, done.output_index);
						},
						_ => {
							tracing::warn!(
								"Skipping unsupported Responses output item during messages stream translation"
							);
						},
					},
					responses::ResponseStreamEvent::ResponseCompleted(completed) => {
						record_response_metadata(&mut state, &log, &completed.response);
						super::record_provider_response(&log, &completed.response);
						state.pending_stop_reason = Some(if state.saw_tool_call {
							messages::StopReason::ToolUse
						} else if state.saw_refusal {
							messages::StopReason::Refusal
						} else {
							messages::StopReason::EndTurn
						});
						flush_message_end(
							&mut state,
							&mut events,
							&log,
							&mut completion,
							&mut tool_calls,
							true,
						);
					},
					responses::ResponseStreamEvent::ResponseIncomplete(incomplete) => {
						record_response_metadata(&mut state, &log, &incomplete.response);
						super::record_provider_response(&log, &incomplete.response);
						state.pending_stop_reason = incomplete
							.response
							.incomplete_details
							.as_ref()
							.and_then(|details| ir::responses::parse_incomplete_reason(&details.reason).ok())
							// Only a completed turn looks at the parts.
							.map(|reason| ir::messages::render_stop_reason(reason, &[]));
						flush_message_end(
							&mut state,
							&mut events,
							&log,
							&mut completion,
							&mut tool_calls,
							true,
						);
					},
					responses::ResponseStreamEvent::ResponseFailed(failed) => {
						record_response_metadata(&mut state, &log, &failed.response);
						super::record_provider_response(&log, &failed.response);
						tracing::warn!(
							"Responses stream failed during messages translation; emitting error event"
						);
						state.failed = true;
						push_event(
							&mut events,
							messages::MessagesStreamEvent::Error {
								error: messages::MessagesError {
									r#type: error_type(
										failed
											.response
											.error
											.as_ref()
											.and_then(|e| serde_json::to_value(&e.code).ok())
											.as_ref()
											.and_then(Value::as_str),
									)
									.to_string(),
									message: failed
										.response
										.error
										.map(|error| error.message)
										.unwrap_or_else(|| "responses stream failed".to_string()),
								},
							},
						);
					},
					responses::ResponseStreamEvent::ResponseError(error) => {
						tracing::warn!(
							"Responses stream error during messages translation: {}",
							error.message
						);
						state.failed = true;
						push_event(
							&mut events,
							messages::MessagesStreamEvent::Error {
								error: messages::MessagesError {
									r#type: error_type(error.code.as_deref()).to_string(),
									message: error.message,
								},
							},
						);
					},
					// Intermediate progress event; no block state change needed.
					responses::ResponseStreamEvent::ResponseInProgress(_) => {},
					// Text finalization is handled by ResponseContentPartDone.
					responses::ResponseStreamEvent::ResponseOutputTextDone(_) => {},
					responses::ResponseStreamEvent::ResponseReasoningTextDelta(_)
					| responses::ResponseStreamEvent::ResponseReasoningTextDone(_) => {},
				},
			}
			if state.failed {
				// Retain partial output for logging without claiming a successful finish.
				log.update(|r| {
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
					r.response.output_messages = super::take_output_messages(&mut tool_calls, None);
				});
			}
			events
		})
	}

	/// Map an OpenAI Responses error code to an Anthropic error type.
	fn error_type(code: Option<&str>) -> &'static str {
		match code {
			Some("rate_limit_exceeded") => "rate_limit_error",
			Some("server_is_overloaded") => "overloaded_error",
			Some("insufficient_quota") => "billing_error",
			Some(code)
				if code == "context_length_exceeded"
					|| code.starts_with("invalid_")
					|| code.starts_with("image_") =>
			{
				"invalid_request_error"
			},
			_ => "api_error",
		}
	}

	pub fn translate_error(bytes: &Bytes, status: ::http::StatusCode) -> Result<Bytes, AIError> {
		crate::conversion::completions::from_messages::translate_error(bytes, status)
	}

	fn translate_delta_usage(
		usage: Option<&responses::ResponseUsage>,
	) -> messages::MessageDeltaUsage {
		let usage =
			usage.map_or_default(|usage| ir::messages::render_usage(ir::responses::parse_usage(usage)));
		messages::MessageDeltaUsage {
			input_tokens: Some(usage.input_tokens),
			output_tokens: Some(usage.output_tokens),
			cache_creation_input_tokens: usage.cache_creation_input_tokens,
			cache_read_input_tokens: usage.cache_read_input_tokens,
			..Default::default()
		}
	}
}
