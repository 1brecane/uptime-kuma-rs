use crate::error::AppError;
use crate::model::Snapshot;

/// Primary data source: the public status-page JSON endpoints (low-level §4). No auth required.
pub struct StatusPageClient {
    // Will hold base URL, slug, and a shared reqwest client.
}

impl StatusPageClient {
    pub fn new(base_url: String, slug: String, http: reqwest::Client) -> Self {
        todo!("construct StatusPageClient")
    }

    /// Fetch current heartbeats + 24h uptime and build a snapshot.
    pub async fn fetch(&self) -> Result<Snapshot, AppError> {
        todo!("GET /api/status-page/heartbeat/:slug and map into Snapshot")
    }
}
