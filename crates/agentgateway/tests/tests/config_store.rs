use std::time::{Duration, Instant};

use http::{Method, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};

use crate::common::gateway::AgentGateway;

#[tokio::test]
async fn hybrid_resources_reload_and_survive_restart() -> anyhow::Result<()> {
	let dir = tempfile::tempdir()?;
	let database_url = format!("sqlite://{}", dir.path().join("config.db").display());
	let config = hybrid_config(&database_url);

	let gateway = AgentGateway::new(config.clone()).await?;
	let response = gateway
		.send_request_json_method(
			Method::PUT,
			"http://localhost/api/config/resources/llm.policy/notAPolicy",
			json!({"value": {}}),
		)
		.await;
	anyhow::ensure!(
		response.status() == StatusCode::UNPROCESSABLE_ENTITY,
		"unknown policy should be rejected by local config validation: {}",
		response.status()
	);
	put_model(&gateway, "db-model").await?;
	put_cors_policy(&gateway).await?;
	wait_for_models(&gateway, &["db-model"]).await?;
	gateway.shutdown().await;

	let gateway = AgentGateway::new(config).await?;
	assert_resource(&gateway, "db-model").await?;
	assert_cors_policy(&gateway).await?;
	wait_for_models(&gateway, &["db-model"]).await?;

	put_model(&gateway, "renamed-model").await?;
	assert_resource(&gateway, "renamed-model").await?;
	wait_for_models(&gateway, &["renamed-model"]).await?;

	let response = gateway
		.send_request(
			Method::DELETE,
			"http://localhost/api/config/resources/llm.model/model-e2e",
		)
		.await;
	assert_eq!(response.status(), StatusCode::OK);
	wait_for_models(&gateway, &[]).await?;

	Ok(())
}

async fn put_cors_policy(gateway: &AgentGateway) -> anyhow::Result<()> {
	let response = gateway
		.send_request_json_method(
			Method::PUT,
			"http://localhost/api/config/resources/llm.policy/cors",
			json!({
				"value": {
					"allowOrigins": ["https://example.com"],
					"allowMethods": ["POST"],
					"allowHeaders": ["*"]
				}
			}),
		)
		.await;
	let status = response.status();
	let body = response.into_body().collect().await?.to_bytes();
	anyhow::ensure!(
		status == StatusCode::OK,
		"policy upsert failed ({status}): {}",
		String::from_utf8_lossy(&body)
	);
	Ok(())
}

async fn assert_cors_policy(gateway: &AgentGateway) -> anyhow::Result<()> {
	let response = gateway
		.send_request(
			Method::GET,
			"http://localhost/api/config/resources/llm.policy",
		)
		.await;
	anyhow::ensure!(
		response.status() == StatusCode::OK,
		"policy resource list failed: {}",
		response.status()
	);
	let body = response.into_body().collect().await?.to_bytes();
	let value: Value = serde_json::from_slice(&body)?;
	anyhow::ensure!(
		value["resources"].as_array().is_some_and(|resources| {
			resources.len() == 1
				&& resources[0]["id"] == "cors"
				&& resources[0]["value"]["allowOrigins"][0] == "https://example.com"
		}),
		"persisted CORS policy did not match: {value}"
	);
	Ok(())
}

fn hybrid_config(database_url: &str) -> String {
	format!(
		r#"
config:
  database:
    url: {database_url}
  configStore:
    mode: hybrid
gateways:
  default:
    port: $PORT
ui:
  gateways: default
llm:
  gateways: default
  models: []
"#,
	)
}

async fn put_model(gateway: &AgentGateway, name: &str) -> anyhow::Result<()> {
	let response = gateway
		.send_request_json_method(
			Method::PUT,
			"http://localhost/api/config/resources/llm.model",
			json!({
				"resources": [{
					"value": {
						"id": "model-e2e",
						"name": name,
						"provider": "ollama",
						"params": {"model": "llama3"}
					}
				}]
			}),
		)
		.await;
	let status = response.status();
	let body = response.into_body().collect().await?.to_bytes();
	anyhow::ensure!(
		status == StatusCode::OK,
		"model upsert failed ({status}): {}",
		String::from_utf8_lossy(&body)
	);
	Ok(())
}

async fn assert_resource(gateway: &AgentGateway, expected_name: &str) -> anyhow::Result<()> {
	let response = gateway
		.send_request(
			Method::GET,
			"http://localhost/api/config/resources/llm.model",
		)
		.await;
	anyhow::ensure!(
		response.status() == StatusCode::OK,
		"resource list failed: {}",
		response.status()
	);
	let body = response.into_body().collect().await?.to_bytes();
	let value: Value = serde_json::from_slice(&body)?;
	anyhow::ensure!(
		value["resources"].as_array().is_some_and(|resources| {
			resources.len() == 1
				&& resources[0]["id"] == "model-e2e"
				&& resources[0]["value"]["name"] == expected_name
		}),
		"persisted resource did not match model-e2e/{expected_name}: {value}"
	);
	Ok(())
}

async fn wait_for_models(gateway: &AgentGateway, expected: &[&str]) -> anyhow::Result<()> {
	let deadline = Instant::now() + Duration::from_secs(5);
	loop {
		let response = gateway
			.send_request(Method::GET, "http://localhost/v1/models")
			.await;
		if response.status() == StatusCode::OK {
			let body = response.into_body().collect().await?.to_bytes();
			let value: Value = serde_json::from_slice(&body)?;
			let models = value["data"]
				.as_array()
				.into_iter()
				.flatten()
				.filter_map(|model| model["id"].as_str())
				.collect::<Vec<_>>();
			if models == expected {
				return Ok(());
			}
		}
		if Instant::now() >= deadline {
			anyhow::bail!("timed out waiting for models {expected:?}");
		}
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}
