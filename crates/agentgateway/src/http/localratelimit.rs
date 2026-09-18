use quick_cache::sync::Cache;
use serde::de::Error;

use crate::cel::{Executor, Expression};
use crate::proxy::ProxyError;
use crate::*;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "RateLimitSpec"))]
#[derive(serde::Serialize)]
pub struct RateLimit {
	/// The bucket used when no key is configured, and for requests whose key is empty or cannot
	/// be evaluated.
	#[serde(skip_serializing)]
	ratelimit: Arc<ratelimit::Ratelimiter>,
	/// One bucket per key value, created on first use, bounded by `MAX_BUCKETS`.
	#[serde(skip_serializing)]
	keyed: Arc<Cache<String, Arc<ratelimit::Ratelimiter>>>,
	#[serde(flatten)]
	pub spec: RateLimitSpec,
}

impl<'de> serde::Deserialize<'de> for RateLimit {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let ratelimit = RateLimitSpec::deserialize(deserializer)?;
		RateLimit::try_from(ratelimit).map_err(D::Error::custom)
	}
}

#[apply(schema!)]
pub struct RateLimitSpec {
	/// Maximum number of tokens that can accumulate in the local bucket.
	#[serde(default)]
	pub max_tokens: u64,
	/// Number of tokens added to the local bucket each fill interval.
	#[serde(default)]
	pub tokens_per_fill: u64,
	/// How often the local bucket is refilled.
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub fill_interval: Duration,
	/// Whether this limit counts requests or LLM tokens.
	#[serde(default)]
	#[serde(rename = "type")]
	pub limit_type: RateLimitType,
	/// CEL expression selecting the bucket, for example `jwt.sub` for a per-user limit or
	/// `jwt.team` for a per-team limit. Each distinct value gets its own bucket with the limits
	/// above. Requests without a key, or whose key cannot be evaluated, share one bucket. The key
	/// is evaluated where the rule is checked, so a token limit can also read the parsed LLM
	/// request. Buckets are local to one proxy instance, which keeps a bounded number of them per
	/// rule and drops the least used ones.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub key: Option<Arc<Expression>>,
}

#[apply(schema!)]
#[derive(Default, Eq, PartialEq)]
pub enum RateLimitType {
	/// Count each request as one unit.
	#[serde(rename = "requests")]
	#[default]
	Requests,
	/// Count LLM token usage.
	#[serde(rename = "tokens")]
	Tokens,
}

fn build_bucket(spec: &RateLimitSpec) -> Result<ratelimit::Ratelimiter, ratelimit::Error> {
	ratelimit::Ratelimiter::builder(spec.tokens_per_fill, spec.fill_interval)
		.initial_available(spec.max_tokens)
		.max_tokens(spec.max_tokens)
		.build()
}

impl TryFrom<RateLimitSpec> for RateLimit {
	type Error = ratelimit::Error;
	fn try_from(value: RateLimitSpec) -> Result<Self, Self::Error> {
		let rl = build_bucket(&value)?;
		Ok(RateLimit {
			ratelimit: Arc::new(rl),
			keyed: Arc::new(Cache::new(MAX_BUCKETS)),
			spec: value,
		})
	}
}

/// How many keyed buckets one rule keeps. A key the caller picks, such as a request header, would
/// otherwise let anyone grow the map without bound; past this the least used buckets are dropped,
/// which is the same as never having seen those keys.
const MAX_BUCKETS: usize = 65_536;

/// A bucket a request was charged against, kept so the real token usage can be settled once the
/// response is known, or the charge given back if a later rule rejects the request.
#[derive(Debug, Clone)]
pub struct ChargedBucket {
	bucket: Arc<ratelimit::Ratelimiter>,
	/// What was taken when the request was admitted.
	charged: u64,
}

impl ChargedBucket {
	/// Remove tokens from the bucket after the fact. This is useful for true-up scenarios where
	/// the actual cost is discovered after making a request. The bucket never goes negative.
	pub fn amend_tokens(&self, tokens_to_remove: i64) {
		self.bucket.amend_tokens(tokens_to_remove);
	}

	/// Give back what the admission took, because the request was rejected by another rule.
	pub fn refund(&self) {
		self
			.bucket
			.amend_tokens(-i64::try_from(self.charged).unwrap_or(i64::MAX));
	}

	#[cfg(test)]
	pub fn available(&self) -> u64 {
		self.bucket.available()
	}
}

/// Snapshot of a local rate limit bucket, used to emit `x-ratelimit-*` headers on allowed responses.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitStatus {
	pub limit: u64,
	pub remaining: u64,
	pub reset_seconds: u64,
}

impl RateLimitStatus {
	/// Returns the tighter of two limits (fewest tokens remaining), used to report the limit a
	/// client is most likely to hit next when several rate limits apply.
	pub fn most_constrained(a: Option<Self>, b: Option<Self>) -> Option<Self> {
		match (a, b) {
			(Some(a), Some(b)) => Some(if b.remaining < a.remaining { b } else { a }),
			(a, b) => a.or(b),
		}
	}

	pub(crate) fn to_headers(self) -> http::HeaderMap {
		let mut hm = http::HeaderMap::new();
		http::x_headers::set_ratelimit_headers(&mut hm, self.limit, self.remaining, self.reset_seconds);
		hm
	}
}

fn status(bucket: &ratelimit::Ratelimiter) -> RateLimitStatus {
	let now = clocksource::precise::Instant::now();
	let next = bucket.next_refill();
	let reset_seconds = if next > now {
		(next - now).as_secs()
	} else {
		0
	};
	RateLimitStatus {
		limit: bucket.max_tokens(),
		remaining: bucket.available(),
		reset_seconds,
	}
}

impl From<RateLimitStatus> for ProxyError {
	fn from(status: RateLimitStatus) -> Self {
		ProxyError::RateLimitExceeded {
			limit: status.limit,
			remaining: status.remaining,
			reset_seconds: status.reset_seconds,
		}
	}
}

impl RateLimit {
	/// The bucket this request counts against: the one for its key, or the shared one when there
	/// is no key, or it is empty, or it does not evaluate to a string.
	fn bucket(&self, exec: &Executor<'_>) -> Arc<ratelimit::Ratelimiter> {
		let Some(expr) = &self.spec.key else {
			return self.ratelimit.clone();
		};
		let value = exec.eval(expr);
		let key = value.as_ref().ok().and_then(|v| v.as_str().ok());
		let Some(key) = key.as_deref().filter(|k| !k.is_empty()) else {
			debug!(expression = %expr.original_expression, "no rate limit key for this request; using the shared bucket");
			return self.ratelimit.clone();
		};
		self
			.keyed
			.get_or_insert_with(key, || build_bucket(&self.spec).map(Arc::new))
			// The spec already built the shared bucket, so this cannot happen; stay safe anyway.
			.unwrap_or_else(|_: ratelimit::Error| self.ratelimit.clone())
	}

	/// The bucket used when no key applies.
	#[cfg(test)]
	pub fn shared_bucket(&self) -> ChargedBucket {
		ChargedBucket {
			bucket: self.ratelimit.clone(),
			charged: 0,
		}
	}

	/// Take `n` tokens from the bucket, or report the bucket that rejected them.
	fn take(bucket: &ratelimit::Ratelimiter, n: u64) -> Result<RateLimitStatus, ProxyError> {
		bucket
			.try_wait_n(n)
			.map(|()| status(bucket))
			.map_err(|(limit, remaining, reset)| {
				RateLimitStatus {
					limit,
					remaining,
					reset_seconds: reset.as_secs(),
				}
				.into()
			})
	}

	/// Take one request from the bucket. Returns `None` for token limits. On success the charged
	/// bucket is returned, so the caller can give the request back if a later rule rejects it.
	pub fn check_request(
		&self,
		exec: &Executor<'_>,
	) -> Result<Option<(RateLimitStatus, ChargedBucket)>, ProxyError> {
		if self.spec.limit_type != RateLimitType::Requests {
			return Ok(None);
		}
		let bucket = self.bucket(exec);
		let status = Self::take(&bucket, 1)?;
		Ok(Some((status, ChargedBucket { bucket, charged: 1 })))
	}

	/// Charge the request's input tokens to the bucket. Returns `None` for request limits. On
	/// success the charged bucket is returned, so the response side can settle the real usage
	/// with `ChargedBucket::amend_tokens`.
	pub fn charge_tokens(
		&self,
		input_tokens: Option<u64>,
		exec: &Executor<'_>,
	) -> Result<Option<(RateLimitStatus, ChargedBucket)>, ProxyError> {
		if self.spec.limit_type != RateLimitType::Tokens {
			return Ok(None);
		}
		let bucket = self.bucket(exec);
		let status = if let Some(it) = input_tokens {
			// If we tokenized the request, check to make sure we permit that many tokens
			// We will add the response tokens in `amend_tokens`
			Self::take(&bucket, it)?
		} else {
			// Otherwise, make sure at least 1 token is allowed.
			// Note this may lead to large over-allowance, especially with fast fill_intervals.
			if bucket.available_refill() == 0 {
				return Err(status(&bucket).into());
			}
			status(&bucket)
		};
		Ok(Some((
			status,
			ChargedBucket {
				bucket,
				charged: input_tokens.unwrap_or(0),
			},
		)))
	}
}

impl crate::store::RequestPolicyTrait for Vec<RateLimit> {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut http::Request,
	) -> Result<http::PolicyResponse, crate::proxy::ProxyResponse> {
		let exec = Executor::new_request(req);
		let mut status: Option<RateLimitStatus> = None;
		let mut taken = Vec::new();
		for rate_limit in self {
			match rate_limit.check_request(&exec) {
				Ok(Some((s, bucket))) => {
					status = RateLimitStatus::most_constrained(status, Some(s));
					taken.push(bucket);
				},
				Ok(None) => {},
				Err(e) => {
					// The request is rejected, so it must not count against the rules that admitted it.
					for bucket in taken {
						bucket.refund();
					}
					return Err(e.into());
				},
			}
		}
		let mut res = http::PolicyResponse::default();
		if let Some(status) = status {
			res.response_headers = Some(status.to_headers());
		}
		Ok(res)
	}

	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.iter().filter_map(|r| r.spec.key.as_deref())
	}
}

#[cfg(test)]
#[path = "localratelimit_tests.rs"]
mod proxy_tests;

#[cfg(test)]
mod policy_tests {
	use super::*;

	fn requests_limit(max: u64) -> RateLimit {
		RateLimit::try_from(RateLimitSpec {
			max_tokens: max,
			tokens_per_fill: max,
			fill_interval: std::time::Duration::from_secs(60),
			limit_type: RateLimitType::Requests,
			key: None,
		})
		.unwrap()
	}

	fn keyed_requests_limit(max: u64, key: &str) -> RateLimit {
		RateLimit::try_from(RateLimitSpec {
			max_tokens: max,
			tokens_per_fill: max,
			fill_interval: std::time::Duration::from_secs(60),
			limit_type: RateLimitType::Requests,
			key: Some(Arc::new(Expression::new_strict(key).unwrap())),
		})
		.unwrap()
	}

	fn request(user: &str) -> http::Request {
		request_with(&[("x-user", user)])
	}

	fn request_with(headers: &[(&str, &str)]) -> http::Request {
		let mut req = ::http::Request::builder().uri("http://localhost/v1/chat/completions");
		for (name, value) in headers {
			req = req.header(*name, *value);
		}
		req.body(http::Body::empty()).unwrap()
	}

	#[test]
	fn check_request_returns_status_on_success() {
		let rl = requests_limit(10);
		let req = request("alice");
		let (status, _) = rl
			.check_request(&Executor::new_request(&req))
			.expect("request is allowed")
			.expect("status is reported on success");
		assert_eq!(status.limit, 10);
		// One token was consumed by this request.
		assert_eq!(status.remaining, 9);
	}

	#[test]
	fn check_request_is_noop_for_token_limit() {
		let mut rl = requests_limit(10);
		rl.spec.limit_type = RateLimitType::Tokens;
		let req = request("alice");
		assert!(
			rl.check_request(&Executor::new_request(&req))
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn keyed_buckets_are_independent() {
		let rl = keyed_requests_limit(1, r#"request.headers["x-user"]"#);
		let alice = request("alice");
		let bob = request("bob");
		assert!(
			rl.check_request(&Executor::new_request(&alice))
				.unwrap()
				.is_some()
		);
		assert!(matches!(
			rl.check_request(&Executor::new_request(&alice)),
			Err(ProxyError::RateLimitExceeded { limit: 1, .. })
		));
		// Another key has its own bucket.
		assert!(
			rl.check_request(&Executor::new_request(&bob))
				.unwrap()
				.is_some()
		);
		assert_eq!(rl.keyed.len(), 2);
	}

	#[test]
	fn missing_key_uses_the_shared_bucket() {
		let rl = keyed_requests_limit(1, r#"request.headers["x-missing"]"#);
		let req = request("alice");
		assert!(
			rl.check_request(&Executor::new_request(&req))
				.unwrap()
				.is_some()
		);
		assert!(rl.check_request(&Executor::new_request(&req)).is_err());
		assert_eq!(rl.keyed.len(), 0);
		assert_eq!(rl.shared_bucket().available(), 0);
	}

	#[test]
	fn a_key_without_its_context_uses_the_shared_bucket() {
		// A request limit is checked before the LLM request is parsed, so a key that reads it
		// cannot be evaluated and the rule holds every request to one bucket.
		let rl = keyed_requests_limit(1, "llm.requestModel");
		let req = request("alice");
		let exec = Executor::new_request(&req);
		assert!(rl.check_request(&exec).unwrap().is_some());
		assert!(rl.check_request(&exec).is_err());
		assert_eq!(rl.keyed.len(), 0);
	}

	#[test]
	fn token_limits_are_charged_per_key() {
		let rl = RateLimit::try_from(RateLimitSpec {
			max_tokens: 10,
			tokens_per_fill: 10,
			fill_interval: std::time::Duration::from_secs(60),
			limit_type: RateLimitType::Tokens,
			key: Some(Arc::new(
				Expression::new_strict(r#"request.headers["x-user"]"#).unwrap(),
			)),
		})
		.unwrap();
		let alice = request("alice");
		let bob = request("bob");
		let (_, charged) = rl
			.charge_tokens(Some(4), &Executor::new_request(&alice))
			.unwrap()
			.unwrap();
		// The response cost 5 more tokens than the request estimate.
		charged.amend_tokens(5);
		assert_eq!(charged.available(), 1);
		assert!(
			rl.charge_tokens(Some(4), &Executor::new_request(&alice))
				.is_err()
		);
		// Bob's bucket is untouched.
		let (status, _) = rl
			.charge_tokens(Some(4), &Executor::new_request(&bob))
			.unwrap()
			.unwrap();
		assert_eq!(status.remaining, 6);
	}

	#[test]
	fn the_number_of_keyed_buckets_is_bounded() {
		let rl = keyed_requests_limit(2, r#"request.headers["x-user"]"#);
		for i in 0..MAX_BUCKETS + 1024 {
			let _ = rl.keyed.get_or_insert_with(&format!("user-{i}"), || {
				build_bucket(&rl.spec).map(Arc::new)
			});
		}
		assert!(rl.keyed.len() <= MAX_BUCKETS);
	}

	#[test]
	fn most_constrained_prefers_fewest_remaining() {
		let a = RateLimitStatus {
			limit: 100,
			remaining: 9,
			reset_seconds: 10,
		};
		let b = RateLimitStatus {
			limit: 5,
			remaining: 2,
			reset_seconds: 30,
		};
		let best = RateLimitStatus::most_constrained(Some(a), Some(b)).unwrap();
		assert_eq!(best.remaining, 2);
		assert_eq!(best.limit, 5);
		assert_eq!(
			RateLimitStatus::most_constrained(None, Some(a))
				.unwrap()
				.remaining,
			9
		);
		assert!(RateLimitStatus::most_constrained(None, None).is_none());
	}
}

// Forked from https://github.com/pelikan-io/rustcommon/tree/main/ratelimit to provide some additional functions
mod ratelimit {
	use core::sync::atomic::{AtomicU64, Ordering};

	use clocksource::precise::{AtomicInstant, Duration, Instant};
	use thiserror::Error;

	#[derive(Error, Debug, PartialEq, Eq)]
	pub enum Error {
		#[error("available tokens cannot be set higher than max tokens")]
		AvailableTokensTooHigh,
		#[error("max tokens cannot be less than the refill amount")]
		MaxTokensTooLow,
		#[error("refill amount cannot exceed the max tokens")]
		RefillAmountTooHigh,
		#[error("refill interval in nanoseconds exceeds maximum u64")]
		RefillIntervalTooLong,
	}

	#[derive(Debug, Clone, Copy, Eq, PartialEq)]
	struct Parameters {
		capacity: u64,
		refill_amount: u64,
		refill_interval: Duration,
	}

	#[derive(Debug)]
	pub struct Ratelimiter {
		available: AtomicU64,
		dropped: AtomicU64,
		parameters: Parameters,
		refill_at: AtomicInstant,
	}

	impl Ratelimiter {
		/// Initialize a builder that will construct a `Ratelimiter` that adds the
		/// specified `amount` of tokens to the token bucket after each `interval`
		/// has elapsed.
		///
		/// Note: In practice, the system clock resolution imposes a lower bound on
		/// the `interval`. To be safe, it is recommended to set the interval to be
		/// no less than 1 microsecond. This also means that the number of tokens
		/// per interval should be > 1 to achieve rates beyond 1 million tokens/s.
		pub fn builder(amount: u64, interval: core::time::Duration) -> Builder {
			Builder::new(amount, interval)
		}

		/// Returns the maximum number of tokens that can
		pub fn max_tokens(&self) -> u64 {
			self.parameters.capacity
		}

		/// Returns the number of tokens currently available.
		#[allow(dead_code)]
		pub fn available(&self) -> u64 {
			self.available.load(Ordering::Relaxed)
		}

		/// Returns the number of tokens currently available. This will refill if needed;
		pub fn available_refill(&self) -> u64 {
			let _ = self.refill(Instant::now());
			self.available.load(Ordering::Relaxed)
		}

		/// Returns the time of the next refill.
		pub fn next_refill(&self) -> Instant {
			self.refill_at.load(Ordering::Relaxed)
		}

		/// Returns the number of tokens that have been dropped due to bucket
		/// overflowing.
		#[allow(dead_code)]
		pub fn dropped(&self) -> u64 {
			self.dropped.load(Ordering::Relaxed)
		}

		/// Remove tokens from the bucket after the fact. This is useful for true-up
		/// scenarios where you discover the actual cost after making a request.
		/// This function cannot fail and will not allow the bucket to go negative.
		/// If there are fewer tokens available than requested to remove, the bucket
		/// will be set to 0.
		pub fn amend_tokens(&self, tokens_to_remove: i64) {
			if tokens_to_remove == 0 {
				return;
			}

			let capacity = self.parameters.capacity;
			let _ = self
				.available
				.try_update(Ordering::AcqRel, Ordering::Acquire, |v| {
					if tokens_to_remove < 0 {
						// Never exceed the capacity: `refill` assumes `available <= capacity`.
						Some(
							v.saturating_add(tokens_to_remove.unsigned_abs())
								.min(capacity),
						)
					} else {
						Some(v.saturating_sub(tokens_to_remove.unsigned_abs()))
					}
				});
		}

		/// Internal function to refill the token bucket. Called as part of
		/// `try_wait()`
		fn refill(&self, time: Instant) -> Result<(), core::time::Duration> {
			// will hold the number of elapsed refill intervals
			let mut intervals;
			// will hold a read lock for the refill parameters
			let mut parameters;

			loop {
				// determine when next refill should occur
				let refill_at = self.next_refill();

				// if this time is before the next refill is due, return
				if time < refill_at {
					return Err(core::time::Duration::from_nanos(
						(refill_at - time).as_nanos(),
					));
				}

				// acquire read lock for refill parameters
				parameters = self.parameters;

				intervals = (time - refill_at).as_nanos() / parameters.refill_interval.as_nanos() + 1;

				// calculate when the following refill would be
				let next_refill =
					refill_at + Duration::from_nanos(intervals * parameters.refill_interval.as_nanos());

				// compare/exchange, if race, loop and check if we still need to
				// refill before trying again
				if self
					.refill_at
					.compare_exchange(refill_at, next_refill, Ordering::AcqRel, Ordering::Acquire)
					.is_ok()
				{
					break;
				}
			}

			// figure out how many tokens we might add
			let amount = intervals * parameters.refill_amount;

			let available = self.available.load(Ordering::Acquire);

			if available + amount >= parameters.capacity {
				// we will fill the bucket up to the capacity
				let to_add = parameters.capacity - available;
				self.available.fetch_add(to_add, Ordering::Release);

				// and increment the number of tokens dropped
				self.dropped.fetch_add(amount - to_add, Ordering::Relaxed);
			} else {
				self.available.fetch_add(amount, Ordering::Release);
			}

			Ok(())
		}

		/// Non-blocking function to "wait" for a single token. On success, a single
		/// token has been acquired. On failure, a `Duration` hinting at when the
		/// next refill would occur is returned.
		#[cfg(test)]
		pub fn try_wait(&self) -> Result<(), (u64, u64, core::time::Duration)> {
			self.try_wait_n(1)
		}

		/// Non-blocking function to "wait" for multiple tokens. On success, all requested
		/// tokens have been acquired. On failure, a `Duration` hinting at when the
		/// next refill would occur is returned. Either all tokens are acquired or none.
		pub fn try_wait_n(&self, n: u64) -> Result<(), (u64, u64, core::time::Duration)> {
			if n == 0 || n > self.parameters.capacity {
				return Err((
					self.parameters.capacity,
					self.available.load(Ordering::Acquire),
					core::time::Duration::from_nanos(0),
				));
			}

			// We have an outer loop that drives the refilling of the token bucket.
			// This will only be repeated if we refill successfully, but somebody
			// else takes the newly available token(s) before we can attempt to
			// acquire them.
			loop {
				// Attempt to refill the bucket. This makes sure we are moving the
				// time forward, issuing new tokens, hitting our max capacity, etc.
				let refill_result = self.refill(Instant::now());

				// Note: right now it doesn't matter if refill succeeded or failed.
				// We might already have tokens available. Even if refill failed we
				// check if there are tokens and attempt to acquire them.

				// Our inner loop deals with acquiring tokens. It will only repeat
				// if there is a race on the available tokens.
				loop {
					// load the count of available tokens
					let available = self.available.load(Ordering::Acquire);

					// Check if we have enough tokens available
					if available < n {
						match refill_result {
							Ok(_) => {
								// This means we raced. Refill succeeded but another
								// caller has taken some tokens. We break the inner
								// loop and try to refill again.
								break;
							},
							Err(e) => {
								// Refill failed and there weren't enough tokens already
								// available. We return the error which contains a
								// duration until the next refill.
								return Err((self.parameters.capacity, available, e));
							},
						}
					}

					// If we made it here, available is >= n and so we can attempt to
					// acquire n tokens by doing a compare exchange on available.
					let new = available - n;

					if self
						.available
						.compare_exchange(available, new, Ordering::AcqRel, Ordering::Acquire)
						.is_ok()
					{
						// We have acquired all n tokens and can return successfully
						return Ok(());
					}

					// If we raced on the compare exchange, we need to repeat the
					// token acquisition. Either there will be enough tokens we can
					// try to acquire, or we will break and attempt a refill again.
				}
			}
		}
	}

	pub struct Builder {
		initial_available: u64,
		max_tokens: u64,
		refill_amount: u64,
		refill_interval: core::time::Duration,
	}

	impl Builder {
		/// Initialize a new builder that will add `amount` tokens after each
		/// `interval` has elapsed.
		fn new(amount: u64, interval: core::time::Duration) -> Self {
			Self {
				// default of zero tokens initially
				initial_available: 0,
				// default of one to prohibit bursts
				max_tokens: 1,
				refill_amount: amount,
				refill_interval: interval,
			}
		}

		/// Set the max tokens that can be held in the the `Ratelimiter` at any
		/// time. This limits the size of any bursts by placing an upper bound on
		/// the number of tokens available for immediate use.
		///
		/// By default, the max_tokens will be set to one unless the refill amount
		/// requires a higher value.
		///
		/// The selected value cannot be lower than the refill amount.
		pub fn max_tokens(mut self, tokens: u64) -> Self {
			self.max_tokens = tokens;
			self
		}

		/// Set the number of tokens that are initially available. For admission
		/// control scenarios, you may wish for there to be some tokens initially
		/// available to avoid delays or discards until the ratelimit is hit. When
		/// using it to enforce a ratelimit on your own process, for example when
		/// generating outbound requests, you may want there to be zero tokens
		/// availble initially to make your application more well-behaved in event
		/// of process restarts.
		///
		/// The default is that no tokens are initially available.
		pub fn initial_available(mut self, tokens: u64) -> Self {
			self.initial_available = tokens;
			self
		}

		/// Consumes this `Builder` and attempts to construct a `Ratelimiter`.
		pub fn build(self) -> Result<Ratelimiter, Error> {
			if self.max_tokens < self.refill_amount {
				return Err(Error::MaxTokensTooLow);
			}

			if self.refill_interval.as_nanos() > u64::MAX as u128 {
				return Err(Error::RefillIntervalTooLong);
			}

			let available = AtomicU64::new(self.initial_available);

			let parameters = Parameters {
				capacity: self.max_tokens,
				refill_amount: self.refill_amount,
				refill_interval: Duration::from_nanos(self.refill_interval.as_nanos() as u64),
			};

			let refill_at = AtomicInstant::new(Instant::now() + self.refill_interval);

			Ok(Ratelimiter {
				available,
				dropped: AtomicU64::new(0),
				parameters,
				refill_at,
			})
		}
	}

	#[cfg(test)]
	mod tests {
		use std::sync::Arc;
		use std::time::Duration;

		use clocksource::precise::{Duration as ClockDuration, Instant as ClockInstant};

		use super::*;

		fn force_refill_due(rl: &Ratelimiter, elapsed: ClockDuration) {
			rl.refill_at.store(
				ClockInstant::now()
					.checked_sub(elapsed)
					.unwrap_or_else(|| ClockInstant::from_nanos(0)),
				std::sync::atomic::Ordering::Relaxed,
			);
		}

		// quick test that a ratelimiter yields tokens at the desired rate
		#[test]
		pub fn wait() {
			let rl = Ratelimiter::builder(1, Duration::from_secs(1))
				.build()
				.unwrap();

			assert!(rl.try_wait().is_err());
			force_refill_due(&rl, ClockDuration::from_nanos(1));
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_err());
		}

		// quick test that an idle ratelimiter doesn't build up excess capacity
		#[test]
		pub fn idle() {
			let rl = Ratelimiter::builder(10, Duration::from_secs(3600))
				.max_tokens(10)
				.initial_available(10)
				.build()
				.unwrap();

			force_refill_due(&rl, ClockDuration::from_nanos(1));
			assert!(rl.next_refill() < ClockInstant::now());

			assert_eq!(rl.available_refill(), 10);
			assert!(rl.dropped() >= 10);
			assert!(rl.try_wait_n(10).is_ok());
			assert!(rl.try_wait().is_err());
			assert!(rl.next_refill() >= ClockInstant::now());

			force_refill_due(&rl, ClockDuration::from_nanos(1));
			assert!(rl.next_refill() < ClockInstant::now());
		}

		// quick test that capacity acts as expected
		#[test]
		pub fn capacity() {
			let rl = Ratelimiter::builder(10, Duration::from_secs(3600))
				.max_tokens(10)
				.initial_available(0)
				.build()
				.unwrap();

			force_refill_due(&rl, ClockDuration::from_nanos(1));
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_err());
		}

		// Test that try_wait_n correctly acquires multiple tokens
		#[test]
		pub fn try_wait_n() {
			let rl = Ratelimiter::builder(10, Duration::from_secs(3600))
				.max_tokens(20)
				.initial_available(15)
				.build()
				.unwrap();

			// Should be able to acquire 10 tokens
			assert!(rl.try_wait_n(10).is_ok());
			assert_eq!(rl.available(), 5);

			// Should be able to acquire remaining 5 tokens
			assert!(rl.try_wait_n(5).is_ok());
			assert_eq!(rl.available(), 0);

			// Should fail to acquire 6 tokens when only 5 are available
			assert!(rl.try_wait_n(6).is_err());

			// Should fail to acquire 0 tokens
			assert!(rl.try_wait_n(0).is_err());

			// Should fail to acquire more than max_tokens
			assert!(rl.try_wait_n(21).is_err());
		}

		// Test that try_wait_n maintains atomicity
		#[test]
		pub fn try_wait_n_atomicity() {
			let rl = Arc::new(
				Ratelimiter::builder(1, Duration::from_secs(3600))
					.max_tokens(10)
					.initial_available(5)
					.build()
					.unwrap(),
			);

			let mut handles = vec![];
			let success_count = Arc::new(AtomicU64::new(0));

			// Spawn multiple threads trying to acquire 3 tokens each
			for _ in 0..5 {
				let rl = Arc::clone(&rl);
				let success_count = Arc::clone(&success_count);
				handles.push(std::thread::spawn(move || {
					if rl.try_wait_n(3).is_ok() {
						success_count.fetch_add(1, Ordering::SeqCst);
					}
				}));
			}

			// Wait for all threads to complete
			for handle in handles {
				handle.join().unwrap();
			}

			// Only one thread should have succeeded in acquiring 3 tokens
			// since we started with 5 tokens and each request needs 3
			assert_eq!(success_count.load(Ordering::SeqCst), 1);
			assert_eq!(rl.available(), 2);
		}

		// Test that try_wait_n works correctly with refills
		#[test]
		pub fn try_wait_n_with_refill() {
			let rl = Ratelimiter::builder(5, Duration::from_secs(3600))
				.max_tokens(10)
				.initial_available(5)
				.build()
				.unwrap();

			// Acquire all initial tokens
			assert!(rl.try_wait_n(3).is_ok());
			assert_eq!(rl.available(), 2);
			assert!(rl.try_wait_n(2).is_ok());
			assert_eq!(rl.available(), 0);
			assert!(rl.try_wait_n(1).is_err());

			force_refill_due(&rl, ClockDuration::from_nanos(1));
			assert!(rl.try_wait_n(5).is_ok());

			let remain = rl.available();
			assert!(remain <= 5, "expected <5 remaining tokens, got {remain}");
		}

		// Test basic amend_tokens functionality
		#[test]
		pub fn amend_tokens_basic() {
			let rl = Ratelimiter::builder(1, Duration::from_millis(10))
				.max_tokens(10)
				.initial_available(7)
				.build()
				.unwrap();

			assert_eq!(rl.available(), 7);

			// Remove 5 tokens, should have 2 left
			rl.amend_tokens(5);
			assert_eq!(rl.available(), 2);

			// Remove 1 more token, should have 1 left
			rl.amend_tokens(1);
			assert_eq!(rl.available(), 1);

			// Remove 3 tokens, should have 0 left (not negative)
			rl.amend_tokens(3);
			assert_eq!(rl.available(), 0);
		}

		// Test amend_tokens with zero tokens
		#[test]
		pub fn amend_tokens_zero() {
			let rl = Ratelimiter::builder(1, Duration::from_millis(10))
				.max_tokens(10)
				.initial_available(5)
				.build()
				.unwrap();

			assert_eq!(rl.available(), 5);

			// Removing 0 tokens should not change anything
			rl.amend_tokens(0);
			assert_eq!(rl.available(), 5);
		}

		// Test amend_tokens when removing more than available
		#[test]
		pub fn amend_tokens_overflow() {
			let rl = Ratelimiter::builder(1, Duration::from_millis(10))
				.max_tokens(10)
				.initial_available(3)
				.build()
				.unwrap();

			assert_eq!(rl.available(), 3);

			// Remove more tokens than available, should result in 0
			rl.amend_tokens(5);
			assert_eq!(rl.available(), 0);

			// Try to remove more tokens when already at 0
			rl.amend_tokens(10);
			assert_eq!(rl.available(), 0);
		}

		// Adding tokens back never pushes the bucket above its capacity
		#[test]
		pub fn amend_tokens_refund_is_capped() {
			let rl = Ratelimiter::builder(1, Duration::from_millis(10))
				.max_tokens(10)
				.initial_available(9)
				.build()
				.unwrap();
			rl.amend_tokens(-5);
			assert_eq!(rl.available(), 10);
			force_refill_due(&rl, ClockDuration::from_nanos(1));
			assert!(rl.try_wait().is_ok());
			assert_eq!(rl.available(), 9);
		}

		// Test amend_tokens with concurrent access
		#[test]
		pub fn amend_tokens_concurrent() {
			let rl = Arc::new(
				Ratelimiter::builder(1, Duration::from_millis(10))
					.max_tokens(20)
					.initial_available(15)
					.build()
					.unwrap(),
			);

			let mut handles = vec![];

			// Spawn multiple threads that amend tokens concurrently
			for i in 0..5 {
				let rl = Arc::clone(&rl);
				handles.push(std::thread::spawn(move || {
					// Each thread removes a different amount
					rl.amend_tokens(i + 1);
				}));
			}

			// Wait for all threads to complete
			for handle in handles {
				handle.join().unwrap();
			}

			// The final result should be deterministic: 15 - (1+2+3+4+5) = 0
			assert_eq!(rl.available(), 0);
		}

		// Test amend_tokens in combination with try_wait
		#[test]
		pub fn amend_tokens_with_try_wait() {
			let rl = Ratelimiter::builder(1, Duration::from_secs(3600))
				.max_tokens(10)
				.initial_available(8)
				.build()
				.unwrap();

			assert_eq!(rl.available(), 8);

			// First acquire some tokens normally
			assert!(rl.try_wait_n(3).is_ok());
			assert_eq!(rl.available(), 5);

			// Then amend tokens after discovering the actual cost
			rl.amend_tokens(2);
			assert_eq!(rl.available(), 3);

			// Should still be able to acquire tokens
			assert!(rl.try_wait_n(2).is_ok());
			assert_eq!(rl.available(), 1);

			// Amend more tokens than available
			rl.amend_tokens(5);
			assert_eq!(rl.available(), 0);

			// Should not be able to acquire more tokens
			assert!(rl.try_wait().is_err());
		}

		// Test amend_tokens with refills
		#[test]
		pub fn amend_tokens_with_refills() {
			let rl = Ratelimiter::builder(5, Duration::from_secs(3600))
				.max_tokens(10)
				.initial_available(5)
				.build()
				.unwrap();

			assert_eq!(rl.available(), 5);

			// Remove all tokens
			rl.amend_tokens(5);
			assert_eq!(rl.available(), 0);

			force_refill_due(&rl, ClockDuration::from_nanos(1));
			assert!(rl.try_wait().is_ok());

			let available_after_refill = rl.available();
			assert!(available_after_refill > 0);
			rl.amend_tokens(2);
			assert_eq!(rl.available(), available_after_refill - 2);
		}
	}
}
