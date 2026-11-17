use chrono::Utc;
use rust_decimal::Decimal;

use super::{BudgetPolicy, UNBOUNDED_WINDOW};

/// A point-in-time status of budgets.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusResponse {
	/// When the status was read, in milliseconds since the Unix epoch.
	pub observed_at: i64,
	/// The budgets, ordered by API key name and budget name.
	pub budgets: Vec<BudgetStatus>,
}

/// User-facing snapshot of one budget's definition, current usage, and fixed window.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatus {
	/// Display name of the API key that the budget belongs to.
	pub api_key_name: String,
	/// Name of the budget.
	pub name: String,
	/// The configured limit.
	pub limit: BudgetStatusLimit,
	/// Usage in the current window.
	pub usage: BudgetStatusUsage,
	/// The window that the counter tracks.
	pub window: BudgetStatusWindow,
	/// Action taken when the budget is exceeded: `Audit` or `Block`.
	pub on_budget_exceeded: String,
	/// When the usage was last recorded or reset, in milliseconds since the Unix epoch.
	pub updated_at: i64,
}

/// The limit of a budget.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusLimit {
	/// Unit of the limit: `USD` or `Tokens`.
	pub unit: String,
	/// Maximum usage in one window, as a decimal string.
	pub amount: String,
}

/// The usage of a budget in its current window.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusUsage {
	/// Usage so far, as a decimal string. Zero when the window has expired.
	pub used: String,
	/// The limit minus the usage, never below zero, as a decimal string.
	pub remaining: String,
	/// Whether the usage has reached the limit.
	pub exceeded: bool,
}

/// The usage window that the counter of a budget tracks.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusWindow {
	/// Start of the window, in milliseconds since the Unix epoch. Zero for a budget without a window.
	pub start: i64,
	/// End of the window, in milliseconds since the Unix epoch. Null for a budget without a window,
	/// which never resets.
	pub end: Option<i64>,
	/// Length of the window in milliseconds. Null for a budget without a window.
	pub duration_ms: Option<i64>,
	/// Whether the tracked window has ended and the counter has not advanced to the current window.
	pub expired: bool,
}

impl BudgetPolicy {
	/// Returns a point-in-time status snapshot, optionally filtered by API key display name.
	/// Expired counters are reported with zero usage even if no request has advanced their window.
	pub fn status(&self, api_key_name: Option<&str>) -> anyhow::Result<BudgetStatusResponse> {
		let observed_at = Utc::now();
		let mut budgets = self
			.counters
			.iter()
			.filter_map(|counter| {
				let definition = counter.definition.as_ref()?;
				if !api_key_name.is_none_or(|name| definition.api_key == name) {
					return None;
				}
				let limit = definition.budget.limit.amount.decimal();
				let expired = observed_at >= counter.window_end;
				let used = if expired {
					Decimal::ZERO
				} else {
					counter.amount
				};
				let remaining = (limit - used).max(Decimal::ZERO);
				Some(BudgetStatus {
					api_key_name: definition.api_key.clone(),
					name: definition.budget.name.clone(),
					limit: BudgetStatusLimit {
						unit: definition.budget.limit.unit.as_str().to_owned(),
						amount: limit.normalize().to_string(),
					},
					usage: BudgetStatusUsage {
						used: used.normalize().to_string(),
						remaining: remaining.normalize().to_string(),
						exceeded: !expired && used >= limit,
					},
					window: BudgetStatusWindow {
						start: counter.window_start.timestamp_millis(),
						end: (counter.rolling != UNBOUNDED_WINDOW)
							.then(|| counter.window_end.timestamp_millis()),
						duration_ms: (counter.rolling != UNBOUNDED_WINDOW).then(|| {
							i64::try_from(counter.rolling.as_millis()).expect("budget duration was validated")
						}),
						expired,
					},
					on_budget_exceeded: definition.budget.on_budget_exceeded.as_str().to_owned(),
					updated_at: counter.updated_at.timestamp_millis(),
				})
			})
			.collect::<Vec<_>>();
		budgets.sort_by(|a, b| (&a.api_key_name, &a.name).cmp(&(&b.api_key_name, &b.name)));
		Ok(BudgetStatusResponse {
			observed_at: observed_at.timestamp_millis(),
			budgets,
		})
	}
}
