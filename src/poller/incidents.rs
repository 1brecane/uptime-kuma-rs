use crate::model::{Incident, Monitor};

/// Derives incidents by diffing heartbeat status transitions across polls (low-level §4).
pub fn derive(previous: &[Monitor], current: &[Monitor]) -> Vec<Incident> {
    todo!("open incidents on up->down, close on down->up")
}
