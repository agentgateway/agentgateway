use hashbrown::HashMap;
use hashbrown::hash_map::EntryRef;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::error::Error as StdError;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use crate::client::RequestBody;
use crate::common::exec;
use crate::common::exec::Exec;
use crate::common::timer::Timer;
use crate::connect::Connected;
use crate::pool;
use futures_channel::oneshot;
use futures_core::ready;
use futures_util::future::Either;
use http::{Request, Response};
use hyper::rt::{Sleep, Timer as _};
use tracing::{debug, trace};

// per https://datatracker.ietf.org/doc/html/rfc9113#section-6.5.2-2.6.1, servers are expected to
// but not required to set this to at least 100. In practice, a wide range of clients will have failures
// with settings less than this, so it seems a safe default.
pub const DEFAULT_EXPECTED_HTTP2_CAPACITY: usize = 100;

// Fairly arbitrary number; we may want to explore tuning this.
const SHARD_COUNT: usize = 16;

type HostMap<K> = HashMap<K, HostPool<K>>;

/// Shard the host map to avoid one mega-mutex contention.
/// While under extremely high concurrency, this shows ~20% improvement in throughput and is pretty low cost.
/// Note: a key is deterministically in a single shard; this does not increase the number of outbound connections.
#[derive(Debug)]
struct HostShards<K: Key> {
	shards: [Mutex<HostMap<K>>; SHARD_COUNT],
}

impl<K: Key> HostShards<K> {
	fn new() -> Self {
		Self {
			shards: std::array::from_fn(|_| Mutex::new(HashMap::new())),
		}
	}

	fn shard_index(key: &K) -> usize {
		key.shard() % SHARD_COUNT
	}

	fn lock_shard(&self, key: &K) -> MutexGuard<'_, HostMap<K>> {
		self.shards[Self::shard_index(key)].lock()
	}

	fn lock_host(&self, key: &K) -> MappedMutexGuard<'_, HostPool<K>> {
		MutexGuard::map(self.lock_shard(key), |hosts| match hosts.entry_ref(key) {
			EntryRef::Occupied(entry) => entry.into_mut(),
			EntryRef::Vacant(entry) => {
				entry.insert_with_key(key.clone(), HostPool::new(key.expected_capacity()))
			},
		})
	}

	fn clear_expired(&self, settings: &PoolSettings) {
		let now = settings.timer.now();
		for shard in &self.shards {
			let mut hosts = shard.lock();
			Pool::<K>::clear_expired(settings, now, &mut hosts);
		}
	}
}

/// Pool is a connection pool for a set of hosts.
/// Each host shares the same top level settings, and individual per-K entries maintain state for
/// each host under mutex.
#[derive(Clone, Debug)]
pub struct Pool<K: Key> {
	hosts: Arc<HostShards<K>>,
	pub settings: Arc<PoolSettings>,
}

#[derive(Debug)]
pub struct PoolSettings {
	max_idle_per_host: usize,
	idle_interval_spawned: AtomicBool,
	exec: Exec,
	timer: Timer,
	timeout: Option<Duration>,
	pub expected_http2_capacity: usize,
}

impl<K: Key> Pool<K> {
	/// This should *only* be called by the IdleTask
	fn clear_expired(settings: &PoolSettings, now: Instant, hosts: &mut HostMap<K>) {
		let dur = settings.timeout.expect("interval assumes timeout");

		hosts.retain(|key, host| {
			host.idle.retain(|entry| {
				if !entry.value.is_open() {
					trace!("idle interval evicting closed for {:?}", key);
					return false;
				}

				// Avoid `Instant::sub` to avoid issues like rust-lang/rust#86470.
				if now.saturating_duration_since(entry.idle_at) > dur {
					trace!("idle interval evicting expired for {:?}", key);
					return false;
				}

				// Otherwise, keep this value...
				true
			});
			let empty = host.idle.is_empty()
				&& host.active_h2.0.is_empty()
				&& host.connecting == 0
				&& host.waiters.is_empty();
			!empty
		});
	}
	fn lock_hosts<'a>(hosts: &'a HostShards<K>, k: &K) -> MappedMutexGuard<'a, HostPool<K>> {
		hosts.lock_host(k)
	}
	fn host(&self, k: &K) -> MappedMutexGuard<'_, HostPool<K>> {
		Pool::<K>::lock_hosts(self.hosts.as_ref(), k)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpectedCapacity {
	// Always HTTP1: only a single concurrent request is allowed.
	Http1,
	// Always HTTP2: multiple concurrent requests are allowed.
	Http2,
	// HTTP/1 or HTTP/2, depending on the connection (ALPN)
	Auto,
}

pub trait Key: Eq + Hash + Clone + Debug + Unpin + Send + Sync + 'static {
	fn expected_capacity(&self) -> ExpectedCapacity;

	fn shard(&self) -> usize {
		let mut hasher = DefaultHasher::new();
		self.hash(&mut hasher);
		hasher.finish() as usize
	}
}

#[derive(Debug)]
enum CapacityCache {
	// Based on the request properties, what we expect the capacity will be
	Guess(ExpectedCapacity),
	// Based on historical requests, what we expect the capacity will be.
	#[allow(dead_code)]
	Cached(usize),
}

impl CapacityCache {
	fn expected_capacity(&self, expected_http2_capacity: usize) -> usize {
		match self {
			CapacityCache::Guess(ExpectedCapacity::Http1) => 1,
			CapacityCache::Guess(ExpectedCapacity::Http2) => expected_http2_capacity,
			// Assume we are going to get HTTP2; this ensures we don't flood with connections for HTTP/1.1
			// If we don't get it, we will just try again with the new expected value cached.
			CapacityCache::Guess(ExpectedCapacity::Auto) => expected_http2_capacity,
			CapacityCache::Cached(exact) => *exact,
		}
	}
}

#[derive(Default)]
struct H2Pool(VecDeque<ReservedHttp2Connection>);

impl Debug for H2Pool {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let active: Vec<_> = self
			.0
			.iter()
			.map(|h| h.load.active_streams.load(Ordering::Acquire))
			.collect();
		write!(f, "H2Pool({active:?})")
	}
}

impl H2Pool {
	pub fn ensure_tracked(&mut self, c: &ReservedHttp2Connection) {
		if !self.0.iter().any(|entry| Arc::ptr_eq(&entry.load, &c.load)) {
			let cpy = ReservedHttp2Connection {
				info: c.info.clone(),
				tx: c.tx.clone(),
				load: c.load.clone(),
			};
			if c.load.remaining_capacity() == 0 {
				self.0.push_back(cpy)
			} else {
				self.0.push_front(cpy)
			}
		}
	}
	/// increment_load attempts to increment the load on a connection if it is HTTP2.
	/// This automatically manages tracking of position
	pub fn increment_load(&mut self, c: &HttpConnection) -> Option<CapacityReservationResult> {
		match c {
			HttpConnection::Http1(_) => None,
			HttpConnection::Http2(h2) => {
				let res = h2.load.try_reserve_stream_slot();
				if res == CapacityReservationResult::ReservedAndFilled {
					self.mark_full(h2);
				}
				Some(res)
			},
		}
	}
	pub fn mark_full(&mut self, c: &ReservedHttp2Connection) {
		if let Some(old) = self.remove(c) {
			// Push to the back of the queue
			self.0.push_back(old)
		}
	}
	pub fn remove_by_load(&mut self, rc: &Arc<H2Load>) -> Option<ReservedHttp2Connection> {
		let pos = self
			.0
			.iter()
			.position(|entry| Arc::ptr_eq(&entry.load, rc))?;
		self.0.remove(pos)
	}
	pub fn remove(&mut self, rc: &ReservedHttp2Connection) -> Option<ReservedHttp2Connection> {
		let pos = self
			.0
			.iter()
			.position(|entry| Arc::ptr_eq(&entry.load, &rc.load))?;
		self.0.remove(pos)
	}
	fn mark_active_by_load(&mut self, c: &Arc<H2Load>) {
		if let Some(v) = self.remove_by_load(c) {
			// Push to the front of the queue; it will be the next connection to get used.
			self.0.push_front(v);
		}
	}
	fn mark_active(&mut self, c: ReservedHttp2Connection) {
		self.remove(&c);
		// Push to the front of the queue; it will be the next connection to get used.
		self.0.push_front(c);
	}
	/// maybe_insert_new inserts the connection as an active one (if it is HTTP2).
	fn maybe_insert_new(&mut self, conn: HttpConnection, reserve: bool) -> HttpConnection {
		if let HttpConnection::Http2(h) = conn {
			self.0.push_front(h.clone_without_load_incremented());
			if reserve {
				debug_assert!(
					h.load.try_reserve_stream_slot() != CapacityReservationResult::NoCapacity,
					"a new stream should always be able to be reserved"
				);
			}
			HttpConnection::Http2(ReservedHttp2Connection {
				info: h.info,
				tx: h.tx,
				load: h.load,
			})
		} else {
			conn
		}
	}
	fn reserve(&mut self) -> Option<ReservedHttp2Connection> {
		while let Some(h) = self.0.front() {
			if !h.tx.is_ready() {
				// Connection is dead... remove it.
				let _ = self.0.pop_front();
				debug!("removing dead http2 connection");
				continue;
			}
			match h.load.try_reserve_stream_slot() {
				CapacityReservationResult::NoCapacity => {
					// We know the front is the one that was most recently returned, thus must be available
					return None;
				},
				CapacityReservationResult::ReservedAndFilled => {
					let ret = Some(ReservedHttp2Connection {
						info: h.info.clone(),
						tx: h.tx.clone(),
						load: h.load.clone(),
					});
					// Move the connection to the back of the queue.
					if let Some(v) = self.0.pop_front() {
						self.0.push_back(v);
					}
					return ret;
				},
				CapacityReservationResult::ReservedButNotFilled => {
					// Keep the connection at the front.
					return Some(ReservedHttp2Connection {
						info: h.info.clone(),
						tx: h.tx.clone(),
						load: h.load.clone(),
					});
				},
			}
		}
		None
	}
}

pub(crate) struct ReservedHttp1Connection {
	pub(crate) info: Connected,
	pub(crate) tx: hyper::client::conn::http1::SendRequest<RequestBody>,
}

pub(crate) enum HttpConnection {
	Http1(ReservedHttp1Connection),
	Http2(ReservedHttp2Connection),
}

impl fmt::Debug for HttpConnection {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let v = match self {
			HttpConnection::Http1(_) => "http1",
			HttpConnection::Http2(_) => "http2",
		};
		f.debug_struct("HttpConnection")
			.field("version", &v)
			.finish()
	}
}

impl HttpConnection {
	pub fn maybe_clone(&self) -> Option<Self> {
		match self {
			HttpConnection::Http1(_) => None,
			HttpConnection::Http2(inner) => Some(HttpConnection::Http2(
				inner.clone_without_load_incremented(),
			)),
		}
	}
	pub fn capacity(&self) -> usize {
		match self {
			HttpConnection::Http1(_) => 1,
			HttpConnection::Http2(h) => h.load.remaining_capacity(),
		}
	}
	pub fn try_send_request(
		&mut self,
		req: Request<RequestBody>,
	) -> impl Future<
		Output = Result<
			Response<hyper::body::Incoming>,
			hyper::client::conn::TrySendError<Request<RequestBody>>,
		>,
	> {
		match self {
			HttpConnection::Http1(h) => Either::Left(h.tx.try_send_request(req)),
			HttpConnection::Http2(h) => Either::Right(h.tx.try_send_request(req)),
		}
	}
	pub fn conn_info(&self) -> &Connected {
		match self {
			HttpConnection::Http1(h) => &h.info,
			HttpConnection::Http2(h) => &h.info,
		}
	}
	pub fn is_open(&self) -> bool {
		match self {
			HttpConnection::Http1(h1) => h1.tx.is_ready(),
			HttpConnection::Http2(h2) => h2.tx.is_ready(),
		}
	}
}

#[derive(Debug)]
pub struct H2CapacityGuard<K: Key> {
	value: Option<(K, Arc<H2Load>)>,
	pool: Weak<HostShards<K>>,
	settings: Arc<PoolSettings>,
}

pub(crate) struct ReservedHttp2Connection {
	pub(crate) info: Connected,
	pub(crate) tx: hyper::client::conn::http2::SendRequest<RequestBody>,
	pub(crate) load: Arc<H2Load>,
}

impl ReservedHttp2Connection {
	fn clone_without_load_incremented(&self) -> Self {
		Self {
			info: self.info.clone(),
			tx: self.tx.clone(),
			load: self.load.clone(),
		}
	}
}

#[derive(Debug)]
pub(crate) struct H2Load {
	active_streams: AtomicUsize,
	max_streams: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CapacityReservationResult {
	NoCapacity,
	ReservedAndFilled,
	ReservedButNotFilled,
}

impl H2Load {
	pub(crate) fn new(max_streams: usize) -> Self {
		Self {
			active_streams: AtomicUsize::new(0),
			max_streams: AtomicUsize::new(max_streams.max(1)),
		}
	}

	fn remaining_capacity(&self) -> usize {
		self.max_streams.load(Ordering::Acquire) - self.active_streams.load(Ordering::Acquire)
	}
	fn try_reserve_stream_slot(&self) -> CapacityReservationResult {
		let max = self.max_streams.load(Ordering::Acquire);
		let prev = self
			.active_streams
			.fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
				if active < max { Some(active + 1) } else { None }
			});

		match prev {
			Err(_) => CapacityReservationResult::NoCapacity,
			Ok(prev_val) => {
				if prev_val + 1 >= max {
					CapacityReservationResult::ReservedAndFilled
				} else {
					CapacityReservationResult::ReservedButNotFilled
				}
			},
		}
	}

	fn release_stream_slot(&self) -> (usize, bool) {
		let prev = self.active_streams.fetch_sub(1, Ordering::AcqRel);
		let max = self.max_streams.load(Ordering::Acquire);
		debug_assert!(prev > 0, "active_streams must be > 0 before release");
		(prev - 1, prev == max)
	}
}

// HostPool stores information for a single host.
#[derive(Debug)]
struct HostPool<K: Key> {
	// The number of currently establishing connections
	connecting: usize,
	// The expected number of requests the `connecting` connections are estimated to handle.
	expected_connecting_capacity: usize,
	// Expected capacity
	per_connection_capacity_cache: CapacityCache,
	// These are internal Conns sitting in the event loop in the KeepAlive
	// state, waiting to receive a new Request to send on the socket.
	idle: Vec<Idle>,
	// Active h2 connections. These are stored (unlike http/1.1) as active connections may be used.
	// Busy items are pushed to the backend of the queue, while free items are in the front.
	// If the first item is busy, that implies all items are busy; grabbing a free connection never requires
	// a search.
	active_h2: H2Pool,
	// These are outstanding Checkouts that are waiting for a socket to be
	// able to send a Request one. This is used when "racing" for a new
	// connection.
	//
	// The Client starts 2 tasks, 1 to connect a new socket, and 1 to wait
	// for the Pool to receive an idle Conn. When a Conn becomes idle,
	// this list is checked for any parked Checkouts, and tries to notify
	// them that the Conn could be used instead of waiting for a brand new
	// connection.
	waiters: VecDeque<oneshot::Sender<Result<Pooled<K>, ClientConnectError>>>,
}

impl<K: Key> HostPool<K> {
	fn new(capacity: ExpectedCapacity) -> HostPool<K> {
		Self {
			connecting: 0,
			expected_connecting_capacity: 0,
			per_connection_capacity_cache: CapacityCache::Guess(capacity),
			idle: Vec::new(),
			active_h2: H2Pool::default(),
			waiters: Default::default(),
		}
	}
	fn return_h2_stream(
		&mut self,
		settings: Arc<PoolSettings>,
		pool: Arc<HostShards<K>>,
		k: K,
		load: Arc<H2Load>,
	) {
		let (remaining, was_at_max) = load.release_stream_slot();
		if remaining == 0 {
			if let Some(v) = self.active_h2.remove_by_load(&load) {
				if v.tx.is_ready() {
					self.return_idle(settings, pool, k, HttpConnection::Http2(v))
				} else {
					trace!("skip moving h2 connection to idle; not open")
				}
			}
		} else if was_at_max {
			self.active_h2.mark_active_by_load(&load);
		}
	}
	fn release_h2_stream_without_returning_to_idle(&mut self, s: &HttpConnection) {
		let HttpConnection::Http2(h2) = s else {
			return;
		};
		let (remaining, was_at_max) = h2.load.release_stream_slot();
		if remaining == 0 {
			// Do NOT remove it here; this is the caller responsibility
			// This ensures we don't end up dropping it if we need to rollback.
		} else if was_at_max {
			self.active_h2.mark_active_by_load(&h2.load);
		}
	}
	fn return_connection(
		&mut self,
		settings: Arc<PoolSettings>,
		pool: Arc<HostShards<K>>,
		k: K,
		value: HttpConnection,
	) {
		match value {
			HttpConnection::Http1(h) => self.return_idle(settings, pool, k, HttpConnection::Http1(h)),
			HttpConnection::Http2(h) => {
				let (remaining, was_at_max) = h.load.release_stream_slot();
				if remaining == 0 {
					self.active_h2.remove(&h);
					self.return_idle(settings, pool, k, HttpConnection::Http2(h))
				} else if was_at_max {
					self.active_h2.mark_active(h);
				}
			},
		}
	}
	fn return_dead_connection(&mut self, value: HttpConnection) {
		match value {
			HttpConnection::Http1(_) => {
				// Just drop it
			},
			HttpConnection::Http2(h) => {
				// Even if it has capacity its dead now; do not track it
				self.active_h2.remove(&h);
			},
		}
	}
	pub fn forget_pending_connection(
		&mut self,
		key: K,
		capacity: usize,
		mut err: Option<crate::Error>,
		for_under_capacity_new_connection: bool,
	) {
		if !for_under_capacity_new_connection {
			// for_under_capacity_new_connection means we got a connection, it just was too small
			self.connecting -= 1;
		}
		self.expected_connecting_capacity -= capacity;

		let mut to_notify = capacity;
		if !for_under_capacity_new_connection {
			to_notify = to_notify.saturating_sub(1);
			// For the first, notify with the original error. The rest get an error to just retry.
			loop {
				let Some(tx) = self.waiters.pop_front() else {
					break;
				};
				if tx.is_canceled() {
					trace!("insert new error; removing canceled waiter for {:?}", key);
					continue;
				}
				#[cfg(test)]
				Pool::<K>::run_forget_pending_test_hook();
				let res = if let Some(e) = err.take() {
					tx.send(Err(ClientConnectError::Normal(e)))
				} else {
					tx.send(Err(ClientConnectError::CheckoutIsClosed(
						pool::Error::ConnectionDroppedWithoutCompletion,
					)))
				};
				if let Err(Err(ClientConnectError::Normal(e))) = res {
					err = Some(e);
					continue;
				}

				break;
			}
		}

		while to_notify > 0 {
			let Some(tx) = self.waiters.pop_front() else {
				break;
			};
			if tx.is_canceled() {
				trace!("insert new error; removing canceled waiter for {:?}", key);
				continue;
			}
			to_notify -= 1;
			let e = if for_under_capacity_new_connection {
				pool::Error::ConnectionLowCapacity
			} else {
				pool::Error::WaitingOnSharedFailedConnection
			};
			let _ = tx.send(Err(ClientConnectError::CheckoutIsClosed(e)));
		}
	}
	pub fn return_idle(
		&mut self,
		settings: Arc<PoolSettings>,
		pool: Arc<HostShards<K>>,
		key: K,
		conn: HttpConnection,
	) {
		trace!(waiters=%self.waiters.len(), "return idle");
		// we are returning, so there should only ever been 1 additional spot free
		let capacity = 1;
		Pool::send_connection("idle", key, capacity, self, &pool, &settings, conn);
	}

	fn push_idle_with_cap(
		&mut self,
		max_idle_per_host: usize,
		key: K,
		value: HttpConnection,
		idle_at: Instant,
	) {
		if max_idle_per_host == 0 {
			debug!(
				"dropping idle connection for {:?}; max_idle_per_host=0",
				key
			);
			return;
		}
		if self.idle.len() >= max_idle_per_host {
			debug!(
				"evicting oldest idle connection for {:?}; max_idle_per_host reached",
				key
			);
			let _ = self.idle.remove(0);
		}
		debug!("pooling idle connection for {:?}", key);
		self.idle.push(Idle { value, idle_at });
	}
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
	pub idle_timeout: Option<Duration>,
	pub max_idle_per_host: usize,
	pub expected_http2_capacity: usize,
}

impl<K: Key> Pool<K> {
	pub fn new<E, M>(config: Config, executor: E, timer: M) -> Pool<K>
	where
		E: hyper::rt::Executor<exec::BoxSendFuture> + Send + Sync + Clone + 'static,
		M: hyper::rt::Timer + Send + Sync + Clone + 'static,
	{
		let exec = Exec::new(executor);
		let timer = Timer::new(timer);

		Pool {
			hosts: Arc::new(HostShards::new()),
			settings: Arc::new(PoolSettings {
				idle_interval_spawned: AtomicBool::new(false),
				max_idle_per_host: config.max_idle_per_host,
				exec,
				timer,
				timeout: config.idle_timeout,
				expected_http2_capacity: config.expected_http2_capacity,
			}),
		}
	}
}

#[derive(Debug)]
pub(crate) struct WaitForConnection<K: Key> {
	pub should_connect: Option<ShouldConnect<K>>,
	pub waiter: oneshot::Receiver<Result<Pooled<K>, ClientConnectError>>,
}

#[derive(Debug)]
struct ShouldConnectInner<K: Key> {
	expected_capacity: usize,
	key: K,
	pool: Weak<HostShards<K>>,
}

#[derive(Debug)]
pub(crate) struct ShouldConnect<K: Key> {
	inner: Option<ShouldConnectInner<K>>,
}

impl<K: Key> Drop for ShouldConnect<K> {
	fn drop(&mut self) {
		let Some(inner) = self.inner.take() else {
			return;
		};
		if let Some(pool) = inner.pool.upgrade() {
			let mut hosts = Pool::lock_hosts(pool.as_ref(), &inner.key);
			hosts.forget_pending_connection(inner.key, inner.expected_capacity, None, false);
		}
	}
}

#[derive(Debug)]
pub(crate) enum CheckoutResult<K: Key> {
	Checkout(Pooled<K>),
	Wait(WaitForConnection<K>),
}

impl<K: Key> Pool<K> {
	#[cfg(test)]
	fn send_connection_test_hook() -> &'static Mutex<Option<Box<dyn FnMut() -> bool + Send>>> {
		static HOOK: Mutex<Option<Box<dyn FnMut() -> bool + Send>>> = Mutex::new(None);
		&HOOK
	}

	#[cfg(test)]
	fn run_send_connection_test_hook() {
		let mut hook = Self::send_connection_test_hook().lock();
		if let Some(hook_fn) = hook.as_mut()
			&& !hook_fn()
		{
			*hook = None;
		}
	}

	#[cfg(test)]
	fn forget_pending_test_hook() -> &'static Mutex<Option<Box<dyn FnMut() -> bool + Send>>> {
		static HOOK: Mutex<Option<Box<dyn FnMut() -> bool + Send>>> = Mutex::new(None);
		&HOOK
	}

	#[cfg(test)]
	fn run_forget_pending_test_hook() {
		let mut hook = Self::forget_pending_test_hook().lock();
		if let Some(hook_fn) = hook.as_mut()
			&& !hook_fn()
		{
			*hook = None;
		}
	}

	pub(crate) fn insert_new_connection_error(
		&self,
		mut should_connect: ShouldConnect<K>,
		err: crate::Error,
	) {
		let ShouldConnectInner {
			expected_capacity,
			key,
			..
		} = should_connect
			.inner
			.take()
			.expect("insert_new_connection requires an active should_connect token");
		let mut host = self.host(&key);
		host.forget_pending_connection(key, expected_capacity, Some(err), false)
	}
	pub(crate) fn insert_new_connection(
		&self,
		mut should_connect: ShouldConnect<K>,
		conn: HttpConnection,
	) {
		let ShouldConnectInner {
			expected_capacity,
			key,
			..
		} = should_connect
			.inner
			.take()
			.expect("insert_new_connection requires an active should_connect token");
		let mut host = self.host(&key);
		// Do not drop again as we explicitly inserted
		let capacity = conn.capacity();
		host.connecting -= 1;
		// Min of capacity and expected to handle the over-capacity case.
		// For under capacity, we handle it below in forget_pending_connection
		host.expected_connecting_capacity -= std::cmp::min(capacity, expected_capacity);
		trace!(?key, ?host.connecting, %host.expected_connecting_capacity, "inserting new connection");

		let conn = host.active_h2.maybe_insert_new(conn, false);
		trace!(waiters=%host.waiters.len(), "insert new");
		// First, send to any waiters...
		Pool::send_connection(
			"new",
			key.clone(),
			capacity,
			&mut host,
			&self.hosts,
			&self.settings,
			conn,
		);

		// If we had expected this to have more capacity, we need to notify any waiters that its not going to
		// arrive...
		if capacity < expected_capacity {
			trace!(
				"handle capacity mismatch: expected {} but got {} ",
				expected_capacity, capacity
			);
			let excess = expected_capacity - capacity;
			host.per_connection_capacity_cache = CapacityCache::Cached(capacity);
			host.forget_pending_connection(key, excess, None, true);
		}
	}

	fn ensure_idle_interval(pool: &Arc<HostShards<K>>, settings: &Arc<PoolSettings>) {
		let Some(duration) = settings.timeout else {
			return;
		};
		if settings.idle_interval_spawned.swap(true, Ordering::AcqRel) {
			return;
		}

		let timer = settings.timer.clone();
		let interval = IdleTask {
			timer: timer.clone(),
			duration,
			deadline: Instant::now(),
			fut: timer.sleep_until(Instant::now()), // ready at first tick
			pool: Arc::downgrade(pool),
			settings: settings.clone(),
		};

		settings.exec.execute(interval);
	}

	fn send_connection(
		reason: &str,
		key: K,
		mut capacity: usize,
		host: &mut HostPool<K>,
		pool: &Arc<HostShards<K>>,
		settings: &Arc<PoolSettings>,
		original_con: HttpConnection,
	) {
		let mut next_conn = Some(original_con);
		let mut sent = 0;
		while capacity > 0 {
			let Some(tx) = host.waiters.pop_front() else {
				break;
			};
			if tx.is_canceled() {
				trace!("insert new; removing canceled waiter for {:?}", key);
				continue;
			}

			let Some(this_conn) = next_conn.take() else {
				break;
			};
			next_conn = this_conn.maybe_clone();

			if let Some(CapacityReservationResult::NoCapacity) = host.active_h2.increment_load(&this_conn)
			{
				break;
			}
			let pooled = Pooled {
				value: Some((key.clone(), this_conn)),
				is_reused: reason == "idle",
				pool: Arc::downgrade(pool),
				settings: settings.clone(),
			};
			#[cfg(test)]
			Self::run_send_connection_test_hook();
			match tx.send(Ok(pooled)) {
				Ok(()) => {
					capacity -= 1;
					sent += 1;
					if let Some(HttpConnection::Http2(h2)) = &next_conn {
						// We need to make sure its actively tracked; if we hit this from the idle flow it may have been removed.
						host.active_h2.ensure_tracked(h2);
					}
				},
				Err(Ok(mut e)) => {
					trace!("send failed");
					// Recover the connection without dropping the pooled wrapper
					// We verify its Ok() explicitly above
					next_conn = e.value.take().map(|(_, c)| c);
					if let Some(next_conn) = &next_conn {
						// We reserved it above, now drop it back to avoid double counting
						host.release_h2_stream_without_returning_to_idle(next_conn);
					}
				},
				Err(_) => unreachable!("Ok() always above"),
			}
		}
		trace!(fulfilled=%sent, "sent {reason} connection");
		if sent == 0
			&& let Some(c) = next_conn
		{
			if let HttpConnection::Http2(h2) = &c {
				// If we tried to send, we may have put it in the active_h2 list and need to remove it
				let _ = host.active_h2.remove(h2);
			}
			trace!("nobody wanted {reason} connection; inserting as idle");
			Self::ensure_idle_interval(pool, settings);
			let now = settings.timer.now();
			host.push_idle_with_cap(settings.max_idle_per_host, key, c, now);
		}
	}

	pub(crate) fn checkout_or_register_waker(&self, key: K) -> CheckoutResult<K> {
		let mut host = self.host(&key);
		// First attempt: find any active H2 streams with available capacity and attach to that.
		if let Some(reserved) = host.active_h2.reserve() {
			trace!("found active h2 connection with capacity");
			let p = Pooled {
				value: Some((key, HttpConnection::Http2(reserved))),
				is_reused: true,
				pool: Arc::downgrade(&self.hosts),
				settings: self.settings.clone(),
			};
			return CheckoutResult::Checkout(p);
		}

		{
			let expiration = Expiration::new(self.settings.timeout);
			let now = self.settings.timer.now();
			let popper = IdlePopper {
				key: &key,
				list: &mut host.idle,
			};
			if let Some(got) = popper.pop(&expiration, now) {
				trace!("found idle connection");
				let c = got.value;
				// For HTTP2, as they are shared, we keep active connections tracked.
				// Otherwise, there is no need and we just return is as Owned.
				let c = host.active_h2.maybe_insert_new(c, true);
				let p = Pooled {
					value: Some((key, c)),
					is_reused: true,
					pool: Arc::downgrade(&self.hosts),
					settings: self.settings.clone(),
				};
				return CheckoutResult::Checkout(p);
			}
		}
		// At this point nothing is immediately available to us.
		// We will register ourselves as a waiter, and indicate to the caller if they should spawn
		// a connection or not.
		let pending = host.expected_connecting_capacity;
		// Clear cancelled waiters
		host.waiters.retain(|w| !w.is_canceled());
		let waiters = host.waiters.len();
		trace!("checkout waiting for idle connection: {:?}", key);
		let should_connect = if pending <= waiters {
			// We need more capacity! Start a connection
			// We will assume the caller is actually going to do this
			let expected = host
				.per_connection_capacity_cache
				.expected_capacity(self.settings.expected_http2_capacity);
			host.connecting += 1;
			host.expected_connecting_capacity += expected;
			Some(ShouldConnect {
				inner: Some(ShouldConnectInner {
					expected_capacity: expected,
					key,
					pool: Arc::downgrade(&self.hosts),
				}),
			})
		} else {
			None
		};
		trace!(should_connect=%should_connect.is_some(), "no active or idle connections available");
		let (tx, rx) = oneshot::channel();
		host.waiters.push_back(tx);
		CheckoutResult::Wait(WaitForConnection {
			waiter: rx,
			should_connect,
		})
	}
}

/// Pop off this list, looking for a usable connection that hasn't expired.
struct IdlePopper<'a, K> {
	key: &'a K,
	list: &'a mut Vec<Idle>,
}

impl<'a, K: Debug> IdlePopper<'a, K> {
	fn pop(self, expiration: &Expiration, now: Instant) -> Option<Idle> {
		while let Some(entry) = self.list.pop() {
			// If the connection has been closed, or is older than our idle
			// timeout, simply drop it and keep looking...
			if !entry.value.is_open() {
				trace!("removing closed connection for {:?}", self.key);
				continue;
			}
			// TODO: Actually, since the `idle` list is pushed to the end always,
			// that would imply that if *this* entry is expired, then anything
			// "earlier" in the list would *have* to be expired also... Right?
			//
			// In that case, we could just break out of the loop and drop the
			// whole list...
			if expiration.expires(entry.idle_at, now) {
				trace!("removing expired connection for {:?}", self.key);
				continue;
			}

			return Some(entry);
		}

		None
	}
}

/// A wrapped poolable value that tries to reinsert to the Pool on Drop.
pub(crate) struct Pooled<K: Key> {
	value: Option<(K, HttpConnection)>,
	is_reused: bool,
	pool: Weak<HostShards<K>>,
	settings: Arc<PoolSettings>,
}

impl<K: Key> Debug for Pooled<K> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Pooled")
			.field("value", &self.value.is_some())
			.field("is_reused", &self.is_reused)
			.finish()
	}
}

impl<K: Key> Pooled<K> {}

impl<K: Key> Pooled<K> {
	pub(crate) fn into_h2_parts(mut self) -> (ReservedHttp2Connection, H2CapacityGuard<K>) {
		let (k, v) = self.value.take().expect("not dropped");
		let HttpConnection::Http2(h2) = v else {
			panic!("into_h2_parts must be used on http2")
		};
		let guard = H2CapacityGuard {
			value: Some((k, h2.load.clone())),
			pool: self.pool.clone(),
			settings: self.settings.clone(),
		};
		(h2, guard)
	}

	pub fn is_reused(&self) -> bool {
		self.is_reused
	}

	fn as_ref(&self) -> &HttpConnection {
		self.value.as_ref().map(|v| &v.1).expect("not dropped")
	}

	fn as_mut(&mut self) -> &mut HttpConnection {
		self.value.as_mut().map(|v| &mut v.1).expect("not dropped")
	}
	pub fn is_http2(&self) -> bool {
		match self.as_ref() {
			HttpConnection::Http1(_) => false,
			HttpConnection::Http2(_) => true,
		}
	}
	pub fn is_http1(&self) -> bool {
		!self.is_http2()
	}
}

impl<K: Key> Deref for Pooled<K> {
	type Target = HttpConnection;
	fn deref(&self) -> &HttpConnection {
		self.as_ref()
	}
}

impl<K: Key> DerefMut for Pooled<K> {
	fn deref_mut(&mut self) -> &mut HttpConnection {
		self.as_mut()
	}
}

impl<K: Key> Drop for Pooled<K> {
	fn drop(&mut self) {
		let Some((k, value)) = self.value.take() else {
			// Already handled
			return;
		};
		let Some(pool) = self.pool.upgrade() else {
			trace!("pool dropped, dropping pooled ({:?})", k);
			return;
		};
		let mut hosts = Pool::lock_hosts(pool.as_ref(), &k);
		if value.is_open() {
			trace!(key=?k, "returning connection to pool");
			hosts.return_connection(self.settings.clone(), pool.clone(), k, value);
		} else {
			trace!("connection already closed; skip idle pool insertion");
			// If we *already* know the connection is done here,
			// it shouldn't be re-inserted back into the pool.
			hosts.return_dead_connection(value)
		}
	}
}

impl<K: Key> Drop for H2CapacityGuard<K> {
	fn drop(&mut self) {
		let Some((k, value)) = self.value.take() else {
			// Already handled
			return;
		};
		let Some(pool) = self.pool.upgrade() else {
			trace!("pool dropped, dropping pooled ({:?})", k);
			return;
		};
		let mut hosts = Pool::lock_hosts(pool.as_ref(), &k);
		hosts.return_h2_stream(self.settings.clone(), pool.clone(), k, value);
	}
}

#[derive(Debug)]
struct Idle {
	idle_at: Instant,
	value: HttpConnection,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
	PoolDisabled,
	CheckoutNoLongerWanted,
	CheckedOutClosedValue,
	WaitingOnSharedFailedConnection,
	ConnectionDroppedWithoutCompletion,
	ConnectionLowCapacity,
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Error::PoolDisabled => "pool is disabled",
			Error::CheckedOutClosedValue => "checked out connection was closed",
			Error::CheckoutNoLongerWanted => "request was canceled",
			Error::WaitingOnSharedFailedConnection => "shared wait failed",
			Error::ConnectionDroppedWithoutCompletion => "connection dropped without completion",
			Error::ConnectionLowCapacity => "connection didn't have enough capacity",
		})
	}
}

impl StdError for Error {}

struct Expiration(Option<Duration>);

impl Expiration {
	fn new(dur: Option<Duration>) -> Expiration {
		Expiration(dur)
	}

	fn expires(&self, instant: Instant, now: Instant) -> bool {
		match self.0 {
			// Avoid `Instant::elapsed` to avoid issues like rust-lang/rust#86470.
			Some(timeout) => now.saturating_duration_since(instant) > timeout,
			None => false,
		}
	}
}

#[derive(Debug)]
pub(crate) enum ClientConnectError {
	Normal(crate::Error),
	CheckoutIsClosed(Error),
}

pin_project_lite::pin_project! {
	struct IdleTask<K: Key> {
		timer: Timer,
		duration: Duration,
		deadline: Instant,
		fut: Pin<Box<dyn Sleep>>,
		pool: Weak<HostShards<K>>,
		settings: Arc<PoolSettings>,
	}
}

impl<K: Key> Future for IdleTask<K> {
	type Output = ();

	fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
		let mut this = self.project();
		loop {
			ready!(Pin::new(&mut this.fut).poll(cx));
			// Set this task to run after the next deadline
			// If the poll missed the deadline by a lot, set the deadline
			// from the current time instead
			*this.deadline += *this.duration;
			if *this.deadline < Instant::now() - Duration::from_millis(5) {
				*this.deadline = Instant::now() + *this.duration;
			}
			*this.fut = this.timer.sleep_until(*this.deadline);

			if let Some(inner) = this.pool.upgrade() {
				trace!("idle interval checking for expired");
				inner.clear_expired(this.settings.as_ref());
				continue;
			}
			trace!("pool closed, canceling idle interval");
			return Poll::Ready(());
		}
	}
}

#[cfg(all(test, not(miri)))]
mod tests {
	use super::*;
	use super::{ExpectedCapacity, Key, Pool};
	use crate::connect::Connected;
	use crate::rt::{TokioExecutor, TokioIo};
	use assert_matches::assert_matches;
	use bytes::Bytes;
	use futures_channel::oneshot::Receiver;
	use http_body_util::Full;
	use hyper::body::Incoming;
	use hyper::rt::Sleep;
	use hyper::server::conn::{http1, http2};
	use hyper::service::service_fn;
	use hyper::{Request, Response};
	use std::collections::HashSet;
	use std::fmt::Debug;
	use std::future::Future;
	use std::hash::Hash;
	use std::pin::Pin;
	use std::sync::Arc;
	use std::sync::Once;
	use std::task::{self, Poll};
	use std::time::{Duration, Instant};
	use tracing_subscriber::EnvFilter;

	#[derive(Clone, Debug, PartialEq, Eq, Hash)]
	struct KeyImpl(http::uri::Scheme, http::uri::Authority, ExpectedCapacity);

	impl Key for KeyImpl {
		fn expected_capacity(&self) -> ExpectedCapacity {
			self.2
		}
	}

	fn host_key(s: &str) -> KeyImpl {
		KeyImpl(
			http::uri::Scheme::HTTP,
			s.parse().expect("host key"),
			ExpectedCapacity::Http1,
		)
	}

	fn host_key_h2(s: &str) -> KeyImpl {
		KeyImpl(
			http::uri::Scheme::HTTP,
			s.parse().expect("host key"),
			ExpectedCapacity::Http2,
		)
	}

	fn host_key_auto(s: &str) -> KeyImpl {
		KeyImpl(
			http::uri::Scheme::HTTP,
			s.parse().expect("host key"),
			ExpectedCapacity::Auto,
		)
	}

	#[derive(Clone, Debug, Default)]
	struct MockTimer {
		next_now: Arc<parking_lot::Mutex<Option<Instant>>>,
	}

	#[derive(Debug)]
	struct ReadySleep {
		polled: bool,
	}

	impl Future for ReadySleep {
		type Output = ();

		fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
			if !self.polled {
				self.polled = true;
				cx.waker().wake_by_ref();
				return Poll::Pending;
			}
			Poll::Ready(())
		}
	}

	impl Sleep for ReadySleep {}

	impl hyper::rt::Timer for MockTimer {
		fn sleep(&self, duration: Duration) -> Pin<Box<dyn Sleep>> {
			self.sleep_until(self.now() + duration)
		}

		fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Sleep>> {
			*self.next_now.lock() = Some(deadline + Duration::from_millis(1));
			Box::pin(ReadySleep { polled: false })
		}

		fn now(&self) -> Instant {
			self.next_now.lock().take().unwrap_or_else(Instant::now)
		}
	}

	fn init_test_tracing() {
		static INIT: Once = Once::new();

		INIT.call_once(|| {
			let _ = tracing_subscriber::fmt()
				.with_test_writer()
				.with_env_filter(EnvFilter::new("agent_pool=trace"))
				.try_init();
		});
	}

	fn pool<K: Key>() -> Pool<K> {
		init_test_tracing();
		pool_max_idle(usize::MAX)
	}

	fn pool_max_idle<K: Key>(max_idle: usize) -> Pool<K> {
		Pool::new(
			super::Config {
				idle_timeout: Some(Duration::from_millis(100)),
				max_idle_per_host: max_idle,
				expected_http2_capacity: DEFAULT_EXPECTED_HTTP2_CAPACITY,
			},
			TokioExecutor::new(),
			MockTimer::default(),
		)
	}

	fn pool_with_idle_timeout<K: Key>(idle_timeout: Duration) -> Pool<K> {
		init_test_tracing();
		Pool::new(
			super::Config {
				idle_timeout: Some(idle_timeout),
				max_idle_per_host: usize::MAX,
				expected_http2_capacity: DEFAULT_EXPECTED_HTTP2_CAPACITY,
			},
			TokioExecutor::new(),
			MockTimer::default(),
		)
	}

	fn pool_with_expected_h2_capacity_idle<K: Key>(
		expected_http2_capacity: usize,
		idle: Duration,
	) -> Pool<K> {
		init_test_tracing();
		Pool::new(
			super::Config {
				idle_timeout: Some(idle),
				max_idle_per_host: usize::MAX,
				expected_http2_capacity,
			},
			TokioExecutor::new(),
			MockTimer::default(),
		)
	}
	fn pool_with_expected_h2_capacity<K: Key>(expected_http2_capacity: usize) -> Pool<K> {
		pool_with_expected_h2_capacity_idle(expected_http2_capacity, Duration::from_secs(10))
	}

	fn install_send_connection_test_hook(f: impl FnMut() -> bool + Send + 'static) {
		*Pool::<KeyImpl>::send_connection_test_hook().lock() = Some(Box::new(f));
	}

	fn clear_send_connection_test_hook() {
		*Pool::<KeyImpl>::send_connection_test_hook().lock() = None;
	}

	struct SendConnectionTestHookGuard {
		_guard: tokio::sync::MutexGuard<'static, ()>,
	}

	impl Drop for SendConnectionTestHookGuard {
		fn drop(&mut self) {
			clear_send_connection_test_hook();
		}
	}

	async fn lock_send_connection_test_hook() -> SendConnectionTestHookGuard {
		static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
		SendConnectionTestHookGuard {
			_guard: TEST_LOCK.lock().await,
		}
	}

	fn assert_h2_queue_invariants(pool: &Pool<KeyImpl>, key: &KeyImpl, context: &str) {
		let host = pool.host(key);
		let mut active_ids = HashSet::new();
		let mut any_available = false;
		let mut front_available = false;

		for (idx, conn) in host.active_h2.0.iter().enumerate() {
			let id = Arc::as_ptr(&conn.load);
			assert!(
				active_ids.insert(id),
				"{context}: duplicate h2 entry in active queue for {key:?}: {:?}",
				host.active_h2
			);
			let active = conn.load.active_streams.load(Ordering::Acquire);
			let max = conn.load.max_streams.load(Ordering::Acquire);
			assert!(
				active <= max,
				"{context}: h2 load exceeded max for {key:?}: active={active} max={max}"
			);
			let has_capacity = active < max;
			any_available |= has_capacity;
			if idx == 0 {
				front_available = has_capacity;
			}
		}

		for idle in &host.idle {
			if let HttpConnection::Http2(conn) = &idle.value {
				let idle_id = Arc::as_ptr(&conn.load);
				assert!(
					!active_ids.contains(&idle_id),
					"{context}: h2 connection present in both idle and active queues for {key:?}"
				);
			}
		}

		assert!(
			!any_available || front_available,
			"{context}: available h2 capacity hidden behind a full front entry for {key:?}: {:?}",
			host.active_h2
		);
	}

	fn must_want_new_connection(
		pool: &Pool<KeyImpl>,
		key: KeyImpl,
	) -> (
		ShouldConnect<KeyImpl>,
		Receiver<Result<Pooled<KeyImpl>, ClientConnectError>>,
	) {
		let checkout_result = pool.checkout_or_register_waker(key.clone());
		assert_matches!(
			checkout_result,
			CheckoutResult::Wait(WaitForConnection {
				should_connect: Some(sc),
				waiter,
				..
			}) => (sc, waiter),
			"wanted new connection, but didn't get one."
		)
	}

	fn must_wait_for_existing_connection(
		pool: &Pool<KeyImpl>,
		key: KeyImpl,
	) -> Receiver<Result<Pooled<KeyImpl>, ClientConnectError>> {
		let checkout_result = pool.checkout_or_register_waker(key.clone());
		assert_matches!(
			checkout_result,
			CheckoutResult::Wait(WaitForConnection {
				should_connect: None,
				waiter,
				..
			}) => waiter,
			"wanted existing connection, but didn't get one."
		)
	}

	fn must_checkout(pool: &Pool<KeyImpl>, key: KeyImpl) -> Pooled<KeyImpl> {
		let checkout_result = pool.checkout_or_register_waker(key.clone());
		assert_matches!(
			checkout_result,
			CheckoutResult::Checkout(p) => p
		)
	}

	async fn mock_http1_connection() -> HttpConnection {
		mock_http1_connection_with_control().await.0
	}

	struct MockHttp1Control {
		server_task: tokio::task::JoinHandle<()>,
		conn_task: tokio::task::JoinHandle<()>,
	}

	impl MockHttp1Control {
		async fn close(self) {
			self.server_task.abort();
			self.conn_task.abort();
			tokio::task::yield_now().await;
		}
	}

	async fn mock_http1_connection_with_control() -> (HttpConnection, MockHttp1Control) {
		let (client, server) = tokio::io::duplex(8192);
		let server_task = tokio::spawn(async move {
			let service = service_fn(|_req: Request<Incoming>| async move {
				Ok::<_, std::convert::Infallible>(
					Response::builder()
						.status(200)
						.body(Full::new(Bytes::from_static(b"ok")))
						.expect("response body"),
				)
			});
			let _ = http1::Builder::new()
				.serve_connection(TokioIo::new(server), service)
				.await;
		});

		let (mut tx, conn) = hyper::client::conn::http1::Builder::new()
			.handshake(TokioIo::new(client))
			.await
			.expect("client handshake");
		let conn_task = tokio::spawn(async move {
			let _ = conn.await;
		});
		tx.ready().await.expect("client sender ready");

		(
			HttpConnection::Http1(ReservedHttp1Connection {
				info: Connected::new(),
				tx,
			}),
			MockHttp1Control {
				server_task,
				conn_task,
			},
		)
	}

	async fn mock_http2_connection(max_streams: usize) -> HttpConnection {
		mock_http2_connection_with_control(max_streams).await.0
	}

	struct MockHttp2Control {
		server_task: tokio::task::JoinHandle<()>,
		conn_task: tokio::task::JoinHandle<()>,
	}

	impl MockHttp2Control {
		async fn close(self) {
			self.server_task.abort();
			self.conn_task.abort();
			tokio::task::yield_now().await;
		}
	}

	async fn mock_http2_connection_with_control(
		max_streams: usize,
	) -> (HttpConnection, MockHttp2Control) {
		let (client, server) = tokio::io::duplex(8192);
		let server_task = tokio::spawn(async move {
			let service = service_fn(|_req: Request<Incoming>| async move {
				Ok::<_, std::convert::Infallible>(
					Response::builder()
						.status(200)
						.body(Full::new(Bytes::from_static(b"ok")))
						.expect("response body"),
				)
			});
			let _ = http2::Builder::new(TokioExecutor::new())
				.max_concurrent_streams(max_streams as u32)
				.serve_connection(TokioIo::new(server), service)
				.await;
		});

		let (mut tx, conn) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
			.handshake(TokioIo::new(client))
			.await
			.expect("client h2 handshake");
		let conn_task = tokio::spawn(async move {
			let _ = conn.await;
		});
		tx.ready().await.expect("client h2 sender ready");

		(
			HttpConnection::Http2(ReservedHttp2Connection {
				info: Connected::new(),
				tx,
				load: Arc::new(H2Load::new(max_streams)),
			}),
			MockHttp2Control {
				server_task,
				conn_task,
			},
		)
	}

	#[tokio::test]
	async fn first_checkout_requires_connection() {
		let pool = pool();
		let key = host_key("foo");
		let _ = must_want_new_connection(&pool, key);
	}

	#[tokio::test]
	async fn test_zero_expected_http2_capacity_should_not_panic_when_connect_is_dropped() {
		// This test is really dumb but whatever, I suppose we shouldn't panic here
		let pool = pool_with_expected_h2_capacity::<KeyImpl>(0);
		let key = host_key_h2("foo");
		let (sc1, _w1) = must_want_new_connection(&pool, key.clone());

		// Dropping the connect token should cleanly fail the waiter, even if the
		// configured expected HTTP/2 capacity is zero.
		drop(sc1);
	}

	#[tokio::test]
	async fn test_pool_new_connection() {
		let pool = pool();
		let key = host_key("foo");
		let (sc, w) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc, mock_http1_connection().await);
		let pooled = w
			.await
			.expect("waiter should receive inserted connection")
			.unwrap();
		assert!(pooled.is_http1());
		assert!(!pooled.is_reused);
	}

	#[tokio::test]
	async fn test_pool_new_connection_and_return() {
		let pool = pool();
		let key = host_key("foo");
		let (sc, w) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc, mock_http1_connection().await);
		let pooled = w.await.expect("waiter should receive inserted connection");
		drop(pooled);
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_idle_interval_evicts_before_checkout_timeout() {
		let pool = pool();
		let key = host_key("foo");
		let (sc, waiter) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc, mock_http1_connection().await);
		let pooled = waiter
			.await
			.expect("waiter should receive inserted connection");
		drop(pooled);

		tokio::time::sleep(Duration::from_millis(10)).await;

		let checkout_result = pool.checkout_or_register_waker(key.clone());
		assert_matches!(
			checkout_result,
			CheckoutResult::Wait(WaitForConnection {
				should_connect: Some(_),
				..
			})
		);
	}

	#[tokio::test]
	async fn test_pool_multi_race() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http1_connection().await);
		let pooled1 = w1.await.expect("waiter should receive inserted connection");
		pool.insert_new_connection(sc2, mock_http1_connection().await);
		let pooled2 = w2.await.expect("waiter should receive inserted connection");
		drop(pooled1);
		drop(pooled2);
		let _ = must_checkout(&pool, key.clone());
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_cancelled_waiter_without_insert() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		// Simulate this task cancelling before the connection is inserted.
		drop(sc1);
		drop(w1);
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());
		pool.insert_new_connection(sc2, mock_http1_connection().await);
		let pooled2 = w2.await.expect("waiter should receive inserted connection");
		drop(pooled2);
		// This should get the pooled2 idle conn
		let _c1 = must_checkout(&pool, key.clone());
		// Should get a new one requested
		let _ = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_cancelled_waiter_with_insert() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		// Simulate this task cancelling after the connection is inserted.
		pool.insert_new_connection(sc1, mock_http1_connection().await);
		drop(w1);
		// We should be able to checkout the connection since w1 didn't want it
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_canceled_pending_waiter_should_not_force_extra_connection() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());

		// The original request went away while its connection attempt is still in flight.
		drop(w1);

		// A new request should wait on the already-pending connection instead of
		// immediately demanding another one.
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http1_connection().await);
		let _pooled2 = w2
			.await
			.expect("live waiter should receive the existing pending connection")
			.unwrap();
	}

	#[tokio::test]
	async fn test_pool_cancelled_waiter_with_insert_drop_first() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		// Simulate this task cancelling before the connection is inserted.
		drop(w1);
		pool.insert_new_connection(sc1, mock_http1_connection().await);
		// We should be able to checkout the connection since w1 didn't want it
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_cancelled_waiter_with_insert_race() {
		let pool = pool();
		let key = host_key("foo");
		// Similar to test_pool_cancelled_waiter_with_insert but this time we start another connection between
		// the initial and insert_new_connection.
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		// Simulate this task cancelling after the connection is inserted.
		pool.insert_new_connection(sc1, mock_http1_connection().await);
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());
		pool.insert_new_connection(sc2, mock_http1_connection().await);
		drop(w1);
		// w2 should get its connection
		let _ = w2.await.expect("waiter should receive inserted connection");
		// We should be able to checkout the connection since w1 didn't want it
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_cancelled_waiter_and_connection() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());

		// The first request goes away, but the second request still has its own
		// connection attempt in flight via sc2.
		drop(w1);
		drop(sc1);

		pool.insert_new_connection(sc2, mock_http1_connection().await);
		// The error from sc1 delivers to w2; this is expected. waiters and new connections are not coupled at all.
		assert_matches!(w2.await, Ok(Err(ClientConnectError::CheckoutIsClosed(_))))
	}

	#[tokio::test]
	async fn test_pool_cancelled_connection_while_waiting() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		drop(sc1);
		let _pooled1 = w1.await.expect("waiter should receive connection");
	}
	#[tokio::test]
	async fn test_pool_return_idle_with_only_cancelled_waiters_keeps_connection_reusable() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http1_connection().await);
		let pooled1 = w1.await.expect("waiter should receive inserted connection");

		let (sc2, w2) = must_want_new_connection(&pool, key.clone());
		drop(sc2);
		drop(w2);

		drop(pooled1);

		let _pooled2 = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_return_idle_skips_cancelled_waiter_then_wakes_live_waiter() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http1_connection().await);
		let pooled1 = w1
			.await
			.expect("waiter should receive inserted connection")
			.unwrap();

		// Fully cancelled the connection
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());
		drop(sc2);
		drop(w2);
		let (_sc3, w3) = must_want_new_connection(&pool, key.clone());

		let mut w3 = Box::pin(w3);
		assert!(
			futures_util::poll!(&mut w3).is_pending(),
			"live waiter should still be pending"
		);
		drop(pooled1);

		let _pooled3 = w3
			.await
			.expect("live waiter should receive returned connection");
	}

	#[tokio::test]
	async fn test_pool_checkout_skips_expired_idle_connection() {
		let pool = pool_with_idle_timeout(Duration::from_millis(5));
		let key = host_key("foo");
		let (sc, w) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc, mock_http1_connection().await);
		let pooled = w.await.expect("waiter should receive inserted connection");
		drop(pooled);

		tokio::time::sleep(Duration::from_millis(8)).await;

		let (_sc2, _w2) = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_waiter_fairness_with_staggered_inserts_and_return() {
		let pool = pool();
		let key = host_key("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());
		let (sc3, w3) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http1_connection().await);
		let pooled1 = w1
			.await
			.expect("first waiter should receive first connection")
			.unwrap();
		pool.insert_new_connection(sc2, mock_http1_connection().await);
		let _pooled2 = w2
			.await
			.expect("second waiter should receive second connection")
			.unwrap();
		drop(sc3);
		assert_matches!(
			w3.await
				.expect("third waiter should receive third connection"),
			Err(ClientConnectError::CheckoutIsClosed(_))
		);
		drop(pooled1);

		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_host_isolation() {
		let pool = pool();
		let key_a = host_key("foo");
		let key_b = host_key("bar");
		let (sc_a, w_a) = must_want_new_connection(&pool, key_a.clone());
		pool.insert_new_connection(sc_a, mock_http1_connection().await);
		drop(w_a);
		let (_sc_b, _w_b) = must_want_new_connection(&pool, key_b.clone());
	}

	#[tokio::test]
	async fn test_pool_closed_http1_connection_not_reused_after_return() {
		let pool = pool();
		let key = host_key("foo");
		let (sc, w) = must_want_new_connection(&pool, key.clone());
		let (conn, control) = mock_http1_connection_with_control().await;

		pool.insert_new_connection(sc, conn);
		let pooled = w.await.expect("waiter should receive inserted connection");
		drop(pooled);

		control.close().await;

		let (_sc2, _w2) = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1
			.await
			.expect("first waiter should receive h2 connection")
			.unwrap();
		assert!(pooled1.is_http2());
		let _pooled2 = w2
			.await
			.expect("second waiter should receive shared h2 connection");

		// At capacity, should need a new connection
		let (_sc3, _w3) = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_reuse() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("get h2");
		drop(pooled1);
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_reuse_many() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("get h2");
		let _pooled2 = w2.await.expect("get h2");
		drop(pooled1);
		let _w2 = must_checkout(&pool, key.clone());

		// At capacity, should need a new connection
		let (_sc3, _w3) = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_returned_capacity_wakes_parked_waiter() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("get h2");
		let _pooled2 = w2.await.expect("get h2");

		let (sc3, w3) = must_want_new_connection(&pool, key.clone());
		drop(sc3);

		assert_matches!(
			w3.await
				.expect("third waiter should receive third connection"),
			Err(ClientConnectError::CheckoutIsClosed(_))
		);

		drop(pooled1);

		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_returned_active_capacity_does_not_wake_existing_waiter() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("first waiter should receive h2 connection");
		let pooled2 = w2
			.await
			.expect("second waiter should receive h2 connection");

		// One extra request starts another connect, and the next request parks
		// behind it. If the current h2 connection regains capacity first, that
		// parked waiter should be woken by the returned stream.
		let (sc2, _w3) = must_want_new_connection(&pool, key.clone());
		let mut w4 = Box::pin(must_wait_for_existing_connection(&pool, key.clone()));

		drop(pooled1);

		// This should probably be ready immediately instead of waiting for sc2... but its not a big deal.
		assert!(
			!futures_util::poll!(&mut w4).is_ready(),
			"returning an h2 stream with a parked waiter should wake that waiter immediately"
		);

		drop(sc2);
		drop(pooled2);
		// Should get cancelled from sc2 dropping
		w4.await
			.expect("waiter should receive h2 connection")
			.expect_err("should fail");
	}

	#[tokio::test]
	async fn test_h2_last_stream_returned_to_waiter_keeps_remaining_capacity_tracked() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("first waiter should receive h2 connection");
		let pooled2 = w2
			.await
			.expect("second waiter should receive h2 connection");

		// With the first connection full, the next request starts another connect and
		// a second request waits behind it.
		let (sc2, w3) = must_want_new_connection(&pool, key.clone());
		let mut w4 = Some(must_wait_for_existing_connection(&pool, key.clone()));

		// Free one slot first, then return the last active stream. The last return
		// will hand the idle connection directly to w3.
		drop(pooled1);
		drop(pooled2);

		let _pooled3 = w3
			.await
			.expect("first queued waiter should receive the returned h2 connection")
			.unwrap();

		// Remove the stale queued waiter so fairness does not mask whether the
		// returned h2 connection still has its spare slot tracked.
		drop(w4.take());

		// The same h2 connection still has one more free stream slot, so an immediate
		// checkout should reuse it instead of waiting for sc2's future connection.
		let _pooled4 = must_checkout(&pool, key.clone());

		drop(sc2);
	}

	#[tokio::test]
	async fn test_h2_reuse_cancel() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		// sc1 was supposed to open a connection for w1 and w2 but it dropped...
		drop(sc1);

		let _pooled1 = w1.await.expect("get h2");
		let _pooled2 = w2.await.expect("get h2");
	}

	#[tokio::test]
	async fn test_h2_many_concurrent_connections() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		// We can ask for multiple concurrent requests
		let (sc3, w3) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let _pooled1 = w1.await.expect("get h2");
		let _pooled2 = w2.await.expect("get h2");
		pool.insert_new_connection(sc3, mock_http2_connection(2).await);
		let _pooled3 = w3.await.expect("get h2");
		// connection 2 has room
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_over_capacity() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		// We expected 4 but it got more capacity
		pool.insert_new_connection(sc1, mock_http2_connection(4).await);
		let _pooled1 = w1.await.expect("get h2");
		let _pooled2 = w2.await.expect("get h2");
		// Since we had more capacity, we should be able to checkout.
		// NOTE: client.rs does not follow this pattern and caps the capacity to the expected size.
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_checkout_skips_full_front_connection_and_reuses_open_behind() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		let (sc2, w3) = must_want_new_connection(&pool, key.clone());
		let w4 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1
			.await
			.expect("first waiter should receive first h2 connection");
		let pooled2 = w2
			.await
			.expect("second waiter should receive first h2 connection");

		// Make the older connection open again before inserting the newer one fully busy.
		drop(pooled1);

		pool.insert_new_connection(sc2, mock_http2_connection(2).await);
		let pooled3 = w3
			.await
			.expect("third waiter should receive second h2 connection");
		let pooled4 = w4
			.await
			.expect("fourth waiter should receive second h2 connection");

		// There is still spare capacity on the older connection, so this should reuse it
		// instead of asking for a third connection.
		assert_eq!(2, pool.host(&key).active_h2.0.len());
		let _ = must_checkout(&pool, key.clone());

		assert_eq!(2, pool.host(&key).active_h2.0.len());

		drop(pooled3);
		assert_eq!(2, pool.host(&key).active_h2.0.len());
		drop(pooled2);
		assert_eq!(1, pool.host(&key).active_h2.0.len());
		drop(pooled4);
		assert_eq!(0, pool.host(&key).active_h2.0.len());
	}

	#[tokio::test]
	async fn test_h2_checkout_idle() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("get h2");
		drop(pooled1);

		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_unique_connection_is_not_reused_past_capacity_after_becoming_idle() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("first waiter should receive h2 connection");
		let pooled2 = w2
			.await
			.expect("second waiter should receive shared h2 connection");

		drop(pooled1);
		drop(pooled2);

		let reused1 = must_checkout(&pool, key.clone());
		let reused2 = must_checkout(&pool, key.clone());

		let (_sc2, _w3) = must_want_new_connection(&pool, key.clone());

		drop(reused1);
		drop(reused2);
	}

	#[tokio::test]
	async fn test_h2_waiter_cancelled() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		// Cancel wait
		drop(w2);
		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let _pooled1 = w1.await.expect("get h2");
		let _pooled2 = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_waiter_cancelled_race() {
		let _guard = lock_send_connection_test_hook().await;
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let _pooled1 = w1
			.await
			.expect("first waiter should receive first h2 connection");
		let _pooled2 = w2
			.await
			.expect("second waiter should receive first h2 connection");

		// With the first h2 connection full, a second connection is needed. We then cancel the
		// second waiter for that new connection in the narrow window after `is_canceled()` has
		// been checked and before `tx.send(...)`, so the send-failure rollback path runs after
		// the connection has already been marked full.
		let (sc2, w3) = must_want_new_connection(&pool, key.clone());
		let w4 = must_wait_for_existing_connection(&pool, key.clone());
		let mut hook_calls = 0;
		let mut w4 = Some(w4);
		install_send_connection_test_hook(move || {
			hook_calls += 1;
			if hook_calls == 2 {
				drop(w4.take());
				false
			} else {
				true
			}
		});

		pool.insert_new_connection(sc2, mock_http2_connection(2).await);
		let _pooled3 = w3
			.await
			.expect("first waiter should receive second h2 connection");
		assert_h2_queue_invariants(&pool, &key, "deterministic canceled waiter repro");
		must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_canceled_only_waiter_keeps_connection_in_idle_and_active() {
		let _guard = lock_send_connection_test_hook().await;
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let mut w1 = Some(w1);
		install_send_connection_test_hook(move || {
			drop(w1.take());
			false
		});

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);

		assert_h2_queue_invariants(
			&pool,
			&key,
			"single canceled waiter send race should not leave h2 in idle and active",
		);
	}

	#[tokio::test]
	async fn test_h2_canceled_only_waiter_allows_duplicate_checkout_of_same_max1_connection() {
		let _guard = lock_send_connection_test_hook().await;
		let pool = pool_with_expected_h2_capacity(1);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let mut w1 = Some(w1);
		install_send_connection_test_hook(move || {
			drop(w1.take());
			false
		});

		pool.insert_new_connection(sc1, mock_http2_connection(1).await);

		// First checkout comes from the lingering active_h2 entry.
		let _pooled1 = must_checkout(&pool, key.clone());
		// Now we should need another
		let (_sc2, _w2) = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_first_send_race_cancellation_can_drop_active_tracking_for_second_waiter() {
		let _guard = lock_send_connection_test_hook().await;
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		let mut w1 = Some(w1);
		let mut calls = 0;
		install_send_connection_test_hook(move || {
			calls += 1;
			if calls == 1 {
				drop(w1.take());
				false
			} else {
				true
			}
		});

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);

		let _pooled2 = w2
			.await
			.expect("second waiter should receive the raced h2 connection")
			.unwrap();

		// The same h2 connection still has spare capacity and should be reused immediately.
		// If rollback removed it from `active_h2`, the pool will incorrectly request a new
		// connection here.
		let _pooled3 = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_h2_first_send_race_cancellation_can_starve_third_waiter_despite_free_capacity() {
		let _guard = lock_send_connection_test_hook().await;
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		let (_sc3, w3) = must_want_new_connection(&pool, key.clone());
		let mut w1 = Some(w1);
		let mut calls = 0;
		install_send_connection_test_hook(move || {
			calls += 1;
			if calls == 1 {
				drop(w1.take());
				false
			} else {
				true
			}
		});

		pool.insert_new_connection(sc1, mock_http2_connection(2).await);

		let _pooled2 = w2
			.await
			.expect("second waiter should receive the shared h2 connection")
			.unwrap();

		let _pooled3 = w3.await.expect("get").unwrap();
	}

	#[tokio::test]
	async fn test_h2_checkout_closed() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc1, _w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());

		drop(sc1);

		assert_matches!(w2.await, Ok(Err(ClientConnectError::CheckoutIsClosed(_))));
	}

	#[tokio::test]
	async fn test_h2_checkout_idle_expired() {
		let pool = pool_with_expected_h2_capacity_idle(2, Duration::from_millis(5));
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let pooled1 = w1.await.expect("get h2");
		drop(pooled1);

		tokio::time::sleep(Duration::from_millis(80)).await;

		let _ = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_auto_http2() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_auto("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		// Insert with capacity 2 (i.e. this was HTTP2).
		pool.insert_new_connection(sc1, mock_http2_connection(2).await);
		let _pooled1 = w1.await.expect("get h2").unwrap();
		let _w2 = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_auto_http1() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_auto("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		// Insert with capacity 1 (i.e. this was HTTP/1.1).
		pool.insert_new_connection(sc1, mock_http2_connection(1).await);
		let _pooled1 = w1.await.expect("get h2").unwrap();

		assert_matches!(
			w2.await.expect("get"),
			Err(ClientConnectError::CheckoutIsClosed(
				pool::Error::ConnectionLowCapacity
			))
		);
		let _ = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_auto_http1_caches() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_auto("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		// Insert with capacity 1 (i.e. this was HTTP/1.1).
		pool.insert_new_connection(sc1, mock_http2_connection(1).await);
		let _pooled1 = w1.await.expect("get h2").unwrap();

		assert_matches!(
			w2.await.expect("get"),
			Err(ClientConnectError::CheckoutIsClosed(
				pool::Error::ConnectionLowCapacity
			))
		);
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		// We learned from last time that we expect HTTP/1.1
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());
		// Insert with capacity 1 (i.e. this was HTTP/1.1).
		pool.insert_new_connection(sc1, mock_http2_connection(1).await);
		pool.insert_new_connection(sc2, mock_http2_connection(1).await);
		// This time, we should get success since we cached
		let _pooled1 = w1.await.expect("get h2").unwrap();
		let _pooled2 = w2.await.expect("get h2").unwrap();
	}

	#[tokio::test]
	async fn test_under_capacity_first_connect_cancelled_waiter() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_auto("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		let (sc2, w3) = must_want_new_connection(&pool, key.clone());

		// The shared waiter goes away, but the later waiter still has its own
		// pending connection via sc2.
		drop(w2);

		// The first connection comes in under-capacity (HTTP/1.1 instead of the
		// expected HTTP/2). That should not fail w3, since sc2 is still live.
		pool.insert_new_connection(sc1, mock_http2_connection(1).await);
		let _pooled1 = w1
			.await
			.expect("first waiter should receive connection")
			.unwrap();

		pool.insert_new_connection(sc2, mock_http2_connection(1).await);
		// In theory, we could directly send sc2's connection. However, this has the same practical result
		// as the retry loop will check it out.
		assert_matches!(w3.await, Ok(Err(ClientConnectError::CheckoutIsClosed(_))));
		let _ = must_checkout(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_pool_closed_http2_connection_not_reused() {
		let pool = pool_with_expected_h2_capacity(2);
		let key = host_key_h2("foo");
		let (sc, w) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		let (conn, control) = mock_http2_connection_with_control(2).await;

		pool.insert_new_connection(sc, conn);
		let pooled = w.await.expect("waiter should receive inserted connection");
		drop(pooled);
		let _pooled = w2.await.expect("waiter should receive inserted connection");

		control.close().await;

		let (_sc2, _w2) = must_want_new_connection(&pool, key.clone());
	}

	#[tokio::test]
	async fn test_closed_http2_guard_does_not_park_dead_connection_in_idle() {
		let pool = pool_with_expected_h2_capacity_idle(2, Duration::from_secs(60));
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let (conn, control) = mock_http2_connection_with_control(2).await;

		pool.insert_new_connection(sc1, conn);
		let pooled = w1
			.await
			.expect("waiter should receive inserted connection")
			.unwrap();
		let (_c, guard) = pooled.into_h2_parts();

		control.close().await;
		drop(guard);

		let host = pool.host(&key);
		assert!(
			host.idle.is_empty(),
			"dead h2 connection should not be parked in idle after guard drop"
		);
		assert!(
			host.active_h2.0.is_empty(),
			"dead h2 connection should not remain active after guard drop"
		);
	}

	#[tokio::test]
	async fn test_closed_checked_out_http2_connection_is_cleared_without_future_checkout() {
		let pool = pool_with_expected_h2_capacity_idle(2, Duration::from_millis(5));
		let key = host_key_h2("foo");
		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let w2 = must_wait_for_existing_connection(&pool, key.clone());
		let (conn, control) = mock_http2_connection_with_control(2).await;

		pool.insert_new_connection(sc1, conn);
		let pooled1 = w1
			.await
			.expect("first waiter should receive inserted connection")
			.unwrap();
		let pooled2 = w2
			.await
			.expect("second waiter should receive inserted connection")
			.unwrap();

		control.close().await;
		drop(pooled1);
		drop(pooled2);

		{
			let hosts = pool.hosts.lock_shard(&key);
			let host = hosts
				.get(&key)
				.expect("host entry should exist before cleanup");
			assert_eq!(host.active_h2.0.len(), 0, "should have no active");
			assert_eq!(host.idle.len(), 0, "should have no idle");
		}
	}

	#[tokio::test]
	async fn test_pool_max_idle_per_host_for_http1_connections() {
		let pool = pool_max_idle(2);
		let key = host_key("foo");

		let (sc1, w1) = must_want_new_connection(&pool, key.clone());
		let (sc2, w2) = must_want_new_connection(&pool, key.clone());
		let (sc3, w3) = must_want_new_connection(&pool, key.clone());

		pool.insert_new_connection(sc1, mock_http1_connection().await);
		pool.insert_new_connection(sc2, mock_http1_connection().await);
		pool.insert_new_connection(sc3, mock_http1_connection().await);

		let pooled1 = w1.await.expect("waiter should receive inserted connection");
		let pooled2 = w2.await.expect("waiter should receive inserted connection");
		let pooled3 = w3.await.expect("waiter should receive inserted connection");

		drop(pooled1);
		drop(pooled2);
		drop(pooled3);

		assert_eq!(
			pool.host(&key).idle.len(),
			2,
			"max_idle_per_host should cap idle HTTP/1 connections"
		);
	}
}
