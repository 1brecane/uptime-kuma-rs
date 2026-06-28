# High-Level Analysis — `uptime-kuma-rs`

## 1. Goal

Uptime Kuma is a widely used self-hosted monitoring tool, but it exposes its data exclusively
through a **socket.io** connection — undocumented and subject to change between versions. This
makes it hard to consume its data from external services without depending on a fragile,
hard-to-maintain socket.io client.

`uptime-kuma-rs` exists to solve this problem: it is a lightweight scraper written in Rust that
periodically queries an Uptime Kuma instance and **re-exposes the data through a clean, stable
REST API**.

A secondary goal is to give the community a missing tool — there is currently no Rust scraper
for Uptime Kuma — designed to be composable, i.e. easy to integrate as a data source in larger
monitoring pipelines.

---

## 2. Architecture

The service follows a simple pattern: **poll → cache → serve**.

```
[Uptime Kuma instance]
        │
        │  HTTP polling of the PUBLIC status-page JSON API
        │  (GET /api/status-page/heartbeat/:slug)
        ▼
[uptime-kuma-rs]
        │
        ├── /api/monitors       status and latency of each monitor
        ├── /api/uptime         uptime % per time window
        └── /api/incidents      incident history (derived locally)
```

The service separates storage by need. Two tiers are always present (in-memory + SQLite); a
third (Redis) is optional, used only when scaling out:

- **In-memory snapshot** *(default)* — the latest poll result, served on the hot request path
  with no I/O. For the expected single-instance deployment this is all the live cache that is
  needed.
- **SQLite** *(required)* — a small embedded database that retains **heartbeat history**. The
  status-page API only exposes 24h uptime, so longer windows (7d/30d) and reliable incident
  reconstruction require accumulating beats locally over time. SQLite keeps this history durable
  across restarts without operating a separate database server — it is a file, not a service.
- **Redis** *(optional)* — a shared snapshot cache used **only** when running **multiple
  replicas** that must serve from one poller's output. A single instance does not need it; the
  in-memory cache covers that case. The `Cache` abstraction keeps it a drop-in, off by default.

In short: in-memory for speed (always), SQLite for durable history (always), Redis only if you
scale out to multiple replicas.

---

## 3. Technical Choices

### Rust + Axum
Rust guarantees a minimal footprint (< 10MB of memory at steady state) and adequate performance
for a service of this kind. Axum is the most mature asynchronous HTTP framework in the Rust
ecosystem, built on Tokio.

### Polling the public status-page API instead of socket.io
Uptime Kuma's only general-purpose read interface is a **socket.io** API used by its own UI.
It is unsuitable as a dependency:

- It is **not officially supported** for third-party use; breaking changes can land between
  versions without notice
- socket.io client libraries for Rust are immature

Uptime Kuma does **not** expose a general read REST API. What it *does* expose publicly are the
**status-page JSON endpoints** that its own status pages consume:

- `GET /api/status-page/:slug` — status-page config and the list of monitors on it
- `GET /api/status-page/heartbeat/:slug` — per-monitor recent heartbeats (`status`, `time`,
  `ping`, `msg`) plus an `uptimeList` of uptime percentages (e.g. `"<id>_24"` for 24h)

These are **public, require no authentication, and return clean JSON** — making them the chosen
primary data source. The only prerequisite is that a status page exists and is published with
the relevant monitors on it. This approach is less real-time than socket.io but far more robust
and maintainable.

**Secondary / fallback sources** (documented in the low-level analysis):

- **`/metrics`** (Prometheus) — exports only `monitor_status` and `monitor_response_time` with
  no monitor id and no uptime/incident data; usable only as a supplementary current-status
  signal, and requires Basic Auth or an API key.
- **socket.io** — the only way to obtain arbitrary uptime windows (7d/30d) and the full monitor
  list independently of a status page, accepted as a last resort despite its instability.

**Incidents are not exposed by any endpoint** and are derived locally from heartbeat
status transitions (`up → down` opens, `down → up` closes).

### Configurable polling interval
The polling TTL is configurable via a configuration file or environment variable, allowing the
trade-off between data freshness and load on the Uptime Kuma instance to be balanced.

---

## 4. Exposed API

All endpoints are read-only (`GET`).

### `GET /api/monitors`
Returns the current status of all configured monitors.

```json
[
  {
    "id": 1,
    "name": "Portfolio",
    "status": "up",
    "latency_ms": 42
  }
]
```

### `GET /api/uptime`
Returns the uptime percentage for standard time windows. 24h comes from Uptime Kuma; 7d/30d are
computed from locally accumulated history. Each long window carries a `coverage` value in
`[0,1]` (history span / window) so consumers can tell a complete figure from one still based on
partial history on a freshly started instance.

```json
[
  {
    "monitor_id": 1,
    "uptime_24h": 99.98,
    "uptime_7d": 99.95,
    "uptime_30d": 99.90,
    "coverage_7d": 1.0,
    "coverage_30d": 0.42
  }
]
```

### `GET /api/incidents`
Returns the history of incidents (monitors that went down).

```json
[
  {
    "monitor_id": 1,
    "started_at": "2025-06-10T14:32:00Z",
    "resolved_at": "2025-06-10T14:38:00Z",
    "duration_seconds": 360
  }
]
```

---

## 5. Security

The service is intended to run on a **private network** (homelab, VPN, LAN), not exposed
directly to the internet.

- Uptime Kuma credentials (if configured) are never re-exposed in responses
- The service never writes anything to the Uptime Kuma instance — read-only
- Configurable CORS to restrict authorized consumers
- Optional authentication via an API key on the `X-Api-Key` header to protect endpoints in
  semi-exposed environments

---

## 6. Integration with `homelab-api`

`uptime-kuma-rs` is the Uptime Kuma data source for
[`homelab-api`](https://github.com/1brecane/homelab-api), the private gateway that aggregates
monitoring and Proxmox data for the portfolio frontend.

```
[uptime-kuma-rs]  ──GET /api/*──> [homelab-api] ──> [Portfolio frontend]
```

`homelab-api` consumes this service's endpoints, combines them with Proxmox data, filters
further, and serves everything through a single `/api/dashboard` endpoint. The two services are
deliberately kept separate to keep `uptime-kuma-rs` generic and reusable independently of the
portfolio context.
