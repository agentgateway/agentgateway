// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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
		(None, None) => {
			if running_in_kubernetes() && std::env::var_os("LOCAL_XDS_PATH").is_none() {
				anyhow::bail!(
					"configuration is required when running in Kubernetes; pass --config, --file, or set LOCAL_XDS_PATH"
				);
			}
			if std::env::var_os("LOCAL_XDS_PATH").is_some() {
				return Ok(("{}".to_string(), None));
			}
			let dir = default_config_dir()?;
			let file = dir.join("config.yaml");
			ensure_default_config_file(&file)?;
			let contents = fs_err::read_to_string(&file)?;
			Ok((contents, Some(ConfigSource::File(file))))
		},
	}
}

fn running_in_kubernetes() -> bool {
	std::env::var_os("KUBERNETES_SERVICE_HOST").is_some()
}

fn default_config_dir() -> anyhow::Result<PathBuf> {
	let config_dir = Path::new("/config");
	if existing_writable_dir(config_dir) {
		return Ok(config_dir.to_path_buf());
	}
	if config_dir.exists() {
		anyhow::bail!(
			"{} exists but is not writable; make it writable, pass --file, or pass --config",
			config_dir.display()
		);
	}
	if running_in_official_container() {
		anyhow::bail!(
			"{} is not mounted; mount a writable {}, pass --file, or pass --config",
			config_dir.display(),
			config_dir.display()
		);
	}
	let home = std::env::var_os("HOME")
		.map(PathBuf::from)
		.ok_or_else(|| anyhow::anyhow!("HOME is not set; pass --config or --file"))?;
	Ok(home.join(".config").join("agentgateway"))
}

fn running_in_official_container() -> bool {
	std::env::var_os("AGENTGATEWAY_ENV").as_deref() == Some(OsStr::new("container"))
}

fn ensure_default_config_file(path: &std::path::Path) -> anyhow::Result<()> {
	if path.exists() {
		return Ok(());
	}
	if let Some(parent) = path.parent() {
		fs_err::create_dir_all(parent)?;
	}
	let parent = path
		.parent()
		.ok_or_else(|| anyhow::anyhow!("config path has no parent: {}", path.display()))?;
	fs_err::write(path, default_config_contents(parent))?;
	Ok(())
}

fn default_config_contents(dir: &std::path::Path) -> String {
	let db = dir.join("data.db");
	let admin = if running_in_official_container() {
		"  adminAddr: 0.0.0.0:15000\n"
	} else {
		""
	};
	format!(
		r#"# yaml-language-server: $schema=https://agentgateway.dev/schema/config
config:
{}  database:
    url: sqlite://{}
"#,
		admin,
		db.display()
	)
}

fn existing_writable_dir(path: &std::path::Path) -> bool {
	if !path.is_dir() {
		return false;
	}
	let probe = path.join(format!(".agentgateway-write-test-{}", std::process::id()));
	match fs_err::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&probe)
	{
		Ok(_) => {
			let _ = fs_err::remove_file(probe);
			true
		},
		Err(_) => false,
	}
}

fn is_read_once_path(path: &std::path::Path) -> bool {
	path == std::path::Path::new("/dev/stdin")
		|| path.starts_with("/dev/fd")
		|| path.starts_with("/proc/self/fd")
}
