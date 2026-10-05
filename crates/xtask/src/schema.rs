use std::collections::HashSet;
use std::io::Write;

use agentgateway::cel;
use anyhow::{Result, bail};
use schemars::JsonSchema;

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
	}

	let xtask_path = std::env::var("CARGO_MANIFEST_DIR")?;
	let schemas = vec![
		SchemaDoc {
			name: "Configuration File",
			mdfile: Some("config.md"),
			file: "config.json",
			schema_json: make::<agentgateway::types::local::LocalConfig>(false)?,
			shared: SHARED_CONFIG_TYPES,
		},
		SchemaDoc {
			name: "CEL context",
			mdfile: Some("cel.md"),
			file: "cel.json",
			// CEL is simpler so we just always inline
			schema_json: make::<cel::ExecutorSerde>(true)?,
			shared: &[],
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

		let mut file = fs_err::File::create(format!("{xtask_path}/../../schema/{mdfile}"))?;
		file.write_all(readme.as_bytes())?;
	}
	Ok(())
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
