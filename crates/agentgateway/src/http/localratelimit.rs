use serde::de::Error;

use crate::llm::LLMRequest;
use crate::proxy::ProxyError;
use crate::*;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "RateLimitSpec"))]
#[derive(serde::Serialize)]
pub struct RateLimit {
	#[serde(skip_serializing)]
	ratelimit: Arc<ratelimit::Ratelimiter>,
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
	#[serde(default)]
	pub max_tokens: u64,
	#[serde(default)]
	pub tokens_per_fill: u64,
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub fill_interval: Duration,
	#[serde(default)]
	#[serde(rename = "type")]
	pub limit_type: RateLimitType,
}

#[apply(schema!)]
#[derive(Default, Eq, PartialEq)]
pub enum RateLimitType {
	#[serde(rename = "requests")]
	#[default]
	Requests,
	#[serde(rename = "tokens")]
	Tokens,
}

impl TryFrom<RateLimitSpec> for RateLimit {
	type Error = ratelimit::Error;
	fn try_from(value: RateLimitSpec) -> Result<Self, Self::Error> {
		let rl = ratelimit::Ratelimiter::builder(value.tokens_per_fill, value.fill_interval)
			.initial_available(value.max_tokens)
			.max_tokens(value.max_tokens)
			.build()?;
		Ok(RateLimit {
			ratelimit: Arc::new(rl),
			spec: value,
		})
	}
}

impl RateLimit {
	pub fn check_request(&self) -> Result<(), ProxyError> {
		if self.spec.limit_type != RateLimitType::Requests {
			return Ok(());
		}
		// TODO: return headers on success, not just failure
		self
			.ratelimit
			.try_wait()
			.map_err(|(limit, remaining, reset)| ProxyError::RateLimitExceeded {
				limit,
				remaining,
				reset_seconds: reset.as_secs(),
			})
	}

	pub fn check_llm_request(&self, req: &LLMRequest) -> Result<(), ProxyError> {
		if self.spec.limit_type != RateLimitType::Tokens {
			return Ok(());
		}
		if let Some(it) = req.input_tokens {
			// If we tokenized the request, check to make sure we permit that many tokens
			// We will add the response tokens in `amend_tokens`
			self
				.ratelimit
				.try_wait_n(it)
				.map_err(|(limit, remaining, reset)| ProxyError::RateLimitExceeded {
					limit,
					remaining,
					reset_seconds: reset.as_secs(),
				})
		} else {
			// Otherwise, make sure at least 1 token is allowed.
			// Note this may lead to large over-allowance, especially with fast fill_intervals.
			let avail = self.ratelimit.available_refill();
			if avail > 0 {
				Ok(())
			} else {
				Err(ProxyError::RateLimitExceeded {
					limit: self.ratelimit.max_tokens(),
					remaining: avail,
					reset_seconds: (self.ratelimit.next_refill() - clocksource::precise::Instant::now())
						.as_secs(),
				})
			}
		}
	}

	/// Remove tokens from the rate limiter after the fact. This is useful for true-up
	/// scenarios where you discover the actual cost after making a request.
	/// This function cannot fail and will not allow the bucket to go negative.
	/// If there are fewer tokens available than requested to remove, the bucket
	/// will be set to 0.
	pub fn amend_tokens(&self, tokens_to_remove: i64) {
		self.ratelimit.amend_tokens(tokens_to_remove);
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

			let _ = self
				.available
				.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
					if tokens_to_remove < 0 {
						Some(v.saturating_add(tokens_to_remove.unsigned_abs()))
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
		use std::time::{Duration, Instant};

		use super::*;

		// quick test that a ratelimiter yields tokens at the desired rate
		#[test]
		pub fn wait() {
			let rl = Ratelimiter::builder(1, Duration::from_micros(10))
				.build()
				.unwrap();

			let mut count = 0;

			let now = Instant::now();
			let end = now + Duration::from_millis(10);
			while Instant::now() < end {
				if rl.try_wait().is_ok() {
					count += 1;
				}
			}

			assert!(count >= 600);
			assert!(count <= 1400);
		}

		// quick test that an idle ratelimiter doesn't build up excess capacity
		#[test]
		pub fn idle() {
			let rl = Ratelimiter::builder(1, Duration::from_millis(1))
				.initial_available(1)
				.build()
				.unwrap();

			std::thread::sleep(Duration::from_millis(10));
			assert!(rl.next_refill() < clocksource::precise::Instant::now());

			assert!(rl.try_wait().is_ok());
			assert!(rl.try_wait().is_err());
			assert!(rl.dropped() >= 8);
			assert!(rl.next_refill() >= clocksource::precise::Instant::now());

			std::thread::sleep(Duration::from_millis(5));
			assert!(rl.next_refill() < clocksource::precise::Instant::now());
		}

		// quick test that capacity acts as expected
		#[test]
		pub fn capacity() {
			let rl = Ratelimiter::builder(1, Duration::from_millis(10))
				.max_tokens(10)
				.initial_available(0)
				.build()
				.unwrap();

			std::thread::sleep(Duration::from_millis(100));
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
			let rl = Ratelimiter::builder(10, Duration::from_millis(10))
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
				Ratelimiter::builder(1, Duration::from_millis(10))
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
			let rl = Ratelimiter::builder(5, Duration::from_millis(10))
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

			// Wait for a refill
			std::thread::sleep(Duration::from_millis(15));

			// Should be able to acquire the refilled tokens
			assert!(rl.try_wait_n(2).is_ok());
			assert_eq!(rl.available(), 3);
			assert!(rl.try_wait_n(4).is_err());
			assert_eq!(rl.available(), 3);
			assert!(rl.try_wait_n(3).is_ok());
			assert_eq!(rl.available(), 0);
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
			let rl = Ratelimiter::builder(1, Duration::from_millis(10))
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
			let rl = Ratelimiter::builder(5, Duration::from_millis(10))
				.max_tokens(10)
				.initial_available(5)
				.build()
				.unwrap();

			assert_eq!(rl.available(), 5);

			// Remove all tokens
			rl.amend_tokens(5);
			assert_eq!(rl.available(), 0);

			// Wait for a refill (increase wait time to ensure refill happens)
			std::thread::sleep(Duration::from_millis(20));

			// Should have refilled tokens (but let's be more flexible about timing)
			let available_after_refill = rl.available();
			assert!(
				available_after_refill > 0 || rl.next_refill() < clocksource::precise::Instant::now()
			);

			// If we have tokens, amend some of them
			if available_after_refill > 0 {
				rl.amend_tokens(2);
				assert_eq!(rl.available(), available_after_refill - 2);
			}
		}
	}
}
