# Design Spec — Vertical Slice: live `GET /api/monitors`

> **Status:** Approved design, ready for implementation planning.
> **Date:** 2026-06-28
> **Scope:** First vertical slice on top of the skeleton. Derived from
> [`docs/project/low-level-analysis.md`](../../project/low-level-analysis.md) §4 (polling) and §7 (HTTP).

## 1. Goal

Make one endpoint work end-to-end against the real Uptime Kuma instance: a background poller
fetches the public status-page endpoints, maps them into the domain model, and stores a
`Snapshot` in the in-memory cache; `GET /api/monitors` serves that snapshot. This proves the
core **poll → cache → serve** pattern with real data before investing in storage and the other
endpoints.

**Success criteria:**

- The binary, configured with `KUMA_BASE_URL=https://uptime.samueleruaro.com` and
  `KUMA_STATUS_PAGE_SLUG=homelab`, starts, polls, and serves `GET /api/monitors` returning the
  10 real monitors with name, group, status, and latency.
- `GET /api/monitors` returns `503` until the first successful poll completes.
- A failed poll logs a `warn` and keeps serving the last good snapshot (does not clear it).
- `cargo build`, `cargo clippy --all-targets`, `cargo fmt --check`, `cargo test` all pass.
- Mapper and handler are covered by unit/integration tests over sanitized fixtures (no network in tests).

## 2. In scope / Out of scope

**In scope (becomes real):**
- `poller/status_page.rs` — client for both status-page endpoints + internal DTOs.
- The heartbeat+config → `Monitor` mapper.
- `cache/memory.rs` — real `ArcSwap`-backed `MemoryCache`.
- `poller/mod.rs` — `spawn` background interval loop.
- `api/monitors.rs` — real handler.
- `store/noop.rs` — a do-nothing `HeartbeatStore` so `AppState` can be constructed.
- `main.rs` — real wiring of config, cache, noop store, poller, server.
- `model.rs` — add `group: Option<String>` to `Monitor`.

**Out of scope (stays stubbed / deferred to later plans):**
- SQLite history (`store/sqlite.rs`), 7d/30d uptime, `GET /api/uptime`.
- Incident derivation, `GET /api/incidents`.
- Redis cache, Prometheus fallback.
- Auth (`X-Api-Key`), CORS, `/health`.

## 3. Upstream data shape (captured from the live instance)

Real fixtures captured to `docs/project/fixtures/` (gitignored — they contain real monitor
names). Structure:

**`GET /api/status-page/:slug`** (names + grouping):
```jsonc
{
  "config": { ... },
  "incidents": [ ... ],          // status-page manual incidents (NOT used in this slice)
  "maintenanceList": [ ... ],
  "publicGroupList": [
    { "name": "Servizi",
      "monitorList": [ { "id": 7, "name": "baikal", "type": "http", "sendUrl": 0 }, ... ] }
  ]
}
```
- Monitors are nested under `publicGroupList[].monitorList[]`. The group `name` is the source of
  `Monitor.group`. This endpoint is the **only** source of monitor names.

**`GET /api/status-page/heartbeat/:slug`** (live status):
```jsonc
{
  "heartbeatList": {
    "7": [ { "status": 1, "time": "2026-06-28 15:59:49.191", "msg": "", "ping": 5 }, ... ]
  },
  "uptimeList": { "7_24": 1 }     // <id>_24 only; 24h uptime; not used in this slice
}
```
- `status`: 0=down, 1=up, 2=pending, 3=maintenance. `ping` = latency ms.
- `time` format: `"%Y-%m-%d %H:%M:%S%.f"`, **no timezone**, interpreted as UTC.
- `uptimeList` is ignored in this slice (uptime windows are a later plan).

## 4. Data flow

```
tokio::time::interval(POLL_INTERVAL_SECONDS, default 60)
  each tick:
    GET /api/status-page/:slug            → StatusPageConfigDto
    GET /api/status-page/heartbeat/:slug  → HeartbeatDto
    map(config, heartbeat)                → Vec<Monitor>
    Snapshot { monitors, uptime: vec![], incidents: vec![], last_updated: Utc::now() }
    cache.put_snapshot(snapshot)

GET /api/monitors:
    cache.get_snapshot() → Some(s) → 200 Json(s.monitors)
                         → None    → 503 (AppError::NoSnapshot)
```

No upstream call on the request path — every request is served from cache.

## 5. Mapping rules

Join config and heartbeat on monitor id (config `id` ↔ heartbeat key as string):

- `Monitor.id` = config `id` (`i64`).
- `Monitor.name` = config `name`.
- `Monitor.group` = the `publicGroupList` group `name` the monitor belongs to (`Some`); `None`
  if the monitor is not in any group.
- `Monitor.status` = the **latest** heartbeat for that id (the **last** element of the array —
  beats are in ascending time order, confirmed in the captured fixture): 0→Down, 1→Up,
  2→Pending, 3→Maintenance.
- `Monitor.latency_ms` = `Some(ping as u32)` when status is `Up`, else `None`.
- A monitor present in config but absent from `heartbeatList` (no beats yet) → status `Pending`,
  `latency_ms = None`.
- A monitor present in heartbeat but absent from config → skipped (no name available).

All raw JSON is deserialized into internal DTOs (`#[derive(Deserialize)]`) inside
`status_page.rs`; only the mapped domain `Monitor` leaves the module, so upstream schema drift is
contained there (low-level §3, §4).

## 6. Components & files

| File | Change | Responsibility |
| --- | --- | --- |
| `src/model.rs` | modify | add `group: Option<String>` to `Monitor` |
| `src/poller/status_page.rs` | real | `StatusPageClient` (base_url, slug, http); `fetch()` → both endpoints → `Vec<Monitor>`; internal DTOs; mapper |
| `src/cache/memory.rs` | real | `MemoryCache` over `arc_swap::ArcSwapOption<Arc<Snapshot>>` |
| `src/poller/mod.rs` | real | `spawn(state) -> JoinHandle<()>`: interval loop, poll, `warn` on error, keep last snapshot |
| `src/api/monitors.rs` | real | handler: read cache, 200 or 503 |
| `src/api/mod.rs` | keep | router already wires `/api/monitors` |
| `src/store/noop.rs` | new | `NoopStore`: `HeartbeatStore` returning empty/`Ok` |
| `src/store/mod.rs` | modify | `pub mod noop;` |
| `src/main.rs` | real | wire config + `MemoryCache` + `NoopStore` + poller + `axum::serve` |

The `NoopStore` is the agreed interim for the store dependency: `AppState` requires
`Arc<dyn HeartbeatStore>`, but this slice never touches it and `SqliteStore::connect` is still a
panicking stub. `NoopStore` constructs trivially and lets the binary run. The real `SqliteStore`
replaces it in the SQLite plan.

## 7. Error handling

- Network/HTTP failure or JSON parse failure during a poll → `AppError::Upstream`/`Parse`, logged
  at `warn`, snapshot left unchanged. The loop continues on the next tick.
- `GET /api/monitors` with no snapshot yet → `503` via `AppError::NoSnapshot` (already mapped in
  `IntoResponse`).
- Reqwest timeout set on the client so a hung upstream cannot stall the poller indefinitely.

## 8. Testing

- **Fixtures:** sanitized copies committed under `tests/fixtures/` —
  `status-page-config.json` and `status-page-heartbeat.json` with monitor names renamed
  (`service-a`, `service-b`, …) but identical structure (groups, ids, beat shape, time format,
  `uptimeList`). Real captures stay in the gitignored `docs/project/fixtures/`.
- **Mapper unit tests** (TDD, pure function over fixture JSON, no network):
  - all-up page → correct count, names, group, statuses, latencies;
  - a down monitor → `status=Down`, `latency_ms=None`;
  - pending/maintenance status codes map correctly;
  - monitor in config but missing from heartbeat → `Pending`, `None`;
  - UTC time parsing of `"%Y-%m-%d %H:%M:%S%.f"`.
- **Handler integration test** via `tower::ServiceExt::oneshot` on the router:
  - seeded cache → `200` with expected JSON body;
  - empty cache → `503`.
- **Live verification (manual):** run the binary against `KUMA_BASE_URL=https://uptime.samueleruaro.com`,
  `KUMA_STATUS_PAGE_SLUG=homelab`; `curl localhost:8080/api/monitors`; confirm the 10 real
  monitors with names, group `Servizi`, statuses, and latencies. Commands invoked via
  `~/.cargo/bin/cargo` (cargo not on PATH).

## 9. Config used (already defined)

`KUMA_BASE_URL` (required), `KUMA_STATUS_PAGE_SLUG` (required), `POLL_INTERVAL_SECONDS`
(default 60), `LISTEN_ADDR` (default `0.0.0.0:8080`). A local `.env` / `config.toml` provides
them in dev (gitignored).

## 10. References

- `docs/project/low-level-analysis.md` §4 (polling, status_page source, failure isolation),
  §5a (Cache, MemoryCache), §6 (AppState), §7 (HTTP layer), §9 (errors).
- `docs/superpowers/specs/2026-06-28-project-skeleton-design.md` — the skeleton this builds on.
- Real captured fixtures: `docs/project/fixtures/` (gitignored).
