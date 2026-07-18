# Incident derivation + `GET /api/incidents` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reconstruct incident history from stored heartbeats in SQLite and serve it via `GET /api/incidents`, computed in the poller and served from the in-memory snapshot (same pattern as `/api/monitors` and `/api/uptime`).

**Architecture:** `SqliteStore::incidents` runs one SQL query with a `LAG` window function over `heartbeats` to find status-transition rows, then walks them in Rust to pair up open/close transitions into `Incident` records (open incidents have `resolved_at: None`). The poller calls this each tick and stores the result on `Snapshot.incidents`; the handler serves it straight from cache. The old two-poll in-memory diff stub (`poller/incidents.rs`) is deleted — superseded by the SQL reconstruction.

**Tech Stack:** Rust 2024 (rustc 1.95.0), sqlx (sqlite, runtime-tokio), chrono, Tokio, Axum, serde. Dev: tempfile (already a dev-dependency).

---

## Conventions

- **cargo is NOT on PATH** → always `~/.cargo/bin/cargo`.
- Branch **`development`** (already checked out, fast-forwarded to `main`'s tip). Commit per task.
- Crate root keeps `#![allow(dead_code, unused_variables, unused_imports)]` (redis/prometheus/auth still stubbed).
- TDD for the store logic and the handler. Poller wiring is glue, verified live.
- Reuse the existing time format: heartbeats are stored as fixed-width `"%Y-%m-%dT%H:%M:%S%.3fZ"`; read back with `DateTime::parse_from_rfc3339` (same as `SqliteStore::uptime`'s `oldest` lookup).
- `MonitorStatus` → integer mapping (already defined in `store/sqlite.rs`'s `status_to_i64`): `0=down, 1=up, 2=pending, 3=maintenance`. Only `0` (`Down`) matters for transition detection — the query only needs to know "is this row `Down`", not decode the other three values.

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `src/store/sqlite.rs` | modify | real `incidents(since)` implementation + tests |
| `src/poller/mod.rs` | modify | call `store.incidents`, set `Snapshot.incidents`; drop `pub mod incidents;` |
| `src/poller/incidents.rs` | delete | superseded by SQL reconstruction in `SqliteStore::incidents` |
| `src/api/incidents.rs` | modify | real handler serving `snapshot.incidents` + tests |

---

## Task 1: `SqliteStore::incidents` — TDD

**Files:** `src/store/sqlite.rs`

**Interfaces:**
- Consumes: existing `HeartbeatStore::incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError>` trait signature (`store/mod.rs`, unchanged); `Incident { monitor_id: i64, started_at: DateTime<Utc>, resolved_at: Option<DateTime<Utc>>, duration_seconds: Option<u64> }` (`model.rs`, unchanged); the `heartbeats(monitor_id, time, status, ping_ms)` table (`schema.sql`, unchanged); `fmt_time`/`beat` test helpers already in `store/sqlite.rs`.
- Produces: a working `incidents` query other tasks (poller) call directly — no new public types.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/store/sqlite.rs` (after the existing `prune_removes_old_beats` test):

```rust
    #[tokio::test]
    async fn closed_incident_has_started_resolved_and_duration() {
        let (store, _dir) = new_store().await;
        store
            .record_beats(&[
                beat(1, 30, MonitorStatus::Up),
                beat(1, 20, MonitorStatus::Down),
                beat(1, 10, MonitorStatus::Up),
            ])
            .await
            .unwrap();

        let incidents = store.incidents(Utc::now() - Duration::hours(1)).await.unwrap();

        assert_eq!(incidents.len(), 1);
        let inc = &incidents[0];
        assert_eq!(inc.monitor_id, 1);
        assert!(inc.resolved_at.is_some());
        assert_eq!(
            inc.duration_seconds,
            Some((inc.resolved_at.unwrap() - inc.started_at).num_seconds() as u64)
        );
        assert!((inc.resolved_at.unwrap() - inc.started_at).num_minutes() >= 9);
    }

    #[tokio::test]
    async fn ongoing_incident_has_no_resolved_at() {
        let (store, _dir) = new_store().await;
        store
            .record_beats(&[
                beat(1, 30, MonitorStatus::Up),
                beat(1, 20, MonitorStatus::Down),
            ])
            .await
            .unwrap();

        let incidents = store.incidents(Utc::now() - Duration::hours(1)).await.unwrap();

        assert_eq!(incidents.len(), 1);
        assert_eq!(incidents[0].resolved_at, None);
        assert_eq!(incidents[0].duration_seconds, None);
    }

    #[tokio::test]
    async fn first_ever_beat_down_still_opens_an_incident() {
        let (store, _dir) = new_store().await;
        store
            .record_beats(&[beat(1, 5, MonitorStatus::Down)])
            .await
            .unwrap();

        let incidents = store.incidents(Utc::now() - Duration::hours(1)).await.unwrap();

        assert_eq!(incidents.len(), 1);
        assert_eq!(incidents[0].resolved_at, None);
    }

    #[tokio::test]
    async fn pending_and_maintenance_never_open_or_close_incidents() {
        let (store, _dir) = new_store().await;
        store
            .record_beats(&[
                beat(1, 40, MonitorStatus::Up),
                beat(1, 30, MonitorStatus::Pending),
                beat(1, 20, MonitorStatus::Maintenance),
                beat(1, 10, MonitorStatus::Up),
            ])
            .await
            .unwrap();

        let incidents = store.incidents(Utc::now() - Duration::hours(1)).await.unwrap();

        assert_eq!(incidents.len(), 0);
    }

    #[tokio::test]
    async fn since_excludes_incidents_resolved_before_it_but_keeps_ongoing_ones() {
        let (store, _dir) = new_store().await;
        store
            .record_beats(&[
                // Monitor 1: resolved long ago — should be excluded by `since`.
                beat(1, 500, MonitorStatus::Up),
                beat(1, 490, MonitorStatus::Down),
                beat(1, 480, MonitorStatus::Up),
                // Monitor 2: still down — must be included regardless of `since`.
                beat(2, 60, MonitorStatus::Up),
                beat(2, 50, MonitorStatus::Down),
            ])
            .await
            .unwrap();

        let incidents = store
            .incidents(Utc::now() - Duration::minutes(100))
            .await
            .unwrap();

        assert_eq!(incidents.len(), 1);
        assert_eq!(incidents[0].monitor_id, 2);
    }

    #[tokio::test]
    async fn two_monitors_are_tracked_independently() {
        let (store, _dir) = new_store().await;
        store
            .record_beats(&[
                beat(1, 30, MonitorStatus::Up),
                beat(1, 20, MonitorStatus::Down),
                beat(1, 10, MonitorStatus::Up),
                beat(2, 25, MonitorStatus::Down),
                beat(2, 15, MonitorStatus::Up),
            ])
            .await
            .unwrap();

        let mut incidents = store.incidents(Utc::now() - Duration::hours(1)).await.unwrap();
        incidents.sort_by_key(|i| i.monitor_id);

        assert_eq!(incidents.len(), 2);
        assert_eq!(incidents[0].monitor_id, 1);
        assert_eq!(incidents[1].monitor_id, 2);
        assert!(incidents.iter().all(|i| i.resolved_at.is_some()));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `~/.cargo/bin/cargo test --lib incidents`
Expected: FAIL — `incidents` currently returns `Ok(Vec::new())`, so `closed_incident_has_started_resolved_and_duration`, `ongoing_incident_has_no_resolved_at`, `first_ever_beat_down_still_opens_an_incident`, and `two_monitors_are_tracked_independently` all fail on `assert_eq!(incidents.len(), 1 or 2)`; `pending_and_maintenance_never_open_or_close_incidents` and `since_excludes_incidents_resolved_before_it_but_keeps_ongoing_ones` happen to pass already (empty is a subset of what they check) — that's fine, they'll still pass after the real implementation.

- [ ] **Step 3: Implement `incidents`**

In `src/store/sqlite.rs`, replace:

```rust
    async fn incidents(&self, _since: DateTime<Utc>) -> Result<Vec<Incident>, AppError> {
        Ok(Vec::new()) // implemented in the incidents slice
    }
```

with:

```rust
    async fn incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError> {
        let rows = sqlx::query(
            "WITH ordered AS ( \
                SELECT monitor_id, time, status, \
                       LAG(status) OVER (PARTITION BY monitor_id ORDER BY time) AS prev_status \
                FROM heartbeats \
             ) \
             SELECT monitor_id, time, status \
             FROM ordered \
             WHERE (status = 0 AND (prev_status IS NULL OR prev_status != 0)) \
                OR (prev_status = 0 AND status != 0) \
             ORDER BY monitor_id, time",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Store(e.to_string()))?;

        let mut open: std::collections::HashMap<i64, DateTime<Utc>> = std::collections::HashMap::new();
        let mut incidents = Vec::new();
        for row in rows {
            let monitor_id: i64 = row.get("monitor_id");
            let time_str: String = row.get("time");
            let status: i64 = row.get("status");
            let time = DateTime::parse_from_rfc3339(&time_str)
                .map_err(|e| AppError::Store(e.to_string()))?
                .with_timezone(&Utc);

            if status == 0 {
                open.insert(monitor_id, time);
            } else if let Some(started_at) = open.remove(&monitor_id) {
                let duration = (time - started_at).num_seconds().max(0) as u64;
                incidents.push(Incident {
                    monitor_id,
                    started_at,
                    resolved_at: Some(time),
                    duration_seconds: Some(duration),
                });
            }
        }
        for (monitor_id, started_at) in open {
            incidents.push(Incident {
                monitor_id,
                started_at,
                resolved_at: None,
                duration_seconds: None,
            });
        }

        incidents.retain(|i| i.resolved_at.is_none_or(|r| r >= since));
        incidents.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(incidents)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `~/.cargo/bin/cargo test --lib incidents`
Expected: PASS — all 6 tests green.

- [ ] **Step 5: Commit**

```bash
git add src/store/sqlite.rs
git commit -m "feat: implement SqliteStore::incidents via SQL transition detection"
```

---

## Task 2: Poller wiring — drop the diff stub, populate `Snapshot.incidents`

**Files:** `src/poller/mod.rs`, `src/poller/incidents.rs` (deleted)

**Interfaces:**
- Consumes: `HeartbeatStore::incidents(since: DateTime<Utc>) -> Result<Vec<Incident>, AppError>` (Task 1); existing `retention` (`ChronoDuration`) and `state.store` already in scope in `spawn`.
- Produces: `Snapshot.incidents` populated from real data — Task 3's handler reads it.

- [ ] **Step 1: Delete the superseded diff stub**

```bash
rm src/poller/incidents.rs
```

In `src/poller/mod.rs`, remove the line:

```rust
pub mod incidents;
```

- [ ] **Step 2: Populate `Snapshot.incidents` in the poll loop**

In `src/poller/mod.rs`, inside `spawn`'s `Ok(data) => { ... }` branch, after the existing `let uptime = build_uptime(...).await;` line and before the `let snapshot = Snapshot { ... };` line, add:

```rust
                    let incidents = state
                        .store
                        .incidents(Utc::now() - retention)
                        .await
                        .inspect_err(|e| tracing::warn!("incidents query failed: {e}"))
                        .unwrap_or_default();
```

Then change the `Snapshot` construction from:

```rust
                    let snapshot = Snapshot {
                        monitors: data.monitors,
                        uptime,
                        incidents: Vec::new(),
                        last_updated: Utc::now(),
                    };
```

to:

```rust
                    let snapshot = Snapshot {
                        monitors: data.monitors,
                        uptime,
                        incidents,
                        last_updated: Utc::now(),
                    };
```

- [ ] **Step 3: Verify it builds**

Run: `~/.cargo/bin/cargo build`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/poller/mod.rs
git rm src/poller/incidents.rs
git commit -m "feat: populate Snapshot.incidents from SqliteStore in the poll loop"
```

---

## Task 3: `GET /api/incidents` handler — TDD

**Files:** `src/api/incidents.rs`

**Interfaces:**
- Consumes: `AppState { cache: Arc<dyn Cache>, .. }` (`state.rs`, unchanged); `Cache::get_snapshot(&self) -> Option<Arc<Snapshot>>` (`cache/mod.rs`, unchanged); `AppError::NoSnapshot` (`error.rs`, unchanged); `Snapshot.incidents: Vec<Incident>` (`model.rs`, unchanged, now populated by Task 2).
- Produces: `pub async fn handler(...)` wired at `/api/incidents` in `api/mod.rs` (already routed — no change needed there).

- [ ] **Step 1: Write the failing tests**

Replace the entire contents of `src/api/incidents.rs` with:

```rust
use axum::Json;
use axum::extract::State;

use crate::error::AppError;
use crate::model::Incident;
use crate::state::AppState;

/// `GET /api/incidents` — history of incidents (monitors that went down) (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Incident>>, AppError> {
    todo!("read snapshot from cache and return incidents")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use chrono::Utc;

    use crate::cache::Cache;
    use crate::cache::memory::MemoryCache;
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

    fn incident() -> Incident {
        Incident {
            monitor_id: 1,
            started_at: Utc::now(),
            resolved_at: None,
            duration_seconds: None,
        }
    }

    #[tokio::test]
    async fn returns_incidents_from_cache() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        cache
            .put_snapshot(Snapshot {
                monitors: vec![],
                uptime: vec![],
                incidents: vec![incident()],
                last_updated: Utc::now(),
            })
            .await
            .unwrap();

        let Json(body) = handler(State(state_with(cache))).await.unwrap();
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].monitor_id, 1);
        assert_eq!(body[0].resolved_at, None);
    }

    #[tokio::test]
    async fn returns_no_snapshot_error_when_empty() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        let err = handler(State(state_with(cache))).await.unwrap_err();
        assert!(matches!(err, AppError::NoSnapshot));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `~/.cargo/bin/cargo test --lib api::incidents`
Expected: FAIL — panics with `not yet implemented: read snapshot from cache and return incidents` (the `todo!()`).

- [ ] **Step 3: Implement the handler**

In `src/api/incidents.rs`, replace:

```rust
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Incident>>, AppError> {
    todo!("read snapshot from cache and return incidents")
}
```

with:

```rust
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Incident>>, AppError> {
    match state.cache.get_snapshot().await {
        Some(snapshot) => Ok(Json(snapshot.incidents.clone())),
        None => Err(AppError::NoSnapshot),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `~/.cargo/bin/cargo test --lib api::incidents`
Expected: PASS — both tests green.

- [ ] **Step 5: Commit**

```bash
git add src/api/incidents.rs
git commit -m "feat: serve GET /api/incidents from cache"
```

---

## Task 4: Full verification + live smoke test

**Files:** none (verification only).

- [ ] **Step 1: Test, clippy, fmt**

```bash
~/.cargo/bin/cargo test
~/.cargo/bin/cargo clippy --all-targets
~/.cargo/bin/cargo fmt --check
```
Expected: all tests PASS (existing 15 + this slice's 8 new ones = 23); clippy CLEAN; fmt no diff (if diff, run `~/.cargo/bin/cargo fmt` and commit as a `style:` commit).

- [ ] **Step 2: Live smoke test over Tailscale**

Start the server (reuses `data/uptime.db` from the previous slice's run, gitignored):

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
curl -s http://127.0.0.1:8088/api/incidents | python3 -m json.tool
```

Expected: `200` with a JSON array (possibly empty if every monitor has been up for the whole
retained history — that's a valid outcome, not a bug). If `data/uptime.db` already has a recorded
`Down` beat from an earlier session, expect at least one entry with `started_at`/`resolved_at` (or
`resolved_at: null` if it's still down). Cross-check monitor ids against `curl -s
http://127.0.0.1:8088/api/monitors`. Stop the server (Ctrl-C).

- [ ] **Step 3: Commit any formatting changes**

```bash
git add -A
git commit -m "style: cargo fmt incidents slice" || echo "nothing to commit"
```

---

## Self-Review Notes

- **Spec coverage:** SQL-based reconstruction from stored heartbeats (§2/§4) → Task 1; only
  `Down` transitions open/close, `Pending`/`Maintenance` inert (§2) → Task 1
  `pending_and_maintenance_never_open_or_close_incidents`; first-beat-`Down` edge case (§2) →
  Task 1 `first_ever_beat_down_still_opens_an_incident`; `since` semantics (§2) → Task 1
  `since_excludes_incidents_resolved_before_it_but_keeps_ongoing_ones`; ordering most-recent-first
  (§2) → Task 1 `incidents.sort_by`; computed in poller, served from cache (§5/§6) → Task 2/Task
  3; removal of `poller/incidents.rs` (§2/§5) → Task 2; `503` until first snapshot (§1) → Task 3
  `returns_no_snapshot_error_when_empty`; live verification (§8) → Task 4.
- **Placeholder scan:** none; every code step is complete and every test has real assertions.
- **Type consistency:** `HeartbeatStore::incidents(since: DateTime<Utc>) -> Result<Vec<Incident>, AppError>`
  (unchanged trait signature) implemented in Task 1, called in Task 2 as
  `state.store.incidents(Utc::now() - retention)`, and its output flows into
  `Snapshot.incidents: Vec<Incident>` (`model.rs`, unchanged) read by the Task 3 handler as
  `snapshot.incidents.clone()` — same shape end to end. `Incident` field names
  (`monitor_id`, `started_at`, `resolved_at`, `duration_seconds`) match `model.rs` throughout.
