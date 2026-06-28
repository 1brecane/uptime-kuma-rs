use crate::state::AppState;

pub mod incidents;
pub mod prometheus;
pub mod status_page;

/// Spawns the background poll loop (low-level §4): on each tick, fetch from the source(s),
/// build a snapshot, persist beats, and atomically replace the cached snapshot.
pub fn spawn(state: AppState) {
    todo!("spawn tokio interval loop driving the configured sources")
}
