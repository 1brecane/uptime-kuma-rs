# Design Spec — Slice: Incident derivation + `GET /api/incidents`

> **Status:** Approved design, ready for implementation planning.
> **Date:** 2026-07-18
> **Scope:** Third vertical slice, on top of the monitors and SQLite/uptime slices. Derived from
> [`docs/project/low-level-analysis.md`](../../project/low-level-analysis.md) §4 (incident
> derivation) and §5b (heartbeat store).

## 1. Goal

Reconstruct incident history (periods a monitor was down) from the heartbeats already persisted
in SQLite, and serve it via `GET /api/incidents`, mirroring the monitors/uptime slices: computed
in the poller, served from the in-memory snapshot.

**Success criteria:**

- With the binary configured against the real instance (slug `homelab`, reachable over Tailscale),
  `GET /api/incidents` returns closed incidents (`resolved_at` + `duration_seconds` set) and any
  currently-ongoing incident (`resolved_at: None`) per monitor.
- Incidents survive process restarts — they are reconstructed from stored heartbeats, not
  in-process diffing between polls.
- `Pending`/`Maintenance` states never open or close an incident on their own; only a transition
  into `Down` opens one, and a transition out of `Down` closes it.
- `GET /api/incidents` returns `503` until the first successful poll (same as `/api/monitors` and
  `/api/uptime`).
- `cargo build`, `cargo clippy --all-targets`, `cargo fmt --check`, `cargo test` all pass; store
  logic is covered by tests against a temporary SQLite database (no network in tests).

## 2. Decisions (settled during brainstorming)

| Decision | Choice | Rationale |
| --- | --- | --- |
| Derivation source | **Stored heartbeats (SQL)**, not in-process diffing between polls | Survives restarts; a single source of truth shared with `/api/uptime`. Supersedes the `poller/incidents.rs::derive` stub. |
| Where computed | **In the poller → cache**, like uptime/monitors | Lock-free read path; no per-request DB load. |
| What opens/closes an incident | Transition **into** `Down` opens; transition **out of** `Down` (to `Up`, `Pending`, or `Maintenance`) closes | Matches low-level §4 ("up → down opens, down → up closes"); avoids maintenance windows being misread as incidents. |
| Detection mechanism | SQL window function (`LAG` over `(monitor_id, time)`) to find transition rows, resolved into open/close pairs in Rust | One indexed scan of the (already retention-bounded) table; no need to fetch every row into the app to replay state. |
| First-ever beat is `Down` | Opens an incident at that beat's time (no earlier data to know when it actually started) | Consistent with the "partial coverage" philosophy already used for `/api/uptime`: report what the data supports, don't fabricate an earlier start. |
| `since` semantics | Compute over the full retained history (already bounded by pruning), then **keep** an incident if it's still open OR `resolved_at >= since` | Cheap at this scale (homelab, single instance); avoids losing context on incidents that started before `since` but are still relevant. |
| `poller/incidents.rs` | **Removed** | Its live two-poll diffing approach is superseded by the SQL reconstruction; keeping it around as dead code violates the project's no-dead-code stance. |
| Ordering | Most recent `started_at` first | More useful default for API consumers than insertion order. |

## 3. Data model

No changes. `Incident { monitor_id: i64, started_at: DateTime<Utc>, resolved_at: Option<DateTime<Utc>>, duration_seconds: Option<u64> }`
(`model.rs`) already matches this shape.

## 4. `SqliteStore::incidents` (`store/sqlite.rs`)

Replace the `Ok(Vec::new())` stub with a real implementation:

1. Query transition rows only (not every heartbeat):
   ```sql
   WITH ordered AS (
       SELECT monitor_id, time, status,
              LAG(status) OVER (PARTITION BY monitor_id ORDER BY time) AS prev_status
       FROM heartbeats
   )
   SELECT monitor_id, time, status
   FROM ordered
   WHERE (status = 0 AND (prev_status IS NULL OR prev_status != 0))   -- opens
      OR (prev_status = 0 AND status != 0)                           -- closes
   ORDER BY monitor_id, time
   ```
2. Walk the ordered rows once, keeping a `HashMap<monitor_id, started_at>` of currently-open
   incidents:
   - `status == Down` → record/overwrite the open start time for that monitor.
   - otherwise (row only appears here because `prev_status == Down`) → pop the open start time,
     emit a closed `Incident` with `duration_seconds = (time - started_at)`.
3. Any entries left in the open map after the scan become ongoing incidents
   (`resolved_at: None`, `duration_seconds: None`).
4. Filter: keep incidents where `resolved_at.is_none() || resolved_at >= since`.
5. Sort by `started_at` descending.

Time parsing reuses the existing `DateTime::parse_from_rfc3339` approach already used for the
`oldest` beat time in `uptime()`.

## 5. Poller changes (`poller/mod.rs`)

After building `uptime` and before assembling the snapshot:
```rust
let incidents = state
    .store
    .incidents(Utc::now() - retention)
    .await
    .inspect_err(|e| tracing::warn!("incidents query failed: {e}"))
    .unwrap_or_default();
```
`Snapshot.incidents` is set from this instead of the current hardcoded `Vec::new()`. On error, log
`warn` and degrade to an empty list for that tick — the rest of the snapshot (monitors, uptime)
is still fresh and gets published regardless. This mirrors the existing precedent: a failed
`uptime()` call for one monitor already degrades to a zeroed `UptimeResult` rather than blocking
the tick or reusing stale data.

`poller/incidents.rs` and its `pub mod incidents;` declaration are deleted.

## 6. HTTP (`api/incidents.rs`)

```rust
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Incident>>, AppError> {
    match state.cache.get_snapshot().await {
        Some(snapshot) => Ok(Json(snapshot.incidents.clone())),
        None => Err(AppError::NoSnapshot),
    }
}
```
Route `/api/incidents` is already wired in `api/mod.rs`.

## 7. Wiring (`main.rs`)

No changes — `SqliteStore` is already wired in from the previous slice.

## 8. Testing

- **`SqliteStore::incidents`** against a temporary DB:
  - a `Down` beat followed later by an `Up` beat produces one closed incident with the correct
    `started_at`/`resolved_at`/`duration_seconds`;
  - a monitor still `Down` at the end of the recorded beats produces an incident with
    `resolved_at: None`;
  - a monitor whose very first recorded beat is `Down` still opens an incident (no crash on
    `prev_status IS NULL`);
  - transitions among `Up`/`Pending`/`Maintenance` (never touching `Down`) produce zero incidents;
  - an incident fully resolved before `since` is excluded; one resolved after `since`, and any
    ongoing incident regardless of `since`, are included;
  - two monitors with interleaved beats are tracked independently (no cross-monitor bleed).
- **`/api/incidents` handler**: seeded cache → `200` with the expected incidents; empty cache →
  `503` (mirrors the existing `uptime.rs` handler tests).
- **Live verification** (manual): run against `KUMA_BASE_URL=https://uptime.samueleruaro.com`,
  `KUMA_STATUS_PAGE_SLUG=homelab` (Tailscale up); `curl /api/incidents` and confirm the shape; if
  no monitor is currently/recently down, verify against the previous session's `data/uptime.db`
  history instead of waiting for a live down event.

## 9. Out of scope (deferred)

Redis; Prometheus; auth; CORS; pagination/filtering on `/api/incidents`; excluding maintenance
periods from surrounding incidents' duration; a dedicated `open`/`ongoing` boolean field (implied
by `resolved_at: None`).

## 10. References

- `docs/project/low-level-analysis.md` §4 (incident derivation), §5b (store).
- `docs/superpowers/specs/2026-07-03-sqlite-uptime-slice-design.md` — the slice this builds on;
  same poller-computes/cache-serves pattern.
