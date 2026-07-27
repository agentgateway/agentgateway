use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::PgListener;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Row, Sqlite, SqlitePool, Transaction};
use tokio::sync::watch;
use tracing::{error, warn};

use crate::database::DatabasePool;
use crate::telemetry::log_store;

#[derive(Clone, Debug)]
pub struct ConfigResourceStore {
	pool: DatabasePool,
	change_tx: watch::Sender<()>,
	notification_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResource {
	pub kind: ConfigResourceKind,
	pub id: String,
	pub value: Value,
	pub revision: i64,
	pub created_at: DateTime<Utc>,
	pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResourcesResponse {
	pub resources: Vec<ConfigResource>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigResourceError {
	#[error("{0}")]
	InvalidRequest(String),
	#[error("{0}")]
	Conflict(String),
	#[error("{0}")]
	NotFound(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ConfigResourceKind {
	#[serde(rename = "modelCatalog")]
	ModelCatalog,
	#[serde(rename = "llm.provider")]
	LlmProvider,
	#[serde(rename = "llm.model")]
	LlmModel,
	#[serde(rename = "llm.virtualModel")]
	LlmVirtualModel,
	#[serde(rename = "llm.apiKey")]
	LlmApiKey,
	#[serde(rename = "llm.policy")]
	LlmPolicy,
	#[serde(rename = "mcp.target")]
	McpTarget,
	#[serde(rename = "mcp.policy")]
	McpPolicy,
	#[serde(rename = "mcp.settings")]
	McpSettings,
	#[serde(rename = "traffic.gateway")]
	TrafficGateway,
	#[serde(rename = "traffic.route")]
	TrafficRoute,
	#[serde(rename = "traffic.tcpRoute")]
	TrafficTcpRoute,
	#[serde(rename = "ui.policy")]
	UiPolicy,
}

impl ConfigResourceKind {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::ModelCatalog => "modelCatalog",
			Self::LlmProvider => "llm.provider",
			Self::LlmModel => "llm.model",
			Self::LlmVirtualModel => "llm.virtualModel",
			Self::LlmApiKey => "llm.apiKey",
			Self::LlmPolicy => "llm.policy",
			Self::McpTarget => "mcp.target",
			Self::McpPolicy => "mcp.policy",
			Self::McpSettings => "mcp.settings",
			Self::TrafficGateway => "traffic.gateway",
			Self::TrafficRoute => "traffic.route",
			Self::TrafficTcpRoute => "traffic.tcpRoute",
			Self::UiPolicy => "ui.policy",
		}
	}
}

impl fmt::Display for ConfigResourceKind {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl FromStr for ConfigResourceKind {
	type Err = ConfigResourceError;

	fn from_str(kind: &str) -> Result<Self, Self::Err> {
		match kind {
			"modelCatalog" => Ok(Self::ModelCatalog),
			"llm.provider" => Ok(Self::LlmProvider),
			"llm.model" => Ok(Self::LlmModel),
			"llm.virtualModel" => Ok(Self::LlmVirtualModel),
			"llm.apiKey" => Ok(Self::LlmApiKey),
			"llm.policy" => Ok(Self::LlmPolicy),
			"mcp.target" => Ok(Self::McpTarget),
			"mcp.policy" => Ok(Self::McpPolicy),
			"mcp.settings" => Ok(Self::McpSettings),
			"traffic.gateway" => Ok(Self::TrafficGateway),
			"traffic.route" => Ok(Self::TrafficRoute),
			"traffic.tcpRoute" => Ok(Self::TrafficTcpRoute),
			"ui.policy" => Ok(Self::UiPolicy),
			_ => Err(ConfigResourceError::InvalidRequest(format!(
				"unsupported config resource kind: {kind}"
			))),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResourceUpsertRequest {
	#[serde(default)]
	pub resources: Vec<ConfigResourceUpsert>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResourceUpsert {
	pub value: Value,
}

pub async fn setup(cfg: &log_store::Config) -> anyhow::Result<ConfigResourceStore> {
	ConfigResourceStore::connect(&cfg.url).await
}

impl ConfigResourceStore {
	async fn connect(url: &str) -> anyhow::Result<Self> {
		Self::from_pool(DatabasePool::connect(url).await?).await
	}

	async fn from_pool(pool: DatabasePool) -> anyhow::Result<Self> {
		let (change_tx, _) = watch::channel(());
		match &pool {
			DatabasePool::Sqlite(pool) => {
				sqlx::raw_sql(SQLITE_SCHEMA).execute(pool).await?;
				Ok(Self {
					pool: DatabasePool::Sqlite(pool.clone()),
					change_tx,
					notification_id: None,
				})
			},
			DatabasePool::Postgres(pool) => {
				sqlx::raw_sql(POSTGRES_SCHEMA).execute(pool).await?;
				let notification_id = uuid::Uuid::new_v4().to_string();
				let mut listener = PgListener::connect_with(pool).await?;
				listener.listen(POSTGRES_CHANGE_CHANNEL).await?;
				let listener_change_tx = change_tx.clone();
				let listener_notification_id = notification_id.clone();
				tokio::spawn(async move {
					loop {
						match listener.try_recv().await {
							Ok(Some(notification)) => {
								if notification.payload() != listener_notification_id {
									let _ = listener_change_tx.send(());
								}
							},
							Ok(None) => {
								warn!("postgres config change listener reconnected; reloading config");
								let _ = listener_change_tx.send(());
							},
							Err(err) => {
								error!(?err, "postgres config change listener failed");
								tokio::time::sleep(std::time::Duration::from_secs(1)).await;
							},
						}
					}
				});
				Ok(Self {
					pool: DatabasePool::Postgres(pool.clone()),
					change_tx,
					notification_id: Some(notification_id),
				})
			},
		}
	}

	pub fn pool(&self) -> DatabasePool {
		self.pool.clone()
	}

	pub fn subscribe_changes(&self) -> watch::Receiver<()> {
		self.change_tx.subscribe()
	}

	pub async fn list(
		&self,
		kind: Option<ConfigResourceKind>,
	) -> anyhow::Result<Vec<ConfigResource>> {
		match &self.pool {
			DatabasePool::Sqlite(pool) => list_sqlite(pool, kind).await,
			DatabasePool::Postgres(pool) => list_postgres(pool, kind).await,
		}
	}

	pub(crate) async fn upsert_prepared(
		&self,
		prepared: Vec<PreparedResource>,
	) -> anyhow::Result<ConfigResourcesResponse> {
		let resources = match &self.pool {
			DatabasePool::Sqlite(pool) => upsert_sqlite(pool, prepared).await?,
			DatabasePool::Postgres(pool) => {
				upsert_postgres(
					pool,
					prepared,
					self
						.notification_id
						.as_deref()
						.expect("postgres store has a notification ID"),
				)
				.await?
			},
		};
		if !resources.is_empty() {
			self.notify_changed();
		}
		Ok(ConfigResourcesResponse { resources })
	}

	pub(crate) async fn rename_prepared(
		&self,
		previous_kind: ConfigResourceKind,
		previous_id: &str,
		prepared: PreparedResource,
	) -> anyhow::Result<ConfigResourcesResponse> {
		validate_id(previous_id)?;
		validate_id(&prepared.id)?;
		let resource = match &self.pool {
			DatabasePool::Sqlite(pool) => {
				rename_sqlite(pool, previous_kind, previous_id, prepared).await?
			},
			DatabasePool::Postgres(pool) => {
				rename_postgres(
					pool,
					previous_kind,
					previous_id,
					prepared,
					self
						.notification_id
						.as_deref()
						.expect("postgres store has a notification ID"),
				)
				.await?
			},
		};
		self.notify_changed();
		Ok(ConfigResourcesResponse {
			resources: vec![resource],
		})
	}

	pub async fn delete(&self, kind: ConfigResourceKind, id: &str) -> anyhow::Result<()> {
		validate_id(id)?;
		let deleted = match &self.pool {
			DatabasePool::Sqlite(pool) => delete_sqlite(pool, kind, id).await?,
			DatabasePool::Postgres(pool) => {
				delete_postgres(
					pool,
					kind,
					id,
					self
						.notification_id
						.as_deref()
						.expect("postgres store has a notification ID"),
				)
				.await?
			},
		};
		if !deleted {
			return Err(
				ConfigResourceError::NotFound(format!("config resource not found: {kind}/{id}")).into(),
			);
		}
		self.notify_changed();
		Ok(())
	}

	fn notify_changed(&self) {
		let _ = self.change_tx.send(());
	}
}

fn validate_id(id: &str) -> anyhow::Result<()> {
	if id.is_empty() {
		return Err(
			ConfigResourceError::InvalidRequest("config resource id cannot be empty".to_string()).into(),
		);
	}
	Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedResource {
	pub kind: ConfigResourceKind,
	pub id: String,
	pub value: Value,
}

pub(crate) const MCP_SETTINGS_FIELDS: [&str; 5] = [
	"gateways",
	"port",
	"statefulMode",
	"prefixMode",
	"failureMode",
];

/// Older file keys have no stored ID, so expose their array position to the resource API.
fn file_api_key_id(value: &Value, index: usize) -> String {
	value
		.pointer("/metadata/id")
		.and_then(Value::as_str)
		.filter(|id| !id.is_empty())
		.map(ToString::to_string)
		.unwrap_or_else(|| format!("@index:{index}"))
}

/// Direct mappings from a resource kind to its native YAML collection.
enum FileResourceCollection {
	List(&'static [&'static str]),
	Map(&'static [&'static str]),
}

fn file_resource_collection(kind: ConfigResourceKind) -> Option<FileResourceCollection> {
	match kind {
		ConfigResourceKind::LlmProvider => Some(FileResourceCollection::List(&["llm", "providers"])),
		ConfigResourceKind::LlmModel => Some(FileResourceCollection::List(&["llm", "models"])),
		ConfigResourceKind::LlmVirtualModel => {
			Some(FileResourceCollection::List(&["llm", "virtualModels"]))
		},
		ConfigResourceKind::McpTarget => Some(FileResourceCollection::List(&["mcp", "targets"])),
		ConfigResourceKind::LlmPolicy => Some(FileResourceCollection::Map(&["llm", "policies"])),
		ConfigResourceKind::McpPolicy => Some(FileResourceCollection::Map(&["mcp", "policies"])),
		ConfigResourceKind::UiPolicy => Some(FileResourceCollection::Map(&["ui", "policies"])),
		ConfigResourceKind::TrafficGateway => Some(FileResourceCollection::Map(&["gateways"])),
		ConfigResourceKind::TrafficRoute => Some(FileResourceCollection::List(&["routes"])),
		ConfigResourceKind::TrafficTcpRoute => Some(FileResourceCollection::List(&["tcpRoutes"])),
		ConfigResourceKind::ModelCatalog
		| ConfigResourceKind::LlmApiKey
		| ConfigResourceKind::McpSettings => None,
	}
}

pub(crate) fn upsert_file_config_resource(
	config: &mut Value,
	prepared: &PreparedResource,
	previous_id: Option<&str>,
) -> anyhow::Result<()> {
	if let Some(collection) = file_resource_collection(prepared.kind) {
		return match collection {
			FileResourceCollection::List(path) => {
				upsert_file_list_resource(config, path, prepared, previous_id)
			},
			FileResourceCollection::Map(path) => {
				upsert_file_map_resource(config, path, prepared, previous_id)
			},
		};
	}
	match prepared.kind {
		ConfigResourceKind::ModelCatalog => upsert_file_model_catalog(config, &prepared.value),
		ConfigResourceKind::LlmApiKey => upsert_file_api_key(config, prepared, previous_id),
		ConfigResourceKind::McpSettings => upsert_file_mcp_settings(config, &prepared.value),
		ConfigResourceKind::LlmProvider
		| ConfigResourceKind::LlmModel
		| ConfigResourceKind::LlmVirtualModel
		| ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpTarget
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::TrafficGateway
		| ConfigResourceKind::TrafficRoute
		| ConfigResourceKind::TrafficTcpRoute
		| ConfigResourceKind::UiPolicy => unreachable!("direct file resources handled above"),
	}
}

pub(crate) fn delete_file_config_resource(
	config: &mut Value,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<bool> {
	if let Some(collection) = file_resource_collection(kind) {
		return match collection {
			FileResourceCollection::List(path) => delete_file_list_resource(config, path, kind, id),
			FileResourceCollection::Map(path) => delete_file_map_resource(config, path, id),
		};
	}
	match kind {
		ConfigResourceKind::ModelCatalog => delete_file_model_catalog(config),
		ConfigResourceKind::LlmApiKey => delete_file_api_key(config, id),
		ConfigResourceKind::McpSettings => delete_file_mcp_settings(config),
		ConfigResourceKind::LlmProvider
		| ConfigResourceKind::LlmModel
		| ConfigResourceKind::LlmVirtualModel
		| ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpTarget
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::TrafficGateway
		| ConfigResourceKind::TrafficRoute
		| ConfigResourceKind::TrafficTcpRoute
		| ConfigResourceKind::UiPolicy => unreachable!("direct file resources handled above"),
	}
}

/// Creates missing parent objects and the array at `path`, rejecting incompatible shapes.
fn ensure_file_array<'a>(
	config: &'a mut Value,
	path: &[&str],
) -> anyhow::Result<&'a mut Vec<Value>> {
	let (field, parents) = path
		.split_last()
		.ok_or_else(|| anyhow::anyhow!("file resource path cannot be empty"))?;
	let mut current = config;
	for parent in parents {
		let object = current
			.as_object_mut()
			.ok_or_else(|| anyhow::anyhow!("local config {} must be a JSON object", parents.join(".")))?;
		current = object
			.entry((*parent).to_string())
			.or_insert_with(|| Value::Object(serde_json::Map::new()));
	}
	current
		.as_object_mut()
		.ok_or_else(|| anyhow::anyhow!("local config {} must be a JSON object", parents.join(".")))?
		.entry((*field).to_string())
		.or_insert_with(|| Value::Array(Vec::new()))
		.as_array_mut()
		.ok_or_else(|| anyhow::anyhow!("local config {} must be an array", path.join(".")))
}

/// Creates missing objects along `path`, rejecting any existing non-object value.
fn ensure_file_object<'a>(
	config: &'a mut Value,
	path: &[&str],
) -> anyhow::Result<&'a mut serde_json::Map<String, Value>> {
	let mut current = config;
	for field in path {
		let object = current
			.as_object_mut()
			.ok_or_else(|| anyhow::anyhow!("local config {} must be a JSON object", path.join(".")))?;
		current = object
			.entry((*field).to_string())
			.or_insert_with(|| Value::Object(serde_json::Map::new()));
	}
	current
		.as_object_mut()
		.ok_or_else(|| anyhow::anyhow!("local config {} must be a JSON object", path.join(".")))
}

fn upsert_file_list_resource(
	config: &mut Value,
	path: &[&str],
	prepared: &PreparedResource,
	previous_id: Option<&str>,
) -> anyhow::Result<()> {
	let values = ensure_file_array(config, path)?;
	let existing = values.iter().position(|value| {
		resource_id(prepared.kind, value).is_ok_and(|id| id == previous_id.unwrap_or(&prepared.id))
	});
	if let Some(previous_id) = previous_id {
		let Some(existing) = existing else {
			return Err(
				ConfigResourceError::NotFound(format!(
					"config resource not found: {}/{}",
					prepared.kind, previous_id
				))
				.into(),
			);
		};
		if previous_id != prepared.id
			&& values.iter().enumerate().any(|(index, value)| {
				index != existing && resource_id(prepared.kind, value).is_ok_and(|id| id == prepared.id)
			}) {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource already exists: {}/{}",
					prepared.kind, prepared.id
				))
				.into(),
			);
		}
		values[existing] = prepared.value.clone();
	} else if let Some(existing) = existing {
		values[existing] = prepared.value.clone();
	} else {
		values.push(prepared.value.clone());
	}
	Ok(())
}

fn delete_file_list_resource(
	config: &mut Value,
	path: &[&str],
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<bool> {
	let Some(values) = crate::json::traverse_mut(config, path).and_then(Value::as_array_mut) else {
		return Ok(false);
	};
	let before = values.len();
	values.retain(|value| resource_id(kind, value).map_or(true, |resource_id| resource_id != id));
	Ok(values.len() != before)
}

fn upsert_file_map_resource(
	config: &mut Value,
	path: &[&str],
	prepared: &PreparedResource,
	previous_id: Option<&str>,
) -> anyhow::Result<()> {
	let values = ensure_file_object(config, path)?;
	if let Some(previous_id) = previous_id {
		let policy_upsert = previous_id == prepared.id
			&& matches!(
				prepared.kind,
				ConfigResourceKind::LlmPolicy
					| ConfigResourceKind::McpPolicy
					| ConfigResourceKind::UiPolicy
			);
		if !values.contains_key(previous_id) && !policy_upsert {
			return Err(
				ConfigResourceError::NotFound(format!(
					"config resource not found: {}/{}",
					prepared.kind, previous_id
				))
				.into(),
			);
		}
		if previous_id != prepared.id && values.contains_key(&prepared.id) {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource already exists: {}/{}",
					prepared.kind, prepared.id
				))
				.into(),
			);
		}
		if previous_id != prepared.id {
			values.remove(previous_id);
		}
	}
	let mut value = prepared.value.clone();
	// API keys are managed as separate resources and must survive policy updates.
	if prepared.kind == ConfigResourceKind::LlmPolicy && prepared.id == "apiKey" {
		let existing_keys = values
			.get("apiKey")
			.and_then(|policy| policy.get("keys"))
			.cloned();
		if let Some(keys) = existing_keys {
			value
				.as_object_mut()
				.ok_or_else(|| anyhow::anyhow!("llm.policy/apiKey must be an object"))?
				.insert("keys".to_string(), keys);
		}
	}
	// A gateway's resource ID is the key in the YAML map, not a field in its value.
	if prepared.kind == ConfigResourceKind::TrafficGateway {
		value
			.as_object_mut()
			.ok_or_else(|| anyhow::anyhow!("traffic.gateway/{} must be an object", prepared.id))?
			.remove("name");
	}
	values.insert(prepared.id.clone(), value);
	Ok(())
}

fn delete_file_map_resource(config: &mut Value, path: &[&str], id: &str) -> anyhow::Result<bool> {
	Ok(
		crate::json::traverse_mut(config, path)
			.and_then(Value::as_object_mut)
			.and_then(|values| values.remove(id))
			.is_some(),
	)
}

fn upsert_file_api_key(
	config: &mut Value,
	prepared: &PreparedResource,
	previous_id: Option<&str>,
) -> anyhow::Result<()> {
	let keys = ensure_file_array(config, &["llm", "policies", "apiKey", "keys"])?;
	let lookup_id = previous_id.unwrap_or(&prepared.id);
	if let Some(existing) = keys
		.iter()
		.enumerate()
		.position(|(index, value)| file_api_key_id(value, index) == lookup_id)
	{
		keys[existing] = prepared.value.clone();
	} else if previous_id.is_some() {
		return Err(
			ConfigResourceError::NotFound(format!(
				"config resource not found: {}/{}",
				prepared.kind, lookup_id
			))
			.into(),
		);
	} else {
		keys.push(prepared.value.clone());
	}
	Ok(())
}

fn delete_file_api_key(config: &mut Value, id: &str) -> anyhow::Result<bool> {
	let Some(keys) = crate::json::traverse_mut(config, &["llm", "policies", "apiKey", "keys"])
		.and_then(Value::as_array_mut)
	else {
		return Ok(false);
	};
	if let Some(index) = keys
		.iter()
		.enumerate()
		.position(|(index, value)| file_api_key_id(value, index) == id)
	{
		keys.remove(index);
		return Ok(true);
	}
	Ok(false)
}

/// Projects the singleton MCP settings resource onto its top-level `mcp` fields.
fn upsert_file_mcp_settings(config: &mut Value, value: &Value) -> anyhow::Result<()> {
	let value = value
		.as_object()
		.ok_or_else(|| anyhow::anyhow!("mcp.settings/default must be an object"))?;
	let mcp = ensure_file_object(config, &["mcp"])?;
	for field in MCP_SETTINGS_FIELDS {
		if let Some(value) = value.get(field) {
			mcp.insert(field.to_string(), value.clone());
		} else {
			mcp.remove(field);
		}
	}
	Ok(())
}

fn delete_file_mcp_settings(config: &mut Value) -> anyhow::Result<bool> {
	let Some(mcp) = crate::json::traverse_mut(config, &["mcp"]).and_then(Value::as_object_mut) else {
		return Ok(false);
	};
	let mut deleted = false;
	for field in MCP_SETTINGS_FIELDS {
		deleted |= mcp.remove(field).is_some();
	}
	Ok(deleted)
}

/// Replaces only the inline catalog overlay, preserving file-backed catalog sources.
fn upsert_file_model_catalog(config: &mut Value, value: &Value) -> anyhow::Result<()> {
	let value = value
		.as_object()
		.ok_or_else(|| anyhow::anyhow!("modelCatalog resource must be an object"))?;
	let sources = ensure_file_array(config, &["config", "modelCatalog"])?;
	sources.retain(|source| source.get("inline").is_none());
	if let Some(custom) = value.get("custom") {
		sources.push(serde_json::json!({ "inline": custom }));
	}
	Ok(())
}

fn delete_file_model_catalog(config: &mut Value) -> anyhow::Result<bool> {
	let Some(sources) =
		crate::json::traverse_mut(config, &["config", "modelCatalog"]).and_then(Value::as_array_mut)
	else {
		return Ok(false);
	};
	let before = sources.len();
	sources.retain(|source| source.get("inline").is_none());
	Ok(sources.len() != before)
}

pub(crate) fn prepare_file_api_key_update(
	id: String,
	mut value: Value,
) -> anyhow::Result<PreparedResource> {
	validate_id(&id)?;
	ensure_no_api_key_id(&value)?;
	if !id.starts_with("@index:") {
		set_api_key_id(&mut value, id.clone())?;
	}
	Ok(PreparedResource {
		kind: ConfigResourceKind::LlmApiKey,
		id,
		value,
	})
}
pub(crate) fn prepare_resources(
	kind: ConfigResourceKind,
	request: ConfigResourceUpsertRequest,
) -> anyhow::Result<Vec<PreparedResource>> {
	request
		.resources
		.into_iter()
		.map(|resource| prepare_resource(kind, resource.value))
		.collect()
}

pub(crate) fn prepare_api_key_update(
	id: String,
	mut value: Value,
) -> anyhow::Result<PreparedResource> {
	validate_id(&id)?;
	ensure_no_api_key_id(&value)?;
	set_api_key_id(&mut value, id.clone())?;
	Ok(PreparedResource {
		kind: ConfigResourceKind::LlmApiKey,
		id,
		value,
	})
}

pub(crate) fn prepare_policy_upsert(
	kind: ConfigResourceKind,
	id: String,
	value: Value,
) -> anyhow::Result<PreparedResource> {
	validate_id(&id)?;
	if !matches!(
		kind,
		ConfigResourceKind::LlmPolicy | ConfigResourceKind::McpPolicy | ConfigResourceKind::UiPolicy
	) {
		return Err(
			ConfigResourceError::InvalidRequest(format!("{kind} is not a policy resource")).into(),
		);
	}
	if kind == ConfigResourceKind::LlmPolicy && id == "apiKey" && value.get("keys").is_some() {
		return Err(
			ConfigResourceError::InvalidRequest(
				"llm.policy/apiKey must not include keys; use llm.apiKey resources".to_string(),
			)
			.into(),
		);
	}
	Ok(PreparedResource { kind, id, value })
}

pub fn model_catalog_sources(
	resources: &[ConfigResource],
) -> anyhow::Result<Vec<crate::ModelCatalogSource>> {
	let Some(resource) = resources
		.iter()
		.find(|resource| resource.kind == ConfigResourceKind::ModelCatalog)
	else {
		return Ok(Vec::new());
	};
	let value = resource
		.value
		.as_object()
		.ok_or_else(|| anyhow::anyhow!("modelCatalog resource must be an object"))?;
	["base", "custom"]
		.into_iter()
		.filter_map(|field| value.get(field))
		.map(|inline| {
			Ok(crate::ModelCatalogSource::InlineCatalog {
				inline: serde_json::from_value(inline.clone())?,
			})
		})
		.collect()
}

pub(crate) fn apply_prepared_upsert(
	mut resources: Vec<ConfigResource>,
	prepared: &[PreparedResource],
) -> anyhow::Result<Vec<ConfigResource>> {
	for prepared in prepared {
		validate_id(&prepared.id)?;
		if let Some(existing) = resources
			.iter_mut()
			.find(|resource| resource.kind == prepared.kind && resource.id == prepared.id)
		{
			existing.value = prepared.value.clone();
			continue;
		}
		resources.push(ConfigResource {
			kind: prepared.kind,
			id: prepared.id.clone(),
			value: prepared.value.clone(),
			revision: 1,
			created_at: Utc::now(),
			updated_at: Utc::now(),
		});
	}
	Ok(resources)
}

pub(crate) fn apply_delete(
	resources: Vec<ConfigResource>,
	kind: ConfigResourceKind,
	id: &str,
) -> Vec<ConfigResource> {
	resources
		.into_iter()
		.filter(|resource| !(resource.kind == kind && resource.id == id))
		.collect()
}

pub async fn materialize_hybrid_config(
	source: &crate::ConfigSource,
	store: &ConfigResourceStore,
) -> anyhow::Result<String> {
	let base = source.read_to_string().await?;
	let resources = store.list(None).await?;
	materialize_config(base.as_str(), &resources)
}

pub(crate) fn materialize_config(
	base: &str,
	resources: &[ConfigResource],
) -> anyhow::Result<String> {
	let mut config: Value = crate::yamlviajson::from_str(base)?;
	overlay_config_resources(&mut config, resources)?;
	crate::yamlviajson::to_string(&config)
}

fn overlay_config_resources(
	config: &mut Value,
	resources: &[ConfigResource],
) -> anyhow::Result<()> {
	let has_llm_resources = resources.iter().any(|resource| {
		matches!(
			resource.kind,
			ConfigResourceKind::LlmProvider
				| ConfigResourceKind::LlmModel
				| ConfigResourceKind::LlmVirtualModel
				| ConfigResourceKind::LlmApiKey
				| ConfigResourceKind::LlmPolicy
		)
	});
	let has_ui_resources = resources
		.iter()
		.any(|resource| resource.kind == ConfigResourceKind::UiPolicy);
	let has_mcp_resources = resources.iter().any(|resource| {
		matches!(
			resource.kind,
			ConfigResourceKind::McpTarget
				| ConfigResourceKind::McpPolicy
				| ConfigResourceKind::McpSettings
		)
	});
	let has_traffic_resources = resources.iter().any(|resource| {
		matches!(
			resource.kind,
			ConfigResourceKind::TrafficGateway
				| ConfigResourceKind::TrafficRoute
				| ConfigResourceKind::TrafficTcpRoute
		)
	});
	let has_llm_policies = resources
		.iter()
		.any(|resource| resource.kind == ConfigResourceKind::LlmPolicy);
	if !has_llm_resources && !has_mcp_resources && !has_traffic_resources && !has_ui_resources {
		return Ok(());
	}

	let Some(root) = config.as_object_mut() else {
		anyhow::bail!("local config root must be a JSON object");
	};
	if has_llm_resources {
		if has_llm_policies && !root.contains_key("llm") {
			return Err(
				ConfigResourceError::Conflict(
					"DB-backed LLM policies require llm in the file config".to_string(),
				)
				.into(),
			);
		}
		let llm = root
			.entry("llm")
			.or_insert_with(|| Value::Object(serde_json::Map::new()));
		let Some(llm) = llm.as_object_mut() else {
			if has_llm_policies {
				return Err(
					ConfigResourceError::Conflict(
						"DB-backed LLM policies require llm to be an object in the file config".to_string(),
					)
					.into(),
				);
			}
			anyhow::bail!("local config llm must be a JSON object");
		};

		append_policy_kind(llm, resources, ConfigResourceKind::LlmPolicy, "llm")?;
		append_llm_kind(llm, resources, ConfigResourceKind::LlmProvider, "providers")?;
		append_llm_kind(llm, resources, ConfigResourceKind::LlmModel, "models")?;
		append_llm_kind(
			llm,
			resources,
			ConfigResourceKind::LlmVirtualModel,
			"virtualModels",
		)?;
		append_api_keys(llm, resources)?;
	}
	if has_mcp_resources {
		let mcp = root
			.entry("mcp")
			.or_insert_with(|| Value::Object(serde_json::Map::new()));
		let Some(mcp) = mcp.as_object_mut() else {
			anyhow::bail!("local config mcp must be a JSON object");
		};
		mcp
			.entry("targets")
			.or_insert_with(|| Value::Array(Vec::new()));

		append_mcp_settings(mcp, resources)?;
		append_policy_kind(mcp, resources, ConfigResourceKind::McpPolicy, "mcp")?;
		append_list_kind(
			mcp,
			resources,
			ConfigResourceKind::McpTarget,
			"targets",
			"mcp",
		)?;
	}
	if has_traffic_resources {
		append_traffic_gateways(root, resources)?;
		append_traffic_routes(root, resources, ConfigResourceKind::TrafficRoute, "routes")?;
		append_traffic_routes(
			root,
			resources,
			ConfigResourceKind::TrafficTcpRoute,
			"tcpRoutes",
		)?;
	}
	if has_ui_resources {
		let Some(ui) = root.get_mut("ui") else {
			return Err(
				ConfigResourceError::Conflict(
					"DB-backed UI policies require ui in the file config".to_string(),
				)
				.into(),
			);
		};
		let Some(ui) = ui.as_object_mut() else {
			return Err(
				ConfigResourceError::Conflict(
					"DB-backed UI policies require ui to be an object in the file config".to_string(),
				)
				.into(),
			);
		};
		append_policy_kind(ui, resources, ConfigResourceKind::UiPolicy, "ui")?;
	}
	Ok(())
}

fn append_policy_kind(
	section: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
	section_name: &str,
) -> anyhow::Result<()> {
	let Some(db_resources) = non_empty_resources(resources, kind) else {
		return Ok(());
	};
	let policies = section
		.entry("policies")
		.or_insert_with(|| Value::Object(serde_json::Map::new()));
	if policies.is_null() {
		*policies = Value::Object(serde_json::Map::new());
	}
	let policies = policies.as_object_mut().ok_or_else(|| {
		ConfigResourceError::Conflict(format!(
			"DB-backed {section_name} policies require {section_name}.policies to be an object in the file config"
		))
	})?;
	for resource in db_resources {
		if policies.contains_key(&resource.id) {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource {kind}/{} conflicts with file-owned resource",
					resource.id
				))
				.into(),
			);
		}
		let mut value = resource.value.clone();
		if kind == ConfigResourceKind::LlmPolicy && resource.id == "apiKey" {
			let Some(policy) = value.as_object_mut() else {
				return Err(
					ConfigResourceError::InvalidRequest("llm.policy/apiKey must be an object".to_string())
						.into(),
				);
			};
			if policy.contains_key("keys") {
				return Err(
					ConfigResourceError::InvalidRequest(
						"llm.policy/apiKey must not include keys; use llm.apiKey resources".to_string(),
					)
					.into(),
				);
			}
			policy.insert("keys".to_string(), Value::Array(Vec::new()));
		}
		policies.insert(resource.id.clone(), value);
	}
	Ok(())
}

fn append_api_keys(
	llm: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
) -> anyhow::Result<()> {
	let Some(db_resources) = non_empty_resources(resources, ConfigResourceKind::LlmApiKey) else {
		return Ok(());
	};
	let policies = llm
		.get_mut("policies")
		.ok_or_else(|| {
			ConfigResourceError::Conflict("DB-backed API keys require llm.policies.apiKey".to_string())
		})?
		.as_object_mut()
		.ok_or_else(|| anyhow::anyhow!("local config llm.policies must be an object"))?;
	let policy = policies
		.get_mut("apiKey")
		.ok_or_else(|| {
			ConfigResourceError::Conflict("DB-backed API keys require llm.policies.apiKey".to_string())
		})?
		.as_object_mut()
		.ok_or_else(|| anyhow::anyhow!("local config llm.policies.apiKey must be an object"))?;
	let keys = policy
		.entry("keys")
		.or_insert_with(|| Value::Array(Vec::new()))
		.as_array_mut()
		.ok_or_else(|| anyhow::anyhow!("local config llm.policies.apiKey.keys must be an array"))?;

	keys.extend(
		db_resources
			.into_iter()
			.map(|resource| resource.value.clone()),
	);
	Ok(())
}

fn append_list_kind(
	section: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
	field: &str,
	section_name: &str,
) -> anyhow::Result<()> {
	let Some(db_resources) = non_empty_resources(resources, kind) else {
		return Ok(());
	};

	let values = section
		.entry(field)
		.or_insert_with(|| Value::Array(Vec::new()));
	let Some(values) = values.as_array_mut() else {
		anyhow::bail!("local config {section_name}.{field} must be an array");
	};

	for existing in values.iter() {
		let id = resource_id(kind, existing)?;
		if db_resources.iter().any(|resource| resource.id == id) {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource {kind}/{id} conflicts with file-owned resource"
				))
				.into(),
			);
		}
	}

	for resource in db_resources {
		values.push(resource.value.clone());
	}
	Ok(())
}

fn append_llm_kind(
	llm: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
	field: &str,
) -> anyhow::Result<()> {
	append_list_kind(llm, resources, kind, field, "llm")
}

fn append_mcp_settings(
	mcp: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
) -> anyhow::Result<()> {
	let Some(settings) = resources
		.iter()
		.find(|resource| resource.kind == ConfigResourceKind::McpSettings)
	else {
		return Ok(());
	};
	let value = settings.value.as_object().ok_or_else(|| {
		ConfigResourceError::InvalidRequest("mcp.settings/default must be an object".to_string())
	})?;
	for (field, value) in value {
		if !MCP_SETTINGS_FIELDS.contains(&field.as_str()) {
			return Err(
				ConfigResourceError::InvalidRequest(format!(
					"mcp.settings/default contains unsupported field: {field}"
				))
				.into(),
			);
		}
		if mcp.contains_key(field) {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource mcp.settings/default field {field} conflicts with file-owned configuration"
				))
				.into(),
			);
		}
		mcp.insert(field.clone(), value.clone());
	}
	Ok(())
}

fn append_traffic_gateways(
	root: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
) -> anyhow::Result<()> {
	let Some(db_resources) = non_empty_resources(resources, ConfigResourceKind::TrafficGateway)
	else {
		return Ok(());
	};
	let gateways = root
		.entry("gateways")
		.or_insert_with(|| Value::Object(serde_json::Map::new()));
	let Some(gateways) = gateways.as_object_mut() else {
		anyhow::bail!("local config gateways must be a JSON object");
	};
	for resource in db_resources {
		if gateways.contains_key(&resource.id) {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource traffic.gateway/{} conflicts with file-owned resource",
					resource.id
				))
				.into(),
			);
		}
		let mut value = resource.value.clone();
		let Some(value) = value.as_object_mut() else {
			return Err(
				ConfigResourceError::InvalidRequest(format!(
					"traffic.gateway/{} must be an object",
					resource.id
				))
				.into(),
			);
		};
		value.remove("name");
		gateways.insert(resource.id.clone(), Value::Object(std::mem::take(value)));
	}
	Ok(())
}

fn append_traffic_routes(
	root: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
	field: &str,
) -> anyhow::Result<()> {
	let Some(mut db_resources) = non_empty_resources(resources, kind) else {
		return Ok(());
	};
	db_resources.sort_by(|left, right| left.id.cmp(&right.id));
	let routes = root
		.entry(field)
		.or_insert_with(|| Value::Array(Vec::new()));
	let Some(routes) = routes.as_array_mut() else {
		anyhow::bail!("local config {field} must be an array");
	};
	for existing in routes.iter() {
		let Some(name) = existing.get("name").and_then(Value::as_str) else {
			continue;
		};
		if db_resources.iter().any(|resource| resource.id == name) {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource {kind}/{name} conflicts with file-owned resource"
				))
				.into(),
			);
		}
	}
	for resource in db_resources {
		routes.push(resource.value.clone());
	}
	Ok(())
}

fn non_empty_resources(
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
) -> Option<Vec<&ConfigResource>> {
	let resources = resources
		.iter()
		.filter(|resource| resource.kind == kind)
		.collect::<Vec<_>>();
	(!resources.is_empty()).then_some(resources)
}

pub(crate) fn prepare_resource(
	kind: ConfigResourceKind,
	mut value: Value,
) -> anyhow::Result<PreparedResource> {
	let id = match kind {
		ConfigResourceKind::LlmApiKey => {
			ensure_no_api_key_id(&value)?;
			let id = uuid::Uuid::new_v4().to_string();
			set_api_key_id(&mut value, id.clone())?;
			id
		},
		ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::UiPolicy => {
			return Err(
				ConfigResourceError::InvalidRequest(format!("{kind} resources require an item ID")).into(),
			);
		},
		_ => resource_id(kind, &value)?,
	};
	Ok(PreparedResource { kind, id, value })
}

fn resource_id(kind: ConfigResourceKind, value: &Value) -> anyhow::Result<String> {
	match kind {
		ConfigResourceKind::ModelCatalog => Ok("default".to_string()),
		ConfigResourceKind::McpSettings => Ok("default".to_string()),
		ConfigResourceKind::LlmProvider
		| ConfigResourceKind::LlmVirtualModel
		| ConfigResourceKind::McpTarget
		| ConfigResourceKind::TrafficGateway
		| ConfigResourceKind::TrafficRoute
		| ConfigResourceKind::TrafficTcpRoute => string_field(value, "name", || {
			format!("{kind} resources require value.name")
		}),
		ConfigResourceKind::LlmModel => string_field(value, "id", || {
			"llm.model resources require value.id or value.name".to_string()
		})
		.or_else(|_| {
			string_field(value, "name", || {
				"llm.model resources require value.id or value.name".to_string()
			})
		}),
		ConfigResourceKind::LlmApiKey => value
			.pointer("/metadata/id")
			.and_then(Value::as_str)
			.map(ToString::to_string)
			.ok_or_else(|| {
				ConfigResourceError::InvalidRequest(
					"llm.apiKey resources require value.metadata.id".to_string(),
				)
				.into()
			}),
		ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::UiPolicy => Err(
			ConfigResourceError::InvalidRequest(format!("{kind} resources require an item ID")).into(),
		),
	}
}

fn ensure_no_api_key_id(value: &Value) -> anyhow::Result<()> {
	if value.pointer("/metadata/id").is_some() {
		return Err(
			ConfigResourceError::InvalidRequest(
				"llm.apiKey resources must not include value.metadata.id".to_string(),
			)
			.into(),
		);
	}
	Ok(())
}

fn set_api_key_id(value: &mut Value, id: String) -> anyhow::Result<()> {
	let Some(object) = value.as_object_mut() else {
		return Err(
			ConfigResourceError::InvalidRequest("llm.apiKey resources must be JSON objects".to_string())
				.into(),
		);
	};
	let metadata = object
		.entry("metadata")
		.or_insert_with(|| Value::Object(serde_json::Map::new()));
	let Some(metadata) = metadata.as_object_mut() else {
		return Err(
			ConfigResourceError::InvalidRequest(
				"llm.apiKey resources require value.metadata to be an object".to_string(),
			)
			.into(),
		);
	};
	metadata.insert("id".to_string(), Value::String(id));
	Ok(())
}

fn string_field(
	value: &Value,
	field: &str,
	error: impl FnOnce() -> String,
) -> anyhow::Result<String> {
	value
		.get(field)
		.and_then(Value::as_str)
		.filter(|value| !value.is_empty())
		.map(ToString::to_string)
		.ok_or_else(|| ConfigResourceError::InvalidRequest(error()).into())
}

async fn list_sqlite(
	pool: &SqlitePool,
	kind: Option<ConfigResourceKind>,
) -> anyhow::Result<Vec<ConfigResource>> {
	let rows = if let Some(kind) = kind {
		sqlx::query(
			"SELECT kind, id, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL AND kind = ? \
			 ORDER BY id",
		)
		.bind(kind.as_str())
		.fetch_all(pool)
		.await?
	} else {
		sqlx::query(
			"SELECT kind, id, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL \
			 ORDER BY kind, id",
		)
		.fetch_all(pool)
		.await?
	};
	rows.into_iter().map(sqlite_row_to_resource).collect()
}

async fn list_postgres(
	pool: &PgPool,
	kind: Option<ConfigResourceKind>,
) -> anyhow::Result<Vec<ConfigResource>> {
	let rows = if let Some(kind) = kind {
		sqlx::query(
			"SELECT kind, id, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL AND kind = $1 \
			 ORDER BY id",
		)
		.bind(kind.as_str())
		.fetch_all(pool)
		.await?
	} else {
		sqlx::query(
			"SELECT kind, id, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL \
			 ORDER BY kind, id",
		)
		.fetch_all(pool)
		.await?
	};
	rows.into_iter().map(postgres_row_to_resource).collect()
}

async fn upsert_sqlite(
	pool: &SqlitePool,
	prepared: Vec<PreparedResource>,
) -> anyhow::Result<Vec<ConfigResource>> {
	let mut tx = pool.begin().await?;
	let mut changed = Vec::with_capacity(prepared.len());
	for PreparedResource { kind, id, value } in prepared {
		validate_id(&id)?;
		let now = Utc::now().to_rfc3339();
		sqlx::query(
			"INSERT INTO agw_config_resources \
			 (kind, id, value_json, revision, created_at, updated_at, deleted_at) \
			 VALUES (?, ?, ?, 1, ?, ?, NULL) \
			 ON CONFLICT(kind, id) DO UPDATE SET \
				value_json = excluded.value_json, \
				revision = agw_config_resources.revision + 1, \
				updated_at = excluded.updated_at, \
				deleted_at = NULL",
		)
		.bind(kind.as_str())
		.bind(&id)
		.bind(serde_json::to_string(&value)?)
		.bind(&now)
		.bind(&now)
		.execute(&mut *tx)
		.await?;
		changed.push((kind, id));
	}

	let mut resources = Vec::with_capacity(changed.len());
	for (kind, id) in changed {
		if let Some(resource) = fetch_sqlite_resource(&mut tx, kind, &id).await? {
			resources.push(resource);
		}
	}
	tx.commit().await?;
	Ok(resources)
}

async fn upsert_postgres(
	pool: &PgPool,
	prepared: Vec<PreparedResource>,
	notification_id: &str,
) -> anyhow::Result<Vec<ConfigResource>> {
	let mut tx = pool.begin().await?;
	let mut changed = Vec::with_capacity(prepared.len());
	for PreparedResource { kind, id, value } in prepared {
		validate_id(&id)?;
		let now = Utc::now();
		sqlx::query(
			"INSERT INTO agw_config_resources \
			 (kind, id, value_json, revision, created_at, updated_at, deleted_at) \
			 VALUES ($1, $2, $3, 1, $4, $4, NULL) \
			 ON CONFLICT(kind, id) DO UPDATE SET \
				value_json = excluded.value_json, \
				revision = agw_config_resources.revision + 1, \
				updated_at = excluded.updated_at, \
				deleted_at = NULL",
		)
		.bind(kind.as_str())
		.bind(&id)
		.bind(Json(value))
		.bind(now)
		.execute(&mut *tx)
		.await?;
		changed.push((kind, id));
	}

	let mut resources = Vec::with_capacity(changed.len());
	for (kind, id) in changed {
		if let Some(resource) = fetch_postgres_resource(&mut tx, kind, &id).await? {
			resources.push(resource);
		}
	}
	if !resources.is_empty() {
		notify_postgres(&mut tx, notification_id).await?;
	}
	tx.commit().await?;
	Ok(resources)
}

async fn rename_sqlite(
	pool: &SqlitePool,
	previous_kind: ConfigResourceKind,
	previous_id: &str,
	prepared: PreparedResource,
) -> anyhow::Result<ConfigResource> {
	if previous_kind != prepared.kind || previous_id == prepared.id {
		return Err(
			ConfigResourceError::InvalidRequest("config resource rename requires a new ID".to_string())
				.into(),
		);
	}
	let mut tx = pool.begin().await?;
	let now = Utc::now().to_rfc3339();
	if !soft_delete_sqlite(&mut tx, previous_kind, previous_id, &now).await? {
		return Err(
			ConfigResourceError::NotFound(format!(
				"config resource not found: {previous_kind}/{previous_id}"
			))
			.into(),
		);
	}

	let inserted = sqlx::query(
		"INSERT INTO agw_config_resources \
		 (kind, id, value_json, revision, created_at, updated_at, deleted_at) \
		 VALUES (?, ?, ?, 1, ?, ?, NULL) \
		 ON CONFLICT(kind, id) DO UPDATE SET \
			value_json = excluded.value_json, \
			revision = agw_config_resources.revision + 1, \
			updated_at = excluded.updated_at, \
			deleted_at = NULL \
		 WHERE agw_config_resources.deleted_at IS NOT NULL",
	)
	.bind(prepared.kind.as_str())
	.bind(&prepared.id)
	.bind(serde_json::to_string(&prepared.value)?)
	.bind(&now)
	.bind(&now)
	.execute(&mut *tx)
	.await?;
	if inserted.rows_affected() == 0 {
		return Err(
			ConfigResourceError::Conflict(format!(
				"config resource already exists: {}/{}",
				prepared.kind, prepared.id
			))
			.into(),
		);
	}
	let resource = fetch_sqlite_resource(&mut tx, prepared.kind, &prepared.id)
		.await?
		.ok_or_else(|| anyhow::anyhow!("renamed config resource was not found"))?;
	tx.commit().await?;
	Ok(resource)
}

async fn rename_postgres(
	pool: &PgPool,
	previous_kind: ConfigResourceKind,
	previous_id: &str,
	prepared: PreparedResource,
	notification_id: &str,
) -> anyhow::Result<ConfigResource> {
	if previous_kind != prepared.kind || previous_id == prepared.id {
		return Err(
			ConfigResourceError::InvalidRequest("config resource rename requires a new ID".to_string())
				.into(),
		);
	}
	let mut tx = pool.begin().await?;
	let now = Utc::now();
	if !soft_delete_postgres(&mut tx, previous_kind, previous_id, now).await? {
		return Err(
			ConfigResourceError::NotFound(format!(
				"config resource not found: {previous_kind}/{previous_id}"
			))
			.into(),
		);
	}

	let inserted = sqlx::query(
		"INSERT INTO agw_config_resources \
		 (kind, id, value_json, revision, created_at, updated_at, deleted_at) \
		 VALUES ($1, $2, $3, 1, $4, $4, NULL) \
		 ON CONFLICT(kind, id) DO UPDATE SET \
			value_json = excluded.value_json, \
			revision = agw_config_resources.revision + 1, \
			updated_at = excluded.updated_at, \
			deleted_at = NULL \
		 WHERE agw_config_resources.deleted_at IS NOT NULL",
	)
	.bind(prepared.kind.as_str())
	.bind(&prepared.id)
	.bind(Json(prepared.value))
	.bind(now)
	.execute(&mut *tx)
	.await?;
	if inserted.rows_affected() == 0 {
		return Err(
			ConfigResourceError::Conflict(format!(
				"config resource already exists: {}/{}",
				prepared.kind, prepared.id
			))
			.into(),
		);
	}
	let resource = fetch_postgres_resource(&mut tx, prepared.kind, &prepared.id)
		.await?
		.ok_or_else(|| anyhow::anyhow!("renamed config resource was not found"))?;
	notify_postgres(&mut tx, notification_id).await?;
	tx.commit().await?;
	Ok(resource)
}

async fn delete_sqlite(
	pool: &SqlitePool,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<bool> {
	let mut tx = pool.begin().await?;
	let now = Utc::now().to_rfc3339();
	let deleted = soft_delete_sqlite(&mut tx, kind, id, &now).await?;
	tx.commit().await?;
	Ok(deleted)
}

async fn soft_delete_sqlite(
	tx: &mut Transaction<'_, Sqlite>,
	kind: ConfigResourceKind,
	id: &str,
	now: &str,
) -> anyhow::Result<bool> {
	let result = sqlx::query(
		"UPDATE agw_config_resources \
		 SET revision = revision + 1, updated_at = ?, deleted_at = ? \
		 WHERE kind = ? AND id = ? AND deleted_at IS NULL",
	)
	.bind(now)
	.bind(now)
	.bind(kind.as_str())
	.bind(id)
	.execute(&mut **tx)
	.await?;
	Ok(result.rows_affected() > 0)
}

async fn delete_postgres(
	pool: &PgPool,
	kind: ConfigResourceKind,
	id: &str,
	notification_id: &str,
) -> anyhow::Result<bool> {
	let mut tx = pool.begin().await?;
	let now = Utc::now();
	let deleted = soft_delete_postgres(&mut tx, kind, id, now).await?;
	if deleted {
		notify_postgres(&mut tx, notification_id).await?;
	}
	tx.commit().await?;
	Ok(deleted)
}

async fn soft_delete_postgres(
	tx: &mut Transaction<'_, Postgres>,
	kind: ConfigResourceKind,
	id: &str,
	now: DateTime<Utc>,
) -> anyhow::Result<bool> {
	let result = sqlx::query(
		"UPDATE agw_config_resources \
		 SET revision = revision + 1, updated_at = $1, deleted_at = $1 \
		 WHERE kind = $2 AND id = $3 AND deleted_at IS NULL",
	)
	.bind(now)
	.bind(kind.as_str())
	.bind(id)
	.execute(&mut **tx)
	.await?;
	Ok(result.rows_affected() > 0)
}

async fn notify_postgres(
	tx: &mut Transaction<'_, Postgres>,
	notification_id: &str,
) -> anyhow::Result<()> {
	sqlx::query("SELECT pg_notify($1, $2)")
		.bind(POSTGRES_CHANGE_CHANNEL)
		.bind(notification_id)
		.execute(&mut **tx)
		.await?;
	Ok(())
}

async fn fetch_sqlite_resource(
	tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<Option<ConfigResource>> {
	sqlx::query(
		"SELECT kind, id, value_json, revision, created_at, updated_at \
		 FROM agw_config_resources \
		 WHERE kind = ? AND id = ? AND deleted_at IS NULL",
	)
	.bind(kind.as_str())
	.bind(id)
	.fetch_optional(&mut **tx)
	.await?
	.map(sqlite_row_to_resource)
	.transpose()
}

async fn fetch_postgres_resource(
	tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<Option<ConfigResource>> {
	sqlx::query(
		"SELECT kind, id, value_json, revision, created_at, updated_at \
		 FROM agw_config_resources \
		 WHERE kind = $1 AND id = $2 AND deleted_at IS NULL",
	)
	.bind(kind.as_str())
	.bind(id)
	.fetch_optional(&mut **tx)
	.await?
	.map(postgres_row_to_resource)
	.transpose()
}

fn sqlite_row_to_resource(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<ConfigResource> {
	let value_json: String = row.try_get("value_json")?;
	let kind: String = row.try_get("kind")?;
	let created_at: String = row.try_get("created_at")?;
	let updated_at: String = row.try_get("updated_at")?;
	Ok(ConfigResource {
		kind: kind.parse()?,
		id: row.try_get("id")?,
		value: serde_json::from_str(&value_json)?,
		revision: row.try_get("revision")?,
		created_at: created_at.parse()?,
		updated_at: updated_at.parse()?,
	})
}

fn postgres_row_to_resource(row: sqlx::postgres::PgRow) -> anyhow::Result<ConfigResource> {
	let kind: String = row.try_get("kind")?;
	let Json(value) = row.try_get("value_json")?;
	Ok(ConfigResource {
		kind: kind.parse()?,
		id: row.try_get("id")?,
		value,
		revision: row.try_get("revision")?,
		created_at: row.try_get("created_at")?,
		updated_at: row.try_get("updated_at")?,
	})
}

const SQLITE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agw_config_resources (
	kind TEXT NOT NULL,
	id TEXT NOT NULL,
	value_json TEXT NOT NULL CHECK (json_valid(value_json)),
	revision INTEGER NOT NULL DEFAULT 1,
	created_at TEXT NOT NULL,
	updated_at TEXT NOT NULL,
	deleted_at TEXT,
	PRIMARY KEY (kind, id)
);

CREATE INDEX IF NOT EXISTS idx_agw_config_resources_kind_updated
	ON agw_config_resources(kind, updated_at);
"#;

const POSTGRES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agw_config_resources (
	kind TEXT NOT NULL,
	id TEXT NOT NULL,
	value_json JSONB NOT NULL,
	revision BIGINT NOT NULL DEFAULT 1,
	created_at TIMESTAMPTZ NOT NULL,
	updated_at TIMESTAMPTZ NOT NULL,
	deleted_at TIMESTAMPTZ,
	PRIMARY KEY (kind, id)
);

CREATE INDEX IF NOT EXISTS idx_agw_config_resources_kind_updated
	ON agw_config_resources(kind, updated_at);
"#;

const POSTGRES_CHANGE_CHANNEL: &str = "agentgateway_config_changed";

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn test_resource(kind: ConfigResourceKind, id: &str, value: Value) -> ConfigResource {
		ConfigResource {
			kind,
			id: id.to_string(),
			value,
			revision: 1,
			created_at: Utc::now(),
			updated_at: Utc::now(),
		}
	}

	#[test]
	fn derives_resource_ids_and_manages_api_key_ids() {
		let err = "traffic.listener"
			.parse::<ConfigResourceKind>()
			.expect_err("unsupported kind should fail");
		assert!(err.to_string().contains("unsupported config resource kind"));
		assert_eq!(
			resource_id(ConfigResourceKind::LlmProvider, &json!({"name": "openai"}))
				.expect("provider id"),
			"openai"
		);
		assert_eq!(
			resource_id(ConfigResourceKind::LlmModel, &json!({"name": "fast"})).expect("model id"),
			"fast"
		);
		assert_eq!(
			resource_id(
				ConfigResourceKind::LlmModel,
				&json!({"id": "model_01", "name": "fast"})
			)
			.expect("stable model id"),
			"model_01"
		);
		assert_eq!(
			resource_id(
				ConfigResourceKind::LlmVirtualModel,
				&json!({"name": "router"})
			)
			.expect("virtual model id"),
			"router"
		);
		assert_eq!(
			resource_id(
				ConfigResourceKind::McpTarget,
				&json!({"name": "everything"})
			)
			.expect("MCP target id"),
			"everything"
		);
		assert_eq!(
			resource_id(ConfigResourceKind::McpSettings, &json!({})).expect("MCP settings id"),
			"default"
		);
		assert_eq!(
			resource_id(ConfigResourceKind::TrafficRoute, &json!({"name": "api"}))
				.expect("traffic route id"),
			"api"
		);
		assert!(
			resource_id(ConfigResourceKind::TrafficRoute, &json!({})).is_err(),
			"DB-backed routes require a name"
		);

		let created = prepare_resource(
			ConfigResourceKind::LlmApiKey,
			json!({"metadata": {"name": "ci"}}),
		)
		.expect("api key id");
		uuid::Uuid::parse_str(&created.id).expect("UUID v4");
		assert_eq!(
			created.value.pointer("/metadata/id"),
			Some(&Value::String(created.id))
		);
		assert!(
			prepare_resource(
				ConfigResourceKind::LlmApiKey,
				json!({"metadata": {"id": "client-id"}}),
			)
			.is_err()
		);
		assert!(
			prepare_policy_upsert(
				ConfigResourceKind::LlmPolicy,
				"apiKey".to_string(),
				json!({"keys": []}),
			)
			.is_err()
		);
	}

	#[tokio::test]
	async fn renames_resources_atomically() {
		let store = ConfigResourceStore::connect("sqlite::memory:")
			.await
			.expect("connect config resource store");
		store
			.upsert_prepared(vec![PreparedResource {
				kind: ConfigResourceKind::LlmProvider,
				id: "old".to_string(),
				value: json!({"name": "old", "provider": "openAI"}),
			}])
			.await
			.expect("create resource");
		let response = store
			.rename_prepared(
				ConfigResourceKind::LlmProvider,
				"old",
				PreparedResource {
					kind: ConfigResourceKind::LlmProvider,
					id: "new".to_string(),
					value: json!({"name": "new", "provider": "openAI"}),
				},
			)
			.await
			.expect("rename resource");
		assert_eq!(response.resources[0].id, "new");
		assert_eq!(
			store
				.list(Some(ConfigResourceKind::LlmProvider))
				.await
				.expect("list resources")
				.into_iter()
				.map(|resource| resource.id)
				.collect::<Vec<_>>(),
			vec!["new"]
		);

		store
			.upsert_prepared(vec![PreparedResource {
				kind: ConfigResourceKind::LlmProvider,
				id: "other".to_string(),
				value: json!({"name": "other", "provider": "openAI"}),
			}])
			.await
			.expect("create rename target");
		let err = store
			.rename_prepared(
				ConfigResourceKind::LlmProvider,
				"new",
				PreparedResource {
					kind: ConfigResourceKind::LlmProvider,
					id: "other".to_string(),
					value: json!({"name": "other", "provider": "anthropic"}),
				},
			)
			.await
			.expect_err("rename collision should fail");
		assert!(err.to_string().contains("already exists"));
		assert_eq!(
			store
				.list(Some(ConfigResourceKind::LlmProvider))
				.await
				.expect("list resources after failed rename")
				.into_iter()
				.map(|resource| resource.id)
				.collect::<Vec<_>>(),
			vec!["new", "other"]
		);
	}

	#[test]
	fn materializes_config_resources_into_base_config() {
		let base = r#"
llm:
  models:
  - name: file-model
    provider:
      openAI:
        model: gpt-4o-mini
ui:
  gateways: default
mcp:
  targets:
  - name: file-target
    mcp:
      host: http://localhost:3001/mcp
"#;
		let resources = vec![
			test_resource(
				ConfigResourceKind::LlmProvider,
				"openai",
				json!({"name": "openai", "provider": "openAI", "params": {"model": "gpt-4o"}}),
			),
			test_resource(
				ConfigResourceKind::LlmModel,
				"model_01",
				json!({"id": "model_01", "name": "db-model", "provider": {"reference": "openai"}}),
			),
			test_resource(
				ConfigResourceKind::LlmVirtualModel,
				"router",
				json!({"name": "router", "routing": {"weighted": {"targets": [{"model": "db-model"}]}}}),
			),
			test_resource(
				ConfigResourceKind::LlmPolicy,
				"cors",
				json!({"allowOrigins": ["https://example.com"]}),
			),
			test_resource(
				ConfigResourceKind::LlmPolicy,
				"apiKey",
				json!({"mode": "strict"}),
			),
			test_resource(
				ConfigResourceKind::LlmApiKey,
				"key_01",
				json!({"key": "agw_sk_test", "metadata": {"id": "key_01", "name": "test"}}),
			),
			test_resource(ConfigResourceKind::UiPolicy, "csrf", json!({})),
			test_resource(
				ConfigResourceKind::McpTarget,
				"db-target",
				json!({"name": "db-target", "stdio": {"cmd": "server"}}),
			),
			test_resource(
				ConfigResourceKind::McpPolicy,
				"cors",
				json!({"allowOrigins": ["https://example.com"]}),
			),
			test_resource(
				ConfigResourceKind::McpSettings,
				"default",
				json!({
					"statefulMode": "stateless",
					"prefixMode": "always",
					"failureMode": "failOpen"
				}),
			),
			test_resource(
				ConfigResourceKind::TrafficGateway,
				"public",
				json!({"name": "public", "port": 8080}),
			),
			test_resource(
				ConfigResourceKind::TrafficRoute,
				"later",
				json!({
					"name": "later",
					"gateways": ["public"],
					"backends": [{"host": "later.example:80"}]
				}),
			),
			test_resource(
				ConfigResourceKind::TrafficRoute,
				"earlier",
				json!({
					"name": "earlier",
					"gateways": ["public"],
					"backends": [{"host": "earlier.example:80"}]
				}),
			),
			test_resource(
				ConfigResourceKind::TrafficTcpRoute,
				"tcp",
				json!({
					"name": "tcp",
					"gateways": ["public"],
					"backends": [{"host": "tcp.example:80"}]
				}),
			),
		];

		let materialized = materialize_config(base, &resources).expect("materialize");
		let value: Value = crate::yamlviajson::from_str(&materialized).expect("parse materialized");

		assert_eq!(
			value.pointer("/llm/providers/0/name"),
			Some(&json!("openai"))
		);
		assert_eq!(
			value.pointer("/llm/models/0/name"),
			Some(&json!("file-model"))
		);
		assert_eq!(
			value.pointer("/llm/policies/apiKey/keys/0/key"),
			Some(&json!("agw_sk_test"))
		);
		assert_eq!(
			value.pointer("/llm/policies/cors/allowOrigins/0"),
			Some(&json!("https://example.com"))
		);
		assert_eq!(value.pointer("/ui/policies/csrf"), Some(&json!({})));
		assert_eq!(
			value.pointer("/llm/models/1/name"),
			Some(&json!("db-model"))
		);
		assert_eq!(value.pointer("/llm/models/1/id"), Some(&json!("model_01")));
		assert_eq!(
			value.pointer("/llm/virtualModels/0/name"),
			Some(&json!("router"))
		);
		assert_eq!(
			value.pointer("/mcp/targets/0/name"),
			Some(&json!("file-target"))
		);
		assert_eq!(
			value.pointer("/mcp/targets/1/name"),
			Some(&json!("db-target"))
		);
		assert_eq!(
			value.pointer("/mcp/policies/cors/allowOrigins/0"),
			Some(&json!("https://example.com"))
		);
		assert_eq!(
			value.pointer("/mcp/statefulMode"),
			Some(&json!("stateless"))
		);
		assert_eq!(value.pointer("/mcp/prefixMode"), Some(&json!("always")));
		assert_eq!(value.pointer("/mcp/failureMode"), Some(&json!("failOpen")));
		assert_eq!(value.pointer("/gateways/public/port"), Some(&json!(8080)));
		assert_eq!(
			value.pointer("/gateways/public/name"),
			None,
			"resource identity must not leak into the gateway config"
		);
		assert_eq!(value.pointer("/routes/0/name"), Some(&json!("earlier")));
		assert_eq!(value.pointer("/routes/1/name"), Some(&json!("later")));
		assert_eq!(value.pointer("/tcpRoutes/0/name"), Some(&json!("tcp")));
	}

	#[test]
	fn materialization_rejects_file_owned_identity_conflicts() {
		let base = r#"
llm:
  providers:
  - name: openai
    provider: openAI
"#;
		let resources = vec![test_resource(
			ConfigResourceKind::LlmProvider,
			"openai",
			json!({"name": "openai", "provider": "openAI"}),
		)];

		let err = materialize_config(base, &resources).expect_err("conflict should fail");
		assert!(
			err
				.to_string()
				.contains("conflicts with file-owned resource"),
			"unexpected error: {err}"
		);

		let base = "llm:\n  models: []\n  policies:\n    cors: {}\n";
		let resources = vec![test_resource(
			ConfigResourceKind::LlmPolicy,
			"cors",
			json!({}),
		)];
		let err = materialize_config(base, &resources).expect_err("policy conflict should fail");
		assert!(
			err
				.to_string()
				.contains("config resource llm.policy/cors conflicts with file-owned resource"),
			"unexpected error: {err}"
		);

		let base = r#"
gateways:
  public:
    port: 8080
routes:
- name: api
  gateways: [public]
"#;
		let gateway_resources = vec![test_resource(
			ConfigResourceKind::TrafficGateway,
			"public",
			json!({"name": "public", "port": 8081}),
		)];
		let err =
			materialize_config(base, &gateway_resources).expect_err("gateway conflict should fail");
		assert!(
			err
				.to_string()
				.contains("config resource traffic.gateway/public conflicts with file-owned resource"),
			"unexpected error: {err}"
		);

		let route_resources = vec![test_resource(
			ConfigResourceKind::TrafficRoute,
			"api",
			json!({"name": "api", "gateways": ["public"]}),
		)];
		let err = materialize_config(base, &route_resources).expect_err("route conflict should fail");
		assert!(
			err
				.to_string()
				.contains("config resource traffic.route/api conflicts with file-owned resource"),
			"unexpected error: {err}"
		);
	}

	#[test]
	fn file_resource_writes_update_the_config_shape() {
		let mut config = json!({
			"config": {
				"modelCatalog": [
					{"inline": {"openai": {"gpt-file": {"input": 1.0}}}},
					{"file": "/tmp/base-costs.json"},
					{"inline": {"openai": {"gpt-file": {"output": 2.0}}}}
				]
			},
			"llm": {
				"models": [{
					"name": "file-model",
					"provider": "openai",
					"params": {"model": "gpt-file"}
				}],
				"policies": {
					"apiKey": {
						"mode": "strict",
						"keys": [{"key": "agw_sk_test", "metadata": {"name": "test"}}]
					}
				}
			},
			"mcp": {
				"port": 3000,
				"targets": [{"name": "tools", "mcp": {"host": "http://tools"}}]
			},
			"gateways": {
				"public": {"port": 8080},
				"private": {"port": 8081}
			},
			"routes": [
				{
					"name": "first",
					"gateways": ["public"],
					"backends": [{"host": "example.com:80"}]
				},
				{
					"name": "second",
					"gateways": ["public"],
					"backends": [{"host": "second.example.com:80"}]
				}
			]
		});

		let renamed = prepare_resource(
			ConfigResourceKind::LlmModel,
			json!({
				"name": "renamed-model",
				"provider": "openai",
				"params": {"model": "gpt-renamed"}
			}),
		)
		.expect("prepare model");
		upsert_file_config_resource(&mut config, &renamed, Some("file-model"))
			.expect("rename file model");
		assert_eq!(
			config.pointer("/llm/models/0/name"),
			Some(&json!("renamed-model"))
		);

		let colliding_route = prepare_resource(
			ConfigResourceKind::TrafficRoute,
			json!({"name": "second", "gateways": ["public"]}),
		)
		.expect("prepare colliding route");
		let err = upsert_file_config_resource(&mut config, &colliding_route, Some("first"))
			.expect_err("list rename collision must fail");
		assert!(err.to_string().contains("already exists"));
		assert_eq!(config.pointer("/routes/0/name"), Some(&json!("first")));

		let colliding_gateway = prepare_resource(
			ConfigResourceKind::TrafficGateway,
			json!({"name": "private", "port": 9090}),
		)
		.expect("prepare colliding gateway");
		let err = upsert_file_config_resource(&mut config, &colliding_gateway, Some("public"))
			.expect_err("map rename collision must fail");
		assert!(err.to_string().contains("already exists"));
		assert_eq!(config.pointer("/gateways/public/port"), Some(&json!(8080)));
		assert_eq!(config.pointer("/gateways/private/port"), Some(&json!(8081)));

		let missing_key =
			prepare_file_api_key_update("missing".to_string(), json!({"key": "agw_missing"}))
				.expect("prepare API key update");
		let err = upsert_file_config_resource(&mut config, &missing_key, Some("missing"))
			.expect_err("missing API key update must fail");
		assert!(err.to_string().contains("not found"));
		assert_eq!(
			config
				.pointer("/llm/policies/apiKey/keys")
				.and_then(Value::as_array)
				.map(Vec::len),
			Some(1)
		);

		assert!(
			delete_file_config_resource(&mut config, ConfigResourceKind::McpTarget, "tools")
				.expect("delete MCP target")
		);
		assert_eq!(config.pointer("/mcp/targets"), Some(&json!([])));

		let catalog = prepare_resource(
			ConfigResourceKind::ModelCatalog,
			json!({"custom": {"anthropic": {"claude": {"input": 3.0}}}}),
		)
		.expect("prepare model catalog");
		upsert_file_config_resource(&mut config, &catalog, None).expect("update file model catalog");
		assert_eq!(
			config.pointer("/config/modelCatalog"),
			Some(&json!([
				{"file": "/tmp/base-costs.json"},
				{"inline": {"anthropic": {"claude": {"input": 3.0}}}}
			]))
		);

		assert!(
			prepare_resource(
				ConfigResourceKind::TrafficRoute,
				json!({
					"gateways": ["public"],
					"backends": [{"host": "unnamed.example.com:80"}]
				}),
			)
			.is_err(),
			"traffic route writes require a name"
		);
		let route = prepare_resource(
			ConfigResourceKind::TrafficRoute,
			json!({
				"name": "updated",
				"gateways": ["public"],
				"backends": [{"host": "updated.example.com:80"}]
			}),
		)
		.expect("prepare route");
		upsert_file_config_resource(&mut config, &route, Some("first")).expect("update route");
		assert_eq!(config.pointer("/routes/0/name"), Some(&json!("updated")));
		assert_eq!(
			config.pointer("/routes/0/backends/0/host"),
			Some(&json!("updated.example.com:80"))
		);
		assert!(
			delete_file_config_resource(&mut config, ConfigResourceKind::TrafficRoute, &route.id,)
				.expect("delete route")
		);
		assert_eq!(config.pointer("/routes/0/name"), Some(&json!("second")));
	}
}
