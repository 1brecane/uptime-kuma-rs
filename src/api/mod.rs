use axum::Router;
use axum::routing::get;

use crate::state::AppState;

pub mod auth;
pub mod incidents;
pub mod monitors;
pub mod uptime;

/// Assembles the read-only API router with middleware (low-level §7).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/monitors", get(monitors::handler))
        .route("/api/uptime", get(uptime::handler))
        .route("/api/incidents", get(incidents::handler))
        .with_state(state)
}
