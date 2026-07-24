use std::path::Path;
use std::time::Duration;

use agent_core::prelude::*;
use agent_core::readiness;

use crate::client::Client;
use crate::store::Stores;
use crate::types::agent::ListenerTarget;
use crate::types::discovery::SelfIdentitySource;
use crate::types::proto::agent::Resource as ADPResource;
use crate::types::proto::workload::Address as XdsAddress;
use crate::{ConfigSource, ConfigStoreMode, client, config_store, control, store};

#[derive(serde::Serialize)]
pub struct StateManager {
	#[serde(flatten)]
	stores: Stores,

	#[serde(skip_serializing)]
	xds_client: Option<agent_xds::AdsClient>,

	#[serde(skip_serializing)]
	resource_manager: crate::resource_manager::ResourceManager,
}

pub const ADDRESS_TYPE: Strng = strng::literal!("type.googleapis.com/istio.workload.Address");
pub const AUTHORIZATION_TYPE: Strng =
	strng::literal!("type.googleapis.com/istio.security.Authorization");
pub const ADP_TYPE: Strng =
	strng::literal!("type.googleapis.com/agentgateway.dev.resource.Resource");

impl StateManager {
	pub async fn new(
		config: Arc<crate::Config>,
		client: client::Client,
		config_metrics: Arc<agent_xds::Metrics>,
		awaiting_ready: tokio::sync::watch::Sender<()>,
		config_resource_store: Option<config_store::ConfigResourceStore>,
	) -> anyhow::Result<Self> {
		let xds = &config.xds;
		let stores = Stores::new_with_dynamic_ca_cert_cache(
			config.ipv6_enabled,
			config.threading_mode,
			config.dynamic_ca_cert_cache.clone(),
		);
		let resource_manager = crate::resource_manager::ResourceManager::new(client.clone())?;
		let xds_client = if let Some(addr) = &xds.address {
			let connector = control::grpc_connector(
				client.clone(),
				addr.clone(),
				xds.auth.clone(),
				xds.ca_cert.clone(),
				vec![],
			)
			.await?;
			Some(
				agent_xds::Config::new(
					agent_xds::GrpcClient::new(connector),
					xds.gateway.clone(),
					xds.namespace.clone(),
				)
				.with_watched_handler::<XdsAddress>(ADDRESS_TYPE, stores.clone().discovery.clone())
				.with_watched_handler::<ADPResource>(ADP_TYPE, stores.clone().binds.clone())
				// .with_watched_handler::<XdsAuthorization>(AUTHORIZATION_TYPE, state)
				.build(config_metrics.clone(), awaiting_ready),
			)
		} else {
			None
		};
		if let Some(cfg) = &xds.local_config {
			let local_client = LocalClient {
				config: config.clone(),
				stores: stores.clone(),
				cfg: cfg.clone(),
				config_resource_store: config_resource_store.clone(),
				client,
				resource_manager: resource_manager.clone(),
				gateway: ListenerTarget {
					gateway_name: xds.gateway.clone(),
					gateway_namespace: xds.namespace.clone(),
					listener_name: None,
					port: None,
				},
				metrics: config_metrics,
			};
			Box::pin(local_client.run()).await?;
		}
		Ok(Self {
			stores,
			xds_client,
			resource_manager,
		})
	}

	pub fn stores(&self) -> Stores {
		self.stores.clone()
	}

	pub fn resource_manager(&self) -> crate::resource_manager::ResourceManager {
		self.resource_manager.clone()
	}

	pub async fn run(self) -> anyhow::Result<()> {
		match self.xds_client {
			Some(xds) => xds.run().await.map_err(|e| anyhow::anyhow!(e)),
			None => Ok(()),
		}
	}
}

/// LocalClient serves as a local file reader alternative for XDS. This is intended for testing.
#[derive(Debug, Clone)]
pub struct LocalClient {
	config: Arc<crate::Config>,
	pub cfg: ConfigSource,
	pub config_resource_store: Option<config_store::ConfigResourceStore>,
	pub stores: Stores,
	pub client: Client,
	pub resource_manager: crate::resource_manager::ResourceManager,
	pub gateway: ListenerTarget,
	pub metrics: Arc<agent_xds::Metrics>,
}

impl LocalClient {
	pub async fn run(self) -> Result<(), anyhow::Error> {
		let next_state = self.reload_config(PreviousState::default()).await?;
		if let ConfigSource::File(path) = &self.cfg {
			self.watch_config_file(path, next_state).await?;
		} else {
			self.watch_resource_changes(next_state);
		}

		Ok(())
	}

	async fn watch_config_file(
		&self,
		path: &Path,
		mut next_state: PreviousState,
	) -> anyhow::Result<()> {
		let watch_options = crate::util::WatchFilesOptions::default().close_on_removal(true);
		let mut watched =
			crate::util::watch_files_with_options(vec![path.to_path_buf()], watch_options)?;
		info!("Watching config file: {}", path.display());

		let lc: LocalClient = self.to_owned();
		let path = path.to_path_buf();
		let mut resource_changes = lc.resource_manager.subscribe_changes();
		let mut config_store_changes = lc
			.config_resource_store
			.as_ref()
			.map(config_store::ConfigResourceStore::subscribe_changes);
		tokio::task::spawn(async move {
			loop {
				tokio::select! {
					changed = watched.changed_invalidated() => {
						let Some(invalidated) = changed else {
							break;
						};
						next_state = lc.reload_config_after_change(next_state).await;
						if invalidated {
							match crate::util::watch_files_with_options(vec![path.clone()], watch_options) {
								Ok(new_watched) => watched = new_watched,
								Err(e) => {
									warn!("failed to re-watch config file {}: {e}", path.display());
									break;
								},
							}
						}
					}
					changed = resource_changes.changed() => {
						if changed.is_err() {
							break;
						}
						let resource = resource_changes.borrow().resource.clone();
						info!(resource, "resource changed, reloading");
						next_state = lc.reload_config_after_change(next_state).await;
					}
					changed = async {
						match &mut config_store_changes {
							Some(changes) => changes.changed().await,
							None => std::future::pending().await,
						}
					} => {
						if changed.is_err() {
							break;
						}
						info!("config resource changed, reloading");
						next_state = lc.reload_config_after_change(next_state).await;
					}
				}
			}
		});

		Ok(())
	}

	fn watch_resource_changes(&self, mut next_state: PreviousState) {
		let lc = self.clone();
		let mut resource_changes = self.resource_manager.subscribe_changes();
		let mut config_store_changes = self
			.config_resource_store
			.as_ref()
			.map(config_store::ConfigResourceStore::subscribe_changes);
		tokio::task::spawn(async move {
			loop {
				tokio::select! {
					changed = resource_changes.changed() => {
						if changed.is_err() {
							break;
						}
						let resource = resource_changes.borrow().resource.clone();
						info!(resource, "resource changed, reloading");
						next_state = lc.reload_config_after_change(next_state).await;
					}
					changed = async {
						match &mut config_store_changes {
							Some(changes) => changes.changed().await,
							None => std::future::pending().await,
						}
					} => {
						if changed.is_err() {
							break;
						}
						info!("config resource changed, reloading");
						next_state = lc.reload_config_after_change(next_state).await;
					}
				}
			}
		});
	}

	async fn reload_config(&self, prev: PreviousState) -> anyhow::Result<PreviousState> {
		let config_content = self.config_content().await?;
		let resources =
			crate::resource_manager::ResourceFetcher::managed(self.resource_manager.clone());
		let config = crate::types::local::NormalizedLocalConfig::from(
			&self.config,
			&resources,
			self.gateway.clone(),
			config_content.as_str(),
		)
		.await?;
		info!("loaded config from {:?}", self.cfg);

		// Sync the state
		let next_binds = self.stores.binds.sync_local(
			config.binds,
			config.listener_routes,
			config.listener_tcp_routes,
			config.policies,
			config.backends,
			config.route_groups,
			prev.binds,
		);
		let next_discovery =
			self
				.stores
				.discovery
				.sync_local(config.services, config.workloads, prev.discovery)?;

		Ok(PreviousState {
			binds: next_binds,
			discovery: next_discovery,
		})
	}

	async fn config_content(&self) -> anyhow::Result<String> {
		if self.config.config_store.mode == ConfigStoreMode::Hybrid
			&& let Some(store) = &self.config_resource_store
		{
			return config_store::materialize_hybrid_config(&self.cfg, store).await;
		}
		self.cfg.read_to_string().await
	}

	async fn reload_config_after_change(&self, prev: PreviousState) -> PreviousState {
		debug!("Config dependency changed, reloading...");
		match self.reload_config(prev.clone()).await {
			Ok(nxt) => {
				self.metrics.config_synchronized.set(1);
				debug!("Config reloaded successfully");
				nxt
			},
			Err(e) => {
				self.metrics.config_synchronized.set(0);
				error!("Failed to reload config: {}", e);
				prev
			},
		}
	}
}

#[derive(Clone, Debug, Default)]
pub struct PreviousState {
	pub binds: store::BindPreviousState,
	pub discovery: store::DiscoveryPreviousState,
}

const SELF_WORKLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Populates the discovery store's self_workload according to `config.self_identity`.
///
/// For `Static`, sets the cached workload synchronously and rebuckets.
/// For `Wds`, blocks readiness until WDS delivers the workload or timeout expires.
pub fn start_self_workload_resolution(
	config: &crate::Config,
	stores: Stores,
	ready: &readiness::Ready,
) {
	match &config.self_identity {
		Some(SelfIdentitySource::Static(w)) => {
			let store = stores.discovery.read();
			store.self_workload.set((**w).clone());
			store.rebucket_all();
		},
		Some(SelfIdentitySource::Wds {
			name,
			namespace,
			cluster_id,
		}) => {
			let task = ready.register_task("self workload");
			let name = name.clone();
			let namespace = namespace.clone();
			let cluster_id = cluster_id.clone();
			let has_xds = config.xds.address.is_some();
			tokio::spawn(async move {
				watch_self_workload(stores, name, namespace, cluster_id, Some(task), has_xds).await;
			});
		},
		None => {},
	}
}

async fn watch_self_workload(
	stores: Stores,
	name: Strng,
	namespace: Strng,
	cluster_id: Strng,
	mut ready_task: Option<readiness::BlockReady>,
	has_xds: bool,
) {
	let mut inserts = stores.discovery.read().workloads.subscribe_inserts();

	// allow a cluster id mismatch as a very common misconfiguration is that the control plane and
	// dataplane mismatch on this but if we do hit a conflict (should be rare) we use the cluster_id
	// as a tiebreaker
	let lookup = || {
		let store = stores.discovery.read();
		store
			.workloads
			.find_by_name(&name, &namespace)
			.max_by_key(|w| w.cluster_id == cluster_id)
			.cloned()
	};

	{
		let store = stores.discovery.read();
		if let Some(w) = lookup() {
			store.self_workload.set((*w).clone());
			store.rebucket_all();
			return;
		}
	}

	// Without XDS nothing will ever insert workloads; drop the task and stop.
	if !has_xds {
		return;
	}

	// wait for any change before starting our timeout if the control plane is down, or xDS is
	// otherwise slow we don't want to bail early without locality info
	if inserts.changed().await.is_err() {
		return;
	}

	let deadline = tokio::time::sleep(SELF_WORKLOAD_TIMEOUT);
	tokio::pin!(deadline);
	loop {
		{
			let store = stores.discovery.read();
			if let Some(w) = lookup() {
				store.self_workload.set((*w).clone());
				store.rebucket_all();
				return;
			}
		}
		tokio::select! {
			_ = &mut deadline, if ready_task.is_some() => {
				warn!(
					%namespace, %name,
					"timed out waiting for own workload in WDS after {:?}; unblocking readiness, still watching",
					SELF_WORKLOAD_TIMEOUT
				);
				// drop the task, but keep looping so we can still populate the self_workload if it shows up later
				ready_task = None;
			}
			r = inserts.changed() => {
				if r.is_err() {
					return;
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use agent_core::readiness::Ready;

	use super::*;
	use crate::ConfigSource;
	use crate::store::{DiscoveryPreviousState, LocalWorkload, Stores};
	use crate::types::discovery::Workload;

	const TASK_NAME: &str = "self workload";

	fn test_config() -> crate::Config {
		crate::config::parse_config("{}".to_string(), None).expect("parse default config")
	}

	fn test_stores() -> Stores {
		Stores::new(false, crate::ThreadingMode::Multithreaded)
	}

	fn test_client() -> Client {
		Client::new(
			&client::Config {
				resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
				resolver_opts: hickory_resolver::config::ResolverOpts::default(),
			},
			None,
			crate::BackendConfig::default(),
			None,
		)
	}

	fn local_config(remove_field: &str) -> String {
		format!(
			r#"
frontendPolicies:
  accessLog:
    remove:
    - {remove_field}
"#
		)
	}

	fn wds_identity(name: &str, ns: &str, cluster: &str) -> SelfIdentitySource {
		SelfIdentitySource::Wds {
			name: name.into(),
			namespace: ns.into(),
			cluster_id: cluster.into(),
		}
	}

	async fn wait_task_dropped(ready: &Ready) {
		while ready.pending().contains(TASK_NAME) {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	}

	async fn replace_config(path: &Path, remove_field: &str) {
		let replacement = path.with_extension(format!("{remove_field}.tmp"));
		fs_err::tokio::write(&replacement, local_config(remove_field))
			.await
			.unwrap();
		fs_err::rename(&replacement, path).unwrap();
	}

	async fn wait_for_access_log_remove(config: &crate::Config, stores: &Stores, remove_field: &str) {
		tokio::time::timeout(Duration::from_secs(5), async {
			loop {
				let frontend = stores.binds.read().frontend_policies(config.gateway_ref());
				if frontend
					.access_log
					.as_ref()
					.is_some_and(|access_log| access_log.remove.contains(remove_field))
				{
					return;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
		})
		.await
		.unwrap_or_else(|_| panic!("timed out waiting for access log remove {remove_field}"));
	}

	#[tokio::test]
	async fn wds_without_xds_must_not_block_readiness_forever() {
		let mut config = test_config();
		assert!(
			config.xds.address.is_none(),
			"precondition violated — XDS_ADDRESS leaked from env"
		);
		config.self_identity = Some(wds_identity("gw", "ns", "c"));

		let stores = test_stores();
		let ready = Ready::new();
		start_self_workload_resolution(&config, stores, &ready);

		assert!(ready.pending().contains(TASK_NAME));

		tokio::time::timeout(Duration::from_secs(5), wait_task_dropped(&ready))
			.await
			.expect("'self workload' readiness task blocked forever without XDS");
	}

	#[tokio::test]
	async fn wds_populates_self_workload_when_matching_workload_is_inserted() {
		let mut config = test_config();
		config.xds.address = Some("http://example.invalid:15010".to_string());
		config.self_identity = Some(wds_identity("gw", "ns", "c"));

		let stores = test_stores();
		let ready = Ready::new();
		start_self_workload_resolution(&config, stores.clone(), &ready);

		let workload = Workload {
			uid: "uid-1".into(),
			name: "gw".into(),
			namespace: "ns".into(),
			cluster_id: "c".into(),
			..Default::default()
		};
		stores
			.discovery
			.sync_local(
				vec![],
				vec![LocalWorkload {
					workload,
					services: Default::default(),
				}],
				DiscoveryPreviousState::default(),
			)
			.expect("sync_local");

		tokio::time::timeout(Duration::from_secs(5), wait_task_dropped(&ready))
			.await
			.expect("task should clear once matching workload is inserted");
		assert!(stores.discovery.read().self_workload.get().is_some());
	}

	#[tokio::test]
	async fn file_config_reloads_after_repeated_rename_replacement() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("config.yaml");
		fs_err::tokio::write(&path, local_config("first"))
			.await
			.unwrap();

		let mut config = test_config();
		config.xds.local_config = Some(ConfigSource::File(path.clone()));
		let config = Arc::new(config);
		let stores = test_stores();
		let mut registry = prometheus_client::registry::Registry::default();
		let metrics = Arc::new(agent_xds::Metrics::new(&mut registry));
		let client = test_client();
		let resource_manager = crate::resource_manager::ResourceManager::new(client.clone()).unwrap();
		let local_client = LocalClient {
			config: config.clone(),
			cfg: ConfigSource::File(path.clone()),
			config_resource_store: None,
			stores: stores.clone(),
			client,
			resource_manager,
			gateway: config.gateway(),
			metrics,
		};

		local_client.run().await.unwrap();
		wait_for_access_log_remove(&config, &stores, "first").await;

		fs_err::tokio::write(&path, local_config("ready"))
			.await
			.unwrap();
		wait_for_access_log_remove(&config, &stores, "ready").await;

		replace_config(&path, "second").await;
		wait_for_access_log_remove(&config, &stores, "second").await;

		fs_err::tokio::write(&path, local_config("ready-again"))
			.await
			.unwrap();
		wait_for_access_log_remove(&config, &stores, "ready-again").await;

		replace_config(&path, "third").await;
		wait_for_access_log_remove(&config, &stores, "third").await;
	}
}
