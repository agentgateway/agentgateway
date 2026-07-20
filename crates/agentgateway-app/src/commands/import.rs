use std::io::Read;
use std::path::{Path, PathBuf};

use crate::ImportArgs;

pub(crate) fn execute(args: ImportArgs) -> anyhow::Result<()> {
	let ImportArgs { from, file, output } = args;
	import_file(file, output, &from)
}

fn import_file(path: PathBuf, output: Option<PathBuf>, source: &str) -> anyhow::Result<()> {
	let contents = read_input(&path)?;
	let result = agentgateway::import::import_config(source, &contents)?;
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

fn read_input(path: &Path) -> anyhow::Result<String> {
	if path != Path::new("-") {
		return Ok(fs_err::read_to_string(path)?);
	}
	let mut contents = String::new();
	std::io::stdin().read_to_string(&mut contents)?;
	Ok(contents)
}
