use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context;
use chrono::{DurationRound, TimeDelta, Utc};
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cel::LLMContext;
use crate::proxy::ProxyError;
use crate::{apply, schema_de, serde_dur};

mod database;
mod status;

pub use status::{
	BudgetStatus, BudgetStatusLimit, BudgetStatusResponse, BudgetStatusUsage, BudgetStatusWindow,
};

pub(crate) const NANODOLLARS_PER_USD: i64 = 1_000_000_000;
type UnixDate = chrono::DateTime<Utc>;

/// In-memory state for one budget. Database rows are preloaded at startup and configuration is
/// attached during policy registration, regardless of which happens first.
#[derive(Debug, Clone)]
struct BudgetCounter {
	definition: Option<BudgetDefinition>,
	amount: Decimal,
	pending: Decimal,
	unit: Option<BudgetLimitUnit>,
	/// [`UNBOUNDED_WINDOW`] for a budget without a window, whose usage never resets.
	rolling: Duration,
	window_start: UnixDate,
	window_end: UnixDate,
	updated_at: UnixDate,
}

impl BudgetCounter {
	fn configured(definition: &BudgetDefinition, now: UnixDate) -> anyhow::Result<Self> {
		let budget = &definition.budget;
		let rolling = budget
			.window
			.as_ref()
			.map_or(UNBOUNDED_WINDOW, |window| window.rolling);
		let (window_start, window_end) = budget_window(now, rolling)?;
		Ok(Self {
			definition: Some(definition.clone()),
			amount: Decimal::ZERO,
			pending: Decimal::ZERO,
			unit: Some(budget.limit.unit),
			rolling,
			window_start,
			window_end,
			updated_at: now,
		})
	}

	/// Attaches the latest definition and resets runtime state if its window or unit changed.
	fn configure(&mut self, definition: &BudgetDefinition, now: UnixDate) -> anyhow::Result<()> {
		let budget = &definition.budget;
		let rolling = budget
			.window
			.as_ref()
			.map_or(UNBOUNDED_WINDOW, |window| window.rolling);
		if now >= self.window_end || self.rolling != rolling || self.unit != Some(budget.limit.unit) {
			(self.window_start, self.window_end) = budget_window(now, rolling)?;
			self.amount = Decimal::ZERO;
			self.pending = Decimal::ZERO;
			self.unit = Some(budget.limit.unit);
			self.updated_at = now;
		}
		self.rolling = rolling;
		self.definition = Some(definition.clone());
		Ok(())
	}

	/// Adds usage persisted under a legacy budget id when it belongs to this counter's current
	/// window and unit; usage from another window or unit no longer counts.
	fn absorb(
		&mut self,
		unit: Option<BudgetLimitUnit>,
		window_start: UnixDate,
		window_end: UnixDate,
		amount: Decimal,
		updated_at: UnixDate,
	) {
		if unit == self.unit && window_start == self.window_start && window_end == self.window_end {
			self.amount += amount;
			self.updated_at = self.updated_at.max(updated_at);
		}
	}

	/// Advances an expired counter to the epoch-aligned fixed window containing `now`. Counters
	/// without a window never expire.
	fn refresh(&mut self, now: UnixDate) {
		if now < self.window_end {
			return;
		}
		(self.window_start, self.window_end) =
			budget_window(now, self.rolling).expect("budget duration was validated");
		self.amount = Decimal::ZERO;
		self.pending = Decimal::ZERO;
		self.updated_at = now;
	}
}

#[derive(Debug, Clone)]
struct PersistedBudgetUsage {
	window_start: UnixDate,
	window_end: UnixDate,
	unit: Option<BudgetLimitUnit>,
	used_amount: i64,
	updated_at: UnixDate,
}

#[derive(Debug, Clone)]
struct PendingBudgetUsage {
	budget_id: String,
	window_start: UnixDate,
	window_end: UnixDate,
	unit: BudgetLimitUnit,
	used_amount: i64,
	flushed: Decimal,
}

#[derive(Debug, Clone)]
struct BudgetDefinition {
	api_key: String,
	budget: Budget,
	/// Id this budget was stored under by releases that keyed budgets by the key hash. Only an
	/// alias: usage found under it is folded into this budget; see [`BudgetPolicy::legacy_rows`].
	legacy_id: Option<String>,
}

/// A named budget attached to a standalone API key.
///
/// Usage is charged after an LLM response when the provider reports the tokens or cost required by
/// the configured unit. Requests with unavailable usage are logged but cannot be charged or blocked
/// retroactively.
#[apply(schema_de!)]
pub struct Budget {
	/// Stable name for this budget within its owning API key.
	pub name: String,
	/// Maximum usage allowed during the window.
	pub limit: BudgetLimit,
	/// Rolling window over which usage will be accumulated. If unset, usage accumulates for the
	/// lifetime of the budget and is never reset.
	#[serde(default)]
	pub window: Option<BudgetWindow>,
	/// Action taken when the budget is exceeded.
	pub on_budget_exceeded: BudgetExceededAction,
}

#[apply(schema_de!)]
pub struct BudgetLimit {
	pub unit: BudgetLimitUnit,
	pub amount: BudgetAmount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BudgetAmount(Decimal);

impl BudgetAmount {
	pub fn decimal(self) -> Decimal {
		self.0
	}
}

impl std::fmt::Display for BudgetAmount {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl<'de> Deserialize<'de> for BudgetAmount {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let number = serde_json::Number::deserialize(deserializer)?;
		let amount = Decimal::from_str(&number.to_string()).map_err(serde::de::Error::custom)?;
		if amount < Decimal::ZERO {
			return Err(serde::de::Error::custom(
				"budget amount must not be negative",
			));
		}
		Ok(Self(amount))
	}
}

impl Serialize for BudgetAmount {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serde_json::Number::from_str(&self.0.normalize().to_string())
			.map_err(serde::ser::Error::custom)?
			.serialize(serializer)
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for BudgetAmount {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"BudgetAmount".into()
	}

	fn json_schema(schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
		<f64 as schemars::JsonSchema>::json_schema(schema_gen)
	}
}

#[apply(schema_de!)]
#[derive(Copy, Eq, PartialEq, Hash)]
pub enum BudgetLimitUnit {
	#[serde(rename = "USD")]
	Usd,
	#[serde(rename = "Tokens")]
	Tokens,
}

impl BudgetLimitUnit {
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Usd => "USD",
			Self::Tokens => "Tokens",
		}
	}

	fn from_database(value: &str) -> Option<Self> {
		match value {
			"USD" => Some(Self::Usd),
			"Tokens" => Some(Self::Tokens),
			_ => None,
		}
	}
}

#[apply(schema_de!)]
pub struct BudgetWindow {
	/// Duration of the fixed usage window, for example `1h`, `24h`, or `30d`.
	/// Windows are aligned to the Unix epoch rather than starting with the first request: `1h`
	/// follows UTC clock hours, `24h` starts at midnight UTC, and `30d` uses consecutive 30-day
	/// periods rather than calendar months.
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub rolling: Duration,
}

#[apply(schema_de!)]
#[derive(Copy)]
pub enum BudgetExceededAction {
	#[serde(rename = "Audit")]
	Audit,
	#[serde(rename = "Block")]
	Block,
}

impl BudgetExceededAction {
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Audit => "Audit",
			Self::Block => "Block",
		}
	}
}

#[derive(Debug, Clone)]
pub struct MatchedBudgets {
	pub(crate) api_key: String,
	pub(crate) api_key_id: String,
	pub(crate) budgets: Vec<Budget>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BudgetPolicy {
	/// All known counters, including preloaded rows whose configuration has not been registered.
	#[serde(skip)]
	counters: Arc<DashMap<String, BudgetCounter>>,
	/// Shared database pool installed once during policy initialization.
	#[serde(skip)]
	database: Arc<OnceLock<crate::database::DatabasePool>>,
	/// Serializes periodic, shutdown, and manually requested flushes across policy clones.
	#[serde(skip)]
	flush_lock: Arc<tokio::sync::Mutex<()>>,
	/// Definitions collected while a local configuration is being normalized. Registration policies
	/// share runtime counters with the process-wide policy but do not mutate them until reload succeeds.
	#[serde(skip)]
	registration: Option<Arc<DashMap<String, BudgetDefinition>>>,
	/// Legacy budget ids with a persisted row, mapped to the budget that already counts that usage
	/// in memory. The next flush merges each row into the current one and deletes it.
	///
	/// TODO: temporary migration from key-hash budget ids; remove once releases that wrote them are
	/// no longer supported.
	#[serde(skip)]
	legacy_rows: Arc<DashMap<String, String>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BudgetRegistration(HashMap<String, BudgetDefinition>);

/// Deferred charge captured while applying the budget policy and settled once LLM response usage
/// and cost are available.
#[derive(Debug)]
pub struct BudgetSettlement {
	policy: BudgetPolicy,
	budgets: MatchedBudgets,
}

#[derive(Debug, thiserror::Error)]
#[error("Budget exceeded")]
pub struct BudgetExceeded {
	/// Seconds until the budget resets, or `None` for a budget that never resets.
	pub retry_after: Option<u64>,
}

fn budget_id(api_key_id: &str, budget: &Budget) -> String {
	format!(
		"api-key:{}:{}:{}:{}",
		api_key_id.len(),
		api_key_id,
		budget.name.len(),
		budget.name
	)
}

/// Returns the half-open fixed window `[start, end)` containing `now`.
///
/// Windows are anchored at the Unix epoch and repeat at exact `rolling` intervals.
/// For example, a one-hour duration produces UTC clock-hour windows, while a 30-day duration
/// produces consecutive 30-day periods measured from 1970-01-01 rather than calendar months.
/// UTC and fixed durations deliberately avoid daylight-saving and other local-calendar behavior.
/// [`UNBOUNDED_WINDOW`] spans from the Unix epoch to the latest representable time.
fn budget_window(now: UnixDate, rolling: Duration) -> anyhow::Result<(UnixDate, UnixDate)> {
	let unbounded = rolling == UNBOUNDED_WINDOW;
	let rolling = TimeDelta::from_std(rolling).context("budget rolling window is too large")?;
	let start = if unbounded {
		UnixDate::UNIX_EPOCH
	} else {
		now
			.duration_trunc(rolling)
			.context("failed to align budget rolling window")?
	};
	let end = start
		.checked_add_signed(rolling)
		.context("budget rolling window is too large")?;
	Ok((start, end))
}

/// Window length of budgets that never reset. It is whole milliseconds so the window end
/// round-trips through the database unchanged.
const UNBOUNDED_WINDOW: Duration =
	Duration::from_millis(UnixDate::MAX_UTC.timestamp_millis() as u64);

impl BudgetPolicy {
	/// Creates a policy used while normalizing a candidate local configuration. It shares the live
	/// counters and database handles, but records definitions separately until the candidate wins.
	pub(crate) fn registration_policy(&self) -> Self {
		Self {
			counters: self.counters.clone(),
			database: self.database.clone(),
			flush_lock: self.flush_lock.clone(),
			registration: Some(Arc::new(DashMap::new())),
			legacy_rows: self.legacy_rows.clone(),
		}
	}

	pub(crate) fn registration(&self) -> BudgetRegistration {
		BudgetRegistration(
			self
				.registration
				.as_ref()
				.expect("registration policy")
				.iter()
				.map(|definition| (definition.key().clone(), definition.value().clone()))
				.collect(),
		)
	}

	/// Replaces the complete configured definition set after a local configuration reload succeeds.
	/// Counters without a definition are retained so persisted usage can be reattached later.
	pub(crate) fn apply_registration(&self, registration: BudgetRegistration) -> anyhow::Result<()> {
		let now = Utc::now();
		for (budget_id, definition) in &registration.0 {
			self.configure_counter(budget_id, definition, now)?;
		}
		for mut counter in self.counters.iter_mut() {
			if !registration.0.contains_key(counter.key()) {
				counter.definition = None;
			}
		}
		Ok(())
	}

	/// Registers every configured API key budget in memory. A compatible preloaded database row is
	/// retained; otherwise the counter starts in the current epoch-aligned window.
	pub fn register(
		&self,
		authentication: &crate::http::apikey::APIKeyAuthentication,
		database_configured: bool,
	) -> anyhow::Result<()> {
		let now = Utc::now();
		let has_budgets = authentication
			.users
			.values()
			.any(|policy| policy.budgets.is_some());
		anyhow::ensure!(
			!has_budgets || self.database.get().is_some() || database_configured,
			"API key budgets require config.database to be configured"
		);
		for (key_hash, policy) in authentication.users.iter() {
			let Some(budgets) = policy.budgets.as_ref() else {
				continue;
			};
			for budget in &budgets.budgets {
				let budget_id = budget_id(&budgets.api_key_id, budget);
				let definition = BudgetDefinition {
					api_key: budgets.api_key.clone(),
					budget: budget.clone(),
					// Releases before the key id was used keyed budgets by the key hash.
					legacy_id: (budgets.api_key_id != key_hash.as_str())
						.then(|| self::budget_id(key_hash.as_str(), budget)),
				};
				if let Some(registration) = &self.registration {
					BudgetCounter::configured(&definition, now)?;
					registration.insert(budget_id, definition);
					continue;
				}
				self.configure_counter(&budget_id, &definition, now)?;
			}
		}
		Ok(())
	}

	/// Attaches a definition to its counter, creating the counter if needed.
	fn configure_counter(
		&self,
		budget_id: &str,
		definition: &BudgetDefinition,
		now: UnixDate,
	) -> anyhow::Result<()> {
		match self.counters.entry(budget_id.to_owned()) {
			Entry::Occupied(mut entry) => entry.get_mut().configure(definition, now)?,
			Entry::Vacant(entry) => {
				entry.insert(BudgetCounter::configured(definition, now)?);
			},
		}
		if let Some(legacy_id) = &definition.legacy_id {
			self.absorb_legacy_counter(legacy_id, budget_id, now);
		}
		Ok(())
	}

	/// TODO: temporary migration from key-hash budget ids, see [`Self::legacy_rows`].
	///
	/// A legacy row is never a counter of its own. Rows preloaded before their alias was known are
	/// folded into the current counter here, which also removes them, so a later reload cannot
	/// count them twice. Rows read once the alias is known are folded by `reconcile` instead.
	fn absorb_legacy_counter(&self, legacy_id: &str, budget_id: &str, now: UnixDate) {
		let Some((_, legacy)) = self
			.counters
			.remove_if(legacy_id, |_, counter| counter.definition.is_none())
		else {
			return;
		};
		if let Some(mut counter) = self.counters.get_mut(budget_id) {
			counter.refresh(now);
			counter.absorb(
				legacy.unit,
				legacy.window_start,
				legacy.window_end,
				legacy.amount,
				legacy.updated_at,
			);
		}
		self
			.legacy_rows
			.insert(legacy_id.to_owned(), budget_id.to_owned());
	}

	/// Refreshes each matched counter before a request, logs exceeded budgets, and returns the first
	/// exceeded budget configured to block. Audit-only budgets never block the request.
	fn check(&self, budgets: &MatchedBudgets) -> anyhow::Result<Option<BudgetExceeded>> {
		let now = Utc::now();
		let mut blocked = None;
		for budget in &budgets.budgets {
			let budget_id = budget_id(&budgets.api_key_id, budget);
			let (used, window_end, rolling) = {
				let mut counter = self
					.counters
					.get_mut(&budget_id)
					.context("budget counter was not registered")?;
				counter.refresh(now);
				(counter.amount, counter.window_end, counter.rolling)
			};
			let exceeded = used >= budget.limit.amount.decimal();
			if exceeded {
				tracing::warn!(
					target: "budget",
					api_key = budgets.api_key,
					budget = budget.name,
					used = %used,
					limit_unit = budget.limit.unit.as_str(),
					limit_amount = %budget.limit.amount,
					exceeded,
					"API key budget exceeded"
				);
			} else {
				tracing::debug!(
					target: "budget",
					api_key = budgets.api_key,
					budget = budget.name,
					used = %used,
					limit_unit = budget.limit.unit.as_str(),
					limit_amount = %budget.limit.amount,
					exceeded,
					"API key budget checked"
				);
			}

			if exceeded
				&& matches!(budget.on_budget_exceeded, BudgetExceededAction::Block)
				&& blocked.is_none()
			{
				// Retry-After is whole seconds, rounded up from the remaining window duration.
				let retry_after = (rolling != UNBOUNDED_WINDOW).then(|| {
					let remaining = (window_end - now).to_std().unwrap_or_default();
					remaining
						.as_secs()
						.saturating_add(u64::from(remaining.subsec_nanos() != 0))
				});
				blocked = Some(BudgetExceeded { retry_after });
			}
		}
		Ok(blocked)
	}

	/// Charges completed response cost or tokens to each in-memory counter and its pending database
	/// delta. A counter is advanced first if the request crossed a window boundary.
	fn settle(&self, budgets: &MatchedBudgets, response: &LLMContext) {
		let now = Utc::now();
		for budget in &budgets.budgets {
			let charged = match budget.limit.unit {
				BudgetLimitUnit::Usd => response.cost.as_ref().map(|cost| cost.total()),
				BudgetLimitUnit::Tokens => response.total_tokens.map(Decimal::from),
			};
			let Some(charged) = charged else {
				tracing::debug!(
					target: "budget",
					api_key = budgets.api_key,
					budget = budget.name,
					limit_unit = budget.limit.unit.as_str(),
					"API key budget could not be charged because usage was unavailable"
				);
				continue;
			};

			let budget_id = budget_id(&budgets.api_key_id, budget);
			let Some(mut counter) = self.counters.get_mut(&budget_id) else {
				tracing::warn!(target: "budget", budget_id, "budget counter was not registered before settlement");
				continue;
			};
			counter.refresh(now);
			counter.amount += charged;
			counter.pending += charged;
			counter.updated_at = now;
			let used = counter.amount;
			drop(counter);

			tracing::debug!(
				target: "budget",
				api_key = budgets.api_key,
				budget = budget.name,
				charged = %charged,
				used = %used,
				limit_unit = budget.limit.unit.as_str(),
				limit_amount = %budget.limit.amount,
				"API key budget charged"
			);
		}
	}
}

impl BudgetSettlement {
	pub fn settle(self, response: &LLMContext) {
		self.policy.settle(&self.budgets, response);
	}
}

impl crate::store::RequestPolicyTrait for BudgetPolicy {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, ProxyError> {
		let Some(budgets) = req.extensions_mut().remove::<MatchedBudgets>() else {
			return Ok(crate::http::PolicyResponse::default());
		};

		if let Some(exceeded) = self
			.check(&budgets)
			.map_err(crate::proxy::ProxyError::Processing)?
		{
			return Err(crate::proxy::ProxyError::BudgetExceeded(exceeded));
		}

		log.budgets.push(BudgetSettlement {
			policy: self.clone(),
			budgets,
		});
		Ok(crate::http::PolicyResponse::default())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn definition(api_key: &str, budget: &Budget) -> BudgetDefinition {
		BudgetDefinition {
			api_key: api_key.to_owned(),
			budget: budget.clone(),
			legacy_id: None,
		}
	}

	#[test]
	fn budgets_require_a_database() {
		let keys: crate::http::apikey::LocalAPIKeys = serde_json::from_value(serde_json::json!({
			"keys": [{
				"key": "sk-budget",
				"metadata": {"name": "budgeted-key"},
				"budgets": [{
					"name": "tokens",
					"limit": {"unit": "Tokens", "amount": 40},
					"window": {"rolling": "1h"},
					"onBudgetExceeded": "Block"
				}]
			}]
		}))
		.unwrap();
		let authentication = keys.compile().unwrap();
		let err = BudgetPolicy::default()
			.register(&authentication, false)
			.unwrap_err();
		assert_eq!(
			err.to_string(),
			"API key budgets require config.database to be configured"
		);
	}

	#[test]
	fn budgets_follow_the_key_id_when_the_key_value_changes() {
		let budget_id_for = |key: &str, metadata: serde_json::Value| {
			let keys: crate::http::apikey::LocalAPIKeys = serde_json::from_value(serde_json::json!({
				"keys": [{
					"key": key,
					"metadata": metadata,
					"budgets": [{
						"name": "daily",
						"limit": {"unit": "USD", "amount": 10},
						"onBudgetExceeded": "Block"
					}]
				}]
			}))
			.unwrap();
			let authentication = keys.compile().unwrap();
			let budgets = authentication
				.users
				.values()
				.next()
				.unwrap()
				.budgets
				.clone()
				.unwrap();
			budget_id(&budgets.api_key_id, &budgets.budgets[0])
		};
		let with_id = serde_json::json!({"name": "team", "agentgateway.dev/id": "key-id"});
		assert_eq!(
			budget_id_for("sk-old", with_id.clone()),
			budget_id_for("sk-new", with_id)
		);
		let without_id = serde_json::json!({"name": "team"});
		assert_ne!(
			budget_id_for("sk-old", without_id.clone()),
			budget_id_for("sk-new", without_id)
		);
	}

	#[test]
	fn registration_replaces_definitions_only_when_applied() {
		let current: crate::http::apikey::LocalAPIKeys = serde_json::from_value(serde_json::json!({
			"keys": [{
				"key": "sk-budget",
				"metadata": {"name": "budgeted-key"},
				"budgets": [{
					"name": "old",
					"limit": {"unit": "Tokens", "amount": 40},
					"window": {"rolling": "1h"},
					"onBudgetExceeded": "Block"
				}]
			}]
		}))
		.unwrap();
		let replacement: crate::http::apikey::LocalAPIKeys =
			serde_json::from_value(serde_json::json!({
				"keys": [{
					"key": "sk-budget",
					"metadata": {"name": "budgeted-key"},
					"budgets": [{
						"name": "new",
						"limit": {"unit": "Tokens", "amount": 80},
						"window": {"rolling": "1h"},
						"onBudgetExceeded": "Audit"
					}]
				}]
			}))
			.unwrap();
		let policy = BudgetPolicy::default();
		policy.register(&current.compile().unwrap(), true).unwrap();

		let candidate = policy.registration_policy();
		candidate
			.register(&replacement.compile().unwrap(), true)
			.unwrap();
		assert_eq!(policy.status(None).unwrap().budgets[0].name, "old");

		policy.apply_registration(candidate.registration()).unwrap();
		let status = policy.status(None).unwrap();
		assert_eq!(status.budgets.len(), 1);
		assert_eq!(status.budgets[0].name, "new");
		assert_eq!(policy.counters.len(), 2);
	}

	#[tokio::test]
	async fn flushes_only_new_usage_with_atomic_increments() {
		let pool = sqlx::sqlite::SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await
			.unwrap();
		sqlx::raw_sql(
			r#"
CREATE TABLE budget_usage (
    budget_id TEXT PRIMARY KEY,
    window_start INTEGER NOT NULL,
    window_end INTEGER NOT NULL,
    used_amount INTEGER NOT NULL DEFAULT 0 CHECK (used_amount >= 0),
    updated_at INTEGER NOT NULL
);
"#,
		)
		.execute(&pool)
		.await
		.unwrap();
		let first = Arc::new(BudgetPolicy::default());
		let second = Arc::new(BudgetPolicy::default());
		first
			.initialize(crate::database::DatabasePool::Sqlite(pool.clone()))
			.await
			.unwrap();
		second
			.initialize(crate::database::DatabasePool::Sqlite(pool.clone()))
			.await
			.unwrap();
		let budget = Budget {
			name: "window".to_string(),
			limit: BudgetLimit {
				unit: BudgetLimitUnit::Tokens,
				amount: BudgetAmount(Decimal::from(100)),
			},
			window: Some(BudgetWindow {
				rolling: Duration::from_secs(60 * 60),
			}),
			on_budget_exceeded: BudgetExceededAction::Block,
		};
		let now = Utc::now();
		let (window_start, window_end) = budget_window(now, Duration::from_secs(60 * 60)).unwrap();
		sqlx::query(
			"INSERT INTO budget_usage (budget_id, window_start, window_end, unit, used_amount, updated_at) VALUES ('preloaded', ?, ?, 'Tokens', 9, ?), ('expired', 0, 1, 'Tokens', 8, 0)",
		)
		.bind(window_start.timestamp_millis())
		.bind(window_end.timestamp_millis())
		.bind(now.timestamp_millis())
		.execute(&pool)
		.await
		.unwrap();
		let preloaded = Arc::new(BudgetPolicy::default());
		preloaded.counters.insert(
			"preloaded".to_string(),
			BudgetCounter::configured(&definition("key", &budget), now).unwrap(),
		);
		preloaded
			.initialize(crate::database::DatabasePool::Sqlite(pool.clone()))
			.await
			.unwrap();
		assert_eq!(
			preloaded.counters.get("preloaded").unwrap().amount,
			Decimal::from(9),
		);
		let expired =
			sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM budget_usage WHERE budget_id = 'expired'")
				.fetch_one(&pool)
				.await
				.unwrap();
		assert_eq!(expired, 0);
		first.counters.insert(
			"window".to_string(),
			BudgetCounter::configured(&definition("key", &budget), now).unwrap(),
		);
		second.counters.insert(
			"window".to_string(),
			BudgetCounter::configured(&definition("key", &budget), now).unwrap(),
		);
		assert_eq!(
			first.counters.get("window").unwrap().window_start,
			second.counters.get("window").unwrap().window_start,
		);

		{
			let mut counter = first.counters.get_mut("window").unwrap();
			counter.amount = Decimal::from(3);
			counter.pending = Decimal::from(3);
		}
		{
			let mut counter = second.counters.get_mut("window").unwrap();
			counter.amount = Decimal::from(4);
			counter.pending = Decimal::from(4);
		}

		first.flush().await.unwrap();
		second.flush().await.unwrap();
		first.flush().await.unwrap();
		assert_eq!(
			first.counters.get("window").unwrap().amount,
			Decimal::from(7),
		);

		let used = sqlx::query_scalar::<_, i64>(
			"SELECT used_amount FROM budget_usage WHERE budget_id = 'window'",
		)
		.fetch_one(&pool)
		.await
		.unwrap();
		assert_eq!(used, 7);

		let usd_budget = Budget {
			name: "expired-residue".to_string(),
			limit: BudgetLimit {
				unit: BudgetLimitUnit::Usd,
				amount: BudgetAmount(Decimal::ONE),
			},
			window: Some(BudgetWindow {
				rolling: Duration::from_secs(60 * 60),
			}),
			on_budget_exceeded: BudgetExceededAction::Block,
		};
		let mut expired_residue =
			BudgetCounter::configured(&definition("key", &usd_budget), now).unwrap();
		expired_residue.amount = Decimal::new(1, 10);
		expired_residue.pending = Decimal::new(1, 10);
		expired_residue.window_start = UnixDate::from_timestamp_millis(0).unwrap();
		expired_residue.window_end = now - chrono::TimeDelta::milliseconds(1);
		first
			.counters
			.insert("expired-residue".to_string(), expired_residue);
		first.flush().await.unwrap();
		assert!(
			first
				.counters
				.get("expired-residue")
				.unwrap()
				.pending
				.is_zero()
		);
	}

	#[test]
	fn duplicate_key_ids_are_rejected_on_budgeted_keys() {
		let compile = |second_budgets: serde_json::Value| {
			let keys: crate::http::apikey::LocalAPIKeys = serde_json::from_value(serde_json::json!({
				"keys": [{
					"key": "sk-first",
					"metadata": {"name": "first", "agentgateway.dev/id": "shared"},
					"budgets": [{
						"name": "daily",
						"limit": {"unit": "Tokens", "amount": 10},
						"onBudgetExceeded": "Block"
					}]
				}, {
					"key": "sk-second",
					"metadata": {"name": "second", "agentgateway.dev/id": "shared"},
					"budgets": second_budgets
				}]
			}))
			.unwrap();
			keys.compile()
		};
		let err = compile(serde_json::json!([{
			"name": "daily",
			"limit": {"unit": "Tokens", "amount": 10},
			"onBudgetExceeded": "Block"
		}]))
		.err()
		.unwrap();
		assert_eq!(
			err.to_string(),
			"duplicate agentgateway.dev/id \"shared\" on API keys with budgets"
		);
		assert!(compile(serde_json::json!([])).is_ok());
	}

	/// Usage persisted by a release that keyed budgets by the key hash, under `legacy_id`.
	struct LegacyFixture {
		pool: sqlx::SqlitePool,
		authentication: crate::http::apikey::APIKeyAuthentication,
		budget_id: String,
		legacy_id: String,
		window_start: i64,
		window_end: i64,
	}

	impl LegacyFixture {
		async fn new() -> Self {
			let pool = sqlx::sqlite::SqlitePoolOptions::new()
				.max_connections(1)
				.connect("sqlite::memory:")
				.await
				.unwrap();
			let keys: crate::http::apikey::LocalAPIKeys = serde_json::from_value(serde_json::json!({
				"keys": [{
					"key": "sk-upgraded",
					"metadata": {"name": "upgraded", "agentgateway.dev/id": "upgraded-id"},
					"budgets": [{
						"name": "lifetime",
						"limit": {"unit": "Tokens", "amount": 100},
						"onBudgetExceeded": "Block"
					}]
				}]
			}))
			.unwrap();
			let authentication = keys.compile().unwrap();
			let (key_hash, policy) = authentication.users.iter().next().unwrap();
			let budgets = policy.budgets.clone().unwrap();
			let budget_id = budget_id(&budgets.api_key_id, &budgets.budgets[0]);
			let legacy_id = super::budget_id(key_hash.as_str(), &budgets.budgets[0]);
			assert_ne!(budget_id, legacy_id);
			// Creates the schema.
			Arc::new(BudgetPolicy::default())
				.initialize(crate::database::DatabasePool::Sqlite(pool.clone()))
				.await
				.unwrap();
			let (window_start, window_end) = budget_window(Utc::now(), UNBOUNDED_WINDOW).unwrap();
			Self {
				pool,
				authentication,
				budget_id,
				legacy_id,
				window_start: window_start.timestamp_millis(),
				window_end: window_end.timestamp_millis(),
			}
		}

		async fn write(&self, budget_id: &str, used_amount: i64) {
			sqlx::query(
				"INSERT INTO budget_usage (budget_id, window_start, window_end, unit, used_amount, updated_at) VALUES (?, ?, ?, 'Tokens', ?, 0) \
				ON CONFLICT(budget_id) DO UPDATE SET used_amount = budget_usage.used_amount + excluded.used_amount",
			)
			.bind(budget_id)
			.bind(self.window_start)
			.bind(self.window_end)
			.bind(used_amount)
			.execute(&self.pool)
			.await
			.unwrap();
		}

		async fn rows(&self) -> Vec<(String, i64)> {
			sqlx::query_as("SELECT budget_id, used_amount FROM budget_usage ORDER BY budget_id")
				.fetch_all(&self.pool)
				.await
				.unwrap()
		}

		async fn start(&self) -> Arc<BudgetPolicy> {
			let policy = Arc::new(BudgetPolicy::default());
			policy
				.initialize(crate::database::DatabasePool::Sqlite(self.pool.clone()))
				.await
				.unwrap();
			policy
		}

		/// Loads the configuration the way a reload does.
		fn reload(&self, policy: &BudgetPolicy) {
			let candidate = policy.registration_policy();
			candidate.register(&self.authentication, true).unwrap();
			policy.apply_registration(candidate.registration()).unwrap();
		}

		fn used(&self, policy: &BudgetPolicy) -> Decimal {
			policy.counters.get(&self.budget_id).unwrap().amount
		}
	}

	#[tokio::test]
	async fn legacy_usage_counts_as_soon_as_the_key_is_registered() {
		let fixture = LegacyFixture::new().await;
		fixture.write(&fixture.legacy_id, 7).await;
		// Another upgraded replica already wrote under the key id.
		fixture.write(&fixture.budget_id, 1).await;

		let policy = fixture.start().await;
		fixture.reload(&policy);
		assert_eq!(fixture.used(&policy), Decimal::from(8));
		assert!(!policy.counters.contains_key(&fixture.legacy_id));
		// A reload before the next flush does not count the legacy usage again.
		fixture.reload(&policy);
		assert_eq!(fixture.used(&policy), Decimal::from(8));

		{
			let mut counter = policy.counters.get_mut(&fixture.budget_id).unwrap();
			counter.amount += Decimal::from(2);
			counter.pending += Decimal::from(2);
		}
		policy.flush().await.unwrap();
		assert_eq!(fixture.rows().await, vec![(fixture.budget_id.clone(), 10)]);
		assert_eq!(fixture.used(&policy), Decimal::from(10));
		assert!(policy.legacy_rows.is_empty());

		// Without legacy rows, reloads and flushes have nothing to migrate.
		fixture.reload(&policy);
		assert!(policy.legacy_rows.is_empty());
		policy.flush().await.unwrap();
		assert_eq!(fixture.rows().await, vec![(fixture.budget_id.clone(), 10)]);
		assert_eq!(fixture.used(&policy), Decimal::from(10));
	}

	#[tokio::test]
	async fn legacy_usage_written_after_the_upgrade_is_merged() {
		let fixture = LegacyFixture::new().await;
		let policy = fixture.start().await;
		fixture.reload(&policy);
		fixture.write(&fixture.budget_id, 5).await;
		policy.flush().await.unwrap();
		assert!(policy.legacy_rows.is_empty());

		// A replica still running the previous release charges the key under its hash.
		fixture.write(&fixture.legacy_id, 3).await;
		policy.flush().await.unwrap();
		assert_eq!(fixture.used(&policy), Decimal::from(8));
		assert!(!policy.counters.contains_key(&fixture.legacy_id));
		policy.flush().await.unwrap();
		assert_eq!(fixture.rows().await, vec![(fixture.budget_id.clone(), 8)]);
		assert_eq!(fixture.used(&policy), Decimal::from(8));
		assert!(policy.legacy_rows.is_empty());
	}

	#[tokio::test]
	async fn budgets_without_a_window_never_reset() {
		let pool = sqlx::sqlite::SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await
			.unwrap();
		let keys: crate::http::apikey::LocalAPIKeys = serde_json::from_value(serde_json::json!({
			"keys": [{
				"key": "sk-budget",
				"metadata": {"name": "budgeted-key"},
				"budgets": [{
					"name": "lifetime",
					"limit": {"unit": "Tokens", "amount": 10},
					"onBudgetExceeded": "Block"
				}]
			}]
		}))
		.unwrap();
		let authentication = keys.compile().unwrap();
		let budgets = authentication
			.users
			.values()
			.find_map(|policy| policy.budgets.clone())
			.unwrap();
		let budget_id = budget_id(&budgets.api_key_id, &budgets.budgets[0]);
		let policy = Arc::new(BudgetPolicy::default());
		policy
			.initialize(crate::database::DatabasePool::Sqlite(pool.clone()))
			.await
			.unwrap();
		policy.register(&authentication, true).unwrap();
		{
			let mut counter = policy.counters.get_mut(&budget_id).unwrap();
			counter.amount = Decimal::from(10);
			counter.pending = Decimal::from(10);
		}
		policy.flush().await.unwrap();

		// A fresh process preloads the persisted lifetime usage.
		let restarted = Arc::new(BudgetPolicy::default());
		restarted
			.initialize(crate::database::DatabasePool::Sqlite(pool.clone()))
			.await
			.unwrap();
		restarted.register(&authentication, true).unwrap();
		assert_eq!(
			restarted.counters.get(&budget_id).unwrap().amount,
			Decimal::from(10)
		);

		// Switching to a rolling window replaces the lifetime row instead of being rejected as an
		// older window.
		let mut rolling = budgets.budgets[0].clone();
		rolling.window = Some(BudgetWindow {
			rolling: Duration::from_secs(60 * 60),
		});
		{
			let mut counter = restarted.counters.get_mut(&budget_id).unwrap();
			counter
				.configure(&definition(&budgets.api_key, &rolling), Utc::now())
				.unwrap();
			assert_eq!(counter.amount, Decimal::ZERO);
			counter.amount = Decimal::from(3);
			counter.pending = Decimal::from(3);
		}
		restarted.flush().await.unwrap();
		assert_eq!(
			restarted.counters.get(&budget_id).unwrap().amount,
			Decimal::from(3)
		);
	}
}
