use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Unified application error. Internal detail is logged, not leaked to clients (low-level §9).
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("upstream fetch failed: {0}")]
    Upstream(String),

    #[error("failed to parse upstream payload: {0}")]
    Parse(String),

    #[error("cache error: {0}")]
    Cache(String),

    #[error("storage error: {0}")]
    Store(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("no snapshot available yet")]
    NoSnapshot,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::NoSnapshot => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
            AppError::Parse(_) | AppError::Cache(_) | AppError::Store(_) | AppError::Config(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        // Public message kept generic; full error is logged by the caller / trace layer.
        let body = Json(json!({ "error": status.canonical_reason().unwrap_or("error") }));
        (status, body).into_response()
    }
}
