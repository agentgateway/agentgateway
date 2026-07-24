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

impl DatabasePool {
	pub async fn connect(url: &str) -> anyhow::Result<Self> {
		if url.starts_with("postgres://") || url.starts_with("postgresql://") {
			let pool = PgPoolOptions::new()
				.max_connections(5)
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
			.max_connections(5)
			.connect_with(options)
			.await
			.context("failed to connect sqlite database")?;
		Ok(Self::Sqlite(pool))
	}
}
