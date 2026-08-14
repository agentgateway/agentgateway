use std::time::{Duration, Instant};

use http::{Method, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;

use crate::common::gateway::AgentGateway;

const VALID_CONFIG: &str =
	"config: {}\ngateways:\n  default:\n    port: $PORT\nui:\n  gateways: default\n";

#[tokio::test]
async fn rejected_reload_preserves_last_applied_config() -> anyhow::Result<()> {
	let gateway = AgentGateway::new(VALID_CONFIG).await?;

	let original_config = get_json(&gateway, "http://localhost/api/config").await?;
	let original_effective = get_json(&gateway, "http://localhost/api/config/effective").await?;

	let status = get_json(&gateway, "http://localhost/api/config/status").await?;
	anyhow::ensure!(
		status["lastError"].is_null(),
		"expected no reload error before corrupting the config: {status}"
	);

	tokio::fs::write(gateway.config_path(), "totallyBogusKey: true\n").await?;

	let status = wait_for_reload_error(&gateway).await?;
	anyhow::ensure!(
		status["lastError"]["message"]
			.as_str()
			.is_some_and(|message| message.contains("unknown field")),
		"expected reload error to mention the unknown field: {status}"
	);

	let config_after = get_json(&gateway, "http://localhost/api/config").await?;
	let effective_after = get_json(&gateway, "http://localhost/api/config/effective").await?;
	anyhow::ensure!(
		config_after == original_config,
		"GET /api/config should still return the last-applied config: got {config_after}, want {original_config}"
	);
	anyhow::ensure!(
		effective_after == original_effective,
		"GET /api/config/effective should still return the last-applied config: got {effective_after}, want {original_effective}"
	);

	Ok(())
}

async fn get_json(gateway: &AgentGateway, url: &str) -> anyhow::Result<Value> {
	let response = gateway.send_request(Method::GET, url).await;
	anyhow::ensure!(
		response.status() == StatusCode::OK,
		"GET {url} failed: {}",
		response.status()
	);
	let body = response.into_body().collect().await?.to_bytes();
	Ok(serde_json::from_slice(&body)?)
}

async fn wait_for_reload_error(gateway: &AgentGateway) -> anyhow::Result<Value> {
	let deadline = Instant::now() + Duration::from_secs(5);
	loop {
		let status = get_json(gateway, "http://localhost/api/config/status").await?;
		if !status["lastError"].is_null() {
			return Ok(status);
		}
		anyhow::ensure!(
			Instant::now() < deadline,
			"timed out waiting for a reload error to be recorded: {status}"
		);
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}
