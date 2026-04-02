use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;

use agent_xds::XdsUpdate;
use itertools::Itertools;
use tokio::sync::watch::Sender;
use tracing::{Level, instrument};
use types::discovery::{NamespacedHostname, NetworkAddress};
use types::proto::workload::address::Type as XdsType;
use types::proto::workload::{
	Address as XdsAddress, PortList, Service as XdsService, Workload as XdsWorkload,
};

use crate::types::discovery::{Endpoint, InboundProtocol, NetworkMode, Service, Workload};
use crate::*;

#[derive(Debug)]
pub struct Store {
	pub workloads: WorkloadStore,

	pub services: ServiceStore,
}

impl Store {}

impl Default for Store {
	fn default() -> Self {
		Self::new()
	}
}

impl Store {
	pub fn new() -> Store {
		Store {
			workloads: WorkloadStore {
				insert_notifier: Sender::new(()),
				by_addr: Default::default(),
				by_uid: Default::default(),
			},
			services: Default::default(),
		}
	}
	pub fn insert_address(&mut self, a: XdsAddress) -> anyhow::Result<()> {
		match a.r#type {
			Some(XdsType::Workload(w)) => self.insert_workload(w),
			Some(XdsType::Service(s)) => self.insert_service(s),
			_ => Err(anyhow::anyhow!("unknown address type")),
		}
	}
	#[instrument(
		level = Level::TRACE,
		name="insert_workload",
		skip_all,
		fields(uid=%w.uid),
	)]
	pub fn insert_workload(&mut self, w: XdsWorkload) -> anyhow::Result<()> {
		debug!(uid=%w.uid, "handling insert");

		// Clone services, so we can pass full ownership of the rest of XdsWorkload to build our Workload
		// object, which doesn't include Services.
		// In theory, I think we could avoid this if Workload::try_from returning the services.
		// let services = w.services.clone();
		// Convert the workload.
		let (workload, services) = Workload::try_from_xds_with_services(w)?;
		let workload = Arc::new(workload);

		// First, remove the entry entirely to make sure things are cleaned up properly.
		self.remove_workload_for_insert(&workload.uid);

		// Lock and upstate the stores.
		self.workloads.insert(workload.clone());
		self
			.services
			.insert_endpoint_for_services(&workload, &services)?;

		Ok(())
	}

	#[instrument(
        level = Level::TRACE,
        name="insert_service",
        skip_all,
        fields(name=%service.name),
	)]
	pub fn insert_service(&mut self, service: XdsService) -> anyhow::Result<()> {
		debug!("handling insert");
		let service = Service::try_from(&service)?;
		self.insert_service_internal(service);
		Ok(())
	}
	pub fn insert_service_internal(&mut self, mut service: Service) {
		// If the service already exists, add existing endpoints into the new service.
		if let Some(prev) = self
			.services
			.get_by_namespaced_host(&service.namespaced_hostname())
		{
			// TODO: if health mode changes we are in trouble
			service.endpoints = prev.endpoints.clone();
		}

		self.services.insert(service);
	}

	fn remove(&mut self, xds_name: &Strng) {
		self.remove_internal(xds_name, false);
	}

	fn remove_workload_for_insert(&mut self, xds_name: &Strng) {
		self.remove_internal(xds_name, true);
	}
	#[instrument(
        level = Level::TRACE,
        name="remove",
        skip_all,
        fields(name=%xds_name, for_workload_insert=%for_workload_insert),
	)]
	fn remove_internal(&mut self, xds_name: &Strng, for_workload_insert: bool) {
		// remove workload by UID; if xds_name is a service then this will no-op
		if let Some(prev) = self.workloads.remove(&strng::new(xds_name)) {
			// Also remove service endpoints for the workload.
			self.services.remove_endpoint(&prev);

			// We removed a workload, no reason to attempt to remove a service with the same name
			return;
		}
		if for_workload_insert {
			// This is a workload, don't attempt to remove as a service
			return;
		}

		let Ok(name) = NamespacedHostname::from_str(xds_name) else {
			// we don't have namespace/hostname xds primary key for service
			warn!("tried to remove service but it did not have the expected namespace/hostname format");
			return;
		};

		if name.hostname.contains('/') {
			// avoid trying to delete obvious workload UIDs as a service,
			// which can result in noisy logs when new workloads are added
			// (we remove then add workloads on initial update)
			//
			// we can make this assumption because namespaces and hostnames cannot have `/` in them
			trace!("not a service, not attempting to delete as such",);
			return;
		}
		if !self.services.remove(&name) {
			warn!("tried to remove service, but it was not found");
		}
	}
}

/// A WorkloadStore encapsulates all information about workloads in the mesh
#[derive(Debug)]
pub struct WorkloadStore {
	// TODO this could be expanded to Sender<Workload> + a full subscriber/streaming
	// model, but for now just notifying watchers to wake when _any_ insert happens
	// is simpler (and only requires a channelsize of 1)
	insert_notifier: Sender<()>,

	/// by_addr maps workload network addresses to workloads
	by_addr: HashMap<NetworkAddress, WorkloadByAddr>,
	/// by_uid maps workload UIDs to workloads
	pub(super) by_uid: HashMap<Strng, Arc<Workload>>,
}

impl WorkloadStore {
	pub fn insert(&mut self, w: Arc<Workload>) {
		// First, remove the entry entirely to make sure things are cleaned up properly.
		self.remove(&w.uid);

		if w.network_mode != NetworkMode::HostNetwork {
			for ip in &w.workload_ips {
				let k = network_addr(w.network.clone(), *ip);
				self
					.by_addr
					.entry(k)
					.and_modify(|ws| ws.insert(w.clone()))
					.or_insert_with(|| WorkloadByAddr::Single(w.clone()));
			}
		}
		self.by_uid.insert(w.uid.clone(), w.clone());

		// We have stored a newly inserted workload, notify watchers
		// (if any) to wake.
		self.insert_notifier.send_replace(());
	}

	fn remove(&mut self, uid: &Strng) -> Option<Workload> {
		match self.by_uid.remove(uid) {
			None => {
				trace!("tried to remove workload but it was not found");
				None
			},
			Some(prev) => {
				if prev.network_mode != NetworkMode::HostNetwork {
					for wip in prev.workload_ips.iter() {
						if let Entry::Occupied(mut o) =
							self.by_addr.entry(network_addr(prev.network.clone(), *wip))
							&& o.get_mut().remove_uid(prev.uid.clone())
						{
							o.remove();
						}
					}
				}

				Some(prev.deref().clone())
			},
		}
	}
}

impl WorkloadStore {
	pub fn find_uid(&self, uid: &Strng) -> Option<Arc<Workload>> {
		self.by_uid.get(uid).cloned()
	}

	/// Finds the workload by address, as an arc.
	pub fn find_address(&self, addr: &NetworkAddress) -> Option<Arc<Workload>> {
		self.by_addr.get(addr).map(WorkloadByAddr::get)
	}
}

/// Data store for service information.
#[derive(Default, Debug)]
pub struct ServiceStore {
	/// Maintains a mapping of service key -> (endpoint key -> workload endpoint)
	/// this is used to handle ordering issues if workloads are received before services.
	pub(super) staged_services: HashMap<NamespacedHostname, HashMap<Strng, Endpoint>>,

	/// Allows for lookup of services by network address, the service's xds secondary key.
	pub(super) by_vip: HashMap<NetworkAddress, Arc<Service>>,

	/// Allows for lookup of services by hostname, and then by namespace. XDS uses a combination
	/// of hostname and namespace as the primary key. In most cases, there will be a single
	/// service for a given hostname. However, `ServiceEntry` allows hostnames to be overridden
	/// on a per-namespace basis.
	pub(super) by_host: HashMap<Strng, Vec<Arc<Service>>>,
}

impl ServiceStore {
	fn insert_endpoint_for_services(
		&mut self,
		workload: &Arc<Workload>,
		services: &HashMap<String, PortList>,
	) -> anyhow::Result<()> {
		for (namespaced_host, ports) in services {
			// Parse the namespaced hostname for the service.
			let namespaced_host = NamespacedHostname::from_str(namespaced_host)?;
			for (endpoint_key, endpoint) in service_endpoints(namespaced_host.clone(), workload, ports) {
				self.insert_endpoint(namespaced_host.clone(), endpoint_key, endpoint)
			}
		}
		Ok(())
	}

	fn insert_endpoint(
		&mut self,
		service_name: NamespacedHostname,
		endpoint_key: Strng,
		ep: Endpoint,
	) {
		if let Some(svc) = self.get_by_namespaced_host(&service_name) {
			// We may or may not accept the endpoint based on it's health
			if !svc.should_include_endpoint(ep.status) {
				trace!(
					"service doesn't accept pod with status {:?}, skip",
					ep.status
				);
				return;
			}
			svc.endpoints.insert_key(endpoint_key, ep, 0);
		} else {
			// We received workload endpoints, but don't have the Service yet.
			// This can happen due to ordering issues.
			trace!("pod has service {}, but service not found", service_name);

			// Add a staged entry. This will be added to the service once we receive it.
			self
				.staged_services
				.entry(service_name.clone())
				.or_default()
				.insert(endpoint_key, ep.clone());
		}
	}

	/// Removes entries for the given endpoint address.
	fn remove_endpoint(&mut self, prev_workload: &Workload) {
		let mut services_to_update = HashSet::new();
		let workload_uid = &prev_workload.uid;
		for svc in prev_workload.services.iter() {
			// Remove the endpoint from the staged services.
			if let Entry::Occupied(mut staged) = self.staged_services.entry(svc.clone()) {
				staged
					.get_mut()
					.retain(|_, ep| &ep.workload_uid != workload_uid);
				if staged.get().is_empty() {
					staged.remove();
				}
			}

			services_to_update.insert(svc.clone());
		}

		// Now remove the endpoint from all Services.
		for svc in &services_to_update {
			if let Some(svc) = self.get_by_namespaced_host(svc) {
				svc
					.endpoints
					.remove_matching(|ep| &ep.workload_uid == workload_uid);
			}
		}
	}

	/// Removes the service for the given host and namespace, and returns whether something was removed
	fn remove(&mut self, namespaced_host: &NamespacedHostname) -> bool {
		match self.by_host.get_mut(&namespaced_host.hostname) {
			None => false,
			Some(services) => {
				// Remove the previous service from the by_host map.
				let Some(prev) = ({
					let mut prev = None;
					for i in 0..services.len() {
						if services[i].namespace == namespaced_host.namespace {
							// Remove this service from the list.
							prev = Some(services.remove(i));

							// If the the services list is empty, remove the entire entry.
							if services.is_empty() {
								self.by_host.remove(&namespaced_host.hostname);
							}
							break;
						}
					}
					prev
				}) else {
					// Not found.
					return false;
				};

				// Remove the entries for the previous service VIPs.
				prev.vips.iter().for_each(|addr| {
					self.by_vip.remove(addr);
				});

				// Remove the staged service.
				// TODO(nmittler): no endpoints for this service should be staged at this point.
				self.staged_services.remove(namespaced_host);

				// Remove successful.
				true
			},
		}
	}

	/// Adds the given service.
	fn insert(&mut self, service: Service) {
		self.insert_internal(service, false)
	}

	fn insert_internal(&mut self, service: Service, endpoint_update_only: bool) {
		let namespaced_hostname = service.namespaced_hostname();
		// If we're replacing an existing service, remove the old one from all data structures.
		if !endpoint_update_only {
			// First add any staged service endpoints. Due to ordering issues, we may have received
			// the workloads before their associated services.
			if let Some(endpoints) = self.staged_services.remove(&namespaced_hostname) {
				trace!(
					"staged service found, inserting {} endpoints",
					endpoints.len()
				);
				for (key, ep) in endpoints {
					if service.should_include_endpoint(ep.status) {
						service.endpoints.insert_key(key, ep, 0);
					}
				}
			}

			let _ = self.remove(&namespaced_hostname);
		}

		// Create the Arc.
		let service = Arc::new(service);
		let hostname = &service.hostname;

		// Map the vips to the service.
		for vip in &service.vips {
			self.by_vip.insert(vip.clone(), service.clone());
		}

		// Map the hostname to the service.
		match self.by_host.get_mut(hostname) {
			None => {
				let _ = self.by_host.insert(hostname.clone(), vec![service.clone()]);
			},
			Some(services) => {
				if let Some((cur, _)) = services
					.iter()
					.find_position(|s| s.namespace == service.namespace)
				{
					// Service already exists; replace the slot
					services[cur] = service.clone()
				} else {
					// No service exists yet, append it
					services.push(service.clone());
				}
			},
		}
	}
}

fn service_endpoints(
	service_name: NamespacedHostname,
	workload: &Arc<Workload>,
	ports: &PortList,
) -> Vec<(Strng, Endpoint)> {
	if service_name.hostname.contains(".inference.") && ports.ports.len() > 1 {
		let Some(frontend_port) = ports.ports.first().map(|p| p.service_port as u16) else {
			return Vec::new();
		};
		return ports
			.ports
			.iter()
			.map(|port| {
				(
					strng::format!("{}:{}", workload.uid, port.target_port),
					Endpoint {
						workload_uid: workload.uid.clone(),
						port: HashMap::from([(frontend_port, port.target_port as u16)]),
						status: workload.status,
					},
				)
			})
			.collect();
	}

	vec![(
		workload.uid.clone(),
		Endpoint {
			workload_uid: workload.uid.clone(),
			port: crate::types::discovery::ports_from_xds(ports),
			status: workload.status,
		},
	)]
}

impl ServiceStore {
	/// Returns the [Service] matching the given VIP.
	pub fn get_by_vip(&self, vip: &NetworkAddress) -> Option<Arc<Service>> {
		self.by_vip.get(vip).cloned()
	}
	pub fn get_by_namespaced_host(&self, host: &NamespacedHostname) -> Option<Arc<Service>> {
		// Get the list of services that match the hostname. Typically there will only be one, but
		// ServiceEntry allows configuring arbitrary hostnames on a per-namespace basis.
		match self.by_host.get(&host.hostname) {
			None => None,
			Some(services) => {
				// Return the service that matches the requested namespace.
				for service in services {
					if service.namespace == host.namespace {
						return Some(service.clone());
					}
				}
				None
			},
		}
	}

	/// Returns all services matching the given hostname.
	pub fn get_by_hostname(&self, hostname: &str) -> Option<Vec<Arc<Service>>> {
		self.by_host.get(hostname).map(|v| v.to_vec())
	}
}

#[derive(Debug)]
/// WorkloadByAddr is a small wrapper around a single or multiple Workloads
/// We split these as in the vast majority of cases there is only a single one, so we save vec allocation.
enum WorkloadByAddr {
	Single(Arc<Workload>),
	Many(Vec<Arc<Workload>>),
}

impl WorkloadByAddr {
	// insert adds the workload
	pub fn insert(&mut self, w: Arc<Workload>) {
		match self {
			WorkloadByAddr::Single(workload) => {
				*self = WorkloadByAddr::Many(vec![workload.clone(), w]);
			},
			WorkloadByAddr::Many(v) => {
				v.push(w);
			},
		}
	}
	// remove_uid mutates the address to remove the workload referenced by the UID.
	// If 'true' is returned, there is no workload remaining at all
	pub fn remove_uid(&mut self, uid: Strng) -> bool {
		match self {
			WorkloadByAddr::Single(wl) => {
				// Remove it if the UID matches, else do nothing
				wl.uid == uid
			},
			WorkloadByAddr::Many(ws) => {
				ws.retain(|w| w.uid != uid);
				match ws.as_slice() {
					[] => true,
					[wl] => {
						// We now have one workload, transition to Single
						*self = WorkloadByAddr::Single(wl.clone());
						false
					},
					// We still have many. We removed already so no need to do anything
					_ => false,
				}
			},
		}
	}
	pub fn get(&self) -> Arc<Workload> {
		match self {
			WorkloadByAddr::Single(workload) => workload.clone(),
			WorkloadByAddr::Many(workloads) => workloads
				.iter()
				.max_by_key(|w| {
					// Setup a ranking criteria in the event of a conflict.
					// We prefer pod objects, as they are not (generally) spoof-able and is the most
					// likely to truthfully correspond to what is behind the service.
					let is_pod = w.uid.contains("//Pod/");
					// We fallback to looking for HBONE -- a resource marked as in the mesh is likely
					// to have more useful context than one not in the mesh.
					let is_hbone = w.protocol == InboundProtocol::HBONE;
					match (is_pod, is_hbone) {
						(true, true) => 3,
						(true, false) => 2,
						(false, true) => 1,
						(false, false) => 0,
					}
				})
				.expect("must have at least one workload")
				.clone(),
		}
	}
}

#[derive(serde::Serialize)]
pub struct Dump {
	workloads: Vec<Arc<Workload>>,
	services: Vec<Arc<Service>>,
}

#[derive(Clone, Debug)]
pub struct StoreUpdater {
	state: Arc<RwLock<Store>>,
}

impl StoreUpdater {
	/// Creates a new updater for the given stores.
	pub fn new(state: Arc<RwLock<Store>>) -> Self {
		Self { state }
	}
	pub fn read(&self) -> std::sync::RwLockReadGuard<'_, Store> {
		self.state.read().expect("mutex acquired")
	}
	pub fn dump(&self) -> Dump {
		let store = self.state.read().expect("mutex");
		// Services all have hostname, so use that as the key
		let services: Vec<_> = store
			.services
			.by_host
			.iter()
			.sorted_by_key(|k| k.0)
			.flat_map(|k| k.1)
			.cloned()
			.collect();
		// Workloads all have a UID, so use that as the key
		let workloads: Vec<_> = store
			.workloads
			.by_uid
			.iter()
			.sorted_by_key(|k| k.0)
			.map(|k| k.1.clone())
			.collect();
		Dump {
			workloads,
			services,
		}
	}
	pub fn sync_local(
		&self,
		services: Vec<Service>,
		workloads: Vec<LocalWorkload>,
		prev: PreviousState,
	) -> anyhow::Result<PreviousState> {
		let mut s = self.state.write().expect("mutex acquired");
		let mut old_workloads = prev.workloads;
		let mut old_services = prev.services;
		let mut next_state = PreviousState {
			services: Default::default(),
			workloads: Default::default(),
		};
		for wl in workloads {
			trace!("inserting local workload {}", &wl.workload.uid);
			let w = Arc::new(wl.workload);
			// First, remove the entry entirely to make sure things are cleaned up properly.
			s.remove_workload_for_insert(&w.uid);

			// Lock and upstate the stores.
			s.workloads.insert(w.clone());
			let services: HashMap<String, PortList> = wl
				.services
				.into_iter()
				.map(|(k, v)| (k, crate::types::discovery::port_list_from_ports(v)))
				.collect();
			s.services.insert_endpoint_for_services(&w, &services)?;
			old_workloads.remove(&w.uid);
			next_state.workloads.insert(w.uid.clone());
		}
		for svc in services {
			let key = svc.namespaced_hostname();
			s.insert_service_internal(svc);
			old_services.remove(&key);
			next_state.services.insert(key);
		}
		for remaining_service in old_services {
			s.services.remove(&remaining_service);
		}
		for remaining_workload in old_workloads {
			if let Some(prev) = s.workloads.remove(&remaining_workload) {
				// Also remove service endpoints for the workload.
				s.services.remove_endpoint(&prev);
			}
		}
		Ok(next_state)
	}
}

pub fn network_addr(network: Strng, vip: IpAddr) -> NetworkAddress {
	NetworkAddress {
		network,
		address: vip,
	}
}

impl agent_xds::Handler<XdsAddress> for StoreUpdater {
	fn handle(
		&self,
		updates: Box<&mut dyn Iterator<Item = agent_xds::XdsUpdate<XdsAddress>>>,
	) -> Result<(), Vec<agent_xds::RejectedConfig>> {
		let mut state = self.state.write().unwrap();
		let handle = |res: XdsUpdate<XdsAddress>| {
			match res {
				XdsUpdate::Update(w) => state.insert_address(w.resource)?,
				XdsUpdate::Remove(name) => {
					debug!("handling delete {}", name);
					state.remove(&strng::new(name))
				},
			}
			Ok(())
		};
		agent_xds::handle_single_resource(updates, handle)
	}
}

#[derive(Clone, Debug, Default)]
pub struct PreviousState {
	pub workloads: HashSet<Strng>,
	pub services: HashSet<NamespacedHostname>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalWorkload {
	#[serde(flatten)]
	pub workload: Workload,
	pub services: HashMap<String, HashMap<u16, u16>>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::discovery::{
		AppProtocol, HealthStatus, InboundProtocol, Locality, NetworkMode,
	};
	use std::net::{IpAddr, Ipv4Addr};

	fn inference_service_name() -> NamespacedHostname {
		NamespacedHostname {
			namespace: "default".into(),
			hostname: "gateway-pool.default.inference.cluster.local".into(),
		}
	}

	fn test_workload(service: &NamespacedHostname) -> Arc<Workload> {
		Arc::new(Workload {
			workload_ips: vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))],
			waypoint: None,
			network_gateway: None,
			protocol: InboundProtocol::TCP,
			network_mode: NetworkMode::Standard,
			uid: "wl-1".into(),
			name: "gateway-pod".into(),
			namespace: "default".into(),
			trust_domain: "cluster.local".into(),
			service_account: "default".into(),
			network: "network".into(),
			workload_name: "gateway".into(),
			workload_type: "pod".into(),
			canonical_name: "".into(),
			canonical_revision: "".into(),
			hostname: "".into(),
			node: "".into(),
			authorization_policies: Vec::new(),
			status: HealthStatus::Healthy,
			cluster_id: "cluster".into(),
			locality: Locality::default(),
			services: vec![service.clone()],
			capacity: 1,
		})
	}

	fn test_service(service: &NamespacedHostname) -> Service {
		Service {
			name: "gateway-pool".into(),
			namespace: service.namespace.clone(),
			hostname: service.hostname.clone(),
			vips: Vec::new(),
			ports: HashMap::from([(8000, 8000), (8001, 8001)]),
			app_protocols: HashMap::from([(8000, AppProtocol::Http2), (8001, AppProtocol::Http2)]),
			endpoints: Default::default(),
			subject_alt_names: Vec::new(),
			waypoint: None,
			load_balancer: None,
			ip_families: None,
		}
	}

	fn multi_port_list() -> PortList {
		PortList {
			ports: vec![
				types::proto::workload::Port {
					service_port: 8000,
					target_port: 8000,
					app_protocol: 0,
				},
				types::proto::workload::Port {
					service_port: 8001,
					target_port: 8001,
					app_protocol: 0,
				},
			],
		}
	}

	#[tokio::test]
	async fn insert_endpoint_for_services_splits_multi_port_inference_pool_entries() {
		let service_name = inference_service_name();
		let workload = test_workload(&service_name);
		let services = HashMap::from([(service_name.to_string(), multi_port_list())]);
		let mut store = ServiceStore::default();
		store.insert(test_service(&service_name));

		store
			.insert_endpoint_for_services(&workload, &services)
			.expect("inference pool endpoints should be inserted");

		let svc = store
			.get_by_namespaced_host(&service_name)
			.expect("service should exist");
		let binding = svc.endpoints.iter();
		let endpoints = binding.index();
		assert_eq!(endpoints.len(), 2);

		let first = endpoints
			.get(&strng::new("wl-1:8000"))
			.expect("missing 8000 endpoint");
		assert_eq!(first.endpoint.workload_uid.as_str(), "wl-1");
		assert_eq!(first.endpoint.port.get(&8000), Some(&8000));

		let second = endpoints
			.get(&strng::new("wl-1:8001"))
			.expect("missing 8001 endpoint");
		assert_eq!(second.endpoint.workload_uid.as_str(), "wl-1");
		assert_eq!(second.endpoint.port.get(&8000), Some(&8001));
	}

	#[tokio::test]
	async fn remove_endpoint_removes_all_multi_port_inference_pool_entries() {
		let service_name = inference_service_name();
		let workload = test_workload(&service_name);
		let services = HashMap::from([(service_name.to_string(), multi_port_list())]);
		let mut store = ServiceStore::default();
		store.insert(test_service(&service_name));
		store
			.insert_endpoint_for_services(&workload, &services)
			.expect("inference pool endpoints should be inserted");

		store.remove_endpoint(workload.as_ref());

		let svc = store
			.get_by_namespaced_host(&service_name)
			.expect("service should still exist");
		let binding = svc.endpoints.iter();
		assert!(binding.index().is_empty());
	}
}
