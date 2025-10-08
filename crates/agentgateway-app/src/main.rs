// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::path::PathBuf;
use std::sync::Arc;

use agent_core::{telemetry, version};
use agentgateway::{BackendConfig, Config, LoggingFormat, client, serdes};
use clap::Parser;
use tracing::info;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Enable profiling, unless on musl due to https://github.com/tikv/jemallocator/issues/146
#[cfg(not(target_env = "musl"))]
#[cfg(feature = "jemalloc")]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

lazy_static::lazy_static! {
	// The memory is intentionally leaked here using Box::leak to achieve a 'static lifetime
	// for the version string. This is necessary because the version string is used in a
	// context that requires a 'static lifetime.
	static ref LONG_VERSION: &'static str = Box::leak(version::BuildInfo::new().to_string().into_boxed_str());
	static ref SHORT_VERSION: &'static str = version::BuildInfo::new().version;
}

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
#[command(disable_version_flag = true)]
struct Args {
	/// Use config from bytes
	#[arg(short, long, value_name = "config")]
	config: Option<String>,

	/// Use config from file
	#[arg(short, long, value_name = "file")]
	file: Option<PathBuf>,

	#[arg(long, value_name = "validate-only")]
	validate_only: bool,

	/// Print version (as a simple version string)
	#[arg(short = 'V', value_name = "version")]
	version_short: bool,

	/// Print version (as JSON)
	#[arg(long = "version")]
	version_long: bool,

	/// Copy our own binary to a destination.
	#[arg(long = "copy-self", hide = true)]
	copy_self: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
	let args = Args::parse();
	let Args {
		config,
		file,
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
	return Ok(());
}

async fn validate(contents: String, filename: Option<PathBuf>) -> anyhow::Result<()> {
	let config = agentgateway::config::parse_config(contents, filename)?;
	let client = client::Client::new(&config.dns, None, BackendConfig::default());
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
