mod client;
mod config;
mod server;

pub(crate) use client::{
	CapabilityClient, DefaultClient, setup_capability_client, setup_default_client,
};
pub(crate) use config::{
	multiplex_config, multiplex_transport_matrix_config, simple_multiplex_config,
};
pub(crate) use server::{
	MockMcpServer, start_mock_legacy_sse_server, start_mock_mcp_meta_server, start_mock_mcp_server,
	start_mock_mcp_tools_only_prompt_leak_server,
};
