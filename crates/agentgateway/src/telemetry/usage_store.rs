/// Per-user token usage and cost tracking.
///
/// Tracks cumulative token consumption and estimated cost for each (user, model) pair.
/// Data is held in memory and optionally persisted to a JSON file on disk so it
/// survives gateway restarts.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// ── Pricing ──────────────────────────────────────────────────────────────────

/// Per-model pricing in USD per million tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelPricing {
    /// Cost per million input tokens (USD).
    #[serde(default)]
    pub input: f64,
    /// Cost per million output tokens (USD).
    #[serde(default)]
    pub output: f64,
    /// Cost per million cache-read tokens (USD).
    /// When `None` the input price is used as the fallback.
    pub cache_read: Option<f64>,
    /// Cost per million cache-write tokens (USD).
    /// When `None` the input price is used as the fallback.
    pub cache_write: Option<f64>,
}

impl ModelPricing {
    pub fn compute_cost(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> f64 {
        let m = 1_000_000.0_f64;
        let cache_read_price = self.cache_read.unwrap_or(self.input);
        let cache_write_price = self.cache_write.unwrap_or(self.input);
        (input_tokens as f64) / m * self.input
            + (output_tokens as f64) / m * self.output
            + (cache_read_tokens as f64) / m * cache_read_price
            + (cache_write_tokens as f64) / m * cache_write_price
    }
}

/// Top-level pricing configuration, deserialized from the gateway config file.
///
/// All model name keys are normalized to lowercase at deserialization time so
/// lookups are always O(1) and case-insensitive. When multiple original keys
/// differ only by case, the lexicographically smallest key wins
/// deterministically.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PricingConfig {
    /// Map from model name to pricing (USD per million tokens).
    /// Keys are normalized to lowercase on deserialization.
    #[serde(default, deserialize_with = "deserialize_normalized_models")]
    pub models: HashMap<String, ModelPricing>,
}

/// Deserialize the models map while normalizing all keys to lowercase.
/// When two original keys differ only by case, the lexicographically smallest
/// original key is used (deterministic tie-breaking).
fn deserialize_normalized_models<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, ModelPricing>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = HashMap::<String, ModelPricing>::deserialize(deserializer)?;
    let mut canonical: HashMap<String, String> = HashMap::new();
    let mut normalized: HashMap<String, ModelPricing> = HashMap::new();
    for (key, pricing) in raw {
        let lower = key.to_lowercase();
        // Keep whichever original key is lexicographically smallest.
        let keep = match canonical.get(&lower) {
            Some(existing) => key < *existing,
            None => true,
        };
        if keep {
            canonical.insert(lower.clone(), key);
            normalized.insert(lower, pricing);
        }
    }
    Ok(normalized)
}

/// Zero-cost sentinel returned when a model is not found in the pricing config.
static ZERO_PRICING: ModelPricing = ModelPricing {
    input: 0.0,
    output: 0.0,
    cache_read: None,
    cache_write: None,
};

impl PricingConfig {
    /// Look up pricing for a model name.
    ///
    /// The lookup is case-insensitive (keys are normalized to lowercase at
    /// construction time). Returns a zero-cost sentinel if the model is not
    /// found.
    pub fn for_model(&self, model: &str) -> &ModelPricing {
        let lower = model.to_lowercase();
        self.models.get(&lower).unwrap_or(&ZERO_PRICING)
    }
}

// ── Usage records ─────────────────────────────────────────────────────────────

/// Composite key identifying a single (user, model) usage bucket.
/// The model component is stored in lowercase so that
/// `claude-sonnet-4-5-20250929` and `Claude-Sonnet-4-5-20250929` map to the
/// same bucket.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct UsageKey {
    user_id: String,
    /// Always lowercase.
    model: String,
}

impl UsageKey {
    fn new(user_id: &str, model: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            model: model.to_lowercase(),
        }
    }
}

/// Accumulated usage for one (user, model) pair.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub user_id: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    pub request_count: u64,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

impl UsageRecord {
    fn new(user_id: &str, model: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            // Preserve original casing in the display field.
            model: model.to_owned(),
            ..Default::default()
        }
    }
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// Thread-safe, optionally persistent usage store.
///
/// Call [`UsageStore::record`] from the request completion path to accumulate
/// token counts.  Call [`UsageStore::query`] to read the current state.
#[derive(Debug, Clone)]
pub struct UsageStore {
    inner: Arc<RwLock<StoreInner>>,
}

#[derive(Debug, Default)]
struct StoreInner {
    records: HashMap<UsageKey, UsageRecord>,
    persist_path: Option<PathBuf>,
}

impl UsageStore {
    /// Create a new in-memory store (no persistence).
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StoreInner::default())),
        }
    }

    /// Create a store backed by a JSON file.
    ///
    /// If the file exists its contents are loaded on startup.
    /// After every update the file is rewritten atomically via a background
    /// `spawn_blocking` task so the Tokio worker thread is not stalled.
    pub fn with_persistence(path: PathBuf) -> Self {
        let mut records: HashMap<UsageKey, UsageRecord> = HashMap::new();

        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<Vec<UsageRecord>>(&s) {
                    Ok(loaded) => {
                        for r in loaded {
                            records.insert(UsageKey::new(&r.user_id, &r.model), r);
                        }
                        debug!(path = %path.display(), count = records.len(), "loaded usage records from disk");
                    }
                    Err(e) => warn!(path = %path.display(), error = %e, "failed to parse usage store file, starting empty"),
                },
                Err(e) => warn!(path = %path.display(), error = %e, "failed to read usage store file, starting empty"),
            }
        }

        Self {
            inner: Arc::new(RwLock::new(StoreInner {
                records,
                persist_path: Some(path),
            })),
        }
    }

    /// Record one completed request's token usage.
    ///
    /// - `user_id`: identity of the caller (JWT sub or "anonymous")
    /// - `model`: the model name returned by the provider
    /// - `pricing`: pricing table used to compute estimated cost
    ///
    /// Persistence (if configured) is performed off the calling thread via
    /// `tokio::task::spawn_blocking` to avoid stalling the Tokio worker.
    pub fn record(
        &self,
        user_id: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        pricing: &PricingConfig,
    ) {
        let cost = pricing.for_model(model).compute_cost(
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        );
        let now = Utc::now();
        let key = UsageKey::new(user_id, model);

        // Snapshot path + updated records under the lock, then release before IO.
        let persist = {
            let mut inner = self.inner.write();
            let entry = inner
                .records
                .entry(key)
                .or_insert_with(|| UsageRecord::new(user_id, model));

            entry.input_tokens += input_tokens;
            entry.output_tokens += output_tokens;
            entry.cache_read_tokens += cache_read_tokens;
            entry.cache_write_tokens += cache_write_tokens;
            entry.cost_usd += cost;
            entry.request_count += 1;
            if entry.first_seen.is_none() {
                entry.first_seen = Some(now);
            }
            entry.last_seen = Some(now);

            // Collect snapshot while holding the lock; IO happens after.
            inner.persist_path.clone().map(|path| {
                let snapshot: Vec<UsageRecord> = inner.records.values().cloned().collect();
                (path, snapshot)
            })
        }; // write lock released here

        if let Some((path, snapshot)) = persist {
            tokio::task::spawn_blocking(move || {
                match serde_json::to_string(&snapshot) {
                    Ok(json) => {
                        let tmp = path.with_extension("tmp");
                        if let Err(e) =
                            std::fs::write(&tmp, &json).and_then(|_| std::fs::rename(&tmp, &path))
                        {
                            warn!(path = %path.display(), error = %e, "failed to persist usage store");
                        }
                    }
                    Err(e) => warn!(error = %e, "failed to serialize usage store"),
                }
            });
        }
    }

    /// Return all usage records, optionally filtered by user, model, and/or
    /// minimum `last_seen` timestamp.
    pub fn query(
        &self,
        user_id: Option<&str>,
        model: Option<&str>,
        since: Option<DateTime<Utc>>,
    ) -> Vec<UsageRecord> {
        let model_lower = model.map(|m| m.to_lowercase());
        let inner = self.inner.read();
        inner
            .records
            .values()
            .filter(|r| {
                user_id.map_or(true, |u| r.user_id == u)
                    && model_lower
                        .as_deref()
                        .map_or(true, |m| r.model.to_lowercase() == m)
                    && since.map_or(true, |s| r.last_seen.map_or(false, |ls| ls >= s))
            })
            .cloned()
            .collect()
    }
}

impl Default for UsageStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pricing() -> PricingConfig {
        // Construct via JSON so the normalizing deserializer runs.
        serde_json::from_str(r#"{"models":{"claude-sonnet":{"input":3.0,"output":15.0,"cacheRead":0.3,"cacheWrite":3.75}}}"#).unwrap()
    }

    #[test]
    fn pricing_exact_match() {
        let p = make_pricing();
        let m = p.for_model("claude-sonnet");
        assert_eq!(m.input, 3.0);
        assert_eq!(m.output, 15.0);
    }

    #[test]
    fn pricing_case_insensitive_match() {
        let p = make_pricing();
        assert_eq!(p.for_model("Claude-Sonnet").input, 3.0);
        assert_eq!(p.for_model("CLAUDE-SONNET").output, 15.0);
    }

    #[test]
    fn pricing_unknown_model_zero_cost() {
        let p = make_pricing();
        let m = p.for_model("unknown-model");
        assert_eq!(m.input, 0.0);
        assert_eq!(m.output, 0.0);
    }

    #[test]
    fn pricing_deterministic_case_collision() {
        // When two keys differ only by case, the lexicographically smallest wins.
        // "ModelA" < "modela" lexicographically, so "ModelA" is the canonical key.
        let p: PricingConfig = serde_json::from_str(
            r#"{"models":{"ModelA":{"input":1.0,"output":2.0},"modela":{"input":9.0,"output":9.0}}}"#,
        ).unwrap();
        // All lookups hit the same normalized key; "ModelA" wins (lex smallest).
        assert_eq!(p.for_model("ModelA").input, 1.0);
        assert_eq!(p.for_model("modela").input, 1.0);
        assert_eq!(p.for_model("MODELA").input, 1.0);
    }

    #[test]
    fn compute_cost_correct() {
        let m = ModelPricing { input: 3.0, output: 15.0, cache_read: Some(0.3), cache_write: Some(3.75) };
        // 1M input + 1M output = $3 + $15 = $18
        let cost = m.compute_cost(1_000_000, 1_000_000, 0, 0);
        assert!((cost - 18.0).abs() < 1e-9);
    }

    #[test]
    fn compute_cost_cache_fallback_to_input() {
        // When cache_read/cache_write are None, input price is used as fallback.
        let m = ModelPricing { input: 4.0, output: 12.0, cache_read: None, cache_write: None };
        // 1M cache-read tokens billed at input rate = $4
        let cost = m.compute_cost(0, 0, 1_000_000, 0);
        assert!((cost - 4.0).abs() < 1e-9);
    }

    #[test]
    fn record_and_query_basic() {
        let store = UsageStore::new();
        let pricing = make_pricing();

        store.record("alice", "claude-sonnet", 100, 50, 0, 0, &pricing);
        store.record("alice", "claude-sonnet", 200, 80, 0, 0, &pricing);
        store.record("bob", "claude-sonnet", 10, 5, 0, 0, &pricing);

        let all = store.query(None, None, None);
        assert_eq!(all.len(), 2);

        let alice = store.query(Some("alice"), None, None);
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].input_tokens, 300);
        assert_eq!(alice[0].output_tokens, 130);
        assert_eq!(alice[0].request_count, 2);
        // cost: (300 * 3.0 + 130 * 15.0) / 1_000_000
        let expected = (300.0 * 3.0 + 130.0 * 15.0) / 1_000_000.0;
        assert!((alice[0].cost_usd - expected).abs() < 1e-9);
    }

    #[test]
    fn record_normalizes_model_case() {
        let store = UsageStore::new();
        let pricing = make_pricing();

        store.record("alice", "Claude-Sonnet", 100, 50, 0, 0, &pricing);
        store.record("alice", "CLAUDE-SONNET", 100, 50, 0, 0, &pricing);

        // Both should land in the same bucket.
        let records = store.query(Some("alice"), None, None);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].request_count, 2);
        assert_eq!(records[0].input_tokens, 200);
    }

    #[test]
    fn query_model_filter_case_insensitive() {
        let store = UsageStore::new();
        let pricing = make_pricing();

        store.record("alice", "claude-sonnet", 10, 5, 0, 0, &pricing);

        assert_eq!(store.query(None, Some("claude-sonnet"), None).len(), 1);
        assert_eq!(store.query(None, Some("Claude-Sonnet"), None).len(), 1);
        assert_eq!(store.query(None, Some("other"), None).len(), 0);
    }

    #[test]
    fn query_since_filter() {
        let store = UsageStore::new();
        let pricing = make_pricing();

        store.record("alice", "claude-sonnet", 10, 5, 0, 0, &pricing);

        let future = Utc::now() + chrono::Duration::hours(1);
        assert_eq!(store.query(None, None, Some(future)).len(), 0);
        assert_eq!(store.query(None, None, None).len(), 1);
    }
}
