use std::sync::Arc;

use crate::cache::Cache;
use crate::config::Config;
use crate::store::HeartbeatStore;

/// Shared application state (low-level §6). Cheaply cloneable; attached via `Router::with_state`.
#[derive(Clone)]
pub struct AppState {
    pub cache: Arc<dyn Cache>,
    pub store: Arc<dyn HeartbeatStore>,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
}
