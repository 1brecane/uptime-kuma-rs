use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};

use crate::model::{Snapshot, UptimeWindow};
use crate::state::AppState;
use crate::store::{HeartbeatStore, UptimeResult, Window};

use self::status_page::{PolledData, StatusPageClient};

pub mod incidents;
pub mod prometheus;
pub mod status_page;

/// Compute per-monitor uptime windows: 24h from the status page, 7d/30d from SQLite.
async fn build_uptime(store: &dyn HeartbeatStore, data: &PolledData) -> Vec<UptimeWindow> {
    let mut windows = Vec::new();
    for m in &data.monitors {
        let week = store
            .uptime(m.id, Window::Week)
            .await
            .unwrap_or(UptimeResult {
                ratio: 0.0,
                coverage: 0.0,
            });
        let month = store
            .uptime(m.id, Window::Month)
            .await
            .unwrap_or(UptimeResult {
                ratio: 0.0,
                coverage: 0.0,
            });
        windows.push(UptimeWindow {
            monitor_id: m.id,
            uptime_24h: data.uptime_24h.get(&m.id).copied().unwrap_or(0.0),
            uptime_7d: week.ratio,
            coverage_7d: week.coverage,
            uptime_30d: month.ratio,
            coverage_30d: month.coverage,
        });
    }
    windows
}

/// Spawns the background poll loop (low-level §4): fetch → persist beats → compute windows →
/// prune → replace the cached snapshot. A failed step logs `warn` and keeps the last snapshot.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = StatusPageClient::new(
            state.config.kuma_base_url.clone(),
            state.config.kuma_status_page_slug.clone(),
            state.http.clone(),
        );
        let mut ticker =
            tokio::time::interval(Duration::from_secs(state.config.poll_interval_seconds));
        let retention = ChronoDuration::days(state.config.history_retention_days as i64);

        loop {
            ticker.tick().await;
            match client.fetch().await {
                Ok(data) => {
                    if let Err(e) = state.store.record_beats(&data.beats).await {
                        tracing::warn!("failed to record beats: {e}");
                    }
                    let uptime = build_uptime(state.store.as_ref(), &data).await;
                    let snapshot = Snapshot {
                        monitors: data.monitors,
                        uptime,
                        incidents: Vec::new(),
                        last_updated: Utc::now(),
                    };
                    if let Err(e) = state.cache.put_snapshot(snapshot).await {
                        tracing::warn!("failed to store snapshot: {e}");
                    }
                    if let Err(e) = state.store.prune(Utc::now() - retention).await {
                        tracing::warn!("failed to prune old beats: {e}");
                    }
                }
                Err(e) => tracing::warn!("poll failed: {e}"),
            }
        }
    })
}
