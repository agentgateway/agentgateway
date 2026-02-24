use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{RunArgs, read_config_contents};
use agent_core::{strng, telemetry, version};
use agentgateway::app::Bound;
use agentgateway::types::agent::ListenerTarget;
use agentgateway::{BackendConfig, Config, LoggingFormat, client, serdes};
use tracing::info;

pub(crate) fn execute(args: RunArgs) -> anyhow::Result<()> {
	let RunArgs {
		config,
		validate_only,
		version_short,
		version_long,
		copy_self,
	} = args;

	if version_short {
		println!("{}", version::BuildInfo::new().version);
		return Ok(());
	}
	if version_long {
		println!("{}", version::BuildInfo::new());
		return Ok(());
	}
	if let Some(copy_self) = copy_self {
		return copy_binary(copy_self);
	}
	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap()
		.block_on(async move {
			let (contents, filename) = read_config_contents(&config)?;
			if validate_only {
				return validate(contents, filename).await;
			}
			let mut config = agentgateway::config::parse_config(contents, filename)?;
			// Capture the admin/runtime handle to ensure some background tasks (e.g., OTLP exporters created from dataplane
			// policy initialization) run on the admin runtime rather than the dataplane runtime.
			config.admin_runtime_handle = Some(tokio::runtime::Handle::current());
			let _log_flush = telemetry::setup_logging(
				&config.logging.level,
				config.logging.format == LoggingFormat::Json,
			);
			proxy(Arc::new(config)).await
		})
}

#[cfg(not(target_env = "musl"))]
fn copy_binary(_copy_self: PathBuf) -> anyhow::Result<()> {
	// This is a pretty sketchy command, only allow it in environments will use it
	anyhow::bail!("--copy-self is not supported in this build");
}

#[cfg(target_env = "musl")]
fn copy_binary(copy_self: PathBuf) -> anyhow::Result<()> {
	let Some(our_binary) = std::env::args().next() else {
		anyhow::bail!("no argv[0] set")
	};

	info!("copying our binary ({our_binary}) to {copy_self:?}");
	if let Some(parent) = copy_self.parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::copy(&our_binary, &copy_self)?;
	Ok(())
}

async fn validate(contents: String, filename: Option<PathBuf>) -> anyhow::Result<()> {
	let config = agentgateway::config::parse_config(contents, filename)?;
	let client = client::Client::new(&config.dns, None, BackendConfig::default(), None);
	if let Some(cfg) = config.xds.local_config.as_ref() {
		let cs = cfg.read_to_string().await?;
		agentgateway::types::local::NormalizedLocalConfig::from(
			&config,
			client,
			ListenerTarget {
				gateway_name: strng::literal!("default"),
				gateway_namespace: strng::literal!("default"),
				listener_name: None,
			},
			cs.as_str(),
		)
		.await?;
	} else {
		println!("No local configuration");
	}
	println!("Configuration is valid!");
	Ok(())
}

#[cfg(not(unix))]
fn spawn_readiness(_: &Bound) {}

#[cfg(unix)]
fn spawn_readiness(bound: &Bound) {
	use std::os::fd::{FromRawFd, OwnedFd};
	if let Some(ready_fd) = env::var("READY_FD")
		.ok()
		.and_then(|v| {
			let fd: i32 = v.parse().ok()?;
			Some(fd)
		})
		.map(|v| unsafe { OwnedFd::from_raw_fd(v) })
	{
		let ready = bound.readiness();
		tokio::spawn(async move {
			let mut ready_rx = ready.subscribe();
			if !*ready_rx.borrow() {
				loop {
					if ready_rx.changed().await.is_err() {
						return;
					}
					if *ready_rx.borrow() {
						break;
					}
				}
			}
			drop(ready_fd);
		});
	}
}

async fn proxy(cfg: Arc<Config>) -> anyhow::Result<()> {
	info!("version: {}", version::BuildInfo::new());
	info!(
		"running with config: {}",
		serdes::yamlviajson::to_string(&cfg)?
	);
	let bound = agentgateway::app::run(cfg).await?;
	spawn_readiness(&bound);
	bound.wait_termination().await
}
