use chrono::{DateTime, Utc};

use crate::model::{Incident, Monitor};

/// Detects monitors that transitioned up->down between two polls and returns the newly-opened
/// incidents (low-level §4). Closing incidents (setting resolved_at/duration) is handled by the
/// store layer when it reconstructs from stored history (§5b), not here.
pub fn derive(previous: &[Monitor], current: &[Monitor], now: DateTime<Utc>) -> Vec<Incident> {
    todo!("emit a new Incident for each up->down transition, started_at = now")
}
