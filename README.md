# uptime-kuma-rs

A lightweight Rust service that polls an [Uptime Kuma](https://github.com/louislam/uptime-kuma)
instance and re-exposes its monitoring data through a clean, stable, read-only REST API.

Uptime Kuma has no general-purpose read API of its own — its UI talks to the backend over an
undocumented, version-unstable socket.io interface. `uptime-kuma-rs` polls Uptime Kuma's public
status-page endpoints instead, builds up durable history on its own, and serves everything over
plain JSON that other services can depend on without breaking on every Uptime Kuma upgrade.

## Features

- **Three read-only endpoints:** current monitor status, uptime percentages (24h/7d/30d), and
  derived incident history.
- **No socket.io, no scraping the UI:** reads only the public, unauthenticated status-page JSON
  endpoints.
- **Small footprint:** in-memory hot path, SQLite for durable history, no external services
  required for a single instance.
- **Partial-history honesty:** 7d/30d uptime windows report a `coverage` figure alongside the
  ratio, so a fresh deployment doesn't present partial data as a complete number.

## Quick start

Requires a published Uptime Kuma status page (`Settings → Status Pages`) with the monitors you
want exposed.

```bash
git clone https://github.com/1brecane/uptime-kuma-rs.git
cd uptime-kuma-rs

KUMA_BASE_URL=https://uptime.example.com \
KUMA_STATUS_PAGE_SLUG=homelab \
cargo run
```

The server listens on `0.0.0.0:8080` by default and writes its SQLite database to
`data/uptime.db` (created automatically).

```bash
curl http://localhost:8080/api/monitors
curl http://localhost:8080/api/uptime
curl http://localhost:8080/api/incidents
```

## Configuration

Configuration is env-first, with an optional `config.toml` overlay in the working directory.
There is no `.env` auto-loading — export variables yourself, or `source` a `.env` file before
running, if you keep local config in one.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `KUMA_BASE_URL` | yes | — | Base URL of the Uptime Kuma instance |
| `KUMA_STATUS_PAGE_SLUG` | yes | — | Slug of the published status page to poll |
| `POLL_INTERVAL_SECONDS` | no | `60` | Seconds between polls |
| `LISTEN_ADDR` | no | `0.0.0.0:8080` | Address this service listens on |
| `DATABASE_URL` | no | `sqlite://data/uptime.db` | SQLite database URL |
| `HISTORY_RETENTION_DAYS` | no | `31` | Heartbeats older than this are pruned |
| `API_KEY` | no | unset | If set, requires `X-Api-Key: <value>` on requests *(not yet enforced — planned)* |
| `CORS_ALLOWED_ORIGINS` | no | none | Comma-separated allowed origins *(not yet enforced — planned)* |
| `REDIS_URL` | no | unset | Optional shared snapshot cache for multi-replica deployments *(not yet implemented)* |
| `KUMA_METRICS_API_KEY` | no | unset | API key for the `/metrics` fallback source *(not yet implemented)* |

## API

All endpoints are `GET` and read-only.

### `GET /api/monitors`

Current status and latency of each monitor.

```json
[
  { "id": 7, "name": "baikal", "group": "Servizi", "status": "up", "latency_ms": 9 }
]
```

### `GET /api/uptime`

Uptime ratio over 24h/7d/30d, each 7d/30d figure paired with a `coverage` value (0.0–1.0)
showing how much of that window is actually backed by stored history — useful to distinguish a
complete number from a partial one on a freshly started instance.

```json
[
  {
    "monitor_id": 7,
    "uptime_24h": 1.0,
    "uptime_7d": 1.0,
    "uptime_30d": 1.0,
    "coverage_7d": 0.011,
    "coverage_30d": 0.49
  }
]
```

### `GET /api/incidents`

Periods during which a monitor was down, reconstructed from stored heartbeat history.
`resolved_at`/`duration_seconds` are `null` while an incident is still ongoing.

```json
[
  {
    "monitor_id": 7,
    "started_at": "2026-07-15T02:14:03.000Z",
    "resolved_at": "2026-07-15T02:19:41.000Z",
    "duration_seconds": 338
  }
]
```

Before the first successful poll, every endpoint returns `503`.

## How it works

The service follows a **poll → cache → serve** pattern: a background task talks to Uptime Kuma
and SQLite; HTTP handlers never do either.

1. **Poll** (`poller/`, every `POLL_INTERVAL_SECONDS`) — fetches the status page's monitor list
   and recent heartbeats (no auth required; this is the same JSON Uptime Kuma's public status
   page uses).
2. **Persist** — new heartbeats are written to SQLite (`store/`), deduplicated on
   `(monitor_id, time)`. This is the durable source of truth: the status-page API only returns
   24h of history, so 7d/30d uptime and incident history have to be accumulated locally over
   time.
3. **Compute** — for each monitor: `uptime_24h` comes straight from the status page;
   `uptime_7d`/`uptime_30d` and their `coverage` are aggregated from stored heartbeats; incidents
   are reconstructed by detecting up/down transitions across the stored heartbeat history (a SQL
   window-function query, not live in-process diffing — so incident history survives restarts).
4. **Publish** — the results are assembled into one snapshot and atomically swapped into an
   in-memory cache (`cache/`), lock-free on the read side.
5. **Serve** — HTTP handlers (`api/`) do nothing but read the current snapshot from that cache
   and return the relevant slice of it as JSON. A failed poll logs a warning and keeps serving
   the last good snapshot rather than tearing it down.

Two storage tiers exist because they serve different needs: the in-memory snapshot is what makes
reads fast and lock-free, while SQLite is what makes 7d/30d uptime and incident history possible
and durable across restarts. A Redis-backed cache is planned as an optional third tier, only
relevant when running multiple replicas behind a single poller.

## Status

The three planned endpoints (`/api/monitors`, `/api/uptime`, `/api/incidents`) are implemented
and backed by the poll → cache → serve pipeline described above. Not yet implemented: `X-Api-Key`
auth, configurable CORS, the Redis cache tier, and the `/metrics` fallback source.

## Development

```bash
cargo build              # build
cargo run                # run
cargo test                # run tests
cargo test <test_name>    # run a single test
cargo clippy --all-targets
cargo fmt
```

Designed to run on a private network (homelab, VPN, LAN) — it is read-only against Uptime Kuma
and never writes back to it.
