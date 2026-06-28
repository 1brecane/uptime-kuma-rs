use crate::error::AppError;
use crate::model::Monitor;

/// Optional fallback source: parses the `/metrics` Prometheus endpoint (low-level §4).
/// Provides current status only — no id, no uptime, no incidents.
pub async fn fetch_status(
    metrics_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<Monitor>, AppError> {
    todo!("fetch and parse monitor_status / monitor_response_time metrics")
}
