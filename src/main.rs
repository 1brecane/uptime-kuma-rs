// Temporary during the skeleton phase: behavior is stubbed with `todo!()` and many types are
// defined before they are constructed. Each later implementation plan removes the relevant
// allowances as it fills in real behavior.
#![allow(dead_code, unused_variables, unused_imports)]

mod api;
mod cache;
mod config;
mod error;
mod model;
mod poller;
mod state;
mod store;

use std::sync::Arc;

use crate::cache::memory::MemoryCache;
use crate::config::Config;
use crate::state::AppState;
use crate::store::sqlite::SqliteStore;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // NOTE: skeleton wiring. `Config::load`, the cache/store constructors, the poller, and the
    // server bind are real call sites but their bodies are still `todo!()` — running this will
    // panic at the first stub. Later plans replace the stubs; the shape here is the target.
    let config = Arc::new(Config::load().expect("failed to load config"));

    let cache: Arc<dyn cache::Cache> = Arc::new(MemoryCache::new());
    let store: Arc<dyn store::HeartbeatStore> = Arc::new(
        SqliteStore::connect(&config.database_url)
            .await
            .expect("store connect"),
    );

    let state = AppState {
        cache,
        store,
        config: config.clone(),
        http: reqwest::Client::new(),
    };

    poller::spawn(state.clone());

    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listen address");

    tracing::info!("listening on {}", config.listen_addr);
    axum::serve(listener, app).await.expect("server error");
}
