//! Loads the Bedrock Mantle allow-list (embedded default + configured sources) and hot-reloads file sources.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context as _;

use crate::MantleCatalogSource;

// Load synchronously so early requests don't race an empty list, then watch files for changes.
pub async fn initialize(sources: Vec<MantleCatalogSource>) -> anyhow::Result<()> {
	if sources.is_empty() {
		return Ok(());
	}
	match load_sources(&sources).await {
		Ok(ids) => {
			tracing::info!(models = ids.len(), "loaded Bedrock Mantle allow-list");
			agent_llm::bedrock_model_table::set_mantle_models(ids);
		},
		Err(e) => {
			tracing::warn!("Bedrock Mantle allow-list load failed; embedded defaults remain: {e:#}")
		},
	}

	let file_paths: Vec<PathBuf> = sources
		.iter()
		.filter_map(|s| match s {
			MantleCatalogSource::File { file } => Some(file.clone()),
			MantleCatalogSource::Inline { .. } => None,
		})
		.collect();
	if !file_paths.is_empty() {
		watch_files(file_paths, sources)?;
	}
	Ok(())
}

async fn load_sources(sources: &[MantleCatalogSource]) -> anyhow::Result<HashSet<String>> {
	let mut merged = agent_llm::bedrock_model_table::embedded_default();
	let mut any_loaded = false;
	for source in sources {
		let json = match source {
			MantleCatalogSource::File { file } => match fs_err::tokio::read_to_string(file).await {
				Ok(s) => s,
				Err(e) if e.kind() == ErrorKind::NotFound => {
					tracing::debug!(path = %file.display(), "Bedrock Mantle allow-list file not found, skipping");
					continue;
				},
				Err(e) => return Err(anyhow::Error::from(e)).context("reading Bedrock Mantle allow-list"),
			},
			MantleCatalogSource::Inline { inline } => inline.clone(),
		};
		merged.extend(agent_llm::bedrock_model_table::parse_model_list(&json)?);
		any_loaded = true;
	}
	if !any_loaded {
		anyhow::bail!("no Bedrock Mantle allow-list sources were readable");
	}
	Ok(merged)
}

fn watch_files(file_paths: Vec<PathBuf>, sources: Vec<MantleCatalogSource>) -> anyhow::Result<()> {
	let mut watched = crate::util::watch_files_with_options(
		file_paths,
		crate::util::WatchFilesOptions::default().reload_on_disappearance(true),
	)?;
	tokio::spawn(async move {
		while watched.changed().await {
			match load_sources(&sources).await {
				Ok(ids) => {
					tracing::info!(models = ids.len(), "reloaded Bedrock Mantle allow-list");
					agent_llm::bedrock_model_table::set_mantle_models(ids);
				},
				Err(e) => {
					tracing::error!(
						"failed to reload Bedrock Mantle allow-list; keeping previous list: {e:#}"
					)
				},
			}
		}
	});
	Ok(())
}
