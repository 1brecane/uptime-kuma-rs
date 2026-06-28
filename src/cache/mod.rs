use std::sync::Arc;

use async_trait::async_trait;

use crate::error::AppError;
use crate::model::Snapshot;

pub mod memory;
pub mod redis;

/// Live snapshot cache (low-level §5a). One writer (the poller), many readers (handlers).
#[async_trait]
pub trait Cache: Send + Sync {
    /// Returns the most recent snapshot, or `None` before the first poll completes.
    async fn get_snapshot(&self) -> Option<Arc<Snapshot>>;
    /// Atomically replaces the cached snapshot.
    async fn put_snapshot(&self, snap: Snapshot) -> Result<(), AppError>;
}
