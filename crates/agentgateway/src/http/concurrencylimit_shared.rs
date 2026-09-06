//! In-flight slots kept in Redis, so every proxy instance counts against one limit.
//!
//! A key's slots are a sorted set of members scored by the store's own clock. A slot is taken by
//! a script that first drops the members past their lease, then compares the count with the
//! limit, all in one step. Held slots are renewed on a timer and removed when the request ends,
//! so a slot outlives its request only when the instance that took it went away, and then only
//! until its lease runs out.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use tokio::sync::mpsc;

use crate::*;

const ACQUIRE: &str = r"
local now = redis.call('TIME')
local now_ms = now[1] * 1000 + math.floor(now[2] / 1000)
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now_ms - tonumber(ARGV[1]))
local held = redis.call('ZCARD', KEYS[1])
if held >= tonumber(ARGV[2]) then
  return {0, held}
end
redis.call('ZADD', KEYS[1], now_ms, ARGV[3])
redis.call('PEXPIRE', KEYS[1], ARGV[1])
return {1, held + 1}
";

const RENEW: &str = r"
local now = redis.call('TIME')
local now_ms = now[1] * 1000 + math.floor(now[2] / 1000)
for i = 2, #ARGV do
  redis.call('ZADD', KEYS[1], 'XX', now_ms, ARGV[i])
end
redis.call('PEXPIRE', KEYS[1], ARGV[1])
return 1
";

/// Stores by URL, lease and timeout, so rules that name the same store share one connection.
static STORES: Mutex<Vec<Registered>> = Mutex::new(Vec::new());

struct Registered {
	url: String,
	lease: Duration,
	timeout: Duration,
	store: Weak<Store>,
}

/// The outcome of asking the store for a slot.
pub(super) enum Taken {
	Slot(String),
	Full { in_flight: u32 },
}

pub(super) struct Store {
	conn: ConnectionManager,
	acquire: redis::Script,
	renew: redis::Script,
	lease: Duration,
	/// Tells this instance's members from those other instances put on the same key.
	instance: u64,
	next_member: AtomicU64,
	/// Slots this instance holds, by key, renewed while held.
	held: Mutex<HashMap<String, Vec<String>>>,
	released: mpsc::UnboundedSender<(String, String)>,
}

impl fmt::Debug for Store {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Store")
			.field("lease", &self.lease)
			.finish_non_exhaustive()
	}
}

impl Store {
	/// The store for `url`, connecting on first use.
	pub(super) fn get(
		url: &str,
		lease: Duration,
		timeout: Duration,
	) -> Result<Arc<Store>, redis::RedisError> {
		let mut stores = STORES.lock();
		stores.retain(|r| r.store.strong_count() > 0);
		if let Some(store) = stores
			.iter()
			.find(|r| r.url == url && r.lease == lease && r.timeout == timeout)
			.and_then(|r| r.store.upgrade())
		{
			return Ok(store);
		}
		let client = redis::Client::open(url)?;
		let config = ConnectionManagerConfig::new()
			.set_connection_timeout(Some(timeout))
			.set_response_timeout(Some(timeout))
			.set_number_of_retries(1);
		let conn = client.get_connection_manager_lazy(config)?;
		let (released, releases) = mpsc::unbounded_channel();
		let store = Arc::new(Store {
			conn,
			acquire: redis::Script::new(ACQUIRE),
			renew: redis::Script::new(RENEW),
			lease,
			instance: rand::random(),
			next_member: AtomicU64::new(0),
			held: Default::default(),
			released,
		});
		tokio::spawn(Self::maintain(Arc::downgrade(&store), releases, lease));
		stores.push(Registered {
			url: url.to_string(),
			lease,
			timeout,
			store: Arc::downgrade(&store),
		});
		Ok(store)
	}

	pub(super) async fn acquire(&self, key: &str, limit: u32) -> Result<Taken, redis::RedisError> {
		let member = format!(
			"{:016x}-{}",
			self.instance,
			self.next_member.fetch_add(1, Ordering::Relaxed)
		);
		let mut conn = self.conn.clone();
		let (taken, in_flight): (u8, u32) = self
			.acquire
			.key(key)
			.arg(self.lease.as_millis() as u64)
			.arg(limit)
			.arg(&member)
			.invoke_async(&mut conn)
			.await?;
		if taken == 0 {
			return Ok(Taken::Full { in_flight });
		}
		self
			.held
			.lock()
			.entry(key.to_string())
			.or_default()
			.push(member.clone());
		Ok(Taken::Slot(member))
	}

	/// Gives a slot back. The removal runs in the background, so a drop can call this.
	pub(super) fn release(&self, key: String, member: String) {
		{
			let mut held = self.held.lock();
			if let Some(members) = held.get_mut(&key) {
				members.retain(|m| m != &member);
				if members.is_empty() {
					held.remove(&key);
				}
			}
		}
		let _ = self.released.send((key, member));
	}

	/// Removes released slots and renews held ones until the store is dropped.
	async fn maintain(
		store: Weak<Store>,
		mut releases: mpsc::UnboundedReceiver<(String, String)>,
		lease: Duration,
	) {
		let mut renew = tokio::time::interval((lease / 3).max(Duration::from_millis(1)));
		renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				released = releases.recv() => {
					let Some(first) = released else { break };
					let mut batch = vec![first];
					while let Ok(more) = releases.try_recv() {
						batch.push(more);
					}
					let Some(store) = store.upgrade() else { break };
					store.remove(batch).await;
				},
				_ = renew.tick() => {
					let Some(store) = store.upgrade() else { break };
					store.renew_held().await;
				},
			}
		}
	}

	async fn remove(&self, slots: Vec<(String, String)>) {
		let mut pipe = redis::pipe();
		for (key, member) in &slots {
			pipe.zrem(key, member).ignore();
		}
		let mut conn = self.conn.clone();
		if let Err(e) = pipe.query_async::<()>(&mut conn).await {
			debug!(error = %e, "concurrency store: releasing slots failed; they expire with the lease");
		}
	}

	async fn renew_held(&self) {
		let held: Vec<(String, Vec<String>)> = self
			.held
			.lock()
			.iter()
			.map(|(key, members)| (key.clone(), members.clone()))
			.collect();
		let mut conn = self.conn.clone();
		for (key, members) in held {
			let mut call = self.renew.prepare_invoke();
			call.key(&key).arg(self.lease.as_millis() as u64);
			for member in &members {
				call.arg(member);
			}
			if let Err(e) = call.invoke_async::<()>(&mut conn).await {
				debug!(error = %e, "concurrency store: renewing slots failed");
			}
		}
	}
}
