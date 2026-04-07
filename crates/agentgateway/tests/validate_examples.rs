/// Integration tests that validate all `examples/*/config.yaml` files using the same
/// logic as the `--validate-only` CLI flag, without requiring a full recompile/run cycle.
///
/// Tests that require an external Keycloak instance are skipped unless the
/// `KEYCLOAK_AVAILABLE` environment variable is set to `1` or `true`.
/// To run those tests locally, first start the dependencies with
/// `tools/manage-validation-deps.sh start` and then:
///
///   KEYCLOAK_AVAILABLE=1 cargo test --test validate_examples
use std::path::Path;
use std::sync::OnceLock;

use agentgateway::types::agent::ListenerTarget;
use agentgateway::types::local::NormalizedLocalConfig;
use agentgateway::{BackendConfig, client};

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// Deterministic 32-byte (64 hex-char) cookie secret used for configs that enable
/// OIDC browser auth, matching the value exported by `validate-configs.sh`.
const TEST_OIDC_COOKIE_SECRET: &str =
	"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Change the process working directory to the workspace root exactly once.
///
/// All example configs reference files (JWKS keys, TLS certs, OpenAPI schemas)
/// relative to the workspace root, mirroring what the binary does when run from
/// that directory.
static SETUP: OnceLock<()> = OnceLock::new();

fn workspace_root() -> &'static Path {
	static WORKSPACE_ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();
	WORKSPACE_ROOT.get_or_init(|| {
		Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../..")
			.canonicalize()
			.expect("workspace root should be resolvable")
	})
}

fn setup() {
	SETUP.get_or_init(|| {
		std::env::set_current_dir(workspace_root())
			.expect("should be able to set cwd to workspace root");
	});
}

fn test_config() -> agentgateway::Config {
	// Supply a deterministic OIDC cookie secret so configs that enable browser
	// auth (e.g. oidc/) can be compiled without errors, matching the behaviour of
	// validate-configs.sh which exports OIDC_COOKIE_SECRET.
	let mut config =
		agentgateway::config::parse_config("{}".to_string(), None).expect("parse empty config");
	config.oidc_cookie_encoder = Some(
		agentgateway::http::sessionpersistence::Encoder::aes(TEST_OIDC_COOKIE_SECRET)
			.expect("AES encoder"),
	);
	config
}

fn test_client(config: &agentgateway::Config) -> client::Client {
	client::Client::new(&config.dns, None, BackendConfig::default(), None)
}

async fn validate_example(path: &str) -> Result<(), String> {
	setup();
	let yaml = std::fs::read_to_string(path).map_err(|e| format!("failed to read {path}: {e}"))?;
	let config = test_config();
	let client = test_client(&config);
	NormalizedLocalConfig::from(
		&config,
		client,
		ListenerTarget {
			gateway_name: "default".into(),
			gateway_namespace: "default".into(),
			listener_name: None,
		},
		&yaml,
	)
	.await
	.map(|_| ())
	.map_err(|e| format!("validation failed for {path}: {e}"))
}

/// Returns true when the external Keycloak instance (and the companion auth_server.py)
/// have been started via `tools/manage-validation-deps.sh start`.
fn keycloak_available() -> bool {
	std::env::var("KEYCLOAK_AVAILABLE")
		.map(|v| matches!(v.as_str(), "1" | "true"))
		.unwrap_or(false)
}

fn example_configs() -> Vec<String> {
	fn walk(dir: &Path, configs: &mut Vec<std::path::PathBuf>) {
		let mut entries = std::fs::read_dir(dir)
			.unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()))
			.collect::<Result<Vec<_>, _>>()
			.unwrap_or_else(|e| panic!("failed to list {}: {e}", dir.display()));
		entries.sort_by_key(|entry| entry.path());

		for entry in entries {
			let path = entry.path();
			if path.is_dir() {
				walk(&path, configs);
			} else if path.file_name().is_some_and(|name| name == "config.yaml") {
				configs.push(path);
			}
		}
	}

	let mut configs = Vec::new();
	walk(&workspace_root().join("examples"), &mut configs);
	assert!(
		!configs.is_empty(),
		"expected at least one examples/**/config.yaml file"
	);
	configs
		.into_iter()
		.map(|path| {
			path
				.strip_prefix(workspace_root())
				.unwrap_or_else(|_| panic!("{} should live under the workspace root", path.display()))
				.to_string_lossy()
				.replace('\\', "/")
		})
		.collect()
}

fn example_name(path: &str) -> String {
	let parent = Path::new(path)
		.parent()
		.unwrap_or_else(|| panic!("{path} should have a parent folder"));
	parent
		.strip_prefix("examples")
		.unwrap_or(parent)
		.to_string_lossy()
		.into_owned()
}

fn example_requires_keycloak(path: &str) -> bool {
	let yaml = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
	yaml.contains("http://localhost:7080/realms/") || yaml.contains("http://localhost:9000")
}

#[tokio::test]
async fn test_validate_examples() {
	setup();
	let mut failures = Vec::new();

	for path in dbg!(example_configs()) {
		let name = example_name(&path);
		if example_requires_keycloak(&path) && !keycloak_available() {
			continue;
		}

		if let Err(err) = validate_example(&path).await {
			failures.push(format!("{name} ({path}): {err}"));
		}
	}

	assert!(
		failures.is_empty(),
		"example validation failed for:\n{}",
		failures.join("\n")
	);
}
