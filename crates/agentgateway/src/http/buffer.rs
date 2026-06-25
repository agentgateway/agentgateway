use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use http_body::{Body as HttpBody, Frame, SizeHint};
use pin_project_lite::pin_project;

use crate::http::buflist::BufList;
use crate::*;

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod buffer_tests;

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum OverflowAction {
	// Return error if body is larger than maxbytes
	#[default]
	ReturnError,
	// Continues streaming if body is larger than maxbytes
	ContinueStreaming,
}

#[apply(schema!)]
#[derive(Default)]
pub struct BufferBody {
	/// Maximum body size to buffer in bytes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_bytes: Option<usize>,
	#[serde(default)]
	pub on_overflow: OverflowAction,
}

#[apply(schema!)]
pub struct Buffer {
	/// Buffer incoming request bodies before forwarding.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request: Option<BufferBody>,
	/// Buffer upstream response bodies before sending them to the client.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response: Option<BufferBody>,
}

impl Buffer {
	/// Drains the request body into memory and replaces it with a `Body::from(Bytes)` wrapper.
	/// No-op when buffering is disabled or for upgrade requests (whose "body" only exists
	/// post-handshake as the upgraded byte stream).
	pub async fn apply_to_request(
		&self,
		req: &mut crate::http::Request,
	) -> Result<(), crate::proxy::ProxyResponse> {
		let Some(request) = self.request.as_ref() else {
			trace!("request buffering disabled");
			return Ok(());
		};
		if req.headers().contains_key(::http::header::UPGRADE) {
			debug!("skipping request buffer for upgrade request");
			return Ok(());
		}

		let limit = request
			.max_bytes
			.unwrap_or_else(|| crate::http::buffer_limit(req));
		let body = std::mem::replace(req.body_mut(), crate::http::Body::empty());
		let buffered = match buffer_body(body, limit, request.on_overflow).await {
			Ok(b) => b,
			Err(e) => {
				warn!(limit, error = %e, "failed to buffer request body");
				let resp = ::http::Response::builder()
					.status(::http::StatusCode::PAYLOAD_TOO_LARGE)
					.body(crate::http::Body::empty())
					.expect("static response builds");
				return Err(crate::proxy::ProxyResponse::DirectResponse(Box::new(resp)));
			},
		};
		*req.body_mut() = buffered;

		Ok(())
	}

	/// Drains the response body into memory and replaces it with a `Body::from(Bytes)` wrapper.
	/// No-op when buffering is disabled or for protocol-switching (101) responses whose
	/// "body" is the upgraded byte stream.
	pub async fn apply_to_response(
		&self,
		resp: &mut crate::http::Response,
	) -> Result<(), crate::proxy::ProxyResponse> {
		let Some(response) = self.response.as_ref() else {
			trace!("response buffering disabled");
			return Ok(());
		};
		if resp.status() == ::http::StatusCode::SWITCHING_PROTOCOLS {
			debug!("skipping response buffer for protocol-switching response");
			return Ok(());
		}

		let limit = response
			.max_bytes
			.unwrap_or_else(|| crate::http::response_buffer_limit(resp));
		let body = std::mem::replace(resp.body_mut(), crate::http::Body::empty());
		let buffered = match buffer_body(body, limit, response.on_overflow).await {
			Ok(b) => b,
			Err(e) => {
				warn!(limit, error = %e, "failed to buffer response body");
				let err = ::http::Response::builder()
					.status(::http::StatusCode::BAD_GATEWAY)
					.body(crate::http::Body::empty())
					.expect("static response builds");
				return Err(crate::proxy::ProxyResponse::DirectResponse(Box::new(err)));
			},
		};
		*resp.body_mut() = buffered;

		Ok(())
	}
}

// Buffers `body` up to `limit`, picking what to do on overflow.
//
// `ReturnError` drains the whole body now and fails (so the caller can send a 413/502) if it's  bigger than `limit`.
// `ContinueStreaming` buffers up to `limit` and streams the rest.
async fn buffer_body(
	body: crate::http::Body,
	limit: usize,
	on_overflow: OverflowAction,
) -> Result<crate::http::Body, axum_core::Error> {
	match on_overflow {
		OverflowAction::ReturnError => {
			let bytes = crate::http::read_body_with_limit(body, limit).await?;
			debug!(bytes = bytes.len(), "buffered body");
			Ok(crate::http::Body::from(bytes))
		},
		OverflowAction::ContinueStreaming => {
			debug!(limit, "buffering up to limit, then streaming the rest");
			Ok(crate::http::Body::new(BufferUpToLimitBody::new(
				body, limit,
			)))
		},
	}
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum BufferState {
	/// Accumulating data; frames are withheld from the caller.
	Buffering,
	/// Limit exceeded: flush what we buffered, then stream the rest.
	FlushThenStream,
	/// Passing the rest of the body through unchanged.
	Streaming,
	/// Data is done; trailers still need to be replayed.
	EmitTrailers,
	/// Fully consumed.
	Done,
}

pin_project! {
	/// Buffers data up to `limit`, then streams the rest of the body.
	struct BufferUpToLimitBody {
		#[pin]
		inner: crate::http::Body,
		buffer: BufList,
		trailers: Option<::http::HeaderMap>,
		state: BufferState,
		limit: usize,
		buffered: usize,
	}
}

impl BufferUpToLimitBody {
	fn new(inner: crate::http::Body, limit: usize) -> Self {
		Self {
			inner,
			buffer: BufList::default(),
			trailers: None,
			state: BufferState::Buffering,
			limit,
			buffered: 0,
		}
	}
}

impl HttpBody for BufferUpToLimitBody {
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let mut this = self.project();
		loop {
			match *this.state {
				BufferState::Done => return Poll::Ready(None),
				BufferState::EmitTrailers => {
					*this.state = BufferState::Done;
					if let Some(trailers) = this.trailers.take() {
						return Poll::Ready(Some(Ok(Frame::trailers(trailers))));
					}
					return Poll::Ready(None);
				},
				BufferState::FlushThenStream => {
					*this.state = BufferState::Streaming;
					let len = this.buffer.remaining();
					let bytes = this.buffer.copy_to_bytes(len);
					if bytes.has_remaining() {
						return Poll::Ready(Some(Ok(Frame::data(bytes))));
					}
				},
				BufferState::Streaming => return this.inner.as_mut().poll_frame(cx),
				BufferState::Buffering => {},
			}

			let frame = match futures::ready!(this.inner.as_mut().poll_frame(cx)) {
				Some(Ok(frame)) => frame,
				Some(Err(error)) => {
					*this.state = BufferState::Done;
					return Poll::Ready(Some(Err(error)));
				},
				None => {
					let len = this.buffer.remaining();
					let bytes = this.buffer.copy_to_bytes(len);
					*this.state = if this.trailers.is_some() {
						BufferState::EmitTrailers
					} else {
						BufferState::Done
					};
					return Poll::Ready(Some(Ok(Frame::data(bytes))));
				},
			};

			match frame.into_data().map_err(Frame::into_trailers) {
				Ok(mut data) => {
					let len = data.remaining();
					let exceeds = this
						.buffered
						.checked_add(len)
						.is_none_or(|next| next > *this.limit);
					let bytes = data.copy_to_bytes(len);
					if bytes.has_remaining() {
						this.buffer.push(bytes);
					}
					if exceeds {
						*this.state = BufferState::FlushThenStream;
					} else {
						*this.buffered += len;
					}
				},
				Err(Ok(trailers)) => {
					*this.trailers = Some(trailers);
				},
				Err(Err(_unknown)) => {
					tracing::warn!("An unknown body frame has been buffered");
					*this.state = BufferState::Done;
					return Poll::Ready(None);
				},
			}
		}
	}

	fn is_end_stream(&self) -> bool {
		self.state == BufferState::Done
	}

	fn size_hint(&self) -> SizeHint {
		let mut hint = self.inner.size_hint();
		let buffered = self.buffer.remaining() as u64;
		hint.set_lower(hint.lower().saturating_add(buffered));
		if let Some(upper) = hint.upper() {
			hint.set_upper(upper.saturating_add(buffered));
		}
		hint
	}
}

impl crate::store::RequestPolicyTrait for Buffer {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		self.apply_to_request(req).await?;
		Ok(Default::default())
	}
}

impl crate::store::ResponsePolicyTrait for Buffer {
	async fn apply(
		&self,
		_log: &mut crate::telemetry::log::RequestLog,
		res: &mut crate::http::Response,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		self.apply_to_response(res).await?;
		Ok(Default::default())
	}
}
