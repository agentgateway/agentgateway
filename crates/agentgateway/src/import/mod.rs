//! Import external gateway configuration into standalone agentgateway configuration.
//!
//! Importers normalize source-specific configuration into [`ImportPlan`]. The shared
//! emitter then produces and validates agentgateway configuration, keeping source
//! adapters independent from the target configuration format.

use std::collections::{HashMap, HashSet};

use anyhow::Context;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value, json};

mod litellm;

/// A source-specific configuration adapter.
pub trait ConfigImporter: Send + Sync {
	fn source(&self) -> &'static str;
	fn import(&self, input: &str) -> anyhow::Result<ImportPlan>;
}

/// Source-neutral representation consumed by the agentgateway configuration emitter.
#[derive(Debug, Default)]
pub struct ImportPlan {
	pub providers: Vec<ImportedProvider>,
	pub models: Vec<ImportedModel>,
	pub routes: IndexMap<String, ImportedRoute>,
	pub inbound_api_key: Option<ImportedInboundAPIKey>,
	pub findings: Vec<ImportFinding>,
	/// Outlier-detection policy applied to emitted model backends that do not override it. Populated from
	/// source-wide health settings (for example LiteLLM `router_settings.cooldown_time`).
	pub model_health: Option<Value>,
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
}

impl Default for ImportOptions {
	fn default() -> Self {
		Self {
			database_url: "sqlite://data.db".to_string(),
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
	let plan = importer.import(input)?;
	emit(importer.source(), plan, options)
}

fn importers() -> Vec<Box<dyn ConfigImporter>> {
	vec![Box::new(litellm::LiteLlmImporter)]
}

fn emit(source: &str, plan: ImportPlan, options: &ImportOptions) -> anyhow::Result<ImportResult> {
	let ImportPlan {
		providers,
		models,
		routes,
		inbound_api_key,
		findings,
		model_health,
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
		let routing = if route.fallback_groups.is_empty() {
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
		} else {
			let mut targets = route
				.targets
				.iter()
				.map(|target| {
					let target = direct_model_names.get(target).unwrap_or(target);
					json!({"model": target, "priority": 0})
				})
				.collect::<Vec<_>>();
			for (priority, fallback_group) in route.fallback_groups.iter().enumerate() {
				for fallback in fallback_group {
					let fallback = direct_model_names.get(fallback).unwrap_or(fallback);
					targets.push(json!({"model": fallback, "priority": priority + 1}));
				}
			}
			json!({"failover": {"targets": targets}})
		};
		virtual_models.push(json!({"name": name, "routing": routing}));
	}

	let mut config = crate::types::local::default_standalone_config(&options.database_url);
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
	if let Some(api_key) = inbound_api_key {
		let mut policy = Map::from_iter([
			("keys".to_string(), json!([{"key": api_key.key}])),
			("mode".to_string(), json!("strict")),
		]);
		if let Some(location) = api_key.location {
			policy.insert("location".to_string(), location);
		}
		llm_policies.insert("apiKey".to_string(), Value::Object(policy));
	}
	if !llm_policies.is_empty() {
		llm.insert("policies".to_string(), Value::Object(llm_policies));
	}
	config
		.as_object_mut()
		.expect("default standalone config must be an object")
		.insert("llm".to_string(), Value::Object(llm));
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
