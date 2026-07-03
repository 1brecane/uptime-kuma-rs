# Design Spec — Slice: SQLite history + `GET /api/uptime`

> **Status:** Approved design, ready for implementation planning.
> **Date:** 2026-07-03
> **Scope:** Second vertical slice on top of the monitors slice. Derived from
> [`docs/project/low-level-analysis.md`](../../project/low-level-analysis.md) §5b (heartbeat store)
> and §3/§11 (uptime windows + coverage).

## 1. Goal

Persist heartbeats to SQLite so the service can compute and serve uptime over 24h / 7d / 30d
windows, each with a `coverage` figure that distinguishes a fully-backed number from one computed
on partial history. `GET /api/uptime` serves this from the in-memory snapshot, mirroring the
monitors slice.

**Success criteria:**

- With the binary configured against the real instance (slug `homelab`), `GET /api/uptime`
  returns one entry per monitor with `uptime_24h`, `uptime_7d`, `uptime_30d`, `coverage_7d`,
  `coverage_30d`.
- On a fresh database, 24h is accurate immediately (from the status-page `uptimeList`), while 7d
  and 30d report low `coverage` (partial history) rather than a misleading number or `null`.
- Heartbeats survive restarts (durable SQLite file); duplicates are not double-counted.
- Beats older than `HISTORY_RETENTION_DAYS` are pruned.
- `GET /api/uptime` returns `503` until the first successful poll.
- `cargo build`, `cargo clippy --all-targets`, `cargo fmt --check`, `cargo test` all pass; store
  logic is covered by tests against a temporary SQLite database (no network in tests).

## 2. Decisions (settled during brainstorming)

| Decision | Choice | Rationale |
| --- | --- | --- |
| 7d/30d computation | **Count-based** (`up_beats / total_beats` in window) | Simple, deterministic, matches ~60s sampling. |
| sqlx query style | **Runtime queries** (`sqlx::query`/`query_as`) | No build-time DB dependency; `cargo build` works anywhere. |
| Where uptime is computed | **In the poller → cache** | Uniform with `/api/monitors`; lock-free read path; no per-request DB load. |
| 24h source | **status-page `uptimeList`** (`<id>_24`); `0.0` if absent | Authoritative and accurate even on a fresh DB. 7d/30d from SQLite. |
| Maintenance/pending in ratio | **Count as "not-up"** (only `status == Up` counts as up) | Simplest defensible count; can be refined later. |
| Schema management | **`CREATE TABLE IF NOT EXISTS` at startup** | One table; no `sqlx::migrate!` machinery yet. |
| Retention | **Prune every poll tick** | Indexed `DELETE`; trivial at this scale. |

## 3. Data model & storage

### Domain (already defined, `model.rs`)
`UptimeWindow { monitor_id: i64, uptime_24h, uptime_7d, uptime_30d, coverage_7d, coverage_30d }`
(all `f64`). No changes needed.

### SQLite schema (`store/schema.sql`)
```sql
CREATE TABLE IF NOT EXISTS heartbeats (
    monitor_id INTEGER NOT NULL,
    time       TEXT    NOT NULL,   -- ISO-8601 UTC (chrono RFC3339)
    status     INTEGER NOT NULL,   -- 0=down, 1=up, 2=pending, 3=maintenance
    ping_ms    INTEGER,            -- nullable
    PRIMARY KEY (monitor_id, time)
);
CREATE INDEX IF NOT EXISTS idx_heartbeats_monitor_time ON heartbeats (monitor_id, time);
```
Dedup is enforced by the primary key: inserts use `INSERT OR IGNORE`.

### `HeartbeatStore` trait (`store/mod.rs`)
Add one method for retention; the rest already exist:
```rust
async fn record_beats(&self, beats: &[Beat]) -> Result<(), AppError>;
async fn uptime(&self, monitor_id: i64, window: Window) -> Result<UptimeResult, AppError>;
async fn incidents(&self, since: DateTime<Utc>) -> Result<Vec<Incident>, AppError>;
async fn prune(&self, older_than: DateTime<Utc>) -> Result<(), AppError>;   // NEW
```
`NoopStore` gains a `prune` returning `Ok(())` (it is still used by the monitors handler tests).

## 4. `SqliteStore` (`store/sqlite.rs`)

- **`connect(database_url) -> Result<Self, AppError>`**: build a `SqlitePool` with
  `SqliteConnectOptions` (`create_if_missing(true)`); ensure the parent directory of a file URL
  exists; execute `schema.sql` (embedded via `include_str!`) to create the table + index.
- **`record_beats(&[Beat])`**: batch `INSERT OR IGNORE` inside one transaction. Time stored as
  RFC3339 UTC string; `MonitorStatus` mapped to its integer code.
- **`uptime(monitor_id, window)`** → `UptimeResult { ratio, coverage }`, count-based:
  - `cutoff = now - window_duration`.
  - `total = COUNT(*)`, `up = COUNT(status = 1)` for that monitor with `time >= cutoff`.
  - `ratio = if total == 0 { 0.0 } else { up as f64 / total as f64 }`.
  - `oldest = MIN(time)` for that monitor **within the window** (`time >= cutoff`). This means a
    window that has no in-window beats reports `coverage = 0.0` even if older rows still exist in
    retention. `covered = now - max(oldest, cutoff)`;
    `coverage = clamp(covered / window_duration, 0.0..=1.0)`; `0.0` if no beats.
- **`prune(older_than)`**: `DELETE FROM heartbeats WHERE time < ?`.
- **`incidents(since)`**: returns `Ok(vec![])` for now — real derivation is the next slice. (The
  `/api/incidents` handler stays stubbed; nothing calls this yet.)

`Window` enum (`store/mod.rs`, already present): `Day`, `Week`, `Month` → 1 / 7 / 30 days.

## 5. Upstream parsing changes (`poller/status_page.rs`)

The monitors slice deliberately dropped beat timestamps. This slice restores them:
- `BeatDto` regains a `time: String` field.
- A UTC parser: `NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")` then `.and_utc()`
  → `DateTime<Utc>`. Isolated here and unit-tested.
- `StatusPageClient::fetch` returns a richer struct instead of just `Vec<Monitor>`:
  ```rust
  pub struct PolledData {
      pub monitors: Vec<Monitor>,
      pub beats: Vec<Beat>,                 // all returned beats, parsed
      pub uptime_24h: std::collections::HashMap<i64, f64>,  // from uptimeList "<id>_24"
  }
  ```
  `map_monitors` is unchanged; a new `extract_beats(&HeartbeatDto) -> Vec<Beat>` and
  `extract_uptime_24h(&HeartbeatDto) -> HashMap<i64, f64>` are added (pure, tested).

## 6. Poller changes (`poller/mod.rs`)

Each tick, after a successful `fetch`:
1. `store.record_beats(&data.beats)`.
2. For each monitor build `UptimeWindow`:
   - `uptime_24h` = `data.uptime_24h.get(&id).copied().unwrap_or(0.0)`.
   - `(uptime_7d, coverage_7d)` = `store.uptime(id, Window::Week)`.
   - `(uptime_30d, coverage_30d)` = `store.uptime(id, Window::Month)`.
3. `store.prune(now - retention_days)`.
4. Build `Snapshot { monitors, uptime, incidents: vec![], last_updated }` → `cache.put_snapshot`.

Store/DB errors during a tick are logged at `warn` and the previous snapshot is kept (same
failure isolation as the monitors slice).

## 7. HTTP (`api/uptime.rs`)

```rust
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<UptimeWindow>>, AppError> {
    match state.cache.get_snapshot().await {
        Some(snapshot) => Ok(Json(snapshot.uptime.clone())),
        None => Err(AppError::NoSnapshot),
    }
}
```
Route `/api/uptime` is already wired in `api/mod.rs`.

## 8. Wiring (`main.rs`)

Replace `NoopStore` with `SqliteStore`:
```rust
let store: Arc<dyn store::HeartbeatStore> =
    Arc::new(SqliteStore::connect(&config.database_url).await.expect("store connect"));
```
`DATABASE_URL` default `sqlite://data/uptime.db`; `connect` creates `data/` and the file.

## 9. Testing

- **UTC parser** (unit): `"2026-06-28 15:59:49.191"` → expected `DateTime<Utc>`.
- **`extract_beats` / `extract_uptime_24h`** over the sanitized fixtures: correct count, status,
  ping, parsed time; `uptime_24h` map keys/values.
- **`SqliteStore`** against a temporary DB (dev-dependency `tempfile`):
  - `record_beats` then `uptime` returns the expected `ratio` and `coverage` for a hand-built beat
    set (e.g. 8 up / 10 total → 0.8);
  - dedup: inserting the same beats twice does not change counts;
  - `prune` removes beats older than the cutoff and keeps newer ones.
- **`/api/uptime` handler**: seeded cache → `200` with the expected windows; empty cache → `503`.
- **Live verification** (manual): run against `KUMA_BASE_URL=https://uptime.samueleruaro.com`,
  `KUMA_STATUS_PAGE_SLUG=homelab`; `curl /api/uptime` → `uptime_24h ≈ 1.0`, `uptime_7d/30d`
  present with **low `coverage`** on a fresh DB; leave it running a couple of cycles and confirm
  the DB file grows and coverage stays consistent. Commands via `~/.cargo/bin/cargo`.

## 10. Out of scope (deferred)

Incident derivation and `GET /api/incidents`; Redis; Prometheus; auth; CORS; `sqlx::migrate!`;
time-weighted uptime; excluding maintenance from the denominator.

## 11. Design-doc reconciliation

`low-level-analysis.md` §10 says handlers issue read-only aggregate queries against the pool,
which conflicts with §7 ("every request served from cache"). This slice resolves it in favour of
§7: **uptime is computed in the poller and served from cache**; the SQLite pool is written and
read only by the poller task. `low-level-analysis.md` §10 will be updated to match.

## 12. References

- `docs/project/low-level-analysis.md` §5b (store), §3/§11 (windows + coverage), §4 (poller).
- `docs/superpowers/specs/2026-06-28-vertical-slice-monitors-design.md` — the slice this builds on.
- Real captured fixtures: `docs/project/fixtures/` (gitignored).
