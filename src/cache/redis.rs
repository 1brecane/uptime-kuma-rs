use async_trait::async_trait;

use crate::model::Snapshot;

use super::Cache;

/// Optional shared cache for multi-replica deployments (low-level §5a). Off unless `REDIS_URL` set.
pub struct RedisCache {
    // Will hold a redis connection/pool + key + TTL; wired in the Redis cache task.
}

impl RedisCache {
    pub fn new(redis_url: &str) -> Self {
        todo!("connect to Redis at {redis_url}")
    }
}

#[async_trait]
impl Cache for RedisCache {
    async fn get_snapshot(&self) -> Option<Snapshot> {
        todo!("GET + deserialize snapshot JSON")
    }

    async fn put_snapshot(&self, snap: Snapshot) {
        todo!("SET serialized snapshot JSON with TTL")
    }
}
