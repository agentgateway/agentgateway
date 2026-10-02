//! In-flight request limits, counted per key on this proxy instance.
//!
//! A rate limit asks how many requests started in a window. This asks how many are running right
//! now: a slot is taken when the request is admitted and given back once its response body has
//! been sent, or when the client goes away. Keys are computed with CEL, so a limit can be per
//! caller, per model, or both. Counters are local to one proxy instance.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use http_body_util::BodyExt;
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

/// Limits how many requests may be in flight at once for a key.
#[apply(schema!)]
pub struct ConcurrencyLimit {
	/// Maximum number of in-flight requests allowed per key. Requests over the limit are rejected
	/// with a 429.
	pub max_concurrent: u32,
	/// CEL expression selecting the counter, for example `jwt.sub` or
	/// `jwt.sub + "/" + llm.requestModel`. Requests without a key, or whose key cannot be
	/// evaluated, share one counter. Keys that use `llm` are evaluated once the LLM request has
	/// been parsed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub key: Option<Arc<Expression>>,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	counters: Arc<Counters>,
}

impl ConcurrencyLimit {
	pub fn new(max_concurrent: u32, key: Option<Arc<Expression>>) -> Self {
		Self {
			max_concurrent,
			key,
			counters: Default::default(),
		}
	}

	/// Whether the key needs the parsed LLM request.
	pub fn needs_llm(&self) -> bool {
		self.key.as_ref().is_some_and(|e| e.needs_llm())
	}

	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.key.iter().map(|e| e.as_ref())
	}

	fn evaluate_key(&self, exec: &Executor<'_>) -> String {
		let Some(expr) = &self.key else {
			return String::new();
		};
		let key = exec
			.eval(expr)
			.map_err(|e| format!("failed to evaluate: {e}"))
			.and_then(|v| v.as_string().map_err(|_| "is not a string".to_string()));
		key.unwrap_or_else(|why| {
			debug!(expression = %expr.original_expression, "concurrency key {why}; using the shared counter");
			String::new()
		})
	}

	/// Take a slot for this request, or reject it.
	pub fn take(&self, exec: &Executor<'_>) -> Result<Permit, ProxyError> {
		let key = self.evaluate_key(exec);
		let limit = self.max_concurrent;
		let mut counters = self.counters.0.lock();
		let in_flight = counters.get(&key).copied().unwrap_or(0);
		if in_flight >= limit {
			return Err(ProxyError::ConcurrencyLimitExceeded { limit, in_flight });
		}
		counters.insert(key.clone(), in_flight + 1);
		Ok(Permit {
			counters: self.counters.clone(),
			key,
		})
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
	llm_checked: AtomicBool,
}

impl ConcurrencyGuard {
	pub fn hold(&self, permit: Permit) {
		self.permits.lock().push(permit);
	}

	/// True the first time it is called, so limits that need the LLM request run once even when
	/// the upstream call is retried.
	pub fn begin_llm_check(&self) -> bool {
		!self.llm_checked.swap(true, Ordering::AcqRel)
	}
}

/// Keep the request's slots until the response body is complete or dropped: the closure owns the
/// guard, and the body owns the closure.
pub fn hold_until_complete(resp: &mut Response, guard: Arc<ConcurrencyGuard>) {
	let inner = std::mem::replace(resp.body_mut(), Body::empty());
	*resp.body_mut() = Body::new(inner.map_err(move |e| {
		let _held = &guard;
		e
	}));
}

impl crate::store::RequestPolicyTrait for Vec<ConcurrencyLimit> {
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
		for limit in self.iter().filter(|l| !l.needs_llm()) {
			guard.hold(
				limit
					.take(&exec)
					.map_err(crate::proxy::ProxyResponse::from)?,
			);
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

	fn acquire(limit: &ConcurrencyLimit, req: &crate::http::Request) -> Result<Permit, ProxyError> {
		limit.take(&Executor::new_request(req))
	}

	fn request(user: &str) -> crate::http::Request {
		::http::Request::builder()
			.uri("http://localhost/v1/chat/completions")
			.header("x-user", user)
			.body(Body::empty())
			.unwrap()
	}

	#[test]
	fn slots_are_counted_per_key_and_released_on_drop() {
		let limit = ConcurrencyLimit::new(
			1,
			Some(Arc::new(
				Expression::new_strict(r#"request.headers["x-user"]"#).unwrap(),
			)),
		);
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
	fn missing_key_shares_one_counter() {
		let limit = ConcurrencyLimit::new(
			2,
			Some(Arc::new(
				Expression::new_strict(r#"request.headers["x-missing"]"#).unwrap(),
			)),
		);
		let req = request("alice");
		let _a = acquire(&limit, &req).unwrap();
		let _b = acquire(&limit, &req).unwrap();
		assert_eq!(limit.in_flight(""), 2);
		assert!(acquire(&limit, &req).is_err());
	}

	#[test]
	fn llm_keys_are_deferred() {
		let model = ConcurrencyLimit::new(
			1,
			Some(Arc::new(
				Expression::new_strict("llm.requestModel").unwrap(),
			)),
		);
		assert!(model.needs_llm());
		let user = ConcurrencyLimit::new(
			1,
			Some(Arc::new(Expression::new_strict("jwt.sub").unwrap())),
		);
		assert!(!user.needs_llm());
	}

	#[test]
	fn guard_runs_the_llm_check_once() {
		let guard = ConcurrencyGuard::default();
		assert!(guard.begin_llm_check());
		assert!(!guard.begin_llm_check());
	}
}
