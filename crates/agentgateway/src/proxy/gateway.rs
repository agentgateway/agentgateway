use std::collections::HashMap;
use std::convert::Infallible;
use std::error::Error as StdError;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_core::drain;
use agent_core::drain::{DrainUpgrader, DrainWatcher};
use anyhow::anyhow;
use bytes::Bytes;
use futures::pin_mut;
use futures_util::FutureExt;
use http::StatusCode;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
use rand::RngExt;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinSet};
use tokio_stream::StreamExt;
use tracing::{Instrument, debug, error, event, info, info_span, warn};

use crate::proxy::{ProxyError, WaypointService};
use crate::store::{BindEvent, BindListeners, FrontendPolices};
use crate::telemetry::metrics::TCPLabels;
use crate::transport::BufferLimit;
use crate::transport::stream::{
	Extension, LoggingMode, Socket, TCPConnectionInfo, TLSConnectionInfo,
};
use crate::types::agent::{
	BindKey, BindProtocol, Listener, ListenerProtocol, TransportProtocol, TunnelProtocol,
};
use crate::types::discovery::Service;
use crate::types::discovery::gatewayaddress::Destination;
use crate::types::frontend;
use crate::{ProxyInputs, client};
use agent_core::strng;
use agent_hbone::server::H2Request;

#[cfg(test)]
#[path = "gateway_test.rs"]
mod tests;

#[derive(Debug, Clone, PartialEq)]

pub enum HboneAddress {
	SocketAddr(SocketAddr),
	SvcHostname(Arc<str>, u16),
}

#[allow(dead_code)]
impl HboneAddress {
	pub fn port(&self) -> u16 {
		match self {
			HboneAddress::SocketAddr(s) => s.port(),
			HboneAddress::SvcHostname(_, p) => *p,
		}
	}

	pub fn ip(&self) -> Option<IpAddr> {
		match self {
			HboneAddress::SocketAddr(s) => Some(s.ip()),
			HboneAddress::SvcHostname(_, _) => None,
		}
	}

	pub fn svc_hostname(&self) -> Option<Arc<str>> {
		match self {
			HboneAddress::SocketAddr(_) => None,
			HboneAddress::SvcHostname(s, _) => Some(s.clone()),
		}
	}

	pub fn hostname_addr(&self) -> Option<Arc<str>> {
		match self {
			HboneAddress::SocketAddr(_) => None,
			HboneAddress::SvcHostname(_, _) => Some(Arc::from(self.to_string())),
		}
	}

	pub fn socket_addr(&self) -> Option<SocketAddr> {
		match self {
			HboneAddress::SocketAddr(addr) => Some(*addr),
			HboneAddress::SvcHostname(_, _) => None,
		}
	}
}

impl std::fmt::Display for HboneAddress {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			HboneAddress::SocketAddr(addr) => write!(f, "{addr}"),
			HboneAddress::SvcHostname(host, port) => write!(f, "{host}:{port}"),
		}
	}
}

impl From<SocketAddr> for HboneAddress {
	fn from(socket_addr: SocketAddr) -> Self {
		HboneAddress::SocketAddr(socket_addr)
	}
}

impl From<(Arc<str>, u16)> for HboneAddress {
	fn from(svc_hostname: (Arc<str>, u16)) -> Self {
		HboneAddress::SvcHostname(svc_hostname.0, svc_hostname.1)
	}
}

impl TryFrom<&http::Uri> for HboneAddress {
	type Error = anyhow::Error;

	fn try_from(value: &http::Uri) -> Result<Self, Self::Error> {
		match value.to_string().parse::<SocketAddr>() {
			Ok(addr) => Ok(HboneAddress::SocketAddr(addr)),
			Err(_) => {
				let host = value
					.host()
					.ok_or_else(|| anyhow::anyhow!("No valid authority"))?;
				let port = value
					.port_u16()
					.ok_or_else(|| anyhow::anyhow!("No valid authority"))?;
				Ok(HboneAddress::SvcHostname(host.into(), port))
			},
		}
	}
}

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
		let mut binds = {
			let mut binds = self.pi.stores.binds.write();
			binds.subscribe()
		};
		let mut active: HashMap<BindKey, AbortHandle> = HashMap::new();
		let mut handle_bind = |js: &mut JoinSet<anyhow::Result<()>>, b: BindEvent| {
			let (bind_key, bind, listeners) = match b {
				BindEvent::Add(bind, listeners) => (bind.key.clone(), bind, listeners),
				BindEvent::Remove(bind_key) => {
					if let Some(h) = active.remove(&bind_key) {
						h.abort();
					}
					return;
				},
			};
			if let Some(h) = active.remove(&bind_key) {
				h.abort();
			}

			debug!("add bind {}", bind.address);
			match listeners {
				BindListeners::Single(listener) => {
					let task = js.spawn(
						Self::run_bind(self.pi.clone(), subdrain.clone(), Arc::new(bind), listener)
							.in_current_span(),
					);
					active.insert(bind_key, task);
				},
				BindListeners::PerCore(listeners) => {
					for (core_id, listener) in listeners {
						let subdrain = subdrain.clone();
						let pi = self.pi.clone();
						let bind = bind.clone();
						std::thread::spawn(move || {
							let res = core_affinity::set_for_current(core_id);
							if !res {
								panic!("failed to set current CPU")
							}
							tokio::runtime::Builder::new_current_thread()
								.enable_all()
								.build()
								.unwrap()
								.block_on(async {
									let _ = Self::run_bind(pi, subdrain, Arc::new(bind), listener)
										.in_current_span()
										.await;
								})
						});
					}
				},
			}
		};
		let wait = drain.wait_for_drain();
		tokio::pin!(wait);
		loop {
			tokio::select! {
				Some(res) = binds.next() => {
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
		bind: Arc<crate::types::agent::Bind>,
		listener: std::net::TcpListener,
	) -> anyhow::Result<()> {
		let min_deadline = pi.cfg.termination_min_deadline;
		let max_deadline = pi.cfg.termination_max_deadline;
		let name = bind.key.clone();
		let bind_protocol = bind.protocol;
		let tunnel_protocol = bind.tunnel_protocol;
		let pi = if pi.cfg.threading_mode == crate::ThreadingMode::ThreadPerCore {
			let mut pi = Arc::unwrap_or_clone(pi);
			let client = client::Client::new(
				&pi.cfg.dns,
				None,
				pi.cfg.backend.clone(),
				Some(pi.metrics.clone()),
			);
			pi.upstream = client;
			Arc::new(pi)
		} else {
			pi
		};
		let listener = tokio::net::TcpListener::from_std(listener)?;
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
				let Ok(mut stream) = Socket::from_tcp(stream) else {
					// Can fail if they immediately disconnected; not much we can do.
					return;
				};
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
						_ = Self::handle_tunnel(name.clone(), bind_protocol, tunnel_protocol, stream, pi, drain) => {}
					}
					debug!(bind=?name, dur=?start.elapsed(), "connection completed");
				});
			};
			let wait = drain_watch.wait_for_drain();
			tokio::pin!(wait);
			const BACKOFF_INITIAL: Duration = Duration::from_millis(5);
			const BACKOFF_MAX: Duration = Duration::from_millis(100);
			let mut backoff = BACKOFF_INITIAL;
			// First, accept new connections until a drain is triggered
			// NOTE: Do not use `Ok(...) = listener.accept()` as a select! pattern.
			// If accept() returns Err, select! permanently disables that branch,
			// hanging the loop. Match on the full Result instead.
			let drain_mode = loop {
				tokio::select! {
					res = listener.accept() => match res {
						Ok((stream, _peer)) => {
							backoff = BACKOFF_INITIAL;
							handle_stream(stream, &upgrader);
						}
						Err(e) => {
							if is_accept_error_permanent(&e) {
								error!(bind=?name, "fatal accept error, stopping listener: {e}");
								return;
							}
							if is_accept_error_per_connection(&e) {
								debug!(bind=?name, "per-connection accept error: {e}");
								continue;
							}
							warn!(bind=?name, "accept error: {e}");
							let jittered = Duration::from_millis(
								rand::rng().random_range(0..=backoff.as_millis() as u64)
							);
							tokio::select! {
								_ = tokio::time::sleep(jittered) => {},
								res = &mut wait => { break res; }
							}
							backoff = (backoff * 2).min(BACKOFF_MAX);
							continue;
						}
					},
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
			backoff = BACKOFF_INITIAL;
			loop {
				tokio::select! {
					res = listener.accept() => match res {
						Ok((stream, _peer)) => {
							backoff = BACKOFF_INITIAL;
							handle_stream(stream, &upgrader);
						}
						Err(e) => {
							if is_accept_error_permanent(&e) {
								error!(bind=?name, "fatal accept error during drain, stopping listener: {e}");
								return;
							}
							if is_accept_error_per_connection(&e) {
								debug!(bind=?name, "per-connection accept error during drain: {e}");
								continue;
							}
							warn!(bind=?name, "accept error during drain: {e}");
							let jittered = Duration::from_millis(
								rand::rng().random_range(0..=backoff.as_millis() as u64)
							);
							tokio::select! {
								_ = tokio::time::sleep(jittered) => {},
								_ = &mut drained_for_minimum => { return; }
							}
							backoff = (backoff * 2).min(BACKOFF_MAX);
							continue;
						}
					},
					_ = &mut drained_for_minimum => {
						// We are done! exit.
						// This will stop accepting new connections
						return;
					}
				}
			}
		};

		drain::run_with_drain(component, drain, max_deadline, min_deadline, accept).await;
		Ok(())
	}

	pub async fn proxy_bind(
		bind_name: BindKey,
		bind_protocol: BindProtocol,
		mut raw_stream: Socket,
		inputs: Arc<ProxyInputs>,
		drain: DrainWatcher,
	) {
		let policies = inputs
			.stores
			.read_binds()
			.frontend_policies(inputs.cfg.gateway_ref());
		if let Some(tcp) = policies.tcp.as_ref() {
			raw_stream.apply_tcp_settings(tcp)
		}
		let peer_addr = raw_stream.tcp().peer_addr;
		event!(
			target: "downstream connection",
			parent: None,
			tracing::Level::DEBUG,

			src.addr = %peer_addr,
			protocol = ?bind_protocol,

			"opened",
		);
		match bind_protocol {
			BindProtocol::http => {
				let err = Self::proxy(
					bind_name,
					inputs,
					None,
					raw_stream,
					Arc::new(policies),
					drain,
				)
				.await;
				if let Err(e) = err {
					warn!(src.addr = %peer_addr, "proxy error: {e}");
				}
			},
			BindProtocol::tcp => Self::proxy_tcp(bind_name, inputs, None, raw_stream, drain).await,
			BindProtocol::tls => {
				match Self::maybe_terminate_tls(
					inputs.clone(),
					raw_stream,
					&policies,
					bind_name.clone(),
					false,
				)
				.await
				{
					Ok((selected_listener, stream)) => match selected_listener.protocol {
						ListenerProtocol::HTTPS(_) => {
							let _ = Self::proxy(
								bind_name,
								inputs,
								Some(selected_listener),
								stream,
								Arc::new(policies),
								drain,
							)
							.await;
						},
						ListenerProtocol::TLS(_) => {
							Self::proxy_tcp(bind_name, inputs, Some(selected_listener), stream, drain).await
						},
						_ => {
							error!(
								"invalid: TLS listener protocol is neither HTTPS nor TLS: {:?}",
								selected_listener.protocol
							)
						},
					},
					Err(e) => {
						event!(
							target: "downstream connection",
							parent: None,
							tracing::Level::WARN,

							src.addr = %peer_addr,
							protocol = ?bind_protocol,
							error = ?e.to_string(),

							"failed to terminate TLS",
						);
					},
				}
			},
			BindProtocol::auto => {
				// Auto-detect: peek at first byte to distinguish TLS from plaintext HTTP.
				// No timeout here — existing HTTP header_read_timeout and TLS handshake
				// timeout handle slow/dead clients downstream.
				let (ext, metrics, inner) = raw_stream.into_parts();
				let mut rewind = Socket::new_rewind(inner);
				let mut buf = [0u8; 1];
				match tokio::io::AsyncReadExt::read_exact(&mut rewind, &mut buf).await {
					Ok(_) => {
						rewind.rewind();
						let stream = Socket::from_rewind(ext, metrics, rewind);
						if buf[0] == 0x16 {
							// TLS ClientHello — dispatch as TLS
							match Self::maybe_terminate_tls(
								inputs.clone(),
								stream,
								&policies,
								bind_name.clone(),
								false,
							)
							.await
							{
								Ok((selected_listener, tls_stream)) => match selected_listener.protocol {
									ListenerProtocol::HTTPS(_) => {
										let _ = Self::proxy(
											bind_name,
											inputs,
											Some(selected_listener),
											tls_stream,
											Arc::new(policies),
											drain,
										)
										.await;
									},
									ListenerProtocol::TLS(_) => {
										Self::proxy_tcp(
											bind_name,
											inputs,
											Some(selected_listener),
											tls_stream,
											drain,
										)
										.await
									},
									_ => {
										error!(
											"invalid: TLS listener protocol is neither HTTPS nor TLS: {:?}",
											selected_listener.protocol
										)
									},
								},
								Err(e) => {
									event!(
										target: "downstream connection",
										parent: None,
										tracing::Level::WARN,
										src.addr = %peer_addr,
										protocol = ?bind_protocol,
										error = ?e.to_string(),
										"failed to terminate TLS (auto-detected)",
									);
								},
							}
						} else {
							// Plaintext HTTP
							let err =
								Self::proxy(bind_name, inputs, None, stream, Arc::new(policies), drain).await;
							if let Err(e) = err {
								warn!(src.addr = %peer_addr, "proxy error: {e}");
							}
						}
					},
					Err(e) => {
						warn!(src.addr = %peer_addr, "auto-detect read failed: {e}");
					},
				}
			},
		}
	}

	pub async fn handle_tunnel(
		bind_name: BindKey,
		bind_protocol: BindProtocol,
		tunnel_protocol: TunnelProtocol,
		mut raw_stream: Socket,
		inputs: Arc<ProxyInputs>,
		drain: DrainWatcher,
	) {
		let policies = inputs
			.stores
			.read_binds()
			.frontend_policies(inputs.cfg.gateway_ref());
		if let Some(tcp) = policies.tcp.as_ref() {
			raw_stream.apply_tcp_settings(tcp)
		}
		let peer_addr = raw_stream.tcp().peer_addr;
		event!(
			target: "downstream connection",
			parent: None,
			tracing::Level::TRACE,

			src.addr = %peer_addr,
			tunnel_protocol = ?tunnel_protocol,

			"opened tunnel",
		);
		match tunnel_protocol {
			TunnelProtocol::Direct => {
				// No tunnel
				Self::proxy_bind(bind_name, bind_protocol, raw_stream, inputs, drain).await
			},
			TunnelProtocol::HboneWaypoint => {
				let err =
					Self::terminate_waypoint_hbone(bind_name, inputs, raw_stream, policies, drain).await;
				if let Err(e) = err {
					warn!(src.addr = %peer_addr, "hbone error: {e}");
				}
			},
			TunnelProtocol::HboneGateway => {
				let _ = Self::terminate_gateway_hbone(inputs, raw_stream, policies, drain).await;
			},
			TunnelProtocol::Proxy => {
				let _ =
					Self::terminate_proxy_protocol(bind_name, bind_protocol, inputs, raw_stream, drain).await;
			},
		}
	}

	async fn proxy(
		bind_name: BindKey,
		inputs: Arc<ProxyInputs>,
		selected_listener: Option<Arc<Listener>>,
		mut stream: Socket,
		policies: Arc<FrontendPolices>,
		drain: DrainWatcher,
	) -> anyhow::Result<()> {
		let target_address = stream.target_address();
		let server = auto_server(policies.http.as_ref());

		// Precompute transport labels and metrics before moving `selected_listener` and `inputs`
		let tcp = stream
			.ext::<TCPConnectionInfo>()
			.expect("tcp info must be set");
		let tls = stream.ext::<TLSConnectionInfo>();
		let transport_protocol = if tls.is_some() {
			TransportProtocol::https
		} else {
			TransportProtocol::http
		};

		let transport_labels = TCPLabels {
			bind: Some(&bind_name).into(),
			gateway: selected_listener
				.as_ref()
				.map(|l| l.name.as_gateway_name())
				.into(),
			listener: selected_listener
				.as_ref()
				.map(|l| l.name.listener_name.clone())
				.into(),
			protocol: transport_protocol,
		};

		inputs
			.metrics
			.downstream_connection
			.get_or_create(&transport_labels)
			.inc();

		let unverified_workload = crate::cel::WorkloadContext::from_stores(
			&inputs.stores,
			&inputs.cfg.network,
			tcp.peer_addr.ip(),
		);
		let src = crate::cel::SourceContext::from_tcp_connection(
			tcp,
			tls.and_then(|t| t.src_identity.clone()),
			unverified_workload,
		);
		if let Some(network_authorization) = policies.network_authorization.as_ref()
			&& let Err(e) = network_authorization.apply(&src)
		{
			anyhow::bail!("network authorization denied: {e}");
		}
		stream.ext_mut().insert(src);

		let transport_metrics = inputs.metrics.clone();
		let proxy = super::httpproxy::HTTPProxy {
			bind_name,
			inputs,
			selected_listener,
			target_address,
		};
		let connection = Arc::new(stream.get_ext());
		// export rx/tx bytes on drop
		let mut stream = stream;
		stream.set_transport_metrics(transport_metrics, transport_labels);

		let def = frontend::HTTP::default();
		let buffer = policies
			.http
			.as_ref()
			.map(|h| h.max_buffer_size)
			.unwrap_or(def.max_buffer_size);

		let serve = server.serve_connection_with_upgrades(
			TokioIo::new(stream),
			hyper::service::service_fn(move |mut req| {
				let proxy = proxy.clone();
				let connection = connection.clone();
				req.extensions_mut().insert(BufferLimit::new(buffer));
				async move { proxy.proxy(connection, req).map(Ok::<_, Infallible>).await }
			}),
		);
		// Wrap it in the graceful watcher, will ensure GOAWAY/Connect:clone when we shutdown
		let serve = drain.wrap_connection(serve);
		let res = serve.await;
		match res {
			Ok(_) => Ok(()),
			Err(e) => {
				if should_ignore_downstream_connection_error(e.as_ref()) {
					// Expected for idle keepalive expiry and clients tearing down long-lived
					// streams such as SSE before the server finishes the response.
					return Ok(());
				}
				anyhow::bail!("{e}");
			},
		}
	}

	async fn proxy_tcp(
		bind_name: BindKey,
		inputs: Arc<ProxyInputs>,
		selected_listener: Option<Arc<Listener>>,
		stream: Socket,
		_drain: DrainWatcher,
	) {
		let selected_listener = match selected_listener {
			Some(l) => l,
			None => {
				let Some(bind) = inputs.stores.read_binds().bind(&bind_name) else {
					error!("no bind found for {bind_name}");
					return;
				};
				let Ok(selected_listener) = bind.listeners.get_exactly_one() else {
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

	// maybe_terminate_tls will observe the TLS handshake, and once the client hello has been received, select
	// a listener (based on SNI).
	// Based on the listener, it will passthrough the TLS or terminate it with the appropriate configuration.
	async fn maybe_terminate_tls(
		inp: Arc<ProxyInputs>,
		raw_stream: Socket,
		policies: &FrontendPolices,
		bind_key: BindKey,
		is_https: bool,
	) -> anyhow::Result<(Arc<Listener>, Socket)> {
		let def = frontend::TLS::default();
		let tls_pol = policies.tls.as_ref();
		let to = tls_pol.unwrap_or(&def).handshake_timeout;
		let handshake = async move {
			let Some(bind) = inp.stores.read_binds().bind(&bind_key) else {
				return Err(ProxyError::BindNotFound.into());
			};
			let listeners = &bind.listeners;
			let (mut ext, counter, inner) = raw_stream.into_parts();
			let inner = Socket::new_rewind(inner);
			let acceptor =
				tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), inner);
			pin_mut!(acceptor);
			let tls_start = std::time::Instant::now();
			let mut start = match acceptor.as_mut().await {
				Ok(start) => start,
				Err(e) => {
					if is_https
						&& let Some(io) = acceptor.take_io()
						&& let Some(data) = io.buffered()
						&& tls_looks_like_http(data)
					{
						anyhow::bail!("client sent an HTTP request to an HTTPS listener: {e}");
						// TODO(https://github.com/rustls/tokio-rustls/pull/147): write
						// let _ = io.write_all(b"HTTP/1.0 400 Bad Request\r\n\r\nclient sent an HTTP request to an HTTPS listener\n").await;
						// let _ = io.shutdown().await;
					}
					anyhow::bail!(e);
				},
			};
			let ch = start.client_hello();
			let sni = ch.server_name().unwrap_or_default();
			let best = listeners
				.best_match_tls(sni)
				.ok_or(anyhow!("no TLS listener match for {sni}"))?;
			match best.protocol.tls(tls_pol) {
				Some(Err(e)) => {
					// There is a TLS config for this listener, but its invalid. Reject the connection
					Err(e)
				},
				Some(Ok(cfg)) => {
					let tokio_rustls::StartHandshake { accepted, io, .. } = start;
					let start = tokio_rustls::StartHandshake::from_parts(accepted, Box::new(io.discard()));
					let tls = start.into_stream(cfg).await?;
					let tls_dur = tls_start.elapsed();
					// TLS handshake duration
					let protocol = if matches!(best.protocol, ListenerProtocol::HTTPS(_)) {
						TransportProtocol::https
					} else {
						TransportProtocol::tls
					};
					inp
						.metrics
						.tls_handshake_duration
						.get_or_create(&TCPLabels {
							bind: Some(&bind_key).into(),
							gateway: Some(best.name.as_gateway_name()).into(),
							listener: best.name.listener_name.clone().into(),
							protocol,
						})
						.observe(tls_dur.as_secs_f64());
					let (_, ssl) = tls.get_ref();
					// Rustls doesn't give us a way to say "The client certificate was present, but not verifier,
					// but should be allowed, but should not be used"
					// which is the behavior we want for insecure fallback.
					// So we check again...
					let include_src_identity = best.protocol.include_src_identity_for_connection(ssl);
					Ok((
						best,
						Socket::from_tls_with_identity(ext, counter, tls.into(), include_src_identity)?,
					))
				},
				None => {
					let sni = sni.to_string();
					// Passthrough
					start.io.rewind();
					ext.insert(TLSConnectionInfo {
						server_name: Some(sni),
						..Default::default()
					});
					Ok((best, Socket::from_rewind(ext, counter, start.io)))
				},
			}
		};
		tokio::time::timeout(to, handshake).await?
	}

	/// Handle incoming connection with PROXY protocol v2 header.
	///
	/// Used for Istio sandwich waypoint mode where ztunnel handles mTLS termination
	/// and forwards traffic to agentgateway using PROXY protocol. The PROXY header
	/// contains the original client addresses and peer identity (TLV 0xD0).
	async fn terminate_proxy_protocol(
		bind_name: BindKey,
		bind_protocol: BindProtocol,
		inp: Arc<ProxyInputs>,
		mut raw_stream: Socket,
		drain: DrainWatcher,
	) -> anyhow::Result<()> {
		use std::time::Instant;

		use super::proxy_protocol::parse_proxy_protocol;
		use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
		use crate::transport::tls::TlsInfo;

		// PROXY protocol header is small (~232 bytes max), should arrive quickly.
		// Use a relatively short timeout to detect misbehaving or slow clients.
		const PROXY_PROTOCOL_TIMEOUT: Duration = Duration::from_secs(5);

		// Parse PROXY protocol header from the stream with timeout
		let pp_info = tokio::time::timeout(
			PROXY_PROTOCOL_TIMEOUT,
			parse_proxy_protocol(&mut raw_stream),
		)
		.await??;

		// Capture ztunnel's address (the original TCP peer) before we overwrite it
		let raw_peer_addr = raw_stream.tcp().peer_addr;

		// Update TCPConnectionInfo with real source/dest from PROXY header
		// This overwrites ztunnel's loopback address with the actual client address
		raw_stream.ext_mut().insert(TCPConnectionInfo {
			peer_addr: pp_info.src_addr,
			local_addr: pp_info.dst_addr,
			start: Instant::now(),
			raw_peer_addr: Some(raw_peer_addr),
		});

		// Insert TLSConnectionInfo with identity from TLV 0xD0
		// Even though there's no TLS on this connection, we use this struct
		// to carry the peer identity that ztunnel extracted from mTLS
		if let Some(identity) = pp_info.peer_identity {
			raw_stream.ext_mut().insert(TLSConnectionInfo {
				src_identity: Some(TlsInfo {
					identity: Some(identity),
					subject_alt_names: vec![],
					issuer: crate::strng::EMPTY,
					subject: crate::strng::EMPTY,
					subject_cn: None,
				}),
				server_name: None,
				negotiated_alpn: None,
			});
		}

		// Continue with normal protocol handling - the identity is now in the socket
		// extensions and will flow through to CEL authorization via with_source()
		Self::proxy_bind(bind_name, bind_protocol, raw_stream, inp, drain).await;
		Ok(())
	}

	async fn terminate_waypoint_hbone(
		bind_name: BindKey,
		inp: Arc<ProxyInputs>,
		raw_stream: Socket,
		policies: FrontendPolices,
		drain: DrainWatcher,
	) -> anyhow::Result<()> {
		let Some(ca) = inp.ca.as_ref() else {
			anyhow::bail!("CA is required for waypoint");
		};

		let def = frontend::TLS::default();
		let to = policies.tls.as_ref().unwrap_or(&def).handshake_timeout;

		let cert = ca.get_identity().await?;
		let sc = Arc::new(cert.hbone_termination()?);
		let tls = tokio::time::timeout(to, crate::transport::tls::accept(raw_stream, sc)).await??;

		debug!("accepted connection");
		let cfg = inp.cfg.clone();
		let request_handler = move |req, ext, graceful| {
			Self::serve_waypoint_connect(bind_name.clone(), inp.clone(), req, ext, graceful)
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

	async fn terminate_gateway_hbone(
		inp: Arc<ProxyInputs>,
		raw_stream: Socket,
		policies: FrontendPolices,
		drain: DrainWatcher,
	) -> anyhow::Result<()> {
		let Some(ca) = inp.ca.as_ref() else {
			anyhow::bail!("CA is required for waypoint");
		};

		let def = frontend::TLS::default();
		let to = policies.tls.as_ref().unwrap_or(&def).handshake_timeout;

		let cert = ca.get_identity().await?;
		let sc = Arc::new(cert.hbone_termination()?);
		let tls = tokio::time::timeout(to, crate::transport::tls::accept(raw_stream, sc)).await??;

		debug!("accepted connection");
		let cfg = inp.cfg.clone();
		let request_handler = move |req, ext, graceful| {
			Self::serve_gateway_connect(inp.clone(), req, ext, graceful).instrument(info_span!("inbound"))
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

	/// serve_waypoint_connect handles a single connection from a client.
	#[allow(clippy::too_many_arguments)]
	async fn serve_waypoint_connect(
		bind_name: BindKey,
		pi: Arc<ProxyInputs>,
		req: agent_hbone::server::H2Request,
		ext: Arc<Extension>,
		drain: DrainWatcher,
	) {
		let (socket_addr, svc) = match Self::setup_hbone_info(&pi, &req).await {
			Ok(i) => i,
			Err(e) => {
				warn!("hbone failed: {e}");
				let _ = req
					.send_response(build_response(StatusCode::BAD_REQUEST))
					.await;
				return;
			},
		};

		// Determine protocol from service discovery. Default to HTTP since the vast
		// majority of waypoint traffic is HTTP; only use TCP when there is a positive
		// signal via explicit AppProtocol::Tcp/Tls (from istio/istio#59259).
		let is_http = !svc.port_is_tcp(socket_addr.port());

		let Ok(resp) = req.send_response(build_response(StatusCode::OK)).await else {
			warn!("failed to send response");
			return;
		};
		let con = agent_hbone::RWStream {
			stream: resp,
			buf: Bytes::new(),
			drain_tx: None,
		};

		let socket = Socket::from_hbone(ext, socket_addr, con);
		Self::handle_waypoint(bind_name, pi, svc, socket, is_http, drain).await;
	}

	/// Resolve the HBONE listener and dispatch to the HTTP or TCP proxy.
	pub(crate) async fn handle_waypoint(
		bind_name: BindKey,
		pi: Arc<ProxyInputs>,
		svc: Arc<crate::types::discovery::Service>,
		mut socket: Socket,
		is_http: bool,
		drain: DrainWatcher,
	) {
		// Find HBONE listener, or fall back to a synthetic one using gateway
		// config names so gateway/listener-targeted policies still match.
		let listener = pi
			.stores
			.read_binds()
			.bind(&bind_name)
			.and_then(|b| {
				b.listeners
					.inner
					.values()
					.find(|l| matches!(l.protocol, ListenerProtocol::HBONE))
					.cloned()
			})
			.unwrap_or_else(|| {
				// Synthetic fallback so route selection works via VIP lookup.
				// We may eventually elide the need to generate an HBONE listener at all.
				Arc::new(Listener {
					key: Default::default(),
					name: crate::types::agent::ListenerName {
						gateway_name: pi.cfg.xds.gateway.clone(),
						gateway_namespace: pi.cfg.xds.namespace.clone(),
						listener_name: strng::EMPTY,
						listener_set: None,
					},
					hostname: Default::default(),
					protocol: ListenerProtocol::HBONE,
					tcp_routes: Default::default(),
					routes: Default::default(),
				})
			});

		let should_sniff_tls = svc.port_is_tls(socket.target_address().port());
		let wps = WaypointService(svc);
		// Ensure we load policies per-stream so we don't cache stale policies on long-lived HBONE connections.
		let policies = pi
			.stores
			.read_binds()
			.listener_frontend_policies(&listener.name, Some(wps.as_policy_ref()));
		socket.ext_mut().insert(wps);
		if is_http {
			let _ = Self::proxy(
				bind_name,
				pi,
				Some(listener),
				socket,
				Arc::new(policies),
				drain,
			)
			.await;
		} else {
			// For waypoint TCP traffic, only sniff TLS if the service port's appProtocol is TLS
			socket
				.ext_mut()
				.insert(crate::transport::stream::WaypointTLSInfo { should_sniff_tls });

			Self::proxy_tcp(bind_name, pi, Some(listener), socket, drain).await;
		}
	}

	async fn setup_hbone_info(
		pi: &Arc<ProxyInputs>,
		req: &H2Request,
	) -> anyhow::Result<(SocketAddr, Arc<Service>)> {
		let uri = req.uri();
		let parsed_addr = match HboneAddress::try_from(uri) {
			Ok(addr) => addr,
			Err(err) => {
				anyhow::bail!("invalid URI format: {uri} {err}");
			},
		};

		// Resolve the HBONE address to a socket address and detect the service protocol
		// in a single discovery store lookup to avoid redundant read locks.
		let discovery = pi.stores.read_discovery();
		let network = &pi.cfg.network;

		let (addr, svc) = match parsed_addr {
			HboneAddress::SocketAddr(addr) => {
				let Some(svc) = discovery
					.services
					.get_by_vip(&crate::types::discovery::NetworkAddress {
						network: network.clone(),
						address: addr.ip(),
					})
				else {
					anyhow::bail!("no service found for address {addr}");
				};
				(addr, svc)
			},
			HboneAddress::SvcHostname(hostname, port) => {
				let hostname_str = hostname.to_string();
				let Some(svc) = find_service_by_hostname(&discovery, &hostname_str) else {
					anyhow::bail!("no service found for hostname {hostname_str}");
				};

				let Some(vip) = svc
					.vips
					.iter()
					.find(|vip| vip.network == *network)
					.or_else(|| svc.vips.first())
					.map(|v| v.address)
				else {
					anyhow::bail!("serve_waypoint_connect: no VIP found for service {hostname_str}");
				};
				(SocketAddr::from((vip, port)), svc)
			},
		};

		// Make sure the service is actually bound to us
		let Some(wp) = svc.waypoint.as_ref() else {
			anyhow::bail!(
				"service {}.{} is not bound to a waypoint",
				svc.hostname,
				svc.namespace
			);
		};
		let Some(self_id) = pi.cfg.self_addr.as_ref() else {
			anyhow::bail!("self_id required for waypoint");
		};
		let is_ours = match &wp.destination {
			Destination::Address(addr) => self_id.matches_address(addr, |ns, hostname| {
				let self_svc = discovery.services.get_by_namespaced_host(
					&crate::types::discovery::NamespacedHostname {
						namespace: ns.clone(),
						hostname: hostname.clone(),
					},
				)?;
				Some(self_svc.vips.clone())
			}),
			Destination::Hostname(n) => self_id.matches_hostname(n),
		};
		if !is_ours {
			anyhow::bail!(
				"service {} is meant for waypoint {:?}, but we are {}.{}",
				svc.hostname,
				wp.destination,
				self_id.gateway,
				self_id.namespace
			);
		}

		Ok((addr, svc))
	}

	/// serve_gateway_connect handles a single connection from a client.
	#[allow(clippy::too_many_arguments)]
	async fn serve_gateway_connect(
		pi: Arc<ProxyInputs>,
		req: agent_hbone::server::H2Request,
		ext: Arc<Extension>,
		drain: DrainWatcher,
	) {
		debug!(?req, "received request");

		let uri = req.uri();
		let hbone_addr = HboneAddress::try_from(uri)
			.map_err(|_| InboundError(anyhow::anyhow!("bad request"), StatusCode::BAD_REQUEST))
			.unwrap();
		let socket_addr = hbone_addr
			.socket_addr()
			.ok_or_else(|| {
				InboundError(
					anyhow::anyhow!("hostname resolution not supported"),
					StatusCode::BAD_REQUEST,
				)
			})
			.unwrap();
		let Some(bind) = pi.stores.read_binds().find_bind(socket_addr) else {
			warn!("no bind for {hbone_addr}");
			let Ok(_) = req
				.send_response(build_response(StatusCode::NOT_FOUND))
				.await
			else {
				warn!("failed to send response");
				return;
			};
			return;
		};
		let Ok(resp) = req.send_response(build_response(StatusCode::OK)).await else {
			warn!("failed to send response");
			return;
		};
		let con = agent_hbone::RWStream {
			stream: resp,
			buf: Bytes::new(),
			drain_tx: None,
		};

		Self::proxy_bind(
			bind.key.clone(),
			bind.protocol,
			Socket::from_hbone(ext, socket_addr, con),
			pi,
			drain,
		)
		.await
	}
}

fn tls_looks_like_http(d: Bytes) -> bool {
	d.starts_with(b"GET /")
		|| d.starts_with(b"POST /")
		|| d.starts_with(b"HEAD /")
		|| d.starts_with(b"PUT /")
		|| d.starts_with(b"OPTIONS /")
		|| d.starts_with(b"DELETE /")
}

pub fn auto_server(c: Option<&frontend::HTTP>) -> auto::Builder<::hyper_util::rt::TokioExecutor> {
	let mut b = auto::Builder::new(::hyper_util::rt::TokioExecutor::new());
	b.http2().timer(hyper_util::rt::tokio::TokioTimer::new());
	b.http1().timer(hyper_util::rt::tokio::TokioTimer::new());
	let def = frontend::HTTP::default();

	let frontend::HTTP {
		max_buffer_size: _, // Not handled here
		http1_max_headers,
		http1_idle_timeout,
		http2_window_size,
		http2_connection_window_size,
		http2_frame_size,
		http2_keepalive_interval,
		http2_keepalive_timeout,
	} = c.unwrap_or(&def);

	if let Some(m) = http1_max_headers {
		b.http1().max_headers(*m);
	}
	// See https://github.com/agentgateway/agentgateway/issues/504 for why "idle timeout" is used as "read header timeout"
	b.http1().header_read_timeout(Some(*http1_idle_timeout));

	if http2_window_size.is_some() || http2_connection_window_size.is_some() {
		if let Some(w) = http2_connection_window_size {
			b.http2().initial_connection_window_size(Some(*w));
		}
		if let Some(w) = http2_window_size {
			b.http2().initial_stream_window_size(Some(*w));
		}
	} else {
		b.http2().adaptive_window(true);
	}
	b.http2().keep_alive_interval(*http2_keepalive_interval);
	if let Some(to) = http2_keepalive_timeout {
		b.http2().keep_alive_timeout(*to);
	}
	if let Some(m) = http2_frame_size {
		b.http2().max_frame_size(*m);
	}

	b
}

/// The listening socket itself is broken; retrying won't help.
/// EBADF/ENOTSOCK: fd is dead on all platforms.
/// EINVAL: permanent on Linux (socket not listening), transient on macOS (can recover).
fn is_accept_error_permanent(e: &std::io::Error) -> bool {
	match e.raw_os_error() {
		Some(libc::EBADF | libc::ENOTSOCK) => true,
		#[cfg(target_os = "linux")]
		Some(libc::EINVAL) => true,
		_ => false,
	}
}

/// Per-connection failure (client gone during handshake); harmless, no backoff needed.
fn is_accept_error_per_connection(e: &std::io::Error) -> bool {
	matches!(
		e.raw_os_error(),
		Some(libc::ECONNABORTED | libc::ECONNRESET | libc::EPERM)
	)
}

fn should_ignore_downstream_connection_error(err: &(dyn StdError + 'static)) -> bool {
	if let Some(hyper_err) = err.downcast_ref::<hyper::Error>()
		&& (hyper_err.is_timeout() || hyper_err.is_incomplete_message())
	{
		return true;
	}

	false
}

fn build_response(status: StatusCode) -> ::http::Response<()> {
	::http::Response::builder()
		.status(status)
		.body(())
		.expect("builder with known status code should not fail")
}

fn find_service_by_hostname(
	stores: &crate::store::DiscoveryStore,
	hostname: &str,
) -> Option<Arc<crate::types::discovery::Service>> {
	stores
		.services
		.get_by_hostname(hostname)
		.and_then(|services| {
			// If multiple services have the same hostname, pick the first one that has VIPs
			services.into_iter().find(|s| !s.vips.is_empty())
		})
}

/// InboundError represents an error with an associated status code.
#[derive(Debug)]
#[allow(dead_code)]
struct InboundError(anyhow::Error, StatusCode);
