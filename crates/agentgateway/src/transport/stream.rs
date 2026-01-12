use std::fmt::Display;
use std::io;
use std::io::{Error, IoSlice};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use agent_hbone::RWStream;
use hyper_util::client::legacy::connect::{Connected, Connection};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio_rustls::TlsStream;
use tracing::event;

use crate::telemetry::metrics::{Metrics as TelemetryMetrics, TCPLabels};
use crate::transport::rewind;
use crate::transport::rewind::RewindSocket;
use crate::types::frontend::TCP;

#[derive(Debug, Clone)]
pub struct TCPConnectionInfo {
	pub peer_addr: SocketAddr,
	pub local_addr: SocketAddr,
	pub start: Instant,
	/// Original TCP peer address before PROXY protocol parsing.
	/// For PROXY protocol connections, this is ztunnel's address (useful for debugging).
	/// For regular connections, this is None.
	pub raw_peer_addr: Option<SocketAddr>,
}

#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub enum Alpn {
	Http11,
	H2,
	Other,
}

impl From<&[u8]> for Alpn {
	fn from(value: &[u8]) -> Self {
		if value == b"h2" {
			Alpn::H2
		} else if value == b"http/1.1" {
			Alpn::Http11
		} else {
			Alpn::Other
		}
	}
}

#[derive(Default, Debug, Clone)]
pub struct TLSConnectionInfo {
	pub src_identity: Option<super::tls::TlsInfo>,
	pub server_name: Option<String>,
	pub negotiated_alpn: Option<Alpn>,
}

#[derive(Debug, Clone)]
pub struct HBONEConnectionInfo {
	pub hbone_address: SocketAddr,
}

#[derive(Debug, Default)]
pub struct Metrics {
	counter: Option<BytesCounter>,
	logging: LoggingMode,
	ctx: Option<TransportMetricsCtx>,
}

impl Metrics {
	fn with_counter() -> Metrics {
		Self {
			counter: Some(Default::default()),
			logging: LoggingMode::default(),
			ctx: None,
		}
	}
}

#[derive(Debug, Clone)]
pub struct TransportMetricsCtx {
	pub metrics: std::sync::Arc<TelemetryMetrics>,
	pub labels: TCPLabels,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum LoggingMode {
	#[default]
	None,
	Downstream,
	Upstream,
}

impl Display for LoggingMode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			LoggingMode::None => f.write_str("none"),
			LoggingMode::Downstream => f.write_str("downstream"),
			LoggingMode::Upstream => f.write_str("upstream"),
		}
	}
}

pub struct Socket {
	ext: Extension,
	inner: SocketType,
	metrics: Metrics,
}

impl Connection for Socket {
	fn connected(&self) -> Connected {
		Connected::new()
	}
}

impl hyper_util_fork::client::legacy::connect::Connection for Socket {
	fn connected(&self) -> hyper_util_fork::client::legacy::connect::Connected {
		let mut con = hyper_util_fork::client::legacy::connect::Connected::new();
		match self
			.ext
			.get::<TLSConnectionInfo>()
			.and_then(|c| c.negotiated_alpn)
		{
			Some(Alpn::H2) => con = con.negotiated_h2(),
			Some(Alpn::Http11) => con = con.negotiated_h1(),
			_ => {},
		}
		con
	}
}

impl Socket {
	pub fn new_rewind(st: SocketType) -> RewindSocket {
		RewindSocket::new(st)
	}
	pub fn into_parts(self) -> (Extension, Metrics, SocketType) {
		(self.ext, self.metrics, self.inner)
	}

	pub fn from_rewind(ext: Extension, metrics: Metrics, socket: RewindSocket) -> Socket {
		Self {
			ext,
			inner: SocketType::Rewind(Box::new(socket)),
			metrics,
		}
	}

	pub fn from_parts(ext: Extension, metrics: Metrics, socket: SocketType) -> Socket {
		Self {
			ext,
			inner: socket,
			metrics,
		}
	}

	pub fn set_transport_metrics(
		&mut self,
		metrics: std::sync::Arc<TelemetryMetrics>,
		labels: TCPLabels,
	) {
		self.metrics.ctx = Some(TransportMetricsCtx { metrics, labels });
	}

	pub fn from_memory(stream: DuplexStream, info: TCPConnectionInfo) -> Self {
		let mut ext = Extension::new();
		ext.insert(info);
		Socket {
			ext,
			inner: SocketType::Memory(stream),
			metrics: Metrics::with_counter(),
		}
	}

	pub fn from_tcp(stream: TcpStream) -> io::Result<Self> {
		let mut ext = Extension::new();
		stream.set_nodelay(true)?;
		ext.insert(TCPConnectionInfo {
			peer_addr: to_canonical(stream.peer_addr()?),
			local_addr: to_canonical(stream.local_addr()?),
			start: Instant::now(),
			raw_peer_addr: None,
		});
		Ok(Socket {
			ext,
			inner: SocketType::Tcp(stream),
			metrics: Metrics::with_counter(),
		})
	}

	pub fn from_tls(
		mut ext: Extension,
		metrics: Metrics,
		tls: TlsStream<Box<SocketType>>,
	) -> anyhow::Result<Self> {
		let info = {
			let server_name = match &tls {
				TlsStream::Server(s) => {
					let (_, ssl) = s.get_ref();
					ssl.server_name().map(|s| s.to_string())
				},
				_ => None,
			};
			let (_, ssl) = tls.get_ref();
			TLSConnectionInfo {
				src_identity: crate::transport::tls::identity_from_connection(ssl),
				negotiated_alpn: ssl.alpn_protocol().map(Alpn::from),
				server_name,
			}
		};
		ext.insert(info);
		Ok(Socket {
			ext,
			inner: SocketType::Tls(Box::new(tls)),
			metrics,
		})
	}

	pub fn from_hbone(ext: Arc<Extension>, hbone_address: SocketAddr, hbone: RWStream) -> Self {
		let mut ext = Extension::wrap(ext);
		ext.insert(HBONEConnectionInfo { hbone_address });

		Socket {
			ext,
			inner: SocketType::Hbone(hbone),
			metrics: Metrics::with_counter(),
		}
	}

	pub fn with_logging(&mut self, l: LoggingMode) {
		self.metrics.logging = l;
	}

	pub fn get_ext(&self) -> Extension {
		self.ext.clone()
	}

	pub fn ext<T: Send + Sync + 'static>(&self) -> Option<&T> {
		self.ext.get::<T>()
	}

	pub fn ext_mut(&mut self) -> &mut Extension {
		&mut self.ext
	}

	pub fn must_ext<T: Send + Sync + 'static>(&self) -> &T {
		self.ext().expect("expected required extension")
	}

	pub fn tcp(&self) -> &TCPConnectionInfo {
		self.ext.get::<TCPConnectionInfo>().unwrap()
	}
	/// target_address returns the HBONE destination or the L4 destination
	pub fn target_address(&self) -> SocketAddr {
		if let Some(hci) = self.ext.get::<HBONEConnectionInfo>() {
			hci.hbone_address
		} else {
			self.tcp().local_addr
		}
	}

	pub async fn dial(target: SocketAddr, cfg: &crate::BackendConfig) -> io::Result<Socket> {
		let res = tokio::time::timeout(cfg.connect_timeout, TcpStream::connect(target))
			.await
			.map_err(|to| io::Error::new(io::ErrorKind::TimedOut, to))??;
		if cfg.keepalives.enabled {
			let ka = socket2::TcpKeepalive::new()
				.with_time(cfg.keepalives.time)
				.with_retries(cfg.keepalives.retries)
				.with_interval(cfg.keepalives.interval);
			let res = socket2::SockRef::from(&res).set_tcp_keepalive(&ka);
			tracing::trace!("set keepalive: {:?}", res);
		}
		Socket::from_tcp(res)
	}

	/// Create a Socket from a Unix domain socket stream
	#[cfg(unix)]
	pub fn from_unix(stream: UnixStream) -> io::Result<Self> {
		let ext = Extension::new();
		Ok(Socket {
			ext,
			inner: SocketType::Unix(stream),
			metrics: Metrics::with_counter(),
		})
	}

	/// Dial a Unix domain socket
	#[cfg(unix)]
	pub async fn dial_unix(path: &std::path::Path, cfg: &crate::BackendConfig) -> io::Result<Socket> {
		let res = tokio::time::timeout(cfg.connect_timeout, UnixStream::connect(path))
			.await
			.map_err(|to| io::Error::new(io::ErrorKind::TimedOut, to))??;
		Socket::from_unix(res)
	}
	#[cfg(not(unix))]
	pub async fn dial_unix(
		_path: &std::path::Path,
		_cfg: &crate::BackendConfig,
	) -> io::Result<Socket> {
		Err(io::Error::new(
			io::ErrorKind::Unsupported,
			"UDS is not supported on windows",
		))
	}

	pub fn apply_tcp_settings(&mut self, settings: &TCP) {
		if let SocketType::Tcp(tcp) = &self.inner
			&& settings.keepalives.enabled
		{
			let ka = socket2::TcpKeepalive::new()
				.with_time(settings.keepalives.time)
				.with_retries(settings.keepalives.retries)
				.with_interval(settings.keepalives.interval);
			let res = socket2::SockRef::from(tcp).set_tcp_keepalive(&ka);
			tracing::trace!("set keepalive: {:?}", res);
		}
	}
}

pub enum SocketType {
	Tcp(TcpStream),
	#[cfg(unix)]
	Unix(UnixStream),
	Rewind(Box<rewind::RewindSocket>),
	Tls(Box<TlsStream<Box<SocketType>>>),
	Hbone(RWStream),
	Memory(DuplexStream),
	Boxed(Box<SocketType>),
}

impl AsyncRead for SocketType {
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		match self.get_mut() {
			SocketType::Tcp(inner) => Pin::new(inner).poll_read(cx, buf),
			#[cfg(unix)]
			SocketType::Unix(inner) => Pin::new(inner).poll_read(cx, buf),
			SocketType::Rewind(inner) => Pin::new(inner).poll_read(cx, buf),
			SocketType::Tls(inner) => Pin::new(inner).poll_read(cx, buf),
			SocketType::Hbone(inner) => Pin::new(inner).poll_read(cx, buf),
			SocketType::Memory(inner) => Pin::new(inner).poll_read(cx, buf),
			SocketType::Boxed(inner) => Pin::new(inner).poll_read(cx, buf),
		}
	}
}
impl AsyncWrite for SocketType {
	fn poll_write(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, std::io::Error>> {
		match self.get_mut() {
			SocketType::Tcp(inner) => Pin::new(inner).poll_write(cx, buf),
			#[cfg(unix)]
			SocketType::Unix(inner) => Pin::new(inner).poll_write(cx, buf),
			SocketType::Rewind(inner) => Pin::new(inner).poll_write(cx, buf),
			SocketType::Tls(inner) => Pin::new(inner).poll_write(cx, buf),
			SocketType::Hbone(inner) => Pin::new(inner).poll_write(cx, buf),
			SocketType::Memory(inner) => Pin::new(inner).poll_write(cx, buf),
			SocketType::Boxed(inner) => Pin::new(inner).poll_write(cx, buf),
		}
	}

	fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
		match self.get_mut() {
			SocketType::Tcp(inner) => Pin::new(inner).poll_flush(cx),
			#[cfg(unix)]
			SocketType::Unix(inner) => Pin::new(inner).poll_flush(cx),
			SocketType::Rewind(inner) => Pin::new(inner).poll_flush(cx),
			SocketType::Tls(inner) => Pin::new(inner).poll_flush(cx),
			SocketType::Hbone(inner) => Pin::new(inner).poll_flush(cx),
			SocketType::Memory(inner) => Pin::new(inner).poll_flush(cx),
			SocketType::Boxed(inner) => Pin::new(inner).poll_flush(cx),
		}
	}

	fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
		match self.get_mut() {
			SocketType::Tcp(inner) => Pin::new(inner).poll_shutdown(cx),
			#[cfg(unix)]
			SocketType::Unix(inner) => Pin::new(inner).poll_shutdown(cx),
			SocketType::Rewind(inner) => Pin::new(inner).poll_shutdown(cx),
			SocketType::Tls(inner) => Pin::new(inner).poll_shutdown(cx),
			SocketType::Hbone(inner) => Pin::new(inner).poll_shutdown(cx),
			SocketType::Memory(inner) => Pin::new(inner).poll_shutdown(cx),
			SocketType::Boxed(inner) => Pin::new(inner).poll_shutdown(cx),
		}
	}

	fn poll_write_vectored(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		bufs: &[IoSlice<'_>],
	) -> Poll<Result<usize, std::io::Error>> {
		match self.get_mut() {
			SocketType::Tcp(inner) => Pin::new(inner).poll_write_vectored(cx, bufs),
			#[cfg(unix)]
			SocketType::Unix(inner) => Pin::new(inner).poll_write_vectored(cx, bufs),
			SocketType::Rewind(inner) => Pin::new(inner).poll_write_vectored(cx, bufs),
			SocketType::Tls(inner) => Pin::new(inner).poll_write_vectored(cx, bufs),
			SocketType::Hbone(inner) => Pin::new(inner).poll_write_vectored(cx, bufs),
			SocketType::Memory(inner) => Pin::new(inner).poll_write_vectored(cx, bufs),
			SocketType::Boxed(inner) => Pin::new(inner).poll_write_vectored(cx, bufs),
		}
	}

	fn is_write_vectored(&self) -> bool {
		match &self {
			SocketType::Tcp(inner) => inner.is_write_vectored(),
			#[cfg(unix)]
			SocketType::Unix(inner) => inner.is_write_vectored(),
			SocketType::Rewind(inner) => inner.is_write_vectored(),
			SocketType::Tls(inner) => inner.is_write_vectored(),
			SocketType::Hbone(inner) => inner.is_write_vectored(),
			SocketType::Memory(inner) => inner.is_write_vectored(),
			SocketType::Boxed(inner) => inner.is_write_vectored(),
		}
	}
}

impl AsyncRead for Socket {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		let bytes = buf.filled().len();
		let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
		let bytes = buf.filled().len() - bytes;
		if let Some(c) = &mut self.metrics.counter {
			c.recv(bytes);
		}
		poll
	}
}
impl AsyncWrite for Socket {
	fn poll_write(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, Error>> {
		let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
		if let Some(c) = &mut self.metrics.counter
			&& let Poll::Ready(Ok(bytes)) = poll
		{
			c.sent(bytes);
		};
		poll
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.inner).poll_flush(cx)
	}

	fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.inner).poll_shutdown(cx)
	}

	fn poll_write_vectored(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		bufs: &[IoSlice<'_>],
	) -> Poll<Result<usize, Error>> {
		let poll = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
		if let Some(c) = &mut self.metrics.counter
			&& let Poll::Ready(Ok(bytes)) = poll
		{
			c.sent(bytes);
		};
		poll
	}

	fn is_write_vectored(&self) -> bool {
		self.inner.is_write_vectored()
	}
}

#[derive(Debug, Clone)]
pub enum Extension {
	Single(http::Extensions),
	Wrapped(http::Extensions, Arc<Extension>),
}

impl Default for Extension {
	fn default() -> Self {
		Self::new()
	}
}

impl Extension {
	pub fn new() -> Self {
		Extension::Single(http::Extensions::new())
	}
	fn wrap(ext: Arc<Extension>) -> Self {
		Extension::Wrapped(http::Extensions::new(), ext)
	}

	pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, val: T) -> Option<T> {
		match self {
			Extension::Single(extensions) => extensions.insert(val),
			Extension::Wrapped(extensions, _) => extensions.insert(val),
		}
	}

	pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
		match self {
			Extension::Single(extensions) => extensions.get::<T>(),
			Extension::Wrapped(extensions, inner) => {
				if let Some(got) = extensions.get::<T>() {
					Some(got)
				} else {
					inner.get::<T>()
				}
			},
		}
	}

	pub fn copy<T: Send + Clone + Sync + 'static>(&self, ext: &mut http::Extensions) {
		if let Some(got) = self.get::<T>() {
			ext.insert(got.clone());
		}
	}
}

fn to_canonical(addr: SocketAddr) -> SocketAddr {
	// another match has to be used for IPv4 and IPv6 support
	let ip = addr.ip().to_canonical();
	SocketAddr::from((ip, addr.port()))
}

#[derive(Default, Debug)]
pub struct BytesCounter {
	counts: (usize, usize),
}

impl BytesCounter {
	pub fn sent(&mut self, amt: usize) {
		self.counts.0 += amt;
	}
	pub fn recv(&mut self, amt: usize) {
		self.counts.1 += amt
	}
	pub fn load(&self) -> (u64, u64) {
		(self.counts.0 as u64, self.counts.1 as u64)
	}
}

impl Drop for Metrics {
	fn drop(&mut self) {
		if self.logging == LoggingMode::None {
			return;
		}
		// Export counters if a metrics context is present
		let counts = self.counter.take().map(|c| c.load());
		if let Some(ctx) = &self.ctx
			&& let Some((tx, rx)) = counts
		{
			ctx
				.metrics
				.tcp_downstream_tx_bytes
				.get_or_create(&ctx.labels)
				.inc_by(tx);
			ctx
				.metrics
				.tcp_downstream_rx_bytes
				.get_or_create(&ctx.labels)
				.inc_by(rx);
		}
		let (sent, recv) = if let Some((a, b)) = counts {
			(Some(a), Some(b))
		} else {
			(None, None)
		};
		match self.logging {
			LoggingMode::None => {},
			LoggingMode::Upstream => {
				event!(
					target: "upstream connection",
					parent: None,
					tracing::Level::DEBUG,

					sent,
					recv,

					"closed"
				);
			},
			LoggingMode::Downstream => {
				event!(
					target: "downstream connection",
					parent: None,
					tracing::Level::DEBUG,

					sent,
					recv,

					"closed"
				);
			},
		}
	}
}
