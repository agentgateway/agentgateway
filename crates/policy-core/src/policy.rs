//! Policy execution context and phase-specific policy contracts.

use agent_http::PolicyResponse;

use crate::{BackendDispatcher, BoxError, Expression, PolicyCel};

/// Services supplied by the host for one policy invocation.
///
/// The context is passed by value because an invocation consumes it once.
/// Policies can evaluate CEL and optionally dispatch a callout through a
/// backend selected by the host.
#[non_exhaustive]
pub struct PolicyContext<'a> {
	/// Dispatcher bound to the policy's configured backend, when it has one.
	pub backend: Option<BackendDispatcher>,
	/// Host CEL evaluator.
	pub cel: &'a dyn PolicyCel,
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
}

/// Result of a request policy, optionally carrying state to the response phase.
pub struct RequestAction<S> {
	pub response: PolicyResponse,
	pub response_state: Option<S>,
}

impl<S> RequestAction<S> {
	/// Continues the request and runs the policy's response phase with `state`.
	pub fn with_response_state(state: S) -> Self {
		Self {
			response: PolicyResponse::default(),
			response_state: Some(state),
		}
	}
}

impl<S> Default for RequestAction<S> {
	fn default() -> Self {
		PolicyResponse::default().into()
	}
}

impl<S> From<PolicyResponse> for RequestAction<S> {
	fn from(response: PolicyResponse) -> Self {
		Self {
			response,
			response_state: None,
		}
	}
}

/// Policy invoked while processing an inbound request and, optionally, its response.
#[allow(async_fn_in_trait)]
pub trait RequestPolicy: Send + Sync + 'static {
	/// Per-request state passed from [`Self::apply`] to [`Self::apply_response`].
	///
	/// Request-only policies use [`std::convert::Infallible`], so they cannot return response state.
	type ResponseState: Send + 'static;

	async fn apply(
		&self,
		ctx: PolicyContext<'_>,
		req: &mut agent_http::Request,
	) -> Result<RequestAction<Self::ResponseState>, BoxError>;

	/// Runs on the response of a request whose [`Self::apply`] returned response state.
	async fn apply_response(
		&self,
		_ctx: PolicyContext<'_>,
		_state: Self::ResponseState,
		_resp: &mut agent_http::Response,
	) -> Result<PolicyResponse, BoxError> {
		Ok(PolicyResponse::default())
	}

	/// Returns every CEL expression owned by this policy.
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::empty()
	}
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
