//! Shared contracts for independently compiled agentgateway policies.
//!
//! This crate is the dependency boundary between policy implementations and the
//! gateway runtime. Policies depend on stable HTTP types plus the interfaces
//! here; the gateway supplies CEL evaluation, tracing, and backend dispatch.
//! Keeping gateway-owned request snapshots and clients behind these interfaces
//! allows each policy crate to compile and test in isolation.
//!
//! A request policy typically implements [`RequestPolicy`] and uses the
//! invocation's [`PolicyContext`] for optional host services:
//!
//! ```no_run
//! use agent_http::{PolicyResponse, Request};
//! use agent_policy::{BoxError, PolicyContext, RequestPolicy};
//!
//! struct Example;
//!
//! impl RequestPolicy for Example {
//!     async fn apply(
//!         &self,
//!         _ctx: PolicyContext<'_>,
//!         _request: &mut Request,
//!     ) -> Result<PolicyResponse, BoxError> {
//!         Ok(PolicyResponse::default())
//!     }
//! }
//! ```

mod backend;
mod expression;
mod policy;
mod trace;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use backend::{
	BackendChannel, BackendDispatcher, BackendError, BackendReferenceClient, GrpcReferenceChannel,
	PolicyCall,
};
pub use expression::{
	Attributes, Error, Expression, PolicyCel, attributes_for_ast, install_custom_function_attributes,
};
pub use policy::{BackendPolicy, PolicyContext, RequestPolicy, ResponsePolicy};
pub use trace::{
	NoopTrace, PolicyTrace, TraceScope, TraceSeverity, install_policy_trace, policy_trace,
};

/// Error type returned by policy phase callbacks.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[cfg(test)]
mod tests;
