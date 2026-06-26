# Analisi di Alto Livello — `uptime-kuma-rs`

## 1. Obiettivo

Uptime Kuma è uno strumento di monitoring self-hosted molto diffuso, ma espone i propri dati esclusivamente tramite una connessione **socket.io** — non documentata ufficialmente e soggetta a cambiamenti tra versioni. Questo rende difficile consumarne i dati da servizi esterni senza dipendere da un client socket.io fragile e difficile da mantenere.

`uptime-kuma-rs` nasce per risolvere questo problema: è uno scraper leggero scritto in Rust che interroga periodicamente un'istanza di Uptime Kuma e **ri-espone i dati tramite una REST API pulita e stabile**.

L'obiettivo secondario è fornire alla community uno strumento mancante — ad oggi non esiste uno scraper Rust per Uptime Kuma — pensato per essere composable, ovvero facilmente integrabile come sorgente dati in pipeline di monitoring più ampie.

---

## 2. Architettura

Il servizio segue un pattern semplice: **poll → cache → serve**.

```
[Uptime Kuma instance]
        │
        │  HTTP polling (REST + /metrics Prometheus)
        ▼
[uptime-kuma-rs]
        │
        ├── /api/monitors       stato e latenza di ogni monitor
        ├── /api/uptime         uptime % per finestre temporali
        └── /api/incidents      storico degli incident
```

Il servizio è **stateless** rispetto alla persistenza — non ha un database proprio. I dati vengono tenuti in memoria tra un poll e l'altro, con un layer di cache opzionale (Redis) per ridurre la pressione sull'istanza Uptime Kuma in caso di molti consumer.

---

## 3. Scelte Tecniche

### Rust + Axum
Rust garantisce un footprint minimo (< 10MB di memoria a regime) e performance adeguate per un servizio di questo tipo. Axum è il framework HTTP asincrono più maturo nell'ecosistema Rust, costruito su Tokio.

### Polling HTTP invece di socket.io
Uptime Kuma espone internamente un'interfaccia socket.io usata dalla propria UI. Tuttavia:

- Il protocollo **non è documentato** ufficialmente
- Può cambiare tra versioni minori senza preavviso
- Le librerie socket.io per Rust non sono mature

La strategia adottata è quindi il **polling HTTP periodico**, con due sorgenti:

- **Endpoint REST interni** di Uptime Kuma per dati strutturati (monitor status, latenza)
- **`/metrics`** in formato Prometheus (se abilitato nelle impostazioni di Uptime Kuma) come sorgente alternativa per uptime % e serie temporali

Questo approccio è meno real-time rispetto a socket.io, ma molto più robusto e manutenibile nel lungo periodo.

### Intervallo di polling configurabile
Il TTL del polling è configurabile via file di configurazione o variabile d'ambiente, permettendo di bilanciare freschezza dei dati e carico sull'istanza Uptime Kuma.

---

## 4. API Esposta

Tutti gli endpoint sono in sola lettura (`GET`).

### `GET /api/monitors`
Restituisce lo stato attuale di tutti i monitor configurati.

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
Restituisce la percentuale di uptime per finestre temporali standard.

```json
[
  {
    "monitor_id": 1,
    "uptime_24h": 99.98,
    "uptime_7d": 99.95,
    "uptime_30d": 99.90
  }
]
```

### `GET /api/incidents`
Restituisce lo storico degli incident (monitor andati down).

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

## 5. Sicurezza

Il servizio è pensato per girare in una **rete privata** (homelab, VPN, LAN), non esposto direttamente a Internet.

- Le credenziali di Uptime Kuma (se configurate) non vengono mai ri-esposte nelle risposte
- Il servizio non scrive nulla sull'istanza Uptime Kuma — solo lettura
- CORS configurabile per restringere i consumer autorizzati
- Autenticazione opzionale tramite API key sull'header `X-Api-Key` per proteggere gli endpoint in ambienti semi-esposti

---

## 6. Integrazione con `homelab-api`

`uptime-kuma-rs` è la sorgente dati Uptime Kuma per [`homelab-api`](https://github.com/1brecane/homelab-api), il gateway privato che aggrega monitoring e dati Proxmox per il portfolio frontend.

```
[uptime-kuma-rs]  ──GET /api/*──> [homelab-api] ──> [Portfolio frontend]
```

`homelab-api` consuma gli endpoint di questo servizio, li combina con i dati Proxmox, filtra ulteriormente e serve tutto tramite un unico endpoint `/api/dashboard`. I due servizi sono volutamente separati per mantenere `uptime-kuma-rs` generico e riutilizzabile indipendentemente dal contesto portfolio.
