//! In-flight request limits, counted per key on this proxy instance.
//!
//! A rate limit asks how many requests started in a window. This asks how many are running right
//! now: a slot is taken when the request is admitted and given back once its response body has
//! been sent, or when the client goes away. Keys are computed with CEL, so a limit can be per
//! caller, per model, or both. Counters are local to one proxy instance.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::cel::{Executor, Expression};
use crate::http::{Body, Response};
use crate::proxy::ProxyError;
use crate::*;

#[derive(Debug, Default)]
struct Counters(Mutex<HashMap<String, u32>>);

/// A slot taken from a counter. The slot is returned when the permit is dropped.
#[derive(Debug)]
pub struct Permit {
	counters: Arc<Counters>,
	key: String,
}

impl Drop for Permit {
	fn drop(&mut self) {
		let mut counters = self.counters.0.lock();
		if let Some(in_flight) = counters.get_mut(&self.key) {
			*in_flight = in_flight.saturating_sub(1);
			if *in_flight == 0 {
				counters.remove(&self.key);
			}
		}
	}
}

/// What happens to a request whose key cannot be evaluated to a string.
#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "ConcurrencyLimitFailureMode"))]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum FailureMode {
	/// Reject the request with a 500 (default).
	#[default]
	#[serde(rename = "failClosed", alias = "FailClosed")]
	FailClosed,
	/// Let the request through without counting it.
	#[serde(rename = "failOpen", alias = "FailOpen")]
	FailOpen,
}

/// Limits how many requests may be in flight at once for a key.
#[apply(schema!)]
pub struct ConcurrencyLimit {
	/// Maximum number of in-flight requests allowed per key. Requests over the limit are rejected
	/// with a 429.
	pub limit: u32,
	/// CEL expression selecting the counter, for example `jwt.sub` or
	/// `jwt.sub + "/" + llm.requestModel`. Without a key, every request the rule applies to
	/// shares one counter.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub key: Option<Arc<Expression>>,
	/// What happens to a request whose key cannot be evaluated to a string: `failClosed` (default)
	/// rejects it with a 500, `failOpen` lets it through without counting it.
	#[serde(default)]
	pub failure_mode: FailureMode,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	counters: Arc<Counters>,
}

impl ConcurrencyLimit {
	pub fn new(limit: u32, key: Option<Arc<Expression>>, failure_mode: FailureMode) -> Self {
		Self {
			limit,
			key,
			failure_mode,
			counters: Default::default(),
		}
	}

	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.key.iter().map(|e| e.as_ref())
	}

	/// The counter the request counts against, or `None` when its key cannot be evaluated.
	fn evaluate_key(&self, exec: &Executor<'_>) -> Option<String> {
		let Some(expr) = &self.key else {
			return Some(String::new());
		};
		let key = exec
			.eval(expr)
			.map_err(|e| format!("failed to evaluate: {e}"))
			.and_then(|v| v.as_string().map_err(|_| "is not a string".to_string()));
		key
			.inspect_err(|why| debug!(expression = %expr.original_expression, "concurrency key {why}"))
			.ok()
	}

	/// Take a slot for this request, or reject it. A request whose key cannot be evaluated takes
	/// no slot: under `failOpen` it is let through uncounted (`None`).
	pub fn take(&self, exec: &Executor<'_>) -> Result<Option<Permit>, ProxyError> {
		let Some(key) = self.evaluate_key(exec) else {
			return match self.failure_mode {
				FailureMode::FailOpen => Ok(None),
				FailureMode::FailClosed => Err(ProxyError::ConcurrencyLimitKeyFailed),
			};
		};
		let limit = self.limit;
		let mut counters = self.counters.0.lock();
		let in_flight = counters.get(&key).copied().unwrap_or(0);
		if in_flight >= limit {
			return Err(ProxyError::ConcurrencyLimitExceeded { limit, in_flight });
		}
		counters.insert(key.clone(), in_flight + 1);
		Ok(Some(Permit {
			counters: self.counters.clone(),
			key,
		}))
	}

	#[cfg(test)]
	pub fn in_flight(&self, key: &str) -> u32 {
		self.counters.0.lock().get(key).copied().unwrap_or(0)
	}
}

/// The slots one request holds. It lives in the request extensions, so every retry attempt sees
/// the same guard, and in the final response body, so the slots are released when the body is done.
#[derive(Debug, Default)]
pub struct ConcurrencyGuard {
	permits: Mutex<Vec<Permit>>,
}

impl ConcurrencyGuard {
	pub fn hold(&self, permit: Permit) {
		self.permits.lock().push(permit);
	}
}

/// Keep the request's slots until the response body is complete or dropped.
pub fn hold_until_complete(resp: &mut Response, guard: Arc<ConcurrencyGuard>) {
	let body = std::mem::replace(resp.body_mut(), Body::empty());
	*resp.body_mut() = body.with_drop_guard(guard);
}

/// The concurrency limits of one policy; every one of them must admit the request.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
pub struct ConcurrencyLimits(pub Vec<ConcurrencyLimit>);

impl std::ops::Deref for ConcurrencyLimits {
	type Target = Vec<ConcurrencyLimit>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl crate::store::RequestPolicyTrait for ConcurrencyLimits {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		let guard = req
			.extensions_mut()
			.get_or_insert_default::<Arc<ConcurrencyGuard>>()
			.clone();
		let exec = Executor::new_request(req);
		for limit in self.iter() {
			if let Some(permit) = limit
				.take(&exec)
				.map_err(crate::proxy::ProxyResponse::from)?
			{
				guard.hold(permit);
			}
		}
		Ok(Default::default())
	}

	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.iter().flat_map(|l| l.expressions())
	}
}

#[cfg(test)]
#[path = "concurrencylimit_tests.rs"]
mod proxy_tests;

#[cfg(test)]
mod tests {
	use super::*;

	fn acquire(
		limit: &ConcurrencyLimit,
		req: &crate::http::Request,
	) -> Result<Option<Permit>, ProxyError> {
		limit.take(&Executor::new_request(req))
	}

	fn request(user: &str) -> crate::http::Request {
		::http::Request::builder()
			.uri("http://localhost/v1/chat/completions")
			.header("x-user", user)
			.body(Body::empty())
			.unwrap()
	}

	fn keyed(limit: u32, key: &str, failure_mode: FailureMode) -> ConcurrencyLimit {
		ConcurrencyLimit::new(
			limit,
			Some(Arc::new(Expression::new_strict(key).unwrap())),
			failure_mode,
		)
	}

	#[test]
	fn slots_are_counted_per_key_and_released_on_drop() {
		let limit = keyed(1, r#"request.headers["x-user"]"#, FailureMode::FailClosed);
		let alice = request("alice");
		let bob = request("bob");
		let first = acquire(&limit, &alice).unwrap();
		assert_eq!(limit.in_flight("alice"), 1);
		let err = acquire(&limit, &alice).unwrap_err();
		assert!(matches!(
			err,
			ProxyError::ConcurrencyLimitExceeded {
				limit: 1,
				in_flight: 1
			}
		));
		let other = acquire(&limit, &bob).unwrap();
		assert_eq!(limit.in_flight("bob"), 1);
		drop(first);
		assert_eq!(limit.in_flight("alice"), 0);
		acquire(&limit, &alice).unwrap();
		drop(other);
		assert_eq!(limit.in_flight("bob"), 0);
	}

	#[test]
	fn a_key_that_cannot_be_evaluated_fails_closed() {
		let limit = keyed(
			2,
			r#"request.headers["x-missing"]"#,
			FailureMode::FailClosed,
		);
		let err = acquire(&limit, &request("alice")).unwrap_err();
		assert!(matches!(err, ProxyError::ConcurrencyLimitKeyFailed));
		assert_eq!(limit.in_flight(""), 0);
	}

	#[test]
	fn a_key_that_cannot_be_evaluated_fails_open_uncounted() {
		let limit = keyed(1, r#"request.headers["x-missing"]"#, FailureMode::FailOpen);
		let req = request("alice");
		for _ in 0..3 {
			assert!(acquire(&limit, &req).unwrap().is_none());
		}
		assert_eq!(limit.in_flight(""), 0);
	}

	#[test]
	fn keys_can_read_the_requested_model() {
		let limit = keyed(1, "llm.requestModel", FailureMode::FailClosed);
		let with_model = |model: &str| {
			let mut req = request("alice");
			req.extensions_mut().insert(crate::cel::RequestModel {
				request_model: model.into(),
			});
			req
		};
		let qwen = with_model("qwen3");
		let _held = acquire(&limit, &qwen).unwrap();
		assert!(acquire(&limit, &qwen).is_err());
		assert!(acquire(&limit, &with_model("llama")).unwrap().is_some());
	}
}
