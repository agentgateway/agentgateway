use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::Context as _;
use arc_swap::ArcSwap;

use crate::ModelCatalogSource;

static DEFAULT_RUNTIME_MODELS_JSON: &str = include_str!("bedrock_runtime_models.json");

#[derive(serde::Deserialize)]
struct WrappedRuntimeTable {
	models: Vec<String>,
}

/// Accepts both the wrapped `{"source":"...","models":[...]}` format produced by
/// `agctl bedrock import` and a plain JSON array `["model-a","model-b"]`.
fn parse_runtime_list(json: &str) -> anyhow::Result<HashSet<String>> {
	let ids: Vec<String> = if let Ok(wrapped) = serde_json::from_str::<WrappedRuntimeTable>(json) {
		wrapped.models
	} else {
		serde_json::from_str(json).context(
			"bedrock runtime catalog must be a JSON array or \
                 wrapped {\"source\":\"...\",\"models\":[...]}",
		)?
	};
	Ok(ids.into_iter().collect())
}

fn load_embedded_default() -> HashSet<String> {
	parse_runtime_list(DEFAULT_RUNTIME_MODELS_JSON)
		.expect("embedded runtime_models.json must be valid JSON")
}

static RUNTIME_MODELS: OnceLock<ArcSwap<HashSet<String>>> = OnceLock::new();

fn runtime_models() -> &'static ArcSwap<HashSet<String>> {
	RUNTIME_MODELS.get_or_init(|| ArcSwap::from_pointee(load_embedded_default()))
}

fn set_runtime_models(ids: HashSet<String>) {
	runtime_models().store(Arc::new(ids));
}

pub fn initialize(sources: Vec<ModelCatalogSource>) -> anyhow::Result<()> {
	// Ensure the embedded default is loaded before any async work touches it.
	let _ = runtime_models();
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

	let sources_clone = sources.clone();
	tokio::spawn(async move {
		match load_sources(&sources_clone).await {
			Ok(ids) => {
				tracing::info!(
					models = ids.len(),
					"loaded Bedrock runtime model catalog (embedded default + user sources)"
				);
				set_runtime_models(ids);
			},
			Err(e) => {
				tracing::warn!("Bedrock runtime model catalog load failed; embedded defaults remain: {e:#}")
			},
		}
	});

	if !file_paths.is_empty() {
		watch_files(file_paths, sources)?;
	}
	Ok(())
}

/// Loads all sources and merges them with the embedded baseline so user files
/// only need to list additions, not the full set.
async fn load_sources(sources: &[ModelCatalogSource]) -> anyhow::Result<HashSet<String>> {
	let mut merged = load_embedded_default();
	let mut any_loaded = false;
	for source in sources {
		let json = match source {
			ModelCatalogSource::File { file } => match fs_err::tokio::read_to_string(file).await {
				Ok(s) => s,
				Err(e) if e.kind() == ErrorKind::NotFound => {
					tracing::debug!(
							path = %file.display(),
							"Bedrock runtime catalog file not found, skipping"
					);
					continue;
				},
				Err(e) => return Err(anyhow::Error::from(e)).context("reading Bedrock runtime catalog"),
			},
			ModelCatalogSource::Inline { inline } => inline.clone(),
			ModelCatalogSource::InlineCatalog { .. } => continue,
		};
		merged.extend(parse_runtime_list(&json)?);
		any_loaded = true;
	}
	if !any_loaded {
		anyhow::bail!("no Bedrock runtime catalog sources were readable");
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
					tracing::info!(models = ids.len(), "reloaded Bedrock runtime model catalog");
					set_runtime_models(ids);
				},
				Err(e) => {
					tracing::error!("failed to reload Bedrock runtime catalog; keeping previous list: {e:#}")
				},
			}
		}
	});
	Ok(())
}

/// Returns true when `model_id` is NOT in the runtime model catalog, meaning
/// it can only be reached via the Mantle endpoint.
/// The caller is responsible for stripping any CRIS inference-profile prefix
/// before calling this function.
pub fn is_mantle_only(model_id: &str) -> bool {
	!runtime_models().load().contains(model_id)
}

#[cfg(test)]
mod tests {
	use super::*;

	// Tests that mutate the global RUNTIME_MODELS must hold this lock to prevent data races.
	static MODELS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

	fn restore_default() {
		set_runtime_models(load_embedded_default());
	}

	#[test]
	fn models_in_runtime_catalog_are_not_mantle_only() {
		let _lock = MODELS_LOCK.lock().unwrap();
		restore_default();
		// These models are present in the embedded runtime_models.json.
		assert!(!is_mantle_only("amazon.nova-pro-v1:0"));
		assert!(!is_mantle_only("anthropic.claude-3-5-haiku-20241022-v1:0"));
		assert!(!is_mantle_only("anthropic.claude-fable-5"));
		assert!(!is_mantle_only("deepseek.v3.2"));
	}

	#[test]
	fn models_absent_from_catalog_are_mantle_only() {
		let _lock = MODELS_LOCK.lock().unwrap();
		restore_default();
		// Models not in the embedded runtime catalog route to Mantle.
		// These are intentionally obscure IDs unlikely to appear in the list.
		assert!(is_mantle_only("vendor.totally-fake-model-xyz"));
		assert!(is_mantle_only("anthropic.claude-mythos-preview"));
	}

	#[test]
	fn set_runtime_models_marks_model_as_runtime_capable() {
		let _lock = MODELS_LOCK.lock().unwrap();
		set_runtime_models(["vendor.new-model".to_string()].into());
		assert!(!is_mantle_only("vendor.new-model"));
		assert!(is_mantle_only("vendor.something-else"));
		restore_default();
	}

	#[test]
	fn parse_runtime_list_bare_array() {
		let json = r#"["vendor.model-a", "vendor.model-b"]"#;
		let ids = parse_runtime_list(json).unwrap();
		assert!(ids.contains("vendor.model-a"));
		assert!(ids.contains("vendor.model-b"));
		assert_eq!(ids.len(), 2);
	}

	#[test]
	fn parse_runtime_list_wrapped_format() {
		let json = r#"{"source":"https://example.com","models":["vendor.model-a","vendor.model-b"]}"#;
		let ids = parse_runtime_list(json).unwrap();
		assert!(ids.contains("vendor.model-a"));
		assert!(ids.contains("vendor.model-b"));
		assert_eq!(ids.len(), 2);
	}

	#[test]
	fn parse_runtime_list_rejects_invalid_json() {
		assert!(parse_runtime_list(r#"not json"#).is_err());
		assert!(parse_runtime_list(r#"{"models": 42}"#).is_err());
	}

	#[test]
	fn embedded_default_is_valid_and_nonempty() {
		let ids = load_embedded_default();
		assert!(
			!ids.is_empty(),
			"embedded runtime_models.json should have entries"
		);
		// Spot-check a stable model ID from the embedded list.
		assert!(ids.contains("amazon.nova-pro-v1:0"));
	}

	#[test]
	fn no_scraping_artifacts_in_embedded_default() {
		let ids = load_embedded_default();
		// Ensure the scraping artifact removed from runtime_models.json stays gone.
		assert!(!ids.contains("bedrock-first-request.py"));
	}

	#[tokio::test]
	async fn load_sources_merges_with_embedded_default() {
		let json = r#"["vendor.extra-model"]"#;
		let sources = vec![ModelCatalogSource::Inline {
			inline: json.to_string(),
		}];
		let ids = load_sources(&sources).await.unwrap();
		// User-provided model is present.
		assert!(ids.contains("vendor.extra-model"));
		// Embedded defaults are also present.
		assert!(ids.contains("amazon.nova-pro-v1:0"));
	}

	#[tokio::test]
	async fn load_sources_skips_missing_file() {
		let dir = tempfile::tempdir().unwrap();
		let missing = dir.path().join("missing.json");
		let inline = r#"["my.model"]"#;
		let sources = vec![
			ModelCatalogSource::File { file: missing },
			ModelCatalogSource::Inline {
				inline: inline.to_string(),
			},
		];
		let ids = load_sources(&sources).await.unwrap();
		assert!(ids.contains("my.model"));
	}

	#[tokio::test]
	async fn load_sources_supports_wrapped_format() {
		let json = r#"{"source":"https://example.com","models":["a.model","b.model"]}"#;
		let sources = vec![ModelCatalogSource::Inline {
			inline: json.to_string(),
		}];
		let ids = load_sources(&sources).await.unwrap();
		assert!(ids.contains("a.model"));
		assert!(ids.contains("b.model"));
	}
}
