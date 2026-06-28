# Design Spec — `uptime-kuma-rs` Project Skeleton

> **Status:** Approved design, ready for implementation planning.
> **Date:** 2026-06-28
> **Scope:** Skeleton only. Derived from
> [`docs/project/low-level-analysis.md`](../../project/low-level-analysis.md) (the authoritative
> implementation blueprint) and [`docs/project/high-level-analysis.md`](../../project/high-level-analysis.md).

## 1. Goal

Produce a compiling Rust binary crate whose module tree mirrors §1 of the low-level analysis,
with the dependencies from §2 declared and every planned module present as a typed-but-mostly-
unimplemented stub. This is the first implementation step: it establishes structure so later
plans can fill in behavior module by module.

**Success criteria:**

- `cargo build` succeeds.
- `cargo clippy --all-targets` is clean (no warnings).
- `cargo fmt --check` passes.
- The module tree matches §1 of the low-level analysis exactly.
- No real polling, serving, storage, or caching logic — behavior is stubbed.

## 2. Decisions (settled during brainstorming)

| Decision | Choice | Rationale |
| --- | --- | --- |
| Setup scope | Skeleton only | Establish structure before behavior. |
| Rust edition | **2024** | Greenfield project on Rust 1.95.0; latest stable edition. |
| Config crate | **figment** (`env` + `toml`) | Matches §8's "env-first + optional file overlay" intent. |
| Optional backends | **Stub files included now** | Complete module tree per §1; nothing wired as default. |
| Optional backend gating | **Runtime config presence, not Cargo features** | Keeps the skeleton simple; `redis`/prometheus compile in but stay inert unless config selects them. |
| Definitions vs. behavior | **Real types/traits/config now; behavior stubbed** | Skeleton compiles meaningfully; later plans fill bodies. |

**Toolchain note:** Rust 1.95.0 is installed at `~/.cargo/bin` but **not on the shell PATH**.
Scaffolding and verification commands must invoke cargo via its full path
(`~/.cargo/bin/cargo`).

## 3. Crate Initialization & Manifest

- Initialize with `cargo init --bin --edition 2024` in the repo root (crate name
  `uptime-kuma-rs`).
- `Cargo.toml` declares the dependencies from low-level analysis §2:
  - `axum`
  - `tokio` — features `rt-multi-thread`, `macros`
  - `reqwest` — features `json`, `rustls-tls`
  - `serde` (derive), `serde_json`
  - `chrono` — feature `serde`
  - `figment` — features `env`, `toml`
  - `tracing`, `tracing-subscriber`
  - `tower`, `tower-http` — features `cors`, `trace`, `timeout`
  - `sqlx` — features `sqlite`, `runtime-tokio`
  - `redis` — feature `tokio-comp` (pooling via `deadpool-redis` deferred until the Redis cache
    is actually implemented; the skeleton stub needs only the base client to compile)
  - `thiserror`
  - `arc-swap`
  - `async-trait`
- Optional backends (Redis cache, Prometheus fallback) are compiled in unconditionally and
  remain inert at runtime unless config selects them. **No Cargo feature flags** in this step.

## 4. Module Tree (stubs, mirroring low-level analysis §1)

```
src/
├── main.rs           # entrypoint: init logging, build state, (stub) spawn poller + serve router
├── config.rs         # Config struct + figment loader  [REAL]
├── error.rs          # AppError enum + IntoResponse     [REAL]
├── state.rs          # AppState struct                  [REAL]
├── model.rs          # Monitor, UptimeWindow, Incident, MonitorStatus, Snapshot  [REAL]
├── poller/
│   ├── mod.rs        # poll loop                         [STUB]
│   ├── status_page.rs# primary source client            [STUB]
│   ├── incidents.rs  # incident derivation               [STUB]
│   └── prometheus.rs # optional /metrics fallback        [STUB]
├── cache/
│   ├── mod.rs        # Cache trait definition            [REAL trait, STUB nothing]
│   ├── memory.rs     # in-memory ArcSwap snapshot impl   [STUB bodies]
│   └── redis.rs      # Redis snapshot impl (optional)    [STUB bodies]
├── store/
│   ├── mod.rs        # HeartbeatStore trait definition   [REAL trait]
│   ├── schema.sql    # placeholder schema file           [PLACEHOLDER]
│   └── sqlite.rs     # SQLite-backed impl                [STUB bodies]
└── api/
    ├── mod.rs        # Router assembly                   [STUB/minimal]
    ├── monitors.rs   # GET /api/monitors handler         [STUB]
    ├── uptime.rs     # GET /api/uptime handler           [STUB]
    ├── incidents.rs  # GET /api/incidents handler        [STUB]
    └── auth.rs       # X-Api-Key middleware              [STUB]
```

## 5. Stub Contents — What Is Real vs. Stubbed

### Real now (compiles meaningfully)

- **`model.rs`** — the public domain types from low-level analysis §3: `MonitorStatus`,
  `Monitor`, `UptimeWindow`, `Incident`, plus the `Snapshot` bundle from §5a
  (`Vec<Monitor>`, `Vec<UptimeWindow>`, `Vec<Incident>`, `last_updated: DateTime<Utc>`).
  All with `serde` derives. Monitor id fields (`Monitor.id`, `UptimeWindow.monitor_id`,
  `Incident.monitor_id`) are `i64` — SQLite/sqlx has no u64 codec; ids are small positive
  integers. `duration_seconds` stays `Option<u64>` (it's a duration, not an id).
- **`error.rs`** — `AppError` enum (`thiserror`) covering upstream-fetch, parse, cache, and
  auth failures, with an `axum::response::IntoResponse` impl mapping variants to status codes
  (401 auth, 503 no-snapshot, 502 upstream) per §9.
- **`config.rs`** — `Config` struct with the keys from §8 and a figment-based loader
  (env-first + optional TOML overlay).
- **`state.rs`** — `AppState` struct (§6): `Arc<dyn Cache>`, `Arc<dyn HeartbeatStore>`,
  `Arc<Config>`, `reqwest::Client`. Derives `Clone`.
- **`cache/mod.rs`** — the `Cache` trait (§5a): `get_snapshot`, `put_snapshot` (returns
  `Result<(), AppError>` — fallible so Redis and other impls can propagate errors).
- **`store/mod.rs`** — the `HeartbeatStore` trait (§5b): `record_beats`, `uptime`, `incidents`,
  plus supporting types (`Beat`, `Window`, `UptimeResult`). Monitor id fields (`Beat.monitor_id`,
  `uptime` parameter) are `i64` — SQLite/sqlx has no u64 codec; ids are small positive integers.

### Stubbed (`todo!()` / `unimplemented!()`)

- `poller/*` — loop and all source clients.
- `cache/memory.rs`, `cache/redis.rs` — method bodies.
- `store/sqlite.rs` — method bodies.
- `api/*` handlers and `auth.rs` middleware.
- `main.rs` — initializes `tracing` logging and builds a minimal Axum router that binds
  `LISTEN_ADDR`; routes may return `todo!()` or a placeholder. It compiles; it is not expected
  to do useful work yet.

Stubs use `todo!()` rather than empty/dead-code bodies so clippy stays clean and intent is
explicit.

## 6. Verification

Run and confirm with actual output before claiming completion:

- `~/.cargo/bin/cargo build`
- `~/.cargo/bin/cargo clippy --all-targets`
- `~/.cargo/bin/cargo fmt --check`

## 7. Out of Scope (deferred to later plans)

- Any real polling, serving, caching, or storage logic.
- SQLite migration content beyond a placeholder `schema.sql`.
- Dockerfile and `docker-compose.yml`.
- Tests beyond what `cargo build`/`clippy` provide as a compile check.
- Cargo feature flags for optional backends.
- `/health` / `/ready` endpoints.

## 8. References

- `docs/project/low-level-analysis.md` — authoritative implementation blueprint (§1 layout,
  §2 deps, §3 model, §5 storage traits, §6 state, §8 config, §9 errors).
- `docs/project/high-level-analysis.md` — authoritative high-level design.
- `CLAUDE.md` — project conventions (docs layout, `.env` handling, commands).
