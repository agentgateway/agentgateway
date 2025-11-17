mod dns;
mod hyperrustls;

use std::str::FromStr;
use std::task;

use ::http::Uri;
use ::http::uri::{Authority, Scheme};
use async_trait::async_trait;
use azure_core::error::ResultExt;
use azure_core::http::BufResponse;
use futures::TryStreamExt;
use http_body_util::BodyExt;
use hyper_util_fork::rt::TokioIo;
use rustls_pki_types::{DnsName, ServerName};
use tonic::codegen::Service;
use tracing::event;
use typespec_client_core::http::Sanitizer;

use crate::http::backendtls::VersionedBackendTLS;
use crate::http::filters;
use crate::proxy::ProxyError;
use crate::transport::hbone::WorkloadKey;
use crate::transport::stream::{LoggingMode, Socket};
use crate::transport::{hbone, stream};
use crate::types::agent::Target;
use crate::*;

#[derive(Clone)]
pub struct Client {
	resolver: Arc<dns::CachedResolver>,
	client: hyper_util_fork::client::legacy::Client<Connector, http::Body, PoolKey>,
	connector: Connector,
}

impl Debug for Client {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Client").finish()
	}
}

#[async_trait]
impl azure_core::http::HttpClient for Client {
	async fn execute_request(
		&self,
		request: &azure_core::http::Request,
	) -> azure_core::Result<BufResponse> {
		let url = request.url().clone();
		let method = request.method();
		let mut req = ::http::Request::builder();
		req = req.method(from_method(method)?).uri(url.as_str());
		for (name, value) in request.headers().iter() {
			req = req.header(name.as_str(), value.as_str());
		}
		let body = request.body().clone();

		let request = match body {
			azure_core::http::Body::Bytes(bytes) => req.body(crate::http::Body::from(bytes)),

			// We cannot currently implement `Body::SeekableStream` for WASM
			// because `reqwest::Body::wrap_stream()` is not implemented for WASM.
			#[cfg(not(target_arch = "wasm32"))]
			azure_core::http::Body::SeekableStream(seekable_stream) => {
				req.body(crate::http::Body::from_stream(seekable_stream))
			},
		}
		.map_err(|e| {
			azure_core::Error::full(
				azure_core::error::ErrorKind::Other,
				e,
				"failed to build `agentgateway::client::Client` request",
			)
		})?;

		debug!(
			"performing request {method} '{}' with `agentgateway::client::Client`",
			url.sanitize(&typespec_client_core::http::DEFAULT_ALLOWED_QUERY_PARAMETERS)
		);
		let rsp = self
			.call(Call {
				req: request,
				target: match url.host().expect("url must have a host") {
					url::Host::Domain(h) => Target::try_from((h, url.port_or_known_default().unwrap_or(80)))
						.map_err(|e| {
							azure_core::Error::full(
								azure_core::error::ErrorKind::Other,
								e,
								"failed to parse host for `agentgateway::client::Client` request",
							)
						})?,
					url::Host::Ipv4(ip) => Target::Address(SocketAddr::from((
						ip,
						url.port_or_known_default().unwrap_or(80),
					))),
					url::Host::Ipv6(ip) => Target::Address(SocketAddr::from((
						ip,
						url.port_or_known_default().unwrap_or(80),
					))),
				},
				transport: if url.scheme() == "https" {
					Transport::Tls(http::backendtls::SYSTEM_TRUST.base_config())
				} else {
					Transport::Plaintext
				},
			})
			.await
			.map_err(|e| {
				error!("request failed: {e}");
				azure_core::Error::full(
					azure_core::error::ErrorKind::Io,
					e,
					"failed to execute `agentgateway::client::Client` request",
				)
			})?;

		let status = rsp.status();
		let headers = to_headers(rsp.headers());

		let body: azure_core::http::response::PinnedStream =
			Box::pin(rsp.into_data_stream().map_err(|error| {
				azure_core::Error::full(
					azure_core::error::ErrorKind::Io,
					error,
					"error converting `reqwest` request into a byte stream",
				)
			}));

		Ok(BufResponse::new(status.as_u16().into(), headers, body))
	}
}

fn from_method(method: azure_core::http::Method) -> azure_core::Result<http::Method> {
	match method {
		azure_core::http::Method::Get => Ok(http::Method::GET),
		azure_core::http::Method::Head => Ok(http::Method::HEAD),
		azure_core::http::Method::Post => Ok(http::Method::POST),
		azure_core::http::Method::Put => Ok(http::Method::PUT),
		azure_core::http::Method::Delete => Ok(http::Method::DELETE),
		azure_core::http::Method::Patch => Ok(http::Method::PATCH),
		_ => {
			http::Method::from_str(method.as_str()).map_kind(azure_core::error::ErrorKind::DataConversion)
		},
	}
}

fn to_headers(map: &::http::HeaderMap) -> azure_core::http::headers::Headers {
	let map = map
		.iter()
		.filter_map(|(k, v)| {
			let key = k.as_str();
			if let Ok(value) = v.to_str() {
				Some((
					azure_core::http::headers::HeaderName::from(key.to_owned()),
					azure_core::http::headers::HeaderValue::from(value.to_owned()),
				))
			} else {
				warn!("header value for `{key}` is not utf8");
				None
			}
		})
		.collect::<HashMap<_, _>>();
	azure_core::http::headers::Headers::from(map)
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
pub enum Transport {
	#[default]
	Plaintext,
	Tls(VersionedBackendTLS),
	Hbone(Option<VersionedBackendTLS>, Identity),
	DoubleHbone {
		gateway_address: SocketAddr, // Address of network gateway to connect to
		gateway_identity: Identity,  // Identity of network gateway
		waypoint_identity: Identity, // Identity of waypoint/workload
		inner_tls: Option<VersionedBackendTLS>,
	},
}
impl Transport {
	pub fn name(&self) -> &'static str {
		match self {
			Transport::Plaintext => "plaintext",
			Transport::Tls(_) => "tls",
			Transport::Hbone(_, _) => "hbone",
			Transport::DoubleHbone { .. } => "doublehbone",
		}
	}
}

impl From<Option<VersionedBackendTLS>> for Transport {
	fn from(tls: Option<VersionedBackendTLS>) -> Self {
		if let Some(tls) = tls {
			client::Transport::Tls(tls)
		} else {
			client::Transport::Plaintext
		}
	}
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct PoolKey(Target, SocketAddr, Transport, ::http::Version);

impl Transport {
	pub fn scheme(&self) -> Scheme {
		match self {
			Transport::Plaintext => Scheme::HTTP,
			// TODO: make sure this is right, envoy had all sorts of issues around this.
			Transport::Tls(_) => Scheme::HTTPS,
			Transport::Hbone(inner, _) => {
				if inner.is_some() {
					Scheme::HTTPS
				} else {
					// It is a tunnel, so the fact its HTTPS is transparent!
					Scheme::HTTP
				}
			},
			Transport::DoubleHbone { inner_tls, .. } => {
				if inner_tls.is_some() {
					Scheme::HTTPS
				} else {
					// Double tunnel, so HTTPS is transparent!
					Scheme::HTTP
				}
			},
		}
	}
}

#[derive(Debug, Clone)]
struct Connector {
	hbone_pool: Option<agent_hbone::pool::WorkloadHBONEPool<hbone::WorkloadKey>>,
	backend_config: Arc<crate::BackendConfig>,
	metrics: Option<Arc<crate::metrics::Metrics>>,
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
		let mut socket = match transport {
			Transport::Plaintext => Socket::dial(ep, self.backend_config.clone())
				.await
				.map_err(crate::http::Error::new)?,
			Transport::Tls(tls) => {
				let server_name = if let Some(h) = tls.hostname_override {
					h
				} else {
					match target {
						Target::Address(_) => ServerName::IpAddress(ep.ip().into()),
						Target::Hostname(host, _) => ServerName::DnsName(
							DnsName::try_from(host.to_string()).expect("TODO: hostname conversion failed"),
						),
					}
				};

				let mut tls = self::hyperrustls::TLSConnector {
					tls_config: tls.config.clone(),
					server_name,
					backend_config: self.backend_config.clone(),
				};

				tls.call(ep).await.map_err(crate::http::Error::new)?
			},
			Transport::Hbone(inner, identity) => {
				if inner.is_some() {
					return Err(crate::http::Error::new(anyhow::anyhow!(
						"todo: inner TLS is not currently supported"
					)));
				}
				let uri = Uri::builder()
					.scheme(Scheme::HTTPS)
					.authority(ep.to_string())
					.path_and_query("/")
					.build()
					.expect("todo");
				tracing::debug!("will use HBONE");
				let req = ::http::Request::builder()
					.uri(uri)
					.method(hyper::Method::CONNECT)
					.version(hyper::Version::HTTP_2)
					.body(())
					.expect("builder with known status code should not fail");

				let pool_key = Box::new(WorkloadKey {
					dst_id: vec![identity],
					dst: SocketAddr::from((ep.ip(), 15008)),
				});
				let mut pool = self
					.hbone_pool
					.clone()
					.ok_or_else(|| crate::http::Error::new(anyhow::anyhow!("hbone pool disabled")))?;

				let upgraded = Box::pin(pool.send_request_pooled(&pool_key, req))
					.await
					.map_err(crate::http::Error::new)?;
				let rw = agent_hbone::RWStream {
					stream: upgraded,
					buf: Default::default(),
					drain_tx: None,
				};
				Socket::from_hbone(Arc::new(stream::Extension::new()), pool_key.dst, rw)
			},
			Transport::DoubleHbone {
				gateway_address,
				gateway_identity,
				waypoint_identity,
				inner_tls,
			} => {
				if inner_tls.is_some() {
					return Err(crate::http::Error::new(anyhow::anyhow!(
						"todo: inner TLS after double hbone is not currently supported"
					)));
				}

				tracing::debug!(
					"will use DOUBLE HBONE: gateway {} -> workload {}",
					gateway_address,
					ep
				);

				// Fetch the pool once and reuse throughout this branch
				let pool = self.hbone_pool.as_ref().ok_or_else(|| {
					crate::http::Error::new(anyhow::anyhow!("hbone pool required for double hbone"))
				})?;

				// Create outer HBONE connection to network gateway
				// The outer HBONE CONNECT request uses the service hostname (target) as the authority
				// This tells the gateway what service we want to reach
				let outer_uri = Uri::builder()
					.scheme(Scheme::HTTPS)
					.authority(match &target {
						Target::Hostname(host, port) => format!("{}:{}", host, port),
						Target::Address(addr) => addr.to_string(),
					})
					.path_and_query("/")
					.build()
					.expect("uri build should not fail");
				let outer_req = ::http::Request::builder()
					.uri(outer_uri)
					.method(hyper::Method::CONNECT)
					.version(hyper::Version::HTTP_2)
					.body(())
					.expect("builder with known status code should not fail");

				// Connect to the network gateway at its HBONE port
				let outer_pool_key = Box::new(WorkloadKey {
					dst_id: vec![gateway_identity.clone()],
					dst: gateway_address,
				});
				let mut pool_clone = pool.clone();

				let outer_upgraded = Box::pin(pool_clone.send_request_pooled(&outer_pool_key, outer_req))
					.await
					.map_err(crate::http::Error::new)?;

				// Wrap upgraded to implement tokio's Async{Write,Read}
				let outer_rw = agent_hbone::RWStream {
					stream: outer_upgraded,
					buf: Default::default(),
					drain_tx: None,
				};

				// For the inner one, we do it manually to avoid connection pooling.
				// Otherwise, we would only ever reach one workload in the remote cluster.
				// We also need to abort tasks the right way to get graceful terminations.
				let wl_key = WorkloadKey {
					dst_id: vec![waypoint_identity.clone()],
					dst: ep,
				};

				// Use the pool's certificate fetcher to get TLS config for the waypoint
				let tls_config = pool
					.fetch_certificate(WorkloadKey {
						dst_id: vec![waypoint_identity.clone()],
						dst: ep,
					})
					.await
					.map_err(crate::http::Error::new)?;

				let tls_connector = tokio_rustls::TlsConnector::from(tls_config);

				// Use dummy value for domain because server name verification is not performed in this context.
				let tls_stream = tls_connector
					.connect(
						rustls_pki_types::ServerName::IpAddress(std::net::Ipv4Addr::new(0, 0, 0, 0).into()),
						outer_rw,
					)
					.await
					.map_err(crate::http::Error::new)?;

				// Spawn inner CONNECT tunnel
				let (drain_tx, drain_rx) = tokio::sync::watch::channel(false);
				let hbone_cfg = pool.config();
				let mut sender =
					agent_hbone::client::spawn_connection(hbone_cfg, tls_stream, drain_rx, wl_key)
						.await
						.map_err(crate::http::Error::new)?;

				// For inner HBONE, use the target (hostname or IP), not ep (which may be a placeholder)
				let inner_authority = match &target {
					Target::Hostname(host, port) => format!("{}:{}", host, port),
					Target::Address(addr) => addr.to_string(),
				};
				let inner_uri = Uri::builder()
					.scheme(Scheme::HTTPS)
					.authority(inner_authority)
					.path_and_query("/")
					.build()
					.expect("uri build should not fail");
				let inner_req = ::http::Request::builder()
					.uri(inner_uri)
					.method(hyper::Method::CONNECT)
					.version(hyper::Version::HTTP_2)
					.body(())
					.expect("builder with known status code should not fail");

				let inner_upgraded = sender
					.send_request(inner_req)
					.await
					.map_err(crate::http::Error::new)?;

				let final_rw = agent_hbone::RWStream {
					stream: inner_upgraded,
					buf: Default::default(),
					drain_tx: Some(drain_tx),
				};

				Socket::from_hbone(Arc::new(stream::Extension::new()), ep, final_rw)
			},
		};

		let connect_ms = connect_start.elapsed().as_millis();
		if let Some(m) = &self.metrics {
			let labels = crate::telemetry::metrics::ConnectLabels {
				transport: agent_core::strng::RichStrng::from(transport_name).into(),
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
			hbone_pool,
			backend_config: Arc::new(backend_config),
			metrics,
		};
		let client = b.build_with_pool_key(connector.clone());
		Client {
			resolver: Arc::new(resolver),
			client,
			connector,
		}
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
			Transport::Tls(http::backendtls::SYSTEM_TRUST.base_config())
		} else {
			Transport::Plaintext
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
		// For double HBONE, we don't need to resolve the hostname locally
		// The gateway will resolve it. Use a placeholder dest (won't be used).
		let dest = match (&target, &transport) {
			(Target::Address(addr), _) => *addr,
			(
				Target::Hostname(hostname, _port),
				Transport::DoubleHbone {
					gateway_address, ..
				},
			) => {
				// Don't resolve hostname for double HBONE - gateway will handle it
				tracing::debug!(
					hostname=%hostname,
					"skipping DNS resolution for double hbone, gateway will resolve"
				);
				*gateway_address // Placeholder, won't be used for actual connection
			},
			(Target::Hostname(hostname, port), _) => {
				// For non-double-HBONE, resolve hostname locally
				let ip = self
					.resolver
					.resolve(hostname.clone())
					.await
					.map_err(|_| ProxyError::DnsResolution)?;
				SocketAddr::from((ip, *port))
			},
		};

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
		// For double HBONE, we don't need to resolve the hostname locally
		// The gateway will resolve it. Use a placeholder dest (won't be used).
		let dest = match (&target, &transport) {
			(Target::Address(addr), _) => *addr,
			(
				Target::Hostname(hostname, _port),
				Transport::DoubleHbone {
					gateway_address, ..
				},
			) => {
				// Don't resolve hostname for double HBONE - gateway will handle it
				tracing::debug!(
					hostname=%hostname,
					"skipping DNS resolution for double hbone (HTTP), gateway will resolve"
				);
				*gateway_address // Placeholder, won't be used for actual connection
			},
			(Target::Hostname(hostname, port), _) => {
				// For non-double-HBONE, resolve hostname locally
				let ip = self
					.resolver
					.resolve(hostname.clone())
					.await
					.map_err(|_| ProxyError::DnsResolution)?;
				SocketAddr::from((ip, *port))
			},
		};
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
		let resp = self
			.client
			.request(req)
			.await
			.map_err(ProxyError::UpstreamCallFailed);
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
		resp
			.extensions_mut()
			.insert(transport::BufferLimit::new(buffer_limit));
		Ok(resp)
	}
}
