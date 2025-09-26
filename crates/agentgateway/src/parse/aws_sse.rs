use aws_event_stream_parser::{EventStreamCodec, Message};
use bytes::Bytes;
use serde::Serialize;
use tokio_sse_codec::{Event, Frame, SseEncoder};

use super::transform::parser as transform_parser;
use crate::*;

pub fn transform<O: Serialize>(
	b: http::Body,
	mut f: impl FnMut(Message) -> Option<O> + Send + 'static,
) -> http::Body {
	let decoder = EventStreamCodec;
	let encoder = SseEncoder::new();

	transform_parser(b, decoder, encoder, move |o| {
		let transformed = f(o)?;
		let json_bytes = serde_json::to_vec(&transformed).ok()?;
		Some(Frame::Event(Event::<Bytes> {
			data: Bytes::from(json_bytes),
			name: std::borrow::Cow::Borrowed(""),
			id: None,
		}))
	})
}

/// Transform AWS EventStream to SSE, allowing multiple output events per input event
pub fn transform_multi<O: Serialize>(
	b: http::Body,
	mut f: impl FnMut(Message) -> Vec<O> + Send + 'static,
) -> http::Body {
	use std::collections::VecDeque;
	let decoder = EventStreamCodec;
	let encoder = SseEncoder::new();
	let mut pending: VecDeque<Option<Frame<Bytes>>> = VecDeque::new();

	transform_parser(b, decoder, encoder, move |o| {
		// Process current event to 0..N outputs first
		for event in f(o) {
			if let Ok(json_bytes) = serde_json::to_vec(&event) {
				pending.push_back(Some(Frame::Event(Event::<Bytes> {
					data: Bytes::from(json_bytes),
					name: std::borrow::Cow::Borrowed(""),
					id: None,
				})));
			}
		}
		// Yield exactly one frame (if any)
		pending.pop_front()
	})
}
