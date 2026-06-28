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
    async fn get_snapshot(&self) -> Option<Snapshot> {
        self.snapshot.load_full().map(|arc| (*arc).clone())
    }

    async fn put_snapshot(&self, snap: Snapshot) -> Result<(), AppError> {
        self.snapshot.store(Some(Arc::new(snap)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Snapshot;
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
        cache.put_snapshot(empty_snapshot()).await.unwrap();
        assert!(cache.get_snapshot().await.is_some());
    }
}
