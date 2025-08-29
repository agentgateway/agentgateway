mod openapi;
mod stdio;
mod streamablehttp;

use crate::mcp::router::{McpBackendGroup, McpTarget};
use crate::mcp::{ClientError, mergestream, upstream};
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::McpTargetSpec;
use crate::*;
use anyhow::anyhow;
use rmcp::model::{
	ClientNotification, ClientRequest, JsonRpcRequest,
};
use rmcp::transport::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use std::io;
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum UpstreamError {
	#[error("unauthorized tool call")]
	Authorization,
	#[error("invalid request: {0}")]
	InvalidRequest(String),
	#[error("stdio upstream error: {0}")]
	ServiceError(#[from] rmcp::ServiceError),
	#[error("http upstream error: {0}")]
	Http(#[from] mcp::ClientError),
	#[error("openapi upstream error: {0}")]
	OpenAPIError(#[from] anyhow::Error),
	#[error("stdio upstream error: {0}")]
	Stdio(#[from] io::Error),
	#[error("upstream closed on send")]
	Send,
	#[error("upstream closed on receive")]
	Recv,
}

// UpstreamTarget defines a source for MCP information.
#[derive(Debug)]
pub(crate) enum Upstream {
	McpHttp(streamablehttp::Client),
	McpStdio(stdio::Process),
	OpenAPI(Box<openapi::Handler>),
}

impl Upstream {
	pub(crate) async fn delete(&self, user_headers: &http::HeaderMap) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => {
				c.stop().await?;
			},
			Upstream::McpHttp(c) => {
				c.send_delete(user_headers).await?;
			},
			Upstream::OpenAPI(_m) => todo!(),
		}
		Ok(())
	}
	pub(crate) async fn get_event_stream(
		&self,
		user_headers: &http::HeaderMap,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => Ok(c.get_event_stream().await),
			Upstream::McpHttp(c) => c
				.get_event_stream(user_headers)
				.await?
				.try_into()
				.map_err(Into::into),
			Upstream::OpenAPI(_m) => todo!(),
		}
	}
	pub(crate) async fn generic_stream(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		user_headers: &http::HeaderMap,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => {
				let receiver = c.send_message(request).await?;
				let response = receiver.await.map_err(|_| UpstreamError::Recv)?;
				Ok(mergestream::Messages::from(response))
			},
			Upstream::McpHttp(c) => {
				let is_init = matches!(&request.request, &ClientRequest::InitializeRequest(_));
				let res = c.send_message(request, user_headers).await?;
				if is_init {
					let sid = match &res {
						StreamableHttpPostResponse::Accepted => None,
						StreamableHttpPostResponse::Json(_, sid) => sid.as_ref(),
						StreamableHttpPostResponse::Sse(_, sid) => sid.as_ref(),
					};
					if let Some(sid) = sid {
						c.set_session_id(sid.clone())
					}
				}
				res.try_into().map_err(Into::into)
			},
			Upstream::OpenAPI(_m) => todo!(),
		}
	}

	pub(crate) async fn generic_notification(
		&self,
		request: ClientNotification,
		user_headers: &http::HeaderMap,
	) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => {
				c.send_notification(request).await?;
			},
			Upstream::McpHttp(c) => {
				c.send_notification(request, user_headers).await?;
			},
			Upstream::OpenAPI(_m) => todo!(),
		}
		Ok(())
	}
}

#[derive(Debug)]
pub(crate) struct UpstreamGroup {
	pi: Arc<ProxyInputs>,
	backend: McpBackendGroup,
	client: PolicyClient,
	by_name: IndexMap<Strng, Arc<upstream::Upstream>>,
	stateful: bool,
}

impl UpstreamGroup {
	pub(crate) fn new(
		pi: Arc<ProxyInputs>,
		client: PolicyClient,
		backend: McpBackendGroup,
		stateful: bool,
	) -> anyhow::Result<Self> {
		let mut s = Self {
			backend,
			client,
			pi,
			by_name: IndexMap::new(),
			stateful,
		};
		s.setup_connections()?;
		Ok(s)
	}

	pub(crate) fn setup_connections(&mut self) -> anyhow::Result<()> {
		for tgt in &self.backend.targets {
			debug!("initializing target: {}", tgt.name);
			let transport = self.setup_upstream(tgt.as_ref())?;
			self.by_name.insert(tgt.name.clone(), Arc::new(transport));
		}
		Ok(())
	}

	pub(crate) fn iter(&self) -> impl Iterator<Item = Arc<upstream::Upstream>> {
		self.by_name.values().cloned()
	}
	pub(crate) fn iter_named(&self) -> impl Iterator<Item = (Strng, Arc<upstream::Upstream>)> {
		self.by_name.iter().map(|(k, v)| (k.clone(), v.clone()))
	}
	pub(crate) fn get(&self, name: &str) -> anyhow::Result<&upstream::Upstream> {
		self
			.by_name
			.get(name)
			.map(|v| v.as_ref())
			.ok_or_else(|| anyhow::anyhow!("requested target {name} is not initialized",))
	}

	fn setup_upstream(&self, target: &McpTarget) -> Result<upstream::Upstream, anyhow::Error> {
		trace!("connecting to target: {}", target.name);
		let target = match &target.spec {
			McpTargetSpec::Sse(_) => {
				todo!()
			},
			McpTargetSpec::Mcp(mcp) => {
				debug!(
					"starting streamable http transport for target: {}",
					target.name
				);
				let path = match mcp.path.as_str() {
					"" => "/mcp",
					_ => mcp.path.as_str(),
				};
				let be = crate::proxy::resolve_simple_backend(&mcp.backend, &self.pi)?;
				let client = streamablehttp::Client::new(
					be,
					path.into(),
					self.client.clone(),
					target.backend_policies.clone(),
				);

				upstream::Upstream::McpHttp(client)
			},
			McpTargetSpec::Stdio { cmd, args, env } => {
				debug!("starting stdio transport for target: {}", target.name);
				#[cfg(target_os = "windows")]
				// Command has some weird behavior on Windows where it expects the executable extension to be
				// .exe. The which create will resolve the actual command for us.
				// See https://github.com/rust-lang/rust/issues/37519#issuecomment-1694507663
				// for more context.
				let cmd = which::which(cmd)?;
				#[cfg(target_family = "unix")]
				let mut c = Command::new(cmd);
				#[cfg(target_os = "windows")]
				let mut c = Command::new(&cmd);
				c.args(args);
				for (k, v) in env {
					c.env(k, v);
				}
				let proc =
					TokioChildProcess::new(c).context(format!("failed to run command '{:?}'", &cmd))?;
				upstream::Upstream::McpStdio(upstream::stdio::Process::new(proc))
			},
			McpTargetSpec::OpenAPI(open) => {
				// Renamed for clarity
				debug!("starting OpenAPI transport for target: {}", target.name);

				let tools = openapi::parse_openapi_schema(&open.schema).map_err(|e| {
					anyhow::anyhow!(
						"Failed to parse tools from OpenAPI schema for target {}: {}",
						target.name,
						e
					)
				})?;

				let prefix = openapi::get_server_prefix(&open.schema).map_err(|e| {
					anyhow::anyhow!(
						"Failed to get server prefix from OpenAPI schema for target {}: {}",
						target.name,
						e
					)
				})?;
				let be = crate::proxy::resolve_simple_backend(&open.backend, &self.pi)?;
				upstream::Upstream::OpenAPI(Box::new(openapi::Handler {
					backend: be,
					client: self.client.clone(),
					default_policies: target.backend_policies.clone(),
					tools,  // From parse_openapi_schema
					prefix, // From get_server_prefix
				}))
			},
		};

		Ok(target)
	}
}

pub async fn expect_accepted(res: StreamableHttpPostResponse) -> Result<(), ClientError> {
	match res {
		StreamableHttpPostResponse::Accepted => Ok(()),
		StreamableHttpPostResponse::Json(_, _) => {
			Err(ClientError::new(anyhow!("unexpected 'json' response")))
		},
		StreamableHttpPostResponse::Sse(_, _) => {
			Err(ClientError::new(anyhow!("unexpected 'sse' response")))
		},
	}
}
