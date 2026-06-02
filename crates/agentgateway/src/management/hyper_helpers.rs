// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_core::drain::DrainWatcher;
use futures_util::{StreamExt, TryFutureExt};
use hyper::Request;
use hyper::server::conn::http1;
use hyper_util::rt::TokioTimer;
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tracing::info;

use crate::http::{Body, Response};
use crate::transport::stream::Socket;
use crate::types::frontend;

pub fn http1_server() -> http1::Builder {
	let mut b = http1::Builder::new();
	b.timer(TokioTimer::new());
	b
}

pub fn empty_response(code: hyper::StatusCode) -> Response {
	::http::Response::builder()
		.status(code)
		.body(Body::empty())
		.unwrap()
}

pub fn plaintext_response(code: hyper::StatusCode, body: String) -> Response {
	::http::Response::builder()
		.status(code)
		.header(hyper::header::CONTENT_TYPE, "text/plain")
		.body(body.into())
		.unwrap()
}

/// Server implements a generic HTTP server with the follow behavior:
/// * HTTP/1.1 plaintext only
/// * Draining
pub struct Server<S> {
	name: String,
	binds: Vec<Listener>,
	drain_rx: DrainWatcher,
	state: S,
	allow_proxy_protocol: bool,
}

enum Listener {
	Tcp(TcpListener),
	#[cfg(unix)]
	Unix {
		listener: UnixListener,
		path: PathBuf,
	},
}

impl Listener {
	fn tcp_addr(&self) -> Option<SocketAddr> {
		match self {
			Listener::Tcp(listener) => listener.local_addr().ok(),
			#[cfg(unix)]
			Listener::Unix { .. } => None,
		}
	}

	fn address(&self) -> String {
		match self {
			Listener::Tcp(listener) => listener
				.local_addr()
				.map(|addr| addr.to_string())
				.unwrap_or_else(|_| "<unknown>".to_string()),
			#[cfg(unix)]
			Listener::Unix { path, .. } => format!("unix:{}", path.display()),
		}
	}
}

#[cfg(unix)]
async fn bind_unix(path: PathBuf) -> anyhow::Result<Listener> {
	remove_stale_unix_socket(&path)?;
	let listener = UnixListener::bind(&path)?;
	Ok(Listener::Unix { listener, path })
}

#[cfg(not(unix))]
async fn bind_unix(path: PathBuf) -> anyhow::Result<Listener> {
	anyhow::bail!(
		"Unix domain sockets are not supported on this platform: {}",
		path.display()
	)
}

#[cfg(unix)]
fn remove_stale_unix_socket(path: &Path) -> anyhow::Result<()> {
	match std::fs::symlink_metadata(path) {
		Ok(metadata) if metadata.file_type().is_socket() => {
			std::fs::remove_file(path)?;
			Ok(())
		},
		Ok(_) => anyhow::bail!("refusing to remove non-socket file at {}", path.display()),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(err) => Err(err.into()),
	}
}

fn listener_stream(
	listener: Listener,
	allow_proxy_protocol: bool,
) -> futures_util::stream::BoxStream<'static, anyhow::Result<Socket>> {
	match listener {
		Listener::Tcp(listener) => tokio_stream::wrappers::TcpListenerStream::new(listener)
			.then(move |socket| async move {
				let socket = socket?;
				prepare_socket(socket, allow_proxy_protocol).await
			})
			.boxed(),
		#[cfg(unix)]
		Listener::Unix { listener, .. } => tokio_stream::wrappers::UnixListenerStream::new(listener)
			.map(|socket| Ok(Socket::from_unix(socket?)?))
			.boxed(),
	}
}

impl<S> Server<S> {
	pub async fn bind(
		name: &str,
		addrs: crate::Address,
		drain_rx: DrainWatcher,
		s: S,
	) -> anyhow::Result<Self> {
		let mut binds = vec![];
		match addrs {
			crate::Address::Off => {},
			crate::Address::UnixSocket(path) => {
				binds.push(bind_unix(path).await?);
			},
			addrs => {
				for addr in addrs.into_iter() {
					binds.push(Listener::Tcp(TcpListener::bind(&addr).await?))
				}
			},
		}
		Ok(Server {
			name: name.to_string(),
			binds,
			drain_rx,
			state: s,
			allow_proxy_protocol: false,
		})
	}

	pub fn with_optional_proxy_protocol(mut self) -> Self {
		self.allow_proxy_protocol = true;
		self
	}

	pub fn address(&self) -> Option<SocketAddr> {
		self.binds.first().and_then(Listener::tcp_addr)
	}

	pub fn state_mut(&mut self) -> &mut S {
		&mut self.state
	}

	pub fn spawn<F, R>(self, f: F)
	where
		S: Send + Sync + 'static,
		F: Fn(Arc<S>, Request<hyper::body::Incoming>) -> R + Send + Sync + 'static,
		R: Future<Output = Result<crate::http::Response, anyhow::Error>> + Send + 'static,
	{
		if self.binds.is_empty() {
			info!(component = self.name, "listener disabled");
			return;
		}
		let drain = self.drain_rx;
		let state = Arc::new(self.state);
		let f = Arc::new(f);
		let allow_proxy_protocol = self.allow_proxy_protocol;
		for bind in self.binds {
			let address = bind.address();
			info!(
					%address,
					component=self.name,
					"listener established",
			);
			let drain_stream = drain.clone();
			let drain_connections = drain.clone();
			let state = state.clone();
			let name = self.name.clone();
			let f = f.clone();
			tokio::spawn(async move {
				let stream = listener_stream(bind, allow_proxy_protocol);
				let mut stream = stream.take_until(Box::pin(drain_stream.wait_for_drain()));
				while let Some(socket) = stream.next().await {
					let socket = match socket {
						Ok(socket) => socket,
						Err(err) => {
							tracing::warn!(%err, "management connection setup failed");
							continue;
						},
					};
					let drain = drain_connections.clone();
					let f = f.clone();
					let state = state.clone();
					tokio::spawn(async move {
						let serve = http1_server()
							.half_close(true)
							.header_read_timeout(Duration::from_secs(2))
							.max_buf_size(8 * 1024)
							.serve_connection(
								hyper_util::rt::TokioIo::new(socket),
								hyper::service::service_fn(move |req| {
									let state = state.clone();

									// Failures would abort the whole connection; we just want to return an HTTP error
									f(state, req).or_else(|err| async move {
										Ok::<_, Infallible>(
											::http::Response::builder()
												.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
												.body(crate::http::Body::new(err.to_string()))
												.expect("builder with known status code should not fail"),
										)
									})
								}),
							);
						// Wait for drain to signal or connection serving to complete
						match futures_util::future::select(Box::pin(drain.wait_for_drain()), serve).await {
							// We got a shutdown request. Start gracful shutdown and wait for the pending requests to complete.
							futures_util::future::Either::Left((_shutdown, mut serve)) => {
								let drain = std::pin::Pin::new(&mut serve);
								drain.graceful_shutdown();
								serve.await
							},
							// Serving finished, just return the result.
							futures_util::future::Either::Right((serve, _shutdown)) => serve,
						}
					});
				}
				info!(
						%address,
						component=name,
						"listener drained",
				);
			});
		}
	}
}

async fn prepare_socket(
	socket: tokio::net::TcpStream,
	allow_proxy_protocol: bool,
) -> anyhow::Result<Socket> {
	let socket = Socket::from_tcp(socket)?;
	if !allow_proxy_protocol {
		return Ok(socket);
	}

	let (ext, metrics, inner) = socket.into_parts();
	let mut rewind = Socket::new_rewind(inner);
	let pp_info = tokio::time::timeout(
		Duration::from_secs(5),
		crate::proxy::proxy_protocol::detect_proxy_protocol(&mut rewind, frontend::ProxyVersion::All),
	)
	.await??;

	Ok(match pp_info {
		Some(pp_info) => Socket::from_rewind(ext, metrics, rewind.keep_after(pp_info.consumed_len)),
		None => {
			rewind.rewind();
			Socket::from_rewind(ext, metrics, rewind)
		},
	})
}
