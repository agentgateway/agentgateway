//! Finish metadata is independent of content capture and token usage.
use agent_core::strng::{self, Strng};
use serde_json::Value;

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
