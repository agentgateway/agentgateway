use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use itertools::Itertools;
use reqwest::Client;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use wiremock::MockServer;

/// Test framework for comparing agentgateway against Envoy
pub struct ProxyComparisonTest {
	pub backend_server: MockServer,
	pub agentgateway_task: tokio::task::JoinHandle<()>,
	pub agentgateway_port: u16,
	pub envoy_process: Child,
	pub envoy_port: u16,
	// Used to store temp dirs so they are dropped when the test completes
	pub _temp_dirs: Vec<TempDir>,
}

impl ProxyComparisonTest {
	pub fn should_run() -> bool {
		if which::which("envoy").is_ok() {
			true
		} else {
			warn!("skipping test, 'envoy' not found");
			false
		}
	}
	pub async fn new() -> anyhow::Result<Self> {
		let backend_server = MockServer::start().await;
		let backend_port = backend_server.address().port();
		let agentgateway_port = find_free_port().await?;
		let mut temp_dirs = Vec::new();
		let agentgateway_task =
			Self::start_agentgateway(&mut temp_dirs, agentgateway_port, backend_port).await?;
		let envoy_port = find_free_port().await?;
		let envoy_process = Self::start_envoy(&mut temp_dirs, envoy_port, backend_port).await?;
		Ok(Self {
			backend_server,
			agentgateway_task,
			agentgateway_port,
			envoy_process,
			envoy_port,
			_temp_dirs: temp_dirs,
		})
	}

	/// Start agentgateway process
	async fn start_agentgateway(
		temp_dirs: &mut Vec<TempDir>,
		port: u16,
		backend_port: u16,
	) -> Result<JoinHandle<()>> {
		let config = format!(
			r#"config: {{}}
binds:
- port: {port}
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: default
      backends:
        - host: 127.0.0.1:{backend_port}
"#,
		);
		let (temp, config) = create_temp_config_file(&config).await?;
		temp_dirs.push(temp);
		info!("starting agent...");
		let task = tokio::task::spawn(async {
			let config = agentgateway::config::parse_config("{}".to_string(), Some(config)).unwrap();
			agentgateway::app::run(Arc::new(config))
				.await
				.unwrap()
				.wait_termination()
				.await
				.unwrap()
		});

		info!("waiting for agent...");
		wait_for_port(port).await?;
		info!("agent ready!...");

		Ok(task)
	}

	/// Start Envoy process
	async fn start_envoy(
		temp_dirs: &mut Vec<TempDir>,
		port: u16,
		backend_port: u16,
	) -> Result<Child> {
		let config = format!(
			r#"static_resources:
  listeners:
  - address:
      socket_address:
        address: 0.0.0.0
        port_value: {port}
    filter_chains:
    - filters:
      - name: envoy.http_connection_manager
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          codec_type: auto
          stat_prefix: ingress_http
          route_config:
            name: local_route
            virtual_hosts:
            - name: host-one # prefix route
              domains:
              - "*"
              routes:
              - match:
                  prefix: "/"
                route:
                  cluster: mock
          http_filters:
          - name: envoy.router
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router
              suppress_envoy_headers: true
  clusters:
  - name: mock
    connect_timeout: 5s
    type: STATIC
    lb_policy: round_robin
    load_assignment:
      cluster_name: mock
      endpoints:
      - lb_endpoints:
        - endpoint:
            address:
              socket_address:
                address: 127.0.0.1
                port_value: {backend_port}"#
		);
		let (temp, config_file) = create_temp_config_file(&config).await?;
		temp_dirs.push(temp);

		let mut cmd = Command::new("envoy");
		cmd
			.args([
				"-c",
				&config_file.to_string_lossy(),
				"--disable-hot-restart",
			])
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());

		let child = cmd.spawn()?;
		// Wait for Envoy to start
		wait_for_port(port).await?;

		Ok(child)
	}

	/// Send the same request to both proxies and compare responses
	pub async fn compare_request(
		&self,
		method: &str,
		path: &str,
		headers: Option<HashMap<String, String>>,
		body: Option<&str>,
	) -> Result<ProxyComparison> {
		let client = Client::new();

		// Send request to agentgateway
		let agentgateway_response = self
			.send_request(
				&client,
				self.agentgateway_port,
				method,
				path,
				headers.clone(),
				body,
			)
			.await?;

		// Send request to Envoy
		let envoy_response = self
			.send_request(&client, self.envoy_port, method, path, headers, body)
			.await?;

		Ok(ProxyComparison {
			agentgateway: agentgateway_response,
			envoy: envoy_response,
		})
	}

	/// Send a request to a specific proxy
	async fn send_request(
		&self,
		client: &Client,
		port: u16,
		method: &str,
		path: &str,
		headers: Option<HashMap<String, String>>,
		body: Option<&str>,
	) -> Result<ProxyResponse> {
		let url = format!("http://localhost:{port}{path}");
		let mut request_builder = match method.to_uppercase().as_str() {
			"GET" => client.get(&url),
			"POST" => client.post(&url),
			"PUT" => client.put(&url),
			"DELETE" => client.delete(&url),
			"PATCH" => client.patch(&url),
			_ => return Err(anyhow::anyhow!("Unsupported HTTP method: {}", method)),
		};

		if let Some(headers) = headers {
			for (key, value) in headers {
				request_builder = request_builder.header(key, value);
			}
		}

		if let Some(body) = body {
			request_builder = request_builder.body(body.to_string());
		}

		let response = request_builder.send().await?;
		let status = response.status();
		let headers: HashMap<String, String> = response
			.headers()
			.iter()
			.map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
			.collect();
		let body_bytes = response.bytes().await?;
		let body_text = String::from_utf8_lossy(&body_bytes).to_string();

		Ok(ProxyResponse {
			status,
			headers,
			body: body_text,
		})
	}

	/// Stop both proxies
	pub fn stop(&mut self) {
		self.agentgateway_task.abort();
		let _ = self.envoy_process.kill();
	}
}

impl Drop for ProxyComparisonTest {
	fn drop(&mut self) {
		self.stop();
	}
}

/// Response from a proxy
#[derive(Debug, Clone)]
pub struct ProxyResponse {
	pub status: reqwest::StatusCode,
	pub headers: HashMap<String, String>,
	pub body: String,
}

/// Comparison between agentgateway and Envoy responses
#[derive(Debug)]
pub struct ProxyComparison {
	pub agentgateway: ProxyResponse,
	pub envoy: ProxyResponse,
}

impl ProxyComparison {
	/// Assert that both proxies returned the same status code
	pub fn assert_same_status(&self) -> Result<()> {
		assert_eq!(
			self.agentgateway.status, self.envoy.status,
			"Status codes differ: agentgateway={}, envoy={}",
			self.agentgateway.status, self.envoy.status
		);
		Ok(())
	}

	/// Assert that both proxies returned the same body
	pub fn assert_same_body(&self) -> Result<()> {
		assert_eq!(
			self.agentgateway.body, self.envoy.body,
			"Bodies differ:\nagentgateway: {}\nenvoy: {}",
			self.agentgateway.body, self.envoy.body
		);
		Ok(())
	}

	/// Assert that both proxies returned the same headers (case-insensitive)
	pub fn assert_same_headers(&self, ignore_headers: &[&str]) -> Result<()> {
		let agentgateway_headers: Vec<(String, String)> = self
			.agentgateway
			.headers
			.iter()
			.filter(|(k, _)| !ignore_headers.iter().any(|h| k.eq_ignore_ascii_case(h)))
			.map(|(k, v)| (k.to_lowercase(), v.clone()))
			.sorted()
			.collect();

		let envoy_headers: Vec<(String, String)> = self
			.envoy
			.headers
			.iter()
			.filter(|(k, _)| !ignore_headers.iter().any(|h| k.eq_ignore_ascii_case(h)))
			.map(|(k, v)| (k.to_lowercase(), v.clone()))
			.sorted()
			.collect();

		assert_eq!(
			agentgateway_headers, envoy_headers,
			"Headers differ:\nagentgateway: {agentgateway_headers:?}\nenvoy: {envoy_headers:?}"
		);
		Ok(())
	}

	/// Assert that both proxies behave identically
	pub fn assert_identical(&self) -> Result<()> {
		info!("Envoy response: {:?}", self.envoy);
		info!("Agentgateway response: {:?}", self.agentgateway);
		self.assert_same_status()?;
		self.assert_same_headers(&["server", "date", "x-envoy-upstream-service-time"])?;
		self.assert_same_body()?;
		Ok(())
	}
}

/// Note: this is racy, since we drop it. But it will at least prevent taking long-running ports.
pub(super) async fn find_free_port() -> Result<u16> {
	let listener = TcpListener::bind("127.0.0.1:0").await?;
	let addr = listener.local_addr()?;
	Ok(addr.port())
}

/// Helper function to wait for a port to be available
pub(super) async fn wait_for_port(port: u16) -> Result<()> {
	let timeout_duration = Duration::from_secs(10);
	let start = std::time::Instant::now();

	while start.elapsed() < timeout_duration {
		if tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
			.await
			.is_ok()
		{
			return Ok(());
		}
		tokio::time::sleep(Duration::from_millis(100)).await;
	}

	Err(anyhow::anyhow!("Timeout waiting for port {}", port))
}

/// Helper function to create a temporary config file
pub(super) async fn create_temp_config_file(content: &str) -> Result<(TempDir, PathBuf)> {
	let temp_dir = TempDir::new()?;
	let config_path = temp_dir.path().join("config.yaml");
	tokio::fs::write(&config_path, content).await?;

	Ok((temp_dir, config_path))
}
