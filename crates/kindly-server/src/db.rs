//! Database pool and migrations.

use crate::errors::ApiResult;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::time::Duration;

pub async fn connect(database_url: &str) -> ApiResult<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)
        .map_err(|e| crate::errors::ApiError::Internal(format!("database url: {e}")))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(|e| crate::errors::ApiError::Internal(format!("database connect: {e}")))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| crate::errors::ApiError::Internal(format!("migrations: {e}")))?;

    Ok(pool)
}
