mod azure;
mod connect_tunnel;
mod dns;
mod hbone_tunnel;
mod tls;

use std::str::FromStr;
use std::task;

use ::http::uri::{Authority, Scheme};
use hyper_util_fork::rt::TokioIo;
use tracing::event;

use crate::http::backendtls::VersionedBackendTLS;
use crate::http::filters;
use crate::http::filters::BackendRequestTimeout;
use crate::proxy::ProxyError;
use crate::transport::hbone;
use crate::transport::stream::{LoggingMode, Socket};
use crate::types::agent::Target;
use crate::*;

#[derive(Clone)]
pub struct Client {
	client: hyper_util_fork::client::legacy::Client<Connector, http::Body, PoolKey>,
	connector: Connector,
}

impl Debug for Client {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Client").finish()
	}
}

pub struct Call {
	pub req: http::Request,
	pub target: Target,
	pub transport: Transport,
}

pub struct TCPCall {
	pub source: Socket,
	pub target: Target,
	pub transport: Transport,
}

#[derive(Default, Debug, Clone, Hash, PartialEq, Eq)]
pub enum ApplicationTransport {
	#[default]
	Plaintext,
	Tls(VersionedBackendTLS),
}

impl From<Option<VersionedBackendTLS>> for ApplicationTransport {
	fn from(value: Option<VersionedBackendTLS>) -> Self {
		match value {
			Some(tls) => ApplicationTransport::Tls(tls),
			None => ApplicationTransport::Plaintext,
		}
	}
}

impl ApplicationTransport {
	pub fn name(&self) -> &'static str {
		match self {
			ApplicationTransport::Plaintext => "plaintext",
			ApplicationTransport::Tls(_) => "tls",
		}
	}
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct TunnelConfig {
	pub proxy: Target,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Transport {
	Plain(ApplicationTransport),
	Tunnel(ApplicationTransport, TunnelConfig),
	Hbone(ApplicationTransport, Identity),
	DoubleHbone {
		gateway_address: SocketAddr, // Address of network gateway to connect to
		gateway_identity: Identity,  // Identity of network gateway
		waypoint_identity: Identity, // Identity of waypoint/workload
		inner: ApplicationTransport,
	},
}

impl From<ApplicationTransport> for Transport {
	fn from(value: ApplicationTransport) -> Self {
		Transport::Plain(value)
	}
}

impl Default for Transport {
	fn default() -> Self {
		Transport::Plain(ApplicationTransport::Plaintext)
	}
}

impl Transport {
	pub fn application(&self) -> &ApplicationTransport {
		match self {
			Transport::Plain(inner) => inner,
			Transport::Tunnel(inner, _) => inner,
			Transport::Hbone(inner, _) => inner,
			Transport::DoubleHbone { inner, .. } => inner,
		}
	}

	pub fn skip_dns_resolution(&self) -> bool {
		// For double HBONE, we don't need to resolve the hostname locally
		// The gateway will resolve it. Use a placeholder dest (won't be used).
		matches!(self, Transport::DoubleHbone { .. })
	}

	pub fn name(&self) -> &'static str {
		match self {
			Transport::Hbone(ApplicationTransport::Plaintext, _) => "hbone",
			Transport::Hbone(ApplicationTransport::Tls(_), _) => "hbone-tls",
			Transport::Plain(ApplicationTransport::Plaintext) => "plaintext",
			Transport::Plain(ApplicationTransport::Tls(_)) => "tls",
			Transport::Tunnel(ApplicationTransport::Plaintext, _) => "tunnel",
			Transport::Tunnel(ApplicationTransport::Tls(_), _) => "tunnel-tls",
			Transport::DoubleHbone {
				inner: ApplicationTransport::Plaintext,
				..
			} => "doublehbone",
			Transport::DoubleHbone {
				inner: ApplicationTransport::Tls(_),
				..
			} => "doublehbone-tls",
		}
	}
}

impl From<Option<VersionedBackendTLS>> for Transport {
	fn from(tls: Option<VersionedBackendTLS>) -> Self {
		if let Some(tls) = tls {
			ApplicationTransport::Tls(tls).into()
		} else {
			ApplicationTransport::Plaintext.into()
		}
	}
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct PoolKey(Target, SocketAddr, Transport, ::http::Version);

#[derive(Debug, Clone, Copy)]
pub struct ResolvedDestination(pub SocketAddr);

impl Transport {
	pub fn scheme(&self) -> Scheme {
		match *self.application() {
			ApplicationTransport::Plaintext => Scheme::HTTP,
			// TODO: make sure this is right, envoy had all sorts of issues around this.
			ApplicationTransport::Tls(_) => Scheme::HTTPS,
		}
	}
}

#[derive(Debug, Clone)]
struct Connector {
	hbone_pool: Option<agent_hbone::pool::WorkloadHBONEPool<hbone::WorkloadKey>>,
	backend_config: Arc<crate::BackendConfig>,
	metrics: Option<Arc<crate::metrics::Metrics>>,
	resolver: Arc<dns::CachedResolver>,
}

async fn dial(
	target: &Target,
	ep: SocketAddr,
	backend: &crate::BackendConfig,
) -> Result<Socket, http::Error> {
	match target {
		Target::UnixSocket(uds) => Socket::dial_unix(uds, backend)
			.await
			.map_err(crate::http::Error::new),
		_ => Socket::dial(ep, backend)
			.await
			.map_err(crate::http::Error::new),
	}
}

impl Connector {
	async fn connect(
		&mut self,
		target: Target,
		ep: SocketAddr,
		transport: Transport,
	) -> Result<Socket, http::Error> {
		let connect_start = std::time::Instant::now();
		let transport_name = transport.name();
		let skip_dns = transport.skip_dns_resolution();
		let tls = match transport.application() {
			ApplicationTransport::Plaintext => None,
			ApplicationTransport::Tls(application) => Some(application.clone()),
		};
		let stream = match transport {
			Transport::Plain(_) => dial(&target, ep, &self.backend_config).await?,
			Transport::Tunnel(_, tcfg) => {
				let proxy_dst: SocketAddr = self
					.resolve_target(skip_dns, &tcfg.proxy)
					.await
					.map_err(crate::http::Error::new)?;
				let dest = target.to_string();

				let mut con = dial(&tcfg.proxy, proxy_dst, &self.backend_config).await?;

				connect_tunnel::handshake(&mut con, &dest)
					.await
					.map_err(crate::http::Error::new)?;
				con
			},
			Transport::Hbone(_, identity) => {
				let pool = self
					.hbone_pool
					.clone()
					.ok_or_else(|| crate::http::Error::new(anyhow::anyhow!("hbone pool disabled")))?;
				hbone_tunnel::handshake(pool, ep, identity).await?
			},

			Transport::DoubleHbone {
				gateway_address,
				gateway_identity,
				waypoint_identity,
				inner: _,
			} => {
				let pool = self
					.hbone_pool
					.clone()
					.ok_or_else(|| crate::http::Error::new(anyhow::anyhow!("hbone pool disabled")))?;
				hbone_tunnel::handshake_double(
					pool,
					&target,
					ep,
					gateway_address,
					gateway_identity,
					waypoint_identity,
				)
				.await?
			},
		};

		// Apply application level TLS, if applicable
		let mut socket = if let Some(tls_cfg) = tls {
			tls::handshake(stream, &tls_cfg, target).await?
		} else {
			stream
		};

		let connect_ms = connect_start.elapsed().as_millis();
		if let Some(m) = &self.metrics {
			let labels = metrics::ConnectLabels {
				transport: strng::RichStrng::from(transport_name).into(),
			};
			// Note: convert from ms to seconds since Prometheus convention for histogram buckets is seconds.
			m.upstream_connect_duration
				.get_or_create(&labels)
				.observe((connect_ms as f64) / 1000.0);
		}

		event!(
			target: "upstream tcp",
			parent: None,
			tracing::Level::DEBUG,

			endpoint = %ep,
			transport = %transport_name,

			connect_ms = connect_ms,

			"connected"
		);

		socket.with_logging(LoggingMode::Upstream);
		Ok(socket)
	}

	async fn resolve_target(
		&self,
		skip_resolution: bool,
		target: &Target,
	) -> Result<SocketAddr, ProxyError> {
		if skip_resolution {
			// For double HBONE, we don't need to resolve the hostname locally
			// The gateway will resolve it. Use a placeholder dest (won't be used).
			return Ok(SocketAddr::from(([0, 0, 0, 0], 0)));
		}
		let dest = match &target {
			Target::Address(addr) => *addr,
			Target::Hostname(hostname, port) => {
				let ip = self
					.resolver
					.resolve(hostname.clone())
					.await
					.map_err(|_| ProxyError::DnsResolution)?;
				SocketAddr::from((ip, *port))
			},
			Target::UnixSocket(_) => {
				// Placeholder address for Unix sockets - the actual connection
				// uses the path from the Target, not this address
				SocketAddr::from(([0, 0, 0, 0], 0))
			},
		};
		Ok(dest)
	}
}

impl tower::Service<::http::Extensions> for Connector {
	type Response = TokioIo<Socket>;
	type Error = crate::http::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, mut dst: ::http::Extensions) -> Self::Future {
		let mut it = self.clone();

		Box::pin(async move {
			let PoolKey(target, ep, transport, _) =
				dst.remove::<PoolKey>().expect("pool key must be set");

			it.connect(target, ep, transport).await.map(TokioIo::new)
		})
	}
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Config {
	pub resolver_cfg: ResolverConfig,
	pub resolver_opts: ResolverOpts,
}

impl Client {
	pub fn new(
		cfg: &Config,
		hbone_pool: Option<agent_hbone::pool::WorkloadHBONEPool<hbone::WorkloadKey>>,
		backend_config: BackendConfig,
		metrics: Option<Arc<crate::metrics::Metrics>>,
	) -> Client {
		let resolver = dns::CachedResolver::new(cfg.resolver_cfg.clone(), cfg.resolver_opts.clone());
		let mut b =
			::hyper_util_fork::client::legacy::Client::builder(::hyper_util::rt::TokioExecutor::new());
		b.pool_timer(hyper_util::rt::tokio::TokioTimer::new());
		b.pool_idle_timeout(backend_config.pool_idle_timeout);
		b.timer(hyper_util::rt::tokio::TokioTimer::new());
		if let Some(pool_max) = backend_config.pool_max_size {
			b.pool_max_idle_per_host(pool_max);
		};

		let connector = Connector {
			resolver: Arc::new(resolver),
			hbone_pool,
			backend_config: Arc::new(backend_config),
			metrics,
		};
		let client = b.build_with_pool_key(connector.clone());
		Client { client, connector }
	}

	pub async fn simple_call(&self, req: http::Request) -> Result<http::Response, ProxyError> {
		let host = req
			.uri()
			.host()
			.ok_or_else(|| ProxyError::ProcessingString("no hostname set".to_string()))?;
		let scheme = req
			.uri()
			.scheme()
			.ok_or_else(|| ProxyError::ProcessingString("no scheme set".to_string()))?;
		let port = req
			.uri()
			.port()
			.map(|p| p.as_u16())
			.unwrap_or_else(|| if scheme == &Scheme::HTTPS { 443 } else { 80 });
		let transport = if scheme == &Scheme::HTTPS {
			ApplicationTransport::Tls(http::backendtls::SYSTEM_TRUST.base_config()).into()
		} else {
			ApplicationTransport::Plaintext.into()
		};
		let target = Target::try_from((host, port))
			.map_err(|e| ProxyError::ProcessingString(format!("failed to parse host: {e}")))?;
		self
			.call(Call {
				req,
				target,
				transport,
			})
			.await
	}

	pub async fn call_tcp(&self, call: TCPCall) -> Result<(), ProxyError> {
		let start = std::time::Instant::now();
		let TCPCall {
			source,
			target,
			transport,
		} = call;

		let dest = self
			.connector
			.resolve_target(transport.skip_dns_resolution(), &target)
			.await?;

		let transport_name = transport.name();
		let target_name = target.to_string();

		event!(
			target: "upstream tcp",
			parent: None,
			tracing::Level::DEBUG,

			target = %target_name,
			endpoint = %dest,
			transport = %transport_name,

			"started"
		);
		let upstream = self
			.connector
			.clone()
			.connect(target, dest, transport)
			.await
			.map_err(ProxyError::UpstreamTCPCallFailed)?;

		agent_core::copy::copy_bidirectional(source, upstream, &agent_core::copy::ConnectionResult {})
			.await
			.map_err(ProxyError::UpstreamTCPProxy)?;

		let dur = format!("{}ms", start.elapsed().as_millis());
		event!(
			target: "upstream tcp",
			parent: None,
			tracing::Level::DEBUG,

			target = %target_name,
			endpoint = %dest,
			transport = %transport_name,

			duration = dur,

			"completed"
		);
		Ok(())
	}

	pub async fn call(&self, call: Call) -> Result<http::Response, ProxyError> {
		let start = std::time::Instant::now();
		let Call {
			mut req,
			target,
			transport,
		} = call;
		let dest = self
			.connector
			.resolve_target(transport.skip_dns_resolution(), &target)
			.await?;
		let auto_host = req.extensions().get::<filters::AutoHostname>().is_some();
		http::modify_req_uri(&mut req, |uri| {
			let scheme = transport.scheme();
			// Strip the port from the hostname if its the default already
			// The hyper client does this for HTTP/1.1 but not for HTTP2
			if let Some(a) = uri.authority.as_mut()
				&& ((scheme == Scheme::HTTPS && a.port_u16() == Some(443))
					|| (scheme == Scheme::HTTP && a.port_u16() == Some(80)))
			{
				*a = Authority::from_str(a.host()).expect("host must be valid since it was already a host");
			}
			uri.scheme = Some(scheme);

			if let Target::Hostname(h, _) = &target
				&& auto_host
				&& let Some(a) = uri.authority.as_mut()
			{
				*a = Authority::from_str(h)?
			}
			Ok(())
		})
		.map_err(ProxyError::Processing)?;
		let version = req.version();
		let transport_name = transport.name();
		let target_name = target.to_string();
		let key = PoolKey(target, dest, transport, version);
		trace!(?req, ?key, "sending request");
		req.extensions_mut().insert(key);
		let method = req.method().clone();
		let uri = req.uri().clone();
		let path = uri.path();
		let host = uri.authority().to_owned();
		event!(
			target: "upstream request",
			parent: None,
			tracing::Level::TRACE,

			request =?req
		);
		let buffer_limit = http::buffer_limit(&req);
		let to = req.extensions().get::<BackendRequestTimeout>().cloned();
		let call = self.client.request(req);
		let resp = if let Some(to) = to {
			match tokio::time::timeout(to.0, call).await {
				Err(_) => Err(ProxyError::UpstreamCallTimeout),
				Ok(Err(e)) => Err(ProxyError::UpstreamCallFailed(e)),
				Ok(Ok(resp)) => Ok(resp),
			}
		} else {
			call.await.map_err(ProxyError::UpstreamCallFailed)
		};
		let dur = format!("{}ms", start.elapsed().as_millis());
		// If version changed due to ALPN negotiation, make sure we get the real version
		let version = resp.as_ref().map(|resp| resp.version()).unwrap_or(version);
		event!(
			target: "upstream request",
			parent: None,
			tracing::Level::DEBUG,

			target = %target_name,
			endpoint = %dest,
			transport = %transport_name,

			http.method = %method,
			http.host = host.as_ref().map(display),
			http.path = %path,
			http.version = ?version,
			http.status = resp.as_ref().ok().map(|s| s.status().as_u16()).unwrap_or_default(),

			duration = dur,
		);
		let mut resp = resp?.map(http::Body::new);

		event!(
			target: "upstream response",
			parent: None,
			tracing::Level::TRACE,

			response =?resp
		);

		resp
			.extensions_mut()
			.insert(transport::BufferLimit::new(buffer_limit));
		resp.extensions_mut().insert(ResolvedDestination(dest));
		Ok(resp)
	}
}
