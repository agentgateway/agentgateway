use axum_core::body::Body;
use bytes::{Bytes, BytesMut};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_sse_codec::{Event, Frame, SseDecoder, SseEncoder};
use tokio_util::codec::{BytesCodec, Decoder};

use super::passthrough::parser as passthrough_parser;
use super::transform::parser as transform_parser;

pub fn json_passthrough<F: DeserializeOwned>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(Option<anyhow::Result<F>>) + Send + 'static,
) -> Body {
	let decoder = SseDecoder::<Bytes>::with_max_size(buffer_limit);

	passthrough_parser(b, decoder, move |o| {
		let Some(data) = unwrap_sse_data(o) else {
			return;
		};
		if data.as_ref() == b"[DONE]" {
			f(None);
			return;
		}
		let obj = serde_json::from_slice::<F>(&data);
		f(Some(obj.map_err(anyhow::Error::from)))
	})
}

pub(crate) fn remove_done(b: Body, buffer_limit: usize) -> Body {
	let decoder = SseDecoder::<Bytes>::with_max_size(buffer_limit);
	let encoder = SseEncoder::new();

	transform_parser(b, decoder, encoder, |frame| {
		if matches!(&frame, Frame::Event(event) if event.data.as_ref() == b"[DONE]") {
			None
		} else {
			Some(frame)
		}
	})
}

pub fn permissive_json_passthrough<F: DeserializeOwned>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(Option<anyhow::Result<F>>) + Send + 'static,
) -> Body {
	let decoder = SseDecoder::<Bytes>::with_max_size(buffer_limit);

	crate::parse::passthrough::full_passthrough_parser(b, decoder, move |o| {
		let Some(data) = unwrap_sse_data(o) else {
			return;
		};
		if data.as_ref() == b"[DONE]" {
			f(None);
			return;
		}
		let obj = serde_json::from_slice::<F>(&data);
		f(Some(obj.map_err(anyhow::Error::from)))
	})
}

pub fn json_transform<I: DeserializeOwned, O: Serialize>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(anyhow::Result<I>) -> Option<O> + Send + 'static,
) -> Body {
	let decoder = SseDecoder::<Bytes>::with_max_size(buffer_limit);
	let encoder = BytesCodec::new();

	transform_parser(b, decoder, encoder, move |o| {
		let data = unwrap_sse_data(o)?;
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

pub enum SseJsonEvent<I> {
	Data(anyhow::Result<I>),
	Done,
}

enum SseDecoderItem {
	Frame(Frame<Bytes>),
	Eof,
}

struct SseDecoderWithEof {
	inner: SseDecoder<Bytes>,
	eof_emitted: bool,
}

impl Decoder for SseDecoderWithEof {
	type Item = SseDecoderItem;
	type Error = <SseDecoder<Bytes> as Decoder>::Error;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		self
			.inner
			.decode(src)
			.map(|frame| frame.map(SseDecoderItem::Frame))
	}

	fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		if let Some(frame) = self.inner.decode_eof(src)? {
			return Ok(Some(SseDecoderItem::Frame(frame)));
		}
		if !self.eof_emitted {
			self.eof_emitted = true;
			return Ok(Some(SseDecoderItem::Eof));
		}
		Ok(None)
	}
}

pub(crate) enum StrictSseJsonEvent<I> {
	Data {
		event_name: Option<String>,
		data: anyhow::Result<I>,
	},
	Done,
	Eof,
	TransportError,
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
	let decoder = SseDecoder::<Bytes>::with_max_size(buffer_limit);
	let encoder = BytesCodec::new();

	transform_parser(b, decoder, encoder, move |o| {
		let data = unwrap_sse_data(o);
		if let Some(data) = &data
			&& data.as_ref() == b"[DONE]"
		{
			return f(SseJsonEvent::Done)
				.into_iter()
				.filter_map(|(event_name, item)| {
					let json_bytes = serde_json::to_vec(&item).ok()?;
					Some(crate::parse::encode_sse_event(
						event_name,
						Bytes::from(json_bytes),
					))
				})
				.collect();
		}
		let Some(data) = data else {
			return vec![];
		};

		let obj = serde_json::from_slice::<I>(&data);
		f(SseJsonEvent::Data(obj.map_err(anyhow::Error::from)))
			.into_iter()
			.filter_map(|(event_name, item)| {
				let json_bytes = serde_json::to_vec(&item).ok()?;
				Some(crate::parse::encode_sse_event(
					event_name,
					Bytes::from(json_bytes),
				))
			})
			.collect()
	})
}

pub(crate) fn json_transform_strict_with_eof<I: DeserializeOwned, O: Serialize>(
	b: Body,
	buffer_limit: usize,
	mut f: impl FnMut(StrictSseJsonEvent<I>) -> (Vec<(&'static str, O)>, bool) + Send + 'static,
) -> Body {
	let decoder = SseDecoderWithEof {
		inner: SseDecoder::with_max_size(buffer_limit),
		eof_emitted: false,
	};
	let encoder = BytesCodec::new();
	let encode_output = |(event_name, item): (&'static str, O)| {
		let json_bytes = serde_json::to_vec(&item).ok()?;
		Some(crate::parse::encode_sse_event(
			event_name,
			Bytes::from(json_bytes),
		))
	};

	super::transform::strict_parser(b, decoder, encoder, move |o| {
		let event = match o {
			Err(()) => StrictSseJsonEvent::TransportError,
			Ok(SseDecoderItem::Eof) => StrictSseJsonEvent::Eof,
			Ok(SseDecoderItem::Frame(Frame::Event(Event { name, data, .. }))) => {
				if data.as_ref() == b"[DONE]" {
					StrictSseJsonEvent::Done
				} else {
					let event_name = (name.as_ref() != "message").then(|| name.into_owned());
					let data = serde_json::from_slice::<I>(&data).map_err(anyhow::Error::from);
					StrictSseJsonEvent::Data { event_name, data }
				}
			},
			Ok(SseDecoderItem::Frame(_)) => return (vec![], false),
		};
		let (items, terminate) = f(event);
		(
			items.into_iter().filter_map(encode_output).collect(),
			terminate,
		)
	})
}

fn unwrap_sse_data(frame: Frame<Bytes>) -> Option<Bytes> {
	let Frame::Event(Event::<Bytes> { data, .. }) = frame else {
		return None;
	};
	Some(data)
}

#[allow(dead_code)]
pub(super) fn unwrap_json<T: DeserializeOwned>(frame: Frame<Bytes>) -> anyhow::Result<Option<T>> {
	Ok(
		unwrap_sse_data(frame)
			.map(|b| serde_json::from_slice(&b))
			.transpose()?,
	)
}
