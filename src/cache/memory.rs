use std::sync::Arc;

use arc_swap::ArcSwapOption;
use async_trait::async_trait;

use crate::error::AppError;
use crate::model::Snapshot;

use super::Cache;

/// Default cache for single-instance deployments: lock-free reads via `ArcSwap` (low-level §5a).
pub struct MemoryCache {
    snapshot: ArcSwapOption<Snapshot>,
}

impl MemoryCache {
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwapOption::empty(),
        }
    }
}

#[async_trait]
impl Cache for MemoryCache {
    async fn get_snapshot(&self) -> Option<Arc<Snapshot>> {
        self.snapshot.load_full()
    }

    async fn put_snapshot(&self, snap: Snapshot) -> Result<(), AppError> {
        self.snapshot.store(Some(Arc::new(snap)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Monitor, MonitorStatus, Snapshot};
    use chrono::Utc;

    fn empty_snapshot() -> Snapshot {
        Snapshot {
            monitors: vec![],
            uptime: vec![],
            incidents: vec![],
            last_updated: Utc::now(),
        }
    }

    #[tokio::test]
    async fn empty_until_put() {
        let cache = MemoryCache::new();
        assert!(cache.get_snapshot().await.is_none());
    }

    #[tokio::test]
    async fn returns_last_put_snapshot() {
        let cache = MemoryCache::new();
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
        let snap = cache.get_snapshot().await.expect("snapshot present");
        assert_eq!(snap.monitors.len(), 1);
        assert_eq!(snap.monitors[0].name, "service-a");
    }
}
