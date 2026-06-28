# Low-Level Analysis — `uptime-kuma-rs`

> **Status:** Proposed design. No code exists yet; this document describes the intended
> implementation derived from [`high-level-analysis.md`](./high-level-analysis.md). Treat it as
> a blueprint to validate during scaffolding, not as a description of existing code.

## 1. Crate Layout

A single binary crate is sufficient for the initial scope. Modules:

```
src/
├── main.rs          # entrypoint: load config, build state, spawn poller, serve router
├── config.rs        # Config struct + loading (env + optional file)
├── error.rs         # AppError enum + IntoResponse impl
├── state.rs         # AppState: shared cache handle + config + http client
├── model.rs         # domain types: Monitor, UptimeWindow, Incident
├── poller/
│   ├── mod.rs        # poll loop: schedule, orchestrate sources, write to cache
│   ├── status_page.rs# PRIMARY: client for the public status-page JSON endpoints
│   ├── incidents.rs  # derive incidents by diffing heartbeat status transitions
│   └── prometheus.rs # OPTIONAL fallback: parser for the /metrics endpoint
├── cache/
│   ├── mod.rs       # Cache trait (live snapshot)
│   ├── memory.rs    # in-memory snapshot (local dev / single instance)
│   └── redis.rs     # Redis snapshot cache (optional; multi-replica only)
├── store/
│   ├── mod.rs       # HeartbeatStore: persist beats, query uptime windows & incidents
│   ├── schema.sql   # SQLite schema + migrations
│   └── sqlite.rs    # SQLite-backed implementation
└── api/
    ├── mod.rs       # Router assembly + middleware wiring
    ├── monitors.rs  # GET /api/monitors
    ├── uptime.rs    # GET /api/uptime
    ├── incidents.rs # GET /api/incidents
    └── auth.rs      # X-Api-Key middleware
```

## 2. Core Dependencies

| Concern        | Crate                                | Notes                                    |
| -------------- | ------------------------------------ | ---------------------------------------- |
| HTTP server    | `axum`                               | on Tokio                                 |
| Async runtime  | `tokio` (`rt-multi-thread`, `macros`)| `#[tokio::main]`                         |
| HTTP client    | `reqwest` (`json`, `rustls-tls`)     | polling upstream                         |
| Serialization  | `serde`, `serde_json`                | derive on all model types                |
| Time           | `chrono` (`serde`)                   | `DateTime<Utc>` for incident timestamps  |
| Config         | `figment` or `envy` + `serde`        | env-first, optional file overlay         |
| Logging        | `tracing`, `tracing-subscriber`      | structured logs, `RUST_LOG`              |
| Middleware     | `tower`, `tower-http`                | CORS, trace, timeout layers              |
| **Persistence**| `sqlx` (`sqlite`, `runtime-tokio`)   | heartbeat history; compile-time queries  |
| Redis (opt)    | `redis` (`tokio-comp`) / `deadpool-redis` | multi-replica snapshot cache; off by default |
| Errors         | `thiserror`                          | on `AppError`                            |

## 3. Domain Model (`model.rs`)

```rust
pub enum MonitorStatus { Up, Down, Pending, Maintenance }

pub struct Monitor {
    pub id: i64,             // i64 — SQLite/sqlx has no u64 codec; ids are small positive integers
    pub name: String,
    pub status: MonitorStatus,
    pub latency_ms: Option<u32>,   // None when down / unknown
}

pub struct UptimeWindow {
    pub monitor_id: i64,     // i64 — SQLite/sqlx has no u64 codec; ids are small positive integers
    pub uptime_24h: f64,
    pub uptime_7d: f64,
    pub uptime_30d: f64,
    /// Per-window data coverage: actual history span / requested window, in [0.0, 1.0].
    /// e.g. a DB with only 3h of data reports coverage_30d ≈ 0.004. Lets consumers tell a
    /// fully-backed figure from one computed on partial history. See §11.
    pub coverage_7d: f64,
    pub coverage_30d: f64,
}

pub struct Incident {
    pub monitor_id: i64,     // i64 — SQLite/sqlx has no u64 codec; ids are small positive integers
    pub started_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,  // None while ongoing
    /// Denormalized convenience field: always set together with resolved_at (None while ongoing).
    pub duration_seconds: Option<u64>,
}
```

These are the **public** API shapes. Upstream Uptime Kuma payloads are deserialized into
separate internal DTOs and mapped into these, so upstream schema drift is contained to the
poller layer.

## 4. Polling Engine (`poller/`)

- Runs as a single detached Tokio task spawned at startup (`tokio::spawn`).
- Loop: `interval = tokio::time::interval(config.poll_interval)`; on each tick, fetch from the
  source(s), build the snapshot, and atomically replace the cached snapshot.

### Primary source — public status-page JSON (`status_page.rs`)

Uptime Kuma has **no general read REST API** and its socket.io interface is unofficial/unstable.
The chosen primary source is the **public status-page endpoints** (no auth required):

- `GET /api/status-page/:slug` → page config + the monitor list (id, name, type) shown on the
  page. Polled infrequently (monitors change rarely); server-cached ~5 min upstream.
- `GET /api/status-page/heartbeat/:slug` → the live data, polled every tick. Shape:

  ```jsonc
  {
    "heartbeatList": {
      "1": [ { "status": 1, "time": "2026-06-27 10:00:00", "ping": 42, "msg": "" }, ... ]
    },
    "uptimeList": { "1_24": 0.9998 }   // key = "<monitorId>_<hours>"; status pages expose 24h
  }
  ```

  `status`: `0 = down`, `1 = up`, `2 = pending`, `3 = maintenance`. `ping` is latency in ms.
  Upstream-cached ~1 min, so polling faster than ~60s gains nothing.

Mapping into the domain model:
- `Monitor.status`/`latency_ms` ← the most recent heartbeat per monitor id.
- `UptimeWindow.uptime_24h` ← `uptimeList["<id>_24"]`. **7d/30d are not provided by the
  status-page API** — see §11; populate as `None`/optional or compute locally from accumulated
  heartbeats.

**Prerequisite:** a status page must exist and be published with the target monitors on it.
`config.status_page_slug` selects which one.

### Incident derivation (`incidents.rs`)

No endpoint exposes incidents. They are derived by diffing heartbeat status across successive
polls (or by scanning the returned heartbeat history): a monitor going `up → down` opens an
incident; `down → up` closes it and sets `resolved_at`/`duration_seconds`. Open incidents are
retained in the snapshot.

### Optional fallback source — `/metrics` (`prometheus.rs`)

Behind config. Exports only `monitor_status` and `monitor_response_time`, labelled by
`monitor_name`/`type`/`url`/`hostname`/`port` (**no monitor id**, no uptime, no incidents), and
requires Basic Auth or an API key. Useful only as a supplementary current-status signal when a
status page is unavailable; cannot satisfy `/api/uptime` or `/api/incidents` on its own.

### Failure isolation

A failed poll logs at `warn` and keeps serving the last good snapshot rather than clearing it.
A `last_updated: DateTime<Utc>` timestamp travels with the snapshot so consumers (and a future
`/health`) can detect staleness.

## 5. Storage Layers

Two tiers are always present (in-memory snapshot + SQLite history); Redis is an optional third
for multi-replica only. Different data has different lifetimes, so it lives in different places
rather than being forced into one store.

### 5a. Live snapshot cache (`cache/`)

```rust
#[async_trait]
pub trait Cache: Send + Sync {
    async fn get_snapshot(&self) -> Option<Arc<Snapshot>>;
    async fn put_snapshot(&self, snap: Snapshot) -> Result<(), AppError>;
}
```

- `Snapshot` bundles `Vec<Monitor>`, `Vec<UptimeWindow>`, `Vec<Incident>`, and `last_updated`.
- **`memory.rs`** — **default**, and all a single instance needs. An `arc_swap::ArcSwap<Snapshot>`
  (or `RwLock<Arc<Snapshot>>`) giving lock-free reads on the hot path; the poller swaps the whole
  `Arc` on update.
- **`redis.rs`** — **optional, off by default; only relevant for multi-replica deployments.**
  Serializes the snapshot to JSON under a fixed key with a TTL slightly longer than the poll
  interval, so **many replicas serve reads from Redis while only one runs the poller** and a
  restarted replica warms from Redis instead of re-polling Uptime Kuma. Selected at startup only
  when `config.redis_url` is set; otherwise the in-memory backend is used. Handlers depend only
  on the `Cache` trait, so this is a drop-in with no handler changes.

### 5b. Heartbeat history (`store/`) — SQLite

The status-page API only returns 24h uptime and a bounded window of recent beats, so durable
history must be accumulated locally.

```rust
#[async_trait]
pub trait HeartbeatStore: Send + Sync {
    async fn record_beats(&self, beats: &[Beat]) -> Result<(), AppError>;
    /// Returns the uptime ratio AND the data coverage of the window (history span / window).
    async fn uptime(&self, monitor_id: i64, window: Window) -> Result<UptimeResult, AppError>;
    async fn incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError>;
}
// UptimeResult { ratio: f64, coverage: f64 } — coverage feeds UptimeWindow.coverage_* (§3, §11).
```

- On each poll, new beats are upserted (dedup on `(monitor_id, time)`).
- `uptime_7d` / `uptime_30d` are computed by aggregating stored beats; `uptime_24h` still comes
  straight from the status-page `uptimeList` (cheaper and authoritative for that window).
- Each computed window also reports `coverage` (oldest stored beat vs. window start), so the API
  can flag figures still backed by partial history rather than presenting them as complete.
- Incidents survive restarts because they can be reconstructed from the stored history, not just
  from in-process diffing.
- A periodic retention job prunes beats older than the longest window (+ margin) to bound DB size.
- Schema + migrations live in `store/schema.sql`, applied at startup via `sqlx::migrate!`.

### 5c. Why not one store?

Redis is fast and shared but ephemeral and ill-suited to range aggregation; SQLite is durable
and queryable but local to one process. In-memory is fastest but lost on restart. Each tier does
what it is best at: **in-memory for speed, Redis for sharing/coordination, SQLite for durable
history.**

## 6. Application State (`state.rs`)

```rust
#[derive(Clone)]
pub struct AppState {
    pub cache: Arc<dyn Cache>,           // live snapshot (memory or Redis)
    pub store: Arc<dyn HeartbeatStore>,  // SQLite-backed history
    pub config: Arc<Config>,
    pub http: reqwest::Client,           // shared, connection-pooled
}
```

Cheaply cloneable (all `Arc`/handle fields); attached to the router with `.with_state(state)`
and shared by the poller task and all handlers.

## 7. HTTP Layer (`api/`)

- Router built in `api::mod`, composing the three route modules plus middleware via `tower`:
  - `tower_http::trace::TraceLayer` — request logging.
  - `tower_http::cors::CorsLayer` — origins from `config.cors_allowed_origins`.
  - `tower_http::timeout::TimeoutLayer` — bound handler time.
  - `auth` middleware — when `config.api_key` is set, require a matching `X-Api-Key` header
    (constant-time compare); otherwise the middleware is a no-op (private-network mode).
- Handlers are thin: read the snapshot from the cache, project the relevant slice, return
  `Json<Vec<T>>`. No upstream calls happen on the request path — every request is served from
  cache.

## 8. Configuration (`config.rs`)

Env-first (12-factor), with an optional TOML file overlay. Keys:

| Key                          | Env                       | Default        |
| ---------------------------- | ------------------------- | -------------- |
| Uptime Kuma base URL         | `KUMA_BASE_URL`           | — (required)   |
| Status-page slug (primary)   | `KUMA_STATUS_PAGE_SLUG`   | — (required)   |
| Poll interval                | `POLL_INTERVAL_SECONDS`   | `60`           |
| `/metrics` API key (fallback)| `KUMA_METRICS_API_KEY`    | none           |
| Listen address               | `LISTEN_ADDR`             | `0.0.0.0:8080` |
| API key (this service's auth)| `API_KEY`                 | none           |
| CORS allowed origins         | `CORS_ALLOWED_ORIGINS`    | none (deny)    |
| SQLite database path         | `DATABASE_URL`            | `sqlite://data/uptime.db` |
| Heartbeat retention (days)   | `HISTORY_RETENTION_DAYS`  | `31`           |
| Redis URL (snapshot cache)   | `REDIS_URL`               | none → in-memory |

Notes:
- Poll interval defaults to `60` because the upstream status-page heartbeat endpoint is itself
  cached ~1 min — polling faster does not yield fresher data.
- `REDIS_URL` unset falls back to the in-memory cache (single-instance / dev). Set it to enable
  the multi-replica deployment described in §5a.
- `HISTORY_RETENTION_DAYS` must exceed the longest uptime window (30d) plus margin.

Secrets are loaded from the environment / `.env`; the config struct is never serialized back
into any API response.

## 9. Error Handling (`error.rs`)

- One `AppError` enum (`thiserror`) covering upstream-fetch, parse, cache, and auth failures.
- Implements `axum::response::IntoResponse`, mapping variants to status codes (`401` for auth,
  `503` when no snapshot is yet available, `502` for upstream issues) with a small JSON error
  body. Internal details are logged, not leaked to clients.

## 10. Concurrency Summary

- **One** poller task writes the live snapshot (and persists beats to SQLite); **N** request
  handlers read.
- Snapshot writes are whole-snapshot swaps → readers never observe a partial snapshot and never
  block the poller. This is the key invariant that keeps the read path lock-free.
- SQLite writes happen only on the poller task; handlers issue read-only aggregate queries
  (`uptime`, `incidents`) against the `sqlx` pool, so they never contend with the writer for the
  write lock.
- **Multi-replica:** with Redis configured, exactly one replica should run the poller (and own
  the SQLite file); the others serve reads from Redis. Poller ownership coordination (env flag or
  a Redis lock) is an open item — see §11.

## 11. Open Questions / To Validate

- **7d/30d uptime — decided: compute everything locally from SQLite history** (§5b). The
  `/api/badge/:id/uptime/:duration` endpoint was considered as a way to get arbitrary windows
  directly from Uptime Kuma, but rejected: badges return SVG (not clean JSON), so SQLite is the
  single source of truth for all uptime windows beyond the status-page 24h figure, and for
  incidents. Bootstrapping — **decided:** always return the computed ratio **plus** a per-window
  `coverage` field (§3); on a fresh DB `uptime_30d` is real but `coverage_30d` is low, so
  consumers can tell a complete figure from a partial one instead of being misled. (Not `null`,
  not a bare number.)
- **Multi-replica poller ownership.** With Redis enabled, only one replica may run the poller and
  own the SQLite file. Coordinate via a simple env flag (one designated writer) or a Redis lock /
  leader election. Pick the simplest that fits the deployment.
- **Heartbeat history depth.** The endpoint returns the most recent ~100 beats per monitor;
  confirm this is enough to backfill SQLite reliably between polls, or rely on poll-to-poll
  diffing for gap detection.
- **Exact response shape across versions.** Pin a minimum tested Uptime Kuma version and isolate
  all parsing in `status_page.rs` / `prometheus.rs`; the status-page API is consumed by the UI
  but is not a contractually stable third-party API.
- **Status page as a hard dependency.** The primary source requires a published status page. If
  that is unacceptable, the design must fall back to socket.io for the monitor list and uptime —
  decide whether to invest in a (currently immature) Rust socket.io client.
- **`/health` (and maybe `/ready`)** endpoint for deployment probes, reporting snapshot
  `last_updated` staleness.

## 12. Deployment

Target: **single self-hosted instance** (homelab), distributed as a container.

- **Dockerfile** — multi-stage: build the release binary, then copy it into a minimal runtime
  image (distroless / `debian:slim`). The result is a small static-ish image with just the
  binary.
- **SQLite = a file on a volume, not a service.** Mount a volume (e.g. `/data`) and point
  `DATABASE_URL=sqlite:///data/uptime.db` at it so history survives container restarts. No
  database container.
- **`docker-compose.yml`** — provided for one-command startup:
  - The `app` service (this binary) + the `/data` volume. This is the whole default deployment.
  - A `redis` service guarded by a **compose profile** (`profiles: ["redis"]`), so it is **not**
    started unless explicitly requested.
  - Default run: `docker compose up` → app + SQLite only.
  - Multi-replica run: `docker compose --profile redis up` → also starts Redis, and `REDIS_URL`
    switches the cache backend (see §5a).
- **Config** via environment / `.env` (§8). The image ships no secrets.

Rationale: SQLite needs no infrastructure, and Redis is opt-in, so the common case is a single
container + a volume — nothing more.
