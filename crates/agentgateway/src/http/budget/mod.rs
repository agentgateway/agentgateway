use std::collections::{HashMap, HashSet};
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
use crate::types::local::validate_budgets;
use crate::{apply, schema_de, serde_dur};

mod database;
mod status;

pub use status::{
	BudgetStatus, BudgetStatusLimit, BudgetStatusResponse, BudgetStatusScope, BudgetStatusUsage,
	BudgetStatusWindow,
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
	fn configured(scope: &ResolvedScope, budget: &Budget, now: UnixDate) -> anyhow::Result<Self> {
		let rolling = budget.window.rolling;
		anyhow::ensure!(
			!rolling.is_zero(),
			"budget rolling window must be greater than zero"
		);
		let (window_start, window_end) = budget_window(now, rolling)?;
		Ok(Self {
			definition: Some(BudgetDefinition {
				scope: scope.clone(),
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
	fn configure(
		&mut self,
		scope: &ResolvedScope,
		budget: &Budget,
		now: UnixDate,
	) -> anyhow::Result<()> {
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
			scope: scope.clone(),
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
	scope: ResolvedScope,
	budget: Budget,
}

/// A named budget attached to a standalone API key.
///
/// Usage is charged after an LLM response when the provider reports the tokens or cost required by
/// the configured unit. Requests with unavailable usage are logged but cannot be charged or blocked
/// retroactively.
#[apply(schema_de!)]
pub struct Budget {
	/// Stable name for this budget within the API key or configuration that declares it. The name
	/// identifies the counter that accumulates usage, so renaming a budget starts a new one.
	pub name: String,
	/// Maximum usage allowed during the window.
	pub limit: BudgetLimit,
	/// Rolling window over which usage will be accumulated.
	pub window: BudgetWindow,
	/// Action taken when the budget is exceeded.
	pub on_budget_exceeded: BudgetExceededAction,
	/// Which API keys share this budget's counter. Defaults to one counter per key.
	#[serde(default)]
	pub scope: BudgetScope,
}

#[apply(schema_de!)]
#[derive(Default)]
pub enum BudgetScope {
	/// One counter per API key, identified by the key itself.
	#[default]
	PerKey,
	/// One counter per distinct value of this metadata field.
	GroupBy(String),
	/// One counter shared by every key whose metadata matches all of these fields.
	Selector(HashMap<String, String>),
}

#[derive(Debug, Clone)]
pub enum ResolvedScope {
	PerKey { api_key_id: String, api_key: String }, // hash for the id, name for status
	GroupBy { field: String, value: String },
	Selector, // name comes from the Budget
}

impl std::fmt::Display for ResolvedScope {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::PerKey { api_key, .. } => write!(f, "api-key {api_key}"),
			Self::GroupBy { field, value } => write!(f, "{field}={value}"),
			Self::Selector => f.write_str("selector"),
		}
	}
}

/// Renders an API key metadata field as a budget scope key.
///
/// Scalars are coerced to their display form so that unquoted YAML such as `tier: 1` scopes the
/// same way as `tier: "1"`. Silently leaving such a key unbudgeted would be a worse outcome than
/// coercing. Values with no meaningful scope identity yield `None`, and the key is simply not
/// budgeted by this scope.
fn metadata_scope_value(metadata: &serde_json::Value, field: &str) -> Option<String> {
	match metadata.get(field)? {
		serde_json::Value::String(value) => Some(value.clone()),
		serde_json::Value::Number(value) => Some(value.to_string()),
		serde_json::Value::Bool(value) => Some(value.to_string()),
		other => {
			tracing::warn!(
				target: "budget",
				field,
				kind = match other {
					serde_json::Value::Null => "null",
					serde_json::Value::Array(_) => "array",
					_ => "object",
				},
				"API key metadata field cannot scope a budget"
			);
			None
		},
	}
}

fn metadata_matches(selector: &HashMap<String, String>, metadata: &serde_json::Value) -> bool {
	selector.iter().all(|(field, expected)| {
		metadata_scope_value(metadata, field).is_some_and(|value| &value == expected)
	})
}

/// Resolves every budget that applies to one API key, pairing each with the counter it shares.
///
/// Document-level budgets are matched against the key's metadata; budgets declared inline on the
/// key always apply. Returns `None` when no budget applies, leaving the key unbudgeted.
pub fn resolve_budgets(
	specs: &[Budget],
	inline: &[Budget],
	api_key_id: &str,
	metadata: &serde_json::Value,
) -> anyhow::Result<Option<MatchedBudgets>> {
	let api_key = metadata
		.get("name")
		.and_then(serde_json::Value::as_str)
		.filter(|name| !name.is_empty());

		
	validate_budgets(inline,"api-key")?;
	let mut budgets = Vec::new();
	let mut counters = HashSet::new();
	for budget in inline.iter().chain(specs) {
		let Some(scope) = resolve_scope(budget, api_key_id, api_key, metadata)? else {
			continue;
		};
		let budget_id = budget_id(&scope, budget);
		anyhow::ensure!(
			counters.insert(budget_id.clone()),
			"budget {:?} shares a counter with another budget applying to API key {:?}",
			budget.name,
			api_key.unwrap_or_default(),
		);
		budgets.push(MatchedBudget {
			budget_id,
			scope,
			budget: budget.clone(),
		});
	}

	if budgets.is_empty() {
		return Ok(None);
	}
	Ok(Some(MatchedBudgets {
		api_key: api_key.unwrap_or_default().to_owned(),
		budgets,
	}))
}

/// Resolves one budget's scope against an API key, or `None` when it does not apply to that key.
fn resolve_scope(
	budget: &Budget,
	api_key_id: &str,
	api_key: Option<&str>,
	metadata: &serde_json::Value,
) -> anyhow::Result<Option<ResolvedScope>> {
	Ok(match &budget.scope {
		BudgetScope::PerKey => Some(ResolvedScope::PerKey {
			api_key_id: api_key_id.to_owned(),
			api_key: api_key
				.context("API keys with per-key budgets must have a metadata.name")?
				.to_owned(),
		}),
		BudgetScope::GroupBy(field) => {
			metadata_scope_value(metadata, field).map(|value| ResolvedScope::GroupBy {
				field: field.clone(),
				value,
			})
		},
		BudgetScope::Selector(selector) => {
			metadata_matches(selector, metadata).then_some(ResolvedScope::Selector)
		},
	})
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

/// Every budget that applies to one authenticated API key, resolved when the API key policy was
/// compiled. Attached to the request so the budget policy can charge without touching metadata.
#[derive(Debug, Clone)]
pub struct MatchedBudgets {
	/// Display name of the API key, used only for logging.
	pub(crate) api_key: String,
	pub(crate) budgets: Vec<MatchedBudget>,
}

/// One budget applying to one API key, with the counter it shares already identified.
#[derive(Debug, Clone)]
pub struct MatchedBudget {
	/// Counter this budget charges. Keys resolving to the same identifier share their usage.
	pub(crate) budget_id: String,
	pub(crate) scope: ResolvedScope,
	pub(crate) budget: Budget,
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
	/// Document-level budget definitions from the configuration being normalized. Installed once
	/// while that configuration is converted, before any API key policy is compiled.
	#[serde(skip)]
	specs: Arc<OnceLock<Vec<Budget>>>,
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

fn budget_id(scope: &ResolvedScope, budget: &Budget) -> String {
	match scope {
		// Unchanged from the original encoding so existing budget_usage rows keep resolving.
		ResolvedScope::PerKey { api_key_id, .. } => format!(
			"api-key:{}:{}:{}:{}",
			api_key_id.len(),
			api_key_id,
			budget.name.len(),
			budget.name
		),
		ResolvedScope::GroupBy { field, value } => format!(
			"group:{}:{}:{}:{}:{}:{}",
			budget.name.len(),
			budget.name,
			field.len(),
			field,
			value.len(),
			value
		),
		// The selector is deliberately absent: editing membership must not reset accumulated usage.
		ResolvedScope::Selector => format!("budget:{}:{}", budget.name.len(), budget.name),
	}
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
			specs: Arc::new(OnceLock::new()),
		}
	}

	/// Installs the document-level budgets for the configuration being normalized. Called once per
	/// reload, before any API key policy is compiled.
	pub fn set_specs(&self, budgets: Vec<Budget>) {
		self
			.specs
			.set(budgets)
			.expect("budget specs are installed once per configuration");
	}

	/// Document-level budgets from the configuration being normalized. Empty on the process-wide
	/// policy, which never resolves budgets itself.
	pub(crate) fn specs(&self) -> &[Budget] {
		self.specs.get().map_or(&[], Vec::as_slice)
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
						.configure(&definition.scope, &definition.budget, now)?
				},
				Entry::Vacant(entry) => {
					entry.insert(BudgetCounter::configured(
						&definition.scope,
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
			.any(|policy| policy.budgets.is_some())
			|| !self.specs().is_empty();
		anyhow::ensure!(
			!has_budgets || self.database.get().is_some() || database_configured,
			"API key budgets require config.database to be configured"
		);
		for policy in authentication.users.values() {
			let Some(budgets) = policy.budgets.as_ref() else {
				continue;
			};
			for MatchedBudget {
				budget_id,
				scope,
				budget,
			} in &budgets.budgets
			{
				if let Some(registration) = &self.registration {
					BudgetCounter::configured(scope, budget, now)?;
					registration.insert(
						budget_id.clone(),
						BudgetDefinition {
							scope: scope.clone(),
							budget: budget.clone(),
						},
					);
					continue;
				}
				match self.counters.entry(budget_id.clone()) {
					Entry::Occupied(mut entry) => {
						entry.get_mut().configure(scope, budget, now)?;
					},
					Entry::Vacant(entry) => {
						entry.insert(BudgetCounter::configured(scope, budget, now)?);
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
		for MatchedBudget {
			budget_id,
			scope,
			budget,
		} in &budgets.budgets
		{
			let (used, window_end) = {
				let mut counter = self
					.counters
					.get_mut(budget_id)
					.context("budget counter was not registered")?;
				counter.refresh(now);
				(counter.amount, counter.window_end)
			};
			let exceeded = used >= budget.limit.amount.decimal();
			if exceeded {
				tracing::warn!(
					target: "budget",
					api_key = budgets.api_key,
					scope = %scope,
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
					scope = %scope,
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
		for MatchedBudget {
			budget_id,
			scope,
			budget,
		} in &budgets.budgets
		{
			let charged = match budget.limit.unit {
				BudgetLimitUnit::Usd => response.cost.as_ref().map(|cost| cost.total()),
				BudgetLimitUnit::Tokens => response.total_tokens.map(Decimal::from),
			};
			let Some(charged) = charged else {
				tracing::debug!(
					target: "budget",
					api_key = budgets.api_key,
					scope = %scope,
					budget = budget.name,
					limit_unit = budget.limit.unit.as_str(),
					"API key budget could not be charged because usage was unavailable"
				);
				continue;
			};

			let Some(mut counter) = self.counters.get_mut(budget_id) else {
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
				scope = %scope,
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

	fn test_scope() -> ResolvedScope {
		ResolvedScope::PerKey {
			api_key_id: "hash".to_string(),
			api_key: "key".to_string(),
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
		let authentication = keys.compile(&[]).unwrap();
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
		policy
			.register(&current.compile(&[]).unwrap(), true)
			.unwrap();

		let candidate = policy.registration_policy();
		candidate
			.register(&replacement.compile(&[]).unwrap(), true)
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
			scope: BudgetScope::PerKey,
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
			BudgetCounter::configured(&test_scope(), &budget, now).unwrap(),
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
			BudgetCounter::configured(&test_scope(), &budget, now).unwrap(),
		);
		second.counters.insert(
			"window".to_string(),
			BudgetCounter::configured(&test_scope(), &budget, now).unwrap(),
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
			scope: BudgetScope::PerKey,
		};
		let mut expired_residue = BudgetCounter::configured(&test_scope(), &usd_budget, now).unwrap();
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

	fn scoped_budget(name: &str, amount: i64, scope: BudgetScope) -> Budget {
		Budget {
			name: name.to_string(),
			limit: BudgetLimit {
				unit: BudgetLimitUnit::Tokens,
				amount: BudgetAmount(Decimal::from(amount)),
			},
			window: BudgetWindow {
				rolling: Duration::from_secs(60 * 60),
			},
			on_budget_exceeded: BudgetExceededAction::Block,
			scope,
		}
	}

	fn local_keys(keys: serde_json::Value) -> crate::http::apikey::LocalAPIKeys {
		serde_json::from_value(keys).unwrap()
	}

	/// Every counter identifier produced for the compiled keys, so tests can assert which keys were
	/// pooled together without depending on map iteration order.
	fn counter_ids(authentication: &crate::http::apikey::APIKeyAuthentication) -> Vec<String> {
		let mut ids = authentication
			.users
			.values()
			.filter_map(|policy| policy.budgets.as_ref())
			.flat_map(|budgets| budgets.budgets.iter())
			.map(|matched| matched.budget_id.clone())
			.collect::<Vec<_>>();
		ids.sort();
		ids
	}

	#[test]
	fn group_scoped_budgets_pool_keys_sharing_a_metadata_value() {
		let specs = vec![scoped_budget(
			"team",
			100,
			BudgetScope::GroupBy("group".to_string()),
		)];
		let authentication = local_keys(serde_json::json!({
			"keys": [
				{"key": "sk-a", "metadata": {"name": "alice", "group": "research"}},
				{"key": "sk-b", "metadata": {"name": "bob", "group": "research"}},
				{"key": "sk-c", "metadata": {"name": "carol", "group": "platform"}},
			]
		}))
		.compile(&specs)
		.unwrap();

		let ids = counter_ids(&authentication);
		assert_eq!(ids.len(), 3, "every key is budgeted");
		let mut distinct = ids.clone();
		distinct.dedup();
		assert_eq!(
			distinct.len(),
			2,
			"the two research keys share a counter, platform has its own: {ids:?}"
		);

		// Each pooled counter is reported once, described by the metadata value it partitions on
		// rather than by any one of the keys contributing to it.
		let policy = BudgetPolicy::default();
		policy.register(&authentication, true).unwrap();
		let reported = policy.status(None).unwrap().budgets;
		assert_eq!(reported.len(), 2);
		assert_eq!(reported[0].scope.kind, "groupBy");
		assert_eq!(reported[0].scope.field.as_deref(), Some("group"));
		let mut values = reported
			.iter()
			.map(|budget| budget.scope.value.as_deref().unwrap())
			.collect::<Vec<_>>();
		values.sort();
		assert_eq!(values, ["platform", "research"]);
	}

	#[test]
	fn shared_budgets_only_pool_keys_the_selector_matches() {
		let specs = vec![scoped_budget(
			"research",
			100,
			BudgetScope::Selector(HashMap::from([(
				"group".to_string(),
				"research".to_string(),
			)])),
		)];
		let authentication = local_keys(serde_json::json!({
			"keys": [
				{"key": "sk-a", "metadata": {"name": "alice", "group": "research"}},
				{"key": "sk-b", "metadata": {"name": "bob", "group": "research"}},
				{"key": "sk-c", "metadata": {"name": "carol", "group": "platform"}},
			]
		}))
		.compile(&specs)
		.unwrap();

		let ids = counter_ids(&authentication);
		assert_eq!(ids.len(), 2, "the unmatched key is not budgeted");
		assert_eq!(ids[0], ids[1], "matched keys share one counter");
	}

	/// Unquoted YAML numbers deserialize as JSON numbers. Requiring a quoted selector would leave
	/// such keys silently unbudgeted, so scalars are compared by their display form.
	#[test]
	fn numeric_metadata_matches_a_string_selector() {
		let specs = vec![scoped_budget(
			"gold",
			100,
			BudgetScope::Selector(HashMap::from([("tier".to_string(), "1".to_string())])),
		)];
		let authentication = local_keys(serde_json::json!({
			"keys": [{"key": "sk-a", "metadata": {"name": "alice", "tier": 1}}]
		}))
		.compile(&specs)
		.unwrap();

		assert_eq!(counter_ids(&authentication).len(), 1);
	}

	/// A document-level per-key budget and an inline budget of the same name resolve to the same
	/// counter, which would otherwise let one silently replace the other's limit.
	#[test]
	fn a_document_budget_cannot_collide_with_an_inline_budget() {
		let specs = vec![scoped_budget("daily", 100, BudgetScope::PerKey)];
		let err = local_keys(serde_json::json!({
			"keys": [{
				"key": "sk-a",
				"metadata": {"name": "alice"},
				"budgets": [{
					"name": "daily",
					"limit": {"unit": "Tokens", "amount": 40},
					"window": {"rolling": "1h"},
					"onBudgetExceeded": "Block"
				}]
			}]
		}))
		.compile(&specs)
		.unwrap_err();

		assert!(
			err.to_string().contains("shares a counter"),
			"unexpected error: {err}"
		);
	}

	/// A group counter's accumulated usage survives a limit change, because the counter identity
	/// does not include the limit. Changing the window resets it, because the window does.
	#[test]
	fn editing_a_document_budget_preserves_usage_unless_the_window_changes() {
		let keys = serde_json::json!({
			"keys": [{"key": "sk-a", "metadata": {"name": "alice", "group": "research"}}]
		});
		let raised = vec![scoped_budget(
			"team",
			200,
			BudgetScope::GroupBy("group".to_string()),
		)];
		let policy = BudgetPolicy::default();
		policy
			.register(
				&local_keys(keys.clone())
					.compile(&[scoped_budget(
						"team",
						100,
						BudgetScope::GroupBy("group".to_string()),
					)])
					.unwrap(),
				true,
			)
			.unwrap();

		let budget_id = counter_ids(&local_keys(keys.clone()).compile(&raised).unwrap())
			.pop()
			.unwrap();
		policy.counters.get_mut(&budget_id).unwrap().amount = Decimal::from(30);

		let candidate = policy.registration_policy();
		candidate
			.register(&local_keys(keys.clone()).compile(&raised).unwrap(), true)
			.unwrap();
		policy.apply_registration(candidate.registration()).unwrap();

		let status = policy.status(None).unwrap();
		assert_eq!(status.budgets[0].limit.amount, "200");
		assert_eq!(
			status.budgets[0].usage.used, "30",
			"usage survives a raised limit"
		);

		let mut shortened = raised.clone();
		shortened[0].window.rolling = Duration::from_secs(60);
		let candidate = policy.registration_policy();
		candidate
			.register(&local_keys(keys).compile(&shortened).unwrap(), true)
			.unwrap();
		policy.apply_registration(candidate.registration()).unwrap();

		let status = policy.status(None).unwrap();
		assert_eq!(
			status.budgets[0].usage.used, "0",
			"a new window resets usage"
		);
	}
}
