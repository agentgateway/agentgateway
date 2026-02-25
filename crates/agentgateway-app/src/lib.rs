// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::path::PathBuf;

use clap::{Args as ClapArgs, Parser, Subcommand};

mod commands;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Enable profiling, unless on musl due to https://github.com/tikv/jemallocator/issues/146
#[cfg(not(target_env = "musl"))]
#[cfg(feature = "jemalloc")]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

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

#[derive(Subcommand, Debug)]
enum Commands {
	/// Run agentgateway as a subprocess and exec a command when ready.
	#[cfg(unix)]
	Oneshot(OneshotArgs),
}

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
#[command(disable_version_flag = true)]
struct Cli {
	#[command(flatten)]
	run: RunArgs,

	#[cfg(target_os = "linux")]
	#[command(subcommand)]
	command: Option<Commands>,
}

pub fn run() -> anyhow::Result<()> {
	let args = Cli::parse();
	#[cfg(target_os = "linux")]
	match args.command {
		Some(Commands::Oneshot(oneshot)) => commands::oneshot::execute(oneshot),
		None => commands::run::execute(args.run),
	}
	#[cfg(not(target_os = "linux"))]
	commands::run::execute(args.run)
}

pub(crate) fn read_config_contents(
	config: &ConfigArgs,
) -> anyhow::Result<(String, Option<PathBuf>)> {
	match (&config.config, &config.file) {
		(Some(_), Some(_)) => anyhow::bail!("only one of --config or --file"),
		(Some(config), None) => Ok((config.clone(), None)),
		(None, Some(file)) => {
			let contents = fs_err::read_to_string(file)?;
			Ok((contents, Some(file.clone())))
		},
		(None, None) => Ok(("{}".to_string(), None)),
	}
}
