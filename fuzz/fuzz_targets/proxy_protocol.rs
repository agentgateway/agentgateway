#![no_main]

use std::io::Cursor;

use agentgateway::proxy::proxy_protocol::detect_proxy_protocol;
use agentgateway::types::frontend::ProxyVersion;
use libfuzzer_sys::fuzz_target;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

const V1_PREFIX: &[u8] = b"PROXY ";
const V1_MAX_BODY_LEN: usize = 107 - V1_PREFIX.len() - 2;
const V2_SIGNATURE: &[u8] = b"\r\n\r\n\0\r\nQUIT\n";
const V2_MAX_SUFFIX_LEN: usize = 528 - V2_SIGNATURE.len();

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
	});
});
