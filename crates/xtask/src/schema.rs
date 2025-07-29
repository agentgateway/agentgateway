use std::io::Write;

use agentgateway::cel;
use anyhow::Result;
use schemars::JsonSchema;

pub fn generate_schema() -> Result<()> {
	let xtask_path = std::env::var("CARGO_MANIFEST_DIR")?;
	let schemas = vec![
		(
			"Configuration File",
			make::<agentgateway::types::local::LocalConfig>()?,
			"local.json",
		),
		("CEL context", make::<cel::ExpressionContext>()?, "cel.json"),
	];
	for (_, schema, file) in &schemas {
		let rule_path = format!("{xtask_path}/../../schema/{file}");
		let mut file = fs_err::File::create(rule_path)?;
		file.write_all(schema.as_bytes())?;
	}
	let mut readme = r#"# Schemas
This folder contains JSON schemas for various parts of the project

"#
	.to_owned();
	for (name, _, file) in schemas {
		let rule_path = format!("{xtask_path}/../../schema/{file}");
		let cmd_path = format!("{xtask_path}/../../common/scripts/schema-to-md.sh");
		let o = std::process::Command::new(cmd_path)
			.arg(&rule_path)
			.output()?;
		readme.push_str(&format!("## {name}\n\n"));
		readme.push_str(&String::from_utf8_lossy(&o.stdout));
	}
	let mut file = fs_err::File::create(format!("{xtask_path}/../../schema/README.md"))?;
	file.write_all(readme.as_bytes())?;
	Ok(())
}

pub fn make<T: JsonSchema>() -> anyhow::Result<String> {
	let settings = schemars::generate::SchemaSettings::default().with(|s| s.inline_subschemas = true);
	let gens = schemars::SchemaGenerator::new(settings);
	let schema = gens.into_root_schema_for::<T>();
	Ok(serde_json::to_string_pretty(&schema)?)
}
