use std::collections::HashMap;

use serde::Deserialize;

use crate::error::AppError;
use crate::model::{Monitor, MonitorStatus};

// --- Internal DTOs: deserialize the raw status-page JSON. Never leave this module. ---

#[derive(Debug, Deserialize)]
pub(crate) struct StatusPageConfigDto {
    #[serde(rename = "publicGroupList")]
    pub(crate) public_group_list: Vec<GroupDto>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroupDto {
    pub(crate) name: String,
    #[serde(rename = "monitorList")]
    pub(crate) monitor_list: Vec<MonitorDto>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MonitorDto {
    pub(crate) id: i64,
    pub(crate) name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeartbeatDto {
    #[serde(rename = "heartbeatList")]
    pub(crate) heartbeat_list: HashMap<String, Vec<BeatDto>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BeatDto {
    pub(crate) status: u8,
    pub(crate) ping: Option<f64>,
}

/// Join config (names + groups) and heartbeat (status + latency) into domain `Monitor`s.
/// "Latest" status is the LAST beat in each list (beats are in ascending time order).
pub(crate) fn map_monitors(config: &StatusPageConfigDto, heartbeat: &HeartbeatDto) -> Vec<Monitor> {
    let mut monitors = Vec::new();
    for group in &config.public_group_list {
        for m in &group.monitor_list {
            let last_beat = heartbeat
                .heartbeat_list
                .get(&m.id.to_string())
                .and_then(|beats| beats.last());

            let status = match last_beat {
                Some(b) => match b.status {
                    0 => MonitorStatus::Down,
                    1 => MonitorStatus::Up,
                    2 => MonitorStatus::Pending,
                    3 => MonitorStatus::Maintenance,
                    _ => MonitorStatus::Pending,
                },
                None => MonitorStatus::Pending,
            };

            let latency_ms = match (status, last_beat) {
                (MonitorStatus::Up, Some(b)) => b.ping.map(|p| p as u32),
                _ => None,
            };

            monitors.push(Monitor {
                id: m.id,
                name: m.name.clone(),
                group: Some(group.name.clone()),
                status,
                latency_ms,
            });
        }
    }
    monitors
}

/// Primary data source client: the public status-page JSON endpoints (low-level §4). No auth.
pub struct StatusPageClient {
    base_url: String,
    slug: String,
    http: reqwest::Client,
}

impl StatusPageClient {
    pub fn new(base_url: String, slug: String, http: reqwest::Client) -> Self {
        Self {
            base_url,
            slug,
            http,
        }
    }

    /// Fetch both status-page endpoints and map them into the domain monitors.
    pub async fn fetch(&self) -> Result<Vec<Monitor>, AppError> {
        let config_url = format!("{}/api/status-page/{}", self.base_url, self.slug);
        let heartbeat_url = format!("{}/api/status-page/heartbeat/{}", self.base_url, self.slug);

        let config: StatusPageConfigDto = self
            .http
            .get(&config_url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::Parse(e.to_string()))?;

        let heartbeat: HeartbeatDto = self
            .http
            .get(&heartbeat_url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::Parse(e.to_string()))?;

        Ok(map_monitors(&config, &heartbeat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MonitorStatus;

    fn load() -> (StatusPageConfigDto, HeartbeatDto) {
        let config: StatusPageConfigDto =
            serde_json::from_str(include_str!("../../tests/fixtures/status-page-config.json"))
                .expect("config fixture parses");
        let heartbeat: HeartbeatDto =
            serde_json::from_str(include_str!("../../tests/fixtures/status-page-heartbeat.json"))
                .expect("heartbeat fixture parses");
        (config, heartbeat)
    }

    #[test]
    fn maps_status_latency_and_group() {
        let (config, heartbeat) = load();
        let monitors = map_monitors(&config, &heartbeat);
        let by_id = |id: i64| monitors.iter().find(|m| m.id == id).expect("monitor present");

        assert_eq!(monitors.len(), 5);

        // group passed through from publicGroupList
        assert_eq!(by_id(1).group.as_deref(), Some("Servizi"));

        // up: status + latency from the LAST beat (ascending order)
        assert_eq!(by_id(1).status, MonitorStatus::Up);
        assert_eq!(by_id(1).latency_ms, Some(7));

        // down: no latency
        assert_eq!(by_id(2).status, MonitorStatus::Down);
        assert_eq!(by_id(2).latency_ms, None);

        // maintenance (status 3)
        assert_eq!(by_id(3).status, MonitorStatus::Maintenance);
        assert_eq!(by_id(3).latency_ms, None);

        // explicit pending (status 2)
        assert_eq!(by_id(5).status, MonitorStatus::Pending);
        assert_eq!(by_id(5).latency_ms, None);

        // in config but missing from heartbeat -> pending, no latency
        assert_eq!(by_id(4).status, MonitorStatus::Pending);
        assert_eq!(by_id(4).latency_ms, None);
    }
}
