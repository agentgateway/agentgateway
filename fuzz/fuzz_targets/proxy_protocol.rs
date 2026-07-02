#![no_main]

use std::io::Cursor;

use agentgateway::proxy::proxy_protocol::detect_proxy_protocol;
use agentgateway::types::frontend::ProxyVersion;
use libfuzzer_sys::fuzz_target;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

static RT: Lazy<Runtime> = Lazy::new(|| {
	tokio::runtime::Builder::new_current_thread()
		.enable_io()
		.build()
		.expect("fuzz runtime should build")
});

fuzz_target!(|data: &[u8]| {
	RT.block_on(async {
		let mut input = Cursor::new(data);
		let _ = detect_proxy_protocol(&mut input, ProxyVersion::All).await;
	});
});
