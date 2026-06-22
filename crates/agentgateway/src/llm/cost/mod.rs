use std::fmt;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use arc_swap::ArcSwap;
pub use catalog::Breakdown;
use catalog::{Catalog as CatalogData, Rates, Usage};
use prometheus_client::encoding::EncodeLabelValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use super::{CacheTokenConvention, LLMInfo, LLMResponse};
use crate::ModelCatalogSource;

mod catalog;
pub mod refresh;

const TRACE_POLICY_KIND: &str = "llm_cost";

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
	pub fn new(sources: Vec<ModelCatalogSource>) -> anyhow::Result<Arc<Self>> {
		let catalog = Arc::new(Self::default());
		if sources.is_empty() {
			return Ok(catalog);
		}
		let file_paths: Vec<PathBuf> = sources
			.iter()
			.filter_map(|s| match s {
				ModelCatalogSource::File { file } => Some(file.clone()),
				ModelCatalogSource::Inline { .. } => None,
			})
			.collect();
		tokio::spawn({
			let sources = sources.clone();
			let catalog = catalog.clone();
			async move {
				match load_sources(&sources).await {
					Ok(loaded) => {
						log_loaded_catalog("loaded model catalog", &loaded);
						catalog.snapshot.store(Arc::new(loaded.snapshot));
					},
					Err(e) => {
						warn!("model catalog load failed; will load when the files become valid: {e:#}")
					},
				}
			}
		});
		if !file_paths.is_empty() {
			watch_catalog_files(file_paths, sources, catalog.clone())?;
		}
		Ok(catalog)
	}

	pub fn empty() -> Arc<Self> {
		Arc::new(Self::default())
	}

	pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
		self.snapshot.load_full()
	}

	pub fn list_models(&self) -> ModelCatalogModels {
		self.snapshot.load().list_models()
	}

	pub fn replace(&self, snapshot: CatalogSnapshot) {
		self.snapshot.store(Arc::new(snapshot));
	}

	pub fn project(&self, info: &LLMInfo) -> CostProjection {
		let provider = info.request.provider.as_str();
		let snapshot = self.snapshot.load();
		if let Some(provider_model) = &info.response.provider_model {
			let projection = snapshot.project_with_missing_trace(
				provider,
				provider_model.as_str(),
				&info.response,
				info.request.cache_convention,
				false,
			);
			if projection.status != CostLookupStatus::Missing {
				return projection;
			}
		}
		snapshot.project(
			provider,
			info.request.request_model.as_str(),
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

	fn list_models(&self) -> ModelCatalogModels {
		let Some(catalog) = &self.catalog else {
			return ModelCatalogModels {
				loaded: false,
				providers: Vec::new(),
			};
		};
		ModelCatalogModels {
			loaded: true,
			providers: catalog
				.providers
				.iter()
				.map(|(provider, data)| ModelCatalogProviderModels {
					provider: provider.clone(),
					models: data.models.keys().cloned().collect(),
				})
				.collect(),
		}
	}

	fn project(
		&self,
		provider: &str,
		model: &str,
		resp: &LLMResponse,
		convention: CacheTokenConvention,
	) -> CostProjection {
		self.project_with_missing_trace(provider, model, resp, convention, true)
	}

	fn project_with_missing_trace(
		&self,
		provider: &str,
		model: &str,
		resp: &LLMResponse,
		convention: CacheTokenConvention,
		trace_missing: bool,
	) -> CostProjection {
		let Some(catalog) = self.catalog.as_ref() else {
			crate::proxy::dtrace::pol_event!(
				TRACE_POLICY_KIND,
				crate::proxy::dtrace::Severity::Warn,
				details = serde_json::json!({
					"provider": provider,
					"model": model,
					"status": status_name(CostLookupStatus::NoCatalog),
					"reason": "no model catalog",
				}),
			);
			return CostProjection::unpriced(CostLookupStatus::NoCatalog);
		};
		let entry = catalog.resolve(provider, model);
		let Some(entry) = entry else {
			if trace_missing {
				crate::proxy::dtrace::pol_event!(
					TRACE_POLICY_KIND,
					crate::proxy::dtrace::Severity::Warn,
					details = serde_json::json!({
						"provider": provider,
						"model": model,
						"status": status_name(CostLookupStatus::Missing),
						"reason": "no catalog entry for provider/model",
					}),
				);
			}
			return CostProjection::unpriced(CostLookupStatus::Missing);
		};

		let provisional_usage = usage_for(convention, resp, true);
		// Tier selection must be invariant to cache-read repricing below: the
		// cache tokens may move between input/cache_read, but their sum is stable.
		let context_tokens = provisional_usage.context_tokens();
		let rates = entry.effective_rates(context_tokens);
		if rates.is_empty() {
			crate::proxy::dtrace::pol_event!(
				TRACE_POLICY_KIND,
				crate::proxy::dtrace::Severity::Warn,
				details = serde_json::json!({
					"provider": provider,
					"model": model,
					"status": status_name(CostLookupStatus::Unpriced),
					"reason": "catalog entry has no effective rates",
					"cacheTokenConvention": cache_convention_name(convention),
					"contextTokens": context_tokens,
					"usage": &provisional_usage,
				}),
			);
			return CostProjection::unpriced(CostLookupStatus::Unpriced);
		}

		let prices_cache_read = rates.cache_read.is_some();
		let usage = if prices_cache_read {
			provisional_usage
		} else {
			usage_for(convention, resp, false)
		};
		let breakdown = rates.breakdown(&usage);
		let cost = CostBreakdown::from(&breakdown);
		let cost_rates = CostRates::from(&rates);
		crate::proxy::dtrace::pol_event!(
			TRACE_POLICY_KIND,
			crate::proxy::dtrace::Severity::Info,
			details = serde_json::json!({
				"provider": provider,
				"model": model,
				"status": status_name(CostLookupStatus::Exact),
				"cacheTokenConvention": cache_convention_name(convention),
				"contextTokens": context_tokens,
				"pricesCacheRead": prices_cache_read,
				"usage": &usage,
				"rates": cost_rates,
				"cost": cost,
			}),
		);
		CostProjection {
			status: CostLookupStatus::Exact,
			cost: Some(breakdown),
			cost_rates: Some(cost_rates),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogModels {
	pub loaded: bool,
	pub providers: Vec<ModelCatalogProviderModels>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogProviderModels {
	pub provider: String,
	pub models: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CostProjection {
	pub status: CostLookupStatus,
	pub cost: Option<Breakdown>,
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

fn breakdown_f64(d: Decimal) -> f64 {
	d.to_f64().unwrap_or_default()
}

impl Breakdown {
	// (CEL field name, value) pairs. `total` is computed, the rest are stored.
	fn components(&self) -> [(&'static str, Decimal); 8] {
		[
			("total", self.total()),
			("input", self.input),
			("output", self.output),
			("cacheRead", self.cache_read),
			("cacheWrite", self.cache_write),
			("reasoning", self.reasoning),
			("inputAudio", self.input_audio),
			("outputAudio", self.output_audio),
		]
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
		CostBreakdown {
			total: breakdown_f64(b.total()),
			input: breakdown_f64(b.input),
			output: breakdown_f64(b.output),
			cache_read: breakdown_f64(b.cache_read),
			cache_write: breakdown_f64(b.cache_write),
			reasoning: breakdown_f64(b.reasoning),
			input_audio: breakdown_f64(b.input_audio),
			output_audio: breakdown_f64(b.output_audio),
		}
	}
}

impl From<CostBreakdown> for Breakdown {
	fn from(b: CostBreakdown) -> Self {
		let d = |v| Decimal::from_f64(v).unwrap_or_default();
		Breakdown {
			input: d(b.input),
			output: d(b.output),
			cache_read: d(b.cache_read),
			cache_write: d(b.cache_write),
			reasoning: d(b.reasoning),
			input_audio: d(b.input_audio),
			output_audio: d(b.output_audio),
		}
	}
}

impl ::cel::types::dynamic::DynamicType for Breakdown {
	fn materialize(&self) -> ::cel::Value<'_> {
		let mut map = vector_map::VecMap::with_capacity(8);
		for (name, value) in self.components() {
			map.insert(
				::cel::objects::KeyRef::from(name),
				::cel::Value::from(breakdown_f64(value)),
			);
		}
		::cel::Value::Map(::cel::objects::MapValue::Borrow(map))
	}

	// Lazy: a field is only converted to f64 when a CEL expression reads it.
	fn field(&self, field: &str) -> Option<::cel::Value<'_>> {
		self
			.components()
			.into_iter()
			.find(|(name, _)| *name == field)
			.map(|(_, value)| ::cel::Value::from(breakdown_f64(value)))
	}
}

impl Serialize for Breakdown {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		CostBreakdown::from(self).serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for Breakdown {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		CostBreakdown::deserialize(deserializer).map(Into::into)
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Breakdown {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"CostBreakdown".into()
	}

	fn json_schema(schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
		CostBreakdown::json_schema(schema_gen)
	}
}

#[derive(Debug)]
struct LoadedCatalog {
	snapshot: CatalogSnapshot,
	missing: Vec<PathBuf>,
}

async fn load_sources(sources: &[ModelCatalogSource]) -> anyhow::Result<LoadedCatalog> {
	if sources.is_empty() {
		bail!("no model catalog sources supplied");
	}

	let mut catalogs = Vec::with_capacity(sources.len());
	let mut missing = Vec::new();
	for source in sources {
		match source {
			ModelCatalogSource::File { file } => {
				let json = match fs_err::tokio::read_to_string(file).await {
					Ok(json) => json,
					Err(e) if e.kind() == ErrorKind::NotFound => {
						missing.push(file.clone());
						continue;
					},
					Err(e) => {
						return Err(e).context("reading model catalog");
					},
				};
				let catalog = catalog::from_json(&json)
					.with_context(|| format!("invalid model catalog at {}", file.display()))?;
				catalogs.push(catalog);
			},
			ModelCatalogSource::Inline { inline } => {
				let catalog = catalog::from_json(inline).context("invalid inline model catalog")?;
				catalogs.push(catalog);
			},
		}
	}
	if catalogs.is_empty() {
		bail!(
			"no configured model catalog sources are currently readable; missing files: {}",
			missing
				.iter()
				.map(|p| p.display().to_string())
				.collect::<Vec<_>>()
				.join(", ")
		);
	}
	Ok(LoadedCatalog {
		snapshot: CatalogSnapshot::from_catalogs(catalogs),
		missing,
	})
}

fn log_loaded_catalog(message: &'static str, loaded: &LoadedCatalog) {
	let catalog = loaded.snapshot.catalog.as_ref();
	let providers = catalog.map_or(0, |catalog| catalog.providers.len());
	let models = catalog.map_or(0, |catalog| {
		catalog.providers.values().map(|p| p.models.len()).sum()
	});
	info!(providers, models, "{}", message);
	if !loaded.missing.is_empty() {
		debug!(files = ?loaded.missing, "{} configured but missing", message);
	}
}

fn watch_catalog_files(
	file_paths: Vec<PathBuf>,
	all_sources: Vec<ModelCatalogSource>,
	catalog: Arc<ModelCatalog>,
) -> anyhow::Result<()> {
	let mut watched = crate::util::watch_files_with_options(
		file_paths,
		crate::util::WatchFilesOptions::default().reload_on_disappearance(true),
	)?;
	info!(
		count = watched.paths().len(),
		"watching model catalog files"
	);
	tokio::task::spawn(async move {
		while watched.changed().await {
			match load_sources(&all_sources).await {
				Ok(loaded) => {
					log_loaded_catalog("model catalog reloaded", &loaded);
					catalog.snapshot.store(Arc::new(loaded.snapshot));
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

fn status_name(status: CostLookupStatus) -> &'static str {
	match status {
		CostLookupStatus::Exact => "exact",
		CostLookupStatus::Unpriced => "unpriced",
		CostLookupStatus::Missing => "missing",
		CostLookupStatus::NoCatalog => "noCatalog",
	}
}

fn cache_convention_name(convention: CacheTokenConvention) -> &'static str {
	match convention {
		CacheTokenConvention::InputIncludesCache => "inputIncludesCache",
		CacheTokenConvention::InputExcludesCache => "inputExcludesCache",
	}
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
	use rust_decimal::prelude::ToPrimitive;

	use super::*;

	fn test_catalog(input_rate: &str) -> String {
		format!(
			r#"{{"providers":{{"openai":{{"models":{{"my-model":{{"rates":{{"input":"{input_rate}","output":"2"}}}}}}}}}}}}"#
		)
	}

	fn test_llm_info(request_model: &str, provider_model: Option<&str>) -> LLMInfo {
		LLMInfo {
			request: crate::llm::LLMRequest {
				input_tokens: None,
				input_format: crate::llm::InputFormat::Completions,
				native_format: None,
				cache_convention: CacheTokenConvention::InputIncludesCache,
				request_model: request_model.into(),
				provider: "openai".into(),
				streaming: false,
				params: Default::default(),
				prompt: None,
			},
			response: LLMResponse {
				input_tokens: Some(1_000_000),
				output_tokens: Some(500_000),
				provider_model: provider_model.map(Into::into),
				..Default::default()
			},
		}
	}

	fn model_catalog(json: &str) -> ModelCatalog {
		ModelCatalog {
			snapshot: ArcSwap::from_pointee(CatalogSnapshot::parse(json).unwrap()),
		}
	}

	impl CatalogSnapshot {
		fn price(
			&self,
			provider: &str,
			model: &str,
			resp: &LLMResponse,
			convention: CacheTokenConvention,
		) -> (Option<f64>, CostLookupStatus) {
			let p = self.project(provider, model, resp, convention);
			(p.cost.and_then(|c| c.total().to_f64()), p.status)
		}
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
	fn project_falls_back_to_request_model_when_provider_model_is_missing() {
		let catalog = model_catalog(&test_catalog("1"));
		let projection = catalog.project(&test_llm_info("my-model", Some("unknown-provider-model")));

		assert_eq!(projection.status, CostLookupStatus::Exact);
		assert_eq!(
			projection.cost.and_then(|c| c.total().to_f64()),
			Some(2.0),
			"request model should price when provider model is absent from catalog"
		);
	}

	#[test]
	fn project_keeps_unpriced_provider_model_result() {
		let catalog = model_catalog(
			r#"{"providers":{"openai":{"models":{
				"my-model":{"rates":{"input":"1","output":"2"}},
				"provider-model":{"rates":{}}
			}}}}"#,
		);
		let projection = catalog.project(&test_llm_info("my-model", Some("provider-model")));

		assert_eq!(projection.status, CostLookupStatus::Unpriced);
		assert!(
			projection.cost.is_none(),
			"provider model was found, so request model fallback must not hide unpriced rates"
		);
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

	#[tokio::test]
	async fn missing_later_layer_is_skipped() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path().join("base.json");
		let override_file = dir.path().join("overrides.json");
		fs_err::tokio::write(&base, test_catalog("1"))
			.await
			.unwrap();

		let loaded = load_sources(&[
			ModelCatalogSource::File { file: base },
			ModelCatalogSource::File {
				file: override_file,
			},
		])
		.await
		.unwrap();
		assert_eq!(loaded.missing.len(), 1);
		assert_eq!(loaded.missing[0].file_name().unwrap(), "overrides.json");

		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			..Default::default()
		};
		let (cost, _) = loaded.snapshot.price(
			"openai",
			"my-model",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(cost, Some(1.0), "base layer remains usable");
	}

	#[tokio::test]
	async fn all_missing_layers_are_not_loaded() {
		let dir = tempfile::tempdir().unwrap();
		let err = load_sources(&[ModelCatalogSource::File {
			file: dir.path().join("base.json"),
		}])
		.await
		.unwrap_err();

		assert!(
			err
				.to_string()
				.contains("no configured model catalog sources are currently readable")
		);
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
		assert_eq!(
			p.cost.expect("priced projection has cost").total().to_f64(),
			Some(0.525),
			"tier rates apply to the whole request"
		);
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

	#[tokio::test]
	async fn inline_source_is_loaded() {
		let inline_json = test_catalog("5");
		let loaded = load_sources(&[ModelCatalogSource::Inline {
			inline: inline_json,
		}])
		.await
		.unwrap();
		assert_eq!(loaded.missing.len(), 0);
		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			..Default::default()
		};
		let (cost, status) = loaded.snapshot.price(
			"openai",
			"my-model",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(5.0));
	}

	#[tokio::test]
	async fn inline_source_overrides_file_source() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path().join("base.json");
		fs_err::tokio::write(&base, test_catalog("1"))
			.await
			.unwrap();

		let loaded = load_sources(&[
			ModelCatalogSource::File { file: base },
			ModelCatalogSource::Inline {
				inline: test_catalog("7"),
			},
		])
		.await
		.unwrap();
		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			..Default::default()
		};
		let (cost, _) = loaded.snapshot.price(
			"openai",
			"my-model",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		assert_eq!(cost, Some(7.0), "inline layer overrides file layer");
	}

	#[tokio::test]
	async fn invalid_inline_source_is_an_error() {
		let err = load_sources(&[ModelCatalogSource::Inline {
			inline: "not valid json".to_string(),
		}])
		.await
		.unwrap_err();
		assert!(err.to_string().contains("invalid inline model catalog"));
	}
}
