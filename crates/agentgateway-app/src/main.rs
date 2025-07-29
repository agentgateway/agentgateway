// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::path::PathBuf;
use std::sync::Arc;

use agent_core::{telemetry, version};
use agentgateway::{Config, client, serdes};
use clap::Parser;
use tracing::info;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

lazy_static::lazy_static! {
	// The memory is intentionally leaked here using Box::leak to achieve a 'static lifetime
	// for the version string. This is necessary because the version string is used in a
	// context that requires a 'static lifetime.
	static ref LONG_VERSION: &'static str = Box::leak(version::BuildInfo::new().to_string().into_boxed_str());
}

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
#[command(version = *LONG_VERSION, long_version = *LONG_VERSION)]
struct Args {
	/// Use config from bytes
	#[arg(short, long, value_name = "config")]
	config: Option<String>,

	/// Use config from file
	#[arg(short, long, value_name = "file")]
	file: Option<PathBuf>,

	#[arg(long, value_name = "validate-only")]
	validate_only: bool,
}

fn main() -> anyhow::Result<()> {
	let _log_flush = telemetry::setup_logging();

	let args = Args::parse();

	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap()
		.block_on(async move {
			let Args {
				config,
				file,
				validate_only,
			} = args;

			let (contents, filename) = match (config, file) {
				(Some(_), Some(_)) => {
					anyhow::bail!("only one of --config or --file")
				},
				(Some(config), None) => (config, None),
				(None, Some(file)) => {
					let contents = fs_err::read_to_string(&file)?;
					(contents, Some(file))
				},
				(None, None) => ("{}".to_string(), None),
			};
			if validate_only {
				return validate(contents, filename).await;
			}
			let config = agentgateway::config::parse_config(contents, filename)?;
			proxy(Arc::new(config)).await
		})
}

async fn validate(contents: String, filename: Option<PathBuf>) -> anyhow::Result<()> {
	let config = agentgateway::config::parse_config(contents, filename)?;
	let client = client::Client::new(&config.dns, None);
	if let Some(cfg) = config.xds.local_config {
		let cs = cfg.read_to_string().await?;
		agentgateway::types::local::NormalizedLocalConfig::from(client, cs.as_str()).await?;
	} else {
		println!("No local configuration");
	}
	println!("Configuration is valid!");
	Ok(())
}

async fn proxy(cfg: Arc<Config>) -> anyhow::Result<()> {
	info!("version: {}", version::BuildInfo::new());
	info!(
		"running with config: {}",
		serdes::yamlviajson::to_string(&cfg)?
	);
	agentgateway::app::run(cfg).await?.wait_termination().await
}
