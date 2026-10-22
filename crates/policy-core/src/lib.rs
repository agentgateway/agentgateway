//! Shared contracts for independently compiled agentgateway policies.
//!
//! This crate is the dependency boundary between policy implementations and the
//! gateway runtime. Policies depend on stable HTTP types plus the interfaces
//! here; the gateway supplies CEL evaluation, tracing, and backend dispatch.
//! Keeping gateway-owned request snapshots and clients behind these interfaces
//! allows each policy crate to compile and test in isolation.
//!
//! ```no_run
//! use std::convert::Infallible;
//!
//! use agent_http::Request;
//! use agent_policy::{BoxError, PolicyContext, RequestAction, RequestPolicy};
//!
//! struct Example;
//!
//! impl RequestPolicy for Example {
//!     type ResponseState = Infallible;
//!
//!     async fn apply(
//!         &self,
//!         _ctx: PolicyContext<'_>,
//!         _request: &mut Request,
//!     ) -> Result<RequestAction<Infallible>, BoxError> {
//!         Ok(RequestAction::default())
//!     }
//! }
//! ```

mod backend;
mod expression;
mod policy;
mod properties;
mod trace;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use agent_core::metrics::OutboundCallSubtype;
pub use backend::{BackendChannel, BackendDispatcher, BackendError, BackendReferenceClient};
pub use expression::{
	Attributes, Error, Expression, PolicyCel, attributes_for, install_custom_function_attributes,
};
pub use policy::{BackendPolicy, PolicyContext, RequestAction, RequestPolicy};
pub use trace::{
	NoopTrace, PolicyOutcome, PolicyTrace, TraceScope, TraceSeverity, install_policy_trace,
	policy_trace,
};

/// Error type returned by policy phase callbacks.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[cfg(test)]
mod tests;
