use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ::http::request::Parts;
use agent_core::drain::DrainWatcher;
use bytes::Bytes;
use futures_util::FutureExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, watch};
use tracing::{Instrument, debug};

use crate::*;

pub struct H2Request {
	request: Parts,
	recv: ::h2::RecvStream,
	send: ::h2::server::SendResponse<Bytes>,
}

impl Debug for H2Request {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("H2Request")
			.field("request", &self.request)
			.finish()
	}
}

impl H2Request {
	/// The request's URI
	pub fn uri(&self) -> &http::Uri {
		&self.request.uri
	}
	pub fn send_error(mut self, resp: ::http::Response<()>) -> anyhow::Result<()> {
		let _ = self.send.send_response(resp, true)?;
		Ok(())
	}

	pub async fn send_response(self, resp: ::http::Response<()>) -> anyhow::Result<crate::H2Stream> {
		let H2Request { recv, mut send, .. } = self;
		let send = send.send_response(resp, false)?;
		let read = crate::H2StreamReadHalf {
			recv_stream: recv,
			_dropped: None, // We do not need to track on the server
		};
		let write = crate::H2StreamWriteHalf {
			send_stream: send,
			_dropped: None, // We do not need to track on the server
		};
		let h2 = crate::H2Stream { read, write };
		Ok(h2)
	}

	pub fn get_request(&self) -> &Parts {
		&self.request
	}

	pub fn headers(&self) -> &http::HeaderMap<http::HeaderValue> {
		self.request.headers()
	}
}

pub trait RequestParts {
	fn uri(&self) -> &http::Uri;
	fn method(&self) -> &http::Method;
	fn headers(&self) -> &http::HeaderMap<http::HeaderValue>;
}

impl RequestParts for Parts {
	fn uri(&self) -> &http::Uri {
		&self.uri
	}

	fn method(&self) -> &http::Method {
		&self.method
	}

	fn headers(&self) -> &http::HeaderMap<http::HeaderValue> {
		&self.headers
	}
}

pub async fn serve_connection<F, IO, Ctx, Fut>(
	cfg: Arc<Config>,
	s: IO,
	ctx: Ctx,
	drain: DrainWatcher,
	mut force_shutdown: watch::Receiver<()>,
	handler: F,
) -> anyhow::Result<()>
where
	Ctx: Clone,
	IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
	F: Fn(H2Request, Ctx, DrainWatcher) -> Fut,
	Fut: Future<Output = ()> + Send + 'static,
{
	let mut builder = ::h2::server::Builder::new();
	let mut conn = builder
		.initial_window_size(cfg.h2.window_size)
		.initial_connection_window_size(cfg.h2.connection_window_size)
		.max_frame_size(cfg.h2.frame_size)
		// 64KB max; default is 16MB driven from Golang's defaults
		// Since we know we are going to receive a bounded set of headers, more is overkill.
		.max_header_list_size(65536)
		// 400kb, default from hyper
		.max_send_buffer_size(1024 * 400)
		// default from hyper
		.max_concurrent_streams(200)
		.handshake(s)
		.await?;

	let ping_pong = conn
		.ping_pong()
		.expect("new connection should have ping_pong");
	// for ping to inform this fn to drop the connection
	let (ping_drop_tx, mut ping_drop_rx) = oneshot::channel::<()>();
	// for this fn to inform ping to give up when it is already dropped
	let dropped = Arc::new(AtomicBool::new(false));
	tokio::task::spawn(crate::do_ping_pong(
		ping_pong,
		ping_drop_tx,
		dropped.clone(),
	));
	let handler = |req, ext, drain| handler(req, ext, drain).map(|_| ());

	loop {
		let drain_send = drain.clone();
		let drain_shutdown = drain.clone();
		tokio::select! {
			request = conn.accept() => {
				let Some(request) = request else {
					// done!
					// Signal to the ping_pong it should also stop.
					dropped.store(true, Ordering::Relaxed);
					return Ok(());
				};
				let (request, send) = request?;
				let (request, recv) = request.into_parts();
				let req = H2Request {
					request,
					recv,
					send,
				};
				let handle = handler(req, ctx.clone(), drain_send);
				// Serve the stream in a new task
				tokio::task::spawn(handle.in_current_span());
			}
			_ = &mut ping_drop_rx => {
				// Ideally this would be a warning/error message. However, due to an issue during shutdown,
				// by the time pods with in-pod know to shut down, the network namespace is destroyed.
				// This blocks the ability to send a GOAWAY and gracefully shutdown.
				// See https://github.com/istio/ztunnel/issues/1191.
				debug!("HBONE ping timeout/error, peer may have shutdown");
				conn.abrupt_shutdown(h2::Reason::NO_ERROR);
				break
			}
			_shutdown = drain_shutdown.wait_for_drain() => {
				debug!("starting graceful drain...");
				// Drain the HBONE layer itself
				conn.graceful_shutdown();
				break;
			}
		}
	}
	// Signal to the ping_pong it should also stop.
	dropped.store(true, Ordering::Relaxed);
	let poll_closed = futures_util::future::poll_fn(move |cx| conn.poll_closed(cx));
	tokio::select! {
			_ = force_shutdown.changed() => {
					anyhow::bail!("drain timeout");
			}
			_ = poll_closed => {}
	}
	// Mark we are done with the connection
	drop(drain);
	Ok(())
}
