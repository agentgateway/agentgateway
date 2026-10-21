use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::client::ResolvedDestination;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::SimpleBackend;

/// HTTP client for MCP upstream backends with optional stateful session affinity.
/// Used by all HTTP-based MCP upstream transports (SSE, StreamableHTTP, OpenAPI).
///
/// When `stateful` is true, this client captures the resolved backend address
/// from the first response and pins all subsequent requests to that same endpoint.
/// This ensures session affinity for stateful MCP backends with multiple replicas.
#[derive(Debug, Clone)]
pub(crate) struct McpHttpClient {
	client: PolicyClient,
	backend: Arc<SimpleBackend>,
	base_policies: BackendPolicies,
	pinned_dest: Arc<Mutex<Option<ResolvedDestination>>>,
	stateful: bool,
	target_name: String,
}

impl McpHttpClient {
	pub fn new(
		client: PolicyClient,
		backend: SimpleBackend,
		policies: BackendPolicies,
		stateful: bool,
		target_name: String,
	) -> Self {
		Self {
			client,
			backend: Arc::new(backend),
			base_policies: policies,
			pinned_dest: Arc::new(Mutex::new(None)),
			stateful,
			target_name,
		}
	}

	pub async fn call(
		&self,
		mut req: crate::http::Request,
	) -> Result<http::Response<crate::http::Body>, ProxyError> {
		if req
			.extensions()
			.get::<crate::telemetry::log::SpanWriter>()
			.is_some_and(|writer| writer.upstream_calls.is_some())
		{
			req
				.extensions_mut()
				.insert(crate::telemetry::upstream::UpstreamTarget(
					self.target_name.clone(),
				));
		}
		let client = self.client.with_parent(&req);
		let mut policies = self.base_policies.clone();

		if self.stateful
			&& policies.override_dest.is_none()
			&& let Some(pinned) = *self.pinned_dest.lock().unwrap()
		{
			tracing::trace!(
				target = %self.target_name,
				backend = %self.backend,
				endpoint = %pinned.0,
				"using pinned backend endpoint"
			);
			policies.override_dest = Some(pinned.0);
		}

		let resp = Box::pin(
			client
				.with_outbound(OutboundCallKind::Primary, OutboundCallSubtype::Mcp)
				.with_dtrace_scope(format!("mcp {}", self.target_name))
				// The Upstream layer owns the semantic MCP span so it can include the MCP method,
				// target, and tool name. Tracing this HTTP transport would create a duplicate span.
				.call_with_explicit_policies_untraced(req, &self.backend, policies),
		)
		.await?;

		// Capture resolved destination on first request if stateful
		if self.stateful
			&& self.pinned_dest.lock().unwrap().is_none()
		// Only pin to services. Pinning to a specific IP for DNS resolution is not appropriate.
		// With Service, we know there are replicas of the pod and we can pin to that.
		&& let SimpleBackend::Service(_, _) = &*self.backend
			&& let Some(resolved) = resp.extensions().get::<ResolvedDestination>()
		{
			self.pin_backend(*resolved);
		}

		Ok(resp)
	}

	pub fn pin_backend(&self, resolved: ResolvedDestination) {
		tracing::debug!(
			target = %self.target_name,
			backend = %self.backend,
			endpoint = %resolved.0,
			"pinned stateful MCP session to backend endpoint"
		);
		*self.pinned_dest.lock().unwrap() = Some(resolved);
	}

	pub fn pinned_backend(&self) -> Option<SocketAddr> {
		Some((*self.pinned_dest.lock().unwrap())?.0)
	}

	pub fn target_name(&self) -> &str {
		&self.target_name
	}

	pub fn backend(&self) -> &SimpleBackend {
		&self.backend
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::Body;
	use crate::telemetry::log::SpanWriter;
	use crate::telemetry::upstream::UpstreamCalls;
	use crate::test_helpers::proxymock::setup_proxy_test;

	#[tokio::test]
	async fn upstream_calls_capture_mcp_fanout_credentials_after_backend_auth() {
		use crate::http::auth::{BackendAuth, BackendAuthKind};
		use crate::store::BackendPolicies;
		use crate::types::agent::{ResourceName, SimpleBackend, Target};

		let server = wiremock::MockServer::start().await;
		wiremock::Mock::given(wiremock::matchers::method("POST"))
			.respond_with(wiremock::ResponseTemplate::new(200))
			.mount(&server)
			.await;
		let calls = UpstreamCalls::default();
		let mut writer = SpanWriter::default();
		writer.upstream_calls = Some(calls.clone());
		let policy_client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi);
		let make_client = |name: &str| {
			McpHttpClient::new(
				policy_client.clone(),
				SimpleBackend::Opaque(
					ResourceName::new(name.into(), "".into()),
					Target::Address(*server.address()),
				),
				BackendPolicies {
					backend_auth: Some(BackendAuth::new(BackendAuthKind::Key {
						value: secrecy::SecretString::from(name.to_owned()).into(),
						location: None,
					})),
					..Default::default()
				},
				false,
				name.to_owned(),
			)
		};
		let make_request = || {
			let mut req = http::Request::builder()
				.method("POST")
				.uri(server.uri())
				.header("authorization", "Bearer inbound")
				.body(Body::empty())
				.unwrap();
			req.extensions_mut().insert(writer.clone());
			req
		};
		let search = make_client("search");
		let tickets = make_client("tickets");
		let (a, b) = tokio::join!(search.call(make_request()), tickets.call(make_request()));
		assert_eq!(a.unwrap().status(), 200);
		assert_eq!(b.unwrap().status(), 200);
		let calls = calls.take();
		assert_eq!(calls.len(), 2);
		for name in ["search", "tickets"] {
			let call = calls.iter().find(|c| c.target == name).unwrap();
			assert_eq!(call.kind, "Primary");
			assert_eq!(call.subtype, "Mcp");
			assert_eq!(
				call.request.headers["authorization"],
				format!("Bearer {name}")
			);
		}
	}
}
