use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::AppError;
use crate::model::Incident;

use super::{Beat, HeartbeatStore, UptimeResult, Window};

/// SQLite-backed heartbeat history (low-level §5b). Schema in `schema.sql`, applied at startup.
pub struct SqliteStore {
    // Will hold a `sqlx::SqlitePool`; wired in the store implementation task.
}

impl SqliteStore {
    pub async fn connect(database_url: &str) -> Result<Self, AppError> {
        todo!("open SqlitePool at {database_url} and run migrations")
    }
}

#[async_trait]
impl HeartbeatStore for SqliteStore {
    async fn record_beats(&self, beats: &[Beat]) -> Result<(), AppError> {
        todo!("upsert beats, dedup on (monitor_id, time)")
    }

    async fn uptime(&self, monitor_id: i64, window: Window) -> Result<UptimeResult, AppError> {
        todo!("aggregate stored beats into ratio + coverage")
    }

    async fn incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError> {
        todo!("reconstruct incidents from stored history")
    }
}
