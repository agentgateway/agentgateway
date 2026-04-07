use hashbrown::HashMap;
use hashbrown::hash_map::EntryRef;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};
use std::collections::VecDeque;
use std::error::Error as StdError;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::{self, Poll};
use std::time::{Duration, Instant};

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

pub const DEFAULT_EXPECTED_HTTP2_CAPACITY: usize = 100;

#[derive(Clone)]
pub struct Pool<K: Key> {
	hosts: Arc<Mutex<HashMap<K, HostPool<K>>>>,
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
	fn clear_expired(settings: &PoolSettings, hosts: &mut HashMap<K, HostPool<K>>) {
		let dur = settings.timeout.expect("interval assumes timeout");

		let now = settings.timer.now();

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
	fn lock_hosts<'a>(
		hosts: &'a Mutex<HashMap<K, HostPool<K>>>,
		k: &K,
	) -> MappedMutexGuard<'a, HostPool<K>> {
		MutexGuard::map(hosts.lock(), |l| match l.entry_ref(k) {
			EntryRef::Occupied(entry) => entry.into_mut(),
			EntryRef::Vacant(entry) => {
				entry.insert_with_key(k.clone(), HostPool::new(k.expected_capacity()))
			},
		})
	}
	fn host(&self, k: &K) -> MappedMutexGuard<'_, HostPool<K>> {
		Pool::<K>::lock_hosts(&self.hosts, k)
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
			// TODO: actually implement the cache part.
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
					h.load.try_reserve_stream_slot() == CapacityReservationResult::ReservedButNotFilled,
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
	pub(crate) tx: hyper::client::conn::http1::SendRequest<axum_core::body::Body>,
}

pub(crate) enum HttpConnection {
	Http1(ReservedHttp1Connection),
	Http2(ReservedHttp2Connection),
}

impl HttpConnection {
	pub fn capacity(&self) -> usize {
		match self {
			HttpConnection::Http1(_) => 1,
			HttpConnection::Http2(h) => h.load.remaining_capacity(),
		}
	}
	pub fn try_send_request(
		&mut self,
		req: Request<axum_core::body::Body>,
	) -> impl Future<
		Output = Result<
			Response<hyper::body::Incoming>,
			hyper::client::conn::TrySendError<Request<axum_core::body::Body>>,
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
	pool: Weak<Mutex<HashMap<K, HostPool<K>>>>,
	settings: Arc<PoolSettings>,
}

pub(crate) struct ReservedHttp2Connection {
	pub(crate) info: Connected,
	pub(crate) tx: hyper::client::conn::http2::SendRequest<axum_core::body::Body>,
	pub(crate) load: Arc<H2Load>,
}

impl ReservedHttp2Connection {
	fn clone_increment_load(&self) -> Option<(Self, bool)> {
		match self.load.try_reserve_stream_slot() {
			CapacityReservationResult::NoCapacity => None,
			CapacityReservationResult::ReservedAndFilled => {
				Some((self.clone_without_load_incremented(), true))
			},
			CapacityReservationResult::ReservedButNotFilled => {
				Some((self.clone_without_load_incremented(), false))
			},
		}
	}
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
		pool: Arc<Mutex<HashMap<K, HostPool<K>>>>,
		k: K,
		load: Arc<H2Load>,
	) {
		let (remaining, was_at_max) = load.release_stream_slot();
		if remaining == 0 {
			if let Some(v) = self.active_h2.remove_by_load(&load) {
				self.return_idle(settings, pool, k, HttpConnection::Http2(v))
			} else if was_at_max {
				self.active_h2.mark_active_by_load(&load);
			}
		}
	}
	fn return_connection(
		&mut self,
		settings: Arc<PoolSettings>,
		pool: Arc<Mutex<HashMap<K, HostPool<K>>>>,
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
			to_notify -= 1;
			// For the first, notify with the original error. The rest get an error to just retry.
			loop {
				let Some(tx) = self.waiters.pop_front() else {
					break;
				};
				if tx.is_canceled() {
					trace!("insert new error; removing canceled waiter for {:?}", key);
					continue;
				}
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
		pool: Arc<Mutex<HashMap<K, HostPool<K>>>>,
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
			hosts: Arc::new(Mutex::new(HashMap::new())),
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
	pool: Weak<Mutex<HashMap<K, HostPool<K>>>>,
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
			let mut hosts = Pool::lock_hosts(&pool, &inner.key);
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

	fn ensure_idle_interval(
		pool: &Arc<Mutex<HashMap<K, HostPool<K>>>>,
		settings: &Arc<PoolSettings>,
	) {
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
		pool: &Arc<Mutex<HashMap<K, HostPool<K>>>>,
		settings: &Arc<PoolSettings>,
		original_con: HttpConnection,
	) {
		let mut conn = Some(original_con);
		let mut sent = 0;
		while capacity > 0 {
			let Some(tx) = host.waiters.pop_front() else {
				break;
			};
			if tx.is_canceled() {
				trace!("insert new; removing canceled waiter for {:?}", key);
				continue;
			}

			let Some(raw_conn) = conn.take() else {
				break;
			};
			let pooled = Pooled {
				value: Some((key.clone(), raw_conn)),
				is_reused: reason == "idle",
				pool: Arc::downgrade(pool),
				settings: settings.clone(),
			};
			let (this, next) = pooled.maybe_clone();
			if let Some((mut nc, full)) = next {
				conn = nc.value.take().map(|(_, c)| c);
				if full && let Some(HttpConnection::Http2(h2)) = conn.as_ref() {
					host.active_h2.mark_full(h2);
				}
			}
			capacity -= 1;
			match tx.send(Ok(this)) {
				Ok(()) => {
					sent += 1;
				},
				Err(Ok(mut e)) => {
					trace!("send failed");
					// Recover the connection without dropping the pooled wrapper
					// while the host lock is still held.
					// We verify its Ok() explicitly above
					conn = e.value.take().map(|(_, c)| c);
				},
				Err(_) => unreachable!("Ok() always above"),
			}
		}
		trace!(fulfilled=%sent, "sent {reason} connection");
		if sent == 0
			&& let Some(c) = conn
		{
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
					is_reused: false,
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
	pool: Weak<Mutex<HashMap<K, HostPool<K>>>>,
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
	pub(crate) fn into_guard(mut self) -> H2CapacityGuard<K> {
		H2CapacityGuard {
			value: self.value.take().map(|(k, v)| {
				let HttpConnection::Http2(h2) = v else {
					panic!("into_guard must be used on http2")
				};
				(k, h2.load)
			}),
			pool: self.pool.clone(),
			settings: self.settings.clone(),
		}
	}
	fn maybe_clone(self) -> (Self, Option<(Self, bool)>) {
		match self.value.as_ref() {
			Some((_, HttpConnection::Http1(_h))) => {
				// HTTP/1.1 cannot be cloned
				(self, None)
			},
			Some((k, HttpConnection::Http2(h))) => {
				// HTTP/2 can be cloned unless its at-capacity.
				let cpy = h.clone_increment_load().map(|(c, full)| {
					(
						Self {
							value: Some((k.clone(), HttpConnection::Http2(c))),
							is_reused: true,
							pool: self.pool.clone(),
							settings: self.settings.clone(),
						},
						full,
					)
				});
				(self, cpy)
			},
			None => (self, None),
		}
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
		if let Some((k, value)) = self.value.take() {
			if !value.is_open() {
				trace!("connection already closed; skip idle pool insertion");
				// If we *already* know the connection is done here,
				// it shouldn't be re-inserted back into the pool.
				return;
			}

			if let Some(pool) = self.pool.upgrade() {
				let mut hosts = Pool::lock_hosts(&pool, &k);
				trace!(key=?k, "returning connection to pool");
				hosts.return_connection(self.settings.clone(), pool.clone(), k, value);
			} else {
				trace!("pool dropped, dropping pooled ({:?})", k);
			}
		}
	}
}

impl<K: Key> Drop for H2CapacityGuard<K> {
	fn drop(&mut self) {
		if let Some((k, v)) = self.value.take() {
			if let Some(pool) = self.pool.upgrade() {
				let mut hosts = Pool::lock_hosts(&pool, &k);
				trace!(key=?k, "returning connection to pool");
				hosts.return_h2_stream(self.settings.clone(), pool.clone(), k, v);
			} else {
				trace!("pool dropped, dropping pooled ({:?})", k);
			}
		}
	}
}

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
			// TODO see this too much
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
		pool: Weak<Mutex<HashMap<K, HostPool<K>>>>,
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
				let mut l = inner.lock();
				trace!("idle interval checking for expired");
				Pool::clear_expired(this.settings, &mut l);
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
		let pool = Pool::new(
			super::Config {
				idle_timeout: Some(Duration::from_millis(100)),
				max_idle_per_host: max_idle,
				expected_http2_capacity: DEFAULT_EXPECTED_HTTP2_CAPACITY,
			},
			TokioExecutor::new(),
			MockTimer::default(),
		);
		pool
	}

	fn pool_with_idle_timeout<K: Key>(idle_timeout: Duration) -> Pool<K> {
		init_test_tracing();
		let pool = Pool::new(
			super::Config {
				idle_timeout: Some(idle_timeout),
				max_idle_per_host: usize::MAX,
				expected_http2_capacity: DEFAULT_EXPECTED_HTTP2_CAPACITY,
			},
			TokioExecutor::new(),
			MockTimer::default(),
		);
		pool
	}

	fn pool_with_expected_h2_capacity_idle<K: Key>(
		expected_http2_capacity: usize,
		idle: Duration,
	) -> Pool<K> {
		init_test_tracing();
		let pool = Pool::new(
			super::Config {
				idle_timeout: Some(idle),
				max_idle_per_host: usize::MAX,
				expected_http2_capacity,
			},
			TokioExecutor::new(),
			MockTimer::default(),
		);
		pool
	}
	fn pool_with_expected_h2_capacity<K: Key>(expected_http2_capacity: usize) -> Pool<K> {
		pool_with_expected_h2_capacity_idle(expected_http2_capacity, Duration::from_secs(10))
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
