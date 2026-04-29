// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::path::PathBuf;

use clap::{Args as ClapArgs, Parser, Subcommand};

mod commands;

#[cfg(not(any(feature = "glibc", feature = "jemalloc", feature = "mimalloc",)))]
compile_error!("exactly one allocator feature must be enabled: glibc, jemalloc, or mimalloc");

#[cfg(any(
	all(feature = "glibc", feature = "jemalloc"),
	all(feature = "glibc", feature = "mimalloc"),
	all(feature = "jemalloc", feature = "mimalloc"),
))]
compile_error!(
	"allocator features are mutually exclusive; enable exactly one of glibc, jemalloc, or mimalloc"
);

#[cfg(any(feature = "glibc", all(feature = "jemalloc", not(target_os = "linux"))))]
#[global_allocator]
static GLOBAL: pprof_alloc::PprofAlloc<std::alloc::System> =
	pprof_alloc::PprofAlloc::from_allocator(std::alloc::System)
		.with_pprof_sample_rate_from_env(pprof_alloc::DEFAULT_PPROF_SAMPLE_RATE)
		.with_stats();

#[cfg(any(feature = "glibc", all(feature = "jemalloc", not(target_os = "linux"))))]
pprof_alloc::declare_allocator_kind!(pprof_alloc::allocator::AllocatorKind::Glibc);

#[cfg(all(feature = "jemalloc", target_os = "linux"))]
#[global_allocator]
static GLOBAL: pprof_alloc::PprofAlloc<tikv_jemallocator::Jemalloc> =
	pprof_alloc::PprofAlloc::from_allocator(tikv_jemallocator::Jemalloc)
		.with_pprof_sample_rate_from_env(pprof_alloc::DEFAULT_PPROF_SAMPLE_RATE)
		.with_stats();

#[cfg(all(feature = "jemalloc", target_os = "linux"))]
pprof_alloc::declare_allocator_kind!(pprof_alloc::allocator::AllocatorKind::Jemalloc);

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: pprof_alloc::PprofAlloc<mimalloc::MiMalloc> =
	pprof_alloc::PprofAlloc::from_allocator(mimalloc::MiMalloc)
		.with_pprof_sample_rate_from_env(pprof_alloc::DEFAULT_PPROF_SAMPLE_RATE)
		.with_stats();

#[cfg(feature = "mimalloc")]
pprof_alloc::declare_allocator_kind!(pprof_alloc::allocator::AllocatorKind::Mimalloc);

#[cfg(all(feature = "jemalloc", target_os = "linux"))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] = b"thp:never,background_thread:true,dirty_decay_ms:5000,muzzy_decay_ms:5000\0";

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
