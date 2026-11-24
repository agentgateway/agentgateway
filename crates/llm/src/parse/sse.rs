use agent_http::{Body, RawBody};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_util::codec::{BytesCodec, Decoder};

use super::passthrough::parser as passthrough_parser;
use super::sse_decoder::{SseDecoder, SseError, SseFrame};
use super::transform::{TransformEvent, parser as transform_parser};

/// Append an OpenAI `[DONE]` event after a body closes successfully.
pub fn append_done_on_success(body: Body) -> Body {
	body.transform_stream(|body| {
		let done = crate::parse::encode_sse_event("", Bytes::from_static(b"[DONE]"));
		let stream = stream::unfold(
			(Some(body.into_data_stream().boxed()), Some(done)),
			|(stream, done): (
				Option<BoxStream<'static, Result<Bytes, axum_core::Error>>>,
				Option<Bytes>,
			)| async move {
				let mut stream = stream?;
				match stream.next().await {
					Some(Ok(chunk)) => Some((Ok(chunk), (Some(stream), done))),
					Some(Err(err)) => Some((Err(err), (None, None))),
					None => done.map(|done| (Ok(done), (None, None))),
				}
			},
		)
		.fuse();
		RawBody::from_stream(stream)
	})
}

pub fn json_passthrough<F: DeserializeOwned>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(Option<anyhow::Result<F>>) + Send + 'static,
) -> Body {
	passthrough_parser(b, SseDecoder::new(buffer_limit), move |frame| {
		f(passthrough_json(frame));
	})
}

pub fn permissive_json_passthrough<F: DeserializeOwned>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(Option<anyhow::Result<F>>) + Send + 'static,
) -> Body {
	crate::parse::passthrough::full_passthrough_parser(
		b,
		SseDecoder::new(buffer_limit),
		move |frame| f(passthrough_json(frame)),
	)
}

pub fn json_transform<I: DeserializeOwned, O: Serialize>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(anyhow::Result<I>) -> Option<O> + Send + 'static,
) -> Body {
	let decoder = WholeEvents(SseDecoder::new(buffer_limit));
	let encoder = BytesCodec::new();

	transform_parser(b, decoder, encoder, move |event| {
		let data = match event {
			TransformEvent::Item(data) => data,
			TransformEvent::Eof => return None,
			TransformEvent::Error => {
				let transformed = f(Err(anyhow::anyhow!("upstream SSE stream failed")))?;
				let json_bytes = serde_json::to_vec(&transformed).ok()?;
				return Some(crate::parse::encode_sse_event("", Bytes::from(json_bytes)));
			},
		};
		// Pass through [DONE] events unchanged
		if data.as_ref() == b"[DONE]" {
			return Some(crate::parse::encode_sse_event(
				"",
				Bytes::from_static(b"[DONE]"),
			));
		}
		let obj = serde_json::from_slice::<I>(&data);
		let transformed = f(obj.map_err(anyhow::Error::from))?;
		let json_bytes = serde_json::to_vec(&transformed).ok()?;
		Some(crate::parse::encode_sse_event("", Bytes::from(json_bytes)))
	})
}

/// Re-encode an SSE stream of JSON events as one streamed JSON array (`[{...},\r\n{...}]`), the
/// shape Gemini's `:streamGenerateContent` returns without `?alt=sse`.
pub fn to_json_array(b: Body, buffer_limit: usize) -> Body {
	let decoder = WholeEvents(SseDecoder::new(buffer_limit));
	let mut first = true;
	transform_parser(b, decoder, BytesCodec::new(), move |event| {
		let mut out = BytesMut::new();
		match event {
			TransformEvent::Item(data) => {
				out.extend_from_slice(if first { b"[" } else { b",\r\n" });
				out.extend_from_slice(&data);
				first = false;
			},
			TransformEvent::Eof => out.extend_from_slice(if first { b"[]" } else { b"]" }),
			TransformEvent::Error => return None,
		}
		Some(out.freeze())
	})
}

#[derive(Debug)]
pub enum SseJsonEvent<I> {
	Data(anyhow::Result<I>),
	Done,
	Eof,
	Error,
}

pub fn json_transform_multi<I: DeserializeOwned, O: Serialize, It>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(SseJsonEvent<I>) -> It + Send + 'static,
) -> Body
where
	It: IntoIterator<Item = (&'static str, O)>,
	It::IntoIter: Send,
{
	let decoder = WholeEvents(SseDecoder::new(buffer_limit));
	let encoder = BytesCodec::new();

	transform_parser(b, decoder, encoder, move |event| {
		let event = match event {
			TransformEvent::Eof => SseJsonEvent::Eof,
			TransformEvent::Error => SseJsonEvent::Error,
			TransformEvent::Item(data) => {
				if data.as_ref() == b"[DONE]" {
					SseJsonEvent::Done
				} else {
					let obj = serde_json::from_slice::<I>(&data);
					SseJsonEvent::Data(obj.map_err(anyhow::Error::from))
				}
			},
		};
		f(event)
			.into_iter()
			.filter_map(|(event_name, item)| {
				let json_bytes = serde_json::to_vec(&item).ok()?;
				Some(crate::parse::encode_sse_event(
					event_name,
					Bytes::from(json_bytes),
				))
			})
			.collect::<Vec<_>>()
	})
}

/// An oversized event becomes an observer error rather than failing the client's stream, whose
/// bytes are forwarded either way.
fn passthrough_json<F: DeserializeOwned>(frame: SseFrame) -> Option<anyhow::Result<F>> {
	let data = match frame {
		SseFrame::Event(data) => data,
		SseFrame::TooLarge => return Some(Err(SseError::TooLarge.into())),
	};
	if data.as_ref() == b"[DONE]" {
		return None;
	}
	Some(serde_json::from_slice(&data).map_err(anyhow::Error::from))
}

/// Fails on an oversized event, since a translated stream that skipped one would be wrong.
struct WholeEvents(SseDecoder);

impl Decoder for WholeEvents {
	type Item = Bytes;
	type Error = SseError;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Bytes>, SseError> {
		Self::whole(self.0.decode(src)?)
	}

	fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Bytes>, SseError> {
		Self::whole(self.0.decode_eof(src)?)
	}
}

impl WholeEvents {
	fn whole(frame: Option<SseFrame>) -> Result<Option<Bytes>, SseError> {
		match frame {
			None => Ok(None),
			Some(SseFrame::Event(data)) => Ok(Some(data)),
			Some(SseFrame::TooLarge) => Err(SseError::TooLarge),
		}
	}
}
