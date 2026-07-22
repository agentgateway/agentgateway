use std::sync::Arc;
use std::time::Duration;

use agent_core::version::BuildInfo;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Redirect, Response, Sse};
use axum::routing::{get, post, put};
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
use crate::config_store::{
	ConfigResourceError, ConfigResourceKind, ConfigResourceStore, ConfigResourceUpsertRequest,
	ConfigResourcesResponse,
};
use crate::llm::cost::ModelCatalog;
use crate::{Config, ConfigSource, ConfigStoreMode, yamlviajson};

const BASE_COSTS_FILE: &str = "base-costs.json";
const CONFIG_SCHEMA_HEADER: &str =
	"# yaml-language-server: $schema=https://agentgateway.dev/schema/config\n";

#[derive(Clone, Debug)]
struct App {
	state: Arc<Config>,
	config_resource_store: Option<ConfigResourceStore>,
	resource_manager: crate::resource_manager::ResourceManager,
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

	fn config_resource_store(&self) -> Result<ConfigResourceStore, ErrorResponse> {
		if self.state.config_store.mode != ConfigStoreMode::Hybrid {
			return Err(ErrorResponse::Status(
				StatusCode::FORBIDDEN,
				"config resource APIs require configStore.mode=hybrid".to_string(),
			));
		}
		self
			.config_resource_store
			.clone()
			.ok_or_else(|| ErrorResponse::String("config resource store was not initialized".to_string()))
	}
}

lazy_static::lazy_static! {
	static ref ASSETS_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../ui/out");
}

pub fn router(
	cfg: Arc<Config>,
	model_catalog: Arc<ModelCatalog>,
	config_resource_store: Option<ConfigResourceStore>,
	resource_manager: crate::resource_manager::ResourceManager,
) -> Router {
	let ui_service = tower::service_fn(move |req| serve_ui_asset(req, &ASSETS_DIR));
	Router::new()
		// Redirect to the UI
		.route("/api/runtime", get(get_runtime))
		.route("/api/config", get(get_config).post(write_config))
		.route("/api/config/resources", get(list_config_resources))
		.route(
			"/api/config/resources/{kind}",
			get(list_config_resources_by_kind).put(upsert_config_resources_by_kind),
		)
		.route(
			"/api/config/resources/{kind}/{id}",
			put(update_config_resource).delete(delete_config_resource),
		)
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
			config_resource_store,
			resource_manager,
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
	config_store_mode: ConfigStoreMode,
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
			config_store_mode: app.state.config_store.mode,
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
	#[error("{1}")]
	Status(StatusCode, String),
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
		let status = match &self {
			Self::Status(status, _) => *status,
			Self::String(_) | Self::Anyhow(_) => StatusCode::INTERNAL_SERVER_ERROR,
		};
		(status, Json(self)).into_response()
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

	let resources =
		crate::resource_manager::ResourceFetcher::cached_or_direct(app.resource_manager.clone());
	if let Err(e) = Box::pin(crate::types::local::NormalizedLocalConfig::from(
		&app.state,
		&resources,
		app.state.gateway(),
		yaml_content.as_str(),
	))
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

async fn list_config_resources(
	State(app): State<App>,
) -> Result<Json<ConfigResourcesResponse>, ErrorResponse> {
	let store = app.config_resource_store()?;
	let resources = store.list(None).await.map_err(resource_api_error)?;
	Ok(Json(ConfigResourcesResponse { resources }))
}

async fn list_config_resources_by_kind(
	State(app): State<App>,
	Path(kind): Path<String>,
) -> Result<Json<ConfigResourcesResponse>, ErrorResponse> {
	let store = app.config_resource_store()?;
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	let resources = store.list(Some(kind)).await.map_err(resource_api_error)?;
	Ok(Json(ConfigResourcesResponse { resources }))
}

async fn upsert_config_resources_by_kind(
	State(app): State<App>,
	Path(kind): Path<String>,
	Json(request): Json<ConfigResourceUpsertRequest>,
) -> Result<Json<ConfigResourcesResponse>, ErrorResponse> {
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	upsert_config_resources(&app, kind, request).await.map(Json)
}

async fn upsert_config_resources(
	app: &App,
	kind: ConfigResourceKind,
	request: ConfigResourceUpsertRequest,
) -> Result<ConfigResourcesResponse, ErrorResponse> {
	let store = app.config_resource_store()?;
	let prepared =
		crate::config_store::prepare_resources(kind, request).map_err(resource_api_error)?;
	let resources = store.list(None).await.map_err(resource_api_error)?;
	let candidate =
		crate::config_store::apply_prepared_upsert(resources, &prepared).map_err(resource_api_error)?;
	validate_materialized_config(app, &candidate).await?;
	let response = store
		.upsert_prepared(prepared)
		.await
		.map_err(resource_api_error)?;
	if kind == ConfigResourceKind::ModelCatalog {
		reload_model_catalog(app, &candidate).await?;
	}
	Ok(response)
}

async fn update_config_resource(
	State(app): State<App>,
	Path((kind, id)): Path<(String, String)>,
	Json(resource): Json<crate::config_store::ConfigResourceUpsert>,
) -> Result<Json<ConfigResourcesResponse>, ErrorResponse> {
	let store = app.config_resource_store()?;
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	if kind != ConfigResourceKind::LlmApiKey {
		return Err(resource_api_error(ConfigResourceError::InvalidRequest(
			"item updates are only supported for llm.apiKey resources".to_string(),
		)));
	}
	let resources = store.list(None).await.map_err(resource_api_error)?;
	if !resources
		.iter()
		.any(|resource| resource.kind == kind && resource.id == id)
	{
		return Err(resource_api_error(ConfigResourceError::NotFound(format!(
			"config resource not found: {kind}/{id}"
		))));
	}
	let prepared = vec![
		crate::config_store::prepare_api_key_update(id, resource.value).map_err(resource_api_error)?,
	];
	let candidate =
		crate::config_store::apply_prepared_upsert(resources, &prepared).map_err(resource_api_error)?;
	validate_materialized_config(&app, &candidate).await?;
	store
		.upsert_prepared(prepared)
		.await
		.map(Json)
		.map_err(resource_api_error)
}

async fn delete_config_resource(
	State(app): State<App>,
	Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Value>, ErrorResponse> {
	let store = app.config_resource_store()?;
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	let resources = store.list(None).await.map_err(resource_api_error)?;
	if !resources
		.iter()
		.any(|resource| resource.kind == kind && resource.id == id)
	{
		return Err(resource_api_error(ConfigResourceError::NotFound(format!(
			"config resource not found: {kind}/{id}"
		))));
	}
	let candidate = crate::config_store::apply_delete(resources, kind, &id);
	validate_materialized_config(&app, &candidate).await?;
	store.delete(kind, &id).await.map_err(resource_api_error)?;
	if kind == ConfigResourceKind::ModelCatalog {
		reload_model_catalog(&app, &candidate).await?;
	}
	Ok(Json(
		serde_json::json!({"status": "success", "message": "Configuration resource deleted successfully"}),
	))
}

async fn reload_model_catalog(
	app: &App,
	resources: &[crate::config_store::ConfigResource],
) -> Result<(), ErrorResponse> {
	let mut sources =
		crate::config_store::model_catalog_sources(resources).map_err(resource_api_error)?;
	sources.extend(app.state.model_catalog.sources.clone());
	app
		.model_catalog
		.replace_sources(sources)
		.await
		.map_err(resource_api_error)
}

async fn validate_materialized_config(
	app: &App,
	resources: &[crate::config_store::ConfigResource],
) -> Result<(), ErrorResponse> {
	let base = app.cfg()?.read_to_string().await?;
	let config_content = crate::config_store::materialize_config(base.as_str(), resources)
		.map_err(resource_api_error)?;
	let resources =
		crate::resource_manager::ResourceFetcher::cached_or_direct(app.resource_manager.clone());
	crate::types::local::NormalizedLocalConfig::from(
		&app.state,
		&resources,
		app.state.gateway(),
		config_content.as_str(),
	)
	.await
	.map_err(|err| ErrorResponse::Status(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()))?;
	Ok(())
}

fn resource_api_error(err: impl Into<anyhow::Error>) -> ErrorResponse {
	let err = err.into();
	let message = err.to_string();
	let status = match err.downcast_ref::<ConfigResourceError>() {
		Some(ConfigResourceError::InvalidRequest(_)) => StatusCode::BAD_REQUEST,
		Some(ConfigResourceError::Conflict(_)) => StatusCode::CONFLICT,
		Some(ConfigResourceError::NotFound(_)) => StatusCode::NOT_FOUND,
		None => StatusCode::INTERNAL_SERVER_ERROR,
	};
	ErrorResponse::Status(status, message)
}

async fn refresh_base_costs(State(app): State<App>) -> Result<Json<Value>, ErrorResponse> {
	let configured_file = app.state.model_catalog.sources.iter().find_map(|source| {
		if let crate::ModelCatalogSource::File { file } = source {
			Some(file)
		} else {
			None
		}
	});
	if configured_file.is_none() && app.state.config_store.mode == ConfigStoreMode::Hybrid {
		let refreshed = crate::llm::cost::refresh::fetch_models_dev_base_catalog().await?;
		let resources = app
			.config_resource_store()?
			.list(None)
			.await
			.map_err(resource_api_error)?;
		let mut value = resources
			.iter()
			.find(|resource| resource.kind == ConfigResourceKind::ModelCatalog)
			.map(|resource| resource.value.clone())
			.unwrap_or_else(|| serde_json::json!({}));
		let object = value.as_object_mut().ok_or_else(|| {
			resource_api_error(anyhow::anyhow!("modelCatalog resource must be an object"))
		})?;
		object.insert(
			"base".to_string(),
			serde_json::to_value(&refreshed.catalog).map_err(|err| ErrorResponse::Anyhow(err.into()))?,
		);
		upsert_config_resources(
			&app,
			ConfigResourceKind::ModelCatalog,
			ConfigResourceUpsertRequest {
				resources: vec![crate::config_store::ConfigResourceUpsert { value }],
			},
		)
		.await?;
		return serde_json::to_value(refreshed)
			.map(Json)
			.map_err(|err| ErrorResponse::Anyhow(err.into()));
	}
	let base_costs_file = if let Some(file) = configured_file {
		file.clone()
	} else {
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
		dir.join(BASE_COSTS_FILE)
	};

	let refreshed = crate::llm::cost::refresh::refresh_models_dev_base_catalog(
		&base_costs_file,
		app.model_catalog.as_ref(),
	)
	.await?;

	let mut response =
		serde_json::to_value(refreshed).map_err(|e| ErrorResponse::Anyhow(e.into()))?;
	if configured_file.is_none()
		&& let Value::Object(fields) = &mut response
	{
		fields.insert(
			"file".to_string(),
			Value::String(base_costs_file.to_string_lossy().to_string()),
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
