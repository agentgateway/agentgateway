use crate::*;
use agent_core::responsechannel;
use indexmap::IndexMap;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::time::sleep_until;

type EndpointKey = Strng;

#[derive(Debug, Clone, Default, Serialize)]
pub struct EndpointWithInfo<T> {
	pub endpoint: Arc<T>,
	pub info: Arc<EndpointInfo>,
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

pub struct EndpointSet<T> {
	bucket: Atomic<EndpointGroup<T>>,
	tx: responsechannel::AckSender<EndpointEvent<T>>,
}

pub enum EndpointEvent<T> {
	Add(EndpointKey, EndpointWithInfo<T>),
	Delete(EndpointKey),
	Evict(EndpointKey, Instant),
}
impl<T: Clone + Sync + Send + 'static> Default for EndpointSet<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Clone + Sync + Send + 'static> EndpointSet<T> {
	pub fn new() -> Self {
		let (tx, rx) = responsechannel::new(10);
		let bucket: Atomic<EndpointGroup<T>> = Default::default();
		Self::worker(rx, bucket.clone());
		Self { bucket, tx }
	}
	fn worker(
		mut events: responsechannel::AckReceiver<EndpointEvent<T>>,
		bucket: Atomic<EndpointGroup<T>>,
	) {
		tokio::task::spawn(async move {
			let mut uneviction_heap: BinaryHeap<(Instant, EndpointKey)> = Default::default();
			let handle_eviction = |uneviction_heap: &mut BinaryHeap<(Instant, EndpointKey)>| {
				let (_, key) = uneviction_heap.pop().expect("heap is empty");

				let mut eps = Arc::unwrap_or_clone(bucket.load_full());
				if let Some(ep) = eps.rejected.swap_remove(&key) {
					ep.info.evicted_until.store(None);
					eps.active.insert(key, ep);
				}
			};
			let handle_recv =
				|uneviction_heap: &mut BinaryHeap<(Instant, EndpointKey)>,
				 o: Option<(EndpointEvent<T>, tokio::sync::oneshot::Sender<()>)>| {
					let Some((item, resp)) = o else {
						return;
					};

					let mut eps = Arc::unwrap_or_clone(bucket.load_full());
					match item {
						EndpointEvent::Add(key, ep) => {
							eps.rejected.swap_remove(&key);
							eps.active.insert(key, ep);
						},
						EndpointEvent::Delete(key) => {
							eps.active.swap_remove(&key);
							eps.rejected.swap_remove(&key);
						},
						EndpointEvent::Evict(key, timer) => {
							uneviction_heap.push((timer, key.clone()));
							if let Some(ep) = eps.active.swap_remove(&key) {
								eps.rejected.insert(key, ep);
							}
						},
					}
					bucket.store(Arc::new(eps));
					let _ = resp.send(());
				};
			loop {
				let evict_at = uneviction_heap.peek().map(|x| x.0);
				tokio::select! {
					true = maybe_sleep_until(evict_at) => handle_eviction(&mut uneviction_heap),
					item = events.recv() => handle_recv(&mut uneviction_heap, item)
				}
			}
		});
	}
	pub async fn evict(&mut self, key: EndpointKey, time: Instant) {
		if let Some(cur) = self.bucket.load_full().active.get(&key) {
			// Immediately store in the endpoint the eviction time, if its not already been evicted
			let prev = cur
				.info
				.evicted_until
				.compare_and_swap(&None::<Arc<_>>, Some(Arc::new(time)));
			if prev.is_none() {
				let tx = self.tx.clone();
				// If we were the one to evict it, trigger the real eviction async
				tokio::spawn(async move {
					let _ = tx.send_ignore(EndpointEvent::Evict(key, time)).await;
				});
			}
		}
	}
}

const ALPHA: f64 = 0.3;

#[derive(Debug, Default, Serialize)]
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

impl EndpointInfo {
	pub fn new() -> Self {
		Self::default()
	}
	pub fn start_request(self: &Arc<Self>) -> ActiveHandle {
		self.total_requests.fetch_add(1, Ordering::Relaxed);
		// self.evicted_until.store(Some(Arc::new(Instant::now() + Duration::from_secs(10))));
		ActiveHandle(self.clone(), self.pending_requests.0.clone())
	}
}

#[derive(Debug, Default, Serialize)]
pub struct Ewma(atomic_float::AtomicF64);

impl Ewma {
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

#[derive(Clone, Debug, Default)]
pub struct ActiveHandle(Arc<EndpointInfo>, #[allow(dead_code)] Arc<()>);

impl ActiveHandle {
	pub fn finish_request(self, success: bool, latency: Duration, eviction_time: Option<Duration>) {
		if success {
			self.0.request_latency.record(latency.as_secs_f64());
			self.0.health.record(1.0);
		} else {
			// Do not record request_latency on failure; its common for failures to be fast and skew results.
			self.0.health.record(0.0)
		};
		if let Some(eviction_time) = eviction_time {
			self.0.evicted_until.store(Some(Arc::new(Instant::now() + eviction_time)));
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
