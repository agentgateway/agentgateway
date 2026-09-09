use std::path::Path;

use anyhow::Context;
use serde::Serialize;

use crate::llm::catalog::{ModelCatalog, model};

const CATALOG_URL: &str = "https://agentgateway.dev/model-catalog";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshBaseCatalogResponse {
	pub providers: usize,
	pub models: usize,
	#[serde(skip_serializing)]
	pub catalog: model::Catalog,
}

pub async fn refresh_base_catalog(
	file: &Path,
	model_catalog: Option<&ModelCatalog>,
) -> anyhow::Result<RefreshBaseCatalogResponse> {
	let refreshed = fetch_base_catalog().await?;
	let json = serde_json::to_vec_pretty(&refreshed.catalog).context("marshal model catalog")?;
	if let Some(parent) = file.parent() {
		fs_err::tokio::create_dir_all(parent).await?;
	}
	fs_err::tokio::write(file, &json).await?;
	if let Some(model_catalog) = model_catalog {
		model_catalog.reload().await?;
	}
	Ok(refreshed)
}

pub async fn fetch_base_catalog() -> anyhow::Result<RefreshBaseCatalogResponse> {
	let client = reqwest::Client::builder()
		.redirect(reqwest::redirect::Policy::limited(10))
		.build()?;
	let catalog: model::Catalog = client
		.get(CATALOG_URL)
		.send()
		.await
		.context("fetch model catalog from GitHub")?
		.error_for_status()
		.context("fetch model catalog from GitHub")?
		.json()
		.await
		.context("decode model catalog from GitHub")?;
	catalog.validate()?;
	let providers = catalog.providers.len();
	let models = catalog
		.providers
		.values()
		.map(|provider| provider.models.len())
		.sum();
	Ok(RefreshBaseCatalogResponse {
		providers,
		models,
		catalog,
	})
}
