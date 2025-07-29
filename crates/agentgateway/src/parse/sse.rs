use std::fmt;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum_core::Error;
use bytes::{Buf, Bytes, BytesMut};
use futures::{Stream, StreamExt, TryStreamExt};
use http_body::Body;
use http_body_util::BodyExt;
use pin_project_lite::pin_project;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_sse_codec::{Event, Frame, SseDecoder, SseEncoder};
use tokio_util::codec::{Decoder, Encoder, FramedRead};
use tokio_util::io::StreamReader;

use super::passthrough::parser as passthrough_parser;
use super::transform::parser as transform_parser;
use crate::*;

pub fn json_passthrough<F: DeserializeOwned>(
	b: http::Body,
	mut f: impl FnMut(Option<anyhow::Result<F>>) + Send + 'static,
) -> http::Body {
	let decoder = SseDecoder::<Bytes>::with_max_size(2_097_152);

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
	mut f: impl FnMut(anyhow::Result<I>) -> Option<O> + Send + 'static,
) -> http::Body {
	let decoder = SseDecoder::<Bytes>::with_max_size(2_097_152);
	let encoder = SseEncoder::new();

	transform_parser(b, decoder, encoder, move |o| {
		let data = unwrap_sse_data(o)?;
		// Pass through [DONE] events unchanged
		if data.as_ref() == b"[DONE]" {
			return Some(Frame::Event(Event::<Bytes> {
				data: Bytes::copy_from_slice(b"[DONE]"),
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

fn unwrap_sse_data(frame: Frame<Bytes>) -> Option<Bytes> {
	let Frame::Event(Event::<Bytes> { data, .. }) = frame else {
		return None;
	};
	Some(data)
}

pub(super) fn unwrap_json<T: DeserializeOwned>(frame: Frame<Bytes>) -> anyhow::Result<Option<T>> {
	Ok(
		unwrap_sse_data(frame)
			.map(|b| serde_json::from_slice(&b))
			.transpose()?,
	)
}
