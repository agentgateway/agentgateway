use std::sync::{Arc, Weak};

use arc_swap::ArcSwap;
use rustls::ServerConfig;
use tracing::{info, warn};

use crate::AdminTlsConfig;
use crate::types::agent::ServerTLSConfig;

#[derive(Clone)]
pub struct ReloadableServerConfig {
	current: Arc<ArcSwap<ServerConfig>>,
}

impl ReloadableServerConfig {
	pub async fn watch(config: AdminTlsConfig) -> anyhow::Result<Self> {
		let current = Arc::new(ArcSwap::from(build(&config).await?));
		let mut paths = vec![config.cert.clone(), config.key.clone()];
		paths.extend(config.client_ca.clone());
		let mut watched = crate::util::watch_files(paths)?;
		let weak = Arc::downgrade(&current);
		tokio::spawn(async move {
			while watched.changed().await {
				let Some(current) = Weak::upgrade(&weak) else {
					return;
				};
				match build(&config).await {
					Ok(reloaded) => {
						current.store(reloaded);
						info!("admin TLS certificates reloaded");
					},
					Err(err) => warn!(%err, "failed to reload admin TLS certificates, keeping previous"),
				}
			}
		});
		Ok(Self { current })
	}

	pub fn current(&self) -> Arc<ServerConfig> {
		self.current.load_full()
	}
}

async fn build(config: &AdminTlsConfig) -> anyhow::Result<Arc<ServerConfig>> {
	let cert = tokio::fs::read(&config.cert).await?;
	let key = tokio::fs::read(&config.key).await?;
	let client_ca = match &config.client_ca {
		Some(path) => Some(tokio::fs::read(path).await?),
		None => None,
	};
	ServerTLSConfig::from_pem(cert, key, client_ca, vec![b"http/1.1".to_vec()])?
		.config_for(None, None, None)
		.await
}
