use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::AppError;
use crate::model::Incident;

use super::{Beat, HeartbeatStore, UptimeResult, Window};

/// Do-nothing store used until SQLite lands. Satisfies the `AppState` store dependency for
/// slices that don't touch history (e.g. the monitors slice). All methods are inert.
pub struct NoopStore;

impl NoopStore {
    pub fn new() -> Self {
        NoopStore
    }
}

#[async_trait]
impl HeartbeatStore for NoopStore {
    async fn record_beats(&self, _beats: &[Beat]) -> Result<(), AppError> {
        Ok(())
    }

    async fn uptime(&self, _monitor_id: i64, _window: Window) -> Result<UptimeResult, AppError> {
        Ok(UptimeResult {
            ratio: 0.0,
            coverage: 0.0,
        })
    }

    async fn incidents(&self, _since: DateTime<Utc>) -> Result<Vec<Incident>, AppError> {
        Ok(Vec::new())
    }
}
