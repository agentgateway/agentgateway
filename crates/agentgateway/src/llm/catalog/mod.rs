use std::fmt;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use arc_swap::ArcSwap;
pub use model::{Breakdown, Catalog, CatalogMetadata};
use model::{Catalog as CatalogData, Rates, Usage};
use prometheus_client::encoding::EncodeLabelValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use super::{CacheTokenConvention, LLMInfo, LLMResponse};
use crate::{ModelCatalogSource, apply, schema};

mod model;
pub mod refresh;

const TRACE_POLICY_KIND: &str = "llm_cost";
const BUILTIN_CATALOG_JSON: &str = include_str!("../../../../../catalog/model-catalog.json");

pub struct ModelCatalog {
	state: ArcSwap<ModelCatalogState>,
	file_watch: Mutex<Option<tokio::task::AbortHandle>>,
}

struct ModelCatalogState {
	snapshot: Arc<CatalogSnapshot>,
	sources: Vec<ModelCatalogSource>,
}

impl fmt::Debug for ModelCatalog {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let state = self.state.load();
		f.debug_struct("ModelCatalog")
			.field("snapshot", &state.snapshot)
			.finish()
	}
}

impl Default for ModelCatalog {
	fn default() -> Self {
		Self {
			state: ArcSwap::from_pointee(ModelCatalogState {
				snapshot: Arc::new(CatalogSnapshot::empty()),
				sources: Vec::new(),
			}),
			file_watch: Mutex::new(None),
		}
	}
}

impl ModelCatalog {
	pub async fn new(sources: Vec<ModelCatalogSource>) -> anyhow::Result<Arc<Self>> {
		let builtin =
			model::from_json(BUILTIN_CATALOG_JSON).context("invalid built-in model catalog")?;
		let catalog = Arc::new(Self {
			state: ArcSwap::from_pointee(ModelCatalogState {
				snapshot: Arc::new(CatalogSnapshot::from_catalogs([builtin])),
				sources,
			}),
			file_watch: Mutex::new(None),
		});
		if !catalog.state.load().sources.is_empty()
			&& let Err(e) = catalog.reload().await
		{
			warn!(
				"model catalog overlay load failed; using built-in catalog until the configured sources become valid: {e:#}"
			)
		}
		catalog.update_file_watch()?;
		Ok(catalog)
	}

	pub fn empty() -> Arc<Self> {
		Arc::new(Self::default())
	}

	pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
		self.state.load().snapshot.clone()
	}

	pub fn list_models(&self) -> ModelCatalogModels {
		self.state.load().snapshot.list_models()
	}

	pub async fn replace_sources(
		self: &Arc<Self>,
		sources: Vec<ModelCatalogSource>,
	) -> anyhow::Result<()> {
		if self.state.load().sources == sources {
			return Ok(());
		}
		let loaded = load_sources(&sources).await?;
		log_loaded_catalog("model catalog reloaded", &loaded.snapshot, &loaded.missing);
		self.state.store(Arc::new(ModelCatalogState {
			snapshot: Arc::new(loaded.snapshot),
			sources,
		}));
		self.update_file_watch()?;
		Ok(())
	}

	fn update_file_watch(self: &Arc<Self>) -> anyhow::Result<()> {
		let file_paths = self
			.state
			.load()
			.sources
			.iter()
			.filter_map(|source| match source {
				ModelCatalogSource::File { file } => Some(file.clone()),
				ModelCatalogSource::Inline { .. } | ModelCatalogSource::InlineCatalog { .. } => None,
			})
			.collect::<Vec<_>>();
		let next = if file_paths.is_empty() {
			None
		} else {
			Some(watch_catalog_files(file_paths, self.clone())?)
		};
		let previous = std::mem::replace(
			&mut *self
				.file_watch
				.lock()
				.expect("model catalog file watch mutex poisoned"),
			next,
		);
		if let Some(previous) = previous {
			previous.abort();
		}
		Ok(())
	}

	pub(super) async fn reload(&self) -> anyhow::Result<()> {
		loop {
			let current = self.state.load_full();
			let loaded = load_sources(&current.sources).await?;
			let next = Arc::new(ModelCatalogState {
				snapshot: Arc::new(loaded.snapshot),
				sources: current.sources.clone(),
			});
			let previous = self.state.compare_and_swap(&current, next.clone());
			if Arc::ptr_eq(&previous, &current) {
				log_loaded_catalog("model catalog loaded", &next.snapshot, &loaded.missing);
				return Ok(());
			}
		}
	}

	pub fn project(&self, info: &LLMInfo) -> CostProjection {
		let provider = info.request.provider.as_str();
		let state = self.state.load();
		let snapshot = &state.snapshot;
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

impl ModelCatalog {
	/// Borrow as the cross-crate catalog handle threaded through the request path.
	pub fn as_handle(&self) -> &dyn agent_llm::model_catalog::ModelCatalogHandle {
		self
	}
}

impl agent_llm::model_catalog::ModelCatalogHandle for ModelCatalog {
	fn get_model_tags(&self, model_id: &str) -> Option<Arc<std::collections::BTreeSet<String>>> {
		self.state.load().snapshot.get_model_tags(model_id)
	}
}

pub struct CatalogSnapshot {
	catalog: Option<CatalogData>,
	/// Precomputed `model_id -> tags` (merged across providers) for O(1) attribute lookups.
	model_tags: std::collections::HashMap<String, Arc<std::collections::BTreeSet<String>>>,
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
		Ok(Self::from_catalogs([model::from_json(json)?]))
	}

	fn get_model_tags(&self, model_id: &str) -> Option<Arc<std::collections::BTreeSet<String>>> {
		self.model_tags.get(model_id).cloned()
	}

	fn from_catalogs(catalogs: impl IntoIterator<Item = CatalogData>) -> Self {
		let mut base: Option<CatalogData> = None;
		let mut overlays = Vec::new();
		for catalog in catalogs {
			let Some(candidate_metadata) = catalog.metadata.as_ref() else {
				overlays.push(catalog);
				continue;
			};
			let replace = base
				.as_ref()
				.and_then(|catalog| catalog.metadata.as_ref())
				.is_none_or(|current| candidate_metadata.generated_at >= current.generated_at);
			if replace {
				base = Some(catalog);
			}
		}
		let merged = overlays
			.into_iter()
			.fold(base.unwrap_or_default(), CatalogData::override_with);
		let model_tags = merged
			.providers
			.values()
			.flat_map(|p| p.models.iter())
			.filter(|(_, m)| !m.tags.is_empty())
			.map(|(id, m)| (id.clone(), Arc::new(m.tags.clone())))
			.collect();
		CatalogSnapshot {
			catalog: Some(merged),
			model_tags,
		}
	}

	fn empty() -> Self {
		CatalogSnapshot {
			catalog: None,
			model_tags: std::collections::HashMap::new(),
		}
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

		let provisional_usage = usage_for(convention, resp, true, true);
		// Tier selection must be invariant to cache repricing below: cache tokens
		// may move between input and their cache buckets, but their sum is stable.
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
		let prices_cache_write = rates.cache_write.is_some();
		let usage = if prices_cache_read && prices_cache_write {
			provisional_usage
		} else {
			usage_for(convention, resp, prices_cache_read, prices_cache_write)
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
				"pricesCacheWrite": prices_cache_write,
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

#[apply(schema!)]
#[derive(Copy, Default, ::cel::DynamicType)]
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
		let f = |m: &Option<model::Money>| m.as_ref().and_then(|m| m.0.to_f64());
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

#[apply(schema!)]
#[derive(Copy, Default, ::cel::DynamicType)]
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
	let builtin = model::from_json(BUILTIN_CATALOG_JSON).context("invalid built-in model catalog")?;
	let mut catalogs = Vec::with_capacity(sources.len() + 1);
	catalogs.push(builtin);
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
				let catalog = model::from_json(&json)
					.with_context(|| format!("invalid model catalog at {}", file.display()))?;
				catalogs.push(catalog);
			},
			ModelCatalogSource::Inline { inline } => {
				let catalog = model::from_json(inline).context("invalid inline model catalog")?;
				catalogs.push(catalog);
			},
			ModelCatalogSource::InlineCatalog { inline } => {
				inline.validate().context("invalid inline model catalog")?;
				catalogs.push(inline.clone());
			},
		}
	}
	Ok(LoadedCatalog {
		snapshot: CatalogSnapshot::from_catalogs(catalogs),
		missing,
	})
}

fn log_loaded_catalog(message: &'static str, snapshot: &CatalogSnapshot, missing: &[PathBuf]) {
	let catalog = snapshot.catalog.as_ref();
	let providers = catalog.map_or(0, |catalog| catalog.providers.len());
	let models = catalog.map_or(0, |catalog| {
		catalog.providers.values().map(|p| p.models.len()).sum()
	});
	info!(providers, models, "{}", message);
	if !missing.is_empty() {
		debug!(files = ?missing, "{} configured but missing", message);
	}
}

fn watch_catalog_files(
	file_paths: Vec<PathBuf>,
	catalog: Arc<ModelCatalog>,
) -> anyhow::Result<tokio::task::AbortHandle> {
	let watch_options = crate::util::WatchFilesOptions::default()
		.reload_on_disappearance(true)
		.close_on_removal(true);
	let mut watched = crate::util::watch_files_with_options(file_paths, watch_options)?;
	info!(
		count = watched.paths().len(),
		"watching model catalog files"
	);
	let task = tokio::task::spawn(async move {
		while let Some(invalidated) = watched.changed_invalidated().await {
			if let Err(e) = catalog.reload().await {
				error!("failed to reload model catalog; keeping last valid catalog: {e:#}")
			}
			if invalidated {
				match crate::util::watch_files_with_options(watched.paths().to_vec(), watch_options) {
					Ok(new_watched) => watched = new_watched,
					Err(e) => {
						warn!("failed to re-watch model catalog files: {e}");
						break;
					},
				}
			}
		}
	});
	Ok(task.abort_handle())
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
	prices_cache_write: bool,
) -> Usage {
	let mut cache_read = resp.cached_input_tokens.unwrap_or(0);
	let mut cache_write = resp.cache_creation_input_tokens.unwrap_or(0);
	let input_audio = resp.input_audio_tokens.unwrap_or(0);
	let output_audio = resp.output_audio_tokens.unwrap_or(0);
	let reasoning = resp.reasoning_tokens.unwrap_or(0);

	let mut input = resp.input_tokens.unwrap_or(0).saturating_sub(input_audio);
	match convention {
		CacheTokenConvention::InputIncludesCache => {
			if prices_cache_read {
				input = input.saturating_sub(cache_read);
			} else {
				cache_read = 0;
			}
			if prices_cache_write {
				input = input.saturating_sub(cache_write);
			} else {
				cache_write = 0;
			}
		},
		CacheTokenConvention::InputExcludesCache => {
			if !prices_cache_read {
				input = input.saturating_add(cache_read);
				cache_read = 0;
			}
			if !prices_cache_write {
				input = input.saturating_add(cache_write);
				cache_write = 0;
			}
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
	use std::path::Path;
	use std::time::Duration;

	use rust_decimal::prelude::ToPrimitive;

	use super::*;

	fn test_catalog(input_rate: &str) -> String {
		format!(
			r#"{{"providers":{{"openai":{{"models":{{"my-model":{{"rates":{{"input":"{input_rate}","output":"2"}}}}}}}}}}}}"#
		)
	}

	async fn write_catalog(path: &Path, input_rate: &str) {
		fs_err::tokio::write(path, test_catalog(input_rate))
			.await
			.unwrap();
	}

	async fn replace_catalog(path: &Path, input_rate: &str) {
		let replacement = path.with_extension(format!("{input_rate}.tmp"));
		write_catalog(&replacement, input_rate).await;
		fs_err::rename(&replacement, path).unwrap();
	}

	async fn wait_for_catalog_rate(catalog: &ModelCatalog, input_rate: f64) {
		let response = LLMResponse {
			input_tokens: Some(1_000_000),
			..Default::default()
		};
		tokio::time::timeout(Duration::from_secs(5), async {
			loop {
				let (cost, status) = catalog.snapshot().price(
					"openai",
					"my-model",
					&response,
					CacheTokenConvention::InputIncludesCache,
				);
				if status == CostLookupStatus::Exact && cost == Some(input_rate) {
					return;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
		})
		.await
		.unwrap_or_else(|_| panic!("timed out waiting for catalog input rate {input_rate}"));
	}

	fn test_llm_info(request_model: &str, provider_model: Option<&str>) -> LLMInfo {
		LLMInfo {
			request: crate::llm::LLMRequest {
				input_tokens: None,
				input_format: crate::llm::InputFormat::Completions,
				cache_convention: CacheTokenConvention::InputIncludesCache,
				request_model: request_model.into(),
				provider: "openai".into(),
				streaming: false,
				params: Default::default(),
				prompt: None,
				provider_state: None,
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
			state: ArcSwap::from_pointee(ModelCatalogState {
				snapshot: Arc::new(CatalogSnapshot::parse(json).unwrap()),
				sources: Vec::new(),
			}),
			file_watch: Mutex::new(None),
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
	fn snapshot_reads_model_tags_from_the_catalog() {
		let json = r#"{"providers":{"openai":{"models":{
			"gpt-oss-120b":{"tags":["preview"]},
			"gpt-4o":{"rates":{"input":"3.00"}}
		}}}}"#;
		let snapshot = CatalogSnapshot::parse(json).unwrap();
		let tags = snapshot
			.get_model_tags("gpt-oss-120b")
			.expect("tags present");
		assert!(tags.contains("preview"));
		// A model with rates but no tags has no entry.
		assert!(snapshot.get_model_tags("gpt-4o").is_none());
		assert!(snapshot.get_model_tags("unknown.model").is_none());
	}

	#[test]
	fn openai_family_splits_cached_out_of_input_when_priced() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			output_tokens: Some(500),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true, true);
		assert_eq!(u.input, 700, "fresh input excludes the cached portion");
		assert_eq!(u.cache_read, 300);
		assert_eq!(u.output, 500);
	}

	#[test]
	fn openai_family_splits_cache_reads_and_writes_out_of_input() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			cache_creation_input_tokens: Some(200),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true, true);
		assert_eq!(u.input, 500);
		assert_eq!(u.cache_read, 300);
		assert_eq!(u.cache_write, 200);
		assert_eq!(u.input + u.cache_read + u.cache_write, 1000);
	}

	#[test]
	fn openai_keeps_cache_in_input_when_unpriced() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			output_tokens: Some(500),
			..Default::default()
		};
		let u = usage_for(
			CacheTokenConvention::InputIncludesCache,
			&resp,
			false,
			false,
		);
		assert_eq!(u.input, 1000, "cached tokens remain billable in input");
		assert_eq!(u.cache_read, 0, "no separate cache bucket");
	}

	#[test]
	fn openai_still_splits_cache_writes_when_reads_are_unpriced() {
		let resp = LLMResponse {
			input_tokens: Some(1000),
			cached_input_tokens: Some(300),
			cache_creation_input_tokens: Some(200),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, false, true);
		assert_eq!(u.input, 800);
		assert_eq!(u.cache_read, 0);
		assert_eq!(u.cache_write, 200);
	}

	#[test]
	fn openai_prices_cache_writes_once() {
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"openai":{"models":{"gpt-5.6":{"rates":{"input":"1","cacheRead":"0.1","cacheWrite":"1.25"}}}}}}"#,
		)
		.unwrap();
		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			cached_input_tokens: Some(300_000),
			cache_creation_input_tokens: Some(200_000),
			..Default::default()
		};

		let projection = snap.project(
			"openai",
			"gpt-5.6",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		let cost = projection.cost.expect("model is priced");
		assert_eq!(cost.input.to_f64(), Some(0.5));
		assert_eq!(cost.cache_read.to_f64(), Some(0.03));
		assert_eq!(cost.cache_write.to_f64(), Some(0.25));
		assert_eq!(cost.total().to_f64(), Some(0.78));
	}

	#[test]
	fn openai_folds_cache_writes_into_input_when_unpriced() {
		let snap = CatalogSnapshot::parse(
			r#"{"providers":{"openai":{"models":{"gpt-5.6":{"rates":{"input":"1","cacheRead":"0.1"}}}}}}"#,
		)
		.unwrap();
		let resp = LLMResponse {
			input_tokens: Some(1_000_000),
			cached_input_tokens: Some(300_000),
			cache_creation_input_tokens: Some(200_000),
			..Default::default()
		};

		let projection = snap.project(
			"openai",
			"gpt-5.6",
			&resp,
			CacheTokenConvention::InputIncludesCache,
		);
		let cost = projection.cost.expect("model is priced");
		assert_eq!(cost.input.to_f64(), Some(0.7));
		assert_eq!(cost.cache_read.to_f64(), Some(0.03));
		assert_eq!(cost.cache_write.to_f64(), Some(0.0));
		assert_eq!(cost.total().to_f64(), Some(0.73));
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
		let u = usage_for(CacheTokenConvention::InputExcludesCache, &resp, true, true);
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
		let u = usage_for(CacheTokenConvention::InputExcludesCache, &resp, true, true);
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
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true, true);
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
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true, true);
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
	fn gemini_normalized_output_splits_reasoning_back_out() {
		// Gemini reports thoughts disjointly from candidates; UsageMetadata::counts()
		// normalizes output_tokens to candidates + thoughts (the convention the split
		// below assumes), so the split recovers the original buckets and thinking
		// tokens are billed at the output rate rather than vanishing from `output`.
		// Figures from a live gemini-2.5-flash response:
		// promptTokenCount 14, candidatesTokenCount 293, thoughtsTokenCount 203.
		let resp = LLMResponse {
			input_tokens: Some(14),
			output_tokens: Some(496),
			reasoning_tokens: Some(203),
			total_tokens: Some(510),
			..Default::default()
		};
		let u = usage_for(CacheTokenConvention::InputIncludesCache, &resp, true, true);
		assert_eq!(u.input, 14);
		assert_eq!(u.output, 293, "visible output = candidates");
		assert_eq!(u.reasoning, 203);
		assert_eq!(u.output + u.reasoning, 496);
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
		let base = model::from_json(&test_catalog("1")).unwrap();
		let overlay = model::from_json(&test_catalog("9")).unwrap();
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
	fn newest_generated_base_wins_before_user_overlays() {
		let generated = |day: u8, rate: &str| {
			model::from_json(&format!(
				r#"{{"metadata":{{"source":"models.dev","generatedAt":"2026-08-{day:02}T00:00:00Z"}},"providers":{{"openai":{{"models":{{"my-model":{{"rates":{{"input":"{rate}"}}}}}}}}}}}}"#
			))
			.unwrap()
		};
		let input_cost = |snapshot: &CatalogSnapshot| {
			snapshot
				.price(
					"openai",
					"my-model",
					&LLMResponse {
						input_tokens: Some(1_000_000),
						..Default::default()
					},
					CacheTokenConvention::InputIncludesCache,
				)
				.0
		};

		let day7 = CatalogSnapshot::from_catalogs([generated(5, "5"), generated(7, "7")]);
		assert_eq!(input_cost(&day7), Some(7.0));

		let day10 = CatalogSnapshot::from_catalogs([generated(10, "10"), generated(7, "7")]);
		assert_eq!(input_cost(&day10), Some(10.0));

		let user_override = model::from_json(&test_catalog("12")).unwrap();
		let overridden =
			CatalogSnapshot::from_catalogs([generated(10, "10"), generated(7, "7"), user_override]);
		assert_eq!(input_cost(&overridden), Some(12.0));
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
	async fn all_missing_layers_fall_back_to_builtin() {
		let dir = tempfile::tempdir().unwrap();
		let loaded = load_sources(&[ModelCatalogSource::File {
			file: dir.path().join("base.json"),
		}])
		.await
		.unwrap();

		assert_eq!(loaded.missing.len(), 1);
		assert!(loaded.snapshot.catalog.is_some());
		assert!(
			loaded
				.snapshot
				.catalog
				.as_ref()
				.unwrap()
				.resolve("openai", "gpt-4o-mini")
				.is_some(),
			"built-in catalog remains available"
		);
	}

	#[tokio::test]
	async fn metadata_free_file_is_an_overlay_regardless_of_name() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("base-costs.json");
		fs_err::tokio::write(
			&file,
			r#"{"providers":{"openai":{"models":{"gpt-4o-mini":{"rates":{"input":"999"}}}}}}"#,
		)
		.await
		.unwrap();
		let loaded = load_sources(&[ModelCatalogSource::File { file }])
			.await
			.unwrap();
		let (cost, status) = loaded.snapshot.price(
			"openai",
			"gpt-4o-mini",
			&LLMResponse {
				input_tokens: Some(1_000_000),
				..Default::default()
			},
			CacheTokenConvention::InputIncludesCache,
		);

		assert_eq!(status, CostLookupStatus::Exact);
		assert_eq!(cost, Some(999.0));
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
		let u = usage_for(
			CacheTokenConvention::InputExcludesCache,
			&resp,
			false,
			false,
		);
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
	async fn file_catalog_reloads_after_in_place_and_rename_updates() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("catalog.json");
		write_catalog(&path, "1").await;

		let catalog = ModelCatalog::new(vec![ModelCatalogSource::File { file: path.clone() }])
			.await
			.unwrap();
		wait_for_catalog_rate(&catalog, 1.0).await;

		write_catalog(&path, "2").await;
		wait_for_catalog_rate(&catalog, 2.0).await;

		replace_catalog(&path, "3").await;
		wait_for_catalog_rate(&catalog, 3.0).await;

		write_catalog(&path, "4").await;
		wait_for_catalog_rate(&catalog, 4.0).await;

		fs_err::rename(&path, dir.path().join("catalog_2.json")).unwrap();
		write_catalog(&path, "5").await;
		wait_for_catalog_rate(&catalog, 5.0).await;

		replace_catalog(&path, "6").await;
		wait_for_catalog_rate(&catalog, 6.0).await;

		let replacement_path = dir.path().join("replacement.json");
		write_catalog(&replacement_path, "7").await;
		catalog
			.replace_sources(vec![ModelCatalogSource::File {
				file: replacement_path.clone(),
			}])
			.await
			.unwrap();
		wait_for_catalog_rate(&catalog, 7.0).await;

		write_catalog(&replacement_path, "8").await;
		wait_for_catalog_rate(&catalog, 8.0).await;
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
