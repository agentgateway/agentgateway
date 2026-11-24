use std::fmt;
use std::str::FromStr;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::PgListener;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Row, Sqlite, SqlitePool, Transaction};
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::database::DatabasePool;

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
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub uid: Option<String>,
	pub value: Value,
	pub revision: i64,
	pub created_at: DateTime<Utc>,
	pub updated_at: DateTime<Utc>,
	/// Set on resources returned from an upsert.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub result: Option<WriteResult>,
}

/// What a write did to a stored resource.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WriteResult {
	/// The resource did not exist and was created.
	Created,
	/// The resource existed and was changed; its `revision` was incremented.
	Updated,
	/// The resource existed with the same value; nothing was written and its `revision` is
	/// unchanged.
	Unchanged,
}

impl WriteResult {
	fn of(written: bool, revision: i64) -> Self {
		match (written, revision) {
			(false, _) => Self::Unchanged,
			(true, 1) => Self::Created,
			(true, _) => Self::Updated,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResourcesResponse {
	pub resources: Vec<ConfigResource>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub generation: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
	pub resources: Vec<ConfigResource>,
	pub generation: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigResourceError {
	#[error("{0}")]
	InvalidRequest(String),
	#[error("{0}")]
	Conflict(String),
	#[error("{0}")]
	NotFound(String),
	#[error("config store changed during validation (expected generation {expected}, found {found})")]
	GenerationConflict { expected: i64, found: i64 },
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
	#[serde(rename = "llm.settings")]
	LlmSettings,
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
	#[serde(rename = "frontend.policy")]
	FrontendPolicy,
	#[serde(rename = "session.key")]
	SessionKey,
}

impl ConfigResourceKind {
	pub const ALL: [Self; 15] = [
		Self::ModelCatalog,
		Self::LlmProvider,
		Self::LlmModel,
		Self::LlmVirtualModel,
		Self::LlmApiKey,
		Self::LlmPolicy,
		Self::McpTarget,
		Self::McpPolicy,
		Self::LlmSettings,
		Self::McpSettings,
		Self::TrafficGateway,
		Self::TrafficRoute,
		Self::TrafficTcpRoute,
		Self::UiPolicy,
		Self::FrontendPolicy,
	];

	pub fn settings_fields(self) -> Option<(&'static str, &'static [&'static str])> {
		match self {
			Self::LlmSettings => Some(("llm", &["gateways", "port", "tls"])),
			Self::McpSettings => Some(("mcp", &MCP_SETTINGS_FIELDS)),
			_ => None,
		}
	}

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
			Self::LlmSettings => "llm.settings",
			Self::TrafficGateway => "traffic.gateway",
			Self::TrafficRoute => "traffic.route",
			Self::TrafficTcpRoute => "traffic.tcpRoute",
			Self::UiPolicy => "ui.policy",
			Self::FrontendPolicy => "frontend.policy",
			Self::SessionKey => "session.key",
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
			"llm.settings" => Ok(Self::LlmSettings),
			"traffic.gateway" => Ok(Self::TrafficGateway),
			"traffic.route" => Ok(Self::TrafficRoute),
			"traffic.tcpRoute" => Ok(Self::TrafficTcpRoute),
			"ui.policy" => Ok(Self::UiPolicy),
			"frontend.policy" => Ok(Self::FrontendPolicy),
			"session.key" => Ok(Self::SessionKey),
			_ => Err(ConfigResourceError::InvalidRequest(format!(
				"unsupported config resource kind: {kind}"
			))),
		}
	}
}

/// Resources of one kind to create or replace.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResourceUpsertRequest {
	/// The resources to write. Policy kinds cannot be written with this request; use
	/// `PUT /api/config/resources/{kind}/{id}` with the policy id. For other kinds, the server assigns
	/// each id: `default` for `modelCatalog`, `llm.settings`, and `mcp.settings`, a generated id for
	/// `llm.apiKey`, and otherwise the value's `name` (`id` or `name` for `llm.model`).
	#[serde(default)]
	pub resources: Vec<ConfigResourceUpsert>,
}

/// A resource to write.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResourceUpsert {
	/// The resource, in the configuration file format of its kind. The value is an object for
	/// non-policy kinds. For policy kinds, it has the policy's configuration shape, which can be an
	/// array or null for some IDs.
	pub value: Value,
}

/// Generates a random AES-256-GCM key, hex encoded.
pub fn generate_session_key() -> anyhow::Result<String> {
	Ok(hex::encode(crate::crypto::rand::bytes(32)?))
}

pub async fn setup(cfg: &crate::database::DatabaseConfig) -> anyhow::Result<ConfigResourceStore> {
	ConfigResourceStore::connect(&cfg.url, cfg.max_connections).await
}

impl ConfigResourceStore {
	async fn connect(url: &str, max_connections: Option<u32>) -> anyhow::Result<Self> {
		Self::from_pool(DatabasePool::connect_with_max_connections(url, max_connections).await?).await
	}

	pub async fn from_pool(pool: DatabasePool) -> anyhow::Result<Self> {
		let (change_tx, _) = watch::channel(());
		match &pool {
			DatabasePool::Sqlite(pool) => {
				setup_sqlite_schema(pool).await?;
				Ok(Self {
					pool: DatabasePool::Sqlite(pool.clone()),
					change_tx,
					notification_id: None,
				})
			},
			DatabasePool::Postgres(pool) => {
				setup_postgres_schema(pool).await?;
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

	/// Returns the stored session key, generating and storing one if none exists yet.
	pub async fn session_key(&self) -> anyhow::Result<String> {
		loop {
			let snapshot = self.snapshot(Some(ConfigResourceKind::SessionKey)).await?;
			if let Some(key) = snapshot
				.resources
				.first()
				.and_then(|r| r.value.get("key"))
				.and_then(Value::as_str)
			{
				return Ok(key.to_string());
			}
			let key = generate_session_key()?;
			let prepared = PreparedResource {
				kind: ConfigResourceKind::SessionKey,
				id: "default".to_string(),
				value: serde_json::json!({ "key": key }),
			};
			match self
				.upsert_prepared(vec![prepared], Some(snapshot.generation))
				.await
			{
				Ok(_) => {
					info!(
						"generated a session key and stored it in the database; we recommend setting SESSION_KEY instead"
					);
					return Ok(key);
				},
				// Another writer (possibly another replica generating its own key) won; re-read.
				Err(err)
					if matches!(
						err.downcast_ref(),
						Some(ConfigResourceError::GenerationConflict { .. })
					) => {},
				Err(err) => return Err(err),
			}
		}
	}

	pub async fn list(
		&self,
		kind: Option<ConfigResourceKind>,
	) -> anyhow::Result<Vec<ConfigResource>> {
		Ok(self.snapshot(kind).await?.resources)
	}

	/// Unfiltered snapshots exclude the internal `session.key` resource; it is only read through
	/// `session_key`.
	pub async fn snapshot(&self, kind: Option<ConfigResourceKind>) -> anyhow::Result<Snapshot> {
		match &self.pool {
			DatabasePool::Sqlite(pool) => list_sqlite(pool, kind).await,
			DatabasePool::Postgres(pool) => list_postgres(pool, kind).await,
		}
	}

	pub(crate) async fn upsert_prepared(
		&self,
		prepared: Vec<PreparedResource>,
		expected_generation: Option<i64>,
	) -> anyhow::Result<ConfigResourcesResponse> {
		if prepared.is_empty() {
			let generation = self.snapshot(None).await?.generation;
			if let Some(expected) = expected_generation
				&& expected != generation
			{
				return Err(
					ConfigResourceError::GenerationConflict {
						expected,
						found: generation,
					}
					.into(),
				);
			}
			return Ok(ConfigResourcesResponse {
				resources: Vec::new(),
				generation: Some(generation),
			});
		}
		let (resources, generation) = match &self.pool {
			DatabasePool::Sqlite(pool) => upsert_sqlite(pool, prepared, expected_generation).await?,
			DatabasePool::Postgres(pool) => {
				upsert_postgres(
					pool,
					prepared,
					expected_generation,
					self
						.notification_id
						.as_deref()
						.expect("postgres store has a notification ID"),
				)
				.await?
			},
		};
		if resources
			.iter()
			.any(|resource| resource.result != Some(WriteResult::Unchanged))
		{
			self.notify_changed();
		}
		Ok(ConfigResourcesResponse {
			resources,
			generation: Some(generation),
		})
	}

	pub(crate) async fn rename_prepared(
		&self,
		previous_kind: ConfigResourceKind,
		previous_id: &str,
		prepared: PreparedResource,
		expected_generation: Option<i64>,
	) -> anyhow::Result<ConfigResourcesResponse> {
		validate_id(previous_id)?;
		validate_id(&prepared.id)?;
		let (resource, generation) = match &self.pool {
			DatabasePool::Sqlite(pool) => {
				rename_sqlite(
					pool,
					previous_kind,
					previous_id,
					prepared,
					expected_generation,
				)
				.await?
			},
			DatabasePool::Postgres(pool) => {
				rename_postgres(
					pool,
					previous_kind,
					previous_id,
					prepared,
					expected_generation,
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
			generation: Some(generation),
		})
	}

	pub async fn delete(
		&self,
		kind: ConfigResourceKind,
		id: &str,
		expected_generation: Option<i64>,
	) -> anyhow::Result<i64> {
		validate_id(id)?;
		let deleted = match &self.pool {
			DatabasePool::Sqlite(pool) => delete_sqlite(pool, kind, id, expected_generation).await?,
			DatabasePool::Postgres(pool) => {
				delete_postgres(
					pool,
					kind,
					id,
					expected_generation,
					self
						.notification_id
						.as_deref()
						.expect("postgres store has a notification ID"),
				)
				.await?
			},
		};
		let Some(generation) = deleted else {
			return Err(
				ConfigResourceError::NotFound(format!("config resource not found: {kind}/{id}")).into(),
			);
		};
		self.notify_changed();
		Ok(generation)
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

pub(crate) const MCP_SETTINGS_FIELDS: [&str; 8] = [
	"gateways",
	"port",
	"statefulMode",
	"prefixMode",
	"failureMode",
	"sseKeepAlive",
	"dnsRebindingProtection",
	"server",
];

const API_KEY_METADATA_PREFIX: &str = "agentgateway.dev/";
const API_KEY_ID_METADATA: &str = "agentgateway.dev/id";
const API_KEY_CREATED_AT_METADATA: &str = "agentgateway.dev/createdAt";
const API_KEY_HINT_METADATA: &str = "agentgateway.dev/keyHint";

/// Older file keys have no stored ID, so expose their array position to the resource API.
fn file_api_key_id(value: &Value, index: usize) -> String {
	api_key_metadata(value)
		.and_then(|metadata| metadata.get(API_KEY_ID_METADATA))
		.and_then(Value::as_str)
		.filter(|id| !id.is_empty())
		.map(ToString::to_string)
		.unwrap_or_else(|| format!("@index:{index}"))
}

fn api_key_metadata(value: &Value) -> Option<&serde_json::Map<String, Value>> {
	value.get("metadata").and_then(Value::as_object)
}

pub(crate) fn file_api_key_created_at(config: &Value, id: &str) -> Option<i64> {
	crate::json::traverse(config, &["llm", "policies", "apiKey", "keys"])
		.and_then(Value::as_array)?
		.iter()
		.enumerate()
		.find(|(index, value)| file_api_key_id(value, *index) == id)
		.and_then(|(_, value)| api_key_created_at(value))
}

pub(crate) fn api_key_created_at(value: &Value) -> Option<i64> {
	api_key_metadata(value)?
		.get(API_KEY_CREATED_AT_METADATA)?
		.as_i64()
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
		ConfigResourceKind::LlmPolicy => Some(FileResourceCollection::Map(&["llm", "policies"])),
		ConfigResourceKind::UiPolicy => Some(FileResourceCollection::Map(&["ui", "policies"])),
		ConfigResourceKind::FrontendPolicy => Some(FileResourceCollection::Map(&["frontendPolicies"])),
		ConfigResourceKind::TrafficGateway => Some(FileResourceCollection::Map(&["gateways"])),
		ConfigResourceKind::TrafficRoute => Some(FileResourceCollection::List(&["routes"])),
		ConfigResourceKind::TrafficTcpRoute => Some(FileResourceCollection::List(&["tcpRoutes"])),
		ConfigResourceKind::ModelCatalog
		| ConfigResourceKind::LlmApiKey
		| ConfigResourceKind::McpTarget
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::McpSettings
		| ConfigResourceKind::LlmSettings
		| ConfigResourceKind::SessionKey => None,
	}
}

/// MCP target and policy resources in an endpoint have IDs of the form `<endpoint path>:<name>`.
fn split_mcp_endpoint_id(id: &str) -> (Option<&str>, &str) {
	match id.rsplit_once(':') {
		Some((endpoint, name)) if id.starts_with('/') => (Some(endpoint), name),
		_ => (None, id),
	}
}

/// Splits an MCP target or policy resource into its endpoint, its name, and its value as written
/// inside that endpoint.
fn from_endpoint_scoped(id: &str, mut value: Value) -> (Option<&str>, &str, Value) {
	let (endpoint, name) = split_mcp_endpoint_id(id);
	if let Some(object) = value.as_object_mut() {
		object.remove("endpoint");
	}
	(endpoint, name, value)
}

/// The endpoint a settings resource applies to, and the fields it may set. An `mcp.settings`
/// resource other than `default` is identified by its endpoint path.
pub(crate) fn settings_scope(
	kind: ConfigResourceKind,
	id: &str,
) -> (Option<&str>, &'static [&'static str]) {
	let (_, fields) = kind.settings_fields().expect("settings resource");
	if kind == ConfigResourceKind::McpSettings && id != "default" {
		// An endpoint shares the gateway binding of `mcp`.
		(Some(id), &fields[2..])
	} else {
		(None, fields)
	}
}

/// Returns the settings object `kind`/`id` is written to in a file-backed config.
pub(crate) fn file_settings<'a>(
	config: &'a Value,
	kind: ConfigResourceKind,
	id: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
	let (section, _) = kind.settings_fields().expect("settings resource");
	match settings_scope(kind, id).0 {
		Some(endpoint) => file_mcp_container(config, Some(endpoint))?.as_object(),
		None => config.get(section)?.as_object(),
	}
}

fn validate_mcp_endpoint_path(path: &str) -> anyhow::Result<()> {
	if !path.starts_with('/') || path.ends_with('/') {
		return Err(
			ConfigResourceError::InvalidRequest(format!(
				"mcp endpoint {path:?} must start with '/' and must not end with '/'"
			))
			.into(),
		);
	}
	Ok(())
}

fn file_mcp_endpoint_path(endpoint: &Value) -> Option<&str> {
	Some(endpoint.get("path")?.as_str()?.trim_end_matches('/'))
}

/// Returns the object holding MCP targets and policies: `mcp` itself, or the endpoint at `endpoint`.
fn file_mcp_container<'a>(config: &'a Value, endpoint: Option<&str>) -> Option<&'a Value> {
	let mcp = config.get("mcp")?;
	let Some(endpoint) = endpoint else {
		return Some(mcp);
	};
	mcp
		.get("endpoints")?
		.as_array()?
		.iter()
		.find(|g| file_mcp_endpoint_path(g) == Some(endpoint))
}

/// Like [`file_mcp_container`], creating the container if it does not exist.
fn ensure_file_mcp_container<'a>(
	config: &'a mut Value,
	endpoint: Option<&str>,
) -> anyhow::Result<&'a mut Value> {
	let Some(endpoint) = endpoint else {
		ensure_file_object(config, &["mcp"])?;
		return Ok(config.get_mut("mcp").expect("mcp was just created"));
	};
	let endpoints = ensure_file_array(config, &["mcp", "endpoints"])?;
	let index = match endpoints
		.iter()
		.position(|g| file_mcp_endpoint_path(g) == Some(endpoint))
	{
		Some(index) => index,
		None => {
			endpoints.push(serde_json::json!({ "path": endpoint }));
			endpoints.len() - 1
		},
	};
	Ok(&mut endpoints[index])
}

fn file_mcp_resources(config: &Value, kind: ConfigResourceKind) -> Vec<(String, Value)> {
	let endpoints = config
		.pointer("/mcp/endpoints")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|g| Some((Some(file_mcp_endpoint_path(g)?), g)));
	let containers = config
		.get("mcp")
		.map(|mcp| (None, mcp))
		.into_iter()
		.chain(endpoints);
	let qualify = |endpoint: Option<&str>, name: &str| match endpoint {
		Some(endpoint) => format!("{endpoint}:{name}"),
		None => name.to_string(),
	};
	let mut resources = Vec::new();
	for (endpoint, container) in containers {
		if kind == ConfigResourceKind::McpPolicy {
			let policies = container.get("policies").and_then(Value::as_object);
			resources.extend(
				policies
					.into_iter()
					.flatten()
					.map(|(id, value)| (qualify(endpoint, id), value.clone())),
			);
			continue;
		}
		let targets = container.get("targets").and_then(Value::as_array);
		for target in targets.into_iter().flatten() {
			let Ok(name) = resource_id(kind, target) else {
				continue;
			};
			let mut target = target.clone();
			if let (Some(endpoint), Some(object)) = (endpoint, target.as_object_mut()) {
				object.insert("endpoint".to_string(), Value::String(endpoint.to_string()));
			}
			resources.push((qualify(endpoint, &name), target));
		}
	}
	resources
}

fn file_mcp_resource<'a>(
	config: &'a Value,
	kind: ConfigResourceKind,
	id: &str,
) -> Option<&'a Value> {
	let (endpoint, name) = split_mcp_endpoint_id(id);
	let container = file_mcp_container(config, endpoint)?;
	if kind == ConfigResourceKind::McpPolicy {
		return container.get("policies")?.get(name);
	}
	container
		.get("targets")?
		.as_array()?
		.iter()
		.find(|value| resource_id(kind, value).is_ok_and(|current| current == name))
}

fn upsert_file_mcp_resource(
	config: &mut Value,
	prepared: &PreparedResource,
	previous_id: Option<&str>,
) -> anyhow::Result<()> {
	let kind = prepared.kind;
	let (endpoint, name, value) = from_endpoint_scoped(&prepared.id, prepared.value.clone());
	let (previous_endpoint, previous_name) =
		split_mcp_endpoint_id(previous_id.unwrap_or(&prepared.id));
	let local = PreparedResource {
		kind,
		id: name.to_string(),
		value,
	};
	let previous_name = if endpoint == previous_endpoint {
		previous_id.map(|_| previous_name)
	} else {
		// Moving to another endpoint: remove from the old endpoint, then insert as new.
		if file_mcp_resource(config, kind, &prepared.id).is_some() {
			return Err(
				ConfigResourceError::Conflict(format!(
					"config resource already exists: {kind}/{}",
					prepared.id
				))
				.into(),
			);
		}
		let previous_id = previous_id.expect("endpoints differ only when renaming");
		if !delete_file_mcp_resource(config, kind, previous_id)? {
			return Err(
				ConfigResourceError::NotFound(format!("config resource not found: {kind}/{previous_id}"))
					.into(),
			);
		}
		None
	};
	let container = ensure_file_mcp_container(config, endpoint)?;
	if kind == ConfigResourceKind::McpPolicy {
		upsert_file_map_resource(container, &["policies"], &local, previous_name)
	} else {
		upsert_file_list_resource(container, &["targets"], &local, previous_name)
	}
}

fn delete_file_mcp_resource(
	config: &mut Value,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<bool> {
	let (endpoint, name) = split_mcp_endpoint_id(id);
	if file_mcp_container(config, endpoint).is_none() {
		return Ok(false);
	}
	let container = ensure_file_mcp_container(config, endpoint)?;
	let deleted = if kind == ConfigResourceKind::McpPolicy {
		delete_file_map_resource(container, &["policies"], name)?
	} else {
		delete_file_list_resource(container, &["targets"], kind, name)?
	};
	if let Some(endpoint) = endpoint {
		prune_file_mcp_endpoint(config, endpoint);
	}
	Ok(deleted)
}

/// Drops the endpoint once it no longer configures anything beyond its path.
fn prune_file_mcp_endpoint(config: &mut Value, endpoint: &str) {
	let Some(endpoints) = config
		.pointer_mut("/mcp/endpoints")
		.and_then(Value::as_array_mut)
	else {
		return;
	};
	endpoints.retain(|g| {
		file_mcp_endpoint_path(g) != Some(endpoint)
			|| g.as_object().is_none_or(|g| {
				g.iter().any(|(key, value)| match key.as_str() {
					"path" => false,
					"targets" => value.as_array().is_none_or(|v| !v.is_empty()),
					"policies" => value.as_object().is_none_or(|v| !v.is_empty()),
					_ => true,
				})
			})
	});
}

/// Lists the resources defined in a file-backed config. API keys are listed as `llm.apiKey`
/// resources rather than inside `llm.policy/apiKey`; settings list the fields set in the file.
pub(crate) fn file_config_resources(
	config: &Value,
	kind: Option<ConfigResourceKind>,
) -> Vec<PreparedResource> {
	let array = |path: &[&str]| {
		crate::json::traverse(config, path)
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
	};
	let kinds = kind.map_or(ConfigResourceKind::ALL.to_vec(), |kind| vec![kind]);
	let mut resources = Vec::new();
	for kind in kinds {
		let items: Vec<(String, Value)> = match file_resource_collection(kind) {
			Some(FileResourceCollection::List(path)) => array(path)
				.filter_map(|value| Some((resource_id(kind, value).ok()?, value.clone())))
				.collect(),
			Some(FileResourceCollection::Map(path)) => crate::json::traverse(config, path)
				.and_then(Value::as_object)
				.into_iter()
				.flatten()
				.map(|(id, value)| (id.clone(), value.clone()))
				.collect(),
			None
				if matches!(
					kind,
					ConfigResourceKind::McpTarget | ConfigResourceKind::McpPolicy
				) =>
			{
				file_mcp_resources(config, kind)
			},
			None if kind == ConfigResourceKind::LlmApiKey => {
				array(&["llm", "policies", "apiKey", "keys"])
					.enumerate()
					.map(|(index, value)| (file_api_key_id(value, index), value.clone()))
					.collect()
			},
			None => kind
				.settings_fields()
				.into_iter()
				.flat_map(|(section, _)| {
					let endpoints = array(&["mcp", "endpoints"])
						.filter(|_| kind == ConfigResourceKind::McpSettings)
						.filter_map(|value| Some((file_mcp_endpoint_path(value)?, value)));
					config
						.get(section)
						.map(|value| ("default", value))
						.into_iter()
						.chain(endpoints)
				})
				.filter_map(|(id, value)| {
					let (endpoint, fields) = settings_scope(kind, id);
					let mut settings: serde_json::Map<_, _> = fields
						.iter()
						.filter_map(|&field| Some((field.to_string(), value.get(field)?.clone())))
						.collect();
					if settings.is_empty() {
						return None;
					}
					if let Some(endpoint) = endpoint {
						settings.insert("endpoint".to_string(), Value::String(endpoint.to_string()));
					}
					Some((id.to_string(), Value::Object(settings)))
				})
				.collect(),
		};
		for (id, mut value) in items {
			if kind == ConfigResourceKind::LlmPolicy
				&& id == "apiKey"
				&& let Some(policy) = value.as_object_mut()
			{
				policy.remove("keys");
			}
			resources.push(PreparedResource { kind, id, value });
		}
	}
	resources
}

pub(crate) fn file_config_resource<'a>(
	config: &'a Value,
	kind: ConfigResourceKind,
	id: &str,
) -> Option<&'a Value> {
	if kind == ConfigResourceKind::LlmApiKey {
		return config
			.pointer("/llm/policies/apiKey/keys")?
			.as_array()?
			.iter()
			.enumerate()
			.find(|(index, value)| file_api_key_id(value, *index) == id)
			.map(|(_, value)| value);
	}
	if matches!(
		kind,
		ConfigResourceKind::McpTarget | ConfigResourceKind::McpPolicy
	) {
		return file_mcp_resource(config, kind, id);
	}
	match file_resource_collection(kind)? {
		FileResourceCollection::Map(path) => crate::json::traverse(config, path)?.get(id),
		FileResourceCollection::List(path) => crate::json::traverse(config, path)?
			.as_array()?
			.iter()
			.find(|value| resource_id(kind, value).is_ok_and(|current| current == id)),
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
		ConfigResourceKind::McpTarget | ConfigResourceKind::McpPolicy => {
			upsert_file_mcp_resource(config, prepared, previous_id)
		},
		ConfigResourceKind::McpSettings | ConfigResourceKind::LlmSettings => {
			upsert_file_settings(config, prepared)
		},
		ConfigResourceKind::SessionKey => Err(session_key_file_error()),
		ConfigResourceKind::LlmProvider
		| ConfigResourceKind::LlmModel
		| ConfigResourceKind::LlmVirtualModel
		| ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::TrafficGateway
		| ConfigResourceKind::TrafficRoute
		| ConfigResourceKind::TrafficTcpRoute
		| ConfigResourceKind::UiPolicy
		| ConfigResourceKind::FrontendPolicy => unreachable!("direct file resources handled above"),
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
		ConfigResourceKind::McpTarget | ConfigResourceKind::McpPolicy => {
			delete_file_mcp_resource(config, kind, id)
		},
		ConfigResourceKind::McpSettings | ConfigResourceKind::LlmSettings => {
			delete_file_settings(config, kind, id)
		},
		ConfigResourceKind::SessionKey => Err(session_key_file_error()),
		ConfigResourceKind::LlmProvider
		| ConfigResourceKind::LlmModel
		| ConfigResourceKind::LlmVirtualModel
		| ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::TrafficGateway
		| ConfigResourceKind::TrafficRoute
		| ConfigResourceKind::TrafficTcpRoute
		| ConfigResourceKind::UiPolicy
		| ConfigResourceKind::FrontendPolicy => unreachable!("direct file resources handled above"),
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
					| ConfigResourceKind::FrontendPolicy
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
		value
			.as_object_mut()
			.ok_or_else(|| anyhow::anyhow!("llm.policy/apiKey must be an object"))?
			.insert(
				"keys".to_string(),
				existing_keys.unwrap_or_else(|| Value::Array(Vec::new())),
			);
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

/// Projects the singleton surface settings resource onto its top-level fields.
fn session_key_file_error() -> anyhow::Error {
	ConfigResourceError::InvalidRequest(
		"session.key is stored in the database; use config.session.key in file mode".to_string(),
	)
	.into()
}

fn upsert_file_settings(config: &mut Value, prepared: &PreparedResource) -> anyhow::Result<()> {
	let PreparedResource { kind, id, value } = prepared;
	let (section, _) = kind.settings_fields().expect("settings resource");
	let (endpoint, fields) = settings_scope(*kind, id);
	let value = value
		.as_object()
		.ok_or_else(|| anyhow::anyhow!("{kind}/{id} must be an object"))?;
	let settings = match endpoint {
		Some(endpoint) => ensure_file_mcp_container(config, Some(endpoint))?
			.as_object_mut()
			.ok_or_else(|| anyhow::anyhow!("local config mcp.endpoints entries must be objects"))?,
		None => ensure_file_object(config, &[section])?,
	};
	for &field in fields {
		if let Some(value) = value.get(field) {
			settings.insert(field.to_string(), value.clone());
		} else {
			settings.remove(field);
		}
	}
	if let Some(endpoint) = endpoint {
		prune_file_mcp_endpoint(config, endpoint);
	}
	Ok(())
}

fn delete_file_settings(
	config: &mut Value,
	kind: ConfigResourceKind,
	id: &str,
) -> anyhow::Result<bool> {
	let (section, _) = kind.settings_fields().expect("settings resource");
	let (endpoint, fields) = settings_scope(kind, id);
	let settings = match endpoint {
		Some(endpoint) if file_mcp_container(config, Some(endpoint)).is_some() => {
			ensure_file_mcp_container(config, Some(endpoint))?
		},
		Some(_) => return Ok(false),
		None => match config.get_mut(section) {
			Some(settings) => settings,
			None => return Ok(false),
		},
	};
	let Some(settings) = settings.as_object_mut() else {
		return Ok(false);
	};
	let mut deleted = false;
	for &field in fields {
		deleted |= settings.remove(field).is_some();
	}
	if let Some(endpoint) = endpoint {
		prune_file_mcp_endpoint(config, endpoint);
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
	created_at: Option<i64>,
	existing: Option<&Value>,
) -> anyhow::Result<PreparedResource> {
	validate_id(&id)?;
	validate_api_key_update_metadata(&mut value, &id, created_at, existing)?;
	if !id.starts_with("@index:") {
		set_api_key_managed_metadata(&mut value, id.clone(), created_at)?;
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
	created_at: Option<i64>,
	existing: Option<&Value>,
) -> anyhow::Result<PreparedResource> {
	validate_id(&id)?;
	validate_api_key_update_metadata(&mut value, &id, created_at, existing)?;
	set_api_key_managed_metadata(&mut value, id.clone(), created_at)?;
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
		ConfigResourceKind::LlmPolicy
			| ConfigResourceKind::McpPolicy
			| ConfigResourceKind::UiPolicy
			| ConfigResourceKind::FrontendPolicy
	) {
		return Err(
			ConfigResourceError::InvalidRequest(format!("{kind} is not a policy resource")).into(),
		);
	}
	if kind == ConfigResourceKind::McpPolicy
		&& let (Some(endpoint), _) = split_mcp_endpoint_id(&id)
	{
		validate_mcp_endpoint_path(endpoint)?;
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

pub fn merge_model_catalog_sources(
	resources: &[ConfigResource],
	mut configured: Vec<crate::ModelCatalogSource>,
) -> anyhow::Result<Vec<crate::ModelCatalogSource>> {
	let Some(resource) = resources
		.iter()
		.find(|resource| resource.kind == ConfigResourceKind::ModelCatalog)
	else {
		return Ok(configured);
	};
	let value = resource
		.value
		.as_object()
		.ok_or_else(|| anyhow::anyhow!("modelCatalog resource must be an object"))?;
	let mut sources = Vec::new();
	if let Some(base) = value.get("base") {
		let mut inline: crate::llm::catalog::Catalog = serde_json::from_value(base.clone())?;
		if inline.metadata.is_none() {
			inline.metadata = Some(crate::llm::catalog::CatalogMetadata {
				source: None,
				// Legacy base catalogs predate generatedAt. Treat them as older than every
				// timestamped catalog rather than guessing from the resource timestamp,
				// which may also reflect an unrelated custom-overlay edit.
				generated_at: DateTime::<Utc>::UNIX_EPOCH,
				unknown: Default::default(),
			});
		}
		sources.push(crate::ModelCatalogSource::InlineCatalog { inline });
	}
	if let Some(custom) = value.get("custom") {
		sources.push(crate::ModelCatalogSource::InlineCatalog {
			inline: serde_json::from_value(custom.clone())?,
		});
	}
	sources.append(&mut configured);
	Ok(sources)
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
			uid: None,
			value: prepared.value.clone(),
			revision: 1,
			created_at: Utc::now(),
			updated_at: Utc::now(),
			result: None,
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
	materialize_runtime_config(base.as_str(), resources)
}

/// Materialize the config to load. Environment variables are expanded in the file config only; UI
/// managed values are kept literal, except for explicit provider API key environment variables.
pub(crate) fn materialize_runtime_config(
	base: &str,
	mut resources: Vec<ConfigResource>,
) -> anyhow::Result<String> {
	let base = crate::types::local::expand_env(base)?;
	for resource in &mut resources {
		if !matches!(
			resource.kind,
			ConfigResourceKind::LlmProvider | ConfigResourceKind::LlmModel
		) {
			continue;
		}
		// API keys support environment variables, such as `$OPENAI_API_KEY` or `{file: $HOME/key}`.
		let key = match resource.value.pointer_mut("/params/apiKey") {
			Some(Value::Object(key)) => key.get_mut("file"),
			key => key,
		};
		if let Some(Value::String(key)) = key {
			*key = shellexpand::full(key)?.into_owned();
		}
	}
	materialize_config(&base, &resources)
}

pub(crate) fn materialize_config(
	base: &str,
	resources: &[ConfigResource],
) -> anyhow::Result<String> {
	let mut config: Value = crate::yaml::from_str(base)?;
	overlay_config_resources(&mut config, resources)?;
	crate::yaml::to_string(&config)
}

fn overlay_config_resources(
	config: &mut Value,
	resources: &[ConfigResource],
) -> anyhow::Result<()> {
	let has_llm_resources = resources.iter().any(|resource| {
		matches!(
			resource.kind,
			ConfigResourceKind::LlmSettings
				| ConfigResourceKind::LlmProvider
				| ConfigResourceKind::LlmModel
				| ConfigResourceKind::LlmVirtualModel
				| ConfigResourceKind::LlmApiKey
				| ConfigResourceKind::LlmPolicy
		)
	});
	let has_ui_resources = resources
		.iter()
		.any(|resource| resource.kind == ConfigResourceKind::UiPolicy);
	let has_frontend_resources = resources
		.iter()
		.any(|resource| resource.kind == ConfigResourceKind::FrontendPolicy);
	let has_model_catalog = resources
		.iter()
		.any(|resource| resource.kind == ConfigResourceKind::ModelCatalog);
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
	if !has_llm_resources
		&& !has_mcp_resources
		&& !has_traffic_resources
		&& !has_ui_resources
		&& !has_frontend_resources
		&& !has_model_catalog
	{
		return Ok(());
	}

	if has_model_catalog {
		let configured = config
			.pointer("/config/modelCatalog")
			.cloned()
			.map(serde_json::from_value)
			.transpose()?
			.unwrap_or_default();
		let sources = merge_model_catalog_sources(resources, configured)?;
		*ensure_file_array(config, &["config", "modelCatalog"])? = serde_json::to_value(sources)?
			.as_array()
			.expect("model catalog sources serialize as an array")
			.clone();
	}

	let Some(root) = config.as_object_mut() else {
		anyhow::bail!("local config root must be a JSON object");
	};
	if has_llm_resources {
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

		append_settings(llm, resources, ConfigResourceKind::LlmSettings)?;
		append_policy_kind(
			llm,
			"policies",
			resources,
			ConfigResourceKind::LlmPolicy,
			"llm.policies",
		)?;
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

		append_settings(mcp, resources, ConfigResourceKind::McpSettings)?;
		let (scoped, top_level): (Vec<_>, Vec<_>) = resources
			.iter()
			.filter(|resource| {
				matches!(
					resource.kind,
					ConfigResourceKind::McpTarget | ConfigResourceKind::McpPolicy
				)
			})
			.cloned()
			.partition(|resource| split_mcp_endpoint_id(&resource.id).0.is_some());
		append_policy_kind(
			mcp,
			"policies",
			&top_level,
			ConfigResourceKind::McpPolicy,
			"mcp.policies",
		)?;
		append_list_kind(
			mcp,
			&top_level,
			ConfigResourceKind::McpTarget,
			"targets",
			"mcp",
		)?;
		append_mcp_endpoints(mcp, scoped)?;
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
		append_policy_kind(
			ui,
			"policies",
			resources,
			ConfigResourceKind::UiPolicy,
			"ui.policies",
		)?;
	}
	if has_frontend_resources {
		append_policy_kind(
			root,
			"frontendPolicies",
			resources,
			ConfigResourceKind::FrontendPolicy,
			"frontendPolicies",
		)?;
	}
	Ok(())
}

fn append_policy_kind(
	section: &mut serde_json::Map<String, Value>,
	key: &str,
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
	path: &str,
) -> anyhow::Result<()> {
	let Some(db_resources) = non_empty_resources(resources, kind) else {
		return Ok(());
	};
	let policies = section
		.entry(key)
		.or_insert_with(|| Value::Object(serde_json::Map::new()));
	if policies.is_null() {
		*policies = Value::Object(serde_json::Map::new());
	}
	let policies = policies.as_object_mut().ok_or_else(|| {
		ConfigResourceError::Conflict(format!(
			"DB-backed {kind} resources require {path} to be an object in the file config"
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

/// Appends scoped MCP target and policy resources to their `mcp.endpoints` entry, creating it if needed.
fn append_mcp_endpoints(
	mcp: &mut serde_json::Map<String, Value>,
	resources: Vec<ConfigResource>,
) -> anyhow::Result<()> {
	let mut by_endpoint: Vec<(String, Vec<ConfigResource>)> = Vec::new();
	for mut resource in resources {
		let (Some(endpoint), name, value) =
			from_endpoint_scoped(&resource.id, std::mem::take(&mut resource.value))
		else {
			continue;
		};
		let (endpoint, name) = (endpoint.to_string(), name.to_string());
		resource.id = name;
		resource.value = value;
		match by_endpoint.iter_mut().find(|(path, _)| *path == endpoint) {
			Some((_, resources)) => resources.push(resource),
			None => by_endpoint.push((endpoint, vec![resource])),
		}
	}
	for (path, resources) in by_endpoint {
		let endpoint = mcp_endpoint_entry(mcp, &path)?;
		append_policy_kind(
			endpoint,
			"policies",
			&resources,
			ConfigResourceKind::McpPolicy,
			"mcp.endpoints[].policies",
		)?;
		append_list_kind(
			endpoint,
			&resources,
			ConfigResourceKind::McpTarget,
			"targets",
			"mcp.endpoints[]",
		)?;
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

fn append_settings(
	settings: &mut serde_json::Map<String, Value>,
	resources: &[ConfigResource],
	kind: ConfigResourceKind,
) -> anyhow::Result<()> {
	for resource in resources.iter().filter(|resource| resource.kind == kind) {
		let id = &resource.id;
		let (endpoint, fields) = settings_scope(kind, id);
		let value = resource.value.as_object().ok_or_else(|| {
			ConfigResourceError::InvalidRequest(format!("{kind}/{id} must be an object"))
		})?;
		let settings = match endpoint {
			Some(endpoint) => mcp_endpoint_entry(settings, endpoint)?,
			None => &mut *settings,
		};
		for (field, value) in value {
			if endpoint.is_some() && field == "endpoint" {
				continue;
			}
			if !fields.contains(&field.as_str()) {
				return Err(
					ConfigResourceError::InvalidRequest(format!(
						"{kind}/{id} contains unsupported field: {field}"
					))
					.into(),
				);
			}
			if settings.contains_key(field) {
				return Err(
					ConfigResourceError::Conflict(format!(
						"config resource {kind}/{id} field {field} conflicts with file-owned configuration"
					))
					.into(),
				);
			}
			settings.insert(field.clone(), value.clone());
		}
	}
	Ok(())
}

/// Returns the entry for `path` in `mcp.endpoints`, creating it if it does not exist.
fn mcp_endpoint_entry<'a>(
	mcp: &'a mut serde_json::Map<String, Value>,
	path: &str,
) -> anyhow::Result<&'a mut serde_json::Map<String, Value>> {
	let Some(endpoints) = mcp
		.entry("endpoints")
		.or_insert_with(|| Value::Array(Vec::new()))
		.as_array_mut()
	else {
		anyhow::bail!("local config mcp.endpoints must be an array");
	};
	let index = match endpoints
		.iter()
		.position(|g| file_mcp_endpoint_path(g) == Some(path))
	{
		Some(index) => index,
		None => {
			endpoints.push(serde_json::json!({ "path": path }));
			endpoints.len() - 1
		},
	};
	endpoints[index]
		.as_object_mut()
		.ok_or_else(|| anyhow::anyhow!("local config mcp.endpoints entries must be objects"))
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
			validate_api_key_metadata(&value)?;
			let id = uuid::Uuid::new_v4().to_string();
			set_api_key_managed_metadata(&mut value, id.clone(), Some(Utc::now().timestamp()))?;
			id
		},
		ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::UiPolicy
		| ConfigResourceKind::FrontendPolicy => {
			return Err(
				ConfigResourceError::InvalidRequest(format!("{kind} resources require an item ID")).into(),
			);
		},
		ConfigResourceKind::SessionKey => {
			return Err(
				ConfigResourceError::InvalidRequest(
					"session.key is generated by agentgateway and cannot be written".to_string(),
				)
				.into(),
			);
		},
		_ => resource_id(kind, &value)?,
	};
	Ok(PreparedResource { kind, id, value })
}

fn resource_id(kind: ConfigResourceKind, value: &Value) -> anyhow::Result<String> {
	match kind {
		ConfigResourceKind::ModelCatalog | ConfigResourceKind::SessionKey => Ok("default".to_string()),
		ConfigResourceKind::LlmSettings => Ok("default".to_string()),
		ConfigResourceKind::McpSettings => match value.get("endpoint").and_then(Value::as_str) {
			Some(endpoint) => {
				validate_mcp_endpoint_path(endpoint)?;
				Ok(endpoint.to_string())
			},
			None => Ok("default".to_string()),
		},
		ConfigResourceKind::McpTarget => {
			let name = string_field(value, "name", || {
				format!("{kind} resources require value.name")
			})?;
			if name.contains(':') {
				return Err(
					ConfigResourceError::InvalidRequest(format!("{kind} name {name:?} must not contain ':'"))
						.into(),
				);
			}
			match value.get("endpoint").and_then(Value::as_str) {
				Some(endpoint) => {
					validate_mcp_endpoint_path(endpoint)?;
					Ok(format!("{endpoint}:{name}"))
				},
				None => Ok(name),
			}
		},
		ConfigResourceKind::LlmProvider
		| ConfigResourceKind::LlmVirtualModel
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
			.get("metadata")
			.and_then(Value::as_object)
			.and_then(|metadata| metadata.get(API_KEY_ID_METADATA))
			.and_then(Value::as_str)
			.map(ToString::to_string)
			.ok_or_else(|| {
				ConfigResourceError::InvalidRequest(format!(
					"llm.apiKey resources require value.metadata.{API_KEY_ID_METADATA}"
				))
				.into()
			}),
		ConfigResourceKind::LlmPolicy
		| ConfigResourceKind::McpPolicy
		| ConfigResourceKind::UiPolicy
		| ConfigResourceKind::FrontendPolicy => Err(
			ConfigResourceError::InvalidRequest(format!("{kind} resources require an item ID")).into(),
		),
	}
}

fn validate_api_key_metadata(value: &Value) -> anyhow::Result<()> {
	if let Some(field) = value
		.get("metadata")
		.and_then(Value::as_object)
		.and_then(|metadata| {
			metadata
				.keys()
				// Key hint is not really required to be trusted so we can allow that
				.find(|field| field.starts_with(API_KEY_METADATA_PREFIX) && *field != API_KEY_HINT_METADATA)
		}) {
		return Err(
			ConfigResourceError::InvalidRequest(format!(
				"llm.apiKey metadata field {field} uses the reserved agentgateway.dev/ prefix"
			))
			.into(),
		);
	}
	Ok(())
}

/// Updates may echo back server-managed metadata from a previous read, as long as it is unchanged.
/// Echoed fields are removed so the server-owned values are re-applied.
fn validate_api_key_update_metadata(
	value: &mut Value,
	id: &str,
	created_at: Option<i64>,
	existing: Option<&Value>,
) -> anyhow::Result<()> {
	let Some(metadata) = value.get_mut("metadata").and_then(Value::as_object_mut) else {
		return Ok(());
	};
	let reserved = metadata
		.keys()
		.filter(|field| field.starts_with(API_KEY_METADATA_PREFIX) && *field != API_KEY_HINT_METADATA)
		.cloned()
		.collect::<Vec<_>>();
	for field in reserved {
		let current = match field.as_str() {
			API_KEY_ID_METADATA if !id.starts_with("@index:") => Some(Value::String(id.to_string())),
			API_KEY_CREATED_AT_METADATA => created_at.map(Value::from),
			_ => existing
				.and_then(api_key_metadata)
				.and_then(|existing| existing.get(&field))
				.cloned(),
		};
		if metadata.get(&field) != current.as_ref() {
			return Err(
				ConfigResourceError::InvalidRequest(format!(
					"llm.apiKey metadata field {field} cannot be changed"
				))
				.into(),
			);
		}
		metadata.remove(&field);
	}
	Ok(())
}

fn set_api_key_managed_metadata(
	value: &mut Value,
	id: String,
	created_at: Option<i64>,
) -> anyhow::Result<()> {
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
	metadata.insert(API_KEY_ID_METADATA.to_string(), Value::String(id));
	if let Some(created_at) = created_at {
		metadata.insert(
			API_KEY_CREATED_AT_METADATA.to_string(),
			Value::Number(created_at.into()),
		);
	}
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
) -> anyhow::Result<Snapshot> {
	let mut tx = pool.begin().await?;
	let generation: i64 = sqlx::query_scalar("SELECT generation FROM agw_config_meta WHERE id = 1")
		.fetch_one(&mut *tx)
		.await?;
	let rows = if let Some(kind) = kind {
		sqlx::query(
			"SELECT kind, id, uid, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL AND kind = ? \
			 ORDER BY id",
		)
		.bind(kind.as_str())
		.fetch_all(&mut *tx)
		.await?
	} else {
		sqlx::query(
			"SELECT kind, id, uid, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL AND kind <> 'session.key' \
			 ORDER BY kind, id",
		)
		.fetch_all(&mut *tx)
		.await?
	};
	tx.commit().await?;
	let resources = rows
		.into_iter()
		.map(sqlite_row_to_resource)
		.collect::<anyhow::Result<Vec<_>>>()?;
	Ok(Snapshot {
		resources,
		generation,
	})
}

async fn list_postgres(
	pool: &PgPool,
	kind: Option<ConfigResourceKind>,
) -> anyhow::Result<Snapshot> {
	let mut tx = pool
		.begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
		.await?;
	let generation: i64 = sqlx::query_scalar("SELECT generation FROM agw_config_meta WHERE id = 1")
		.fetch_one(&mut *tx)
		.await?;
	let rows = if let Some(kind) = kind {
		sqlx::query(
			"SELECT kind, id, uid, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL AND kind = $1 \
			 ORDER BY id",
		)
		.bind(kind.as_str())
		.fetch_all(&mut *tx)
		.await?
	} else {
		sqlx::query(
			"SELECT kind, id, uid, value_json, revision, created_at, updated_at \
			 FROM agw_config_resources \
			 WHERE deleted_at IS NULL AND kind <> 'session.key' \
			 ORDER BY kind, id",
		)
		.fetch_all(&mut *tx)
		.await?
	};
	tx.commit().await?;
	let resources = rows
		.into_iter()
		.map(postgres_row_to_resource)
		.collect::<anyhow::Result<Vec<_>>>()?;
	Ok(Snapshot {
		resources,
		generation,
	})
}

// A deferred SQLite lock upgrade can fail without waiting for busy_timeout.
const SQLITE_BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

async fn bump_generation_sqlite(
	tx: &mut Transaction<'_, Sqlite>,
	expected: Option<i64>,
) -> anyhow::Result<i64> {
	let bumped: Option<i64> = sqlx::query_scalar(
		"UPDATE agw_config_meta SET generation = generation + 1 \
		 WHERE id = 1 AND (? IS NULL OR generation = ?) \
		 RETURNING generation",
	)
	.bind(expected)
	.bind(expected)
	.fetch_optional(&mut **tx)
	.await?;
	match bumped {
		Some(generation) => Ok(generation),
		None => {
			let found: i64 = sqlx::query_scalar("SELECT generation FROM agw_config_meta WHERE id = 1")
				.fetch_one(&mut **tx)
				.await?;
			Err(
				ConfigResourceError::GenerationConflict {
					expected: expected.unwrap_or(found),
					found,
				}
				.into(),
			)
		},
	}
}

async fn bump_generation_postgres(
	tx: &mut Transaction<'_, Postgres>,
	expected: Option<i64>,
) -> anyhow::Result<i64> {
	let bumped: Option<i64> = sqlx::query_scalar(
		"UPDATE agw_config_meta SET generation = generation + 1 \
		 WHERE id = 1 AND ($1::BIGINT IS NULL OR generation = $1) \
		 RETURNING generation",
	)
	.bind(expected)
	.fetch_optional(&mut **tx)
	.await?;
	match bumped {
		Some(generation) => Ok(generation),
		None => {
			let found: i64 = sqlx::query_scalar("SELECT generation FROM agw_config_meta WHERE id = 1")
				.fetch_one(&mut **tx)
				.await?;
			Err(
				ConfigResourceError::GenerationConflict {
					expected: expected.unwrap_or(found),
					found,
				}
				.into(),
			)
		},
	}
}

async fn upsert_sqlite(
	pool: &SqlitePool,
	prepared: Vec<PreparedResource>,
	expected_generation: Option<i64>,
) -> anyhow::Result<(Vec<ConfigResource>, i64)> {
	let mut tx = pool.begin_with(SQLITE_BEGIN_IMMEDIATE).await?;
	let generation = bump_generation_sqlite(&mut tx, expected_generation).await?;
	let mut changed = Vec::with_capacity(prepared.len());
	for PreparedResource {
		kind,
		id,
		mut value,
	} in prepared
	{
		validate_id(&id)?;
		// Sorted keys make the text comparison below independent of key order.
		value.sort_all_objects();
		let now = Utc::now().to_rfc3339();
		// Unchanged resources are left alone so their revision only moves on real changes.
		let written = sqlx::query(
			"INSERT INTO agw_config_resources \
			 (kind, id, value_json, revision, created_at, updated_at, deleted_at) \
			 VALUES (?, ?, ?, 1, ?, ?, NULL) \
			 ON CONFLICT(kind, id) DO UPDATE SET \
				uid = CASE WHEN agw_config_resources.deleted_at IS NOT NULL THEN excluded.uid \
					ELSE agw_config_resources.uid END, \
				value_json = excluded.value_json, \
				revision = agw_config_resources.revision + 1, \
				updated_at = excluded.updated_at, \
				deleted_at = NULL \
			 WHERE agw_config_resources.deleted_at IS NOT NULL \
				OR agw_config_resources.value_json IS NOT excluded.value_json",
		)
		.bind(kind.as_str())
		.bind(&id)
		.bind(serde_json::to_string(&value)?)
		.bind(&now)
		.bind(&now)
		.execute(&mut *tx)
		.await?
		.rows_affected()
			> 0;
		changed.push((kind, id, written));
	}

	let mut resources = Vec::with_capacity(changed.len());
	for (kind, id, written) in changed {
		if let Some(mut resource) = fetch_sqlite_resource(&mut tx, kind, &id).await? {
			resource.result = Some(WriteResult::of(written, resource.revision));
			resources.push(resource);
		}
	}
	tx.commit().await?;
	Ok((resources, generation))
}

async fn upsert_postgres(
	pool: &PgPool,
	prepared: Vec<PreparedResource>,
	expected_generation: Option<i64>,
	notification_id: &str,
) -> anyhow::Result<(Vec<ConfigResource>, i64)> {
	let mut tx = pool.begin().await?;
	let generation = bump_generation_postgres(&mut tx, expected_generation).await?;
	let mut changed = Vec::with_capacity(prepared.len());
	let mut wrote = false;
	for PreparedResource { kind, id, value } in prepared {
		validate_id(&id)?;
		let now = Utc::now();
		// Unchanged resources are left alone so their revision only moves on real changes.
		let result = sqlx::query(
			"INSERT INTO agw_config_resources \
			 (kind, id, value_json, revision, created_at, updated_at, deleted_at) \
			 VALUES ($1, $2, $3, 1, $4, $4, NULL) \
			 ON CONFLICT(kind, id) DO UPDATE SET \
				uid = CASE WHEN agw_config_resources.deleted_at IS NOT NULL THEN excluded.uid \
					ELSE agw_config_resources.uid END, \
				value_json = excluded.value_json, \
				revision = agw_config_resources.revision + 1, \
				updated_at = excluded.updated_at, \
				deleted_at = NULL \
			 WHERE agw_config_resources.deleted_at IS NOT NULL \
				OR agw_config_resources.value_json IS DISTINCT FROM excluded.value_json",
		)
		.bind(kind.as_str())
		.bind(&id)
		.bind(Json(value))
		.bind(now)
		.execute(&mut *tx)
		.await?;
		let written = result.rows_affected() > 0;
		wrote |= written;
		changed.push((kind, id, written));
	}

	let mut resources = Vec::with_capacity(changed.len());
	for (kind, id, written) in changed {
		if let Some(mut resource) = fetch_postgres_resource(&mut tx, kind, &id).await? {
			resource.result = Some(WriteResult::of(written, resource.revision));
			resources.push(resource);
		}
	}
	if wrote {
		notify_postgres(&mut tx, notification_id).await?;
	}
	tx.commit().await?;
	Ok((resources, generation))
}

async fn rename_sqlite(
	pool: &SqlitePool,
	previous_kind: ConfigResourceKind,
	previous_id: &str,
	prepared: PreparedResource,
	expected_generation: Option<i64>,
) -> anyhow::Result<(ConfigResource, i64)> {
	if previous_kind != prepared.kind || previous_id == prepared.id {
		return Err(
			ConfigResourceError::InvalidRequest("config resource rename requires a new ID".to_string())
				.into(),
		);
	}
	let mut tx = pool.begin_with(SQLITE_BEGIN_IMMEDIATE).await?;
	let generation = bump_generation_sqlite(&mut tx, expected_generation).await?;
	let now = Utc::now().to_rfc3339();
	let Some(uid) = soft_delete_sqlite(&mut tx, previous_kind, previous_id, &now).await? else {
		return Err(
			ConfigResourceError::NotFound(format!(
				"config resource not found: {previous_kind}/{previous_id}"
			))
			.into(),
		);
	};

	// Free the source UID for the destination while retaining the old lookup key's tombstone.
	sqlx::query("UPDATE agw_config_resources SET uid = ? WHERE kind = ? AND id = ?")
		.bind(uuid::Uuid::new_v4().to_string())
		.bind(previous_kind.as_str())
		.bind(previous_id)
		.execute(&mut *tx)
		.await?;

	let inserted = sqlx::query(
		"INSERT INTO agw_config_resources \
		 (kind, id, value_json, revision, created_at, updated_at, deleted_at, uid) \
		 VALUES (?, ?, ?, 1, ?, ?, NULL, ?) \
		 ON CONFLICT(kind, id) DO UPDATE SET \
			uid = excluded.uid, \
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
	.bind(uid)
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
	Ok((resource, generation))
}

async fn rename_postgres(
	pool: &PgPool,
	previous_kind: ConfigResourceKind,
	previous_id: &str,
	prepared: PreparedResource,
	expected_generation: Option<i64>,
	notification_id: &str,
) -> anyhow::Result<(ConfigResource, i64)> {
	if previous_kind != prepared.kind || previous_id == prepared.id {
		return Err(
			ConfigResourceError::InvalidRequest("config resource rename requires a new ID".to_string())
				.into(),
		);
	}
	let mut tx = pool.begin().await?;
	let generation = bump_generation_postgres(&mut tx, expected_generation).await?;
	let now = Utc::now();
	let Some(uid) = soft_delete_postgres(&mut tx, previous_kind, previous_id, now).await? else {
		return Err(
			ConfigResourceError::NotFound(format!(
				"config resource not found: {previous_kind}/{previous_id}"
			))
			.into(),
		);
	};

	// Free the source UID for the destination while retaining the old lookup key's tombstone.
	sqlx::query("UPDATE agw_config_resources SET uid = $1 WHERE kind = $2 AND id = $3")
		.bind(uuid::Uuid::new_v4())
		.bind(previous_kind.as_str())
		.bind(previous_id)
		.execute(&mut *tx)
		.await?;

	let inserted = sqlx::query(
		"INSERT INTO agw_config_resources \
		 (kind, id, value_json, revision, created_at, updated_at, deleted_at, uid) \
		 VALUES ($1, $2, $3, 1, $4, $4, NULL, $5) \
		 ON CONFLICT(kind, id) DO UPDATE SET \
			uid = excluded.uid, \
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
	.bind(uid)
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
	Ok((resource, generation))
}

async fn delete_sqlite(
	pool: &SqlitePool,
	kind: ConfigResourceKind,
	id: &str,
	expected_generation: Option<i64>,
) -> anyhow::Result<Option<i64>> {
	let mut tx = pool.begin_with(SQLITE_BEGIN_IMMEDIATE).await?;
	let generation = bump_generation_sqlite(&mut tx, expected_generation).await?;
	let now = Utc::now().to_rfc3339();
	if soft_delete_sqlite(&mut tx, kind, id, &now).await?.is_none() {
		return Ok(None);
	}
	tx.commit().await?;
	Ok(Some(generation))
}

async fn soft_delete_sqlite(
	tx: &mut Transaction<'_, Sqlite>,
	kind: ConfigResourceKind,
	id: &str,
	now: &str,
) -> anyhow::Result<Option<String>> {
	Ok(
		sqlx::query_scalar(
			"UPDATE agw_config_resources \
		 SET revision = revision + 1, updated_at = ?, deleted_at = ? \
		 WHERE kind = ? AND id = ? AND deleted_at IS NULL \
		 RETURNING uid",
		)
		.bind(now)
		.bind(now)
		.bind(kind.as_str())
		.bind(id)
		.fetch_optional(&mut **tx)
		.await?,
	)
}

async fn delete_postgres(
	pool: &PgPool,
	kind: ConfigResourceKind,
	id: &str,
	expected_generation: Option<i64>,
	notification_id: &str,
) -> anyhow::Result<Option<i64>> {
	let mut tx = pool.begin().await?;
	let generation = bump_generation_postgres(&mut tx, expected_generation).await?;
	let now = Utc::now();
	if soft_delete_postgres(&mut tx, kind, id, now)
		.await?
		.is_none()
	{
		return Ok(None);
	}
	notify_postgres(&mut tx, notification_id).await?;
	tx.commit().await?;
	Ok(Some(generation))
}

async fn soft_delete_postgres(
	tx: &mut Transaction<'_, Postgres>,
	kind: ConfigResourceKind,
	id: &str,
	now: DateTime<Utc>,
) -> anyhow::Result<Option<uuid::Uuid>> {
	Ok(
		sqlx::query_scalar(
			"UPDATE agw_config_resources \
		 SET revision = revision + 1, updated_at = $1, deleted_at = $1 \
		 WHERE kind = $2 AND id = $3 AND deleted_at IS NULL \
		 RETURNING uid",
		)
		.bind(now)
		.bind(kind.as_str())
		.bind(id)
		.fetch_optional(&mut **tx)
		.await?,
	)
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
		"SELECT kind, id, uid, value_json, revision, created_at, updated_at \
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
		"SELECT kind, id, uid, value_json, revision, created_at, updated_at \
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
		uid: Some(row.try_get("uid")?),
		value: serde_json::from_str(&value_json)?,
		revision: row.try_get("revision")?,
		created_at: created_at.parse()?,
		updated_at: updated_at.parse()?,
		result: None,
	})
}

fn postgres_row_to_resource(row: sqlx::postgres::PgRow) -> anyhow::Result<ConfigResource> {
	let kind: String = row.try_get("kind")?;
	let Json(value) = row.try_get("value_json")?;
	Ok(ConfigResource {
		kind: kind.parse()?,
		id: row.try_get("id")?,
		uid: Some(row.try_get::<uuid::Uuid, _>("uid")?.to_string()),
		value,
		revision: row.try_get("revision")?,
		created_at: row.try_get("created_at")?,
		updated_at: row.try_get("updated_at")?,
		result: None,
	})
}

async fn setup_sqlite_schema(pool: &SqlitePool) -> anyhow::Result<()> {
	let mut migrator = sqlx_macros::migrate!("./src/config_store/sqlite_migrations");
	migrator.dangerous_set_table_name(MIGRATIONS_TABLE);
	// SQLite's SQLx migrator relies on the caller to serialize migration discovery and execution.
	let mut tx = pool.begin_with(SQLITE_BEGIN_IMMEDIATE).await?;
	migrator
		.run_direct(None, &mut *tx, false)
		.await
		.context("failed to migrate SQLite config store schema")?;
	tx.commit().await?;
	Ok(())
}

async fn setup_postgres_schema(pool: &PgPool) -> anyhow::Result<()> {
	let mut migrator = sqlx_macros::migrate!("./src/config_store/postgres_migrations");
	migrator.dangerous_set_table_name(MIGRATIONS_TABLE);
	let mut connection = pool.acquire().await?;
	// Keep migration timeouts and any advisory lock left after an error out of the pool.
	connection.close_on_drop();
	sqlx::query(
		"SELECT set_config('lock_timeout', '10s', false), \
		        set_config('statement_timeout', '0', false)",
	)
	.execute(&mut *connection)
	.await
	.context("failed to configure Postgres config store migration timeouts")?;
	migrator
		.run_direct(None, &mut *connection, false)
		.await
		.context("failed to migrate Postgres config store schema")?;
	Ok(())
}

const MIGRATIONS_TABLE: &str = "_agentgateway_config_migrations";
const POSTGRES_CHANGE_CHANNEL: &str = "agentgateway_config_changed";

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn test_resource(kind: ConfigResourceKind, id: &str, value: Value) -> ConfigResource {
		ConfigResource {
			kind,
			id: id.to_string(),
			uid: None,
			value,
			revision: 1,
			created_at: Utc::now(),
			updated_at: Utc::now(),
			result: None,
		}
	}

	fn test_provider(id: &str) -> PreparedResource {
		PreparedResource {
			kind: ConfigResourceKind::LlmProvider,
			id: id.to_string(),
			value: json!({"name": id, "provider": "openAI"}),
		}
	}

	async fn file_backed_store(dir: &tempfile::TempDir) -> ConfigResourceStore {
		let url = format!("sqlite://{}", dir.path().join("config.db").display());
		ConfigResourceStore::connect(&url, None)
			.await
			.expect("connect config resource store")
	}

	fn is_generation_conflict(err: &anyhow::Error) -> bool {
		matches!(
			err.downcast_ref::<ConfigResourceError>(),
			Some(ConfigResourceError::GenerationConflict { .. })
		)
	}

	#[tokio::test]
	async fn rejects_a_write_pinned_to_a_stale_generation() {
		let dir = tempfile::tempdir().expect("tempdir");
		let store = file_backed_store(&dir).await;
		let snapshot = store.snapshot(None).await.expect("snapshot");

		store
			.upsert_prepared(vec![test_provider("a")], Some(snapshot.generation))
			.await
			.expect("write against a current generation");

		let err = store
			.upsert_prepared(vec![test_provider("b")], Some(snapshot.generation))
			.await
			.expect_err("the same generation is now stale");
		assert!(is_generation_conflict(&err), "unexpected error: {err}");

		let after = store.snapshot(None).await.expect("snapshot");
		assert_eq!(
			after
				.resources
				.iter()
				.map(|resource| resource.id.as_str())
				.collect::<Vec<_>>(),
			vec!["a"],
			"a rejected write must leave nothing behind"
		);
		assert_eq!(after.generation, snapshot.generation + 1);
	}

	#[tokio::test]
	async fn concurrent_writers_cannot_both_commit_one_generation() {
		let dir = tempfile::tempdir().expect("tempdir");
		let store = file_backed_store(&dir).await;
		let generation = store.snapshot(None).await.expect("snapshot").generation;

		let (first, second) = tokio::join!(
			store.upsert_prepared(vec![test_provider("a")], Some(generation)),
			store.upsert_prepared(vec![test_provider("b")], Some(generation)),
		);
		let results = [first, second];

		assert_eq!(
			results.iter().filter(|result| result.is_ok()).count(),
			1,
			"exactly one writer may commit a given generation"
		);
		let err = results
			.iter()
			.find_map(|result| result.as_ref().err())
			.expect("one writer must lose");
		assert!(is_generation_conflict(err), "unexpected error: {err}");

		let after = store.snapshot(None).await.expect("snapshot");
		assert_eq!(after.resources.len(), 1, "only the winner's write landed");
		assert_eq!(after.generation, generation + 1);
	}

	#[tokio::test]
	async fn unpinned_writes_still_advance_the_generation() {
		let dir = tempfile::tempdir().expect("tempdir");
		let store = file_backed_store(&dir).await;
		let start = store.snapshot(None).await.expect("snapshot").generation;

		store
			.upsert_prepared(vec![test_provider("a")], None)
			.await
			.expect("unpinned upsert");
		store
			.rename_prepared(
				ConfigResourceKind::LlmProvider,
				"a",
				test_provider("b"),
				None,
			)
			.await
			.expect("unpinned rename");
		store
			.delete(ConfigResourceKind::LlmProvider, "b", None)
			.await
			.expect("unpinned delete");

		assert_eq!(
			store.snapshot(None).await.expect("snapshot").generation,
			start + 3,
			"every mutation advances the generation"
		);
	}

	#[tokio::test]
	async fn upsert_reports_write_result() {
		let dir = tempfile::tempdir().expect("tempdir");
		let store = file_backed_store(&dir).await;
		let upsert = |prepared: PreparedResource| async {
			let resource = store
				.upsert_prepared(vec![prepared], None)
				.await
				.expect("upsert")
				.resources
				.remove(0);
			(resource.result, resource.revision)
		};
		assert_eq!(
			upsert(test_provider("a")).await,
			(Some(WriteResult::Created), 1)
		);
		// Same value with keys in a different order.
		let same = PreparedResource {
			kind: ConfigResourceKind::LlmProvider,
			id: "a".to_string(),
			value: json!({"provider": "openAI", "name": "a"}),
		};
		assert_eq!(upsert(same).await, (Some(WriteResult::Unchanged), 1));
		let mut different = test_provider("a");
		different.value["provider"] = json!("anthropic");
		assert_eq!(upsert(different).await, (Some(WriteResult::Updated), 2));
	}

	#[tokio::test]
	async fn delete_reports_the_generation_it_produced() {
		let dir = tempfile::tempdir().expect("tempdir");
		let store = file_backed_store(&dir).await;
		let created = store
			.upsert_prepared(vec![test_provider("a")], None)
			.await
			.expect("create resource")
			.generation
			.expect("upsert reports a generation");

		let deleted = store
			.delete(ConfigResourceKind::LlmProvider, "a", Some(created))
			.await
			.expect("delete pinned to the current generation");
		assert_eq!(deleted, created + 1);
		assert_eq!(
			store.snapshot(None).await.expect("snapshot").generation,
			deleted,
			"the reported generation must be the one a later write can pin to"
		);
	}

	#[tokio::test]
	async fn an_empty_upsert_still_answers_the_pin() {
		let dir = tempfile::tempdir().expect("tempdir");
		let store = file_backed_store(&dir).await;
		let generation = store.snapshot(None).await.expect("snapshot").generation;

		let response = store
			.upsert_prepared(Vec::new(), Some(generation))
			.await
			.expect("an empty write against a current pin is a no-op");
		assert_eq!(response.generation, Some(generation));
		assert!(response.resources.is_empty());

		let err = store
			.upsert_prepared(Vec::new(), Some(generation - 1))
			.await
			.expect_err("a stale pin must not be silently accepted");
		assert!(is_generation_conflict(&err), "unexpected error: {err}");

		assert_eq!(
			store.snapshot(None).await.expect("snapshot").generation,
			generation,
			"an empty write must not consume a generation"
		);
	}

	#[tokio::test]
	async fn a_delete_that_matches_nothing_does_not_advance_the_generation() {
		let dir = tempfile::tempdir().expect("tempdir");
		let store = file_backed_store(&dir).await;
		let generation = store.snapshot(None).await.expect("snapshot").generation;

		let err = store
			.delete(ConfigResourceKind::LlmProvider, "missing", None)
			.await
			.expect_err("deleting a missing resource fails");
		assert!(err.to_string().contains("not found"), "unexpected: {err}");
		assert_eq!(
			store.snapshot(None).await.expect("snapshot").generation,
			generation,
			"a rolled back delete must not consume a generation"
		);
	}

	#[tokio::test]
	async fn seeds_the_generation_for_databases_written_before_it_existed() {
		let dir = tempfile::tempdir().expect("tempdir");
		let url = format!("sqlite://{}", dir.path().join("config.db").display());
		let DatabasePool::Sqlite(pool) = DatabasePool::connect(&url).await.unwrap() else {
			panic!("expected a SQLite pool");
		};
		sqlx::raw_sql(include_str!(
			"config_store/sqlite_migrations/0001_create_config_store.sql"
		))
		.execute(&pool)
		.await
		.unwrap();
		sqlx::raw_sql(
			"DROP TABLE agw_config_meta;
			 INSERT INTO agw_config_resources (kind, id, value_json, created_at, updated_at, deleted_at)
			 VALUES ('llm.provider', 'a', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL),
			        ('llm.provider', 'b', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
		)
		.execute(&pool)
		.await
		.unwrap();

		let (reopened, _) = tokio::try_join!(
			ConfigResourceStore::from_pool(DatabasePool::Sqlite(pool.clone())),
			ConfigResourceStore::from_pool(DatabasePool::Sqlite(pool.clone())),
		)
		.expect("reconnect config resource stores");
		let snapshot = reopened.snapshot(None).await.expect("snapshot");
		assert_eq!(snapshot.generation, 1, "the counter seeds on startup");
		assert_eq!(snapshot.resources.len(), 1);
		assert_eq!(snapshot.resources[0].id, "a");
		let uids: Vec<String> =
			sqlx::query_scalar("SELECT uid FROM agw_config_resources WHERE id IN ('a', 'b')")
				.fetch_all(&pool)
				.await
				.unwrap();
		assert_eq!(uids.len(), 2);
		assert_ne!(uids[0], uids[1]);
	}

	#[test]
	fn legacy_catalog_base_is_older_than_timestamped_bases() {
		let resource = ConfigResource {
			..test_resource(
				ConfigResourceKind::ModelCatalog,
				"default",
				json!({"base": {"providers": {}}}),
			)
		};

		let sources = merge_model_catalog_sources(&[resource], Vec::new()).unwrap();
		let crate::ModelCatalogSource::InlineCatalog { inline } = &sources[0] else {
			panic!("base must be an inline catalog")
		};
		assert_eq!(
			inline.metadata,
			Some(crate::llm::catalog::CatalogMetadata {
				source: None,
				generated_at: DateTime::<Utc>::UNIX_EPOCH,
				unknown: Default::default(),
			})
		);
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
			created
				.value
				.get("metadata")
				.and_then(Value::as_object)
				.and_then(|metadata| metadata.get(API_KEY_ID_METADATA)),
			Some(&Value::String(created.id.clone()))
		);
		let created_at = api_key_created_at(&created.value).expect("managed creation timestamp");
		assert!(created_at > 0);
		let updated = prepare_api_key_update(
			created.id.clone(),
			json!({"metadata": {"name": "updated"}}),
			Some(created_at),
			Some(&created.value),
		)
		.expect("API key update");
		assert_eq!(api_key_created_at(&updated.value), Some(created_at));
		assert_eq!(
			api_key_metadata(&updated.value).and_then(|metadata| metadata.get(API_KEY_ID_METADATA)),
			Some(&Value::String(created.id))
		);
		let err = prepare_resource(
			ConfigResourceKind::LlmApiKey,
			json!({"metadata": {"agentgateway.dev/owner": "client"}}),
		)
		.expect_err("reserved API key metadata should fail");
		assert!(
			err
				.to_string()
				.contains("reserved agentgateway.dev/ prefix")
		);
		let existing = json!({"metadata": {"agentgateway.dev/owner": "client"}});
		let updated = prepare_api_key_update(
			"key-id".to_string(),
			json!({"metadata": {"agentgateway.dev/owner": "client", "agentgateway.dev/id": "key-id"}}),
			None,
			Some(&existing),
		)
		.expect("API key update with unchanged managed metadata");
		assert_eq!(
			updated.value["metadata"],
			json!({"agentgateway.dev/id": "key-id"})
		);
		for metadata in [
			json!({"agentgateway.dev/owner": "other"}),
			json!({"agentgateway.dev/id": "other"}),
		] {
			let err = prepare_api_key_update(
				"key-id".to_string(),
				json!({"metadata": metadata}),
				None,
				Some(&existing),
			)
			.expect_err("changed managed metadata should fail");
			assert!(err.to_string().contains("cannot be changed"));
		}
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
		let store = ConfigResourceStore::connect("sqlite::memory:", None)
			.await
			.expect("connect config resource store");
		let DatabasePool::Sqlite(pool) = store.pool() else {
			panic!("expected a SQLite pool");
		};
		let pool = &pool;
		let uid = |id: &'static str| async move {
			sqlx::query_scalar::<_, String>(
				"SELECT uid FROM agw_config_resources WHERE kind = 'llm.provider' AND id = ?",
			)
			.bind(id)
			.fetch_one(pool)
			.await
			.unwrap()
		};
		store
			.upsert_prepared(
				vec![PreparedResource {
					kind: ConfigResourceKind::LlmProvider,
					id: "old".to_string(),
					value: json!({"name": "old", "provider": "openAI"}),
				}],
				None,
			)
			.await
			.expect("create resource");
		let original_uid = uid("old").await;
		let mut updated = test_provider("old");
		updated.value["provider"] = json!("anthropic");
		store.upsert_prepared(vec![updated], None).await.unwrap();
		assert_eq!(uid("old").await, original_uid);
		let response = store
			.rename_prepared(
				ConfigResourceKind::LlmProvider,
				"old",
				PreparedResource {
					kind: ConfigResourceKind::LlmProvider,
					id: "new".to_string(),
					value: json!({"name": "new", "provider": "openAI"}),
				},
				None,
			)
			.await
			.expect("rename resource");
		assert_eq!(response.resources[0].id, "new");
		assert_eq!(
			response.resources[0].uid.as_deref(),
			Some(original_uid.as_str())
		);
		assert_eq!(uid("new").await, original_uid);
		assert_ne!(uid("old").await, original_uid);
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
			.upsert_prepared(
				vec![PreparedResource {
					kind: ConfigResourceKind::LlmProvider,
					id: "other".to_string(),
					value: json!({"name": "other", "provider": "openAI"}),
				}],
				None,
			)
			.await
			.expect("create rename target");
		let other_uid = uid("other").await;
		let err = store
			.rename_prepared(
				ConfigResourceKind::LlmProvider,
				"new",
				PreparedResource {
					kind: ConfigResourceKind::LlmProvider,
					id: "other".to_string(),
					value: json!({"name": "other", "provider": "anthropic"}),
				},
				None,
			)
			.await
			.expect_err("rename collision should fail");
		assert!(err.to_string().contains("already exists"));
		assert_eq!(uid("new").await, original_uid);
		assert_eq!(uid("other").await, other_uid);
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

		let kind = ConfigResourceKind::LlmProvider;
		store.delete(kind, "other", None).await.unwrap();
		store
			.rename_prepared(kind, "new", test_provider("other"), None)
			.await
			.unwrap();
		assert_eq!(uid("other").await, original_uid);

		store.delete(kind, "other", None).await.unwrap();
		store
			.upsert_prepared(vec![test_provider("other")], None)
			.await
			.unwrap();
		assert_ne!(uid("other").await, original_uid);
	}

	#[test]
	fn runtime_config_keeps_resource_values_literal() {
		let env_name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
		let home = std::env::var(env_name).unwrap_or_else(|_| panic!("{env_name}"));
		let braced_env = format!("${{{env_name}}}");
		let env = format!("${env_name}");
		let base = format!("binds:\n- port: 1\n  listeners:\n  - name: {braced_env}\n");
		let resources = vec![
			test_resource(
				ConfigResourceKind::LlmProvider,
				"openai",
				json!({
					"name": format!("$5 {braced_env}"),
					"provider": "openAI",
					"params": {"apiKey": format!("sk-{env}"), "model": env},
				}),
			),
			test_resource(
				ConfigResourceKind::LlmModel,
				"model",
				json!({"name": "model", "params": {"apiKey": {"file": format!("{env}/key")}}}),
			),
		];
		let materialized = materialize_runtime_config(&base, resources).expect("materialize");
		let value: Value = crate::yaml::from_str(&materialized).expect("parse");
		assert_eq!(
			value.pointer("/binds/0/listeners/0/name"),
			Some(&json!(home))
		);
		assert_eq!(
			value.pointer("/llm/providers/0"),
			Some(&json!({
				"name": format!("$5 {braced_env}"),
				"provider": "openAI",
				"params": {"apiKey": format!("sk-{home}"), "model": env},
			}))
		);
		assert_eq!(
			value.pointer("/llm/models/0/params/apiKey/file"),
			Some(&json!(format!("{home}/key")))
		);
	}

	#[test]
	fn materializes_config_resources_into_base_config() {
		let base = r#"
config:
  modelCatalog:
  - inline:
      providers:
        file:
          models:
            file-model: {}
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
				ConfigResourceKind::ModelCatalog,
				"default",
				json!({
					"custom": {
						"providers": {
							"database": {
								"models": {
									"database-model": {}
								}
							}
						}
					}
				}),
			),
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
				json!({"key": "agw_sk_test", "metadata": {"agentgateway.dev/id": "key_01", "name": "test"}}),
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
					"failureMode": "failOpen",
					"sseKeepAlive": "15s",
					"dnsRebindingProtection": true,
					"server": {"name": "db-gateway", "version": "1.0.0", "instructions": "Use carefully"}
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
		let value: Value = crate::yaml::from_str(&materialized).expect("parse materialized");

		assert_eq!(
			value.pointer("/config/modelCatalog/0/inline/providers/database/models/database-model"),
			Some(&json!({}))
		);
		assert_eq!(
			value.pointer("/config/modelCatalog/1/inline/providers/file/models/file-model"),
			Some(&json!({}))
		);
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
		assert_eq!(value.pointer("/mcp/sseKeepAlive"), Some(&json!("15s")));
		assert_eq!(
			value.pointer("/mcp/dnsRebindingProtection"),
			Some(&json!(true))
		);
		assert_eq!(
			value.pointer("/mcp/server/name"),
			Some(&json!("db-gateway"))
		);
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
	fn mcp_endpoint_resources_map_to_endpoints() {
		let mut config = json!({"mcp": {"targets": [{"name": "tools", "stdio": {"cmd": "a"}}]}});
		let target = prepare_resource(
			ConfigResourceKind::McpTarget,
			json!({"name": "tools", "endpoint": "/eng/mcp", "stdio": {"cmd": "b"}}),
		)
		.expect("prepare scoped target");
		assert_eq!(target.id, "/eng/mcp:tools");
		upsert_file_config_resource(&mut config, &target, None).expect("add scoped target");
		let policy = prepare_policy_upsert(
			ConfigResourceKind::McpPolicy,
			"/eng/mcp:cors".to_string(),
			json!({"allowOrigins": ["*"]}),
		)
		.expect("prepare scoped policy");
		upsert_file_config_resource(&mut config, &policy, Some(&policy.id)).expect("add policy");
		assert_eq!(
			config.pointer("/mcp/targets"),
			Some(&json!([{"name": "tools", "stdio": {"cmd": "a"}}]))
		);
		assert_eq!(
			config.pointer("/mcp/endpoints"),
			Some(&json!([{
				"path": "/eng/mcp",
				"targets": [{"name": "tools", "stdio": {"cmd": "b"}}],
				"policies": {"cors": {"allowOrigins": ["*"]}}
			}]))
		);
		let listed = file_config_resources(&config, Some(ConfigResourceKind::McpTarget));
		assert_eq!(
			listed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
			["tools", "/eng/mcp:tools"]
		);
		assert_eq!(listed[1].value["endpoint"], json!("/eng/mcp"));

		// Moving the target to another endpoint removes it from the old one.
		let moved = prepare_resource(
			ConfigResourceKind::McpTarget,
			json!({"name": "moved", "endpoint": "/mcp", "stdio": {"cmd": "b"}}),
		)
		.expect("prepare moved target");
		upsert_file_config_resource(&mut config, &moved, Some("/eng/mcp:tools")).expect("move");
		assert_eq!(config.pointer("/mcp/endpoints/0/targets"), Some(&json!([])));
		assert_eq!(
			config.pointer("/mcp/endpoints/1/targets/0/name"),
			Some(&json!("moved"))
		);
		assert!(
			delete_file_config_resource(&mut config, ConfigResourceKind::McpPolicy, "/eng/mcp:cors")
				.expect("delete policy")
		);
		assert_eq!(
			config.pointer("/mcp/endpoints"),
			Some(&json!([{"path": "/mcp", "targets": [{"name": "moved", "stdio": {"cmd": "b"}}]}]))
		);

		let materialized: Value = crate::yaml::from_str(
			&materialize_config(
				"mcp:\n  endpoints:\n  - path: /eng/mcp\n    statefulMode: stateless\n",
				&[
					test_resource(
						ConfigResourceKind::McpTarget,
						"/eng/mcp:db",
						json!({"name": "db", "endpoint": "/eng/mcp", "stdio": {"cmd": "c"}}),
					),
					test_resource(
						ConfigResourceKind::McpTarget,
						"/sales/mcp:db",
						json!({"name": "db", "endpoint": "/sales/mcp", "stdio": {"cmd": "d"}}),
					),
					test_resource(
						ConfigResourceKind::McpSettings,
						"/sales/mcp",
						json!({"endpoint": "/sales/mcp", "prefixMode": "always"}),
					),
				],
			)
			.expect("materialize"),
		)
		.expect("parse materialized");
		assert_eq!(
			materialized.pointer("/mcp/endpoints"),
			Some(&json!([
				{"path": "/eng/mcp", "statefulMode": "stateless", "targets": [{"name": "db", "stdio": {"cmd": "c"}}]},
				{"path": "/sales/mcp", "prefixMode": "always", "targets": [{"name": "db", "stdio": {"cmd": "d"}}]}
			]))
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

		let missing_key = prepare_file_api_key_update(
			"missing".to_string(),
			json!({"key": "agw_missing"}),
			None,
			None,
		)
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
