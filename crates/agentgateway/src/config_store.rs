use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{PgPool, Row, SqlitePool};
use tokio::sync::watch;

use crate::database::DatabasePool;
use crate::telemetry::log_store;

#[derive(Clone, Debug)]
pub struct ConfigResourceStore {
	pool: DatabasePool,
	change_tx: watch::Sender<()>,
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
		match &pool {
			DatabasePool::Sqlite(pool) => {
				sqlx::raw_sql(SQLITE_SCHEMA).execute(pool).await?;
			},
			DatabasePool::Postgres(pool) => {
				sqlx::raw_sql(POSTGRES_SCHEMA).execute(pool).await?;
			},
		};
		let (change_tx, _) = watch::channel(());
		Ok(Self { pool, change_tx })
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

	#[cfg_attr(not(feature = "ui"), allow(dead_code))]
	pub(crate) async fn upsert_prepared(
		&self,
		prepared: Vec<PreparedResource>,
	) -> anyhow::Result<ConfigResourcesResponse> {
		let resources = match &self.pool {
			DatabasePool::Sqlite(pool) => upsert_sqlite(pool, prepared).await?,
			DatabasePool::Postgres(pool) => upsert_postgres(pool, prepared).await?,
		};
		if !resources.is_empty() {
			self.notify_changed();
		}
		Ok(ConfigResourcesResponse { resources })
	}

	pub async fn delete(&self, kind: ConfigResourceKind, id: &str) -> anyhow::Result<()> {
		validate_id(id)?;
		let deleted = match &self.pool {
			DatabasePool::Sqlite(pool) => delete_sqlite(pool, kind, id).await?,
			DatabasePool::Postgres(pool) => delete_postgres(pool, kind, id).await?,
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

#[cfg_attr(not(feature = "ui"), allow(dead_code))]
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

#[cfg_attr(not(feature = "ui"), allow(dead_code))]
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

#[cfg_attr(not(feature = "ui"), allow(dead_code))]
pub(crate) fn prepare_policy_upsert(
	kind: ConfigResourceKind,
	id: String,
	value: Value,
) -> anyhow::Result<PreparedResource> {
	validate_id(&id)?;
	if !matches!(
		kind,
		ConfigResourceKind::LlmPolicy | ConfigResourceKind::UiPolicy
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

#[cfg_attr(not(feature = "ui"), allow(dead_code))]
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

#[cfg_attr(not(feature = "ui"), allow(dead_code))]
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
	let has_llm_policies = resources
		.iter()
		.any(|resource| resource.kind == ConfigResourceKind::LlmPolicy);
	if !has_llm_resources && !has_ui_resources {
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

fn append_llm_kind(
	llm: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
	field: &str,
) -> anyhow::Result<()> {
	let Some(db_resources) = non_empty_resources(resources, kind) else {
		return Ok(());
	};

	let values = llm.entry(field).or_insert_with(|| Value::Array(Vec::new()));
	let Some(values) = values.as_array_mut() else {
		anyhow::bail!("local config llm.{field} must be an array");
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

#[cfg_attr(not(any(feature = "ui", test)), allow(dead_code))]
fn prepare_resource(
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
		ConfigResourceKind::LlmPolicy | ConfigResourceKind::UiPolicy => {
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
		ConfigResourceKind::LlmProvider | ConfigResourceKind::LlmVirtualModel => {
			string_field(value, "name", || {
				format!("{kind} resources require value.name")
			})
		},
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
		ConfigResourceKind::LlmPolicy | ConfigResourceKind::UiPolicy => Err(
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
	tx.commit().await?;
	Ok(resources)
}

async fn delete_sqlite(
	pool: &SqlitePool,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<bool> {
	let now = Utc::now().to_rfc3339();
	let result = sqlx::query(
		"UPDATE agw_config_resources \
		 SET revision = revision + 1, updated_at = ?, deleted_at = ? \
		 WHERE kind = ? AND id = ? AND deleted_at IS NULL",
	)
	.bind(&now)
	.bind(&now)
	.bind(kind.as_str())
	.bind(id)
	.execute(pool)
	.await?;
	Ok(result.rows_affected() > 0)
}

async fn delete_postgres(
	pool: &PgPool,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<bool> {
	let now = Utc::now();
	let result = sqlx::query(
		"UPDATE agw_config_resources \
		 SET revision = revision + 1, updated_at = $1, deleted_at = $1 \
		 WHERE kind = $2 AND id = $3 AND deleted_at IS NULL",
	)
	.bind(now)
	.bind(kind.as_str())
	.bind(id)
	.execute(pool)
	.await?;
	Ok(result.rows_affected() > 0)
}

#[cfg_attr(not(feature = "ui"), allow(dead_code))]
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

#[cfg_attr(not(feature = "ui"), allow(dead_code))]
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

	#[test]
	fn materializes_llm_resources_into_base_config() {
		let base = r#"
llm:
  models:
  - name: file-model
    provider:
      openAI:
        model: gpt-4o-mini
ui:
  gateways: default
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
	}
}
