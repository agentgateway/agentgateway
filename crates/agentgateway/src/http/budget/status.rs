use chrono::Utc;
use rust_decimal::Decimal;

use super::{BudgetPolicy, ResolvedScope};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusResponse {
	pub observed_at: i64,
	pub budgets: Vec<BudgetStatus>,
}

/// User-facing snapshot of one budget's definition, current usage, and fixed window.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatus {
	pub scope: BudgetStatusScope,
	pub name: String,
	pub limit: BudgetStatusLimit,
	pub usage: BudgetStatusUsage,
	pub window: BudgetStatusWindow,
	pub on_budget_exceeded: String,
	pub updated_at: i64,
}

/// Identifies which API keys share a counter, so a budget can be attributed to a single key, to a
/// metadata value such as a group or tier, or to every key a selector matched.
///
/// Field order is also the sort order for reported budgets.
#[derive(Debug, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusScope {
	/// `perKey`, `groupBy`, or `shared`.
	pub kind: &'static str,
	/// Metadata field partitioning the counter. Only set for `groupBy`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub field: Option<String>,
	/// Value identifying this counter within its kind: the API key display name for `perKey`, the
	/// metadata value for `groupBy`. Absent for `shared`, where the budget name is the identity.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub value: Option<String>,
}

impl From<&ResolvedScope> for BudgetStatusScope {
	fn from(scope: &ResolvedScope) -> Self {
		match scope {
			ResolvedScope::PerKey { api_key, .. } => Self {
				kind: "perKey",
				field: None,
				value: Some(api_key.clone()),
			},
			ResolvedScope::GroupBy { field, value } => Self {
				kind: "groupBy",
				field: Some(field.clone()),
				value: Some(value.clone()),
			},
			ResolvedScope::Shared => Self {
				kind: "shared",
				field: None,
				value: None,
			},
		}
	}
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusLimit {
	pub unit: String,
	pub amount: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusUsage {
	pub used: String,
	pub remaining: String,
	pub exceeded: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusWindow {
	pub start: i64,
	pub end: i64,
	pub duration_ms: i64,
	pub expired: bool,
}

impl BudgetPolicy {
	/// Returns a point-in-time status snapshot, optionally narrowed to the budgets that can apply to
	/// one API key display name.
	///
	/// Expired counters are reported with zero usage even if no request has advanced their window.
	/// Group and shared counters do not record which keys contribute to them, so they are reported
	/// for every key rather than hidden from the key whose spend they may be limiting.
	pub fn status(&self, api_key_name: Option<&str>) -> anyhow::Result<BudgetStatusResponse> {
		let observed_at = Utc::now();
		let applies_to_key = |scope: &ResolvedScope| match scope {
			ResolvedScope::PerKey { api_key, .. } => api_key_name.is_none_or(|name| api_key == name),
			ResolvedScope::GroupBy { .. } | ResolvedScope::Shared => true,
		};
		let mut budgets = self
			.counters
			.iter()
			.filter_map(|counter| {
				let definition = counter.definition.as_ref()?;
				if !applies_to_key(&definition.scope) {
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
					scope: (&definition.scope).into(),
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
						end: counter.window_end.timestamp_millis(),
						duration_ms: i64::try_from(counter.rolling.as_millis())
							.expect("budget duration was validated"),
						expired,
					},
					on_budget_exceeded: definition.budget.on_budget_exceeded.as_str().to_owned(),
					updated_at: counter.updated_at.timestamp_millis(),
				})
			})
			.collect::<Vec<_>>();
		budgets.sort_by(|a, b| (&a.scope, &a.name).cmp(&(&b.scope, &b.name)));
		Ok(BudgetStatusResponse {
			observed_at: observed_at.timestamp_millis(),
			budgets,
		})
	}
}
