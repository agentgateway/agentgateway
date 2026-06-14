use std::time::Instant;

use super::*;

fn request_from(peer: &str) -> ::http::Request<()> {
	let mut req = ::http::Request::builder()
		.uri("http://localhost:15000/config_dump")
		.body(())
		.unwrap();
	req.extensions_mut().insert(TCPConnectionInfo {
		peer_addr: peer.parse().unwrap(),
		local_addr: "127.0.0.1:15000".parse().unwrap(),
		start: Instant::now(),
		raw_peer_addr: None,
	});
	req
}

#[test]
fn test_admin_default_allowlist_is_loopback_only() {
	let allowed = crate::defaults::admin_allowed_ips();
	assert!(peer_ip_allowed(&allowed, &request_from("127.0.0.1:12345")));
	assert!(peer_ip_allowed(&allowed, &request_from("[::1]:12345")));
	assert!(!peer_ip_allowed(&allowed, &request_from("10.0.0.1:12345")));
	assert!(!peer_ip_allowed(
		&allowed,
		&request_from("192.168.1.50:12345")
	));
}

async fn spawn_admin(cfg: &str) -> (SocketAddr, agent_core::drain::DrainTrigger) {
	let config = Arc::new(crate::config::parse_config(cfg.to_string(), None).unwrap());
	let stores = crate::store::Stores::new(config.ipv6_enabled, config.threading_mode);
	let shutdown = signal::Shutdown::new();
	let (drain_tx, drain_rx) = agent_core::drain::new();
	let svc = Service::new(
		config,
		crate::llm::cost::ModelCatalog::empty(),
		stores,
		shutdown.trigger(),
		drain_rx,
		Handle::current(),
	)
	.await
	.expect("admin server should bind");
	let addr = svc.address().expect("admin server should have an address");
	svc.spawn();
	(addr, drain_tx)
}

#[tokio::test]
async fn test_admin_config_dump_allowed_from_loopback_and_redacts_secrets() {
	let cfg = r#"
config:
  adminAddr: localhost:0
  tracing:
    otlpEndpoint: http://localhost:4317
    headers:
      authorization: super-secret-otlp-token
"#;
	let (addr, _drain_tx) = spawn_admin(cfg).await;

	// With the default allowlist, requests from loopback are permitted
	let resp = reqwest::get(format!("http://{addr}/config_dump"))
		.await
		.expect("request should succeed");
	assert_eq!(resp.status(), reqwest::StatusCode::OK);

	// The dump must not contain secret material such as OTLP credentials
	let body = resp.text().await.unwrap();
	assert!(
		!body.contains("super-secret-otlp-token"),
		"config dump must not leak secrets: {body}"
	);
}

#[tokio::test]
async fn test_admin_rejects_peer_outside_allowlist_with_403() {
	// The allowlist replaces the loopback default, so a loopback client is now disallowed,
	// demonstrating end-to-end that non-allowlisted peers receive 403 (not 404)
	let cfg = r#"
config:
  adminAddr: localhost:0
  adminAllowedIps: ["10.0.0.1"]
"#;
	let (addr, _drain_tx) = spawn_admin(cfg).await;

	let resp = reqwest::get(format!("http://{addr}/config_dump"))
		.await
		.expect("request should succeed");
	assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
}
