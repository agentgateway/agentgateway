use std::fmt;
use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use quick_cache::sync::{Cache, EntryAction, EntryResult};

// Caps TTLs so `Instant` arithmetic cannot overflow; effectively "never expires".
const MAX_TTL: Duration = Duration::from_secs(100 * 365 * 24 * 60 * 60);

/// Bounded cache of remotely fetched values, each with its own TTL.
/// Shortly before an entry expires, a single caller is elected to refresh it while
/// everyone else keeps getting the cached value.
pub struct ExpiringCache<K, V>(Arc<Cache<K, Entry<V>>>);

#[derive(Clone)]
struct Entry<V> {
	value: V,
	refresh_at: Instant,
	expires_at: Instant,
	refreshing: Arc<AtomicBool>,
}

#[derive(Debug)]
pub enum Lookup<V> {
	Hit(V),
	/// The entry is near expiry and this caller was elected to refresh it.
	Refresh,
	Expired,
	Vacant,
}

impl<V: Clone> Entry<V> {
	fn lookup(&self, now: Instant) -> Lookup<V> {
		if self.expires_at <= now {
			Lookup::Expired
		} else if now < self.refresh_at || self.refreshing.swap(true, Ordering::AcqRel) {
			Lookup::Hit(self.value.clone())
		} else {
			Lookup::Refresh
		}
	}
}

impl<K: Eq + Hash + Clone, V: Clone> ExpiringCache<K, V> {
	pub fn new(capacity: usize) -> Self {
		Self(Arc::new(Cache::new(capacity)))
	}

	pub fn capacity(&self) -> u64 {
		self.0.capacity()
	}

	pub fn lookup(&self, key: &K) -> Lookup<V> {
		let Some(entry) = self.0.get(key) else {
			return Lookup::Vacant;
		};
		let now = Instant::now();
		let lookup = entry.lookup(now);
		if matches!(lookup, Lookup::Expired) {
			self.0.remove_if(key, |entry| entry.expires_at <= now);
		}
		lookup
	}

	pub fn insert(&self, key: K, value: V, ttl: Duration) {
		self.0.insert(key, new_entry(value, ttl));
	}

	/// Returns the cached value, or fetches it. Concurrent misses for the same key share
	/// one fetch. If a refresh fails, the cached value is served until it expires.
	/// `fetch` returns the value and how long to cache it, or `None` to not cache it.
	pub async fn get_or_fetch<F, Fut, E>(&self, key: K, fetch: F) -> Result<V, E>
	where
		F: FnOnce() -> Fut,
		Fut: Future<Output = Result<(V, Option<Duration>), E>>,
		E: fmt::Display,
	{
		let now = Instant::now();
		let guard = match self
			.0
			.entry_async(&key, |_, entry| match entry.lookup(now) {
				Lookup::Hit(value) => EntryAction::Retain(Ok(value)),
				Lookup::Refresh => EntryAction::Retain(Err(entry.value.clone())),
				Lookup::Expired | Lookup::Vacant => EntryAction::ReplaceWithGuard,
			})
			.await
		{
			EntryResult::Retained(Ok(value)) => return Ok(value),
			EntryResult::Retained(Err(stale)) => {
				return Ok(match fetch().await {
					Ok((value, ttl)) => {
						if let Some(ttl) = ttl {
							self.insert(key, value.clone(), ttl);
						}
						value
					},
					Err(e) => {
						tracing::debug!("refresh failed, serving cached value: {e}");
						stale
					},
				});
			},
			EntryResult::Vacant(guard) | EntryResult::Replaced(guard, _) => guard,
			EntryResult::Removed(_, _) | EntryResult::Timeout => unreachable!(),
		};
		let (value, ttl) = fetch().await?;
		if let Some(ttl) = ttl {
			let _ = guard.insert(new_entry(value.clone(), ttl));
		}
		Ok(value)
	}
}

fn new_entry<V>(value: V, ttl: Duration) -> Entry<V> {
	let ttl = ttl.min(MAX_TTL);
	let now = Instant::now();
	Entry {
		value,
		refresh_at: now + ttl - refresh_window(ttl),
		expires_at: now + ttl,
		refreshing: Arc::new(AtomicBool::new(false)),
	}
}

fn refresh_window(ttl: Duration) -> Duration {
	std::cmp::min(ttl / 10, Duration::from_secs(5))
}

impl<K, V> Clone for ExpiringCache<K, V> {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}

impl<K, V> fmt::Debug for ExpiringCache<K, V> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("ExpiringCache")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Inserts an entry already inside its refresh window.
	fn refreshable(cache: &ExpiringCache<&'static str, i32>, value: i32) {
		let now = Instant::now();
		cache.0.insert(
			"k",
			Entry {
				value,
				refresh_at: now,
				expires_at: now + Duration::from_secs(60),
				refreshing: Arc::new(AtomicBool::new(false)),
			},
		);
	}

	#[test]
	fn lookup_elects_one_refresher() {
		let cache = ExpiringCache::new(8);
		refreshable(&cache, 1);
		assert!(matches!(cache.lookup(&"k"), Lookup::Refresh));
		assert!(matches!(cache.lookup(&"k"), Lookup::Hit(1)));

		cache.insert("k", 2, Duration::ZERO);
		assert!(matches!(cache.lookup(&"k"), Lookup::Expired));
		assert!(matches!(cache.lookup(&"k"), Lookup::Vacant));
	}

	#[tokio::test]
	async fn failed_refresh_serves_cached_value() {
		let cache = ExpiringCache::new(8);
		refreshable(&cache, 1);
		let v = cache
			.get_or_fetch("k", || async { Err::<(i32, _), _>("down") })
			.await;
		assert_eq!(v, Ok(1));
		// The election stays taken, so no further refresh is attempted before expiry.
		let v = cache
			.get_or_fetch("k", || async { panic!("unexpected refresh") })
			.await;
		assert_eq!(v, Ok::<_, &str>(1));
	}
}
