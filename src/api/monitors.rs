use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::model::Monitor;
use crate::state::AppState;

/// `GET /api/monitors` — current status and latency of each monitor. Served from cache (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Monitor>>, AppError> {
    todo!("read snapshot from cache and return monitors")
}
