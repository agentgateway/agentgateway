use std::path::{Path, PathBuf, absolute};
use std::time::Duration;

use agent_core::prelude::*;
use agent_core::readiness;
use notify::{EventKind, RecursiveMode};
use tokio::fs;

use crate::client::Client;
use crate::store::Stores;
use crate::types::agent::ListenerTarget;
use crate::types::discovery::SelfIdentitySource;
use crate::types::proto::agent::Resource as ADPResource;
use crate::types::proto::workload::Address as XdsAddress;
use crate::{ConfigSource, client, control, store};

#[derive(serde::Serialize)]
pub struct StateManager {
	#[serde(flatten)]
	stores: Stores,

	#[serde(skip_serializing)]
	xds_client: Option<agent_xds::AdsClient>,
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
		xds_metrics: agent_xds::Metrics,
		awaiting_ready: tokio::sync::watch::Sender<()>,
	) -> anyhow::Result<Self> {
		let xds = &config.xds;
		let stores = Stores::new(config.ipv6_enabled, config.threading_mode);
		match crate::http::oidc::OidcCookieEncoder::from_session_encoder(&config.session_encoder) {
			Ok(encoder) => {
				stores.binds.write().set_oidc_cookie_encoder(encoder);
			},
			Err(err) => {
				tracing::warn!(
					error = %err,
					"OIDC cookie encoder not installed; OIDC policies will be rejected at xDS decode until SESSION_KEY is configured"
				);
			},
		}
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
				.build(xds_metrics, awaiting_ready),
			)
		} else {
			None
		};
		if let Some(cfg) = &xds.local_config {
			let local_client = LocalClient {
				config: config.clone(),
				stores: stores.clone(),
				cfg: cfg.clone(),
				client,
				gateway: ListenerTarget {
					gateway_name: xds.gateway.clone(),
					gateway_namespace: xds.namespace.clone(),
					listener_name: None,
					port: None,
				},
			};
			Box::pin(local_client.run()).await?;
		}
		Ok(Self { stores, xds_client })
	}

	pub fn stores(&self) -> Stores {
		self.stores.clone()
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
	pub stores: Stores,
	pub client: Client,
	pub gateway: ListenerTarget,
}

impl LocalClient {
	pub async fn run(self) -> Result<(), anyhow::Error> {
		if let ConfigSource::File(path) = &self.cfg {
			// Load initial state then watch
			self.watch_config_file(path).await?;
		} else {
			// Load it once
			self.reload_config(PreviousState::default()).await?;
		}

		Ok(())
	}

	async fn watch_config_file(&self, path: &Path) -> anyhow::Result<()> {
		let (tx, mut rx) = tokio::sync::mpsc::channel(1);

		// Create a watcher with a 250ms debounce
		let mut watcher =
			notify_debouncer_full::new_debouncer(Duration::from_millis(250), None, move |res| {
				futures::executor::block_on(async {
					tx.send(res).await.unwrap();
				})
			})
			.map_err(|e| anyhow::anyhow!("Failed to create file watcher: {}", e))?;

		// Watch the config file
		let abspath = absolute(path)?;
		let parent = abspath.parent().ok_or(anyhow::anyhow!(
			"Failed to get the parent of the config file"
		))?;
		watcher
			.watch(parent, RecursiveMode::NonRecursive)
			.map_err(|e| anyhow::anyhow!("Failed to watch config file: {}", e))?;

		info!("Watching config file: {}", path.display());

		let lc: LocalClient = self.to_owned();
		let mut next_state = lc.reload_config(PreviousState::default()).await?;
		tokio::task::spawn(async move {
			// Resolve initial target (symlink or not)
			let mut real_config_path = lc.resolve_symlink(&abspath).await.ok();

			// Handle file change events
			while let Some(Ok(events)) = rx.recv().await {
				let current_config_path = lc.resolve_symlink(&abspath).await.ok();

				// Only process if we have actual content changes
				if events.iter().any(|e| {
					matches!(
						e.kind,
						EventKind::Modify(_) | EventKind::Create(_) if e.paths.last().is_some_and(|p| p == &abspath)
						|| (current_config_path.is_some() && current_config_path != real_config_path))
				}) {
					real_config_path = current_config_path.clone();
					debug!("Config file changed, reloading...");
					match lc.reload_config(next_state.clone()).await {
						Ok(nxt) => {
							next_state = nxt;
							debug!("Config reloaded successfully")
						},
						Err(e) => {
							error!("Failed to reload config: {}", e)
						},
					}
				}
			}
			drop(watcher);
		});

		Ok(())
	}

	/// Resolves a symlink to its final target. If the file is not a symlink, returns the original path.
	/// If symlink resolution fails, returns the original path as fallback.
	async fn resolve_symlink(&self, path: &Path) -> anyhow::Result<PathBuf> {
		match fs::symlink_metadata(path).await {
			Ok(metadata) if metadata.file_type().is_symlink() => {
				match fs::canonicalize(path).await {
					Ok(target) => Ok(target),
					Err(_) => Ok(path.to_path_buf()), // Fallback to original path on error
				}
			},
			Ok(_) => Ok(path.to_path_buf()),
			Err(_) => Ok(path.to_path_buf()), // Fallback to original path on metadata error
		}
	}

	async fn reload_config(&self, prev: PreviousState) -> anyhow::Result<PreviousState> {
		let config_content = self.cfg.read_to_string().await?;
		let config = crate::types::local::NormalizedLocalConfig::from(
			&self.config,
			self.client.clone(),
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
	use agent_core::readiness::Ready;

	use super::*;
	use crate::store::{DiscoveryPreviousState, LocalWorkload, Stores};
	use crate::types::discovery::Workload;

	const TASK_NAME: &str = "self workload";

	fn test_config() -> crate::Config {
		crate::config::parse_config("{}".to_string(), None).expect("parse default config")
	}

	fn test_stores() -> Stores {
		Stores::new(false, crate::ThreadingMode::Multithreaded)
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
}
