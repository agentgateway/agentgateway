use crate::http::backendtls::VersionedBackendTLS;
use crate::transport::stream::Socket;
use crate::types::agent::Target;
use itertools::Itertools;
use rustls_pki_types::{DnsName, ServerName};
use tokio_rustls::TlsConnector;
use tracing::debug;

pub async fn handshake(
	tcp: Socket,
	cfg: &VersionedBackendTLS,
	target: Target,
) -> Result<Socket, crate::http::Error> {
	let server_name = if let Some(h) = cfg.hostname_override.clone() {
		h
	} else {
		match target {
			Target::Address(addr) => ServerName::IpAddress(addr.ip().into()),
			Target::Hostname(host, _) => ServerName::DnsName(
				DnsName::try_from(host.to_string()).expect("TODO: hostname conversion failed"),
			),
			Target::UnixSocket(_) => {
				// Use a dummy IP address here; there is no "ServerName" for UDS.
				ServerName::IpAddress(std::net::Ipv4Addr::new(0, 0, 0, 0).into())
			},
		}
	};

	debug!(hostname=?server_name,
			alpn=?cfg.config.alpn_protocols.iter().map(|bytes| String::from_utf8_lossy(bytes.as_slice())).collect_vec(),
			"connecting tls");

	let (ext, counter, tcp) = tcp.into_parts();
	let tls = TlsConnector::from(cfg.config.clone())
		.connect(server_name, Box::new(tcp))
		.await
		.map_err(crate::http::Error::new)?;
	let socket = Socket::from_tls(ext, counter, tls.into()).map_err(crate::http::Error::new)?;
	Ok(socket)
}
