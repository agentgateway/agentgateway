use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::llm::cost::{CatalogSnapshot, ModelCatalog, catalog};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshBaseCatalogResponse {
	pub providers: usize,
	pub models: usize,
}

pub async fn refresh_models_dev_base_catalog(
	file: &Path,
	model_catalog: &ModelCatalog,
) -> anyhow::Result<RefreshBaseCatalogResponse> {
	let catalog = fetch_models_dev_catalog().await?;
	let providers = catalog.providers.len();
	let models = catalog
		.providers
		.values()
		.map(|provider| provider.models.len())
		.sum();
	let json = serde_json::to_vec_pretty(&catalog).context("marshal models.dev catalog")?;
	if let Some(parent) = file.parent() {
		fs_err::tokio::create_dir_all(parent).await?;
	}
	fs_err::tokio::write(file, &json).await?;
	model_catalog.replace(CatalogSnapshot {
		catalog: Some(catalog),
	});
	Ok(RefreshBaseCatalogResponse { providers, models })
}

async fn fetch_models_dev_catalog() -> anyhow::Result<crate::llm::cost::catalog::Catalog> {
	let response = reqwest::get("https://models.dev/api.json")
		.await
		.context("fetch models.dev api.json")?
		.error_for_status()
		.context("fetch models.dev api.json")?;
	let api: BTreeMap<String, ModelsDevProvider> = response
		.json()
		.await
		.context("decode models.dev api.json")?;
	models_dev_transform(api)
}

fn models_dev_transform(
	api: BTreeMap<String, ModelsDevProvider>,
) -> anyhow::Result<crate::llm::cost::catalog::Catalog> {
	let mut catalog = crate::llm::cost::catalog::Catalog::default();
	for (source_id, gateway_id) in MODELS_DEV_PROVIDER_IDS {
		let Some(source) = api.get(*source_id) else {
			continue;
		};
		for (model_id, model) in &source.models {
			if model.status == "deprecated" {
				continue;
			}
			let Some(cost) = &model.cost else {
				continue;
			};
			let entry = catalog::Model {
				rates: models_dev_rates(&cost.rates).with_context(|| format!("{gateway_id}/{model_id}"))?,
				tiers: models_dev_tiers(&cost.tiers).with_context(|| format!("{gateway_id}/{model_id}"))?,
			};
			if entry.rates.is_empty() && entry.tiers.is_empty() {
				continue;
			}
			catalog
				.providers
				.entry((*gateway_id).to_string())
				.or_default()
				.models
				.insert(model_id.clone(), entry);
		}
	}
	if catalog.providers.is_empty() {
		bail!("models.dev did not contain any supported priced models");
	}
	catalog.validate()?;
	Ok(catalog)
}

#[derive(Debug, Deserialize)]
struct ModelsDevProvider {
	#[serde(default)]
	models: BTreeMap<String, ModelsDevModel>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModel {
	#[serde(default)]
	status: String,
	cost: Option<ModelsDevCost>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevCost {
	#[serde(flatten)]
	rates: ModelsDevRates,
	#[serde(default)]
	tiers: Vec<ModelsDevTier>,
}

#[derive(Debug, Deserialize, Default)]
struct ModelsDevRates {
	input: Option<serde_json::Number>,
	output: Option<serde_json::Number>,
	cache_read: Option<serde_json::Number>,
	cache_write: Option<serde_json::Number>,
	reasoning: Option<serde_json::Number>,
	input_audio: Option<serde_json::Number>,
	output_audio: Option<serde_json::Number>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevTier {
	#[serde(flatten)]
	rates: ModelsDevRates,
	tier: ModelsDevTierKind,
}

#[derive(Debug, Deserialize)]
struct ModelsDevTierKind {
	#[serde(rename = "type")]
	kind: String,
	size: u64,
}

fn models_dev_tiers(tiers: &[ModelsDevTier]) -> anyhow::Result<Vec<catalog::Tier>> {
	let mut converted = Vec::new();
	for tier in tiers {
		if tier.tier.kind != "context" || tier.tier.size == 0 {
			continue;
		}
		converted.push(catalog::Tier {
			context_over: tier.tier.size,
			rates: models_dev_rates(&tier.rates)?,
		});
	}
	converted.sort_by_key(|tier| tier.context_over);
	Ok(converted)
}

fn models_dev_rates(rates: &ModelsDevRates) -> anyhow::Result<catalog::Rates> {
	Ok(catalog::Rates {
		input: models_dev_money(&rates.input)?,
		output: models_dev_money(&rates.output)?,
		cache_read: models_dev_money(&rates.cache_read)?,
		cache_write: models_dev_money(&rates.cache_write)?,
		reasoning: models_dev_money(&rates.reasoning)?,
		input_audio: models_dev_money(&rates.input_audio)?,
		output_audio: models_dev_money(&rates.output_audio)?,
	})
}

fn models_dev_money(value: &Option<serde_json::Number>) -> anyhow::Result<Option<catalog::Money>> {
	let Some(value) = value else {
		return Ok(None);
	};
	let decimal = Decimal::from_str(&value.to_string()).map_err(|e| anyhow::anyhow!("{e}"))?;
	let rounded = if decimal.scale() > 6 {
		decimal.round_dp(6)
	} else {
		decimal
	};
	catalog::Money::parse(&rounded.to_string())
		.map(Some)
		.map_err(anyhow::Error::msg)
}

const MODELS_DEV_PROVIDER_IDS: &[(&str, &str)] = &[
	("openai", "openai"),
	("anthropic", "anthropic"),
	("amazon-bedrock", "aws.bedrock"),
	("google", "gcp.gemini"),
	("google-vertex", "gcp.vertex_ai"),
	("azure", "azure"),
	("github-copilot", "copilot"),
	("cohere", "cohere"),
	("baseten", "baseten"),
	("cerebras", "cerebras"),
	("deepinfra", "deepinfra"),
	("deepseek", "deepseek"),
	("groq", "groq"),
	("huggingface", "huggingface"),
	("mistral", "mistral"),
	("openrouter", "openrouter"),
	("togetherai", "togetherai"),
	("xai", "xai"),
	("fireworks-ai", "fireworks"),
];
