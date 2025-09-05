mod binds;

use std::sync::Arc;

pub use binds::{
	BackendPolicies, GatewayPolicies, LLMRequestPolicies, LLMResponsePolicies, RoutePolicies,
	Store as BindStore,
};
use serde::{Serialize, Serializer};
mod discovery;
use std::sync::RwLock;

pub use binds::PreviousState as BindPreviousState;
pub use discovery::{
	LocalWorkload, PreviousState as DiscoveryPreviousState, Store as DiscoveryStore, WorkloadStore,
};

use crate::store;

#[derive(Clone, Debug)]
pub enum Event<T> {
	Add(T),
	Remove(T),
}

#[derive(Clone, Debug)]
pub struct Stores {
	pub discovery: discovery::StoreUpdater,
	pub binds: binds::StoreUpdater,
}

impl Default for Stores {
	fn default() -> Self {
		Self::new()
	}
}

impl Stores {
	pub fn new() -> Stores {
		Stores {
			discovery: discovery::StoreUpdater::new(Arc::new(RwLock::new(discovery::Store::new()))),
			binds: binds::StoreUpdater::new(Arc::new(RwLock::new(binds::Store::new()))),
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
