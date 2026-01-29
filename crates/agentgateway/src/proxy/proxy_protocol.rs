//! PROXY protocol v2 parser for Istio sandwich waypoint mode.
//!
//! When agentgateway operates as an Istio ambient mesh waypoint in "sandwich" mode,
//! ztunnel handles mTLS termination and forwards traffic to the waypoint using
//! PROXY protocol v2. This module parses the PROXY header to extract:
//!
//! - Original source/destination addresses (standard PROXY protocol)
//! - Peer identity from TLV 0xD0 (SPIFFE URI of the source workload)
//!
//! The extracted identity flows through to CEL authorization via TLSConnectionInfo.

use std::net::SocketAddr;

use anyhow::bail;
use ppp::{HeaderResult, v2};
use tokio::io::{AsyncRead, AsyncReadExt};
use tracing::trace;

use crate::transport::tls::IstioIdentity;
use crate::types::discovery::Identity;

/// TLV type for peer identity (SPIFFE URI) - matches ztunnel's PROXY_PROTOCOL_AUTHORITY_TLV
const PROXY_PROTOCOL_AUTHORITY_TLV: u8 = 0xD0;

/// PROXY protocol v2 signature (12 bytes)
const PROXY_V2_SIGNATURE: [u8; 12] = [
	0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];

/// Minimum header size: 12 (signature) + 4 (version/command/family/length)
const PROXY_V2_MIN_HEADER: usize = 16;

/// Maximum allowed address/TLV data size.
/// IPv6 addresses need 36 bytes, plus TLVs. 512 bytes is plenty for typical use
/// (ztunnel only sends identity TLV) while preventing allocation attacks.
const PROXY_V2_MAX_ADDR_LEN: usize = 512;

/// Information extracted from PROXY protocol v2 header.
#[derive(Debug)]
pub struct ProxyProtocolInfo {
	/// Original source address of the client (before ztunnel)
	pub src_addr: SocketAddr,
	/// Original destination address (the service VIP)
	pub dst_addr: SocketAddr,
	/// Peer identity extracted from TLV 0xD0, if present
	pub peer_identity: Option<IstioIdentity>,
}

/// Parse PROXY protocol v2 header from stream.
///
/// Reads exactly the header bytes (no more) so subsequent reads get the HTTP request.
/// Uses the length field in the PROXY header to determine exact read size.
pub async fn parse_proxy_protocol<S: AsyncRead + Unpin>(
	stream: &mut S,
) -> anyhow::Result<ProxyProtocolInfo> {
	// Read the fixed 16-byte header prefix
	let mut header_prefix = [0u8; PROXY_V2_MIN_HEADER];
	stream.read_exact(&mut header_prefix).await?;

	// Verify signature
	if header_prefix[..12] != PROXY_V2_SIGNATURE {
		bail!("invalid PROXY v2 signature");
	}

	// Extract version/command (byte 12) and address length (bytes 14-15, big-endian)
	let version_cmd = header_prefix[12];
	if version_cmd >> 4 != 2 {
		bail!(
			"expected PROXY protocol v2, got version {}",
			version_cmd >> 4
		);
	}

	let addr_len = u16::from_be_bytes([header_prefix[14], header_prefix[15]]) as usize;
	if addr_len > PROXY_V2_MAX_ADDR_LEN {
		bail!(
			"PROXY v2 address/TLV length {} exceeds maximum {}",
			addr_len,
			PROXY_V2_MAX_ADDR_LEN
		);
	}
	trace!(
		addr_len,
		"PROXY v2 header indicates {} bytes of addresses/TLVs", addr_len
	);

	// Read the remaining address/TLV data
	let mut full_header = vec![0u8; PROXY_V2_MIN_HEADER + addr_len];
	full_header[..PROXY_V2_MIN_HEADER].copy_from_slice(&header_prefix);

	if addr_len > 0 {
		stream
			.read_exact(&mut full_header[PROXY_V2_MIN_HEADER..])
			.await?;
	}

	// Now parse the complete header
	let header = match HeaderResult::parse(&full_header) {
		HeaderResult::V2(Ok(h)) => h,
		HeaderResult::V2(Err(e)) => bail!("invalid PROXY v2 header: {e:?}"),
		HeaderResult::V1(_) => bail!("PROXY v1 not supported, expected v2"),
	};

	// Extract addresses
	let (src_addr, dst_addr) = match header.addresses {
		v2::Addresses::IPv4(ref a) => (
			SocketAddr::new(a.source_address.into(), a.source_port),
			SocketAddr::new(a.destination_address.into(), a.destination_port),
		),
		v2::Addresses::IPv6(ref a) => (
			SocketAddr::new(a.source_address.into(), a.source_port),
			SocketAddr::new(a.destination_address.into(), a.destination_port),
		),
		_ => bail!("unsupported PROXY protocol address family"),
	};

	// Extract peer identity from TLV 0xD0
	let peer_identity = header
		.tlvs()
		.filter_map(|t| t.ok())
		.find(|t| t.kind == PROXY_PROTOCOL_AUTHORITY_TLV)
		.and_then(|t| parse_spiffe_identity(&t.value));

	trace!(
		src = %src_addr,
		dst = %dst_addr,
		identity = ?peer_identity,
		"parsed PROXY protocol v2 header"
	);

	Ok(ProxyProtocolInfo {
		src_addr,
		dst_addr,
		peer_identity,
	})
}

/// Parse a SPIFFE URI into IstioIdentity components.
///
/// Uses the existing `Identity::FromStr` implementation for parsing,
/// then converts to `IstioIdentity` for compatibility with `TLSConnectionInfo`.
///
/// Expected format: `spiffe://trust-domain/ns/namespace/sa/service-account`
fn parse_spiffe_identity(data: &[u8]) -> Option<IstioIdentity> {
	let uri = std::str::from_utf8(data).ok()?;
	// Use existing Identity::FromStr impl (types/discovery.rs)
	let identity: Identity = uri.parse().ok()?;
	// Convert to IstioIdentity (same pattern as tls.rs:577-588)
	let Identity::Spiffe {
		trust_domain,
		namespace,
		service_account,
	} = identity;
	Some(IstioIdentity::new(trust_domain, namespace, service_account))
}

#[cfg(test)]
mod tests {
	use std::net::SocketAddrV4;

	use ppp::v2::{Builder, Command, Protocol, Version};

	use super::*;

	fn build_proxy_header(src: &str, dst: &str, identity: Option<&[u8]>) -> Vec<u8> {
		let src: SocketAddrV4 = src.parse().unwrap();
		let dst: SocketAddrV4 = dst.parse().unwrap();
		let addresses = ppp::v2::Addresses::IPv4(ppp::v2::IPv4 {
			source_address: *src.ip(),
			destination_address: *dst.ip(),
			source_port: src.port(),
			destination_port: dst.port(),
		});
		let mut builder =
			Builder::with_addresses(Version::Two | Command::Proxy, Protocol::Stream, addresses);
		if let Some(id) = identity {
			builder = builder.write_tlv(PROXY_PROTOCOL_AUTHORITY_TLV, id).unwrap();
		}
		builder.build().unwrap()
	}

	#[test]
	fn test_parse_spiffe_identity() {
		let cases = [
			(b"spiffe://cluster.local/ns/default/sa/svc".as_slice(), true),
			(b"spiffe://cluster.local/ns/default", false), // missing sa
			(b"https://example.com", false),               // wrong scheme
			(&[0xff, 0xfe][..], false),                    // invalid UTF-8
			(b"spiffe://cluster.local/ns/default/sa/svc/extra", false), // extra segment
			(b"spiffe://cluster.local/namespace/default/sa/svc", false), // wrong marker
		];
		for (input, should_parse) in cases {
			assert_eq!(
				parse_spiffe_identity(input).is_some(),
				should_parse,
				"{input:?}"
			);
		}
	}

	#[tokio::test]
	async fn test_parse_proxy_protocol() {
		let header = build_proxy_header("192.168.1.1:12345", "10.0.0.1:8080", None);
		let mut data = header.clone();
		data.extend_from_slice(b"GET / HTTP/1.1\r\n"); // trailing HTTP

		let mut cursor = std::io::Cursor::new(data);
		let info = parse_proxy_protocol(&mut cursor).await.unwrap();

		assert_eq!(info.src_addr.to_string(), "192.168.1.1:12345");
		assert_eq!(info.dst_addr.to_string(), "10.0.0.1:8080");
		assert!(info.peer_identity.is_none());
		assert_eq!(cursor.position() as usize, header.len()); // didn't consume HTTP
	}

	#[tokio::test]
	async fn test_parse_proxy_protocol_with_identity() {
		let header = build_proxy_header(
			"192.168.1.1:12345",
			"10.0.0.1:8080",
			Some(b"spiffe://cluster.local/ns/default/sa/my-service"),
		);

		let mut cursor = std::io::Cursor::new(header);
		let info = parse_proxy_protocol(&mut cursor).await.unwrap();

		assert_eq!(info.src_addr.to_string(), "192.168.1.1:12345");
		assert_eq!(info.dst_addr.to_string(), "10.0.0.1:8080");
		assert_eq!(
			info.peer_identity.unwrap().to_string(),
			"spiffe://cluster.local/ns/default/sa/my-service"
		);
	}
}
