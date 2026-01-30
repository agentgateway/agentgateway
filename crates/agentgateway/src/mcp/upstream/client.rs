use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::client::ResolvedDestination;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
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
		req: http::Request<crate::http::Body>,
	) -> Result<http::Response<crate::http::Body>, ProxyError> {
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

		let resp = self
			.client
			.call_with_default_policies(req, &self.backend, policies)
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

	pub fn backend(&self) -> &SimpleBackend {
		&self.backend
	}
}
