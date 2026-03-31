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
	/// EventStream frame exceeded the configured buffer limit
	FrameTooLarge { actual: usize, limit: usize },
}

impl std::fmt::Display for EventStreamError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Protocol(e) => write!(f, "{e}"),
			Self::Io(e) => write!(f, "{e}"),
			Self::FrameTooLarge { actual, limit } => {
				write!(
					f,
					"eventstream frame size {actual} exceeds buffer limit {limit}"
				)
			},
		}
	}
}

impl std::error::Error for EventStreamError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Protocol(e) => Some(e),
			Self::Io(e) => Some(e),
			Self::FrameTooLarge { .. } => None,
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
	max_frame_size: Option<usize>,
}

impl EventStreamCodec {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn with_max_size(max_frame_size: usize) -> Self {
		Self {
			inner: MessageFrameDecoder::new(),
			max_frame_size: Some(max_frame_size),
		}
	}

	fn validate_frame_size(&self, src: &BytesMut) -> Result<(), EventStreamError> {
		let Some(limit) = self.max_frame_size else {
			return Ok(());
		};

		// AWS EventStream prelude starts with a big-endian u32 total frame length.
		if src.len() >= std::mem::size_of::<u32>() {
			let actual =
				u32::from_be_bytes(src[..4].try_into().expect("slice length already checked")) as usize;
			if actual > limit {
				return Err(EventStreamError::FrameTooLarge { actual, limit });
			}
		}

		Ok(())
	}
}

impl Decoder for EventStreamCodec {
	type Item = Message;
	type Error = EventStreamError;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		self.validate_frame_size(src)?;
		match self.inner.decode_frame(src)? {
			DecodedFrame::Complete(message) => Ok(Some(message)),
			DecodedFrame::Incomplete => Ok(None),
		}
	}
}

pub fn transform<O: Serialize>(
	b: http::Body,
	buffer_limit: usize,
	mut f: impl FnMut(Message) -> Option<O> + Send + 'static,
) -> http::Body {
	let decoder = EventStreamCodec::with_max_size(buffer_limit);
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
	buffer_limit: usize,
	mut f: impl FnMut(Message) -> Vec<(&'static str, O)> + Send + 'static,
) -> http::Body {
	let decoder = EventStreamCodec::with_max_size(buffer_limit);
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

#[cfg(test)]
mod tests {
	use aws_smithy_eventstream::frame::write_message_to;
	use tokio_util::codec::Decoder;

	use super::*;

	#[test]
	fn eventstream_codec_rejects_oversized_frames() {
		let mut encoded = BytesMut::new();
		let message = Message::new(Bytes::from(vec![0u8; 32]));
		write_message_to(&message, &mut encoded).expect("message should encode");

		let mut codec = EventStreamCodec::with_max_size(16);
		let err = codec
			.decode(&mut encoded)
			.expect_err("oversized frame should fail before decoding");

		assert!(matches!(
			err,
			EventStreamError::FrameTooLarge {
				actual,
				limit: 16
			} if actual > 16
		));
	}
}
