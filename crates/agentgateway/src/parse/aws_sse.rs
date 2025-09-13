use bytes::Bytes;
use serde::Serialize;
use tokio_sse_codec::{Event, Frame, SseEncoder};

use super::transform::parser as transform_parser;
use crate::*;

pub fn transform<O: Serialize>(
	b: http::Body,
	mut f: impl FnMut(aws_event_stream_parser::Message) -> Option<O> + Send + 'static,
) -> http::Body {
	let decoder = aws_event_stream_parser::EventStreamCodec;
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
