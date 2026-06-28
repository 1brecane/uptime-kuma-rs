use serde::Deserialize;

use crate::error::AppError;

/// Service configuration, loaded env-first with an optional TOML overlay (low-level §8).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Uptime Kuma base URL (required). Env: `KUMA_BASE_URL`.
    pub kuma_base_url: String,
    /// Status-page slug, the primary data source (required). Env: `KUMA_STATUS_PAGE_SLUG`.
    pub kuma_status_page_slug: String,
    /// Poll interval in seconds. Env: `POLL_INTERVAL_SECONDS`.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_seconds: u64,
    /// Optional `/metrics` API key (fallback source). Env: `KUMA_METRICS_API_KEY`.
    #[serde(default)]
    pub kuma_metrics_api_key: Option<String>,
    /// Listen address. Env: `LISTEN_ADDR`.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    /// This service's own API key for `X-Api-Key` auth. Env: `API_KEY`.
    #[serde(default)]
    pub api_key: Option<String>,
    /// CORS allowed origins. Env: `CORS_ALLOWED_ORIGINS`.
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
    /// SQLite database URL. Env: `DATABASE_URL`.
    #[serde(default = "default_database_url")]
    pub database_url: String,
    /// Heartbeat retention in days (must exceed 30d window + margin). Env: `HISTORY_RETENTION_DAYS`.
    #[serde(default = "default_retention_days")]
    pub history_retention_days: u32,
    /// Redis URL; when unset, the in-memory cache is used. Env: `REDIS_URL`.
    #[serde(default)]
    pub redis_url: Option<String>,
}

fn default_poll_interval() -> u64 {
    60
}
fn default_listen_addr() -> String {
    "0.0.0.0:8080".to_string()
}
fn default_database_url() -> String {
    "sqlite://data/uptime.db".to_string()
}
fn default_retention_days() -> u32 {
    31
}

impl Config {
    /// Load configuration: optional `config.toml` overlaid by environment variables.
    pub fn load() -> Result<Self, AppError> {
        use figment::Figment;
        use figment::providers::{Env, Format, Toml};

        Figment::new()
            .merge(Toml::file("config.toml"))
            .merge(Env::raw())
            .extract()
            .map_err(|e| AppError::Cache(format!("config load failed: {e}")))
    }
}
