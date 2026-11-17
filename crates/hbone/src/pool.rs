use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use flurry;
use pingora_pool;
use rustls::pki_types::ServerName;
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, watch};
use tonic::async_trait;
use tracing::{Instrument, debug, trace};

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
}

impl<K> Debug for WorkloadHBONEPool<K> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WorkloadHBONEPool").finish()
	}
}

// PoolState is effectively the gnarly inner state stuff that needs thread/task sync, and should be wrapped in a Mutex.
struct PoolState<K> {
	timeout_tx: watch::Sender<bool>, /* This is already impl clone? rustc complains that it isn't, tho */
	// this is effectively just a convenience data type - a rwlocked hashmap with keying and LRU drops
	// and has no actual hyper/http/connection logic.
	connected_pool: Arc<pingora_pool::ConnectionPool<H2ConnectClient<K>>>,
	// this must be an atomic/concurrent-safe list-of-locks, so we can lock per-key, not globally, and avoid holding up all conn attempts
	established_conn_writelock: flurry::HashMap<u64, Arc<Mutex<()>>>,
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
			crate::client::spawn_connection(&self.cfg.h2, tls_stream, self.timeout_rx.clone(), key)
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
					.idle_timeout(&pool_key_ref, Some(release_timeout), evict, rx, pickup)
					.await;
				debug!(
					"connection {:?} was removed/checked out/timed out of the pool",
					pool_key_ref
				)
			}
			.in_current_span(),
		);
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

	// Hold the async mutex across connection creation to backpressure callers for
	// this key. Releasing the guard wakes waiters on success, error, or cancellation.
	async fn checkout_conn_under_writelock(
		&self,
		workload_key: &K,
		pool_key: &pingora_pool::ConnectionMeta,
	) -> anyhow::Result<H2ConnectClient<K>> {
		let exist_conn_lock = {
			let guard = self.established_conn_writelock.guard();
			match self.established_conn_writelock.get(&pool_key.key, &guard) {
				Some(lock) => lock.clone(),
				None => match self.established_conn_writelock.try_insert(
					pool_key.key,
					Arc::new(Mutex::new(())),
					&guard,
				) {
					Ok(lock) => lock.clone(),
					Err(existing) => existing.current.clone(),
				},
			}
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
		Ok(returned_connection)
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
		let (timeout_send, timeout_recv) = watch::channel(false);
		let pool_duration = cfg.pool_unused_release_timeout;

		let spawner = ConnSpawner {
			cfg,
			certificates: local_workload,
			timeout_rx: timeout_recv.clone(),
		};

		Self {
			state: Arc::new(PoolState {
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
		}
	}

	pub async fn send_request_pooled(
		&self,
		workload_key: &K,
		request: http::Request<()>,
	) -> anyhow::Result<H2Stream> {
		let mut connection = self.connect(workload_key).await?;

		connection.send_request(request).await
	}

	/// Get the HBONE config used by this pool
	pub fn config(&self) -> Arc<crate::Config> {
		self.state.spawner.cfg.clone()
	}

	/// Fetch a certificate for the given workload key
	pub async fn fetch_certificate(
		&self,
		key: K,
	) -> anyhow::Result<Arc<rustls::client::ClientConfig>> {
		self.state.spawner.certificates.fetch_certificate(key).await
	}

	// Obtain a pooled connection. Will prefer to retrieve an existing conn from the pool, but
	// if none exist, or the existing conn is maxed out on streamcount, will spawn a new one,
	// even if it is to the same dest+port.
	//
	// If many `connects` request a connection to the same dest at once, all will wait until exactly
	// one connection is created, before deciding if they should create more or just use that one.
	async fn connect(&self, workload_key: &K) -> anyhow::Result<H2ConnectClient<K>> {
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

		self
			.state
			.checkout_conn_under_writelock(workload_key, &pool_key)
			.await
	}
}
