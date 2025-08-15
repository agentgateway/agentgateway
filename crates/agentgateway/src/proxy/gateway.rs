use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use agent_core::drain;
use agent_core::drain::{DrainUpgrader, DrainWatcher};
use anyhow::anyhow;
use bytes::Bytes;
use futures_util::FutureExt;
use http::StatusCode;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
#[cfg(target_family = "unix")]
use net2::unix::UnixTcpBuilderExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinSet};
use tokio_stream::StreamExt;
use tracing::{Instrument, debug, event, info, info_span, warn};

use crate::store::Event;
use crate::transport::stream::{Extension, LoggingMode, Socket};
use crate::types::agent::{Bind, BindName, BindProtocol, Listener, ListenerProtocol};
use crate::{ProxyInputs, client};

#[cfg(test)]
#[path = "gateway_test.rs"]
mod tests;

pub struct Gateway {
	pi: Arc<ProxyInputs>,
	drain: drain::DrainWatcher,
}

impl Gateway {
	pub fn new(pi: Arc<ProxyInputs>, drain: DrainWatcher) -> Gateway {
		Gateway { drain, pi }
	}

	pub async fn run(self) {
		let drain = self.drain.clone();
		let subdrain = self.drain.clone();
		let mut js = JoinSet::new();
		let (initial_binds, mut binds) = {
			let binds = self.pi.stores.read_binds();
			(binds.all(), binds.subscribe())
		};
		let mut active: HashMap<SocketAddr, AbortHandle> = HashMap::new();
		let mut handle_bind = |js: &mut JoinSet<anyhow::Result<()>>, b: Event<Arc<Bind>>| {
			let b = match b {
				Event::Add(b) => b,
				Event::Remove(to_remove) => {
					if let Some(h) = active.remove(&to_remove.address) {
						h.abort();
					}
					return;
				},
			};
			if active.contains_key(&b.address) {
				debug!("bind already exists");
				return;
			}

			debug!("add bind {}", b.address);
			if self.pi.cfg.threading_mode == crate::ThreadingMode::ThreadPerCore {
				let core_ids = core_affinity::get_core_ids().unwrap();
				let _ = core_ids
					.into_iter()
					.map(|id| {
						let subdrain = subdrain.clone();
						let pi = self.pi.clone();
						let b = b.clone();
						std::thread::spawn(move || {
							let res = core_affinity::set_for_current(id);
							if !res {
								panic!("failed to set current CPU")
							}
							tokio::runtime::Builder::new_current_thread()
								.enable_all()
								.build()
								.unwrap()
								.block_on(async {
									let _ = Self::run_bind(pi.clone(), subdrain.clone(), b.clone())
										.in_current_span()
										.await;
								})
						})
					})
					.collect::<Vec<_>>();
			} else {
				let task =
					js.spawn(Self::run_bind(self.pi.clone(), subdrain.clone(), b.clone()).in_current_span());
				active.insert(b.address, task);
			}
		};
		for bind in initial_binds {
			handle_bind(&mut js, Event::Add(bind))
		}

		let wait = drain.wait_for_drain();
		tokio::pin!(wait);
		loop {
			tokio::select! {
				Some(res) = binds.next() => {
					let Ok(res) = res else {
						// TODO: move to unbuffered
						warn!("lagged on bind update");
						continue;
					};
					handle_bind(&mut js, res);
				}
				Some(res) = js.join_next() => {
					warn!("bind complete {res:?}");
				}
				_ = &mut wait => {
					info!("stop listening for binds; drain started");
					while let Some(res) = js.join_next().await  {
						info!("bind complete {res:?}");
					}
					info!("binds drained");
					return
				}
			}
		}
	}

	pub(super) async fn run_bind(
		pi: Arc<ProxyInputs>,
		drain: DrainWatcher,
		b: Arc<Bind>,
	) -> anyhow::Result<()> {
		let min_deadline = pi.cfg.termination_min_deadline;
		let max_deadline = pi.cfg.termination_max_deadline;
		let name = b.key.clone();
		let (pi, listener) = if pi.cfg.threading_mode == crate::ThreadingMode::ThreadPerCore {
			let mut pi = Arc::unwrap_or_clone(pi);
			let client = client::Client::new(&pi.cfg.dns, None);
			pi.upstream = client;
			let pi = Arc::new(pi);
			let builder = if b.address.is_ipv4() {
				net2::TcpBuilder::new_v4()
			} else {
				net2::TcpBuilder::new_v6()
			};
			#[cfg(target_family = "unix")]
			let mut builder = builder?;
			#[cfg(target_family = "unix")]
			let mut builder = builder.reuse_port(true);
			let listener = builder?.bind(b.address)?.listen(1024)?;
			listener.set_nonblocking(true)?;
			let listener = tokio::net::TcpListener::from_std(listener)?;
			(pi, listener)
		} else {
			(pi, TcpListener::bind(b.address).await?)
		};
		info!(bind = name.as_str(), "started bind");
		let component = format!("bind {name}");

		// Desired drain semantics:
		// A drain will start when SIGTERM is sent.
		// On drain start, we will want to immediately start suggesting to clients to go away. This is done
		//  by sending a GOAWAY for HTTP2 and setting `connection: close` for HTTP1.
		// However, this is race-y. Clients will not know immediately to stop connecting, so we need to continue
		//  to serve new clients.
		// Therefor, we should have a minimum drain time and a maximum drain time.
		// No matter what, we will continue accepting connections for <min time>. Any new connections will
		// be "discouraged" via disabling keepalive.
		// After that, we will continue processing connections as long as there are any remaining open.
		// This handles gracefully serving any long-running requests.
		// New connections may still be made during this time which we will attempt to serve, though they
		// are at increased risk of early termination.
		let accept = |drain: DrainWatcher, force_shutdown: watch::Receiver<()>| async move {
			// We will need to be able to watch for drains, so take a copy
			let drain_watch = drain.clone();
			// Subtle but important: we need to be able to create drain-blockers for each accepted connection.
			// However, we don't want to block from our listen() loop, or we would never finish.
			// Having a weak reference allows us to listen() forever without blocking, but create blockers for accepted connections.
			let (mut upgrader, weak) = drain.into_weak();
			let (inner_trigger, inner_drain) = drain::new();
			drop(inner_drain);
			let handle_stream = |stream: TcpStream, upgrader: &DrainUpgrader| {
				let mut stream = Socket::from_tcp(stream).expect("todo");
				stream.with_logging(LoggingMode::Downstream);
				let pi = pi.clone();
				// We got the connection; make a strong drain blocker.
				let drain = upgrader.upgrade(weak.clone());
				let start = Instant::now();
				let mut force_shutdown = force_shutdown.clone();
				let name = name.clone();
				tokio::spawn(async move {
					debug!(bind=?name, "connection started");
					tokio::select! {
						// We took too long; shutdown now.
						_ = force_shutdown.changed() => {
							info!(bind=?name, "connection forcefully terminated");
						}
						_ = Self::proxy_bind(name.clone(), stream, pi, drain) => {}
					}
					debug!(bind=?name, dur=?start.elapsed(), "connection completed");
				});
			};
			let wait = drain_watch.wait_for_drain();
			tokio::pin!(wait);
			// First, accept new connections until a drain is triggered
			let drain_mode = loop {
				tokio::select! {
					Ok((stream, _peer)) = listener.accept() => handle_stream(stream, &upgrader),
					res = &mut wait => {
						break res;
					}
				}
			};
			upgrader.disable();
			// Now we are draining. We need to immediately start draining the inner requests
			// Wait for Min_duration complete AND inner join complete
			let mode = drain_mode.mode(); // TODO: handle mode differently?
			drop(drain_mode);
			let drained_for_minimum = async move {
				tokio::join!(
					inner_trigger.start_drain_and_wait(mode),
					tokio::time::sleep(min_deadline)
				);
			};
			tokio::pin!(drained_for_minimum);
			// We still need to accept new connections during this time though, so race them
			loop {
				tokio::select! {
					Ok((stream, _peer)) = listener.accept() => handle_stream(stream, &upgrader),
					_ = &mut drained_for_minimum => {
						// We are done! exit.
						// This will stop accepting new connections
						return;
					}
				}
			}
		};

		drain::run_with_drain(component, drain, max_deadline, accept).await;
		Ok(())
	}

	pub async fn proxy_bind(
		bind_name: BindName,
		raw_stream: Socket,
		inputs: Arc<ProxyInputs>,
		drain: DrainWatcher,
	) {
		let bind_protocol = bind_protocol(inputs.clone(), bind_name.clone());
		event!(
			target: "downstream connection",
			parent: None,
			tracing::Level::DEBUG,

			src.addr = %raw_stream.tcp().peer_addr,
			protocol = ?bind_protocol,

			"opened",
		);
		match bind_protocol {
			BindProtocol::http => {
				let err = Self::proxy(bind_name, inputs, None, raw_stream, drain).await;
				if let Err(e) = err {
					warn!("proxy error: {e}");
				}
			},
			BindProtocol::tcp => Self::proxy_tcp(bind_name, inputs, None, raw_stream, drain).await,
			BindProtocol::tls => {
				let Ok((selected_listener, stream)) =
					Self::terminate_tls(inputs.clone(), raw_stream, bind_name.clone()).await
				else {
					warn!("failed to terminate TLS");
					// TODO: log
					return;
				};
				Self::proxy_tcp(bind_name, inputs, Some(selected_listener), stream, drain).await
			},
			BindProtocol::https => {
				let (selected_listener, stream) =
					match Self::terminate_tls(inputs.clone(), raw_stream, bind_name.clone()).await {
						Ok(res) => res,
						Err(e) => {
							warn!("failed to terminate HTTPS: {e}");
							return;
						},
					};
				let _ = Self::proxy(bind_name, inputs, Some(selected_listener), stream, drain).await;
			},
			BindProtocol::hbone => {
				let _ = Self::terminate_hbone(bind_name, inputs, raw_stream, drain).await;
			},
		}
	}

	async fn proxy(
		bind_name: BindName,
		inputs: Arc<ProxyInputs>,
		selected_listener: Option<Arc<Listener>>,
		stream: Socket,
		drain: DrainWatcher,
	) -> anyhow::Result<()> {
		let target_address = stream.target_address();
		let proxy = super::httpproxy::HTTPProxy {
			bind_name,
			inputs,
			selected_listener,
			target_address,
		};
		let server = auto_server();
		let connection = Arc::new(stream.get_ext());
		let serve = server // TODO: tune all optinos
			.serve_connection_with_upgrades(
				TokioIo::new(stream),
				hyper::service::service_fn(move |req| {
					let proxy = proxy.clone();
					let connection = connection.clone();
					async move { proxy.proxy(connection, req).map(Ok::<_, Infallible>).await }
				}),
			);
		// Wrap it in the graceful watcher, will ensure GOAWAY/Connect:clone when we shutdown
		let serve = drain.wrap_connection(serve);
		let res = serve.await;
		match res {
			Ok(_) => Ok(()),
			Err(e) => {
				anyhow::bail!("{e}");
			},
		}
	}

	async fn proxy_tcp(
		bind_name: BindName,
		inputs: Arc<ProxyInputs>,
		selected_listener: Option<Arc<Listener>>,
		stream: Socket,
		_drain: DrainWatcher,
	) {
		let selected_listener = match selected_listener {
			Some(l) => l,
			None => {
				let listeners = inputs
					.stores
					.read_binds()
					.listeners(bind_name.clone())
					.unwrap();
				let Ok(selected_listener) = listeners.get_exactly_one() else {
					return;
				};
				selected_listener
			},
		};
		let target_address = stream.target_address();
		let proxy = super::tcpproxy::TCPProxy {
			bind_name,
			inputs,
			selected_listener,
			target_address,
		};
		proxy.proxy(stream).await
	}

	async fn terminate_tls(
		inp: Arc<ProxyInputs>,
		raw_stream: Socket,
		bind: BindName,
	) -> anyhow::Result<(Arc<Listener>, Socket)> {
		let listeners = inp.stores.read_binds().listeners(bind.clone()).unwrap();
		let (ext, counter, inner) = raw_stream.into_parts();
		let acceptor =
			tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), Box::new(inner));
		let start = acceptor.await?;
		let ch = start.client_hello();
		let best = listeners
			.best_match(ch.server_name().unwrap_or_default())
			.ok_or(anyhow!("no TLS listener match"))?;
		let cfg = best.protocol.tls().unwrap();
		let tls = start.into_stream(cfg).await?;
		Ok((best, Socket::from_tls(ext, counter, tls.into())?))
	}

	async fn terminate_hbone(
		bind_name: BindName,
		inp: Arc<ProxyInputs>,
		raw_stream: Socket,
		drain: DrainWatcher,
	) -> anyhow::Result<()> {
		let Some(ca) = inp.ca.as_ref() else {
			anyhow::bail!("CA is required for waypoint");
		};

		let cert = ca.get_identity().await?;
		let sc = Arc::new(cert.hbone_termination()?);
		let tls = crate::transport::tls::accept(raw_stream, sc).await?;

		debug!("accepted connection");
		let cfg = inp.cfg.clone();
		let request_handler = move |req, ext, graceful| {
			Self::serve_connect(bind_name.clone(), inp.clone(), req, ext, graceful)
				.instrument(info_span!("inbound"))
		};

		let (_, force_shutdown) = watch::channel(());
		let ext = Arc::new(tls.get_ext());
		let serve_conn = agent_hbone::server::serve_connection(
			cfg.hbone.clone(),
			tls,
			ext,
			drain,
			force_shutdown,
			request_handler,
		);
		serve_conn.await
	}
	/// serve_connect handles a single connection from a client.
	#[allow(clippy::too_many_arguments)]
	async fn serve_connect(
		bind_name: BindName,
		pi: Arc<ProxyInputs>,
		req: agent_hbone::server::H2Request,
		ext: Arc<Extension>,
		drain: DrainWatcher,
	) {
		debug!(?req, "received request");

		let hbone_addr = req
			.uri()
			.to_string()
			.as_str()
			.parse::<SocketAddr>()
			.map_err(|_| InboundError(anyhow::anyhow!("bad request"), StatusCode::BAD_REQUEST))
			.unwrap();
		let Ok(resp) = req.send_response(build_response(StatusCode::OK)).await else {
			warn!("failed to send response");
			return;
		};
		let con = agent_hbone::RWStream {
			stream: resp,
			buf: Bytes::new(),
		};

		let _ = Self::proxy(
			bind_name,
			pi,
			None,
			Socket::from_hbone(ext, hbone_addr, con),
			drain,
		)
		.await;
	}
}

fn bind_protocol(inp: Arc<ProxyInputs>, bind: BindName) -> BindProtocol {
	let listeners = inp.stores.read_binds().listeners(bind).unwrap();
	if listeners
		.iter()
		.any(|l| matches!(l.protocol, ListenerProtocol::HBONE))
	{
		return BindProtocol::hbone;
	}
	if listeners
		.iter()
		.any(|l| matches!(l.protocol, ListenerProtocol::HTTPS(_)))
	{
		return BindProtocol::https;
	}
	if listeners
		.iter()
		.any(|l| matches!(l.protocol, ListenerProtocol::TLS(_)))
	{
		return BindProtocol::tls;
	}
	if listeners
		.iter()
		.any(|l| matches!(l.protocol, ListenerProtocol::TCP))
	{
		return BindProtocol::tcp;
	}
	BindProtocol::http
}

pub fn auto_server() -> auto::Builder<::hyper_util::rt::TokioExecutor> {
	let mut b = auto::Builder::new(::hyper_util::rt::TokioExecutor::new());
	b.http2().timer(hyper_util::rt::tokio::TokioTimer::new());
	b
}

fn build_response(status: StatusCode) -> ::http::Response<()> {
	::http::Response::builder()
		.status(status)
		.body(())
		.expect("builder with known status code should not fail")
}

/// InboundError represents an error with an associated status code.
#[derive(Debug)]
#[allow(dead_code)]
struct InboundError(anyhow::Error, StatusCode);
