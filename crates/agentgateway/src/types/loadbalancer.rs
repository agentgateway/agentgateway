use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use arc_swap::ArcSwap;
use indexmap::IndexMap;
use itertools::Itertools;
use rand::RngExt;
use serde::ser::SerializeSeq;
use tokio::sync::mpsc;
use tokio::time::sleep_until;

use crate::types::discovery::{
	Endpoint, LoadBalancer, LoadBalancerMode, LoadBalancerScopes, Service, Workload,
};
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

	// Updates to `buckets` are atomically swapped to make reads fast, but every writer does
	// load→modify→store, which races when two writers touch the same bucket concurrently.
	// action_mutex serializes all mutators: XDS add/delete, rebucket, and the eviction worker.
	// Readers don't take it — they just load_full the ArcSwap.
	action_mutex: Arc<Mutex<()>>,
}
fn contains_target_port(ep: &Endpoint, wanted_target: u16) -> bool {
	ep.port.values().any(|tp| *tp == wanted_target)
}
impl EndpointSet<Endpoint> {
	pub fn insert(&self, ep: Endpoint, dest_workload: &Workload, ranker: &LocalityRanker) {
		let bucket = match ranker.bucket_for(dest_workload) {
			Some(b) => b,
			None => return, // Strict mode mismatch — drop
		};
		self.insert_key(ep.workload_uid.clone(), ep, bucket)
	}

	pub fn select_endpoint(
		&self,
		workloads: &store::WorkloadStore,
		svc: &Service,
		svc_port: u16,
		override_dest: Option<SocketAddr>,
	) -> Option<(Arc<Endpoint>, ActiveHandle, Arc<Workload>)> {
		let Some(target_port) = svc.ports.get(&svc_port).copied() else {
			debug!("service {} does not have port {}", svc.hostname, svc_port);
			return None;
		};

		let viable = |endpoint: &Arc<Endpoint>| -> Option<Arc<Workload>> {
			let Some(wl) = workloads.find_uid(&endpoint.workload_uid) else {
				debug!("failed to fetch workload for {}", endpoint.workload_uid);
				return None;
			};
			if target_port == 0 && !endpoint.port.contains_key(&svc_port) {
				trace!(
					"filter endpoint {}, no service port {}",
					endpoint.workload_uid, svc_port
				);
				return None;
			}
			Some(wl)
		};

		let selected = if let Some(o) = override_dest {
			// Explicit destination bypasses bucketing and health — search every endpoint
			// (active + rejected) so an evicted-but-explicitly-targeted backend is still reachable.
			self.all_endpoints().find_map(|(ep, info)| {
				let Some(wl) = workloads.find_uid(&ep.workload_uid) else {
					debug!("failed to fetch workload for {}", ep.workload_uid);
					return None;
				};
				if !wl.workload_ips.contains(&o.ip()) {
					return None;
				}
				if !contains_target_port(&ep, o.port()) {
					return None;
				}
				Some((ep, info, wl))
			})
		} else {
			// best_bucket() picks the first non-empty bucket (best locality tier).
			let iter = svc.endpoints.iter();
			let index = iter.index();
			if index.is_empty() {
				return None;
			}
			// Do not use `rand::seq::index::sample` so we can pick the same element twice
			// This avoids starvation where the worst endpoint gets 0 traffic
			let mut rng = rand::rng();
			let a = rng.random_range(0..index.len());
			let b = rng.random_range(0..index.len());
			let best = [a, b]
				.into_iter()
				.filter_map(|idx| {
					let (_, EndpointWithInfo { endpoint, info }) =
						index.get_index(idx).expect("index already checked");
					let wl = viable(endpoint)?;
					Some((endpoint.clone(), info.clone(), wl))
				})
				.max_by(|(_, a, _), (_, b, _)| a.score().total_cmp(&b.score()));

			best.or_else(|| {
				// Slow fallback: scan buckets in locality order, returning the first bucket
				// that yields any match. Per-bucket: prefer active, fall back to rejected
				// when active is empty (mirrors the fast path's `index()` semantics).
				self.buckets.iter().find_map(|bucket| {
					let group = bucket.load_full();
					let map = if !group.active.is_empty() {
						&group.active
					} else {
						&group.rejected
					};
					map
						.iter()
						.filter_map(|(_, ewi)| {
							let wl = viable(&ewi.endpoint)?;
							Some((ewi.endpoint.clone(), ewi.info.clone(), wl))
						})
						.max_by(|(_, a, _), (_, b, _)| a.score().total_cmp(&b.score()))
				})
			})
		};
		let (ep, ep_info, wl) = selected?;
		let handle = svc
			.endpoints
			.start_request(ep.workload_uid.clone(), &ep_info);
		Some((ep, handle, wl))
	}
}

/// Computes an endpoint's locality bucket from a service's `routing_preferences`.
/// Rank = length of the consecutive-matching prefix of preferences.
/// Bucket = `num_preferences - rank` (so bucket 0 = best match).
pub struct LocalityRanker<'a> {
	lb: Option<&'a LoadBalancer>,
	source: Option<&'a Workload>,
}

impl<'a> LocalityRanker<'a> {
	pub fn new(lb: Option<&'a LoadBalancer>, source: Option<&'a Workload>) -> Self {
		Self { lb, source }
	}

	/// Number of buckets needed. Modes that don't bucket (Standard/Passthrough) get 1 even if
	/// preferences are set, so every endpoint has somewhere to land.
	pub fn priority_levels(&self) -> usize {
		match self.lb {
			Some(lb) if Self::uses_buckets(lb) => lb.routing_preferences.len() + 1,
			_ => 1,
		}
	}

	/// Bucket index for the given destination workload. Lower = better match.
	/// Returns `None` for Strict-mode endpoints that don't fully match (should be dropped).
	/// If source is unknown, returns 0 so all endpoints stay reachable until rebucketed.
	pub fn bucket_for(&self, wl: &Workload) -> Option<usize> {
		if self.source.is_none() {
			return Some(0);
		}
		// Non-bucketing modes collapse to bucket 0 — preferences, if present, are ignored
		// instead of silently dropping endpoints into out-of-range buckets.
		if let Some(lb) = self.lb
			&& !Self::uses_buckets(lb)
		{
			return Some(0);
		}
		let rank = self.rank(wl)?;
		let n = self.lb.map(|lb| lb.routing_preferences.len()).unwrap_or(0);
		Some(n.saturating_sub(rank))
	}

	fn uses_buckets(lb: &LoadBalancer) -> bool {
		!matches!(
			lb.mode,
			LoadBalancerMode::Standard | LoadBalancerMode::Passthrough
		)
	}

	/// Returns the rank for this endpoint, or `None` if Strict mode requires full match and we
	/// did not reach it.
	pub fn rank(&self, wl: &Workload) -> Option<usize> {
		let (lb, src) = match (self.lb, self.source) {
			(Some(lb), Some(src)) => (lb, src),
			_ => return Some(0),
		};
		let mut rank = 0usize;
		for scope in &lb.routing_preferences {
			let matches = match scope {
				LoadBalancerScopes::Region => src.locality.region == wl.locality.region,
				LoadBalancerScopes::Zone => src.locality.zone == wl.locality.zone,
				LoadBalancerScopes::Subzone => src.locality.subzone == wl.locality.subzone,
				LoadBalancerScopes::Node => src.node == wl.node,
				LoadBalancerScopes::Cluster => src.cluster_id == wl.cluster_id,
				LoadBalancerScopes::Network => src.network == wl.network,
			};
			if matches {
				rank += 1;
			} else {
				break;
			}
		}
		if lb.mode == LoadBalancerMode::Strict && rank != lb.routing_preferences.len() {
			return None;
		}
		Some(rank)
	}
}

#[derive(Debug)]
pub enum EndpointEvent<T> {
	Add(EndpointKey, EndpointWithInfo<T>, usize),
	Delete(EndpointKey),
}

#[derive(Debug)]
pub enum EvictionEvent {
	Evict {
		key: EndpointKey,
		until: Instant,
		restore_health: Option<f64>,
	},
}

/// Entry for the uneviction heap. Ordered so the earliest `until` is popped first (min-heap via reversed Ord).
#[derive(Debug)]
struct UnevictEntry(Instant, EndpointKey, Option<f64>);

impl PartialEq for UnevictEntry {
	fn eq(&self, other: &Self) -> bool {
		self.0 == other.0 && self.1 == other.1
	}
}
impl Eq for UnevictEntry {}
impl PartialOrd for UnevictEntry {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}
impl Ord for UnevictEntry {
	fn cmp(&self, other: &Self) -> Ordering {
		// Reverse so earliest instant is "greater" and gets popped first from BinaryHeap (max-heap).
		other.0.cmp(&self.0).then_with(|| self.1.cmp(&other.1))
	}
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
		// Each bucket needs its own ArcSwap; `vec![x; n]` would clone one Arc n times
		// and have every bucket share the same backing storage.
		Self::new_with_buckets((0..priority_levels).map(|_| Default::default()).collect())
	}
	fn new_with_buckets(buckets: Vec<Atomic<EndpointGroup<T>>>) -> Self {
		let (tx_eviction, rx_eviction) = mpsc::channel(10);
		let action_mutex = Arc::new(Mutex::new(()));
		Self::worker(rx_eviction, buckets.clone(), action_mutex.clone());
		Self {
			buckets,
			tx_eviction,
			action_mutex,
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

	/// Iterate every endpoint across all buckets. Active endpoints from all buckets
	/// are yielded before any rejected endpoint, e.g.:
	///   active in bucket 0
	///   active in bucket 1
	///   rejected in bucket 0
	///   rejected in bucket 1
	pub fn all_endpoints(&self) -> AllEndpointsIter<'_, T> {
		AllEndpointsIter {
			buckets: &self.buckets,
			bucket_idx: 0,
			current: None,
			in_rejected: false,
		}
	}

	pub fn insert_key(&self, key: EndpointKey, ep: T, bucket: usize) {
		self.event(EndpointEvent::Add(key, EndpointWithInfo::new(ep), bucket))
	}
	pub fn remove(&self, key: EndpointKey) {
		self.event(EndpointEvent::Delete(key))
	}

	pub fn num_buckets(&self) -> usize {
		self.buckets.len()
	}

	/// Re-distribute every endpoint across buckets using `f`. Endpoints where `f` returns
	/// `None` are dropped (e.g. Strict mode mismatch). EndpointInfo (health, latency, ejection
	/// state) is preserved — same Arcs, just moved between buckets.
	///
	/// Bucket count stays the same. If the number of buckets needs to change (LB config change),
	/// rebuild the EndpointSet instead.
	pub fn rebucket<F>(&self, ranker: F)
	where
		F: Fn(&T) -> Option<usize>,
	{
		let _mu = self.action_mutex.lock();
		let n = self.buckets.len();

		let mut new_groups: Vec<EndpointGroup<T>> = (0..n).map(|_| EndpointGroup::default()).collect();

		for bucket in &self.buckets {
			let g = bucket.load_full();
			for (entries, rejected) in [(&g.active, false), (&g.rejected, true)] {
				for (key, ep) in entries {
					let Some(b) = ranker(ep.endpoint.as_ref()) else {
						continue;
					};
					if b >= n {
						continue;
					}
					let target = if rejected {
						&mut new_groups[b].rejected
					} else {
						&mut new_groups[b].active
					};
					target.insert(key.clone(), ep.clone());
				}
			}
		}

		for (i, group) in new_groups.into_iter().enumerate() {
			self.buckets[i].store(Arc::new(group));
		}
	}

	fn event(&self, item: EndpointEvent<T>) {
		let _mu = self.action_mutex.lock();

		match item {
			EndpointEvent::Add(key, ep, bucket) => {
				let Some(slot) = self.buckets.get(bucket) else {
					// TODO this currently cannot happen, but we could maybe get better
					// structural guarantees if we stored the lb settings along with the EndpointSet
					// so that an inserter will always be able to tell the bucket count
					trace!(
						"bucket {bucket} out of range (have {}), dropping endpoint {key}",
						self.buckets.len()
					);
					return;
				};
				let mut eps = Arc::unwrap_or_clone(slot.load_full());
				eps.rejected.swap_remove(&key);
				eps.active.insert(key, ep);
				slot.store(Arc::new(eps));
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
		action_mutex: Arc<Mutex<()>>,
	) {
		tokio::task::spawn(async move {
			let mut uneviction_heap: BinaryHeap<UnevictEntry> = Default::default();
			let handle_eviction = |uneviction_heap: &mut BinaryHeap<UnevictEntry>| {
				let UnevictEntry(_until, key, restore_health) =
					uneviction_heap.pop().expect("heap is empty");

				trace!(%key, "unevict");
				// Serialize against XDS add/delete and rebucket — without this, their load→store
				// can overwrite (or be overwritten by) this handler's mutation.
				let _mu = action_mutex.lock();
				let Some(bucket) = Self::find_bucket_atomic(buckets.as_slice(), &key) else {
					return;
				};
				let mut eps = Arc::unwrap_or_clone(bucket.load_full());
				if let Some(ep) = eps.rejected.swap_remove(&key) {
					ep.info.evicted_until.store(None);
					if let Some(h) = restore_health {
						// Health scoring assumes normalized values in [0.0, 1.0].
						ep.info.health.set(h.clamp(0.0, 1.0));
					}
					eps.active.insert(key, ep);
				}
				bucket.store(Arc::new(eps));
			};
			let handle_recv_evict = |uneviction_heap: &mut BinaryHeap<UnevictEntry>,
			                         o: Option<EvictionEvent>| {
				let Some(item) = o else {
					return;
				};

				let EvictionEvent::Evict {
					key,
					until,
					restore_health,
				} = item;

				let _mu = action_mutex.lock();
				let Some(bucket) = Self::find_bucket_atomic(buckets.as_slice(), &key) else {
					return;
				};
				let mut eps = Arc::unwrap_or_clone(bucket.load_full());

				uneviction_heap.push(UnevictEntry(until, key.clone(), restore_health));
				if let Some(ep) = eps.active.swap_remove(&key) {
					eps.rejected.insert(key, ep);
				}
				bucket.store(Arc::new(eps));
			};
			loop {
				let evict_at = uneviction_heap.peek().map(|e| e.0);
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
			let prev = cur
				.info
				.evicted_until
				.compare_and_swap(&None::<Arc<_>>, Some(Arc::new(time)));
			if prev.is_none() {
				let tx = self.tx_eviction.clone();
				tokio::spawn(async move {
					let _ = tx
						.send(EvictionEvent::Evict {
							key,
							until: time,
							restore_health: None,
						})
						.await;
				});
			}
		}
	}
}

const ALPHA: f64 = 0.3;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointInfo {
	/// health keeps track of the success rate for the endpoint.
	health: Ewma,
	/// request latency tracks the latency of requests
	request_latency: Ewma,
	/// pending_requests keeps track of the total number of pending requests.
	pending_requests: ActiveCounter,
	/// total_requests keeps track of the total number of requests.
	total_requests: AtomicU64,
	/// Number of consecutive unhealthy responses (reset to 0 on success).
	consecutive_failures: AtomicU64,
	/// Number of times this endpoint has been ejected. Used as a multiplier on
	/// the base ejection duration so repeatedly-failing hosts stay out longer.
	/// Reset to 0 when the endpoint handles a successful request.
	times_ejected: AtomicU64,
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
			consecutive_failures: Default::default(),
			times_ejected: Default::default(),
			evicted_until: Arc::new(Default::default()),
		}
	}
}

impl EndpointInfo {
	pub fn new() -> Self {
		Self::default()
	}
	/// Current health score (0.0–1.0) for threshold-based eviction.
	pub fn health_score(&self) -> f64 {
		self.health.load()
	}
	pub fn consecutive_failures(&self) -> u64 {
		self.consecutive_failures.load(AtomicOrdering::Relaxed)
	}
	pub fn times_ejected(&self) -> u64 {
		self.times_ejected.load(AtomicOrdering::Relaxed)
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
		self.total_requests.fetch_add(1, AtomicOrdering::Relaxed);
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
		self.0.load(AtomicOrdering::Relaxed)
	}
	/// Set the value directly (e.g. when unevicting to give the endpoint a recovery score).
	pub fn set(&self, value: f64) {
		self.0.store(value, AtomicOrdering::Relaxed);
	}
	pub fn record(&self, nv: f64) {
		let _ = self
			.0
			.fetch_update(AtomicOrdering::SeqCst, AtomicOrdering::Relaxed, |old| {
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
	/// Current endpoint health score (0.0–1.0) for eviction threshold checks.
	pub fn health_score(&self) -> f64 {
		self.info.health_score()
	}
	pub fn consecutive_failures(&self) -> u64 {
		self.info.consecutive_failures()
	}
	pub fn times_ejected(&self) -> u64 {
		self.info.times_ejected()
	}
	pub fn finish_request(
		self,
		success: bool,
		latency: Duration,
		eviction_time: Option<Duration>,
		restore_health: Option<f64>,
	) {
		if success {
			self.info.request_latency.record(latency.as_secs_f64());
			self.info.health.record(1.0);
			self
				.info
				.consecutive_failures
				.store(0, AtomicOrdering::Relaxed);
			self.info.times_ejected.store(0, AtomicOrdering::Relaxed);
		} else {
			self.info.health.record(0.0);
			self
				.info
				.consecutive_failures
				.fetch_add(1, AtomicOrdering::Relaxed);
		};
		if let Some(eviction_time) = eviction_time {
			self
				.info
				.times_ejected
				.fetch_add(1, AtomicOrdering::Relaxed);
			let time = Instant::now() + eviction_time;
			let prev = self
				.info
				.evicted_until
				.compare_and_swap(&None::<Arc<_>>, Some(Arc::new(time)));
			if prev.is_none() {
				let tx = self.tx.clone();
				let key = self.key.clone();
				tokio::spawn(async move {
					let _ = tx
						.send(EvictionEvent::Evict {
							key,
							until: time,
							restore_health,
						})
						.await;
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

/// Non-allocating iterator over every endpoint across all buckets. Yields all active
/// endpoints first (across every bucket), then all rejected.
///
/// Snapshot is per-bucket-per-phase, not whole-set: a concurrent rebucket or eviction
/// can be partially observed across loads. Callers that scan-then-discard (e.g. selection)
/// are unaffected; do not use for invariants that span buckets.
pub struct AllEndpointsIter<'a, T> {
	buckets: &'a [Atomic<EndpointGroup<T>>],
	bucket_idx: usize,
	current: Option<(Arc<EndpointGroup<T>>, usize)>,
	in_rejected: bool,
}

impl<T> Iterator for AllEndpointsIter<'_, T> {
	type Item = (Arc<T>, Arc<EndpointInfo>);

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			if let Some((group, idx)) = &mut self.current {
				let map = if self.in_rejected {
					&group.rejected
				} else {
					&group.active
				};
				if let Some((_, ewi)) = map.get_index(*idx) {
					*idx += 1;
					return Some((ewi.endpoint.clone(), ewi.info.clone()));
				}
				self.current = None;
			}
			if self.bucket_idx < self.buckets.len() {
				let bucket = &self.buckets[self.bucket_idx];
				self.bucket_idx += 1;
				self.current = Some((bucket.load_full(), 0));
				continue;
			}
			// Active phase exhausted across all buckets — restart for rejected phase.
			if !self.in_rejected {
				self.in_rejected = true;
				self.bucket_idx = 0;
				continue;
			}
			return None;
		}
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

#[cfg(test)]
mod tests {
	use super::*;

	// --- Ewma ---

	#[test]
	fn ewma_initial_value() {
		let ewma = Ewma::new(1.0);
		assert_eq!(ewma.load(), 1.0);
	}

	#[test]
	fn ewma_default_is_zero() {
		let ewma = Ewma::default();
		assert_eq!(ewma.load(), 0.0);
	}

	#[test]
	fn ewma_set_overwrites() {
		let ewma = Ewma::new(1.0);
		ewma.set(0.42);
		assert_eq!(ewma.load(), 0.42);
	}

	#[test]
	fn ewma_first_record_from_zero_sets_directly() {
		let ewma = Ewma::default(); // 0.0
		ewma.record(1.0);
		// When old==0.0, result is nv directly
		assert_eq!(ewma.load(), 1.0);
	}

	#[test]
	fn ewma_record_sequence_failures() {
		let ewma = Ewma::new(1.0);
		// ALPHA = 0.3, record(0.0) simulates a failure
		ewma.record(0.0); // 0.3*0 + 0.7*1.0 = 0.7
		assert!((ewma.load() - 0.7).abs() < 1e-10);
		ewma.record(0.0); // 0.3*0 + 0.7*0.7 = 0.49
		assert!((ewma.load() - 0.49).abs() < 1e-10);
		ewma.record(0.0); // 0.3*0 + 0.7*0.49 = 0.343
		assert!((ewma.load() - 0.343).abs() < 1e-10);
	}

	#[test]
	fn ewma_recovery_after_failures() {
		let ewma = Ewma::new(0.343);
		ewma.record(1.0); // 0.3*1.0 + 0.7*0.343 = 0.5401
		assert!((ewma.load() - 0.5401).abs() < 1e-10);
	}

	#[test]
	fn ewma_restore_health_full_reset() {
		let ewma = Ewma::new(1.0);
		// 3 failures: 1.0 → 0.7 → 0.49 → 0.343
		ewma.record(0.0);
		ewma.record(0.0);
		ewma.record(0.0);
		assert!((ewma.load() - 0.343).abs() < 1e-10);

		// Simulate uneviction with restoreHealth = 1.0
		ewma.set(1.0);
		assert_eq!(ewma.load(), 1.0);

		// Failures start fresh from 1.0
		ewma.record(0.0); // 0.7
		assert!((ewma.load() - 0.7).abs() < 1e-10);
		ewma.record(0.0); // 0.49
		assert!((ewma.load() - 0.49).abs() < 1e-10);
	}

	#[test]
	fn ewma_restore_health_zero() {
		let ewma = Ewma::new(1.0);
		ewma.record(0.0);
		ewma.record(0.0);
		ewma.record(0.0);

		// Simulate uneviction with restoreHealth = 0.0
		ewma.set(0.0);
		assert_eq!(ewma.load(), 0.0);

		// record(0.0) when old==0.0: result = nv = 0.0 (stays at zero)
		ewma.record(0.0);
		assert_eq!(ewma.load(), 0.0);
	}

	#[test]
	fn ewma_restore_health_partial() {
		let ewma = Ewma::new(1.0);
		ewma.record(0.0);
		ewma.record(0.0);
		ewma.record(0.0);

		// Simulate uneviction with restoreHealth = 0.5
		ewma.set(0.5);
		assert_eq!(ewma.load(), 0.5);

		// Next failure: 0.3*0 + 0.7*0.5 = 0.35
		ewma.record(0.0);
		assert!((ewma.load() - 0.35).abs() < 1e-10);
	}

	// --- EndpointInfo ---

	#[test]
	fn endpoint_info_default_health() {
		let info = EndpointInfo::default();
		assert_eq!(info.health_score(), 1.0);
		assert_eq!(info.consecutive_failures(), 0);
		assert_eq!(info.times_ejected(), 0);
	}

	// --- EndpointSet eviction integration ---

	#[tokio::test]
	async fn endpoint_set_eviction_and_uneviction() {
		let key: Strng = "ep1".into();
		let eps = EndpointSet::new(vec![vec![(key.clone(), "backend1")]]);

		// Endpoint is initially in the active set
		let group = eps.best_bucket();
		assert!(group.active.contains_key(&key));
		assert!(!group.rejected.contains_key(&key));

		// Start a request and finish with eviction
		let info = group.active.get(&key).unwrap().info.clone();
		let handle = eps.start_request(key.clone(), &info);
		handle.finish_request(
			false,
			Duration::from_millis(10),
			Some(Duration::from_millis(100)),
			Some(1.0),
		);

		// Poll until the eviction event is processed
		agent_core::test_helpers::check_eventually(
			Duration::from_millis(500),
			|| async { eps.best_bucket().rejected.contains_key(&key) },
			|rejected| *rejected,
		)
		.await
		.expect("endpoint should be evicted");

		// Wait for uneviction (100ms eviction duration + buffer)
		tokio::time::sleep(Duration::from_millis(150)).await;

		// Endpoint should be back in active with health reset to 1.0
		let group = eps.best_bucket();
		assert!(
			group.active.contains_key(&key),
			"endpoint should be unevicted"
		);
		let ep_info = &group.active.get(&key).unwrap().info;
		assert_eq!(ep_info.health_score(), 1.0, "health should be reset to 1.0");
	}

	#[tokio::test]
	async fn endpoint_set_uneviction_restore_health_zero() {
		let key: Strng = "ep1".into();
		let eps = EndpointSet::new(vec![vec![(key.clone(), "backend1")]]);

		let group = eps.best_bucket();
		let info = group.active.get(&key).unwrap().info.clone();
		let handle = eps.start_request(key.clone(), &info);
		handle.finish_request(
			false,
			Duration::from_millis(10),
			Some(Duration::from_millis(100)),
			Some(0.0),
		);

		tokio::time::sleep(Duration::from_millis(50)).await;
		let group = eps.best_bucket();
		assert!(group.rejected.contains_key(&key));

		// Wait for uneviction
		tokio::time::sleep(Duration::from_millis(150)).await;

		let group = eps.best_bucket();
		assert!(group.active.contains_key(&key));
		let ep_info = &group.active.get(&key).unwrap().info;
		assert_eq!(
			ep_info.health_score(),
			0.0,
			"health should be set to 0.0 on uneviction"
		);
	}

	#[tokio::test]
	async fn endpoint_set_uneviction_no_restore_health() {
		let key: Strng = "ep1".into();
		let eps = EndpointSet::new(vec![vec![(key.clone(), "backend1")]]);

		let group = eps.best_bucket();
		let info = group.active.get(&key).unwrap().info.clone();

		// Record a failure to lower health before eviction
		info.health.record(0.0); // 0.3*0 + 0.7*1.0 = 0.7

		let handle = eps.start_request(key.clone(), &info);
		handle.finish_request(
			false,
			Duration::from_millis(10),
			Some(Duration::from_millis(100)),
			None,
		);

		tokio::time::sleep(Duration::from_millis(50)).await;

		// Wait for uneviction
		tokio::time::sleep(Duration::from_millis(150)).await;

		let group = eps.best_bucket();
		assert!(group.active.contains_key(&key));
		let ep_info = &group.active.get(&key).unwrap().info;
		// Health was recorded as 0.0 in finish_request (failure),
		// starting from 0.7: 0.3*0 + 0.7*0.7 = 0.49
		assert!(
			(ep_info.health_score() - 0.49).abs() < 1e-10,
			"health should be unchanged from pre-eviction EWMA, got {}",
			ep_info.health_score()
		);
	}

	#[test]
	fn consecutive_failures_increments_on_failure() {
		let info = EndpointInfo::default();
		assert_eq!(info.consecutive_failures(), 0);

		info
			.consecutive_failures
			.fetch_add(1, AtomicOrdering::Relaxed);
		assert_eq!(info.consecutive_failures(), 1);

		info
			.consecutive_failures
			.fetch_add(1, AtomicOrdering::Relaxed);
		assert_eq!(info.consecutive_failures(), 2);
	}

	#[test]
	fn consecutive_failures_not_reset_by_uneviction() {
		let info = EndpointInfo::default();
		// Simulate 3 failures
		info.consecutive_failures.store(3, AtomicOrdering::Relaxed);
		// Simulate what uneviction does: only resets health, not consecutive_failures
		info.health.set(1.0);
		assert_eq!(
			info.consecutive_failures(),
			3,
			"consecutive_failures should NOT be reset on uneviction"
		);
	}

	// --- AllEndpointsIter ---

	fn build_group(
		active: &[&'static str],
		rejected: &[&'static str],
	) -> EndpointGroup<&'static str> {
		let mut group = EndpointGroup::default();
		for v in active {
			group.active.insert((*v).into(), EndpointWithInfo::new(*v));
		}
		for v in rejected {
			group
				.rejected
				.insert((*v).into(), EndpointWithInfo::new(*v));
		}
		group
	}

	fn install(eps: &EndpointSet<&'static str>, idx: usize, g: EndpointGroup<&'static str>) {
		eps.buckets[idx].store(Arc::new(g));
	}

	fn collect_values(eps: &EndpointSet<&'static str>) -> Vec<&'static str> {
		eps.all_endpoints().map(|(ep, _)| *ep).collect()
	}

	#[tokio::test]
	async fn all_endpoints_empty() {
		let eps = EndpointSet::<&'static str>::new_empty(2);
		assert!(collect_values(&eps).is_empty());
	}

	#[tokio::test]
	async fn all_endpoints_active_before_rejected_across_buckets() {
		let eps = EndpointSet::<&'static str>::new_empty(2);
		install(&eps, 0, build_group(&["a0"], &["r0"]));
		install(&eps, 1, build_group(&["a1"], &["r1"]));
		// All actives across buckets first, then all rejecteds.
		assert_eq!(collect_values(&eps), vec!["a0", "a1", "r0", "r1"]);
	}

	#[tokio::test]
	async fn all_endpoints_skips_empty_buckets_and_phases() {
		let eps = EndpointSet::<&'static str>::new_empty(3);
		install(&eps, 0, build_group(&["a0"], &[]));
		// bucket 1 left empty
		install(&eps, 2, build_group(&[], &["r2"]));
		assert_eq!(collect_values(&eps), vec!["a0", "r2"]);
	}

	// --- rebucket ---

	#[tokio::test]
	async fn rebucket_moves_endpoints_between_buckets() {
		// Start with endpoints "a" and "b" both in bucket 0.
		let eps = EndpointSet::<&'static str>::new_empty(2);
		install(&eps, 0, build_group(&["a", "b"], &[]));

		// Rebucket: send "a" to bucket 1, keep "b" in bucket 0.
		eps.rebucket(|endpoint| Some(if *endpoint == "a" { 1 } else { 0 }));

		let bucket_0 = eps.buckets[0].load_full();
		let bucket_1 = eps.buckets[1].load_full();

		assert_eq!(bucket_0.active.len(), 1, "bucket 0 should only contain b");
		assert!(bucket_0.active.contains_key(&Strng::from("b")));

		assert_eq!(bucket_1.active.len(), 1, "bucket 1 should only contain a");
		assert!(bucket_1.active.contains_key(&Strng::from("a")));
	}

	#[tokio::test]
	async fn rebucket_drops_none_and_out_of_range() {
		let eps = EndpointSet::<&'static str>::new_empty(2);
		install(&eps, 0, build_group(&["keep", "drop", "out_of_range"], &[]));

		eps.rebucket(|endpoint| match *endpoint {
			"keep" => Some(0),
			"drop" => None,
			// Callers should size buckets correctly; this guards against crashing
			// if an out-of-range index sneaks through.
			"out_of_range" => Some(99),
			_ => None,
		});

		assert_eq!(eps.buckets[0].load_full().active.len(), 1);
		assert_eq!(eps.buckets[1].load_full().active.len(), 0);
	}

	#[tokio::test]
	async fn rebucket_preserves_active_rejected_split() {
		let eps = EndpointSet::<&'static str>::new_empty(2);
		install(&eps, 0, build_group(&["a"], &["r"]));

		// Move everything to bucket 1.
		eps.rebucket(|_| Some(1));

		let bucket_0 = eps.buckets[0].load_full();
		let bucket_1 = eps.buckets[1].load_full();

		assert!(
			bucket_1.active.contains_key(&Strng::from("a")),
			"active stays active"
		);
		assert!(
			bucket_1.rejected.contains_key(&Strng::from("r")),
			"rejected stays rejected"
		);
		assert_eq!(bucket_0.active.len(), 0);
		assert_eq!(bucket_0.rejected.len(), 0);
	}

	#[tokio::test]
	async fn rebucket_preserves_endpoint_info_arc() {
		// Health/eviction state lives in EndpointInfo; rebucket must share the
		// same Arc rather than cloning the value.
		let eps = EndpointSet::<&'static str>::new_empty(2);
		install(&eps, 0, build_group(&["a"], &[]));

		let key = Strng::from("a");
		let info_before = Arc::as_ptr(&eps.buckets[0].load_full().active.get(&key).unwrap().info);

		eps.rebucket(|_| Some(1));

		let info_after = Arc::as_ptr(&eps.buckets[1].load_full().active.get(&key).unwrap().info);
		assert_eq!(info_before, info_after);
	}

	// --- LocalityRanker ---

	use crate::types::discovery::{
		LoadBalancer, LoadBalancerHealthPolicy, LoadBalancerMode, LoadBalancerScopes, Locality,
	};

	fn wl(network: &str, region: &str, zone: &str, node: &str, cluster: &str) -> Workload {
		Workload {
			network: network.into(),
			locality: Locality {
				region: region.into(),
				zone: zone.into(),
				subzone: "".into(),
			},
			node: node.into(),
			cluster_id: cluster.into(),
			..Default::default()
		}
	}

	fn lb(mode: LoadBalancerMode, prefs: Vec<LoadBalancerScopes>) -> LoadBalancer {
		LoadBalancer {
			routing_preferences: prefs,
			mode,
			health_policy: LoadBalancerHealthPolicy::default(),
		}
	}

	#[test]
	fn ranker_no_source_always_bucket_zero() {
		for mode in [
			LoadBalancerMode::Failover,
			LoadBalancerMode::Strict,
			LoadBalancerMode::Standard,
			LoadBalancerMode::Passthrough,
		] {
			let lbc = lb(mode, vec![LoadBalancerScopes::Zone]);
			let r = LocalityRanker::new(Some(&lbc), None);
			assert_eq!(r.bucket_for(&wl("n1", "r1", "z1", "_", "_")), Some(0));
		}
	}

	#[test]
	fn ranker_standard_and_passthrough_ignore_preferences() {
		// the control plane should not send preferences along with this mode, this is a defensive
		// check to ensure we ignore them if we do receive them in a non-bucketing mode
		let src = wl("n1", "r1", "z1", "_", "_");
		for mode in [LoadBalancerMode::Standard, LoadBalancerMode::Passthrough] {
			let lbc = lb(mode.clone(), vec![LoadBalancerScopes::Zone]);
			let r = LocalityRanker::new(Some(&lbc), Some(&src));
			assert_eq!(r.priority_levels(), 1, "mode {mode:?}");
			assert_eq!(
				r.bucket_for(&wl("n1", "r1", "z1", "_", "_")),
				Some(0),
				"mode {mode:?} matching endpoint"
			);
			assert_eq!(
				r.bucket_for(&wl("n1", "r1", "z9", "_", "_")),
				Some(0),
				"mode {mode:?} non-matching endpoint"
			);
		}
	}

	#[test]
	fn ranker_prefix_match_counts() {
		let src = wl("n1", "r1", "z1", "node1", "c1");
		let lbc = lb(
			LoadBalancerMode::Failover,
			vec![
				LoadBalancerScopes::Network,
				LoadBalancerScopes::Region,
				LoadBalancerScopes::Zone,
			],
		);
		let r = LocalityRanker::new(Some(&lbc), Some(&src));
		// full match -> 3
		assert_eq!(r.rank(&wl("n1", "r1", "z1", "_", "_")), Some(3));
		// miss on zone -> 2
		assert_eq!(r.rank(&wl("n1", "r1", "z2", "_", "_")), Some(2));
		// miss on region breaks the chain before zone is evaluated
		assert_eq!(r.rank(&wl("n1", "r2", "z1", "_", "_")), Some(1));
		// miss on first preference -> 0
		assert_eq!(r.rank(&wl("n2", "r1", "z1", "_", "_")), Some(0));
	}

	#[test]
	fn ranker_strict_drops_sub_full_match() {
		let src = wl("n1", "r1", "z1", "_", "_");
		let lbc = lb(
			LoadBalancerMode::Strict,
			vec![LoadBalancerScopes::Network, LoadBalancerScopes::Zone],
		);
		let r = LocalityRanker::new(Some(&lbc), Some(&src));
		assert_eq!(r.rank(&wl("n1", "_", "z1", "_", "_")), Some(2));
		assert_eq!(r.rank(&wl("n1", "_", "z2", "_", "_")), None);
		assert_eq!(r.rank(&wl("n2", "_", "z1", "_", "_")), None);
	}

	#[test]
	fn ranker_node_and_cluster_scopes() {
		let src = wl("_", "_", "_", "nodeA", "clusterA");
		let lbc = lb(
			LoadBalancerMode::Failover,
			vec![LoadBalancerScopes::Cluster, LoadBalancerScopes::Node],
		);
		let r = LocalityRanker::new(Some(&lbc), Some(&src));
		assert_eq!(r.rank(&wl("_", "_", "_", "nodeA", "clusterA")), Some(2));
		assert_eq!(r.rank(&wl("_", "_", "_", "nodeB", "clusterA")), Some(1));
		assert_eq!(r.rank(&wl("_", "_", "_", "nodeA", "clusterB")), Some(0));
	}
}
