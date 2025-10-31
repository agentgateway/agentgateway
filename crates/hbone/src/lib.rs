use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::io::Error;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use agent_core::copy;
use agent_core::prelude::*;
use bytes::{BufMut, Bytes};
use h2::Reason;
use tokio::sync::oneshot;
use tracing::trace;

pub mod client;
pub mod pool;
pub mod server;

pub trait Key: Display + Clone + Hash + Debug + PartialEq + Eq + Send + Sync + 'static {
	fn dest(&self) -> SocketAddr;
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Config {
	pub window_size: u32,
	pub connection_window_size: u32,
	pub frame_size: u32,
	pub pool_max_streams_per_conn: u16,
	pub pool_unused_release_timeout: Duration,
}

async fn do_ping_pong(
	mut ping_pong: h2::PingPong,
	tx: oneshot::Sender<()>,
	dropped: Arc<AtomicBool>,
) {
	const PING_INTERVAL: Duration = Duration::from_secs(10);
	const PING_TIMEOUT: Duration = Duration::from_secs(20);
	// delay before sending the first ping, no need to race with the first request
	tokio::time::sleep(PING_INTERVAL).await;
	loop {
		if dropped.load(Ordering::Relaxed) {
			return;
		}
		let ping_fut = ping_pong.ping(h2::Ping::opaque());
		trace!("ping sent");
		match tokio::time::timeout(PING_TIMEOUT, ping_fut).await {
			Err(_) => {
				// We will log this again up in drive_connection, so don't worry about a high log level
				trace!("ping timeout");
				let _ = tx.send(());
				return;
			},
			Ok(r) => match r {
				Ok(_) => {
					trace!("pong received");
					tokio::time::sleep(PING_INTERVAL).await;
				},
				Err(e) => {
					if dropped.load(Ordering::Relaxed) {
						// drive_connection() exits first, no need to error again
						return;
					}
					error!("ping error: {e}");
					let _ = tx.send(());
					return;
				},
			},
		}
	}
}

/// RWStream is an adapter that takes an H2Stream and makes it implement the standard async IO traits
pub struct RWStream {
	pub stream: H2Stream,
	pub buf: Bytes,
	/// Optional drain sender - when dropped, signals the H2 connection to drain.
	/// Used for double HBONE to properly terminate the inner connection.
	pub drain_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl tokio::io::AsyncRead for RWStream {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		read_buf: &mut tokio::io::ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		use copy::ResizeBufRead;
		let this = &mut self;
		if this.buf.is_empty() {
			let res = std::task::ready!(Pin::new(&mut this.stream.read).poll_bytes(cx))?;
			this.buf = res;
		}
		let cnt = std::cmp::min(this.buf.len(), read_buf.remaining());
		read_buf.put_slice(&this.buf[..cnt]);
		this.buf = this.buf.split_off(cnt);
		Poll::Ready(Ok(()))
	}
}

impl tokio::io::AsyncWrite for RWStream {
	fn poll_write(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, Error>> {
		use copy::AsyncWriteBuf;
		Pin::new(&mut self.stream.write).poll_write_buf(cx, Bytes::copy_from_slice(buf))
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		use copy::AsyncWriteBuf;
		Pin::new(&mut self.stream.write).poll_flush(cx)
	}

	fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		use copy::AsyncWriteBuf;
		Pin::new(&mut self.stream.write).poll_shutdown(cx)
	}
}

impl Drop for RWStream {
	fn drop(&mut self) {
		// Send drain signal if we have a drain_tx
		if let Some(drain_tx) = &self.drain_tx {
			let _ = drain_tx.send(true);
			trace!("sent drain signal for H2 connection");
		}
	}
}
// H2Stream represents an active HTTP2 stream. Consumers can only Read/Write
pub struct H2Stream {
	read: H2StreamReadHalf,
	write: H2StreamWriteHalf,
}

pub struct H2StreamReadHalf {
	recv_stream: h2::RecvStream,
	_dropped: Option<DropCounter>,
}

pub struct H2StreamWriteHalf {
	send_stream: h2::SendStream<Bytes>,
	_dropped: Option<DropCounter>,
}

pub struct TokioH2Stream(H2Stream);

struct DropCounter {
	// Whether the other end of this shared counter has already dropped.
	// We only decrement if they have, so we do not double count
	half_dropped: Arc<()>,
	active_count: Arc<AtomicU16>,
}

impl DropCounter {
	pub fn new(active_count: Arc<AtomicU16>) -> (Option<DropCounter>, Option<DropCounter>) {
		let half_dropped = Arc::new(());
		let d1 = DropCounter {
			half_dropped: half_dropped.clone(),
			active_count: active_count.clone(),
		};
		let d2 = DropCounter {
			half_dropped,
			active_count,
		};
		(Some(d1), Some(d2))
	}
}

impl copy::BufferedSplitter for H2Stream {
	type R = H2StreamReadHalf;
	type W = H2StreamWriteHalf;
	fn split_into_buffered_reader(self) -> (H2StreamReadHalf, H2StreamWriteHalf) {
		let H2Stream { read, write } = self;
		(read, write)
	}
}

impl H2StreamWriteHalf {
	fn write_slice(&mut self, buf: Bytes, end_of_stream: bool) -> Result<(), std::io::Error> {
		self
			.send_stream
			.send_data(buf, end_of_stream)
			.map_err(h2_to_io_error)
	}
}

impl Drop for DropCounter {
	fn drop(&mut self) {
		let mut half_dropped = Arc::new(());
		std::mem::swap(&mut self.half_dropped, &mut half_dropped);
		if Arc::into_inner(half_dropped).is_none() {
			// other half already dropped
			let left = self.active_count.fetch_sub(1, Ordering::SeqCst);
			trace!("dropping H2Stream, has {} active streams left", left - 1);
		} else {
			trace!("dropping H2Stream, other half remains");
		}
	}
}

// We can't directly implement tokio::io::{AsyncRead, AsyncWrite} for H2Stream because
// then the specific implementation will conflict with the generic one.
impl TokioH2Stream {
	pub fn new(stream: H2Stream) -> Self {
		Self(stream)
	}
}

impl tokio::io::AsyncRead for TokioH2Stream {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut tokio::io::ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		let pinned = std::pin::Pin::new(&mut self.0.read);
		copy::ResizeBufRead::poll_bytes(pinned, cx).map(|r| match r {
			Ok(bytes) => {
				if buf.remaining() < bytes.len() {
					Err(Error::other(format!(
						"kould overflow buffer of with {} remaining",
						buf.remaining()
					)))
				} else {
					buf.put(bytes);
					Ok(())
				}
			},
			Err(e) => Err(e),
		})
	}
}

impl tokio::io::AsyncWrite for TokioH2Stream {
	fn poll_write(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, tokio::io::Error>> {
		let pinned = std::pin::Pin::new(&mut self.0.write);
		let buf = Bytes::copy_from_slice(buf);
		copy::AsyncWriteBuf::poll_write_buf(pinned, cx, buf)
	}

	fn poll_flush(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Result<(), std::io::Error>> {
		let pinned = std::pin::Pin::new(&mut self.0.write);
		copy::AsyncWriteBuf::poll_flush(pinned, cx)
	}

	fn poll_shutdown(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Result<(), std::io::Error>> {
		let pinned = std::pin::Pin::new(&mut self.0.write);
		copy::AsyncWriteBuf::poll_shutdown(pinned, cx)
	}
}

impl copy::ResizeBufRead for H2StreamReadHalf {
	fn poll_bytes(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<Bytes>> {
		let this = self.get_mut();
		loop {
			match ready!(this.recv_stream.poll_data(cx)) {
				None => return Poll::Ready(Ok(Bytes::new())),
				Some(Ok(buf)) if buf.is_empty() && !this.recv_stream.is_end_stream() => continue,
				Some(Ok(buf)) => {
					// TODO: Hyper and Go make their pinging data aware and don't send pings when data is received
					// Pingora, and our implementation, currently don't do this.
					// We may want to; if so, modify here.
					// this.ping.record_data(buf.len());
					let _ = this.recv_stream.flow_control().release_capacity(buf.len());
					return Poll::Ready(Ok(buf));
				},
				Some(Err(e)) => {
					return Poll::Ready(match e.reason() {
						Some(Reason::NO_ERROR) | Some(Reason::CANCEL) => {
							return Poll::Ready(Ok(Bytes::new()));
						},
						Some(Reason::STREAM_CLOSED) => Err(Error::new(std::io::ErrorKind::BrokenPipe, e)),
						_ => Err(h2_to_io_error(e)),
					});
				},
			}
		}
	}

	fn resize(self: Pin<&mut Self>, _new_size: usize) {
		// NOP, we don't need to resize as we are abstracting the h2 buffer
	}
}

impl copy::AsyncWriteBuf for H2StreamWriteHalf {
	fn poll_write_buf(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: Bytes,
	) -> Poll<std::io::Result<usize>> {
		if buf.is_empty() {
			return Poll::Ready(Ok(0));
		}
		self.send_stream.reserve_capacity(buf.len());

		// We ignore all errors returned by `poll_capacity` and `write`, as we
		// will get the correct from `poll_reset` anyway.
		let cnt = match ready!(self.send_stream.poll_capacity(cx)) {
			None => Some(0),
			Some(Ok(cnt)) => self.write_slice(buf.slice(..cnt), false).ok().map(|()| cnt),
			Some(Err(_)) => None,
		};

		if let Some(cnt) = cnt {
			return Poll::Ready(Ok(cnt));
		}

		Poll::Ready(Err(h2_to_io_error(
			match ready!(self.send_stream.poll_reset(cx)) {
				Ok(Reason::NO_ERROR) | Ok(Reason::CANCEL) | Ok(Reason::STREAM_CLOSED) => {
					return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
				},
				Ok(reason) => reason.into(),
				Err(e) => e,
			},
		)))
	}

	fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Poll::Ready(Ok(()))
	}

	fn poll_shutdown(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Result<(), std::io::Error>> {
		let r = self.write_slice(Bytes::new(), true);
		if r.is_ok() {
			return Poll::Ready(Ok(()));
		}

		Poll::Ready(Err(h2_to_io_error(
			match ready!(self.send_stream.poll_reset(cx)) {
				Ok(Reason::NO_ERROR) => return Poll::Ready(Ok(())),
				Ok(Reason::CANCEL) | Ok(Reason::STREAM_CLOSED) => {
					return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
				},
				Ok(reason) => reason.into(),
				Err(e) => e,
			},
		)))
	}
}

fn h2_to_io_error(e: h2::Error) -> std::io::Error {
	if e.is_io() {
		e.into_io().unwrap()
	} else {
		std::io::Error::other(e)
	}
}
