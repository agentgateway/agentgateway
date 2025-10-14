use crate::transport::stream::SocketType;
use bytes::{Buf, Bytes, BytesMut};
use std::cmp;
use std::io::{Error, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tokio::io::AsyncWrite;
use tokio::io::{AsyncRead, ReadBuf};

#[cfg(test)]
#[path = "rewind_tests.rs"]
mod tests;

pub struct RewindSocket {
	state: State,
	io: SocketType,
}

impl RewindSocket {
	pub fn new(io: SocketType) -> Self {
		Self {
			state: State::Filling(Default::default()),
			io,
		}
	}

	pub fn rewind(&mut self) {
		match std::mem::replace(&mut self.state, State::Draining(None)) {
			State::Filling(b) => {
				self.state = State::Draining(Some(b.freeze()));
			},
			State::Draining(_) => {
				panic!("rewind() may only be called once")
			},
		}
	}

	pub fn discard(self) -> SocketType {
		self.io
	}
}

pub enum State {
	Filling(BytesMut),
	Draining(Option<Bytes>),
}

impl AsyncRead for RewindSocket {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		let this = &mut *self;
		match &mut this.state {
			State::Filling(b) => {
				let filled_length = buf.filled().len();
				ready!(Pin::new(&mut this.io).poll_read(cx, buf))?;
				let written = &buf.filled()[filled_length..];
				b.extend_from_slice(written);
				Poll::Ready(Ok(()))
			},
			State::Draining(pre) => {
				if let Some(mut prefix) = pre.take() {
					// If there are no remaining bytes, let the bytes get dropped.
					if !prefix.is_empty() {
						let copy_len = cmp::min(prefix.len(), buf.remaining());
						buf.put_slice(&prefix[..copy_len]);
						prefix.advance(copy_len);
						// Put back what's left
						if !prefix.is_empty() {
							let _ = std::mem::replace(&mut self.state, State::Draining(Some(prefix)));
						}

						return Poll::Ready(Ok(()));
					}
				}
				Pin::new(&mut self.io).poll_read(cx, buf)
			},
		}
	}
}

impl AsyncWrite for RewindSocket {
	fn poll_write(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, Error>> {
		Pin::new(&mut self.io).poll_write(cx, buf)
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.io).poll_flush(cx)
	}

	fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.io).poll_shutdown(cx)
	}

	fn poll_write_vectored(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		bufs: &[IoSlice<'_>],
	) -> Poll<Result<usize, Error>> {
		Pin::new(&mut self.io).poll_write_vectored(cx, bufs)
	}

	fn is_write_vectored(&self) -> bool {
		self.io.is_write_vectored()
	}
}
