use agent_core::telemetry::testing::setup_test_logging;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::gateway::AgentGateway;
use crate::common::mcp::{
	CapabilityClient, DefaultClient, MockMcpServer, multiplex_config,
	multiplex_transport_matrix_config, setup_capability_client, setup_default_client,
	start_mock_legacy_sse_server, start_mock_mcp_server,
};

pub(super) struct MultiplexFixture {
	pub(super) client: CapabilityClient,
	pub(super) update_count: Arc<AtomicUsize>,
	_gw: AgentGateway,
	_mcp1: MockMcpServer,
	_mcp2: MockMcpServer,
	_s3_mock: MockServer,
}

impl MultiplexFixture {
	pub(super) async fn setup() -> anyhow::Result<Self> {
		setup_test_logging();

		let mcp1 = start_mock_mcp_server("s1", true).await;
		let mcp2 = start_mock_mcp_server("s2", false).await;

		let s3_mock = MockServer::start().await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(500).set_body_string("CRASH"))
			.mount(&s3_mock)
			.await;

		let config = multiplex_config(&mcp1, &mcp2, *s3_mock.address());
		let gw = AgentGateway::new(config).await?;
		let mcp_url = format!("http://localhost:{}/mcp", gw.port());
		let update_count = Arc::new(AtomicUsize::new(0));
		let client = setup_capability_client(&mcp_url, update_count.clone()).await?;

		Ok(Self {
			client,
			update_count,
			_gw: gw,
			_mcp1: mcp1,
			_mcp2: mcp2,
			_s3_mock: s3_mock,
		})
	}
}

pub(super) struct TransportMatrixFixture {
	pub(super) client: CapabilityClient,
	pub(super) update_count: Arc<AtomicUsize>,
	_gw: AgentGateway,
	_streamable: MockMcpServer,
	_sse: MockMcpServer,
	_s3_mock: MockServer,
}

impl TransportMatrixFixture {
	pub(super) async fn setup() -> anyhow::Result<Self> {
		setup_test_logging();

		let streamable = start_mock_mcp_server("stream", true).await;
		let sse = start_mock_legacy_sse_server().await;

		let s3_mock = MockServer::start().await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(500).set_body_string("CRASH"))
			.mount(&s3_mock)
			.await;

		let config = multiplex_transport_matrix_config(&streamable, &sse, *s3_mock.address());
		let gw = AgentGateway::new(config).await?;
		let mcp_url = format!("http://localhost:{}/mcp", gw.port());
		let update_count = Arc::new(AtomicUsize::new(0));
		let client = setup_capability_client(&mcp_url, update_count.clone()).await?;

		Ok(Self {
			client,
			update_count,
			_gw: gw,
			_streamable: streamable,
			_sse: sse,
			_s3_mock: s3_mock,
		})
	}
}

pub(super) async fn setup_default_client_for_gateway(
	gw: &AgentGateway,
) -> anyhow::Result<DefaultClient> {
	let mcp_url = format!("http://localhost:{}/mcp", gw.port());
	setup_default_client(&mcp_url).await
}

pub(super) async fn unused_loopback_addr() -> anyhow::Result<SocketAddr> {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let addr = listener.local_addr()?;
	drop(listener);
	Ok(addr)
}
