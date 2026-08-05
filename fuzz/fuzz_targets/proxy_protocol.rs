#![no_main]

use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use agentgateway::proxy::proxy_protocol::{detect_proxy_protocol, parse_proxy_protocol};
use agentgateway::types::discovery::Identity;
use agentgateway::types::frontend::ProxyVersion;
use libfuzzer_sys::fuzz_target;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

const V1_PREFIX: &[u8] = b"PROXY ";
const V1_MAX_BODY_LEN: usize = 107 - V1_PREFIX.len() - 2;
const V2_SIGNATURE: &[u8] = b"\r\n\r\n\0\r\nQUIT\n";
const V2_MAX_SUFFIX_LEN: usize = 528 - V2_SIGNATURE.len();
const V2_IPV4_ADDR_LEN: usize = 12;
const V2_TLV_HEADER_LEN: usize = 3;
const V2_MAX_IDENTITY_LEN: usize = 512 - V2_IPV4_ADDR_LEN - V2_TLV_HEADER_LEN;

static RT: Lazy<Runtime> = Lazy::new(|| {
	tokio::runtime::Builder::new_current_thread()
		.enable_io()
		.build()
		.expect("fuzz runtime should build")
});

fn trim_line_end(mut data: &[u8]) -> &[u8] {
	while matches!(data.last(), Some(b'\r' | b'\n')) {
		data = &data[..data.len() - 1];
	}
	data
}

fn v2_with_identity(identity: &[u8]) -> Vec<u8> {
	let payload_len = V2_IPV4_ADDR_LEN + V2_TLV_HEADER_LEN + identity.len();
	let mut header = Vec::with_capacity(V2_SIGNATURE.len() + 4 + payload_len);
	header.extend_from_slice(V2_SIGNATURE);
	header.extend_from_slice(&[0x21, 0x11]); // v2 PROXY, TCP over IPv4
	header.extend_from_slice(&(payload_len as u16).to_be_bytes());
	header.extend_from_slice(&[192, 168, 1, 1, 10, 0, 0, 1]);
	header.extend_from_slice(&12345u16.to_be_bytes());
	header.extend_from_slice(&8080u16.to_be_bytes());
	header.push(0xd0); // ztunnel authority/identity TLV
	header.extend_from_slice(&(identity.len() as u16).to_be_bytes());
	header.extend_from_slice(identity);
	header
}

fuzz_target!(|data: &[u8]| {
	RT.block_on(async {
		// Exercise arbitrary, possibly truncated wire bytes.
		let mut input = Cursor::new(data);
		if let Ok(Some(parsed)) = detect_proxy_protocol(&mut input, ProxyVersion::All).await {
			assert!(parsed.consumed_len <= data.len());
		}

		// Keep mutations inside a well-formed v1 envelope so the parser reaches
		// address and command handling instead of mostly rejecting the prefix.
		let line = trim_line_end(data);
		if line.len() <= V1_MAX_BODY_LEN {
			let mut v1 = Vec::with_capacity(V1_PREFIX.len() + line.len() + 2);
			v1.extend_from_slice(V1_PREFIX);
			v1.extend_from_slice(line);
			v1.extend_from_slice(b"\r\n");
			let mut input = Cursor::new(&v1);
			if let Ok(Some(parsed)) = detect_proxy_protocol(&mut input, ProxyVersion::V1).await {
				assert_eq!(parsed.consumed_len, v1.len());
			}
		}

		// Preserve the v2 signature while mutating its prelude, addresses, and TLVs.
		if data.len() <= V2_MAX_SUFFIX_LEN {
			let mut v2 = Vec::with_capacity(V2_SIGNATURE.len() + data.len());
			v2.extend_from_slice(V2_SIGNATURE);
			v2.extend_from_slice(data);
			let mut input = Cursor::new(&v2);
			if let Ok(Some(parsed)) = detect_proxy_protocol(&mut input, ProxyVersion::V2).await {
				assert!(parsed.consumed_len <= v2.len());
			}
		}

		// Always reach v2 address/TLV parsing with a valid frame. The fuzz bytes
		// become the SPIFFE identity value, including malformed UTF-8 and shapes.
		let identity = trim_line_end(data);
		if identity.len() <= V2_MAX_IDENTITY_LEN {
			let v2 = v2_with_identity(identity);
			let mut input = Cursor::new(&v2);
			let parsed = parse_proxy_protocol(&mut input, ProxyVersion::V2)
				.await
				.expect("constructed v2 frame must parse");
			assert_eq!(parsed.consumed_len, v2.len());
			assert_eq!(
				parsed.info.src_addr,
				Some(SocketAddr::new(
					IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
					12345
				))
			);
			assert_eq!(
				parsed.info.dst_addr,
				Some(SocketAddr::new(
					IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
					8080
				))
			);

			let expected_identity = std::str::from_utf8(identity)
				.ok()
				.and_then(|value| value.parse::<Identity>().ok())
				.map(|value| value.to_string());
			assert_eq!(
				parsed.info.peer_identity.map(|value| value.to_string()),
				expected_identity
			);

			let mut input = Cursor::new(&v2);
			assert!(
				parse_proxy_protocol(&mut input, ProxyVersion::V1)
					.await
					.is_err()
			);
		}
	});
});
