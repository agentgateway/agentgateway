//! External MCP policy hooks (extMcp).
//!
//! Single-target methods (`tools/call`, ...) fire server-facing in the upstream's
//! native namespace — drivers see unmuxed names (`echo`, not `serverA_echo`) and the
//! backend name as `service_name`. Fanout methods (`*/list`, ...) run the hook once
//! for the whole client call (request hook before fanout, response hook on the merged
//! result). Names and `service_name` there match the client-facing view, which tracks
//! the multiplexing config rather than the method: muxed names and all backend names
//! joined when multiplexing, but a single backend's unmuxed names and lone name when
//! there is just one (the usual single-backend case).

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

#[apply(schema!)]
#[derive(Default)]
pub struct ExtMcp {
	// Processed in order; first `Reject` short-circuits. Drivers may run on the request
	// or response side, or both; see `Driver.methods`.
	#[cfg_attr(feature = "schema", schemars(length(min = 1)))]
	pub drivers: Vec<Driver>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Driver {
	/// Allowlist: only methods listed here run through this driver, at the
	/// configured phase. Keys may be exact (`tools/call`), prefix (`tools/*`),
	/// or suffix (`*/list`) wildcards, or `*` for all methods. Methods matching
	/// no key bypass this driver; see [`phase::resolve`] for match precedence.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub methods: HashMap<String, Phase>,
	#[serde(flatten)]
	pub kind: DriverKind,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

	/// Config warnings to surface at load time (xds diagnostics or logs).
	pub fn load_warnings(&self) -> Vec<String> {
		let mut out = Vec::new();
		for m in methods::REQUEST_PHASE_UNSUPPORTED {
			if self.runs_request(m) {
				out.push(format!(
					"extMcp: methods match {m:?} with a request phase, but only the response phase runs for this method"
				));
			}
		}
		out
	}
}

// TLS, retries, and load balancing come from the backend referenced by `target`.
#[apply(schema!)]
pub struct Remote {
	/// Reference to the external MCP policy service backend.
	pub target: Arc<SimpleBackendReference>,
	/// Behavior when the driver is unavailable or returns an error.
	#[serde(default)]
	pub failure_mode: FailureMode,
	/// CEL expressions evaluated per request and sent to the driver as metadata.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub metadata: HashMap<String, Arc<cel::Expression>>,
	/// Which incoming request headers are forwarded to the policy server.
	#[serde(default, skip_serializing_if = "HeaderFilter::is_default")]
	pub request_headers: HeaderFilter,
}

/// Allow/deny filter over request headers, mirroring ext_authz: empty `allowed`
/// forwards every header plus all pseudo-headers (`:authority`, `:method`, ...);
/// a non-empty `allowed` forwards only the listed names. `disallowed` always
/// wins. Header names match case-insensitively; pseudo-headers match exactly.
#[apply(schema!)]
#[derive(Default)]
pub struct HeaderFilter {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub allowed: Vec<crate::http::HeaderOrPseudo>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
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
	#[default]
	FailClosed,
	FailOpen,
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn deser_local_config() {
		let cfg = r#"
drivers:
  - kind: remote
    methods: { "tools/call": request, "*/list": response }
    target: { host: 127.0.0.1:9999 }
    failureMode: failOpen
    requestHeaders:
      allowed: [x-tenant]
      disallowed: [":authority"]
  - kind: remote
    methods: { "tools/call": full }
    target: { backend: my-backend }
"#;
		let ext: ExtMcp = serde_yaml::from_str(cfg).expect("deser ExtMcp");
		assert_eq!(ext.drivers.len(), 2);

		let d0 = &ext.drivers[0];
		assert_eq!(d0.methods.get("tools/call"), Some(&Phase::Request));
		assert_eq!(d0.methods.get("*/list"), Some(&Phase::Response));
		let DriverKind::Remote(r0) = &d0.kind;
		assert!(matches!(
			r0.target.as_ref(),
			SimpleBackendReference::InlineBackend(_)
		));
		assert_eq!(r0.failure_mode, FailureMode::FailOpen);
		assert_eq!(r0.request_headers.allowed.len(), 1);
		assert!(
			r0.request_headers
				.disallowed
				.contains(&crate::http::HeaderOrPseudo::Authority)
		);

		let DriverKind::Remote(r1) = &ext.drivers[1].kind;
		assert!(matches!(
			r1.target.as_ref(),
			SimpleBackendReference::Backend(_)
		));
		assert_eq!(r1.failure_mode, FailureMode::FailClosed);
	}

	fn ext_with_methods(pairs: &[(&str, Phase)]) -> ExtMcp {
		ExtMcp {
			drivers: vec![Driver {
				methods: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
				kind: DriverKind::Remote(Remote {
					target: Arc::new(SimpleBackendReference::Backend("b".into())),
					failure_mode: FailureMode::default(),
					metadata: HashMap::new(),
					request_headers: HeaderFilter::default(),
				}),
			}],
		}
	}

	#[test]
	fn warns_on_request_phase_for_unsupported_methods() {
		// A catchall request phase matches subscribe/unsubscribe/complete, none of
		// which run the request hook.
		let warnings = ext_with_methods(&[("*", Phase::Full)]).load_warnings();
		assert_eq!(warnings.len(), 3, "{warnings:?}");
		assert!(warnings[0].contains("resources/subscribe"));

		// Response-only and supported-method configs are clean.
		assert!(
			ext_with_methods(&[("*", Phase::Response), ("tools/call", Phase::Full)])
				.load_warnings()
				.is_empty()
		);
	}
}
