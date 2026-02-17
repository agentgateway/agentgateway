use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use arc_swap::ArcSwapOption;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::{ResolveError, TokioResolver};

use crate::*;

const ERROR_BACKOFF_MAX: Duration = Duration::from_secs(5);
const ERROR_BACKOFF_BASE: Duration = Duration::from_millis(100);

#[derive(Debug)]
struct CircularBuffer<T> {
	data: Box<[T]>,
	index: AtomicUsize,
}

impl<T> CircularBuffer<T> {
	fn new(items: Box<[T]>) -> Self {
		Self {
			data: items,
			index: AtomicUsize::new(0),
		}
	}
}

impl<T: Copy> CircularBuffer<T> {
	fn get_and_advance(&self) -> Option<T> {
		if self.data.is_empty() {
			return None;
		}
		let current = self.index.fetch_add(1, Ordering::Relaxed);
		Some(self.data[current % self.data.len()])
	}
}

#[derive(Debug, Clone)]
pub struct CachedResolver {
	dns: Arc<Resolver>,
	entries: Arc<Mutex<HashMap<Strng, Arc<CacheEntry>>>>,
}

#[derive(Debug)]
pub struct CacheEntry {
	// active keeps track of whether we have fetched this since the last fetch
	active: AtomicBool,
	entries: ArcSwapOption<CircularBuffer<IpAddr>>,
	notify: tokio::sync::Notify,
	background_task: ArcSwapOption<tokio::task::JoinHandle<()>>,
}

impl CacheEntry {
	async fn background(
		&self,
		name: Strng,
		resolver: Arc<Resolver>,
		cache: Arc<Mutex<HashMap<Strng, Arc<CacheEntry>>>>,
	) {
		self.active.store(true, Ordering::Relaxed);

		let mut backoff = ERROR_BACKOFF_BASE;

		loop {
			// Mark this is inactive, so we can see if there are any request before the next refresh timer.
			let was_active = self.active.swap(false, Ordering::Relaxed);
			if !was_active {
				// We are done; no one requested this.
				// Remove the cache entry if there is one.
				if let Ok(mut cache) = cache.lock() {
					cache.remove(&name);
				}
				return;
			}
			let (next_refresh, respect_small_value) = match resolver.resolve(name.as_str()).await {
				Ok((ips, expiry)) => {
					let cb = CircularBuffer::new(ips);
					self.entries.store(Some(Arc::new(cb)));
					// reset backoff on success
					backoff = ERROR_BACKOFF_BASE;
					(expiry, false)
				},
				Err(e) => {
					let cb = CircularBuffer::new(Default::default());
					// We got a result, its just empty
					self.entries.store(Some(Arc::new(cb)));
					// if we got an error, retain the last state
					debug!("resolution failed: {e:?}");

					backoff = std::cmp::min(backoff * 2, ERROR_BACKOFF_MAX);
					(Instant::now() + backoff, true)
				},
			};
			// NB: this will run even on error, so the first fetch for a failed response will hit this and
			// not block
			self.notify.notify_waiters();
			sleep_until_expired(next_refresh, respect_small_value).await;
		}
	}

	pub async fn next(&self) -> Option<IpAddr> {
		// Mark as active
		self.active.store(true, Ordering::Relaxed);
		// Is there an entry right now? If so return it.
		let notify = self.notify.notified();
		if let Some(entry) = self.entries.load().as_ref() {
			return entry.get_and_advance();
		}
		// Wait until a change happens
		notify.await;
		// Now attempt to load or return None if there is nothing available
		self
			.entries
			.load()
			.as_ref()
			.and_then(|cb| cb.get_and_advance())
	}
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum Resolver {
	Real(TokioResolver),
	#[cfg(test)]
	Mock(Arc<tests::Mock>),
}

impl Resolver {
	async fn resolve(&self, host: &str) -> Result<(Box<[IpAddr]>, Instant), ResolveError> {
		match self {
			Resolver::Real(resolver) => resolver.lookup_ip(host).await.map(|lookup| {
				let expiry = lookup.valid_until();
				let ips = lookup.iter().collect::<Box<_>>();
				(ips, expiry)
			}),
			#[cfg(test)]
			Resolver::Mock(resolver) => resolver.resolve(host).await,
		}
	}
}

impl CachedResolver {
	pub fn new(config: ResolverConfig, opts: ResolverOpts) -> Self {
		let mut rb =
			hickory_resolver::Resolver::builder_with_config(config, TokioConnectionProvider::default());
		*rb.options_mut() = opts;
		let dns_resolver = rb.build();
		CachedResolver {
			entries: Arc::new(Mutex::new(HashMap::new())),
			dns: Arc::new(Resolver::Real(dns_resolver)),
		}
	}

	pub async fn resolve(&self, name: Strng) -> anyhow::Result<IpAddr> {
		// Check if we already have an entry
		let entry = {
			let mut cache = self.entries.lock().unwrap();
			let existing_entry = cache.get(&name).cloned();
			if let Some(entry) = existing_entry {
				// Mark as active and return next IP
				entry.active.store(true, Ordering::Relaxed);
				entry
			} else {
				let entry = Arc::new(CacheEntry {
					active: AtomicBool::new(false),
					entries: Default::default(),
					notify: Default::default(),
					background_task: Default::default(),
				});

				cache.insert(name.clone(), entry.clone());
				// Start background task
				let bg_entry = entry.clone();
				let dns = self.dns.clone();
				let cache = self.entries.clone();
				let handle = tokio::task::spawn(async move {
					bg_entry.background(name, dns, cache).await;
				});
				entry.background_task.store(Some(Arc::new(handle)));

				entry
			}
		};

		// Return next IP
		entry.next().await.ok_or_else(|| anyhow!("no ip"))
	}
}

async fn sleep_until_expired(valid_until: Instant, respect_small_value: bool) {
	const MINIMUM_TTL: Duration = Duration::from_secs(5);
	let minimum = Instant::now() + MINIMUM_TTL;

	let deadline = if respect_small_value || valid_until >= minimum {
		valid_until
	} else {
		minimum
	};

	tokio::time::sleep_until(deadline.into()).await;
}

#[cfg(test)]
#[path = "dns_tests.rs"]
mod tests;
