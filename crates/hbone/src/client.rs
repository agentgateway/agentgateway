use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::task::{Context, Poll};

use anyhow::anyhow;
use bytes::{Buf, Bytes};
use h2::SendStream;
use h2::client::{Connection, SendRequest};
use http::{HeaderMap, Request, StatusCode};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio::sync::watch::Receiver;
use tracing::{Instrument, debug, error, trace, warn};

use crate::Key;

/// A non-successful response to an HTTP/2 CONNECT request.
///
/// Callers that establish protocol-specific tunnels may need the response
/// headers to distinguish a retryable rejection from a generic failure.
#[derive(Debug)]
pub struct UnexpectedConnectResponse {
	pub status: StatusCode,
	pub headers: HeaderMap,
}

impl std::fmt::Display for UnexpectedConnectResponse {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "unexpected status: {}", self.status)
	}
}

impl std::error::Error for UnexpectedConnectResponse {}

#[derive(Debug, Clone)]
// H2ConnectClient is a wrapper abstracting h2
pub struct H2ConnectClient<K> {
	sender: SendRequest<Bytes>,
	pub max_allowed_streams: u16,
	stream_count: Arc<AtomicU16>,
	wl_key: K,
}

impl<K: Key> H2ConnectClient<K> {
	pub fn is_for_workload(&self, wl_key: &K) -> anyhow::Result<()> {
		if !(self.wl_key == *wl_key) {
			Err(anyhow::anyhow!("connection does not match workload key!"))
		} else {
			Ok(())
		}
	}

	// will_be_at_max_streamcount checks if a stream will be maxed out if we send one more request on it
	pub fn will_be_at_max_streamcount(&self) -> bool {
		let future_count = self.stream_count.load(Ordering::Relaxed) + 1;
		trace!(
			"checking streamcount: {future_count} >= {}",
			self.max_allowed_streams
		);
		future_count >= self.max_allowed_streams
	}

	pub fn ready_to_use(&mut self) -> bool {
		let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
		match self.sender.poll_ready(cx) {
			Poll::Ready(Ok(_)) => true,
			// We may have gotten GoAway, etc
			Poll::Ready(Err(_)) => false,
			Poll::Pending => {
				// Given our current usage, I am not sure this can ever be the case.
				// If it is, though, err on the safe side and do not use the connection
				warn!("checked out connection is Pending, skipping");
				false
			},
		}
	}

	pub async fn send_request(&mut self, req: http::Request<()>) -> anyhow::Result<crate::H2Stream> {
		let cur = self.stream_count.fetch_add(1, Ordering::SeqCst);
		trace!(current_streams = cur, "sending request");
		let (send, recv) = match self.internal_send(req).await {
			Ok(r) => r,
			Err(e) => {
				// Request failed, so drop the stream now
				self.stream_count.fetch_sub(1, Ordering::SeqCst);
				return Err(e);
			},
		};

		let (dropped1, dropped2) = crate::DropCounter::new(self.stream_count.clone());
		let read = crate::H2StreamReadHalf {
			recv_stream: recv,
			_dropped: dropped1,
		};
		let write = crate::H2StreamWriteHalf {
			send_stream: send,
			_dropped: dropped2,
		};
		let h2 = crate::H2Stream { read, write };
		Ok(h2)
	}

	// helper to allow us to handle errors once
	async fn internal_send(
		&mut self,
		req: Request<()>,
	) -> anyhow::Result<(SendStream<Bytes>, h2::RecvStream)> {
		// "This function must return `Ready` before `send_request` is called"
		// We should always be ready though, because we make sure we don't go over the max stream limit out of band.
		futures::future::poll_fn(|cx| self.sender.poll_ready(cx)).await?;
		let (response, stream) = self.sender.send_request(req, false)?;
		let response = response.await?;
		if response.status() != 200 {
			return Err(
				UnexpectedConnectResponse {
					status: response.status(),
					headers: response.headers().clone(),
				}
				.into(),
			);
		}
		Ok((stream, response.into_body()))
	}
}

pub async fn spawn_connection<K>(
	cfg: &crate::H2Config,
	s: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
	driver_drain: Receiver<bool>,
	wl_key: K,
) -> anyhow::Result<H2ConnectClient<K>> {
	let mut builder = h2::client::Builder::new();
	builder
		.initial_window_size(cfg.window_size)
		.initial_connection_window_size(cfg.connection_window_size)
		.max_frame_size(cfg.frame_size)
		.initial_max_send_streams(cfg.max_streams_per_conn as usize)
		.max_header_list_size(1024 * 16)
		// 4mb. Aligned with window_size such that we can fill up the buffer, then flush it all in one go, without buffering up too much.
		.max_send_buffer_size(cfg.window_size as usize)
		.enable_push(false);

	let (send_req, connection) = builder
		.handshake::<_, Bytes>(s)
		.await
		.map_err(|e| anyhow!("handshake failed: {}", e))?;

	// We store max as u16, so if they report above that max size we just cap at u16::MAX
	let max_allowed_streams = std::cmp::min(
		cfg.max_streams_per_conn,
		connection
			.max_concurrent_send_streams()
			.try_into()
			.unwrap_or(u16::MAX),
	);
	// spawn a task to poll the connection and drive the HTTP state
	// if we got a drain for that connection, respect it in a race
	// it is important to have a drain here, or this connection will never terminate
	tokio::spawn(
		async move {
			drive_connection(connection, driver_drain).await;
		}
		.in_current_span(),
	);

	let c = H2ConnectClient::<K> {
		sender: send_req,
		stream_count: Arc::new(AtomicU16::new(0)),
		max_allowed_streams,
		wl_key,
	};
	Ok(c)
}

async fn drive_connection<S, B>(mut conn: Connection<S, B>, mut driver_drain: Receiver<bool>)
where
	S: AsyncRead + AsyncWrite + Send + Unpin,
	B: Buf,
{
	let ping_pong = conn
		.ping_pong()
		.expect("ping_pong should only be called once");
	// for ping to inform this fn to drop the connection
	let (ping_drop_tx, ping_drop_rx) = oneshot::channel::<()>();
	// for this fn to inform ping to give up when it is already dropped
	let dropped = Arc::new(AtomicBool::new(false));
	tokio::task::spawn(
		super::do_ping_pong(ping_pong, ping_drop_tx, dropped.clone()).in_current_span(),
	);

	tokio::select! {
		_ = driver_drain.changed() => {
			debug!("draining outer HBONE connection");
		}
		_ = ping_drop_rx => {
			warn!("HBONE ping timeout/error");
		}
		res = conn => {
			match res {
				Err(e) => {
					error!("Error in HBONE connection handshake: {:?}", e);
				}
				Ok(_) => {
					debug!("done with HBONE connection handshake: {:?}", res);
				}
			}
		}
	}
	// Signal to the ping_pong it should also stop.
	dropped.store(true, Ordering::Relaxed);
}
