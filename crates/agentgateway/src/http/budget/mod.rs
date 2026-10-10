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
	rolling: Duration,
	window_start: UnixDate,
	window_end: UnixDate,
	updated_at: UnixDate,
}

impl BudgetCounter {
	fn configured(api_key: &str, budget: &Budget, now: UnixDate) -> anyhow::Result<Self> {
		let rolling = budget.window.rolling;
		anyhow::ensure!(
			!rolling.is_zero(),
			"budget rolling window must be greater than zero"
		);
		let (window_start, window_end) = budget_window(now, rolling)?;
		Ok(Self {
			definition: Some(BudgetDefinition {
				api_key: api_key.to_owned(),
				budget: budget.clone(),
			}),
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
	fn configure(&mut self, api_key: &str, budget: &Budget, now: UnixDate) -> anyhow::Result<()> {
		let rolling = budget.window.rolling;
		anyhow::ensure!(
			!rolling.is_zero(),
			"budget rolling window must be greater than zero"
		);
		if now >= self.window_end || self.rolling != rolling || self.unit != Some(budget.limit.unit) {
			(self.window_start, self.window_end) = budget_window(now, rolling)?;
			self.amount = Decimal::ZERO;
			self.pending = Decimal::ZERO;
			self.unit = Some(budget.limit.unit);
			self.updated_at = now;
		}
		self.rolling = rolling;
		self.definition = Some(BudgetDefinition {
			api_key: api_key.to_owned(),
			budget: budget.clone(),
		});
		Ok(())
	}

	/// Advances an expired counter to the epoch-aligned fixed window containing `now`.
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
	/// Rolling window over which usage will be accumulated.
	pub window: BudgetWindow,
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
	pub retry_after: u64,
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
fn budget_window(now: UnixDate, rolling: Duration) -> anyhow::Result<(UnixDate, UnixDate)> {
	let rolling = TimeDelta::from_std(rolling).context("budget rolling window is too large")?;
	let start = now
		.duration_trunc(rolling)
		.context("failed to align budget rolling window")?;
	let end = start
		.checked_add_signed(rolling)
		.context("budget rolling window is too large")?;
	Ok((start, end))
}

impl BudgetPolicy {
	/// Creates a policy used while normalizing a candidate local configuration. It shares the live
	/// counters and database handles, but records definitions separately until the candidate wins.
	pub(crate) fn registration_policy(&self) -> Self {
		Self {
			counters: self.counters.clone(),
			database: self.database.clone(),
			flush_lock: self.flush_lock.clone(),
			registration: Some(Arc::new(DashMap::new())),
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
			match self.counters.entry(budget_id.clone()) {
				Entry::Occupied(mut entry) => {
					entry
						.get_mut()
						.configure(&definition.api_key, &definition.budget, now)?
				},
				Entry::Vacant(entry) => {
					entry.insert(BudgetCounter::configured(
						&definition.api_key,
						&definition.budget,
						now,
					)?);
				},
			}
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
		for policy in authentication.users.values() {
			let Some(budgets) = policy.budgets.as_ref() else {
				continue;
			};
			for budget in &budgets.budgets {
				let budget_id = budget_id(&budgets.api_key_id, budget);
				if let Some(registration) = &self.registration {
					BudgetCounter::configured(&budgets.api_key, budget, now)?;
					registration.insert(
						budget_id,
						BudgetDefinition {
							api_key: budgets.api_key.clone(),
							budget: budget.clone(),
						},
					);
					continue;
				}
				match self.counters.entry(budget_id) {
					Entry::Occupied(mut entry) => {
						entry.get_mut().configure(&budgets.api_key, budget, now)?;
					},
					Entry::Vacant(entry) => {
						entry.insert(BudgetCounter::configured(&budgets.api_key, budget, now)?);
					},
				}
			}
		}
		Ok(())
	}

	/// Refreshes each matched counter before a request, logs exceeded budgets, and returns the first
	/// exceeded budget configured to block. Audit-only budgets never block the request.
	fn check(&self, budgets: &MatchedBudgets) -> anyhow::Result<Option<BudgetExceeded>> {
		let now = Utc::now();
		let mut blocked = None;
		for budget in &budgets.budgets {
			let budget_id = budget_id(&budgets.api_key_id, budget);
			let (used, window_end) = {
				let mut counter = self
					.counters
					.get_mut(&budget_id)
					.context("budget counter was not registered")?;
				counter.refresh(now);
				(counter.amount, counter.window_end)
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
				let retry_after = (window_end - now).to_std().unwrap_or_default();
				blocked = Some(BudgetExceeded {
					// Retry-After is whole seconds, rounded up from the remaining window duration.
					retry_after: retry_after
						.as_secs()
						.saturating_add(u64::from(retry_after.subsec_nanos() != 0)),
				});
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
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		let Some(budgets) = req.extensions_mut().remove::<MatchedBudgets>() else {
			return Ok(crate::http::PolicyResponse::default());
		};

		if let Some(exceeded) = self
			.check(&budgets)
			.map_err(|err| crate::proxy::ProxyResponse::from(crate::proxy::ProxyError::Processing(err)))?
		{
			return Err(crate::proxy::ProxyError::BudgetExceeded(exceeded).into());
		}

		log.budgets = Some(BudgetSettlement {
			policy: self.clone(),
			budgets,
		});
		Ok(crate::http::PolicyResponse::default())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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
			window: BudgetWindow {
				rolling: Duration::from_secs(60 * 60),
			},
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
			BudgetCounter::configured("key", &budget, now).unwrap(),
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
			BudgetCounter::configured("key", &budget, now).unwrap(),
		);
		second.counters.insert(
			"window".to_string(),
			BudgetCounter::configured("key", &budget, now).unwrap(),
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
			window: BudgetWindow {
				rolling: Duration::from_secs(60 * 60),
			},
			on_budget_exceeded: BudgetExceededAction::Block,
		};
		let mut expired_residue = BudgetCounter::configured("key", &usd_budget, now).unwrap();
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
}
