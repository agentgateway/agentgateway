use aws_smithy_eventstream::frame::{DecodedFrame, MessageFrameDecoder};
pub use aws_smithy_types::event_stream::Message;
use bytes::{Bytes, BytesMut};
use serde::Serialize;
use tokio_sse_codec::{Event, Frame, SseEncoder};
use tokio_util::codec::Decoder;

use super::transform::parser as transform_parser;
use crate::*;

/// Error type for EventStream decoding.
///
/// Wraps AWS Smithy's eventstream errors and satisfies the `tokio_util::codec::Decoder`
/// requirement of implementing `From<io::Error>`.
#[derive(Debug)]
pub enum EventStreamError {
	/// AWS EventStream protocol error (CRC mismatch, invalid headers, etc.)
	Protocol(aws_smithy_eventstream::error::Error),
	/// I/O error during decoding
	Io(std::io::Error),
}

impl std::fmt::Display for EventStreamError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Protocol(e) => write!(f, "{e}"),
			Self::Io(e) => write!(f, "{e}"),
		}
	}
}

impl std::error::Error for EventStreamError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Protocol(e) => Some(e),
			Self::Io(e) => Some(e),
		}
	}
}

impl From<std::io::Error> for EventStreamError {
	fn from(err: std::io::Error) -> Self {
		Self::Io(err)
	}
}

impl From<aws_smithy_eventstream::error::Error> for EventStreamError {
	fn from(err: aws_smithy_eventstream::error::Error) -> Self {
		Self::Protocol(err)
	}
}

/// A `tokio_util::codec::Decoder` wrapper around AWS Smithy's `MessageFrameDecoder`.
///
/// This provides a streaming decoder for AWS EventStream binary protocol messages,
/// compatible with the transform pipeline infrastructure.
#[derive(Default)]
pub struct EventStreamCodec {
	inner: MessageFrameDecoder,
}

impl EventStreamCodec {
	pub fn new() -> Self {
		Self::default()
	}
}

impl Decoder for EventStreamCodec {
	type Item = Message;
	type Error = EventStreamError;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		match self.inner.decode_frame(src)? {
			DecodedFrame::Complete(message) => Ok(Some(message)),
			DecodedFrame::Incomplete => Ok(None),
		}
	}
}

pub fn transform<O: Serialize>(
	b: http::Body,
	mut f: impl FnMut(Message) -> Option<O> + Send + 'static,
) -> http::Body {
	let decoder = EventStreamCodec::new();
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
	let decoder = EventStreamCodec::new();
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
