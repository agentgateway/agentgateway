use itertools::Itertools;
use rmcp::model::*;
use rmcp::transport::common::http_header::HEADER_SESSION_ID;
use serde_json::json;
use tokio::time::Duration;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::gateway::AgentGateway;
use crate::common::mcp::{
	multiplex_config, simple_multiplex_config, start_mock_mcp_meta_server, start_mock_mcp_server,
	start_mock_mcp_tools_only_prompt_leak_server,
};

mod assertions;
mod elicitation;
mod events;
mod fixtures;
mod multiplex;
mod session;
mod snapshots;
mod transport;
mod wire;

use assertions::*;
use fixtures::*;
use snapshots::*;
use wire::*;
