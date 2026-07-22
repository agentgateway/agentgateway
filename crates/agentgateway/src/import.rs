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
	pub models: Vec<ImportedModel>,
	pub routes: IndexMap<String, ImportedRoute>,
	pub findings: Vec<ImportFinding>,
}

#[derive(Debug)]
pub struct ImportedModel {
	pub name: String,
	pub provider: String,
	pub params: Map<String, Value>,
	pub defaults: Map<String, Value>,
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

pub fn available_sources() -> Vec<&'static str> {
	importers()
		.iter()
		.map(|importer| importer.source())
		.collect()
}

pub fn import_config(source: &str, input: &str) -> anyhow::Result<ImportResult> {
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
	emit(importer.source(), plan)
}

fn importers() -> Vec<Box<dyn ConfigImporter>> {
	vec![Box::new(LiteLlmImporter)]
}

fn emit(source: &str, plan: ImportPlan) -> anyhow::Result<ImportResult> {
	let ImportPlan {
		models,
		routes,
		findings,
	} = plan;
	let model_by_name: HashMap<_, _> = models
		.iter()
		.map(|model| (model.name.as_str(), model))
		.collect();
	let emitted_models = models
		.iter()
		.map(|model| {
			let mut value = Map::from_iter([
				("name".to_string(), json!(model.name)),
				("visibility".to_string(), json!("internal")),
				("provider".to_string(), json!(model.provider)),
				("params".to_string(), Value::Object(model.params.clone())),
			]);
			if !model.defaults.is_empty() {
				value.insert(
					"defaults".to_string(),
					Value::Object(model.defaults.clone()),
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
		let routing = if route.fallback_groups.is_empty() {
			let targets = route
				.targets
				.iter()
				.map(|target| {
					let weight = model_by_name.get(target.as_str()).map_or(1, |m| m.weight);
					json!({"model": target, "weight": weight})
				})
				.collect::<Vec<_>>();
			json!({"weighted": {"targets": targets}})
		} else {
			let mut targets = route
				.targets
				.iter()
				.map(|target| json!({"model": target, "priority": 0}))
				.collect::<Vec<_>>();
			for (priority, fallback_group) in route.fallback_groups.iter().enumerate() {
				for fallback in fallback_group {
					targets.push(json!({"model": fallback, "priority": priority + 1}));
				}
			}
			json!({"failover": {"targets": targets}})
		};
		virtual_models.push(json!({"name": name, "routing": routing}));
	}

	let config = json!({
		"llm": {
			"port": 4000,
			"models": emitted_models,
			"virtualModels": virtual_models,
		}
	});
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
			let Some(imported) = import_litellm_model(
				internal_name.clone(),
				model,
				&source_path,
				use_rpm_weights,
				&mut plan.findings,
			) else {
				continue;
			};
			plan
				.routes
				.entry(imported.0)
				.or_default()
				.targets
				.push(internal_name);
			plan.models.push(imported.1);
		}

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
			report_unmapped_fields(
				&mut plan.findings,
				section,
				values,
				ImportStatus::Manual,
				"Requires manual review and was not emitted",
			);
		}
		for setting in config.litellm_settings.keys() {
			if setting == "fallbacks" {
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

fn import_litellm_model(
	name: String,
	model: LiteLlmModel,
	source_path: &str,
	use_rpm_weights: bool,
	findings: &mut Vec<ImportFinding>,
) -> Option<(String, ImportedModel)> {
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
	if public_name.contains('*') || model_id.contains('*') {
		let wildcard_path = if public_name.contains('*') {
			format!("{source_path}.model_name")
		} else {
			format!("{source_path}.litellm_params.model")
		};
		findings.push(ImportFinding {
			source_path: wildcard_path,
			status: ImportStatus::Unsupported,
			message: "LiteLLM wildcard models are not yet supported and were not emitted".to_string(),
		});
		return None;
	}
	let (provider_prefix, upstream_model) = split_provider(&model_id);
	let Some(provider) = map_provider(provider_prefix) else {
		findings.push(ImportFinding {
			source_path: format!("{source_path}.litellm_params.model"),
			status: ImportStatus::Unsupported,
			message: format!("LiteLLM provider {provider_prefix:?} is not supported by this importer"),
		});
		return None;
	};

	let mut output_params = Map::new();
	output_params.insert("model".to_string(), json!(upstream_model));
	move_string(&mut params, "api_base", &mut output_params, "baseUrl");
	move_string(&mut params, "base_url", &mut output_params, "baseUrl");
	move_string(
		&mut params,
		"api_version",
		&mut output_params,
		"azureApiVersion",
	);
	move_string(
		&mut params,
		"aws_region_name",
		&mut output_params,
		"awsRegion",
	);
	move_string(
		&mut params,
		"vertex_project",
		&mut output_params,
		"vertexProject",
	);
	move_string(
		&mut params,
		"vertex_location",
		&mut output_params,
		"vertexRegion",
	);
	if let Some(api_key) = params.remove("api_key") {
		output_params.insert(
			"apiKey".to_string(),
			normalize_environment_references(api_key),
		);
	}

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
			provider: provider.to_string(),
			params: output_params,
			defaults,
			weight,
		},
	))
}

fn split_provider(model: &str) -> (&str, &str) {
	match model.split_once('/') {
		Some((provider, model)) => (provider, model),
		None => ("openai", model),
	}
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
	fn reports_wildcard_models_without_emitting_malformed_names() {
		assert_litellm_golden("wildcard-model");
	}

	#[test]
	fn lists_registered_sources_in_stable_order() {
		assert_eq!(available_sources(), vec!["litellm"]);
	}
}
