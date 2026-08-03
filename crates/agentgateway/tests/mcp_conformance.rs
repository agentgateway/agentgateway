//! Opt-in coverage of the official MCP conformance framework.
//!
//! Each graded suite is run directly against the official reference server and
//! through agentgateway. The direct run makes a gateway failure attributable
//! instead of assuming that every failed check is a gateway regression.

#[allow(dead_code)]
#[path = "common/gateway.rs"]
mod gateway;

use std::collections::{BTreeMap, BTreeSet};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gateway::AgentGateway;
use serde::Deserialize;
use tokio::process::{Child, Command};
use tokio::sync::OnceCell;

const FRAMEWORK_SHA: &str = include_str!("conformance/framework.sha");
const TYPESCRIPT_SDK_SHA: &str = include_str!("conformance/typescript-sdk.sha");
const INVENTORY: &str = include_str!("conformance/suite-inventory.json");
const CONFORMANCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance");
const TYPESCRIPT_SDK_SERVER: &str = "test/conformance/src/everythingServer.ts";

#[derive(Debug, Deserialize)]
struct SuiteInventory {
	framework: String,
	suites: BTreeMap<String, BTreeSet<String>>,
	#[serde(rename = "gatedPendingScenarios")]
	gated_pending_scenarios: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
struct ExpectedFailures {
	#[serde(default)]
	server: Vec<BaselineEntry>,
}

#[derive(Debug, Deserialize)]
struct BaselineAdapterOutput {
	path: String,
	#[serde(rename = "expectedFailures")]
	expected_failures: ExpectedFailures,
}

#[derive(Debug, Deserialize)]
struct BaselineEntry {
	scenario: String,
}

#[derive(Clone, Copy, Debug)]
enum Topology {
	Direct,
	Gateway,
}

impl Topology {
	fn name(self) -> &'static str {
		match self {
			Self::Direct => "direct",
			Self::Gateway => "gateway",
		}
	}
}

#[derive(Clone, Copy)]
enum RunMode {
	Grade,
	Capture,
	Probe,
}

#[derive(Clone, Copy)]
struct Suite {
	name: &'static str,
	framework_name: &'static str,
	inventory_name: &'static str,
	scenario: Option<&'static str>,
}

const RELEASE_2025_11_25: Suite = Suite {
	name: "2025-11-25",
	framework_name: "active",
	inventory_name: "2025-11-25",
	scenario: None,
};
const RELEASE_2026_07_28: Suite = Suite {
	name: "2026-07-28",
	// The pinned framework exposes released 2026-07-28 scenarios as `draft`.
	framework_name: "draft",
	inventory_name: "2026-07-28",
	scenario: None,
};
const PENDING: Suite = Suite {
	name: "pending",
	framework_name: "pending",
	inventory_name: "pending",
	scenario: None,
};
const PENDING_JSON_SCHEMA_2020_12: Suite = Suite {
	name: "pending-json-schema-2020-12",
	framework_name: "pending",
	inventory_name: "pending",
	scenario: Some("json-schema-2020-12"),
};
/// Suites graded against expected-failure baselines by `make mcp-conformance`.
const GRADED: [Suite; 3] = [
	RELEASE_2025_11_25,
	RELEASE_2026_07_28,
	PENDING_JSON_SCHEMA_2020_12,
];

impl Suite {
	fn scenarios(self, inventory: &SuiteInventory, scenario: Option<&str>) -> BTreeSet<String> {
		let available = inventory
			.suites
			.get(self.inventory_name)
			.expect("known inventory suite");
		match scenario {
			Some(scenario) => {
				assert!(
					available.contains(scenario),
					"{scenario} is not in the upstream {} suite",
					self.inventory_name
				);
				BTreeSet::from([scenario.to_string()])
			},
			None => available.clone(),
		}
	}
}

fn enabled() -> bool {
	std::env::var("MCP_CONFORMANCE")
		.map(|value| matches!(value.as_str(), "1" | "true"))
		.unwrap_or(false)
}

fn framework_sha() -> &'static str {
	FRAMEWORK_SHA.trim()
}

fn typescript_sdk_sha() -> &'static str {
	TYPESCRIPT_SDK_SHA.trim()
}

fn conformance_dir() -> Option<PathBuf> {
	if !enabled() {
		eprintln!("skipping: set MCP_CONFORMANCE=1 and MCP_CONFORMANCE_DIR=<clone> to run");
		return None;
	}
	let dir = PathBuf::from(
		std::env::var("MCP_CONFORMANCE_DIR")
			.expect("MCP_CONFORMANCE=1 requires MCP_CONFORMANCE_DIR=<clone>"),
	);
	Some(dir)
}

fn typescript_sdk_dir() -> PathBuf {
	let dir = PathBuf::from(
		std::env::var("MCP_TYPESCRIPT_SDK_DIR")
			.expect("MCP_CONFORMANCE=1 requires MCP_TYPESCRIPT_SDK_DIR=<typescript-sdk clone>"),
	);
	assert!(
		dir.join(TYPESCRIPT_SDK_SERVER).is_file(),
		"{} is not a TypeScript SDK clone; missing {TYPESCRIPT_SDK_SERVER}",
		dir.display()
	);
	dir
}

fn tsx_bin(dir: &Path, install_hint: &str) -> PathBuf {
	let tsx = dir.join("node_modules/.bin/tsx");
	assert!(
		tsx.is_file(),
		"{} missing; run {install_hint} in {}",
		tsx.display(),
		dir.display()
	);
	tsx
}

async fn command_output(mut command: Command, action: &str) -> String {
	let output = command
		.output()
		.await
		.unwrap_or_else(|error| panic!("{action}: {error}"));
	assert!(
		output.status.success(),
		"{action} failed ({}): {}",
		output.status,
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).expect("command output is UTF-8")
}

async fn assert_node_version() {
	let mut node = Command::new("node");
	node.arg("--version");
	let version = command_output(node, "read Node.js version").await;
	assert!(
		version.trim().starts_with("v24."),
		"Node.js 24 LTS is required by the pinned MCP conformance framework; found {}",
		version.trim()
	);
}

async fn assert_pinned_clean(dir: &Path, pin: &str, variable: &str) {
	let mut head = Command::new("git");
	head.args(["-C"]).arg(dir).args(["rev-parse", "HEAD"]);
	assert_eq!(
		command_output(head, &format!("read {variable} revision"))
			.await
			.trim(),
		pin,
		"{variable} must be checked out at {pin}"
	);

	let mut status = Command::new("git");
	status
		.args(["-C"])
		.arg(dir)
		.args(["status", "--porcelain", "--untracked-files=no"]);
	assert!(
		command_output(status, &format!("check {variable} worktree"))
			.await
			.trim()
			.is_empty(),
		"{variable} has tracked changes; use a clean checkout at {pin}"
	);
}

async fn preflight(dir: &Path) -> SuiteInventory {
	assert_node_version().await;
	assert_pinned_clean(dir, framework_sha(), "MCP_CONFORMANCE_DIR").await;

	let inventory: SuiteInventory =
		serde_json::from_str(INVENTORY).expect("valid suite inventory JSON");
	assert_eq!(
		inventory.framework,
		framework_sha(),
		"inventory pin must match framework.sha"
	);
	assert_inventory_shape(&inventory);

	// Re-run the committed generator so the check and the documented
	// regeneration path (`make mcp-conformance-inventory`) cannot drift.
	let mut enumerate = Command::new(tsx_bin(dir, "npm ci"));
	enumerate
		.arg(format!("{CONFORMANCE}/generate-inventory.ts"))
		.arg(dir)
		.current_dir(dir);
	let live = command_output(enumerate, "enumerate framework suites").await;
	assert_eq!(
		live, INVENTORY,
		"framework suites changed; regenerate with `make mcp-conformance-inventory` and re-triage"
	);

	validate_expected_failures(dir, &inventory).await;
	inventory
}

async fn preflight_typescript_sdk(dir: &Path) {
	assert_pinned_clean(dir, typescript_sdk_sha(), "MCP_TYPESCRIPT_SDK_DIR").await;
	tsx_bin(dir, "pnpm install --frozen-lockfile");
	assert!(
		dir
			.join("test/conformance/node_modules/@modelcontextprotocol/express/dist/index.mjs")
			.is_file(),
		"{} has no built workspace packages; run pnpm build:all",
		dir.display()
	);
}

fn assert_inventory_shape(inventory: &SuiteInventory) {
	let released_2025 = &inventory.suites[RELEASE_2025_11_25.name];
	let released = &inventory.suites[RELEASE_2026_07_28.name];
	let pending = &inventory.suites[PENDING.name];
	assert_eq!(released_2025.len(), 31);
	assert_eq!(released.len(), 19);
	assert_eq!(pending.len(), 14);
	assert_eq!(
		inventory.gated_pending_scenarios.len(),
		1,
		"each gated pending scenario needs a matching Suite constant and graded test"
	);
	assert!(
		inventory.gated_pending_scenarios.contains(
			PENDING_JSON_SCHEMA_2020_12
				.scenario
				.expect("pending scenario")
		),
		"the reviewed pending scenario needs a matching Suite constant and graded test"
	);
	assert_eq!(
		released
			.intersection(pending)
			.cloned()
			.collect::<BTreeSet<_>>(),
		BTreeSet::from([
			"http-custom-header-server-validation".to_string(),
			"http-header-validation".to_string()
		]),
		"the only 2026-07-28/pending overlap is the upstream SEP-2243 fixture gap"
	);
}

async fn validate_expected_failures(dir: &Path, inventory: &SuiteInventory) {
	let mut expectations = Vec::new();
	for suite in GRADED {
		if let Some(scenario) = suite.scenario {
			assert!(
				inventory.gated_pending_scenarios.contains(scenario),
				"{scenario} is not an approved additional pending scenario"
			);
			// report.py derives the status suite name from the scenario with this rule.
			assert_eq!(
				suite.name,
				format!("pending-{scenario}"),
				"gated pending suites must be named pending-<scenario>"
			);
		}
		let scenarios = suite.scenarios(inventory, suite.scenario);
		for topology in [Topology::Direct, Topology::Gateway] {
			expectations.push((
				expected_failures_path(topology, suite.name),
				suite.name,
				scenarios.clone(),
			));
		}
	}

	let mut parse = Command::new(tsx_bin(dir, "npm ci"));
	parse
		.arg(format!("{CONFORMANCE}/parse-expected-failures.ts"))
		.arg(dir)
		.args(expectations.iter().map(|(path, _, _)| path))
		.current_dir(dir);
	let parsed: Vec<BaselineAdapterOutput> = serde_json::from_str(
		command_output(parse, "parse expected failures with the upstream parser")
			.await
			.trim(),
	)
	.expect("expected-failures adapter JSON");
	assert_eq!(
		parsed.len(),
		expectations.len(),
		"adapter must return one entry per expected-failures file"
	);
	for (output, (path, suite, scenarios)) in parsed.iter().zip(&expectations) {
		assert_eq!(
			Path::new(&output.path),
			path,
			"adapter entries must be in input order"
		);
		for entry in &output.expected_failures.server {
			assert!(
				scenarios.contains(&entry.scenario),
				"{} references '{}' outside the {suite} suite",
				path.display(),
				entry.scenario
			);
		}
	}
}

fn expected_failures_path(topology: Topology, suite: &str) -> PathBuf {
	Path::new(CONFORMANCE).join(format!("expected-failures-{}-{suite}.yml", topology.name()))
}

fn free_port() -> u16 {
	TcpListener::bind("127.0.0.1:0")
		.expect("bind ephemeral port")
		.local_addr()
		.expect("read ephemeral port")
		.port()
}

async fn start_everything_server(typescript_sdk_dir: &Path) -> (Child, u16) {
	let port = free_port();
	let child = Command::new("node")
		.args(["--import", "tsx", TYPESCRIPT_SDK_SERVER])
		.current_dir(typescript_sdk_dir)
		.env("PORT", port.to_string())
		.kill_on_drop(true)
		.spawn()
		.expect("start official reference server");
	gateway::wait_for_port(port, Duration::from_secs(60))
		.await
		.unwrap_or_else(|_| panic!("timed out waiting for reference server on 127.0.0.1:{port}"));
	(child, port)
}

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
              host: http://127.0.0.1:{upstream_port}/mcp
      matches:
      - path:
          exact: /mcp
"#
	))
	.await
	.expect("start gateway")
}

fn output_dir(topology: Topology, suite: &str) -> PathBuf {
	let root = PathBuf::from(
		std::env::var("MCP_CONFORMANCE_OUT").expect("MCP_CONFORMANCE_OUT must name an output root"),
	);
	// Cargo runs test binaries from the package directory, so a relative root
	// would silently land under crates/agentgateway/.
	assert!(
		root.is_absolute(),
		"MCP_CONFORMANCE_OUT must be an absolute path, got {}",
		root.display()
	);
	std::fs::create_dir_all(&root).expect("create conformance output root");
	let output = root.join(format!("{}-{suite}", topology.name()));
	assert!(
		!output.exists(),
		"{} already exists; each conformance run needs a fresh output directory",
		output.display()
	);
	std::fs::create_dir(&output).expect("create conformance output directory");
	output
}

fn output_scenarios(
	output: &Path,
	expected: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
	let entries = std::fs::read_dir(output).map_err(|error| error.to_string())?;
	let mut actual = BTreeSet::new();
	for entry in entries.filter_map(Result::ok) {
		let path = entry.path();
		if !path.join("checks.json").is_file() {
			continue;
		}
		let name = path
			.file_name()
			.and_then(|name| name.to_str())
			.ok_or_else(|| format!("result directory name is not UTF-8: {}", path.display()))?;
		let matches = expected
			.iter()
			.filter(|scenario| name.starts_with(&format!("server-{scenario}-")))
			.collect::<Vec<_>>();
		if matches.len() != 1 {
			return Err(format!(
				"unexpected or ambiguous result directory {}",
				path.display()
			));
		}
		let scenario = matches[0].clone();
		if !actual.insert(scenario.clone()) {
			return Err(format!(
				"duplicate result for {scenario} at {}",
				path.display()
			));
		}
	}
	Ok(actual)
}

fn verify_complete_output(output: &Path, expected: &BTreeSet<String>) {
	let actual = output_scenarios(output, expected).unwrap_or_else(|error| panic!("{error}"));
	assert_eq!(
		actual,
		*expected,
		"incomplete or unexpected conformance output at {}; a thrown scenario can omit checks.json",
		output.display()
	);
}

fn verify_capture_output(output: &Path, expected: &BTreeSet<String>) {
	match output_scenarios(output, expected) {
		Ok(actual) if actual == *expected => {},
		Ok(actual) => eprintln!(
			"capture output at {} is incomplete: expected {expected:?}, found {actual:?}",
			output.display()
		),
		Err(error) => eprintln!("capture output at {} is invalid: {error}", output.display()),
	}
}

#[test]
#[should_panic(expected = "ambiguous result directory")]
fn ambiguous_result_directory_is_rejected() {
	let output = tempfile::tempdir().expect("create output directory");
	let result = output.path().join("server-short-long-123");
	std::fs::create_dir(&result).expect("create result directory");
	std::fs::write(result.join("checks.json"), "[]").expect("write checks");

	verify_complete_output(
		output.path(),
		&BTreeSet::from(["short".to_string(), "short-long".to_string()]),
	);
}

#[test]
#[should_panic(expected = "duplicate result")]
fn duplicate_result_directory_is_rejected() {
	let output = tempfile::tempdir().expect("create output directory");
	for suffix in ["one", "two"] {
		let result = output.path().join(format!("server-short-{suffix}"));
		std::fs::create_dir(&result).expect("create result directory");
		std::fs::write(result.join("checks.json"), "[]").expect("write checks");
	}

	verify_complete_output(output.path(), &BTreeSet::from(["short".to_string()]));
}

#[test]
fn incomplete_capture_output_is_retained() {
	let output = tempfile::tempdir().expect("create output directory");
	verify_capture_output(output.path(), &BTreeSet::from(["short".to_string()]));
}

// The shell driver runs every graded test in one process; the preflights only
// depend on the pinned clones, so run them once and share the result.
static PREFLIGHT: OnceCell<SuiteInventory> = OnceCell::const_new();
static TYPESCRIPT_SDK_PREFLIGHT: OnceCell<()> = OnceCell::const_new();

async fn run_suite(topology: Topology, suite: Suite, mode: RunMode, scenario: Option<&str>) {
	let Some(dir) = conformance_dir() else { return };
	let inventory = PREFLIGHT.get_or_init(|| preflight(&dir)).await;
	let typescript_sdk_dir = typescript_sdk_dir();
	TYPESCRIPT_SDK_PREFLIGHT
		.get_or_init(|| preflight_typescript_sdk(&typescript_sdk_dir))
		.await;
	let scenario = scenario.or(suite.scenario);
	let expected = suite.scenarios(inventory, scenario);
	let output = output_dir(topology, suite.name);
	let (_server, upstream_port) = start_everything_server(&typescript_sdk_dir).await;
	let gateway = match topology {
		Topology::Direct => None,
		Topology::Gateway => Some(gateway_fronting(upstream_port).await),
	};
	let url = match gateway.as_ref() {
		Some(gateway) => format!("http://127.0.0.1:{}/mcp", gateway.port()),
		None => format!("http://127.0.0.1:{upstream_port}/mcp"),
	};
	let mut command = Command::new(tsx_bin(&dir, "npm ci"));
	command
		.args([
			"src/index.ts",
			"server",
			"--url",
			&url,
			"--suite",
			suite.framework_name,
			"-o",
		])
		.arg(&output)
		.current_dir(&dir);
	if let Some(scenario) = scenario {
		command.args(["--scenario", scenario]);
	}
	if matches!(mode, RunMode::Grade) {
		command.args(["--expected-failures"]);
		command.arg(expected_failures_path(topology, suite.name));
	}
	let status = command
		.status()
		.await
		.expect("run official conformance suite");
	match mode {
		RunMode::Grade => {
			assert!(
				status.success(),
				"{}/{} did not match {}; new failures are regressions and newly passing baseline entries are stale",
				topology.name(),
				suite.name,
				expected_failures_path(topology, suite.name).display()
			);
			verify_complete_output(&output, &expected);
		},
		RunMode::Probe => {
			assert!(
				status.success(),
				"{}/{} did not pass against the pinned reference fixture",
				topology.name(),
				suite.name
			);
			verify_complete_output(&output, &expected);
		},
		RunMode::Capture => verify_capture_output(&output, &expected),
	}
}

#[tokio::test]
#[ignore = "opt-in: MCP_CONFORMANCE=1, MCP_CONFORMANCE_DIR=<pinned clone>, MCP_TYPESCRIPT_SDK_DIR=<pinned clone>, and MCP_CONFORMANCE_OUT=<output root>"]
async fn direct_2025_11_25() {
	run_suite(Topology::Direct, RELEASE_2025_11_25, RunMode::Grade, None).await;
}

#[tokio::test]
#[ignore = "opt-in: MCP_CONFORMANCE=1, MCP_CONFORMANCE_DIR=<pinned clone>, MCP_TYPESCRIPT_SDK_DIR=<pinned clone>, and MCP_CONFORMANCE_OUT=<output root>"]
async fn gateway_2025_11_25() {
	run_suite(Topology::Gateway, RELEASE_2025_11_25, RunMode::Grade, None).await;
}

#[tokio::test]
#[ignore = "opt-in: MCP_CONFORMANCE=1, MCP_CONFORMANCE_DIR=<pinned clone>, MCP_TYPESCRIPT_SDK_DIR=<pinned clone>, and MCP_CONFORMANCE_OUT=<output root>"]
async fn direct_2026_07_28() {
	run_suite(Topology::Direct, RELEASE_2026_07_28, RunMode::Grade, None).await;
}

#[tokio::test]
#[ignore = "opt-in: MCP_CONFORMANCE=1, MCP_CONFORMANCE_DIR=<pinned clone>, MCP_TYPESCRIPT_SDK_DIR=<pinned clone>, and MCP_CONFORMANCE_OUT=<output root>"]
async fn gateway_2026_07_28() {
	run_suite(Topology::Gateway, RELEASE_2026_07_28, RunMode::Grade, None).await;
}

#[tokio::test]
#[ignore = "opt-in: MCP_CONFORMANCE=1, MCP_CONFORMANCE_DIR=<pinned clone>, MCP_TYPESCRIPT_SDK_DIR=<pinned clone>, and MCP_CONFORMANCE_OUT=<output root>"]
async fn direct_pending_json_schema_2020_12() {
	run_suite(
		Topology::Direct,
		PENDING_JSON_SCHEMA_2020_12,
		RunMode::Grade,
		None,
	)
	.await;
}

#[tokio::test]
#[ignore = "opt-in: MCP_CONFORMANCE=1, MCP_CONFORMANCE_DIR=<pinned clone>, MCP_TYPESCRIPT_SDK_DIR=<pinned clone>, and MCP_CONFORMANCE_OUT=<output root>"]
async fn gateway_pending_json_schema_2020_12() {
	run_suite(
		Topology::Gateway,
		PENDING_JSON_SCHEMA_2020_12,
		RunMode::Grade,
		None,
	)
	.await;
}

#[tokio::test]
#[ignore = "opt-in pending-fixture probe: use make mcp-conformance-pending-availability SCENARIO=<scenario>"]
async fn pending_fixture_available() {
	let scenario = std::env::var("MCP_CONFORMANCE_SCENARIO")
		.expect("pending-fixture probe requires MCP_CONFORMANCE_SCENARIO=<pending scenario>");
	run_suite(Topology::Direct, PENDING, RunMode::Probe, Some(&scenario)).await;
}

#[tokio::test]
#[ignore = "opt-in capture: use make mcp-conformance-capture SUITE=<2025-11-25|2026-07-28|pending>"]
async fn capture() {
	if !enabled() {
		eprintln!("skipping: set MCP_CONFORMANCE=1 to run capture");
		return;
	}
	let suite =
		std::env::var("MCP_CONFORMANCE_SUITE").expect("capture requires MCP_CONFORMANCE_SUITE");
	let suite = match suite.as_str() {
		"2025-11-25" => RELEASE_2025_11_25,
		"2026-07-28" => RELEASE_2026_07_28,
		"pending" => PENDING,
		_ => panic!("MCP_CONFORMANCE_SUITE must be 2025-11-25, 2026-07-28, or pending"),
	};
	let topology = match std::env::var("MCP_CONFORMANCE_TOPOLOGY").as_deref() {
		Ok("direct") => Topology::Direct,
		Ok("gateway") => Topology::Gateway,
		_ => panic!("capture requires MCP_CONFORMANCE_TOPOLOGY=direct or gateway"),
	};
	let scenario = std::env::var("MCP_CONFORMANCE_SCENARIO")
		.ok()
		.filter(|scenario| !scenario.is_empty());
	run_suite(topology, suite, RunMode::Capture, scenario.as_deref()).await;
}
