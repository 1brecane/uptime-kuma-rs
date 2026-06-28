# Project Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scaffold the `uptime-kuma-rs` binary crate so its module tree mirrors low-level-analysis §1, declares the §2 dependencies, and compiles cleanly with real type/trait/config definitions and `todo!()`-stubbed behavior.

**Architecture:** Single binary crate. Real definitions now (domain model, error type, config loader, `Cache`/`HeartbeatStore` traits, `AppState`); all behavior (poller, cache impls, sqlite impl, HTTP handlers, auth) stubbed with `todo!()`. Optional backends (Redis, Prometheus) are compiled in but inert — selected at runtime by config presence, not Cargo features.

**Tech Stack:** Rust 2024 edition (rustc 1.95.0), Axum, Tokio, reqwest, serde, chrono, figment, sqlx (sqlite), redis, tracing, tower-http, thiserror, arc-swap, async-trait.

---

## Conventions for this plan

- **cargo is not on PATH.** Always invoke it as `~/.cargo/bin/cargo`.
- **Verification gate = compilation.** The spec (§7) defers unit tests. Each task's "test" is
  `~/.cargo/bin/cargo check` (fast) and, at the end, `clippy` + `fmt`. There are no runtime
  assertions to write yet.
- **Stub discipline.** Behavior bodies are `todo!()`. To keep clippy clean while structs/fields
  are defined-but-not-yet-constructed, the crate root carries a clearly-marked temporary
  `#![allow(dead_code, unused_variables, unused_imports)]` that later implementation plans remove
  module by module.
- **Commit after every task.**

## File Structure

| File | Responsibility | State |
| --- | --- | --- |
| `Cargo.toml` | crate metadata + dependencies (§2) | real |
| `src/main.rs` | crate-root attrs, `mod` decls, logging init, minimal router bind | real wiring / stub routes |
| `src/model.rs` | domain types: `MonitorStatus`, `Monitor`, `UptimeWindow`, `Incident`, `Snapshot` | real |
| `src/error.rs` | `AppError` enum + `IntoResponse` | real |
| `src/config.rs` | `Config` struct + figment loader | real |
| `src/state.rs` | `AppState` struct | real |
| `src/cache/mod.rs` | `Cache` trait + submodule decls | real trait |
| `src/cache/memory.rs` | in-memory ArcSwap impl | stub bodies |
| `src/cache/redis.rs` | Redis impl (optional) | stub bodies |
| `src/store/mod.rs` | `HeartbeatStore` trait + `Beat`/`Window`/`UptimeResult` + submodule decls | real trait/types |
| `src/store/sqlite.rs` | SQLite impl | stub bodies |
| `src/store/schema.sql` | placeholder schema | placeholder |
| `src/poller/mod.rs` | poll loop | stub |
| `src/poller/status_page.rs` | primary source client | stub |
| `src/poller/incidents.rs` | incident derivation | stub |
| `src/poller/prometheus.rs` | optional `/metrics` fallback | stub |
| `src/api/mod.rs` | router assembly | minimal/stub |
| `src/api/monitors.rs` | `GET /api/monitors` | stub |
| `src/api/uptime.rs` | `GET /api/uptime` | stub |
| `src/api/incidents.rs` | `GET /api/incidents` | stub |
| `src/api/auth.rs` | `X-Api-Key` middleware | stub |

---

## Task 1: Initialize crate and declare dependencies

**Files:**
- Create: `Cargo.toml`, `src/main.rs` (via cargo init)

- [ ] **Step 1: Initialize the crate**

Run: `~/.cargo/bin/cargo init --bin --edition 2024 --name uptime-kuma-rs`
Expected: creates `Cargo.toml` and `src/main.rs` with a hello-world `main`. (The repo already has
`.git`; cargo init reuses it.)

- [ ] **Step 2: Add dependencies**

Run each (cargo resolves latest compatible versions for Rust 1.95.0):

```bash
~/.cargo/bin/cargo add axum
~/.cargo/bin/cargo add tokio --features rt-multi-thread,macros
~/.cargo/bin/cargo add reqwest --no-default-features --features json,rustls-tls
~/.cargo/bin/cargo add serde --features derive
~/.cargo/bin/cargo add serde_json
~/.cargo/bin/cargo add chrono --features serde
~/.cargo/bin/cargo add figment --features env,toml
~/.cargo/bin/cargo add tracing
~/.cargo/bin/cargo add tracing-subscriber --features env-filter
~/.cargo/bin/cargo add tower
~/.cargo/bin/cargo add tower-http --features cors,trace,timeout
~/.cargo/bin/cargo add sqlx --no-default-features --features sqlite,runtime-tokio
~/.cargo/bin/cargo add redis --features tokio-comp
~/.cargo/bin/cargo add thiserror
~/.cargo/bin/cargo add arc-swap
~/.cargo/bin/cargo add async-trait
```

- [ ] **Step 3: Verify dependencies compile**

Run: `~/.cargo/bin/cargo build`
Expected: PASS (downloads and compiles all crates; first build is slow). Hello-world `main` still
present.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -m "chore: scaffold crate and declare dependencies"
```

---

## Task 2: Domain model (`model.rs`)

**Files:**
- Create: `src/model.rs`
- Modify: `src/main.rs` (add `mod model;`)

- [ ] **Step 1: Write `src/model.rs`**

```rust
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
    pub id: u64,
    pub name: String,
    pub status: MonitorStatus,
    /// `None` when down / unknown.
    pub latency_ms: Option<u32>,
}

/// Uptime ratios over standard windows, with per-window data coverage (see low-level §3, §11).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UptimeWindow {
    pub monitor_id: u64,
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
    pub monitor_id: u64,
    pub started_at: DateTime<Utc>,
    /// `None` while the incident is ongoing.
    pub resolved_at: Option<DateTime<Utc>>,
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
```

- [ ] **Step 2: Declare the module in `src/main.rs`**

Add near the top of `src/main.rs` (above `fn main`):

```rust
mod model;
```

- [ ] **Step 3: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (warnings about unused code are acceptable for now; no errors).

- [ ] **Step 4: Commit**

```bash
git add src/model.rs src/main.rs
git commit -m "feat: add domain model types"
```

---

## Task 3: Error type (`error.rs`)

**Files:**
- Create: `src/error.rs`
- Modify: `src/main.rs` (add `mod error;`)

- [ ] **Step 1: Write `src/error.rs`**

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Unified application error. Internal detail is logged, not leaked to clients (low-level §9).
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("upstream fetch failed: {0}")]
    Upstream(String),

    #[error("failed to parse upstream payload: {0}")]
    Parse(String),

    #[error("cache error: {0}")]
    Cache(String),

    #[error("storage error: {0}")]
    Store(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("no snapshot available yet")]
    NoSnapshot,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::NoSnapshot => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
            AppError::Parse(_) | AppError::Cache(_) | AppError::Store(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        // Public message kept generic; full error is logged by the caller / trace layer.
        let body = Json(json!({ "error": status.canonical_reason().unwrap_or("error") }));
        (status, body).into_response()
    }
}
```

- [ ] **Step 2: Declare the module in `src/main.rs`**

```rust
mod error;
```

- [ ] **Step 3: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors).

- [ ] **Step 4: Commit**

```bash
git add src/error.rs src/main.rs
git commit -m "feat: add AppError with IntoResponse"
```

---

## Task 4: Configuration (`config.rs`)

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs` (add `mod config;`)

- [ ] **Step 1: Write `src/config.rs`**

Keys and defaults per low-level §8.

```rust
use serde::Deserialize;

use crate::error::AppError;

/// Service configuration, loaded env-first with an optional TOML overlay (low-level §8).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Uptime Kuma base URL (required). Env: `KUMA_BASE_URL`.
    pub kuma_base_url: String,
    /// Status-page slug, the primary data source (required). Env: `KUMA_STATUS_PAGE_SLUG`.
    pub kuma_status_page_slug: String,
    /// Poll interval in seconds. Env: `POLL_INTERVAL_SECONDS`.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_seconds: u64,
    /// Optional `/metrics` API key (fallback source). Env: `KUMA_METRICS_API_KEY`.
    #[serde(default)]
    pub kuma_metrics_api_key: Option<String>,
    /// Listen address. Env: `LISTEN_ADDR`.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    /// This service's own API key for `X-Api-Key` auth. Env: `API_KEY`.
    #[serde(default)]
    pub api_key: Option<String>,
    /// CORS allowed origins. Env: `CORS_ALLOWED_ORIGINS`.
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
    /// SQLite database URL. Env: `DATABASE_URL`.
    #[serde(default = "default_database_url")]
    pub database_url: String,
    /// Heartbeat retention in days (must exceed 30d window + margin). Env: `HISTORY_RETENTION_DAYS`.
    #[serde(default = "default_retention_days")]
    pub history_retention_days: u32,
    /// Redis URL; when unset, the in-memory cache is used. Env: `REDIS_URL`.
    #[serde(default)]
    pub redis_url: Option<String>,
}

fn default_poll_interval() -> u64 {
    60
}
fn default_listen_addr() -> String {
    "0.0.0.0:8080".to_string()
}
fn default_database_url() -> String {
    "sqlite://data/uptime.db".to_string()
}
fn default_retention_days() -> u32 {
    31
}

impl Config {
    /// Load configuration: optional `config.toml` overlaid by environment variables.
    pub fn load() -> Result<Self, AppError> {
        use figment::providers::{Env, Format, Toml};
        use figment::Figment;

        Figment::new()
            .merge(Toml::file("config.toml"))
            .merge(Env::raw())
            .extract()
            .map_err(|e| AppError::Cache(format!("config load failed: {e}")))
    }
}
```

- [ ] **Step 2: Declare the module in `src/main.rs`**

```rust
mod config;
```

- [ ] **Step 3: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS. (`Env::raw()` maps `KUMA_BASE_URL` → `kuma_base_url` via lowercasing; confirm no
errors.)

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: add Config with figment loader"
```

---

## Task 5: Cache trait and stub impls (`cache/`)

**Files:**
- Create: `src/cache/mod.rs`, `src/cache/memory.rs`, `src/cache/redis.rs`
- Modify: `src/main.rs` (add `mod cache;`)

- [ ] **Step 1: Write `src/cache/mod.rs`**

```rust
use async_trait::async_trait;

use crate::model::Snapshot;

pub mod memory;
pub mod redis;

/// Live snapshot cache (low-level §5a). One writer (the poller), many readers (handlers).
#[async_trait]
pub trait Cache: Send + Sync {
    /// Returns the most recent snapshot, or `None` before the first poll completes.
    async fn get_snapshot(&self) -> Option<Snapshot>;
    /// Atomically replaces the cached snapshot.
    async fn put_snapshot(&self, snap: Snapshot);
}
```

- [ ] **Step 2: Write `src/cache/memory.rs`**

```rust
use async_trait::async_trait;

use crate::model::Snapshot;

use super::Cache;

/// Default cache for single-instance deployments: lock-free reads via `ArcSwap` (low-level §5a).
pub struct MemoryCache {
    // Will hold `arc_swap::ArcSwapOption<Snapshot>`; wired in the cache implementation task.
}

impl MemoryCache {
    pub fn new() -> Self {
        todo!("initialize ArcSwap-backed in-memory cache")
    }
}

#[async_trait]
impl Cache for MemoryCache {
    async fn get_snapshot(&self) -> Option<Snapshot> {
        todo!("load current Arc<Snapshot>")
    }

    async fn put_snapshot(&self, snap: Snapshot) {
        todo!("swap in the new snapshot")
    }
}
```

- [ ] **Step 3: Write `src/cache/redis.rs`**

```rust
use async_trait::async_trait;

use crate::model::Snapshot;

use super::Cache;

/// Optional shared cache for multi-replica deployments (low-level §5a). Off unless `REDIS_URL` set.
pub struct RedisCache {
    // Will hold a redis connection/pool + key + TTL; wired in the Redis cache task.
}

impl RedisCache {
    pub fn new(redis_url: &str) -> Self {
        todo!("connect to Redis at {redis_url}")
    }
}

#[async_trait]
impl Cache for RedisCache {
    async fn get_snapshot(&self) -> Option<Snapshot> {
        todo!("GET + deserialize snapshot JSON")
    }

    async fn put_snapshot(&self, snap: Snapshot) {
        todo!("SET serialized snapshot JSON with TTL")
    }
}
```

- [ ] **Step 4: Declare the module in `src/main.rs`**

```rust
mod cache;
```

- [ ] **Step 5: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors; `todo!()` bodies satisfy the return types).

- [ ] **Step 6: Commit**

```bash
git add src/cache src/main.rs
git commit -m "feat: add Cache trait with memory and redis stubs"
```

---

## Task 6: HeartbeatStore trait, types, and stub impl (`store/`)

**Files:**
- Create: `src/store/mod.rs`, `src/store/sqlite.rs`, `src/store/schema.sql`
- Modify: `src/main.rs` (add `mod store;`)

- [ ] **Step 1: Write `src/store/mod.rs`**

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::AppError;
use crate::model::{Incident, MonitorStatus};

pub mod sqlite;

/// A single recorded heartbeat to be persisted (low-level §5b).
#[derive(Debug, Clone)]
pub struct Beat {
    pub monitor_id: u64,
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
    async fn uptime(&self, monitor_id: u64, window: Window) -> Result<UptimeResult, AppError>;
    async fn incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError>;
}
```

- [ ] **Step 2: Write `src/store/sqlite.rs`**

```rust
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

    async fn uptime(&self, monitor_id: u64, window: Window) -> Result<UptimeResult, AppError> {
        todo!("aggregate stored beats into ratio + coverage")
    }

    async fn incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError> {
        todo!("reconstruct incidents from stored history")
    }
}
```

- [ ] **Step 3: Write `src/store/schema.sql`**

```sql
-- Placeholder schema for uptime-kuma-rs heartbeat history.
-- Real tables (heartbeats, indexes, retention) are defined in the store implementation plan.
-- See docs/project/low-level-analysis.md §5b.
```

- [ ] **Step 4: Declare the module in `src/main.rs`**

```rust
mod store;
```

- [ ] **Step 5: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors).

- [ ] **Step 6: Commit**

```bash
git add src/store src/main.rs
git commit -m "feat: add HeartbeatStore trait with sqlite stub"
```

---

## Task 7: Application state (`state.rs`)

**Files:**
- Create: `src/state.rs`
- Modify: `src/main.rs` (add `mod state;`)

- [ ] **Step 1: Write `src/state.rs`**

```rust
use std::sync::Arc;

use crate::cache::Cache;
use crate::config::Config;
use crate::store::HeartbeatStore;

/// Shared application state (low-level §6). Cheaply cloneable; attached via `Router::with_state`.
#[derive(Clone)]
pub struct AppState {
    pub cache: Arc<dyn Cache>,
    pub store: Arc<dyn HeartbeatStore>,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
}
```

- [ ] **Step 2: Declare the module in `src/main.rs`**

```rust
mod state;
```

- [ ] **Step 3: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors).

- [ ] **Step 4: Commit**

```bash
git add src/state.rs src/main.rs
git commit -m "feat: add AppState"
```

---

## Task 8: Poller stubs (`poller/`)

**Files:**
- Create: `src/poller/mod.rs`, `src/poller/status_page.rs`, `src/poller/incidents.rs`, `src/poller/prometheus.rs`
- Modify: `src/main.rs` (add `mod poller;`)

- [ ] **Step 1: Write `src/poller/mod.rs`**

```rust
use crate::state::AppState;

pub mod incidents;
pub mod prometheus;
pub mod status_page;

/// Spawns the background poll loop (low-level §4): on each tick, fetch from the source(s),
/// build a snapshot, persist beats, and atomically replace the cached snapshot.
pub fn spawn(state: AppState) {
    todo!("spawn tokio interval loop driving the configured sources")
}
```

- [ ] **Step 2: Write `src/poller/status_page.rs`**

```rust
use crate::error::AppError;
use crate::model::Snapshot;

/// Primary data source: the public status-page JSON endpoints (low-level §4). No auth required.
pub struct StatusPageClient {
    // Will hold base URL, slug, and a shared reqwest client.
}

impl StatusPageClient {
    /// Fetch current heartbeats + 24h uptime and build a snapshot.
    pub async fn fetch(&self) -> Result<Snapshot, AppError> {
        todo!("GET /api/status-page/heartbeat/:slug and map into Snapshot")
    }
}
```

- [ ] **Step 3: Write `src/poller/incidents.rs`**

```rust
use crate::model::{Incident, Monitor};

/// Derives incidents by diffing heartbeat status transitions across polls (low-level §4).
pub fn derive(previous: &[Monitor], current: &[Monitor]) -> Vec<Incident> {
    todo!("open incidents on up->down, close on down->up")
}
```

- [ ] **Step 4: Write `src/poller/prometheus.rs`**

```rust
use crate::error::AppError;
use crate::model::Monitor;

/// Optional fallback source: parses the `/metrics` Prometheus endpoint (low-level §4).
/// Provides current status only — no id, no uptime, no incidents.
pub async fn fetch_status(metrics_url: &str, api_key: Option<&str>) -> Result<Vec<Monitor>, AppError> {
    todo!("fetch and parse monitor_status / monitor_response_time metrics")
}
```

- [ ] **Step 5: Declare the module in `src/main.rs`**

```rust
mod poller;
```

- [ ] **Step 6: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors).

- [ ] **Step 7: Commit**

```bash
git add src/poller src/main.rs
git commit -m "feat: add poller module stubs"
```

---

## Task 9: API handler and auth stubs (`api/`)

**Files:**
- Create: `src/api/mod.rs`, `src/api/monitors.rs`, `src/api/uptime.rs`, `src/api/incidents.rs`, `src/api/auth.rs`
- Modify: `src/main.rs` (add `mod api;`)

- [ ] **Step 1: Write `src/api/monitors.rs`**

```rust
use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::model::Monitor;
use crate::state::AppState;

/// `GET /api/monitors` — current status and latency of each monitor. Served from cache (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Monitor>>, AppError> {
    todo!("read snapshot from cache and return monitors")
}
```

- [ ] **Step 2: Write `src/api/uptime.rs`**

```rust
use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::model::UptimeWindow;
use crate::state::AppState;

/// `GET /api/uptime` — uptime % over 24h / 7d / 30d windows with coverage (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<UptimeWindow>>, AppError> {
    todo!("read snapshot from cache and return uptime windows")
}
```

- [ ] **Step 3: Write `src/api/incidents.rs`**

```rust
use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::model::Incident;
use crate::state::AppState;

/// `GET /api/incidents` — history of incidents (monitors that went down) (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Incident>>, AppError> {
    todo!("read snapshot from cache and return incidents")
}
```

- [ ] **Step 4: Write `src/api/auth.rs`**

```rust
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::AppError;
use crate::state::AppState;

/// `X-Api-Key` auth middleware (low-level §7). No-op when `config.api_key` is unset.
pub async fn require_api_key(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    todo!("constant-time compare X-Api-Key against config.api_key when set")
}
```

- [ ] **Step 5: Write `src/api/mod.rs`**

```rust
use axum::routing::get;
use axum::Router;

use crate::state::AppState;

pub mod auth;
pub mod incidents;
pub mod monitors;
pub mod uptime;

/// Assembles the read-only API router with middleware (low-level §7).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/monitors", get(monitors::handler))
        .route("/api/uptime", get(uptime::handler))
        .route("/api/incidents", get(incidents::handler))
        .with_state(state)
}
```

- [ ] **Step 6: Declare the module in `src/main.rs`**

```rust
mod api;
```

- [ ] **Step 7: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors).

- [ ] **Step 8: Commit**

```bash
git add src/api src/main.rs
git commit -m "feat: add API router and handler stubs"
```

---

## Task 10: Wire `main.rs`

**Files:**
- Modify: `src/main.rs` (replace the cargo-init hello-world body; keep the `mod` declarations from prior tasks)

- [ ] **Step 1: Replace `src/main.rs` with the assembled entrypoint**

Keep all the `mod` lines added in earlier tasks. The full file:

```rust
// Temporary during the skeleton phase: behavior is stubbed with `todo!()` and many types are
// defined before they are constructed. Each later implementation plan removes the relevant
// allowances as it fills in real behavior.
#![allow(dead_code, unused_variables, unused_imports)]

mod api;
mod cache;
mod config;
mod error;
mod model;
mod poller;
mod state;
mod store;

use std::sync::Arc;

use crate::cache::memory::MemoryCache;
use crate::config::Config;
use crate::state::AppState;
use crate::store::sqlite::SqliteStore;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // NOTE: skeleton wiring. `Config::load`, the cache/store constructors, the poller, and the
    // server bind are real call sites but their bodies are still `todo!()` — running this will
    // panic at the first stub. Later plans replace the stubs; the shape here is the target.
    let config = Arc::new(Config::load().expect("failed to load config"));

    let cache: Arc<dyn cache::Cache> = Arc::new(MemoryCache::new());
    let store: Arc<dyn store::HeartbeatStore> =
        Arc::new(SqliteStore::connect(&config.database_url).await.expect("store connect"));

    let state = AppState {
        cache,
        store,
        config: config.clone(),
        http: reqwest::Client::new(),
    };

    poller::spawn(state.clone());

    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listen address");

    tracing::info!("listening on {}", config.listen_addr);
    axum::serve(listener, app).await.expect("server error");
}
```

- [ ] **Step 2: Verify it compiles**

Run: `~/.cargo/bin/cargo check`
Expected: PASS (no errors, and the crate-level allow silences stub warnings).

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire main entrypoint (skeleton)"
```

---

## Task 11: Final verification

- [ ] **Step 1: Full build**

Run: `~/.cargo/bin/cargo build`
Expected: PASS, no errors.

- [ ] **Step 2: Clippy**

Run: `~/.cargo/bin/cargo clippy --all-targets`
Expected: PASS, no warnings (crate-level allow covers stub dead-code/unused).

- [ ] **Step 3: Format check**

Run: `~/.cargo/bin/cargo fmt --check`
Expected: PASS (no diff). If it reports a diff, run `~/.cargo/bin/cargo fmt` and re-check.

- [ ] **Step 4: Confirm module tree matches spec**

Run: `find src -type f | sort`
Expected: exactly the 20 files listed in the plan's File Structure table (plus `store/schema.sql`).

- [ ] **Step 5: Final commit (if fmt changed anything)**

```bash
git add -A
git commit -m "style: cargo fmt skeleton" || echo "nothing to commit"
```

---

## Self-Review Notes

- **Spec coverage:** crate init + edition 2024 (T1), deps §2 (T1), model §3 (T2), error §9
  (T3), config §8 (T4), Cache trait + memory/redis stubs §5a (T5), HeartbeatStore + sqlite +
  schema.sql §5b (T6), AppState §6 (T7), poller + status_page/incidents/prometheus §4 (T8), api
  handlers + auth §7 (T9), main wiring (T10), verification build/clippy/fmt §6 (T11). Module tree
  §1 covered across T2–T9. Runtime-gated optional backends: redis/prometheus compile in, selected
  by config, no Cargo features (T5/T8/T10). All spec sections map to a task.
- **Placeholder check:** `todo!()` bodies and `schema.sql` are intentional, spec-sanctioned stubs
  (spec §5), not plan placeholders — every step shows complete, compilable code.
- **Type consistency:** `Cache`/`HeartbeatStore` trait method names and signatures in T5/T6 match
  their use in `AppState` (T7) and `main.rs` (T10); `Snapshot` fields (T2) match handler/cache
  usage; `MemoryCache::new`, `SqliteStore::connect`, `api::router`, `poller::spawn` are defined
  exactly as called in T10.
