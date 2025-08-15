use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use rustls::pki_types::ServerName;
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, watch};
use tonic::async_trait;
use tracing::{Instrument, debug, trace};
use {flurry, pingora_pool};

use super::{Config, Key};
use crate::H2Stream;
use crate::client::H2ConnectClient;

#[async_trait]
pub trait CertificateFetcher<K>: Send + Sync {
	async fn fetch_certificate(&self, key: K) -> anyhow::Result<Arc<rustls::client::ClientConfig>>;
}

// A relatively nonstandard HTTP/2 connection pool designed to allow multiplexing proxied workload connections
// over a (smaller) number of HTTP/2 mTLS tunnels.
//
// The following invariants apply to this pool:
// - Every workload (inpod mode) gets its own connpool.
// - Every unique src/dest key gets their own dedicated connections inside the pool.
// - Every unique src/dest key gets 1-n dedicated connections, where N is (currently) unbounded but practically limited
//   by flow control throttling.
#[derive(Clone)]
pub struct WorkloadHBONEPool<K> {
	state: Arc<PoolState<K>>,
	pool_watcher: watch::Receiver<bool>,
}

impl<K> Debug for WorkloadHBONEPool<K> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WorkloadHBONEPool").finish()
	}
}

// PoolState is effectively the gnarly inner state stuff that needs thread/task sync, and should be wrapped in a Mutex.
struct PoolState<K> {
	pool_notifier: watch::Sender<bool>, /* This is already impl clone? rustc complains that it isn't, tho */
	timeout_tx: watch::Sender<bool>, /* This is already impl clone? rustc complains that it isn't, tho */
	// this is effectively just a convenience data type - a rwlocked hashmap with keying and LRU drops
	// and has no actual hyper/http/connection logic.
	connected_pool: Arc<pingora_pool::ConnectionPool<H2ConnectClient<K>>>,
	// this must be an atomic/concurrent-safe list-of-locks, so we can lock per-key, not globally, and avoid holding up all conn attempts
	established_conn_writelock: flurry::HashMap<u64, Option<Arc<Mutex<()>>>>,
	pool_unused_release_timeout: Duration,
	// This is merely a counter to track the overall number of conns this pool spawns
	// to ensure we get unique poolkeys-per-new-conn, it is not a limit
	pool_global_conn_count: AtomicI32,
	spawner: ConnSpawner<K>,
}

struct ConnSpawner<K> {
	cfg: Arc<Config>,
	certificates: Arc<dyn CertificateFetcher<K>>,
	timeout_rx: watch::Receiver<bool>,
}

// Does nothing but spawn new conns when asked
impl<K: Key> ConnSpawner<K> {
	async fn new_pool_conn(&self, key: K) -> anyhow::Result<H2ConnectClient<K>> {
		debug!("spawning new pool conn for {}", key);

		let connector = self.certificates.fetch_certificate(key.clone()).await?;
		// TODOMERGE: timeout, nodelay
		let tcp_stream = TcpStream::connect(key.dest())
			.await
			.map_err(|e: io::Error| match e.kind() {
				io::ErrorKind::TimedOut => {
					anyhow::anyhow!(
						"connection timed out, maybe a NetworkPolicy is blocking HBONE port 15008: {e}"
					)
				},
				_ => e.into(),
			})?;

		let tls_stream = connect(connector, tcp_stream).await?;
		trace!("connector connected, handshaking");
		let sender =
			crate::client::spawn_connection(self.cfg.clone(), tls_stream, self.timeout_rx.clone(), key)
				.await?;
		Ok(sender)
	}
}

pub async fn connect<IO>(
	cfg: Arc<rustls::ClientConfig>,
	stream: IO,
) -> Result<tokio_rustls::client::TlsStream<IO>, io::Error>
where
	IO: AsyncRead + AsyncWrite + Unpin,
{
	let c = tokio_rustls::TlsConnector::from(cfg);
	// Use dummy value for domain because it doesn't matter.
	c.connect(
		ServerName::IpAddress(std::net::Ipv4Addr::new(0, 0, 0, 0).into()),
		stream,
	)
	.await
}

impl<K: Key> PoolState<K> {
	// This simply puts the connection back into the inner pool,
	// and sets up a timed popper, which will resolve
	// - when this reference is popped back out of the inner pool (doing nothing)
	// - when this reference is evicted from the inner pool (doing nothing)
	// - when the timeout_idler is drained (will pop)
	// - when the timeout is hit (will pop)
	//
	// Idle poppers are safe to invoke if the conn they are popping is already gone
	// from the inner queue, so we will start one for every insert, let them run or terminate on their own,
	// and poll them to completion on shutdown - any duplicates from repeated checkouts/checkins of the same conn
	// will simply resolve as a no-op in order.
	//
	// Note that "idle" in the context of this pool means "no one has asked for it or dropped it in X time, so prune it".
	//
	// Pruning the idle connection from the pool does not close it - it simply ensures the pool stops holding a ref.
	// hyper self-closes client conns when all refs are dropped and streamcount is 0, so pool consumers must
	// drop their checked out conns and/or terminate their streams as well.
	//
	// Note that this simply removes the client ref from this pool - if other things hold client/streamrefs refs,
	// they must also drop those before the underlying connection is fully closed.
	fn maybe_checkin_conn(&self, conn: H2ConnectClient<K>, pool_key: pingora_pool::ConnectionMeta) {
		if conn.will_be_at_max_streamcount() {
			debug!(
				"checked out connection for {:?} is now at max streamcount; removing from pool",
				pool_key
			);
			return;
		}
		let (evict, pickup) = self.connected_pool.put(&pool_key, conn);
		let rx = self.spawner.timeout_rx.clone();
		let pool_ref = self.connected_pool.clone();
		let pool_key_ref = pool_key.clone();
		let release_timeout = self.pool_unused_release_timeout;
		tokio::spawn(
			async move {
				debug!("starting an idle timeout for connection {:?}", pool_key_ref);
				pool_ref
					.idle_timeout(&pool_key_ref, release_timeout, evict, rx, pickup)
					.await;
				debug!(
					"connection {:?} was removed/checked out/timed out of the pool",
					pool_key_ref
				)
			}
			.in_current_span(),
		);
		let _ = self.pool_notifier.send(true);
	}

	// Since we are using a hash key to do lookup on the inner pingora pool, do a get guard
	// to make sure what we pull out actually deep-equals the workload_key, to avoid *sigh* crossing the streams.
	fn guarded_get(
		&self,
		hash_key: &u64,
		workload_key: &K,
	) -> anyhow::Result<Option<H2ConnectClient<K>>> {
		match self.connected_pool.get(hash_key) {
			None => Ok(None),
			Some(conn) => match Self::enforce_key_integrity(conn, workload_key) {
				Err(e) => Err(e),
				Ok(conn) => Ok(Some(conn)),
			},
		}
	}

	// Just for safety's sake, since we are using a hash thanks to pingora NOT supporting arbitrary Eq, Hash
	// types, do a deep equality test before returning the conn, returning an error if the conn's key does
	// not equal the provided key
	//
	// this is a final safety check for collisions, we will throw up our hands and refuse to return the conn
	fn enforce_key_integrity(
		conn: H2ConnectClient<K>,
		expected_key: &K,
	) -> anyhow::Result<H2ConnectClient<K>> {
		match conn.is_for_workload(expected_key) {
			Ok(()) => Ok(conn),
			Err(e) => Err(e),
		}
	}

	// 1. Tries to get a writelock.
	// 2. If successful, hold it, spawn a new connection, check it in, return a clone of it.
	// 3. If not successful, return nothing.
	//
	// This is useful if we want to race someone else to the writelock to spawn a connection,
	// and expect the losers to queue up and wait for the (singular) winner of the writelock
	//
	// This function should ALWAYS return a connection if it wins the writelock for the provided key.
	// This function should NEVER return a connection if it does not win the writelock for the provided key.
	// This function should ALWAYS propagate Error results to the caller
	//
	// It is important that the *initial* check here is authoritative, hence the locks, as
	// we must know if this is a connection for a key *nobody* has tried to start yet
	// (i.e. no writelock for our key in the outer map)
	// or if other things have already established conns for this key (writelock for our key in the outer map).
	//
	// This is so we can backpressure correctly if 1000 tasks all demand a new connection
	// to the same key at once, and not eagerly open 1000 tunnel connections.
	async fn start_conn_if_win_writelock(
		&self,
		workload_key: &K,
		pool_key: &pingora_pool::ConnectionMeta,
	) -> anyhow::Result<Option<H2ConnectClient<K>>> {
		let inner_conn_lock = {
			trace!("getting keyed lock out of lockmap");
			let guard = self.established_conn_writelock.guard();

			let exist_conn_lock = self
				.established_conn_writelock
				.get(&pool_key.key, &guard)
				.unwrap();
			trace!("got keyed lock out of lockmap");
			exist_conn_lock.as_ref().unwrap().clone()
		};

		trace!("attempting to win connlock for {}", workload_key);

		let inner_lock = inner_conn_lock.try_lock();
		match inner_lock {
			Ok(_guard) => {
				// BEGIN take inner writelock
				debug!("nothing else is creating a conn and we won the lock, make one");
				let client = self.spawner.new_pool_conn(workload_key.clone()).await?;

				debug!(
					"checking in new conn for {} with pk {:?}",
					workload_key, pool_key
				);
				self.maybe_checkin_conn(client.clone(), pool_key.clone());
				Ok(Some(client))
				// END take inner writelock
			},
			Err(_) => {
				debug!(
					"did not win connlock for {}, something else has it",
					workload_key
				);
				Ok(None)
			},
		}
	}

	// Does an initial, naive check to see if we have a writelock inserted into the map for this key
	//
	// If we do, take the writelock for that key, clone (or create) a connection, check it back in,
	// and return a cloned ref, then drop the writelock.
	//
	// Otherwise, return None.
	//
	// This function should ALWAYS return a connection if a writelock exists for the provided key.
	// This function should NEVER return a connection if no writelock exists for the provided key.
	// This function should ALWAYS propagate Error results to the caller
	//
	// It is important that the *initial* check here is authoritative, hence the locks, as
	// we must know if this is a connection for a key *nobody* has tried to start yet
	// (i.e. no writelock for our key in the outer map)
	// or if other things have already established conns for this key (writelock for our key in the outer map).
	//
	// This is so we can backpressure correctly if 1000 tasks all demand a new connection
	// to the same key at once, and not eagerly open 1000 tunnel connections.
	async fn checkout_conn_under_writelock(
		&self,
		workload_key: &K,
		pool_key: &pingora_pool::ConnectionMeta,
	) -> anyhow::Result<Option<H2ConnectClient<K>>> {
		let found_conn = {
			trace!("pool connect outer map - take guard");
			let guard = self.established_conn_writelock.guard();

			trace!("pool connect outer map - check for keyed mutex");
			let exist_conn_lock = self.established_conn_writelock.get(&pool_key.key, &guard);
			exist_conn_lock.and_then(|e_conn_lock| e_conn_lock.clone())
		};
		let Some(exist_conn_lock) = found_conn else {
			return Ok(None);
		};
		debug!(
			"checkout - found mutex for pool key {:?}, waiting for writelock",
			pool_key
		);
		let _conn_lock = exist_conn_lock.as_ref().lock().await;

		trace!(
			"checkout - got writelock for conn with key {} and hash {:?}",
			workload_key, pool_key.key
		);
		let returned_connection = loop {
			match self.guarded_get(&pool_key.key, workload_key)? {
				Some(mut existing) => {
					if !existing.ready_to_use() {
						// We checked this out, and will not check it back in
						// Loop again to find another/make a new one
						debug!(
							"checked out broken connection for {}, dropping it",
							workload_key
						);
						continue;
					}
					debug!("re-using connection for {}", workload_key);
					break existing;
				},
				None => {
					debug!("new connection needed for {}", workload_key);
					break self.spawner.new_pool_conn(workload_key.clone()).await?;
				},
			};
		};

		// For any connection, we will check in a copy and return the other unless its already maxed out
		// TODO: in the future, we can keep track of these and start to use them once they finish some streams.
		self.maybe_checkin_conn(returned_connection.clone(), pool_key.clone());
		Ok(Some(returned_connection))
	}
}

// When the Arc-wrapped PoolState is finally dropped, trigger the drain,
// which will terminate all connection driver spawns, as well as cancel all outstanding eviction timeout spawns
impl<K> Drop for PoolState<K> {
	fn drop(&mut self) {
		debug!(
			"poolstate dropping, stopping all connection drivers and cancelling all outstanding eviction timeout spawns"
		);
		let _ = self.timeout_tx.send(true);
	}
}

impl<K: Key> WorkloadHBONEPool<K> {
	// Creates a new pool instance, which should be owned by a single proxied workload.
	// The pool will watch the provided drain signal and drain itself when notified.
	// Callers should then be safe to drop() the pool instance.
	pub fn new(
		cfg: Arc<crate::Config>,
		local_workload: Arc<dyn CertificateFetcher<K>>,
	) -> WorkloadHBONEPool<K> {
		let (timeout_tx, timeout_rx) = watch::channel(false);
		let (timeout_send, timeout_recv) = watch::channel(false);
		let pool_duration = cfg.pool_unused_release_timeout;

		let spawner = ConnSpawner {
			cfg,
			certificates: local_workload,
			timeout_rx: timeout_recv.clone(),
		};

		Self {
			state: Arc::new(PoolState {
				pool_notifier: timeout_tx,
				timeout_tx: timeout_send,
				// timeout_rx: timeout_recv,
				// the number here is simply the number of unique src/dest keys
				// the pool is expected to track before the inner hashmap resizes.
				connected_pool: Arc::new(pingora_pool::ConnectionPool::new(500)),
				established_conn_writelock: flurry::HashMap::new(),
				pool_unused_release_timeout: pool_duration,
				pool_global_conn_count: AtomicI32::new(0),
				spawner,
			}),
			pool_watcher: timeout_rx,
		}
	}

	pub async fn send_request_pooled(
		&mut self,
		workload_key: &K,
		request: http::Request<()>,
	) -> anyhow::Result<H2Stream> {
		let mut connection = self.connect(workload_key).await?;

		connection.send_request(request).await
	}

	// Obtain a pooled connection. Will prefer to retrieve an existing conn from the pool, but
	// if none exist, or the existing conn is maxed out on streamcount, will spawn a new one,
	// even if it is to the same dest+port.
	//
	// If many `connects` request a connection to the same dest at once, all will wait until exactly
	// one connection is created, before deciding if they should create more or just use that one.
	async fn connect(&mut self, workload_key: &K) -> anyhow::Result<H2ConnectClient<K>> {
		trace!("pool connect START");
		// TODO BML this may not be collision resistant, or a fast hash. It should be resistant enough for workloads tho.
		// We are doing a deep-equals check at the end to mitigate any collisions, will see about bumping Pingora
		let mut s = DefaultHasher::new();
		workload_key.hash(&mut s);
		let hash_key = s.finish();
		let id = self
			.state
			.pool_global_conn_count
			.fetch_add(1, Ordering::SeqCst);
		#[cfg(target_os = "windows")]
		let id = id.try_into()?; // id is usize on windows so try and convert
		let pool_key = pingora_pool::ConnectionMeta::new(hash_key, id);

		// First, see if we can naively take an inner lock for our specific key, and get a connection.
		// This should be the common case, except for the first establishment of a new connection/key.
		// This will be done under outer readlock (nonexclusive)/inner keyed writelock (exclusive).
		let existing_conn = self
			.state
			.checkout_conn_under_writelock(workload_key, &pool_key)
			.await?;

		// Early return, no need to do anything else
		if let Some(e) = existing_conn {
			debug!("initial attempt - found existing conn, done");
			return Ok(e);
		}

		// We couldn't get a writelock for this key. This means nobody has tried to establish any conns for this key yet,
		// So, we will take a nonexclusive readlock on the outer lockmap, and attempt to insert one.
		//
		// (if multiple threads try to insert one, only one will succeed.)
		{
			debug!(
				"didn't find a connection for key {:?}, making sure lockmap has entry",
				hash_key
			);
			let guard = self.state.established_conn_writelock.guard();
			match self.state.established_conn_writelock.try_insert(
				hash_key,
				Some(Arc::new(Mutex::new(()))),
				&guard,
			) {
				Ok(_) => {
					debug!("inserting conn mutex for key {:?} into lockmap", hash_key);
				},
				Err(_) => {
					debug!("already have conn for key {:?} in lockmap", hash_key);
				},
			}
		}

		// If we get here, it means the following are true:
		// 1. We have a guaranteed sharded mutex in the outer map for our current key
		// 2. We can now, under readlock(nonexclusive) in the outer map, attempt to
		// take the inner writelock for our specific key (exclusive).
		//
		// This doesn't block other tasks spawning connections against other keys, but DOES block other
		// tasks spawning connections against THIS key - which is what we want.

		// NOTE: The inner, key-specific mutex is a tokio::async::Mutex, and not a stdlib sync mutex.
		// these differ from the stdlib sync mutex in that they are (slightly) slower
		// (they effectively sleep the current task) and they can be held over an await.
		// The tokio docs (rightly) advise you to not use these,
		// because holding a lock over an await is a great way to create deadlocks if the await you
		// hold it over does not resolve.
		//
		// HOWEVER. Here we know this connection will either establish or timeout (or fail with error)
		// and we WANT other tasks to go back to sleep if a task is already trying to create a new connection for this key.
		//
		// So the downsides are actually useful (we WANT task contention -
		// to block other parallel tasks from trying to spawn a connection for this key if we are already doing so)
		trace!("fallback attempt - trying win win connlock");
		let res = match self
			.state
			.start_conn_if_win_writelock(workload_key, &pool_key)
			.await?
		{
			Some(client) => client,
			None => {
				debug!("we didn't win the lock, something else is creating a conn, wait for it");
				// If we get here, it means the following are true:
				// 1. We have a writelock in the outer map for this key (either we inserted, or someone beat us to it - but it's there)
				// 2. We could not get the exclusive inner writelock to add a new conn for this key.
				// 3. Someone else got the exclusive inner writelock, and is adding a new conn for this key.
				//
				// So, loop and wait for the pool_watcher to tell us a new conn was enpooled,
				// so we can pull it out and check it.
				loop {
					match self.pool_watcher.changed().await {
						Ok(_) => {
							trace!(
								"notified a new conn was enpooled, checking for hash {:?}",
								hash_key
							);
							// Notifier fired, try and get a conn out for our key.
							let existing_conn = self
								.state
								.checkout_conn_under_writelock(workload_key, &pool_key)
								.await?;
							match existing_conn {
								None => {
									trace!(
										"woke up on pool notification, but didn't find a conn for {:?} yet",
										hash_key
									);
									continue;
								},
								Some(e_conn) => {
									debug!("found existing conn after waiting");
									break e_conn;
								},
							}
						},
						Err(_) => {
							return Err(anyhow!("pool draining"));
						},
					}
				}
			},
		};
		Ok(res)
	}
}
