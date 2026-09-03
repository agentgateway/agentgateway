mod azure;
pub(crate) mod connect_tunnel;
mod dns;
mod hbone_tunnel;
mod tls;

use std::str::FromStr;
use std::task;

use ::http::HeaderValue;
use ::http::uri::{Authority, Scheme};
use agent_pool::pool::ExpectedCapacity;
use agent_pool::rt::TokioIo;
use tracing::event;

use crate::http::backendtls::VersionedBackendTLS;
use crate::http::filters;
use crate::http::filters::BackendRequestTimeout;
use crate::proxy::ProxyError;
use crate::transport::stream::{LoggingMode, Socket};
use crate::transport::{hbone, stream};
use crate::types::agent::Target;
use crate::*;

#[derive(Clone)]
pub struct Client {
	client: agent_pool::Client<Connector, PoolKey>,
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
	pub connection: ConnectionConfig,
}

pub struct TCPCall {
	pub source: Socket,
	pub target: Target,
	pub connection: ConnectionConfig,
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
	pub target: Target,
	pub connection: Box<ConnectionConfig>,
	pub token: Option<HeaderValue>,
	pub connect: bool,
}

/// The role this agentgateway is acting as when originating an HBONE CONNECT.
/// Will be used as the `x-istio-source` header value.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum HboneSourceRole {
	Waypoint,
	Gateway,
}

impl HboneSourceRole {
	pub fn as_header_value(self) -> &'static str {
		match self {
			HboneSourceRole::Waypoint => "waypoint",
			HboneSourceRole::Gateway => "gateway",
		}
	}

	/// Map an originating bind's tunnel protocol to the role it identifies as on
	/// outbound HBONE CONNECTs. Returns `None` when no source role applies.
	pub fn from_tunnel(tp: types::agent::TunnelProtocol) -> Option<Self> {
		use types::agent::TunnelProtocol;
		match tp {
			TunnelProtocol::HboneWaypoint => Some(HboneSourceRole::Waypoint),
			TunnelProtocol::HboneGateway => Some(HboneSourceRole::Gateway),
			TunnelProtocol::Direct | TunnelProtocol::Proxy | TunnelProtocol::Connect => None,
		}
	}
}

/// Headers added to outbound HBONE CONNECT requests so that downstream Istio
/// components can identify the originator.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct HboneHeaders {
	pub source: Option<HboneSourceRole>,
	pub forwarded_network: Strng,
	/// Baggage describing the originating workload (cluster/namespace/etc.).
	/// Only emitted on the inner CONNECT.
	pub baggage: Option<Strng>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Transport {
	Plain(ApplicationTransport),
	Tunnel(ApplicationTransport, TunnelConfig),
	Hbone(ApplicationTransport, u16, Vec<Identity>, HboneHeaders),
	DoubleHbone {
		gateway_address: SocketAddr, // Address of network gateway to connect to
		gateway_identities: Vec<Identity>, // Identities of network gateway (workload + service SANs)
		waypoint_identities: Vec<Identity>, // Identities of waypoint/workload (workload + service SANs)
		inner: ApplicationTransport,
		headers: HboneHeaders,
	},
	/// HBONE tunnel to a waypoint proxy. The CONNECT URI uses the service target
	/// (VIP or hostname) so the waypoint can route, while the physical connection
	/// goes to the waypoint address.
	HboneWaypoint {
		waypoint_address: SocketAddr, // Physical address of waypoint (IP:hbone_port)
		identities: Vec<Identity>,    // Service SANs for mTLS verification
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
			Transport::Hbone(inner, _, _, _) => inner,
			Transport::DoubleHbone { inner, .. } => inner,
			Transport::HboneWaypoint { inner, .. } => inner,
		}
	}

	pub fn skip_dns_resolution(&self) -> bool {
		// For double HBONE, we don't need to resolve the hostname locally
		// The gateway will resolve it. Use a placeholder dest (won't be used).
		// Same with Tunnel and HboneWaypoint (we connect to the waypoint address directly).
		matches!(
			self,
			Transport::DoubleHbone { .. } | Transport::Tunnel(_, _) | Transport::HboneWaypoint { .. }
		)
	}

	pub fn name(&self) -> &'static str {
		match self {
			Transport::Hbone(ApplicationTransport::Plaintext, _, _, _) => "hbone",
			Transport::Hbone(ApplicationTransport::Tls(_), _, _, _) => "hbone-tls",
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
			Transport::HboneWaypoint {
				inner: ApplicationTransport::Plaintext,
				..
			} => "hbone-waypoint",
			Transport::HboneWaypoint {
				inner: ApplicationTransport::Tls(_),
				..
			} => "hbone-waypoint-tls",
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
pub struct ConnectionConfig {
	pub transport: Transport,
	pub tcp: Option<types::backend::TCP>,
	pub max_connection_duration: Option<Duration>,
}

impl From<Transport> for ConnectionConfig {
	fn from(transport: Transport) -> Self {
		Self {
			transport,
			tcp: None,
			max_connection_duration: None,
		}
	}
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PoolKey(Target, SocketAddr, ConnectionConfig, ::http::Version);

impl agent_pool::pool::Key for PoolKey {
	fn expected_capacity(&self) -> ExpectedCapacity {
		match self.2.transport.application() {
			ApplicationTransport::Plaintext => {
				if self.3 == ::http::Version::HTTP_11 {
					ExpectedCapacity::Http1
				} else {
					ExpectedCapacity::Http2
				}
			},
			ApplicationTransport::Tls(c) => {
				let mut h2 = false;
				let mut h1 = false;
				for alpn in &c.config.alpn_protocols {
					if alpn == b"h2" {
						h2 = true
					}
					if alpn == b"http/1.1" {
						h1 = true
					}
				}
				if h1 && !h2 {
					ExpectedCapacity::Http1
				} else if h2 && !h1 {
					ExpectedCapacity::Http2
				} else {
					ExpectedCapacity::Auto
				}
			},
		}
	}

	fn shard(&self) -> usize {
		match self.1.ip() {
			std::net::IpAddr::V4(addr) => addr.octets()[3] as usize,
			std::net::IpAddr::V6(addr) => addr.segments()[7] as usize,
		}
	}

	fn connect_timeout(&self) -> Option<Duration> {
		self.2.tcp.as_ref().and_then(|tcp| tcp.connect_timeout)
	}
}

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
	h2_config: Arc<agent_hbone::H2Config>,
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
		connection: ConnectionConfig,
		http: bool,
	) -> Result<Socket, http::Error> {
		let ConnectionConfig {
			transport,
			tcp,
			max_connection_duration,
		} = connection;
		let connect_start = std::time::Instant::now();
		let transport_name = transport.name();
		let tls = match transport.application() {
			ApplicationTransport::Plaintext => None,
			ApplicationTransport::Tls(application) => Some(application.clone()),
		};
		trace!(?transport, "connecting");
		let stream = match transport {
			Transport::Plain(_) => {
				let mut backend_config = (*self.backend_config).clone();
				if let Some(tcp) = tcp.as_ref() {
					if let Some(connect_timeout) = tcp.connect_timeout {
						backend_config.connect_timeout = connect_timeout;
					}
					if let Some(keepalives) = tcp.keepalives.as_ref() {
						backend_config.keepalives = keepalives.clone();
					}
				}
				dial(&target, ep, &backend_config).await?
			},
			Transport::Tunnel(_, tcfg) if tcfg.connect || tls.is_some() || !http => {
				// Use CONNECT when required by the transport or explicitly configured.
				let proxy_dst: SocketAddr = self
					// Never skip resolution for the actually proxy itself
					.resolve_target(false, &tcfg.target)
					.await
					.map_err(crate::http::Error::new)?;
				let dest = target.to_string();
				// This is recursive but bounded: we cannot even tunnel to a tunnel
				let con = Box::pin(self.connect(tcfg.target, proxy_dst, *tcfg.connection, false)).await?;

				let con = connect_tunnel::handshake(con, &dest, tcfg.token, self.h2_config.clone())
					.await
					.map_err(crate::http::Error::new)?;
				debug!(%dest, "connected to tunnel proxy (CONNECT)");
				con
			},
			Transport::Tunnel(_, tcfg) => {
				// Tunnel case two: use absolute form for plaintext HTTP
				let proxy_dst: SocketAddr = self
					// Never skip resolution for the actually proxy itself
					.resolve_target(false, &tcfg.target)
					.await
					.map_err(crate::http::Error::new)?;
				debug!("connected to tunnel proxy (HTTP)");
				// This is recursive but bounded: we cannot even tunnel to a tunnel
				let mut socket =
					Box::pin(self.connect(tcfg.target, proxy_dst, *tcfg.connection, false)).await?;
				socket.ext_mut().insert(stream::HttpProxy);
				socket
			},
			Transport::Hbone(_, hbone_port, identities, headers) => {
				let pool = self
					.hbone_pool
					.clone()
					.ok_or_else(|| crate::http::Error::new(anyhow::anyhow!("hbone pool disabled")))?;
				hbone_tunnel::handshake(pool, ep, hbone_port, identities, headers).await?
			},

			Transport::DoubleHbone {
				gateway_address,
				gateway_identities,
				waypoint_identities,
				inner: _,
				headers,
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
					gateway_identities,
					waypoint_identities,
					headers,
				)
				.await?
			},

			Transport::HboneWaypoint {
				waypoint_address,
				identities,
				inner: _,
			} => {
				let pool = self
					.hbone_pool
					.clone()
					.ok_or_else(|| crate::http::Error::new(anyhow::anyhow!("hbone pool disabled")))?;
				hbone_tunnel::handshake_waypoint(pool, &target, waypoint_address, identities).await?
			},
		};

		// Apply application level TLS, if applicable
		let mut socket = if let Some(tls_cfg) = tls {
			tls::handshake(stream, &tls_cfg, target).await?
		} else {
			stream
		};

		let connect_dur = connect_start.elapsed();
		if let Some(m) = &self.metrics {
			let labels = metrics::ConnectLabels {
				transport: strng::RichStrng::from(transport_name).into(),
			};
			m.upstream_connect_duration
				.get_or_create(&labels)
				.observe(connect_dur.as_secs_f64());
		}

		event!(
			target: "upstream tcp",
			parent: None,
			tracing::Level::DEBUG,

			endpoint = %ep,
			transport = %transport_name,

			connect_ms = connect_dur.as_millis(),

			"connected"
		);

		if let Some(max_age) = max_connection_duration {
			socket
				.ext_mut()
				.insert(stream::ConnectionDeadline(connect_start + max_age));
		}

		socket.with_logging(LoggingMode::Upstream);
		Ok(socket)
	}

	async fn resolve_target(
		&self,
		skip_resolution: bool,
		target: &Target,
	) -> Result<SocketAddr, ProxyError> {
		let dest = match &target {
			Target::Address(addr) => *addr,
			Target::Hostname(hostname, port) => {
				if skip_resolution {
					// For double HBONE, we don't need to resolve the hostname locally
					// The gateway will resolve it. Use a placeholder dest (won't be used).
					return Ok(SocketAddr::from(([0, 0, 0, 0], 0)));
				}
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
			let PoolKey(target, ep, connection, _) =
				dst.remove::<PoolKey>().expect("pool key must be set");

			it.connect(target, ep, connection, true)
				.await
				.map(TokioIo::new)
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
		let h2_config = hbone_pool
			.as_ref()
			.map(|pool| Arc::new(pool.config().h2.clone()))
			.unwrap_or_else(|| Arc::new(agent_hbone::H2Config::default()));
		Self::new_with_h2_config(cfg, hbone_pool, h2_config, backend_config, metrics)
	}

	pub fn new_with_h2_config(
		cfg: &Config,
		hbone_pool: Option<agent_hbone::pool::WorkloadHBONEPool<hbone::WorkloadKey>>,
		h2_config: Arc<agent_hbone::H2Config>,
		backend_config: BackendConfig,
		metrics: Option<Arc<crate::metrics::Metrics>>,
	) -> Client {
		let resolver = dns::CachedResolver::new(cfg.resolver_cfg.clone(), cfg.resolver_opts.clone());
		let mut b = agent_pool::Client::<_, PoolKey>::builder(::hyper_util::rt::TokioExecutor::new());
		b.pool_timer(hyper_util::rt::tokio::TokioTimer::new());
		b.pool_idle_timeout(backend_config.pool_idle_timeout);
		b.connect_timeout(backend_config.connect_timeout);
		b.timer(hyper_util::rt::tokio::TokioTimer::new());
		b.http1_preserve_header_case(true);
		if let Some(pool_max) = backend_config.pool_max_size {
			b.pool_max_idle_per_host(pool_max);
		};
		if !backend_config.h2_keepalive_interval.is_zero() {
			b.http2_keep_alive_interval(Some(backend_config.h2_keepalive_interval));
			b.http2_keep_alive_timeout(backend_config.h2_keepalive_timeout);
			b.http2_keep_alive_while_idle(true);
		}

		let connector = Connector {
			resolver: Arc::new(resolver),
			hbone_pool,
			h2_config,
			backend_config: Arc::new(backend_config),
			metrics,
		};
		let client = b.build(connector.clone());
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
		let transport = Transport::from(if scheme == &Scheme::HTTPS {
			ApplicationTransport::Tls(http::backendtls::SYSTEM_TRUST.base_config())
		} else {
			ApplicationTransport::Plaintext
		});
		let target = Target::from((host, port));
		self
			.call(Call {
				req,
				target,
				connection: transport.into(),
			})
			.await
	}

	pub async fn call_tcp(&self, call: TCPCall) -> Result<(), ProxyError> {
		let start = std::time::Instant::now();
		let TCPCall {
			source,
			target,
			connection,
		} = call;

		let dest = self
			.connector
			.resolve_target(connection.transport.skip_dns_resolution(), &target)
			.await?;

		let transport_name = connection.transport.name();
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
			.connect(target, dest, connection, false)
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

	pub async fn connect_raw(
		&self,
		target: Target,
		connection: ConnectionConfig,
	) -> Result<Socket, ProxyError> {
		let dest = self
			.connector
			.resolve_target(connection.transport.skip_dns_resolution(), &target)
			.await?;
		self
			.connector
			.clone()
			.connect(target, dest, connection, false)
			.await
			.map_err(ProxyError::UpstreamTCPCallFailed)
	}

	pub fn call(
		&self,
		call: Call,
	) -> impl std::future::Future<Output = Result<http::Response, ProxyError>> + '_ {
		let client = &self.client;
		let connector = &self.connector;
		let start = std::time::Instant::now();
		let Call {
			mut req,
			target,
			connection,
		} = call;
		async move {
			let dest = connector
				.resolve_target(connection.transport.skip_dns_resolution(), &target)
				.await?;
			http::modify_req_uri(&mut req, |uri| {
				let scheme = connection.transport.scheme();
				// Strip the port from the hostname if its the default already
				// The hyper client does this for HTTP/1.1 but not for HTTP2
				if let Some(a) = uri.authority.as_mut()
					&& ((scheme == Scheme::HTTPS && a.port_u16() == Some(443))
						|| (scheme == Scheme::HTTP && a.port_u16() == Some(80)))
				{
					*a =
						Authority::from_str(a.host()).expect("host must be valid since it was already a host");
				}
				uri.scheme = Some(scheme);

				Ok(())
			})
			.map_err(ProxyError::Processing)?;
			if req.extensions().get::<filters::AutoHostname>().is_some() {
				req.headers_mut().remove(::http::header::HOST);
			}
			let version = req.version();
			let transport_name = connection.transport.name();
			// We are going to do a HTTP absolute form tunnel request. For CONNECT this is handled
			// in the connect layer, but here we need to merge it into the request
			if let Transport::Tunnel(app, tc) = &connection.transport
				&& let Some(h) = tc.token.as_ref()
				&& matches!(app, ApplicationTransport::Plaintext)
				&& !tc.connect
			{
				req
					.headers_mut()
					.insert(http::header::PROXY_AUTHORIZATION, h.clone());
			}
			let key = PoolKey(target.clone(), dest, connection, version);
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

				request =? req,
				extensions =? crate::http::DebugExtensions(&req)
			);
			let buffer_limit = http::buffer_limit(&req);
			let to = req.extensions().get::<BackendRequestTimeout>().cloned();
			let call = client.request(req);
			let map_error = |err: agent_pool::Error| {
				if err.is_connect_timeout() {
					ProxyError::UpstreamCallTimeout
				} else if connect_tunnel::is_stale_assignment(&err) {
					ProxyError::StaleAssignment
				} else {
					ProxyError::UpstreamCallFailed(err)
				}
			};
			let resp = if let Some(to) = to {
				match tokio::time::timeout(to.0, call).await {
					Err(_) => Err(ProxyError::UpstreamCallTimeout),
					Ok(Err(err)) => Err(map_error(err)),
					Ok(Ok(resp)) => Ok(resp),
				}
			} else {
				call.await.map_err(map_error)
			};
			let dur = format!("{}ms", start.elapsed().as_millis());
			// If version changed due to ALPN negotiation, make sure we get the real version
			let version = resp.as_ref().map(|resp| resp.version()).unwrap_or(version);
			event!(
				target: "upstream request",
				parent: None,
				tracing::Level::DEBUG,

				target = %target,
				endpoint = %dest,
				transport = %transport_name,

				http.method = %method,
				http.host = host.as_ref().map(display),
				http.path = %path,
				http.version = ?version,
				http.status = resp.as_ref().ok().map(|s| s.status().as_u16()).unwrap_or_default(),

				duration = dur,
			);
			let mut resp = resp?;

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
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::agent::TunnelProtocol;

	#[test]
	fn waypoint_bind_renders_as_waypoint() {
		assert_eq!(
			HboneSourceRole::from_tunnel(TunnelProtocol::HboneWaypoint).map(|r| r.as_header_value()),
			Some("waypoint"),
		);
	}

	#[test]
	fn gateway_bind_renders_as_gateway() {
		assert_eq!(
			HboneSourceRole::from_tunnel(TunnelProtocol::HboneGateway).map(|r| r.as_header_value()),
			Some("gateway"),
		);
	}

	#[test]
	fn non_role_tunnel_protocols_have_no_source_role() {
		for tp in [
			TunnelProtocol::Direct,
			TunnelProtocol::Proxy,
			TunnelProtocol::Connect,
		] {
			assert_eq!(
				HboneSourceRole::from_tunnel(tp),
				None,
				"tunnel protocol {tp:?} should map to no source role",
			);
		}
	}

	#[tokio::test]
	async fn max_connection_duration_sets_pool_deadline_on_connect() {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move { while listener.accept().await.is_ok() {} });

		let client = Client::new(
			&Config {
				resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
				resolver_opts: hickory_resolver::config::ResolverOpts::default(),
			},
			None,
			crate::BackendConfig::default(),
			None,
		);

		let max_age = Duration::from_secs(60);
		let before = std::time::Instant::now();
		let socket = client
			.connect_raw(
				Target::Address(addr),
				ConnectionConfig {
					transport: Transport::Plain(ApplicationTransport::Plaintext),
					tcp: None,
					max_connection_duration: Some(max_age),
				},
			)
			.await
			.expect("connect to loopback listener");
		let deadline = agent_pool::connect::Connection::connected(&socket)
			.get_valid_until()
			.expect("deadline should be set");
		assert!(deadline >= before + max_age);
		assert!(deadline <= std::time::Instant::now() + max_age);
	}

	/// Millisecond truncation put every observation on an exact multiple of 1ms, and a loopback
	/// connect on 0. Asserting the recorded value is finer than a millisecond catches that however
	/// long the connect actually takes.
	#[tokio::test]
	async fn upstream_connect_duration_records_sub_millisecond_connects() {
		use frozen_collections::FzHashSet;
		use prometheus_client::registry::Registry;

		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move { while listener.accept().await.is_ok() {} });

		let mut registry = Registry::default();
		let metrics = Arc::new(crate::metrics::Metrics::new(
			&mut registry,
			FzHashSet::default(),
			Default::default(),
		));

		let client = Client::new(
			&Config {
				resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
				resolver_opts: hickory_resolver::config::ResolverOpts::default(),
			},
			None,
			crate::BackendConfig::default(),
			Some(metrics),
		);
		client
			.connect_raw(
				Target::Address(addr),
				Transport::Plain(ApplicationTransport::Plaintext).into(),
			)
			.await
			.expect("connect to loopback listener");

		let mut encoded = String::new();
		prometheus_client::encoding::text::encode(&mut encoded, &registry).unwrap();
		let sum = encoded
			.lines()
			.find_map(|l| {
				l.strip_prefix("upstream_connect_duration_seconds_sum{transport=\"plaintext\"} ")
			})
			.unwrap_or_else(|| panic!("no connect duration sum in:\n{encoded}"))
			.parse::<f64>()
			.unwrap();
		let ms = sum * 1000.0;
		assert!(
			(ms - ms.round()).abs() > 1e-9,
			"connect duration {ms}ms is quantized to whole milliseconds"
		);
	}
}
