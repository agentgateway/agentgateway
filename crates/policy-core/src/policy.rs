//! Policy execution context and phase-specific policy contracts.

use crate::{
	BackendDispatcher, BoxError, Expression, PolicyCel, PolicyTrace, install_policy_trace,
	policy_trace,
};

/// Services supplied by the host for one policy invocation.
///
/// The context is passed by value because an invocation consumes it once.
/// Policies can borrow CEL and tracing services and optionally dispatch a
/// callout through a backend selected by the host.
pub struct PolicyContext<'a> {
	backend: Option<BackendDispatcher>,
	cel: &'a dyn PolicyCel,
}

impl<'a> PolicyContext<'a> {
	/// Creates a context with CEL evaluation and no backend dispatcher.
	pub fn new(cel: &'a dyn PolicyCel) -> Self {
		Self { backend: None, cel }
	}

	/// Adds a dispatcher already bound to the policy's configured backend.
	pub fn with_backend(mut self, backend: BackendDispatcher) -> Self {
		self.backend = Some(backend);
		self
	}

	/// Installs the host's process-wide policy trace implementation.
	pub fn with_trace(self, trace: &'static dyn PolicyTrace) -> Self {
		install_policy_trace(trace);
		self
	}

	/// Returns the configured backend dispatcher, when this policy has one.
	pub fn backend(&self) -> Option<&BackendDispatcher> {
		self.backend.as_ref()
	}

	/// Returns the host CEL evaluator.
	pub fn cel(&self) -> &dyn PolicyCel {
		self.cel
	}

	/// Returns the process-wide policy trace implementation.
	pub fn trace(&self) -> &dyn PolicyTrace {
		policy_trace()
	}
}

/// Policy invoked while processing an inbound request.
#[allow(async_fn_in_trait)]
pub trait RequestPolicy: Send + Sync + 'static {
	async fn apply(
		&self,
		ctx: PolicyContext<'_>,
		req: &mut agent_http::Request,
	) -> Result<agent_http::PolicyResponse, BoxError>;

	/// Returns every CEL expression owned by this policy.
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::empty()
	}
}

/// Policy invoked while processing an upstream response.
#[allow(async_fn_in_trait)]
pub trait ResponsePolicy: Send + Sync + 'static {
	async fn apply_response(
		&self,
		ctx: PolicyContext<'_>,
		resp: &mut agent_http::Response,
	) -> Result<agent_http::PolicyResponse, BoxError>;
}

/// Policy invoked for each backend attempt, including policy callouts.
#[allow(async_fn_in_trait)]
pub trait BackendPolicy: Send + Sync + 'static {
	async fn apply_backend(
		&self,
		ctx: PolicyContext<'_>,
		req: &mut agent_http::Request,
	) -> Result<agent_http::PolicyResponse, BoxError>;

	/// Returns every CEL expression owned by this policy.
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::empty()
	}
}
