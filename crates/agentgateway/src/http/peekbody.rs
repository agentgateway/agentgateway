use crate::http::Body;
use crate::http::buflist::BufList;
use axum_core::Error;
use bytes::{Buf, Bytes};
use http_body::Frame;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use pin_project_lite::pin_project;
use std::cmp;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

pin_project! {
	struct PeekBody {
		limit: usize,
		sender: Option<tokio::sync::oneshot::Sender<Bytes>>,
		buffer: BufList,
		#[pin]
		inner: Body,
	}
}

impl http_body::Body for PeekBody {
	type Data = Bytes;
	type Error = crate::http::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let this = self.project();
		let res = match ready!(this.inner.poll_frame(cx)) {
			None => {
				let want = cmp::min(this.buffer.remaining(), *this.limit);

				// We are done! Send the buffer to the sender and return None
				let _ = this
					.sender
					.take()
					.expect("polled None twice")
					.send(this.buffer.copy_to_bytes(want));
				None
			},
			Some(Ok(frame)) => {
				if let Some(data) = frame.data_ref() {
					if this.sender.is_some() {
						let want = cmp::min(*this.limit - this.buffer.remaining(), data.len());
						// We are still trying to peek
						this.buffer.push(data.slice(0..want));
						if this.buffer.remaining() >= *this.limit {
							let _ = this
								.sender
								.take()
								.expect("polled None twice")
								.send(this.buffer.copy_to_bytes(*this.limit));
						}
					}
					Some(Ok(frame))
				} else {
					Some(Ok(frame))
				}
			},
			Some(Err(err)) => Some(Err(err)),
		};
		Poll::Ready(res)
	}
}

pub async fn inspect_body(body: &mut Body, limit: usize) -> anyhow::Result<Bytes> {
	let (sender, receiver) = tokio::sync::oneshot::channel();
	let orig = std::mem::replace(body, Body::empty());
	let pb = PeekBody {
		limit,
		sender: Some(sender),
		buffer: Default::default(),
		inner: orig,
	};
	*body = Body::new(pb);
	receiver.await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::Body;
	use bytes::Bytes;
	use http::HeaderMap;
	use std::collections::HashMap;

	pub async fn read(body: Body) -> Bytes {
		axum::body::to_bytes(body, 2_097_152).await.unwrap()
	}

	// -----------------------------------------------------------------
	// 4.1  Simple sanity checks
	// -----------------------------------------------------------------
	#[tokio::test]
	async fn inspect_empty_body() {
		let mut original = Body::empty();
		let inspected = inspect_body(&mut original, 100).await.unwrap();

		assert!(inspected.is_empty());
		assert!(read(original).await.is_empty());
	}

	#[tokio::test]
	async fn inspect_short_body() {
		let payload = b"hello world";
		let mut original = Body::from(payload.as_slice());

		let inspected = inspect_body(&mut original, 100).await.unwrap();

		assert_eq!(inspected, Bytes::from_static(payload));

		assert_eq!(read(original).await, Bytes::from_static(payload));
	}

	#[tokio::test]
	async fn inspect_partial() {
		// 100 repeated 'a' bytes
		let payload = Bytes::from_iter(std::iter::repeat(b'a').take(100));
		let mut original = Body::from(payload.clone());

		let inspected = inspect_body(&mut original, 99).await.unwrap();

		assert_eq!(inspected, payload.slice(0..99));
		assert_eq!(read(original).await, payload);
	}

	#[tokio::test]
	async fn inspect_long_body_multiple_chunks() {
		use http_body_util::BodyExt;
		// 100 repeated 'a' bytes, each their own chunk, with trailers
		let payload = Bytes::from_iter(std::iter::repeat(b'a').take(100));
		let trailers =
			HeaderMap::try_from(&HashMap::from([("k".to_string(), "v".to_string())])).unwrap();
		let frames = std::iter::repeat(b'a')
			.take(100)
			.map(|msg| Ok::<_, std::io::Error>(http_body::Frame::data(Bytes::copy_from_slice(&[msg]))))
			.chain(std::iter::once(Ok::<_, std::io::Error>(
				http_body::Frame::trailers(trailers.clone()),
			)));
		let mut original = crate::http::Body::new(http_body_util::StreamBody::new(
			futures_util::stream::iter(frames),
		));

		let inspected = inspect_body(&mut original, 99).await.unwrap();

		assert_eq!(inspected, payload.slice(0..99));

		let result = original.collect().await.unwrap();
		assert_eq!(Some(&trailers), result.trailers());
		assert_eq!(result.to_bytes(), payload);
	}
}
