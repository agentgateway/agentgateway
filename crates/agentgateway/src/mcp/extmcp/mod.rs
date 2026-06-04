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
pub mod phase;

pub use phase::Phase;

#[derive(Debug)]
pub enum Outcome {
	Pass,
	Mutated,
	Reject(rmcp::model::ErrorData),
}

pub mod wire {
	pub use protos::ext_mcp::*;
}

#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtMcp {
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub drivers: Vec<Driver>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Driver {
	/// Allowlist: only methods listed here run through this driver, at the
	/// configured phase. Keys may be exact (`tools/call`), prefix (`tools/*`),
	/// or suffix (`*/list`) wildcards, or `*` for all methods. Methods matching
	/// no key bypass this driver; see [`phase::resolve`] for match precedence.
	#[serde(skip_serializing_if = "HashMap::is_empty")]
	pub methods: HashMap<String, Phase>,
	#[serde(flatten)]
	pub kind: DriverKind,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DriverKind {
	Remote(Remote),
}

impl ExtMcp {
	/// Whether any driver runs the request side for `method`.
	pub fn runs_request(&self, method: &str) -> bool {
		self.drivers.iter().any(|d| d.runs_request(method))
	}

	/// Whether any driver runs the response side for `method`.
	pub fn runs_response(&self, method: &str) -> bool {
		self.drivers.iter().any(|d| d.runs_response(method))
	}
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

/// Allow/deny filter over request headers, mirroring ext_authz: empty `allowed`
/// forwards every header plus all pseudo-headers (`:authority`, `:method`, ...);
/// a non-empty `allowed` forwards only the listed names. `disallowed` always
/// wins. Header names match case-insensitively; pseudo-headers match exactly.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HeaderFilter {
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub allowed: Vec<crate::http::HeaderOrPseudo>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub disallowed: Vec<crate::http::HeaderOrPseudo>,
}

impl HeaderFilter {
	fn is_default(&self) -> bool {
		self.allowed.is_empty() && self.disallowed.is_empty()
	}
	/// Whether a header (or pseudo-header) should be sent to the policy server.
	pub fn allows(&self, name: &crate::http::HeaderOrPseudo) -> bool {
		if self.disallowed.iter().any(|n| n == name) {
			return false;
		}
		self.allowed.is_empty() || self.allowed.iter().any(|n| n == name)
	}
}

// Behavior when a driver errors or returns an unhandleable response.
#[apply(schema_enum!)]
#[derive(Default)]
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
	fn runs_request(&self, method: &str) -> bool {
		phase::resolve(method, &self.methods).runs_request()
	}

	fn runs_response(&self, method: &str) -> bool {
		phase::resolve(method, &self.methods).runs_response()
	}

	async fn call_request(
		&self,
		ctx: &mut CallRequestCtx<'_>,
		req_ctx: &mut IncomingRequestContext,
		client: &PolicyClient,
	) -> Outcome {
		match &self.kind {
			DriverKind::Remote(remote) => {
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
		match &self.kind {
			DriverKind::Remote(remote) => {
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
	let mut composed = Outcome::Pass;
	for driver in &ext.drivers {
		if !driver.runs_request(ctx.method) {
			continue;
		}
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
	let mut composed = Outcome::Pass;
	for driver in &ext.drivers {
		if !driver.runs_response(method) {
			continue;
		}
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
