use std::sync::Arc;

use agentgateway::http::{Body, Response};
use agentgateway::proxy::request_builder::RequestBuilder;
use agentgateway::yamlviajson;
use http::Method;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use serde_json::Value;
use shellexpand::LookupError;
use tempfile::TempDir;
use tracing::info;
use url::Url;

pub struct AgentGateway {
	// Used to store temp dirs so they are dropped when the test completes
	pub _temp_dirs: Vec<TempDir>,
	port: u16,
	task: tokio::task::JoinHandle<()>,
	client: Client<HttpConnector, Body>,
	shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl AgentGateway {
	pub async fn new(raw_config: impl Into<String>) -> anyhow::Result<Self> {
		agent_core::telemetry::testing::setup_test_logging();
		let port = Arc::new(std::sync::Mutex::new(0u16));
		let raw_config = raw_config.into();
		let config = shellexpand::env_with_context(&raw_config, |key| match key {
			"PORT" => futures::executor::block_on(async {
				let p = crate::common::compare::find_free_port().await?;
				*port.lock().unwrap() = p;
				Ok(Some(p.to_string()))
			}),
			_ => Ok(None),
		})
		.map_err(|e: LookupError<anyhow::Error>| anyhow::anyhow!("failed to expand env: {}", e))?;
		let mut js: Value = yamlviajson::from_str(&config).unwrap();
		let config = js.pointer_mut("/config").unwrap();
		config.as_object_mut().unwrap().insert(
			"adminAddr".to_string(),
			Value::String("127.0.0.1:0".to_string()),
		);
		config.as_object_mut().unwrap().insert(
			"statsAddr".to_string(),
			Value::String("127.0.0.1:0".to_string()),
		);
		config.as_object_mut().unwrap().insert(
			"readinessAddr".to_string(),
			Value::String("127.0.0.1:0".to_string()),
		);

		let js = serde_json::to_string(&js).unwrap();
		let mut temp_dirs = Vec::new();
		let (temp, config) = crate::common::compare::create_temp_config_file(&js).await?;
		temp_dirs.push(temp);
		info!("starting agent...");

		let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

		let task = tokio::task::spawn(async move {
			let config = agentgateway::config::parse_config(js, Some(config)).unwrap();
			let app = agentgateway::app::run(Arc::new(config)).await.unwrap();

			// Wait for either shutdown signal or termination
			tokio::select! {
				_ = shutdown_rx => {
					info!("graceful shutdown requested");
				}
				_ = app.wait_termination() => {
					info!("app terminated");
				}
			}
		});

		info!("waiting for agent...");
		let port = *port.lock().unwrap();
		crate::common::compare::wait_for_port(port).await?;
		info!("agent ready!...");
		let client = ::hyper_util::client::legacy::Client::builder(TokioExecutor::new())
			.timer(TokioTimer::new())
			.build_http();
		Ok(Self {
			_temp_dirs: Vec::new(),
			port,
			task,
			client,
			shutdown_tx: Some(shutdown_tx),
		})
	}

	pub async fn shutdown(mut self) {
		if let Some(tx) = self.shutdown_tx.take() {
			let _ = tx.send(());
			// The shutdown signal has been sent, the task will terminate gracefully
		}
		self.task.abort();
	}

	pub async fn send_request(&self, method: Method, url: &str) -> Response {
		let mut url = Url::parse(url).unwrap();
		url.set_port(Some(self.port)).unwrap();
		RequestBuilder::new(method, url)
			.send(self.client.clone())
			.await
			.unwrap()
	}

	pub fn port(&self) -> u16 {
		self.port
	}
}

impl Drop for AgentGateway {
	fn drop(&mut self) {
		// Send shutdown signal if not already sent
		if let Some(tx) = self.shutdown_tx.take() {
			let _ = tx.send(());
		}
		self.task.abort();
	}
}
