use axum::Json;
use axum::extract::State;

use crate::error::AppError;
use crate::model::Incident;
use crate::state::AppState;

/// `GET /api/incidents` — history of incidents (monitors that went down) (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Incident>>, AppError> {
    match state.cache.get_snapshot().await {
        Some(snapshot) => Ok(Json(snapshot.incidents.clone())),
        None => Err(AppError::NoSnapshot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use chrono::Utc;

    use crate::cache::Cache;
    use crate::cache::memory::MemoryCache;
    use crate::config::Config;
    use crate::model::Snapshot;
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

    fn incident() -> Incident {
        Incident {
            monitor_id: 1,
            started_at: Utc::now(),
            resolved_at: None,
            duration_seconds: None,
        }
    }

    #[tokio::test]
    async fn returns_incidents_from_cache() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        cache
            .put_snapshot(Snapshot {
                monitors: vec![],
                uptime: vec![],
                incidents: vec![incident()],
                last_updated: Utc::now(),
            })
            .await
            .unwrap();

        let Json(body) = handler(State(state_with(cache))).await.unwrap();
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].monitor_id, 1);
        assert_eq!(body[0].resolved_at, None);
    }

    #[tokio::test]
    async fn returns_no_snapshot_error_when_empty() {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        let err = handler(State(state_with(cache))).await.unwrap_err();
        assert!(matches!(err, AppError::NoSnapshot));
    }
}
