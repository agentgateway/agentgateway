//! Finish metadata is independent of content capture and token usage. Keep a slot for every
//! observed generation until the stream ends, including when the body is dropped early.
use std::collections::BTreeMap;

use agent_core::strng::{self, Strng};
use serde_json::Value;

use crate::{StreamingUsageGuard, types};

pub(crate) fn error() -> Strng {
	strng::literal!("error")
}

pub(crate) fn buffered(reasons: impl IntoIterator<Item = Option<Strng>>) -> Option<Vec<Strng>> {
	let reasons: Vec<_> = reasons
		.into_iter()
		.map(|r| r.unwrap_or_else(error))
		.collect();
	(!reasons.is_empty()).then_some(reasons)
}

pub(crate) fn response_status(status: Option<&str>) -> Option<Strng> {
	match status {
		Some("completed" | "incomplete") => status.map(strng::new),
		Some("failed" | "cancelled") => Some(error()),
		_ => None,
	}
}

#[derive(Default)]
pub(crate) struct FinishReasons(BTreeMap<u64, Option<Strng>>);

impl FinishReasons {
	fn observe(&mut self, index: u64, reason: Option<Strng>) {
		// The first terminal reason wins. A repeated terminal event or a later usage-only
		// chunk must not erase it or create another generation.
		let entry = self.0.entry(index).or_default();
		if entry.is_none() {
			*entry = reason;
		}
	}

	fn complete(&self) -> Option<Vec<Strng>> {
		if self.0.is_empty() {
			return None;
		}
		self.0.values().cloned().collect()
	}

	fn finalized(&self) -> Option<Vec<Strng>> {
		buffered(self.0.values().cloned())
	}
}

impl StreamingUsageGuard {
	pub(crate) fn record_finish_reason(&self, index: u64, reason: Option<Strng>) {
		let mut reasons = self.finish_reasons.borrow_mut();
		reasons.observe(index, reason);
		if let Some(complete) = reasons.complete() {
			self.update(|info| info.response.finish_reasons = Some(complete.clone()));
		}
	}

	pub(crate) fn finalize_finish_reasons(&self) {
		if let Some(reasons) = self.finish_reasons.borrow().finalized() {
			self.update(|info| info.response.finish_reasons = Some(reasons.clone()));
		}
	}

	pub(crate) fn observe_finish_reasons(&self, value: &Value) {
		observe_json(value, true, &mut |i, r| self.record_finish_reason(i, r));
	}

	pub(crate) fn observe_messages(
		&self,
		event: &types::messages::typed::MessagesStreamEvent,
		map: impl Fn(&types::messages::typed::StopReason) -> Option<Strng>,
	) {
		use types::messages::typed::MessagesStreamEvent as E;
		match event {
			E::MessageStart { message } => {
				self.record_finish_reason(0, message.stop_reason.as_ref().and_then(map))
			},
			E::MessageDelta { delta, .. } => {
				self.record_finish_reason(0, delta.stop_reason.as_ref().and_then(map))
			},
			E::ContentBlockStart { .. }
			| E::ContentBlockDelta { .. }
			| E::ContentBlockStop { .. }
			| E::MessageStop => self.record_finish_reason(0, None),
			E::Ping | E::Error { .. } => {},
		}
	}

	pub(crate) fn observe_bedrock(
		&self,
		event: &types::bedrock::ConverseStreamOutput,
		map: impl Fn(&types::bedrock::StopReason) -> Option<Strng>,
	) {
		use types::bedrock::ConverseStreamOutput as E;
		match event {
			E::MessageStop(stop) => self.record_finish_reason(0, map(&stop.stop_reason)),
			E::Metadata(_) => {},
			_ => self.record_finish_reason(0, None),
		}
	}

	pub(crate) fn observe_responses(&self, event: &types::responses::typed::ResponseStreamEvent) {
		use types::responses::typed::ResponseStreamEvent as E;
		let reason = match event {
			E::ResponseCompleted(e) => {
				types::serialize_str(&e.response.status).and_then(|s| response_status(Some(&s)))
			},
			E::ResponseIncomplete(e) => {
				types::serialize_str(&e.response.status).and_then(|s| response_status(Some(&s)))
			},
			E::ResponseFailed(_) => Some(error()),
			E::ResponseError(_) => return,
			_ => None,
		};
		self.record_finish_reason(0, reason);
	}
}

pub(crate) fn detect_buffered(value: &Value) -> Option<Vec<Strng>> {
	let mut reasons = Vec::new();
	observe_json(value, false, &mut |_, r| reasons.push(r));
	buffered(reasons)
}

fn observe_json(value: &Value, streaming: bool, observe: &mut impl FnMut(u64, Option<Strng>)) {
	// Cloud Code Gemini wraps generateContent responses in `response`.
	if let Some(response) = value.get("response") {
		observe_json(response, streaming, observe);
	}
	for (field, reason_field) in [("choices", "finish_reason"), ("candidates", "finishReason")] {
		if let Some(choices) = value.get(field).and_then(Value::as_array) {
			for (position, choice) in choices.iter().enumerate() {
				let index = if streaming {
					choice.get("index").and_then(Value::as_u64)
				} else {
					None
				};
				observe(
					index.unwrap_or(position as u64),
					choice
						.get(reason_field)
						.and_then(Value::as_str)
						.map(strng::new),
				);
			}
			return;
		}
	}
	let event = value
		.get("type")
		.and_then(Value::as_str)
		.unwrap_or_default();
	if event.starts_with("response.")
		|| value.get("object").and_then(Value::as_str) == Some("response")
		|| (value.get("output").is_some_and(Value::is_array) && value.get("status").is_some())
	{
		let status = value
			.get("response")
			.unwrap_or(value)
			.get("status")
			.and_then(Value::as_str);
		// A buffered background response does not promise a terminal generation yet.
		// Streams still track it so an interrupted generation is finalized as an error.
		if !streaming && matches!(status, Some("queued" | "in_progress")) {
			return;
		}
		observe(
			0,
			response_status(status).or_else(|| response_status(event.strip_prefix("response."))),
		);
		return;
	}
	let message = if event == "message_start" {
		&value["message"]
	} else if event == "message_delta" {
		&value["delta"]
	} else {
		value
	};
	if message.get("stop_reason").is_some()
		|| event == "message"
		|| matches!(
			event,
			"message_start"
				| "message_delta"
				| "message_stop"
				| "content_block_start"
				| "content_block_delta"
				| "content_block_stop"
		) {
		observe(
			0,
			message
				.get("stop_reason")
				.and_then(Value::as_str)
				.map(strng::new),
		);
	} else if value.pointer("/messageStop/stopReason").is_some()
		|| value.get("stopReason").is_some()
		|| value.pointer("/output/message").is_some()
		|| value.get("messageStart").is_some()
		|| value.get("contentBlockDelta").is_some()
	{
		observe(
			0,
			value
				.get("stopReason")
				.or_else(|| value.pointer("/messageStop/stopReason"))
				.and_then(Value::as_str)
				.map(strng::new),
		);
	}
}

#[cfg(test)]
#[path = "finish_reasons_tests.rs"]
mod tests;
