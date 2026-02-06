// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::drain::DrainWatcher;
use agent_core::version::BuildInfo;
use agent_core::{signal, telemetry};
use hyper::Request;
use hyper::body::Incoming;
use hyper::header::{CONTENT_TYPE, HeaderValue};
use tokio::runtime::Handle;
use tokio::time;
use tracing::{info, warn};
use tracing_subscriber::filter;

use super::hyper_helpers::{Server, empty_response, plaintext_response};
use crate::Config;
use crate::http::Response;

// Constants for pprof profiling
#[cfg(target_os = "linux")]
const PPROF_DEFAULT_SECONDS: u64 = 10;
#[cfg(target_os = "linux")]
const PPROF_MIN_SECONDS: u64 = 1;
#[cfg(target_os = "linux")]
const PPROF_MAX_SECONDS: u64 = 300;

pub trait ConfigDumpHandler: Sync + Send {
	fn key(&self) -> &'static str;
	// sadly can't use async trait because no Sync
	// see: https://github.com/dtolnay/async-trait/issues/248, https://github.com/dtolnay/async-trait/issues/142
	// we can't use FutureExt::shared because our result is not clonable
	fn handle(&self) -> anyhow::Result<serde_json::Value>;
}

pub type AdminResponse = std::pin::Pin<Box<dyn Future<Output = crate::http::Response> + Send>>;

pub trait AdminFallback: Sync + Send {
	// sadly can't use async trait because no Sync
	// see: https://github.com/dtolnay/async-trait/issues/248, https://github.com/dtolnay/async-trait/issues/142
	// we can't use FutureExt::shared because our result is not clonable
	fn handle(&self, req: http::Request<Incoming>) -> AdminResponse;
}

struct State {
	stores: crate::store::Stores,
	config: Arc<Config>,
	shutdown_trigger: signal::ShutdownTrigger,
	config_dump_handlers: Vec<Arc<dyn ConfigDumpHandler>>,
	admin_fallback: Option<Arc<dyn AdminFallback>>,
	dataplane_handle: Handle,
}

pub struct Service {
	s: Server<State>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDump {
	#[serde(flatten)]
	stores: crate::store::Stores,
	version: BuildInfo,
	config: Arc<Config>,
}

#[derive(serde::Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CertDump {
	// Not available via Envoy, but still useful.
	pem: String,
	serial_number: String,
	valid_from: String,
	expiration_time: String,
}

#[derive(serde::Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CertsDump {
	identity: String,
	state: String,
	cert_chain: Vec<CertDump>,
	root_certs: Vec<CertDump>,
}

impl Service {
	pub async fn new(
		config: Arc<Config>,
		stores: crate::store::Stores,
		shutdown_trigger: signal::ShutdownTrigger,
		drain_rx: DrainWatcher,
		dataplane_handle: Handle,
	) -> anyhow::Result<Self> {
		Server::<State>::bind(
			"admin",
			config.admin_addr,
			drain_rx,
			State {
				config,
				stores,
				shutdown_trigger,
				config_dump_handlers: vec![],
				admin_fallback: None,
				dataplane_handle,
			},
		)
		.await
		.map(|s| Service { s })
	}

	pub fn address(&self) -> SocketAddr {
		self.s.address()
	}

	pub fn add_config_dump_handler(&mut self, handler: Arc<dyn ConfigDumpHandler>) {
		self.s.state_mut().config_dump_handlers.push(handler);
	}

	pub fn set_admin_handler(&mut self, handler: Arc<dyn AdminFallback>) {
		self.s.state_mut().admin_fallback = Some(handler);
	}

	pub fn spawn(self) {
		self.s.spawn(|state, req| async move {
			match req.uri().path() {
				#[cfg(target_os = "linux")]
				"/debug/pprof/profile" => handle_pprof(req).await,
				#[cfg(target_os = "linux")]
				"/debug/pprof/heap" => handle_jemalloc_pprof_heapgen(req).await,
				"/quitquitquit" => Ok(
					handle_server_shutdown(
						state.shutdown_trigger.clone(),
						req,
						state.config.termination_min_deadline,
					)
					.await,
				),
				"/debug/tasks" => handle_tokio_tasks(req, &state.dataplane_handle).await,
				"/config_dump" => {
					handle_config_dump(
						&state.config_dump_handlers,
						ConfigDump {
							stores: state.stores.clone(),
							version: BuildInfo::new(),
							config: state.config.clone(),
						},
					)
					.await
				},
				"/logging" => Ok(handle_logging(req).await),
				_ => {
					if let Some(h) = &state.admin_fallback {
						Ok(h.handle(req).await)
					} else if req.uri().path() == "/" {
						Ok(handle_dashboard(req).await)
					} else {
						Ok(empty_response(hyper::StatusCode::NOT_FOUND))
					}
				},
			}
		})
	}
}

async fn handle_dashboard(_req: Request<Incoming>) -> Response {
	let apis = &[
		(
			"debug/pprof/profile",
			"build profile using the pprof profiler (if supported). Use ?seconds=N to specify duration (1-300s, default: 10s)",
		),
		(
			"debug/pprof/heap",
			"collect heap profiling data (if supported, requires jmalloc)",
		),
		("quitquitquit", "shut down the server"),
		("config_dump", "dump the current agentgateway configuration"),
		("logging", "query/changing logging levels"),
	];

	let mut api_rows = String::new();

	for (index, (path, description)) in apis.iter().copied().enumerate() {
		api_rows.push_str(&format!(
            "<tr class=\"{row_class}\"><td class=\"home-data\"><a href=\"{path}\">{path}</a></td><td class=\"home-data\">{description}</td></tr>\n",
            row_class = if index % 2 == 1 { "gray" } else { "vert-space" },
            path = path,
            description = description
        ));
	}

	let html_str = include_str!("../assets/dashboard.html");
	let html_str = html_str.replace("<!--API_ROWS_PLACEHOLDER-->", &api_rows);

	let mut response = plaintext_response(hyper::StatusCode::OK, html_str);
	response.headers_mut().insert(
		CONTENT_TYPE,
		HeaderValue::from_static("text/html; charset=utf-8"),
	);

	response
}

#[cfg(target_os = "linux")]
async fn handle_pprof(req: Request<Incoming>) -> anyhow::Result<Response> {
	use pprof::protos::Message;

	// Parse query parameters to extract optional "seconds" parameter
	let qp: HashMap<String, String> = req
		.uri()
		.query()
		.map(|v| {
			url::form_urlencoded::parse(v.as_bytes())
				.into_owned()
				.collect()
		})
		.unwrap_or_default();

	// Extract seconds parameter with validation
	let seconds = if let Some(seconds_str) = qp.get("seconds") {
		match seconds_str.parse::<u64>() {
			Ok(s) if (PPROF_MIN_SECONDS..=PPROF_MAX_SECONDS).contains(&s) => s,
			_ => PPROF_DEFAULT_SECONDS, // Default if invalid or out of range
		}
	} else {
		PPROF_DEFAULT_SECONDS // Default if not provided
	};

	let guard = pprof::ProfilerGuardBuilder::default()
		.frequency(1000)
		// .blocklist(&["libc", "libgcc", "pthread", "vdso"])
		.build()?;

	tokio::time::sleep(Duration::from_secs(seconds)).await;
	let report = guard.report().build()?;
	let profile = report.pprof()?;

	let body = profile.write_to_bytes()?;

	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.body(body.into())
			.expect("builder with known status code should not fail"),
	)
}

async fn handle_server_shutdown(
	shutdown_trigger: signal::ShutdownTrigger,
	_req: Request<Incoming>,
	self_term_wait: Duration,
) -> Response {
	match *_req.method() {
		hyper::Method::POST => {
			match time::timeout(self_term_wait, shutdown_trigger.shutdown_now()).await {
				Ok(()) => info!("Shutdown completed gracefully"),
				Err(_) => warn!(
					"Graceful shutdown did not complete in {:?}, terminating now",
					self_term_wait
				),
			}
			plaintext_response(hyper::StatusCode::OK, "shutdown now\n".into())
		},
		_ => empty_response(hyper::StatusCode::METHOD_NOT_ALLOWED),
	}
}

#[cfg(target_os = "linux")]
#[derive(serde::Serialize)]
struct TaskDump {
	admin: Vec<String>,
	workload: Vec<String>,
}

#[cfg(target_os = "linux")]
async fn handle_tokio_tasks(
	_req: Request<Incoming>,
	dataplane_handle: &Handle,
) -> anyhow::Result<Response> {
	let mut task_dump = TaskDump {
		admin: Vec::new(),
		workload: Vec::new(),
	};

	let handle = tokio::runtime::Handle::current();
	if let Ok(dump) = tokio::time::timeout(Duration::from_secs(5), handle.dump()).await {
		for task in dump.tasks().iter() {
			let trace = task.trace();
			task_dump.admin.push(trace.to_string());
		}
	} else {
		task_dump
			.admin
			.push("failed to dump admin workload tasks".to_string());
	}

	if let Ok(dump) = tokio::time::timeout(Duration::from_secs(10), dataplane_handle.dump()).await {
		for task in dump.tasks().iter() {
			let trace = task.trace();
			task_dump.workload.push(trace.to_string());
		}
	} else {
		task_dump
			.workload
			.push("failed to dump workload tasks".to_string());
	}

	let json_body = serde_json::to_string(&task_dump)?;

	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.header("Content-Type", "application/json")
			.body(json_body.into())
			.expect("builder with known status code should not fail"),
	)
}

#[cfg(not(target_os = "linux"))]
async fn handle_tokio_tasks(
	_req: Request<Incoming>,
	_dataplane_handle: &Handle,
) -> anyhow::Result<Response> {
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
			.body("task dump is not available".into())
			.expect("builder with known status code should not fail"),
	)
}

async fn handle_config_dump(
	handlers: &[Arc<dyn ConfigDumpHandler>],
	dump: ConfigDump,
) -> anyhow::Result<Response> {
	let serde_json::Value::Object(mut kv) = serde_json::to_value(&dump)? else {
		anyhow::bail!("config dump is not a key-value pair")
	};

	for h in handlers {
		let x = h.handle()?;
		kv.insert(h.key().to_string(), x);
	}
	let body = serde_json::to_string_pretty(&kv)?;
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.header(hyper::header::CONTENT_TYPE, "application/json")
			.body(body.into())
			.expect("builder with known status code should not fail"),
	)
}

// mirror envoy's behavior: https://www.envoyproxy.io/docs/envoy/latest/operations/admin#post--logging
// NOTE: multiple query parameters is not supported, for example
// curl -X POST http://127.0.0.1:15000/logging?"tap=debug&router=debug"
static HELP_STRING: &str = "
usage: POST /logging\t\t\t\t\t\t(To list current level)
usage: POST /logging?level=<level>\t\t\t\t(To change global levels)
usage: POST /logging?level={mod1}:{level1},{mod2}:{level2}\t(To change specific mods' logging level)

hint: loglevel:\terror|warn|info|debug|trace|off
hint: mod_name:\tthe module name, i.e. ztunnel::agentgateway
";
async fn handle_logging(req: Request<Incoming>) -> Response {
	match *req.method() {
		hyper::Method::POST => {
			let qp: HashMap<String, String> = req
				.uri()
				.query()
				.map(|v| {
					url::form_urlencoded::parse(v.as_bytes())
						.into_owned()
						.collect()
				})
				.unwrap_or_default();
			let level = qp.get("level").cloned();
			let reset = qp.get("reset").cloned();
			if level.is_some() || reset.is_some() {
				change_log_level(reset.is_some(), &level.unwrap_or_default())
			} else {
				list_loggers()
			}
		},
		_ => plaintext_response(
			hyper::StatusCode::METHOD_NOT_ALLOWED,
			format!("Invalid HTTP method\n {HELP_STRING}"),
		),
	}
}

fn list_loggers() -> Response {
	match telemetry::get_current_loglevel() {
		Ok(loglevel) => plaintext_response(
			hyper::StatusCode::OK,
			format!("current log level is {loglevel}\n"),
		),
		Err(err) => plaintext_response(
			hyper::StatusCode::INTERNAL_SERVER_ERROR,
			format!("failed to get the log level: {err}\n {HELP_STRING}"),
		),
	}
}

fn validate_log_level(level: &str) -> anyhow::Result<()> {
	for clause in level.split(',') {
		// We support 2 forms, compared to the underlying library
		// <level>: supported, sets the default
		// <scope>:<level>: supported, sets a scope's level
		// <scope>: sets the scope to 'trace' level. NOT SUPPORTED.
		match clause {
			"off" | "error" | "warn" | "info" | "debug" | "trace" => continue,
			s if s.contains('=') => {
				filter::Targets::from_str(s)?;
			},
			s => anyhow::bail!("level {s} is invalid"),
		}
	}
	Ok(())
}

fn change_log_level(reset: bool, level: &str) -> Response {
	if !reset && level.is_empty() {
		return list_loggers();
	}
	if !level.is_empty()
		&& let Err(_e) = validate_log_level(level)
	{
		// Invalid level provided
		return plaintext_response(
			hyper::StatusCode::BAD_REQUEST,
			format!("Invalid level provided: {level}\n{HELP_STRING}"),
		);
	};
	match telemetry::set_level(reset, level) {
		Ok(_) => list_loggers(),
		Err(e) => plaintext_response(
			hyper::StatusCode::BAD_REQUEST,
			format!("Failed to set new level: {e}\n{HELP_STRING}"),
		),
	}
}

#[cfg(all(feature = "jemalloc", target_os = "linux"))]
async fn handle_jemalloc_pprof_heapgen(_req: Request<Incoming>) -> anyhow::Result<Response> {
	let Some(prof_ctrl) = jemalloc_pprof::PROF_CTL.as_ref() else {
		return Ok(
			::http::Response::builder()
				.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
				.body("jemalloc profiling is not enabled".into())
				.expect("builder with known status code should not fail"),
		);
	};
	let mut prof_ctl = prof_ctrl.lock().await;
	if !prof_ctl.activated() {
		return Ok(
			::http::Response::builder()
				.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
				.body("jemalloc not enabled".into())
				.expect("builder with known status code should not fail"),
		);
	}
	let pprof = prof_ctl.dump_pprof()?;
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.body(bytes::Bytes::from(pprof).into())
			.expect("builder with known status code should not fail"),
	)
}

#[cfg(all(not(feature = "jemalloc"), target_os = "linux"))]
async fn handle_jemalloc_pprof_heapgen(_req: Request<Incoming>) -> anyhow::Result<Response> {
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
			.body("jemalloc not enabled".into())
			.expect("builder with known status code should not fail"),
	)
}
