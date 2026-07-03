use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;

use crate::error::AppError;
use crate::model::{Incident, MonitorStatus};

use super::{Beat, HeartbeatStore, UptimeResult, Window};

/// SQLite-backed heartbeat history (low-level §5b). Written and read only by the poller task.
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn connect(database_url: &str) -> Result<Self, AppError> {
        ensure_parent_dir(database_url)?;
        let opts = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| AppError::Store(e.to_string()))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| AppError::Store(e.to_string()))?;
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await
            .map_err(|e| AppError::Store(e.to_string()))?;
        Ok(Self { pool })
    }
}

/// Fixed-width UTC timestamp so lexicographic ordering matches chronological ordering.
fn fmt_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn status_to_i64(s: MonitorStatus) -> i64 {
    match s {
        MonitorStatus::Down => 0,
        MonitorStatus::Up => 1,
        MonitorStatus::Pending => 2,
        MonitorStatus::Maintenance => 3,
    }
}

/// Ensure the parent directory of a file-based SQLite URL exists.
fn ensure_parent_dir(database_url: &str) -> Result<(), AppError> {
    let path = database_url.strip_prefix("sqlite://").unwrap_or(database_url);
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path.starts_with(':') {
        return Ok(()); // e.g. sqlite::memory:
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::Store(e.to_string()))?;
        }
    }
    Ok(())
}

#[async_trait]
impl HeartbeatStore for SqliteStore {
    async fn record_beats(&self, beats: &[Beat]) -> Result<(), AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::Store(e.to_string()))?;
        for b in beats {
            sqlx::query(
                "INSERT OR IGNORE INTO heartbeats (monitor_id, time, status, ping_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(b.monitor_id)
            .bind(fmt_time(b.time))
            .bind(status_to_i64(b.status))
            .bind(b.ping_ms.map(|p| p as i64))
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Store(e.to_string()))?;
        }
        tx.commit().await.map_err(|e| AppError::Store(e.to_string()))?;
        Ok(())
    }

    async fn uptime(&self, monitor_id: i64, window: Window) -> Result<UptimeResult, AppError> {
        let now = Utc::now();
        let window_dur = ChronoDuration::days(window.days());
        let start = now - window_dur;
        let cutoff = fmt_time(start);

        let row = sqlx::query(
            "SELECT COUNT(*) AS total, \
             COALESCE(SUM(CASE WHEN status = 1 THEN 1 ELSE 0 END), 0) AS up \
             FROM heartbeats WHERE monitor_id = ?1 AND time >= ?2",
        )
        .bind(monitor_id)
        .bind(&cutoff)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Store(e.to_string()))?;
        let total: i64 = row.get("total");
        let up: i64 = row.get("up");

        let oldest: Option<String> =
            sqlx::query_scalar("SELECT MIN(time) FROM heartbeats WHERE monitor_id = ?1")
                .bind(monitor_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AppError::Store(e.to_string()))?;

        let ratio = if total == 0 {
            0.0
        } else {
            up as f64 / total as f64
        };

        let coverage = match oldest {
            Some(s) => {
                let oldest_dt = DateTime::parse_from_rfc3339(&s)
                    .map_err(|e| AppError::Store(e.to_string()))?
                    .with_timezone(&Utc);
                let effective = if oldest_dt < start { start } else { oldest_dt };
                let covered = (now - effective).num_seconds().max(0) as f64;
                (covered / window_dur.num_seconds() as f64).clamp(0.0, 1.0)
            }
            None => 0.0,
        };

        Ok(UptimeResult { ratio, coverage })
    }

    async fn incidents(&self, _since: DateTime<Utc>) -> Result<Vec<Incident>, AppError> {
        Ok(Vec::new()) // implemented in the incidents slice
    }

    async fn prune(&self, older_than: DateTime<Utc>) -> Result<(), AppError> {
        sqlx::query("DELETE FROM heartbeats WHERE time < ?1")
            .bind(fmt_time(older_than))
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Store(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    use crate::model::MonitorStatus;

    async fn new_store() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("test.db").display());
        let store = SqliteStore::connect(&url).await.unwrap();
        (store, dir)
    }

    fn beat(monitor_id: i64, minutes_ago: i64, status: MonitorStatus) -> Beat {
        Beat {
            monitor_id,
            time: Utc::now() - Duration::minutes(minutes_ago),
            status,
            ping_ms: Some(10),
        }
    }

    #[tokio::test]
    async fn uptime_is_up_over_total() {
        let (store, _dir) = new_store().await;
        let mut beats = Vec::new();
        for i in 0..8 {
            beats.push(beat(1, i + 1, MonitorStatus::Up));
        }
        for i in 8..10 {
            beats.push(beat(1, i + 1, MonitorStatus::Down));
        }
        store.record_beats(&beats).await.unwrap();

        let r = store.uptime(1, Window::Week).await.unwrap();
        assert!((r.ratio - 0.8).abs() < 1e-9, "ratio was {}", r.ratio);
        assert!(r.coverage > 0.0 && r.coverage <= 1.0);
    }

    #[tokio::test]
    async fn dedup_ignores_duplicate_beats() {
        let (store, _dir) = new_store().await;
        let beats = vec![beat(1, 5, MonitorStatus::Up)];
        store.record_beats(&beats).await.unwrap();
        store.record_beats(&beats).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM heartbeats")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn empty_window_reports_zero() {
        let (store, _dir) = new_store().await;
        let r = store.uptime(999, Window::Month).await.unwrap();
        assert_eq!(r.ratio, 0.0);
        assert_eq!(r.coverage, 0.0);
    }

    #[tokio::test]
    async fn prune_removes_old_beats() {
        let (store, _dir) = new_store().await;
        store
            .record_beats(&[
                beat(1, 60 * 24 * 40, MonitorStatus::Up), // 40 days ago
                beat(1, 5, MonitorStatus::Up),            // recent
            ])
            .await
            .unwrap();
        store.prune(Utc::now() - Duration::days(31)).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM heartbeats")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
