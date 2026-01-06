use crate::http::Error;
use crate::transport::hbone::WorkloadKey;
use crate::transport::stream::Socket;
use crate::transport::{hbone, stream};
use crate::types::agent::Target;
use crate::types::discovery::Identity;
use http::Uri;
use http::uri::Scheme;
use std::net::SocketAddr;
use std::sync::Arc;

pub async fn handshake(
	mut hbone_pool: agent_hbone::pool::WorkloadHBONEPool<hbone::WorkloadKey>,
	ep: SocketAddr,
	identity: Identity,
) -> Result<Socket, Error> {
	let uri = Uri::builder()
		.scheme(Scheme::HTTPS)
		.authority(ep.to_string())
		.path_and_query("/")
		.build()
		.expect("static builder must be accepted");
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

	let upgraded = Box::pin(hbone_pool.send_request_pooled(&pool_key, req))
		.await
		.map_err(crate::http::Error::new)?;
	let rw = agent_hbone::RWStream {
		stream: upgraded,
		buf: Default::default(),
		drain_tx: None,
	};
	Ok(Socket::from_hbone(
		Arc::new(stream::Extension::new()),
		pool_key.dst,
		rw,
	))
}

pub async fn handshake_double(
	pool: agent_hbone::pool::WorkloadHBONEPool<hbone::WorkloadKey>,
	target: &Target,
	ep: SocketAddr,
	gateway_address: SocketAddr,
	gateway_identity: Identity,
	waypoint_identity: Identity,
) -> Result<Socket, Error> {
	tracing::debug!(
		"will use DOUBLE HBONE: gateway {} -> workload {}",
		gateway_address,
		ep
	);

	// Create outer HBONE connection to network gateway
	// The outer HBONE CONNECT request uses the service hostname (target) as the authority
	// This tells the gateway what service we want to reach
	let outer_uri = Uri::builder()
		.scheme(Scheme::HTTPS)
		.authority(match &target {
			Target::Hostname(host, port) => format!("{}:{}", host, port),
			Target::Address(addr) => addr.to_string(),
			Target::UnixSocket(_) => {
				// This should be unreachable - Unix sockets are handled above
				unreachable!("Unix sockets should not reach DoubleHbone connection path")
			},
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
	let mut sender = agent_hbone::client::spawn_connection(hbone_cfg, tls_stream, drain_rx, wl_key)
		.await
		.map_err(crate::http::Error::new)?;

	// For inner HBONE, use the target (hostname or IP), not ep (which may be a placeholder)
	let inner_authority = match &target {
		Target::Hostname(host, port) => format!("{}:{}", host, port),
		Target::Address(addr) => addr.to_string(),
		Target::UnixSocket(_) => {
			// This should be unreachable - Unix sockets are handled above
			unreachable!("Unix sockets should not reach DoubleHbone connection path")
		},
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

	Ok(Socket::from_hbone(
		Arc::new(stream::Extension::new()),
		ep,
		final_rw,
	))
}
