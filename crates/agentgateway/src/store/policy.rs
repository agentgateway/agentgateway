use std::sync::Arc;

use http::HeaderMap;
use serde::Serialize;

use crate::cel::{ContextBuilder, Expression};
use crate::http::Response;
use crate::proxy;
use crate::proxy::dtrace;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;

pub trait HasExpressions: Send + Sync + 'static {
	/// Returns a list of expressions that are used in this policy.
	/// Any expressions used in the policy MUST be included here or they will be ignored.
	/// Policies that are also response policies MUST include the response-side expressions as well.
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::empty()
	}
}

impl<T: RequestPolicyTrait> HasExpressions for T {
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		RequestPolicyTrait::expressions(self)
	}
}

/// Request policies are policies that run on the request side. These will run exactly once per request,
/// and are not repeated on retries.
/// Policies that need to do response-time processing will additionally be called on the response phase
/// through the ResponsePolicyTrait trait; the same policy struct will be used for both.
#[allow(async_fn_in_trait)]
pub trait RequestPolicyTrait: Send + Sync + 'static {
	async fn apply(
		&self,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse>;

	/// Returns a list of expressions that are used in this policy.
	/// Any expressions used in the policy MUST be included here or they will be ignored.
	/// Policies that are also response policies MUST include the response-side expressions as well.
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::empty()
	}
}

/// Response policies are policies that run on the response side. The vast majority of the time, these
/// are also request policies that have a response-time component, but it is possible to only be a response policy.
/// These are run exactly once per request, after all retry attempts.
#[allow(async_fn_in_trait)]
pub trait ResponsePolicyTrait: Send + Sync + 'static {
	async fn apply(
		&self,
		log: &mut RequestLog,
		resp: &mut crate::http::Response,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse>;
}

/// A backend policy is a policy that runs per-backend. These are similar to request policies except run on
/// each backend call. This could be a retry attempt or a policy call.
#[allow(async_fn_in_trait)]
pub trait BackendPolicyTrait: Send + Sync + 'static {
	async fn apply(
		&self,
		log: &mut Option<&mut RequestLog>,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse>;

	/// Returns a list of expressions that are used in this policy.
	/// Any expressions used in the policy MUST be included here or they will be ignored.
	/// Policies that are also response policies MUST include the response-side expressions as well.
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		std::iter::empty()
	}
}

/// RequestPolicy is a wrapper around a request policy implementation to handle common construction
/// and usage around conditional policies, etc.
/// This is cheaply clone-able.
#[derive(Debug)]
pub enum RequestPolicy<T> {
	Empty,
	Single(PolicyWithCondition<T>),
	/// Multiple policies are run in order, and the first one that matches is used.
	/// In the future, we *may* support running all matches instead of the first.
	Multiple(Vec<PolicyWithCondition<T>>),
}
impl<T> Default for RequestPolicy<T> {
	fn default() -> RequestPolicy<T> {
		Self::Empty
	}
}

impl<T> Clone for RequestPolicy<T> {
	fn clone(&self) -> Self {
		match self {
			RequestPolicy::Empty => RequestPolicy::Empty,
			RequestPolicy::Single(inner) => RequestPolicy::Single(inner.clone()),
			RequestPolicy::Multiple(inners) => RequestPolicy::Multiple(inners.clone()),
		}
	}
}

#[derive(Debug, Serialize)]
pub struct PolicyWithCondition<T> {
	pub pol: Arc<T>,
	pub condition: Option<Arc<crate::cel::Expression>>,
}

impl<T> Clone for PolicyWithCondition<T> {
	fn clone(&self) -> Self {
		Self {
			pol: self.pol.clone(),
			condition: self.condition.clone(),
		}
	}
}

impl<T: Serialize> Serialize for RequestPolicy<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		match self {
			RequestPolicy::Empty => serializer.serialize_none(),
			RequestPolicy::Single(inner) if inner.condition.is_none() => inner.pol.serialize(serializer),
			RequestPolicy::Single(inner) => inner.serialize(serializer),
			RequestPolicy::Multiple(inners) => inners.serialize(serializer),
		}
	}
}

impl<T> RequestPolicy<T> {
	pub fn single(pol: T) -> Self {
		RequestPolicy::Single(PolicyWithCondition {
			pol: Arc::new(pol),
			condition: None,
		})
	}

	pub fn from_policy_inners<I>(policies: I) -> Self
	where
		I: IntoIterator<Item = PolicyWithCondition<T>>,
	{
		let policies = policies.into_iter().collect::<Vec<_>>();
		match policies.len() {
			0 => RequestPolicy::Empty,
			1 => RequestPolicy::Single(policies.into_iter().next().expect("len checked")),
			_ => RequestPolicy::Multiple(policies),
		}
	}

	pub fn from_policies<I>(policies: I) -> Self
	where
		I: IntoIterator<Item = (T, Option<Arc<crate::cel::Expression>>)>,
	{
		Self::from_policy_inners(
			policies
				.into_iter()
				.map(|(pol, condition)| PolicyWithCondition {
					pol: Arc::new(pol),
					condition,
				}),
		)
	}

	pub fn into_policy_inners(self) -> Vec<PolicyWithCondition<T>> {
		match self {
			RequestPolicy::Empty => Vec::new(),
			RequestPolicy::Single(inner) => vec![inner],
			RequestPolicy::Multiple(inners) => inners,
		}
	}

	pub fn iter(&self) -> impl Iterator<Item = &PolicyWithCondition<T>> {
		match self {
			RequestPolicy::Empty => [].as_slice(),
			RequestPolicy::Single(inner) => std::slice::from_ref(inner),
			RequestPolicy::Multiple(inners) => inners.as_slice(),
		}
		.iter()
	}

	/// Selects the first matching policy without applying it.
	///
	/// This is for policies whose selected config must be turned into separate per-request state
	/// before it can run. ExtProc uses this to select an `ExtProc`, build an `ExtProcRequest`,
	/// and keep that request state for the response phase.
	pub fn select(&self, name: &'static str, req: &crate::http::Request) -> Option<Arc<T>> {
		for pol in self.iter() {
			if let Some(cond) = &pol.condition {
				let exec = crate::cel::Executor::new_request(req);
				if !exec.eval_bool(cond.as_ref()) {
					dtrace::pol_result!(
						name,
						dtrace::Info,
						Skip,
						"condition not met, skipping policy"
					);
					continue;
				};
			};
			return Some(pol.pol.clone());
		}
		None
	}

	pub fn set_if_unset(&mut self, policy: &RequestPolicy<T>) {
		if matches!(self, RequestPolicy::Empty) {
			*self = policy.clone();
		}
	}
}

impl<T: HasExpressions> RequestPolicy<T> {
	pub(crate) fn register_expressions(&self, ctx: &mut ContextBuilder) {
		for p in self.iter() {
			if let Some(c) = p.condition.as_ref() {
				ctx.register_expression(c)
			}
			for expr in p.pol.expressions() {
				ctx.register_expression(expr)
			}
		}
	}
}

impl<T: RequestPolicyTrait> RequestPolicy<T> {
	/// apply_without_response runs the request policy for policy types that do NOT implement response
	/// policies.
	pub async fn apply_without_response(
		&self,
		name: &'static str,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut crate::http::Request,
		response_headers: &mut HeaderMap,
	) -> Result<(), proxy::ProxyResponse> {
		self
			.apply_internal(name, client, log, req, response_headers)
			.await
			.map(|_| ())
	}

	/// apply_selected runs the request policy and returns the policy that was run.
	pub async fn apply_selected(
		&self,
		name: &'static str,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut crate::http::Request,
		response_headers: &mut HeaderMap,
	) -> Result<Option<Arc<T>>, proxy::ProxyResponse> {
		self
			.apply_internal(name, client, log, req, response_headers)
			.await
	}

	async fn apply_internal(
		&self,
		name: &'static str,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut crate::http::Request,
		response_headers: &mut HeaderMap,
	) -> Result<Option<Arc<T>>, proxy::ProxyResponse> {
		for pol in self.iter() {
			if let Some(cond) = &pol.condition {
				let exec = crate::cel::Executor::new_request(req);
				if !exec.eval_bool(cond.as_ref()) {
					dtrace::pol_result!(
						name,
						dtrace::Info,
						Skip,
						"condition not met, skipping policy"
					);
					continue;
				};
			};
			let res = pol
				.pol
				.apply(client, log, req)
				.await?
				.apply(response_headers);
			dtrace::snapshot!(Request, name, &req);
			// Return the policy, for response handling.
			// Conditions are ignored; we already evaluated them on the request side
			// We do not allow response-side conditions
			return res.map(|_| Some(pol.pol.to_owned()));
		}
		Ok(None)
	}
}

impl<T: RequestPolicyTrait + ResponsePolicyTrait> RequestPolicy<T> {
	/// Apply applies request policies and returns back the RespnsePolicy to run the response side.
	pub async fn apply(
		&self,
		name: &'static str,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut crate::http::Request,
		response_headers: &mut HeaderMap,
	) -> Result<ResponsePolicy<T>, proxy::ProxyResponse> {
		self
			.apply_internal(name, client, log, req, response_headers)
			.await
			.map(|p| ResponsePolicy(p))
	}
}

#[must_use]
#[derive(Debug, Clone, Serialize)]
pub struct ResponsePolicy<T>(Option<Arc<T>>);

impl<T> Default for ResponsePolicy<T> {
	fn default() -> Self {
		Self(None)
	}
}

impl<T: ResponsePolicyTrait + BackendPolicyTrait> BackendPolicy<T> {
	pub fn as_response_policy(&self) -> ResponsePolicy<T> {
		ResponsePolicy(self.0.clone())
	}
}
impl<T: ResponsePolicyTrait> ResponsePolicy<T> {
	pub async fn apply(
		&self,
		name: &'static str,
		log: &mut RequestLog,
		resp: &mut Response,
		response_headers: &mut HeaderMap,
	) -> Result<(), proxy::ProxyResponse> {
		let Some(ref pol) = self.0 else { return Ok(()) };
		let res = pol.apply(log, resp).await?.apply(response_headers);
		dtrace::snapshot!(Response, name, log, &resp);
		res
	}

	pub fn set_if_unset(&mut self, pol: &Arc<T>) {
		if self.0.is_none() {
			self.0 = Some(pol.clone());
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendPolicy<T>(Option<Arc<T>>);

impl<T> Default for BackendPolicy<T> {
	fn default() -> Self {
		Self(None)
	}
}

impl<T: BackendPolicyTrait> BackendPolicy<T> {
	pub fn as_ref(&self) -> Option<&Arc<T>> {
		self.0.as_ref()
	}
	pub fn or(self, other: Self) -> Self {
		BackendPolicy(self.0.or(other.0))
	}

	pub fn set_if_unset(&mut self, pol: &Arc<T>) {
		if self.0.is_none() {
			self.0 = Some(pol.clone());
		}
	}

	pub async fn apply(
		&self,
		name: &'static str,
		log: &mut Option<&mut RequestLog>,
		req: &mut crate::http::Request,
		response_headers: &mut HeaderMap,
	) -> Result<ResponsePolicy<T>, proxy::ProxyResponse> {
		let Some(ref pol) = self.0 else {
			return Ok(ResponsePolicy(None));
		};

		let res = pol.apply(log, req).await?.apply(response_headers);
		dtrace::snapshot!(Request, name, &req);
		res.map(|_| ResponsePolicy(Some(pol.clone())))
	}

	pub fn register_expressions(&self, ctx: &mut ContextBuilder) {
		if let Some(pol) = self.0.as_ref() {
			for expr in pol.expressions() {
				ctx.register_expression(expr)
			}
		}
	}
}
