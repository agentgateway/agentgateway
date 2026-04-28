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

use std::sync::RwLock;

pub use binds::PreviousState as BindPreviousState;
pub use discovery::{
	LocalWorkload, PreviousState as DiscoveryPreviousState, Store as DiscoveryStore,
	StoreUpdater as DiscoveryStoreUpdater, WorkloadStore,
};
pub use policy::{
	BackendPolicy, BackendPolicyTrait, RequestPolicy, RequestPolicyTrait, ResponsePolicy,
	ResponsePolicyTrait,
};

use crate::store;
use crate::types::discovery::Workload;

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
	pub fn is_resolved(&self) -> bool {
		self.0.get().is_some()
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
		Stores {
			discovery: discovery::StoreUpdater::new(Arc::new(RwLock::new(discovery::Store::new()))),
			binds: binds::StoreUpdater::new(Arc::new(RwLock::new(binds::Store::new(
				ipv6_enabled,
				threading_mode,
			)))),
		}
	}
	pub fn read_binds(&self) -> std::sync::RwLockReadGuard<'_, store::BindStore> {
		self.binds.read()
	}

	pub fn read_discovery(&self) -> std::sync::RwLockReadGuard<'_, store::DiscoveryStore> {
		self.discovery.read()
	}
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoresDump {
	#[serde(flatten)]
	discovery: discovery::Dump,
	#[serde(flatten)]
	binds: binds::Dump,
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
