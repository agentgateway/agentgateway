use std::sync::Arc;
use std::time::Duration;

use agent_core::version::BuildInfo;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Redirect, Response, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use include_dir::{Dir, include_dir};
use serde::{Serialize, Serializer};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceExt;
use tower_serve_static::ServeDir;

use crate::cel::{self, ExecutorSerde};
use crate::llm::cost::ModelCatalog;
use crate::{Config, ConfigSource, client, yamlviajson};

const BASE_COSTS_FILE: &str = "base-costs.json";
const CONFIG_SCHEMA_HEADER: &str =
	"# yaml-language-server: $schema=https://agentgateway.dev/schema/config\n";

#[derive(Clone, Debug)]
struct App {
	state: Arc<Config>,
	client: client::Client,
	model_catalog: Arc<ModelCatalog>,
}

impl App {
	pub fn cfg(&self) -> Result<ConfigSource, ErrorResponse> {
		self
			.state
			.xds
			.local_config
			.clone()
			.ok_or(ErrorResponse::String("local config not setup".to_string()))
	}
}

lazy_static::lazy_static! {
	static ref ASSETS_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../ui/out");
}

pub fn router(cfg: Arc<Config>, model_catalog: Arc<ModelCatalog>) -> Router {
	let ui_service = tower::service_fn(move |req| serve_ui_asset(req, &ASSETS_DIR));
	Router::new()
		// Redirect to the UI
		.route("/api/runtime", get(get_runtime))
		.route("/api/config", get(get_config).post(write_config))
		// Legacy path
		.route("/cel", axum::routing::post(handle_cel))
		.route("/api/cel", axum::routing::post(handle_cel))
		.route("/api/logs/search", post(search_logs))
		.route("/api/logs/get", post(get_log))
		.route("/api/logs/tail", post(tail_logs))
		.route("/api/logs/analytics/summary", post(analytics_summary))
		.route("/api/costs/models", get(cost_models))
		.route("/api/costs/refresh-base", post(refresh_base_costs))
		.nest_service("/ui", ui_service)
		.route("/", get(|| async { Redirect::permanent("/ui") }))
		.with_state(App {
			state: cfg.clone(),
			client: client::Client::new(&cfg.dns, None, Default::default(), None),
			model_catalog,
		})
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInfo {
	build: RuntimeBuildInfo,
	ui: RuntimeUiInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeBuildInfo {
	version: &'static str,
	git_revision: &'static str,
	rust_version: &'static str,
	build_profile: &'static str,
	build_target: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeUiInfo {
	gateway_mode: GatewayRuntimeMode,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum GatewayRuntimeMode {
	Standalone,
	Xds,
}

async fn get_runtime(State(app): State<App>) -> Json<RuntimeInfo> {
	let build = BuildInfo::new();
	Json(RuntimeInfo {
		build: RuntimeBuildInfo {
			version: build.version,
			git_revision: build.git_revision,
			rust_version: build.rust_version,
			build_profile: build.build_profile,
			build_target: build.build_target,
		},
		ui: RuntimeUiInfo {
			gateway_mode: if app.state.xds.address.is_some() {
				GatewayRuntimeMode::Xds
			} else {
				GatewayRuntimeMode::Standalone
			},
		},
	})
}

async fn serve_ui_asset(
	req: http::Request<axum::body::Body>,
	assets: &'static Dir<'static>,
) -> Result<Response, std::convert::Infallible> {
	let req = if should_serve_ui_index(req.uri().path()) {
		request_with_path(req, "/index.html")
	} else {
		req
	};
	ServeDir::new(assets)
		.oneshot(req)
		.await
		.map(|response| response.map(axum::body::Body::new))
}

fn should_serve_ui_index(path: &str) -> bool {
	let path = path.trim_start_matches('/');
	path.is_empty() || (!path.starts_with("assets/") && !path.contains('.'))
}

fn request_with_path<B>(mut req: http::Request<B>, path: &str) -> http::Request<B> {
	let mut parts = req.uri().clone().into_parts();
	parts.path_and_query = Some(match req.uri().query() {
		Some(query) => format!("{path}?{query}").parse().expect("valid UI path"),
		None => path.parse().expect("valid UI path"),
	});
	*req.uri_mut() = Uri::from_parts(parts).expect("valid UI uri");
	req
}

#[derive(Debug, thiserror::Error)]
enum ErrorResponse {
	#[error("{0}")]
	String(String),
	#[error("{0}")]
	Anyhow(#[from] anyhow::Error),
}

impl Serialize for ErrorResponse {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.to_string().serialize(serializer)
	}
}

impl IntoResponse for ErrorResponse {
	fn into_response(self) -> Response {
		(StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
	}
}

async fn get_config(State(app): State<App>) -> Result<Json<Value>, ErrorResponse> {
	let s = app.cfg()?.read_to_string().await?;
	let v: Value = yamlviajson::from_str(&s).map_err(|e| ErrorResponse::Anyhow(e.into()))?;
	Ok(Json(v))
}

async fn write_config(
	State(app): State<App>,
	Json(config_json): Json<Value>,
) -> Result<Json<Value>, ErrorResponse> {
	let config_source = app.cfg()?;

	let file_path = match &config_source {
		ConfigSource::File(path) => path,
		ConfigSource::Static(_) => {
			return Err(ErrorResponse::String(
				"Cannot write to static config".to_string(),
			));
		},
	};
	let yaml_content =
		yamlviajson::to_string(&config_json).map_err(|e| ErrorResponse::Anyhow(e.into()))?;
	let yaml_file_content = format!("{CONFIG_SCHEMA_HEADER}{yaml_content}");

	if let Err(e) = crate::types::local::NormalizedLocalConfig::from(
		&app.state,
		app.client.clone(),
		app.state.gateway(),
		yaml_content.as_str(),
	)
	.await
	{
		return Err(ErrorResponse::String(e.to_string()));
	}

	// Write the YAML content to the file
	fs_err::tokio::write(file_path, yaml_file_content)
		.await
		.map_err(|e| ErrorResponse::Anyhow(e.into()))?;

	// Return success response
	Ok(Json(
		serde_json::json!({"status": "success", "message": "Configuration written successfully"}),
	))
}

async fn refresh_base_costs(State(app): State<App>) -> Result<Json<Value>, ErrorResponse> {
	let config_source = app.cfg()?;
	let file_path = match &config_source {
		ConfigSource::File(path) => path,
		ConfigSource::Static(_) => {
			return Err(ErrorResponse::String(
				"Cannot refresh base costs for static config".to_string(),
			));
		},
	};
	let dir = file_path.parent().ok_or_else(|| {
		ErrorResponse::String(format!(
			"config file has no parent: {}",
			file_path.display()
		))
	})?;
	let base_costs_file = dir.join(BASE_COSTS_FILE);

	let refreshed = crate::llm::cost::refresh::refresh_models_dev_base_catalog(
		&base_costs_file,
		app.model_catalog.as_ref(),
	)
	.await?;

	let mut response =
		serde_json::to_value(refreshed).map_err(|e| ErrorResponse::Anyhow(e.into()))?;
	if let Value::Object(fields) = &mut response {
		fields.insert(
			"file".to_string(),
			Value::String(BASE_COSTS_FILE.to_string()),
		);
	}
	Ok(Json(response))
}

async fn cost_models(
	State(app): State<App>,
) -> Result<Json<crate::llm::cost::ModelCatalogModels>, ErrorResponse> {
	Ok(Json(app.model_catalog.list_models()))
}

#[derive(serde::Deserialize)]
struct CelRequest {
	expression: String,
	#[serde(default)]
	data: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
struct CelResponse {
	result: Option<serde_json::Value>,
	error: Option<String>,
}

async fn handle_cel(Json(request): Json<CelRequest>) -> Response {
	// Compile the expression
	let expression = match cel::Expression::new_strict(&request.expression) {
		Ok(expr) => expr,
		Err(e) => {
			let resp = CelResponse {
				result: None,
				error: Some(format!("Failed to compile expression: {}", e)),
			};
			return (StatusCode::BAD_REQUEST, Json(resp)).into_response();
		},
	};

	// Deserialize the input data or use empty data if not provided
	let executor_serde: ExecutorSerde = match request.data {
		Some(data) => match serde_json::from_value(data) {
			Ok(serde) => serde,
			Err(e) => {
				let resp = CelResponse {
					result: None,
					error: Some(format!("Failed to parse input data: {}", e)),
				};
				return (StatusCode::BAD_REQUEST, Json(resp)).into_response();
			},
		},
		_ => ExecutorSerde::default(),
	};

	// Create the executor and evaluate the expression
	let executor = executor_serde.as_executor();
	let resp = match executor.eval(&expression) {
		Ok(value) => match value.json() {
			Ok(json) => CelResponse {
				result: Some(json),
				error: None,
			},
			Err(e) => CelResponse {
				result: None,
				error: Some(format!("Failed to convert result to JSON: {}", e)),
			},
		},
		Err(e) => CelResponse {
			result: None,
			error: Some(format!("Evaluation error: {}", e)),
		},
	};

	(StatusCode::OK, Json(resp)).into_response()
}

async fn search_logs(
	Json(request): Json<crate::telemetry::log_store::SearchRequest>,
) -> Result<Json<crate::telemetry::log_store::SearchResponse>, ErrorResponse> {
	crate::telemetry::log_store::search(request)
		.await
		.map(Json)
		.map_err(ErrorResponse::Anyhow)
}

async fn get_log(
	Json(request): Json<crate::telemetry::log_store::GetRequest>,
) -> Result<Json<crate::telemetry::log_store::GetResponse>, ErrorResponse> {
	crate::telemetry::log_store::get(request)
		.await
		.map(Json)
		.map_err(ErrorResponse::Anyhow)
}

async fn analytics_summary(
	Json(request): Json<crate::telemetry::log_store::AnalyticsSummaryRequest>,
) -> Result<Json<crate::telemetry::log_store::AnalyticsSummaryResponse>, ErrorResponse> {
	crate::telemetry::log_store::analytics_summary(request)
		.await
		.map(Json)
		.map_err(ErrorResponse::Anyhow)
}

async fn tail_logs(
	Json(mut request): Json<crate::telemetry::log_store::TailRequest>,
) -> Result<Sse<ReceiverStream<Result<Event, std::convert::Infallible>>>, ErrorResponse> {
	if !crate::telemetry::log_store::enabled() {
		return Err(ErrorResponse::String(
			"request log database is not configured".to_string(),
		));
	}
	let mut cursor = request
		.cursor
		.clone()
		.or_else(|| Some(crate::telemetry::log_store::encode_cursor(Utc::now(), "")));
	request.limit = Some(request.limit.unwrap_or(100).clamp(1, 500));

	let (tx, rx) = mpsc::channel(32);
	tokio::spawn(async move {
		let mut poll = tokio::time::interval(Duration::from_secs(1));
		let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
		loop {
			tokio::select! {
				_ = poll.tick() => {
					let mut batch_request = request.clone();
					batch_request.cursor = cursor.clone();
					match crate::telemetry::log_store::tail(batch_request).await {
						Ok(response) => {
							for log in response.logs {
								let next = crate::telemetry::log_store::encode_cursor(log.completed_at, &log.id);
								cursor = Some(next.clone());
								let event = crate::telemetry::log_store::TailEvent {
									entry: log,
									cursor: next,
								};
								let Ok(data) = serde_json::to_string(&event) else {
									continue;
								};
								if tx.send(Ok(Event::default().event("log").data(data))).await.is_err() {
									return;
								}
							}
							if let Some(next) = response.next_cursor {
								cursor = Some(next);
							}
						},
						Err(err) => {
							let event = Event::default()
								.event("error")
								.data(serde_json::json!({ "message": err.to_string() }).to_string());
							let _ = tx.send(Ok(event)).await;
							return;
						},
					}
				},
				_ = heartbeat.tick() => {
					if tx.send(Ok(Event::default().event("heartbeat").data("{}"))).await.is_err() {
						return;
					}
				},
			}
		}
	});

	Ok(Sse::new(ReceiverStream::new(rx)))
}
