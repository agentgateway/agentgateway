use std::io::Write;

use agentgateway::cel;
use anyhow::{Result, bail};
use schemars::JsonSchema;

pub fn generate_schema() -> Result<()> {
	let xtask_path = std::env::var("CARGO_MANIFEST_DIR")?;
	let schemas = vec![
		(
			"Configuration File",
			"config.md",
			make::<agentgateway::types::local::LocalConfig>()?,
			"config.json",
		),
		(
			"CEL context",
			"cel.md",
			make::<cel::ExecutorSerde>()?,
			"cel.json",
		),
	];
	for (_, _, schema, file) in &schemas {
		let rule_path = format!("{xtask_path}/../../schema/{file}");
		let mut file = fs_err::File::create(rule_path)?;
		file.write_all(schema.as_bytes())?;
	}

	for (name, mdfile, _, file) in schemas {
		let mut readme = format!("# {name} Schema\n\n");
		let rule_path = format!("{xtask_path}/../../schema/{file}");
		let o = if cfg!(target_os = "windows") {
			let cmd_path: String = format!("{xtask_path}/../../tools/schema-to-md.ps1");
			std::process::Command::new("powershell")
				.arg("-Command")
				.arg(cmd_path)
				.arg(&rule_path)
				.output()?
		} else {
			let cmd_path: String = format!("{xtask_path}/../../tools/schema-to-md.sh");
			std::process::Command::new(cmd_path)
				.arg(&rule_path)
				.output()?
		};
		if !o.stderr.is_empty() {
			bail!(
				"schema documentation generation failed: {}",
				String::from_utf8_lossy(&o.stderr)
			);
		}
		readme.push_str(&String::from_utf8_lossy(&o.stdout));

		let mut file = fs_err::File::create(format!("{xtask_path}/../../schema/{mdfile}"))?;
		file.write_all(readme.as_bytes())?;
	}
	Ok(())
}

pub fn make<T: JsonSchema>() -> anyhow::Result<String> {
	let settings = schemars::generate::SchemaSettings::default().with(|s| s.inline_subschemas = true);
	let gens = schemars::SchemaGenerator::new(settings);
	let schema = gens.into_root_schema_for::<T>();
	Ok(serde_json::to_string_pretty(&schema)?)
}
