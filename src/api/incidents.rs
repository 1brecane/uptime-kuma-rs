use axum::Json;
use axum::extract::State;

use crate::error::AppError;
use crate::model::Incident;
use crate::state::AppState;

/// `GET /api/incidents` — history of incidents (monitors that went down) (low-level §7).
pub async fn handler(State(state): State<AppState>) -> Result<Json<Vec<Incident>>, AppError> {
    todo!("read snapshot from cache and return incidents")
}
