//! PostgreSQL integration tests for budget schema setup.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::postgres::{PgPool, PgPoolOptions};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};

use super::*;
use crate::database::DatabasePool;

const POSTGRES_VERSION: &str = "17-alpine";
/// The `budget_usage` table as created before the `unit` column existed.
const LEGACY_SCHEMA: &str = r#"
CREATE TABLE budget_usage (
    budget_id TEXT PRIMARY KEY,
    window_start BIGINT NOT NULL,
    window_end BIGINT NOT NULL,
    used_amount BIGINT NOT NULL DEFAULT 0 CHECK (used_amount >= 0),
    updated_at BIGINT NOT NULL
);
"#;
/// Proxies started at once in each round of the concurrent startup test.
const CONCURRENT_INITIALIZERS: usize = 8;
/// Rounds of the concurrent startup test, each against a fresh database.
const CONCURRENT_ROUNDS: usize = 3;

struct TestPostgres {
	_container: ContainerAsync<Postgres>,
	base_url: String,
}

impl TestPostgres {
	/// Starts Postgres, or returns `None` to skip the test when Docker is unavailable outside CI.
	async fn start() -> Option<Self> {
		let container = match Postgres::default().with_tag(POSTGRES_VERSION).start().await {
			Ok(container) => container,
			Err(err) if std::env::var_os("CI").is_none() => {
				eprintln!("skipping Postgres budget test because Docker is unavailable: {err}");
				return None;
			},
			Err(err) => panic!("Postgres budget tests require Docker in CI: {err}"),
		};
		let host = container.get_host().await.unwrap();
		let port = container.get_host_port_ipv4(5432).await.unwrap();
		Some(Self {
			base_url: format!("postgres://postgres:postgres@{host}:{port}"),
			_container: container,
		})
	}

	/// Creates an empty database named `name`.
	async fn create_database(&self, name: &str) {
		let admin = self.pool("postgres", 1).await;
		sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
			.execute(&admin)
			.await
			.unwrap();
		admin.close().await;
	}

	/// Opens a separate pool on database `name`, like a separate proxy would.
	async fn pool(&self, name: &str, max_connections: u32) -> PgPool {
		PgPoolOptions::new()
			.max_connections(max_connections)
			.connect(&format!("{}/{name}", self.base_url))
			.await
			.unwrap()
	}
}

async fn unit_column_count(pool: &PgPool) -> i64 {
	sqlx::query_scalar(
		"SELECT COUNT(*) FROM pg_attribute WHERE attrelid = 'budget_usage'::regclass \
		 AND attname = 'unit' AND NOT attisdropped",
	)
	.fetch_one(pool)
	.await
	.unwrap()
}

/// Several proxies sharing one database start at once. Without the advisory lock, concurrent
/// `CREATE TABLE IF NOT EXISTS` / `ADD COLUMN` statements can fail and abort startup.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_initializers_create_and_migrate_the_schema() {
	let Some(postgres) = TestPostgres::start().await else {
		return;
	};
	for round in 0..CONCURRENT_ROUNDS {
		for (case, legacy) in [("fresh", false), ("legacy", true)] {
			let database = format!("concurrent_{case}_{round}");
			postgres.create_database(&database).await;
			let inspect = postgres.pool(&database, 1).await;
			if legacy {
				sqlx::raw_sql(LEGACY_SCHEMA)
					.execute(&inspect)
					.await
					.unwrap();
			}

			// Connect every pool first so the initializers start as close together as possible.
			let mut pools = Vec::with_capacity(CONCURRENT_INITIALIZERS);
			for _ in 0..CONCURRENT_INITIALIZERS {
				pools.push(postgres.pool(&database, 1).await);
			}
			let start = Arc::new(tokio::sync::Barrier::new(CONCURRENT_INITIALIZERS));
			let mut initializers = tokio::task::JoinSet::new();
			for (initializer, pool) in pools.into_iter().enumerate() {
				let start = start.clone();
				initializers.spawn(async move {
					start.wait().await;
					let result = Arc::new(BudgetPolicy::default())
						.initialize(DatabasePool::Postgres(pool))
						.await;
					(initializer, result)
				});
			}
			while let Some(joined) = initializers.join_next().await {
				let (initializer, result) = joined.unwrap();
				if let Err(err) = result {
					panic!("{case} round {round}: initializer {initializer} failed: {err:#}");
				}
			}

			assert_eq!(
				unit_column_count(&inspect).await,
				1,
				"{case} round {round}: unit column missing"
			);
			inspect.close().await;
		}
	}
}

/// The lock timeout and advisory lock are transaction-scoped, so a pooled connection that ran
/// schema setup must come back with neither.
#[tokio::test]
async fn schema_setup_does_not_leak_lock_settings_into_the_pool() {
	let Some(postgres) = TestPostgres::start().await else {
		return;
	};
	postgres.create_database("no_leak").await;
	// One connection, so the checks below run on the connection that did the schema setup.
	let pool = postgres.pool("no_leak", 1).await;
	let default_lock_timeout: String = sqlx::query_scalar("SHOW lock_timeout")
		.fetch_one(&pool)
		.await
		.unwrap();

	Arc::new(BudgetPolicy::default())
		.initialize(DatabasePool::Postgres(pool.clone()))
		.await
		.unwrap();

	let lock_timeout: String = sqlx::query_scalar("SHOW lock_timeout")
		.fetch_one(&pool)
		.await
		.unwrap();
	assert_eq!(lock_timeout, default_lock_timeout);
	let advisory_locks: i64 = sqlx::query_scalar(
		"SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' \
		 AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
	)
	.fetch_one(&pool)
	.await
	.unwrap();
	assert_eq!(advisory_locks, 0);
}

/// Restarting against an up-to-date schema must not take the ACCESS EXCLUSIVE lock of
/// `ALTER TABLE`, which would wait on running proxies' open transactions.
#[tokio::test]
async fn restart_does_not_wait_on_open_readers() {
	let Some(postgres) = TestPostgres::start().await else {
		return;
	};
	postgres.create_database("open_reader").await;
	Arc::new(BudgetPolicy::default())
		.initialize(DatabasePool::Postgres(
			postgres.pool("open_reader", 1).await,
		))
		.await
		.unwrap();

	// A running proxy's transaction that has read the table holds ACCESS SHARE until it ends.
	let running = postgres.pool("open_reader", 1).await;
	let mut reader = running.begin().await.unwrap();
	sqlx::query("SELECT COUNT(*) FROM budget_usage")
		.execute(&mut *reader)
		.await
		.unwrap();

	let restart = Arc::new(BudgetPolicy::default());
	let restarted = tokio::time::timeout(
		Duration::from_secs(5),
		restart.initialize(DatabasePool::Postgres(
			postgres.pool("open_reader", 1).await,
		)),
	)
	.await
	.expect("schema setup waited on an open reader");
	restarted.unwrap();
	reader.rollback().await.unwrap();
}

/// Pruning runs after the schema transaction commits; expired rows must still be removed and
/// live rows preloaded into the in-memory counters.
#[tokio::test]
async fn initialization_prunes_expired_rows_and_preloads_live_ones() {
	let Some(postgres) = TestPostgres::start().await else {
		return;
	};
	postgres.create_database("prune").await;
	let pool = postgres.pool("prune", 2).await;
	Arc::new(BudgetPolicy::default())
		.initialize(DatabasePool::Postgres(pool.clone()))
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
		"INSERT INTO budget_usage (budget_id, window_start, window_end, unit, used_amount, updated_at) \
		 VALUES ('preloaded', $1, $2, 'Tokens', 9, $3), ('expired', 0, 1, 'Tokens', 8, 0)",
	)
	.bind(window_start.timestamp_millis())
	.bind(window_end.timestamp_millis())
	.bind(now.timestamp_millis())
	.execute(&pool)
	.await
	.unwrap();

	let restarted = Arc::new(BudgetPolicy::default());
	restarted.counters.insert(
		"preloaded".to_string(),
		BudgetCounter::configured("key", &budget, now).unwrap(),
	);
	restarted
		.initialize(DatabasePool::Postgres(pool.clone()))
		.await
		.unwrap();

	assert_eq!(
		restarted.counters.get("preloaded").unwrap().amount,
		Decimal::from(9),
	);
	let remaining: Vec<String> =
		sqlx::query_scalar("SELECT budget_id FROM budget_usage ORDER BY budget_id")
			.fetch_all(&pool)
			.await
			.unwrap();
	assert_eq!(remaining, vec!["preloaded".to_string()]);
}
