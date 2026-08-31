use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::types::agent::BindKey;

/// Stable, bind-scoped counters shared by accept loops and HTTP connections.
#[derive(Debug, Default)]
pub struct AdmissionRegistry {
	binds: Mutex<HashMap<BindKey, Arc<BindAdmission>>>,
}

impl AdmissionRegistry {
	pub fn bind(&self, key: &BindKey) -> Arc<BindAdmission> {
		let mut binds = self.binds.lock().expect("admission registry poisoned");
		binds.entry(key.clone()).or_default().clone()
	}
}

#[derive(Debug, Default)]
pub struct BindAdmission {
	/// Physical connections currently being processed by this bind.
	pub connections: Limiter,
	/// HTTP/1 requests and HTTP/2 streams currently being processed by this bind.
	pub requests: Limiter,
}

#[derive(Debug, Default)]
pub struct Limiter {
	active: Arc<AtomicUsize>,
}

impl Limiter {
	/// Reserves capacity.
	/// Limit is passed in as an argument, rather than as part of the Limiter to allow dynamically
	/// changing the limits.
	pub fn try_acquire(&self, limit: Option<NonZeroU32>) -> Option<Permit> {
		let Some(limit) = limit else {
			return Some(Permit { active: None });
		};
		let limit = limit.get() as usize;
		self
			.active
			.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
				(active < limit).then_some(active + 1)
			})
			.ok()?;
		Some(Permit {
			active: Some(self.active.clone()),
		})
	}

	#[cfg(test)]
	fn active(&self) -> usize {
		self.active.load(Ordering::Acquire)
	}
}

pub struct Permit {
	// Unlimited permits have no counter to release.
	active: Option<Arc<AtomicUsize>>,
}

impl Drop for Permit {
	fn drop(&mut self) {
		if let Some(active) = &self.active {
			active.fetch_sub(1, Ordering::Relaxed);
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Barrier;

	use super::*;

	#[test]
	fn concurrent_acquisition_never_exceeds_limit() {
		let limiter = Arc::new(Limiter::default());
		let barrier = Arc::new(Barrier::new(33));
		let mut tasks = Vec::new();
		for _ in 0..32 {
			let limiter = limiter.clone();
			let barrier = barrier.clone();
			tasks.push(std::thread::spawn(move || {
				barrier.wait();
				limiter.try_acquire(NonZeroU32::new(4))
			}));
		}
		barrier.wait();
		let permits: Vec<_> = tasks
			.into_iter()
			.filter_map(|task| task.join().unwrap())
			.collect();
		assert_eq!(permits.len(), 4);
		assert_eq!(limiter.active(), 4);
		drop(permits);
		assert_eq!(limiter.active(), 0);
	}
}
