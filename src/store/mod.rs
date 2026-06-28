use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::AppError;
use crate::model::{Incident, MonitorStatus};

pub mod sqlite;

/// A single recorded heartbeat to be persisted (low-level §5b).
#[derive(Debug, Clone)]
pub struct Beat {
    /// i64 — SQLite/sqlx has no u64 codec; ids are small positive integers.
    pub monitor_id: i64,
    pub time: DateTime<Utc>,
    pub status: MonitorStatus,
    pub ping_ms: Option<u32>,
}

/// An uptime window to aggregate over.
#[derive(Debug, Clone, Copy)]
pub enum Window {
    Day,
    Week,
    Month,
}

/// Result of an uptime query: the ratio plus how much of the window is actually backed by data.
#[derive(Debug, Clone, Copy)]
pub struct UptimeResult {
    pub ratio: f64,
    pub coverage: f64,
}

/// Durable heartbeat history used to compute 7d/30d uptime and reconstruct incidents (low-level §5b).
#[async_trait]
pub trait HeartbeatStore: Send + Sync {
    async fn record_beats(&self, beats: &[Beat]) -> Result<(), AppError>;
    async fn uptime(&self, monitor_id: i64, window: Window) -> Result<UptimeResult, AppError>;
    async fn incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError>;
}
