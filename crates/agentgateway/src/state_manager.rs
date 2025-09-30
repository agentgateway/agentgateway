use std::path::{Path, PathBuf, absolute};
use std::time::Duration;

use agent_core::prelude::*;
use notify::{EventKind, RecursiveMode};
use tokio::fs;

use crate::client::Client;
use crate::store::Stores;
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

pub const ADDRESS_TYPE: Strng =
	strng::literal!("type.googleapis.com/agentgateway.dev.workload.Address");
pub const AUTHORIZATION_TYPE: Strng =
	strng::literal!("type.googleapis.com/istio.security.Authorization");
pub const ADP_TYPE: Strng =
	strng::literal!("type.googleapis.com/agentgateway.dev.resource.Resource");

impl StateManager {
	pub async fn new(
		config: &crate::XDSConfig,
		client: client::Client,
		xds_metrics: agent_xds::Metrics,
		awaiting_ready: tokio::sync::watch::Sender<()>,
	) -> anyhow::Result<Self> {
		let stores = Stores::new();

		let xds_client = if config.address.is_some() {
			let connector = control::grpc_connector(
				client.clone(),
				config.address.as_ref().unwrap().clone(),
				config.auth.clone(),
				config.ca_cert.clone(),
			)
			.await?;
			Some(
				agent_xds::Config::new(
					agent_xds::GrpcClient::new(connector),
					config.gateway.clone(),
					config.namespace.clone(),
				)
				.with_watched_handler::<XdsAddress>(ADDRESS_TYPE, stores.clone().discovery.clone())
				.with_watched_handler::<ADPResource>(ADP_TYPE, stores.clone().binds.clone())
				// .with_watched_handler::<XdsAuthorization>(AUTHORIZATION_TYPE, state)
				.build(xds_metrics, awaiting_ready),
			)
		} else {
			None
		};
		if let Some(cfg) = &config.local_config {
			let local_client = LocalClient {
				stores: stores.clone(),
				cfg: cfg.clone(),
				client,
			};
			local_client.run().await?;
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
	pub cfg: ConfigSource,
	pub stores: Stores,
	pub client: Client,
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
					info!("Config file changed, reloading...");
					match lc.reload_config(next_state.clone()).await {
						Ok(nxt) => {
							next_state = nxt;
							info!("Config reloaded successfully")
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
			self.client.clone(),
			config_content.as_str(),
		)
		.await?;
		info!("loaded config from {:?}", self.cfg);

		// Sync the state
		let next_binds =
			self
				.stores
				.binds
				.sync_local(config.binds, config.policies, config.backends, prev.binds);
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
