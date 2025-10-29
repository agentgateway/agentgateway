use http::{Method, StatusCode};
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

	// Start the HBONE server in ReadWrite (echo) mode on port 15008 (standard HBONE port)
	// It will prefix all echoed data with "waypoint:" to prove the connection went through it
	// Note: The HBONE client in agentgateway hardcodes port 15008 for HBONE connections
	let hbone_port = 15008_u16;
	start_hbone_server(hbone_port, WAYPOINT_PREFIX.to_vec()).await;
	wait_for_port(hbone_port).await?;

	// The service port can be anything - the HBONE tunnel terminates at the HBONE server
	let service_port = find_free_port().await?;

	// Configure agentgateway with CA and a workload that uses HBONE protocol
	// The workload's protocol: HBONE tells AGW to connect via HBONE to port 15008
	// The service port mapping is for the logical service port
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
        "8080": {service_port}
services:
  - name: "test-service"
    namespace: "default"
    hostname: "test-service.default.svc.cluster.local"
    vips:
      - "/127.0.0.1"
    ports:
      "8080": {service_port}
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

	// Give the CA client time to fetch the certificate
	tokio::time::sleep(std::time::Duration::from_millis(500)).await;

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

	tokio::time::sleep(std::time::Duration::from_millis(100)).await;

	Ok(())
}

async fn start_hbone_server(port: u16, waypoint_message: Vec<u8>) {
	tokio::spawn(async move {
		let server = HboneTestServer::new(Mode::ReadWrite, "test-server", waypoint_message, port).await;
		server.run().await;
	});
}
