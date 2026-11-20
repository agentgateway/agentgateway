//! PostgreSQL integration tests for budget schema setup.
//!
//! These tests run against the server named by `AGW_TEST_POSTGRES_URL` and are skipped when it is
//! unset, unless `AGW_TEST_REQUIRE_POSTGRES` is set. Each test creates its own database, so one
//! server can be reused across tests and runs. To run them locally:
//!
//! ```sh
//! docker run -d --name agw-test-postgres -e POSTGRES_PASSWORD=test \
//!   -p 127.0.0.1:55432:5432 postgres:17-alpine
//! until docker exec agw-test-postgres pg_isready -U postgres -h 127.0.0.1; do sleep 1; done
//! AGW_TEST_POSTGRES_URL=postgres://postgres:test@127.0.0.1:55432/postgres \
//!   cargo test -p agentgateway --lib budget::postgres_tests
//! docker rm -f agw-test-postgres
//! ```

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

use super::*;
use crate::database::DatabasePool;

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
/// The `budget_usage` table as created by releases that had the `unit` column but no migrations.
const UNTRACKED_SCHEMA: &str = r#"
CREATE TABLE budget_usage (
    budget_id TEXT PRIMARY KEY,
    window_start BIGINT NOT NULL,
    window_end BIGINT NOT NULL,
    unit TEXT NOT NULL,
    used_amount BIGINT NOT NULL DEFAULT 0 CHECK (used_amount >= 0),
    updated_at BIGINT NOT NULL
);
"#;
/// Proxies started at once in each round of the concurrent startup test.
const CONCURRENT_INITIALIZERS: usize = 8;
/// Rounds of the concurrent startup test, each against a fresh database.
const CONCURRENT_ROUNDS: usize = 3;

/// Reads an environment variable, treating an empty value as unset.
fn env_var(name: &str) -> Option<String> {
	std::env::var(name).ok().filter(|value| !value.is_empty())
}

struct TestPostgres {
	server: PgConnectOptions,
}

impl TestPostgres {
	/// Connects to the server `AGW_TEST_POSTGRES_URL` names, or returns `None` to skip the test
	/// when it is unset and `AGW_TEST_REQUIRE_POSTGRES` is not.
	fn from_env() -> Option<Self> {
		let Some(url) = env_var("AGW_TEST_POSTGRES_URL") else {
			assert!(
				env_var("AGW_TEST_REQUIRE_POSTGRES").is_none(),
				"AGW_TEST_REQUIRE_POSTGRES is set, but AGW_TEST_POSTGRES_URL is not"
			);
			eprintln!("skipping Postgres budget test because AGW_TEST_POSTGRES_URL is unset");
			return None;
		};
		let server = PgConnectOptions::from_str(&url).expect("invalid AGW_TEST_POSTGRES_URL");
		Some(Self { server })
	}

	/// Creates an empty database with a unique name starting with `prefix`, and returns the name.
	async fn create_database(&self, prefix: &str) -> String {
		let name = format!("{prefix}_{}", uuid::Uuid::new_v4().simple());
		let admin = PgPool::connect_with(self.server.clone()).await.unwrap();
		sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
			.execute(&admin)
			.await
			.unwrap();
		admin.close().await;
		name
	}

	/// Opens a separate pool on database `name`, like a separate proxy would.
	async fn pool(&self, name: &str, max_connections: u32) -> PgPool {
		PgPoolOptions::new()
			.max_connections(max_connections)
			.connect_with(self.server.clone().database(name))
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
/// `CREATE TABLE IF NOT EXISTS` statements on a fresh database can fail and abort startup. The
/// legacy case checks that concurrent startups also all migrate an existing table.
#[tokio::test]
async fn concurrent_initializers_create_and_migrate_the_schema() {
	let Some(postgres) = TestPostgres::from_env() else {
		return;
	};
	for round in 0..CONCURRENT_ROUNDS {
		for (case, legacy) in [("fresh", false), ("legacy", true)] {
			let database = postgres
				.create_database(&format!("concurrent_{case}_{round}"))
				.await;
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
			// Polled together in this task rather than spawned: the migration future is not `Send`.
			// Each initializer has its own connection, so the server still runs them concurrently.
			let results = futures::future::join_all(pools.into_iter().map(|pool| async move {
				Arc::new(BudgetPolicy::default())
					.initialize(DatabasePool::Postgres(pool))
					.await
			}))
			.await;
			for (initializer, result) in results.into_iter().enumerate() {
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

/// Migrations set a session lock timeout and take a session advisory lock, then close their
/// connection, so neither may reach a connection the pool hands out afterwards.
#[tokio::test]
async fn schema_setup_does_not_leak_lock_settings_into_the_pool() {
	let Some(postgres) = TestPostgres::from_env() else {
		return;
	};
	let database = postgres.create_database("no_leak").await;
	// One connection, so a migration connection returned to the pool would be the one checked below.
	let pool = postgres.pool(&database, 1).await;
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

/// Starting against a table that already has the current columns must not take the
/// ACCESS EXCLUSIVE lock of `ALTER TABLE`, which would wait on running proxies' open transactions.
/// Covers both a restart (migrations already recorded) and the first upgrade of a database from a
/// release without a migrations table (migrations apply over the existing table).
#[tokio::test]
async fn startup_does_not_wait_on_open_readers() {
	let Some(postgres) = TestPostgres::from_env() else {
		return;
	};
	for case in ["restart", "upgrade"] {
		let database = postgres
			.create_database(&format!("open_reader_{case}"))
			.await;
		let setup = postgres.pool(&database, 1).await;
		if case == "restart" {
			Arc::new(BudgetPolicy::default())
				.initialize(DatabasePool::Postgres(setup))
				.await
				.unwrap();
		} else {
			sqlx::raw_sql(UNTRACKED_SCHEMA)
				.execute(&setup)
				.await
				.unwrap();
			setup.close().await;
		}

		// A running proxy's transaction that has read the table holds ACCESS SHARE until it ends.
		let running = postgres.pool(&database, 1).await;
		let mut reader = running.begin().await.unwrap();
		sqlx::query("SELECT COUNT(*) FROM budget_usage")
			.execute(&mut *reader)
			.await
			.unwrap();

		let starting = Arc::new(BudgetPolicy::default());
		let started = tokio::time::timeout(
			Duration::from_secs(5),
			starting.initialize(DatabasePool::Postgres(postgres.pool(&database, 1).await)),
		)
		.await
		.unwrap_or_else(|_| panic!("{case}: schema setup waited on an open reader"));
		started.unwrap_or_else(|err| panic!("{case}: schema setup failed: {err:#}"));
		reader.rollback().await.unwrap();
	}
}

/// Databases set up by releases without a migrations table must upgrade in place: the migrations
/// apply over the existing table, keep its rows, and are recorded so later restarts skip them.
#[tokio::test]
async fn upgrade_from_untracked_schema_keeps_rows() {
	let Some(postgres) = TestPostgres::from_env() else {
		return;
	};
	for (case, schema, unit_insert) in [
		("legacy", LEGACY_SCHEMA, ""),
		("untracked", UNTRACKED_SCHEMA, "'Tokens', "),
	] {
		let database = postgres.create_database(&format!("upgrade_{case}")).await;
		let pool = postgres.pool(&database, 2).await;
		sqlx::raw_sql(schema).execute(&pool).await.unwrap();
		let unit_column = if unit_insert.is_empty() { "" } else { "unit, " };
		sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
			"INSERT INTO budget_usage (budget_id, window_start, window_end, {unit_column}used_amount, \
			 updated_at) VALUES ('kept', 0, {window_end}, {unit_insert}7, 0)",
			window_end = (Utc::now() + chrono::Duration::days(1)).timestamp_millis(),
		)))
		.execute(&pool)
		.await
		.unwrap();

		Arc::new(BudgetPolicy::default())
			.initialize(DatabasePool::Postgres(pool.clone()))
			.await
			.unwrap_or_else(|err| panic!("{case}: upgrade failed: {err:#}"));

		assert_eq!(
			unit_column_count(&pool).await,
			1,
			"{case}: unit column missing"
		);
		let kept: i64 =
			sqlx::query_scalar("SELECT used_amount FROM budget_usage WHERE budget_id = 'kept'")
				.fetch_one(&pool)
				.await
				.unwrap();
		assert_eq!(kept, 7, "{case}: existing row lost");
		let applied: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
			"SELECT COUNT(*) FROM {} WHERE success",
			super::database::POSTGRES_MIGRATIONS_TABLE
		)))
		.fetch_one(&pool)
		.await
		.unwrap();
		assert_eq!(applied, 2, "{case}: migrations not recorded");
		pool.close().await;
	}
}

/// Pruning runs after the migrations finish; expired rows must still be removed and
/// live rows preloaded into the in-memory counters.
#[tokio::test]
async fn initialization_prunes_expired_rows_and_preloads_live_ones() {
	let Some(postgres) = TestPostgres::from_env() else {
		return;
	};
	let database = postgres.create_database("prune").await;
	let pool = postgres.pool(&database, 2).await;
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

#[tokio::test]
async fn upsert_ignores_stale_windows_and_accepts_reconfigured_ones() {
	let Some(postgres) = TestPostgres::from_env() else {
		return;
	};
	let database = postgres.create_database("upsert").await;
	let pool = postgres.pool(&database, 2).await;
	Arc::new(BudgetPolicy::default())
		.initialize(DatabasePool::Postgres(pool.clone()))
		.await
		.unwrap();

	super::tests::assert_upsert_cases!(
		&pool,
		"postgres_upsert.sql",
		"SELECT window_start, window_end, used_amount FROM budget_usage WHERE budget_id = $1"
	);
}
