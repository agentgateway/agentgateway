use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body::{Frame, SizeHint};

use crate::idle_timeout::IdleTimeout;
use crate::{BodyInspection, RawBody, RecordedBodyHandle};

#[cfg(test)]
#[path = "body_test.rs"]
mod tests;

/// A passive observer attached to a managed body's lifetime.
///
/// Observers see frames emitted by the current body representation and remain
/// attached, without resetting, when the content is replaced. They may therefore
/// see frames from both versions if content changes after delivery has started.
/// Dropping the body also drops its observers, allowing them to own lifecycle guards.
pub trait BodyObserver: Send + 'static {
	fn on_frame(&mut self, _frame: &Frame<Bytes>) {}
	fn on_error(&mut self, _error: &axum_core::Error) {}
}

#[derive(Default)]
struct BodyObservers(Vec<Box<dyn BodyObserver>>);

impl std::fmt::Debug for BodyObservers {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("BodyObservers").field(&self.0.len()).finish()
	}
}

/// DropGuard is a no-op observer that is attached to the lifetime of the body.
/// The purpose is only to hold a value alive until the body is dropped.
struct DropGuard<D> {
	_guard: D,
}

impl<D: Send + 'static> BodyObserver for DropGuard<D> {}

struct TraceSnapshot<F: FnOnce(Bytes)> {
	recorded: RecordedBodyHandle,
	on_snapshot: Option<F>,
}

impl<F: FnOnce(Bytes)> Drop for TraceSnapshot<F> {
	fn drop(&mut self) {
		if let Some(on_snapshot) = self.on_snapshot.take() {
			on_snapshot(self.recorded.bytes());
		}
	}
}

/// A request or response body together with state derived from that body.
#[derive(Debug)]
pub struct Body(Box<BodyInner>);

// Keep request/response futures small even as managed state grows. This is
// deliberately private: callers cannot bypass the content replacement APIs.
#[derive(Debug)]
struct BodyInner {
	// Sticky inspection intent, independent of whether content happens to be bytes.
	needs_inspection: bool,
	representation: Representation,
	recorded: Option<RecordedBodyHandle>,
	observers: BodyObservers,
	idle_timeout: Option<IdleTimeout>,
}

/// State shared across sequential delivery attempts of the same original content.
/// Recording is per-attempt; lifecycle observers live until all attempts are dropped.
pub struct ReplayBodyState {
	needs_inspection: bool,
	inspection: Option<BodyInspection>,
	record_limit: Option<usize>,
	observers: std::sync::Arc<parking_lot::Mutex<BodyObservers>>,
	idle_timeout: Option<IdleTimeout>,
}

struct ReplayObservers(std::sync::Arc<parking_lot::Mutex<BodyObservers>>);

impl BodyObserver for ReplayObservers {
	fn on_error(&mut self, error: &axum_core::Error) {
		for observer in &mut self.0.lock().0 {
			observer.on_error(error);
		}
	}
	fn on_frame(&mut self, frame: &Frame<Bytes>) {
		for observer in &mut self.0.lock().0 {
			observer.on_frame(frame);
		}
	}
}

impl ReplayBodyState {
	/// `content` must replay the original content, before per-attempt transformations.
	pub fn wrap(&self, content: RawBody) -> Body {
		let representation = match &self.inspection {
			Some(BodyInspection::Complete(bytes)) => Representation::Buffered {
				bytes: bytes.clone(),
				emitted: false,
				trailers: None,
				delivery: Some(content),
			},
			inspection => Representation::Streaming {
				body: content,
				inspected_prefix: inspection.as_ref().map(|inspection| match inspection {
					BodyInspection::Partial(bytes) => bytes.clone(),
					BodyInspection::Complete(_) => unreachable!(),
				}),
			},
		};
		let mut body = Body(Box::new(BodyInner {
			needs_inspection: self.needs_inspection,
			representation,
			recorded: None,
			observers: BodyObservers::default(),
			idle_timeout: self.idle_timeout.clone(),
		}));
		if !self.observers.lock().0.is_empty() {
			body = body.with_observer(ReplayObservers(self.observers.clone()));
		}
		if let Some(limit) = self.record_limit {
			body.record(limit);
		}
		body
	}
}

/// No body progress was observed within the configured idle interval.
#[derive(Debug)]
pub struct BodyTimeoutError;

impl std::error::Error for BodyTimeoutError {}

impl std::fmt::Display for BodyTimeoutError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "response idle timeout")
	}
}

/// State-free content used when replacing a managed body's representation.
#[derive(Debug)]
pub enum BodyContent {
	Streaming(RawBody),
	Buffered(Bytes),
}

#[derive(Debug)]
enum Representation {
	Streaming {
		body: RawBody,
		/// A known prefix means a prior inspection proved that the complete
		/// body was larger than this prefix (otherwise it would be converted to Buffered)
		inspected_prefix: Option<Bytes>,
	},
	Buffered {
		bytes: Bytes,
		emitted: bool,
		trailers: Option<::http::HeaderMap>,
		/// Optional content-preserving delivery wrapper. Inspection can still use
		/// `bytes`, but forwarding/collection must poll this stream for its side effects.
		delivery: Option<RawBody>,
	},
}

impl Representation {
	fn into_body(self) -> RawBody {
		match self {
			Self::Streaming { body, .. } => body,
			buffered => RawBody::new(buffered),
		}
	}
}

impl Default for Representation {
	fn default() -> Self {
		Self::Buffered {
			bytes: Bytes::new(),
			emitted: false,
			trailers: None,
			delivery: None,
		}
	}
}

impl Body {
	/// Separate replayable input from delivery state. Each attempt must be wrapped
	/// with the returned state, so parsing input never records as forwarded output.
	pub fn into_replay_parts(mut self) -> (Body, ReplayBodyState) {
		let state = ReplayBodyState {
			needs_inspection: self.0.needs_inspection,
			inspection: self.inspection(),
			record_limit: self.0.recorded.as_ref().map(RecordedBodyHandle::limit),
			observers: std::sync::Arc::new(parking_lot::Mutex::new(std::mem::take(
				&mut self.0.observers,
			))),
			idle_timeout: self.0.idle_timeout.clone(),
		};
		self.0.recorded = None;
		(self, state)
	}

	pub fn new<B>(body: B) -> Self
	where
		B: http_body::Body<Data = Bytes> + Send + 'static,
		B::Error: Into<axum_core::BoxError>,
	{
		Body(Box::new(BodyInner {
			needs_inspection: false,
			representation: Representation::Streaming {
				body: RawBody::new(body),
				inspected_prefix: None,
			},
			recorded: None,
			observers: BodyObservers::default(),
			idle_timeout: None,
		}))
	}

	pub fn empty() -> Self {
		Self::from(Bytes::new())
	}

	pub fn from_stream<S>(stream: S) -> Self
	where
		S: futures_core::TryStream + Send + 'static,
		S::Ok: Into<Bytes>,
		S::Error: Into<axum_core::BoxError>,
	{
		Self::new(RawBody::from_stream(stream))
	}

	/// Convert to the opaque body expected by concrete framework boundaries.
	/// Boxing preserves all managed polling behavior while the body is drained.
	pub fn into_boxed(self) -> RawBody {
		RawBody::new(self)
	}

	/// Start an idle timeout, including the wait for the first chunk.
	/// Every successful frame resets it, including empty DATA; content replacement does not.
	pub fn set_idle_timeout(&mut self, duration: Duration) {
		self.0.idle_timeout = Some(IdleTimeout::new(duration));
	}

	fn check_idle_timeout(&self) -> Result<(), axum_core::Error> {
		if let Some(timeout) = &self.0.idle_timeout {
			timeout.check()?;
		}
		Ok(())
	}

	/// with_observer adds an Observer that can see all body frames. Note: if the body is modified, the
	/// observer may see the old and new body frames with no ability to distinguish these; its generally
	/// recommended to not mutate the body after an observer is added.
	pub fn with_observer(mut self, observer: impl BodyObserver) -> Self {
		self.0.observers.0.push(Box::new(observer));
		self
	}

	pub fn with_drop_guard(self, guard: impl Send + 'static) -> Self {
		self.with_observer(DropGuard { _guard: guard })
	}

	/// Capture this content stage for debugging, without changing inspection state.
	/// Known bytes are reported immediately, even if never forwarded. Streams are
	/// recorded as consumed and reported when dropped, possibly with only a prefix.
	/// The stream recorder follows the content through extraction and transforms.
	pub fn trace_content(
		mut self,
		limit: usize,
		on_snapshot: impl FnOnce(Bytes) + Send + 'static,
	) -> Self {
		match &mut self.0.representation {
			Representation::Buffered { bytes, .. } => {
				on_snapshot(bytes.slice(..bytes.len().min(limit)));
			},
			Representation::Streaming { body, .. } => {
				let (stream, recorded) =
					crate::RecordedBody::new_with_limit(std::mem::replace(body, RawBody::empty()), limit);
				let snapshot = TraceSnapshot {
					recorded,
					on_snapshot: Some(on_snapshot),
				};
				*body = Self::new(stream).with_drop_guard(snapshot).into_boxed();
			},
		}
		self
	}

	/// Consume the remaining body into contiguous bytes, enforcing `limit`.
	/// Fresh buffered bodies can return their bytes without being boxed, polled,
	/// and collected again.
	pub async fn into_bytes(self, limit: usize) -> Result<Bytes, axum_core::Error> {
		self.check_idle_timeout()?;
		if let Representation::Buffered {
			bytes,
			emitted: false,
			delivery: None,
			..
		} = &self.0.representation
			&& bytes.len() <= limit
		{
			return Ok(bytes.clone());
		}
		axum::body::to_bytes(self.into_boxed(), limit).await
	}

	/// Wrap delivery while retaining state derived from the original content.
	///
	/// Callers MUST preserve payload bytes and their order, and preserve trailers
	/// when present. Failing delivery is allowed; silently rewriting, dropping, or
	/// appending content is not. Use `transform_stream` for content changes.
	/// Cached inspection can bypass the wrapper, so it MUST NOT implement validation
	/// that must run before policy evaluation. This contract is not enforced by types.
	pub fn dangerous_wrap_stream_preserving_content(
		mut self,
		f: impl FnOnce(RawBody) -> RawBody,
	) -> Self {
		let representation = std::mem::take(&mut self.0.representation);
		let representation = match representation {
			Representation::Streaming {
				body,
				inspected_prefix,
			} => Representation::Streaming {
				body: f(body),
				inspected_prefix,
			},
			buffered @ Representation::Buffered { .. } => {
				let Representation::Buffered { bytes, .. } = &buffered else {
					unreachable!()
				};
				Representation::Buffered {
					bytes: bytes.clone(),
					emitted: false,
					trailers: None,
					delivery: Some(f(buffered.into_body())),
				}
			},
		};
		self.0.representation = representation;
		self
	}

	/// Transform the stream's content. Cached bytes are invalidated while
	/// persistent observers are retained. Only recording is reset for the new content.
	pub fn transform_stream(mut self, f: impl FnOnce(RawBody) -> RawBody) -> Self {
		let representation = std::mem::take(&mut self.0.representation);
		self.replace_content(f(representation.into_body()).into());
		self
	}

	pub fn replace_bytes(&mut self, bytes: Bytes) {
		self.replace_content(BodyContent::Buffered(bytes));
	}

	/// Move only the content out, leaving persistent observers attached to this
	/// body so a later replacement continues to be observed.
	pub fn take_content(&mut self) -> Body {
		let representation = std::mem::take(&mut self.0.representation);
		// Keep the shared recording handle, but discard bytes from the old content.
		if let Some(recorded) = &self.0.recorded {
			recorded.reset();
		}
		Body(Box::new(BodyInner {
			needs_inspection: self.0.needs_inspection,
			representation,
			recorded: None,
			observers: BodyObservers::default(),
			// Reads by extracted content reset its owner's idle timer too.
			idle_timeout: self.0.idle_timeout.clone(),
		}))
	}

	/// Replace this body's content asynchronously. The input retains state derived
	/// from the old content, while the state-free result invalidates that state.
	/// On error or cancellation, the body remains terminally failed; observers remain attached.
	pub async fn try_modify<F, Fut, E>(&mut self, f: F) -> Result<(), E>
	where
		F: FnOnce(Body) -> Fut,
		Fut: Future<Output = Result<BodyContent, E>>,
	{
		let content = self.take_content();
		// Install before invoking/awaiting the closure so cancellation cannot leave
		// a successful empty body. Only a successful result replaces this sentinel.
		self.replace_content(
			RawBody::new(crate::peekbody::FailedBody(
				"body modification failed or was cancelled",
			))
			.into(),
		);
		let replacement = f(content).await?;
		self.replace_content(replacement);
		Ok(())
	}

	/// Restore unmodified content previously extracted with `take_content`, including
	/// its inspection and remaining delivery state. The owner's observers and timeout
	/// remain attached. This is not a replacement API for unrelated request/response bodies.
	pub fn restore_content(&mut self, mut content: Body) {
		debug_assert!(content.0.recorded.is_none() && content.0.observers.0.is_empty());
		self.0.representation = std::mem::take(&mut content.0.representation);
		if let Some(recorded) = &self.0.recorded {
			recorded.reset();
			if http_body::Body::is_end_stream(&self.0.representation) {
				recorded.complete();
			}
		}
	}

	/// Install new content, invalidating inspection and recording while retaining
	/// lifecycle observers and the idle timeout.
	pub fn replace_content(&mut self, replacement: BodyContent) {
		// TODO: centralize fulfilling retained inspection requirements after streaming
		// replacement; callers must currently re-inspect before body-dependent CEL.
		self.0.representation = match replacement {
			BodyContent::Streaming(body) => Representation::Streaming {
				body,
				inspected_prefix: None,
			},
			BodyContent::Buffered(bytes) => Representation::Buffered {
				bytes,
				emitted: false,
				trailers: None,
				delivery: None,
			},
		};
		// Keep the shared recording handle, but discard bytes from the old content.
		if let Some(recorded) = &self.0.recorded {
			recorded.reset();
			// As in record(), Hyper may never poll an already-empty body.
			if http_body::Body::is_end_stream(&self.0.representation) {
				recorded.complete();
			}
		}
	}

	pub fn known_bytes(&self) -> Option<&Bytes> {
		match &self.0.representation {
			Representation::Buffered { bytes, .. } => Some(bytes),
			_ => None,
		}
	}

	pub fn inspection(&self) -> Option<BodyInspection> {
		match &self.0.representation {
			// A prefix is stored on a stream only after inspection reached the
			// limit without reaching EOF, therefore it is partial.
			Representation::Streaming {
				inspected_prefix: Some(bytes),
				..
			} => Some(BodyInspection::Partial(bytes.clone())),
			// Buffered content is already fully known. How it became buffered, and
			// the limit used by any prior caller, are not properties of the body.
			Representation::Buffered { bytes, .. } => Some(BodyInspection::Complete(bytes.clone())),
			// A stream with no prefix has not been inspected.
			Representation::Streaming { .. } => None,
		}
	}

	pub fn recorded(&self) -> Option<&RecordedBodyHandle> {
		self.0.recorded.as_ref()
	}

	pub fn record(&mut self, limit: usize) {
		if self.0.recorded.is_none() {
			let recorded = RecordedBodyHandle::new(limit);
			// Hyper need not poll a body that already reports end-of-stream.
			if http_body::Body::is_end_stream(&self.0.representation) {
				recorded.complete();
			}
			self.0.recorded = Some(recorded);
		}
	}

	/// Whether a consumer requires inspection to remain available across replacements.
	/// Neither constructing buffered content nor a one-time `inspect()` sets this.
	pub fn needs_inspection(&self) -> bool {
		self.0.needs_inspection
	}

	/// Retain an inspection requirement for downstream evaluation. This does not read
	/// the body; callers must still inspect it, including after streaming replacement.
	pub fn require_inspection(&mut self) {
		self.0.needs_inspection = true;
	}

	pub async fn inspect(&mut self, limit: usize) -> anyhow::Result<BodyInspection> {
		self.check_idle_timeout()?;
		if let Some(inspection) = self.cached_inspection(limit) {
			return Ok(inspection);
		}

		let Representation::Streaming {
			body,
			inspected_prefix,
		} = &mut self.0.representation
		else {
			unreachable!("buffered representations are handled by cached_inspection")
		};
		// A failed/cancelled read leaves a terminal error body. Clear the old
		// prefix first so a later, smaller inspection cannot bypass that failure.
		*inspected_prefix = None;
		// Inspection bypasses managed polling, so enforce/reset idle timeout on
		// each frame here without recording inspection reads as forwarded bytes.
		let inspected =
			crate::peekbody::inspect_body(body, limit.saturating_add(1), self.0.idle_timeout.as_mut())
				.await?;
		let all_bytes = inspected.bytes;
		let complete = inspected.complete;
		let result = if complete && all_bytes.len() <= limit {
			BodyInspection::Complete(all_bytes.clone())
		} else {
			BodyInspection::Partial(all_bytes.slice(..limit))
		};

		if complete {
			self.0.representation = Representation::Buffered {
				bytes: all_bytes,
				emitted: false,
				trailers: inspected.trailers,
				delivery: None,
			};
		} else {
			let Representation::Streaming {
				inspected_prefix, ..
			} = &mut self.0.representation
			else {
				unreachable!()
			};
			*inspected_prefix = Some(all_bytes.slice(..limit));
		}
		Ok(result)
	}

	fn cached_inspection(&self, limit: usize) -> Option<BodyInspection> {
		match &self.0.representation {
			// Buffered retains the complete content even after emission. Inspection
			// describes that content, not how many bytes remain to be sent.
			Representation::Buffered { bytes, .. } if bytes.len() <= limit => {
				Some(BodyInspection::Complete(bytes.clone()))
			},
			// All bytes are known, but this caller requested a smaller view. Return
			// Partial without truncating the stored bytes or changing the representation.
			Representation::Buffered { bytes, .. } => Some(BodyInspection::Partial(bytes.slice(..limit))),
			// A stored streaming prefix proves that at least one more byte exists
			// beyond it: inspection reads one byte past its limit. Thus even equality
			// proves this caller's limit is exceeded; no additional polling is needed.
			Representation::Streaming {
				inspected_prefix: Some(bytes),
				..
			} if bytes.len() >= limit => Some(BodyInspection::Partial(bytes.slice(..limit))),
			// No prefix, or this caller allows more than the previous inspection.
			// The old Partial result says nothing about this larger limit. Inspect
			// the replayable stream further to find EOF or exceed the new limit.
			Representation::Streaming { .. } => None,
		}
	}
}

impl Default for Body {
	fn default() -> Self {
		Self::empty()
	}
}

impl From<RawBody> for BodyContent {
	fn from(body: RawBody) -> Self {
		Self::Streaming(body)
	}
}

impl From<Bytes> for BodyContent {
	fn from(bytes: Bytes) -> Self {
		Self::Buffered(bytes)
	}
}

impl From<Vec<u8>> for BodyContent {
	fn from(bytes: Vec<u8>) -> Self {
		Self::Buffered(bytes.into())
	}
}

impl From<RawBody> for Body {
	fn from(body: RawBody) -> Self {
		Self::new(body)
	}
}

impl From<Vec<u8>> for Body {
	fn from(body: Vec<u8>) -> Self {
		Self::from(Bytes::from(body))
	}
}

impl From<Bytes> for Body {
	fn from(bytes: Bytes) -> Self {
		Body(Box::new(BodyInner {
			needs_inspection: false,
			representation: Representation::Buffered {
				bytes,
				emitted: false,
				trailers: None,
				delivery: None,
			},
			recorded: None,
			observers: BodyObservers::default(),
			idle_timeout: None,
		}))
	}
}

impl From<String> for Body {
	fn from(body: String) -> Self {
		Self::from(Bytes::from(body))
	}
}

impl From<&'static str> for Body {
	fn from(body: &'static str) -> Self {
		Self::from(Bytes::from_static(body.as_bytes()))
	}
}

impl http_body::Body for Body {
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let this = &mut *self.get_mut().0;
		if let Some(timeout) = &mut this.idle_timeout
			&& timeout.poll_expired(cx)
		{
			let error = axum_core::Error::new(BodyTimeoutError);
			for observer in &mut this.observers.0 {
				observer.on_error(&error);
			}
			return Poll::Ready(Some(Err(error)));
		}
		let poll = Pin::new(&mut this.representation).poll_frame(cx);
		if let Poll::Ready(frame) = &poll {
			if let Some(Err(error)) = frame {
				for observer in &mut this.observers.0 {
					observer.on_error(error);
				}
			}
			if let Some(Ok(frame)) = frame {
				if let Some(timeout) = &mut this.idle_timeout {
					timeout.on_frame(frame);
				}
				for observer in &mut this.observers.0 {
					observer.on_frame(frame);
				}
			}
			if let Some(recorded) = &this.recorded {
				match frame {
					Some(Ok(frame)) => {
						if let Some(data) = frame.data_ref() {
							recorded.push(data.clone());
						}
						// Consumers may stop after the last frame without polling None.
						if frame.trailers_ref().is_some() || this.representation.is_end_stream() {
							recorded.complete();
						}
					},
					None => recorded.complete(),
					Some(Err(_)) => {},
				}
			}
		}
		poll
	}

	fn is_end_stream(&self) -> bool {
		self.0.representation.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		self.0.representation.size_hint()
	}
}

impl http_body::Body for Representation {
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		match self.get_mut() {
			Self::Streaming { body, .. } => Pin::new(body).poll_frame(cx),
			Self::Buffered {
				delivery: Some(body),
				..
			} => Pin::new(body).poll_frame(cx),
			Self::Buffered {
				bytes,
				emitted,
				trailers,
				..
			} => {
				if !*emitted {
					*emitted = true;
					if !bytes.is_empty() {
						return Poll::Ready(Some(Ok(Frame::data(bytes.clone()))));
					}
				}
				Poll::Ready(
					trailers
						.take()
						.map(|trailers| Ok(Frame::trailers(trailers))),
				)
			},
		}
	}

	fn is_end_stream(&self) -> bool {
		match self {
			Self::Streaming { body, .. } => body.is_end_stream(),
			Self::Buffered {
				delivery: Some(body),
				..
			} => body.is_end_stream(),
			Self::Buffered {
				bytes,
				emitted,
				trailers,
				..
			} => (*emitted || bytes.is_empty()) && trailers.is_none(),
		}
	}

	fn size_hint(&self) -> SizeHint {
		match self {
			Self::Streaming { body, .. } => body.size_hint(),
			Self::Buffered {
				delivery: Some(body),
				..
			} => body.size_hint(),
			// Exact length selects Content-Length in HTTP/1, which cannot carry
			// trailers. Keep inspected trailer-bearing bodies chunked.
			Self::Buffered {
				trailers: Some(_), ..
			} => SizeHint::default(),
			Self::Buffered { bytes, emitted, .. } => {
				SizeHint::with_exact(if *emitted { 0 } else { bytes.len() as u64 })
			},
		}
	}
}
