//! The legacy HTTP Client from 0.14.x
//!
//! This `Client` will eventually be deconstructed into more composable parts.
//! For now, to enable people to use hyper 1.0 quicker, this `Client` exists
//! in much the same way it did in hyper 0.14.

use std::error::Error as StdError;
use std::fmt;
use std::fmt::Debug;
use std::future::{Future, poll_fn};
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, ready, Context, Poll};
use std::time::Duration;
use axum_core::BoxError;
use futures_util::future::{FutureExt, TryFutureExt};
use http::uri::Scheme;
use hyper::body::{Body, Bytes, Frame, SizeHint};
use hyper::header::{HOST, HeaderValue};
use hyper::rt::Timer;
use hyper::{Method, Request, Response, Uri, Version};
use tracing::{debug, trace, warn};

use super::connect::{Alpn, Connect, Connected, Connection};
use super::pool::{
	self, CheckoutResult, ClientConnectError, H2Load, HttpConnection, Key, ReservedHttp1Connection,
	ReservedHttp2Connection,
};
use crate::common::{Exec, SyncWrapper, timer};

type BoxSendFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A Client to make outgoing HTTP requests.
///
/// `Client` is cheap to clone and cloning is the recommended way to share a `Client`. The
/// underlying connection pool will be reused.
pub struct Client<C, PK: Key> {
	connector: C,
	exec: Exec,
	h1_builder: hyper::client::conn::http1::Builder,
	h2_builder: hyper::client::conn::http2::Builder<Exec>,
	pool: pool::Pool<PK>,
}

/// Client errors
pub struct Error {
	kind: ErrorKind,
	source: Option<Box<dyn StdError + Send + Sync>>,
	connect_info: Option<Connected>,
}

#[derive(Debug)]
enum ErrorKind {
	Canceled,
	Connect,
	SendRequest,
	NoPoolKey,
	WaitCanceled,
}

macro_rules! e {
	($kind:ident) => {
		Error {
			kind: ErrorKind::$kind,
			source: None,
			connect_info: None,
		}
	};
	($kind:ident, $src:expr) => {
		Error {
			kind: ErrorKind::$kind,
			source: Some($src.into()),
			connect_info: None,
		}
	};
}

#[allow(clippy::large_enum_variant)]
enum TrySendError<B> {
	Retryable {
		error: Error,
		req: Request<B>,
		connection_reused: bool,
	},
	Nope(Error),
}

/// A `Future` that will resolve to an HTTP Response.
///
/// This is returned by `Client::request` (and `Client::get`).
#[must_use = "futures do nothing unless polled"]
pub struct ResponseFuture {
	#[allow(clippy::type_complexity)]
	inner: SyncWrapper<
		Pin<Box<dyn Future<Output = Result<Response<axum_core::body::Body>, Error>> + Send>>,
	>,
}

// ===== impl Client =====

impl<PK: Key> Client<(), PK> {
	/// Create a builder to configure a new `Client`.
	///
	/// # Example
	///
	/// ```ignore
	/// # #[cfg(feature = "tokio")]
	/// # fn run () {
	/// use std::time::Duration;
	/// use hyper_util::client::legacy::Client;
	/// use hyper_util::rt::{TokioExecutor, TokioTimer};
	///
	/// let client = Client::builder(TokioExecutor::new())
	///     .pool_timer(TokioTimer::new())
	///     .pool_idle_timeout(Duration::from_secs(30))
	///     .http2_only(true)
	///     .build_http();
	/// # let infer: Client<_, http_body_util::Full<bytes::Bytes>> = client;
	/// # drop(infer);
	/// # }
	/// # fn main() {}
	/// ```
	pub fn builder<E>(executor: E) -> Builder
	where
		E: hyper::rt::Executor<BoxSendFuture> + Send + Sync + Clone + 'static,
	{
		Builder::new(executor)
	}
}

impl<C, PK> Client<C, PK>
where
	C: Connect + Clone + Send + Sync + 'static,
	PK: pool::Key,
{
	/// Send a constructed `Request` using this `Client`.
	///
	/// # Example
	///
	/// ```ignore
	/// # #[cfg(feature = "tokio")]
	/// # fn run () {
	/// use hyper::{Method, Request};
	/// use hyper_util::client::legacy::Client;
	/// use http_body_util::Full;
	/// use hyper_util::rt::TokioExecutor;
	/// use bytes::Bytes;
	///
	/// let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
	///
	/// let req: Request<Full<Bytes>> = Request::builder()
	///     .method(Method::POST)
	///     .uri("http://httpbin.org/post")
	///     .body(Full::from("Hallo!"))
	///     .expect("request builder");
	///
	/// let future = client.request(req);
	/// # }
	/// # fn main() {}
	/// ```
	pub fn request(&self, req: Request<axum_core::body::Body>) -> ResponseFuture {
		ResponseFuture::new(self.clone().send_request(req))
	}

	async fn send_request(
		self,
		mut req: Request<axum_core::body::Body>,
	) -> Result<Response<axum_core::body::Body>, Error> {
		// We may change URI so clone it to keep the original
		let uri = req.uri().clone();

		loop {
			req = match self.try_send_request(req).await {
				Ok(resp) => return Ok(resp),
				Err(TrySendError::Nope(err)) => return Err(err),
				Err(TrySendError::Retryable {
					mut req,
					error,
					connection_reused,
				}) => {
					if !connection_reused {
						// if client disabled, don't retry
						// a fresh connection means we definitely can't retry
						return Err(error);
					}

					trace!(
						"unstarted request canceled, trying again (reason={:?})",
						error
					);
					*req.uri_mut() = uri.clone();
					req
				},
			}
		}
	}

	async fn try_send_request(
		&self,
		req: Request<axum_core::body::Body>,
	) -> Result<Response<axum_core::body::Body>, TrySendError<axum_core::body::Body>> {
		let (parts, body) = req.into_parts();

		let mut pooled = self
			.connection_for(&parts)
			.await
			// `connection_for` already retries checkout errors, so if
			// it returns an error, there's not much else to retry
			.map_err(TrySendError::Nope)?;
		let mut req = Request::from_parts(parts, body);

		if pooled.is_http1() {
			if req.version() == Version::HTTP_2 {
				// This means we negotiated down in ALPN
				*req.version_mut() = Version::HTTP_11;
				trace!("Connection is HTTP/1, but request was HTTP/2");
			}

			let uri = req.uri().clone();
			req.headers_mut().entry(HOST).or_insert_with(|| {
				let hostname = uri.host().expect("authority implies host");
				if let Some(port) = get_non_default_port(&uri) {
					let mut s = String::with_capacity(hostname.len() + port.as_str().len() + 1);
					s.push_str(hostname);
					s.push(':');
					s.push_str(port.as_str());
					HeaderValue::from_maybe_shared(hyper::body::Bytes::from(s))
				} else {
					HeaderValue::from_str(hostname)
				}
				.expect("uri host is valid header value")
			});

			// CONNECT always sends authority-form, so check it first...
			if req.method() == Method::CONNECT {
				authority_form(req.uri_mut());
			} else if pooled.conn_info().is_proxied {
				absolute_form(req.uri_mut());
			} else {
				origin_form(req.uri_mut());
			}
		} else if req.method() == Method::CONNECT && !pooled.is_http2() {
			authority_form(req.uri_mut());
		}

		let mut res = match pooled.try_send_request(req).await {
			Ok(res) => res,
			Err(mut err) => {
				return if let Some(req) = err.take_message() {
					Err(TrySendError::Retryable {
						connection_reused: pooled.is_reused() || pooled.is_http2(),
						error: e!(Canceled, err.into_error()).with_connect_info(pooled.conn_info().clone()),
						req,
					})
				} else {
					Err(TrySendError::Nope(
						e!(SendRequest, err.into_error()).with_connect_info(pooled.conn_info().clone()),
					))
				};
			},
		};

		// If the Connector included 'extra' info, add to Response...
		if let Some(extra) = &pooled.conn_info().extra {
			extra.set(res.extensions_mut());
		}

		let res = if pooled.is_http2() {
			let guard = pooled.into_guard();
			// Insert the guard into the response extensions... AND the body
			// This ensures that its not dropped until both are done.
			// Inserting into the response is a guard against the body being dropped before the stream is complete.
			// This is still possible to drop both the response and the body, but less likely; if it does occur,
			// the impact is that we attempt to send on a stream that is full, which will block until it has capacity.
			// res.extensions_mut().insert(guard.clone());
			res.map(|b| BodyLog::wrap(b, Some(guard)))
		} else {
			// when pooled is dropped, it will try to insert back into the
			// pool. To delay that, spawn a future that completes once the
			// sender is ready again.
			//
			// This *should* only be once the related `Connection` has polled
			// for a new request to start.
			let on_idle = poll_fn(move |cx| {
				let HttpConnection::Http1(h1) = pooled.deref_mut() else {
					panic!("asserted http1 above")
				};
				h1.tx.poll_ready(cx)
			})
			.map(move |_| ());

			self.exec.execute(on_idle);

			res.map(|b| BodyLog::wrap(b, None::<()>))
		};
		Ok(res)
	}

	async fn connection_for(&self, dst: &http::request::Parts) -> Result<pool::Pooled<PK>, Error> {
		loop {
			match self.one_connection_for(dst).await {
				Ok(pooled) => return Ok(pooled),
				Err(ClientConnectError::Normal(err)) => return Err(err),
				Err(ClientConnectError::CheckoutIsClosed(reason)) => {
					trace!(
						"unstarted request canceled, trying again (reason={:?})",
						reason,
					);
					continue;
				},
			};
		}
	}

	async fn one_connection_for(
		&self,
		dst: &http::request::Parts,
	) -> Result<pool::Pooled<PK>, ClientConnectError> {
		// Return a single connection if pooling is not enabled
		let Some(pool_key) = dst.extensions.get::<PK>() else {
			return Err(ClientConnectError::Normal(e!(NoPoolKey)));
		};

		let checkout_result = self.pool.checkout_or_register_waker(pool_key.clone());
		match checkout_result {
			CheckoutResult::Checkout(pooled) => {
				trace!(result = "checkout", "pooled request");
				Ok(pooled)
			},
			CheckoutResult::Wait(wait) => {
				// If we should connect, do so.
				// Note: we wait for any connection, not necesarily this one. This ensures fairness:
				// Request 1 may spin up Conn 1 while request 2 spins up Conn 2.
				// If conn 2 establishes first, request 1 will take wihile request 2 will take conn 1.
				if let Some(sc) = wait.should_connect {
					trace!(result = "connect", "pooled request");
					let client = self.clone();
					let ver = dst.version;
					let pk = pool_key.clone();
					self.exec.execute(async move {
						let res = client.connect_to(ver, pk).await;
						match res {
							Ok(hc) => client.pool.insert_new_connection(sc, hc),
							Err(err) => {
								client.pool.insert_new_connection_error(sc, err);
							},
						}
					});
				} else {
					trace!(result = "wait", "pooled request");
				}
				let Ok(conn) = wait.waiter.await else {
					// This should never happen
					return Err(ClientConnectError::Normal(e!(WaitCanceled)));
				};
				conn
			},
		}
	}

	async fn connect_to(&self, version: Version, pk: PK) -> Result<pool::HttpConnection, Error> {
		let executor = self.exec.clone();
		let is_ver_h2 = if version == Version::HTTP_2 {
			// Explicitly HTTP2
			true
		} else {
			// Auto
			false
		};
		let connector = self.connector.clone();
		// TODO: it would be nice to just pass pk directly, but the tower::Service makes this tricky to make
		// it generic.
		let mut ext = http::Extensions::new();
		ext.insert(pk);

		let io = connector
			.connect(super::connect::sealed::Internal, ext)
			.map_err(|src| e!(Connect, src))
			.await?;
		// TODO: true-up our estimate
		let connected = io.connected();
		let is_h2 = (is_ver_h2 && connected.alpn == Alpn::None) || connected.alpn == Alpn::H2;

		let cx = if is_h2 {
			let (mut tx, conn) = self.h2_builder.handshake(io).await.map_err(Error::tx)?;

			// TODO: this needs to actually check within the execution.
			// Currently we do not allow exceeding the expected capacity (though it can be less than)
			let expected = self.pool.settings.expected_http2_capacity;
			let cur_max = std::cmp::min(conn.current_max_send_streams(), expected);
			trace!("http2 handshake complete, spawning background dispatcher task");
			executor.execute(
				conn
					.map_err(|e| debug!("client connection error: {}", e))
					.map(|_| ()),
			);

			// Wait for 'conn' to ready up before we
			// declare this tx as usable
			tx.ready().await.map_err(Error::tx)?;
			pool::HttpConnection::Http2(ReservedHttp2Connection {
				info: connected,
				tx,
				// Important: we do NOT reserve the stream slot here yet; we are only establishing a connection
				// not attaching any requests to it yet.
				load: Arc::new(H2Load::new(cur_max)),
			})
		} else {
			// Perform the HTTP/1.1 handshake on the provided I/O stream.
			// Uses the h1_builder to establish a connection, returning a sender (tx) for requests
			// and a connection task (conn) that manages the connection lifecycle.
			let (mut tx, conn) = self
				.h1_builder
				.handshake(io)
				.await
				.map_err(crate::client::legacy::client::Error::tx)?;
			// This indicates the connection is established and ready for request processing.
			trace!("http1 handshake complete, spawning background dispatcher task");
			// Create a oneshot channel to communicate errors from the connection task.
			// err_tx sends errors from the connection task, and err_rx receives them
			// to correlate connection failures with request readiness errors.
			let (err_tx, err_rx) = tokio::sync::oneshot::channel();
			// Spawn the connection task in the background using the executor.
			// The task manages the HTTP/1.1 connection, including upgrades (e.g., WebSocket).
			// Errors are sent via err_tx to ensure they can be checked if the sender (tx) fails.
			executor.execute(
				conn
					.with_upgrades()
					.map_err(|e| {
						// Log the connection error at debug level for diagnostic purposes.
						debug!("client connection error: {:?}", e);
						// Log that the error is being sent to the error channel.
						trace!("sending connection error to error channel");
						// Send the error via the oneshot channel, ignoring send failures
						// (e.g., if the receiver is dropped, which is handled later).
						let _ = err_tx.send(e);
					})
					.map(|_| ()),
			);
			// Readiness indicates the sender (tx) can accept a request without blocking.
			trace!("waiting for connection to be ready");
			// Check if the sender is ready to accept a request.
			// This ensures the connection is fully established before proceeding.
			// aka:
			// Wait for 'conn' to ready up before we
			// declare this tx as usable
			match tx.ready().await {
				// If ready, the connection is usable for sending requests.
				Ok(_) => {
					// Log that the connection is ready for use.
					trace!("connection is ready");
					// Drop the error receiver, as it’s no longer needed since the sender is ready.
					// This prevents waiting for errors that won’t occur in a successful case.
					drop(err_rx);
					pool::HttpConnection::Http1(ReservedHttp1Connection {
						info: connected,
						tx,
					})
				},
				// If the sender fails with a closed channel error, check for a specific connection error.
				// This distinguishes between a vague ChannelClosed error and an actual connection failure.
				Err(e) if e.is_closed() => {
					// Log that the channel is closed, indicating a potential connection issue.
					trace!("connection channel closed, checking for connection error");
					// Check the oneshot channel for a specific error from the connection task.
					match err_rx.await {
						// If an error was received, it’s a specific connection failure.
						Ok(err) => {
							// Log the specific connection error for diagnostics.
							trace!("received connection error: {:?}", err);
							// Return the error wrapped in Error::tx to propagate it.
							return Err(Error::tx(err));
						},
						// If the error channel is closed, no specific error was sent.
						// Fall back to the vague ChannelClosed error.
						Err(_) => {
							// Log that the error channel is closed, indicating no specific error.
							trace!("error channel closed, returning the vague ChannelClosed error");
							// Return the original error wrapped in Error::tx.
							return Err(Error::tx(e));
						},
					}
				},
				// For other errors (e.g., timeout, I/O issues), propagate them directly.
				// These are not ChannelClosed errors and don’t require error channel checks.
				Err(e) => {
					// Log the specific readiness failure for diagnostics.
					trace!("connection readiness failed: {:?}", e);
					// Return the error wrapped in Error::tx to propagate it.
					return Err(Error::tx(e));
				},
			}
		};
		Ok(cx)
	}
}

impl<C: Clone, PK: pool::Key> Clone for Client<C, PK> {
	fn clone(&self) -> Client<C, PK> {
		Client {
			exec: self.exec.clone(),
			h1_builder: self.h1_builder.clone(),
			h2_builder: self.h2_builder.clone(),
			connector: self.connector.clone(),
			pool: self.pool.clone(),
		}
	}
}

impl<C, PK: Key> fmt::Debug for Client<C, PK> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Client").finish()
	}
}

// ===== impl ResponseFuture =====

impl ResponseFuture {
	fn new<F>(value: F) -> Self
	where
		F: Future<Output = Result<Response<axum_core::body::Body>, Error>> + Send + 'static,
	{
		Self {
			inner: SyncWrapper::new(Box::pin(value)),
		}
	}
}

impl fmt::Debug for ResponseFuture {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.pad("Future<Response>")
	}
}

impl Future for ResponseFuture {
	type Output = Result<Response<axum_core::body::Body>, Error>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
		self.inner.get_mut().as_mut().poll(cx)
	}
}

fn origin_form(uri: &mut Uri) {
	let path = match uri.path_and_query() {
		Some(path) if path.as_str() != "/" => {
			let mut parts = ::http::uri::Parts::default();
			parts.path_and_query = Some(path.clone());
			Uri::from_parts(parts).expect("path is valid uri")
		},
		_none_or_just_slash => {
			debug_assert!(Uri::default() == "/");
			Uri::default()
		},
	};
	*uri = path
}

fn absolute_form(uri: &mut Uri) {
	debug_assert!(uri.scheme().is_some(), "absolute_form needs a scheme");
	debug_assert!(
		uri.authority().is_some(),
		"absolute_form needs an authority"
	);
	// If the URI is to HTTPS, and the connector claimed to be a agentgateway,
	// then it *should* have tunneled, and so we don't want to send
	// absolute-form in that case.
	if uri.scheme() == Some(&Scheme::HTTPS) {
		origin_form(uri);
	}
}

fn authority_form(uri: &mut Uri) {
	if let Some(path) = uri.path_and_query() {
		// `https://hyper.rs` would parse with `/` path, don't
		// annoy people about that...
		if path != "/" {
			warn!("HTTP/1.1 CONNECT request stripping path: {:?}", path);
		}
	}
	*uri = match uri.authority() {
		Some(auth) => {
			let mut parts = ::http::uri::Parts::default();
			parts.authority = Some(auth.clone());
			Uri::from_parts(parts).expect("authority is valid")
		},
		None => {
			unreachable!("authority_form with relative uri");
		},
	};
}

fn get_non_default_port(uri: &Uri) -> Option<http::uri::Port<&str>> {
	match (uri.port().map(|p| p.as_u16()), is_schema_secure(uri)) {
		(Some(443), true) => None,
		(Some(80), false) => None,
		_ => uri.port(),
	}
}

fn is_schema_secure(uri: &Uri) -> bool {
	uri
		.scheme_str()
		.map(|scheme_str| matches!(scheme_str, "wss" | "https"))
		.unwrap_or_default()
}


pin_project_lite::pin_project! {
	/// BodyLog wraps a body with logging on errors. These otherwise get masked by hyper.
	/// Additionally, it can keep-alive some data (T) to RAII
	#[must_use]
	#[derive(Debug)]
	struct BodyLog<B, T> {
		#[pin]
		body: B,
		keep_alive: Option<T>,
	}
}

impl<B, T> BodyLog<B, T> {
	pub fn wrap(body: B, keep_alive: Option<T>) -> axum_core::body::Body
	where
	T: Send + 'static,
	B: Body<Data = Bytes> + Unpin + Send + 'static,
	B::Error: Into<BoxError> + Debug,
	{
		axum_core::body::Body::new(BodyLog { body, keep_alive })
	}
}

impl<B, T> Body for BodyLog<B, T>
where
B: Body + Unpin,
<B as Body>::Error: std::fmt::Debug,
{
	type Data = B::Data;
	type Error = B::Error;

	#[inline]
	fn poll_frame(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let res = ready!(self.as_mut().project().body.poll_frame(cx));
		if let Some(Err(err)) = &res {
			debug!("warning: error from body stream: {err:?}")
		};
		Poll::Ready(res)
	}

	#[inline]
	fn is_end_stream(&self) -> bool {
		self.body.is_end_stream()
	}

	#[inline]
	fn size_hint(&self) -> SizeHint {
		self.body.size_hint()
	}
}


/// A builder to configure a new [`Client`](Client).
///
/// # Example
///
/// ```ignore
/// # #[cfg(feature = "tokio")]
/// # fn run () {
/// use std::time::Duration;
/// use hyper_util::client::legacy::Client;
/// use hyper_util::rt::TokioExecutor;
///
/// let client = Client::builder(TokioExecutor::new())
///     .pool_idle_timeout(Duration::from_secs(30))
///     .http2_only(true)
///     .build_http();
/// # let infer: Client<_, http_body_util::Full<bytes::Bytes>> = client;
/// # drop(infer);
/// # }
/// # fn main() {}
/// ```
#[derive(Clone)]
pub struct Builder {
	exec: Exec,
	h1_builder: hyper::client::conn::http1::Builder,
	h2_builder: hyper::client::conn::http2::Builder<Exec>,
	pool_config: pool::Config,
	pool_timer: Option<timer::Timer>,
}

impl Builder {
	/// Construct a new Builder.
	pub fn new<E>(executor: E) -> Self
	where
		E: hyper::rt::Executor<BoxSendFuture> + Send + Sync + Clone + 'static,
	{
		let exec = Exec::new(executor);
		Self {
			exec: exec.clone(),
			h1_builder: hyper::client::conn::http1::Builder::new(),
			h2_builder: hyper::client::conn::http2::Builder::new(exec),
			pool_config: pool::Config {
				idle_timeout: Some(Duration::from_secs(90)),
				max_idle_per_host: usize::MAX,
				expected_http2_capacity: pool::DEFAULT_EXPECTED_HTTP2_CAPACITY,
			},
			pool_timer: None,
		}
	}
	/// Set an optional timeout for idle sockets being kept-alive.
	/// A `Timer` is required for this to take effect. See `Builder::pool_timer`
	///
	/// Pass `None` to disable timeout.
	///
	/// Default is 90 seconds.
	///
	/// # Example
	///
	/// ```ignore
	/// # #[cfg(feature = "tokio")]
	/// # fn run () {
	/// use std::time::Duration;
	/// use hyper_util::client::legacy::Client;
	/// use hyper_util::rt::{TokioExecutor, TokioTimer};
	///
	/// let client = Client::builder(TokioExecutor::new())
	///     .pool_idle_timeout(Duration::from_secs(30))
	///     .pool_timer(TokioTimer::new())
	///     .build_http();
	///
	/// # let infer: Client<_, http_body_util::Full<bytes::Bytes>> = client;
	/// # }
	/// # fn main() {}
	/// ```
	pub fn pool_idle_timeout<D>(&mut self, val: D) -> &mut Self
	where
		D: Into<Option<Duration>>,
	{
		self.pool_config.idle_timeout = val.into();
		self
	}

	#[doc(hidden)]
	#[deprecated(note = "renamed to `pool_max_idle_per_host`")]
	pub fn max_idle_per_host(&mut self, max_idle: usize) -> &mut Self {
		self.pool_config.max_idle_per_host = max_idle;
		self
	}

	/// Sets the maximum idle connection per host allowed in the pool.
	///
	/// Default is `usize::MAX` (no limit).
	pub fn pool_max_idle_per_host(&mut self, max_idle: usize) -> &mut Self {
		self.pool_config.max_idle_per_host = max_idle;
		self
	}

	// HTTP/1 options

	/// Sets the exact size of the read buffer to *always* use.
	///
	/// Note that setting this option unsets the `http1_max_buf_size` option.
	///
	/// Default is an adaptive read buffer.
	pub fn http1_read_buf_exact_size(&mut self, sz: usize) -> &mut Self {
		self.h1_builder.read_buf_exact_size(Some(sz));
		self
	}

	/// Set the maximum buffer size for the connection.
	///
	/// Default is ~400kb.
	///
	/// Note that setting this option unsets the `http1_read_exact_buf_size` option.
	///
	/// # Panics
	///
	/// The minimum value allowed is 8192. This method panics if the passed `max` is less than the minimum.
	pub fn http1_max_buf_size(&mut self, max: usize) -> &mut Self {
		self.h1_builder.max_buf_size(max);
		self
	}

	/// Set whether HTTP/1 connections will accept spaces between header names
	/// and the colon that follow them in responses.
	///
	/// Newline codepoints (`\r` and `\n`) will be transformed to spaces when
	/// parsing.
	///
	/// You probably don't need this, here is what [RFC 7230 Section 3.2.4.] has
	/// to say about it:
	///
	/// > No whitespace is allowed between the header field-name and colon. In
	/// > the past, differences in the handling of such whitespace have led to
	/// > security vulnerabilities in request routing and response handling. A
	/// > server MUST reject any received request message that contains
	/// > whitespace between a header field-name and colon with a response code
	/// > of 400 (Bad Request). A agentgateway MUST remove any such whitespace from a
	/// > response message before forwarding the message downstream.
	///
	/// Note that this setting does not affect HTTP/2.
	///
	/// Default is false.
	///
	/// [RFC 7230 Section 3.2.4.]: https://tools.ietf.org/html/rfc7230#section-3.2.4
	pub fn http1_allow_spaces_after_header_name_in_responses(&mut self, val: bool) -> &mut Self {
		self
			.h1_builder
			.allow_spaces_after_header_name_in_responses(val);
		self
	}

	/// Set whether HTTP/1 connections will accept obsolete line folding for
	/// header values.
	///
	/// You probably don't need this, here is what [RFC 7230 Section 3.2.4.] has
	/// to say about it:
	///
	/// > A server that receives an obs-fold in a request message that is not
	/// > within a message/http container MUST either reject the message by
	/// > sending a 400 (Bad Request), preferably with a representation
	/// > explaining that obsolete line folding is unacceptable, or replace
	/// > each received obs-fold with one or more SP octets prior to
	/// > interpreting the field value or forwarding the message downstream.
	///
	/// > A agentgateway or gateway that receives an obs-fold in a response message
	/// > that is not within a message/http container MUST either discard the
	/// > message and replace it with a 502 (Bad Gateway) response, preferably
	/// > with a representation explaining that unacceptable line folding was
	/// > received, or replace each received obs-fold with one or more SP
	/// > octets prior to interpreting the field value or forwarding the
	/// > message downstream.
	///
	/// > A user agent that receives an obs-fold in a response message that is
	/// > not within a message/http container MUST replace each received
	/// > obs-fold with one or more SP octets prior to interpreting the field
	/// > value.
	///
	/// Note that this setting does not affect HTTP/2.
	///
	/// Default is false.
	///
	/// [RFC 7230 Section 3.2.4.]: https://tools.ietf.org/html/rfc7230#section-3.2.4
	pub fn http1_allow_obsolete_multiline_headers_in_responses(&mut self, val: bool) -> &mut Self {
		self
			.h1_builder
			.allow_obsolete_multiline_headers_in_responses(val);
		self
	}

	/// Sets whether invalid header lines should be silently ignored in HTTP/1 responses.
	///
	/// This mimics the behaviour of major browsers. You probably don't want this.
	/// You should only want this if you are implementing a agentgateway whose main
	/// purpose is to sit in front of browsers whose users access arbitrary content
	/// which may be malformed, and they expect everything that works without
	/// the agentgateway to keep working with the agentgateway.
	///
	/// This option will prevent Hyper's client from returning an error encountered
	/// when parsing a header, except if the error was caused by the character NUL
	/// (ASCII code 0), as Chrome specifically always reject those.
	///
	/// The ignorable errors are:
	/// * empty header names;
	/// * characters that are not allowed in header names, except for `\0` and `\r`;
	/// * when `allow_spaces_after_header_name_in_responses` is not enabled,
	///   spaces and tabs between the header name and the colon;
	/// * missing colon between header name and colon;
	/// * characters that are not allowed in header values except for `\0` and `\r`.
	///
	/// If an ignorable error is encountered, the parser tries to find the next
	/// line in the input to resume parsing the rest of the headers. An error
	/// will be emitted nonetheless if it finds `\0` or a lone `\r` while
	/// looking for the next line.
	pub fn http1_ignore_invalid_headers_in_responses(&mut self, val: bool) -> &mut Builder {
		self.h1_builder.ignore_invalid_headers_in_responses(val);
		self
	}

	/// Set whether HTTP/1 connections should try to use vectored writes,
	/// or always flatten into a single buffer.
	///
	/// Note that setting this to false may mean more copies of body data,
	/// but may also improve performance when an IO transport doesn't
	/// support vectored writes well, such as most TLS implementations.
	///
	/// Setting this to true will force hyper to use queued strategy
	/// which may eliminate unnecessary cloning on some TLS backends
	///
	/// Default is `auto`. In this mode hyper will try to guess which
	/// mode to use
	pub fn http1_writev(&mut self, enabled: bool) -> &mut Builder {
		self.h1_builder.writev(enabled);
		self
	}

	/// Set whether HTTP/1 connections will write header names as title case at
	/// the socket level.
	///
	/// Note that this setting does not affect HTTP/2.
	///
	/// Default is false.
	pub fn http1_title_case_headers(&mut self, val: bool) -> &mut Self {
		self.h1_builder.title_case_headers(val);
		self
	}

	/// Set whether to support preserving original header cases.
	///
	/// Currently, this will record the original cases received, and store them
	/// in a private extension on the `Response`. It will also look for and use
	/// such an extension in any provided `Request`.
	///
	/// Since the relevant extension is still private, there is no way to
	/// interact with the original cases. The only effect this can have now is
	/// to forward the cases in a agentgateway-like fashion.
	///
	/// Note that this setting does not affect HTTP/2.
	///
	/// Default is false.
	pub fn http1_preserve_header_case(&mut self, val: bool) -> &mut Self {
		self.h1_builder.preserve_header_case(val);
		self
	}

	/// Set the maximum number of headers.
	///
	/// When a response is received, the parser will reserve a buffer to store headers for optimal
	/// performance.
	///
	/// If client receives more headers than the buffer size, the error "message header too large"
	/// is returned.
	///
	/// The headers is allocated on the stack by default, which has higher performance. After
	/// setting this value, headers will be allocated in heap memory, that is, heap memory
	/// allocation will occur for each response, and there will be a performance drop of about 5%.
	///
	/// Note that this setting does not affect HTTP/2.
	///
	/// Default is 100.
	pub fn http1_max_headers(&mut self, val: usize) -> &mut Self {
		self.h1_builder.max_headers(val);
		self
	}

	/// Configures the maximum number of pending reset streams allowed before a GOAWAY will be sent.
	///
	/// This will default to the default value set by the [`h2` crate](https://crates.io/crates/h2).
	/// As of v0.4.0, it is 20.
	///
	/// See <https://github.com/hyperium/hyper/issues/2877> for more information.
	pub fn http2_max_pending_accept_reset_streams(
		&mut self,
		max: impl Into<Option<usize>>,
	) -> &mut Self {
		self.h2_builder.max_pending_accept_reset_streams(max.into());
		self
	}

	/// Sets the [`SETTINGS_INITIAL_WINDOW_SIZE`][spec] option for HTTP2
	/// stream-level flow control.
	///
	/// Passing `None` will do nothing.
	///
	/// If not set, hyper will use a default.
	///
	/// [spec]: https://http2.github.io/http2-spec/#SETTINGS_INITIAL_WINDOW_SIZE
	pub fn http2_initial_stream_window_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
		self.h2_builder.initial_stream_window_size(sz.into());
		self
	}

	/// Sets the max connection-level flow control for HTTP2
	///
	/// Passing `None` will do nothing.
	///
	/// If not set, hyper will use a default.
	pub fn http2_initial_connection_window_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
		self.h2_builder.initial_connection_window_size(sz.into());
		self
	}

	/// Sets the initial maximum of locally initiated (send) streams.
	///
	/// This value will be overwritten by the value included in the initial
	/// SETTINGS frame received from the peer as part of a [connection preface].
	///
	/// Passing `None` will do nothing.
	///
	/// If not set, hyper will use a default.
	///
	/// [connection preface]: https://httpwg.org/specs/rfc9113.html#preface
	pub fn http2_initial_max_send_streams(&mut self, initial: impl Into<Option<usize>>) -> &mut Self {
		self.h2_builder.initial_max_send_streams(initial);
		self
	}

	/// Sets whether to use an adaptive flow control.
	///
	/// Enabling this will override the limits set in
	/// `http2_initial_stream_window_size` and
	/// `http2_initial_connection_window_size`.
	pub fn http2_adaptive_window(&mut self, enabled: bool) -> &mut Self {
		self.h2_builder.adaptive_window(enabled);
		self
	}

	/// Sets the maximum frame size to use for HTTP2.
	///
	/// Passing `None` will do nothing.
	///
	/// If not set, hyper will use a default.
	pub fn http2_max_frame_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
		self.h2_builder.max_frame_size(sz);
		self
	}

	/// Sets the max size of received header frames for HTTP2.
	///
	/// Default is currently 16KB, but can change.
	pub fn http2_max_header_list_size(&mut self, max: u32) -> &mut Self {
		self.h2_builder.max_header_list_size(max);
		self
	}

	/// Sets an interval for HTTP2 Ping frames should be sent to keep a
	/// connection alive.
	///
	/// Pass `None` to disable HTTP2 keep-alive.
	///
	/// Default is currently disabled.
	pub fn http2_keep_alive_interval(&mut self, interval: impl Into<Option<Duration>>) -> &mut Self {
		self.h2_builder.keep_alive_interval(interval);
		self
	}

	/// Sets a timeout for receiving an acknowledgement of the keep-alive ping.
	///
	/// If the ping is not acknowledged within the timeout, the connection will
	/// be closed. Does nothing if `http2_keep_alive_interval` is disabled.
	///
	/// Default is 20 seconds.
	pub fn http2_keep_alive_timeout(&mut self, timeout: Duration) -> &mut Self {
		self.h2_builder.keep_alive_timeout(timeout);
		self
	}

	/// Sets whether HTTP2 keep-alive should apply while the connection is idle.
	///
	/// If disabled, keep-alive pings are only sent while there are open
	/// request/responses streams. If enabled, pings are also sent when no
	/// streams are active. Does nothing if `http2_keep_alive_interval` is
	/// disabled.
	///
	/// Default is `false`.
	pub fn http2_keep_alive_while_idle(&mut self, enabled: bool) -> &mut Self {
		self.h2_builder.keep_alive_while_idle(enabled);
		self
	}

	/// Sets the maximum number of HTTP2 concurrent locally reset streams.
	///
	/// See the documentation of [`h2::client::Builder::max_concurrent_reset_streams`] for more
	/// details.
	///
	/// The default value is determined by the `h2` crate.
	///
	/// [`h2::client::Builder::max_concurrent_reset_streams`]: https://docs.rs/h2/client/struct.Builder.html#method.max_concurrent_reset_streams
	pub fn http2_max_concurrent_reset_streams(&mut self, max: usize) -> &mut Self {
		self.h2_builder.max_concurrent_reset_streams(max);
		self
	}

	/// Provide a timer to be used for h2
	///
	/// See the documentation of [`h2::client::Builder::timer`] for more
	/// details.
	///
	/// [`h2::client::Builder::timer`]: https://docs.rs/h2/client/struct.Builder.html#method.timer
	pub fn timer<M>(&mut self, timer: M) -> &mut Self
	where
		M: Timer + Send + Sync + 'static,
	{
		self.h2_builder.timer(timer);
		self
	}

	/// Provide a timer to be used for timeouts and intervals in connection pools.
	pub fn pool_timer<M>(&mut self, timer: M) -> &mut Self
	where
		M: Timer + Clone + Send + Sync + 'static,
	{
		self.pool_timer = Some(timer::Timer::new(timer.clone()));
		self
	}

	/// Set the maximum write buffer size for each HTTP/2 stream.
	///
	/// Default is currently 1MB, but may change.
	///
	/// # Panics
	///
	/// The value must be no larger than `u32::MAX`.
	pub fn http2_max_send_buf_size(&mut self, max: usize) -> &mut Self {
		self.h2_builder.max_send_buf_size(max);
		self
	}

	/// Combine the configuration of this builder with a connector to create a `Client`, with a custom pooling key.
	/// A function to extract the pool key from the request is required.
	pub fn build<C, PK>(&self, connector: C) -> Client<C, PK>
	where
		C: Connect + Clone,
		PK: pool::Key,
	{
		let exec = self.exec.clone();
		let timer = self.pool_timer.clone().expect("pool_timer must be set");
		Client {
			exec: exec.clone(),
			h1_builder: self.h1_builder.clone(),
			h2_builder: self.h2_builder.clone(),
			connector,
			pool: pool::Pool::<PK>::new(self.pool_config, exec, timer),
		}
	}
}

impl fmt::Debug for Builder {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Builder")
			.field("pool_config", &self.pool_config)
			.finish()
	}
}

// ==== impl Error ====

impl fmt::Debug for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if let Some(ref cause) = self.source {
			if let Some(he) = cause.downcast_ref::<hyper::Error>() {
				if let Some(src) = he.source() {
					return write!(f, "{:?}: {}: {}", self.kind, cause, src);
				}
			}
			write!(f, "{:?}: {}", self.kind, cause)
		} else {
			write!(f, "{:?}", self.kind)
		}
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if let Some(ref cause) = self.source {
			write!(f, "{:?}: {}", self.kind, cause)
		} else {
			write!(f, "{:?}", self.kind)
		}
	}
}

impl StdError for Error {
	fn source(&self) -> Option<&(dyn StdError + 'static)> {
		self.source.as_ref().map(|e| &**e as _)
	}
}

impl Error {
	/// Returns the info of the client connection on which this error occurred.
	pub fn connect_info(&self) -> Option<&Connected> {
		self.connect_info.as_ref()
	}

	fn with_connect_info(self, connect_info: Connected) -> Self {
		Self {
			connect_info: Some(connect_info),
			..self
		}
	}

	fn tx(src: hyper::Error) -> Self {
		e!(SendRequest, src)
	}
}
