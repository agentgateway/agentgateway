use std::collections::HashSet;
use std::io::Write;

use agentgateway::cel;
use agentgateway::config_store::ConfigResourceKind;
use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde_json::{Value, json};

/// Types that appear in many places in the configuration. Their fields are documented once,
/// in their own section, rather than under every field that uses them.
/// Each entry is the schema definition name and the name shown in the docs.
const SHARED_CONFIG_TYPES: &[(&str, &str)] = &[
	("SimpleLocalBackendPolicies", "BackendPolicies"),
	("BackendAuthCompat", "BackendAuth"),
	("PromptGuard", "Guardrails"),
];

pub fn generate_schema() -> Result<()> {
	struct SchemaDoc {
		name: &'static str,
		mdfile: Option<&'static str>,
		file: &'static str,
		schema_json: String,
		shared: &'static [(&'static str, &'static str)],
		appendix: String,
	}

	let xtask_path = std::env::var("CARGO_MANIFEST_DIR")?;
	let (config_json, resources_md) =
		add_resources(&make::<agentgateway::types::local::LocalConfig>(false)?)?;
	let schemas = vec![
		SchemaDoc {
			name: "Configuration File",
			mdfile: Some("config.md"),
			file: "config.json",
			schema_json: config_json,
			shared: SHARED_CONFIG_TYPES,
			appendix: resources_md,
		},
		SchemaDoc {
			name: "CEL context",
			mdfile: Some("cel.md"),
			file: "cel.json",
			// CEL is simpler so we just always inline
			schema_json: make::<cel::ExecutorSerde>(true)?,
			shared: &[],
			appendix: String::new(),
		},
		SchemaDoc {
			name: "Admin Configuration Dump",
			mdfile: None,
			file: "admin.json",
			schema_json: make_with_contract::<agentgateway::store::StoresDump>(
				false,
				schemars::generate::Contract::Serialize,
			)?,
			shared: &[],
			appendix: String::new(),
		},
	];
	for schema in &schemas {
		let rule_path = format!("{xtask_path}/../../schema/{}", schema.file);
		let mut file = fs_err::File::create(rule_path)?;
		file.write_all(schema.schema_json.as_bytes())?;
	}

	for schema in schemas {
		let Some(mdfile) = schema.mdfile else {
			continue;
		};
		let rule_path = format!("{xtask_path}/../../schema/{}", schema.file);
		let shared = format!(
			"{{{}}}",
			schema
				.shared
				.iter()
				.map(|(def, name)| format!("\"{def}\":\"{name}\""))
				.collect::<Vec<_>>()
				.join(",")
		);
		let table = |root: &str| -> Result<String> {
			let o = std::process::Command::new(format!("{xtask_path}/../../tools/schema-to-md.sh"))
				.arg(&rule_path)
				.args(["--argjson", "shared", &shared, "--arg", "root", root])
				.output()?;
			if !o.stderr.is_empty() {
				bail!(
					"schema documentation generation failed: {}",
					String::from_utf8_lossy(&o.stderr)
				);
			}
			Ok(dedupe_lines(&String::from_utf8_lossy(&o.stdout)))
		};

		let mut readme = format!("# {} Schema\n\n", schema.name);
		readme.push_str(&table("")?);
		if !schema.shared.is_empty() {
			readme.push_str(
				"\n## Shared types\n\nThese types are used by many fields. Fields of these types link here \
				instead of listing every nested field, and their fields are prefixed with the type, such as \
				`<BackendPolicies>.backendTLS`.\n",
			);
			for (def, name) in schema.shared {
				readme.push_str(&format!("\n### `<{name}>`\n\n"));
				readme.push_str(&table(def)?);
			}
		}
		readme.push_str(&schema.appendix);

		let mut file = fs_err::File::create(format!("{xtask_path}/../../schema/{mdfile}"))?;
		file.write_all(readme.as_bytes())?;
	}
	Ok(())
}

/// Adds a definition for each config resource kind, named after the kind, describing the values
/// accepted by the `/api/config/resources/{kind}` API. Also returns the documentation for them.
fn add_resources(config_json: &str) -> Result<(String, String)> {
	let mut schema: Value = serde_json::from_str(config_json)?;
	let defs = schema["$defs"]
		.as_object_mut()
		.context("config schema has no $defs")?;
	let r = |def: &str| json!({ "$ref": format!("#/$defs/{def}") });
	let mut md = String::from(
		"\n## Config resources\n\nValues accepted by the `/api/config/resources/{kind}` API. The JSON schema \
		for each kind is the definition of the same name, such as `config.json#/$defs/llm.provider`. \
		For policies, select the schema option whose title matches the item ID.\n\n\
		|Kind|Value|ID|Collection `PUT`|\n|-|-|-|-|\n",
	);
	for kind in ConfigResourceKind::ALL {
		let (def, value, id) = match kind {
			ConfigResourceKind::ModelCatalog => (
				json!({
					"type": "object",
					"properties": {
						"base": { "description": "Base model catalog.", "$ref": "#/$defs/Catalog" },
						"custom": { "description": "Custom model catalog, taking precedence over the base catalog.", "$ref": "#/$defs/Catalog" },
					},
					"additionalProperties": false,
				}),
				"`base` and `custom` catalogs, merged ahead of `config.modelCatalog[]`".to_string(),
				"`default`",
			),
			ConfigResourceKind::LlmProvider => (
				r("LocalLLMProvider"),
				"An item of `llm.providers[]`".to_string(),
				"`name`",
			),
			ConfigResourceKind::LlmModel => (
				r("LocalLLMModels"),
				"An item of `llm.models[]`".to_string(),
				"`id`, or `name` if unset",
			),
			ConfigResourceKind::LlmVirtualModel => (
				r("LocalLLMVirtualModel"),
				"An item of `llm.virtualModels[]`".to_string(),
				"`name`",
			),
			ConfigResourceKind::LlmApiKey => (
				r("LocalAPIKey"),
				"An item of `llm.policies.apiKey.keys[]`. Metadata prefixed with `agentgateway.dev/` is managed by agentgateway".to_string(),
				"Generated, and stored in `metadata` under `agentgateway.dev/id`",
			),
			ConfigResourceKind::LlmPolicy => (
				policy_values(defs, "LocalLLMPolicy")?,
				"`llm.policies.<id>`. Manage `apiKey.keys` through `llm.apiKey`".to_string(),
				"Field name, such as `apiKey`",
			),
			ConfigResourceKind::McpTarget => (
				r("LocalMcpTarget"),
				"An item of `mcp.targets[]`".to_string(),
				"`name`",
			),
			ConfigResourceKind::McpPolicy => (
				policy_values(defs, "FilterOrPolicy")?,
				"`mcp.policies.<id>`".to_string(),
				"Field name",
			),
			ConfigResourceKind::LlmSettings | ConfigResourceKind::McpSettings => {
				let (section, fields) = kind
					.settings_fields()
					.context("settings kind has no fields")?;
				let def = match section {
					"llm" => "LocalLLMConfig",
					"mcp" => "LocalSimpleMcpConfig",
					_ => bail!("unknown settings section {section}"),
				};
				let properties = fields
					.iter()
					.map(|field| {
						let property = defs[def]["properties"]
							.get(*field)
							.with_context(|| format!("{def} has no field {field}"))?;
						Ok((field.to_string(), property.clone()))
					})
					.collect::<Result<serde_json::Map<_, _>>>()?;
				let value = fields
					.iter()
					.map(|field| format!("`{section}.{field}`"))
					.collect::<Vec<_>>()
					.join(", ");
				(
					json!({ "type": "object", "properties": properties, "additionalProperties": false }),
					value,
					"`default`",
				)
			},
			ConfigResourceKind::TrafficGateway => (
				named_resource(defs, "LocalGateway")?,
				"`gateways.<id>` with a required `name` field".to_string(),
				"`name`",
			),
			ConfigResourceKind::TrafficRoute => (
				named_resource(defs, "LocalAttachedRoute")?,
				"An item of `routes[]` with a required `name` field".to_string(),
				"`name`",
			),
			ConfigResourceKind::TrafficTcpRoute => (
				named_resource(defs, "LocalAttachedTCPRoute")?,
				"An item of `tcpRoutes[]` with a required `name` field".to_string(),
				"`name`",
			),
			ConfigResourceKind::UiPolicy => (
				policy_values(defs, "LocalUIPolicy")?,
				"`ui.policies.<id>`".to_string(),
				"Field name",
			),
			ConfigResourceKind::FrontendPolicy => (
				policy_values(defs, "LocalFrontendPolicies")?,
				"`frontendPolicies.<id>`".to_string(),
				"Field name",
			),
			ConfigResourceKind::SessionKey => continue,
		};
		let mut def = def;
		def["description"] = Value::String(format!("{value}. ID: {id}."));
		defs.insert(kind.as_str().to_string(), def);
		let collection_put = match kind {
			ConfigResourceKind::LlmPolicy
			| ConfigResourceKind::McpPolicy
			| ConfigResourceKind::UiPolicy
			| ConfigResourceKind::FrontendPolicy => "Use item `PUT`",
			ConfigResourceKind::LlmApiKey => "Yes. Each request creates a new key.",
			_ => "Yes",
		};
		md.push_str(&format!("|`{kind}`|{value}|{id}|{collection_put}|\n"));
	}
	Ok((serde_json::to_string_pretty(&schema)?, md))
}

fn policy_values(defs: &serde_json::Map<String, Value>, container: &str) -> Result<Value> {
	let properties = defs[container]["properties"]
		.as_object()
		.with_context(|| format!("{container} has no properties"))?;
	let mut values = Vec::with_capacity(properties.len());
	for id in properties.keys() {
		let mut value = if container == "LocalLLMPolicy" && id == "apiKey" {
			let mut value = defs["LocalAPIKeys"].clone();
			// The API supplies keys separately before deserializing the config.
			value["properties"]
				.as_object_mut()
				.context("LocalAPIKeys has no properties")?
				.remove("keys");
			value["required"]
				.as_array_mut()
				.context("LocalAPIKeys has no required fields")?
				.retain(|field| field != "keys");
			value["description"] = json!("Manage keys through llm.apiKey resources.");
			value
		} else {
			let field = id.replace('~', "~0").replace('/', "~1");
			json!({ "$ref": format!("#/$defs/{container}/properties/{field}") })
		};
		value["title"] = json!(id);
		values.push(value);
	}
	Ok(json!({ "anyOf": values }))
}

fn named_resource(defs: &serde_json::Map<String, Value>, source: &str) -> Result<Value> {
	let mut value = defs[source].clone();
	let properties = value["properties"]
		.as_object_mut()
		.with_context(|| format!("{source} has no properties"))?;
	let name = properties
		.entry("name")
		.or_insert_with(|| json!({ "description": "Name identifying this gateway." }));
	name["type"] = json!("string");
	name["minLength"] = json!(1);
	name
		.as_object_mut()
		.context("invalid name schema")?
		.remove("default");
	let required = value
		.as_object_mut()
		.with_context(|| format!("{source} is not an object schema"))?
		.entry("required")
		.or_insert_with(|| json!([]))
		.as_array_mut()
		.with_context(|| format!("{source} has invalid required fields"))?;
	if !required.iter().any(|field| field == "name") {
		required.push(json!("name"));
	}
	Ok(value)
}

fn dedupe_lines(input: &str) -> String {
	let mut seen = HashSet::new();
	let mut output = input
		.lines()
		.filter(|line| seen.insert(*line))
		.collect::<Vec<_>>()
		.join("\n");
	if !output.is_empty() {
		output.push('\n');
	}
	output
}

pub fn make<T: JsonSchema>(inline_subschemas: bool) -> anyhow::Result<String> {
	make_with_contract::<T>(inline_subschemas, schemars::generate::Contract::Deserialize)
}

pub fn make_with_contract<T: JsonSchema>(
	inline_subschemas: bool,
	contract: schemars::generate::Contract,
) -> anyhow::Result<String> {
	let settings = schemars::generate::SchemaSettings::default().with(|s| {
		s.inline_subschemas = inline_subschemas;
		s.contract = contract;
	});
	let gens = schemars::SchemaGenerator::new(settings);
	let schema = gens.into_root_schema_for::<T>();
	Ok(serde_json::to_string_pretty(&schema)?)
}
