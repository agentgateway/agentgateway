// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::path::PathBuf;

use agentgateway::ConfigSource;
use clap::{Args as ClapArgs, Parser, Subcommand};
use pprof_alloc::Allocator;

mod commands;

cfg_select! {
	all(target_os = "linux", target_env = "musl", target_arch = "aarch64") => {
		#[global_allocator]
		static GLOBAL:  pprof_alloc::PprofAlloc = pprof_alloc::PprofAlloc::new()
							.with_default(pprof_alloc::Allocator::Mimalloc);
	}
	target_os = "linux" => {
		#[global_allocator]
		static GLOBAL:  pprof_alloc::PprofAlloc = pprof_alloc::PprofAlloc::new()
							.with_default(pprof_alloc::Allocator::Jemalloc)
							.with_pprof()
							.with_stats();
	}
	_ => {
		#[global_allocator]
		static GLOBAL:  pprof_alloc::PprofAlloc = pprof_alloc::PprofAlloc::new()
							.with_default(pprof_alloc::Allocator::System);
	}
}

#[cfg(all(feature = "jemalloc", target_os = "linux", target_env = "musl"))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] =
	b"thp:never,background_thread:true,dirty_decay_ms:5000,muzzy_decay_ms:5000\0";

#[cfg(all(feature = "jemalloc", target_os = "linux", not(target_env = "musl")))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] =
	b"thp:never,background_thread:true,dirty_decay_ms:5000,muzzy_decay_ms:5000,prof:true\0";

#[derive(ClapArgs, Debug, Clone)]
pub(crate) struct ConfigArgs {
	/// Use config from bytes
	#[arg(short, long, value_name = "config")]
	pub(crate) config: Option<String>,

	/// Use config from file
	#[arg(short, long, value_name = "file")]
	pub(crate) file: Option<PathBuf>,
}

#[derive(ClapArgs, Debug)]
pub(crate) struct RunArgs {
	#[command(flatten)]
	pub(crate) config: ConfigArgs,

	#[arg(long, value_name = "validate-only")]
	pub(crate) validate_only: bool,

	/// Print version (as a simple version string)
	#[arg(short = 'V', value_name = "version")]
	pub(crate) version_short: bool,

	/// Print version (as JSON)
	#[arg(long = "version")]
	pub(crate) version_long: bool,

	/// Copy our own binary to a destination.
	#[arg(long = "copy-self", hide = true)]
	pub(crate) copy_self: Option<PathBuf>,
}

#[derive(ClapArgs, Debug)]
pub(crate) struct OneshotArgs {
	#[command(flatten)]
	pub(crate) config: ConfigArgs,

	/// Write agentgateway subprocess stdout/stderr to a file (use /dev/null to silence).
	#[arg(long = "output", value_name = "file")]
	pub(crate) agentgateway_output: Option<PathBuf>,

	/// Command to run after agentgateway is ready.
	#[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
	pub(crate) command: Vec<std::ffi::OsString>,
}

#[derive(ClapArgs, Debug)]
pub(crate) struct MigrateArgs {
	/// Use config from file
	#[arg(short, long, value_name = "file")]
	pub(crate) file: PathBuf,
}

#[derive(Subcommand, Debug)]
enum Commands {
	/// Run agentgateway as a subprocess and exec a command when ready.
	#[cfg(target_os = "linux")]
	Oneshot(OneshotArgs),
	/// Migrate deprecated local config fields to frontendPolicies.
	Migrate(MigrateArgs),
}

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
#[command(disable_version_flag = true)]
struct Cli {
	#[command(flatten)]
	run: RunArgs,

	#[command(subcommand)]
	command: Option<Commands>,
}

pub fn run() -> anyhow::Result<()> {
	cfg_select! {
		all(target_os = "linux", target_env = "musl", target_arch = "aarch64") => {
			pprof_alloc::configure_with_default(Allocator::Mimalloc)?;
		}
		target_os = "linux" => {
			pprof_alloc::configure_with_default(Allocator::Jemalloc)?;
		}
		_ => {
			pprof_alloc::configure_with_default(Allocator::System)?;
		}
	}
	let args = Cli::parse();
	match args.command {
		#[cfg(target_os = "linux")]
		Some(Commands::Oneshot(oneshot)) => commands::oneshot::execute(oneshot),
		Some(Commands::Migrate(migrate)) => commands::migrate::execute(migrate),
		None => commands::run::execute(args.run),
	}
}

pub(crate) fn read_config_contents(
	config: &ConfigArgs,
) -> anyhow::Result<(String, Option<ConfigSource>)> {
	match (&config.config, &config.file) {
		(Some(_), Some(_)) => anyhow::bail!("only one of --config or --file"),
		(Some(config), None) => Ok((
			config.clone(),
			Some(ConfigSource::Static(config.clone().into())),
		)),
		(None, Some(file)) => {
			let file = if file == std::path::Path::new("-") {
				&PathBuf::from("/dev/stdin")
			} else {
				file
			};

			let contents = fs_err::read_to_string(file)?;
			let source = if is_read_once_path(file) {
				ConfigSource::Static(contents.clone().into())
			} else {
				ConfigSource::File(file.clone())
			};
			Ok((contents, Some(source)))
		},
		(None, None) => Ok(("{}".to_string(), None)),
	}
}

fn is_read_once_path(path: &std::path::Path) -> bool {
	path == std::path::Path::new("/dev/stdin")
		|| path.starts_with("/dev/fd")
		|| path.starts_with("/proc/self/fd")
}
