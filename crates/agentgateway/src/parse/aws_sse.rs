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

pub fn transform_multi<O: Serialize>(
	b: http::Body,
	mut f: impl FnMut(Message) -> Vec<(&'static str, O)> + Send + 'static,
) -> http::Body {
	let decoder = EventStreamCodec;
	let encoder = SseEncoder::new();

	transform_parser(b, decoder, encoder, move |msg| {
		f(msg)
			.into_iter()
			.filter_map(|(event_name, event)| {
				serde_json::to_vec(&event).ok().map(|json_bytes| {
					Frame::Event(Event::<Bytes> {
						data: Bytes::from(json_bytes),
						name: std::borrow::Cow::Borrowed(event_name),
						id: None,
					})
				})
			})
			.collect::<Vec<_>>()
	})
}
