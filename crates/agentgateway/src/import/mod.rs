//! Import external gateway configuration into standalone agentgateway configuration.
//!
//! Importers normalize source-specific configuration into [`ImportPlan`]. The shared
//! emitter then produces and validates agentgateway configuration, keeping source
//! adapters independent from the target configuration format.

use std::collections::{HashMap, HashSet};

use anyhow::Context;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value, json};

mod litellm;
pub use litellm::{DbStateKey, DbStateMcpServer, DbStateTeam, LiteLlmDbState, parse_db_state};

/// A source-specific configuration adapter.
pub trait ConfigImporter: Send + Sync {
	fn source(&self) -> &'static str;
	fn import(&self, input: &str, options: &ImportOptions) -> anyhow::Result<ImportPlan>;
}

/// Source-neutral representation consumed by the agentgateway configuration emitter.
#[derive(Debug, Default)]
pub struct ImportPlan {
	pub providers: Vec<ImportedProvider>,
	pub models: Vec<ImportedModel>,
	pub routes: IndexMap<String, ImportedRoute>,
	pub findings: Vec<ImportFinding>,
	/// Outlier-detection policy applied to emitted model backends that do not override it. Populated from
	/// source-wide health settings (for example LiteLLM `router_settings.cooldown_time`).
	pub model_health: Option<Value>,
	/// Request timeout policy applied once across the whole `llm:` block, since standalone
	/// configuration has no per-model or per-route timeout attachment point. Populated from
	/// source-wide timeout settings (for example LiteLLM `router_settings.timeout`).
	pub timeout: Option<Value>,
	/// Top-level standalone `routes` entries (outside the `llm:` block), fully built as JSON.
	/// Populated from source-wide pass-through settings (for example LiteLLM
	/// `general_settings.pass_through_endpoints`).
	pub pass_through_routes: Vec<Value>,
	/// The source's inbound apiKey policy, if it configured one (and, if set, a custom header
	/// name). Reused below by the MCP mapping, which mirrors this same policy onto
	/// `mcp.policies` rather than leave the MCP gateway unauthenticated.
	pub inbound_api_key: Option<ImportedInboundAPIKey>,
	/// `llm.policies.guardrails` (agentgateway's `PromptGuard`), applied to every configured
	/// model since standalone configuration has no per-request opt-in. Populated from
	/// source-wide guardrail settings (for example LiteLLM's top-level `guardrails`).
	pub guardrails: Option<Value>,
	/// Top-level standalone `mcp.targets` entries, fully built as JSON. Populated from
	/// source-wide MCP settings (for example LiteLLM's top-level `mcp_servers`).
	pub mcp_targets: Vec<Value>,
	/// Virtual API key entries built from LiteLLM's DB-managed state (teams + keys), fully
	/// built as JSON `LocalAPIKey::Sha256` entries. Merged into the same
	/// `llm.policies.apiKey.keys[]` array as [`ImportPlan::inbound_api_key`] by `emit`.
	/// Populated by `import_litellm_db_state` from a separate DB-state input document, never
	/// from the source document this importer was given.
	pub db_api_keys: Vec<Value>,
	/// Original LiteLLM MCP server identifier (the config.yaml dict key, and its alias if one
	/// is set) mapped to the final sanitized agentgateway target name actually emitted into
	/// [`ImportPlan::mcp_targets`]. Populated by `import_litellm_mcp_servers` as it sanitizes
	/// and deduplicates names, so `import_litellm_db_state` can resolve a DB-state MCP grant
	/// (which references LiteLLM's own server name/id, never agentgateway's target name) to the
	/// right target without re-deriving sanitization - and without missing a collision-driven
	/// rename.
	pub mcp_target_names: HashMap<String, String>,
	/// Same virtual keys as [`ImportPlan::db_api_keys`] (same hashes, same `metadata.name`),
	/// mirrored under `mcp.policies.apiKey`: MCP authorization can't see a key authenticated
	/// only under `llm.policies.apiKey`, so it needs its own attachment with the same hashes.
	/// Omits `allowedModels`/`budgets`, which have no meaning on the MCP side.
	pub mcp_api_keys: Vec<Value>,
	/// `mcp.policies.mcpAuthorization.rules[]` CEL strings built from DB-state MCP grants
	/// (`apiKey.name == "<key>" && mcp.tool.target == "<target>"`), `allow_all_keys` servers
	/// (an unconditional `mcp.tool.target == "<target>"`), and an unconditional admin-bypass
	/// rule for the migrated master key, if any.
	pub mcp_authorization_rules: Vec<String>,
	/// Fully-built target specs held back from [`ImportPlan::mcp_targets`] for a reason that
	/// requires a DB-state grant to override - config.yaml omitted `allow_all_keys` (LiteLLM
	/// defaults that to false), set it to false, or set a non-boolean value; or (see
	/// `import_litellm_db_state`) DB-state disagreed with an otherwise-open config.yaml server.
	/// The paired string is that reason, reused verbatim in the promotion finding when a real
	/// grant restores the target. Promoted into `mcp_targets` only once a DB-state grant actually
	/// restricts it to specific keys via `mcpAuthorization`, never unconditionally: closed-world
	/// semantics deny every target with no matching rule, so promoting it unconditionally would
	/// leave it unreachable by anyone, not open to everyone.
	pub pending_restricted_mcp_targets: Vec<(Value, &'static str)>,
	/// LiteLLM MCP server identifiers (config.yaml dict key and alias, if any) whose specs
	/// were held back from [`ImportPlan::mcp_targets`]. Disagreement must consult this set
	/// so a restricted server cannot be mistaken for a different, already-emitted server
	/// that happens to share the same sanitized target name.
	pub held_back_mcp_servers: HashSet<String>,
}

#[derive(Debug)]
pub struct ImportedInboundAPIKey {
	pub key: String,
	/// `None` omits `location` entirely, relying on the schema's own default
	/// (`Authorization: Bearer`). `Some` is used only for a custom header,
	/// which the schema has no default for.
	pub location: Option<Value>,
}

#[derive(Debug)]
pub struct ImportedProvider {
	pub name: String,
	pub provider: String,
	pub params: Map<String, Value>,
	pub request_headers: IndexMap<String, String>,
}

#[derive(Debug)]
pub enum ImportedModelProvider {
	Inline(String),
	Reference(String),
}

#[derive(Debug)]
pub struct ImportedModel {
	pub name: String,
	pub provider: ImportedModelProvider,
	pub params: Map<String, Value>,
	pub defaults: Map<String, Value>,
	pub request_headers: IndexMap<String, String>,
	pub request_headers_override_defaults: bool,
	pub weight: usize,
	/// False when the deployment overrides source-wide health settings, so
	/// [`ImportPlan::model_health`] is not applied to it.
	pub inherits_model_health: bool,
	/// LiteLLM `litellm_params.order` (lower tried first). Used to build failover priority
	/// tiers among deployments sharing a model name when every deployment in the group sets it.
	pub order: Option<i64>,
	/// This deployment's own source path (e.g. `model_list[2]`), so a later finding about the
	/// group of deployments sharing a model name can point at the real entries instead of a
	/// path that doesn't exist in the source document.
	pub source_path: String,
}

#[derive(Debug, Default)]
pub struct ImportedRoute {
	pub targets: Vec<String>,
	pub fallback_groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImportStatus {
	Exact,
	Approximate,
	Manual,
	Unsupported,
}

impl ImportStatus {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Exact => "exact",
			Self::Approximate => "approximate",
			Self::Manual => "manual",
			Self::Unsupported => "unsupported",
		}
	}
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportFinding {
	pub source_path: String,
	pub status: ImportStatus,
	pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
	pub source: String,
	pub config: Value,
	pub findings: Vec<ImportFinding>,
}

#[derive(Debug, Clone)]
pub struct ImportOptions {
	pub database_url: String,
	/// Reference point for expiry comparisons (keys with an expiry in the past vs. future).
	/// Threaded through explicitly, rather than reading the system clock inside the importer,
	/// so golden tests stay deterministic.
	pub now: DateTime<Utc>,
	/// LiteLLM's DB-managed state (teams, virtual keys), extracted separately from config.yaml.
	/// `None` skips DB-state mapping entirely (config.yaml-only import, unchanged behavior).
	pub db_state: Option<LiteLlmDbState>,
}

impl Default for ImportOptions {
	fn default() -> Self {
		Self {
			database_url: "sqlite://data.db".to_string(),
			now: Utc::now(),
			db_state: None,
		}
	}
}

pub fn available_sources() -> Vec<&'static str> {
	importers()
		.iter()
		.map(|importer| importer.source())
		.collect()
}

pub fn import_config(source: &str, input: &str) -> anyhow::Result<ImportResult> {
	import_config_with_options(source, input, &ImportOptions::default())
}

pub fn import_config_with_options(
	source: &str,
	input: &str,
	options: &ImportOptions,
) -> anyhow::Result<ImportResult> {
	let importer = importers()
		.into_iter()
		.find(|importer| importer.source().eq_ignore_ascii_case(source))
		.ok_or_else(|| {
			anyhow::anyhow!(
				"unsupported import source {source:?}; supported sources: {}",
				available_sources().join(", ")
			)
		})?;
	let plan = importer.import(input, options)?;
	emit(importer.source(), plan, options)
}

fn importers() -> Vec<Box<dyn ConfigImporter>> {
	vec![Box::new(litellm::LiteLlmImporter)]
}

/// Resolves `litellm_params.order` into failover priority tiers for the deployments sharing
/// one model name, only when every deployment in the group sets it and at least two distinct
/// values exist. A group with no `order` at all, a partial setting, or a single tied value is
/// left to the existing weighted (no fallbacks) or flat priority-0 (with fallbacks) behavior,
/// since there is nothing safe to convert.
fn resolve_order_priorities(
	targets: &[String],
	model_by_name: &HashMap<&str, &ImportedModel>,
	findings: &mut Vec<ImportFinding>,
) -> Option<HashMap<String, usize>> {
	if targets.len() < 2 {
		return None;
	}
	let source_paths = || -> Vec<&str> {
		targets
			.iter()
			.filter_map(|target| model_by_name.get(target.as_str()))
			.map(|model| model.source_path.as_str())
			.collect()
	};
	let orders: Vec<Option<i64>> = targets
		.iter()
		.map(|target| model_by_name.get(target.as_str()).and_then(|m| m.order))
		.collect();
	if orders.iter().all(Option::is_none) {
		return None;
	}
	if orders.iter().any(Option::is_none) {
		findings.push(ImportFinding {
			source_path: source_paths().join(", "),
			status: ImportStatus::Manual,
			message: "order is only set on some deployments sharing this model name; order was not used for routing priority"
				.to_string(),
		});
		return None;
	}
	let mut distinct: Vec<i64> = orders
		.into_iter()
		.map(|o| o.expect("checked above"))
		.collect();
	distinct.sort_unstable();
	distinct.dedup();
	if distinct.len() < 2 {
		return None;
	}
	let priorities: HashMap<String, usize> = targets
		.iter()
		.map(|target| {
			let order = model_by_name[target.as_str()].order.expect("checked above");
			let priority = distinct
				.binary_search(&order)
				.expect("value came from distinct set");
			(target.clone(), priority)
		})
		.collect();
	// Order-based failover carries no weight field at all, so any earlier per-deployment
	// "used RPM as weight" finding for these same targets no longer describes what will
	// actually be emitted - retract it rather than leave it standing next to this one.
	let rpm_paths: HashSet<String> = targets
		.iter()
		.filter_map(|target| model_by_name.get(target.as_str()))
		.map(|model| format!("{}.litellm_params.rpm", model.source_path))
		.collect();
	findings.retain(|finding| {
		!(rpm_paths.contains(&finding.source_path)
			&& finding.message == "Used RPM as the relative weight for generated weighted routes")
	});
	findings.push(ImportFinding {
		source_path: source_paths().join(", "),
		status: ImportStatus::Approximate,
		message: "Mapped order to failover priority between these deployments; any RPM-based weighting between them no longer applies"
			.to_string(),
	});
	Some(priorities)
}

/// Builds the `apiKey` policy JSON for an inbound key, shared by the two places that attach one
/// (`llm.policies.apiKey` and `mcp.policies.apiKey`) so both stay in sync.
fn api_key_policy(api_key: &ImportedInboundAPIKey) -> Value {
	let mut policy = Map::from_iter([
		("keys".to_string(), json!([{"key": api_key.key.clone()}])),
		("mode".to_string(), json!("strict")),
	]);
	if let Some(location) = &api_key.location {
		policy.insert("location".to_string(), location.clone());
	}
	Value::Object(policy)
}

fn emit(source: &str, plan: ImportPlan, options: &ImportOptions) -> anyhow::Result<ImportResult> {
	let ImportPlan {
		providers,
		models,
		routes,
		mut findings,
		model_health,
		timeout,
		pass_through_routes,
		guardrails,
		mcp_targets,
		inbound_api_key,
		db_api_keys,
		mcp_target_names: _,
		mcp_api_keys,
		mcp_authorization_rules,
		pending_restricted_mcp_targets: _,
		held_back_mcp_servers: _,
	} = plan;
	let model_by_name: HashMap<_, _> = models
		.iter()
		.map(|model| (model.name.as_str(), model))
		.collect();
	let internal_names = models
		.iter()
		.map(|model| model.name.as_str())
		.collect::<HashSet<_>>();
	let direct_model_names = routes
		.iter()
		.filter_map(|(public_name, route)| {
			let [target] = route.targets.as_slice() else {
				return None;
			};
			if !route.fallback_groups.is_empty()
				|| (internal_names.contains(public_name.as_str()) && public_name != target)
			{
				return None;
			}
			Some((target.clone(), public_name.clone()))
		})
		.collect::<HashMap<_, _>>();
	let emitted_models = models
		.iter()
		.map(|model| {
			let direct_name = direct_model_names.get(&model.name);
			let mut value = Map::new();
			value.insert(
				"name".to_string(),
				json!(direct_name.unwrap_or(&model.name)),
			);
			if direct_name.is_none() {
				value.insert("visibility".to_string(), json!("internal"));
			}
			let provider = match &model.provider {
				ImportedModelProvider::Inline(provider) => json!(provider),
				ImportedModelProvider::Reference(reference) => {
					json!({"reference": reference})
				},
			};
			value.insert("provider".to_string(), provider);
			value.insert("params".to_string(), Value::Object(model.params.clone()));
			if !model.defaults.is_empty() {
				value.insert(
					"defaults".to_string(),
					Value::Object(model.defaults.clone()),
				);
			}
			if model.request_headers_override_defaults || !model.request_headers.is_empty() {
				value.insert(
					"requestHeaders".to_string(),
					json!({"set": model.request_headers}),
				);
			}
			if model.inherits_model_health
				&& let Some(health) = &model_health
			{
				value.insert("health".to_string(), health.clone());
			}
			Value::Object(value)
		})
		.collect::<Vec<_>>();

	let mut virtual_models = Vec::new();
	for (name, route) in routes {
		if route.targets.is_empty() {
			continue;
		}
		if let [target] = route.targets.as_slice()
			&& direct_model_names.get(target) == Some(&name)
		{
			continue;
		}
		let order_priorities = resolve_order_priorities(&route.targets, &model_by_name, &mut findings);
		let routing = if route.fallback_groups.is_empty() {
			if let Some(priorities) = &order_priorities {
				let targets = route
					.targets
					.iter()
					.map(|target| {
						let priority = priorities[target];
						let target = direct_model_names.get(target).unwrap_or(target);
						json!({"model": target, "priority": priority})
					})
					.collect::<Vec<_>>();
				json!({"failover": {"targets": targets}})
			} else {
				let targets = route
					.targets
					.iter()
					.map(|target| {
						let weight = model_by_name.get(target.as_str()).map_or(1, |m| m.weight);
						let target = direct_model_names.get(target).unwrap_or(target);
						json!({"model": target, "weight": weight})
					})
					.collect::<Vec<_>>();
				json!({"weighted": {"targets": targets}})
			}
		} else {
			let own_group_tier_count = order_priorities
				.as_ref()
				.map(|priorities| priorities.values().copied().max().unwrap_or(0) + 1)
				.unwrap_or(1);
			let mut targets = route
				.targets
				.iter()
				.map(|target| {
					let priority = order_priorities
						.as_ref()
						.map(|priorities| priorities[target])
						.unwrap_or(0);
					let target = direct_model_names.get(target).unwrap_or(target);
					json!({"model": target, "priority": priority})
				})
				.collect::<Vec<_>>();
			for (tier, fallback_group) in route.fallback_groups.iter().enumerate() {
				let priority = own_group_tier_count + tier;
				for fallback in fallback_group {
					let fallback = direct_model_names.get(fallback).unwrap_or(fallback);
					targets.push(json!({"model": fallback, "priority": priority}));
				}
			}
			json!({"failover": {"targets": targets}})
		};
		virtual_models.push(json!({"name": name, "routing": routing}));
	}

	let mut config = crate::types::local::default_standalone_config(&options.database_url);
	let models_is_empty = emitted_models.is_empty();
	let mut llm = Map::from_iter([
		("gateways".to_string(), json!("default")),
		("models".to_string(), Value::Array(emitted_models)),
	]);
	if !providers.is_empty() {
		let providers = providers
			.into_iter()
			.map(|provider| {
				let mut value = Map::from_iter([
					("name".to_string(), json!(provider.name)),
					("provider".to_string(), json!(provider.provider)),
					("params".to_string(), Value::Object(provider.params)),
				]);
				if !provider.request_headers.is_empty() {
					value.insert(
						"defaults".to_string(),
						json!({
							"requestHeaders": {"set": provider.request_headers}
						}),
					);
				}
				Value::Object(value)
			})
			.collect();
		llm.insert("providers".to_string(), Value::Array(providers));
	}
	if !virtual_models.is_empty() {
		llm.insert("virtualModels".to_string(), Value::Array(virtual_models));
	}
	let mut llm_policies = Map::new();
	if let Some(timeout) = timeout {
		llm_policies.insert("timeout".to_string(), timeout);
	}
	if let Some(guardrails) = guardrails {
		llm_policies.insert("guardrails".to_string(), guardrails);
	}
	{
		let mut keys: Vec<Value> = Vec::new();
		let mut location: Option<Value> = None;
		if let Some(api_key) = &inbound_api_key {
			location = api_key.location.clone();
			let mut entry = Map::from_iter([("key".to_string(), json!(api_key.key.clone()))]);
			if !db_api_keys.is_empty() {
				// Only attach metadata when DB-state keys are also being merged in - a
				// config.yaml-only import has no other key to disambiguate this one from, so
				// this entry is unaffected either way.
				entry.insert("metadata".to_string(), json!({"name": "master"}));
			}
			keys.push(Value::Object(entry));
		}
		keys.extend(db_api_keys.iter().cloned());
		if !keys.is_empty() {
			let mut api_key_policy = Map::from_iter([
				("keys".to_string(), Value::Array(keys)),
				("mode".to_string(), json!("strict")),
			]);
			if let Some(location) = location {
				api_key_policy.insert("location".to_string(), location);
			}
			llm_policies.insert("apiKey".to_string(), Value::Object(api_key_policy));
		}
	}
	if !llm_policies.is_empty() {
		llm.insert("policies".to_string(), Value::Object(llm_policies));
	}
	let config_object = config
		.as_object_mut()
		.expect("default standalone config must be an object");
	// Omit `llm:` entirely for an MCP-only import; standalone configuration has no use for an
	// LLM gateway block with zero models. A source adapter that populated timeout/guardrails/an
	// inbound key but imported no models is responsible for its own findings explaining why
	// those fields don't appear here - this is purely a target-schema fact, not a decision.
	if !models_is_empty {
		config_object.insert("llm".to_string(), Value::Object(llm));
	}
	if !pass_through_routes.is_empty() {
		config_object.insert("routes".to_string(), Value::Array(pass_through_routes));
	}
	// Whether an empty mcp_targets list (e.g. because a source adapter decided its migrated
	// servers couldn't be safely protected) needs its own findings is that adapter's call, made
	// before the plan ever reaches this function. Which of the two optional, already-resolved
	// auth shapes to serialize - a source's own per-key `mcp_api_keys`/`mcp_authorization_rules`,
	// or a plain mirror of the inbound key - is plumbing, not a decision: both are generic
	// `ImportPlan` fields any source can populate the same way.
	if !mcp_targets.is_empty() {
		let has_db_state_mcp_auth = !mcp_api_keys.is_empty() || !mcp_authorization_rules.is_empty();
		let mut mcp_policies = Map::new();
		if has_db_state_mcp_auth {
			let mut api_key_policy = Map::from_iter([
				("keys".to_string(), Value::Array(mcp_api_keys)),
				("mode".to_string(), json!("strict")),
			]);
			// Carry the same custom header (if any) that protects the llm: block, so a
			// client using litellm_key_header_name isn't rejected on /mcp alone.
			if let Some(api_key) = &inbound_api_key
				&& let Some(location) = &api_key.location
			{
				api_key_policy.insert("location".to_string(), location.clone());
			}
			mcp_policies.insert("apiKey".to_string(), Value::Object(api_key_policy));
			mcp_policies.insert(
				"mcpAuthorization".to_string(),
				json!({"rules": mcp_authorization_rules}),
			);
		} else if let Some(api_key) = &inbound_api_key {
			mcp_policies.insert("apiKey".to_string(), api_key_policy(api_key));
		}
		let mcp_object = json!({
			"gateways": "default",
			"targets": mcp_targets,
			"policies": Value::Object(mcp_policies),
		});
		config_object.insert("mcp".to_string(), mcp_object);
	}
	let yaml = crate::yaml::to_string(&config)?;
	let _: crate::types::local::LocalConfig =
		crate::yaml::from_str(&yaml).context("generated agentgateway configuration is invalid")?;
	Ok(ImportResult {
		source: source.to_string(),
		config,
		findings,
	})
}

fn move_string(
	source: &mut Map<String, Value>,
	from: &str,
	target: &mut Map<String, Value>,
	to: &str,
) {
	if let Some(value) = source.remove(from) {
		target.insert(to.to_string(), normalize_environment_references(value));
	}
}

fn normalize_environment_references(value: Value) -> Value {
	match value {
		Value::String(value) => match value.strip_prefix("os.environ/") {
			Some(environment) => json!(format!("${environment}")),
			None => json!(value),
		},
		Value::Array(values) => Value::Array(
			values
				.into_iter()
				.map(normalize_environment_references)
				.collect(),
		),
		Value::Object(values) => Value::Object(
			values
				.into_iter()
				.map(|(key, value)| (key, normalize_environment_references(value)))
				.collect(),
		),
		value => value,
	}
}

fn report_unmapped_fields(
	findings: &mut Vec<ImportFinding>,
	prefix: &str,
	values: &Map<String, Value>,
	status: ImportStatus,
	message: &str,
) {
	for key in values.keys() {
		let separator = if prefix.is_empty() { "" } else { "." };
		findings.push(ImportFinding {
			source_path: format!("{prefix}{separator}{key}"),
			status,
			message: message.to_string(),
		});
	}
}

fn sanitize_name(name: &str) -> String {
	let sanitized = name
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
				character
			} else {
				'-'
			}
		})
		.collect::<String>();
	if sanitized.is_empty() {
		"model".to_string()
	} else {
		sanitized
	}
}

/// Sanitizes an MCP server name (or alias) into agentgateway's Gateway-API-style
/// `SectionName` pattern: lowercase letters, digits, `-`, and `.`, starting and ending with an
/// alphanumeric character. Returns `None` if nothing usable remains.
fn sanitize_mcp_target_name(name: &str) -> Option<String> {
	let sanitized: String = name
		.chars()
		.map(|c| {
			let c = c.to_ascii_lowercase();
			if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.' {
				c
			} else {
				'-'
			}
		})
		.collect();
	let trimmed = sanitized.trim_matches(|c: char| c == '-' || c == '.');
	if trimmed.is_empty() {
		None
	} else {
		Some(trimmed.to_string())
	}
}

fn imported_model_name(source: &str, public_name: &str, deployment: usize) -> String {
	format!(
		"imported/{}/{}/{}",
		sanitize_name(source),
		sanitize_name(public_name),
		deployment
	)
}

fn unique_imported_provider_name(
	source: &str,
	credential: &str,
	providers: &[ImportedProvider],
) -> String {
	let base = format!(
		"imported/{}/{}",
		sanitize_name(source),
		sanitize_name(credential)
	);
	if !providers.iter().any(|candidate| candidate.name == base) {
		return base;
	}
	for suffix in 2.. {
		let candidate = format!("{base}/{suffix}");
		if !providers.iter().any(|provider| provider.name == candidate) {
			return candidate;
		}
	}
	unreachable!()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lists_registered_sources_in_stable_order() {
		assert_eq!(available_sources(), vec!["litellm"]);
	}
}
