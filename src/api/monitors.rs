use axum::Json;
use axum::extract::State;

use crate::error::AppError;
use crate::model::Monitor;
use crate::state::AppState;

/// `GET /api/monitors` — current status and latency of each monitor. Served from cache (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Monitor>>, AppError> {
    match state.cache.get_snapshot().await {
        Some(snapshot) => Ok(Json(snapshot.monitors)),
        None => Err(AppError::NoSnapshot),
    }
}

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
