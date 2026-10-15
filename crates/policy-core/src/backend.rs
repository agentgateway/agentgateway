//! Type-erased dispatch of policy callouts to gateway-owned backend clients.

use std::error::Error;
use std::fmt::{Debug, Display};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use agent_core::metrics::OutboundCallSubtype;
use agent_http::{Request, Response};

use crate::BoxError;

/// Gateway client capable of calling one concrete backend reference type.
///
/// The gateway implements this trait for its client and backend configuration.
/// [`BackendDispatcher`] erases these generic types before the dispatcher is
/// passed into a policy crate.
pub trait BackendReferenceClient<R, P>: Clone + Send + Sync + 'static {
	type Error: Error + Display + Send + Sync + 'static;

	/// Returns a client tagged with the kind of policy call being made.
	fn with_policy_call(&self, call: OutboundCallSubtype) -> Self;

	/// Sends a request to a backend reference using its backend policies.
	fn call_reference_with_policies(
		&self,
		req: Request,
		backend_ref: &R,
		policies: &[P],
	) -> impl Future<Output = Result<Response, Self::Error>> + Send;
}

/// Type-erased error returned by a [`BackendDispatcher`].
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct BackendError(#[from] BoxError);

type BackendFuture = Pin<Box<dyn Future<Output = Result<Response, BackendError>> + Send + 'static>>;

type DispatchFn = dyn Fn(OutboundCallSubtype, Request) -> BackendFuture + Send + Sync;

/// Type-erased dispatcher bound to one backend reference and its policies.
///
/// Binding happens in the gateway, allowing a policy to make callouts without
/// depending on gateway client or backend configuration types.
#[derive(Clone)]
pub struct BackendDispatcher {
	inner: Arc<DispatchFn>,
}

impl Debug for BackendDispatcher {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BackendDispatcher").finish_non_exhaustive()
	}
}

impl BackendDispatcher {
	/// Binds a typed gateway client, backend reference, and backend policies.
	pub fn new<C, R, P>(client: C, target: Arc<R>, policies: Arc<Vec<P>>) -> Self
	where
		C: BackendReferenceClient<R, P>,
		R: Send + Sync + 'static,
		P: Send + Sync + 'static,
	{
		Self {
			inner: Arc::new(move |call, req| {
				let client = client.with_policy_call(call);
				let target = target.clone();
				let policies = policies.clone();
				Box::pin(async move {
					client
						.call_reference_with_policies(req, target.as_ref(), policies.as_slice())
						.await
						.map_err(|err| BackendError(Box::new(err)))
				})
			}),
		}
	}

	/// Sends an HTTP request to the bound backend.
	pub fn send(&self, call: OutboundCallSubtype, req: Request) -> BackendFuture {
		(self.inner)(call, req)
	}

	/// Adapts the bound backend to a tonic-compatible channel.
	pub fn grpc_channel(&self, call: OutboundCallSubtype) -> BackendChannel {
		BackendChannel {
			dispatcher: self.clone(),
			call,
		}
	}
}

/// Tonic-compatible channel backed by a type-erased [`BackendDispatcher`].
#[derive(Clone, Debug)]
pub struct BackendChannel {
	dispatcher: BackendDispatcher,
	call: OutboundCallSubtype,
}

impl tower::Service<http::Request<tonic::body::Body>> for BackendChannel {
	type Response = Response;
	type Error = BackendError;
	type Future = BackendFuture;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
		self
			.dispatcher
			.send(self.call, req.map(agent_http::Body::new))
	}
}
