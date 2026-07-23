//! Startup loader for the Bedrock Mantle allow-list: reads `ModelCatalogSource`s,
//! merges over the embedded default, installs them, and hot-reloads on change.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context as _;

use crate::ModelCatalogSource;

pub fn initialize(sources: Vec<ModelCatalogSource>) -> anyhow::Result<()> {
	if sources.is_empty() {
		return Ok(());
	}
	let file_paths: Vec<PathBuf> = sources
		.iter()
		.filter_map(|s| match s {
			ModelCatalogSource::File { file } => Some(file.clone()),
			ModelCatalogSource::Inline { .. } | ModelCatalogSource::InlineCatalog { .. } => None,
		})
		.collect();

	let sources_for_task = sources.clone();
	tokio::spawn(async move {
		match load_sources(&sources_for_task).await {
			Ok(ids) => {
				tracing::info!(
					models = ids.len(),
					"loaded Bedrock Mantle allow-list (embedded default + user sources)"
				);
				agent_llm::bedrock_model_table::set_mantle_models(ids);
			},
			Err(e) => {
				tracing::warn!("Bedrock Mantle allow-list load failed; embedded defaults remain: {e:#}")
			},
		}
	});

	if !file_paths.is_empty() {
		watch_files(file_paths, sources)?;
	}
	Ok(())
}

async fn load_sources(sources: &[ModelCatalogSource]) -> anyhow::Result<HashSet<String>> {
	let mut merged = agent_llm::bedrock_model_table::embedded_default();
	let mut any_loaded = false;
	for source in sources {
		let json = match source {
			ModelCatalogSource::File { file } => match fs_err::tokio::read_to_string(file).await {
				Ok(s) => s,
				Err(e) if e.kind() == ErrorKind::NotFound => {
					tracing::debug!(
						path = %file.display(),
						"Bedrock Mantle allow-list file not found, skipping"
					);
					continue;
				},
				Err(e) => return Err(anyhow::Error::from(e)).context("reading Bedrock Mantle allow-list"),
			},
			ModelCatalogSource::Inline { inline } => inline.clone(),
			// The priced cost catalog is not a model list; ignore it here.
			ModelCatalogSource::InlineCatalog { .. } => continue,
		};
		merged.extend(agent_llm::bedrock_model_table::parse_model_list(&json)?);
		any_loaded = true;
	}
	if !any_loaded {
		anyhow::bail!("no Bedrock Mantle allow-list sources were readable");
	}
	Ok(merged)
}

fn watch_files(file_paths: Vec<PathBuf>, sources: Vec<ModelCatalogSource>) -> anyhow::Result<()> {
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
