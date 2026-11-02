// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use agentgateway::ConfigSource;
use clap::{Args as ClapArgs, Parser, Subcommand};
use include_dir::Dir;
use pprof_alloc::Allocator;

mod commands;

// Keep embedded asset changes scoped to relinking the binary crate.
#[cfg(ui_assets)]
static UI_ASSETS: Dir<'static> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../ui/dist");
#[cfg(not(ui_assets))]
static UI_ASSETS: Dir<'static> = Dir::new("", &[]);

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

#[derive(ClapArgs, Debug)]
pub(crate) struct ImportArgs {
	/// Source gateway configuration format.
	#[arg(long, value_name = "source")]
	pub(crate) from: String,

	/// Read source configuration from a file.
	#[arg(short, long, value_name = "file")]
	pub(crate) file: PathBuf,

	/// Write generated agentgateway configuration to a file instead of stdout.
	#[arg(short, long, value_name = "file")]
	pub(crate) output: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
	/// Run agentgateway as a subprocess and exec a command when ready.
	#[cfg(target_os = "linux")]
	Oneshot(OneshotArgs),
	/// Import standalone configuration from another gateway.
	Import(ImportArgs),
	/// Migrate deprecated fields in a local agentgateway configuration.
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
	// Disable transparent huge pages process-wide. Allocator settings only cover their own mappings;
	// other mappings (such as thread stacks) can otherwise be backed by 2MB pages and inflate RSS.
	#[cfg(target_os = "linux")]
	unsafe {
		libc::prctl(libc::PR_SET_THP_DISABLE, 1, 0, 0, 0);
	}
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
	// Install the process-global crypto providers for the compiled-in backend
	// (currently the JWT provider) before any cryptographic work.
	agentgateway::crypto::init();
	let args = Cli::parse();
	match args.command {
		#[cfg(target_os = "linux")]
		Some(Commands::Oneshot(oneshot)) => commands::oneshot::execute(oneshot),
		Some(Commands::Import(import)) => commands::import::execute(import),
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
	let config_home = std::env::var_os("XDG_CONFIG_HOME")
		.filter(|path| !path.is_empty())
		.map(PathBuf::from)
		.or_else(|| home_dir().map(|home| home.join(".config")))
		.ok_or_else(|| {
			anyhow::anyhow!(
				"could not determine the user's home directory ({} is not set); pass --config or --file",
				home_var_name()
			)
		})?;
	Ok(config_home.join("agentgateway"))
}

/// Name of the environment variable holding the user's home directory on this platform.
fn home_var_name() -> &'static str {
	if cfg!(windows) { "USERPROFILE" } else { "HOME" }
}

/// The user's home directory, without its `.config` suffix.
///
/// Windows does not set `HOME`, so looking only at `HOME` left Windows users unable to find
/// their config file unless they exported it themselves (see #4219). `USERPROFILE` is what
/// Windows sets instead, with `HOMEDRIVE` + `HOMEPATH` as its legacy pair. Empty values are
/// skipped so they cannot resolve to a relative path.
///
/// CI runs the tests on Windows and macOS as well as Linux, so the platform-specific branch is
/// exercised for real rather than simulated.
fn home_dir() -> Option<PathBuf> {
	let var = |name: &str| {
		std::env::var_os(name)
			.filter(|value| !value.is_empty())
			.map(PathBuf::from)
	};
	if cfg!(windows) {
		return var("USERPROFILE").or_else(|| {
			let drive = var("HOMEDRIVE")?;
			let path = var("HOMEPATH")?;
			Some(drive.join(path))
		});
	}
	var("HOME").or_else(|| var("USERPROFILE")).or_else(|| {
		let drive = var("HOMEDRIVE")?;
		let path = var("HOMEPATH")?;
		Some(drive.join(path))
	})
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
	fs_err::write(path, default_config_contents(parent)?)?;
	Ok(())
}

fn default_config_contents(dir: &std::path::Path) -> anyhow::Result<String> {
	let db = dir.join("data.db");
	let config =
		agentgateway::types::local::default_standalone_config(&format!("sqlite://{}", db.display()));
	let yaml = agentgateway::yaml::to_string(&config)?;
	Ok(format!(
		"# yaml-language-server: $schema=https://agentgateway.dev/schema/config\n{yaml}"
	))
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

#[cfg(test)]
mod tests {
	use super::home_dir;

	/// The home directory this process reports, read from the same variables `home_dir` uses.
	///
	/// These tests read the real environment rather than injecting a lookup: CI runs the suite
	/// on Windows and macOS as well as Linux, so each platform's branch is exercised for real.
	fn expected_home() -> Option<std::path::PathBuf> {
		let var = |name: &str| {
			std::env::var_os(name)
				.filter(|value| !value.is_empty())
				.map(std::path::PathBuf::from)
		};
		if cfg!(windows) {
			return var("USERPROFILE").or_else(|| {
				let drive = var("HOMEDRIVE")?;
				let path = var("HOMEPATH")?;
				Some(drive.join(path))
			});
		}
		var("HOME")
	}

	#[test]
	fn home_dir_matches_the_platform_variable() {
		let home = home_dir();

		// Every CI runner sets a home directory, so this failing means the lookup is broken.
		assert!(
			home.is_some(),
			"home_dir() found nothing on {}",
			std::env::consts::OS
		);
		assert_eq!(home, expected_home());
	}

	#[test]
	#[cfg(windows)]
	fn home_dir_uses_userprofile_on_windows() {
		// The #4219 case: Windows sets USERPROFILE, not HOME.
		let userprofile = std::env::var_os("USERPROFILE").expect("USERPROFILE is set on Windows");

		assert_eq!(home_dir(), Some(std::path::PathBuf::from(userprofile)));
	}

	#[test]
	#[cfg(unix)]
	fn home_dir_uses_home_on_unix() {
		let home = std::env::var_os("HOME").expect("HOME is set on unix");

		assert_eq!(home_dir(), Some(std::path::PathBuf::from(home)));
	}

	#[test]
	fn home_dir_ignores_empty_variables() {
		// An empty variable must not resolve to a relative or empty path. Checked against the
		// live environment rather than by writing to it: a test that mutates the environment
		// races with the platform tests running in parallel.
		let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
		let is_empty = std::env::var_os(name).is_some_and(|value| value.is_empty());
		if !is_empty {
			// Nothing to assert about emptiness here; the platform tests cover the normal path.
			return;
		}

		let resolved = home_dir();

		assert!(
			resolved.as_ref().is_none_or(|p| !p.as_os_str().is_empty()),
			"an empty {name} must not produce an empty path"
		);
	}
}
