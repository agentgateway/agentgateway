//! Type-erased dispatch of policy callouts to gateway-owned backend clients.

use std::error::Error;
use std::fmt::{Debug, Display};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use agent_http::{Request, Response};

use crate::BoxError;

/// Identifies the policy operation responsible for an outbound call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyCall {
	ExtAuthz,
}

/// Gateway client capable of calling one concrete backend reference type.
///
/// The gateway implements this trait for its client and backend configuration.
/// [`BackendDispatcher`] erases these generic types before the dispatcher is
/// passed into a policy crate.
pub trait BackendReferenceClient<R, P>: Clone + Send + Sync + 'static {
	type Error: Error + Display + Send + Sync + 'static;

	/// Returns a client tagged with the kind of policy call being made.
	fn with_policy_call(&self, call: PolicyCall) -> Self;

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

trait BackendDispatch: Send + Sync + 'static {
	fn send(&self, call: PolicyCall, req: Request) -> BackendFuture;
}

struct TypedBackendDispatch<C, R, P> {
	client: C,
	target: Arc<R>,
	policies: Arc<Vec<P>>,
}

impl<C, R, P> BackendDispatch for TypedBackendDispatch<C, R, P>
where
	C: BackendReferenceClient<R, P>,
	R: Send + Sync + 'static,
	P: Send + Sync + 'static,
{
	fn send(&self, call: PolicyCall, req: Request) -> BackendFuture {
		let client = self.client.with_policy_call(call);
		let target = self.target.clone();
		let policies = self.policies.clone();
		Box::pin(async move {
			client
				.call_reference_with_policies(req, target.as_ref(), policies.as_slice())
				.await
				.map_err(|err| BackendError(Box::new(err)))
		})
	}
}

/// Type-erased dispatcher bound to one backend reference and its policies.
///
/// Binding happens in the gateway, allowing a policy to make callouts without
/// depending on gateway client or backend configuration types.
#[derive(Clone)]
pub struct BackendDispatcher {
	inner: Arc<dyn BackendDispatch>,
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
			inner: Arc::new(TypedBackendDispatch {
				client,
				target,
				policies,
			}),
		}
	}

	/// Sends an HTTP request to the bound backend.
	pub fn send(&self, call: PolicyCall, req: Request) -> BackendFuture {
		self.inner.send(call, req)
	}

	/// Adapts the bound backend to a tonic-compatible channel.
	pub fn grpc_channel(&self, call: PolicyCall) -> BackendChannel {
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
	call: PolicyCall,
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

/// Tonic-compatible channel retaining concrete gateway backend types.
///
/// Prefer [`BackendDispatcher::grpc_channel`] once a dispatcher has already
/// been constructed.
#[derive(Clone, Debug)]
pub struct GrpcReferenceChannel<C, R, P> {
	pub target: Arc<R>,
	pub client: C,
	pub policies: Arc<Vec<P>>,
}

impl<C, R, P> tower::Service<http::Request<tonic::body::Body>> for GrpcReferenceChannel<C, R, P>
where
	C: BackendReferenceClient<R, P>,
	R: Send + Sync + 'static,
	P: Send + Sync + 'static,
{
	type Response = Response;
	type Error = C::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
		let client = self.client.clone();
		let target = self.target.clone();
		let policies = self.policies.clone();
		let req = req.map(agent_http::Body::new);
		Box::pin(async move {
			client
				.call_reference_with_policies(req, &target, policies.as_slice())
				.await
		})
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;

	use super::*;

	#[derive(Clone)]
	struct TestClient {
		call: Option<PolicyCall>,
		observed: Arc<Mutex<Option<(PolicyCall, String, Vec<String>, String)>>>,
	}

	impl BackendReferenceClient<String, String> for TestClient {
		type Error = std::convert::Infallible;

		fn with_policy_call(&self, call: PolicyCall) -> Self {
			Self {
				call: Some(call),
				observed: self.observed.clone(),
			}
		}

		async fn call_reference_with_policies(
			&self,
			req: Request,
			backend_ref: &String,
			policies: &[String],
		) -> Result<Response, Self::Error> {
			*self.observed.lock().unwrap() = Some((
				self.call.unwrap(),
				backend_ref.clone(),
				policies.to_vec(),
				req.uri().path().to_string(),
			));
			Ok(
				http::Response::builder()
					.status(http::StatusCode::NO_CONTENT)
					.body(agent_http::Body::empty())
					.unwrap(),
			)
		}
	}

	#[tokio::test]
	async fn dispatcher_binds_client_target_and_policies() {
		let observed = Arc::new(Mutex::new(None));
		let dispatcher = BackendDispatcher::new(
			TestClient {
				call: None,
				observed: observed.clone(),
			},
			Arc::new("auth.example".to_string()),
			Arc::new(vec!["tls".to_string()]),
		);
		let request = http::Request::builder()
			.uri("/check")
			.body(agent_http::Body::empty())
			.unwrap();

		let response = dispatcher
			.send(PolicyCall::ExtAuthz, request)
			.await
			.unwrap();

		assert_eq!(response.status(), http::StatusCode::NO_CONTENT);
		assert_eq!(
			observed.lock().unwrap().take(),
			Some((
				PolicyCall::ExtAuthz,
				"auth.example".to_string(),
				vec!["tls".to_string()],
				"/check".to_string(),
			))
		);
	}
}
