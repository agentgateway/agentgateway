#[cfg(test)]
#[path = "copilot_tests.rs"]
mod tests;

/// Keep Copilot output item IDs anchored to the first output_item.added event
/// for each output index in this response stream.
pub fn normalize_responses_stream(body: agent_http::Body, buffer_limit: usize) -> agent_http::Body {
	use std::collections::HashMap;

	use bytes::Bytes;
	use serde_json::Value;
	use tokio_sse_codec::{Frame, SseEncoder};

	use crate::parse::{sse, transform};

	let mut item_ids = HashMap::new();
	transform::parser(
		body,
		sse::decoder(buffer_limit),
		SseEncoder::new(),
		move |event| {
			let transform::TransformEvent::Item(mut frame) = event else {
				return None;
			};
			if let Frame::Event(event) = &mut frame
				&& let Ok(mut value) = serde_json::from_slice::<Value>(&event.data)
				&& normalize_item_ids(&mut value, &mut item_ids)
				&& let Ok(bytes) = serde_json::to_vec(&value)
			{
				event.data = Bytes::from(bytes);
			}
			Some(frame)
		},
	)
}

fn normalize_item_ids(
	event: &mut serde_json::Value,
	item_ids: &mut std::collections::HashMap<u64, String>,
) -> bool {
	use serde_json::Value;

	let Some(kind) = event.get("type").and_then(Value::as_str) else {
		return false;
	};
	if matches!(
		kind,
		"response.completed" | "response.failed" | "response.incomplete"
	) {
		let Some(output) = event
			.get_mut("response")
			.and_then(|response| response.get_mut("output"))
			.and_then(Value::as_array_mut)
		else {
			return false;
		};
		let mut changed = false;
		for (index, item) in output.iter_mut().enumerate() {
			if let Some(id) = item_ids.get(&(index as u64)) {
				changed |= replace_id(item.get_mut("id"), id);
			}
		}
		return changed;
	}
	if !kind.starts_with("response.") {
		return false;
	}
	let Some(index) = event.get("output_index").and_then(Value::as_u64) else {
		return false;
	};
	let item_event = matches!(
		kind,
		"response.output_item.added" | "response.output_item.done"
	);
	if kind == "response.output_item.added"
		&& let Some(id) = event
			.get("item")
			.and_then(|item| item.get("id"))
			.and_then(Value::as_str)
		&& !id.is_empty()
	{
		item_ids.entry(index).or_insert_with(|| id.to_owned());
	}
	let Some(id) = item_ids.get(&index) else {
		return false;
	};
	if item_event {
		replace_id(
			event.get_mut("item").and_then(|item| item.get_mut("id")),
			id,
		)
	} else {
		replace_id(event.get_mut("item_id"), id)
	}
}

fn replace_id(value: Option<&mut serde_json::Value>, id: &str) -> bool {
	let Some(serde_json::Value::String(value)) = value else {
		return false;
	};
	if value == id {
		return false;
	}
	id.clone_into(value);
	true
}

pub mod from_embeddings {
	use serde_json::Value;

	use crate::types::ResponseType;
	use crate::{AIError, logged_response_parsing, types};

	pub fn translate(req: &types::embeddings::Request) -> Result<Vec<u8>, AIError> {
		if let Value::String(s) = &req.input {
			let mut normalized = req.clone();
			normalized.input = Value::Array(vec![Value::String(s.clone())]);
			serde_json::to_vec(&normalized).map_err(AIError::RequestMarshal)
		} else {
			serde_json::to_vec(req).map_err(AIError::RequestMarshal)
		}
	}

	pub fn translate_response(bytes: &[u8], model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		let mut resp: types::embeddings::Response =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		if resp.model.is_empty() {
			resp.model = model.to_string();
		}
		Ok(Box::new(resp))
	}
}
