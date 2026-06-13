use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use arc_swap::ArcSwap;
use catalog::{Breakdown, Catalog as CatalogData, Rates, Usage};
use prometheus_client::encoding::EncodeLabelValue;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use super::{CacheTokenConvention, LLMInfo, LLMResponse};

mod catalog;

pub struct ModelCatalog {
	snapshot: ArcSwap<CatalogSnapshot>,
}

impl fmt::Debug for ModelCatalog {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ModelCatalog")
			.field("snapshot", &*self.snapshot.load())
			.finish()
	}
}

impl Default for ModelCatalog {
	fn default() -> Self {
		Self {
			snapshot: ArcSwap::from_pointee(CatalogSnapshot::empty()),
		}
	}
}

impl ModelCatalog {
	pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<Arc<Self>> {
		let catalog = Arc::new(Self::default());
		if paths.is_empty() {
			return Ok(catalog);
		}
		tokio::spawn({
			let paths = paths.clone();
			let catalog = catalog.clone();
			async move {
				match load_files(&paths).await {
					Ok(snap) => {
						info!(count = paths.len(), "loaded model catalog");
						catalog.snapshot.store(Arc::new(snap));
					},
					Err(e) => {
						warn!("model catalog load failed; will load when the files become valid: {e:#}")
					},
				}
			}
		});
		watch_catalog_files(paths, catalog.clone())?;
		Ok(catalog)
	}

	pub fn empty() -> Arc<Self> {
		Arc::new(Self::default())
	}

	pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
		self.snapshot.load_full()
	}

	pub fn project(&self, info: &LLMInfo) -> CostProjection {
		let provider = info.request.provider.as_str();
		let model = info
			.response
			.provider_model
			.as_ref()
			.unwrap_or(&info.request.request_model);
		self.snapshot.load().project(
			provider,
			model.as_str(),
			&info.response,
			info.request.cache_convention,
		)
	}
}

pub struct CatalogSnapshot {
	catalog: Option<CatalogData>,
}

impl fmt::Debug for CatalogSnapshot {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("CatalogSnapshot")
			.field("loaded", &self.catalog.is_some())
			.finish()
	}
}

impl CatalogSnapshot {
	#[cfg(test)]
	pub fn parse(json: &str) -> anyhow::Result<Self> {
		Ok(Self::from_catalogs([catalog::from_json(json)?]))
	}

	fn from_catalogs(catalogs: impl IntoIterator<Item = CatalogData>) -> Self {
		let merged = catalogs
			.into_iter()
			.fold(CatalogData::default(), CatalogData::override_with);
		CatalogSnapshot {
			catalog: Some(merged),
		}
	}

	fn empty() -> Self {
		CatalogSnapshot { catalog: None }
	}

	pub fn price(
		&self,
		provider: &str,
		model: &str,
		resp: &LLMResponse,
		convention: CacheTokenConvention,
	) -> (Option<f64>, CostLookupStatus) {
		let p = self.project(provider, model, resp, convention);
		(p.amount(), p.status)
	}

	fn project(
		&self,
		provider: &str,
		model: &str,
		resp: &LLMResponse,
		convention: CacheTokenConvention,
	) -> CostProjection {
		let Some(catalog) = self.catalog.as_ref() else {
			return CostProjection::unpriced(CostLookupStatus::NoCatalog);
		};
		let entry = catalog.resolve(provider, model);
		let Some(entry) = entry else {
			return CostProjection::unpriced(CostLookupStatus::Missing);
		};

		let provisional_usage = usage_for(convention, resp, true);
		// Tier selection must be invariant to cache-read repricing below: the
		// cache tokens may move between input/cache_read, but their sum is stable.
		let context_tokens = provisional_usage.context_tokens();
		let rates = entry.effective_rates(context_tokens);
		if rates.is_empty() {
			return CostProjection::unpriced(CostLookupStatus::Unpriced);
		}

		let usage = if rates.cache_read.is_some() {
			provisional_usage
		} else {
			usage_for(convention, resp, false)
		};
		CostProjection {
			status: CostLookupStatus::Exact,
			cost: Some(CostBreakdown::from(&rates.breakdown(&usage))),
			cost_rates: Some(CostRates::from(&rates)),
		}
	}
}

#[derive(Debug, Clone)]
pub struct CostProjection {
	pub status: CostLookupStatus,
	pub cost: Option<CostBreakdown>,
	pub cost_rates: Option<CostRates>,
}

impl CostProjection {
	fn unpriced(status: CostLookupStatus) -> Self {
		CostProjection {
			status,
			cost: None,
			cost_rates: None,
		}
	}

	pub fn amount(&self) -> Option<f64> {
		self.cost.map(|c| c.total)
	}
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ::cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CostRates {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[dynamic(rename = "cacheRead")]
	pub cache_read: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[dynamic(rename = "cacheWrite")]
	pub cache_write: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reasoning: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[dynamic(rename = "inputAudio")]
	pub input_audio: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[dynamic(rename = "outputAudio")]
	pub output_audio: Option<f64>,
}

impl From<&Rates> for CostRates {
	fn from(r: &Rates) -> Self {
		let f = |m: &Option<catalog::Money>| m.as_ref().and_then(|m| m.0.to_f64());
		CostRates {
			input: f(&r.input),
			output: f(&r.output),
			cache_read: f(&r.cache_read),
			cache_write: f(&r.cache_write),
			reasoning: f(&r.reasoning),
			input_audio: f(&r.input_audio),
			output_audio: f(&r.output_audio),
		}
	}
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ::cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CostBreakdown {
	pub total: f64,
	pub input: f64,
	pub output: f64,
	#[dynamic(rename = "cacheRead")]
	pub cache_read: f64,
	#[dynamic(rename = "cacheWrite")]
	pub cache_write: f64,
	pub reasoning: f64,
	#[dynamic(rename = "inputAudio")]
	pub input_audio: f64,
	#[dynamic(rename = "outputAudio")]
	pub output_audio: f64,
}

impl From<&Breakdown> for CostBreakdown {
	fn from(b: &Breakdown) -> Self {
		let f = |d: rust_decimal::Decimal| d.to_f64().unwrap_or_default();
		CostBreakdown {
			total: f(b.total()),
			input: f(b.input),
			output: f(b.output),
			cache_read: f(b.cache_read),
			cache_write: f(b.cache_write),
			reasoning: f(b.reasoning),
			input_audio: f(b.input_audio),
			output_audio: f(b.output_audio),
		}
	}
}

async fn load_files(paths: &[PathBuf]) -> anyhow::Result<CatalogSnapshot> {
	let mut catalogs = Vec::with_capacity(paths.len());
	for path in paths {
		let json = fs_err::tokio::read_to_string(path)
			.await
			.with_context(|| format!("reading model catalog {}", path.display()))?;
		let catalog = catalog::from_json(&json)
			.with_context(|| format!("invalid model catalog at {}", path.display()))?;
		catalogs.push(catalog);
	}
	Ok(CatalogSnapshot::from_catalogs(catalogs))
}

fn watch_catalog_files(paths: Vec<PathBuf>, catalog: Arc<ModelCatalog>) -> anyhow::Result<()> {
	let watched = crate::util::watch_files(paths)?;
	let paths = watched.paths;
	let mut changes = watched.changes;
	info!(count = paths.len(), "watching model catalog files");
	tokio::task::spawn(async move {
		while changes.recv().await.is_some() {
			match load_files(&paths).await {
				Ok(snap) => {
					info!("model catalog reloaded");
					catalog.snapshot.store(Arc::new(snap));
				},
				Err(e) => {
					error!("failed to reload model catalog; keeping last valid catalog: {e:#}")
				},
			}
		}
	});
	Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EncodeLabelValue)]
pub enum CostLookupStatus {
	Exact,
	Unpriced,
	#[default]
	Missing,
	NoCatalog,
}

fn usage_for(
	convention: CacheTokenConvention,
	resp: &LLMResponse,
	prices_cache_read: bool,
) -> Usage {
	let mut cache_read = resp.cached_input_tokens.unwrap_or(0);
	let cache_write = resp.cache_creation_input_tokens.unwrap_or(0);
	let input_audio = resp.input_audio_tokens.unwrap_or(0);
	let output_audio = resp.output_audio_tokens.unwrap_or(0);
	let reasoning = resp.reasoning_tokens.unwrap_or(0);

	let mut input = resp.input_tokens.unwrap_or(0).saturating_sub(input_audio);
	match (convention, prices_cache_read) {
		(CacheTokenConvention::InputIncludesCache, true) => {
			input = input.saturating_sub(cache_read);
		},
		(CacheTokenConvention::InputIncludesCache, false) => {
			// Cached tokens are already included in input_tokens; zero the separate
			// bucket so they aren't double-counted or left unrated.
			cache_read = 0;
		},
		(CacheTokenConvention::InputExcludesCache, true) => {
			// cache_read is already separate from input; keep as-is.
		},
		(CacheTokenConvention::InputExcludesCache, false) => {
			// No cache_read rate in the catalog: fold cached tokens into input so
			// they're billed at the input rate rather than going unrated ($0).
			input = input.saturating_add(cache_read);
			cache_read = 0;
		},
	}
	let output = resp
		.output_tokens
		.unwrap_or(0)
		.saturating_sub(reasoning)
		.saturating_sub(output_audio);

	Usage {
		input,
		cache_read,
		cache_write,
		output,
		reasoning,
		input_audio,
		output_audio,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_catalog(input_rate: &str) -> String {
		format!(
			r#"{{"providers":{{"openai":{{"models":{{"my-model":{{"rates":{{"input":"{input_rate}","output":"2"}}}}}}}}}}}}"#
		)
	}

	#[test]
	fn openai_family_splits_cached_out_of_input_when_priced() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			output_tokens: Some(500),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true);
		assert_eq!(u.input, 700, "fresh input excludes the cached portion");
		assert_eq!(u.cache_read, 300);
		assert_eq!(u.output, 500);
	}

	#[test]
	fn openai_keeps_cache_in_input_when_unpriced() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			output_tokens: Some(500),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, false);
		assert_eq!(u.input, 1000, "cached tokens remain billable in input");
		assert_eq!(u.cache_read, 0, "no separate cache bucket");
	}

	#[test]
	fn anthropic_reports_fresh_input_with_cache_separate() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			cache_creation_input_tokens: Some(200),
			output_tokens: Some(500),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputExcludesCache, &resp, true);
		assert_eq!(u.input, 1000, "Anthropic input_tokens is already fresh");
		assert_eq!(u.cache_read, 300);
		assert_eq!(u.cache_write, 200);
	}

	#[test]
	fn exclusive_convention_never_subtracts_cache_from_input() {
		// Vertex Anthropic / custom-Messages case: input_tokens is already fresh.
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputExcludesCache, &resp, true);
		assert_eq!(
			u.input, 1000,
			"fresh input must not be reduced by cache_read"
		);
		assert_eq!(u.cache_read, 300);
	}

	#[test]
	fn inclusive_convention_splits_cache_out_of_input() {
		// Regression guard: OpenAI-style providers keep the subtract-once behavior.
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true);
		assert_eq!(u.input, 700);
		assert_eq!(u.cache_read, 300);
	}

	#[test]
	fn openai_splits_audio_and_reasoning_and_conserves_totals() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			input_audio_tokens: Some(200),
			output_tokens: Some(800),
			reasoning_tokens: Some(500),
			output_audio_tokens: Some(100),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true);
		assert_eq!(u.input, 500, "fresh text = 1000 - 300 cached - 200 audio");
		assert_eq!(u.cache_read, 300);
		assert_eq!(u.input_audio, 200);
		assert_eq!(
			u.output, 200,
			"text output = 800 - 500 reasoning - 100 audio"
		);
		assert_eq!(u.reasoning, 500);
		assert_eq!(u.output_audio, 100);
		assert_eq!(u.input + u.cache_read + u.input_audio, 1000);
		assert_eq!(u.output + u.reasoning + u.output_audio, 800);
	}

	#[test]
	fn prices_a_known_model() {
		let snap = CatalogSnapshot::parse(&test_catalog("1")).unwrap();
		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			output_tokens: Some(500_000),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"my-model",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(2.0));
	}

	#[test]
	fn empty_model_catalog_reports_no_catalog() {
		let catalog = ModelCatalog::default();
		let resp = LLMResponse {
			input_tokens: Some(1000),
			..Default::default()
		};
		let (cost, status) = catalog.snapshot().price(
			"openai",
			"my-model",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(cost, None);
		assert_eq!(status, CostLookupStatus::NoCatalog);
	}

	#[test]
	fn unknown_model_is_missing() {
		let snap = CatalogSnapshot::parse(&test_catalog("1")).unwrap();
		let resp = LLMResponse {
			input_tokens: Some(1000),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"totally-made-up",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(cost, None);
		assert_eq!(status, CostLookupStatus::Missing);
	}

	#[test]
	fn later_layer_overrides_earlier() {
		let base = catalog::from_json(&test_catalog("1")).unwrap();
		let overlay = catalog::from_json(&test_catalog("9")).unwrap();
		let snap = CatalogSnapshot::from_catalogs([base, overlay]);
		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			..Default::default()
		};
		let (cost, _) = snap.price(
			"openai",
			"my-model",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(cost, Some(9.0), "later layer's rate wins");
	}

	#[test]
	fn rateless_model_is_unpriced_not_free() {
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"openai":{"models":{
				"listed":{"rates":{}}
			}}}}"#,
		)
		.unwrap();
		let resp = LLMResponse {
			input_tokens: Some(1000),
			output_tokens: Some(500),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"listed",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Unpriced);
		assert_eq!(cost, None, "rate-less entries must not price as $0");
	}

	#[test]
	fn projection_includes_effective_cost_rates() {
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"openai":{"models":{
				"m":{
					"rates":{"input":"1.25","output":"10"},
					"tiers":[{"contextOver":200000,"rates":{"input":"2.5","cacheRead":"0.25"}}]
				}
			}}}}"#,
		)
		.unwrap();
		let resp = LLMResponse {
			input_tokens: Some(300_000),
			cached_input_tokens: Some(100_000),
			..Default::default()
		};
		let p = snap.project(
			"openai",
			"m",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(p.status, CostLookupStatus::Exact);
		let rates = p.cost_rates.expect("priced projection has rates");
		assert_eq!(rates.input, Some(2.5));
		assert_eq!(rates.output, Some(10.0));
		assert_eq!(rates.cache_read, Some(0.25));
	}

	#[test]
	fn exclusive_convention_folds_cache_into_input_when_unpriced() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			output_tokens: Some(500),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputExcludesCache, &resp, false);
		assert_eq!(u.input, 1300, "cached tokens folded into input for billing");
		assert_eq!(u.cache_read, 0, "no separate cache bucket");
		assert_eq!(u.output, 500);
	}

	#[test]
	fn exclusive_unpriced_cache_is_billed_at_input_rate_not_zero() {
		// Anthropic-style provider whose catalog entry has no cacheRead rate.
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"anthropic":{"models":{
				"m":{"rates":{"input":"10","output":"30"}}
			}}}}"#,
		)
		.unwrap();
		let resp = LLMResponse {
			input_tokens: Some(600_000),
			cached_input_tokens: Some(400_000),
			output_tokens: Some(0),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"anthropic",
			"m",
			&resp,
			CacheTokenConvention::InputExcludesCache,
		);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(10.0), "1M tokens @ $10/M = $10");
	}

	#[test]
	fn unpriced_cache_is_billed_at_input_rate_not_zero() {
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"openai":{"models":{
				"m":{"rates":{"input":"10"}}
			}}}}"#,
		)
		.unwrap();
		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			cached_input_tokens: Some(400_000),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"m",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(10.0));
	}

	#[test]
	fn cache_read_rate_only_applies_in_effective_tier() {
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"openai":{"models":{
				"m":{
					"rates":{"input":"10"},
					"tiers":[{"contextOver":200000,"rates":{"cacheRead":"1"}}]
				}
			}}}}"#,
		)
		.unwrap();
		let below_tier = LLMResponse {
			input_tokens: Some(100_000),
			cached_input_tokens: Some(40_000),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"m",
			&below_tier,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(1.0));

		let above_tier = LLMResponse {
			input_tokens: Some(300_000),
			cached_input_tokens: Some(100_000),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"m",
			&above_tier,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(2.1));
	}

	#[test]
	fn tier_only_model_is_unpriced_until_tier_applies() {
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"openai":{"models":{
				"m":{"tiers":[{"contextOver":200000,"rates":{"input":"10"}}]}
			}}}}"#,
		)
		.unwrap();
		let below_tier = LLMResponse {
			input_tokens: Some(100_000),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"m",
			&below_tier,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Unpriced);
		assert_eq!(cost, None);

		let above_tier = LLMResponse {
			input_tokens: Some(300_000),
			..Default::default()
		};
		let (cost, status) = snap.price(
			"openai",
			"m",
			&above_tier,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(3.0));
	}
}
