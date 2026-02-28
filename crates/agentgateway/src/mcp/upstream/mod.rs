mod client;
mod openapi;
mod sse;
mod stdio;
mod streamablehttp;

use std::io;

pub(crate) use client::McpHttpClient;
pub use openapi::ParseError as OpenAPIParseError;
use rmcp::model::{ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcRequest};
use rmcp::transport::TokioChildProcess;
use thiserror::Error;
use tokio::process::Command;

use crate::http::jwt::Claims;
use crate::mcp::mergestream::Messages;
use crate::mcp::router::{McpBackendGroup, McpTarget};
use crate::mcp::streamablehttp::StreamableHttpPostResponse;
use crate::mcp::{mergestream, upstream};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::McpTargetSpec;
use crate::*;

#[derive(Debug, Clone)]
pub struct IncomingRequestContext {
	headers: http::HeaderMap,
	claims: Option<Claims>,
}

impl IncomingRequestContext {
	#[cfg(test)]
	pub fn empty() -> Self {
		Self {
			headers: http::HeaderMap::new(),
			claims: None,
		}
	}
	pub fn new(parts: &::http::request::Parts) -> Self {
		let claims = parts.extensions.get::<Claims>().cloned();
		Self {
			headers: parts.headers.clone(),
			claims,
		}
	}
	pub fn apply(&self, req: &mut http::Request) {
		for (k, v) in &self.headers {
			// Remove headers we do not want to propagate to the backend
			if k == http::header::CONTENT_ENCODING || k == http::header::CONTENT_LENGTH {
				continue;
			}
			if !req.headers().contains_key(k) {
				req.headers_mut().insert(k.clone(), v.clone());
			}
		}
		if let Some(claims) = self.claims.as_ref() {
			req.extensions_mut().insert(claims.clone());
		}
	}
}

#[derive(Debug, Error)]
pub enum UpstreamError {
	#[error("unknown {resource_type}: {resource_name}")]
	Authorization {
		resource_type: String,
		resource_name: String,
	},
	#[error("invalid request: {0}")]
	InvalidRequest(String),
	#[error("unsupported method: {0}")]
	InvalidMethod(String),
	#[error("method {0} is unsupported with multiplexing")]
	InvalidMethodWithMultiplexing(String),
	#[error("stdio upstream error: {0}")]
	ServiceError(#[from] rmcp::ServiceError),
	#[error("http upstream error: {0}")]
	Http(#[from] mcp::ClientError),
	#[error("openapi upstream error: {0}")]
	OpenAPIError(#[from] anyhow::Error),
	#[error("{0}")]
	Proxy(#[from] ProxyError),
	#[error("stdio upstream error: {0}")]
	Stdio(#[from] io::Error),
	#[error("upstream closed on send")]
	Send,
}

// UpstreamTarget defines a source for MCP information.
#[derive(Debug)]
pub(crate) enum Upstream {
	McpStreamable(streamablehttp::Client),
	McpSSE(sse::Client),
	McpStdio(stdio::Process),
	OpenAPI(Box<openapi::Handler>),
}

impl Upstream {
	pub fn get_session_state(&self) -> Option<http::sessionpersistence::MCPSession> {
		match self {
			Upstream::McpStreamable(c) => Some(c.get_session_state()),
			_ => None,
		}
	}

	pub fn set_session_id(&self, id: Option<&str>, pinned: Option<SocketAddr>) {
		match self {
			Upstream::McpStreamable(c) => c.set_session_id(id, pinned),
			Upstream::McpSSE(_) => {},
			Upstream::McpStdio(_) => {},
			Upstream::OpenAPI(_) => {},
		}
	}

	pub(crate) async fn delete(&self, ctx: &IncomingRequestContext) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => {
				c.stop().await?;
			},
			Upstream::McpStreamable(c) => {
				c.send_delete(ctx).await?;
			},
			Upstream::McpSSE(c) => {
				c.stop().await?;
			},
			Upstream::OpenAPI(_) => {
				// No need to do anything here
			},
		}
		Ok(())
	}
	pub(crate) async fn get_event_stream(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => Ok(c.get_event_stream().await),
			Upstream::McpSSE(c) => c.connect_to_event_stream(ctx).await,
			Upstream::McpStreamable(c) => c
				.get_event_stream(ctx)
				.await?
				.try_into()
				.map_err(Into::into),
			Upstream::OpenAPI(_m) => Ok(Messages::pending()),
		}
	}
	pub(crate) async fn generic_stream(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => Ok(mergestream::Messages::from(
				c.send_message(request, ctx).await?,
			)),
			Upstream::McpSSE(c) => Ok(mergestream::Messages::from(
				c.send_message(request, ctx).await?,
			)),
			Upstream::McpStreamable(c) => {
				let is_init = matches!(&request.request, &ClientRequest::InitializeRequest(_));
				let res = c.send_request(request, ctx).await?;
				if is_init {
					let sid = match &res {
						StreamableHttpPostResponse::Accepted => None,
						StreamableHttpPostResponse::Json(_, sid) => sid.as_ref(),
						StreamableHttpPostResponse::Sse(_, sid) => sid.as_ref(),
					};
					c.set_session_id(sid.map(|s| s.as_str()), None)
				}
				res.try_into().map_err(Into::into)
			},
			Upstream::OpenAPI(c) => Ok(c.send_message(request, ctx).await?),
		}
	}

	pub(crate) async fn generic_notification(
		&self,
		request: ClientNotification,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => {
				c.send_notification(request, ctx).await?;
			},
			Upstream::McpSSE(c) => {
				c.send_notification(request, ctx).await?;
			},
			Upstream::McpStreamable(c) => {
				c.send_notification(request, ctx).await?;
			},
			Upstream::OpenAPI(_) => {},
		}
		Ok(())
	}

	pub(crate) async fn send_client_message(
		&self,
		message: ClientJsonRpcMessage,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => {
				c.send_raw(message, ctx).await?;
			},
			Upstream::McpSSE(c) => {
				c.send_client_message(message, ctx).await?;
			},
			Upstream::McpStreamable(c) => {
				c.send_client_message(message, ctx).await?;
			},
			Upstream::OpenAPI(_) => {},
		}
		Ok(())
	}
}

#[derive(Debug)]
pub(crate) struct UpstreamGroup {
	backend: McpBackendGroup,
	client: PolicyClient,
	by_name: IndexMap<Strng, Arc<upstream::Upstream>>,
}

impl UpstreamGroup {
	/// Returns the number of successfully initialized upstream targets.
	///
	/// This may be less than the configured target count when some targets
	/// fail during startup and are skipped.
	pub fn size(&self) -> usize {
		self.by_name.len()
	}

	pub(crate) fn new(client: PolicyClient, backend: McpBackendGroup) -> Result<Self, mcp::Error> {
		let mut s = Self {
			backend,
			client,
			by_name: IndexMap::new(),
		};
		s.setup_connections()?;
		Ok(s)
	}

	fn setup_connections(&mut self) -> Result<(), mcp::Error> {
		let mut failures = Vec::new();
		for tgt in &self.backend.targets {
			debug!("initializing target: {}", tgt.name);
			match self.setup_upstream(tgt.as_ref()) {
				Ok(transport) => {
					self.by_name.insert(tgt.name.clone(), Arc::new(transport));
				},
				Err(e) => {
					if !self.backend.allow_degraded {
						return Err(e);
					}
					warn!(upstream_name = %tgt.name, error = %e, "failed to initialize MCP target; skipping");
					failures.push((tgt.name.clone(), e));
				},
			}
		}
		if !failures.is_empty() && !self.by_name.is_empty() {
			warn!(
				failed_targets = failures.len(),
				total_targets = self.backend.targets.len(),
				initialized_targets = self.by_name.len(),
				"MCP upstream group initialized in degraded mode"
			);
		} else if !self.backend.targets.is_empty() && self.by_name.is_empty() {
			let reason = failures
				.into_iter()
				.map(|(name, err)| format!("{name}: {err}"))
				.collect::<Vec<_>>()
				.join("; ");
			return Err(mcp::Error::SendError(
				None,
				format!("all MCP targets failed to initialize: {reason}"),
			));
		}
		Ok(())
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

	fn setup_upstream(&self, target: &McpTarget) -> Result<upstream::Upstream, mcp::Error> {
		trace!("connecting to target: {}", target.name);
		let target = match &target.spec {
			McpTargetSpec::Sse(sse) => {
				debug!("starting sse transport for target: {}", target.name);
				let path = match sse.path.as_str() {
					"" => "/sse",
					_ => sse.path.as_str(),
				};

				let upstream_client = McpHttpClient::new(
					self.client.clone(),
					target
						.backend
						.clone()
						.expect("there must be a backend for SSE"),
					target.backend_policies.clone(),
					self.backend.stateful,
					target.name.to_string(),
				);
				let client = sse::Client::new(upstream_client, path.into());

				upstream::Upstream::McpSSE(client)
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

				let http_client = McpHttpClient::new(
					self.client.clone(),
					target
						.backend
						.clone()
						.expect("there must be a backend for MCP"),
					target.backend_policies.clone(),
					self.backend.stateful,
					target.name.to_string(),
				);
				let client = streamablehttp::Client::new(http_client, path.into())
					.map_err(|_| mcp::Error::InvalidSessionIdHeader)?;

				upstream::Upstream::McpStreamable(client)
			},
			McpTargetSpec::Stdio { cmd, args, env } => {
				debug!("starting stdio transport for target: {}", target.name);
				#[cfg(target_os = "windows")]
				// Command has some weird behavior on Windows where it expects the executable extension to be
				// .exe. The which create will resolve the actual command for us.
				// See https://github.com/rust-lang/rust/issues/37519#issuecomment-1694507663
				// for more context.
				let cmd = which::which(cmd).map_err(|e| mcp::Error::Stdio(io::Error::other(e)))?;
				#[cfg(target_family = "unix")]
				let mut c = Command::new(cmd);
				#[cfg(target_os = "windows")]
				let mut c = Command::new(&cmd);
				c.args(args);
				for (k, v) in env {
					c.env(k, v);
				}
				let proc = TokioChildProcess::new(c).map_err(mcp::Error::Stdio)?;
				upstream::Upstream::McpStdio(upstream::stdio::Process::new(proc))
			},
			McpTargetSpec::OpenAPI(open) => {
				// Renamed for clarity
				debug!("starting OpenAPI transport for target: {}", target.name);

				let tools = openapi::parse_openapi_schema(&open.schema).map_err(mcp::Error::OpenAPI)?;
				let prefix = openapi::get_server_prefix(&open.schema).map_err(mcp::Error::OpenAPI)?;

				let http_client = McpHttpClient::new(
					self.client.clone(),
					target
						.backend
						.clone()
						.expect("there must be a backend for OpenAPI"),
					target.backend_policies.clone(),
					self.backend.stateful,
					target.name.to_string(),
				);
				upstream::Upstream::OpenAPI(Box::new(openapi::Handler::new(
					http_client,
					tools,  // From parse_openapi_schema
					prefix, // From get_server_prefix
				)))
			},
		};

		Ok(target)
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use super::UpstreamGroup;
	use crate::mcp::router::{McpBackendGroup, McpTarget};
	use crate::proxy::httpproxy::PolicyClient;
	use crate::types::agent::McpTargetSpec;

	fn stdio_target(name: &str, cmd: &str) -> Arc<McpTarget> {
		Arc::new(McpTarget {
			name: name.into(),
			spec: McpTargetSpec::Stdio {
				cmd: cmd.to_string(),
				args: vec![],
				env: Default::default(),
			},
			backend_policies: Default::default(),
			backend: None,
			always_use_prefix: false,
		})
	}

	#[cfg(target_family = "unix")]
	#[tokio::test]
	async fn setup_connections_skips_failed_targets_and_keeps_healthy_ones() {
		// async runtime required because stdio setup uses Tokio child-process internals
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let backend = McpBackendGroup {
			targets: vec![
				stdio_target("healthy", "true"),
				stdio_target("missing", "__agw_missing_command__"),
			],
			stateful: false,
			allow_degraded: true,
		};

		let group = UpstreamGroup::new(
			PolicyClient {
				inputs: test.inputs(),
			},
			backend,
		)
		.expect("at least one target should initialize");

		assert_eq!(group.size(), 1, "only healthy targets should be counted");
		assert!(
			group.get("healthy").is_ok(),
			"healthy target should be available"
		);
		assert!(
			group.get("missing").is_err(),
			"failed target should not be available"
		);
	}

	#[cfg(target_family = "unix")]
	#[tokio::test]
	async fn setup_connections_fails_on_any_target_failure_by_default() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let backend = McpBackendGroup {
			targets: vec![
				stdio_target("healthy", "true"),
				stdio_target("missing", "__agw_missing_command__"),
			],
			stateful: false,
			allow_degraded: false,
		};

		let err = UpstreamGroup::new(
			PolicyClient {
				inputs: test.inputs(),
			},
			backend,
		)
		.expect_err("should fail because allow_degraded is false");

		assert!(err.to_string().contains("failed to start stdio server"));
	}

	#[tokio::test]
	async fn setup_connections_errors_when_all_targets_fail() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let backend = McpBackendGroup {
			targets: vec![
				stdio_target("missing-a", "__agw_missing_command_a__"),
				stdio_target("missing-b", "__agw_missing_command_b__"),
			],
			stateful: false,
			allow_degraded: true,
		};

		let err = UpstreamGroup::new(
			PolicyClient {
				inputs: test.inputs(),
			},
			backend,
		)
		.expect_err("all failed targets should error");
		let msg = err.to_string();
		assert!(
			msg.contains("all MCP targets failed to initialize"),
			"unexpected error: {msg}"
		);
	}
}
