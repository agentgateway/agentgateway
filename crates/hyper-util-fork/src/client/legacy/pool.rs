use hashbrown::HashMap;
use hashbrown::hash_map::EntryRef;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::error::Error as StdError;
use std::fmt::{self, Debug};
use std::future::Future;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use crate::client::legacy::connect::Connected;
use crate::common::exec;
use crate::common::exec::Exec;
use crate::common::timer::Timer;
use futures_channel::oneshot;
use futures_core::ready;
use futures_util::future::Either;
use http::{Request, Response};
use hyper::rt::{Sleep, Timer as _};
use tracing::{debug, trace, warn};

#[derive(Clone)]
pub struct Pool<K: Key> {
	hosts: Arc<Mutex<HashMap<K, HostPool<K>>>>,
	settings: Arc<PoolSettings>,
}

pub struct PoolSettings {
	max_idle_per_host: usize,
	// A oneshot channel is used to allow the interval to be notified when
	// the Pool completely drops. That way, the interval can cancel immediately.
	idle_interval_ref: Option<oneshot::Sender<Infallible>>,
	exec: Exec,
	timer: Timer,
	timeout: Option<Duration>,
}

impl<K: Key> Pool<K> {
	pub fn lock_hosts<'a>(
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
	pub fn host(&self, k: &K) -> MappedMutexGuard<'_, HostPool<K>> {
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
	Cached(usize),
}

impl CapacityCache {
	fn expected_capacity(&self) -> usize {
		match self {
			CapacityCache::Guess(ExpectedCapacity::Http1) => 1,
			CapacityCache::Guess(ExpectedCapacity::Http2) => 100,
			// Currently, we are pessimistically assuming that the connection will be HTTP/1.1
			CapacityCache::Guess(ExpectedCapacity::Auto) => 1,
			CapacityCache::Cached(exact) => *exact,
		}
	}
}

#[derive(Default)]
struct H2Pool(VecDeque<ReservedHttp2Connection>);

impl H2Pool {
	fn return_active(&mut self, c: ReservedHttp2Connection) {
		// Push to the front of the queue; it will be the next connection to get used.
		self.0.push_front(c)
	}
	/// maybe_insert_new inserts the connection as an active one (if it is HTTP2).
	fn maybe_insert_new(&mut self, conn: HttpConnection, reserve: bool) -> HttpConnection {
		if let HttpConnection::Http2(h) = conn {
			self.0.push_front(h.clone());
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
		let h = self.0.front()?;
		match h.load.try_reserve_stream_slot() {
			CapacityReservationResult::NoCapacity => None,
			CapacityReservationResult::ReservedAndFilled => {
				let ret = Some(ReservedHttp2Connection {
					info: h.info.clone(),
					tx: h.tx.clone(),
					load: h.load.clone(),
				});
				// Move the connection to the back of the queue.
				self.0.swap_remove_back(0);
				ret
			},
			CapacityReservationResult::ReservedButNotFilled => {
				// Keep the connection at the front.
				Some(ReservedHttp2Connection {
					info: h.info.clone(),
					tx: h.tx.clone(),
					load: h.load.clone(),
				})
			},
		}
	}
}

pub(crate) struct ReservedHttp1Connection {
	pub(crate) info: Connected,
	pub(crate) tx: hyper::client::conn::http1::SendRequest<axum_core::body::Body>,
}

pub enum HttpConnection {
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

#[derive(Clone)]
pub(crate) struct ReservedHttp2Connection {
	pub(crate) info: Connected,
	pub(crate) tx: hyper::client::conn::http2::SendRequest<axum_core::body::Body>,
	pub(crate) load: Arc<H2Load>,
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
	fn set_max_streams(&self, max_streams: usize) {
		self
			.max_streams
			.store(max_streams.max(1), Ordering::Release);
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

	fn release_stream_slot(&self) -> usize {
		let prev = self.active_streams.fetch_sub(1, Ordering::AcqRel);
		debug_assert!(prev > 0, "active_streams must be > 0 before release");
		prev - 1
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
	waiters: VecDeque<oneshot::Sender<Pooled<K>>>,
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
	fn return_connection(&mut self, settings: Arc<PoolSettings>, pool: Arc<Mutex<HashMap<K, HostPool<K>>>>, k: K, value: HttpConnection) {
		match value {
			HttpConnection::Http1(h) => {
				self.return_idle(settings, pool, k, HttpConnection::Http1(h))
			},
			HttpConnection::Http2(h) => {
				let remaining = h.load.release_stream_slot();
				if remaining == 0 {
					self.return_idle(settings, pool, k, HttpConnection::Http2(h))
				} else {
					self.active_h2.return_active(h);
				}
			},
		}
	}
	pub fn return_idle(&mut self,  settings: Arc<PoolSettings>, pool: Arc<Mutex<HashMap<K, HostPool<K>>>>, key: K, conn: HttpConnection) {
		let mut p = Some(Pooled {
			value: Some((key, conn)),
			is_reused: false,
			pool: Arc::downgrade(&pool),
			settings: settings.clone(),
		});
		trace!(waiters=%self.waiters.len(), "return idle");
		let mut capacity = 1;
		let mut sent = 0;
		// First, send to any waiters...
		while capacity > 0
			&& p.is_some()
			&& let Some(tx) = self.waiters.pop_front()
		{
			let Some(pv) = p else {
				panic!("verified above")
			};
			let (this, next) = pv.maybe_clone();
			p = next;
			capacity -= 1;
			if tx.is_canceled() {
				trace!(
					"insert new; removing canceled waiter for {:?}",
					this.value.as_ref().map(|v| &v.0)
				);
				continue;
			}
			match tx.send(this) {
				Ok(()) => {
					sent += 1;
				},
				Err(e) => {
					trace!("send failed");
					// If this was HTTP/2, we have 2 fungible copies and its fine to drop `next`.
					// If its HTTP/1.1, however, this is the only copy so we must return it back
					p = Some(e)
				},
			}
		}
		trace!(fulfilled=%sent, "sent idle connection");
		// Nobody is waiting but we got a connection..as
		let now = settings.timer.now();
		if sent == 0
			&& let Some(mut pv) = p
		&&  let Some((k, c)) = pv.value.take()
		{
			// TODO max_idle_per_host
			debug!("pooling idle connection for {:?}", k);
			self.idle.push(Idle {
				value: c,
				idle_at: now
			});
			// TODO
			// self.spawn_idle_interval(__pool_ref);
		}
	}
}

// This is because `Weak::new()` *allocates* space for `T`, even if it
// doesn't need it!
struct WeakOpt<T>(Option<Weak<T>>);

#[derive(Clone, Copy, Debug)]
pub struct Config {
	pub idle_timeout: Option<Duration>,
	pub max_idle_per_host: usize,
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
				idle_interval_ref: None,
				max_idle_per_host: config.max_idle_per_host,
				exec,
				timer,
				timeout: config.idle_timeout,
			}),
		}
	}
}

// pub(crate) enum ConnectionGuidance {
// 	// You should connect, and we assumed that the connection would handle
// 	Connect(bool)
// }

pub(crate) struct WaitForConnection<K: Key> {
	pub should_connect: bool,
	key: K,
	pub waiter: oneshot::Receiver<Pooled<K>>,
}

pub(crate) enum CheckoutResult<K: Key> {
	Checkout(Pooled<K>),
	Wait(WaitForConnection<K>),
}

impl<K: Key> Pool<K> {
	pub fn insert_new_connection(&self, key: K, conn: HttpConnection) {
		let mut host = self.host(&key);
		let mut capacity = conn.capacity();
		host.connecting -= 1;
		host.expected_connecting_capacity -= capacity;
		trace!(?key, ?host.connecting, %host.expected_connecting_capacity, "inserting new connection");
		let mut sent = 0;
		let mut conn = Some(host.active_h2.maybe_insert_new(conn, false));
		trace!(waiters=%host.waiters.len(), "insert new");
		// First, send to any waiters...
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
				is_reused: false,
				pool: Arc::downgrade(&self.hosts),
				settings: self.settings.clone(),
			};
			let (this, next) = pooled.maybe_clone();
			conn = next.and_then(|mut pooled| pooled.value.take().map(|(_, c)| c));
			capacity -= 1;
			match tx.send(this) {
				Ok(()) => {
					sent += 1;
				},
				Err(mut e) => {
					trace!("send failed");
					// Recover the connection without dropping the pooled wrapper
					// while the host lock is still held.
					conn = e.value.take().map(|(_, c)| c);
				},
			}
		}
		trace!(fulfilled=%sent, "sent new connection");
		// Nobody is waiting but we got a connection..
		if sent == 0 {
			warn!("dropping connection on the floor")
		}
		// TODO(john): insert as idle insert
	}
	pub fn checkout_or_register_waker(&self, key: K) -> CheckoutResult<K> {
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
		let should_connect = pending <= waiters;
		if should_connect {
			// We need more capacity! Start a connection
			// We will assume the caller is actually going to do this
			host.connecting += 1;
			host.expected_connecting_capacity += host.per_connection_capacity_cache.expected_capacity();
		}
		trace!(%should_connect, "no active or idle connections available");
		let (tx, mut rx) = oneshot::channel();
		trace!("checkout waiting for idle connection: {:?}", key);
		host.waiters.push_back(tx);
		CheckoutResult::Wait(WaitForConnection {
			key,
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

/*
impl<T: Poolable, K: Key> PoolInner<T, K> {
	fn spawn_idle_interval(&mut self, pool_ref: &Arc<Mutex<PoolInner<T, K>>>) {
		if self.idle_interval_ref.is_some() {
			return;
		}
		let dur = if let Some(dur) = self.timeout {
			dur
		} else {
			return;
		};
		let timer = if let Some(timer) = self.timer.clone() {
			timer
		} else {
			return;
		};
		let (tx, rx) = oneshot::channel();
		self.idle_interval_ref = Some(tx);

		let interval = IdleTask {
			timer: timer.clone(),
			duration: dur,
			deadline: Instant::now(),
			fut: timer.sleep_until(Instant::now()), // ready at first tick
			pool: WeakOpt::downgrade(pool_ref),
			pool_drop_notifier: rx,
		};

		self.exec.execute(interval);
	}
}

impl<T, K: Eq + Hash> PoolInner<T, K> {
	/// Any `FutureResponse`s that were created will have made a `Checkout`,
	/// and possibly inserted into the pool that it is waiting for an idle
	/// connection. If a user ever dropped that future, we need to clean out
	/// those parked senders.
	fn clean_waiters(&mut self, key: &K) {
		let mut remove_waiters = false;
		if let Some(waiters) = self.waiters.get_mut(key) {
			waiters.retain(|tx| !tx.is_canceled());
			remove_waiters = waiters.is_empty();
		}
		if remove_waiters {
			self.waiters.remove(key);
		}
	}
}

impl<T: Poolable, K: Key> PoolInner<T, K> {
	/// This should *only* be called by the IdleTask
	fn clear_expired(&mut self) {
		let dur = self.timeout.expect("interval assumes timeout");

		let now = self.now();
		// self.last_idle_check_at = now;

		self.idle.retain(|key, values| {
			values.retain(|entry| {
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

			// returning false evicts this key/val
			!values.is_empty()
		});
	}
}
*/

/// A wrapped poolable value that tries to reinsert to the Pool on Drop.
pub struct Pooled<K: Key> {
	value: Option<(K, HttpConnection)>,
	is_reused: bool,
	pool: Weak<Mutex<HashMap<K, HostPool<K>>>>,
	settings: Arc<PoolSettings>,
}

impl<K: Key> Pooled<K> {}

impl<K: Key> Pooled<K> {
	fn maybe_clone(self) -> (Self, Option<Self>) {
		match self.value.as_ref() {
			Some((_, HttpConnection::Http1(h))) => {
				// HTTP/1.1 cannot be cloned
				(self, None)
			},
			Some((k, HttpConnection::Http2(h))) => {
				// HTTP/2 can be cloned
				let cpy = Some(Self {
					value: Some((k.clone(), HttpConnection::Http2(h.clone()))),
					is_reused: true,
					pool: self.pool.clone(),
					settings: self.settings.clone(),
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
}

impl Error {
	pub(super) fn is_canceled(&self) -> bool {
		matches!(
			self,
			Error::CheckedOutClosedValue | Error::CheckoutNoLongerWanted
		)
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Error::PoolDisabled => "pool is disabled",
			Error::CheckedOutClosedValue => "checked out connection was closed",
			// TODO see this too much
			Error::CheckoutNoLongerWanted => "request was canceled",
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

/*
pin_project_lite::pin_project! {
		struct IdleTask<T, K: Key> {
				timer: Timer,
				duration: Duration,
				deadline: Instant,
				fut: Pin<Box<dyn Sleep>>,
				pool: WeakOpt<Mutex<PoolInner<T, K>>>,
				// This allows the IdleTask to be notified as soon as the entire
				// Pool is fully dropped, and shutdown. This channel is never sent on,
				// but Err(Canceled) will be received when the Pool is dropped.
				#[pin]
				pool_drop_notifier: oneshot::Receiver<Infallible>,
		}
}

impl<T: Poolable + 'static, K: Key> Future for IdleTask<T, K> {
	type Output = ();

	fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
		let mut this = self.project();
		loop {
			match this.pool_drop_notifier.as_mut().poll(cx) {
				Poll::Ready(Ok(n)) => match n {},
				Poll::Pending => (),
				Poll::Ready(Err(_canceled)) => {
					trace!("pool closed, canceling idle interval");
					return Poll::Ready(());
				},
			}

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
				if let Ok(mut inner) = inner.lock() {
					trace!("idle interval checking for expired");
					inner.clear_expired();
					continue;
				}
			}
			return Poll::Ready(());
		}
	}
}
*/
impl<T> WeakOpt<T> {
	fn none() -> Self {
		WeakOpt(None)
	}

	fn downgrade(arc: &Arc<T>) -> Self {
		WeakOpt(Some(Arc::downgrade(arc)))
	}

	fn upgrade(&self) -> Option<Arc<T>> {
		self.0.as_ref().and_then(Weak::upgrade)
	}
}

#[cfg(all(test, not(miri)))]
mod tests {
	use std::fmt::Debug;
	use std::future::Future;
	use std::hash::Hash;
	use std::pin::Pin;
	use std::sync::Arc;
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::task::{self, Poll};
	use std::time::Duration;

	use super::{ExpectedCapacity, Key, Pool, WeakOpt};
	use crate::common::timer;
	use crate::rt::{TokioExecutor, TokioTimer};

	#[derive(Clone, Debug, PartialEq, Eq, Hash)]
	struct KeyImpl(http::uri::Scheme, http::uri::Authority);

	impl Key for KeyImpl {
		fn expected_capacity(&self) -> ExpectedCapacity {
			ExpectedCapacity::Http1
		}
	}

	type KeyTuple = (http::uri::Scheme, http::uri::Authority);

	/// Test unique reservations.
	#[derive(Debug, PartialEq, Eq)]
	struct Uniq<T>(T);

	impl<T: Send + 'static + Unpin> Poolable for Uniq<T> {
		fn is_open(&self) -> bool {
			true
		}

		fn reserve(self) -> Reservation<Self> {
			Reservation::Unique(self)
		}

		fn can_share(&self) -> bool {
			false
		}
	}

	#[derive(Clone, Debug)]
	struct SharedConn {
		id: i32,
		available: Arc<AtomicBool>,
	}

	impl Poolable for SharedConn {
		fn is_open(&self) -> bool {
			true
		}

		fn reserve(self) -> Reservation<Self> {
			if !self.available.load(Ordering::Acquire) {
				return Reservation::Unavailable(self);
			}

			let to_return = self.clone();
			Reservation::Shared(self, to_return)
		}

		fn can_share(&self) -> bool {
			true
		}
	}

	#[derive(Clone, Debug)]
	struct ReservingH2Conn {
		id: i32,
		max_streams: usize,
		current_streams: Arc<std::sync::atomic::AtomicUsize>,
		closed: Arc<AtomicBool>,
	}

	impl ReservingH2Conn {
		fn new(id: i32, max_streams: usize) -> Self {
			Self {
				id,
				max_streams,
				current_streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
				closed: Arc::new(AtomicBool::new(false)),
			}
		}

		fn current_streams(&self) -> usize {
			self
				.current_streams
				.load(std::sync::atomic::Ordering::Relaxed)
		}

		fn release_stream(&self) {
			let prev = self
				.current_streams
				.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
			assert!(prev > 0, "must have an active stream to release");
		}

		fn try_reserve_stream(&self) -> bool {
			self
				.current_streams
				.fetch_update(
					std::sync::atomic::Ordering::SeqCst,
					std::sync::atomic::Ordering::Relaxed,
					|current| {
						if current >= self.max_streams {
							None
						} else {
							Some(current + 1)
						}
					},
				)
				.is_ok()
		}
	}

	impl Poolable for ReservingH2Conn {
		fn is_open(&self) -> bool {
			!self.closed.load(Ordering::Relaxed)
		}

		fn reserve(self) -> Reservation<Self> {
			if !self.try_reserve_stream() {
				return Reservation::Unavailable(self);
			}

			Reservation::Shared(self.clone(), self)
		}

		fn can_share(&self) -> bool {
			true
		}
	}

	fn c<T: Poolable, K: Key>(key: K) -> Connecting<T, K> {
		Connecting {
			key,
			pool: WeakOpt::none(),
		}
	}

	fn host_key(s: &str) -> KeyImpl {
		KeyImpl(http::uri::Scheme::HTTP, s.parse().expect("host key"))
	}

	fn pool_no_timer<T, K: Key>() -> Pool<T, K> {
		pool_max_idle_no_timer(usize::MAX)
	}

	fn pool_max_idle_no_timer<T, K: Key>(max_idle: usize) -> Pool<T, K> {
		let pool = Pool::new(
			super::Config {
				idle_timeout: Some(Duration::from_millis(100)),
				max_idle_per_host: max_idle,
			},
			TokioExecutor::new(),
			Option::<timer::Timer>::None,
		);
		pool.no_timer();
		pool
	}

	#[tokio::test]
	async fn test_pool_checkout_smoke() {
		let pool = pool_no_timer();
		let key = host_key("foo");
		let pooled = pool.pooled(c(key.clone()), Uniq(41));

		drop(pooled);

		match pool.checkout(key).await {
			Ok(pooled) => assert_eq!(*pooled, Uniq(41)),
			Err(_) => panic!("not ready"),
		};
	}

	/// Helper to check if the future is ready after polling once.
	struct PollOnce<'a, F>(&'a mut F);

	impl<F, T, U> Future for PollOnce<'_, F>
	where
		F: Future<Output = Result<T, U>> + Unpin,
	{
		type Output = Option<()>;

		fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
			match Pin::new(&mut self.0).poll(cx) {
				Poll::Ready(Ok(_)) => Poll::Ready(Some(())),
				Poll::Ready(Err(_)) => Poll::Ready(Some(())),
				Poll::Pending => Poll::Ready(None),
			}
		}
	}

	#[tokio::test]
	async fn test_pool_checkout_returns_none_if_expired() {
		let pool = pool_no_timer();
		let key = host_key("foo");
		let pooled = pool.pooled(c(key.clone()), Uniq(41));

		drop(pooled);
		let to = pool.locked().timeout.unwrap();
		tokio::time::sleep(to).await;
		let mut checkout = pool.checkout(key);
		let poll_once = PollOnce(&mut checkout);
		let is_not_ready = poll_once.await.is_none();
		assert!(is_not_ready);
	}

	#[tokio::test]
	async fn test_pool_checkout_removes_expired() {
		let pool = pool_no_timer();
		let key = host_key("foo");

		pool.pooled(c(key.clone()), Uniq(41));
		pool.pooled(c(key.clone()), Uniq(5));
		pool.pooled(c(key.clone()), Uniq(99));

		assert_eq!(
			pool.locked().idle.get(&key).map(|entries| entries.len()),
			Some(3)
		);
		let to = pool.locked().timeout.unwrap();
		tokio::time::sleep(to).await;

		let mut checkout = pool.checkout(key.clone());
		let poll_once = PollOnce(&mut checkout);
		// checkout.await should clean out the expired
		poll_once.await;
		assert!(!pool.locked().idle.contains_key(&key));
	}

	#[test]
	fn test_pool_max_idle_per_host() {
		let pool = pool_max_idle_no_timer(2);
		let key = host_key("foo");

		pool.pooled(c(key.clone()), Uniq(41));
		pool.pooled(c(key.clone()), Uniq(5));
		pool.pooled(c(key.clone()), Uniq(99));

		// pooled and dropped 3, max_idle should only allow 2
		assert_eq!(
			pool.locked().idle.get(&key).map(|entries| entries.len()),
			Some(2)
		);
	}

	#[tokio::test]
	async fn test_pool_timer_removes_expired() {
		let pool = Pool::new(
			super::Config {
				idle_timeout: Some(Duration::from_millis(10)),
				max_idle_per_host: usize::MAX,
			},
			TokioExecutor::new(),
			Some(TokioTimer::new()),
		);

		let key = host_key("foo");

		pool.pooled(c(key.clone()), Uniq(41));
		pool.pooled(c(key.clone()), Uniq(5));
		pool.pooled(c(key.clone()), Uniq(99));

		assert_eq!(
			pool.locked().idle.get(&key).map(|entries| entries.len()),
			Some(3)
		);

		// Let the timer tick passed the expiration...
		tokio::time::sleep(Duration::from_millis(30)).await;
		// Yield so the Interval can reap...
		tokio::task::yield_now().await;

		assert!(!pool.locked().idle.contains_key(&key));
	}

	#[tokio::test]
	async fn test_pool_checkout_task_unparked() {
		use futures_util::FutureExt;
		use futures_util::future::join;

		let pool = pool_no_timer();
		let key = host_key("foo");
		let pooled = pool.pooled(c(key.clone()), Uniq(41));

		let checkout = join(pool.checkout(key), async {
			// the checkout future will park first,
			// and then this lazy future will be polled, which will insert
			// the pooled back into the pool
			//
			// this test makes sure that doing so will unpark the checkout
			drop(pooled);
		})
		.map(|(entry, _)| entry);

		assert_eq!(*checkout.await.unwrap(), Uniq(41));
	}

	#[tokio::test]
	async fn test_pool_checkout_drop_cleans_up_waiters() {
		let pool = pool_no_timer::<Uniq<i32>, KeyImpl>();
		let key = host_key("foo");

		let mut checkout1 = pool.checkout(key.clone());
		let mut checkout2 = pool.checkout(key.clone());

		let poll_once1 = PollOnce(&mut checkout1);
		let poll_once2 = PollOnce(&mut checkout2);

		// first poll needed to get into Pool's parked
		poll_once1.await;
		assert_eq!(pool.locked().waiters.get(&key).unwrap().len(), 1);
		poll_once2.await;
		assert_eq!(pool.locked().waiters.get(&key).unwrap().len(), 2);

		// on drop, clean up Pool
		drop(checkout1);
		assert_eq!(pool.locked().waiters.get(&key).unwrap().len(), 1);

		drop(checkout2);
		assert!(!pool.locked().waiters.contains_key(&key));
	}

	#[derive(Debug)]
	struct CanClose {
		#[allow(unused)]
		val: i32,
		closed: bool,
	}

	impl Poolable for CanClose {
		fn is_open(&self) -> bool {
			!self.closed
		}

		fn reserve(self) -> Reservation<Self> {
			Reservation::Unique(self)
		}

		fn can_share(&self) -> bool {
			false
		}
	}

	#[test]
	fn pooled_drop_if_closed_doesnt_reinsert() {
		let pool = pool_no_timer();
		let key = host_key("foo");
		pool.pooled(
			c(key.clone()),
			CanClose {
				val: 57,
				closed: true,
			},
		);

		assert!(!pool.locked().idle.contains_key(&key));
	}

	#[test]
	fn test_pool_allows_multiple_http2_idle_connections() {
		let pool = pool_no_timer::<SharedConn, KeyImpl>();
		let key = host_key("foo");

		let connecting1 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("must create connecting lock");
		pool.pooled(
			connecting1,
			SharedConn {
				id: 1,
				available: Arc::new(AtomicBool::new(true)),
			},
		);
		let connecting2 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("must create connecting lock");
		pool.pooled(
			connecting2,
			SharedConn {
				id: 2,
				available: Arc::new(AtomicBool::new(true)),
			},
		);

		assert_eq!(
			pool.locked().idle.get(&key).map(|entries| entries.len()),
			Some(2)
		);
	}

	#[tokio::test]
	async fn test_pool_checkout_skips_unavailable_shared_connection() {
		let pool = pool_no_timer::<SharedConn, KeyImpl>();
		let key = host_key("foo");
		let unavailable = Arc::new(AtomicBool::new(true));

		let connecting1 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("must create connecting lock");
		pool.pooled(
			connecting1,
			SharedConn {
				id: 2,
				available: Arc::new(AtomicBool::new(true)),
			},
		);
		let connecting2 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("must create connecting lock");
		pool.pooled(
			connecting2,
			SharedConn {
				id: 1,
				available: unavailable.clone(),
			},
		);

		unavailable.store(false, Ordering::Release);

		let pooled = pool.checkout(key).await.expect("checkout should succeed");
		assert_eq!(pooled.id, 2);
	}

	#[tokio::test]
	async fn test_h2_overestimated_waiters_wake_when_capacity_returns() {
		let pool = pool_no_timer::<ReservingH2Conn, KeyImpl>();
		let key = host_key("foo");
		let conn = ReservingH2Conn::new(1, 1);

		let connecting = match pool.h2_acquire(key.clone(), 100) {
			H2Acquire::Connecting(connecting) => connecting,
			_ => panic!("expected connecting"),
		};
		let mut waiter1: Pin<Box<_>> = Box::pin(match pool.h2_acquire(key.clone(), 100) {
			H2Acquire::Checkout(checkout) => checkout,
			_ => panic!("expected checkout"),
		});
		let _waiter2 = match pool.h2_acquire(key.clone(), 100) {
			H2Acquire::Checkout(checkout) => checkout,
			_ => panic!("expected checkout"),
		};

		let _pooled = pool.pooled(connecting, conn.clone());
		assert_eq!(conn.current_streams(), 1);
		assert_eq!(pool.locked().waiters.get(&key).unwrap().len(), 2);
		assert!(futures_util::poll!(&mut waiter1).is_pending());

		conn.release_stream();
		assert_eq!(conn.current_streams(), 0);

		let poll_after_release = futures_util::poll!(&mut waiter1);
		assert!(
			matches!(poll_after_release, Poll::Ready(Ok(_))),
			"parked waiter should wake once H2 stream capacity returns, got {:?}",
			poll_after_release,
		);
	}

	// ===== HTTP/2 Max Streams Tests =====

	/// Mock HTTP/2 connection with configurable max streams and stream tracking
	#[derive(Debug, Clone)]
	struct H2Connection {
		id: u64,
		max_streams: usize,
		current_streams: Arc<std::sync::atomic::AtomicUsize>,
		closed: Arc<std::sync::atomic::AtomicBool>,
	}

	impl H2Connection {
		fn new(id: u64, max_streams: usize) -> Self {
			Self {
				id,
				max_streams,
				current_streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
				closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
			}
		}

		fn close(&self) {
			self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
		}

		fn current_streams(&self) -> usize {
			self.current_streams.load(Ordering::Relaxed)
		}

		fn has_capacity(&self) -> bool {
			self.current_streams() < self.max_streams
		}

		fn increment_streams(&self) -> bool {
			self
				.current_streams
				.fetch_update(Ordering::SeqCst, Ordering::Relaxed, |current| {
					if current >= self.max_streams {
						None
					} else {
						Some(current + 1)
					}
				})
				.is_ok()
		}

		fn decrement_streams(&self) {
			self.current_streams.fetch_sub(1, Ordering::SeqCst);
		}
	}

	impl PartialEq for H2Connection {
		fn eq(&self, other: &Self) -> bool {
			self.id == other.id
		}
	}

	impl Eq for H2Connection {}

	impl Poolable for H2Connection {
		fn is_open(&self) -> bool {
			!self.closed.load(std::sync::atomic::Ordering::Relaxed)
		}

		fn reserve(self) -> Reservation<Self> {
			if !self.has_capacity() {
				return Reservation::Unavailable(self);
			}

			// HTTP/2 connections are shared while they still have stream capacity.
			Reservation::Shared(self.clone(), self)
		}

		fn can_share(&self) -> bool {
			true
		}
	}

	#[tokio::test]
	async fn test_h2_single_stream_connection_stays_unavailable_while_in_flight() {
		// Test that a max_streams=1 connection cannot be reused while its only
		// stream is still in flight.
		let pool = pool_no_timer();
		let key = host_key("h2-host");

		// Get a proper connecting lock for HTTP/2
		let connecting = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should get connecting lock");

		// Put connection in pool
		let pooled = pool.pooled(connecting, H2Connection::new(1, 1));

		// Simulate taking a stream
		assert!(pooled.increment_streams());
		assert_eq!(pooled.current_streams(), 1);

		// Connection should be full, while still remaining open/alive.
		assert!(pooled.is_open());
		assert!(!pooled.has_capacity());

		// Try to checkout - should pend since connection is full
		let checkout = pool.checkout(key.clone());
		let mut checkout_boxed = Box::pin(checkout);
		let poll_result = futures_util::poll!(&mut checkout_boxed);
		assert!(
			poll_result.is_pending(),
			"checkout should pend when connection is full"
		);
	}

	#[tokio::test]
	async fn test_h2_multiple_streams_single_connection() {
		// Test that a connection with max_streams=3 can handle multiple concurrent streams
		let pool = pool_no_timer();
		let key = host_key("h2-host");
		let conn = H2Connection::new(1, 3);

		let connecting = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should get connecting lock");
		let pooled1 = pool.pooled(connecting, conn.clone());
		assert!(pooled1.increment_streams());

		// Connection should still be open and have spare capacity.
		assert!(pooled1.is_open());
		assert!(pooled1.has_capacity());
		drop(pooled1);

		// Should be able to checkout the same connection
		let pooled2 = pool
			.checkout(key.clone())
			.await
			.expect("should reuse connection");
		assert_eq!(pooled2.id, 1);
		assert!(pooled2.increment_streams());
		assert_eq!(pooled2.current_streams(), 2);

		// Still has capacity
		assert!(pooled2.is_open());
		assert!(pooled2.has_capacity());
		drop(pooled2);

		// Third stream
		let pooled3 = pool
			.checkout(key.clone())
			.await
			.expect("should reuse connection");
		assert_eq!(pooled3.id, 1);
		assert!(pooled3.increment_streams());
		assert_eq!(pooled3.current_streams(), 3);

		// Now full, but still open.
		assert!(pooled3.is_open());
		assert!(!pooled3.has_capacity());

		// Try to checkout - should pend since connection is full
		let checkout = pool.checkout(key.clone());
		let mut checkout_boxed = Box::pin(checkout);
		let poll_result = futures_util::poll!(&mut checkout_boxed);
		assert!(
			poll_result.is_pending(),
			"checkout should pend when connection is full"
		);
	}

	#[tokio::test]
	async fn test_h2_stream_release_makes_connection_available() {
		// Test that decrementing streams makes a full connection available again
		let pool = pool_no_timer();
		let key = host_key("h2-host");
		let conn = H2Connection::new(1, 2);

		let connecting = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should get connecting lock");
		let pooled1 = pool.pooled(connecting, conn.clone());
		assert!(pooled1.increment_streams());
		let pooled2 = pool
			.checkout(key.clone())
			.await
			.expect("should get same connection");
		assert!(pooled2.increment_streams());

		// Connection should be full, but still open.
		assert!(pooled2.is_open());
		assert!(!pooled2.has_capacity());
		assert_eq!(pooled2.current_streams(), 2);

		// Release one stream
		pooled1.decrement_streams();

		// Connection should be available again
		assert!(pooled2.is_open());
		assert!(pooled2.has_capacity());
		assert_eq!(pooled2.current_streams(), 1);
	}

	#[tokio::test]
	async fn test_h2_multiple_connections_when_full() {
		// Test that a second connection is established when the first is full
		// This is the core behavior we're implementing
		let pool = pool_no_timer();
		let key = host_key("h2-host");

		// First connection with max_streams=2
		let conn1 = H2Connection::new(1, 2);
		let connecting1 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should get connecting lock");
		let pooled1 = pool.pooled(connecting1, conn1.clone());
		assert!(pooled1.increment_streams());

		let pooled2 = pool
			.checkout(key.clone())
			.await
			.expect("should reuse conn1");
		assert_eq!(pooled2.id, 1);
		assert!(pooled2.increment_streams());

		// conn1 is now full, but still open.
		assert!(pooled2.is_open());
		assert!(!pooled2.has_capacity());
		assert_eq!(pooled2.current_streams(), 2);

		// Next checkout should allow a new connection since conn1 is full
		let connecting2 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should allow second connection when first is full");
		let conn2 = H2Connection::new(2, 2);
		let pooled3 = pool.pooled(connecting2, conn2.clone());
		assert_eq!(pooled3.id, 2);
		assert!(pooled3.increment_streams());
	}

	#[tokio::test]
	async fn test_h2_lifo_connection_selection() {
		// Test that most-recently-used connection is selected (LIFO/stack behavior)
		let pool = pool_no_timer();
		let key = host_key("h2-host");

		// Create two connections, both with capacity
		let conn1 = H2Connection::new(1, 5);
		let conn2 = H2Connection::new(2, 5);

		let connecting1 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should get connecting lock");
		let pooled1 = pool.pooled(connecting1, conn1.clone());
		assert!(pooled1.increment_streams());
		drop(pooled1);

		let connecting2 = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should allow second connection");
		let pooled2 = pool.pooled(connecting2, conn2.clone());
		assert!(pooled2.increment_streams());
		drop(pooled2);

		// Next checkout should get conn2 (most recently used/inserted)
		let pooled3 = pool
			.checkout(key.clone())
			.await
			.expect("should get connection");
		assert_eq!(pooled3.id, 2, "should select most recently used connection");
	}

	#[tokio::test]
	async fn test_h2_closed_connection_not_reused() {
		// Test that closed connections are not returned from the pool
		let pool = pool_no_timer();
		let key = host_key("h2-host");
		let conn = H2Connection::new(1, 5);

		let connecting = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should get connecting lock");
		let pooled = pool.pooled(connecting, conn.clone());

		// Close the connection
		pooled.close();
		drop(pooled);

		// Checkout should not get the closed connection
		let checkout = pool.checkout(key.clone());
		let mut checkout_boxed = Box::pin(checkout);
		let poll_result = futures_util::poll!(&mut checkout_boxed);
		assert!(
			poll_result.is_pending(),
			"should not return closed connection"
		);
	}

	#[tokio::test]
	async fn test_h2_stream_count_boundary_conditions() {
		// Test edge cases around stream counting
		let pool = pool_no_timer();
		let key = host_key("h2-host");
		let conn = H2Connection::new(1, 100);

		let connecting = pool
			.connecting(key.clone(), Ver::Http2, 100)
			.expect("should get connecting lock");
		let pooled = pool.pooled(connecting, conn.clone());

		// Fill up to max_streams
		for i in 0..100 {
			assert!(
				pooled.increment_streams(),
				"should accept stream {} of 100",
				i + 1
			);
		}

		// Should reject 101st stream
		assert!(
			!pooled.increment_streams(),
			"should reject stream beyond max"
		);
		assert_eq!(pooled.current_streams(), 100);

		// Connection should be full, but still open.
		assert!(pooled.is_open());
		assert!(!pooled.has_capacity());

		// Decrement one
		pooled.decrement_streams();
		assert_eq!(pooled.current_streams(), 99);
		assert!(pooled.is_open());
		assert!(pooled.has_capacity());

		// Should accept another stream now
		assert!(pooled.increment_streams());
		assert_eq!(pooled.current_streams(), 100);
	}
}
