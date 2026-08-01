use std::io::Read;
use std::path::{Path, PathBuf};

use crate::ImportArgs;

pub(crate) fn execute(args: ImportArgs) -> anyhow::Result<()> {
	let ImportArgs { from, file, output } = args;
	import_file(file, output, &from)
}

fn import_file(path: PathBuf, output: Option<PathBuf>, source: &str) -> anyhow::Result<()> {
	let contents = read_input(&path)?;
	let options = agentgateway::import::ImportOptions {
		database_url: import_database_url(output.as_deref()),
	};
	let result = agentgateway::import::import_config_with_options(source, &contents, &options)?;
	let imported = agentgateway::yamlviajson::to_string(&result.config)?;
	for finding in result.findings {
		eprintln!(
			"{}: {}: {}",
			finding.status.as_str(),
			finding.source_path,
			finding.message
		);
	}
	match output {
		Some(path) if path != std::path::Path::new("-") => {
			fs_err::write(&path, imported)?;
			println!("Imported {source} config: {}", path.display());
		},
		_ => print!("{imported}"),
	}
	Ok(())
}

fn import_database_url(output: Option<&Path>) -> String {
	let database_path = output
		.filter(|path| *path != Path::new("-"))
		.and_then(Path::parent)
		.filter(|parent| !parent.as_os_str().is_empty())
		.map(|parent| parent.join("data.db"))
		.unwrap_or_else(|| PathBuf::from("data.db"));
	let database_path = database_path
		.to_string_lossy()
		.replace(std::path::MAIN_SEPARATOR, "/");
	format!("sqlite://{database_path}")
}

fn read_input(path: &Path) -> anyhow::Result<String> {
	if path != Path::new("-") {
		return Ok(fs_err::read_to_string(path)?);
	}
	let mut contents = String::new();
	std::io::stdin().read_to_string(&mut contents)?;
	Ok(contents)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn writes_config_with_reusable_litellm_provider() {
		let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../agentgateway/src/tests/import/litellm/centralized-credentials.yaml");
		let output_dir = tempfile::tempdir().expect("create output directory");
		let output = output_dir.path().join("config.yaml");
		import_file(fixture, Some(output.clone()), "litellm").expect("import LiteLLM config");

		let generated = fs_err::read_to_string(output).expect("read generated config");
		assert!(
			generated.contains("reference: imported/litellm/shared-openai/openAI"),
			"generated model should reference the reusable provider"
		);
		let database_path = output_dir
			.path()
			.join("data.db")
			.to_string_lossy()
			.replace(std::path::MAIN_SEPARATOR, "/");
		assert!(
			generated.contains(&format!("url: sqlite://{database_path}")),
			"generated database should be beside the config"
		);
	}

	#[test]
	fn places_database_beside_generated_config() {
		assert_eq!(
			import_database_url(Some(Path::new("/tmp/imported/config.yaml"))),
			"sqlite:///tmp/imported/data.db"
		);
		assert_eq!(
			import_database_url(Some(Path::new("config.yaml"))),
			"sqlite://data.db"
		);
		assert_eq!(import_database_url(None), "sqlite://data.db");
	}
}
