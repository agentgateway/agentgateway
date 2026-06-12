use aws_smithy_eventstream::frame::{DecodedFrame, MessageFrameDecoder};
pub use aws_smithy_types::event_stream::Message;
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use serde::Serialize;
use tokio_util::codec::{BytesCodec, Decoder};

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

/// Length in bytes of the AWS EventStream message prelude: total length, headers
/// length, and prelude CRC, each a big-endian `u32`.
const EVENTSTREAM_PRELUDE_LEN: usize = 3 * std::mem::size_of::<u32>();

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
			max_frame_size: Some(max_frame_size),
			..Self::default()
		}
	}

	/// Reads the declared total frame length from the prelude, or `None` if the prelude
	/// (the first [`EVENTSTREAM_PRELUDE_LEN`] bytes) has not fully arrived yet.
	fn frame_len(src: &BytesMut) -> Option<usize> {
		if src.len() < EVENTSTREAM_PRELUDE_LEN {
			return None;
		}
		// AWS EventStream prelude starts with a big-endian u32 total frame length.
		Some(u32::from_be_bytes(src[..4].try_into().expect("slice length already checked")) as usize)
	}

	fn validate_frame_size(&self, frame_len: usize) -> Result<(), EventStreamError> {
		let Some(limit) = self.max_frame_size else {
			return Ok(());
		};
		if frame_len > limit {
			return Err(EventStreamError::FrameTooLarge {
				actual: frame_len,
				limit,
			});
		}
		Ok(())
	}
}

impl Decoder for EventStreamCodec {
	type Item = Message;
	type Error = EventStreamError;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Only hand a frame to `decode_frame` once it is fully buffered. The decoder
		// drains the prelude from `src` as soon as it is available — even for an
		// incomplete frame — which would leave the buffer positioned mid-frame, so a
		// later length read (and the size guard) would interpret body bytes as a frame
		// length and spuriously trip `FrameTooLarge`. Reading the declared length up
		// front also lets us reject oversized frames before buffering their body.
		let Some(frame_len) = Self::frame_len(src) else {
			return Ok(None);
		};
		self.validate_frame_size(frame_len)?;
		if src.len() < frame_len {
			return Ok(None);
		}

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
	let encoder = BytesCodec::new();

	transform_parser(b, decoder, encoder, move |o| {
		let transformed = f(o)?;
		let json_bytes = serde_json::to_vec(&transformed).ok()?;
		Some(crate::parse::encode_sse_event("", Bytes::from(json_bytes)))
	})
}

pub fn transform_multi<O: Serialize>(
	b: http::Body,
	buffer_limit: usize,
	mut f: impl FnMut(Message) -> Vec<(&'static str, O)> + Send + 'static,
) -> http::Body {
	let decoder = EventStreamCodec::with_max_size(buffer_limit);
	let encoder = BytesCodec::new();

	transform_parser(b, decoder, encoder, move |msg| {
		f(msg)
			.into_iter()
			.filter_map(|(event_name, event)| {
				serde_json::to_vec(&event)
					.ok()
					.map(|json_bytes| crate::parse::encode_sse_event(event_name, Bytes::from(json_bytes)))
			})
			.collect::<Vec<_>>()
	})
}

pub fn inspect(
	b: http::Body,
	buffer_limit: usize,
	mut f: impl FnMut(Message) + Send + 'static,
) -> http::Body {
	let mut decoder = EventStreamCodec::with_max_size(buffer_limit);
	let mut decode_buffer = BytesMut::new();
	let mut inspect_failed = false;
	let stream = b.into_data_stream().map(move |chunk| {
		let bytes = chunk?;
		if !inspect_failed {
			if decode_buffer.len().saturating_add(bytes.len()) > buffer_limit {
				inspect_failed = true;
				decode_buffer.clear();
				return Ok::<Bytes, http::Error>(bytes);
			}
			decode_buffer.extend_from_slice(&bytes);
			loop {
				match decoder.decode(&mut decode_buffer) {
					Ok(Some(message)) => f(message),
					Ok(None) => break,
					Err(_) => {
						inspect_failed = true;
						decode_buffer.clear();
						break;
					},
				}
			}
		}
		Ok::<Bytes, http::Error>(bytes)
	});
	http::Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
	use aws_smithy_eventstream::frame::write_message_to;
	use http_body_util::BodyExt;
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

	#[test]
	fn eventstream_codec_handles_prelude_split_across_decodes() {
		// Regression: `MessageFrameDecoder` drains the 12-byte prelude as soon as it
		// is available, even for an incomplete frame. The old size guard then misread
		// the post-prelude bytes of the next `decode()` call as a frame length and
		// spuriously returned `FrameTooLarge`. This reproduces the production failure,
		// where a Bedrock event-stream frame's prelude arrives in one chunk and its
		// body in a later one. The 0xFF payload bytes make that misread look like a
		// ~4 GiB length — far over the limit — so this test fails on the old code.
		let payload = vec![0xFFu8; 32];
		let message = Message::new(Bytes::from(payload.clone()));
		let mut encoded = BytesMut::new();
		write_message_to(&message, &mut encoded).expect("message should encode");
		assert!(
			encoded.len() > EVENTSTREAM_PRELUDE_LEN,
			"frame must be larger than the prelude to split"
		);

		// Limit larger than the real frame (~48 bytes) but far smaller than the bogus
		// length the old code would compute from the 0xFF payload bytes.
		let mut codec = EventStreamCodec::with_max_size(1024);

		let mut buf = BytesMut::new();
		buf.extend_from_slice(&encoded[..EVENTSTREAM_PRELUDE_LEN]); // prelude only
		assert!(
			codec
				.decode(&mut buf)
				.expect("prelude-only decode must not error")
				.is_none(),
			"frame body is incomplete, decode should yield None"
		);

		buf.extend_from_slice(&encoded[EVENTSTREAM_PRELUDE_LEN..]); // remainder of frame
		let decoded = codec
			.decode(&mut buf)
			.expect("completing a prelude-split frame must not error")
			.expect("frame should now be complete");
		assert_eq!(decoded.payload().as_ref(), payload.as_slice());
	}

	#[tokio::test]
	async fn inspect_passes_through_invalid_eventstream_bytes() {
		let input = Bytes::from_static(b"not an aws eventstream");
		let body = http::Body::from(input.clone());
		let body = inspect(body, 1024, |_| {
			panic!("invalid stream should not emit messages")
		});

		let output = body.collect().await.unwrap().to_bytes();
		assert_eq!(output, input);
	}

	#[tokio::test]
	async fn transform_decodes_frame_split_across_body_chunks() {
		// End-to-end analogue of the production failure: a single event-stream frame
		// delivered in two body chunks, split immediately after the prelude. Before the
		// fix the transformed body aborted with `FrameTooLarge` ("error from user's Body
		// stream") because the post-prelude bytes were misread as a frame length.
		let payload = vec![0xFFu8; 64];
		let message = Message::new(Bytes::from(payload.clone()));
		let mut encoded = BytesMut::new();
		write_message_to(&message, &mut encoded).expect("message should encode");

		let body = http::Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
			vec![
				Ok::<_, std::convert::Infallible>(http_body::Frame::data(Bytes::copy_from_slice(
					&encoded[..EVENTSTREAM_PRELUDE_LEN],
				))),
				Ok::<_, std::convert::Infallible>(http_body::Frame::data(Bytes::copy_from_slice(
					&encoded[EVENTSTREAM_PRELUDE_LEN..],
				))),
			],
		)));

		let decoded = Arc::new(Mutex::new(vec![]));
		let decoded_clone = decoded.clone();
		// `buffer_limit` far above the real ~48-byte frame, but below the multi-gigabyte
		// length the old guard computed from the 0xFF payload bytes.
		let out = transform(body, 1024, move |msg: Message| {
			decoded_clone.lock().unwrap().push(msg.payload().to_vec());
			Some(msg.payload().len())
		});

		out
			.collect()
			.await
			.expect("transformed body must complete without error");
		assert_eq!(decoded.lock().unwrap().clone(), vec![payload]);
	}
}
