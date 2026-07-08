//! Opt-in MCP conformance harness: runs the upstream MCP Conformance Test
//! Framework (`github.com/modelcontextprotocol/conformance`) in server mode
//! against the gateway-as-proxy composite, and asserts the run matches a
//! committed expected-failures baseline.
//!
//! Topology (see `tests/conformance/README.md`):
//!
//!   conformance (server mode) -> agentgateway /mcp -> everything-server /mcp
//!
//! The suite acts as an MCP client, fires spec requests at the gateway, and
//! grades the responses. `--expected-failures` makes the run a ratchet: a newly
//! *failing* scenario fails the run (regression) and a newly *passing* one also
//! fails it (stale baseline entry). So the committed `baseline-<suite>.yml` files
//! are the progress meter toward the July (`2026-07-28`) release.
//!
//! Skipped unless opted in (mirrors `validate_examples.rs`'s `KEYCLOAK_AVAILABLE`
//! gate). The clone must be `npm install`ed in both the root and the
//! everything-server subdir first, and `RUST_MIN_STACK` must be raised:
//!
//!   export MCP_CONFORMANCE_DIR=/path/to/mcp-conformance
//!   (cd "$MCP_CONFORMANCE_DIR" && npm install)
//!   (cd "$MCP_CONFORMANCE_DIR/examples/servers/typescript" && npm install)
//!   export RUST_MIN_STACK=16777216
//!   MCP_CONFORMANCE=1 cargo test --test mcp_conformance -- --ignored --nocapture
//!
//! `RUST_MIN_STACK=16777216` (16 MiB) is mandatory: the MCP-proxy path enters the
//! large `make_backend_call` async fn twice on one tokio data-plane worker (the
//! inbound `Backend::MCP` arm, then the MCP upstream's own forward via
//! `PolicyClient`), and in a *debug* build the combined frame size overflows the
//! worker's default 2 MiB stack and aborts the process mid-scenario. Release
//! builds use smaller optimized frames and are unaffected, so this is a harness
//! workaround, not a production fix; shrinking the MCP-path frame is a follow-up.
//! `require_big_stack()` enforces this so a missing bump fails loudly, not as an
//! opaque abort. See `tests/conformance/README.md`.

// The gateway harness lives in the integration test's `common` tree. This target
// only needs `gateway.rs`, and dead_code is expected because it uses a subset of
// that helper.
#[allow(dead_code)]
#[path = "common/gateway.rs"]
mod gateway;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gateway::AgentGateway;
use tokio::process::Command;

/// Subdir of the conformance clone holding the reference `2026-07-28` upstream.
const EVERYTHING_SERVER_SUBDIR: &str = "examples/servers/typescript";

fn conformance_enabled() -> bool {
	std::env::var("MCP_CONFORMANCE")
		.map(|v| matches!(v.as_str(), "1" | "true"))
		.unwrap_or(false)
}

/// Resolved, validated path to the conformance clone, or `None` when the harness
/// is not opted in. Panics when opted in but mis-pointed, so a typo'd
/// `MCP_CONFORMANCE_DIR` fails loudly instead of silently skipping.
fn conformance_dir() -> Option<PathBuf> {
	if !conformance_enabled() {
		eprintln!("skipping: set MCP_CONFORMANCE=1 and MCP_CONFORMANCE_DIR=<clone> to run");
		return None;
	}
	let dir = std::env::var("MCP_CONFORMANCE_DIR")
		.expect("MCP_CONFORMANCE=1 set but MCP_CONFORMANCE_DIR is missing");
	let dir = PathBuf::from(dir);
	assert!(
		dir
			.join(EVERYTHING_SERVER_SUBDIR)
			.join("everything-server.ts")
			.is_file(),
		"MCP_CONFORMANCE_DIR={} does not look like the conformance clone (missing {}/everything-server.ts)",
		dir.display(),
		EVERYTHING_SERVER_SUBDIR,
	);
	Some(dir)
}

/// Path to a `node_modules/.bin/tsx` installed by `npm install`. Spawning this
/// binary directly (rather than `npm start`) yields a single killable process.
fn tsx_bin(dir: &Path) -> PathBuf {
	let tsx = dir.join("node_modules/.bin/tsx");
	assert!(
		tsx.is_file(),
		"{} not found. Run `npm install` in {}",
		tsx.display(),
		dir.display(),
	);
	tsx
}

/// 16 MiB, the stack the MCP-proxy path needs in a debug build (see module docs).
const REQUIRED_MIN_STACK: usize = 16 * 1024 * 1024;

/// Fail loudly if `RUST_MIN_STACK` is unset or below 16 MiB. Without it a debug
/// build aborts mid-scenario on a stack overflow rather than producing a graded
/// result, which reads as an inscrutable crash instead of a clear setup error.
fn require_big_stack() {
	let ok = std::env::var("RUST_MIN_STACK")
		.ok()
		.and_then(|v| v.parse::<usize>().ok())
		.is_some_and(|v| v >= REQUIRED_MIN_STACK);
	assert!(
		ok,
		"set RUST_MIN_STACK={REQUIRED_MIN_STACK} (16 MiB): the debug MCP-proxy path \
		 overflows the default 2 MiB worker stack. See tests/conformance/README.md",
	);
}

/// Grab an ephemeral port from the OS, then release it. Small TOCTOU window, but
/// acceptable for an opt-in manual harness.
fn free_port() -> u16 {
	TcpListener::bind("127.0.0.1:0")
		.expect("bind ephemeral port")
		.local_addr()
		.expect("local_addr")
		.port()
}

async fn wait_for_tcp(port: u16, timeout: Duration) -> anyhow::Result<()> {
	let start = std::time::Instant::now();
	while start.elapsed() < timeout {
		if tokio::net::TcpStream::connect(("127.0.0.1", port))
			.await
			.is_ok()
		{
			return Ok(());
		}
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	anyhow::bail!("timeout waiting for 127.0.0.1:{port}")
}

/// Boot a gateway forwarding `/mcp` to a single MCP upstream on `upstream_port`.
/// A single target keeps tool names unprefixed (the gateway prefixes every tool
/// when multiplexing >1 target), which conformance's `Mcp-Name` matching requires.
async fn gateway_fronting(upstream_port: u16) -> AgentGateway {
	AgentGateway::new(format!(
		r#"config: {{}}
binds:
- port: $PORT
  listeners:
  - routes:
    - backends:
      - mcp:
          targets:
          - name: everything
            mcp:
              host: http://localhost:{upstream_port}/mcp
      matches:
      - path:
          exact: /mcp
"#
	))
	.await
	.expect("boot gateway")
}

/// One suite (`active` | `modern-draft`) end to end: boot upstream + gateway, run
/// the suite against the committed baseline, assert the ratchet holds (exit 0).
///
/// `suite` is our label. The upstream framework knows the modern (`2026-07-28` RC)
/// scenario set only as `draft`, so `modern-draft` is mapped to `--suite draft` at
/// the CLI (below); everything we own — baseline file, `-o` dir, status keys — uses
/// the `modern-draft` label, matching the "modern" protocol terminology elsewhere.
///
/// The upstream server also differs by suite. `active` (legacy) runs against the
/// framework clone's reference everything-server. `modern-draft` runs against the
/// vendored strict v2 server (examples/mcp-conformance/everything-server-v2): the v1
/// reference does not enforce the SEP-2243/SEP-2322 prerequisites, so it passes
/// input-required flows that still fail against the stricter v2 server.
async fn run_suite(suite: &str) {
	let Some(dir) = conformance_dir() else { return };
	require_big_stack();
	// The framework has no `modern-draft` suite; its modern scenario set is `draft`.
	let framework_suite = if suite == "modern-draft" {
		"draft"
	} else {
		suite
	};
	let es_dir = if suite == "modern-draft" {
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("../../examples/mcp-conformance/everything-server-v2")
	} else {
		dir.join(EVERYTHING_SERVER_SUBDIR)
	};

	// 1. everything-server (the 2026-07-28 stateless upstream for this suite).
	let es_port = free_port();
	let _everything = Command::new(tsx_bin(&es_dir))
		.arg("everything-server.ts")
		.current_dir(&es_dir)
		.env("PORT", es_port.to_string())
		.kill_on_drop(true)
		.spawn()
		.expect("spawn everything-server");
	wait_for_tcp(es_port, Duration::from_secs(60))
		.await
		.expect("everything-server did not come up");

	// 2. gateway fronting it; single target, default prefix mode gives unprefixed names.
	let gw = gateway_fronting(es_port).await;
	let url = format!("http://127.0.0.1:{}/mcp", gw.port());

	// 3. run the suite from the local clone against the committed baseline.
	let baseline = format!(
		"{}/tests/conformance/baseline-{suite}.yml",
		env!("CARGO_MANIFEST_DIR")
	);
	let mut cmd = Command::new(tsx_bin(&dir));
	cmd
		.args(["src/index.ts", "server"])
		.args(["--url", &url])
		.args(["--suite", framework_suite])
		.args(["--expected-failures", &baseline])
		.current_dir(&dir);

	// Optional structured output for the HTML status report
	// (`make mcp-conformance-report`). Writes per-scenario checks.json under
	// `$MCP_CONFORMANCE_OUT/<suite>/`; grading against the baseline is unaffected.
	if let Ok(out) = std::env::var("MCP_CONFORMANCE_OUT") {
		cmd.args(["-o", &format!("{out}/{suite}")]);
	}

	let status = cmd.status().await.expect("run conformance suite");

	assert!(
		status.success(),
		"conformance suite '{suite}' did not match baseline {baseline} (exit {:?}). \
		 A new failure is a regression; a new pass means an entry must be removed. \
		 Regenerate per tests/conformance/README.md.",
		status.code(),
	);
}

#[tokio::test]
#[ignore = "opt-in: requires MCP_CONFORMANCE=1, an npm-installed MCP_CONFORMANCE_DIR clone, and RUST_MIN_STACK=16777216"]
async fn mcp_conformance_active() {
	// Legacy regression guard: dated releases up to 2025-11-25 (the downstream the
	// gateway supports today). Baseline should be near-empty.
	run_suite("active").await;
}

#[tokio::test]
#[ignore = "opt-in: requires MCP_CONFORMANCE=1, an npm-installed MCP_CONFORMANCE_DIR clone, an npm-installed everything-server-v2, and RUST_MIN_STACK=16777216"]
async fn mcp_conformance_modern_draft() {
	// 2026-07-28 modern (draft/RC) scenarios against the vendored strict v2 server.
	// Baseline captures the current input-required/MRTR gaps and shrinks when routing
	// lands. The framework suite id is `draft` (mapped in run_suite).
	run_suite("modern-draft").await;
}
