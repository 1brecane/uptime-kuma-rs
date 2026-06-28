use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::AppError;
use crate::state::AppState;

/// `X-Api-Key` auth middleware (low-level §7). No-op when `config.api_key` is unset.
pub async fn require_api_key(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    todo!("constant-time compare X-Api-Key against config.api_key when set")
}
