use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::model::UptimeWindow;
use crate::state::AppState;

/// `GET /api/uptime` — uptime % over 24h / 7d / 30d windows with coverage (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<UptimeWindow>>, AppError> {
    todo!("read snapshot from cache and return uptime windows")
}
