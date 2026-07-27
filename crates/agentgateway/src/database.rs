use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{PgPool, SqlitePool};

#[derive(Clone, Debug)]
pub enum DatabasePool {
	Sqlite(SqlitePool),
	Postgres(PgPool),
}

/// Default pool size when `maxConnections` is not set in config. Kept small
/// since agentgateway opens a separate pool per store (request log + config
/// store) per process; each replica's total connection usage is roughly
/// `2 * max_connections`.
const DEFAULT_MAX_CONNECTIONS: u32 = 5;

impl DatabasePool {
	pub async fn connect(url: &str) -> anyhow::Result<Self> {
		Self::connect_with_max_connections(url, None).await
	}

	pub async fn connect_with_max_connections(
		url: &str,
		max_connections: Option<u32>,
	) -> anyhow::Result<Self> {
		let max_connections = max_connections.unwrap_or(DEFAULT_MAX_CONNECTIONS);
		if url.starts_with("postgres://") || url.starts_with("postgresql://") {
			let pool = PgPoolOptions::new()
				.max_connections(max_connections)
				.connect(url)
				.await
				.context("failed to connect postgres database")?;
			return Ok(Self::Postgres(pool));
		}

		let options = url
			.parse::<SqliteConnectOptions>()
			.context("failed to parse sqlite database URL")?
			.create_if_missing(true)
			.journal_mode(SqliteJournalMode::Wal)
			.synchronous(SqliteSynchronous::Normal)
			.busy_timeout(Duration::from_secs(5));
		let pool = SqlitePoolOptions::new()
			.max_connections(max_connections)
			.connect_with(options)
			.await
			.context("failed to connect sqlite database")?;
		Ok(Self::Sqlite(pool))
	}
}
