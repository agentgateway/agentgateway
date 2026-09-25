use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use agent_core::version::BuildInfo;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Redirect, Response, Sse};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use chrono::Utc;
use include_dir::Dir;
use serde::{Serialize, Serializer};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceExt;
use tower_serve_static::ServeDir;
use tracing::debug;

use crate::cel::{self, ExecutorSerde};
use crate::config_store::{
	ConfigResource, ConfigResourceError, ConfigResourceKind, ConfigResourceStore,
	ConfigResourceUpsertRequest, ConfigResourcesResponse, PreparedResource,
};
use crate::llm::catalog::ModelCatalog;
use crate::{Config, ConfigSource, ConfigStoreMode, yaml};

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

	fn ensure_writable(&self) -> Result<(), ErrorResponse> {
		if self.state.storage.mode == ConfigStoreMode::ReadOnly {
			return Err(ErrorResponse::Status(
				StatusCode::FORBIDDEN,
				"UI is configured as read-only".to_string(),
			));
		}
		Ok(())
	}

	fn config_resource_store(&self) -> Result<ConfigResourceStore, ErrorResponse> {
		if self.state.storage.mode != ConfigStoreMode::Hybrid {
			return Err(ErrorResponse::Status(
				StatusCode::FORBIDDEN,
				"config resource APIs require config.storage.mode=hybrid".to_string(),
			));
		}
		self
			.config_resource_store
			.clone()
			.ok_or_else(|| ErrorResponse::String("config resource store was not initialized".to_string()))
	}
}

pub(crate) static EMPTY_ASSETS_DIR: Dir<'static> = Dir::new("", &[]);

pub fn router(
	cfg: Arc<Config>,
	model_catalog: Arc<ModelCatalog>,
	config_resource_store: Option<ConfigResourceStore>,
	resource_manager: crate::resource_manager::ResourceManager,
	assets_dir: &'static Dir<'static>,
) -> Router {
	let app = App {
		state: cfg.clone(),
		config_resource_store,
		resource_manager,
		model_catalog,
	};
	let ui_service = tower::service_fn(move |req| serve_ui_asset(req, assets_dir));
	Router::new()
		// OIDC intercepts this path to start login; without OIDC, return to the UI.
		.route("/api/auth/login", get(|| async { Redirect::to("/ui") }))
		.route("/api/runtime", get(get_runtime))
		.route("/api/config", get(get_config).post(write_config))
		.route("/api/config/effective", get(get_effective_config))
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
		.route("/api/budgets/status", get(budget_status))
		.nest_service("/ui", ui_service)
		.route("/", get(|| async { Redirect::permanent("/ui") }))
		.layer(axum::middleware::from_fn_with_state(
			app.clone(),
			authorize_route,
		))
		.with_state(app)
}

#[derive(Clone, Default)]
struct AuthorizationContext {
	user: Option<String>,
}

impl AuthorizationContext {
	/// Prepares ownership metadata before a management resource write is persisted.
	fn authorize_write(
		&self,
		prepared: &mut PreparedResource,
		_old_id: &str,
		old: Option<&Value>,
	) -> Result<(), ErrorResponse> {
		if prepared.kind == ConfigResourceKind::LlmApiKey {
			let owner = match old {
				Some(old) => old
					.pointer("/metadata/agentgateway.dev~1owner")
					.and_then(Value::as_str)
					.filter(|owner| !owner.is_empty()),
				None => self.user.as_deref(),
			};
			if let Some(owner) = owner {
				if !prepared.value["metadata"].is_object() {
					prepared.value["metadata"] = serde_json::json!({});
				}
				prepared.value["metadata"]["agentgateway.dev/owner"] = owner.into();
			}
		}
		Ok(())
	}
}

/// Attaches the management caller context using the live `standardAttributes.user`
/// CEL mapping and the request's authentication context.
async fn authorize_route(
	State(app): State<App>,
	request: axum::extract::Request,
	next: axum::middleware::Next,
) -> Result<Response, ErrorResponse> {
	let request = request.map(crate::http::Body::new);
	let attributes = app.state.logging.database_fields.load();
	let executor = cel::Executor::new_request(&request);
	let user = attributes
		.add
		.iter()
		.find(|(name, _)| name.as_ref() == "agentgateway.user")
		.and_then(|(_, expression)| executor.eval(expression).ok())
		.and_then(|value| match value {
			cel::Value::String(value) if !value.is_empty() => Some(value.to_string()),
			_ => None,
		});
	let (mut parts, body) = request.into_parts();
	parts.extensions.insert(AuthorizationContext { user });
	Ok(
		next
			.run(axum::extract::Request::from_parts(
				parts,
				axum::body::Body::new(body),
			))
			.await,
	)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInfo {
	user: Option<RuntimeUser>,
	build: RuntimeBuildInfo,
	ui: RuntimeUiInfo,
}

/// Display-only identity from standardAttributes.user, with optional JWT profile details.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeUser {
	pub subject: Option<String>,
	pub name: Option<String>,
	pub email: Option<String>,
	pub can_logout: bool,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UiConfigResource {
	kind: ConfigResourceKind,
	id: String,
	value: Value,
	#[serde(skip_serializing_if = "Option::is_none")]
	revision: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	created_at: Option<chrono::DateTime<Utc>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	updated_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UiConfigResourcesResponse {
	resources: Vec<UiConfigResource>,
	generation: Option<i64>,
}

impl From<ConfigResource> for UiConfigResource {
	fn from(resource: ConfigResource) -> Self {
		Self {
			kind: resource.kind,
			id: resource.id,
			value: resource.value,
			revision: Some(resource.revision),
			created_at: Some(resource.created_at),
			updated_at: Some(resource.updated_at),
		}
	}
}

impl From<PreparedResource> for UiConfigResource {
	fn from(resource: PreparedResource) -> Self {
		Self {
			kind: resource.kind,
			id: resource.id,
			value: resource.value,
			revision: None,
			created_at: None,
			updated_at: None,
		}
	}
}

impl From<ConfigResourcesResponse> for UiConfigResourcesResponse {
	fn from(response: ConfigResourcesResponse) -> Self {
		Self {
			resources: response
				.resources
				.into_iter()
				.map(UiConfigResource::from)
				.collect(),
			generation: response.generation,
		}
	}
}

async fn get_runtime(State(app): State<App>, req: axum::extract::Request) -> impl IntoResponse {
	let req = req.map(crate::http::Body::new);
	// Use the same compiled, reloadable mapping as request logs, even when log
	// storage is disabled. This is display metadata, not an authentication check.
	let attributes = app.state.logging.database_fields.load();
	let executor = cel::Executor::new_request(&req);
	let session = req
		.extensions()
		.get::<crate::http::oidc::AuthenticatedSession>();
	let claims = req.extensions().get::<crate::http::jwt::Claims>();
	let subject = attributes
		.add
		.iter()
		.find(|(name, _)| name.as_ref() == "agentgateway.user")
		.and_then(|(_, expression)| executor.eval(expression).ok())
		.and_then(|value| match value {
			cel::Value::String(value) => Some(value.trim().to_owned()),
			_ => None,
		})
		.filter(|value| !value.is_empty())
		.or_else(|| {
			// A custom display mapping must not hide the account menu and logout
			// for an authenticated OIDC session. Other identities retain that mapping.
			session?;
			claims?
				.inner
				.get("sub")?
				.as_str()
				.map(str::trim)
				.filter(|value| !value.is_empty())
				.map(str::to_owned)
		});
	let user = subject.map(|subject| {
		let [name, email, username] = ["name", "email", "preferred_username"].map(|key| {
			claims
				.and_then(|claims| claims.inner.get(key))
				.and_then(serde_json::Value::as_str)
				.map(str::trim)
				.filter(|v| !v.is_empty())
				.map(str::to_owned)
		});
		RuntimeUser {
			subject: Some(subject),
			name: name.or(username),
			email,
			can_logout: session.is_some_and(|session| session.can_logout),
		}
	});
	let build = BuildInfo::new();
	(
		[("cache-control", "no-store")],
		Json(RuntimeInfo {
			user,
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
				config_store_mode: app.state.storage.mode,
			},
		}),
	)
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
	#[error("{0}")]
	GenerationConflict(String),
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
			Self::GenerationConflict(_) => StatusCode::CONFLICT,
			Self::String(_) | Self::Anyhow(_) => StatusCode::INTERNAL_SERVER_ERROR,
		};
		(status, Json(self)).into_response()
	}
}

async fn get_config(State(app): State<App>) -> Result<Json<Value>, ErrorResponse> {
	Ok(Json(read_file_config(&app).await?))
}

async fn get_effective_config(State(app): State<App>) -> Result<Json<Value>, ErrorResponse> {
	let base = app.cfg()?.read_to_string().await?;
	let config = if app.state.storage.mode == ConfigStoreMode::Hybrid {
		let resources = app
			.config_resource_store()?
			.list(None)
			.await
			.map_err(resource_api_error)?;
		crate::config_store::materialize_config(&base, &resources).map_err(resource_api_error)?
	} else {
		base
	};
	let value = yaml::from_str(&config).map_err(ErrorResponse::Anyhow)?;
	Ok(Json(value))
}

async fn write_config(
	State(app): State<App>,
	Json(config_json): Json<Value>,
) -> Result<Json<Value>, ErrorResponse> {
	app.ensure_writable()?;
	persist_file_config(&app, &config_json).await?;
	Ok(Json(
		serde_json::json!({"status": "success", "message": "Configuration written successfully"}),
	))
}

async fn persist_file_config(app: &App, config_json: &Value) -> Result<(), ErrorResponse> {
	let config_source = app.cfg()?;

	let file_path = match &config_source {
		ConfigSource::File(path) => path,
		ConfigSource::Static(_) => {
			return Err(ErrorResponse::String(
				"Cannot write to static config".to_string(),
			));
		},
	};
	let current = fs_err::tokio::read_to_string(file_path)
		.await
		.map_err(anyhow::Error::from)?;
	let yaml_file_content = {
		let mut document =
			yaml_serde_edit::YamlObject::<Value>::parse(&current).map_err(anyhow::Error::from)?;
		document
			.set(config_json.clone())
			.map_err(anyhow::Error::from)?;
		let content = document.get_string();
		if content
			.lines()
			.any(|line| line.trim_start().starts_with("# yaml-language-server:"))
		{
			content.to_owned()
		} else {
			format!("{CONFIG_SCHEMA_HEADER}{content}")
		}
	};

	if let Err(e) = validate_config_in_task(app, yaml_file_content.clone()).await {
		return Err(ErrorResponse::String(e.to_string()));
	}

	// Write the YAML content to the file
	fs_err::tokio::write(file_path, yaml_file_content)
		.await
		.map_err(|e| ErrorResponse::Anyhow(e.into()))?;
	Ok(())
}

async fn list_config_resources(
	State(app): State<App>,
) -> Result<Json<UiConfigResourcesResponse>, ErrorResponse> {
	list_stored_config_resources(&app, None).await.map(Json)
}

async fn list_config_resources_by_kind(
	State(app): State<App>,
	Path(kind): Path<String>,
) -> Result<Json<UiConfigResourcesResponse>, ErrorResponse> {
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	list_stored_config_resources(&app, Some(kind))
		.await
		.map(Json)
}

async fn list_stored_config_resources(
	app: &App,
	kind: Option<ConfigResourceKind>,
) -> Result<UiConfigResourcesResponse, ErrorResponse> {
	if app.state.storage.mode != ConfigStoreMode::Hybrid {
		return Ok(UiConfigResourcesResponse {
			resources: Vec::new(),
			generation: None,
		});
	}
	let snapshot = app
		.config_resource_store()?
		.snapshot(kind)
		.await
		.map_err(resource_api_error)?;
	Ok(
		ConfigResourcesResponse {
			resources: snapshot.resources,
			generation: Some(snapshot.generation),
		}
		.into(),
	)
}

async fn read_file_config(app: &App) -> Result<Value, ErrorResponse> {
	let config = app.cfg()?.read_to_string().await?;
	yaml::from_str(&config).map_err(ErrorResponse::Anyhow)
}

async fn upsert_config_resources_by_kind(
	State(app): State<App>,
	Extension(auth): Extension<AuthorizationContext>,
	Path(kind): Path<String>,
	headers: HeaderMap,
	Json(request): Json<ConfigResourceUpsertRequest>,
) -> Result<Json<UiConfigResourcesResponse>, ErrorResponse> {
	app.ensure_writable()?;
	let expected = if_match_generation(&headers)?;
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	upsert_config_resources(&app, &auth, kind, request, expected)
		.await
		.map(Json)
}

async fn upsert_config_resources(
	app: &App,
	auth: &AuthorizationContext,
	kind: ConfigResourceKind,
	mut request: ConfigResourceUpsertRequest,
	expected: Option<i64>,
) -> Result<UiConfigResourcesResponse, ErrorResponse> {
	if app.state.storage.mode == ConfigStoreMode::Hybrid && kind.settings_fields().is_some() {
		let file_config = read_file_config(app).await?;
		for resource in &mut request.resources {
			remove_file_owned_settings(kind, &file_config, &mut resource.value)?;
		}
	}
	if app.state.storage.mode == ConfigStoreMode::File {
		let mut prepared =
			crate::config_store::prepare_resources(kind, request).map_err(resource_api_error)?;
		let mut config = read_file_config(app).await?;
		for resource in &mut prepared {
			let old = crate::config_store::file_config_resource(&config, kind, &resource.id);
			let id = resource.id.clone();
			auth.authorize_write(resource, &id, old)?;
			crate::config_store::upsert_file_config_resource(&mut config, resource, None)
				.map_err(resource_api_error)?;
		}
		persist_file_config(app, &config).await?;
		return Ok(UiConfigResourcesResponse {
			resources: prepared.into_iter().map(UiConfigResource::from).collect(),
			generation: None,
		});
	}

	let store = app.config_resource_store()?;
	with_generation_retry(expected, || async {
		let mut prepared =
			crate::config_store::prepare_resources(kind, request.clone()).map_err(resource_api_error)?;
		let snapshot = store.snapshot(None).await.map_err(resource_api_error)?;
		ensure_expected_generation(expected, snapshot.generation)?;
		for resource in &mut prepared {
			let old = snapshot
				.resources
				.iter()
				.find(|old| old.kind == kind && old.id == resource.id);
			let id = resource.id.clone();
			auth.authorize_write(resource, &id, old.map(|old| &old.value))?;
		}

		let candidate = crate::config_store::apply_prepared_upsert(snapshot.resources, &prepared)
			.map_err(resource_api_error)?;
		validate_materialized_config(app, &candidate).await?;
		let response = store
			.upsert_prepared(prepared, Some(snapshot.generation))
			.await
			.map_err(resource_api_error)?;
		Ok(response.into())
	})
	.await
}

fn ensure_expected_generation(expected: Option<i64>, found: i64) -> Result<(), ErrorResponse> {
	match expected {
		Some(expected) if expected != found => Err(ErrorResponse::GenerationConflict(format!(
			"config store changed since generation {expected} (now {found})"
		))),
		_ => Ok(()),
	}
}

async fn update_config_resource(
	State(app): State<App>,
	Extension(auth): Extension<AuthorizationContext>,
	Path((kind, id)): Path<(String, String)>,
	headers: HeaderMap,
	Json(mut resource): Json<crate::config_store::ConfigResourceUpsert>,
) -> Result<Json<UiConfigResourcesResponse>, ErrorResponse> {
	app.ensure_writable()?;
	let expected = if_match_generation(&headers)?;
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	if app.state.storage.mode == ConfigStoreMode::Hybrid && kind.settings_fields().is_some() {
		remove_file_owned_settings(kind, &read_file_config(&app).await?, &mut resource.value)?;
	}
	if app.state.storage.mode == ConfigStoreMode::File {
		return update_file_config_resource(&app, &auth, kind, &id, resource.value)
			.await
			.map(Json);
	}

	let store = app.config_resource_store()?;
	let is_policy = matches!(
		kind,
		ConfigResourceKind::LlmPolicy | ConfigResourceKind::McpPolicy | ConfigResourceKind::UiPolicy
	);
	with_generation_retry(expected, || async {
		let snapshot = store.snapshot(None).await.map_err(resource_api_error)?;
		ensure_expected_generation(expected, snapshot.generation)?;
		let mut prepared =
			prepare_stored_update(&snapshot.resources, kind, &id, resource.value.clone())?;
		let old = snapshot
			.resources
			.iter()
			.find(|old| old.kind == kind && old.id == id);
		auth.authorize_write(&mut prepared, &id, old.map(|old| &old.value))?;

		if old.is_none() && !is_policy {
			return Err(resource_api_error(ConfigResourceError::Conflict(format!(
				"file-owned config resource cannot be updated in hybrid mode: {kind}/{id}"
			))));
		}
		let renamed = prepared.id != id;
		let candidate = if renamed {
			crate::config_store::apply_delete(snapshot.resources, kind, &id)
		} else {
			snapshot.resources
		};
		let candidate =
			crate::config_store::apply_prepared_upsert(candidate, std::slice::from_ref(&prepared))
				.map_err(resource_api_error)?;
		validate_materialized_config(&app, &candidate).await?;
		let response = if renamed {
			store
				.rename_prepared(kind, &id, prepared, Some(snapshot.generation))
				.await
				.map_err(resource_api_error)?
		} else {
			store
				.upsert_prepared(vec![prepared], Some(snapshot.generation))
				.await
				.map_err(resource_api_error)?
		};
		Ok(response.into())
	})
	.await
	.map(Json)
}

async fn update_file_config_resource(
	app: &App,
	auth: &AuthorizationContext,
	kind: ConfigResourceKind,
	id: &str,
	value: Value,
) -> Result<UiConfigResourcesResponse, ErrorResponse> {
	let mut config = read_file_config(app).await?;
	let mut prepared = match kind {
		ConfigResourceKind::LlmApiKey => crate::config_store::prepare_file_api_key_update(
			id.to_string(),
			value,
			crate::config_store::file_api_key_created_at(&config, id),
		)
		.map_err(resource_api_error)?,
		ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::UiPolicy => {
			crate::config_store::prepare_policy_upsert(kind, id.to_string(), value)
				.map_err(resource_api_error)?
		},
		_ => crate::config_store::prepare_resource(kind, value).map_err(resource_api_error)?,
	};
	auth.authorize_write(
		&mut prepared,
		id,
		crate::config_store::file_config_resource(&config, kind, id),
	)?;
	crate::config_store::upsert_file_config_resource(&mut config, &prepared, Some(id))
		.map_err(resource_api_error)?;
	persist_file_config(app, &config).await?;
	Ok(UiConfigResourcesResponse {
		resources: vec![UiConfigResource::from(prepared)],
		generation: None,
	})
}

fn prepare_stored_update(
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
	id: &str,
	value: Value,
) -> Result<PreparedResource, ErrorResponse> {
	match kind {
		ConfigResourceKind::LlmApiKey => {
			let existing = resources
				.iter()
				.find(|resource| resource.kind == kind && resource.id == id)
				.ok_or_else(|| {
					resource_api_error(ConfigResourceError::NotFound(format!(
						"config resource not found: {kind}/{id}"
					)))
				})?;
			let created_at = crate::config_store::api_key_created_at(&existing.value)
				.or_else(|| Some(existing.created_at.timestamp()));
			crate::config_store::prepare_api_key_update(id.to_string(), value, created_at)
				.map_err(resource_api_error)
		},
		ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::UiPolicy => {
			crate::config_store::prepare_policy_upsert(kind, id.to_string(), value)
				.map_err(resource_api_error)
		},
		_ => crate::config_store::prepare_resource(kind, value).map_err(resource_api_error),
	}
}

fn remove_file_owned_settings(
	kind: ConfigResourceKind,
	file_config: &Value,
	value: &mut Value,
) -> Result<(), ErrorResponse> {
	let (section, fields) = kind.settings_fields().expect("settings resource");
	let value = value.as_object_mut().ok_or_else(|| {
		resource_api_error(ConfigResourceError::InvalidRequest(format!(
			"{kind}/default must be an object"
		)))
	})?;
	let file_settings = file_config.get(section).and_then(Value::as_object);
	for &field in fields {
		let Some(file_value) = file_settings.and_then(|settings| settings.get(field)) else {
			continue;
		};
		if value.get(field).is_some_and(|value| value != file_value) {
			return Err(resource_api_error(ConfigResourceError::Conflict(format!(
				"file-owned {kind}/default field cannot be updated in hybrid mode: {field}"
			))));
		}
		value.remove(field);
	}
	Ok(())
}

async fn delete_config_resource(
	State(app): State<App>,
	Path((kind, id)): Path<(String, String)>,
	headers: HeaderMap,
) -> Result<Json<Value>, ErrorResponse> {
	app.ensure_writable()?;
	let expected = if_match_generation(&headers)?;
	let kind = kind
		.parse::<ConfigResourceKind>()
		.map_err(resource_api_error)?;
	if app.state.storage.mode == ConfigStoreMode::File {
		let mut config = read_file_config(&app).await?;
		if !crate::config_store::delete_file_config_resource(&mut config, kind, &id)
			.map_err(resource_api_error)?
		{
			return Err(resource_api_error(ConfigResourceError::NotFound(format!(
				"config resource not found: {kind}/{id}"
			))));
		}
		persist_file_config(&app, &config).await?;
		return Ok(Json(
			serde_json::json!({"status": "success", "message": "Configuration resource deleted successfully"}),
		));
	}

	let store = app.config_resource_store()?;
	let generation = with_generation_retry(expected, || async {
		let snapshot = store.snapshot(None).await.map_err(resource_api_error)?;
		ensure_expected_generation(expected, snapshot.generation)?;
		if !snapshot
			.resources
			.iter()
			.any(|resource| resource.kind == kind && resource.id == id)
		{
			return Err(resource_api_error(ConfigResourceError::NotFound(format!(
				"config resource not found: {kind}/{id}"
			))));
		}
		let candidate = crate::config_store::apply_delete(snapshot.resources, kind, &id);
		validate_materialized_config(&app, &candidate).await?;
		store
			.delete(kind, &id, Some(snapshot.generation))
			.await
			.map_err(resource_api_error)
	})
	.await?;
	Ok(Json(serde_json::json!({
		"status": "success",
		"message": "Configuration resource deleted successfully",
		"generation": generation,
	})))
}

async fn validate_materialized_config(
	app: &App,
	resources: &[crate::config_store::ConfigResource],
) -> Result<(), ErrorResponse> {
	let base = app.cfg()?.read_to_string().await?;
	let config_content = crate::config_store::materialize_config(base.as_str(), resources)
		.map_err(resource_api_error)?;
	validate_config_in_task(app, config_content)
		.await
		.map_err(|err| ErrorResponse::Status(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()))?;
	Ok(())
}

async fn validate_config_in_task(app: &App, config: String) -> anyhow::Result<()> {
	let state = app.state.clone();
	let resources =
		crate::resource_manager::ResourceFetcher::cached_or_direct(app.resource_manager.clone());
	let gateway = state.gateway();

	// Boxing these futures keeps their state on the heap, but each nested `poll` still adds its
	// native stack frame to the gateway and HTTP frames already handling this request. In debug
	// builds the generated config-conversion frames are particularly large, so start a new task to
	// have Tokio poll the conversion from the scheduler root instead.
	tokio::spawn(async move {
		crate::types::local::NormalizedLocalConfig::from(&state, &resources, gateway, config.as_str())
			.await
			.map(|_| ())
	})
	.await??;
	Ok(())
}

fn resource_api_error(err: impl Into<anyhow::Error>) -> ErrorResponse {
	let err = err.into();
	let message = err.to_string();
	let status = match err.downcast_ref::<ConfigResourceError>() {
		Some(ConfigResourceError::InvalidRequest(_)) => StatusCode::BAD_REQUEST,
		Some(ConfigResourceError::Conflict(_)) => StatusCode::CONFLICT,
		Some(ConfigResourceError::NotFound(_)) => StatusCode::NOT_FOUND,
		Some(ConfigResourceError::GenerationConflict { .. }) => {
			return ErrorResponse::GenerationConflict(message);
		},
		None => StatusCode::INTERNAL_SERVER_ERROR,
	};
	ErrorResponse::Status(status, message)
}

const MAX_GENERATION_RETRIES: usize = 3;

fn if_match_generation(headers: &HeaderMap) -> Result<Option<i64>, ErrorResponse> {
	let Some(value) = headers.get(http::header::IF_MATCH) else {
		return Ok(None);
	};
	let value = value
		.to_str()
		.map_err(|_| ErrorResponse::Status(StatusCode::BAD_REQUEST, "invalid If-Match".to_string()))?
		.trim();
	// RFC 7232: `*` matches any current representation, so it pins nothing.
	if value == "*" {
		return Ok(None);
	}
	let value = value
		.strip_prefix("W/")
		.unwrap_or(value)
		.trim_start_matches('"')
		.trim_end_matches('"');
	value.parse::<i64>().map(Some).map_err(|_| {
		ErrorResponse::Status(
			StatusCode::BAD_REQUEST,
			format!("If-Match must be a config store generation, got {value:?}"),
		)
	})
}

async fn with_generation_retry<T, F, Fut>(
	expected: Option<i64>,
	mut attempt: F,
) -> Result<T, ErrorResponse>
where
	F: FnMut() -> Fut,
	Fut: Future<Output = Result<T, ErrorResponse>>,
{
	let attempts = if expected.is_some() {
		1
	} else {
		MAX_GENERATION_RETRIES
	};
	for remaining in (0..attempts).rev() {
		match attempt().await {
			Err(ErrorResponse::GenerationConflict(message)) if remaining > 0 => {
				debug!("{message}; retrying config write against a fresh snapshot");
			},
			other => return other,
		}
	}
	unreachable!("retry loop runs at least once")
}

async fn refresh_base_costs(
	State(app): State<App>,
	Extension(auth): Extension<AuthorizationContext>,
) -> Result<Json<Value>, ErrorResponse> {
	app.ensure_writable()?;
	let configured_file = app.state.model_catalog.sources.iter().find_map(|source| {
		if let crate::ModelCatalogSource::File { file } = source {
			Some(file)
		} else {
			None
		}
	});
	if configured_file.is_none() && app.state.storage.mode == ConfigStoreMode::Hybrid {
		let refreshed = crate::llm::catalog::refresh::fetch_base_catalog().await?;
		let store = app.config_resource_store()?;
		// The read stays inside the retry: this merges into the current value, so retrying with one
		// merged from a superseded read would restore the edit that displaced it.
		with_generation_retry(None, || async {
			let snapshot = store.snapshot(None).await.map_err(resource_api_error)?;
			let mut value = snapshot
				.resources
				.iter()
				.find(|resource| resource.kind == ConfigResourceKind::ModelCatalog)
				.map(|resource| resource.value.clone())
				.unwrap_or_else(|| serde_json::json!({}));
			let object = value.as_object_mut().ok_or_else(|| {
				resource_api_error(anyhow::anyhow!("modelCatalog resource must be an object"))
			})?;
			object.insert(
				"base".to_string(),
				serde_json::to_value(&refreshed.catalog)
					.map_err(|err| ErrorResponse::Anyhow(err.into()))?,
			);
			upsert_config_resources(
				&app,
				&auth,
				ConfigResourceKind::ModelCatalog,
				ConfigResourceUpsertRequest {
					resources: vec![crate::config_store::ConfigResourceUpsert { value }],
				},
				Some(snapshot.generation),
			)
			.await
			.map(|_| ())
		})
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

	let refreshed = crate::llm::catalog::refresh::refresh_base_catalog(
		&base_costs_file,
		configured_file.map(|_| app.model_catalog.as_ref()),
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
) -> Result<Json<crate::llm::catalog::ModelCatalogModels>, ErrorResponse> {
	Ok(Json(app.model_catalog.list_models()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BudgetStatusQuery {
	api_key_name: Option<String>,
}

async fn budget_status(
	State(app): State<App>,
	Query(query): Query<BudgetStatusQuery>,
) -> Result<Json<crate::http::budget::BudgetStatusResponse>, ErrorResponse> {
	Ok(Json(
		app
			.state
			.budget_policy
			.status(query.api_key_name.as_deref())?,
	))
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::client::{self, Client};

	fn if_match(value: &str) -> HeaderMap {
		let mut headers = HeaderMap::new();
		headers.insert(
			http::header::IF_MATCH,
			http::HeaderValue::from_str(value).expect("header value"),
		);
		headers
	}

	fn status_of(err: ErrorResponse) -> StatusCode {
		err.into_response().status()
	}

	#[test]
	fn parses_the_pinned_generation_from_if_match() {
		assert_eq!(
			if_match_generation(&HeaderMap::new()).expect("absent header"),
			None
		);
		assert_eq!(
			if_match_generation(&if_match("42")).expect("bare integer"),
			Some(42)
		);
		assert_eq!(
			if_match_generation(&if_match("\"42\"")).expect("quoted etag"),
			Some(42)
		);
		assert_eq!(
			if_match_generation(&if_match("W/\"42\"")).expect("weak etag"),
			Some(42)
		);
		assert_eq!(
			if_match_generation(&if_match("*")).expect("wildcard"),
			None,
			"`*` matches any current representation, so it pins nothing"
		);
	}

	#[test]
	fn rejects_an_if_match_that_is_not_a_generation() {
		let err = if_match_generation(&if_match("nonsense")).expect_err("not a generation");
		assert_eq!(status_of(err), StatusCode::BAD_REQUEST);
	}

	#[test]
	fn a_stale_pin_is_a_conflict_and_a_current_one_is_not() {
		assert!(ensure_expected_generation(None, 7).is_ok());
		assert!(ensure_expected_generation(Some(7), 7).is_ok());

		let err = ensure_expected_generation(Some(6), 7).expect_err("stale pin");
		assert!(
			matches!(err, ErrorResponse::GenerationConflict(_)),
			"a stale pin must be distinguishable from other conflicts"
		);
		assert_eq!(status_of(err), StatusCode::CONFLICT);
	}

	#[tokio::test]
	async fn an_unpinned_write_retries_a_generation_conflict() {
		let attempts = std::cell::Cell::new(0);
		let result = with_generation_retry(None, || async {
			attempts.set(attempts.get() + 1);
			if attempts.get() < 3 {
				return Err(ErrorResponse::GenerationConflict("stale".to_string()));
			}
			Ok(attempts.get())
		})
		.await;
		assert_eq!(result.expect("eventually succeeds"), 3);
		assert_eq!(attempts.get(), 3);
	}

	#[tokio::test]
	async fn a_pinned_write_surfaces_the_conflict_instead_of_retrying() {
		let attempts = std::cell::Cell::new(0);
		let result: Result<(), _> = with_generation_retry(Some(7), || async {
			attempts.set(attempts.get() + 1);
			Err(ErrorResponse::GenerationConflict("stale".to_string()))
		})
		.await;
		assert!(
			result.is_err(),
			"the client asked to be told about conflicts"
		);
		assert_eq!(attempts.get(), 1, "a pinned write must not be retried");
	}

	#[tokio::test]
	async fn other_conflicts_are_not_retried() {
		let attempts = std::cell::Cell::new(0);
		let result: Result<(), _> = with_generation_retry(None, || async {
			attempts.set(attempts.get() + 1);
			Err(ErrorResponse::Status(
				StatusCode::CONFLICT,
				"config resource already exists".to_string(),
			))
		})
		.await;
		assert!(result.is_err());
		assert_eq!(attempts.get(), 1);
	}

	fn test_app(read_only: bool) -> App {
		let mut config =
			crate::config::parse_config("{}".to_string(), None).expect("parse default config");
		if read_only {
			config.storage.mode = ConfigStoreMode::ReadOnly;
		}
		let client = Client::new(
			&client::Config {
				resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
				resolver_opts: hickory_resolver::config::ResolverOpts::default(),
			},
			None,
			crate::BackendConfig::default(),
			None,
		);
		App {
			state: Arc::new(config),
			config_resource_store: None,
			resource_manager: crate::resource_manager::ResourceManager::new(client)
				.expect("resource manager"),
			model_catalog: Arc::new(crate::llm::catalog::ModelCatalog::default()),
		}
	}

	#[tokio::test]
	async fn config_writes_preserve_yaml_comments_and_validate_before_writing() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("config.yaml");
		let mut app = test_app(false);
		Arc::get_mut(&mut app.state).unwrap().xds.local_config = Some(ConfigSource::File(path.clone()));

		for header in ["", "# yaml-language-server: $schema=./custom-schema.json\n"] {
			let original = format!(
				"{header}# My gateway\nbinds:\n- port: 8080\n  listeners: []\n\n# Keep this section\nconfig: {{}} # global settings\n"
			);
			fs_err::write(&path, &original).unwrap();
			let mut updated: Value = yaml::from_str(&original).unwrap();
			updated["binds"][0]["port"] = serde_json::json!(9090);
			let _ = write_config(State(app.clone()), Json(updated.clone()))
				.await
				.unwrap();
			let written = fs_err::read_to_string(&path).unwrap();
			let expected = original.replace("port: 8080", "port: 9090");
			let expected = if header.is_empty() {
				format!("{CONFIG_SCHEMA_HEADER}{expected}")
			} else {
				expected
			};
			assert_eq!(written, expected);
			assert_eq!(yaml::from_str::<Value>(&written).unwrap(), updated);

			let _ = write_config(State(app.clone()), Json(updated.clone()))
				.await
				.unwrap();
			assert_eq!(fs_err::read_to_string(&path).unwrap(), written);

			updated["binds"][0]["port"] = serde_json::json!("invalid");
			assert!(
				write_config(State(app.clone()), Json(updated))
					.await
					.is_err()
			);
			assert_eq!(fs_err::read_to_string(&path).unwrap(), written);
		}
	}

	#[tokio::test]
	async fn runtime_user_uses_standard_attributes_without_log_storage() {
		let app = test_app(false);
		assert!(app.state.logging.database.is_none());
		for (expression, jwt, expected_subject, can_logout) in [
			(None, true, Some("jwt-user"), false),
			(None, true, Some("jwt-user"), true),
			(None, false, Some("basic-user"), false),
			(
				Some("request.headers['x-display-user']"),
				true,
				Some("custom-user"),
				false,
			),
			(Some("null"), true, None, false),
			(Some("null"), true, Some("jwt-user"), true),
			(
				Some("request.headers['missing']"),
				true,
				Some("jwt-user"),
				true,
			),
			(Some("42"), true, None, false),
			(Some("request.headers['missing']"), true, None, false),
		] {
			// Reuse the app to verify updates replace the mapping used by the handler.
			app.state.logging.database_fields.store(Arc::new(
				crate::config::standard_attributes(Some(&crate::RawStandardAttributes {
					user: expression.map(str::to_owned),
					group: None,
				}))
				.unwrap(),
			));
			let mut req = axum::extract::Request::builder()
				.uri("http://localhost/api/runtime")
				.header("x-display-user", "custom-user")
				.body(axum::body::Body::empty())
				.unwrap();
			if jwt {
				req.extensions_mut().insert(crate::http::jwt::Claims {
					inner: serde_json::json!({"sub": "jwt-user", "name": "Display Name", "email": "user@example.com"}).as_object().unwrap().clone(),
					..Default::default()
				});
			} else {
				req.extensions_mut().insert(crate::http::basicauth::Claims {
					username: "basic-user".into(),
				});
			}
			if can_logout {
				req
					.extensions_mut()
					.insert(crate::http::oidc::AuthenticatedSession { can_logout });
			}
			let response = get_runtime(State(app.clone()), req).await.into_response();
			assert_eq!(response.headers()["cache-control"], "no-store");
			let body = axum::body::to_bytes(response.into_body(), usize::MAX)
				.await
				.unwrap();
			let runtime: Value = serde_json::from_slice(&body).unwrap();
			let expected = expected_subject
				.map(|subject| {
					serde_json::json!({
						"subject": subject,
						"canLogout": can_logout,
						"name": if jwt { Some("Display Name") } else { None },
						"email": if jwt { Some("user@example.com") } else { None },
					})
				})
				.unwrap_or(Value::Null);
			assert_eq!(runtime["user"], expected, "expression: {expression:?}");
		}
	}

	#[tokio::test]
	async fn ensure_writable_blocks_when_read_only() {
		let app = test_app(true);
		let response = app
			.ensure_writable()
			.expect_err("should be forbidden")
			.into_response();
		assert_eq!(response.status(), StatusCode::FORBIDDEN);
	}

	#[tokio::test]
	async fn ensure_writable_allows_when_not_read_only() {
		let app = test_app(false);
		assert!(app.ensure_writable().is_ok());
	}
}
