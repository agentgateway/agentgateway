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
use rstest::rstest;

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

fn setup() {
	SETUP.get_or_init(|| {
		let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
		// CARGO_MANIFEST_DIR = crates/agentgateway  →  ../.. = workspace root
		let workspace_root = manifest_dir
			.join("../..")
			.canonicalize()
			.expect("workspace root should be resolvable");
		std::env::set_current_dir(&workspace_root)
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

async fn validate_example(path: &str) {
	setup();
	let yaml = std::fs::read_to_string(path)
		.unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
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
	.unwrap_or_else(|e| panic!("validation failed for {path}: {e}"));
}

/// Returns true when the external Keycloak instance (and the companion auth_server.py)
/// have been started via `tools/manage-validation-deps.sh start`.
fn keycloak_available() -> bool {
	std::env::var("KEYCLOAK_AVAILABLE")
		.map(|v| matches!(v.as_str(), "1" | "true"))
		.unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests that work without any external services
// ---------------------------------------------------------------------------

#[rstest]
#[case("examples/a2a/config.yaml")]
#[case("examples/ai-prompt-guard/config.yaml")]
#[case("examples/authorization/config.yaml")]
#[case("examples/aws-agentcore/config.yaml")]
#[case("examples/basic/config.yaml")]
#[case("examples/http/config.yaml")]
#[case("examples/multiplex/config.yaml")]
#[case("examples/oauth2-proxy/config.yaml")]
#[case("examples/openapi/config.yaml")]
#[case("examples/prompt-enrichment/config.yaml")]
#[case("examples/ratelimiting/global/config.yaml")]
#[case("examples/ratelimiting/local/config.yaml")]
#[case("examples/tailscale-auth/config.yaml")]
#[case("examples/telemetry/config.yaml")]
#[case("examples/tls/config.yaml")]
#[tokio::test]
async fn test_validate_example(#[case] path: &str) {
	validate_example(path).await;
}

// ---------------------------------------------------------------------------
// Tests that require an external Keycloak instance
// ---------------------------------------------------------------------------

#[rstest]
#[case("examples/mcp-authentication/config.yaml")]
#[case("examples/oidc/config.yaml")]
#[tokio::test]
async fn test_validate_example_with_keycloak(#[case] path: &str) {
	if !keycloak_available() {
		return;
	}
	validate_example(path).await;
}
