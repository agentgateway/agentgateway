use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::ImportArgs;

pub(crate) fn execute(args: ImportArgs) -> anyhow::Result<()> {
	let ImportArgs {
		from,
		file,
		output,
		db_state,
	} = args;
	import_file(file, output, db_state, &from)
}

fn import_file(
	path: PathBuf,
	output: Option<PathBuf>,
	db_state: Option<PathBuf>,
	source: &str,
) -> anyhow::Result<()> {
	let contents = read_input(&path)?;
	let db_state = db_state
		.map(|path| {
			let raw = read_input(&path)?;
			agentgateway::import::parse_db_state(&raw)
				.with_context(|| format!("invalid DB-state JSON in {}", path.display()))
		})
		.transpose()?;
	let options = agentgateway::import::ImportOptions {
		database_url: import_database_url(output.as_deref()),
		db_state,
		..Default::default()
	};
	let result = agentgateway::import::import_config_with_options(source, &contents, &options)?;
	let imported = agentgateway::yaml::to_string(&result.config)?;
	for finding in &result.findings {
		eprintln!(
			"{}: {}: {}",
			finding.status.as_str(),
			finding.source_path,
			finding.message
		);
	}
	print_summary(&result.findings);
	match output {
		Some(path) if path != std::path::Path::new("-") => {
			fs_err::write(&path, imported)?;
			println!("Imported {source} config: {}", path.display());
		},
		_ => print!("{imported}"),
	}
	Ok(())
}

/// Prints a closing summary so a reader doesn't have to scroll back through every
/// interleaved per-field line to see what still needs their attention: how many findings
/// landed in each status, then every `manual`/`unsupported` one listed together.
fn print_summary(findings: &[agentgateway::import::ImportFinding]) {
	use agentgateway::import::ImportStatus;

	let count = |status: ImportStatus| findings.iter().filter(|f| f.status == status).count();
	let exact = count(ImportStatus::Exact);
	let approximate = count(ImportStatus::Approximate);
	let manual = count(ImportStatus::Manual);
	let unsupported = count(ImportStatus::Unsupported);

	eprintln!(
		"\nImport summary: {exact} exact, {approximate} approximate, {manual} manual, {unsupported} unsupported"
	);
	let needs_review: Vec<_> = findings
		.iter()
		.filter(|f| matches!(f.status, ImportStatus::Manual | ImportStatus::Unsupported))
		.collect();
	if !needs_review.is_empty() {
		eprintln!("Needs manual review:");
		for finding in needs_review {
			eprintln!("  - {}: {}", finding.source_path, finding.message);
		}
	}
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
