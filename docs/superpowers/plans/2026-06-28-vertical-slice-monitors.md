# Vertical Slice: live `GET /api/monitors` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `GET /api/monitors` return live monitor data (name, group, status, latency) by polling the public Uptime Kuma status-page endpoints into an in-memory snapshot.

**Architecture:** A background Tokio task polls both status-page endpoints each interval, a pure mapper joins them into `Vec<Monitor>`, the result is stored in an `ArcSwap`-backed `MemoryCache`, and the handler serves it from cache (no upstream call on the request path). A throwaway `NoopStore` satisfies the `AppState` store dependency until SQLite lands.

**Tech Stack:** Rust 2024 (rustc 1.95.0), Axum, Tokio, reqwest (rustls), serde/serde_json, chrono, arc-swap, async-trait, tracing.

---

## Conventions

- **cargo is NOT on PATH** → always `~/.cargo/bin/cargo`.
- We are on branch **`development`** (not `main`). Commit per task with the given message.
- The crate root has `#![allow(dead_code, unused_variables, unused_imports)]` (still valid — sqlite/redis/prometheus/uptime/incidents/auth remain stubbed). Keep it.
- TDD where there is a pure unit to test (mapper, cache, handler). Thin glue (HTTP `fetch`, poll loop, `main` wiring) is verified by the final live run, not unit tests.
- No new dependencies are required.

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `src/model.rs` | modify | add `group: Option<String>` to `Monitor` |
| `tests/fixtures/status-page-config.json` | create | sanitized config fixture (names + group) |
| `tests/fixtures/status-page-heartbeat.json` | create | sanitized heartbeat fixture (status + ping) |
| `src/poller/status_page.rs` | real | internal DTOs, `map_monitors` (tested), `StatusPageClient::fetch` |
| `src/cache/memory.rs` | real | `MemoryCache` over `ArcSwapOption<Snapshot>` |
| `src/store/noop.rs` | create | `NoopStore: HeartbeatStore` (empty/Ok) |
| `src/store/mod.rs` | modify | `pub mod noop;` |
| `src/api/monitors.rs` | real | handler: 200 from cache, 503 when empty |
| `src/poller/mod.rs` | real | `spawn` interval loop |
| `src/main.rs` | real | wire cache + noop store + poller + server |

---

## Task 1: Add `group` field to `Monitor`

**Files:**
- Modify: `src/model.rs`

- [ ] **Step 1: Add the field**

In `src/model.rs`, the `Monitor` struct gains a `group` field. The full struct becomes:

```rust
/// Current status and latency of a single monitor (public API shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    /// Status-page group this monitor belongs to (e.g. "Servizi"); `None` if ungrouped.
    pub group: Option<String>,
    pub status: MonitorStatus,
    /// `None` when down / unknown.
    pub latency_ms: Option<u32>,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors).

- [ ] **Step 3: Commit**

```bash
git add src/model.rs
git commit -m "feat: add group field to Monitor"
```

---

## Task 2: Sanitized test fixtures

**Files:**
- Create: `tests/fixtures/status-page-config.json`
- Create: `tests/fixtures/status-page-heartbeat.json`

These are small hand-crafted fixtures (sanitized monitor names) that mirror the real payload
structure and cover every mapping case: up, down, maintenance, explicit pending (status 2), and
a monitor present in config but missing from the heartbeat list.

- [ ] **Step 1: Write `tests/fixtures/status-page-config.json`**

```json
{
  "config": { "slug": "homelab", "title": "Homelab Status" },
  "incidents": [],
  "maintenanceList": [],
  "publicGroupList": [
    {
      "name": "Servizi",
      "monitorList": [
        { "id": 1, "name": "service-a", "type": "http", "sendUrl": 0 },
        { "id": 2, "name": "service-b", "type": "ping", "sendUrl": 0 },
        { "id": 3, "name": "service-c", "type": "http", "sendUrl": 0 },
        { "id": 4, "name": "service-d", "type": "http", "sendUrl": 0 },
        { "id": 5, "name": "service-e", "type": "http", "sendUrl": 0 }
      ]
    }
  ]
}
```

- [ ] **Step 2: Write `tests/fixtures/status-page-heartbeat.json`**

Note: beats are in ascending time order; monitor `4` is intentionally absent.

```json
{
  "heartbeatList": {
    "1": [
      { "status": 1, "time": "2026-06-28 15:58:49.191", "msg": "", "ping": 5 },
      { "status": 1, "time": "2026-06-28 15:59:49.191", "msg": "", "ping": 7 }
    ],
    "2": [
      { "status": 1, "time": "2026-06-28 15:58:49.191", "msg": "", "ping": 12 },
      { "status": 0, "time": "2026-06-28 15:59:49.191", "msg": "timeout", "ping": 0 }
    ],
    "3": [
      { "status": 3, "time": "2026-06-28 15:59:49.191", "msg": "", "ping": 0 }
    ],
    "5": [
      { "status": 2, "time": "2026-06-28 15:59:49.191", "msg": "", "ping": 0 }
    ]
  },
  "uptimeList": { "1_24": 1, "2_24": 0.98, "3_24": 1, "5_24": 1 }
}
```

- [ ] **Step 3: Commit**

```bash
git add tests/fixtures/status-page-config.json tests/fixtures/status-page-heartbeat.json
git commit -m "test: add sanitized status-page fixtures"
```

---

## Task 3: Status-page DTOs + `map_monitors` (TDD)

**Files:**
- Modify: `src/poller/status_page.rs`

- [ ] **Step 1: Write the failing test**

Replace the contents of `src/poller/status_page.rs` with the test FIRST (implementation comes in Step 3). For now add only this at the bottom; the file already has the stub `StatusPageClient` — leave it for Task 6. Add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MonitorStatus;

    fn load() -> (StatusPageConfigDto, HeartbeatDto) {
        let config: StatusPageConfigDto =
            serde_json::from_str(include_str!("../../tests/fixtures/status-page-config.json"))
                .expect("config fixture parses");
        let heartbeat: HeartbeatDto =
            serde_json::from_str(include_str!("../../tests/fixtures/status-page-heartbeat.json"))
                .expect("heartbeat fixture parses");
        (config, heartbeat)
    }

    #[test]
    fn maps_status_latency_and_group() {
        let (config, heartbeat) = load();
        let monitors = map_monitors(&config, &heartbeat);
        let by_id = |id: i64| monitors.iter().find(|m| m.id == id).expect("monitor present");

        assert_eq!(monitors.len(), 5);

        // group passed through from publicGroupList
        assert_eq!(by_id(1).group.as_deref(), Some("Servizi"));

        // up: status + latency from the LAST beat (ascending order)
        assert_eq!(by_id(1).status, MonitorStatus::Up);
        assert_eq!(by_id(1).latency_ms, Some(7));

        // down: no latency
        assert_eq!(by_id(2).status, MonitorStatus::Down);
        assert_eq!(by_id(2).latency_ms, None);

        // maintenance (status 3)
        assert_eq!(by_id(3).status, MonitorStatus::Maintenance);
        assert_eq!(by_id(3).latency_ms, None);

        // explicit pending (status 2)
        assert_eq!(by_id(5).status, MonitorStatus::Pending);
        assert_eq!(by_id(5).latency_ms, None);

        // in config but missing from heartbeat -> pending, no latency
        assert_eq!(by_id(4).status, MonitorStatus::Pending);
        assert_eq!(by_id(4).latency_ms, None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `~/.cargo/bin/cargo test --lib maps_status_latency_and_group`
Expected: FAIL to **compile** — `map_monitors`, `StatusPageConfigDto`, `HeartbeatDto` not found.

- [ ] **Step 3: Implement DTOs + mapper**

At the TOP of `src/poller/status_page.rs` (above the existing `StatusPageClient` stub and the test module), add the imports, internal DTOs, and the mapper:

```rust
use std::collections::HashMap;

use serde::Deserialize;

use crate::model::{Monitor, MonitorStatus};

// --- Internal DTOs: deserialize the raw status-page JSON. Never leave this module. ---

#[derive(Debug, Deserialize)]
pub(crate) struct StatusPageConfigDto {
    #[serde(rename = "publicGroupList")]
    pub(crate) public_group_list: Vec<GroupDto>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroupDto {
    pub(crate) name: String,
    #[serde(rename = "monitorList")]
    pub(crate) monitor_list: Vec<MonitorDto>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MonitorDto {
    pub(crate) id: i64,
    pub(crate) name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeartbeatDto {
    #[serde(rename = "heartbeatList")]
    pub(crate) heartbeat_list: HashMap<String, Vec<BeatDto>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BeatDto {
    pub(crate) status: u8,
    pub(crate) ping: Option<f64>,
}

/// Join config (names + groups) and heartbeat (status + latency) into domain `Monitor`s.
/// "Latest" status is the LAST beat in each list (beats are in ascending time order).
pub(crate) fn map_monitors(config: &StatusPageConfigDto, heartbeat: &HeartbeatDto) -> Vec<Monitor> {
    let mut monitors = Vec::new();
    for group in &config.public_group_list {
        for m in &group.monitor_list {
            let last_beat = heartbeat
                .heartbeat_list
                .get(&m.id.to_string())
                .and_then(|beats| beats.last());

            let status = match last_beat {
                Some(b) => match b.status {
                    0 => MonitorStatus::Down,
                    1 => MonitorStatus::Up,
                    2 => MonitorStatus::Pending,
                    3 => MonitorStatus::Maintenance,
                    _ => MonitorStatus::Pending,
                },
                None => MonitorStatus::Pending,
            };

            let latency_ms = match (status, last_beat) {
                (MonitorStatus::Up, Some(b)) => b.ping.map(|p| p as u32),
                _ => None,
            };

            monitors.push(Monitor {
                id: m.id,
                name: m.name.clone(),
                group: Some(group.name.clone()),
                status,
                latency_ms,
            });
        }
    }
    monitors
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `~/.cargo/bin/cargo test --lib maps_status_latency_and_group`
Expected: PASS (1 passed).

- [ ] **Step 5: Commit**

```bash
git add src/poller/status_page.rs
git commit -m "feat: add status-page DTOs and monitor mapper"
```

---

## Task 4: Real `MemoryCache` (TDD)

**Files:**
- Modify: `src/cache/memory.rs`

- [ ] **Step 1: Write the failing test**

Add at the bottom of `src/cache/memory.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Snapshot;
    use chrono::Utc;

    fn empty_snapshot() -> Snapshot {
        Snapshot {
            monitors: vec![],
            uptime: vec![],
            incidents: vec![],
            last_updated: Utc::now(),
        }
    }

    #[tokio::test]
    async fn empty_until_put() {
        let cache = MemoryCache::new();
        assert!(cache.get_snapshot().await.is_none());
    }

    #[tokio::test]
    async fn returns_last_put_snapshot() {
        let cache = MemoryCache::new();
        cache.put_snapshot(empty_snapshot()).await.unwrap();
        assert!(cache.get_snapshot().await.is_some());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `~/.cargo/bin/cargo test --lib memory`
Expected: FAIL — `MemoryCache::new` / methods currently `todo!()` (panic) or empty struct without the field.

- [ ] **Step 3: Implement `MemoryCache`**

Replace the contents of `src/cache/memory.rs` (keep the test module from Step 1 at the bottom):

```rust
use std::sync::Arc;

use arc_swap::ArcSwapOption;
use async_trait::async_trait;

use crate::error::AppError;
use crate::model::Snapshot;

use super::Cache;

/// Default cache for single-instance deployments: lock-free reads via `ArcSwap` (low-level §5a).
pub struct MemoryCache {
    snapshot: ArcSwapOption<Snapshot>,
}

impl MemoryCache {
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwapOption::empty(),
        }
    }
}

#[async_trait]
impl Cache for MemoryCache {
    async fn get_snapshot(&self) -> Option<Snapshot> {
        self.snapshot.load_full().map(|arc| (*arc).clone())
    }

    async fn put_snapshot(&self, snap: Snapshot) -> Result<(), AppError> {
        self.snapshot.store(Some(Arc::new(snap)));
        Ok(())
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `~/.cargo/bin/cargo test --lib memory`
Expected: PASS (2 passed).

- [ ] **Step 5: Commit**

```bash
git add src/cache/memory.rs
git commit -m "feat: implement ArcSwap-backed MemoryCache"
```

---

## Task 5: `NoopStore`

**Files:**
- Create: `src/store/noop.rs`
- Modify: `src/store/mod.rs`

- [ ] **Step 1: Write `src/store/noop.rs`**

```rust
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
```

- [ ] **Step 2: Declare the module**

In `src/store/mod.rs`, add alongside the existing `pub mod sqlite;`:

```rust
pub mod noop;
```

- [ ] **Step 3: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/store/noop.rs src/store/mod.rs
git commit -m "feat: add NoopStore"
```

---

## Task 6: `StatusPageClient::fetch`

**Files:**
- Modify: `src/poller/status_page.rs`

The existing stub `StatusPageClient` (struct + `new` + `fetch` with `todo!()`) is replaced with a
real implementation. This is thin glue over the tested `map_monitors`; it is verified by the live
run, not a unit test.

- [ ] **Step 1: Replace the `StatusPageClient` stub**

Find the existing `StatusPageClient` struct and its `impl` block in `src/poller/status_page.rs`
(the one with `fetch` / `new` returning `todo!()`) and replace BOTH with:

```rust
use crate::error::AppError;

/// Primary data source client: the public status-page JSON endpoints (low-level §4). No auth.
pub struct StatusPageClient {
    base_url: String,
    slug: String,
    http: reqwest::Client,
}

impl StatusPageClient {
    pub fn new(base_url: String, slug: String, http: reqwest::Client) -> Self {
        Self {
            base_url,
            slug,
            http,
        }
    }

    /// Fetch both status-page endpoints and map them into the domain monitors.
    pub async fn fetch(&self) -> Result<Vec<Monitor>, AppError> {
        let config_url = format!("{}/api/status-page/{}", self.base_url, self.slug);
        let heartbeat_url = format!("{}/api/status-page/heartbeat/{}", self.base_url, self.slug);

        let config: StatusPageConfigDto = self
            .http
            .get(&config_url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::Parse(e.to_string()))?;

        let heartbeat: HeartbeatDto = self
            .http
            .get(&heartbeat_url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::Parse(e.to_string()))?;

        Ok(map_monitors(&config, &heartbeat))
    }
}
```

- [ ] **Step 2: Verify it compiles and tests still pass**

Run: `~/.cargo/bin/cargo test --lib`
Expected: PASS (mapper + cache tests; no regressions).

- [ ] **Step 3: Commit**

```bash
git add src/poller/status_page.rs
git commit -m "feat: implement StatusPageClient fetch"
```

---

## Task 7: `GET /api/monitors` handler (TDD)

**Files:**
- Modify: `src/api/monitors.rs`

- [ ] **Step 1: Write the failing test**

Add at the bottom of `src/api/monitors.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use chrono::Utc;

    use crate::cache::memory::MemoryCache;
    use crate::cache::Cache;
    use crate::config::Config;
    use crate::model::{Monitor, MonitorStatus, Snapshot};
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

    #[tokio::test]
    async fn returns_monitors_from_cache() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        let monitor = Monitor {
            id: 1,
            name: "service-a".into(),
            group: Some("Servizi".into()),
            status: MonitorStatus::Up,
            latency_ms: Some(7),
        };
        cache
            .put_snapshot(Snapshot {
                monitors: vec![monitor],
                uptime: vec![],
                incidents: vec![],
                last_updated: Utc::now(),
            })
            .await
            .unwrap();

        let Json(body) = handler(State(state_with(cache))).await.unwrap();
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].name, "service-a");
        assert_eq!(body[0].group.as_deref(), Some("Servizi"));
    }

    #[tokio::test]
    async fn returns_no_snapshot_error_when_empty() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        let err = handler(State(state_with(cache))).await.unwrap_err();
        assert!(matches!(err, AppError::NoSnapshot));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `~/.cargo/bin/cargo test --lib monitors`
Expected: FAIL — handler currently `todo!()` (panics) so `returns_monitors_from_cache` panics.

- [ ] **Step 3: Implement the handler**

Replace the `handler` function body in `src/api/monitors.rs` (keep its existing signature and the
`use` lines at the top; keep the test module from Step 1):

```rust
/// `GET /api/monitors` — current status and latency of each monitor. Served from cache (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Monitor>>, AppError> {
    match state.cache.get_snapshot().await {
        Some(snapshot) => Ok(Json(snapshot.monitors)),
        None => Err(AppError::NoSnapshot),
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `~/.cargo/bin/cargo test --lib monitors`
Expected: PASS (2 passed).

- [ ] **Step 5: Commit**

```bash
git add src/api/monitors.rs
git commit -m "feat: serve GET /api/monitors from cache"
```

---

## Task 8: Poller loop (`spawn`)

**Files:**
- Modify: `src/poller/mod.rs`

Thin glue; verified by the live run.

- [ ] **Step 1: Implement `spawn`**

Replace the `spawn` function in `src/poller/mod.rs` (keep the `pub mod` declarations at the top):

```rust
use std::time::Duration;

use chrono::Utc;

use crate::model::Snapshot;
use crate::state::AppState;

use self::status_page::StatusPageClient;

/// Spawns the background poll loop (low-level §4): each tick, fetch the status-page endpoints,
/// build a snapshot, and replace the cached snapshot. A failed poll logs `warn` and leaves the
/// previous snapshot in place.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = StatusPageClient::new(
            state.config.kuma_base_url.clone(),
            state.config.kuma_status_page_slug.clone(),
            state.http.clone(),
        );
        let mut ticker =
            tokio::time::interval(Duration::from_secs(state.config.poll_interval_seconds));

        loop {
            ticker.tick().await;
            match client.fetch().await {
                Ok(monitors) => {
                    let snapshot = Snapshot {
                        monitors,
                        uptime: Vec::new(),
                        incidents: Vec::new(),
                        last_updated: Utc::now(),
                    };
                    if let Err(e) = state.cache.put_snapshot(snapshot).await {
                        tracing::warn!("failed to store snapshot: {e}");
                    }
                }
                Err(e) => tracing::warn!("poll failed: {e}"),
            }
        }
    })
}
```

Note: the existing `pub mod incidents; pub mod prometheus; pub mod status_page;` lines at the top
of the file stay. Remove the old `use crate::state::AppState;` line if it now duplicates the one
above (keep a single import).

- [ ] **Step 2: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/poller/mod.rs
git commit -m "feat: implement status-page poll loop"
```

---

## Task 9: Wire `main.rs`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Update the wiring**

In `src/main.rs`, change the store from `SqliteStore` to `NoopStore` and give the HTTP client a
timeout. Replace the import line `use crate::store::sqlite::SqliteStore;` with
`use crate::store::noop::NoopStore;`, and replace the body of `main` (from the `let config` line
to the end) with:

```rust
    let config = Arc::new(Config::load().expect("failed to load config"));

    let cache: Arc<dyn cache::Cache> = Arc::new(MemoryCache::new());
    let store: Arc<dyn store::HeartbeatStore> = Arc::new(NoopStore::new());

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client");

    let state = AppState {
        cache,
        store,
        config: config.clone(),
        http,
    };

    let _poll_handle = poller::spawn(state.clone());

    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listen address");

    tracing::info!("listening on {}", config.listen_addr);
    axum::serve(listener, app).await.expect("server error");
```

- [ ] **Step 2: Verify it compiles**

Run: `~/.cargo/bin/cargo build`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire monitors slice in main"
```

---

## Task 10: Full verification + live smoke test

**Files:** none (verification only).

- [ ] **Step 1: Test, clippy, fmt**

```bash
~/.cargo/bin/cargo test
~/.cargo/bin/cargo clippy --all-targets
~/.cargo/bin/cargo fmt --check
```
Expected: tests PASS; clippy CLEAN (no warnings); fmt no diff (if it reports a diff, run
`~/.cargo/bin/cargo fmt` and commit the result).

- [ ] **Step 2: Live smoke test against the real instance**

Start the server (env vars provide config; nothing is committed):

```bash
KUMA_BASE_URL=https://uptime.samueleruaro.com \
KUMA_STATUS_PAGE_SLUG=homelab \
POLL_INTERVAL_SECONDS=10 \
LISTEN_ADDR=127.0.0.1:8080 \
RUST_LOG=info \
~/.cargo/bin/cargo run
```

In another shell, after ~10s (one poll cycle):

```bash
curl -s http://127.0.0.1:8080/api/monitors | python3 -m json.tool
```

Expected: a JSON array of ~10 monitors, each with `id`, `name`, `group: "Servizi"`, `status`
(`up`/`down`/…), and `latency_ms`. A request before the first poll returns HTTP 503.

Stop the server (Ctrl-C) when done.

- [ ] **Step 3: Commit any formatting changes**

```bash
git add -A
git commit -m "style: cargo fmt monitors slice" || echo "nothing to commit"
```

---

## Self-Review Notes

- **Spec coverage:** goal + success criteria → Tasks 1–10; data flow (poll→map→cache→serve) →
  T3/T4/T6/T8/T7; mapping rules (status, latency Up-only, group, missing→Pending) → T3 mapper +
  tests; `group` field → T1; sanitized fixtures + real gitignored → T2 (gitignore already set);
  `MemoryCache` ArcSwap → T4; `NoopStore` store dependency → T5; failure isolation (warn, keep
  snapshot) → T8; 503 when empty → T7; live verification → T10. Out-of-scope items (SQLite,
  uptime, incidents, redis, prometheus, auth, CORS) are untouched and remain stubbed.
- **Deviation from spec §8 (logged):** the spec listed a "UTC time parsing" test. This slice
  selects latest status by array position (beats are ascending), so it does not parse beat
  timestamps at all — time parsing is deferred to the SQLite/incidents plan where it is actually
  needed (YAGNI). `BeatDto` therefore omits a `time` field (unknown JSON fields are ignored by
  serde).
- **Placeholder scan:** none — every code step shows complete, compilable code; `todo!()` remains
  only in genuinely out-of-scope stubs (sqlite, redis, prometheus, uptime/incidents handlers, auth).
- **Type consistency:** `map_monitors(&StatusPageConfigDto, &HeartbeatDto) -> Vec<Monitor>` used
  identically in T3 and T6; `MemoryCache::new` + `put_snapshot(...) -> Result` (T4) match the
  `Cache` trait and the T7/T8 call sites; `NoopStore::new` (T5) matches T7/T9 usage;
  `StatusPageClient::new(base_url, slug, http)` (T6) matches T8; `poller::spawn(state) ->
  JoinHandle` (T8) matches T9. `Monitor` fields (id, name, group, status, latency_ms) consistent
  across T1/T3/T7.
