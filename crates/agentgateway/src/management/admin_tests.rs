use std::net::SocketAddr;
use std::sync::Arc;

use tokio::runtime::Handle;

use super::*;

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
async fn test_admin_config_dump_redacts_secrets() {
	let cfg = r#"
config:
  adminAddr: localhost:0
  tracing:
    otlpEndpoint: http://localhost:4317
    headers:
      authorization: super-secret-otlp-token
      x-custom-header: visible-value
"#;
	let (addr, _drain_tx) = spawn_admin(cfg).await;

	let resp = reqwest::get(format!("http://{addr}/config_dump"))
		.await
		.expect("request should succeed");
	assert_eq!(resp.status(), reqwest::StatusCode::OK);

	let body = resp.text().await.unwrap();
	assert!(
		!body.contains("super-secret-otlp-token"),
		"config dump must not leak authorization header value: {body}"
	);
	assert!(
		body.contains("visible-value"),
		"config dump should preserve non-sensitive header values: {body}"
	);
}
