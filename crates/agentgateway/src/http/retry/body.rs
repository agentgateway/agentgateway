// Portions of this code are derived from Linkerd2 and inspired by https://linkerd.io/2021/10/26/how-linkerd-retries-http-requests-with-bodies/
// Licensed under the Apache License, Version 2.0
// Original source: https://github.com/linkerd/linkerd2-proxy
// Thanks!
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use http::HeaderMap;
use http_body::{Body, Frame, SizeHint};
use parking_lot::Mutex;
use thiserror::Error;

use crate::http::buflist::BufList;

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;

/// Wraps an HTTP body type and lazily buffers data as it is read from the inner
/// body.
///
/// When this body is dropped, if a clone exists, any buffered data is shared
/// with its cloned. The first clone to be polled will take ownership over the
/// data until it is dropped. When *that* clone is dropped, the buffered data
/// --- including any new data read from the body by the clone, if the body has
/// not yet completed --- will be shared with any remaining clones.
///
/// The buffered data can then be used to retry the request if the original
/// request fails.
#[derive(Debug)]
pub struct ReplayBody<B = crate::http::Body> {
	/// Buffered state owned by this body if it is actively being polled. If
	/// this body has been polled and no other body owned the state, this will
	/// be `Some`.
	state: Option<BodyState<B>>,

	/// Copy of the state shared across all clones. When the active clone is
	/// dropped, it moves its state back into the shared state to be taken by the
	/// next clone to be polled.
	shared: Arc<SharedState<B>>,

	/// Should this clone replay the buffered body from the shared state before
	/// polling the initial body?
	replay_body: bool,

	/// Should this clone replay trailers from the shared state?
	replay_trailers: bool,
}

#[derive(Debug, Error)]
#[error("replay body discarded after reaching maximum buffered bytes limit")]
pub struct Capped;

#[derive(Debug)]
struct SharedState<B> {
	body: Mutex<Option<BodyState<B>>>,
	/// Did the initial body return `true` from `is_end_stream` before it was
	/// ever polled? If so, always return `true`; the body is completely empty.
	///
	/// We store this separately so that clones of a totally empty body can
	/// always return `true` from `is_end_stream` even when they don't own the
	/// shared state.
	was_empty: bool,

	orig_size_hint: SizeHint,
}

#[derive(Debug)]
struct BodyState<B> {
	replay: BufList,
	replay_idx: usize,
	trailers: Option<HeaderMap>,
	rest: B,
	is_completed: bool,

	/// Maximum number of bytes to buffer.
	max_bytes: usize,
}

impl<B: Body> BodyState<B> {
	/// Records a chunk of data yielded by the inner `B`-typed [`Body`].
	///
	/// This returns the next chunk of data as a chunk of [`Bytes`].
	///
	/// This records the chunk in the replay buffer, unless the maximum capacity has been exceeded.
	/// If the buffer's capacity has been exceeded, the buffer will be emptied. The initial body
	/// will be permitted to continue, but cloned replays will fail with a
	/// [`Capped`][super::Capped] error when polled.
	fn record_bytes(&mut self, mut data: B::Data) -> Bytes {
		let length = data.remaining();
		self.max_bytes = self.max_bytes.saturating_sub(length);

		if self.is_capped() {
			// If there's data in the buffer, discard it now, since we won't
			// allow any clones to have a complete body.
			if self.replay.has_remaining() {
				tracing::debug!(
					buf.size = self.replay.remaining(),
					"Buffered maximum capacity, discarding buffer"
				);
				self.replay = Default::default();
			}
			data.copy_to_bytes(length)
		} else {
			// Buffer a clone of the bytes read on this poll.
			let bytes = data.copy_to_bytes(length);
			self.replay.push(bytes.clone());
			bytes
		}
	}
	#[inline]
	fn is_capped(&self) -> bool {
		self.max_bytes == 0
	}
}

impl<B: Body> ReplayBody<B> {
	/// Wraps an initial `Body` in a `ReplayBody`.
	///
	/// In order to prevent unbounded buffering, this takes a maximum number of bytes to buffer as a
	/// second parameter. If more than than that number of bytes would be buffered, the buffered
	/// data is discarded and any subsequent clones of this body will fail. However, the *currently
	/// active* clone of the body is allowed to continue without erroring. It will simply stop
	/// buffering any additional data for retries.
	///
	/// If the body has a size hint with a lower bound greater than `max_bytes`, the original body
	/// is returned in the error variant.
	pub fn try_new(body: B, max_bytes: usize) -> Result<Self, B> {
		let orig_size_hint = body.size_hint();
		tracing::trace!(body.size_hint = %orig_size_hint.lower(), %max_bytes);
		if orig_size_hint.lower() > max_bytes as u64 {
			return Err(body);
		}

		Ok(Self {
			shared: Arc::new(SharedState {
				body: Mutex::new(None),
				orig_size_hint,
				was_empty: body.is_end_stream(),
			}),
			state: Some(BodyState {
				replay: Default::default(),
				replay_idx: 0,
				trailers: None,
				rest: body,
				is_completed: false,
				max_bytes: max_bytes + 1,
			}),
			// The initial `ReplayBody` has no data to replay.
			replay_body: false,
			replay_trailers: false,
		})
	}

	/// Mutably borrows the body state if this clone currently owns it,
	/// or else tries to acquire it from the shared state.
	///
	/// # Panics
	///
	/// This panics if another clone has currently acquired the state, based on
	/// the assumption that a retry body will not be polled until the previous
	/// request has been dropped.
	fn acquire_state<'a>(
		state: &'a mut Option<BodyState<B>>,
		shared: &Mutex<Option<BodyState<B>>>,
	) -> &'a mut BodyState<B> {
		state.get_or_insert_with(|| shared.lock().take().expect("missing body state"))
	}

	/// Returns `Some(true)` if the body previously exceeded the configured maximum
	/// length limit.
	///
	/// If this is true, the body is now empty, and the request should *not* be
	/// retried with this body.
	///
	/// If this is `None`, another clone has currently acquired the state, and is
	/// still being polled.
	pub fn is_capped(&self) -> Option<bool> {
		self
			.state
			.as_ref()
			.map(BodyState::is_capped)
			.or_else(|| self.shared.body.lock().as_ref().map(BodyState::is_capped))
	}
}

impl<B> Body for ReplayBody<B>
where
	B: Body + Unpin,
	B::Error: Into<axum_core::Error>,
{
	type Data = Bytes;
	type Error = axum_core::Error;

	/// Polls for the next frame in this stream.
	///
	/// # Panics
	///
	/// This panics if another clone has currently acquired the state. A [`ReplayBody<B>`] MUST
	/// NOT be polled until the previous body has been dropped.
	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
		let this = self.get_mut();
		let state = Self::acquire_state(&mut this.state, &this.shared.body);
		// Move these out to avoid mutable borrow issues in the `map` closure
		// when polling the inner body.
		tracing::trace!(
			replay_body = this.replay_body,
			buf.has_remaining = state.replay.has_remaining(),
			body.is_completed = state.is_completed,
			body.max_bytes_remaining = state.max_bytes,
			"ReplayBody::poll_data"
		);

		// If we haven't replayed the buffer yet, and its not empty, return the
		// buffered data first.
		if this.replay_body {
			if let Some(frame) = state.replay.get_chunk(state.replay_idx) {
				tracing::trace!("Replaying body");
				state.replay_idx += 1;
				return Poll::Ready(Some(Ok(Frame::data(frame.clone()))));
			}
			// Don't return the buffered data again on the next poll.
			this.replay_body = false;

			if state.is_capped() {
				tracing::trace!("Cannot replay buffered body, maximum buffer length reached");
				return Poll::Ready(Some(Err(axum_core::Error::new(Capped))));
			}
		}
		if this.replay_trailers {
			this.replay_trailers = false;
			if let Some(ref trailers) = state.trailers {
				tracing::trace!("Replaying trailers");
				return Poll::Ready(Some(Ok(Frame::trailers(trailers.clone()))));
			}
		}

		// If the inner body has previously ended, don't poll it again.
		//
		// NOTE(eliza): we would expect the inner body to just happily return
		// `None` multiple times here, but `hyper::Body::channel` (which we use
		// in the tests) will panic if it is polled after returning `None`, so
		// we have to special-case this. :/
		if state.is_completed {
			return Poll::Ready(None);
		}

		// Poll the inner body for more data. If the body has ended, remember
		// that so that future clones will not try polling it again (as
		// described above).
		let frame: Frame<Self::Data> = {
			use futures::future::Either;
			use futures::ready;
			// Poll the inner body for the next frame.
			tracing::trace!("Polling initial body");
			let poll = Pin::new(&mut state.rest).poll_frame(cx).map_err(Into::into);
			let frame = match ready!(poll) {
				// The body yielded a new frame.
				Some(Ok(frame)) => frame,
				// The body yielded an error.
				Some(Err(error)) => return Poll::Ready(Some(Err(error))),
				// The body has reached the end of the stream.
				None => {
					tracing::trace!("Initial body completed");
					state.is_completed = true;
					return Poll::Ready(None);
				},
			};
			// Now, inspect the frame: was it a chunk of data, or a trailers frame?
			match Self::split_frame(frame) {
				Some(Either::Left(data)) => {
					// If we have buffered the maximum number of bytes, allow *this* body to
					// continue, but don't buffer any more.
					let chunk = state.record_bytes(data);
					Frame::data(chunk)
				},
				Some(Either::Right(trailers)) => {
					tracing::trace!("Initial body completed");
					state.trailers = Some(trailers.clone());
					state.is_completed = true;
					return Poll::Ready(Some(Ok(Frame::trailers(trailers))));
				},
				None => return Poll::Ready(None),
			}
		};

		Poll::Ready(Some(Ok(frame)))
	}

	#[tracing::instrument(
        skip_all,
        level = "trace",
        fields(
            state.is_some = %self.state.is_some(),
            replay_trailers = %self.replay_trailers,
            replay_body = %self.replay_body,
            is_completed = ?self.state.as_ref().map(|s| s.is_completed),
        )
    )]
	fn is_end_stream(&self) -> bool {
		// If the initial body was empty as soon as it was wrapped, then we are finished.
		if self.shared.was_empty {
			tracing::trace!("Initial body was empty, stream has ended");
			return true;
		}

		let Some(state) = self.state.as_ref() else {
			// This body is not currently the "active" replay being polled.
			tracing::trace!("Inactive replay body is not complete");
			return false;
		};

		// if this body has data or trailers remaining to play back, it
		// is not EOS
		let eos = !self.replay_body && !self.replay_trailers
          // if we have replayed everything, the initial body may
          // still have data remaining, so ask it
          && state.rest.is_end_stream();
		tracing::trace!(%eos, "Checked replay body end-of-stream");
		eos
	}

	#[inline]
	fn size_hint(&self) -> SizeHint {
		// If this clone isn't holding the body, return the original size hint.
		let state = match self.state.as_ref() {
			Some(state) => state,
			None => return self.shared.orig_size_hint,
		};

		// Otherwise, if we're holding the state but have dropped the inner
		// body, the entire body is buffered so we know the exact size hint.
		let buffered = state.replay.remaining() as u64;
		let rest_hint = state.rest.size_hint();

		// Otherwise, add the inner body's size hint to the amount of buffered
		// data. An upper limit is only set if the inner body has an upper
		// limit.
		let mut hint = SizeHint::default();
		hint.set_lower(buffered + rest_hint.lower());
		if let Some(rest_upper) = rest_hint.upper() {
			hint.set_upper(buffered + rest_upper);
		}
		hint
	}
}

impl<B> Clone for ReplayBody<B> {
	fn clone(&self) -> Self {
		Self {
			state: None,
			shared: self.shared.clone(),
			// The clone should try to replay from the shared state before
			// reading any additional data from the initial body.
			replay_body: true,
			replay_trailers: true,
		}
	}
}

impl<B> Drop for ReplayBody<B> {
	fn drop(&mut self) {
		// If this clone owned the shared state, put it back.
		if let Some(mut state) = self.state.take() {
			state.replay_idx = 0;
			*self.shared.body.lock() = Some(state);
		}
	}
}

impl<B: Body> ReplayBody<B> {
	/// Splits a `Frame<T>` into a chunk of data or a header map.
	///
	/// Frames do not expose their inner enums, and instead expose `into_data()` and
	/// `into_trailers()` methods. This function breaks the frame into either `Some(Left(data))`
	/// if it is given a DATA frame, and `Some(Right(trailers))` if it is given a TRAILERS frame.
	///
	/// This returns `None` if an unknown frame is provided, that is neither.
	///
	/// This is an internal helper to facilitate pattern matching in `read_body(..)`, above.
	fn split_frame(
		frame: http_body::Frame<B::Data>,
	) -> Option<futures::future::Either<B::Data, HeaderMap>> {
		use futures::future::Either;
		use http_body::Frame;
		match frame.into_data().map_err(Frame::into_trailers) {
			Ok(data) => Some(Either::Left(data)),
			Err(Ok(trailers)) => Some(Either::Right(trailers)),
			Err(Err(_unknown)) => {
				// It's possible that some sort of unknown frame could be encountered.
				tracing::warn!("An unknown body frame has been buffered");
				None
			},
		}
	}
}
