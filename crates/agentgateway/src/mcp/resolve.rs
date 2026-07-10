//! Routing of unprefixed names to targets (`prefixMode: never`): the owning
//! target is discovered by listing every target's names at call time.
//! Deliberately unoptimized; a process-level cache can sit in front later.

use std::sync::atomic::{AtomicU64, Ordering};

use agent_core::prelude::Strng;
use futures_util::StreamExt;
use itertools::Itertools;
use rmcp::model::{ClientRequest, JsonRpcRequest, RequestId, ServerJsonRpcMessage, ServerResult};
use tracing::warn;

use crate::mcp::FailureMode;
use crate::mcp::handler::Relay;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};

/// What kind of name is being resolved to a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveKind {
	Tool,
	Prompt,
}

impl ResolveKind {
	pub fn as_str(&self) -> &'static str {
		match self {
			ResolveKind::Tool => "tool",
			ResolveKind::Prompt => "prompt",
		}
	}

	fn list_request(&self) -> ClientRequest {
		match self {
			ResolveKind::Tool => ClientRequest::ListToolsRequest(rmcp::model::ListToolsRequest {
				method: Default::default(),
				params: None,
				extensions: Default::default(),
			}),
			ResolveKind::Prompt => ClientRequest::ListPromptsRequest(rmcp::model::ListPromptsRequest {
				method: Default::default(),
				params: None,
				extensions: Default::default(),
			}),
		}
	}

	fn contains_name(&self, result: &ServerResult, name: &str) -> bool {
		match (self, result) {
			(ResolveKind::Tool, ServerResult::ListToolsResult(r)) => {
				r.tools.iter().any(|t| t.name == name)
			},
			(ResolveKind::Prompt, ServerResult::ListPromptsResult(r)) => {
				r.prompts.iter().any(|p| p.name == name)
			},
			_ => false,
		}
	}
}

// Gateway-generated list requests share upstream sessions with forwarded
// client requests, so their ids must not collide with client-chosen ids.
static RESOLVE_ID: AtomicU64 = AtomicU64::new(0);

fn next_resolve_id() -> RequestId {
	RequestId::String(
		format!(
			"agentgateway-resolve-{}",
			RESOLVE_ID.fetch_add(1, Ordering::Relaxed)
		)
		.into(),
	)
}

impl Relay {
	/// Find the single target serving the unprefixed `name` by listing every
	/// target. Errors if no target or more than one target serves it.
	pub(crate) async fn resolve_unprefixed(
		&self,
		kind: ResolveKind,
		name: &str,
		ctx: &IncomingRequestContext,
	) -> Result<Strng, UpstreamError> {
		let req = JsonRpcRequest {
			jsonrpc: Default::default(),
			id: next_resolve_id(),
			request: kind.list_request(),
		};
		let futs: Vec<_> = self
			.upstreams
			.iter_named()
			.map(|(target, con)| {
				let req = req.clone();
				async move {
					let res = match con.generic_stream(req, ctx).await {
						Ok(stream) => Self::first_response(stream).await,
						Err(e) => Err(e),
					};
					(target, res)
				}
			})
			.collect();

		let mut owners: Vec<Strng> = Vec::new();
		for (target, res) in futures::future::join_all(futs).await {
			match res {
				Ok(result) => {
					if kind.contains_name(&result, name) {
						owners.push(target);
					}
				},
				Err(e) => {
					if self.upstreams.failure_mode == FailureMode::FailOpen {
						warn!(
							"upstream '{}' failed while resolving {} '{}', skipping (failure_mode=FailOpen): {}",
							target,
							kind.as_str(),
							name,
							e
						);
					} else {
						return Err(e);
					}
				},
			}
		}

		match owners.as_slice() {
			[one] => Ok(one.clone()),
			// Match the authorization-denied message so callers cannot probe
			// which names exist.
			[] => Err(UpstreamError::Authorization {
				resource_type: kind.as_str().to_string(),
				resource_name: name.to_string(),
			}),
			many => Err(UpstreamError::InvalidRequest(format!(
				"{} '{}' is served by multiple targets ({}); names must be unique across targets when prefixMode is 'never'",
				kind.as_str(),
				name,
				many.iter().join(", "),
			))),
		}
	}

	/// Consume a response stream until the first result, error data, or end.
	async fn first_response(
		stream: crate::mcp::mergestream::Messages,
	) -> Result<ServerResult, UpstreamError> {
		let mut stream = std::pin::pin!(stream);
		while let Some(msg) = stream.next().await {
			match msg {
				Ok(ServerJsonRpcMessage::Response(resp)) => return Ok(resp.result),
				Ok(ServerJsonRpcMessage::Error(err)) => {
					return Err(UpstreamError::InvalidRequest(err.error.message.to_string()));
				},
				Ok(_) => {},
				Err(e) => return Err(e.into()),
			}
		}
		Err(UpstreamError::Recv)
	}
}
