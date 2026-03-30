mod binds;

use std::sync::Arc;

pub use binds::{
	BackendPolicies, FrontendPolices, GatewayPolicies, LLMRequestPolicies, LLMResponsePolicies,
	RoutePath, RoutePolicies, Store as BindStore, StoreUpdater as BindStoreUpdater,
};
use serde::{Serialize, Serializer};
mod discovery;
use std::sync::RwLock;

pub use binds::PreviousState as BindPreviousState;
pub use discovery::{
	LocalWorkload, PreviousState as DiscoveryPreviousState, Store as DiscoveryStore,
	StoreUpdater as DiscoveryStoreUpdater, WorkloadStore,
};

use crate::store;

#[derive(Clone, Debug)]
pub enum Event<T> {
	Add(T),
	Remove(T),
}

/// A bind event with an optional acknowledgment channel.
///
/// When `ack` is `Some`, the consumer should send `()` on it after the TCP
/// listener is actually bound. This lets callers of `insert_bind` wait for
/// the listener to be ready instead of racing.
pub struct BindEvent {
	pub event: Event<Arc<crate::types::agent::Bind>>,
	pub ack: Option<tokio::sync::oneshot::Sender<()>>,
}

impl std::fmt::Debug for BindEvent {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BindEvent")
			.field("event", &self.event)
			.field("has_ack", &self.ack.is_some())
			.finish()
	}
}

#[derive(Clone, Debug)]
pub struct Stores {
	pub discovery: discovery::StoreUpdater,
	pub binds: binds::StoreUpdater,
}

/// Return type from [`Stores::with_ipv6_enabled`] — the stores plus the bind
/// event receiver that [`crate::proxy::gateway::Gateway`] must consume.
pub struct StoresWithBindRx {
	pub stores: Stores,
	pub bind_rx: tokio::sync::mpsc::UnboundedReceiver<BindEvent>,
}

impl Default for Stores {
	fn default() -> Self {
		Self::with_ipv6_enabled(true).stores
	}
}

impl Stores {
	pub fn with_ipv6_enabled(ipv6_enabled: bool) -> StoresWithBindRx {
		let (bind_store, bind_rx) = binds::Store::with_ipv6_enabled(ipv6_enabled);
		let stores = Stores {
			discovery: discovery::StoreUpdater::new(Arc::new(RwLock::new(discovery::Store::new()))),
			binds: binds::StoreUpdater::new(Arc::new(RwLock::new(bind_store))),
		};
		StoresWithBindRx { stores, bind_rx }
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
