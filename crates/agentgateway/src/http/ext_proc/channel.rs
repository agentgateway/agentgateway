use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::http;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference};

#[derive(Clone, Debug)]
pub struct GrpcReferenceChannel {
	pub target: Arc<SimpleBackendReference>,
	pub client: PolicyClient,
	pub policies: Arc<Vec<BackendTrafficPolicy>>,
}

impl tower::Service<::http::Request<tonic::body::Body>> for GrpcReferenceChannel {
	type Response = http::Response;
	type Error = ProxyError;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Ok(()).into()
	}

	fn call(&mut self, req: ::http::Request<tonic::body::Body>) -> Self::Future {
		let client = self.client.clone();
		let target = self.target.clone();
		let policies = self.policies.clone();
		let req = req.map(http::Body::new);
		Box::pin(async move {
			client
				.call_reference_with_policies(req, &target, policies.as_slice())
				.await
		})
	}
}
