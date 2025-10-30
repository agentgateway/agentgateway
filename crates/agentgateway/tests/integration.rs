use http::{Method, StatusCode};
use std::net::SocketAddr;
use wiremock::{Mock, ResponseTemplate};

mod common;
use common::compare::*;
use common::gateway::*;
use common::hbone_server::*;
use common::mock_ca_server::*;

#[tokio::test]
async fn test_basic_proxy_comparison() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();
	if !ProxyComparisonTest::should_run() {
		return Ok(());
	}
	// Set up the test framework
	let test = ProxyComparisonTest::new().await?;
	// Configure the backend to return a simple response
	Mock::given(wiremock::matchers::method("GET"))
		.and(wiremock::matchers::path("/test"))
		.respond_with(
			ResponseTemplate::new(200)
				.set_body_string("Hello, World!")
				.insert_header("content-type", "text/plain"),
		)
		.mount(&test.backend_server)
		.await;

	// Send the same request to both proxies
	let comparison = test.compare_request("GET", "/test", None, None).await?;

	// Assert they behave identically
	comparison.assert_identical()?;

	Ok(())
}

#[tokio::test]
async fn test_basic_routes() -> anyhow::Result<()> {
	let mock = wiremock::MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200))
		.mount(&mock)
		.await;
	let gw = AgentGateway::new(format!(
		r#"config: {{}}
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: default
      policies:
        urlRewrite:
          path:
            prefix: /xxxx
        transformations:
          request:
          response:
            add:
              x-resp: '"foo"'
      backends:
        - host: {}
"#,
		mock.address()
	))
	.await?;
	let resp = gw.send_request(Method::GET, "http://localhost").await;
	assert_eq!(resp.status(), StatusCode::OK);
	let rh = resp.headers().get("x-resp").unwrap();
	assert_eq!(rh.to_str().unwrap(), "foo");
	Ok(())
}

#[tokio::test]
async fn test_hbone() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();

	const WAYPOINT_PREFIX: &[u8] = b"waypoint:";

	// Start the mock CA server that provides test certificates
	let ca_addr = start_mock_ca_server().await?;

	// Start the HBONE server in ReadWrite (echo) mode on the standard HBONE port 15008
	// It will prefix all echoed data with "waypoint:" to prove the connection went through it
	// Note: The HBONE client in agentgateway hardcodes port 15008 for HBONE connections
	let _hbone_port = start_hbone_server(15008, "test-server", WAYPOINT_PREFIX.to_vec()).await;

	// Configure agentgateway with CA and a workload that uses HBONE protocol
	// The workload's protocol: HBONE tells AGW to connect via HBONE to port 15008
	let gw_config = format!(
		r#"config:
  namespace: default
  serviceAccount: default
  trustDomain: cluster.local
  caAddress: "http://{ca_addr}"
workloads:
  - uid: "test-hbone-workload"
    name: "test-server"
    namespace: "default"
    serviceAccount: "test-server"
    trustDomain: "cluster.local"
    workloadIps: ["127.0.0.1"]
    protocol: HBONE
    services:
      default/test-service.default.svc.cluster.local:
        "8080": 8080
services:
  - name: "test-service"
    namespace: "default"
    hostname: "test-service.default.svc.cluster.local"
    vips:
      - "/127.0.0.1"
    ports:
      "8080": 8080
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: TCP
    tcpRoutes:
    - name: default
      backends:
        - service:
            name: default/test-service.default.svc.cluster.local
            port: 8080
"#
	);

	let gw = AgentGateway::new(gw_config).await?;

	// Connect directly via TCP to the gateway and send raw bytes
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	use tokio::net::TcpStream;

	let mut stream = TcpStream::connect(("127.0.0.1", gw.port()))
		.await
		.expect("Failed to connect to gateway");

	// Send a test message
	let test_message = b"hello from client";
	stream
		.write_all(test_message)
		.await
		.expect("Failed to write");
	stream.flush().await.expect("Failed to flush");

	// Shutdown the write side to signal EOF to the server
	// This tells the server we're done sending and it can echo everything back
	stream.shutdown().await.expect("Failed to shutdown write");

	// Read all data until the connection closes
	let mut buffer = Vec::new();
	tokio::time::timeout(
		std::time::Duration::from_secs(2),
		stream.read_to_end(&mut buffer),
	)
	.await
	.expect("Timeout reading response")
	.expect("Failed to read");

	let response = String::from_utf8_lossy(&buffer);
	let expected = format!("waypoint:{}", std::str::from_utf8(test_message).unwrap());

	// Verify the HBONE server echoed back our message with the waypoint prefix
	assert_eq!(
		response.as_ref(),
		expected,
		"Expected 'waypoint:' prefix followed by echoed message"
	);

	// Gracefully close the connection to avoid connection reset errors during cleanup
	drop(stream);

	// Explicitly shutdown the gateway for cleaner teardown
	gw.shutdown().await;

	Ok(())
}

#[tokio::test]
async fn test_double_hbone() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();

	const WAYPOINT_PREFIX: &[u8] = b"waypoint:";

	// Start the mock CA server that provides test certificates
	let ca_addr = start_mock_ca_server().await?;

	// Start the waypoint HBONE server (the final destination) on an OS-assigned port
	// It echoes back data with "waypoint:" prefix
	// The waypoint must have the identity of the remote-server workload
	let waypoint_port = start_hbone_server(0, "remote-server", WAYPOINT_PREFIX.to_vec()).await;

	// Start the E/W gateway HBONE server on an OS-assigned port
	// It forwards connections to the waypoint's HBONE port
	let waypoint_addr: SocketAddr = format!("127.0.0.1:{}", waypoint_port).parse().unwrap();
	let gateway_port = start_hbone_forward_server(0, "ew-gateway", waypoint_addr).await;

	// Configure agentgateway with:
	// 1. AGW itself is on network "" (default)
	// 2. E/W gateway workload on network "remote"
	// 3. Remote workload on network "remote" with network_gateway pointing to E/W gateway
	// 4. Service that routes to the remote workload
	let gw_config = format!(
		r#"config:
  network: ""
  namespace: default
  serviceAccount: default
  trustDomain: cluster.local
  caAddress: "http://{ca_addr}"
workloads:
  # E/W Gateway on remote network
  - uid: "ew-gateway"
    name: "ew-gateway"
    namespace: "default"
    serviceAccount: "ew-gateway"
    trustDomain: "cluster.local"
    workloadIps: ["127.0.0.1"]
    network: "remote"
    protocol: HBONE
    services: {{}}
  # Waypoint/workload on remote network (accessed through gateway)
  - uid: "remote-workload"
    name: "remote-server"
    namespace: "default"
    serviceAccount: "remote-server"
    trustDomain: "cluster.local"
    workloadIps: ["127.0.0.2"]
    network: "remote"
    protocol: HBONE
    networkGateway:
      destination: "remote/127.0.0.1"
      hboneMtlsPort: {gateway_port}
    services:
      default/remote-service.default.svc.cluster.local:
        "8080": 8080
services:
  - name: "remote-service"
    namespace: "default"
    hostname: "remote-service.default.svc.cluster.local"
    vips:
      - "/127.0.0.2"
    ports:
      "8080": 8080
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: TCP
    tcpRoutes:
    - name: default
      backends:
        - service:
            name: default/remote-service.default.svc.cluster.local
            port: 8080
"#
	);

	let gw = AgentGateway::new(gw_config).await?;

	// Connect to the gateway and send test data
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	use tokio::net::TcpStream;

	let mut stream = TcpStream::connect(("127.0.0.1", gw.port()))
		.await
		.expect("Failed to connect to gateway");

	// Send a test message
	let test_message = b"hello from client";
	stream
		.write_all(test_message)
		.await
		.expect("Failed to write");
	stream.flush().await.expect("Failed to flush");

	// Shutdown the write side to signal EOF
	stream.shutdown().await.expect("Failed to shutdown write");

	// Read all data until the connection closes
	let mut buffer = Vec::new();
	tokio::time::timeout(
		std::time::Duration::from_secs(2),
		stream.read_to_end(&mut buffer),
	)
	.await
	.expect("Timeout reading response")
	.expect("Failed to read");

	let response = String::from_utf8_lossy(&buffer);
	let expected = format!("waypoint:{}", std::str::from_utf8(test_message).unwrap());

	// Verify the message went through double hbone:
	// Client -> AGW -> (outer HBONE to gateway) -> gateway forwards -> (inner HBONE to waypoint) -> waypoint echoes
	assert_eq!(
		response.as_ref(),
		expected,
		"Expected 'waypoint:' prefix followed by echoed message (double hbone path)"
	);

	// Gracefully close the connection
	drop(stream);

	// Explicitly shutdown the gateway for cleaner teardown
	gw.shutdown().await;

	Ok(())
}

async fn start_hbone_server(port: u16, name: &str, waypoint_message: Vec<u8>) -> u16 {
	let name = name.to_string();
	let server = HboneTestServer::new(Mode::ReadWrite, &name, waypoint_message, port).await;
	let actual_port = server.port();
	tokio::spawn(async move {
		server.run().await;
	});
	actual_port
}

async fn start_hbone_forward_server(port: u16, name: &str, forward_to: SocketAddr) -> u16 {
	let name = name.to_string();
	let server = HboneTestServer::new(Mode::Forward(forward_to), &name, vec![], port).await;
	let actual_port = server.port();
	tokio::spawn(async move {
		server.run().await;
	});
	actual_port
}
