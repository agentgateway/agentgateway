mod binds;

use std::sync::{Arc, OnceLock};

pub use binds::{
	BackendPolicies, BindEvent, BindListeners, FrontendPolices, GatewayPolicies, LLMRequestPolicies,
	LLMResponsePolicies, RoutePath, RoutePolicies, Store as BindStore,
	StoreUpdater as BindStoreUpdater,
};
use serde::{Serialize, Serializer};
mod discovery;
mod policy;

use std::cell::Cell;
use std::ops::Deref;
use std::sync::{RwLock, RwLockReadGuard};
use std::thread::LocalKey;

pub use binds::{Dump as BindsDump, PreviousState as BindPreviousState};
pub use discovery::{
	Dump as DiscoveryDump, LocalWorkload, PreviousState as DiscoveryPreviousState,
	Store as DiscoveryStore, StoreUpdater as DiscoveryStoreUpdater, WorkloadStore,
};
pub use policy::{
	BackendPolicy, BackendPolicyTrait, HasExpressions, PolicyExpressions, RequestPolicy,
	RequestPolicyTrait, ResponsePolicy, ResponsePolicyTrait,
};

use crate::types::discovery::Workload;
use crate::{apply, schema_ser_schema, store};

/// Set-once holder for the gateway's own Workload (locality-aware LB reads this).
/// Populated at startup in Static mode, or when WDS delivers a matching workload in Wds mode.
/// TODO ArcSwap or something to support updates after startup
#[derive(Clone, Debug, Default)]
pub struct SelfWorkload(Arc<OnceLock<Workload>>);

impl SelfWorkload {
	pub fn new() -> Self {
		Self::default()
	}
	pub fn get(&self) -> Option<&Workload> {
		self.0.get()
	}
	/// First call wins; later calls are no-ops.
	pub fn set(&self, w: Workload) {
		let _ = self.0.set(w);
	}
}

#[derive(Clone, Debug)]
pub struct Stores {
	pub discovery: discovery::StoreUpdater,
	pub binds: binds::StoreUpdater,
}

impl Default for Stores {
	fn default() -> Self {
		Self::with_ipv6_enabled(true)
	}
}

impl Stores {
	pub fn with_ipv6_enabled(ipv6_enabled: bool) -> Stores {
		Self::new(ipv6_enabled, crate::ThreadingMode::Multithreaded)
	}

	pub fn new(ipv6_enabled: bool, threading_mode: crate::ThreadingMode) -> Stores {
		Self::new_with_dynamic_ca_cert_cache(ipv6_enabled, threading_mode, Default::default())
	}

	pub fn new_with_dynamic_ca_cert_cache(
		ipv6_enabled: bool,
		threading_mode: crate::ThreadingMode,
		dynamic_ca_cert_cache: crate::DynamicCaCertCacheConfig,
	) -> Stores {
		Stores {
			discovery: discovery::StoreUpdater::new(Arc::new(RwLock::new(discovery::Store::new()))),
			binds: binds::StoreUpdater::new(Arc::new(RwLock::new(binds::Store::new(
				ipv6_enabled,
				threading_mode,
				dynamic_ca_cert_cache,
			)))),
		}
	}
	#[track_caller]
	pub fn read_binds(&self) -> ReadGuard<'_, store::BindStore> {
		self.binds.read()
	}

	#[track_caller]
	pub fn read_discovery(&self) -> ReadGuard<'_, store::DiscoveryStore> {
		self.discovery.read()
	}
}

/// Read guard for a store lock. A second read of a std RwLock on the same thread can deadlock if a
/// writer is queued in between, so debug builds assert that a thread holds at most one guard per lock.
pub struct ReadGuard<'a, T> {
	inner: RwLockReadGuard<'a, T>,
	#[cfg(debug_assertions)]
	depth: &'static LocalKey<Cell<u32>>,
}

impl<'a, T> ReadGuard<'a, T> {
	#[track_caller]
	#[cfg_attr(not(debug_assertions), allow(unused_variables))]
	fn new(lock: &'a RwLock<T>, depth: &'static LocalKey<Cell<u32>>) -> Self {
		#[cfg(debug_assertions)]
		assert_eq!(
			depth.get(),
			0,
			"re-entrant store read lock; this can deadlock against a writer"
		);
		let inner = lock.read().expect("mutex acquired");
		#[cfg(debug_assertions)]
		depth.set(depth.get() + 1);
		Self {
			inner,
			#[cfg(debug_assertions)]
			depth,
		}
	}
}

impl<T> Deref for ReadGuard<'_, T> {
	type Target = T;
	fn deref(&self) -> &T {
		&self.inner
	}
}

impl<T> Drop for ReadGuard<'_, T> {
	fn drop(&mut self) {
		#[cfg(debug_assertions)]
		self.depth.set(self.depth.get() - 1);
	}
}

#[apply(schema_ser_schema!)]
pub struct StoresDump {
	#[serde(flatten)]
	pub discovery: discovery::Dump,
	#[serde(flatten)]
	pub binds: binds::Dump,
}

impl Serialize for Stores {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let serializable = StoresDump {
			discovery: self.discovery.dump(),
			binds: self.binds.dump(),
		};
		serializable.serialize(serializer)
	}
}
