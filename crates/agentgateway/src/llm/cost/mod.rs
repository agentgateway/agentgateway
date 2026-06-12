use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Context;
use arc_swap::ArcSwap;
use catalog::{Breakdown, Catalog, Rates, Usage};
use notify::RecursiveMode;
use prometheus_client::encoding::EncodeLabelValue;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use super::{LLMInfo, LLMResponse};

mod catalog;

static CATALOG: LazyLock<ArcSwap<CatalogSnapshot>> =
	LazyLock::new(|| ArcSwap::from_pointee(CatalogSnapshot::empty()));

pub struct CatalogSnapshot {
	catalog: Option<Catalog>,
}

impl fmt::Debug for CatalogSnapshot {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("CatalogSnapshot")
			.field("loaded", &self.catalog.is_some())
			.finish()
	}
}

impl CatalogSnapshot {
	pub fn parse(json: &str) -> anyhow::Result<Self> {
		Ok(Self::from_catalogs([catalog::from_json(json)?]))
	}

	fn from_catalogs(catalogs: impl IntoIterator<Item = Catalog>) -> Self {
		let merged = catalogs
			.into_iter()
			.fold(Catalog::default(), Catalog::override_with);
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
	) -> (Option<f64>, CostLookupStatus) {
		let p = self.project(provider, model, resp);
		(p.amount(), p.status)
	}

	fn project(&self, provider: &str, model: &str, resp: &LLMResponse) -> CostProjection {
		let Some(catalog) = self.catalog.as_ref() else {
			return CostProjection::unpriced(CostLookupStatus::NoCatalog);
		};
		let entry = catalog.resolve(provider, model);
		let Some(entry) = entry else {
			return CostProjection::unpriced(CostLookupStatus::Missing);
		};

		let provisional_usage = usage_for(provider, resp, true);
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
			usage_for(provider, resp, false)
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

pub fn snapshot() -> Arc<CatalogSnapshot> {
	CATALOG.load_full()
}

pub fn project(info: &LLMInfo) -> CostProjection {
	let provider = info.request.provider.as_str();
	let model = info
		.response
		.provider_model
		.as_ref()
		.unwrap_or(&info.request.request_model);
	CATALOG
		.load()
		.project(provider, model.as_str(), &info.response)
}

pub fn init(paths: Vec<PathBuf>) -> anyhow::Result<()> {
	if paths.is_empty() {
		return Ok(());
	}
	tokio::spawn({
		let paths = paths.clone();
		async move {
			match load_files(&paths).await {
				Ok(snap) => {
					info!(count = paths.len(), "loaded model catalog");
					CATALOG.store(Arc::new(snap));
				},
				Err(e) => {
					warn!("model catalog load failed; will load when the files become valid: {e:#}")
				},
			}
		}
	});
	watch_catalog_files(paths)
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

fn watch_catalog_files(paths: Vec<PathBuf>) -> anyhow::Result<()> {
	let (tx, mut rx) = tokio::sync::mpsc::channel(1);
	let mut watcher =
		notify_debouncer_full::new_debouncer(Duration::from_millis(250), None, move |res| {
			futures::executor::block_on(async {
				let _ = tx.send(res).await;
			})
		})
		.map_err(|e| anyhow::anyhow!("failed to create model catalog watcher: {e}"))?;

	let abspaths = paths
		.iter()
		.map(std::path::absolute)
		.collect::<std::io::Result<Vec<_>>>()?;

	// Watch files and parents to catch edits, ConfigMap symlink rotations, and
	// Docker single-file bind mount updates.
	let mut watch_targets: Vec<PathBuf> = Vec::new();
	for abspath in &abspaths {
		let parent = abspath.parent().ok_or_else(|| {
			anyhow::anyhow!(
				"failed to get the parent of the model catalog file {}",
				abspath.display()
			)
		})?;
		for target in [abspath.as_path(), parent] {
			if !watch_targets.iter().any(|p| p == target) {
				watch_targets.push(target.to_path_buf());
			}
		}
	}

	let mut watched = false;
	let mut watch_errors = Vec::new();
	for target in &watch_targets {
		match watcher.watch(target, RecursiveMode::NonRecursive) {
			Ok(()) => watched = true,
			Err(e) => {
				watch_errors.push(format!("{}: {}", target.display(), e));
				warn!(
					"failed to watch model catalog path {}: {}",
					target.display(),
					e
				);
			},
		}
	}
	if !watched {
		return Err(anyhow::anyhow!(
			"failed to watch model catalog files: {}",
			watch_errors.join(", ")
		));
	}
	info!(count = paths.len(), "watching model catalog files");

	tokio::task::spawn(async move {
		while let Some(events) = rx.recv().await {
			match events {
				Ok(_) => match load_files(&abspaths).await {
					Ok(snap) => {
						info!("model catalog reloaded");
						CATALOG.store(Arc::new(snap));
					},
					Err(e) => {
						error!("failed to reload model catalog; keeping last valid catalog: {e:#}")
					},
				},
				Err(errors) => warn!("model catalog watch error: {errors:?}"),
			}
		}
		drop(watcher);
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

fn input_includes_cache(provider: &str) -> bool {
	!matches!(provider, "anthropic" | "aws.bedrock")
}

fn usage_for(provider: &str, resp: &LLMResponse, prices_cache_read: bool) -> Usage {
	let mut cache_read = resp.cached_input_tokens.unwrap_or(0);
	let cache_write = resp.cache_creation_input_tokens.unwrap_or(0);
	let input_audio = resp.input_audio_tokens.unwrap_or(0);
	let output_audio = resp.output_audio_tokens.unwrap_or(0);
	let reasoning = resp.reasoning_tokens.unwrap_or(0);

	let mut input = resp.input_tokens.unwrap_or(0).saturating_sub(input_audio);
	if input_includes_cache(provider) {
		if prices_cache_read {
			input = input.saturating_sub(cache_read);
		} else {
			// Inclusive providers keep cached tokens in input unless cache reads have
			// their own rate; otherwise cached tokens would become unrated.
			cache_read = 0;
		}
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
		let u = usage_for("openai", &resp, true);
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
		let u = usage_for("openai", &resp, false);
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
		let u = usage_for("anthropic", &resp, true);
		assert_eq!(u.input, 1000, "Anthropic input_tokens is already fresh");
		assert_eq!(u.cache_read, 300);
		assert_eq!(u.cache_write, 200);
	}

	#[test]
	fn bedrock_is_exclusive_like_anthropic() {
		assert!(!input_includes_cache("aws.bedrock"));
		assert!(!input_includes_cache("anthropic"));
		assert!(input_includes_cache("openai"));
		assert!(input_includes_cache("gcp.vertex_ai"));
		assert!(input_includes_cache("custom"));
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
		let u = usage_for("openai", &resp, true);
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
		let (cost, status) = snap.price("openai", "my-model", &resp);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(2.0));
	}

	#[test]
	fn global_snapshot_reports_no_catalog_until_configured() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			..Default::default()
		};
		let (cost, status) = snapshot().price("openai", "my-model", &resp);
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
		let (cost, status) = snap.price("openai", "totally-made-up", &resp);
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
		let (cost, _) = snap.price("openai", "my-model", &resp);
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
		let (cost, status) = snap.price("openai", "listed", &resp);
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
		let p = snap.project("openai", "m", &resp);
		assert_eq!(p.status, CostLookupStatus::Exact);
		let rates = p.cost_rates.expect("priced projection has rates");
		assert_eq!(rates.input, Some(2.5));
		assert_eq!(rates.output, Some(10.0));
		assert_eq!(rates.cache_read, Some(0.25));
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
		let (cost, status) = snap.price("openai", "m", &resp);
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
		let (cost, status) = snap.price("openai", "m", &below_tier);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(1.0));

		let above_tier = LLMResponse {
			input_tokens: Some(300_000),
			cached_input_tokens: Some(100_000),
			..Default::default()
		};
		let (cost, status) = snap.price("openai", "m", &above_tier);
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
		let (cost, status) = snap.price("openai", "m", &below_tier);
		assert_eq!(status, CostLookupStatus::Unpriced);
		assert_eq!(cost, None);

		let above_tier = LLMResponse {
			input_tokens: Some(300_000),
			..Default::default()
		};
		let (cost, status) = snap.price("openai", "m", &above_tier);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(3.0));
	}
}
