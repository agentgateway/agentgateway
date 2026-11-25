use std::future::Future;
use std::io::{Error, IoSlice};
use std::marker::PhantomPinned;
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, ready};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use pin_project_lite::pin_project;
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tracing::trace;

// BufferedSplitter is a trait to expose splitting an IO object into a buffered reader and a writer
pub trait BufferedSplitter: Unpin {
	type R: ResizeBufRead + Unpin;
	type W: AsyncWriteBuf + Unpin;
	fn split_into_buffered_reader(self) -> (Self::R, Self::W);
}

// Generic BufferedSplitter for anything that can Read/Write.
impl<I> BufferedSplitter for I
where
	I: AsyncRead + AsyncWrite + Unpin,
{
	type R = BufReader<io::ReadHalf<I>>;
	type W = WriteAdapter<io::WriteHalf<I>>;
	fn split_into_buffered_reader(self) -> (Self::R, Self::W) {
		let (rh, wh) = tokio::io::split(self);
		let rb = BufReader::new(rh);
		(rb, WriteAdapter(wh))
	}
}

// TcpStreamSplitter is a specialized BufferedSplitter for TcpStream, which is more efficient than the generic
// `tokio::io::split`. The generic method involves locking to access the read and write halves
pub struct TcpStreamSplitter(pub TcpStream);

impl BufferedSplitter for TcpStreamSplitter {
	type R = BufReader<OwnedReadHalf>;
	type W = WriteAdapter<OwnedWriteHalf>;

	fn split_into_buffered_reader(self) -> (Self::R, Self::W) {
		let (rh, wh) = self.0.into_split();
		let rb = BufReader::new(rh);
		(rb, WriteAdapter(wh))
	}
}

// AsyncWriteBuf is like AsyncWrite, but writes a Bytes instead of &[u8]. This allows avoiding copies.
pub trait AsyncWriteBuf {
	fn poll_write_buf(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: Bytes,
	) -> Poll<std::io::Result<usize>>;
	fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>>;
	fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>>;
}

// Allow &T to be AsyncWriteBuf
impl<T: ?Sized + AsyncWriteBuf + Unpin> AsyncWriteBuf for &mut T {
	fn poll_write_buf(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: Bytes,
	) -> Poll<std::io::Result<usize>> {
		Pin::new(&mut **self).poll_write_buf(cx, buf)
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut **self).poll_flush(cx)
	}

	fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut **self).poll_shutdown(cx)
	}
}

// Allow anything that is AsyncWrite to be AsyncWriteBuf.
pub struct WriteAdapter<T>(T);

impl<T: AsyncWrite + Unpin> AsyncWriteBuf for WriteAdapter<T> {
	fn poll_write_buf(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		mut buf: Bytes,
	) -> Poll<std::io::Result<usize>> {
		poll_write_buf(Pin::new(&mut self.0), cx, &mut buf)
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.0).poll_flush(cx)
	}

	fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.0).poll_shutdown(cx)
	}
}

// ResizeBufRead is like AsyncBufRead, but allows triggering a resize.
pub trait ResizeBufRead {
	fn poll_bytes(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<Bytes>>;
	fn resize(self: Pin<&mut Self>, new_size: usize);
}

#[derive(thiserror::Error, Debug)]
pub enum CopyError {
	#[error("failed to bind to address {0}: {1}")]
	Bind(SocketAddr, std::io::Error),

	#[error("io error: {0}")]
	Io(#[from] std::io::Error),

	#[error("while closing connection: {0}")]
	ShutdownError(Box<CopyError>),

	#[error("connection timed out, maybe a NetworkPolicy is blocking HBONE port 15008: {0}")]
	MaybeHBONENetworkPolicyError(std::io::Error),

	#[error("destination disconnected before all data was written")]
	BackendDisconnected,
	#[error("receive: {0}")]
	ReceiveError(Box<CopyError>),

	#[error("client disconnected before all data was written")]
	ClientDisconnected,
	#[error("send: {0}")]
	SendError(Box<CopyError>),

	#[error("no ip addresses were resolved for workload: {0}")]
	NoResolvedAddresses(String),

	#[error("requested service {0}:{1} found, but cannot resolve port")]
	NoPortForServices(String, u16),

	#[error("requested service {0} found, but has no IP addresses")]
	NoIPForService(String),

	#[error(
		"ip addresses were resolved for workload {0}, but valid dns response had no A/AAAA records"
	)]
	EmptyResolvedAddresses(String),

	#[error("attempted recursive call to ourselves")]
	SelfCall,
}

// Initially we create a 1k buffer for each connection. Note currently there are 3 buffers per connection.
// Outbound: downstream to app. Upstream HBONE is optimized to avoid.
// Inbound: downstream HBONE, upstream to app. Downstream HBONE can be optimized, but is not yet.
const INITIAL_BUFFER_SIZE: usize = 1024;
// We increase up to 16k for high traffic connections.
// TLS record size max is 16k. But we also have an H2 frame header, so leave a bit of room for that.
const LARGE_BUFFER_SIZE: usize = 16_384 - 64;
// For ultra-high bandwidth connections, increase up to 256Kb
const JUMBO_BUFFER_SIZE: usize = (16 * 16_384) - 64;
// After 128k of data we will trigger a resize from INITIAL to LARGE
// Loosely inspired by https://github.com/golang/go/blame/5122a6796ef98e3453c994c95abd640596540bea/src/crypto/tls/conn.go#L873
const RESIZE_THRESHOLD_LARGE: u64 = 128 * 1024;
// After 10Mb of data we will trigger a resize from LARGE to JUMBO
const RESIZE_THRESHOLD_JUMBO: u64 = 10 * 1024 * 1024;

/// Byte counts of a `copy_bidirectional` call: `sent` is copied from upstream to downstream,
/// `recv` from downstream to upstream.
#[derive(Debug, Default)]
pub struct ConnectionResult {
	sent: AtomicU64,
	recv: AtomicU64,
}

impl ConnectionResult {
	pub fn increment_recv(&self, amt: u64) {
		self.recv.fetch_add(amt, Ordering::Relaxed);
	}
	pub fn increment_send(&self, amt: u64) {
		self.sent.fetch_add(amt, Ordering::Relaxed);
	}
	pub fn bytes_sent(&self) -> u64 {
		self.sent.load(Ordering::Relaxed)
	}
	pub fn bytes_recv(&self) -> u64 {
		self.recv.load(Ordering::Relaxed)
	}
}

pub async fn copy_bidirectional<A, B>(
	downstream: A,
	upstream: B,
	stats: &ConnectionResult,
) -> Result<(), CopyError>
where
	A: BufferedSplitter,
	B: BufferedSplitter,
{
	let (mut rd, mut wd) = downstream.split_into_buffered_reader();
	let (mut ru, mut wu) = upstream.split_into_buffered_reader();
	let downstream_to_upstream = async {
		let translate_error = |e: io::Error| {
			CopyError::SendError(Box::new(match e.kind() {
				io::ErrorKind::NotConnected => CopyError::BackendDisconnected,
				io::ErrorKind::WriteZero => CopyError::BackendDisconnected,
				io::ErrorKind::UnexpectedEof => CopyError::ClientDisconnected,
				_ => e.into(),
			}))
		};
		let res =
			ignore_io_errors(copy_buf(&mut rd, &mut wu, stats, false).await).map_err(translate_error);
		trace!(?res, "send");
		ignore_shutdown_errors(shutdown(&mut wu).await)
			.map_err(translate_error)
			.map_err(|e| CopyError::ShutdownError(Box::new(e)))?;
		res
	};

	let upstream_to_downstream = async {
		let translate_error = |e: io::Error| {
			CopyError::ReceiveError(Box::new(match e.kind() {
				io::ErrorKind::NotConnected => CopyError::ClientDisconnected,
				io::ErrorKind::WriteZero => CopyError::ClientDisconnected,
				_ => e.into(),
			}))
		};
		let res =
			ignore_io_errors(copy_buf(&mut ru, &mut wd, stats, true).await).map_err(translate_error);
		trace!(?res, "receive");
		ignore_shutdown_errors(shutdown(&mut wd).await)
			.map_err(translate_error)
			.map_err(|e| CopyError::ShutdownError(Box::new(e)))?;
		res
	};

	// join!() them rather than try_join!() so that we keep complete either end once one side is complete.
	let (sent, received) = tokio::join!(downstream_to_upstream, upstream_to_downstream);

	// Convert some error messages to easier to understand
	let sent = sent?;
	let received = received?;
	trace!(sent, received, "copy complete");
	Ok(())
}

// During copying, we may encounter errors from either side closing their connection. Typically, we
// get a fully graceful shutdown with no errors on either end, but can if one end sends a RST directly,
// or if we have other non-graceful behavior, we may see errors. This is generally ok - a TCP connection
// can close at any time, really. Avoid reporting these as errors, as generally users expect errors to
// occur only when we cannot connect to the backend at all.
fn ignore_io_errors<T: Default>(res: Result<T, io::Error>) -> Result<T, io::Error> {
	use io::ErrorKind::*;
	match &res {
		Err(e) => match e.kind() {
			NotConnected | UnexpectedEof | ConnectionReset | BrokenPipe => {
				trace!(err=%e, "io terminated ungracefully");
				// Returning Default here is very hacky, but the data we are returning isn't critical so its no so bad to lose it.
				// Changing this would require refactoring all the interfaces to always return the bytes written even on error.
				Ok(Default::default())
			},
			_ => res,
		},
		_ => res,
	}
}

// During shutdown, the other end may have already disconnected. That is fine, they shutdown for us.
// Ignore it.
fn ignore_shutdown_errors(res: Result<(), io::Error>) -> Result<(), io::Error> {
	match &res {
		Err(e)
			if e.kind() == io::ErrorKind::NotConnected || e.kind() == io::ErrorKind::UnexpectedEof =>
		{
			trace!(err=%e, "failed to shutdown peer, they already shutdown");
			Ok(())
		},
		_ => res,
	}
}

// CopyBuf is a fork of Tokio's same struct, with additional support for resizing and metrics reporting.
#[must_use = "futures do nothing unless you `.await` or poll them"]
struct CopyBuf<'a, R: ?Sized, W: ?Sized> {
	send: bool,
	reader: &'a mut R,
	writer: &'a mut W,
	buf: Option<Bytes>,
	metrics: &'a ConnectionResult,
	amt: u64,
}

async fn copy_buf<'a, R, W>(
	reader: &'a mut R,
	writer: &'a mut W,
	metrics: &ConnectionResult,
	is_send: bool,
) -> std::io::Result<u64>
where
	R: ResizeBufRead + Unpin + ?Sized,
	W: AsyncWriteBuf + Unpin + ?Sized,
{
	CopyBuf {
		send: is_send,
		reader,
		writer,
		buf: None,
		metrics,
		amt: 0,
	}
	.await
}

impl<R, W> Future for CopyBuf<'_, R, W>
where
	R: ResizeBufRead + Unpin + ?Sized,
	W: AsyncWriteBuf + Unpin + ?Sized,
{
	type Output = std::io::Result<u64>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		loop {
			let me = &mut *self;

			// Get our stored buffer if there is any remaining, or fetch some more.
			let buffer = if let Some(buffer) = me.buf.take() {
				buffer
			} else {
				ready!(Pin::new(&mut *me.reader).poll_bytes(cx))?
			};
			if buffer.is_empty() {
				ready!(AsyncWriteBuf::poll_flush(Pin::new(&mut self.writer), cx))?;
				return Poll::Ready(Ok(self.amt));
			}

			// This is just a reference counter. Hold onto it in case the write() is not complete.
			let mut our_copy = buffer.clone();
			let i = match Pin::new(&mut *me.writer).poll_write_buf(cx, buffer) {
				Poll::Ready(written) => written?,
				Poll::Pending => {
					me.buf = Some(our_copy);
					return Poll::Pending;
				},
			};
			if i == 0 {
				return Poll::Ready(Err(std::io::ErrorKind::WriteZero.into()));
			}
			if i < our_copy.len() {
				// We only partially consumed it; store it back for a future call, skipping the number of bytes we did read.
				our_copy.advance(i);
				me.buf = Some(our_copy);
			}
			if me.send {
				me.metrics.increment_send(i as u64);
			} else {
				me.metrics.increment_recv(i as u64);
			}
			let old = self.amt;
			self.amt += i as u64;

			// If we were below the resize threshold before but are now above it, trigger the buffer to resize
			if old < RESIZE_THRESHOLD_LARGE && RESIZE_THRESHOLD_LARGE <= self.amt {
				Pin::new(&mut *self.reader).resize(LARGE_BUFFER_SIZE);
			}
			if old < RESIZE_THRESHOLD_JUMBO && RESIZE_THRESHOLD_JUMBO <= self.amt {
				Pin::new(&mut *self.reader).resize(JUMBO_BUFFER_SIZE);
			}
		}
	}
}

// BufReader is a fork of Tokio's type with resize support
pin_project! {
	pub struct BufReader<R> {
		#[pin]
		inner: R,
		buf: BytesMut,
		buffer_size: usize
	}
}

impl<R: AsyncRead> BufReader<R> {
	/// Creates a new `BufReader` with a default buffer capacity. The default is currently INITIAL_BUFFER_SIZE
	pub fn new(inner: R) -> Self {
		Self {
			inner,
			buf: BytesMut::with_capacity(INITIAL_BUFFER_SIZE),
			buffer_size: INITIAL_BUFFER_SIZE,
		}
	}
}

impl<R: AsyncRead> ResizeBufRead for BufReader<R> {
	fn poll_bytes(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<Bytes>> {
		let me = self.project();

		// Give us enough space to read a full chunk
		me.buf.reserve(*me.buffer_size);
		ready!(poll_read_buf(me.inner, cx, me.buf))?;
		Poll::Ready(Ok(me.buf.split().freeze()))
	}

	fn resize(self: Pin<&mut Self>, new_size: usize) {
		let me = self.project();
		*me.buffer_size = new_size;
	}
}

pin_project! {
	/// A future used to shutdown an I/O object.
	///
	/// Created by the [`AsyncWriteExt::shutdown`][shutdown] function.
	/// [shutdown]: [`crate::io::AsyncWriteExt::shutdown`]
	#[must_use = "futures do nothing unless you `.await` or poll them"]
	#[derive(Debug)]
	pub struct Shutdown<'a, A: ?Sized> {
		a: &'a mut A,
		// Make this future `!Unpin` for compatibility with async trait methods.
		#[pin]
		_pin: PhantomPinned,
	}
}

/// Creates a future which will shutdown an I/O object.
pub fn shutdown<A>(a: &mut A) -> Shutdown<'_, A>
where
	A: AsyncWriteBuf + Unpin + ?Sized,
{
	Shutdown {
		a,
		_pin: PhantomPinned,
	}
}

impl<A> Future for Shutdown<'_, A>
where
	A: AsyncWriteBuf + Unpin + ?Sized,
{
	type Output = std::io::Result<()>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let me = self.project();
		AsyncWriteBuf::poll_shutdown(Pin::new(me.a), cx)
	}
}

pub fn poll_write_buf<T: AsyncWrite + ?Sized, B: Buf>(
	io: Pin<&mut T>,
	cx: &mut Context<'_>,
	buf: &mut B,
) -> Poll<std::io::Result<usize>> {
	const MAX_BUFS: usize = 64;

	if !buf.has_remaining() {
		return Poll::Ready(Ok(0));
	}

	let n = if io.is_write_vectored() {
		let mut slices = [IoSlice::new(&[]); MAX_BUFS];
		let cnt = buf.chunks_vectored(&mut slices);
		ready!(io.poll_write_vectored(cx, &slices[..cnt]))?
	} else {
		ready!(io.poll_write(cx, buf.chunk()))?
	};

	buf.advance(n);

	Poll::Ready(Ok(n))
}

pub fn poll_read_buf<T: AsyncRead + ?Sized, B: BufMut>(
	io: Pin<&mut T>,
	cx: &mut Context<'_>,
	buf: &mut B,
) -> Poll<std::io::Result<usize>> {
	if !buf.has_remaining_mut() {
		return Poll::Ready(Ok(0));
	}

	let n = {
		let dst = buf.chunk_mut();

		// Safety: `chunk_mut()` returns a `&mut UninitSlice`, and `UninitSlice` is a
		// transparent wrapper around `[MaybeUninit<u8>]`.
		let dst = unsafe { &mut *(dst as *mut _ as *mut [MaybeUninit<u8>]) };
		let mut buf = ReadBuf::uninit(dst);
		let ptr = buf.filled().as_ptr();
		ready!(io.poll_read(cx, &mut buf)?);

		// Ensure the pointer does not change from under us
		assert_eq!(ptr, buf.filled().as_ptr());
		buf.filled().len()
	};

	// Safety: This is guaranteed to be the number of initialized (and read)
	// bytes due to the invariants provided by `ReadBuf::filled`.
	unsafe {
		buf.advance_mut(n);
	}

	Poll::Ready(Ok(n))
}

#[cfg(test)]
mod tests {
	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	use super::*;

	#[tokio::test]
	async fn copy_bidirectional_counts_both_directions() {
		let (mut client, downstream) = tokio::io::duplex(64);
		let (upstream, mut server) = tokio::io::duplex(64);
		let stats = ConnectionResult::default();

		let client_side = async {
			client.write_all(&[b'a'; 3000]).await.unwrap();
			client.shutdown().await.unwrap();
			let mut received = Vec::new();
			client.read_to_end(&mut received).await.unwrap();
			received.len()
		};
		let server_side = async {
			let mut received = Vec::new();
			server.read_to_end(&mut received).await.unwrap();
			server.write_all(&[b'b'; 5000]).await.unwrap();
			server.shutdown().await.unwrap();
			received.len()
		};
		let (copied, client_received, server_received) = tokio::join!(
			copy_bidirectional(downstream, upstream, &stats),
			client_side,
			server_side
		);

		copied.unwrap();
		assert_eq!(server_received, 3000);
		assert_eq!(client_received, 5000);
		assert_eq!(stats.bytes_recv(), 3000);
		assert_eq!(stats.bytes_sent(), 5000);
	}
}
