// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agent_core::drain::DrainWatcher;
use agent_core::version::BuildInfo;
use agent_core::{signal, telemetry};
use axum::Router;
use axum::extract::State as AxumState;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use bytes::Bytes;
use futures_util::StreamExt;
use http::header::{AUTHORIZATION, CONTENT_LENGTH};
use http::{HeaderName, Method};
use hyper::header::{CONTENT_TYPE, HeaderValue};
use tokio::runtime::Handle;
use tokio::time;
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tracing_subscriber::filter;

use super::hyper_helpers::{Server, plaintext_response};
use crate::Config;
use crate::http::{Request, Response};

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

struct AdminError(anyhow::Error);

impl IntoResponse for AdminError {
	fn into_response(self) -> axum::response::Response {
		plaintext_response(hyper::StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
	}
}

impl<E> From<E> for AdminError
where
	E: Into<anyhow::Error>,
{
	fn from(error: E) -> Self {
		Self(error.into())
	}
}

#[derive(Clone)]
struct AdminState {
	stores: crate::store::Stores,
	config: Arc<Config>,
	#[cfg_attr(not(feature = "ui"), allow(dead_code))]
	model_catalog: Arc<crate::llm::cost::ModelCatalog>,
	shutdown_trigger: signal::ShutdownTrigger,
	config_dump_handlers: Vec<Arc<dyn ConfigDumpHandler>>,
	#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
	dataplane_handle: Handle,
}

pub struct Service {
	s: Server<AdminState>,
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
		model_catalog: Arc<crate::llm::cost::ModelCatalog>,
		stores: crate::store::Stores,
		shutdown_trigger: signal::ShutdownTrigger,
		drain_rx: DrainWatcher,
		dataplane_handle: Handle,
	) -> anyhow::Result<Self> {
		Server::<AdminState>::bind(
			"admin",
			config.admin_addr.clone(),
			drain_rx,
			AdminState {
				config,
				model_catalog,
				stores,
				shutdown_trigger,
				config_dump_handlers: vec![],
				dataplane_handle,
			},
		)
		.await
		.map(|s| Service { s })
	}

	pub fn address(&self) -> Option<SocketAddr> {
		self.s.address()
	}

	pub fn add_config_dump_handler(&mut self, handler: Arc<dyn ConfigDumpHandler>) {
		self.s.state_mut().config_dump_handlers.push(handler);
	}

	pub fn spawn(self) {
		let router = Arc::new(OnceLock::new());
		self.s.spawn(move |state, req| {
			let router = router.clone();
			async move {
				let router = router.get_or_init(|| admin_router(state)).clone();
				Ok(router.oneshot(req).await.unwrap())
			}
		})
	}
}

fn admin_router(state: Arc<AdminState>) -> Router {
	let router = Router::new();
	#[cfg(target_os = "linux")]
	let router = router.route("/debug/pprof/profile", get(handle_pprof));
	let router = router
		.route("/debug/pprof/heap", get(handle_heap_pprof))
		.route("/memory", get(handle_memory))
		.route("/quitquitquit", post(handle_server_shutdown))
		.route("/debug/tasks", get(handle_tokio_tasks))
		.route("/debug/trace", post(handle_debug_trace))
		.route("/config_dump", get(handle_config_dump))
		.route("/logging", post(handle_logging))
		.with_state(state.clone());

	#[cfg(feature = "ui")]
	let router = router.merge(crate::ui::router(
		state.config.clone(),
		state.model_catalog.clone(),
	));
	#[cfg(not(feature = "ui"))]
	let router = router.route("/", get(handle_dashboard));

	router.layer(add_cors_layer())
}

fn add_cors_layer() -> CorsLayer {
	CorsLayer::new()
		.allow_origin(
			[
				"http://0.0.0.0:3000",
				"http://localhost:3000",
				"http://127.0.0.1:3000",
				"http://0.0.0.0:19000",
				"http://127.0.0.1:19000",
				"http://localhost:19000",
			]
			.map(|origin| origin.parse::<HeaderValue>().unwrap()),
		)
		.allow_headers([
			CONTENT_TYPE,
			AUTHORIZATION,
			HeaderName::from_static("x-requested-with"),
		])
		.allow_methods([
			Method::GET,
			Method::POST,
			Method::PUT,
			Method::DELETE,
			Method::OPTIONS,
		])
		.allow_credentials(true)
		.expose_headers([CONTENT_TYPE, CONTENT_LENGTH])
		.max_age(Duration::from_secs(3600))
}

#[cfg(not(feature = "ui"))]
async fn handle_dashboard(_req: Request) -> Response {
	let apis = &[
		(
			"debug/pprof/profile",
			"build profile using the pprof profiler (if supported). Use ?seconds=N to specify duration (1-300s, default: 10s)",
		),
		(
			"debug/pprof/heap",
			"collect heap profiling data (if supported)",
		),
		("memory", "dump allocator and process memory statistics"),
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
async fn handle_pprof(req: Request) -> Result<Response, AdminError> {
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

async fn handle_server_shutdown(AxumState(state): AxumState<Arc<AdminState>>) -> Response {
	match time::timeout(
		state.config.termination_min_deadline,
		state.shutdown_trigger.clone().shutdown_now(),
	)
	.await
	{
		Ok(()) => info!("Shutdown completed gracefully"),
		Err(_) => warn!(
			"Graceful shutdown did not complete in {:?}, terminating now",
			state.config.termination_min_deadline
		),
	}
	plaintext_response(hyper::StatusCode::OK, "shutdown now\n".into())
}

pub async fn handle_debug_trace(req: Request) -> Response {
	let expression = req.uri().query().and_then(|query| {
		url::form_urlencoded::parse(query.as_bytes())
			.find(|(key, _)| key == "expression")
			.map(|(_, value)| value.into_owned())
	});
	let expression = match expression {
		Some(expression) => match crate::cel::Expression::new_strict(&expression) {
			Ok(expression) => Some(expression),
			Err(err) => {
				return plaintext_response(
					hyper::StatusCode::BAD_REQUEST,
					format!("invalid expression: {err}\n"),
				);
			},
		},
		None => None,
	};
	let rx = crate::proxy::dtrace::track_expression(expression);
	let sse_stream = trace_sse_stream(rx);
	::http::Response::builder()
		.status(hyper::StatusCode::OK)
		.header("Content-Type", "text/event-stream")
		.header("Cache-Control", "no-cache")
		.body(crate::http::Body::from_stream(sse_stream))
		.expect("builder with known status code should not fail")
}

fn trace_sse_stream(
	rx: crate::proxy::dtrace::TraceReceiver,
) -> impl futures_util::Stream<Item = Result<Bytes, Infallible>> {
	let keepalive = time::interval_at(
		time::Instant::now() + Duration::from_secs(1),
		Duration::from_secs(1),
	);
	let events =
		futures_util::stream::unfold((rx, keepalive), |(mut rx, mut keepalive)| async move {
			tokio::select! {
				msg = rx.next() => {
					let msg = msg?;
					let payload = serde_json::to_string(&msg).unwrap_or_else(|e| {
						serde_json::json!({
							"type": "serialization_error",
							"error": e.to_string(),
						})
						.to_string()
					});
					Some((Ok(Bytes::from(format!("data: {payload}\n\n"))), (rx, keepalive)))
				},
				_ = keepalive.tick() => {
					Some((Ok(Bytes::from_static(b": keepalive\n\n")), (rx, keepalive)))
				},
			}
		});
	futures_util::stream::once(async { Ok(Bytes::from_static(b": ready\n\n")) }).chain(events)
}

#[cfg(target_os = "linux")]
#[derive(serde::Serialize)]
struct TaskDump {
	admin: Vec<String>,
	workload: Vec<String>,
}

#[cfg(target_os = "linux")]
async fn handle_tokio_tasks(
	AxumState(state): AxumState<Arc<AdminState>>,
	_req: Request,
) -> Result<Response, AdminError> {
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

	if let Ok(dump) =
		tokio::time::timeout(Duration::from_secs(10), state.dataplane_handle.dump()).await
	{
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
	AxumState(_state): AxumState<Arc<AdminState>>,
	_req: Request,
) -> Result<Response, AdminError> {
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
			.body("task dump is not available".into())
			.expect("builder with known status code should not fail"),
	)
}

async fn handle_config_dump(
	AxumState(state): AxumState<Arc<AdminState>>,
) -> Result<Response, AdminError> {
	let dump = ConfigDump {
		stores: state.stores.clone(),
		version: BuildInfo::new(),
		config: state.config.clone(),
	};
	let serde_json::Value::Object(mut kv) = serde_json::to_value(&dump)? else {
		return Err(AdminError(anyhow::anyhow!(
			"config dump is not a key-value pair"
		)));
	};

	for h in &state.config_dump_handlers {
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
async fn handle_logging(req: Request) -> Response {
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

async fn handle_heap_pprof() -> Result<Response, AdminError> {
	let pprof = pprof_alloc::generate_pprof()?;
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.body(bytes::Bytes::from(pprof).into())
			.expect("builder with known status code should not fail"),
	)
}

async fn handle_memory() -> Result<Response, AdminError> {
	let snap = pprof_alloc::snapshot();
	let body = serde_json::to_string_pretty(&snap)?;
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.header(hyper::header::CONTENT_TYPE, "application/json")
			.body(body.into())
			.expect("builder with known status code should not fail"),
	)
}
