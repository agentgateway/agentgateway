use std::collections::{HashMap, HashSet};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::import::{
	ConfigImporter, ImportFinding, ImportOptions, ImportPlan, ImportStatus, ImportedInboundAPIKey,
	ImportedModel, ImportedModelProvider, ImportedProvider, ImportedRoute, imported_model_name,
	move_string, normalize_environment_references, report_unmapped_fields, sanitize_mcp_target_name,
	unique_imported_provider_name,
};
use crate::types::local::McpBackendHost;

pub(super) struct LiteLlmImporter;

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
	#[serde(default)]
	guardrails: Vec<Value>,
	#[serde(default)]
	mcp_servers: Map<String, Value>,
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

	fn import(&self, input: &str, options: &ImportOptions) -> anyhow::Result<ImportPlan> {
		let config: LiteLlmConfig =
			crate::yaml::from_str(input).context("invalid LiteLLM configuration")?;
		if config.model_list.is_empty() && config.mcp_servers.is_empty() {
			bail!("LiteLLM configuration does not contain any model_list or mcp_servers entries");
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

		let router_default_fallbacks = config.router_settings.get("default_fallbacks");
		let litellm_default_fallbacks = config.litellm_settings.get("default_fallbacks");
		if router_default_fallbacks.is_some() && litellm_default_fallbacks.is_some() {
			plan.findings.push(ImportFinding {
				source_path: "litellm_settings.default_fallbacks".to_string(),
				status: ImportStatus::Manual,
				message:
					"litellm_settings.default_fallbacks is shadowed by router_settings.default_fallbacks; the router_settings value was applied"
						.to_string(),
			});
		}
		let default_fallbacks = router_default_fallbacks
			.map(|default_fallbacks| ("router_settings.default_fallbacks", default_fallbacks))
			.or_else(|| {
				litellm_default_fallbacks
					.map(|default_fallbacks| ("litellm_settings.default_fallbacks", default_fallbacks))
			});
		let max_fallbacks = config
			.router_settings
			.get("max_fallbacks")
			.and_then(Value::as_u64)
			.map(|max_fallbacks| max_fallbacks as usize)
			.unwrap_or(DEFAULT_MAX_FALLBACKS);
		let mut removed_fallback_targets = false;
		if max_fallbacks == 0 {
			// LiteLLM raises before attempting any fallback group, so no failover survives, not even
			// the explicitly configured groups applied above.
			for route in plan.routes.values_mut() {
				if !route.fallback_groups.is_empty() {
					route.fallback_groups = Vec::new();
					removed_fallback_targets = true;
				}
			}
		}
		if let Some((source_path, default_fallbacks)) = default_fallbacks {
			apply_default_fallbacks(
				default_fallbacks,
				source_path,
				fallbacks.map(|(_, fallbacks)| fallbacks),
				max_fallbacks,
				&mut plan,
			);
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
		// Resolved before pass-through endpoints and MCP servers, both of which reuse
		// plan.inbound_api_key rather than re-deriving master_key themselves.
		apply_litellm_inbound_api_key(
			&config.general_settings,
			&config.environment_variables,
			&mut plan,
		)?;
		apply_litellm_health(&config.router_settings, &mut plan);
		import_litellm_timeout(&config.router_settings, &mut plan);
		import_litellm_pass_through_endpoints(&config.general_settings, &mut plan);
		import_litellm_guardrails(&config.guardrails, &mut plan);
		import_litellm_mcp_servers(&config.mcp_servers, &config.litellm_settings, &mut plan);
		for setting in config.router_settings.keys() {
			match setting.as_str() {
				"fallbacks"
				| "routing_strategy"
				| "default_fallbacks"
				| "allowed_fails"
				| "allowed_fails_policy"
				| "cooldown_time"
				| "disable_cooldowns"
				| "timeout" => continue,
				"max_fallbacks" => {
					let (status, message) = match config.router_settings.get(setting).and_then(Value::as_u64)
					{
						None => (
							ImportStatus::Manual,
							"max_fallbacks must be a non-negative integer; the LiteLLM default of 5 was assumed"
								.to_string(),
						),
						Some(0) if removed_fallback_targets => (
							ImportStatus::Exact,
							"max_fallbacks: 0 disables LiteLLM fallbacks; emitted failover targets were removed"
								.to_string(),
						),
						Some(0) => (
							ImportStatus::Exact,
							"max_fallbacks: 0 disables LiteLLM fallbacks".to_string(),
						),
						Some(_) if default_fallbacks.is_none() => (
							ImportStatus::Manual,
							"No automatic mapping is available; review this router setting".to_string(),
						),
						Some(_) => (
							ImportStatus::Exact,
							"max_fallbacks bounds the flattened failover chain length".to_string(),
						),
					};
					plan.findings.push(ImportFinding {
						source_path: format!("router_settings.{setting}"),
						status,
						message,
					});
				},
				"context_window_fallbacks" | "content_policy_fallbacks" => {
					plan.findings.push(ImportFinding {
						source_path: format!("router_settings.{setting}"),
						status: ImportStatus::Manual,
						message: format!(
							"LiteLLM {setting} is conditional on specific error types and cannot be represented by agentgateway's unconditional priority failover; configure equivalent behavior manually"
						),
					});
				},
				"cache_responses" => {
					report_litellm_response_cache(
						&mut plan.findings,
						&format!("router_settings.{setting}"),
						&config.router_settings[setting],
					);
				},
				"redis_url" | "redis_host" | "redis_port" | "redis_password" | "redis_db"
				| "cache_kwargs" => {
					report_litellm_cache_value(
						&mut plan.findings,
						&format!("router_settings.{setting}"),
						&config.router_settings[setting],
						ImportStatus::Manual,
						"LiteLLM router Redis coordination settings were not emitted; review shared routing, cooldown, and usage state separately from response caching. These settings can also configure the backend when router response caching is enabled",
					);
				},
				"caching_groups" => {
					report_litellm_cache_value(
						&mut plan.findings,
						&format!("router_settings.{setting}"),
						&config.router_settings[setting],
						ImportStatus::Manual,
						"LiteLLM response-cache sharing across model groups was not preserved; review cache-key isolation when configuring response caching manually",
					);
				},
				_ => {
					plan.findings.push(ImportFinding {
						source_path: format!("router_settings.{setting}"),
						status: ImportStatus::Manual,
						message: "No automatic mapping is available; review this router setting".to_string(),
					});
				},
			}
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
				if section == "general_settings"
					&& matches!(
						setting.as_str(),
						"pass_through_endpoints" | "master_key" | "litellm_key_header_name"
					) {
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
			if matches!(
				setting.as_str(),
				"fallbacks" | "default_fallbacks" | "mcp_aliases"
			) {
				continue;
			}
			if setting == "cache" {
				report_litellm_response_cache(
					&mut plan.findings,
					"litellm_settings.cache",
					&config.litellm_settings[setting],
				);
				continue;
			}
			if setting == "cache_params" {
				report_litellm_cache_params(&mut plan.findings, &config.litellm_settings[setting]);
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
		if let Some(state) = &options.db_state {
			import_litellm_db_state(state, options, &mut plan);
		}
		finalize_litellm_plan(&mut plan);
		Ok(plan)
	}
}

fn report_litellm_response_cache(
	findings: &mut Vec<ImportFinding>,
	source_path: &str,
	value: &Value,
) {
	let message = match value.as_bool() {
		Some(true) => {
			"LiteLLM response caching was requested but was not preserved; configure a response-cache solution manually before relying on cached responses. Provider promptCaching is not equivalent to caching complete responses"
		},
		Some(false) => {
			"This LiteLLM response-cache flag is false and was not emitted; review other response-cache settings independently"
		},
		None => {
			"Expected a boolean response-cache flag; its value was not resolved or emitted. Review response caching manually"
		},
	};
	report_litellm_cache_value(findings, source_path, value, ImportStatus::Manual, message);
}

const CACHE_BACKEND_GUIDANCE: &str = "LiteLLM response-cache backend selection was not emitted; choose a response-cache solution manually. Provider promptCaching does not cache complete responses";

const CACHE_CONNECTION_GUIDANCE: &str = "LiteLLM response-cache connection or authentication settings were not emitted; configure connectivity and secret references in the chosen cache solution manually";

const CACHE_EXPIRATION_GUIDANCE: &str = "LiteLLM response-cache expiration was not preserved; review TTL units and backend defaults when configuring cache lifetime manually";

const CACHE_NAMESPACE_GUIDANCE: &str = "LiteLLM response-cache namespace was not preserved; configure cache-key isolation manually before sharing a cache backend";

const CACHE_OPERATION_GUIDANCE: &str = "LiteLLM response-cache operation selection was not preserved; review which API operations may reuse cached responses";

const CACHE_MODE_GUIDANCE: &str = "LiteLLM response-cache opt-in or opt-out behavior was not preserved; review which requests may read from or write to the cache";

fn cache_parameter_guidance(key: &str) -> Option<&'static str> {
	match key {
		"type" => Some(CACHE_BACKEND_GUIDANCE),
		"host" | "port" | "url" | "password" => Some(CACHE_CONNECTION_GUIDANCE),
		"ttl" | "default_in_memory_ttl" | "default_in_redis_ttl" => Some(CACHE_EXPIRATION_GUIDANCE),
		"namespace" => Some(CACHE_NAMESPACE_GUIDANCE),
		"supported_call_types" => Some(CACHE_OPERATION_GUIDANCE),
		"mode" => Some(CACHE_MODE_GUIDANCE),
		_ => None,
	}
}

fn report_litellm_cache_params(findings: &mut Vec<ImportFinding>, value: &Value) {
	let source_path = "litellm_settings.cache_params";
	let Some(params) = value.as_object() else {
		report_litellm_cache_value(
			findings,
			source_path,
			value,
			ImportStatus::Manual,
			"Expected a response-cache parameter mapping; it was not resolved or emitted. Review response caching manually",
		);
		return;
	};
	if params.is_empty() {
		report_litellm_cache_value(
			findings,
			source_path,
			value,
			ImportStatus::Manual,
			"Empty LiteLLM response-cache parameters were not emitted; review the source cache defaults before configuring response caching manually",
		);
	}
	for (key, value) in params {
		let message = cache_parameter_guidance(key);
		let (status, message) = match message {
			Some(message) => (ImportStatus::Manual, message),
			None => (
				ImportStatus::Unsupported,
				"No migration guidance is available for this response-cache parameter; it was not emitted. Review it in the chosen cache solution",
			),
		};
		report_litellm_cache_value(
			findings,
			&format!("{source_path}.{key}"),
			value,
			status,
			message,
		);
	}
}

fn report_litellm_cache_value(
	findings: &mut Vec<ImportFinding>,
	source_path: &str,
	value: &Value,
	status: ImportStatus,
	message: &str,
) {
	findings.push(ImportFinding {
		source_path: source_path.to_string(),
		status,
		message: message.to_string(),
	});
	match value {
		Value::Object(values) => {
			for (key, value) in values {
				report_litellm_cache_value(
					findings,
					&format!("{source_path}.{key}"),
					value,
					status,
					message,
				);
			}
		},
		Value::Array(values) => {
			for (index, value) in values.iter().enumerate() {
				report_litellm_cache_value(
					findings,
					&format!("{source_path}[{index}]"),
					value,
					status,
					message,
				);
			}
		},
		_ => {},
	}
}

/// Resolves two LiteLLM-specific decisions that the shared emitter can't make on its own, since
/// they depend on what LiteLLM itself requires rather than on the target schema:
///
/// - LiteLLM requires a key for `/mcp`. If no inbound key was emitted (e.g. an invalid
///   `litellm_key_header_name`), drop the migrated MCP target(s) here, before the plan ever
///   reaches the emitter, rather than mirror a policy that doesn't exist onto an otherwise-open
///   gateway - matching how a DB-restricted server is already dropped rather than exposed
///   unprotected. If an inbound key does exist, mirror it onto the MCP gateway the same way.
/// - A plan with no imported models never gets an `llm:` block (standalone configuration has no
///   use for one with zero models), so any finding that already claimed something was mapped
///   into it (the timeout policy, the master_key policy) would be describing fields that were
///   never actually emitted. Retract those and record why nothing landed instead.
fn finalize_litellm_plan(plan: &mut ImportPlan) {
	if !plan.mcp_targets.is_empty() {
		let has_db_state_mcp_auth =
			!plan.mcp_api_keys.is_empty() || !plan.mcp_authorization_rules.is_empty();
		if has_db_state_mcp_auth {
			// DB-state already protects these targets with its own per-key auth and
			// mcpAuthorization rules (including its own master-key bypass rule), and
			// import_litellm_db_state already explains that via its own findings - nothing
			// to add or retract here.
		} else if plan.inbound_api_key.is_some() {
			plan.findings.push(ImportFinding {
				source_path: "general_settings.master_key".to_string(),
				status: ImportStatus::Approximate,
				message: "Mirrored the inbound API-key policy onto the MCP gateway so /mcp also requires master_key; LiteLLM's own DB-managed per-key MCP grants were not available here and were not applied".to_string(),
			});
		} else {
			// None of these servers will actually be emitted, so drop every per-server finding
			// that claimed otherwise - each would contradict the single "not emitted" finding
			// that follows.
			plan.findings.retain(|finding| {
				!finding
					.message
					.starts_with("Mapped LiteLLM MCP server to agentgateway target")
					&& !finding
						.message
						.starts_with("Multiple MCP servers are multiplexed")
					&& !finding.message.starts_with("Mapped static_headers")
					&& !finding.message.starts_with("Renamed ")
			});
			plan.findings.push(ImportFinding {
				source_path: "mcp_servers".to_string(),
				status: ImportStatus::Unsupported,
				message: "No inbound API key was emitted, so the migrated MCP target(s) could not be protected and were not emitted".to_string(),
			});
			plan.mcp_targets.clear();
		}
	}

	if plan.models.is_empty() {
		let had_timeout = plan.timeout.is_some();
		let had_guardrails = plan.guardrails.is_some();
		let had_inbound_api_key = plan.inbound_api_key.is_some();
		let had_db_api_keys = !plan.db_api_keys.is_empty();
		plan.findings.retain(|finding| {
			!finding
				.message
				.starts_with("Mapped LiteLLM router_settings.timeout")
				&& !finding.message.starts_with("Mapped LiteLLM's master_key")
				&& !finding.message.contains("against every configured model")
				&& !finding.message.starts_with("Migrated key ")
		});
		// Only claim these weren't emitted anywhere if they genuinely weren't: when MCP targets
		// exist, both the inbound key and any DB-state virtual keys are already mirrored onto
		// mcp.policies.apiKey above, and that path's own findings already say so - pushing these
		// too would contradict them.
		if had_db_api_keys && plan.mcp_targets.is_empty() {
			plan.findings.push(ImportFinding {
				source_path: "db_state.keys".to_string(),
				status: ImportStatus::Unsupported,
				message: "No models were imported, so there is no llm: block for these DB-state virtual keys to attach to, and they were not migrated"
					.to_string(),
			});
		}
		if had_timeout {
			plan.findings.push(ImportFinding {
				source_path: "router_settings.timeout".to_string(),
				status: ImportStatus::Unsupported,
				message: "No models were imported, so there is no llm: block for this timeout policy to attach to, and it was not emitted"
					.to_string(),
			});
		}
		if had_inbound_api_key && plan.mcp_targets.is_empty() {
			plan.findings.push(ImportFinding {
				source_path: "general_settings.master_key".to_string(),
				status: ImportStatus::Unsupported,
				message: "No models were imported, so there is no llm: block for this inbound apiKey policy to attach to, and it was not emitted"
					.to_string(),
			});
		}
		if had_guardrails {
			plan.findings.push(ImportFinding {
				source_path: "guardrails".to_string(),
				status: ImportStatus::Unsupported,
				message: "No models were imported, so there is no llm: block for these guardrails to attach to, and they were not emitted"
					.to_string(),
			});
		}
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

/// Maps LiteLLM router-level cooldown settings onto an agentgateway health eviction policy.
///
/// LiteLLM cools a deployment down globally, whereas agentgateway attaches outlier detection to
/// each model backend, so the resolved policy is applied to every emitted model via
/// [`ImportPlan::model_health`]. Deployments with their own `cooldown_time` and exception-specific
/// `allowed_fails_policy` thresholds are left for manual migration.
fn apply_litellm_health(router_settings: &Map<String, Value>, plan: &mut ImportPlan) {
	if router_settings.contains_key("allowed_fails_policy") {
		plan.findings.push(ImportFinding {
			source_path: "router_settings.allowed_fails_policy".to_string(),
			status: ImportStatus::Manual,
			message: "LiteLLM exception-specific allowed_fails_policy thresholds conflict with the single model health eviction threshold and were not mapped"
				.to_string(),
		});
	}

	if let Some(disabled) = router_settings.get("disable_cooldowns") {
		match disabled.as_bool() {
			Some(true) => {
				// Priority failover enables eviction by default when no health policy is set, so
				// omitting `health` only matches LiteLLM when no fallbacks were imported.
				let has_failover = plan
					.routes
					.values()
					.any(|route| !route.fallback_groups.is_empty());
				let (status, message) = if has_failover {
					(
						ImportStatus::Manual,
						"LiteLLM cooldowns are disabled, but agentgateway enables eviction by default for priority failover routes; disable eviction on the generated fallback routes manually",
					)
				} else {
					(
						ImportStatus::Exact,
						"LiteLLM cooldowns are disabled; no model health eviction policy was emitted",
					)
				};
				plan.findings.push(ImportFinding {
					source_path: "router_settings.disable_cooldowns".to_string(),
					status,
					message: message.to_string(),
				});
				report_unmapped_litellm_health(
					router_settings,
					status,
					"LiteLLM cooldowns are disabled",
					&mut plan.findings,
				);
				return;
			},
			Some(false) => plan.findings.push(ImportFinding {
				source_path: "router_settings.disable_cooldowns".to_string(),
				status: ImportStatus::Exact,
				message: "LiteLLM cooldowns remain enabled".to_string(),
			}),
			None => {
				plan.findings.push(ImportFinding {
					source_path: "router_settings.disable_cooldowns".to_string(),
					status: ImportStatus::Manual,
					message:
						"LiteLLM disable_cooldowns must be a boolean; model health settings were not mapped"
							.to_string(),
				});
				report_unmapped_litellm_health(
					router_settings,
					ImportStatus::Manual,
					"disable_cooldowns is not a boolean",
					&mut plan.findings,
				);
				return;
			},
		}
	}

	if router_settings.contains_key("allowed_fails_policy") {
		report_unmapped_litellm_health(
			router_settings,
			ImportStatus::Manual,
			"allowed_fails_policy requires manual migration",
			&mut plan.findings,
		);
		return;
	}

	let allowed_fails = router_settings.get("allowed_fails");
	let cooldown_time = router_settings.get("cooldown_time");
	let (Some(allowed_fails), Some(cooldown_time)) = (allowed_fails, cooldown_time) else {
		if allowed_fails.is_some() {
			plan.findings.push(ImportFinding {
				source_path: "router_settings.allowed_fails".to_string(),
				status: ImportStatus::Manual,
				message: "LiteLLM allowed_fails requires cooldown_time before a model health eviction policy can be mapped"
					.to_string(),
			});
		}
		if cooldown_time.is_some() {
			plan.findings.push(ImportFinding {
				source_path: "router_settings.cooldown_time".to_string(),
				status: ImportStatus::Manual,
				message: "LiteLLM cooldown_time requires allowed_fails before a model health eviction policy can be mapped"
					.to_string(),
			});
		}
		return;
	};

	let consecutive_failures = allowed_fails
		.as_i64()
		.and_then(|value| i32::try_from(value).ok())
		.filter(|count| *count >= 0)
		.and_then(|count| count.checked_add(1));
	let cooldown_seconds = cooldown_time.as_i64().filter(|seconds| *seconds > 0);
	if consecutive_failures.is_none() || cooldown_seconds.is_none() {
		plan.findings.push(ImportFinding {
			source_path: "router_settings.allowed_fails".to_string(),
			status: ImportStatus::Manual,
			message: if consecutive_failures.is_none() {
				"LiteLLM allowed_fails must be a non-negative integer whose value plus one fits the agentgateway threshold and was not mapped"
					.to_string()
			} else {
				"LiteLLM allowed_fails was not mapped because cooldown_time is invalid".to_string()
			},
		});
		plan.findings.push(ImportFinding {
			source_path: "router_settings.cooldown_time".to_string(),
			status: ImportStatus::Manual,
			message: if cooldown_seconds.is_none() {
				"LiteLLM cooldown_time must be a positive integer number of seconds and was not mapped"
					.to_string()
			} else {
				"LiteLLM cooldown_time was not mapped because allowed_fails is invalid".to_string()
			},
		});
		return;
	}

	let consecutive_failures = consecutive_failures.expect("validated above");
	let cooldown_seconds = cooldown_seconds.expect("validated above");
	plan.model_health = Some(json!({
		"eviction": {
			"consecutiveFailures": consecutive_failures,
			"duration": format!("{cooldown_seconds}s"),
		}
	}));
	plan.findings.push(ImportFinding {
		source_path: "router_settings.allowed_fails".to_string(),
		status: ImportStatus::Approximate,
		message: "Mapped LiteLLM allowed_fails + 1 to the model health eviction consecutiveFailures threshold; agentgateway counts consecutive failures rather than failures within a window"
			.to_string(),
	});
	plan.findings.push(ImportFinding {
		source_path: "router_settings.cooldown_time".to_string(),
		status: ImportStatus::Approximate,
		message: "Mapped LiteLLM cooldown_time to the model health eviction duration".to_string(),
	});
}

/// Records `allowed_fails` and `cooldown_time` when they are consumed without emitting a policy.
fn report_unmapped_litellm_health(
	router_settings: &Map<String, Value>,
	status: ImportStatus,
	reason: &str,
	findings: &mut Vec<ImportFinding>,
) {
	for setting in ["allowed_fails", "cooldown_time"] {
		if router_settings.contains_key(setting) {
			findings.push(ImportFinding {
				source_path: format!("router_settings.{setting}"),
				status,
				message: format!("LiteLLM {setting} was not mapped because {reason}"),
			});
		}
	}
}

/// Maps LiteLLM's router-wide per-attempt timeout onto agentgateway's `llm.policies.timeout`.
///
/// Unlike model health, standalone configuration has no per-model timeout field, so this is the
/// only timeout attachment point in the `llm:` block and applies across every emitted model.
/// LiteLLM allows a per-deployment `litellm_params.timeout` override; agentgateway cannot
/// represent that, so deployments setting it are flagged separately in [`import_litellm_model`].
fn import_litellm_timeout(router_settings: &Map<String, Value>, plan: &mut ImportPlan) {
	let Some(timeout) = router_settings.get("timeout") else {
		return;
	};
	// Bounded well above any realistic request timeout so a huge or malformed value is reported
	// as a finding instead of producing a duration string the config loader cannot parse at all.
	const MAX_TIMEOUT_SECONDS: f64 = 86_400.0;
	let seconds = timeout
		.as_f64()
		.filter(|seconds| *seconds > 0.0 && *seconds <= MAX_TIMEOUT_SECONDS);
	let Some(seconds) = seconds else {
		plan.findings.push(ImportFinding {
			source_path: "router_settings.timeout".to_string(),
			status: ImportStatus::Manual,
			message: format!(
				"LiteLLM router timeout must be a positive number of seconds no greater than {MAX_TIMEOUT_SECONDS} and was not mapped"
			),
		});
		return;
	};
	plan.timeout = Some(json!({
		"backendRequestTimeout": format!("{seconds}s"),
	}));
	plan.findings.push(ImportFinding {
		source_path: "router_settings.timeout".to_string(),
		status: ImportStatus::Approximate,
		message: "Mapped LiteLLM router_settings.timeout to the single global llm.policies.timeout backendRequestTimeout; LiteLLM allows a per-deployment override that agentgateway cannot represent"
			.to_string(),
	});
}

/// Maps LiteLLM `general_settings.pass_through_endpoints` to standalone top-level `routes`
/// entries, outside the `llm:` block entirely. Each entry proxies one path to an arbitrary
/// upstream URL, rewriting the path when the target's own path differs from the route path.
///
/// A pass-through endpoint that requires auth reuses `plan.inbound_api_key` (resolved earlier,
/// from the same `general_settings.master_key`/`litellm_key_header_name` the inbound LLM/MCP
/// policies use) rather than re-deriving it, so all three never disagree about whether
/// `master_key` is usable or what header it's checked on.
fn import_litellm_pass_through_endpoints(
	general_settings: &Map<String, Value>,
	plan: &mut ImportPlan,
) {
	let Some(endpoints) = general_settings.get("pass_through_endpoints") else {
		return;
	};
	let Value::Array(endpoints) = endpoints else {
		plan.findings.push(ImportFinding {
			source_path: "general_settings.pass_through_endpoints".to_string(),
			status: ImportStatus::Manual,
			message: "LiteLLM pass_through_endpoints must be an array and was not emitted".to_string(),
		});
		return;
	};
	for (index, endpoint) in endpoints.iter().enumerate() {
		let source_path = format!("general_settings.pass_through_endpoints[{index}]");
		let Value::Object(endpoint) = endpoint else {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM pass-through endpoint must be an object and was not emitted".to_string(),
			});
			continue;
		};
		let mut endpoint = endpoint.clone();
		let path = match endpoint.remove("path") {
			Some(Value::String(path)) if path.starts_with('/') => path.clone(),
			Some(Value::String(_)) => {
				plan.findings.push(ImportFinding {
					source_path: format!("{source_path}.path"),
					status: ImportStatus::Manual,
					message: "LiteLLM pass-through path must start with '/' and was not emitted".to_string(),
				});
				continue;
			},
			_ => {
				plan.findings.push(ImportFinding {
					source_path: format!("{source_path}.path"),
					status: ImportStatus::Manual,
					message: "LiteLLM pass-through endpoint has no path and was not emitted".to_string(),
				});
				continue;
			},
		};
		let Some(Value::String(target)) = endpoint.remove("target") else {
			plan.findings.push(ImportFinding {
				source_path: format!("{source_path}.target"),
				status: ImportStatus::Manual,
				message: "LiteLLM pass-through endpoint has no target and was not emitted".to_string(),
			});
			continue;
		};
		let target_url = match url::Url::parse(&target) {
			Ok(url) => url,
			Err(error) => {
				plan.findings.push(ImportFinding {
					source_path: format!("{source_path}.target"),
					status: ImportStatus::Manual,
					message: format!(
						"LiteLLM pass-through target {target:?} is not a valid URL ({error}) and was not emitted"
					),
				});
				continue;
			},
		};
		let scheme = target_url.scheme();
		if scheme != "http" && scheme != "https" {
			plan.findings.push(ImportFinding {
				source_path: format!("{source_path}.target"),
				status: ImportStatus::Unsupported,
				message: format!(
					"LiteLLM pass-through target scheme {scheme:?} is not supported and was not emitted"
				),
			});
			continue;
		}
		let Some(host) = target_url.host_str() else {
			plan.findings.push(ImportFinding {
				source_path: format!("{source_path}.target"),
				status: ImportStatus::Manual,
				message: "LiteLLM pass-through target has no host and was not emitted".to_string(),
			});
			continue;
		};
		let port = target_url
			.port_or_known_default()
			.expect("http and https always have a known default port");
		let target_path = target_url.path();

		let mut route_policies = Map::new();
		if target_path != path && !target_path.is_empty() {
			route_policies.insert(
				"urlRewrite".to_string(),
				json!({"path": {"full": target_path}}),
			);
		}
		if scheme == "https" {
			route_policies.insert("backendTLS".to_string(), json!({}));
		}

		let auth = match endpoint.remove("auth") {
			None | Some(Value::Bool(false)) => false,
			Some(Value::Bool(true)) => true,
			Some(other) => {
				plan.findings.push(ImportFinding {
					source_path: format!("{source_path}.auth"),
					status: ImportStatus::Manual,
					message: format!(
						"LiteLLM pass-through auth {other:?} is not a boolean; treated as requiring auth rather than defaulting to open"
					),
				});
				true
			},
		};
		let mut emitted = true;
		if auth {
			let resolved_key = plan
				.inbound_api_key
				.as_ref()
				.map(|api_key| (api_key.key.clone(), api_key.location.clone()));
			match resolved_key {
				Some((key, location)) => {
					let mut policy = Map::from_iter([
						("keys".to_string(), json!([{"key": key}])),
						("mode".to_string(), json!("strict")),
					]);
					if let Some(location) = location {
						policy.insert("location".to_string(), location);
					}
					route_policies.insert("apiKey".to_string(), Value::Object(policy));
					plan.findings.push(ImportFinding {
						source_path: format!("{source_path}.auth"),
						status: ImportStatus::Approximate,
						message: "Required inbound auth using LiteLLM's master_key; pass-through auth also accepts any virtual key, which config.yaml alone cannot provide"
							.to_string(),
					});
				},
				None => {
					plan.findings.push(ImportFinding {
						source_path: format!("{source_path}.auth"),
						status: ImportStatus::Unsupported,
						message: "LiteLLM pass-through endpoint requires auth, but general_settings.master_key is missing or invalid; this endpoint was not emitted rather than opened without authentication"
							.to_string(),
					});
					emitted = false;
				},
			}
		}
		if !emitted {
			continue;
		}

		let forward_headers = match endpoint.remove("forward_headers") {
			None | Some(Value::Bool(false)) => false,
			Some(Value::Bool(true)) => true,
			Some(other) => {
				plan.findings.push(ImportFinding {
					source_path: format!("{source_path}.forward_headers"),
					status: ImportStatus::Manual,
					message: format!(
						"LiteLLM pass-through forward_headers {other:?} is not a boolean and was treated as false"
					),
				});
				false
			},
		};
		if !forward_headers {
			plan.findings.push(ImportFinding {
				source_path: format!("{source_path}.forward_headers"),
				status: ImportStatus::Manual,
				message: "LiteLLM forward_headers is false or unset; agentgateway forwards all incoming headers by default and this could not be restricted automatically"
					.to_string(),
			});
		}

		report_unmapped_fields(
			&mut plan.findings,
			&source_path,
			&endpoint,
			ImportStatus::Manual,
			"LiteLLM pass-through endpoint field requires manual review and was not emitted",
		);

		let mut route = Map::from_iter([
			("matches".to_string(), json!([{"path": {"exact": path}}])),
			(
				"backends".to_string(),
				json!([{"host": format!("{host}:{port}")}]),
			),
		]);
		if !route_policies.is_empty() {
			route.insert("policies".to_string(), Value::Object(route_policies));
		}
		plan.pass_through_routes.push(Value::Object(route));
		plan.findings.push(ImportFinding {
			source_path,
			status: ImportStatus::Approximate,
			message: format!(
				"Mapped LiteLLM pass-through endpoint to a standalone route proxying to {host}:{port}"
			),
		});
	}
}

/// Maps LiteLLM's top-level `guardrails` to agentgateway's `llm.policies.guardrails`
/// (`PromptGuard`). Only `guardrail: bedrock` is mapped; other provider types are left
/// `Unsupported`. LiteLLM's `default_on: false` makes a guardrail opt-in per request or virtual
/// key, which config.yaml alone cannot express and which standalone `PromptGuard` has no
/// equivalent for: every emitted guardrail runs on every request against every model, which is
/// a broader scope than an opt-in source guardrail.
fn import_litellm_guardrails(guardrails: &[Value], plan: &mut ImportPlan) {
	let mut request_guards = Vec::new();
	let mut response_guards = Vec::new();
	for (index, guardrail) in guardrails.iter().enumerate() {
		let source_path = format!("guardrails[{index}]");
		let Value::Object(guardrail) = guardrail else {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM guardrail entry must be an object and was not emitted".to_string(),
			});
			continue;
		};
		let label = guardrail
			.get("guardrail_name")
			.and_then(Value::as_str)
			.unwrap_or("this guardrail")
			.to_string();
		let Some(Value::Object(params)) = guardrail.get("litellm_params") else {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM guardrail has no litellm_params and was not emitted".to_string(),
			});
			continue;
		};
		let mut params = params.clone();
		let guardrail_type = params.remove("guardrail");
		let Some(Value::String(guardrail_type)) = guardrail_type else {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM guardrail litellm_params.guardrail must be a string and was not emitted"
					.to_string(),
			});
			continue;
		};
		if guardrail_type != "bedrock" {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: format!(
					"LiteLLM guardrail type {guardrail_type:?} has no mapping; only bedrock is mapped"
				),
			});
			continue;
		}
		let guardrail_identifier = params.remove("guardrailIdentifier");
		let guardrail_version = params.remove("guardrailVersion");
		let region = params.remove("aws_region_name");
		let (
			Some(Value::String(guardrail_identifier)),
			Some(Value::String(guardrail_version)),
			Some(Value::String(region)),
		) = (&guardrail_identifier, &guardrail_version, &region)
		else {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM bedrock guardrail requires string guardrailIdentifier, guardrailVersion, and aws_region_name and was not emitted"
					.to_string(),
			});
			continue;
		};
		let bedrock = json!({
			"guardrailIdentifier": normalize_environment_references(json!(guardrail_identifier)),
			"guardrailVersion": normalize_environment_references(json!(guardrail_version)),
			"region": normalize_environment_references(json!(region)),
		});
		let modes = params.remove("mode");
		let mode_list: Vec<String> = match &modes {
			Some(Value::Array(items)) => {
				let strings: Vec<String> = items
					.iter()
					.filter_map(|v| v.as_str().map(str::to_string))
					.collect();
				if strings.len() != items.len() {
					plan.findings.push(ImportFinding {
						source_path: format!("{source_path}.litellm_params.mode"),
						status: ImportStatus::Manual,
						message: "LiteLLM guardrail mode entries must be strings; non-string entries were not evaluated"
							.to_string(),
					});
				}
				strings
			},
			Some(Value::String(mode)) => vec![mode.clone()],
			_ => Vec::new(),
		};
		if mode_list.is_empty() {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM guardrail litellm_params.mode is required to know whether to run before or after the LLM call and was not emitted"
					.to_string(),
			});
			continue;
		}
		let mut attached = false;
		for mode in &mode_list {
			match mode.as_str() {
				"pre_call" => {
					request_guards.push(json!({"bedrockGuardrails": bedrock}));
					attached = true;
				},
				"post_call" => {
					response_guards.push(json!({"bedrockGuardrails": bedrock}));
					attached = true;
				},
				other => {
					plan.findings.push(ImportFinding {
						source_path: source_path.clone(),
						status: ImportStatus::Unsupported,
						message: format!(
							"LiteLLM guardrail mode {other:?} is not supported; only pre_call and post_call are mapped"
						),
					});
				},
			}
		}
		if !attached {
			continue;
		}
		let default_on = params.remove("default_on");
		let (status, message) = match default_on {
			Some(Value::Bool(false)) | None => (
				ImportStatus::Approximate,
				format!(
					"LiteLLM runs {label} only when a request or virtual key opts in (default_on: false or unset); agentgateway applies it to every request against every configured model, which is stricter than the source, never weaker"
				),
			),
			Some(Value::Bool(true)) => (
				ImportStatus::Approximate,
				format!(
					"Mapped {label} to run on every request against every configured model, matching LiteLLM's default_on: true"
				),
			),
			Some(_) => (
				ImportStatus::Manual,
				format!("{label}'s default_on must be a boolean; it was still mapped as always-on"),
			),
		};
		report_unmapped_fields(
			&mut plan.findings,
			&format!("{source_path}.litellm_params"),
			&params,
			ImportStatus::Manual,
			"LiteLLM guardrail parameter requires manual review and was not emitted",
		);
		plan.findings.push(ImportFinding {
			source_path,
			status,
			message,
		});
	}
	if request_guards.is_empty() && response_guards.is_empty() {
		return;
	}
	let mut guardrails_json = Map::new();
	if !request_guards.is_empty() {
		guardrails_json.insert("request".to_string(), Value::Array(request_guards));
	}
	if !response_guards.is_empty() {
		guardrails_json.insert("response".to_string(), Value::Array(response_guards));
	}
	plan.guardrails = Some(Value::Object(guardrails_json));
}

/// Maps LiteLLM's `general_settings.master_key` (and an optional custom
/// `litellm_key_header_name`) to a standalone inbound `apiKey` policy.
fn apply_litellm_inbound_api_key(
	settings: &Map<String, Value>,
	environment_variables: &Map<String, Value>,
	plan: &mut ImportPlan,
) -> anyhow::Result<()> {
	let Some(master_key) = settings.get("master_key") else {
		let message = if environment_variables.contains_key("LITELLM_MASTER_KEY") {
			"No master_key in config.yaml, but environment_variables.LITELLM_MASTER_KEY is set; LiteLLM loads environment_variables into its own process environment at startup, so that value is very likely the real master_key. This importer does not read environment_variables for security-sensitive values automatically; copy it into general_settings.master_key (or add general_settings.master_key: os.environ/LITELLM_MASTER_KEY if you'd rather keep the literal value out of config.yaml) to make it visible"
				.to_string()
		} else {
			"No master_key in config.yaml; if LiteLLM authenticates from the LITELLM_MASTER_KEY environment variable directly (common in Docker deployments), this importer can't see it from config.yaml alone. Set general_settings.master_key: os.environ/LITELLM_MASTER_KEY to make it visible"
				.to_string()
		};
		plan.findings.push(ImportFinding {
			source_path: "general_settings.master_key".to_string(),
			status: ImportStatus::Manual,
			message,
		});
		if settings.contains_key("litellm_key_header_name") {
			plan.findings.push(ImportFinding {
				source_path: "general_settings.litellm_key_header_name".to_string(),
				status: ImportStatus::Manual,
				message:
					"LiteLLM custom API-key header has no master_key to authenticate and was not emitted"
						.to_string(),
			});
		}
		return Ok(());
	};

	let Value::String(master_key) = master_key else {
		bail!("general_settings.master_key must be a string, got {master_key}");
	};

	let mut literal_key_contains_dollar = false;
	match master_key.strip_prefix("os.environ/") {
		Some("") => bail!("general_settings.master_key's environment reference must name a variable"),
		Some(_) => {},
		None => {
			if master_key.is_empty() {
				bail!("general_settings.master_key must not be empty");
			}
			literal_key_contains_dollar = master_key.contains('$');
		},
	}
	// Reuse the same `os.environ/NAME` -> `$NAME` normalization every other
	// imported field already goes through, so the master key expands the same
	// way as everything else agentgateway's config loader resolves.
	let Value::String(key) = normalize_environment_references(Value::String(master_key.clone()))
	else {
		unreachable!("normalize_environment_references preserves the String variant");
	};

	let master_key_caveat =
		"Mapped LiteLLM's master_key (an admin-equivalent credential, not a per-user virtual key)";
	let virtual_key_caveat =
		"Per-user virtual-key migration needs LiteLLM's database state, not config.yaml alone.";
	let (location, status, message) = match settings.get("litellm_key_header_name") {
		None => (
			None,
			ImportStatus::Approximate,
			format!(
				"{master_key_caveat} to strict Authorization Bearer authentication; LiteLLM also accepts other credential headers. {virtual_key_caveat}"
			),
		),
		Some(Value::String(name)) => {
			let Ok(name) = ::http::HeaderName::from_bytes(name.as_bytes()) else {
				bail!("general_settings.litellm_key_header_name {name:?} is not a valid HTTP header name");
			};
			(
				Some(json!({"header": {"name": name.as_str()}})),
				ImportStatus::Approximate,
				format!(
					"{master_key_caveat} and custom API-key header to inbound API-key authentication; LiteLLM also accepts the same key with a Bearer prefix on this header, which this does not replicate. {virtual_key_caveat}"
				),
			)
		},
		Some(other) => {
			bail!("general_settings.litellm_key_header_name must be a string, got {other}")
		},
	};

	plan.inbound_api_key = Some(ImportedInboundAPIKey { key, location });
	plan.findings.push(ImportFinding {
		source_path: "general_settings.master_key".to_string(),
		status,
		message,
	});
	if literal_key_contains_dollar {
		plan.findings.push(ImportFinding {
			source_path: "general_settings.master_key".to_string(),
			status: ImportStatus::Manual,
			message: "This literal master_key contains '$'; agentgateway's config loader expands $VAR references in string values, so this key may not load as written. Move it to an environment variable with os.environ/VAR_NAME instead"
				.to_string(),
		});
	}
	if matches!(
		settings.get("litellm_key_header_name"),
		Some(Value::String(_))
	) {
		plan.findings.push(ImportFinding {
			source_path: "general_settings.litellm_key_header_name".to_string(),
			status: ImportStatus::Exact,
			message: "Mapped LiteLLM's custom API-key header name to the inbound API-key policy"
				.to_string(),
		});
	}
	Ok(())
}

/// Maps LiteLLM's top-level `mcp_servers` to standalone `mcp.targets` on the existing `default`
/// gateway, carrying environment-backed stdio values and static headers into the plan, mapping
/// transports explicitly, and accounting for `alias`/`litellm_settings.mcp_aliases`.
fn import_litellm_mcp_servers(
	mcp_servers: &Map<String, Value>,
	litellm_settings: &Map<String, Value>,
	plan: &mut ImportPlan,
) {
	if mcp_servers.is_empty() {
		return;
	}
	let mut emitted_count = 0usize;
	let mcp_aliases: HashMap<&str, &str> = match litellm_settings.get("mcp_aliases") {
		Some(Value::Object(aliases)) => aliases
			.iter()
			.filter_map(|(alias, server)| server.as_str().map(|server| (server, alias.as_str())))
			.collect(),
		_ => HashMap::new(),
	};
	let mut used_names: HashSet<String> = HashSet::new();
	for (server_key, server) in mcp_servers {
		let source_path = format!("mcp_servers.{server_key}");
		let Value::Object(server) = server else {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "LiteLLM MCP server must be an object and was not emitted".to_string(),
			});
			continue;
		};
		let mut server = server.clone();

		// A restricted server still goes through every mapping step below and gets a fully-built
		// spec, stashed rather than emitted into mcp.targets: a later DB-state import
		// (import_litellm_db_state) can splice it in once it proves an mcpAuthorization rule
		// restricts it to specific keys. It shares this path with an open server, so it gets the
		// same transport/auth_type/extra_headers/static_headers handling and error findings.
		// LiteLLM defaults a missing allow_all_keys to false (load_servers_from_config in
		// litellm/proxy/mcp_server/mcp_server_manager.py); only an explicit true counts as open.
		let allow_all_keys_value = server.remove("allow_all_keys");
		let allow_all_keys_restriction: Option<&str> = match &allow_all_keys_value {
			None => Some("omitted"),
			Some(Value::Bool(false)) => Some("false"),
			Some(Value::Bool(true)) => None,
			Some(_) => {
				plan.findings.push(ImportFinding {
					source_path: source_path.clone(),
					status: ImportStatus::Manual,
					message: "LiteLLM allow_all_keys must be a boolean; treating this server as restricted (LiteLLM's own default is false)".to_string(),
				});
				Some("non-boolean")
			},
		};
		let is_restricted = allow_all_keys_restriction.is_some();

		let display_name = match server.remove("alias") {
			Some(Value::String(alias)) => alias,
			_ => mcp_aliases
				.get(server_key.as_str())
				.map(|alias| alias.to_string())
				.unwrap_or_else(|| server_key.clone()),
		};
		let Some(target_name) = sanitize_mcp_target_name(&display_name) else {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: "This server has no name that can be represented as an agentgateway MCP target name and was not emitted"
					.to_string(),
			});
			continue;
		};
		if target_name != display_name {
			plan.findings.push(ImportFinding {
				source_path: source_path.clone(),
				status: ImportStatus::Approximate,
				message: format!(
					"Renamed {display_name:?} to {target_name:?} to satisfy agentgateway's Gateway-API-style MCP target name pattern (lowercase letters, digits, '-', '.'; no underscores)"
				),
			});
		}
		let transport = match server.remove("transport") {
			None => "http".to_string(),
			Some(Value::String(transport)) => transport,
			Some(_) => {
				plan.findings.push(ImportFinding {
					source_path,
					status: ImportStatus::Manual,
					message: "LiteLLM MCP transport must be a string and was not emitted".to_string(),
				});
				continue;
			},
		};
		let spec = match transport.as_str() {
			"http" => build_remote_mcp_target_spec(&mut server, &source_path, plan, "mcp"),
			"sse" => build_remote_mcp_target_spec(&mut server, &source_path, plan, "sse"),
			"stdio" => build_stdio_mcp_target_spec(&mut server, &source_path, plan),
			other => {
				plan.findings.push(ImportFinding {
					source_path,
					status: ImportStatus::Unsupported,
					message: format!(
						"LiteLLM MCP transport {other:?} is not supported by this importer and was not emitted"
					),
				});
				continue;
			},
		};
		let Some(mut spec) = spec else {
			continue;
		};
		// Reserved only now that the spec is confirmed buildable - a server that fails
		// transport/url/command validation above must not claim this name, or a later, valid
		// server sanitizing to the same name would be wrongly dropped as a collision. A
		// restricted server is never emitted at all, so it must not reserve its name either.
		if !is_restricted && !used_names.insert(target_name.clone()) {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: format!(
					"Sanitized target name {target_name:?} collides with another server and was not emitted"
				),
			});
			continue;
		}
		// Record both the raw config.yaml key and its display name (if aliased), so DB-state MCP
		// grants - which reference LiteLLM's own server name, never agentgateway's sanitized
		// target name - can be resolved without re-deriving sanitization or missing a
		// collision-driven rename. Done only once a target spec exists and survives the
		// collision check above, so a server dropped either way (bad transport, unbuildable
		// spec, or a colliding name) never leaves an orphaned name mapping behind.
		plan
			.mcp_target_names
			.insert(server_key.clone(), target_name.clone());
		plan
			.mcp_target_names
			.insert(display_name.clone(), target_name.clone());

		match server.remove("auth_type") {
			None => {},
			Some(Value::String(auth_type)) if auth_type == "none" => {},
			Some(Value::String(auth_type)) => {
				plan.findings.push(ImportFinding {
					source_path: source_path.clone(),
					status: ImportStatus::Manual,
					message: format!(
						"LiteLLM MCP auth_type {auth_type:?} has no mapping; connections to this target will be unauthenticated until configured manually"
					),
				});
			},
			Some(_) => {
				plan.findings.push(ImportFinding {
					source_path: source_path.clone(),
					status: ImportStatus::Manual,
					message: "LiteLLM MCP auth_type must be a string; connections to this target will be unauthenticated until configured manually".to_string(),
				});
			},
		}
		if server.remove("extra_headers").is_some() {
			plan.findings.push(ImportFinding {
				source_path: source_path.clone(),
				status: ImportStatus::Manual,
				message: "LiteLLM extra_headers forwards specific caller headers per request; agentgateway has no equivalent at this target and it was not emitted"
					.to_string(),
			});
		}
		if let Some(static_headers) = server.remove("static_headers") {
			match static_headers {
				Value::Object(headers) if transport == "stdio" && !headers.is_empty() => {
					plan.findings.push(ImportFinding {
						source_path: source_path.clone(),
						status: ImportStatus::Unsupported,
						message:
							"LiteLLM static_headers has no meaning for a stdio MCP target and was not applied"
								.to_string(),
					});
				},
				Value::Object(headers) if !headers.is_empty() => {
					let headers = normalize_environment_references(Value::Object(headers));
					if let Value::Object(spec_map) = &mut spec {
						spec_map.insert(
							"policies".to_string(),
							json!({"requestHeaderModifier": {"set": headers}}),
						);
					}
					plan.findings.push(ImportFinding {
						source_path: source_path.clone(),
						status: ImportStatus::Exact,
						message: "Mapped static_headers to a fixed request header modifier on this target"
							.to_string(),
					});
				},
				Value::Object(_) => {},
				_ => {
					plan.findings.push(ImportFinding {
						source_path: source_path.clone(),
						status: ImportStatus::Manual,
						message: "LiteLLM static_headers must be an object and was not emitted".to_string(),
					});
				},
			}
		}

		report_unmapped_fields(
			&mut plan.findings,
			&source_path,
			&server,
			ImportStatus::Manual,
			"LiteLLM MCP server field requires manual review and was not emitted",
		);
		if let Value::Object(spec_map) = &mut spec {
			spec_map.insert("name".to_string(), json!(target_name));
		}
		if is_restricted {
			let held_back_reason = match allow_all_keys_restriction {
				Some("omitted") => {
					"config.yaml omitted allow_all_keys (LiteLLM defaults this to false), which it alone cannot express safely"
				},
				Some("non-boolean") => {
					"config.yaml set a non-boolean allow_all_keys, which it alone cannot express safely"
				},
				_ => "config.yaml set allow_all_keys: false, which it alone cannot express safely",
			};
			let message = if allow_all_keys_restriction == Some("omitted") {
				"LiteLLM defaults an omitted allow_all_keys to false, so this server is restricted to specific virtual keys via DB-managed object_permission.mcp_servers grants, which config.yaml alone cannot express; it was not emitted rather than exposing it to every caller"
			} else {
				"This server is restricted to specific virtual keys via DB-managed object_permission.mcp_servers grants, which config.yaml alone cannot express; it was not emitted rather than exposing it to every caller"
			};
			plan
				.pending_restricted_mcp_targets
				.push((spec, held_back_reason));
			plan.held_back_mcp_servers.insert(server_key.clone());
			plan.held_back_mcp_servers.insert(display_name.clone());
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: message.to_string(),
			});
		} else {
			plan.mcp_targets.push(spec);
			emitted_count += 1;
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Approximate,
				message: format!("Mapped LiteLLM MCP server to agentgateway target {target_name:?}"),
			});
		}
	}
	if emitted_count > 1 {
		plan.findings.push(ImportFinding {
			source_path: "mcp_servers".to_string(),
			status: ImportStatus::Manual,
			message: "Multiple MCP servers are multiplexed behind one endpoint; agentgateway's default conditional tool-name prefixing may not match LiteLLM's own per-server/alias tool naming exactly"
				.to_string(),
		});
	}
}

/// Builds the `mcp`/`sse` remote transport spec (shared shape: just a `url`).
fn build_remote_mcp_target_spec(
	server: &mut Map<String, Value>,
	source_path: &str,
	plan: &mut ImportPlan,
	tag: &str,
) -> Option<Value> {
	let Some(Value::String(url)) = server.remove("url") else {
		plan.findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Manual,
			message: "LiteLLM remote MCP server requires a url and was not emitted".to_string(),
		});
		return None;
	};
	// An env-var reference isn't resolved until agentgateway loads the real config, so its
	// eventual value can't be validated here; only a literal url is checked now.
	if !url.starts_with("os.environ/") {
		let host = json!({"host": &url});
		let valid = serde_json::from_value::<McpBackendHost>(host)
			.ok()
			.and_then(|backend| backend.process().ok())
			.is_some();
		if !valid {
			plan.findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Manual,
				message: "LiteLLM MCP server url could not be resolved to a valid host and was not emitted"
					.to_string(),
			});
			return None;
		}
	}
	let url = normalize_environment_references(json!(url));
	Some(json!({tag: {"host": url}}))
}

/// Builds the `stdio` transport spec, normalizing `os.environ/`-style environment references in
/// `env` the same way the rest of the importer does for model parameters.
fn build_stdio_mcp_target_spec(
	server: &mut Map<String, Value>,
	source_path: &str,
	plan: &mut ImportPlan,
) -> Option<Value> {
	let Some(Value::String(command)) = server.remove("command") else {
		plan.findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Manual,
			message: "LiteLLM stdio MCP server requires command and was not emitted".to_string(),
		});
		return None;
	};
	let args = match server.remove("args") {
		None => Vec::new(),
		Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
			.into_iter()
			.map(|item| item.as_str().expect("checked above").to_string())
			.collect(),
		Some(_) => {
			plan.findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Manual,
				message: "LiteLLM stdio args must be an array of strings and was not emitted".to_string(),
			});
			return None;
		},
	};
	let env = match server.remove("env") {
		None => Map::new(),
		Some(Value::Object(env)) => match normalize_environment_references(Value::Object(env)) {
			Value::Object(env) => env,
			_ => unreachable!("normalize_environment_references preserves the Object variant"),
		},
		Some(_) => {
			plan.findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Manual,
				message: "LiteLLM stdio env must be an object and was not emitted".to_string(),
			});
			return None;
		},
	};
	Some(json!({"stdio": {"cmd": command, "args": args, "env": env}}))
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
		if credential.params.is_empty() && credential_request_headers.is_empty() {
			plan.findings.push(ImportFinding {
				source_path: reference_path,
				status: ImportStatus::Manual,
				message: format!(
					"LiteLLM credential {credential_name:?} contains no supported provider values and was not applied"
				),
			});
			return;
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

	let params_cooldown_override = params.remove("cooldown_time").is_some();
	let health_override = params_cooldown_override
		|| ["cooldown_time", "allowed_fails", "allowed_fails_policy"]
			.iter()
			.any(|key| model_info.contains_key(*key));
	if params_cooldown_override {
		findings.push(ImportFinding {
			source_path: format!("{source_path}.litellm_params.cooldown_time"),
			status: ImportStatus::Manual,
			message: "Per-deployment LiteLLM cooldown_time was not mapped; router-level model health is not applied to this deployment"
				.to_string(),
		});
	}
	if params.remove("timeout").is_some() {
		findings.push(ImportFinding {
			source_path: format!("{source_path}.litellm_params.timeout"),
			status: ImportStatus::Manual,
			message: "Per-deployment LiteLLM timeout was not mapped; agentgateway applies at most one router_settings.timeout across all models"
				.to_string(),
		});
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
	let order = match params.remove("order") {
		None => None,
		Some(value) => match value.as_i64() {
			Some(parsed) => Some(parsed),
			None => {
				findings.push(ImportFinding {
					source_path: format!("{source_path}.litellm_params.order"),
					status: ImportStatus::Manual,
					message: "LiteLLM order must be an integer and was not used for routing priority"
						.to_string(),
				});
				None
			},
		},
	};

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
			inherits_model_health: !health_override,
			order,
			source_path: source_path.to_string(),
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

/// Matches a DB-state model pattern (a team's or key's own `models` entry, which can be a
/// literal name or a one-wildcard prefix/suffix pattern, same shape config.yaml allows) against
/// a real model name, the same way LiteLLM itself resolves one against the other.
fn litellm_model_matches(pattern: &str, model: &str) -> bool {
	if pattern == model {
		return true;
	}
	pattern
		.strip_suffix('*')
		.map(|prefix| model.starts_with(prefix))
		.or_else(|| {
			pattern
				.strip_prefix('*')
				.map(|suffix| model.ends_with(suffix))
		})
		.unwrap_or(false)
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
		"perplexity" => Some("perplexity"),
		"nebius" => Some("nebius"),
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

/// LiteLLM's `ROUTER_MAX_FALLBACKS`, the recursion depth cap for fallback attempts.
const DEFAULT_MAX_FALLBACKS: usize = 5;

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

fn apply_default_fallbacks(
	value: &Value,
	source_path: &str,
	explicit_fallbacks: Option<&Value>,
	max_fallbacks: usize,
	plan: &mut ImportPlan,
) {
	let Some(entries) = value.as_array() else {
		plan.findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Unsupported,
			message: "Default fallbacks must be a list of model names".to_string(),
		});
		return;
	};
	if entries.is_empty() {
		plan.findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Exact,
			message: "default_fallbacks is empty; no routes changed".to_string(),
		});
		return;
	}

	let mut seen = HashSet::new();
	let mut defaults: Vec<String> = Vec::new();
	let mut default_names: Vec<&str> = Vec::new();
	for entry in entries {
		let Some(name) = entry.as_str() else {
			plan.findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Manual,
				message: "Default fallback entry must be a string and was not emitted".to_string(),
			});
			continue;
		};
		if !seen.insert(name) {
			continue;
		}
		default_names.push(name);
		if plan.routes.contains_key(name) {
			defaults.push(name.to_string());
		} else {
			plan.findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Manual,
				message: format!("Default fallback target model {name:?} was not imported"),
			});
		}
	}

	if max_fallbacks == 0 {
		// Fallbacks are already disabled at the call site; only the unknown-model re-assign survives,
		// because LiteLLM resolves that during deployment lookup rather than in run_async_fallback.
		if let Some(first) = defaults.first()
			&& !plan.routes.contains_key("*")
		{
			plan.findings.push(ImportFinding {
				source_path: source_path.to_string(),
				status: ImportStatus::Manual,
				message: format!(
					"LiteLLM serves unknown model names via the first default fallback {first:?}; agentgateway returns 404 for unmatched model names because wildcard matching applies only to concrete llm.models entries; add a \"*\" model manually if unknown names must be served"
				),
			});
		}
		return;
	}

	let mut explicit: IndexMap<&str, Vec<&str>> = IndexMap::new();
	for entry in explicit_fallbacks
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(Value::as_object)
	{
		for (source, targets) in entry {
			let Some(targets) = targets.as_array() else {
				continue;
			};
			explicit
				.entry(source.as_str())
				.or_insert_with(|| targets.iter().filter_map(Value::as_str).collect());
		}
	}

	let mut wildcard_routes: Vec<String> = Vec::new();
	let mut updates: Vec<(String, Vec<Vec<String>>)> = Vec::new();
	for (route_name, route) in &plan.routes {
		if route_name.contains('*') {
			if !defaults.is_empty() {
				wildcard_routes.push(route_name.clone());
			}
			continue;
		}
		let mut chain: Vec<&str> = Vec::new();
		let mut visited: HashSet<&str> = HashSet::from([route_name.as_str()]);
		extend_fallback_chain(
			route_name,
			0,
			&explicit,
			&default_names,
			&plan.routes,
			max_fallbacks,
			&mut visited,
			&mut chain,
		);
		let mut reached: HashSet<&str> = route.targets.iter().map(String::as_str).collect();
		let mut groups: Vec<Vec<String>> = Vec::new();
		for hop in chain {
			let group: Vec<String> = plan.routes[hop]
				.targets
				.iter()
				.filter(|target| reached.insert(target.as_str()))
				.cloned()
				.collect();
			if !group.is_empty() {
				groups.push(group);
			}
		}
		if groups != route.fallback_groups {
			updates.push((route_name.clone(), groups));
		}
	}

	let changed = !updates.is_empty();
	for (route_name, groups) in updates {
		plan
			.routes
			.get_mut(&route_name)
			.expect("route name taken from plan.routes")
			.fallback_groups = groups;
	}

	for route_name in wildcard_routes {
		plan.findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Manual,
			message: format!(
				"Default fallbacks cannot be attached to wildcard model {route_name:?}: failover requires a virtual model and virtual model names match exactly; configure failover for requests served by this wildcard manually"
			),
		});
	}
	plan.findings.push(if changed {
		ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Approximate,
			message:
				"Flattened LiteLLM fallback recursion (per-hop explicit fallbacks, default_fallbacks at hops without one, attempted models skipped) into static priority-ordered failover targets"
					.to_string(),
		}
	} else {
		ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Exact,
			message:
				"default_fallbacks changed no routes; every chain already matches the explicit fallbacks or self-exclusion removed every candidate"
					.to_string(),
		}
	});
	if let Some(first) = defaults.first()
		&& !plan.routes.contains_key("*")
	{
		plan.findings.push(ImportFinding {
			source_path: source_path.to_string(),
			status: ImportStatus::Manual,
			message: format!(
				"LiteLLM serves unknown model names via the first default fallback {first:?}; agentgateway returns 404 for unmatched model names because wildcard matching applies only to concrete llm.models entries; add a \"*\" model manually if unknown names must be served"
			),
		});
	}
}

/// Walks the LiteLLM fallback graph depth-first in attempt order, appending every model group that
/// would be tried after `node` fails.
///
/// Mirrors `run_async_fallback`: the budget check happens once per frame on entry, and `depth`
/// increments per attempted sibling within a frame and is inherited by that sibling's own frame, so
/// earlier siblings' attempts consume the descent budget of later siblings rather than capping the
/// sibling list itself. Children that were never imported consume a depth slot and visitation like
/// any attempted sibling, because LiteLLM attempts and fails them, but they are not emitted and
/// their own fallback entries are not walked.
#[allow(clippy::too_many_arguments)]
fn extend_fallback_chain<'a>(
	node: &'a str,
	depth: usize,
	explicit: &IndexMap<&'a str, Vec<&'a str>>,
	defaults: &[&'a str],
	routes: &'a IndexMap<String, ImportedRoute>,
	max_fallbacks: usize,
	visited: &mut HashSet<&'a str>,
	chain: &mut Vec<&'a str>,
) {
	if depth >= max_fallbacks {
		return;
	}
	let mut depth = depth;
	let children = explicit.get(node).map(Vec::as_slice).unwrap_or(defaults);
	for child in children.iter().copied() {
		if !visited.insert(child) {
			continue;
		}
		depth += 1;
		if !routes.contains_key(child) {
			continue;
		}
		chain.push(child);
		extend_fallback_chain(
			child,
			depth,
			explicit,
			defaults,
			routes,
			max_fallbacks,
			visited,
			chain,
		);
	}
}

/// LiteLLM's DB-managed runtime state (teams and virtual API keys), extracted separately
/// through LiteLLM's own Admin API. Kept apart from the config.yaml the rest of this importer
/// reads: config.yaml is static deployment configuration, this is live, DB-managed account
/// state.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteLlmDbState {
	#[serde(default)]
	pub teams: Vec<DbStateTeam>,
	#[serde(default)]
	pub keys: Vec<DbStateKey>,
	#[serde(default)]
	pub mcp_servers: Vec<DbStateMcpServer>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbStateMcpServer {
	pub server_id: String,
	pub server_name: String,
	#[serde(default)]
	pub allow_all_keys: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbStateTeam {
	pub team_id: String,
	/// Human-readable team name, used only in finding messages - `team_id` (a UUID) remains
	/// the stable identifier everywhere a finding's `source_path` references a team.
	pub team_alias: Option<String>,
	#[serde(default)]
	pub models: Vec<String>,
	#[serde(default)]
	pub blocked: bool,
	pub max_budget: Option<f64>,
	pub spend: Option<f64>,
	pub budget_duration: Option<String>,
	pub rpm_limit: Option<i64>,
	pub tpm_limit: Option<i64>,
	/// Team-level MCP server grants (server IDs or names, same forms as a key's own
	/// `mcp_servers`). When non-empty, a key on this team can only reach a server that's in
	/// *both* its own grant list and this one - see the intersection in
	/// `import_litellm_db_state`.
	#[serde(default)]
	pub object_permission: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbStateKey {
	pub key_hash: String,
	pub key_alias: Option<String>,
	pub team_id: Option<String>,
	pub user_id: Option<String>,
	#[serde(default)]
	pub models: Vec<String>,
	pub expires: Option<String>,
	pub blocked: Option<bool>,
	pub max_budget: Option<f64>,
	pub spend: Option<f64>,
	pub soft_budget: Option<f64>,
	pub budget_duration: Option<String>,
	pub rpm_limit: Option<i64>,
	pub tpm_limit: Option<i64>,
	#[serde(default)]
	pub mcp_servers: Vec<String>,
}

/// Parses a DB-state JSON document (LiteLLM's DB-managed teams, keys, and MCP server grants)
/// for use as [`ImportOptions::db_state`]. A thin wrapper so callers (the CLI) don't need
/// their own `serde_json` dependency just for this.
pub fn parse_db_state(json: &str) -> anyhow::Result<LiteLlmDbState> {
	serde_json::from_str(json).context("invalid LiteLLM DB-state JSON")
}

/// A team's human-readable name for finding messages, falling back to its `team_id` (a UUID)
/// when LiteLLM never set `team_alias`. Never used for `source_path`, which stays on `team_id`
/// as the stable, structural reference.
fn team_label(team: &DbStateTeam) -> &str {
	team.team_alias.as_deref().unwrap_or(team.team_id.as_str())
}

/// Maps a LiteLLM `budget_duration` string onto agentgateway's `window.rolling` duration,
/// only for the two cases where both sides reset on the identical UTC instant: LiteLLM's
/// `duration_parser` resolves "1d"/"24h"/"daily" to the next midnight UTC and "1h"/"hourly" to
/// the next hour boundary, which is exactly what agentgateway's epoch-aligned "24h"/"1h"
/// windows already do (Unix epoch 0 falls exactly on a midnight UTC hour boundary). Anything
/// else (weekly - Monday-anchored, not epoch-aligned; monthly/"30d" - a calendar month, not a
/// fixed 30-day span; any other custom value) is deliberately left unmapped rather than
/// approximated, since a fixed-length rolling window would drift against LiteLLM's
/// calendar-anchored reset over time.
fn litellm_budget_duration_to_window(duration: &str) -> Option<&'static str> {
	match duration.to_ascii_lowercase().as_str() {
		"1d" | "24h" | "daily" => Some("24h"),
		"1h" | "hourly" => Some("1h"),
		_ => None,
	}
}

/// Escapes a string as a CEL string literal, including the surrounding quotes. Rust's `{:?}`
/// uses Rust's own escape rules, not CEL's, and a control character in a user-controlled value
/// (a key alias, a server name) embedded that way can break the generated CEL expression
/// entirely.
fn cel_escape_string(value: &str) -> String {
	let mut out = String::with_capacity(value.len() + 2);
	out.push('"');
	for c in value.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'"' => out.push_str("\\\""),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Builds an `mcpAuthorization` rule fragment matching a target by name across every MCP
/// resource type (tool, prompt, resource) rather than only `mcp.tool.target`: LiteLLM's own
/// access grants cover a server's tools, prompts and resources together, and a tool-only rule
/// would deny the other two kinds outright once any mcpAuthorization rule exists at all. Each
/// resource type's own CEL context only carries its own field (a tool request has no
/// `mcp.prompt`), so `has(...)` guards every branch instead of letting a mismatched field
/// access fail.
fn mcp_target_rule(target: &str) -> String {
	let target = cel_escape_string(target);
	format!(
		"(has(mcp.tool) && mcp.tool.target == {target}) || \
		 (has(mcp.prompt) && mcp.prompt.target == {target}) || \
		 (has(mcp.resource) && mcp.resource.target == {target})"
	)
}

/// Maps LiteLLM's DB-managed teams/keys onto `llm.policies.apiKey.keys[]` entries
/// (`ImportPlan::db_api_keys`), separate from the config.yaml import. A key is skipped
/// entirely (never emitted, even disabled) rather than migrated in a degraded form when:
/// blocked (itself or its team - re-enabling a revoked credential would be a regression);
/// already expired; or not yet expired (agentgateway API keys have no expiry field at all, so
/// migrating one as a plain key would make a time-boxed credential permanent) - skip it by
/// default rather than add that risk silently.
pub(super) fn import_litellm_db_state(
	state: &LiteLlmDbState,
	options: &ImportOptions,
	plan: &mut ImportPlan,
) {
	let teams: HashMap<&str, &DbStateTeam> = state
		.teams
		.iter()
		.map(|t| (t.team_id.as_str(), t))
		.collect();

	for team in &state.teams {
		let mut unmapped = Vec::new();
		if let Some(duration) = &team.budget_duration {
			unmapped.push(format!("budget_duration: {duration:?}"));
		}
		let has_rate_limit = team.rpm_limit.is_some() || team.tpm_limit.is_some();
		if let Some(rpm) = team.rpm_limit {
			unmapped.push(format!("rpm_limit: {rpm}"));
		}
		if let Some(tpm) = team.tpm_limit {
			unmapped.push(format!("tpm_limit: {tpm}"));
		}
		if !unmapped.is_empty() {
			let explanation = match (team.budget_duration.is_some(), has_rate_limit) {
				(true, true) => "the budget never resets and no rate limit is applied at the team level",
				(true, false) => "the budget never resets at the team level",
				_ => "no rate limit is applied at the team level",
			};
			plan.findings.push(ImportFinding {
				source_path: format!("db_state.teams[{}]", team.team_id),
				status: ImportStatus::Manual,
				message: format!(
					"Team {:?} has its own {} that was not migrated - {explanation}",
					team_label(team),
					unmapped.join(", ")
				),
			});
		}
	}

	// Phase 1: skip blocked/team-blocked/expired keys up front, so the budget split below (which
	// must see only the keys that will actually be emitted) never divides a team's pool across a
	// key that was never going to exist.
	let mut kept: Vec<(usize, &DbStateKey)> = Vec::new();
	for (index, key) in state.keys.iter().enumerate() {
		let source_path = format!("db_state.keys[{index}]");
		if let Some(team_id) = key.team_id.as_deref()
			&& !teams.contains_key(team_id)
		{
			// Can't verify this key's team isn't blocked, or what models/budget it should
			// inherit, so skip it entirely rather than silently treating it as team-less -
			// the same "unsafe to migrate in a degraded form" reasoning this function's own
			// doc comment already applies to a blocked or expired key.
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: format!(
					"Key's team_id {team_id:?} does not match any team in the DB-state export, so its team's blocked/model/budget state can't be checked; it was not migrated"
				),
			});
			continue;
		}
		let team = key.team_id.as_deref().and_then(|id| teams.get(id).copied());

		if key.blocked == Some(true) {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: "Key is blocked in LiteLLM and was not migrated".to_string(),
			});
			continue;
		}
		if team.is_some_and(|t| t.blocked) {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: "Key's team is blocked in LiteLLM and was not migrated".to_string(),
			});
			continue;
		}

		if let Some(expires) = &key.expires {
			match expires.parse::<DateTime<Utc>>() {
				Ok(expiry) if expiry <= options.now => {
					plan.findings.push(ImportFinding {
						source_path,
						status: ImportStatus::Unsupported,
						message: format!("Key expired at {expiry} in LiteLLM and was not migrated"),
					});
					continue;
				},
				Ok(expiry) => {
					plan.findings.push(ImportFinding {
						source_path,
						status: ImportStatus::Unsupported,
						message: format!(
							"Key expires at {expiry} in LiteLLM, but agentgateway API keys have no expiry field at all; migrating it as a plain key would make it permanent, so it was not migrated. Re-issue it without an expiry, or migrate it manually once an expiry-aware option exists."
						),
					});
					continue;
				},
				Err(_) => {
					plan.findings.push(ImportFinding {
						source_path,
						status: ImportStatus::Manual,
						message: format!(
							"Key expires value {expires:?} could not be parsed as a timestamp and was not migrated"
						),
					});
					continue;
				},
			}
		}

		kept.push((index, key));
	}

	// Phase 1.5: drop keys that can never be safely emitted at all (a duplicate key_hash, a
	// hash colliding with the master key, or a generated name collision) before the budget pool
	// split below, so a team's pool is only ever divided across keys that will actually be
	// emitted - not stranded on a share computed for a key that gets dropped afterward.
	let mut used_names: HashSet<String> = HashSet::new();
	if plan.inbound_api_key.is_some() {
		// Reserve "master" so a virtual key aliased to the same name can never collide into the
		// admin-bypass identity the master key gets mirrored under below.
		used_names.insert("master".to_string());
	}
	let master_hash = plan.inbound_api_key.as_ref().and_then(|master| {
		if master.key.starts_with('$') {
			None
		} else {
			Some(hex::encode(crate::crypto::digest::sha256(
				master.key.as_bytes(),
			)))
		}
	});
	let mut seen_hashes: HashMap<String, usize> = HashMap::new();
	let mut filtered: Vec<(usize, &DbStateKey, String)> = Vec::new();
	for (index, key) in kept {
		let source_path = format!("db_state.keys[{index}]");
		let is_valid_sha256_hex =
			key.key_hash.len() == 64 && key.key_hash.chars().all(|c| c.is_ascii_hexdigit());
		if !is_valid_sha256_hex {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: "Key's key_hash is not a 64-character hex SHA-256 digest and was not migrated"
					.to_string(),
			});
			continue;
		}
		if master_hash.as_deref() == Some(key.key_hash.as_str()) {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: "Key's key_hash matches the resolved master key's hash; migrating it would let it silently assume the master key's identity, so it was not migrated".to_string(),
			});
			continue;
		}
		if let Some(&first_index) = seen_hashes.get(&key.key_hash) {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: format!(
					"Key's key_hash is already used by db_state.keys[{first_index}]; agentgateway compiles keys by hash, so only one of them would ever take effect, and this one was not migrated"
				),
			});
			continue;
		}
		seen_hashes.insert(key.key_hash.clone(), index);
		let name = match key.key_alias.as_deref() {
			Some(alias) if !alias.trim().is_empty() => alias.to_string(),
			// Never derive from LiteLLM's own masked key_name: it contains real key characters.
			// key_hash is a one-way digest, safe to use as a stable fallback identifier. Take
			// chars, not bytes: a byte-boundary slice can land inside a multi-byte character.
			_ => format!("key-{}", key.key_hash.chars().take(12).collect::<String>()),
		};
		if !used_names.insert(name.clone()) {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Unsupported,
				message: format!(
					"Generated name {name:?} collides with another migrated key's name (budgets and rate limits would be shared) and was not migrated"
				),
			});
			continue;
		}
		filtered.push((index, key, name));
	}
	let kept = filtered;

	// Phase 2: split each budget-capped team's remaining pool across its kept keys,
	// so per-key caps can never add up to more than the team's own total. Keys with no
	// budget-capped team fall through untouched below (their own max_budget, if any, stands on
	// its own).
	let mut key_budgets: HashMap<&str, f64> = HashMap::new();
	let mut key_budget_windows: HashMap<&str, &'static str> = HashMap::new();
	let mut team_handled: HashSet<&str> = HashSet::new();
	for team in &state.teams {
		let Some(team_max) = team.max_budget else {
			continue;
		};
		let team_keys: Vec<&DbStateKey> = kept
			.iter()
			.map(|(_, key, _)| *key)
			.filter(|key| key.team_id.as_deref() == Some(team.team_id.as_str()))
			.collect();
		if team_keys.is_empty() {
			continue;
		}
		for key in &team_keys {
			team_handled.insert(key.key_hash.as_str());
		}
		// Split the team's remaining budget so per-key caps can never add up to more than the
		// team total: a key with its own budget keeps min(its own remaining, the team's), and
		// whatever's left of the team cap is split equally across keys with no budget of their
		// own (falling back to $0 rather than leaving them unbounded if the cap is already spent).
		// A negative spend value (malformed export data) is clamped to 0 rather than subtracted
		// as-is, so it can never inflate the remaining cap beyond the real max_budget.
		let team_cap = (team_max - team.spend.unwrap_or(0.0).max(0.0)).max(0.0);
		let with_own: Vec<&DbStateKey> = team_keys
			.iter()
			.copied()
			.filter(|key| key.max_budget.is_some())
			.collect();
		let without_own: Vec<&DbStateKey> = team_keys
			.iter()
			.copied()
			.filter(|key| key.max_budget.is_none())
			.collect();
		// Remaining, not gross, budget: a key that has already spent against its own cap must
		// not also eat into the team pool as if it still had its full original amount.
		let own_remaining: Vec<(&DbStateKey, f64)> = with_own
			.iter()
			.map(|key| {
				(
					*key,
					(key.max_budget.unwrap_or(0.0) - key.spend.unwrap_or(0.0).max(0.0)).max(0.0),
				)
			})
			.collect();
		let own_total: f64 = own_remaining.iter().map(|(_, remaining)| remaining).sum();
		if own_total > team_cap {
			let scale = if own_total > 0.0 {
				team_cap / own_total
			} else {
				0.0
			};
			for (key, remaining) in &own_remaining {
				key_budgets.insert(key.key_hash.as_str(), remaining * scale);
			}
			plan.findings.push(ImportFinding {
				source_path: format!("db_state.teams[{}]", team.team_id),
				status: ImportStatus::Approximate,
				message: format!(
					"Team {:?}'s keys' own remaining budgets (${own_total:.2}) exceed its remaining pool (${team_cap:.2}); scaled every key's budget down proportionally to {:.1}% to keep the team total unchanged",
					team_label(team),
					scale * 100.0
				),
			});
			// The pool is already fully (over-)committed by keys with their own budget, so a key
			// with no budget of its own gets none of it - a hard $0 cap, never "no budgets entry
			// at all" (which agentgateway would read as unlimited, a regression from
			// LiteLLM's team-capped pool).
			if !without_own.is_empty() {
				for key in &without_own {
					key_budgets.insert(key.key_hash.as_str(), 0.0);
				}
				plan.findings.push(ImportFinding {
					source_path: format!("db_state.teams[{}]", team.team_id),
					status: ImportStatus::Manual,
					message: format!(
						"{} key(s) with no budget of their own get none of team {:?}'s pool, which is already fully committed by keys with their own budget; migrated with a $0 budget (checked before every request, so this effectively disables the key) rather than no cap at all - review whether these keys need their own budget",
						without_own.len(),
						team_label(team)
					),
				});
			}
		} else {
			for (key, remaining) in &own_remaining {
				key_budgets.insert(key.key_hash.as_str(), *remaining);
			}
			if !without_own.is_empty() {
				let remaining = team_cap - own_total;
				let share = remaining / without_own.len() as f64;
				for key in &without_own {
					key_budgets.insert(key.key_hash.as_str(), share);
				}
				plan.findings.push(ImportFinding {
					source_path: format!("db_state.teams[{}]", team.team_id),
					status: ImportStatus::Approximate,
					message: format!(
						"Split team {:?}'s remaining budget pool (${remaining:.2}) equally across {} key(s) with no budget of their own (${share:.2} each); agentgateway has no team construct, so this is a one-time snapshot, not a live shared pool",
						team_label(team),
						without_own.len()
					),
				});
			}
		}
	}
	// Phase 2.5: resolve each key's own budget_duration into a resetting window - independent of
	// the pool split above, but only for a key whose budget is its own cap, not a team-pool
	// share. A team-pool share is a one-time snapshot of a (possibly lifetime) pool; applying a
	// recurring window to a fixed share would hand the key that share again every period, which
	// matches neither a lifetime team cap nor a team cap that itself resets on its own schedule.
	for (index, key, _) in &kept {
		let Some(duration) = &key.budget_duration else {
			continue;
		};
		let source_path = format!("db_state.keys[{index}]");
		if team_handled.contains(key.key_hash.as_str()) {
			plan.findings.push(ImportFinding {
				source_path,
				status: ImportStatus::Manual,
				message: format!(
					"Key has a LiteLLM budget_duration ({duration:?}), but its migrated amount is a one-time snapshot of a team budget pool share, not a standalone per-key allowance; a recurring window on a fixed pool share would hand the key that share again every period, so no window was applied - review whether the team itself needs its own budget handling"
				),
			});
			continue;
		}
		match litellm_budget_duration_to_window(duration) {
			Some(window) => {
				key_budget_windows.insert(key.key_hash.as_str(), window);
				plan.findings.push(ImportFinding {
					source_path,
					status: ImportStatus::Exact,
					message: format!(
						"Key has a LiteLLM budget_duration ({duration:?}); LiteLLM resets this on the next UTC calendar day/hour boundary, which is exactly what agentgateway's epoch-aligned rolling window does too, so it was mapped to a resetting budget window"
					),
				});
			},
			None => {
				plan.findings.push(ImportFinding {
					source_path,
					status: ImportStatus::Manual,
					message: format!(
						"Key has a LiteLLM budget_duration ({duration:?}); only exact daily (\"1d\"/\"24h\"/\"daily\") or hourly (\"1h\"/\"hourly\") durations are mapped - LiteLLM resets weekly/monthly durations on calendar week/month boundaries (a 28-31 day month, a Monday-anchored week) that agentgateway's fixed-length rolling window cannot reproduce exactly, so the migrated budget (if any) never resets"
					),
				});
			},
		}
	}
	for (_, key, _) in &kept {
		if team_handled.contains(key.key_hash.as_str()) {
			continue;
		}
		if let Some(max_budget) = key.max_budget {
			// A resetting window means LiteLLM gives the key its full max_budget back every
			// period (spend resets to 0 too), so the window amount is the full max_budget, not
			// the remaining balance - remaining only makes sense for a true lifetime cap.
			let amount = if key_budget_windows.contains_key(key.key_hash.as_str()) {
				max_budget
			} else {
				(max_budget - key.spend.unwrap_or(0.0).max(0.0)).max(0.0)
			};
			key_budgets.insert(key.key_hash.as_str(), amount);
		}
	}

	// Phase 3: emit.
	let mut mcp_api_keys: Vec<Value> = Vec::new();
	let mut migrated: Vec<(usize, &DbStateKey, String)> = Vec::new();
	for (index, key, name) in kept {
		let source_path = format!("db_state.keys[{index}]");
		let team = key.team_id.as_deref().and_then(|id| teams.get(id).copied());

		if key.rpm_limit.is_some() || key.tpm_limit.is_some() {
			let mut unmapped = Vec::new();
			if let Some(rpm) = key.rpm_limit {
				unmapped.push(format!("rpm_limit: {rpm}"));
			}
			if let Some(tpm) = key.tpm_limit {
				unmapped.push(format!("tpm_limit: {tpm}"));
			}
			plan.findings.push(ImportFinding {
				source_path: source_path.clone(),
				status: ImportStatus::Manual,
				message: format!(
					"Key has its own {}; agentgateway's localRateLimit has one numeric spec shared by every key behind a policy, so a per-key rate limit was not migrated. remoteRateLimit can express a per-key limit via a CEL descriptor keyed on apiKey.name, but it calls out to a separately deployed external rate-limit service for the actual counting - that service isn't something this importer can provision, so it isn't emitted automatically",
					unmapped.join(", ")
				),
			});
		}
		if let Some(soft_budget) = key.soft_budget {
			plan.findings.push(ImportFinding {
				source_path: source_path.clone(),
				status: ImportStatus::Unsupported,
				message: format!(
					"Key has a LiteLLM soft_budget (${soft_budget:.2}, an early-warning threshold below max_budget); agentgateway has no equivalent and only the hard max_budget was migrated"
				),
			});
		}
		if let Some(user_id) = &key.user_id {
			plan.findings.push(ImportFinding {
				source_path: source_path.clone(),
				status: ImportStatus::Unsupported,
				message: format!(
					"Key is associated with LiteLLM user {user_id:?}; agentgateway has no per-user construct and this association was not migrated"
				),
			});
		}

		// LiteLLM's own sentinels in a models list, not real model names: "all-proxy-models"
		// means no restriction at all; "all-team-models" on a key means explicit inheritance
		// (the same as an empty list); either one on a team means the team itself imposes no
		// narrowing. Naive exact-string intersection against these turns them into a silent
		// deny-all, which is the opposite of what they mean.
		let key_is_unrestricted = key.models.iter().any(|m| m == "all-proxy-models");
		let key_inherits_team = key.models.iter().any(|m| m == "all-team-models");
		let effective_key_models: &[String] = if key_inherits_team { &[] } else { &key.models };
		let team_is_unrestricted = team.is_some_and(|t| {
			t.models
				.iter()
				.any(|m| m == "all-proxy-models" || m == "all-team-models")
		});
		let effective_team_models: Option<&[String]> = team
			.map(|t| t.models.as_slice())
			.filter(|models| !models.is_empty() && !team_is_unrestricted);

		let allowed_models: Option<Vec<String>> = if key_is_unrestricted {
			plan.findings.push(ImportFinding {
				source_path: source_path.clone(),
				status: ImportStatus::Approximate,
				message: "Key's models includes LiteLLM's all-proxy-models sentinel, meaning no model restriction at all; allowedModels was not set to match".to_string(),
			});
			None
		} else {
			match (effective_key_models, effective_team_models) {
				([], Some(team_models)) => {
					plan.findings.push(ImportFinding {
						source_path: source_path.clone(),
						status: ImportStatus::Approximate,
						message: format!(
							"Key has no model list of its own; LiteLLM inherits its team's {} models. agentgateway has no team construct, so the team's current model list was copied onto this key - a later team model change won't propagate.",
							team_models.len()
						),
					});
					Some(team_models.to_vec())
				},
				([], None) => None,
				(models, Some(team_models)) => {
					// Pattern-aware, matching LiteLLM: a team pattern like "openai/*" grants
					// every model it matches, not only a model literally named "openai/*".
					let overlap: Vec<String> = models
						.iter()
						.filter(|model| {
							team_models
								.iter()
								.any(|pattern| litellm_model_matches(pattern, model))
						})
						.cloned()
						.collect();
					if overlap.is_empty() {
						plan.findings.push(ImportFinding {
							source_path: source_path.clone(),
							status: ImportStatus::Unsupported,
							message: "Key's models have no overlap with its team's models; LiteLLM would deny every model for this key. Migrated with allowedModels: [] (denies all) to match - check whether this key is meant to be usable at all.".to_string(),
						});
					}
					Some(overlap)
				},
				(models, None) => Some(models.to_vec()),
			}
		};

		let mut key_entry = Map::new();
		key_entry.insert(
			"keyHash".to_string(),
			json!(format!("sha256:{}", key.key_hash)),
		);
		key_entry.insert("metadata".to_string(), json!({"name": name}));
		if let Some(models) = &allowed_models {
			// A pattern agentgateway's own model matcher rejects (more than one wildcard, or an
			// empty name) would otherwise pass this import cleanly and only fail when the
			// generated config loads. Drop it instead, same fail-closed direction as the
			// zero-overlap case above.
			let (valid, invalid): (Vec<String>, Vec<String>) = models
				.iter()
				.cloned()
				.partition(|model| !model.is_empty() && is_supported_model_pattern(model));
			if !invalid.is_empty() {
				plan.findings.push(ImportFinding {
					source_path: source_path.clone(),
					status: ImportStatus::Unsupported,
					message: format!(
						"Key's models included {invalid:?}, which agentgateway's own model pattern rules reject (empty, or more than one wildcard); dropped from allowedModels rather than emitting a config the gateway would refuse to load"
					),
				});
			}
			key_entry.insert("allowedModels".to_string(), json!(valid));
		}
		if let Some(amount) = key_budgets.get(key.key_hash.as_str()) {
			let mut budget = Map::from_iter([
				("name".to_string(), json!("litellm")),
				// Floor, never round: rounding up could grant a cent more than is actually left.
				(
					"limit".to_string(),
					json!({"unit": "USD", "amount": (amount * 100.0).floor() / 100.0}),
				),
				("onBudgetExceeded".to_string(), json!("Block")),
			]);
			if let Some(window) = key_budget_windows.get(key.key_hash.as_str()) {
				budget.insert("window".to_string(), json!({"rolling": window}));
			}
			key_entry.insert("budgets".to_string(), json!([budget]));
		}
		plan.db_api_keys.push(Value::Object(key_entry));
		mcp_api_keys.push(json!({
			"keyHash": format!("sha256:{}", key.key_hash),
			"metadata": {"name": name},
		}));
		plan.findings.push(ImportFinding {
			source_path,
			status: ImportStatus::Exact,
			message: format!("Migrated key {name:?} to llm.policies.apiKey (hash-based, no plaintext)"),
		});
		migrated.push((index, key, name));
	}

	// Phase 4: mcp.policies - mirror migrated keys for mcp.policies.apiKey (any valid key must
	// be able to authenticate there to reach an allow_all_keys target, matching LiteLLM's own
	// behavior), and build mcpAuthorization rules from DB-state MCP grants, allow_all_keys
	// servers, and an admin-bypass rule for the master key.
	let has_mcp_candidates =
		!plan.mcp_targets.is_empty() || !plan.pending_restricted_mcp_targets.is_empty();
	if has_mcp_candidates && (!migrated.is_empty() || plan.inbound_api_key.is_some()) {
		let servers_by_id: HashMap<&str, &str> = state
			.mcp_servers
			.iter()
			.map(|server| (server.server_id.as_str(), server.server_name.as_str()))
			.collect();
		let server_names: HashSet<&str> = state
			.mcp_servers
			.iter()
			.map(|server| server.server_name.as_str())
			.collect();

		// Closed wins on a disagreement: if DB-state says allow_all_keys: false for a server
		// config.yaml itself left open, move its already-built spec into the same held-back
		// pool a config.yaml-restricted server uses - reachable only via an explicit
		// per-key/per-team grant from here on, same as if config.yaml had said false itself.
		const DISAGREEMENT_REASON: &str =
			"config.yaml left this server open, but DB-state's allow_all_keys: false disagreed";
		for server in &state.mcp_servers {
			if server.allow_all_keys {
				continue;
			}
			// If config.yaml already held this server back, the emitted target with the same
			// sanitized name belongs to a different server and must not be closed as a
			// disagreement against this one.
			if plan
				.held_back_mcp_servers
				.contains(server.server_name.as_str())
			{
				continue;
			}
			let Some(target_name) = plan
				.mcp_target_names
				.get(server.server_name.as_str())
				.cloned()
			else {
				continue;
			};
			if let Some(position) = plan
				.mcp_targets
				.iter()
				.position(|target| target.get("name").and_then(Value::as_str) == Some(target_name.as_str()))
			{
				let spec = plan.mcp_targets.remove(position);
				plan
					.pending_restricted_mcp_targets
					.push((spec, DISAGREEMENT_REASON));
				plan.findings.push(ImportFinding {
					source_path: format!("db_state.mcp_servers.{}", server.server_name),
					status: ImportStatus::Approximate,
					message: format!(
						"MCP server {:?}: {DISAGREEMENT_REASON} - closed by default, only reachable now via an explicit per-key/per-team grant",
						server.server_name
					),
				});
			}
		}
		let mut rules: Vec<String> = Vec::new();
		let mut activated_targets: HashSet<String> = HashSet::new();
		// Source the open set from plan.mcp_targets (servers config.yaml treated as open, after
		// the disagreement pass above has already moved any DB-closed server into the held-back
		// pool), not from DB-state's own allow_all_keys flag directly: once any mcpAuthorization
		// rule exists below, closed-world semantics deny every target with no matching rule, so
		// an openly-emitted target would get silently locked out if it had no rule of its own.
		for target in &plan.mcp_targets {
			let Some(name) = target.get("name").and_then(Value::as_str) else {
				continue;
			};
			activated_targets.insert(name.to_string());
			rules.push(mcp_target_rule(name));
		}
		for server in &state.mcp_servers {
			if !server.allow_all_keys {
				continue;
			}
			let target = plan.mcp_target_names.get(server.server_name.as_str());
			if !target.is_some_and(|target| activated_targets.contains(target)) {
				plan.findings.push(ImportFinding {
					source_path: format!("db_state.mcp_servers.{}", server.server_name),
					status: ImportStatus::Manual,
					message: format!(
						"MCP server {:?} is open to every key (allow_all_keys) per DB-state, but has no emitted, open agentgateway target to grant access to - check its config.yaml mcp_servers import findings",
						server.server_name
					),
				});
			}
		}
		for (index, key, name) in &migrated {
			// Team-level object_permission.mcp_servers interacts with a key's own grant like
			// LiteLLM's `models` does: a key with no grants of its own inherits its team's full
			// list; a key with its own grants is capped to the intersection with a non-empty
			// team list; an empty/absent team list never narrows or grants anything by itself.
			let team = key.team_id.as_deref().and_then(|id| teams.get(id).copied());
			let team_grants: Option<&Vec<String>> = team
				.map(|team| &team.object_permission)
				.filter(|grants| !grants.is_empty());
			let inherited_from_team = key.mcp_servers.is_empty() && team_grants.is_some();
			let effective_grants: &[String] = if inherited_from_team {
				team_grants.expect("checked above")
			} else {
				&key.mcp_servers
			};
			if inherited_from_team {
				plan.findings.push(ImportFinding {
					source_path: format!("db_state.keys[{index}]"),
					status: ImportStatus::Approximate,
					message: format!(
						"Key {name:?} has no MCP server grants of its own; LiteLLM inherits its team's object_permission.mcp_servers list. agentgateway has no team construct, so the team's current grants were copied onto this key - a later team grant change won't propagate."
					),
				});
			}
			let team_ceiling: Option<HashSet<&str>> = if inherited_from_team {
				None
			} else {
				team_grants.map(|grants| {
					grants
						.iter()
						.filter_map(|grant| {
							servers_by_id
								.get(grant.as_str())
								.copied()
								.or_else(|| server_names.get(grant.as_str()).copied())
						})
						.collect()
				})
			};
			for (grant_index, grant) in effective_grants.iter().enumerate() {
				let source_path = format!("db_state.keys[{index}].mcp_servers[{grant_index}]");
				let resolved_name = servers_by_id
					.get(grant.as_str())
					.copied()
					.or_else(|| server_names.get(grant.as_str()).copied());
				let Some(server_name) = resolved_name else {
					plan.findings.push(ImportFinding {
						source_path,
						status: ImportStatus::Manual,
						message: format!(
							"Key {name:?} is granted MCP server {grant:?}, which is not a known DB-state MCP server, and was not migrated"
						),
					});
					continue;
				};
				if let Some(team_ceiling) = &team_ceiling
					&& !team_ceiling.contains(server_name)
				{
					plan.findings.push(ImportFinding {
						source_path,
						status: ImportStatus::Approximate,
						message: format!(
							"Key {name:?} is granted MCP server {server_name:?}, but its team's own object_permission.mcp_servers does not include it; LiteLLM's effective access is the intersection of both, so this grant was not migrated"
						),
					});
					continue;
				}
				let Some(target) = plan.mcp_target_names.get(server_name) else {
					plan.findings.push(ImportFinding {
						source_path,
						status: ImportStatus::Manual,
						message: format!(
							"Key {name:?} is granted MCP server {server_name:?}, which has no emitted agentgateway target, and was not migrated"
						),
					});
					continue;
				};
				activated_targets.insert(target.clone());
				rules.push(format!(
					"apiKey.name == {} && ({})",
					cel_escape_string(name),
					mcp_target_rule(target)
				));
			}
		}
		if let Some(master) = &plan.inbound_api_key {
			// Mirror the master key's own value (not virtual keys, which only have a hash here)
			// so it can actually authenticate against mcp.policies.apiKey before the bypass rule
			// below ever gets evaluated.
			mcp_api_keys.push(json!({
				"key": master.key.clone(),
				"metadata": {"name": "master"},
			}));
			rules.push("apiKey.name == \"master\"".to_string());
		}
		// Collision safety: if two or more held-back targets ended up sharing the same
		// sanitized name (two restricted config.yaml servers, or a disagreement-closed open
		// server landing on the same name as an already-restricted one), a grant naming either
		// server can't be told apart from the other at promotion time - mcp.tool.target is the
		// only identity agentgateway itself understands. Retire every such name from promotion
		// entirely rather than risk resolving a grant to the wrong backend.
		let mut pending_name_counts: HashMap<&str, usize> = HashMap::new();
		for (spec, _) in &plan.pending_restricted_mcp_targets {
			if let Some(name) = spec.get("name").and_then(Value::as_str) {
				*pending_name_counts.entry(name).or_insert(0) += 1;
			}
		}
		let ambiguous_names: HashSet<String> = pending_name_counts
			.into_iter()
			.filter(|(_, count)| *count > 1)
			.map(|(name, _)| name.to_string())
			.collect();
		let pending = std::mem::take(&mut plan.pending_restricted_mcp_targets);
		for (spec, held_back_reason) in pending {
			let Some(name) = spec.get("name").and_then(Value::as_str) else {
				plan
					.pending_restricted_mcp_targets
					.push((spec, held_back_reason));
				continue;
			};
			if ambiguous_names.contains(name) {
				plan.findings.push(ImportFinding {
					source_path: format!("db_state.mcp_servers.{name}"),
					status: ImportStatus::Unsupported,
					message: format!(
						"MCP target name {name:?} is shared by more than one held-back LiteLLM server; a grant can't be resolved to the right one, so none of them were migrated"
					),
				});
				continue;
			}
			plan
				.pending_restricted_mcp_targets
				.push((spec, held_back_reason));
		}
		// Promote a held-back restricted target into mcp.targets only once it has an actual
		// mcpAuthorization rule protecting it - never unconditionally, since closed-world
		// semantics deny every target with no matching rule (promoting it with no rule would
		// leave it unreachable by anyone, not open to everyone).
		for (spec, held_back_reason) in std::mem::take(&mut plan.pending_restricted_mcp_targets) {
			let Some(name) = spec.get("name").and_then(Value::as_str) else {
				continue;
			};
			if !activated_targets.contains(name) {
				continue;
			}
			let already_emitted = plan
				.mcp_targets
				.iter()
				.any(|target| target.get("name").and_then(Value::as_str) == Some(name));
			if already_emitted {
				plan.findings.push(ImportFinding {
					source_path: format!("db_state.mcp_servers.{name}"),
					status: ImportStatus::Unsupported,
					message: format!(
						"MCP target {name:?}, held back because {held_back_reason}, was not restored; its sanitized name is already used by an emitted server"
					),
				});
				continue;
			}
			plan.findings.push(ImportFinding {
				source_path: format!("db_state.mcp_servers.{name}"),
				status: ImportStatus::Approximate,
				message: format!(
					"Emitted MCP target {name:?}, held back because {held_back_reason}, now restored by a real mcpAuthorization grant"
				),
			});
			plan.mcp_targets.push(spec);
		}
		plan.mcp_authorization_rules = rules;
		plan.mcp_api_keys = mcp_api_keys;
	}
}

#[cfg(test)]
#[path = "../tests/import/cache.rs"]
mod cache_tests;

#[cfg(test)]
mod tests {
	use std::fs;
	use std::path::{Path, PathBuf};

	use super::*;
	use crate::import::{ImportResult, import_config, import_config_with_options};

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
			snapshot_path => "../tests/import/litellm",
		}, {
			insta::assert_yaml_snapshot!(name, result);
		});
	}

	fn assert_litellm_bail(name: &str, expected_message_substring: &str) {
		let input_path = fixture_path(name);
		let input = fs::read_to_string(&input_path)
			.unwrap_or_else(|error| panic!("{}: {error}", input_path.display()));
		let error = import_config("litellm", &input)
			.expect_err(&format!("{}: expected an error", input_path.display()));
		let message = error.to_string();
		assert!(
			message.contains(expected_message_substring),
			"{}: expected {expected_message_substring:?} in {message:?}",
			input_path.display()
		);
	}

	/// Like [`assert_litellm_bail`], but requires the exact message: several
	/// `master_key` failure modes only differ by wording, and a substring match
	/// on "master_key" alone can't tell them apart.
	fn assert_litellm_bail_exact(name: &str, expected_message: &str) {
		let input_path = fixture_path(name);
		let input = fs::read_to_string(&input_path)
			.unwrap_or_else(|error| panic!("{}: {error}", input_path.display()));
		let error = import_config("litellm", &input)
			.expect_err(&format!("{}: expected an error", input_path.display()));
		assert_eq!(
			error.to_string(),
			expected_message,
			"{}",
			input_path.display()
		);
	}

	/// Like `assert_litellm_golden`, but also reads `{name}.state.json` (LiteLLM DB-state) and
	/// feeds it through `import_config_with_options` alongside the config.yaml. `now` fixes the
	/// expiry reference point (an RFC3339 timestamp) so snapshots never rot with wall-clock time.
	fn assert_litellm_db_state_golden(name: &str, now: &str) {
		let input_path = fixture_path(name);
		let input = fs::read_to_string(&input_path)
			.unwrap_or_else(|error| panic!("{}: {error}", input_path.display()));
		let state_path = Path::new("src/tests/import/litellm").join(format!("{name}.state.json"));
		let state_json = fs::read_to_string(&state_path)
			.unwrap_or_else(|error| panic!("{}: {error}", state_path.display()));
		let state: LiteLlmDbState = serde_json::from_str(&state_json)
			.unwrap_or_else(|error| panic!("{}: {error}", state_path.display()));
		let options = ImportOptions {
			now: now
				.parse()
				.unwrap_or_else(|error| panic!("{now:?}: {error}")),
			db_state: Some(state),
			..ImportOptions::default()
		};
		let result = import_config_with_options("litellm", &input, &options)
			.unwrap_or_else(|error| panic!("{}: {error}", input_path.display()));

		insta::with_settings!({
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "../tests/import/litellm",
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
	fn reports_litellm_cache_settings_without_reusing_them() {
		assert_litellm_golden("cache-settings");
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
	fn imports_litellm_inbound_api_key_policy() {
		assert_litellm_golden("inbound-api-key");
	}

	#[test]
	fn imports_literal_litellm_master_key() {
		assert_litellm_golden("inbound-api-key-literal");
	}

	#[test]
	fn rejects_empty_litellm_master_key() {
		assert_litellm_bail_exact(
			"inbound-api-key-empty-literal",
			"general_settings.master_key must not be empty",
		);
	}

	#[test]
	fn warns_on_literal_litellm_master_key_containing_dollar() {
		assert_litellm_golden("inbound-api-key-literal-dollar");
	}

	#[test]
	fn reports_litellm_api_key_header_without_master_key() {
		assert_litellm_golden("inbound-api-key-header-without-master-key");
	}

	#[test]
	fn points_at_environment_variables_master_key_when_general_settings_lacks_one() {
		assert_litellm_golden("inbound-api-key-master-key-in-environment-variables");
	}

	#[test]
	fn rejects_invalid_litellm_api_key_header_name() {
		assert_litellm_bail("inbound-api-key-invalid-header", "litellm_key_header_name");
	}

	#[test]
	fn rejects_non_string_litellm_master_key() {
		assert_litellm_bail_exact(
			"inbound-api-key-non-string",
			"general_settings.master_key must be a string, got 12345",
		);
	}

	#[test]
	fn rejects_empty_litellm_master_key_environment_reference() {
		assert_litellm_bail_exact(
			"inbound-api-key-empty-env",
			"general_settings.master_key's environment reference must name a variable",
		);
	}

	#[test]
	fn rejects_non_string_litellm_api_key_header_name() {
		assert_litellm_bail(
			"inbound-api-key-non-string-header",
			"litellm_key_header_name",
		);
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
	fn maps_litellm_cooldown_settings_to_model_health_eviction() {
		assert_litellm_golden("cooldown-eviction");
	}

	#[test]
	fn maps_litellm_router_timeout_to_llm_policies_timeout() {
		assert_litellm_golden("timeout");
	}

	#[test]
	fn maps_fractional_litellm_router_timeout() {
		assert_litellm_golden("timeout-fractional");
	}

	#[test]
	fn reports_invalid_litellm_router_timeout_without_emitting_policy() {
		assert_litellm_golden("timeout-invalid");
	}

	#[test]
	fn reports_unmapped_per_deployment_litellm_timeout_override() {
		assert_litellm_golden("timeout-per-deployment-override");
	}

	#[test]
	fn maps_litellm_http_mcp_server_to_mcp_target() {
		assert_litellm_golden("mcp-server-http");
	}

	#[test]
	fn maps_litellm_sse_mcp_server_with_static_headers() {
		assert_litellm_golden("mcp-server-sse-static-headers");
	}

	#[test]
	fn maps_litellm_stdio_mcp_server() {
		assert_litellm_golden("mcp-server-stdio");
	}

	#[test]
	fn mirrors_custom_api_key_header_onto_mcp_policies() {
		assert_litellm_golden("mcp-server-with-custom-header");
	}

	#[test]
	fn drops_all_mcp_targets_when_no_inbound_key_exists() {
		assert_litellm_golden("mcp-drop-without-master-key");
	}

	#[test]
	fn multiplexes_an_open_and_a_restricted_server_behind_one_endpoint() {
		assert_litellm_golden("mcp-multiplex-and-restricted");
	}

	#[test]
	fn maps_litellm_mcp_server_alias_and_mcp_aliases() {
		assert_litellm_golden("mcp-alias");
	}

	#[test]
	fn reports_invalid_litellm_mcp_servers_without_emitting_them() {
		assert_litellm_golden("mcp-invalid");
	}

	#[test]
	fn imports_mcp_only_litellm_config_without_llm_block() {
		assert_litellm_golden("mcp-only-no-models");
	}

	#[test]
	fn defaults_omitted_allow_all_keys_to_restricted() {
		assert_litellm_golden("mcp-allow-all-keys-absent");
	}

	#[test]
	fn treats_non_boolean_allow_all_keys_as_restricted() {
		assert_litellm_golden("mcp-allow-all-keys-non-boolean");
	}

	#[test]
	fn does_not_drop_an_open_mcp_server_when_a_restricted_server_shares_its_name() {
		assert_litellm_golden("mcp-restricted-open-name-collision");
	}

	#[test]
	fn maps_litellm_bedrock_guardrail_to_prompt_guard() {
		assert_litellm_golden("guardrail-bedrock");
	}

	#[test]
	fn maps_litellm_bedrock_guardrail_with_default_on_true() {
		assert_litellm_golden("guardrail-bedrock-default-on");
	}

	#[test]
	fn reports_unsupported_litellm_guardrail_type() {
		assert_litellm_golden("guardrail-unsupported-type");
	}

	#[test]
	fn reports_invalid_litellm_guardrails_without_emitting_them() {
		assert_litellm_golden("guardrail-invalid");
	}

	#[test]
	fn maps_litellm_pass_through_endpoint_to_standalone_route() {
		assert_litellm_golden("pass-through-endpoint");
	}

	#[test]
	fn pass_through_endpoint_auth_honors_custom_header() {
		assert_litellm_golden("pass-through-endpoint-custom-header");
	}

	#[test]
	fn maps_litellm_pass_through_endpoint_without_auth_or_rewrite() {
		assert_litellm_golden("pass-through-endpoint-no-auth-no-rewrite");
	}

	#[test]
	fn does_not_emit_litellm_pass_through_endpoint_requiring_auth_without_master_key() {
		assert_litellm_golden("pass-through-endpoint-auth-without-master-key");
	}

	#[test]
	fn reports_invalid_litellm_pass_through_endpoints_without_emitting_them() {
		assert_litellm_golden("pass-through-endpoint-invalid");
	}

	#[test]
	fn maps_https_pass_through_endpoint_and_reports_unmapped_fields() {
		assert_litellm_golden("pass-through-endpoint-https-and-fields");
	}

	#[test]
	fn reports_stdio_mcp_static_headers_and_malformed_args_env() {
		assert_litellm_golden("mcp-server-stdio-invalid");
	}

	#[test]
	fn switches_weighted_routing_to_order_based_failover_without_fallbacks() {
		assert_litellm_golden("order-priority-no-fallbacks");
	}

	#[test]
	fn shifts_fallback_priority_tiers_after_order_based_failover() {
		assert_litellm_golden("order-priority-with-fallbacks");
	}

	#[test]
	fn leaves_weighted_routing_when_order_is_only_set_on_some_deployments() {
		assert_litellm_golden("order-partial");
	}

	#[test]
	fn reports_invalid_litellm_order_without_using_it_for_priority() {
		assert_litellm_golden("order-invalid");
	}

	#[test]
	fn leaves_weighted_routing_when_order_is_tied_across_deployments() {
		assert_litellm_golden("order-tied");
	}

	#[test]
	fn drops_stale_rpm_weight_findings_for_a_mixed_order_tie() {
		assert_litellm_golden("order-mixed-ties");
	}

	#[test]
	fn does_not_emit_llm_policies_timeout_when_router_timeout_absent() {
		let mut plan = ImportPlan::default();
		import_litellm_timeout(
			json!({}).as_object().expect("router settings object"),
			&mut plan,
		);
		assert!(plan.timeout.is_none());
		assert!(plan.findings.is_empty());
	}

	#[test]
	fn does_not_emit_litellm_health_for_partial_or_invalid_pairs() {
		let mut partial = ImportPlan::default();
		apply_litellm_health(
			json!({ "allowed_fails": 3 })
				.as_object()
				.expect("router settings object"),
			&mut partial,
		);
		assert!(partial.model_health.is_none());
		assert_eq!(partial.findings.len(), 1);
		assert_eq!(partial.findings[0].status, ImportStatus::Manual);

		let mut invalid = ImportPlan::default();
		apply_litellm_health(
			json!({ "allowed_fails": i32::MAX, "cooldown_time": 60 })
				.as_object()
				.expect("router settings object"),
			&mut invalid,
		);
		assert!(invalid.model_health.is_none());
		assert_eq!(
			invalid
				.findings
				.iter()
				.map(|finding| finding.status)
				.collect::<Vec<_>>(),
			vec![ImportStatus::Manual, ImportStatus::Manual]
		);
	}

	#[test]
	fn maps_zero_litellm_allowed_fails_to_single_failure_threshold() {
		let mut plan = ImportPlan::default();
		apply_litellm_health(
			json!({ "allowed_fails": 0, "cooldown_time": 60 })
				.as_object()
				.expect("router settings object"),
			&mut plan,
		);
		assert_eq!(
			plan.model_health,
			Some(json!({
				"eviction": {
					"consecutiveFailures": 1,
					"duration": "60s",
				}
			}))
		);
		assert_eq!(
			plan
				.findings
				.iter()
				.map(|finding| finding.status)
				.collect::<Vec<_>>(),
			vec![ImportStatus::Approximate, ImportStatus::Approximate]
		);

		let mut negative = ImportPlan::default();
		apply_litellm_health(
			json!({ "allowed_fails": -1, "cooldown_time": 60 })
				.as_object()
				.expect("router settings object"),
			&mut negative,
		);
		assert!(negative.model_health.is_none());
	}

	#[test]
	fn does_not_emit_litellm_health_for_cooldown_only_input() {
		let mut plan = ImportPlan::default();
		apply_litellm_health(
			json!({ "cooldown_time": 60 })
				.as_object()
				.expect("router settings object"),
			&mut plan,
		);
		assert!(plan.model_health.is_none());
		assert_eq!(plan.findings.len(), 1);
		assert_eq!(
			plan.findings[0].source_path,
			"router_settings.cooldown_time"
		);
		assert_eq!(plan.findings[0].status, ImportStatus::Manual);
	}

	#[test]
	fn does_not_emit_litellm_health_for_valid_threshold_with_invalid_duration() {
		for cooldown_time in [json!(0), json!(-5), json!("60s")] {
			let mut plan = ImportPlan::default();
			apply_litellm_health(
				json!({ "allowed_fails": 3, "cooldown_time": cooldown_time })
					.as_object()
					.expect("router settings object"),
				&mut plan,
			);
			assert!(
				plan.model_health.is_none(),
				"cooldown_time {cooldown_time} should not be mapped"
			);
			assert_eq!(
				plan
					.findings
					.iter()
					.map(|finding| (finding.source_path.as_str(), finding.status))
					.collect::<Vec<_>>(),
				vec![
					("router_settings.allowed_fails", ImportStatus::Manual),
					("router_settings.cooldown_time", ImportStatus::Manual),
				]
			);
			assert_eq!(
				plan.findings[0].message,
				"LiteLLM allowed_fails was not mapped because cooldown_time is invalid"
			);
		}
	}

	#[test]
	fn disable_cooldowns_suppresses_litellm_health() {
		let mut plan = ImportPlan::default();
		apply_litellm_health(
			json!({
				"allowed_fails": 3,
				"cooldown_time": 60,
				"disable_cooldowns": true,
			})
			.as_object()
			.expect("router settings object"),
			&mut plan,
		);
		assert!(plan.model_health.is_none());
		assert_eq!(
			plan
				.findings
				.iter()
				.map(|finding| (finding.source_path.as_str(), finding.status))
				.collect::<Vec<_>>(),
			vec![
				("router_settings.disable_cooldowns", ImportStatus::Exact),
				("router_settings.allowed_fails", ImportStatus::Exact),
				("router_settings.cooldown_time", ImportStatus::Exact),
			]
		);
	}

	#[test]
	fn keeps_litellm_health_findings_for_invalid_disable_cooldowns() {
		let mut plan = ImportPlan::default();
		apply_litellm_health(
			json!({
				"allowed_fails": 3,
				"cooldown_time": 60,
				"disable_cooldowns": "yes",
			})
			.as_object()
			.expect("router settings object"),
			&mut plan,
		);
		assert!(plan.model_health.is_none());
		assert_eq!(
			plan
				.findings
				.iter()
				.map(|finding| (finding.source_path.as_str(), finding.status))
				.collect::<Vec<_>>(),
			vec![
				("router_settings.disable_cooldowns", ImportStatus::Manual),
				("router_settings.allowed_fails", ImportStatus::Manual),
				("router_settings.cooldown_time", ImportStatus::Manual),
			]
		);
	}

	fn find_status(result: &ImportResult, source_path: &str) -> ImportStatus {
		result
			.findings
			.iter()
			.find(|finding| finding.source_path == source_path)
			.unwrap_or_else(|| panic!("missing finding for {source_path}"))
			.status
	}

	fn imported_models(result: &ImportResult) -> &Vec<Value> {
		result.config["llm"]["models"]
			.as_array()
			.expect("generated models")
	}

	#[test]
	fn disable_cooldowns_with_fallbacks_requires_manual_migration() {
		let result = import_config(
			"litellm",
			r#"
model_list:
- model_name: primary
  litellm_params:
    model: openai/gpt-4o
- model_name: backup
  litellm_params:
    model: openai/gpt-4o-mini
router_settings:
  fallbacks:
  - primary: [backup]
  allowed_fails: 3
  cooldown_time: 60
  disable_cooldowns: true
"#,
		)
		.expect("import litellm config");

		assert!(
			imported_models(&result)
				.iter()
				.all(|model| model.get("health").is_none())
		);
		for source_path in [
			"router_settings.disable_cooldowns",
			"router_settings.allowed_fails",
			"router_settings.cooldown_time",
		] {
			assert_eq!(find_status(&result, source_path), ImportStatus::Manual);
		}
	}

	#[test]
	fn leaves_litellm_cooldown_overrides_for_manual_migration() {
		let result = import_config(
			"litellm",
			r#"
model_list:
- model_name: primary
  litellm_params:
    model: openai/gpt-4o
    cooldown_time: 5
- model_name: backup
  litellm_params:
    model: openai/gpt-4o-mini
router_settings:
  allowed_fails: 3
  cooldown_time: 60
"#,
		)
		.expect("import litellm config");

		let health_by_model = imported_models(&result)
			.iter()
			.map(|model| {
				(
					model["params"]["model"].as_str().expect("upstream model"),
					model.get("health").cloned(),
				)
			})
			.collect::<HashMap<_, _>>();
		assert_eq!(health_by_model["gpt-4o"], None);
		assert_eq!(
			health_by_model["gpt-4o-mini"],
			Some(json!({
				"eviction": {
					"consecutiveFailures": 4,
					"duration": "60s",
				}
			}))
		);
		assert_eq!(
			find_status(&result, "model_list[0].litellm_params.cooldown_time"),
			ImportStatus::Manual
		);
		assert_eq!(
			find_status(&result, "router_settings.allowed_fails"),
			ImportStatus::Approximate
		);
	}

	#[test]
	fn exception_specific_thresholds_suppress_generic_litellm_health() {
		let result = import_config(
			"litellm",
			r#"
model_list:
- model_name: primary
  litellm_params:
    model: openai/gpt-4o
router_settings:
  allowed_fails: 3
  cooldown_time: 60
  allowed_fails_policy:
    RateLimitErrorAllowedFails: 10
"#,
		)
		.expect("import litellm config");

		assert!(
			imported_models(&result)
				.iter()
				.all(|model| model.get("health").is_none())
		);
		for source_path in [
			"router_settings.allowed_fails_policy",
			"router_settings.allowed_fails",
			"router_settings.cooldown_time",
		] {
			assert_eq!(find_status(&result, source_path), ImportStatus::Manual);
		}
	}

	#[test]
	fn model_info_health_overrides_suppress_health_only_for_that_deployment() {
		for (key, value) in [
			("cooldown_time", "5"),
			("allowed_fails", "0"),
			("allowed_fails_policy", "{ RateLimitErrorAllowedFails: 1 }"),
		] {
			let result = import_config(
				"litellm",
				&format!(
					r#"
model_list:
- model_name: primary
  litellm_params:
    model: openai/gpt-4o
  model_info:
    {key}: {value}
- model_name: backup
  litellm_params:
    model: openai/gpt-4o-mini
router_settings:
  allowed_fails: 3
  cooldown_time: 60
"#
				),
			)
			.expect("import litellm config");

			let health_by_model = imported_models(&result)
				.iter()
				.map(|model| {
					(
						model["params"]["model"].as_str().expect("upstream model"),
						model.get("health").cloned(),
					)
				})
				.collect::<HashMap<_, _>>();
			assert_eq!(health_by_model["gpt-4o"], None);
			assert_eq!(
				health_by_model["gpt-4o-mini"],
				Some(json!({
					"eviction": {
						"consecutiveFailures": 4,
						"duration": "60s",
					}
				}))
			);
			assert_eq!(
				find_status(&result, &format!("model_list[0].model_info.{key}")),
				ImportStatus::Manual
			);
		}
	}

	#[test]
	fn applies_default_fallbacks_to_routes_without_explicit_fallbacks() {
		assert_litellm_golden("default-fallbacks");
	}

	#[test]
	fn reports_shadowed_and_noop_default_fallbacks() {
		assert_litellm_golden("default-fallbacks-shadowed");
	}

	#[test]
	fn reports_wildcard_default_fallback_limitations() {
		assert_litellm_golden("default-fallbacks-wildcard");
	}

	#[test]
	fn flattens_branching_fallbacks_with_depth_budget() {
		assert_litellm_golden("default-fallbacks-branching");
	}

	#[test]
	fn disables_fallbacks_when_max_fallbacks_is_zero() {
		assert_litellm_golden("default-fallbacks-max-zero");
	}

	#[test]
	fn reports_invalid_max_fallbacks() {
		assert_litellm_golden("default-fallbacks-max-invalid");
	}

	#[test]
	fn disables_explicit_fallbacks_when_max_fallbacks_is_zero_without_defaults() {
		assert_litellm_golden("max-fallbacks-zero-without-defaults");
	}

	#[test]
	fn reports_conditional_fallback_settings_as_manual() {
		assert_litellm_golden("conditional-fallbacks");
	}

	#[test]
	fn counts_unimported_fallback_targets_against_depth() {
		assert_litellm_golden("default-fallbacks-unimported-depth");
	}

	#[test]
	fn maps_db_state_keys_models_and_team_inheritance() {
		assert_litellm_db_state_golden("db-state-keys-basic", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn retracts_db_state_key_migration_claim_when_no_models_were_imported() {
		assert_litellm_db_state_golden("db-state-no-models", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn skips_blocked_expired_and_colliding_db_state_keys() {
		assert_litellm_db_state_golden("db-state-skip-rules", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn splits_team_budget_pool_and_scales_down_on_overflow() {
		assert_litellm_db_state_golden("db-state-budgets", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn maps_mcp_grants_allow_all_keys_and_master_key_bypass() {
		assert_litellm_db_state_golden("db-state-mcp-grants", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn reports_unresolvable_mcp_grants_as_manual() {
		assert_litellm_db_state_golden("db-state-mcp-grant-gaps", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn reports_rate_limits_and_other_unmapped_fields_as_findings() {
		assert_litellm_db_state_golden("db-state-not-consumed-fields", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn applies_team_mcp_grant_inheritance_and_intersection() {
		assert_litellm_db_state_golden("db-state-mcp-team-permission", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn maps_daily_and_hourly_budget_duration_to_a_resetting_window() {
		assert_litellm_db_state_golden("db-state-budget-window", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn carries_custom_header_onto_db_state_mcp_policies() {
		assert_litellm_db_state_golden("db-state-mcp-custom-header", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn promoted_restricted_mcp_server_keeps_its_static_headers() {
		assert_litellm_db_state_golden(
			"db-state-mcp-restricted-static-headers",
			"2026-01-01T00:00:00Z",
		);
	}

	#[test]
	fn never_promotes_an_mcp_grant_to_a_different_server_sharing_its_name() {
		assert_litellm_db_state_golden("db-state-ambiguous-mcp-grant", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn applies_no_resetting_window_to_a_team_lifetime_pool_share() {
		assert_litellm_db_state_golden(
			"db-state-team-lifetime-pool-no-window",
			"2026-01-01T00:00:00Z",
		);
	}

	#[test]
	fn clamps_a_negative_team_spend_instead_of_inflating_the_pool() {
		assert_litellm_db_state_golden("db-state-negative-spend-clamped", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn skips_a_key_whose_hash_duplicates_another_key() {
		assert_litellm_db_state_golden("db-state-duplicate-key-hash", "2026-01-01T00:00:00Z");
	}

	#[test]
	fn resolves_litellm_model_sentinels_and_wildcards() {
		assert_litellm_db_state_golden(
			"db-state-model-sentinels-and-wildcards",
			"2026-01-01T00:00:00Z",
		);
	}

	#[test]
	fn reports_allow_all_keys_disagreement_between_config_and_db_state() {
		assert_litellm_db_state_golden(
			"db-state-mcp-allow-all-keys-mismatch",
			"2026-01-01T00:00:00Z",
		);
	}

	#[test]
	fn restores_restricted_target_for_non_boolean_allow_all_keys_when_granted() {
		assert_litellm_db_state_golden(
			"db-state-mcp-allow-all-keys-non-boolean",
			"2026-01-01T00:00:00Z",
		);
	}

	#[test]
	fn does_not_promote_a_restricted_mcp_server_onto_an_open_server_name() {
		assert_litellm_db_state_golden(
			"db-state-mcp-restricted-open-name-collision",
			"2026-01-01T00:00:00Z",
		);
	}
}
