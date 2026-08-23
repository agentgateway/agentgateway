//! Admission control for downstream connections and in-flight HTTP requests.
//!
//! Two invariants drive the design:
//!
//! 1. A slot is reserved **synchronously, before `tokio::spawn`**. The accept loop is far
//!    hotter than the scheduler, so a limiter that only counts inside the spawned task lets a
//!    flood create thousands of sockets and HTTP/2 sessions before the cap is observed.
//! 2. Waiting uses [`tokio::sync::Semaphore`], not `Notify`. `Notify` stores at most one
//!    pending permit, so releasing N slots at once wakes a single waiter and loses the rest.
//!
//! When nothing is configured every operation is a single relaxed atomic load, so gateways
//! that do not opt in keep their previous behaviour and cost.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::types::agent::BindKey;
use crate::types::frontend;

/// Matches the documented default for `maxConnectionWait` / `maxRequestWait`.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
	pub max_active: u32,
	pub max_pending: u32,
	pub wait: Duration,
}

impl Limits {
	/// `None` means the feature is off: callers must take the zero-cost path.
	fn new(
		max_active: Option<u32>,
		max_pending: Option<u32>,
		wait: Option<Duration>,
	) -> Option<Self> {
		let max_active = max_active.filter(|m| *m > 0)?;
		Some(Self {
			max_active,
			max_pending: max_pending.unwrap_or(max_active),
			wait: wait.unwrap_or(DEFAULT_WAIT),
		})
	}

	pub fn from_tcp(tcp: Option<&frontend::TCP>) -> Option<Self> {
		let tcp = tcp?;
		Self::new(
			tcp.max_connections,
			tcp.max_pending_connections,
			tcp.max_connection_wait,
		)
	}

	pub fn from_http(http: Option<&frontend::HTTP>) -> Option<Self> {
		let http = http?;
		Self::new(
			http.max_concurrent_requests,
			http.max_pending_requests,
			http.max_request_wait,
		)
	}

	/// Overflow is rejected immediately instead of queueing.
	pub fn wait_disabled(&self) -> bool {
		self.max_pending == 0 || self.wait.is_zero()
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitError {
	QueueFull,
	Timeout,
}

impl std::fmt::Display for LimitError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::QueueFull => f.write_str("pending queue full"),
			Self::Timeout => f.write_str("wait for slot timed out"),
		}
	}
}

/// Held for as long as the connection or request occupies a slot.
pub enum Permit {
	Unlimited,
	Active(OwnedSemaphorePermit),
}

impl std::fmt::Debug for Permit {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Unlimited => f.write_str("Unlimited"),
			Self::Active(_) => f.write_str("Active"),
		}
	}
}

/// A reserved place in the wait queue.
///
/// Reserving happens synchronously in [`Limiter::admit`] so queue depth is already accurate
/// when the accept loop decides whether to keep accepting.
pub struct QueuedPermit {
	limiter: Arc<Limiter>,
	wait: Duration,
}

impl QueuedPermit {
	pub async fn wait_for_slot(self) -> Result<Permit, LimitError> {
		let sem = self.limiter.active.clone();
		match tokio::time::timeout(self.wait, sem.acquire_owned()).await {
			Ok(Ok(permit)) => Ok(Permit::Active(permit)),
			// The semaphore is never closed; treat it as "no capacity" rather than panicking.
			Ok(Err(_)) => Err(LimitError::QueueFull),
			Err(_) => Err(LimitError::Timeout),
		}
	}
}

impl Drop for QueuedPermit {
	fn drop(&mut self) {
		self.limiter.pending.fetch_sub(1, Ordering::Relaxed);
	}
}

pub enum Admit {
	/// Nothing to wait for: either unlimited or a slot was free.
	Ready(Permit),
	/// At capacity but queueing is allowed; the caller must await the queued permit.
	Queued(QueuedPermit),
	/// At capacity with no queue space. Drop the connection / reject the request.
	Reject,
}

impl Admit {
	/// `None` when the caller must reject without spawning anything.
	pub fn into_slot(self) -> Option<Slot> {
		match self {
			Self::Ready(permit) => Some(Slot::Held(permit)),
			Self::Queued(queued) => Some(Slot::Queued(queued)),
			Self::Reject => None,
		}
	}
}

/// An admitted unit of work, carried from the synchronous reservation into the spawned task.
pub enum Slot {
	Held(Permit),
	Queued(QueuedPermit),
}

impl Slot {
	pub async fn resolve(self) -> Result<Permit, LimitError> {
		match self {
			Self::Held(permit) => Ok(permit),
			Self::Queued(queued) => queued.wait_for_slot().await,
		}
	}
}

pub struct Limiter {
	/// Fast path for the common "no limits configured" case.
	configured: AtomicBool,
	limits: ArcSwapOption<Limits>,
	active: Arc<Semaphore>,
	/// Capacity currently issued to `active`; kept in sync with `Limits::max_active`.
	capacity: AtomicU32,
	resize: Mutex<()>,
	pending: AtomicUsize,
}

impl std::fmt::Debug for Limiter {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Limiter").finish_non_exhaustive()
	}
}

impl Limiter {
	fn new() -> Self {
		Self {
			configured: AtomicBool::new(false),
			limits: ArcSwapOption::empty(),
			active: Arc::new(Semaphore::new(0)),
			capacity: AtomicU32::new(0),
			resize: Mutex::new(()),
			pending: AtomicUsize::new(0),
		}
	}

	/// Latest limits seen by [`Limiter::admit`]. Used by the accept loop, which must not take
	/// a policy-store read lock per iteration.
	pub fn cached_limits(&self) -> Option<Limits> {
		if !self.configured.load(Ordering::Relaxed) {
			return None;
		}
		self.limits.load().as_deref().copied()
	}

	fn remember(&self, limits: Limits) {
		if self.cached_limits() != Some(limits) {
			self.limits.store(Some(Arc::new(limits)));
			self.configured.store(true, Ordering::Relaxed);
		}
	}

	/// Grow or shrink the semaphore to match `want`.
	///
	/// Shrinking can be partial when permits are checked out; the leftover is reclaimed on a
	/// later call, which is why `capacity` is only updated by what was actually removed.
	fn ensure_capacity(&self, want: u32) {
		if self.capacity.load(Ordering::Relaxed) == want {
			return;
		}
		let _guard = self.resize.lock().expect("admission resize lock poisoned");
		let current = self.capacity.load(Ordering::Relaxed);
		if current < want {
			self.active.add_permits((want - current) as usize);
			self.capacity.store(want, Ordering::Relaxed);
		} else if current > want {
			let removed = self.active.forget_permits((current - want) as usize);
			self
				.capacity
				.store(current - removed as u32, Ordering::Relaxed);
		}
	}

	fn try_reserve_pending(&self, max_pending: usize) -> bool {
		self
			.pending
			.fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
				(cur < max_pending).then_some(cur + 1)
			})
			.is_ok()
	}

	/// Reserve a slot synchronously. Never blocks.
	pub fn admit(self: &Arc<Self>, limits: Option<Limits>) -> Admit {
		let Some(limits) = limits else {
			return Admit::Ready(Permit::Unlimited);
		};
		self.remember(limits);
		self.ensure_capacity(limits.max_active);

		match self.active.clone().try_acquire_owned() {
			Ok(permit) => Admit::Ready(Permit::Active(permit)),
			Err(_) if limits.wait_disabled() => Admit::Reject,
			Err(_) => {
				if self.try_reserve_pending(limits.max_pending as usize) {
					Admit::Queued(QueuedPermit {
						limiter: self.clone(),
						wait: limits.wait,
					})
				} else {
					Admit::Reject
				}
			},
		}
	}

	/// Convenience for call sites that can afford to await (tests, non-accept paths).
	pub async fn acquire(self: &Arc<Self>, limits: Option<Limits>) -> Result<Permit, LimitError> {
		match self.admit(limits) {
			Admit::Ready(permit) => Ok(permit),
			Admit::Queued(queued) => queued.wait_for_slot().await,
			Admit::Reject => Err(LimitError::QueueFull),
		}
	}

	/// True when a freshly accepted connection would be rejected outright.
	pub fn at_capacity(&self) -> bool {
		let Some(limits) = self.cached_limits() else {
			return false;
		};
		if self.active.available_permits() > 0 {
			return false;
		}
		limits.wait_disabled() || self.pending.load(Ordering::Relaxed) >= limits.max_pending as usize
	}

	/// Park until a slot frees up. The borrowed permit is released immediately: this only
	/// exists to get a fair, never-lost wakeup from the semaphore queue.
	pub async fn wait_for_capacity(self: &Arc<Self>) {
		if !self.at_capacity() {
			return;
		}
		if let Ok(permit) = self.active.clone().acquire_owned().await {
			drop(permit);
		}
	}
}

#[derive(Default)]
pub struct AdmissionRegistry {
	inner: Mutex<HashMap<BindKey, Arc<Limiter>>>,
}

impl std::fmt::Debug for AdmissionRegistry {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AdmissionRegistry").finish_non_exhaustive()
	}
}

impl AdmissionRegistry {
	/// Resolve once per bind (accept loop) or per connection (HTTP requests) and reuse the
	/// handle; this takes a global lock and must stay off per-request paths.
	pub fn limiter(&self, bind: &BindKey) -> Arc<Limiter> {
		let mut inner = self.inner.lock().expect("admission registry poisoned");
		inner
			.entry(bind.clone())
			.or_insert_with(|| Arc::new(Limiter::new()))
			.clone()
	}
}

#[cfg(test)]
mod tests {
	use agent_core::strng;

	use super::*;

	fn limits(max: u32, pending: u32, wait: Duration) -> Option<Limits> {
		Some(Limits {
			max_active: max,
			max_pending: pending,
			wait,
		})
	}

	fn limiter() -> Arc<Limiter> {
		AdmissionRegistry::default().limiter(&strng::literal!("b"))
	}

	#[tokio::test]
	async fn unlimited_when_unset() {
		let l = limiter();
		assert!(matches!(l.acquire(None).await.unwrap(), Permit::Unlimited));
		assert!(!l.at_capacity());
	}

	#[tokio::test]
	async fn second_waits_for_first_to_finish() {
		let l = limiter();
		let cfg = limits(1, 4, Duration::from_secs(2));
		let first = l.acquire(cfg).await.unwrap();

		let l2 = l.clone();
		let waiting = tokio::spawn(async move { l2.acquire(cfg).await });
		tokio::time::sleep(Duration::from_millis(50)).await;
		assert!(!waiting.is_finished());

		drop(first);
		let second = tokio::time::timeout(Duration::from_secs(1), waiting)
			.await
			.unwrap()
			.unwrap()
			.unwrap();
		assert!(matches!(second, Permit::Active(_)));
	}

	#[tokio::test]
	async fn queue_full_does_not_wait() {
		let l = limiter();
		let cfg = limits(1, 0, Duration::from_secs(5));
		let _first = l.acquire(cfg).await.unwrap();
		assert_eq!(l.acquire(cfg).await.unwrap_err(), LimitError::QueueFull);
		assert!(l.at_capacity());
	}

	#[tokio::test]
	async fn wait_timeout_does_not_hold_slot() {
		let l = limiter();
		let cfg = limits(1, 4, Duration::from_millis(80));
		let first = l.acquire(cfg).await.unwrap();
		assert_eq!(l.acquire(cfg).await.unwrap_err(), LimitError::Timeout);
		drop(first);
		assert!(matches!(l.acquire(cfg).await.unwrap(), Permit::Active(_)));
	}

	/// Queue depth must be visible before the waiter is polled, otherwise a flood outruns
	/// the cap. This is the regression that made `maxConnections` ineffective.
	#[test]
	fn pending_is_reserved_synchronously() {
		let l = limiter();
		let cfg = limits(1, 2, Duration::from_secs(5));
		let _active = match l.admit(cfg) {
			Admit::Ready(p) => p,
			_ => panic!("first admit must take the active slot"),
		};
		let _q1 = match l.admit(cfg) {
			Admit::Queued(q) => q,
			_ => panic!("second admit must queue"),
		};
		assert!(!l.at_capacity(), "one queue slot is still free");
		let _q2 = match l.admit(cfg) {
			Admit::Queued(q) => q,
			_ => panic!("third admit must queue"),
		};
		assert!(l.at_capacity(), "queue is full without polling any waiter");
		assert!(matches!(l.admit(cfg), Admit::Reject));
	}

	/// `Notify` stored a single permit, so releasing many slots at once woke one waiter and
	/// stranded the rest until their timeout.
	#[tokio::test]
	async fn simultaneous_release_wakes_every_waiter() {
		let l = limiter();
		let cfg = limits(4, 8, Duration::from_secs(5));
		let held: Vec<_> = (0..4).map(|_| l.acquire(cfg)).collect();
		let mut permits = Vec::new();
		for h in held {
			permits.push(h.await.unwrap());
		}

		let waiters: Vec<_> = (0..4)
			.map(|_| {
				let l = l.clone();
				tokio::spawn(async move { l.acquire(cfg).await })
			})
			.collect();
		tokio::time::sleep(Duration::from_millis(50)).await;

		drop(permits);
		for w in waiters {
			let got = tokio::time::timeout(Duration::from_millis(500), w)
				.await
				.expect("waiter must wake without hitting its timeout")
				.unwrap()
				.unwrap();
			assert!(matches!(got, Permit::Active(_)));
		}
	}

	#[tokio::test]
	async fn capacity_shrinks_and_grows_with_config() {
		let l = limiter();
		let small = limits(1, 0, Duration::ZERO);
		let big = limits(3, 0, Duration::ZERO);

		let a = l.acquire(small).await.unwrap();
		assert!(l.acquire(small).await.is_err());

		// Growing must let new work in without dropping the held permit.
		let b = l.acquire(big).await.unwrap();
		let c = l.acquire(big).await.unwrap();
		assert!(l.acquire(big).await.is_err());
		drop((a, b, c));

		// Shrinking back must be observed once permits are returned.
		let _only = l.acquire(small).await.unwrap();
		assert!(l.acquire(small).await.is_err());
	}

	#[tokio::test]
	async fn wait_for_capacity_returns_when_slot_frees() {
		let l = limiter();
		let cfg = limits(1, 0, Duration::ZERO);
		let held = l.acquire(cfg).await.unwrap();
		assert!(l.at_capacity());

		let l2 = l.clone();
		let parked = tokio::spawn(async move { l2.wait_for_capacity().await });
		tokio::time::sleep(Duration::from_millis(50)).await;
		assert!(!parked.is_finished());

		drop(held);
		tokio::time::timeout(Duration::from_secs(1), parked)
			.await
			.expect("accept loop must be woken when a slot frees")
			.unwrap();
	}
}
