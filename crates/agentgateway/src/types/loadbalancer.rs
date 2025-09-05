use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use indexmap::IndexMap;
use itertools::Itertools;
use rand::Rng;
use serde::ser::SerializeSeq;
use tokio::sync::mpsc;
use tokio::time::sleep_until;

use crate::types::discovery::{Endpoint, Service, Workload};
use crate::*;

type EndpointKey = Strng;

#[derive(Debug, Clone, Serialize)]
pub struct EndpointWithInfo<T> {
	pub endpoint: Arc<T>,
	pub info: Arc<EndpointInfo>,
}

impl<T> EndpointWithInfo<T> {
	pub fn new(ep: T) -> Self {
		Self {
			endpoint: Arc::new(ep),
			info: Default::default(),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct EndpointGroup<T> {
	active: IndexMap<EndpointKey, EndpointWithInfo<T>>,
	rejected: IndexMap<EndpointKey, EndpointWithInfo<T>>,
}

impl<T> Default for EndpointGroup<T> {
	fn default() -> Self {
		EndpointGroup::<T> {
			active: IndexMap::new(),
			rejected: IndexMap::new(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct EndpointSet<T> {
	buckets: Vec<Atomic<EndpointGroup<T>>>,
	tx_eviction: mpsc::Sender<EvictionEvent>,

	// Updates to `bucket` are atomically swapped to make read actions fast.
	// However, this introduces a TOCTOU race when we have add/delete and evictions on the same time.
	// Practically speaking, these are all handled by the single main thread, but use a mutex to verify this.
	// Note: we could have both of these handled by the worker, but the add/remove come from XDS without any async support.
	action_mutex: Arc<Mutex<()>>,
}

impl EndpointSet<Endpoint> {
	pub fn insert(&self, ep: Endpoint) {
		// Currently, buckets are not supported
		self.insert_key(ep.workload_uid.clone(), ep, 0)
	}
	pub fn select_endpoint(
		&self,
		workloads: &store::WorkloadStore,
		svc: &Service,
		svc_port: u16,
		override_dest: Option<SocketAddr>,
	) -> Option<(Arc<Endpoint>, ActiveHandle, Arc<Workload>)> {
		let target_port = svc.ports.get(&svc_port).copied();

		if target_port.is_none() {
			// Port doesn't exist on the service at all, this is invalid
			debug!("service {} does not have port {}", svc.hostname, svc_port);
			return None;
		};

		let iter = svc.endpoints.iter();
		let selected = if let Some(o) = override_dest {
			iter.iter().find_map(|(ep, ep_info)| {
				let Some(wl) = workloads.find_uid(&ep.workload_uid) else {
					debug!("failed to fetch workload for {}", ep.workload_uid);
					return None;
				};
				if wl.workload_ips.contains(&o.ip()) {
					Some((ep.clone(), ep_info, wl))
				} else {
					None
				}
			})
		} else {
			let index = iter.index();
			if index.is_empty() {
				return None;
			}
			// Intentionally allow `rand::seq::index::sample` so we can pick the same element twice
			// This avoids starvation where the worst endpoint gets 0 traffic
			let a = rand::rng().random_range(0..index.len());
			let b = rand::rng().random_range(0..index.len());
			let best = [a, b]
				.into_iter()
				.filter_map(|idx| {
					let (_, EndpointWithInfo { endpoint, info }) =
						index.get_index(idx).expect("index already checked");
					let Some(wl) = workloads.find_uid(&endpoint.workload_uid) else {
						debug!("failed to fetch workload for {}", endpoint.workload_uid);
						return None;
					};
					if target_port.unwrap_or_default() == 0 && !endpoint.port.contains_key(&svc_port) {
						// Filter workload out, it doesn't have a matching port
						// This is not great, since if we have a lot of partial endpoints we hit bad cases.
						// However, this is rare enough in typical workloads that its not a big deal ATM.
						trace!(
							"filter endpoint {}, it does not have service port {}",
							endpoint.workload_uid, svc_port
						);
						return None;
					}
					Some((endpoint.clone(), info, wl))
				})
				.max_by(|(_, a, _), (_, b, _)| a.score().total_cmp(&b.score()));
			if let Some(best) = best {
				Some(best)
			} else {
				// Fallback to O(n) lookup
				iter
					.iter()
					.filter_map(|(ep, ep_info)| {
						let Some(wl) = workloads.find_uid(&ep.workload_uid) else {
							debug!("failed to fetch workload for {}", ep.workload_uid);
							return None;
						};
						if target_port.unwrap_or_default() == 0 && !ep.port.contains_key(&svc_port) {
							// Filter workload out, it doesn't have a matching port
							trace!(
								"filter endpoint {}, it does not have service port {}",
								ep.workload_uid, svc_port
							);
							return None;
						}
						Some((ep.clone(), ep_info, wl))
					})
					.max_by(|(_, a, _), (_, b, _)| a.score().total_cmp(&b.score()))
			}
		};
		let (ep, ep_info, wl) = selected?;
		let handle = svc
			.endpoints
			.start_request(ep.workload_uid.clone(), ep_info);
		Some((ep, handle, wl))
	}
}

#[derive(Debug)]
pub enum EndpointEvent<T> {
	Add(EndpointKey, EndpointWithInfo<T>, usize),
	Delete(EndpointKey),
}

#[derive(Debug)]
pub enum EvictionEvent {
	Evict(EndpointKey, Instant),
}

impl<T: Clone + Sync + Send + 'static> Default for EndpointSet<T> {
	fn default() -> Self {
		Self::new_empty(1)
	}
}

impl<T: Clone + Sync + Send + 'static> EndpointSet<T> {
	pub fn new(initial_set: Vec<Vec<(EndpointKey, T)>>) -> Self {
		let buckets = initial_set
			.into_iter()
			.map(|items| {
				let eg = EndpointGroup {
					active: IndexMap::from_iter(
						items
							.into_iter()
							.map(|(k, v)| (k, EndpointWithInfo::new(v))),
					),
					rejected: Default::default(),
				};
				Arc::new(ArcSwap::new(Arc::new(eg)))
			})
			.collect_vec();
		Self::new_with_buckets(buckets)
	}
	pub fn new_empty(priority_levels: usize) -> Self {
		Self::new_with_buckets(vec![Default::default(); priority_levels])
	}
	fn new_with_buckets(buckets: Vec<Atomic<EndpointGroup<T>>>) -> Self {
		let (tx_eviction, rx_eviction) = mpsc::channel(10);
		Self::worker(rx_eviction, buckets.clone());
		Self {
			buckets,
			tx_eviction,
			action_mutex: Arc::new(Mutex::new(())),
		}
	}

	pub fn start_request(&self, key: Strng, info: &Arc<EndpointInfo>) -> ActiveHandle {
		info.start_request(key, self.tx_eviction.clone())
	}

	fn find_bucket(&self, key: &EndpointKey) -> Option<Arc<EndpointGroup<T>>> {
		self.buckets.iter().find_map(|x| {
			let b = x.load_full();
			if b.active.contains_key(key) || b.rejected.contains_key(key) {
				Some(b)
			} else {
				None
			}
		})
	}

	fn find_bucket_atomic(
		buckets: &[Atomic<EndpointGroup<T>>],
		key: &EndpointKey,
	) -> Option<Atomic<EndpointGroup<T>>> {
		buckets.iter().find_map(|x| {
			let b = x.load_full();
			if b.active.contains_key(key) || b.rejected.contains_key(key) {
				Some(x.clone())
			} else {
				None
			}
		})
	}

	fn best_bucket(&self) -> Arc<EndpointGroup<T>> {
		// find the first bucket with healthy endpoints
		self
			.buckets
			.iter()
			.find_map(|x| {
				let b = x.load_full();
				if !b.active.is_empty() { Some(b) } else { None }
			})
			// TODO: allow selecting across multiple buckets.
			.unwrap_or_else(|| self.buckets[0].load_full())
	}

	pub fn any<F>(&self, mut f: F) -> bool
	where
		F: FnMut(&T) -> bool,
	{
		for b in self.buckets.iter() {
			let bb = b.load_full();
			if bb.active.iter().any(|(_k, info)| f(info.endpoint.as_ref())) {
				return true;
			};
			if bb
				.rejected
				.iter()
				.any(|(_k, info)| f(info.endpoint.as_ref()))
			{
				return true;
			};
		}
		false
	}

	pub fn iter(&self) -> ActiveEndpointsIter<T> {
		ActiveEndpointsIter(self.best_bucket())
	}

	pub fn insert_key(&self, key: EndpointKey, ep: T, bucket: usize) {
		self.event(EndpointEvent::Add(key, EndpointWithInfo::new(ep), bucket))
	}
	pub fn remove(&self, key: EndpointKey) {
		self.event(EndpointEvent::Delete(key))
	}
	fn event(&self, item: EndpointEvent<T>) {
		let _mu = self.action_mutex.lock();

		match item {
			EndpointEvent::Add(key, ep, bucket) => {
				let mut eps = Arc::unwrap_or_clone(self.buckets[bucket].load_full());
				eps.rejected.swap_remove(&key);
				eps.active.insert(key, ep);
				self.buckets[bucket].store(Arc::new(eps));
			},
			EndpointEvent::Delete(key) => {
				let Some(bucket) = Self::find_bucket_atomic(self.buckets.as_slice(), &key) else {
					return;
				};
				let mut eps = Arc::unwrap_or_clone(bucket.load_full());
				eps.active.swap_remove(&key);
				eps.rejected.swap_remove(&key);
				bucket.store(Arc::new(eps));
			},
		}
	}
	fn worker(
		mut eviction_events: mpsc::Receiver<EvictionEvent>,
		buckets: Vec<Atomic<EndpointGroup<T>>>,
	) {
		tokio::task::spawn(async move {
			let mut uneviction_heap: BinaryHeap<(Instant, EndpointKey)> = Default::default();
			let handle_eviction = |uneviction_heap: &mut BinaryHeap<(Instant, EndpointKey)>| {
				let (_, key) = uneviction_heap.pop().expect("heap is empty");

				trace!(%key, "unevict");
				let Some(bucket) = Self::find_bucket_atomic(buckets.as_slice(), &key) else {
					return;
				};
				let mut eps = Arc::unwrap_or_clone(bucket.load_full());
				if let Some(ep) = eps.rejected.swap_remove(&key) {
					ep.info.evicted_until.store(None);
					eps.active.insert(key, ep);
				}
				bucket.store(Arc::new(eps));
			};
			let handle_recv_evict = |uneviction_heap: &mut BinaryHeap<(Instant, EndpointKey)>,
			                         o: Option<EvictionEvent>| {
				let Some(item) = o else {
					return;
				};

				let EvictionEvent::Evict(key, timer) = item;

				let Some(bucket) = Self::find_bucket_atomic(buckets.as_slice(), &key) else {
					return;
				};
				let mut eps = Arc::unwrap_or_clone(bucket.load_full());
				uneviction_heap.push((timer, key.clone()));
				if let Some(ep) = eps.active.swap_remove(&key) {
					eps.rejected.insert(key, ep);
				}
				bucket.store(Arc::new(eps));
			};
			loop {
				let evict_at = uneviction_heap.peek().map(|x| x.0);
				tokio::select! {
					true = maybe_sleep_until(evict_at) => handle_eviction(&mut uneviction_heap),
					item = eviction_events.recv() => {
						if item.is_none() { return };
						handle_recv_evict(&mut uneviction_heap, item)
					}
				}
			}
		});
	}
	pub async fn evict(&mut self, key: EndpointKey, time: Instant) {
		let Some(bucket) = self.find_bucket(&key) else {
			return;
		};
		if let Some(cur) = bucket.active.get(&key) {
			// Immediately store in the endpoint the eviction time, if its not already been evicted
			let prev = cur
				.info
				.evicted_until
				.compare_and_swap(&None::<Arc<_>>, Some(Arc::new(time)));
			if prev.is_none() {
				let tx = self.tx_eviction.clone();
				// If we were the one to evict it, trigger the real eviction async
				tokio::spawn(async move {
					let _ = tx.send(EvictionEvent::Evict(key, time)).await;
				});
			}
		}
	}
}

const ALPHA: f64 = 0.3;

#[derive(Debug, Serialize)]
pub struct EndpointInfo {
	/// health keeps track of the success rate for the endpoint.
	health: Ewma,
	/// request latency tracks the latency of requests
	request_latency: Ewma,
	/// pending_requests keeps track of the total number of pending requests.
	pending_requests: ActiveCounter,
	/// total_requests keeps track of the total number of requests.
	total_requests: AtomicU64,
	#[serde(with = "serde_instant_option")]
	/// evicted_until is the time at which the endpoint will be evicted.
	evicted_until: AtomicOption<Instant>,
}

impl Default for EndpointInfo {
	fn default() -> Self {
		Self {
			health: Ewma::new(1.0),
			// TODO: this will overload them on the first request
			request_latency: Default::default(),
			pending_requests: Default::default(),
			total_requests: Default::default(),
			evicted_until: Arc::new(Default::default()),
		}
	}
}

impl EndpointInfo {
	pub fn new() -> Self {
		Self::default()
	}
	// Todo: fine-tune the algorithm here
	pub fn score(&self) -> f64 {
		let latency_penalty =
			self.request_latency.load() * (1.0 + self.pending_requests.countf() * 0.1);
		self.health.load() / (1.0 + latency_penalty)
	}
	pub fn start_request(
		self: &Arc<Self>,
		key: Strng,
		tx_sender: mpsc::Sender<EvictionEvent>,
	) -> ActiveHandle {
		self.total_requests.fetch_add(1, Ordering::Relaxed);
		ActiveHandle {
			info: self.clone(),
			key,
			tx: tx_sender,
			counter: self.pending_requests.0.clone(),
		}
	}
}

#[derive(Debug, Default, Serialize)]
pub struct Ewma(atomic_float::AtomicF64);

impl Ewma {
	pub fn new(f: f64) -> Self {
		Ewma(atomic_float::AtomicF64::new(f))
	}
	pub fn load(&self) -> f64 {
		self.0.load(Ordering::Relaxed)
	}
	pub fn record(&self, nv: f64) {
		let _ = self
			.0
			.fetch_update(Ordering::SeqCst, Ordering::Relaxed, |old| {
				Some(if old == 0.0 {
					nv
				} else {
					ALPHA * nv + (1.0 - ALPHA) * old
				})
			});
	}
}

#[derive(Clone, Debug, Default)]
pub struct ActiveCounter(Arc<()>);

impl Serialize for ActiveCounter {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.count().serialize(serializer)
	}
}

#[derive(Clone, Debug)]
pub struct ActiveHandle {
	info: Arc<EndpointInfo>,
	key: Strng,
	tx: mpsc::Sender<EvictionEvent>,
	#[allow(dead_code)]
	counter: Arc<()>,
}

impl ActiveHandle {
	pub fn finish_request(self, success: bool, latency: Duration, eviction_time: Option<Duration>) {
		if success {
			self.info.request_latency.record(latency.as_secs_f64());
			self.info.health.record(1.0);
		} else {
			// Do not record request_latency on failure; its common for failures to be fast and skew results.
			self.info.health.record(0.0)
		};
		if let Some(eviction_time) = eviction_time {
			let time = Instant::now() + eviction_time;
			// Immediately store in the endpoint the eviction time, if its not already been evicted
			let prev = self
				.info
				.evicted_until
				.compare_and_swap(&None::<Arc<_>>, Some(Arc::new(time)));
			if prev.is_none() {
				let tx = self.tx.clone();
				let key = self.key.clone();
				// If we were the one to evict it, trigger the real eviction async
				tokio::spawn(async move {
					let _ = tx.send(EvictionEvent::Evict(key, time)).await;
				});
			}
		}
	}
}

impl ActiveCounter {
	pub fn new(&self) -> ActiveCounter {
		Default::default()
	}
	/// Count returns the number of active instances.
	pub fn count(&self) -> usize {
		// We have a count, so ignore that one
		Arc::strong_count(&self.0) - 1
	}
	pub fn countf(&self) -> f64 {
		self.count() as f64
	}
}

// tokio::select evaluates each pattern before checking the (optional) associated condition. Work
// around that by returning false to fail the pattern match when sleep is not viable.
async fn maybe_sleep_until(till: Option<Instant>) -> bool {
	match till {
		Some(till) => {
			sleep_until(till.into()).await;
			true
		},
		None => false,
	}
}

impl<T> serde::Serialize for EndpointSet<T>
where
	EndpointWithInfo<T>: Serialize,
	T: Serialize,
{
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let mut seq = serializer.serialize_seq(Some(self.buckets.len()))?;
		for b in self.buckets.iter() {
			seq.serialize_element(&b.load_full())?;
		}
		seq.end()
	}
}

pub struct ActiveEndpointsIter<T>(Arc<EndpointGroup<T>>);
impl<T> ActiveEndpointsIter<T> {
	pub fn iter(&self) -> impl ExactSizeIterator<Item = (&Arc<T>, &Arc<EndpointInfo>)> {
		self.index().iter().map(|(_k, v)| (&v.endpoint, &v.info))
	}
	pub fn index(&self) -> &IndexMap<EndpointKey, EndpointWithInfo<T>> {
		if self.0.active.is_empty() {
			// If we have no active endpoints, return the rejected ones
			&self.0.rejected
		} else {
			&self.0.active
		}
	}
}
