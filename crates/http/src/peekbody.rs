use std::cmp;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use http::HeaderMap;
use http_body::{Body as _, Frame, SizeHint};
use pin_project_lite::pin_project;

use crate::{BufList, RawBody};

pin_project! {
	// Replays everything consumed from `inner` before resuming it.
	struct PrefixBody {
		// Initial prefix (as returned to the inspection)
		prefix: Option<Bytes>,
		// Extra prefix we read but didn't return to the inspection.
		overflow: Option<Bytes>,
		trailers: Option<HeaderMap>,
		#[pin]
		inner: RawBody,
	}
}

impl http_body::Body for PrefixBody {
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		if let Some(br) = self.prefix.take()
			&& !br.is_empty()
		{
			return Poll::Ready(Some(Ok(Frame::data(br))));
		}
		if let Some(br) = self.overflow.take()
			&& !br.is_empty()
		{
			return Poll::Ready(Some(Ok(Frame::data(br))));
		}
		if let Some(br) = self.trailers.take() {
			return Poll::Ready(Some(Ok(Frame::trailers(br))));
		}
		let this = self.project();
		this.inner.poll_frame(cx)
	}

	fn is_end_stream(&self) -> bool {
		self.prefix.as_ref().is_none_or(Bytes::is_empty)
			&& self.overflow.as_ref().is_none_or(Bytes::is_empty)
			&& self.inner.is_end_stream()
			&& self.trailers.is_none()
	}

	/// Returns the bounds on the remaining length of the stream.
	///
	/// When the **exact** remaining length of the stream is known, the upper bound will be set and
	/// will equal the lower bound.
	fn size_hint(&self) -> SizeHint {
		if self.trailers.is_some() {
			// An exact hint can select HTTP/1 Content-Length framing, which cannot
			// carry trailers. Do not hide pending trailers behind an exact length.
			return SizeHint::default();
		}
		let rem =
			self.prefix.as_ref().map_or(0, Bytes::len) + self.overflow.as_ref().map_or(0, Bytes::len);
		let mut rest = self.inner.size_hint();
		if let Some(upper) = rest.upper() {
			rest.set_upper(upper.saturating_add(rem as u64));
		}
		rest.set_lower(rest.lower() + rem as u64);
		rest
	}
}

/// Bytes read for inspection, including trailers when the whole body was read.
pub struct InspectedBody {
	pub bytes: Bytes,
	pub complete: bool,
	pub trailers: Option<HeaderMap>,
}

// Installed before an operation takes ownership of the content. On failure or
// cancellation, later consumers must fail too, not observe a successful empty body.
pub(crate) struct FailedBody(pub &'static str);

impl http_body::Body for FailedBody {
	type Data = Bytes;
	type Error = std::io::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		_cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
		Poll::Ready(Some(Err(std::io::Error::other(self.0))))
	}
}

/// Read up to `limit` bytes without losing them from subsequent delivery.
/// `Body::inspect` supplies its caller's limit + 1 to distinguish an exact fit
/// from an oversized body; this helper's limit is the actual read budget.
///
/// On partial reads, `body` becomes prefix + overflow + remaining stream. On EOF,
/// the caller must install the returned bytes/trailers as a buffered representation;
/// `body` still contains the failure placeholder.
pub async fn inspect_body(
	body: &mut RawBody,
	limit: usize,
	mut idle_timeout: Option<&mut crate::idle_timeout::IdleTimeout>,
) -> anyhow::Result<InspectedBody> {
	let mut orig = std::mem::replace(
		body,
		RawBody::new(FailedBody("body inspection failed or was cancelled")),
	);
	let mut buffer = BufList::default();
	let mut trailers = None;
	let mut inner_eof = false;
	let mut overflow = None;
	let mut want = limit;
	loop {
		if want == 0 {
			// Do not wait for EOF after exhausting the budget. Even if the last
			// frame exactly filled it, completion has not yet been established.
			break;
		}
		let frame = std::future::poll_fn(|cx| {
			if let Some(timeout) = idle_timeout.as_mut()
				&& timeout.poll_expired(cx)
			{
				return Poll::Ready(Some(Err(axum_core::Error::new(crate::BodyTimeoutError))));
			}
			let frame = Pin::new(&mut orig).poll_frame(cx);
			if let Poll::Ready(Some(Ok(frame))) = &frame
				&& let Some(timeout) = idle_timeout.as_mut()
			{
				timeout.on_frame(frame);
			}
			frame
		})
		.await;
		match frame {
			Some(Ok(frame)) => {
				if let Some(data) = frame.data_ref() {
					let want_this_read = cmp::min(data.len(), want);
					if want_this_read == 0 {
						// Empty frames count as timeout progress above, but use no byte
						// budget. Continue until data, EOF, an error, or an idle timeout.
						continue;
					}
					buffer.push(data.slice(..want_this_read));
					want -= want_this_read;
					if want_this_read < data.len() {
						overflow = Some(data.slice(want_this_read..));
					}
				} else {
					trailers = Some(frame.into_trailers().unwrap())
				}
			},
			Some(Err(err)) => {
				return Err(err.into());
			},
			None => {
				inner_eof = true;
				break;
			},
		}
	}

	let total_len = buffer.remaining();
	// CEL/parsers need contiguous bytes. Keep chunks while reading, then coalesce
	// once and share that allocation with the replay prefix when not at EOF.
	let bytes = buffer.copy_to_bytes(total_len);
	if inner_eof {
		Ok(InspectedBody {
			bytes,
			complete: true,
			trailers,
		})
	} else {
		*body = RawBody::new(PrefixBody {
			prefix: Some(bytes.clone()),
			overflow,
			trailers,
			inner: orig,
		});
		Ok(InspectedBody {
			bytes,
			complete: false,
			trailers: None,
		})
	}
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use bytes::Bytes;
	use http::HeaderMap;
	use http_body::Body as _;

	use crate::{Body, BodyInspection};

	pub async fn read(body: Body) -> Bytes {
		crate::read_body_with_limit(body, 1_097_152).await.unwrap()
	}

	// -----------------------------------------------------------------
	// 4.1  Simple sanity checks
	// -----------------------------------------------------------------
	#[tokio::test]
	async fn inspect_empty_body() {
		let mut original = Body::empty();
		let BodyInspection::Complete(inspected) = original.inspect(100).await.unwrap() else {
			panic!("expected complete inspection");
		};

		assert!(inspected.is_empty());
		assert!(read(original).await.is_empty());
	}

	#[tokio::test]
	async fn inspect_short_body() {
		let payload = b"hello world";
		let mut original = Body::from(Bytes::from_static(payload));
		let hint = original.size_hint();

		let BodyInspection::Complete(inspected) = original.inspect(100).await.unwrap() else {
			panic!("expected complete inspection");
		};

		assert_eq!(inspected, Bytes::from_static(payload));
		assert_eq!(hint.lower(), original.size_hint().lower());
		assert_eq!(hint.upper(), original.size_hint().upper());

		assert_eq!(read(original).await, Bytes::from_static(payload));
	}

	#[tokio::test]
	async fn inspect_to_eof_does_not_repoll_non_fused_stream() {
		let stream = futures_util::stream::unfold(false, |emitted| async move {
			if emitted {
				None
			} else {
				Some((
					Ok::<_, std::convert::Infallible>(http_body::Frame::data(Bytes::from_static(b"hello"))),
					true,
				))
			}
		});
		let mut original = Body::new(http_body_util::StreamBody::new(stream));

		let BodyInspection::Complete(inspected) = original.inspect(100).await.unwrap() else {
			panic!("expected complete inspection");
		};

		assert_eq!(inspected, Bytes::from_static(b"hello"));
		assert_eq!(read(original).await, Bytes::from_static(b"hello"));
	}

	#[tokio::test]
	async fn inspect_partial() {
		// 100 repeated 'a' bytes
		let payload = Bytes::from_iter(std::iter::repeat_n(b'a', 100));
		let mut original = Body::from(payload.clone());

		let hint = original.size_hint();
		let BodyInspection::Partial(inspected) = original.inspect(99).await.unwrap() else {
			panic!("expected partial inspection");
		};
		assert_eq!(hint.lower(), original.size_hint().lower());
		assert_eq!(hint.upper(), original.size_hint().upper());

		assert_eq!(inspected, payload.slice(0..99));
		assert_eq!(read(original).await, payload);
	}

	#[tokio::test]
	async fn trailers_buffered() {
		use http_body_util::BodyExt;
		// 10 repeated 'a' bytes, each their own chunk, with trailers
		let payload = Bytes::from_iter(std::iter::repeat_n(b'a', 10));
		let trailers =
			HeaderMap::try_from(&HashMap::from([("k".to_string(), "v".to_string())])).unwrap();
		let frames = std::iter::repeat_n(b'a', 10)
			.map(|msg| Ok::<_, std::io::Error>(http_body::Frame::data(Bytes::copy_from_slice(&[msg]))))
			.chain(std::iter::once(Ok::<_, std::io::Error>(
				http_body::Frame::trailers(trailers.clone()),
			)));
		let mut original = crate::Body::new(http_body_util::StreamBody::new(
			futures_util::stream::iter(frames),
		));

		let BodyInspection::Complete(inspected) = original.inspect(99).await.unwrap() else {
			panic!("expected complete inspection");
		};
		// Exact length would select Content-Length framing and lose HTTP/1 trailers.
		assert_eq!(None, original.size_hint().exact());

		assert_eq!(inspected, payload);

		let result = original.collect().await.unwrap();
		assert_eq!(Some(&trailers), result.trailers());
		assert_eq!(result.to_bytes(), payload);
	}

	#[tokio::test]
	async fn inspect_long_body_multiple_chunks() {
		use http_body_util::BodyExt;
		// 100 repeated 'a' bytes, each their own chunk, with trailers
		let payload = Bytes::from_iter(std::iter::repeat_n(b'a', 100));
		let trailers =
			HeaderMap::try_from(&HashMap::from([("k".to_string(), "v".to_string())])).unwrap();
		let frames = std::iter::repeat_n(b'a', 100)
			.map(|msg| Ok::<_, std::io::Error>(http_body::Frame::data(Bytes::copy_from_slice(&[msg]))))
			.chain(std::iter::once(Ok::<_, std::io::Error>(
				http_body::Frame::trailers(trailers.clone()),
			)));
		let mut original = crate::Body::new(http_body_util::StreamBody::new(
			futures_util::stream::iter(frames),
		));

		let hint = original.size_hint();
		let BodyInspection::Partial(inspected) = original.inspect(99).await.unwrap() else {
			panic!("expected partial inspection");
		};
		// The replayable stream includes the extra byte read to establish overflow.
		assert_eq!(100, original.size_hint().lower());
		assert_eq!(hint.upper(), original.size_hint().upper());

		assert_eq!(inspected, payload.slice(0..99));

		let result = original.collect().await.unwrap();
		assert_eq!(Some(&trailers), result.trailers());
		assert_eq!(result.to_bytes(), payload);
	}
}
