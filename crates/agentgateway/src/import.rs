//! Import external gateway configuration into standalone agentgateway configuration.
//!
//! Importers normalize source-specific configuration into [`ImportPlan`]. The shared
//! emitter then produces and validates agentgateway configuration, keeping source
//! adapters independent from the target configuration format.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, bail};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

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
	pub findings: Vec<ImportFinding>,
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
	vec![Box::new(LiteLlmImporter)]
}

fn emit(source: &str, plan: ImportPlan, options: &ImportOptions) -> anyhow::Result<ImportResult> {
	let ImportPlan {
		providers,
		models,
		routes,
		findings,
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
	config
		.as_object_mut()
		.expect("default standalone config must be an object")
		.insert("llm".to_string(), Value::Object(llm));
	let yaml = crate::yamlviajson::to_string(&config)?;
	let _: crate::types::local::LocalConfig = crate::yamlviajson::from_str(&yaml)
		.context("generated agentgateway configuration is invalid")?;
	Ok(ImportResult {
		source: source.to_string(),
		config,
		findings,
	})
}

struct LiteLlmImporter;

#[derive(Debug, Deserialize)]
struct LiteLlmConfig {
	#[serde(default)]
	model_list: Vec<LiteLlmModel>,
	#[serde(default)]
	credential_list: Vec<Value>,
	#[serde(default)]
	router_settings: Map<String, Value>,
	#[serde(default)]
	litellm_settings: Map<String, Value>,
	#[serde(default)]
	general_settings: Map<String, Value>,
	#[serde(default)]
	environment_variables: Map<String, Value>,
	#[serde(flatten)]
	other: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct LiteLlmModel {
	model_name: String,
	#[serde(default)]
	litellm_params: Map<String, Value>,
	#[serde(default)]
	model_info: Map<String, Value>,
	#[serde(flatten)]
	other: Map<String, Value>,
}

#[derive(Debug)]
struct LiteLlmCredential {
	name: String,
	source_path: String,
	provider_hint: Option<String>,
	params: Map<String, Value>,
	upstream_headers: LiteLlmUpstreamHeaders,
}

#[derive(Debug, Default)]
struct LiteLlmUpstreamHeaders {
	organization: LiteLlmOrganization,
	extra_headers: LiteLlmExtraHeaders,
}

impl LiteLlmUpstreamHeaders {
	fn any_present(&self) -> bool {
		self.organization.present || self.extra_headers.present
	}
}

#[derive(Debug, Default)]
struct LiteLlmOrganization {
	present: bool,
	value: Option<String>,
}

#[derive(Debug, Default)]
struct LiteLlmExtraHeaders {
	present: bool,
	values: IndexMap<String, String>,
	exact_findings: Vec<ImportFinding>,
}

#[derive(Debug)]
struct LiteLlmCredentials {
	entries: IndexMap<String, LiteLlmCredential>,
	ambiguous: HashSet<String>,
	used: HashSet<String>,
	emitted: HashMap<(String, String), String>,
	reported_request_headers: HashSet<String>,
	reported_organization_providers: HashSet<(String, String)>,
}

impl ConfigImporter for LiteLlmImporter {
	fn source(&self) -> &'static str {
		"litellm"
	}

	fn import(&self, input: &str) -> anyhow::Result<ImportPlan> {
		let config: LiteLlmConfig =
			crate::yamlviajson::from_str(input).context("invalid LiteLLM configuration")?;
		if config.model_list.is_empty() {
			bail!("LiteLLM configuration does not contain any model_list entries");
		}

		let mut plan = ImportPlan::default();
		let mut credentials = LiteLlmCredentials::parse(config.credential_list, &mut plan.findings);
		let use_rpm_weights = match config.router_settings.get("routing_strategy") {
			None => true,
			Some(Value::String(strategy)) => strategy == "simple-shuffle",
			Some(_) => false,
		};
		let mut counts = HashMap::<String, usize>::new();
		for (index, model) in config.model_list.into_iter().enumerate() {
			let source_path = format!("model_list[{index}]");
			let deployment = counts.entry(model.model_name.clone()).or_default();
			*deployment += 1;
			let internal_name = imported_model_name(self.source(), &model.model_name, *deployment);
			let Some((public_name, mut imported, credential_name, model_upstream_headers)) =
				import_litellm_model(
					internal_name.clone(),
					model,
					&source_path,
					use_rpm_weights,
					&mut plan.findings,
				)
			else {
				continue;
			};
			if let Some(credential_name) = credential_name {
				credentials.apply(
					self.source(),
					&source_path,
					&credential_name,
					&model_upstream_headers,
					&mut imported,
					&mut plan,
				);
			}
			plan
				.routes
				.entry(public_name)
				.or_default()
				.targets
				.push(internal_name);
			plan.models.push(imported);
		}
		credentials.report_unused(&mut plan.findings);

		let fallbacks = config
			.router_settings
			.get("fallbacks")
			.map(|fallbacks| ("router_settings.fallbacks", fallbacks))
			.or_else(|| {
				config
					.litellm_settings
					.get("fallbacks")
					.map(|fallbacks| ("litellm_settings.fallbacks", fallbacks))
			});
		if let Some((source_path, fallbacks)) = fallbacks {
			apply_fallbacks(fallbacks, source_path, &mut plan);
		}

		if let Some(strategy) = config.router_settings.get("routing_strategy") {
			let (status, message) = match strategy.as_str() {
				Some("simple-shuffle") => (
					ImportStatus::Approximate,
					"Approximated LiteLLM simple-shuffle with generated agentgateway routing; RPM is used only by weighted routes"
						.to_string(),
				),
				Some(strategy) => (
					ImportStatus::Unsupported,
					format!(
						"LiteLLM routing strategy {strategy:?} is not preserved; generated routes use equal weights or priority failover"
					),
				),
				None => (
					ImportStatus::Manual,
					"Routing strategy must be a string and was not preserved".to_string(),
				),
			};
			plan.findings.push(ImportFinding {
				source_path: "router_settings.routing_strategy".to_string(),
				status,
				message,
			});
		}
		for setting in config.router_settings.keys() {
			if matches!(setting.as_str(), "fallbacks" | "routing_strategy") {
				continue;
			}
			plan.findings.push(ImportFinding {
				source_path: format!("router_settings.{setting}"),
				status: ImportStatus::Manual,
				message: "No automatic mapping is available; review this router setting".to_string(),
			});
		}
		for (section, values) in [
			("general_settings", &config.general_settings),
			("environment_variables", &config.environment_variables),
		] {
			for setting in values.keys() {
				if setting == "database_url"
					|| (section == "environment_variables" && setting == "DATABASE_URL")
				{
					report_litellm_database(&mut plan.findings, &format!("{section}.{setting}"));
					continue;
				}
				plan.findings.push(ImportFinding {
					source_path: format!("{section}.{setting}"),
					status: ImportStatus::Manual,
					message: "Requires manual review and was not emitted".to_string(),
				});
			}
		}
		for setting in config.litellm_settings.keys() {
			if setting == "fallbacks" {
				continue;
			}
			if setting == "database_url" {
				report_litellm_database(&mut plan.findings, &format!("litellm_settings.{setting}"));
				continue;
			}
			plan.findings.push(ImportFinding {
				source_path: format!("litellm_settings.{setting}"),
				status: ImportStatus::Unsupported,
				message: "No automatic mapping is available".to_string(),
			});
		}
		report_unmapped_fields(
			&mut plan.findings,
			"",
			&config.other,
			ImportStatus::Unsupported,
			"Unrecognized LiteLLM top-level field was not emitted",
		);
		Ok(plan)
	}
}

fn report_litellm_database(findings: &mut Vec<ImportFinding>, source_path: &str) {
	findings.push(ImportFinding {
		source_path: source_path.to_string(),
		status: ImportStatus::Manual,
		message: "LiteLLM database configuration was not reused; generated config uses a new agentgateway SQLite database. Configure PostgreSQL manually if desired"
			.to_string(),
	});
}

impl LiteLlmCredentials {
	fn parse(entries: Vec<Value>, findings: &mut Vec<ImportFinding>) -> Self {
		let mut parsed = Vec::new();
		let mut counts = HashMap::<String, usize>::new();
		for (index, entry) in entries.into_iter().enumerate() {
			let source_path = format!("credential_list[{index}]");
			let Value::Object(mut entry) = entry else {
				findings.push(ImportFinding {
					source_path,
					status: ImportStatus::Unsupported,
					message: "LiteLLM credential entry must be an object and was not emitted".to_string(),
				});
				continue;
			};
			let name = match entry.remove("credential_name") {
				Some(Value::String(name)) if !name.is_empty() => name,
				_ => {
					findings.push(ImportFinding {
						source_path: format!("{source_path}.credential_name"),
						status: ImportStatus::Unsupported,
						message: "LiteLLM credential_name must be a non-empty string and was not emitted"
							.to_string(),
					});
					continue;
				},
			};
			let mut credential_values = match entry.remove("credential_values") {
				Some(Value::Object(values)) => values,
				_ => {
					findings.push(ImportFinding {
						source_path: format!("{source_path}.credential_values"),
						status: ImportStatus::Unsupported,
						message: "LiteLLM credential_values must be an object and was not emitted".to_string(),
					});
					continue;
				},
			};
			let mut credential_info = match entry.remove("credential_info") {
				None => Map::new(),
				Some(Value::Object(info)) => info,
				Some(_) => {
					findings.push(ImportFinding {
						source_path: format!("{source_path}.credential_info"),
						status: ImportStatus::Manual,
						message: "LiteLLM credential_info must be an object and was not used".to_string(),
					});
					Map::new()
				},
			};
			let provider_hint = match credential_info.remove("provider") {
				None => None,
				Some(Value::String(provider)) => Some(provider),
				Some(_) => {
					findings.push(ImportFinding {
						source_path: format!("{source_path}.credential_info.provider"),
						status: ImportStatus::Manual,
						message: "LiteLLM credential provider metadata must be a string and was not used"
							.to_string(),
					});
					None
				},
			};
			report_unmapped_fields(
				findings,
				&format!("{source_path}.credential_info"),
				&credential_info,
				ImportStatus::Manual,
				"LiteLLM credential metadata requires manual review and was not emitted",
			);
			report_unmapped_fields(
				findings,
				&source_path,
				&entry,
				ImportStatus::Unsupported,
				"Unrecognized LiteLLM credential field was not emitted",
			);

			let mut params = Map::new();
			move_litellm_provider_params(&mut credential_values, &mut params);
			let extra_headers = take_litellm_extra_headers(
				&mut credential_values,
				&format!("{source_path}.credential_values"),
				findings,
			);
			let organization = take_litellm_organization(
				&mut credential_values,
				&format!("{source_path}.credential_values.organization"),
				findings,
			);
			report_unmapped_fields(
				findings,
				&format!("{source_path}.credential_values"),
				&credential_values,
				ImportStatus::Manual,
				"LiteLLM credential value requires manual review and was not emitted",
			);
			*counts.entry(name.clone()).or_default() += 1;
			parsed.push(LiteLlmCredential {
				name,
				source_path,
				provider_hint,
				params,
				upstream_headers: LiteLlmUpstreamHeaders {
					organization,
					extra_headers,
				},
			});
		}

		let ambiguous = counts
			.iter()
			.filter(|(_, count)| **count > 1)
			.map(|(name, _)| name.clone())
			.collect::<HashSet<_>>();
		let mut credentials = IndexMap::new();
		for credential in parsed {
			if ambiguous.contains(&credential.name) {
				findings.push(ImportFinding {
					source_path: format!("{}.credential_name", credential.source_path),
					status: ImportStatus::Unsupported,
					message: format!(
						"LiteLLM credential name {:?} is duplicated and cannot be resolved safely",
						credential.name
					),
				});
			} else {
				credentials.insert(credential.name.clone(), credential);
			}
		}
		Self {
			entries: credentials,
			ambiguous,
			used: HashSet::new(),
			emitted: HashMap::new(),
			reported_request_headers: HashSet::new(),
			reported_organization_providers: HashSet::new(),
		}
	}

	fn report_unused(&self, findings: &mut Vec<ImportFinding>) {
		for credential in self.entries.values() {
			if !self.used.contains(&credential.name) {
				findings.push(ImportFinding {
					source_path: credential.source_path.clone(),
					status: ImportStatus::Manual,
					message: format!(
						"LiteLLM credential {:?} is not referenced by an imported model and was not emitted",
						credential.name
					),
				});
			}
		}
	}

	fn apply(
		&mut self,
		source: &str,
		model_source_path: &str,
		credential_name: &str,
		model_upstream_headers: &LiteLlmUpstreamHeaders,
		model: &mut ImportedModel,
		plan: &mut ImportPlan,
	) {
		let reference_path = format!("{model_source_path}.litellm_params.litellm_credential_name");
		if self.ambiguous.contains(credential_name) {
			plan.findings.push(ImportFinding {
				source_path: reference_path,
				status: ImportStatus::Unsupported,
				message: format!(
					"LiteLLM credential {credential_name:?} is duplicated and was not applied"
				),
			});
			return;
		}
		let Some(credential) = self.entries.get(credential_name) else {
			plan.findings.push(ImportFinding {
				source_path: reference_path,
				status: ImportStatus::Unsupported,
				message: format!(
					"LiteLLM credential {credential_name:?} is not defined and was not applied"
				),
			});
			return;
		};
		let first_use = self.used.insert(credential.name.clone());

		let ImportedModelProvider::Inline(provider) = &model.provider else {
			return;
		};
		let provider = provider.clone();
		if let Some(provider_hint) = &credential.provider_hint {
			match map_provider(provider_hint) {
			Some(mapped) if mapped == provider => {},
			Some(mapped) => plan.findings.push(ImportFinding {
				source_path: format!("{}.credential_info.provider", credential.source_path),
				status: ImportStatus::Manual,
				message: format!(
					"Credential provider metadata maps to {mapped:?}, but the referencing model uses {provider:?}; the model provider was retained"
				),
			}),
			None => plan.findings.push(ImportFinding {
				source_path: format!("{}.credential_info.provider", credential.source_path),
				status: ImportStatus::Manual,
				message: format!(
					"Credential provider metadata {provider_hint:?} is not recognized; the referencing model provider was retained"
				),
			}),
		}
		}

		if credential.params.is_empty()
			&& credential.upstream_headers.extra_headers.values.is_empty()
			&& credential.upstream_headers.organization.value.is_none()
		{
			plan.findings.push(ImportFinding {
				source_path: reference_path,
				status: ImportStatus::Manual,
				message: format!(
					"LiteLLM credential {credential_name:?} contains no supported provider values and was not applied"
				),
			});
			return;
		}
		let can_reference = model
			.params
			.iter()
			.filter(|(key, _)| key.as_str() != "model")
			.all(|(key, value)| credential.params.get(key) == Some(value));
		let mut organization_findings = Vec::new();
		let credential_request_headers = request_headers_for_provider(
			&provider,
			credential.upstream_headers.organization.value.as_deref(),
			&credential.upstream_headers.extra_headers.values,
			&format!("{}.credential_values.organization", credential.source_path),
			&mut organization_findings,
		);
		let credential_organization_emitted =
			can_reference || !model_upstream_headers.organization.present;
		if credential_organization_emitted
			&& credential.upstream_headers.organization.value.is_some()
			&& self
				.reported_organization_providers
				.insert((credential.name.clone(), provider.clone()))
		{
			plan.findings.extend(organization_findings);
		}
		let credential_extra_headers_emitted =
			can_reference || !model_upstream_headers.extra_headers.present;
		if credential_extra_headers_emitted
			&& self
				.reported_request_headers
				.insert(credential.name.clone())
		{
			plan.findings.extend(
				credential
					.upstream_headers
					.extra_headers
					.exact_findings
					.iter()
					.cloned(),
			);
		}

		let organization = if model_upstream_headers.organization.present {
			model_upstream_headers.organization.value.as_deref()
		} else {
			credential.upstream_headers.organization.value.as_deref()
		};
		let extra_headers = if model_upstream_headers.extra_headers.present {
			&model_upstream_headers.extra_headers.values
		} else {
			&credential.upstream_headers.extra_headers.values
		};
		model.request_headers = build_request_headers(&provider, organization, extra_headers);
		model.request_headers_override_defaults = model_upstream_headers.any_present();
		if first_use {
			plan.findings.push(ImportFinding {
				source_path: credential.source_path.clone(),
				status: ImportStatus::Exact,
				message: format!("Imported supported values from LiteLLM credential {credential_name:?}"),
			});
		}

		if can_reference {
			model
				.params
				.retain(|key, _| key == "model" || !credential.params.contains_key(key));
			if !model.request_headers_override_defaults {
				model.request_headers.clear();
			}
			let key = (credential.name.clone(), provider.clone());
			let provider_name = if let Some(provider_name) = self.emitted.get(&key) {
				provider_name.clone()
			} else {
				let provider_name =
					unique_imported_provider_name(source, &credential.name, &plan.providers);
				plan.providers.push(ImportedProvider {
					name: provider_name.clone(),
					provider,
					params: credential.params.clone(),
					request_headers: credential_request_headers,
				});
				self.emitted.insert(key, provider_name.clone());
				provider_name
			};
			model.provider = ImportedModelProvider::Reference(provider_name.clone());
			plan.findings.push(ImportFinding {
			source_path: reference_path,
			status: ImportStatus::Exact,
			message: format!(
				"Mapped LiteLLM credential {credential_name:?} to reusable agentgateway provider {provider_name:?}"
			),
		});
		} else {
			let model_params = std::mem::take(&mut model.params);
			model.params = credential.params.clone();
			model.params.extend(model_params);
			plan.findings.push(ImportFinding {
			source_path: reference_path,
			status: ImportStatus::Approximate,
			message: format!(
				"Applied LiteLLM credential {credential_name:?} inline because model-level provider parameters override the shared credential"
			),
		});
		}
	}
}

fn import_litellm_model(
	name: String,
	model: LiteLlmModel,
	source_path: &str,
	use_rpm_weights: bool,
	findings: &mut Vec<ImportFinding>,
) -> Option<(
	String,
	ImportedModel,
	Option<String>,
	LiteLlmUpstreamHeaders,
)> {
	let LiteLlmModel {
		model_name: public_name,
		litellm_params: mut params,
		model_info,
		other,
	} = model;
	report_unmapped_fields(
		findings,
		&format!("{source_path}.model_info"),
		&model_info,
		ImportStatus::Manual,
		"LiteLLM model metadata requires manual review and was not emitted",
	);
	report_unmapped_fields(
		findings,
		source_path,
		&other,
		ImportStatus::Unsupported,
		"Unrecognized LiteLLM model field was not emitted",
	);
	let model_id = match params.remove("model") {
		Some(Value::String(model)) => model,
		Some(_) => {
			findings.push(ImportFinding {
				source_path: format!("{source_path}.litellm_params.model"),
				status: ImportStatus::Unsupported,
				message: "LiteLLM model must be a string and was not emitted".to_string(),
			});
			return None;
		},
		None => public_name.clone(),
	};
	let credential_name = match params.remove("litellm_credential_name") {
		None => None,
		Some(Value::String(name)) if !name.is_empty() => Some(name),
		Some(_) => {
			findings.push(ImportFinding {
				source_path: format!("{source_path}.litellm_params.litellm_credential_name"),
				status: ImportStatus::Unsupported,
				message: "LiteLLM credential reference must be a non-empty string and was not applied"
					.to_string(),
			});
			None
		},
	};
	let (provider_prefix, upstream_model) = split_provider(&model_id);
	for (path, pattern, configured_pattern) in [
		(
			format!("{source_path}.model_name"),
			public_name.as_str(),
			public_name.as_str(),
		),
		(
			format!("{source_path}.litellm_params.model"),
			upstream_model,
			model_id.as_str(),
		),
	] {
		if !is_supported_model_pattern(pattern) {
			findings.push(ImportFinding {
				source_path: path,
				status: ImportStatus::Unsupported,
				message: format!(
					"LiteLLM model wildcard pattern {configured_pattern:?} must contain at most one wildcard, at the beginning or end; the model was not emitted"
				),
			});
			return None;
		}
	}
	let Some(provider) = map_provider(provider_prefix) else {
		findings.push(ImportFinding {
			source_path: format!("{source_path}.litellm_params.model"),
			status: ImportStatus::Unsupported,
			message: format!("LiteLLM provider {provider_prefix:?} is not supported by this importer"),
		});
		return None;
	};
	if upstream_model.contains('*') && upstream_model != "*" && upstream_model != public_name.as_str()
	{
		findings.push(ImportFinding {
			source_path: format!("{source_path}.litellm_params.model"),
			status: ImportStatus::Unsupported,
			message: format!(
				"LiteLLM upstream wildcard rewrite from public model pattern {public_name:?} to upstream model pattern {model_id:?} cannot be represented safely and the model was not emitted"
			),
		});
		return None;
	}

	let mut output_params = Map::new();
	if !upstream_model.contains('*') {
		output_params.insert("model".to_string(), json!(upstream_model));
	}
	move_litellm_provider_params(&mut params, &mut output_params);
	let organization = take_litellm_organization(
		&mut params,
		&format!("{source_path}.litellm_params.organization"),
		findings,
	);
	let extra_headers = take_litellm_extra_headers(
		&mut params,
		&format!("{source_path}.litellm_params"),
		findings,
	);
	findings.extend(extra_headers.exact_findings.iter().cloned());
	let model_upstream_headers = LiteLlmUpstreamHeaders {
		organization,
		extra_headers,
	};
	let request_headers = request_headers_for_provider(
		provider,
		model_upstream_headers.organization.value.as_deref(),
		&model_upstream_headers.extra_headers.values,
		&format!("{source_path}.litellm_params.organization"),
		findings,
	);

	let rpm = params.remove("rpm");
	let tpm = params.remove("tpm");
	if tpm.is_some() {
		findings.push(ImportFinding {
			source_path: format!("{source_path}.litellm_params.tpm"),
			status: ImportStatus::Manual,
			message: "TPM capacity is not mapped automatically".to_string(),
		});
	}
	let parsed_rpm = rpm
		.as_ref()
		.and_then(Value::as_u64)
		.and_then(|value| usize::try_from(value).ok())
		.filter(|value| *value > 0);
	let weight = if use_rpm_weights && tpm.is_none() {
		parsed_rpm.unwrap_or(1)
	} else {
		1
	};
	if rpm.is_some() {
		let (status, message) = if parsed_rpm.is_none() {
			(
				ImportStatus::Manual,
				"RPM must be a positive integer and was not mapped".to_string(),
			)
		} else if !use_rpm_weights {
			(
				ImportStatus::Manual,
				"RPM capacity was not converted because the routing strategy is not simple-shuffle"
					.to_string(),
			)
		} else if tpm.is_some() {
			(
				ImportStatus::Manual,
				"RPM was not converted because the deployment also specifies unmapped TPM capacity"
					.to_string(),
			)
		} else {
			(
				ImportStatus::Approximate,
				"Used RPM as the relative weight for generated weighted routes".to_string(),
			)
		};
		findings.push(ImportFinding {
			source_path: format!("{source_path}.litellm_params.rpm"),
			status,
			message,
		});
	}
	let mut defaults = Map::new();
	for key in [
		"temperature",
		"max_tokens",
		"max_completion_tokens",
		"top_p",
		"frequency_penalty",
		"presence_penalty",
		"seed",
		"stop",
	] {
		if let Some(value) = params.remove(key) {
			defaults.insert(key.to_string(), normalize_environment_references(value));
		}
	}
	report_unmapped_fields(
		findings,
		&format!("{source_path}.litellm_params"),
		&params,
		ImportStatus::Manual,
		"LiteLLM parameter requires manual review and was not emitted",
	);
	findings.push(ImportFinding {
		source_path: source_path.to_string(),
		status: ImportStatus::Exact,
		message: format!("Imported model deployment for {public_name}"),
	});
	Some((
		public_name,
		ImportedModel {
			name,
			provider: ImportedModelProvider::Inline(provider.to_string()),
			params: output_params,
			defaults,
			request_headers,
			request_headers_override_defaults: false,
			weight,
		},
		credential_name,
		model_upstream_headers,
	))
}

fn split_provider(model: &str) -> (&str, &str) {
	match model.split_once('/') {
		Some((provider, model)) => (provider, model),
		None => ("openai", model),
	}
}

fn is_supported_model_pattern(pattern: &str) -> bool {
	let wildcard_count = pattern
		.chars()
		.filter(|character| *character == '*')
		.count();
	wildcard_count == 0
		|| (wildcard_count == 1
			&& (pattern == "*" || pattern.starts_with('*') || pattern.ends_with('*')))
}

fn map_provider(provider: &str) -> Option<&'static str> {
	match provider.to_ascii_lowercase().as_str() {
		"openai" | "text-completion-openai" => Some("openAI"),
		"azure" | "azure_ai" => Some("azure"),
		"anthropic" => Some("anthropic"),
		"bedrock" => Some("bedrock"),
		"gemini" => Some("gemini"),
		"vertex_ai" | "vertex_ai_beta" | "vertex" => Some("vertex"),
		"ollama" => Some("ollama"),
		"cohere" => Some("cohere"),
		"huggingface" => Some("huggingface"),
		"groq" => Some("groq"),
		"mistral" => Some("mistral"),
		"openrouter" => Some("openrouter"),
		"together_ai" | "togetherai" => Some("togetherai"),
		"xai" => Some("xai"),
		"deepinfra" => Some("deepinfra"),
		"deepseek" => Some("deepseek"),
		"fireworks_ai" | "fireworks" => Some("fireworks"),
		_ => None,
	}
}

fn move_litellm_provider_params(source: &mut Map<String, Value>, target: &mut Map<String, Value>) {
	move_string(source, "api_base", target, "baseUrl");
	move_string(source, "base_url", target, "baseUrl");
	move_string(source, "api_version", target, "azureApiVersion");
	move_string(source, "aws_region_name", target, "awsRegion");
	move_string(source, "vertex_project", target, "vertexProject");
	move_string(source, "vertex_location", target, "vertexRegion");
	if let Some(api_key) = source.remove("api_key") {
		target.insert(
			"apiKey".to_string(),
			normalize_environment_references(api_key),
		);
	}
}

fn take_litellm_extra_headers(
	source: &mut Map<String, Value>,
	source_prefix: &str,
	findings: &mut Vec<ImportFinding>,
) -> LiteLlmExtraHeaders {
	let Some(value) = source.remove("extra_headers") else {
		return LiteLlmExtraHeaders::default();
	};
	let Value::Object(headers) = value else {
		findings.push(ImportFinding {
			source_path: format!("{source_prefix}.extra_headers"),
			status: ImportStatus::Manual,
			message: "LiteLLM extra_headers must be an object of fixed string values and was not emitted"
				.to_string(),
		});
		return LiteLlmExtraHeaders {
			present: true,
			..Default::default()
		};
	};
	if headers.is_empty() {
		return LiteLlmExtraHeaders {
			present: true,
			values: IndexMap::new(),
			exact_findings: vec![ImportFinding {
				source_path: format!("{source_prefix}.extra_headers"),
				status: ImportStatus::Exact,
				message: "Consumed empty LiteLLM extra_headers".to_string(),
			}],
		};
	}

	let mut parsed = Vec::new();
	let mut names = IndexMap::<String, Vec<usize>>::new();
	for (name, value) in headers {
		let source_path = format!("{source_prefix}.extra_headers.{name}");
		let Ok(header_name) = ::http::HeaderName::from_bytes(name.as_bytes()) else {
			findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM extra header name is not a valid HTTP header name and was not emitted"
					.to_string(),
			});
			continue;
		};
		let Value::String(value) = normalize_environment_references(value) else {
			findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM extra header value must be a string and was not emitted".to_string(),
			});
			continue;
		};
		if value.parse::<::http::HeaderValue>().is_err() {
			findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM extra header value is not a valid HTTP header value and was not emitted"
					.to_string(),
			});
			continue;
		}
		let normalized_name = header_name.as_str().to_string();
		let index = parsed.len();
		names
			.entry(normalized_name.clone())
			.or_default()
			.push(index);
		parsed.push((source_path, normalized_name, value));
	}

	let mut result = IndexMap::new();
	let mut exact_findings = Vec::new();
	for indexes in names.values() {
		let first_value = &parsed[indexes[0]].2;
		let conflicting = indexes
			.iter()
			.skip(1)
			.any(|index| parsed[*index].2 != *first_value);
		for index in indexes {
			let (source_path, name, value) = &parsed[*index];
			if conflicting {
				findings.push(ImportFinding {
					source_path: source_path.clone(),
					status: ImportStatus::Manual,
					message: format!(
						"Conflicting case-insensitive values for LiteLLM header {name:?} were not emitted"
					),
				});
			} else {
				result.insert(name.clone(), value.clone());
				exact_findings.push(ImportFinding {
					source_path: source_path.clone(),
					status: ImportStatus::Exact,
					message: format!("Mapped fixed upstream header {name:?}"),
				});
			}
		}
	}
	LiteLlmExtraHeaders {
		present: true,
		values: result,
		exact_findings,
	}
}

fn take_litellm_organization(
	source: &mut Map<String, Value>,
	source_path: &str,
	findings: &mut Vec<ImportFinding>,
) -> LiteLlmOrganization {
	let Some(value) = source.remove("organization") else {
		return LiteLlmOrganization::default();
	};
	let Value::String(value) = normalize_environment_references(value) else {
		findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Manual,
			message: "LiteLLM organization must be a string and was not emitted".to_string(),
		});
		return LiteLlmOrganization {
			present: true,
			value: None,
		};
	};
	if value.is_empty() {
		findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Manual,
			message:
				"Empty LiteLLM organization may resolve from process-level defaults and was not emitted"
					.to_string(),
		});
		return LiteLlmOrganization {
			present: true,
			value: None,
		};
	};
	if value.parse::<::http::HeaderValue>().is_err() {
		findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Manual,
			message: "LiteLLM organization is not a valid HTTP header value and was not emitted"
				.to_string(),
		});
		return LiteLlmOrganization {
			present: true,
			value: None,
		};
	}
	LiteLlmOrganization {
		present: true,
		value: Some(value),
	}
}

fn request_headers_for_provider(
	provider: &str,
	organization: Option<&str>,
	extra_headers: &IndexMap<String, String>,
	source_path: &str,
	findings: &mut Vec<ImportFinding>,
) -> IndexMap<String, String> {
	if organization.is_some() {
		if provider == "openAI" {
			findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Exact,
				message: "Mapped OpenAI organization to the upstream OpenAI-Organization header"
					.to_string(),
			});
		} else {
			findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Manual,
				message: format!(
					"LiteLLM organization was not emitted because provider {provider:?} is not OpenAI"
				),
			});
		}
	}
	build_request_headers(provider, organization, extra_headers)
}

fn build_request_headers(
	provider: &str,
	organization: Option<&str>,
	extra_headers: &IndexMap<String, String>,
) -> IndexMap<String, String> {
	let mut headers = IndexMap::new();
	if provider == "openAI"
		&& let Some(organization) = organization
	{
		headers.insert("openai-organization".to_string(), organization.to_string());
	}
	// OpenAI applies per-request extra headers after its organization default, so a
	// fixed OpenAI-Organization entry in extra_headers intentionally wins here.
	for (name, value) in extra_headers {
		headers.insert(name.clone(), value.clone());
	}
	headers
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

fn apply_fallbacks(value: &Value, source_path: &str, plan: &mut ImportPlan) {
	let Some(entries) = value.as_array() else {
		plan.findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Unsupported,
			message: "Fallbacks must be a list of model mappings".to_string(),
		});
		return;
	};
	for (index, entry) in entries.iter().enumerate() {
		let Some(entry) = entry.as_object() else {
			plan.findings.push(ImportFinding {
				source_path: format!("{source_path}[{index}]"),
				status: ImportStatus::Unsupported,
				message: "Fallback entry must be a model mapping and was not emitted".to_string(),
			});
			continue;
		};
		for (source, fallback_names) in entry {
			if !plan.routes.contains_key(source) {
				plan.findings.push(ImportFinding {
					source_path: source_path.to_string(),
					status: ImportStatus::Manual,
					message: format!("Fallback source model {source:?} was not imported"),
				});
				continue;
			}
			let Some(fallback_names) = fallback_names.as_array() else {
				plan.findings.push(ImportFinding {
					source_path: format!("{source_path}[{index}].{source}"),
					status: ImportStatus::Unsupported,
					message: "Fallback targets must be a list and were not emitted".to_string(),
				});
				continue;
			};
			let mut seen = HashSet::new();
			let mut resolved_groups = Vec::new();
			for fallback_name in fallback_names.iter().filter_map(Value::as_str) {
				if !seen.insert(fallback_name) {
					continue;
				}
				if let Some(fallback) = plan.routes.get(fallback_name) {
					resolved_groups.push(fallback.targets.clone());
				} else {
					plan.findings.push(ImportFinding {
						source_path: source_path.to_string(),
						status: ImportStatus::Manual,
						message: format!("Fallback target model {fallback_name:?} was not imported"),
					});
				}
			}
			plan
				.routes
				.get_mut(source)
				.expect("source route checked above")
				.fallback_groups
				.extend(resolved_groups);
		}
	}
	plan.findings.push(ImportFinding {
		source_path: source_path.to_string(),
		status: ImportStatus::Approximate,
		message: "Mapped ordinary LiteLLM fallbacks to agentgateway priority-based failover"
			.to_string(),
	});
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

#[cfg(test)]
mod tests {
	use std::fs;
	use std::path::{Path, PathBuf};

	use super::*;

	fn fixture_path(name: &str) -> PathBuf {
		Path::new("src/tests/import/litellm").join(format!("{name}.yaml"))
	}

	fn assert_litellm_golden(name: &str) {
		let input_path = fixture_path(name);
		let input = fs::read_to_string(&input_path)
			.unwrap_or_else(|error| panic!("{}: {error}", input_path.display()));
		let result = import_config("litellm", &input)
			.unwrap_or_else(|error| panic!("{}: {error}", input_path.display()));

		insta::with_settings!({
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests/import/litellm",
		}, {
			insta::assert_yaml_snapshot!(name, result);
		});
	}

	#[test]
	fn imports_litellm_models_load_balancing_and_fallbacks() {
		assert_litellm_golden("load-balancing-fallbacks");
	}

	#[test]
	fn reports_unsupported_provider_without_emitting_invalid_route() {
		assert_litellm_golden("unsupported-provider");
	}

	#[test]
	fn reports_unmapped_top_level_and_model_fields() {
		assert_litellm_golden("unmapped-fields");
	}

	#[test]
	fn normalizes_environment_references_in_all_mapped_values() {
		assert_litellm_golden("environment-references");
	}

	#[test]
	fn does_not_apply_capacity_weights_to_incompatible_routing() {
		assert_litellm_golden("incompatible-routing");
	}

	#[test]
	fn imports_safe_wildcard_models_and_reports_unsupported_patterns() {
		assert_litellm_golden("wildcard-model");
	}

	#[test]
	fn reports_litellm_database_settings_without_reusing_them() {
		assert_litellm_golden("database-settings");
	}

	#[test]
	fn imports_reusable_litellm_credentials() {
		assert_litellm_golden("centralized-credentials");
	}

	#[test]
	fn imports_openai_organizations_and_fixed_upstream_headers() {
		assert_litellm_golden("openai-compatible-headers");
	}

	#[test]
	fn reports_invalid_or_ambiguous_upstream_headers() {
		assert_litellm_golden("invalid-upstream-headers");
	}

	#[test]
	fn reports_credential_conflicts_without_dropping_model_overrides() {
		assert_litellm_golden("credential-conflicts");
	}

	#[test]
	fn keeps_routing_for_a_single_model_with_fallbacks() {
		assert_litellm_golden("single-model-fallback");
	}

	#[test]
	fn lists_registered_sources_in_stable_order() {
		assert_eq!(available_sources(), vec!["litellm"]);
	}
}
