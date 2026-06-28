use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Current operational state of a monitor.
/// Maps from Uptime Kuma heartbeat `status`: 0=down, 1=up, 2=pending, 3=maintenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonitorStatus {
    Up,
    Down,
    Pending,
    Maintenance,
}

/// Current status and latency of a single monitor (public API shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Monitor {
    /// i64 — SQLite/sqlx has no u64 codec; ids are small positive integers.
    pub id: i64,
    pub name: String,
    pub status: MonitorStatus,
    /// `None` when down / unknown.
    pub latency_ms: Option<u32>,
}

/// Uptime ratios over standard windows, with per-window data coverage (see low-level §3, §11).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UptimeWindow {
    /// i64 — SQLite/sqlx has no u64 codec; ids are small positive integers.
    pub monitor_id: i64,
    pub uptime_24h: f64,
    pub uptime_7d: f64,
    pub uptime_30d: f64,
    /// Data coverage in [0.0, 1.0]: stored-history span / requested window.
    pub coverage_7d: f64,
    pub coverage_30d: f64,
}

/// A period during which a monitor was down (derived locally; see low-level §4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    /// i64 — SQLite/sqlx has no u64 codec; ids are small positive integers.
    pub monitor_id: i64,
    pub started_at: DateTime<Utc>,
    /// `None` while the incident is ongoing.
    pub resolved_at: Option<DateTime<Utc>>,
    /// Denormalized convenience field: always set together with resolved_at (None while ongoing).
    pub duration_seconds: Option<u64>,
}

/// The full cached view served on the read path (low-level §5a).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub monitors: Vec<Monitor>,
    pub uptime: Vec<UptimeWindow>,
    pub incidents: Vec<Incident>,
    pub last_updated: DateTime<Utc>,
}
