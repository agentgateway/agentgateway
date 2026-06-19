use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use pin_project_lite::pin_project;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::wrappers::ReceiverStream;

use super::{BodySendMode, Error};
use crate::http::buflist::BufList;
use crate::http::{self, bufferbody};

pub(super) enum BufferedBodyPhase {
	Deferred {
		body: http::Body,
		mode: BodySendMode,
		limit: usize,
		error_message: &'static str,
	},
	Replaying {
		original: Bytes,
	},
	PartialReplaying {
		original: Option<Bytes>,
		remainder: Option<http::Body>,
	},
}

pub(super) enum PendingBufferedBody {
	Body {
		body: http::Body,
		handle: bufferbody::BufferedBodyHandle,
		error_message: &'static str,
	},
	Partial {
		body: Bytes,
		end_stream: bool,
		trailers: Option<::http::HeaderMap>,
	},
}

impl BufferedBodyPhase {
	fn new(body: http::Body, mode: BodySendMode, limit: usize, error_message: &'static str) -> Self {
		Self::Deferred {
			body,
			mode,
			limit,
			error_message,
		}
	}

	pub(super) async fn take_pending_send(
		phase: &mut Option<Self>,
	) -> Result<Option<PendingBufferedBody>, Error> {
		let Some(buffered_body) = phase.take() else {
			return Ok(None);
		};
		match buffered_body {
			Self::Deferred {
				body,
				mode,
				limit,
				error_message,
			} => match mode {
				BodySendMode::Buffered => {
					let (buffered_body, handle) = bufferbody::BufferedBody::new_with_limit(body, limit);
					Ok(Some(PendingBufferedBody::Body {
						body: http::Body::new(buffered_body),
						handle,
						error_message,
					}))
				},
				BodySendMode::BufferedPartial => {
					let buffered = buffer_body_partial(body, limit, error_message).await?;
					*phase = Some(Self::PartialReplaying {
						original: Some(buffered.body.clone()),
						remainder: buffered.remainder,
					});
					Ok(Some(PendingBufferedBody::Partial {
						body: buffered.body,
						end_stream: buffered.end_stream,
						trailers: buffered.trailers,
					}))
				},
				_ => Ok(None),
			},
			replaying @ Self::Replaying { .. } => {
				*phase = Some(replaying);
				Ok(None)
			},
			partial @ Self::PartialReplaying { .. } => {
				*phase = Some(partial);
				Ok(None)
			},
		}
	}

	pub(super) fn take_deferred_body(phase: &mut Option<Self>) -> Option<http::Body> {
		let deferred = phase.take()?;
		match deferred {
			Self::Deferred { body, .. } => Some(body),
			other => {
				*phase = Some(other);
				None
			},
		}
	}

	pub(super) fn update_deferred_mode(phase: &mut Option<Self>, next_mode: BodySendMode) {
		if let Some(Self::Deferred { mode, .. }) = phase.as_mut() {
			*mode = next_mode;
		}
	}

	pub(super) fn take_partial_remainder(phase: &mut Option<Self>) -> Option<http::Body> {
		let buffered = phase.take()?;
		match buffered {
			Self::PartialReplaying { remainder, .. } => remainder,
			other => {
				*phase = Some(other);
				None
			},
		}
	}

	pub(super) fn replay_from_handle(
		phase: &mut Option<Self>,
		handle: bufferbody::BufferedBodyHandle,
	) -> Result<(), Error> {
		let Some(original) = handle.bytes() else {
			return Err(Error::BodyBuffer(
				"buffered body completed without captured bytes".into(),
			));
		};
		*phase = Some(Self::Replaying { original });
		Ok(())
	}

	pub(super) async fn restore_original_if(
		phase: &mut Option<Self>,
		restore: bool,
		body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	) {
		if !restore {
			return;
		}
		let Some(buffered_body) = phase.take() else {
			return;
		};
		match buffered_body {
			Self::Replaying { original } => {
				let _ = body_tx.send(Ok(Frame::data(original))).await;
			},
			Self::PartialReplaying {
				mut original,
				remainder,
			} => {
				if let Some(original) = original.take()
					&& !original.is_empty()
				{
					let _ = body_tx.send(Ok(Frame::data(original))).await;
				}
				*phase = Some(Self::PartialReplaying {
					original,
					remainder,
				});
			},
			other => {
				*phase = Some(other);
			},
		}
	}
}

struct BufferedPartialBody {
	body: Bytes,
	end_stream: bool,
	trailers: Option<::http::HeaderMap>,
	remainder: Option<http::Body>,
}

pin_project! {
	// Body wrapper used when the configured partial-buffer limit falls in the middle of a data
	// frame. The bytes after the limit have already been read, so replay them before polling the
	// original body for the rest of the stream.
	struct RemainderBody {
		prefix: Option<Bytes>,
		#[pin]
		inner: http::Body,
	}
}

impl RemainderBody {
	fn new(prefix: Option<Bytes>, inner: http::Body) -> Self {
		Self { prefix, inner }
	}
}

impl Body for RemainderBody {
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let this = self.project();
		if let Some(prefix) = this.prefix.take()
			&& !prefix.is_empty()
		{
			return Poll::Ready(Some(Ok(Frame::data(prefix))));
		}
		this.inner.poll_frame(cx)
	}

	fn is_end_stream(&self) -> bool {
		self.prefix.as_ref().map(Bytes::is_empty).unwrap_or(true) && self.inner.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		let prefix_len = self.prefix.as_ref().map(Bytes::len).unwrap_or_default() as u64;
		let mut hint = self.inner.size_hint();
		hint.set_lower(hint.lower().saturating_add(prefix_len));
		if let Some(upper) = hint.upper() {
			hint.set_upper(upper.saturating_add(prefix_len));
		}
		hint
	}
}

async fn buffer_body_partial(
	mut body: http::Body,
	limit: usize,
	error_message: &'static str,
) -> Result<BufferedPartialBody, Error> {
	// A zero-byte limit still means BufferedPartial should send a body message to ext_proc; it
	// just contains no data and the original body remains entirely local pass-through.
	if limit == 0 {
		return Ok(BufferedPartialBody {
			body: Bytes::new(),
			end_stream: false,
			trailers: None,
			remainder: Some(body),
		});
	}

	let mut buffer = BufList::default();
	let mut buffered = 0usize;
	let mut trailers = None;
	loop {
		let Some(frame) = body
			.frame()
			.await
			.transpose()
			.map_err(|e| Error::BodyBuffer(format!("{error_message}: {e}")))?
		else {
			let body = buffered_bytes(buffer);
			return Ok(BufferedPartialBody {
				body,
				end_stream: trailers.is_none(),
				trailers,
				remainder: None,
			});
		};

		match frame.into_data().map_err(Frame::into_trailers) {
			Ok(mut data) => {
				let len = data.remaining();
				let remaining_limit = limit.saturating_sub(buffered);
				if len <= remaining_limit {
					// The whole frame fits in the partial buffer. If this reaches the limit and
					// the body is not at EOF, stop reading and leave the rest for normal upstream
					// or downstream forwarding.
					let bytes = data.copy_to_bytes(len);
					if bytes.has_remaining() {
						buffer.push(bytes);
						buffered += len;
					}
					if buffered == limit && !body.is_end_stream() {
						return Ok(BufferedPartialBody {
							body: buffered_bytes(buffer),
							end_stream: false,
							trailers: None,
							remainder: Some(body),
						});
					}
				} else {
					// The frame crosses the partial-buffer boundary. Split it so ext_proc sees
					// only the prefix, then replay the remaining bytes before polling the body
					// again.
					let prefix = data.copy_to_bytes(remaining_limit);
					if prefix.has_remaining() {
						buffer.push(prefix);
					}
					let rest = data.copy_to_bytes(data.remaining());
					let remainder = http::Body::new(RemainderBody::new(Some(rest), body));
					return Ok(BufferedPartialBody {
						body: buffered_bytes(buffer),
						end_stream: false,
						trailers: None,
						remainder: Some(remainder),
					});
				}
			},
			Err(Ok(frame_trailers)) => {
				trailers = Some(frame_trailers);
			},
			Err(Err(_unknown)) => {
				tracing::warn!("An unknown body frame has been buffered");
				return Ok(BufferedPartialBody {
					body: buffered_bytes(buffer),
					end_stream: true,
					trailers,
					remainder: None,
				});
			},
		}
	}
}

fn buffered_bytes(mut buffer: BufList) -> Bytes {
	let len = buffer.remaining();
	buffer.copy_to_bytes(len)
}

pub(super) fn attach_request_body_channel(
	req: http::Request,
	rx_chunk: &mut Option<Receiver<Result<Frame<Bytes>, Infallible>>>,
) -> (http::Request, http::Body) {
	let (parts, body) = req.into_parts();
	let rx_chunk = rx_chunk
		.take()
		.expect("request body channel should only be attached once");
	let upstream_body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
	(
		http::Request::from_parts(parts, http::Body::new(upstream_body)),
		body,
	)
}

#[cfg(debug_assertions)]
pub(super) fn debug_assert_preserved_request_body(
	req: &http::Request,
	had_body: bool,
	context: &'static str,
) {
	if had_body {
		debug_assert!(
			!req.body().is_end_stream(),
			"{context}: returning request should preserve the original non-empty body"
		);
	}
}

#[cfg(not(debug_assertions))]
pub(super) fn debug_assert_preserved_request_body(
	_req: &http::Request,
	_had_body: bool,
	_context: &'static str,
) {
}

fn should_buffer_body(body_mode: BodySendMode, had_body: bool) -> bool {
	had_body
		&& matches!(
			body_mode,
			BodySendMode::Buffered | BodySendMode::BufferedPartial
		)
}

pub(super) fn start_buffered_request_body(
	req: http::Request,
	body_mode: BodySendMode,
	had_body: bool,
	rx_chunk: &mut Option<Receiver<Result<Frame<Bytes>, Infallible>>>,
) -> (http::Request, Option<BufferedBodyPhase>) {
	if !should_buffer_body(body_mode, had_body) {
		return (req, None);
	}
	let max_request_bytes = http::buffer_limit(&req);
	let (req, req_body) = attach_request_body_channel(req, rx_chunk);
	let phase = BufferedBodyPhase::new(
		req_body,
		body_mode,
		max_request_bytes,
		"failed to read request body for buffering",
	);
	(req, Some(phase))
}

pub(super) fn start_buffered_response_body(
	body: &mut Option<http::Body>,
	body_mode: BodySendMode,
	had_body: bool,
	max_response_bytes: usize,
) -> Option<BufferedBodyPhase> {
	if !should_buffer_body(body_mode, had_body) {
		return None;
	}
	let body = body
		.take()
		.expect("response body should be available before buffering starts");
	Some(BufferedBodyPhase::new(
		body,
		body_mode,
		max_response_bytes,
		"failed to read response body for buffering",
	))
}
