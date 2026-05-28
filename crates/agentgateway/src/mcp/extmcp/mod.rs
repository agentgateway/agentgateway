//! External MCP policy hooks (extMcp).
//!
//! Hooks fire server-facing in the upstream's native namespace — drivers see
//! unmuxed identifiers (`echo`, not `serverA_echo`) and the backend name as
//! metadata. Fanout methods run the hook once per upstream.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::mcp::upstream::IncomingRequestContext;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::SimpleBackendReference;
use crate::*;

/// Per-request bag of values that `extMcp` request-phase drivers attach via
/// `McpRequestResult.metadata`. Merged into the request extensions and exposed
/// to CEL as `extmcp.<key>` for backend request filters (e.g. `transformation`).
/// Multiple drivers merge into the same map; later writes win on key collisions.
#[apply(schema!)]
#[derive(Default, ::cel::DynamicType)]
pub struct ExtMcpDynamicMetadata(serde_json::Map<String, serde_json::Value>);

impl ExtMcpDynamicMetadata {
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}
}

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
	/// Allowlist: only methods listed here run through the pipeline, at the
	/// configured phase. Methods absent from the map bypass extMcp entirely.
	#[serde(skip_serializing_if = "HashMap::is_empty")]
	pub methods: HashMap<String, Phase>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Driver {
	Remote(Remote),
}

// TLS, retries, and load balancing come from the backend referenced by `target`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
	#[serde(skip)]
	pub target: Arc<SimpleBackendReference>,
	pub failure_mode: FailureMode,
	#[serde(skip_serializing_if = "HashMap::is_empty", skip_deserializing)]
	pub metadata: HashMap<String, Arc<cel::Expression>>,
	/// Which incoming request headers are forwarded to the policy server.
	#[serde(skip_serializing_if = "HeaderFilter::is_default", skip_deserializing)]
	pub request_headers: HeaderFilter,
}

/// Allow/deny filter over request header names. Empty `allowed` forwards every
/// header (ext_authz gRPC default); `disallowed` always wins. Names are matched
/// case-insensitively via `HeaderName`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HeaderFilter {
	#[serde(skip_serializing_if = "Vec::is_empty", serialize_with = "ser_header_names")]
	pub allowed: Vec<::http::HeaderName>,
	#[serde(skip_serializing_if = "Vec::is_empty", serialize_with = "ser_header_names")]
	pub disallowed: Vec<::http::HeaderName>,
}

impl HeaderFilter {
	fn is_default(&self) -> bool {
		self.allowed.is_empty() && self.disallowed.is_empty()
	}
	/// Whether a header with this name should be sent to the policy server.
	pub fn allows(&self, name: &::http::HeaderName) -> bool {
		if self.disallowed.iter().any(|n| n == name) {
			return false;
		}
		self.allowed.is_empty() || self.allowed.iter().any(|n| n == name)
	}
}

fn ser_header_names<S: serde::Serializer>(
	names: &[::http::HeaderName],
	s: S,
) -> Result<S::Ok, S::Error> {
	use serde::ser::SerializeSeq;
	let mut seq = s.serialize_seq(Some(names.len()))?;
	for n in names {
		seq.serialize_element(n.as_str())?;
	}
	seq.end()
}

// Behavior when a driver errors or returns an unhandleable response.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FailureMode {
	Allow,
	#[default]
	Deny,
}

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
		req_ctx: &mut IncomingRequestContext,
		client: &PolicyClient,
	) -> Outcome {
		match self {
			Driver::Remote(remote) => {
				client::check_request(
					remote,
					ctx.method,
					ctx.backend,
					ctx.params.as_deref_mut(),
					req_ctx,
					client,
				)
				.await
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

/// Drivers fire in order; first `Reject` short-circuits leaving `ctx` in whatever
/// partially-mutated state earlier drivers produced. When `ctx.params` is `None`
/// (e.g. `*/list`) mutations are discarded — list filtering belongs in the response phase.
pub async fn run_call_request(
	ext: &ExtMcp,
	ctx: &mut CallRequestCtx<'_>,
	req_ctx: &mut IncomingRequestContext,
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

/// Drivers fire in order; first `Reject` short-circuits.
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
