//! External MCP policy hooks (extMcp).
//!
//! Hooks fire server-facing in the upstream's native namespace — drivers see
//! unmuxed identifiers (`echo`, not `serverA_echo`) and the backend name as
//! metadata. Fanout methods run the hook once per upstream.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::mcp::upstream::IncomingRequestContext;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::SimpleBackendReference;
use crate::*;

mod client;
pub mod methods;
pub mod outcome;
pub mod phase;

pub use outcome::Outcome;
pub use phase::Phase;

pub mod wire {
	pub use protos::ext_mcp::*;
}

#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtMcp {
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub drivers: Vec<Driver>,
	#[serde(skip_serializing_if = "HashMap::is_empty")]
	pub methods: HashMap<String, Phase>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Driver {
	Remote(Remote),
}

// Configuration for a remote MCP-aware policy server, modeled on Envoy's
// ext_authz. TLS, retries, and load balancing to the policy server come
// from the backend referenced by `target`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
	#[serde(skip)]
	pub target: Arc<SimpleBackendReference>,
	pub failure_mode: FailureMode,
	pub timeout: Duration,
	#[serde(skip_serializing_if = "HashMap::is_empty", skip_deserializing)]
	pub metadata: HashMap<String, Arc<cel::Expression>>,
}

// when we have an error executing a driver, or a driver gives a response that
// we can't handle, this determines whether we fail-open or fail-closed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FailureMode {
	Allow,
	#[default]
	Deny,
}

/// Opaque single-call request context. The ext server sees the full
/// JSON-RPC params blob and may replace it wholesale via `Mutated`.
/// `params` is `None` for methods with no per-request body (e.g. `*/list`);
/// any `Mutated` outcome there is logged and discarded.
pub struct CallRequestCtx<'a> {
	pub backend: &'a str,
	pub method: &'a str,
	pub params: Option<&'a mut Value>,
}

impl Driver {
	async fn call_request(
		&self,
		ctx: &mut CallRequestCtx<'_>,
		req_ctx: &IncomingRequestContext,
		client: &PolicyClient,
	) -> Outcome {
		match self {
			Driver::Remote(remote) => {
				let body = ctx.params.as_deref().cloned();
				match client::check_request(remote, ctx.method, ctx.backend, body, req_ctx, client).await {
					client::RequestOutcome::Pass => Outcome::Pass,
					client::RequestOutcome::Mutated(v) => match ctx.params.as_deref_mut() {
						Some(p) => {
							*p = v;
							Outcome::Mutated
						},
						None => {
							tracing::debug!(
								method = ctx.method,
								backend = ctx.backend,
								"extMcp: ignoring mutation on request without body",
							);
							Outcome::Pass
						},
					},
					client::RequestOutcome::Reject(e) => Outcome::Reject(e),
				}
			},
		}
	}

	async fn response(
		&self,
		method: &str,
		backend: &str,
		body: &mut Value,
		req_ctx: &IncomingRequestContext,
		client: &PolicyClient,
	) -> Outcome {
		match self {
			Driver::Remote(remote) => {
				client::check_response(remote, method, backend, body, req_ctx, client).await
			},
		}
	}
}

/// Run the request-phase pipeline. Drivers fire in `ext.drivers` order;
/// first `Reject` short-circuits, mutations compose (each driver acts on the
/// body the previous one left behind). On `Reject`, `ctx` is left in whatever
/// partially-mutated state earlier drivers produced. When `ctx.params` is
/// `None` (e.g. `*/list`) mutations are discarded; list filtering belongs in
/// the response phase.
pub async fn run_call_request(
	ext: &ExtMcp,
	ctx: &mut CallRequestCtx<'_>,
	req_ctx: &IncomingRequestContext,
	client: &PolicyClient,
) -> Outcome {
	if !phase::resolve(ctx.method, &ext.methods).runs_request() {
		return Outcome::Pass;
	}
	let mut composed = Outcome::Pass;
	for driver in &ext.drivers {
		match driver.call_request(ctx, req_ctx, client).await {
			Outcome::Pass => {},
			Outcome::Mutated => composed = Outcome::Mutated,
			Outcome::Reject(e) => return Outcome::Reject(e),
		}
	}
	composed
}

/// Run the response-phase pipeline. Drivers fire in `ext.drivers` order;
/// first `Reject` short-circuits, mutations compose left-to-right.
pub async fn run_response(
	ext: &ExtMcp,
	method: &str,
	backend: &str,
	body: &mut Value,
	req_ctx: &IncomingRequestContext,
	client: &PolicyClient,
) -> Outcome {
	if !phase::resolve(method, &ext.methods).runs_response() {
		return Outcome::Pass;
	}
	let mut composed = Outcome::Pass;
	for driver in &ext.drivers {
		match driver
			.response(method, backend, body, req_ctx, client)
			.await
		{
			Outcome::Pass => {},
			Outcome::Mutated => composed = Outcome::Mutated,
			Outcome::Reject(e) => return Outcome::Reject(e),
		}
	}
	composed
}
