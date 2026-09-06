//! In-flight request limits, counted per key on this proxy instance.
//!
//! A rate limit asks how many requests started in a window. This asks how many are running right
//! now: a slot is taken when the request is admitted and given back once its response body has
//! been sent, or when the client goes away. Keys are computed with CEL, so a limit can be per
//! caller, per model, or both. Counters are local to one proxy instance unless a rule keeps them
//! in a shared store.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use http_body_util::BodyExt;
use parking_lot::Mutex;

use crate::cel::{Executor, Expression, Value};
use crate::http::{Body, Response};
use crate::proxy::ProxyError;
use crate::*;

#[path = "concurrencylimit_shared.rs"]
mod shared;

#[derive(Debug, Default)]
struct Counters(Mutex<HashMap<String, u32>>);

/// A slot taken from a counter. The slot is returned when the permit is dropped.
#[derive(Debug)]
pub struct Permit(Slot);

#[derive(Debug)]
enum Slot {
	Local {
		counters: Arc<Counters>,
		key: String,
	},
	Shared {
		store: Arc<shared::Store>,
		key: String,
		member: String,
	},
	/// The store could not be reached and the rule lets the request through.
	Unheld,
}

impl Permit {
	#[cfg(test)]
	fn is_unheld(&self) -> bool {
		matches!(self.0, Slot::Unheld)
	}
}

impl Drop for Permit {
	fn drop(&mut self) {
		match &mut self.0 {
			Slot::Local { counters, key } => {
				let mut counters = counters.0.lock();
				if let Some(in_flight) = counters.get_mut(key) {
					*in_flight = in_flight.saturating_sub(1);
					if *in_flight == 0 {
						counters.remove(key);
					}
				}
			},
			Slot::Shared { store, key, member } => {
				store.release(std::mem::take(key), std::mem::take(member))
			},
			Slot::Unheld => {},
		}
	}
}

/// Keeps a rule's slots in a store every proxy instance shares, so the limit holds across
/// instances instead of once per instance.
#[apply(schema!)]
pub struct SharedCounters {
	/// The store that keeps the counters.
	pub redis: RedisStore,
	/// How long a slot stays counted without renewal. Slots are renewed while their request runs
	/// and dropped when it ends, so this only bounds how long a slot taken by an instance that went
	/// away is counted. Defaults to 60s.
	#[serde(default = "default_lease", with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub lease: Duration,
	/// How long a store call may take before `failureMode` applies. Defaults to 1s.
	#[serde(default = "default_timeout", with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub timeout: Duration,
	/// Prefix of the store keys. Rules with the same settings and prefix count together, so give
	/// gateways that must not share their slots different prefixes. Defaults to
	/// `agentgateway:concurrency`.
	#[serde(default = "default_key_prefix")]
	pub key_prefix: String,
	/// What happens to a request when the store cannot be reached. Defaults to `allow`.
	#[serde(default)]
	pub failure_mode: FailureMode,
}

/// A Redis store.
#[apply(schema!)]
pub struct RedisStore {
	/// Connection URL, such as `redis://redis:6379/0`, or `rediss://` for TLS.
	pub url: String,
}

#[apply(schema_enum!)]
#[cfg_attr(feature = "schema", schemars(rename = "ConcurrencyFailureMode"))]
#[derive(Default)]
pub enum FailureMode {
	/// The request goes through without taking a slot.
	#[default]
	Allow,
	/// The request is rejected with a 503.
	Deny,
}

pub fn default_lease() -> Duration {
	Duration::from_secs(60)
}

pub fn default_timeout() -> Duration {
	Duration::from_secs(1)
}

pub fn default_key_prefix() -> String {
	"agentgateway:concurrency".to_string()
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
	/// CEL expression computing the limit for this request instead of `maxConcurrent`. It must
	/// evaluate to a non-negative integer; when it does not, `maxConcurrent` applies.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub limit_override: Option<Arc<Expression>>,
	/// Keep the slots in a store every proxy instance shares, instead of on this instance.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub shared: Option<SharedCounters>,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	counters: Arc<Counters>,
	/// The rule's part of a shared store key, from its settings, so rules with the same settings
	/// count together and rules that differ keep apart.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	rule_id: OnceLock<String>,
}

impl ConcurrencyLimit {
	pub fn new(
		max_concurrent: u32,
		key: Option<Arc<Expression>>,
		limit_override: Option<Arc<Expression>>,
	) -> Self {
		Self {
			max_concurrent,
			key,
			limit_override,
			shared: None,
			counters: Default::default(),
			rule_id: Default::default(),
		}
	}

	pub fn with_shared(mut self, shared: Option<SharedCounters>) -> Self {
		self.shared = shared;
		self
	}

	/// Whether the key or limit needs the parsed LLM request.
	pub fn needs_llm(&self) -> bool {
		self.key.as_ref().is_some_and(|e| e.needs_llm())
			|| self.limit_override.as_ref().is_some_and(|e| e.needs_llm())
	}

	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self
			.key
			.iter()
			.chain(self.limit_override.iter())
			.map(|e| e.as_ref())
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

	fn evaluate_limit(&self, exec: &Executor<'_>) -> u32 {
		let Some(expr) = &self.limit_override else {
			return self.max_concurrent;
		};
		let limit = exec
			.eval(expr)
			.map_err(|e| format!("failed to evaluate: {e}"))
			.and_then(|v| match v {
				Value::Int(v) if v >= 0 => Ok(u32::try_from(v).unwrap_or(u32::MAX)),
				Value::UInt(v) => Ok(u32::try_from(v).unwrap_or(u32::MAX)),
				_ => Err("is not a non-negative integer".to_string()),
			});
		limit.unwrap_or_else(|why| {
			debug!(expression = %expr.original_expression, "concurrency limit override {why}");
			self.max_concurrent
		})
	}

	/// The counter and the limit for this request. `take` runs with the result, so no CEL state is
	/// held across the store call.
	pub fn evaluate(&self, exec: &Executor<'_>) -> (String, u32) {
		(self.evaluate_key(exec), self.evaluate_limit(exec))
	}

	/// Take a slot for this request, or reject it.
	pub async fn take(&self, key: String, limit: u32) -> Result<Permit, ProxyError> {
		match &self.shared {
			None => self.take_local(key, limit),
			Some(shared) => self.take_shared(shared, key, limit).await,
		}
	}

	fn take_local(&self, key: String, limit: u32) -> Result<Permit, ProxyError> {
		let mut counters = self.counters.0.lock();
		let in_flight = counters.get(&key).copied().unwrap_or(0);
		if in_flight >= limit {
			return Err(ProxyError::ConcurrencyLimitExceeded { limit, in_flight });
		}
		counters.insert(key.clone(), in_flight + 1);
		Ok(Permit(Slot::Local {
			counters: self.counters.clone(),
			key,
		}))
	}

	async fn take_shared(
		&self,
		shared: &SharedCounters,
		key: String,
		limit: u32,
	) -> Result<Permit, ProxyError> {
		if limit == 0 {
			return Err(ProxyError::ConcurrencyLimitExceeded {
				limit,
				in_flight: 0,
			});
		}
		let key = format!("{}:{}:{key}", shared.key_prefix, self.rule_id());
		let taken = async {
			let store = shared::Store::get(&shared.redis.url, shared.lease, shared.timeout)?;
			let taken = store.acquire(&key, limit).await?;
			Ok::<_, redis::RedisError>((store, taken))
		}
		.await;
		match taken {
			Ok((store, shared::Taken::Slot(member))) => Ok(Permit(Slot::Shared { store, key, member })),
			Ok((_, shared::Taken::Full { in_flight })) => {
				Err(ProxyError::ConcurrencyLimitExceeded { limit, in_flight })
			},
			Err(e) => match shared.failure_mode {
				FailureMode::Allow => {
					warn!(error = %e, "concurrency store unreachable; the request goes through");
					Ok(Permit(Slot::Unheld))
				},
				FailureMode::Deny => Err(ProxyError::Processing(anyhow::anyhow!(
					"concurrency store unreachable: {e}"
				))),
			},
		}
	}

	fn rule_id(&self) -> &str {
		self.rule_id.get_or_init(|| {
			let mut hasher = DefaultHasher::new();
			self.max_concurrent.hash(&mut hasher);
			for expr in self.expressions() {
				expr.original_expression.hash(&mut hasher);
			}
			format!("{:016x}", hasher.finish())
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
		let wanted: Vec<_> = self
			.iter()
			.filter(|l| !l.needs_llm())
			.map(|l| (l, l.evaluate(&exec)))
			.collect();
		drop(exec);
		for (limit, (key, max)) in wanted {
			guard.hold(
				limit
					.take(key, max)
					.await
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

	async fn acquire(
		limit: &ConcurrencyLimit,
		req: &crate::http::Request,
	) -> Result<Permit, ProxyError> {
		let (key, max) = limit.evaluate(&Executor::new_request(req));
		limit.take(key, max).await
	}

	/// Set to run the shared-store tests against a Redis, for example `redis://127.0.0.1:6379`.
	fn redis_url() -> Option<String> {
		std::env::var("AGENTGATEWAY_TEST_REDIS_URL").ok()
	}

	fn shared(url: &str, failure_mode: FailureMode) -> ConcurrencyLimit {
		let unique = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_nanos();
		ConcurrencyLimit::new(1, None, None).with_shared(Some(SharedCounters {
			redis: RedisStore {
				url: url.to_string(),
			},
			lease: Duration::from_millis(300),
			timeout: Duration::from_millis(500),
			key_prefix: format!("test:{unique}"),
			failure_mode,
		}))
	}

	/// The store releases slots in the background, so admission can lag a drop by a moment.
	async fn eventually_taken(limit: &ConcurrencyLimit, key: &str) -> bool {
		for _ in 0..40 {
			if limit.take(key.to_string(), 1).await.is_ok() {
				return true;
			}
			tokio::time::sleep(Duration::from_millis(25)).await;
		}
		false
	}

	#[tokio::test]
	async fn an_unreachable_store_follows_the_failure_mode() {
		let open = shared("redis://127.0.0.1:1", FailureMode::Allow);
		assert!(
			open
				.take(String::new(), 1)
				.await
				.is_ok_and(|p| p.is_unheld())
		);
		let closed = shared("redis://127.0.0.1:1", FailureMode::Deny);
		assert!(matches!(
			closed.take(String::new(), 1).await,
			Err(ProxyError::Processing(_))
		));
		// A zero limit rejects before the store is asked.
		assert!(matches!(
			closed.take(String::new(), 0).await,
			Err(ProxyError::ConcurrencyLimitExceeded { limit: 0, .. })
		));
	}

	#[tokio::test]
	async fn shared_slots_are_counted_across_instances() {
		let Some(url) = redis_url() else { return };
		// Two rules with the same settings stand in for two proxy instances.
		let a = shared(&url, FailureMode::Deny);
		let b = a.clone();
		let first = a.take("alice".into(), 1).await.unwrap();
		assert!(matches!(
			b.take("alice".into(), 1).await,
			Err(ProxyError::ConcurrencyLimitExceeded {
				limit: 1,
				in_flight: 1
			})
		));
		let _bob = b.take("bob".into(), 1).await.unwrap();
		drop(first);
		assert!(eventually_taken(&b, "alice").await);
	}

	#[tokio::test]
	async fn a_lost_instance_holds_its_slot_only_for_the_lease() {
		let Some(url) = redis_url() else { return };
		let limit = shared(&url, FailureMode::Deny);
		let key = format!(
			"{}:{}:alice",
			limit.shared.as_ref().unwrap().key_prefix,
			limit.rule_id()
		);
		{
			// A store of its own, dropped with its slot unreleased, as a crashed instance leaves it.
			let lost =
				shared::Store::get(&url, Duration::from_millis(300), Duration::from_millis(501)).unwrap();
			assert!(matches!(
				lost.acquire(&key, 1).await.unwrap(),
				shared::Taken::Slot(_)
			));
		}
		assert!(limit.take("alice".into(), 1).await.is_err());
		tokio::time::sleep(Duration::from_millis(450)).await;
		assert!(limit.take("alice".into(), 1).await.is_ok());
	}

	#[tokio::test]
	async fn held_shared_slots_are_renewed_past_the_lease() {
		let Some(url) = redis_url() else { return };
		let limit = shared(&url, FailureMode::Deny);
		let _held = limit.take("alice".into(), 1).await.unwrap();
		tokio::time::sleep(Duration::from_millis(700)).await;
		assert!(limit.take("alice".into(), 1).await.is_err());
	}

	fn request(user: &str) -> crate::http::Request {
		::http::Request::builder()
			.uri("http://localhost/v1/chat/completions")
			.header("x-user", user)
			.body(Body::empty())
			.unwrap()
	}

	#[tokio::test]
	async fn slots_are_counted_per_key_and_released_on_drop() {
		let limit = ConcurrencyLimit::new(
			1,
			Some(Arc::new(
				Expression::new_strict(r#"request.headers["x-user"]"#).unwrap(),
			)),
			None,
		);
		let alice = request("alice");
		let bob = request("bob");
		let first = acquire(&limit, &alice).await.unwrap();
		assert_eq!(limit.in_flight("alice"), 1);
		let err = acquire(&limit, &alice).await.unwrap_err();
		assert!(matches!(
			err,
			ProxyError::ConcurrencyLimitExceeded {
				limit: 1,
				in_flight: 1
			}
		));
		let other = acquire(&limit, &bob).await.unwrap();
		assert_eq!(limit.in_flight("bob"), 1);
		drop(first);
		assert_eq!(limit.in_flight("alice"), 0);
		acquire(&limit, &alice).await.unwrap();
		drop(other);
		assert_eq!(limit.in_flight("bob"), 0);
	}

	#[tokio::test]
	async fn missing_key_shares_one_counter() {
		let limit = ConcurrencyLimit::new(
			2,
			Some(Arc::new(
				Expression::new_strict(r#"request.headers["x-missing"]"#).unwrap(),
			)),
			None,
		);
		let req = request("alice");
		let _a = acquire(&limit, &req).await.unwrap();
		let _b = acquire(&limit, &req).await.unwrap();
		assert_eq!(limit.in_flight(""), 2);
		assert!(acquire(&limit, &req).await.is_err());
	}

	#[tokio::test]
	async fn limit_override_wins_when_it_evaluates() {
		let limit = ConcurrencyLimit::new(
			1,
			None,
			Some(Arc::new(
				Expression::new_strict(r#"request.headers["x-user"] == "alice" ? 3 : 0"#).unwrap(),
			)),
		);
		let alice = request("alice");
		let _a = acquire(&limit, &alice).await.unwrap();
		let _b = acquire(&limit, &alice).await.unwrap();
		let _c = acquire(&limit, &alice).await.unwrap();
		assert!(acquire(&limit, &alice).await.is_err());
		// A zero limit makes the route off-limits for everyone else.
		let bob = request("bob");
		assert!(matches!(
			acquire(&limit, &bob).await,
			Err(ProxyError::ConcurrencyLimitExceeded { limit: 0, .. })
		));
		// An override that fails to evaluate falls back to maxConcurrent.
		let fallback = ConcurrencyLimit::new(
			1,
			None,
			Some(Arc::new(
				Expression::new_strict(r#"request.headers["x-missing"] + 1"#).unwrap(),
			)),
		);
		let _d = acquire(&fallback, &bob).await.unwrap();
		assert!(acquire(&fallback, &bob).await.is_err());
	}

	#[test]
	fn llm_keys_are_deferred() {
		let model = ConcurrencyLimit::new(
			1,
			Some(Arc::new(
				Expression::new_strict("llm.requestModel").unwrap(),
			)),
			None,
		);
		assert!(model.needs_llm());
		let user = ConcurrencyLimit::new(
			1,
			Some(Arc::new(Expression::new_strict("jwt.sub").unwrap())),
			None,
		);
		assert!(!user.needs_llm());
		let tiered = ConcurrencyLimit::new(
			1,
			None,
			Some(Arc::new(
				Expression::new_strict(r#"llm.requestModel == "big" ? 1 : 4"#).unwrap(),
			)),
		);
		assert!(tiered.needs_llm());
	}

	#[test]
	fn guard_runs_the_llm_check_once() {
		let guard = ConcurrencyGuard::default();
		assert!(guard.begin_llm_check());
		assert!(!guard.begin_llm_check());
	}
}
