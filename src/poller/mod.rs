use std::time::Duration;

use chrono::Utc;

use crate::model::Snapshot;
use crate::state::AppState;

use self::status_page::StatusPageClient;

pub mod incidents;
pub mod prometheus;
pub mod status_page;

/// Spawns the background poll loop (low-level §4): each tick, fetch the status-page endpoints,
/// build a snapshot, and replace the cached snapshot. A failed poll logs `warn` and leaves the
/// previous snapshot in place.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = StatusPageClient::new(
            state.config.kuma_base_url.clone(),
            state.config.kuma_status_page_slug.clone(),
            state.http.clone(),
        );
        let mut ticker =
            tokio::time::interval(Duration::from_secs(state.config.poll_interval_seconds));

        loop {
            ticker.tick().await;
            match client.fetch().await {
                Ok(monitors) => {
                    let snapshot = Snapshot {
                        monitors,
                        uptime: Vec::new(),
                        incidents: Vec::new(),
                        last_updated: Utc::now(),
                    };
                    if let Err(e) = state.cache.put_snapshot(snapshot).await {
                        tracing::warn!("failed to store snapshot: {e}");
                    }
                }
                Err(e) => tracing::warn!("poll failed: {e}"),
            }
        }
    })
}
