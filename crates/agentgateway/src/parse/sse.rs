use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_sse_codec::{Event, Frame, SseDecoder, SseEncoder};

use super::passthrough::parser as passthrough_parser;
use super::transform::parser as transform_parser;
use crate::*;

pub fn json_passthrough<F: DeserializeOwned>(
	b: http::Body,
	buffer_limit: usize,
	mut f: impl FnMut(Option<anyhow::Result<F>>) + Send + 'static,
) -> http::Body {
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

pub fn json_transform<I: DeserializeOwned, O: Serialize>(
	b: http::Body,
	buffer_limit: usize,
	mut f: impl FnMut(anyhow::Result<I>) -> Option<O> + Send + 'static,
) -> http::Body {
	let decoder = SseDecoder::<Bytes>::with_max_size(buffer_limit);
	let encoder = SseEncoder::new();

	transform_parser(b, decoder, encoder, move |o| {
		let data = unwrap_sse_data(o)?;
		// Pass through [DONE] events unchanged
		if data.as_ref() == b"[DONE]" {
			return Some(Frame::Event(Event::<Bytes> {
				data: Bytes::from_static(b"[DONE]"),
				name: std::borrow::Cow::Borrowed(""),
				id: None,
			}));
		}
		let obj = serde_json::from_slice::<I>(&data);
		let transformed = f(obj.map_err(anyhow::Error::from))?;
		let json_bytes = serde_json::to_vec(&transformed).ok()?;
		Some(Frame::Event(Event::<Bytes> {
			data: Bytes::from(json_bytes),
			name: std::borrow::Cow::Borrowed(""),
			id: None,
		}))
	})
}

pub enum SseJsonEvent<I> {
	Data(anyhow::Result<I>),
	Done,
}

pub fn json_transform_multi<I: DeserializeOwned, O: Serialize, It>(
	b: http::Body,
	buffer_limit: usize,
	mut f: impl FnMut(SseJsonEvent<I>) -> It + Send + 'static,
) -> http::Body
where
	It: IntoIterator<Item = (&'static str, O)>,
	It::IntoIter: Send,
{
	let decoder = SseDecoder::<Bytes>::with_max_size(buffer_limit);
	let encoder = SseEncoder::new();

	transform_parser(b, decoder, encoder, move |o| {
		let data = unwrap_sse_data(o);
		if let Some(data) = &data
			&& data.as_ref() == b"[DONE]"
		{
			return f(SseJsonEvent::Done)
				.into_iter()
				.filter_map(|(event_name, item)| {
					let json_bytes = serde_json::to_vec(&item).ok()?;
					Some(Frame::Event(Event::<Bytes> {
						data: Bytes::from(json_bytes),
						name: std::borrow::Cow::Borrowed(event_name),
						id: None,
					}))
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
				Some(Frame::Event(Event::<Bytes> {
					data: Bytes::from(json_bytes),
					name: std::borrow::Cow::Borrowed(event_name),
					id: None,
				}))
			})
			.collect()
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
