use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::{
	BudgetCounter, BudgetLimitUnit, BudgetPolicy, NANODOLLARS_PER_USD, PendingBudgetUsage,
	PersistedBudgetUsage, UnixDate,
};

const FLUSH_INTERVAL: Duration = Duration::from_secs(5);
const SQLITE_SCHEMA: &str = include_str!("sqlite_schema.sql");
const POSTGRES_SCHEMA: &str = include_str!("postgres_schema.sql");
const SQLITE_UPSERT: &str = include_str!("sqlite_upsert.sql");
const POSTGRES_UPSERT: &str = include_str!("postgres_upsert.sql");

impl BudgetPolicy {
	/// Initializes and migrates the budget tables, prunes expired rows, preloads every persisted
	/// counter into memory, and starts the periodic flush task.
	pub async fn initialize(
		self: &Arc<Self>,
		pool: crate::database::DatabasePool,
	) -> anyhow::Result<()> {
		let now = Utc::now();
		match &pool {
			crate::database::DatabasePool::Sqlite(pool) => {
				sqlx::raw_sql(SQLITE_SCHEMA)
					.execute(pool)
					.await
					.context("failed to initialize budget database")?;
				let has_unit = sqlx::query_scalar::<_, i64>(
					"SELECT COUNT(*) FROM pragma_table_info('budget_usage') WHERE name = 'unit'",
				)
				.fetch_one(pool)
				.await
				.context("failed to inspect budget database schema")?;
				if has_unit == 0 {
					sqlx::query("ALTER TABLE budget_usage ADD COLUMN unit TEXT")
						.execute(pool)
						.await
						.context("failed to migrate budget database schema")?;
				}
				sqlx::query("DELETE FROM budget_usage WHERE window_end <= ?")
					.bind(now.timestamp_millis())
					.execute(pool)
					.await
					.context("failed to prune expired budget usage")?;
			},
			crate::database::DatabasePool::Postgres(pool) => {
				sqlx::raw_sql(POSTGRES_SCHEMA)
					.execute(pool)
					.await
					.context("failed to initialize budget database")?;
				sqlx::query("ALTER TABLE budget_usage ADD COLUMN IF NOT EXISTS unit TEXT")
					.execute(pool)
					.await
					.context("failed to migrate budget database schema")?;
				sqlx::query("DELETE FROM budget_usage WHERE window_end <= $1")
					.bind(now.timestamp_millis())
					.execute(pool)
					.await
					.context("failed to prune expired budget usage")?;
			},
		};
		let persisted = Self::read_persisted(&pool)
			.await
			.context("failed to preload budget usage")?;
		if self.database.set(pool).is_err() {
			return Ok(());
		}
		self.reconcile(persisted, now);

		let policy = Arc::downgrade(self);
		tokio::spawn(async move {
			let mut interval = tokio::time::interval(FLUSH_INTERVAL);
			interval.tick().await;
			loop {
				interval.tick().await;
				let Some(policy) = policy.upgrade() else {
					return;
				};
				if let Err(err) = policy.flush().await {
					tracing::warn!(target: "budget", ?err, "failed to flush budget usage");
				}
			}
		});
		Ok(())
	}

	async fn read_persisted(
		pool: &crate::database::DatabasePool,
	) -> anyhow::Result<HashMap<String, PersistedBudgetUsage>> {
		let rows = match pool {
			crate::database::DatabasePool::Sqlite(pool) => {
				sqlx::query_as::<_, (String, i64, i64, Option<String>, i64, i64)>(
					"SELECT budget_id, window_start, window_end, unit, used_amount, updated_at FROM budget_usage",
				)
				.fetch_all(pool)
				.await?
			},
			crate::database::DatabasePool::Postgres(pool) => {
				sqlx::query_as::<_, (String, i64, i64, Option<String>, i64, i64)>(
					"SELECT budget_id, window_start, window_end, unit, used_amount, updated_at FROM budget_usage",
				)
				.fetch_all(pool)
				.await?
			},
		};
		rows
			.into_iter()
			.map(
				|(budget_id, window_start, window_end, unit, used_amount, updated_at)| {
					Ok((
						budget_id,
						PersistedBudgetUsage {
							window_start: UnixDate::from_timestamp_millis(window_start)
								.context("budget window start is out of range")?,
							window_end: UnixDate::from_timestamp_millis(window_end)
								.context("budget window end is out of range")?,
							unit: unit.as_deref().and_then(BudgetLimitUnit::from_database),
							used_amount,
							updated_at: UnixDate::from_timestamp_millis(updated_at)
								.context("budget update timestamp is out of range")?,
						},
					))
				},
			)
			.collect()
	}

	fn reconcile(&self, persisted: HashMap<String, PersistedBudgetUsage>, now: UnixDate) {
		for (budget_id, row) in &persisted {
			match self.counters.entry(budget_id.clone()) {
				dashmap::mapref::entry::Entry::Occupied(mut entry) => {
					entry.get_mut().reconcile(Some(row), now);
				},
				dashmap::mapref::entry::Entry::Vacant(entry) => {
					entry.insert(BudgetCounter::from_persisted(row));
				},
			}
		}
		for mut counter in self.counters.iter_mut() {
			if !persisted.contains_key(counter.key()) {
				counter.reconcile(None, now);
			}
		}
	}

	/// Snapshots nonzero per-counter deltas, atomically increments them in the database, subtracts
	/// only the successful snapshot, then reconciles totals written by concurrent workers.
	pub async fn flush(&self) -> anyhow::Result<()> {
		let Some(pool) = self.database.get() else {
			return Ok(());
		};
		let _flush = self.flush_lock.lock().await;
		if !self
			.counters
			.iter()
			.any(|counter| counter.definition.is_some() || !counter.pending.is_zero())
		{
			return Ok(());
		}
		let updated_at = Utc::now();
		let mut pending = Vec::new();
		for mut counter in self.counters.iter_mut() {
			if counter.definition.is_some() {
				counter.refresh(updated_at);
			}
			if counter.pending.is_zero() {
				continue;
			}
			let unit = counter
				.unit
				.context("configured budget counter has no unit")?;
			let scaled = match unit {
				BudgetLimitUnit::Usd => counter.pending * Decimal::from(NANODOLLARS_PER_USD),
				BudgetLimitUnit::Tokens => counter.pending,
			};
			let used_amount = scaled
				.trunc()
				.to_i64()
				.context("budget usage exceeds database integer range")?;
			if used_amount == 0 {
				continue;
			}
			pending.push(PendingBudgetUsage {
				budget_id: counter.key().clone(),
				window_start: counter.window_start,
				window_end: counter.window_end,
				unit,
				used_amount,
				flushed: match unit {
					BudgetLimitUnit::Usd => Decimal::new(used_amount, 9),
					BudgetLimitUnit::Tokens => Decimal::from(used_amount),
				},
			});
		}
		pending.sort_by(|a, b| {
			(&a.budget_id, a.window_start, a.window_end).cmp(&(
				&b.budget_id,
				b.window_start,
				b.window_end,
			))
		});

		if !pending.is_empty() {
			match pool {
				crate::database::DatabasePool::Sqlite(pool) => {
					let mut transaction = pool.begin().await?;
					for counter in &pending {
						sqlx::query(SQLITE_UPSERT)
							.bind(&counter.budget_id)
							.bind(counter.window_start.timestamp_millis())
							.bind(counter.window_end.timestamp_millis())
							.bind(counter.unit.as_str())
							.bind(counter.used_amount)
							.bind(updated_at.timestamp_millis())
							.execute(&mut *transaction)
							.await?;
					}
					transaction.commit().await?;
				},
				crate::database::DatabasePool::Postgres(pool) => {
					let mut transaction = pool.begin().await?;
					for counter in &pending {
						sqlx::query(POSTGRES_UPSERT)
							.bind(&counter.budget_id)
							.bind(counter.window_start.timestamp_millis())
							.bind(counter.window_end.timestamp_millis())
							.bind(counter.unit.as_str())
							.bind(counter.used_amount)
							.bind(updated_at.timestamp_millis())
							.execute(&mut *transaction)
							.await?;
					}
					transaction.commit().await?;
				},
			}
		}
		for flushed in pending {
			let Some(mut counter) = self.counters.get_mut(&flushed.budget_id) else {
				continue;
			};
			if counter.window_start == flushed.window_start
				&& counter.window_end == flushed.window_end
				&& counter.unit == Some(flushed.unit)
			{
				counter.pending -= flushed.flushed;
			}
		}
		let persisted = Self::read_persisted(pool).await?;
		self.reconcile(persisted, updated_at);
		Ok(())
	}
}

impl BudgetCounter {
	fn from_persisted(row: &PersistedBudgetUsage) -> Self {
		Self {
			definition: None,
			amount: match row.unit {
				Some(BudgetLimitUnit::Usd) => Decimal::new(row.used_amount, 9),
				Some(BudgetLimitUnit::Tokens) => Decimal::from(row.used_amount),
				None => Decimal::ZERO,
			},
			pending: Decimal::ZERO,
			unit: row.unit,
			rolling: (row.window_end - row.window_start)
				.to_std()
				.unwrap_or(Duration::ZERO),
			window_start: row.window_start,
			window_end: row.window_end,
			updated_at: row.updated_at,
		}
	}

	fn reconcile(&mut self, row: Option<&PersistedBudgetUsage>, now: UnixDate) {
		if self.definition.is_none() {
			if let Some(row) = row {
				*self = Self::from_persisted(row);
			}
			return;
		}

		self.refresh(now);
		let row = row.filter(|row| {
			row.window_end > now
				&& (row.window_end - row.window_start)
					.to_std()
					.is_ok_and(|rolling| rolling == self.rolling)
				&& row.unit == self.unit
		});
		let Some(row) = row else {
			self.amount = self.pending;
			return;
		};
		if self.window_start != row.window_start || self.window_end != row.window_end {
			self.pending = Decimal::ZERO;
		}
		self.amount = match row.unit {
			Some(BudgetLimitUnit::Usd) => Decimal::new(row.used_amount, 9),
			Some(BudgetLimitUnit::Tokens) => Decimal::from(row.used_amount),
			None => Decimal::ZERO,
		} + self.pending;
		self.window_start = row.window_start;
		self.window_end = row.window_end;
		self.updated_at = self.updated_at.max(row.updated_at);
	}
}
