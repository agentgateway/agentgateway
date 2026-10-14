//! Remote and in-process CEL policy hooks for MCP (mcpGuardrails).
//!
//! Single-target methods (`tools/call`, ...) fire server-facing in the upstream's
//! native namespace — processors see unmuxed names (`echo`, not `serverA_echo`) and the
//! lone backend name in `service_names`. Fanout methods (`*/list`, ...) run the hook
//! once for the whole client call (request hook before fanout, response hook on the
//! merged result). Names there match the client-facing view, which tracks the
//! multiplexing config rather than the method: muxed names when multiplexing, a single
//! backend's unmuxed names when there is just one (the usual single-backend case).
//! `service_names` lists every fanned-out backend either way.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use rmcp::model::RequestMetaObject;

use crate::mcp::upstream::IncomingRequestContext;
use crate::proxy::httpproxy::PolicyClient;
#[cfg(test)]
use crate::types::agent::SimpleBackendReference;
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::*;

/// Per-request bag of values that `mcpGuardrails` request-phase processors attach via
/// `McpRequestResult.metadata`. Merged into the request extensions and exposed
/// to CEL as `mcpGuardrails.<key>` for backend request filters (e.g. `transformation`).
/// Multiple processors merge into the same map; later writes win on key collisions.
#[apply(schema!)]
#[derive(Default, ::cel::DynamicType)]
pub struct McpGuardrailsDynamicMetadata(serde_json::Map<String, serde_json::Value>);

impl McpGuardrailsDynamicMetadata {
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}
}

mod client;
mod expression;
pub mod methods;
pub mod phase;

pub use phase::Phase;

#[derive(Debug)]
pub enum Outcome<T> {
	Pass,
	Mutated(T),
	Reject(rmcp::model::ErrorData),
}

pub mod wire {
	pub use protos::ext_mcp::*;
}

#[apply(schema!)]
#[derive(Default)]
pub struct McpGuardrails {
	/// Ordered list of policy processors applied to matched methods; the first
	/// to reject a request short-circuits the chain. Processors may run on the
	/// request or response side, or both; see `Processor.methods`.
	pub processors: Vec<Processor>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
// Flattened alternatives must reject fields belonging to a different action.
#[cfg_attr(feature = "schema", schemars(extend("unevaluatedProperties" = false)))]
pub struct Processor {
	/// Allowlist: only methods listed here run through this processor, at the
	/// configured phase. Keys may be exact (`tools/call`), prefix (`tools/*`),
	/// or suffix (`*/list`) wildcards, or `*` for all methods. Methods matching
	/// no key bypass this processor; see [`phase::resolve`] for match precedence.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub methods: HashMap<String, Phase>,
	#[serde(flatten)]
	pub kind: ProcessorKind,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ProcessorKind {
	Remote(Remote),
	Expression(ExpressionProcessor),
}

/// In-process guardrail driven by CEL expressions.
#[apply(schema!)]
pub struct ExpressionProcessor {
	/// Condition gating the action; absent means always.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub condition: Option<Arc<cel::Expression>>,
	#[serde(flatten)]
	pub action: ExpressionAction,
}

#[apply(schema!)]
pub enum ExpressionAction {
	/// Reject with this message. Exactly one of `reject` or `transform` is required.
	// TODO: make this a CEL expression.
	Reject(String),
	/// Returns a replacement body (`mcp.params` on requests or `mcp.result` on responses).
	/// Use `merge` to preserve fields you do not wish to mutate; `null` leaves the body unchanged.
	Transform(Arc<cel::Expression>),
}

impl McpGuardrails {
	/// Whether any processor runs the request side for `method`.
	pub fn runs_request(&self, method: &str) -> bool {
		self.processors.iter().any(|d| d.runs_request(method))
	}

	/// Whether any processor runs the response side for `method`.
	pub fn runs_response(&self, method: &str) -> bool {
		self.processors.iter().any(|d| d.runs_response(method))
	}

	/// Expressions requiring CEL attribute registration.
	pub fn expressions(&self) -> impl Iterator<Item = &cel::Expression> {
		self
			.processors
			.iter()
			.flat_map(|p| -> Box<dyn Iterator<Item = _>> {
				match &p.kind {
					ProcessorKind::Remote(r) => Box::new(r.metadata.values().map(|e| e.as_ref())),
					ProcessorKind::Expression(c) => {
						let transform = match &c.action {
							ExpressionAction::Transform(t) => Some(t),
							ExpressionAction::Reject(_) => None,
						};
						Box::new(c.condition.iter().chain(transform).map(|e| e.as_ref()))
					},
				}
			})
	}

	/// Config warnings to surface at load time (xds diagnostics or logs).
	pub fn load_warnings(&self) -> Vec<String> {
		let mut out = Vec::new();
		for m in methods::REQUEST_PHASE_UNSUPPORTED {
			if self.runs_request(m) {
				out.push(format!(
					"mcpGuardrails: methods match {m:?} with a request phase, but only the response phase runs for this method"
				));
			}
		}
		let mut bad_patterns: Vec<_> = self
			.processors
			.iter()
			.flat_map(|d| d.methods.keys())
			.filter(|p| !phase::pattern_is_matchable(p))
			.map(|p| {
				format!(
					"mcpGuardrails: methods key {p:?} can never match; use an exact method, 'prefix/*', '*/suffix', or '*'"
				)
			})
			.collect();
		bad_patterns.sort();
		out.append(&mut bad_patterns);
		out
	}
}

// Retries and load balancing come from the backend referenced by `target`;
// TLS/auth may also be set inline via `policies`.
#[apply(schema!)]
pub struct Remote {
	/// Reference to the external MCP policy service backend and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
	/// Behavior when the processor is unavailable or returns an error.
	#[serde(default)]
	pub failure_mode: FailureMode,
	/// CEL expressions evaluated per request and sent to the processor as metadata.
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
	/// Headers to forward; an empty list forwards all headers.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub allowed: Vec<crate::http::HeaderOrPseudo>,
	/// Headers to drop; takes precedence over the allow list.
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

// Behavior when a processor errors or returns an unhandleable response.
#[apply(schema_enum!)]
#[derive(Default)]
pub enum FailureMode {
	#[default]
	FailClosed,
	FailOpen,
}

/// Parsed MCP body with lazy wire and CEL caches shared across guardrail processors.
#[derive(Debug)]
pub(crate) struct MCPBody<'a, T> {
	original: Option<&'a T>,
	pub updated: Option<T>,
	pub wire: Option<bytes::Bytes>,

	/// Fanout requests have no rewritable body, but may still carry request metadata.
	meta: Option<&'a RequestMetaObject>,
	value: OnceLock<Result<::cel::Value<'static>, ::cel::SerializationError>>,
}

impl<'a, T: serde::Serialize> MCPBody<'a, T> {
	pub fn new(body: Option<&'a T>) -> Self {
		Self {
			original: body,
			updated: None,
			wire: None,
			meta: None,
			value: OnceLock::new(),
		}
	}

	pub fn with_meta(mut self, meta: &'a RequestMetaObject) -> Self {
		self.meta = (!meta.0.0.is_empty()).then_some(meta);
		self
	}

	pub fn parsed(&self) -> Option<&T> {
		self.updated.as_ref().or(self.original)
	}

	pub fn is_present(&self) -> bool {
		self.parsed().is_some() || self.meta.is_some()
	}

	// Each processor has already updated or invalidated the wire representation.
	pub fn replace(&mut self, body: T) {
		self.updated = Some(body);
		let _ = self.value.take();
	}

	pub fn wire(&mut self) -> Result<Option<&mut bytes::Bytes>, serde_json::Error> {
		if self.wire.is_none() {
			self.wire = self
				.parsed()
				.map(serde_json::to_vec)
				.transpose()?
				.map(Into::into);
		}
		Ok(self.wire.as_mut())
	}

	pub fn value(&self) -> Result<&::cel::Value<'static>, &::cel::SerializationError> {
		self
			.value
			.get_or_init(|| {
				if let Some(body) = self.parsed() {
					::cel::to_value(body)
				} else {
					#[derive(serde::Serialize)]
					struct Params<'a> {
						#[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
						meta: Option<&'a RequestMetaObject>,
					}
					::cel::to_value(Params { meta: self.meta })
				}
			})
			.as_ref()
	}

	pub fn error(&self) -> Option<&::cel::SerializationError> {
		self.value.get().and_then(|v| v.as_ref().err())
	}
}

impl<T: serde::Serialize + std::fmt::Debug + Send + Sync> ::cel::types::dynamic::DynamicType
	for MCPBody<'_, T>
{
	fn field(&self, name: &str) -> Option<::cel::Value<'_>> {
		let ::cel::Value::Map(map) = self.value().ok()? else {
			return None;
		};
		map.get(&name.into()).cloned()
	}

	fn materialize(&self) -> ::cel::Value<'_> {
		// Callers check error() even when has(...) masks a failed field lookup.
		self.value().cloned().unwrap_or(::cel::Value::Null)
	}
}

/// Shared request state for the processor chain. Bodyless fanout requests can
/// expose metadata, but their mutations are logged and discarded.
pub struct CallRequestCtx<'a, P> {
	pub backends: &'a [String],
	pub method: &'a str,
	pub(crate) body: MCPBody<'a, P>,
}

impl Processor {
	fn runs_request(&self, method: &str) -> bool {
		phase::resolve(method, &self.methods).runs_request()
	}

	fn runs_response(&self, method: &str) -> bool {
		phase::resolve(method, &self.methods).runs_response()
	}

	async fn call_request<
		P: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + Sync,
	>(
		&self,
		ctx: &mut CallRequestCtx<'_, P>,
		req_ctx: &mut IncomingRequestContext,
		client: &PolicyClient,
	) -> Outcome<P> {
		match &self.kind {
			ProcessorKind::Expression(c) => {
				expression::check(c, ctx.method, &mut ctx.body, req_ctx, false)
			},
			ProcessorKind::Remote(remote) => {
				client::check_request(
					remote,
					ctx.method,
					ctx.backends,
					&mut ctx.body,
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
		backends: &[String],
		body: &mut MCPBody<'_, rmcp::model::ServerResult>,
		req_ctx: &IncomingRequestContext,
		client: &PolicyClient,
	) -> Outcome<rmcp::model::ServerResult> {
		match &self.kind {
			ProcessorKind::Expression(c) => expression::check(c, method, body, req_ctx, true),
			ProcessorKind::Remote(remote) => {
				client::check_response(remote, method, backends, body, req_ctx, client).await
			},
		}
	}
}

/// Processors fire in order. CEL borrows the current parsed params; only remote
/// processors need wire bytes. A CEL mutation invalidates those bytes.
pub async fn run_call_request<
	P: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + Sync,
>(
	ext: &McpGuardrails,
	ctx: &mut CallRequestCtx<'_, P>,
	req_ctx: &mut IncomingRequestContext,
	client: &PolicyClient,
) -> Outcome<P> {
	let client = client.with_parent_extensions(req_ctx.extensions());
	for processor in &ext.processors {
		if !processor.runs_request(ctx.method) {
			continue;
		}
		let outcome = processor.call_request(ctx, req_ctx, &client).await;
		match outcome {
			Outcome::Pass => {},
			Outcome::Mutated(p) => ctx.body.replace(p),
			Outcome::Reject(e) => return Outcome::Reject(e),
		}
	}
	ctx
		.body
		.updated
		.take()
		.map_or(Outcome::Pass, Outcome::Mutated)
}

pub async fn run_response(
	ext: &McpGuardrails,
	method: &str,
	backends: &[String],
	result: &rmcp::model::ServerResult,
	req_ctx: &IncomingRequestContext,
	client: &PolicyClient,
) -> Outcome<rmcp::model::ServerResult> {
	let client = client.with_parent_extensions(req_ctx.extensions());
	let mut body = MCPBody::new(Some(result));
	for processor in &ext.processors {
		if !processor.runs_response(method) {
			continue;
		}
		let outcome = processor
			.response(method, backends, &mut body, req_ctx, &client)
			.await;
		match outcome {
			Outcome::Pass => {},
			Outcome::Mutated(r) => body.replace(r),
			Outcome::Reject(e) => return Outcome::Reject(e),
		}
	}
	body.updated.map_or(Outcome::Pass, Outcome::Mutated)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn deser_local_config() {
		let cfg = r#"
processors:
  - kind: remote
    methods: { "tools/call": request, "*/list": response }
    host: 127.0.0.1:9999
    policies:
      backendTLS: {}
    failureMode: failOpen
    requestHeaders:
      allowed: [x-tenant]
      disallowed: [":authority"]
  - kind: remote
    methods: { "tools/call": full }
    backend: my-backend
  - kind: expression
    methods: { "tools/call": request }
    condition: 'mcp.tool.name == "drop_table"'
    reject: drop_table requires admin
"#;
		let ext: McpGuardrails = serde_norway::from_str(cfg).expect("deser McpGuardrails");
		assert_eq!(ext.processors.len(), 3);
		assert!(ext.load_warnings().is_empty());

		let d0 = &ext.processors[0];
		assert_eq!(d0.methods.get("tools/call"), Some(&Phase::Request));
		assert_eq!(d0.methods.get("*/list"), Some(&Phase::Response));
		let ProcessorKind::Remote(r0) = &d0.kind else {
			panic!("expected remote")
		};
		assert!(matches!(
			r0.target.target.as_ref(),
			SimpleBackendReference::InlineBackend(_)
		));
		assert_eq!(r0.failure_mode, FailureMode::FailOpen);
		assert_eq!(r0.target.policies.len(), 1, "backendTLS should translate");
		assert_eq!(r0.request_headers.allowed.len(), 1);
		assert!(
			r0.request_headers
				.disallowed
				.contains(&crate::http::HeaderOrPseudo::Authority)
		);

		let ProcessorKind::Remote(r1) = &ext.processors[1].kind else {
			panic!("expected remote")
		};
		assert!(matches!(
			r1.target.target.as_ref(),
			SimpleBackendReference::Backend(_)
		));
		assert_eq!(r1.failure_mode, FailureMode::FailClosed);

		let ProcessorKind::Expression(c2) = &ext.processors[2].kind else {
			panic!("expected expression")
		};
		assert!(c2.condition.is_some());
		assert!(matches!(&c2.action, ExpressionAction::Reject(m) if m == "drop_table requires admin"));
	}

	#[test]
	fn expression_requires_one_action() {
		for (action, valid) in [
			("", false),
			("reject: denied", true),
			("transform: mcp.params", true),
			("reject: denied\n    transform: mcp.params", false),
			("transform: mcp.params\n    reject: denied", false),
			("reject: null", false),
			("transform: null", false),
			("reject: denied\n    transform: null", false),
			("reject: null\n    transform: mcp.params", false),
		] {
			let cfg = format!(
				"processors:\n  - kind: expression\n    methods: {{ 'tools/call': request }}\n    {action}\n"
			);
			let parsed = serde_norway::from_str::<McpGuardrails>(&cfg);
			assert_eq!(parsed.is_ok(), valid, "{cfg}: {parsed:?}");
		}
	}

	fn ext_with_methods(pairs: &[(&str, Phase)]) -> McpGuardrails {
		McpGuardrails {
			processors: vec![Processor {
				methods: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
				kind: ProcessorKind::Remote(Remote {
					target: SimpleBackendReferenceWithPolicies {
						target: Arc::new(SimpleBackendReference::Backend("b".into())),
						policies: Vec::new(),
					},
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

	#[test]
	fn warns_on_unmatchable_method_patterns() {
		let warnings = ext_with_methods(&[
			("a*b", Phase::Response),
			("**", Phase::Response),
			("", Phase::Response),
			("tools/*", Phase::Response),
			("*/list", Phase::Response),
		])
		.load_warnings();
		assert_eq!(warnings.len(), 3, "{warnings:?}");
		assert!(warnings.iter().all(|w| w.contains("can never match")));
	}
}
