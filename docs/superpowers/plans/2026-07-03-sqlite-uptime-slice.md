# SQLite history + `GET /api/uptime` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist heartbeats to SQLite and serve `GET /api/uptime` with 24h/7d/30d uptime + per-window coverage, computed in the poller and served from the in-memory snapshot.

**Architecture:** The poller records all returned beats into SQLite each tick, computes per-monitor uptime windows (24h from the status-page `uptimeList`, 7d/30d count-based from SQLite) with coverage, stores them in the `Snapshot`, and prunes old beats. The handler serves `snapshot.uptime` from cache. Runtime `sqlx` queries; no build-time DB.

**Tech Stack:** Rust 2024 (rustc 1.95.0), sqlx (sqlite, runtime-tokio), chrono, Tokio, Axum, serde. Dev: tempfile.

---

## Conventions

- **cargo is NOT on PATH** → always `~/.cargo/bin/cargo`.
- Branch **`development`** (already checked out, based on latest `main`). Commit per task.
- Crate root keeps `#![allow(dead_code, unused_variables, unused_imports)]` (redis/prometheus/incidents/auth still stubbed).
- TDD for pure/testable units (time parser, extractors, store logic, handler). Poller loop + `main` wiring are glue, verified live.
- **Time storage format:** fixed-width `"%Y-%m-%dT%H:%M:%S%.3fZ"` (always 3 fractional digits + `Z`). Fixed width ⇒ lexicographic string comparison equals chronological order, which the range queries rely on. RFC3339-parseable for reading `MIN(time)` back.

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `Cargo.toml` | modify | add `tempfile` dev-dependency |
| `.gitignore` | modify | ignore `/data/` (live DB file) |
| `src/store/mod.rs` | modify | add `prune` to `HeartbeatStore`; add `Window::days` |
| `src/store/noop.rs` | modify | implement `prune` (Ok) |
| `src/store/schema.sql` | real | `heartbeats` table + index |
| `src/store/sqlite.rs` | real | `SqliteStore`: connect/record_beats/uptime/prune/incidents + tests |
| `src/poller/status_page.rs` | modify | `BeatDto.time`, `uptimeList`, `parse_kuma_time`, `status_from_code`, `extract_beats`, `extract_uptime_24h`, `PolledData`; `fetch` returns `PolledData` |
| `src/poller/mod.rs` | modify | loop: record beats → build uptime windows → prune → snapshot |
| `src/api/uptime.rs` | real | handler serving `snapshot.uptime` |
| `src/main.rs` | modify | wire `SqliteStore` instead of `NoopStore` |

---

## Task 1: Setup — dep, gitignore, trait `prune`, `Window::days`

**Files:** `Cargo.toml`, `.gitignore`, `src/store/mod.rs`, `src/store/noop.rs`, `src/store/sqlite.rs`

- [ ] **Step 1: Add the dev-dependency**

Run: `~/.cargo/bin/cargo add --dev tempfile`
Expected: `tempfile` added under `[dev-dependencies]`.

- [ ] **Step 2: Ignore the live DB directory**

Append to `.gitignore`:

```
/data/
```

- [ ] **Step 3: Add `prune` to the trait and `Window::days`**

In `src/store/mod.rs`, add the `prune` method to the `HeartbeatStore` trait (after `incidents`):

```rust
    async fn prune(&self, older_than: DateTime<Utc>) -> Result<(), AppError>;
```

And add a `days` helper on the existing `Window` enum (place it just below the `enum Window` definition):

```rust
impl Window {
    /// Length of the window in days.
    pub fn days(self) -> i64 {
        match self {
            Window::Day => 1,
            Window::Week => 7,
            Window::Month => 30,
        }
    }
}
```

- [ ] **Step 4: Implement `prune` on `NoopStore`**

In `src/store/noop.rs`, add inside the `impl HeartbeatStore for NoopStore` block:

```rust
    async fn prune(&self, _older_than: DateTime<Utc>) -> Result<(), AppError> {
        Ok(())
    }
```

- [ ] **Step 5: Add a `prune` stub on `SqliteStore` (keeps the crate compiling)**

In `src/store/sqlite.rs`, inside the existing `impl HeartbeatStore for SqliteStore` block, add:

```rust
    async fn prune(&self, _older_than: DateTime<Utc>) -> Result<(), AppError> {
        todo!("implemented in Task 2")
    }
```

- [ ] **Step 6: Verify it compiles**

Run: `~/.cargo/bin/cargo build`
Expected: PASS (all `HeartbeatStore` impls now have `prune`).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src/store/mod.rs src/store/noop.rs src/store/sqlite.rs
git commit -m "chore: add prune to HeartbeatStore and tempfile dev-dep"
```

---

## Task 2: `SqliteStore` (schema, connect, record, uptime, prune) — TDD

**Files:** `src/store/schema.sql`, `src/store/sqlite.rs`

- [ ] **Step 1: Write the schema**

Replace `src/store/schema.sql` with:

```sql
CREATE TABLE IF NOT EXISTS heartbeats (
    monitor_id INTEGER NOT NULL,
    time       TEXT    NOT NULL,   -- fixed-width UTC "%Y-%m-%dT%H:%M:%S%.3fZ"
    status     INTEGER NOT NULL,   -- 0=down, 1=up, 2=pending, 3=maintenance
    ping_ms    INTEGER,
    PRIMARY KEY (monitor_id, time)
);
CREATE INDEX IF NOT EXISTS idx_heartbeats_monitor_time ON heartbeats (monitor_id, time);
```

- [ ] **Step 2: Write the failing tests**

Add at the bottom of `src/store/sqlite.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `~/.cargo/bin/cargo test sqlite`
Expected: FAIL — `SqliteStore::connect`/`record_beats`/`uptime` are still `todo!()` stubs (panic) and `pool` field doesn't exist yet.

- [ ] **Step 4: Implement `SqliteStore`**

Replace the ENTIRE non-test content of `src/store/sqlite.rs` (imports, struct, `impl SqliteStore`, `impl HeartbeatStore`) with the following, keeping the `#[cfg(test)] mod tests` from Step 2 at the bottom:

```rust
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
```

Note: if the resolved `sqlx` version lacks `sqlx::raw_sql`, replace that call by executing the two statements separately: `sqlx::query("CREATE TABLE ...").execute(&pool).await?` then the `CREATE INDEX`. Keep the intent (table + index created).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `~/.cargo/bin/cargo test sqlite`
Expected: PASS (4 passed).

- [ ] **Step 6: Commit**

```bash
git add src/store/sqlite.rs src/store/schema.sql
git commit -m "feat: implement SqliteStore (record, uptime, prune)"
```

---

## Task 3: Status-page beats/uptime extraction + poller integration — TDD

**Files:** `src/poller/status_page.rs`, `src/poller/mod.rs`

- [ ] **Step 1: Write the failing tests (status_page)**

In `src/poller/status_page.rs`, add these tests to the existing `#[cfg(test)] mod tests` block (keep the current `maps_status_latency_and_group` test and the `load()` helper):

```rust
    #[test]
    fn parses_kuma_utc_time() {
        let t = parse_kuma_time("2026-06-28 15:59:49.191").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-06-28T15:59:49.191+00:00");
    }

    #[test]
    fn rejects_bad_time() {
        assert!(parse_kuma_time("not a time").is_none());
    }

    #[test]
    fn extracts_beats_from_fixture() {
        let (_config, heartbeat) = load();
        let beats = extract_beats(&heartbeat);
        assert_eq!(beats.len(), 6); // ids 1(2) + 2(2) + 3(1) + 5(1)
        let m1: Vec<_> = beats.iter().filter(|b| b.monitor_id == 1).collect();
        assert_eq!(m1.len(), 2);
        assert!(m1
            .iter()
            .any(|b| b.status == MonitorStatus::Up && b.ping_ms == Some(7)));
    }

    #[test]
    fn extracts_uptime_24h_from_fixture() {
        let (_config, heartbeat) = load();
        let up = extract_uptime_24h(&heartbeat);
        assert_eq!(up.len(), 4);
        assert_eq!(up.get(&2).copied(), Some(0.98));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `~/.cargo/bin/cargo test status_page`
Expected: FAIL to compile — `parse_kuma_time`, `extract_beats`, `extract_uptime_24h` not found; `BeatDto` has no `time`; `HeartbeatDto` has no `uptimeList`.

- [ ] **Step 3: Implement the parsing additions**

In `src/poller/status_page.rs`:

(a) Extend the imports at the top to include what the new code needs:

```rust
use chrono::{DateTime, Utc};

use crate::store::Beat;
```

(b) Add a `time` field to `BeatDto` and a `uptimeList` map to `HeartbeatDto`:

```rust
#[derive(Debug, Deserialize)]
struct BeatDto {
    status: u8,
    ping: Option<f64>,
    time: String,
}
```

```rust
#[derive(Debug, Deserialize)]
struct HeartbeatDto {
    #[serde(rename = "heartbeatList")]
    heartbeat_list: HashMap<String, Vec<BeatDto>>,
    #[serde(rename = "uptimeList", default)]
    uptime_list: HashMap<String, f64>,
}
```

(c) Add a shared status mapper and use it in `map_monitors` (replace the inline `match b.status { ... }` inside `map_monitors` with a call to `status_from_code(b.status)`):

```rust
fn status_from_code(code: u8) -> MonitorStatus {
    match code {
        0 => MonitorStatus::Down,
        1 => MonitorStatus::Up,
        2 => MonitorStatus::Pending,
        3 => MonitorStatus::Maintenance,
        _ => MonitorStatus::Pending,
    }
}
```

The `status` line inside `map_monitors` becomes:

```rust
            let status = match last_beat {
                Some(b) => status_from_code(b.status),
                None => MonitorStatus::Pending,
            };
```

(d) Add the time parser and the two extractors:

```rust
/// Parse Uptime Kuma's `"%Y-%m-%d %H:%M:%S%.f"` (no timezone; treated as UTC).
fn parse_kuma_time(s: &str) -> Option<DateTime<Utc>> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .map(|ndt| ndt.and_utc())
}

/// All heartbeats across all monitors, mapped to domain `Beat`s. Unparseable rows are skipped.
fn extract_beats(heartbeat: &HeartbeatDto) -> Vec<Beat> {
    let mut beats = Vec::new();
    for (id_str, list) in &heartbeat.heartbeat_list {
        let Ok(monitor_id) = id_str.parse::<i64>() else {
            continue;
        };
        for b in list {
            let Some(time) = parse_kuma_time(&b.time) else {
                continue;
            };
            beats.push(Beat {
                monitor_id,
                time,
                status: status_from_code(b.status),
                ping_ms: b.ping.map(|p| p.round() as u32),
            });
        }
    }
    beats
}

/// The 24h uptime ratio per monitor id, from `uptimeList` keys shaped `"<id>_24"`.
fn extract_uptime_24h(heartbeat: &HeartbeatDto) -> HashMap<i64, f64> {
    let mut map = HashMap::new();
    for (key, value) in &heartbeat.uptime_list {
        if let Some(prefix) = key.strip_suffix("_24") {
            if let Ok(id) = prefix.parse::<i64>() {
                map.insert(id, *value);
            }
        }
    }
    map
}
```

(e) Add the `PolledData` struct and change `fetch` to return it. Replace the `fetch` method's final `Ok(map_monitors(&config, &heartbeat))` and its return type:

```rust
/// Everything one poll produces from the status-page endpoints.
pub struct PolledData {
    pub monitors: Vec<Monitor>,
    pub beats: Vec<Beat>,
    pub uptime_24h: HashMap<i64, f64>,
}
```

Change the signature to `pub async fn fetch(&self) -> Result<PolledData, AppError>` and its final expression to:

```rust
        Ok(PolledData {
            monitors: map_monitors(&config, &heartbeat),
            beats: extract_beats(&heartbeat),
            uptime_24h: extract_uptime_24h(&heartbeat),
        })
```

- [ ] **Step 4: Update the poller loop to consume `PolledData`**

Replace the body of `spawn` in `src/poller/mod.rs`. Full file content:

```rust
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};

use crate::model::{Snapshot, UptimeWindow};
use crate::state::AppState;
use crate::store::{HeartbeatStore, UptimeResult, Window};

use self::status_page::{PolledData, StatusPageClient};

pub mod incidents;
pub mod prometheus;
pub mod status_page;

/// Compute per-monitor uptime windows: 24h from the status page, 7d/30d from SQLite.
async fn build_uptime(store: &dyn HeartbeatStore, data: &PolledData) -> Vec<UptimeWindow> {
    let mut windows = Vec::new();
    for m in &data.monitors {
        let week = store
            .uptime(m.id, Window::Week)
            .await
            .unwrap_or(UptimeResult { ratio: 0.0, coverage: 0.0 });
        let month = store
            .uptime(m.id, Window::Month)
            .await
            .unwrap_or(UptimeResult { ratio: 0.0, coverage: 0.0 });
        windows.push(UptimeWindow {
            monitor_id: m.id,
            uptime_24h: data.uptime_24h.get(&m.id).copied().unwrap_or(0.0),
            uptime_7d: week.ratio,
            coverage_7d: week.coverage,
            uptime_30d: month.ratio,
            coverage_30d: month.coverage,
        });
    }
    windows
}

/// Spawns the background poll loop (low-level §4): fetch → persist beats → compute windows →
/// prune → replace the cached snapshot. A failed step logs `warn` and keeps the last snapshot.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = StatusPageClient::new(
            state.config.kuma_base_url.clone(),
            state.config.kuma_status_page_slug.clone(),
            state.http.clone(),
        );
        let mut ticker =
            tokio::time::interval(Duration::from_secs(state.config.poll_interval_seconds));
        let retention = ChronoDuration::days(state.config.history_retention_days as i64);

        loop {
            ticker.tick().await;
            match client.fetch().await {
                Ok(data) => {
                    if let Err(e) = state.store.record_beats(&data.beats).await {
                        tracing::warn!("failed to record beats: {e}");
                    }
                    let uptime = build_uptime(state.store.as_ref(), &data).await;
                    let snapshot = Snapshot {
                        monitors: data.monitors,
                        uptime,
                        incidents: Vec::new(),
                        last_updated: Utc::now(),
                    };
                    if let Err(e) = state.cache.put_snapshot(snapshot).await {
                        tracing::warn!("failed to store snapshot: {e}");
                    }
                    if let Err(e) = state.store.prune(Utc::now() - retention).await {
                        tracing::warn!("failed to prune old beats: {e}");
                    }
                }
                Err(e) => tracing::warn!("poll failed: {e}"),
            }
        }
    })
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `~/.cargo/bin/cargo test status_page`
Expected: PASS (existing mapper test + 4 new ones).

- [ ] **Step 6: Verify the whole crate builds**

Run: `~/.cargo/bin/cargo build`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/poller/status_page.rs src/poller/mod.rs
git commit -m "feat: extract beats + uptime_24h and record/compute in poller"
```

---

## Task 4: `GET /api/uptime` handler — TDD

**Files:** `src/api/uptime.rs`

- [ ] **Step 1: Write the failing tests**

Add at the bottom of `src/api/uptime.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use chrono::Utc;

    use crate::cache::memory::MemoryCache;
    use crate::cache::Cache;
    use crate::config::Config;
    use crate::model::Snapshot;
    use crate::store::noop::NoopStore;

    fn test_config() -> Config {
        Config {
            kuma_base_url: "http://example".into(),
            kuma_status_page_slug: "homelab".into(),
            poll_interval_seconds: 60,
            kuma_metrics_api_key: None,
            listen_addr: "0.0.0.0:8080".into(),
            api_key: None,
            cors_allowed_origins: vec![],
            database_url: "sqlite://memory".into(),
            history_retention_days: 31,
            redis_url: None,
        }
    }

    fn state_with(cache: Arc<dyn Cache>) -> AppState {
        AppState {
            cache,
            store: Arc::new(NoopStore::new()),
            config: Arc::new(test_config()),
            http: reqwest::Client::new(),
        }
    }

    fn window() -> UptimeWindow {
        UptimeWindow {
            monitor_id: 1,
            uptime_24h: 1.0,
            uptime_7d: 0.99,
            uptime_30d: 0.95,
            coverage_7d: 0.4,
            coverage_30d: 0.1,
        }
    }

    #[tokio::test]
    async fn returns_uptime_from_cache() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        cache
            .put_snapshot(Snapshot {
                monitors: vec![],
                uptime: vec![window()],
                incidents: vec![],
                last_updated: Utc::now(),
            })
            .await
            .unwrap();

        let Json(body) = handler(State(state_with(cache))).await.unwrap();
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].monitor_id, 1);
        assert_eq!(body[0].uptime_24h, 1.0);
        assert_eq!(body[0].coverage_30d, 0.1);
    }

    #[tokio::test]
    async fn returns_no_snapshot_error_when_empty() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        let err = handler(State(state_with(cache))).await.unwrap_err();
        assert!(matches!(err, AppError::NoSnapshot));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `~/.cargo/bin/cargo test uptime`
Expected: FAIL — handler is still `todo!()`.

- [ ] **Step 3: Implement the handler**

Replace the `handler` body in `src/api/uptime.rs` (keep its signature and the top `use` lines; keep the test module):

```rust
/// `GET /api/uptime` — uptime % over 24h / 7d / 30d windows with coverage (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<UptimeWindow>>, AppError> {
    match state.cache.get_snapshot().await {
        Some(snapshot) => Ok(Json(snapshot.uptime.clone())),
        None => Err(AppError::NoSnapshot),
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `~/.cargo/bin/cargo test uptime`
Expected: PASS (2 passed).

- [ ] **Step 5: Commit**

```bash
git add src/api/uptime.rs
git commit -m "feat: serve GET /api/uptime from cache"
```

---

## Task 5: Wire `SqliteStore` in `main.rs`

**Files:** `src/main.rs`

- [ ] **Step 1: Swap the store**

In `src/main.rs`, replace the import `use crate::store::noop::NoopStore;` with
`use crate::store::sqlite::SqliteStore;`, and replace the store construction line:

```rust
    let store: Arc<dyn store::HeartbeatStore> = Arc::new(NoopStore::new());
```

with:

```rust
    let store: Arc<dyn store::HeartbeatStore> = Arc::new(
        SqliteStore::connect(&config.database_url)
            .await
            .expect("failed to connect to SQLite store"),
    );
```

- [ ] **Step 2: Verify it builds**

Run: `~/.cargo/bin/cargo build`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire SqliteStore in main"
```

---

## Task 6: Full verification + live smoke test

**Files:** none (verification only).

- [ ] **Step 1: Test, clippy, fmt**

```bash
~/.cargo/bin/cargo test
~/.cargo/bin/cargo clippy --all-targets
~/.cargo/bin/cargo fmt --check
```
Expected: all tests PASS; clippy CLEAN; fmt no diff (if diff, run `~/.cargo/bin/cargo fmt` and commit).

- [ ] **Step 2: Live smoke test**

Start the server (writes `data/uptime.db`, gitignored):

```bash
KUMA_BASE_URL=https://uptime.samueleruaro.com \
KUMA_STATUS_PAGE_SLUG=homelab \
POLL_INTERVAL_SECONDS=10 \
LISTEN_ADDR=127.0.0.1:8088 \
DATABASE_URL=sqlite://data/uptime.db \
RUST_LOG=info \
~/.cargo/bin/cargo run
```

After ~15s (one poll), in another shell:

```bash
curl -s http://127.0.0.1:8088/api/uptime | python3 -m json.tool
curl -s http://127.0.0.1:8088/api/monitors | python3 -m json.tool | head
```

Expected: `/api/uptime` returns ~10 entries; each `uptime_24h ≈ 1.0`; `uptime_7d`/`uptime_30d`
present with **low** `coverage_7d`/`coverage_30d` on a fresh DB (a few beats vs a 7d/30d window).
`/api/monitors` still works. Confirm `data/uptime.db` was created. Stop the server (Ctrl-C).

- [ ] **Step 3: Commit any formatting changes**

```bash
git add -A
git commit -m "style: cargo fmt sqlite-uptime slice" || echo "nothing to commit"
```

---

## Self-Review Notes

- **Spec coverage:** schema + `SqliteStore` (§3/§4) → T2; `prune` on trait + NoopStore (§3) → T1;
  count-based ratio + coverage (§2/§4) → T2 `uptime`; 24h from `uptimeList`, 7d/30d from SQLite,
  computed in poller (§2/§6) → T3 `build_uptime` + extractors; UTC parsing restored (§5) → T3
  `parse_kuma_time`; `PolledData` (§5) → T3; `/api/uptime` from cache with 503 (§7) → T4; wiring
  `SqliteStore` (§8) → T5; testing incl. tempfile DB + live (§9) → T2/T4/T6; retention each tick
  (§2) → T3 poller loop + `prune`. Out-of-scope (incidents/redis/prometheus/auth) untouched;
  `incidents` returns `Ok(vec![])` (§4).
- **Design-doc reconciliation (§11):** the plan computes uptime in the poller and serves from
  cache; updating `low-level-analysis.md` §10 wording is a docs follow-up (note it at the end, not
  a code task) — the code already embodies the resolution.
- **Placeholder scan:** none; every code step is complete. The only remaining `todo!()` after T1
  is `SqliteStore::prune`, replaced in T2; no `todo!()` remains in this slice's surface after T2.
- **Type consistency:** `Window::days` (T1) used by `SqliteStore::uptime` (T2) and matches
  `Window` variants; `HeartbeatStore::prune(older_than)` (T1) implemented in NoopStore (T1) and
  SqliteStore (T2), called in poller (T3); `PolledData { monitors, beats, uptime_24h }` (T3) built
  by `fetch` and consumed by `build_uptime`/`spawn` (T3); `UptimeResult { ratio, coverage }`
  consistent T2↔T3; `Beat { monitor_id, time, status, ping_ms }` built in `extract_beats` (T3)
  matches the `store::Beat` definition; `UptimeWindow` fields match `model.rs` in T3 and T4.
```
