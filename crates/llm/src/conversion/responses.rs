use std::time::Instant;

use agent_core::strng;
use serde::Deserialize;

use crate::http::Body;
use crate::llm::types::detect;
use crate::llm::{AmendOnDrop, types};
use crate::parse;

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
	mut log: AmendOnDrop,
	include_completion_in_log: bool,
) -> Body {
	let mut saw_token = false;
	let mut completion = include_completion_in_log.then(String::new);
	parse::sse::json_passthrough::<StreamResponse>(b, buffer_limit, move |event| {
		let Some(Ok(event)) = event else {
			// Stream ended ([DONE]): flush completion if not already set via ResponseCompleted
			if event.is_none() {
				log.non_atomic_mutate(|r| {
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
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
				log.non_atomic_mutate(|r| {
					r.response.provider_model = Some(strng::new(&created.response.model));
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
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
				});
			},
			types::responses::typed::ResponseStreamEvent::ResponseOutputTextDelta(ref delta) => {
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}
				if let Some(c) = completion.as_mut() {
					c.push_str(&delta.delta);
				}
			},
			types::responses::typed::ResponseStreamEvent::ResponseCompleted(completed) => {
				log.non_atomic_mutate(|r| {
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
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
				});
			},
			_ => {},
		}
	})
}
