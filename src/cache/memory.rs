use async_trait::async_trait;

use crate::model::Snapshot;

use super::Cache;

/// Default cache for single-instance deployments: lock-free reads via `ArcSwap` (low-level §5a).
pub struct MemoryCache {
    // Will hold `arc_swap::ArcSwapOption<Snapshot>`; wired in the cache implementation task.
}

impl MemoryCache {
    pub fn new() -> Self {
        todo!("initialize ArcSwap-backed in-memory cache")
    }
}

#[async_trait]
impl Cache for MemoryCache {
    async fn get_snapshot(&self) -> Option<Snapshot> {
        todo!("load current Arc<Snapshot>")
    }

    async fn put_snapshot(&self, snap: Snapshot) {
        todo!("swap in the new snapshot")
    }
}
