use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::AppError;
use crate::model::{Monitor, MonitorStatus};
use crate::store::Beat;

// --- Internal DTOs: deserialize the raw status-page JSON. Never leave this module. ---

#[derive(Debug, Deserialize)]
struct StatusPageConfigDto {
    #[serde(rename = "publicGroupList")]
    public_group_list: Vec<GroupDto>,
}

#[derive(Debug, Deserialize)]
struct GroupDto {
    name: String,
    #[serde(rename = "monitorList")]
    monitor_list: Vec<MonitorDto>,
}

#[derive(Debug, Deserialize)]
struct MonitorDto {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct HeartbeatDto {
    #[serde(rename = "heartbeatList")]
    heartbeat_list: HashMap<String, Vec<BeatDto>>,
    #[serde(rename = "uptimeList", default)]
    uptime_list: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
struct BeatDto {
    status: u8,
    ping: Option<f64>,
    time: String,
}

fn status_from_code(code: u8) -> MonitorStatus {
    match code {
        0 => MonitorStatus::Down,
        1 => MonitorStatus::Up,
        2 => MonitorStatus::Pending,
        3 => MonitorStatus::Maintenance,
        _ => MonitorStatus::Pending,
    }
}

/// Join config (names + groups) and heartbeat (status + latency) into domain `Monitor`s.
/// "Latest" status is the LAST beat in each list (beats are in ascending time order).
fn map_monitors(config: &StatusPageConfigDto, heartbeat: &HeartbeatDto) -> Vec<Monitor> {
    let mut monitors = Vec::new();
    for group in &config.public_group_list {
        for m in &group.monitor_list {
            let last_beat = heartbeat
                .heartbeat_list
                .get(&m.id.to_string())
                .and_then(|beats| beats.last());

            let status = match last_beat {
                Some(b) => status_from_code(b.status),
                None => MonitorStatus::Pending,
            };

            let latency_ms = match (status, last_beat) {
                (MonitorStatus::Up, Some(b)) => b.ping.map(|p| p.round() as u32),
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

/// Parse Uptime Kuma's `"%Y-%m-%d %H:%M:%S%.f"` (no timezone; treated as UTC).
fn parse_kuma_time(s: &str) -> Option<DateTime<Utc>> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .map(|ndt| ndt.and_utc())
}

/// All heartbeats across all monitors, mapped to domain `Beat`s. Unparseable rows are skipped.
fn extract_beats(heartbeat: &HeartbeatDto) -> Vec<Beat> {
    let mut beats = Vec::new();
    for (id_str, list) in &heartbeat.heartbeat_list {
        let Ok(monitor_id) = id_str.parse::<i64>() else {
            continue;
        };
        for b in list {
            let Some(time) = parse_kuma_time(&b.time) else {
                continue;
            };
            beats.push(Beat {
                monitor_id,
                time,
                status: status_from_code(b.status),
                ping_ms: b.ping.map(|p| p.round() as u32),
            });
        }
    }
    beats
}

/// The 24h uptime ratio per monitor id, from `uptimeList` keys shaped `"<id>_24"`.
fn extract_uptime_24h(heartbeat: &HeartbeatDto) -> HashMap<i64, f64> {
    let mut map = HashMap::new();
    for (key, value) in &heartbeat.uptime_list {
        if let Some(prefix) = key.strip_suffix("_24")
            && let Ok(id) = prefix.parse::<i64>()
        {
            map.insert(id, *value);
        }
    }
    map
}

/// Everything one poll produces from the status-page endpoints.
pub struct PolledData {
    pub monitors: Vec<Monitor>,
    pub beats: Vec<Beat>,
    pub uptime_24h: HashMap<i64, f64>,
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

    /// Fetch both status-page endpoints and map them into domain data.
    pub async fn fetch(&self) -> Result<PolledData, AppError> {
        let config_url = format!("{}/api/status-page/{}", self.base_url, self.slug);
        let heartbeat_url = format!("{}/api/status-page/heartbeat/{}", self.base_url, self.slug);

        let resp = self
            .http
            .get(&config_url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;
        let config: StatusPageConfigDto =
            serde_json::from_slice(&bytes).map_err(|e| AppError::Parse(e.to_string()))?;

        let resp = self
            .http
            .get(&heartbeat_url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;
        let heartbeat: HeartbeatDto =
            serde_json::from_slice(&bytes).map_err(|e| AppError::Parse(e.to_string()))?;

        Ok(PolledData {
            monitors: map_monitors(&config, &heartbeat),
            beats: extract_beats(&heartbeat),
            uptime_24h: extract_uptime_24h(&heartbeat),
        })
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
        let heartbeat: HeartbeatDto = serde_json::from_str(include_str!(
            "../../tests/fixtures/status-page-heartbeat.json"
        ))
        .expect("heartbeat fixture parses");
        (config, heartbeat)
    }

    #[test]
    fn parses_kuma_utc_time() {
        let t = parse_kuma_time("2026-06-28 15:59:49.191").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-06-28T15:59:49.191+00:00");
    }

    #[test]
    fn rejects_bad_time() {
        assert!(parse_kuma_time("not a time").is_none());
    }

    #[test]
    fn extracts_beats_from_fixture() {
        let (_config, heartbeat) = load();
        let beats = extract_beats(&heartbeat);
        assert_eq!(beats.len(), 6); // ids 1(2) + 2(2) + 3(1) + 5(1)
        let m1: Vec<_> = beats.iter().filter(|b| b.monitor_id == 1).collect();
        assert_eq!(m1.len(), 2);
        assert!(
            m1.iter()
                .any(|b| b.status == MonitorStatus::Up && b.ping_ms == Some(7))
        );
    }

    #[test]
    fn extracts_uptime_24h_from_fixture() {
        let (_config, heartbeat) = load();
        let up = extract_uptime_24h(&heartbeat);
        assert_eq!(up.len(), 4);
        assert_eq!(up.get(&2).copied(), Some(0.98));
    }

    #[test]
    fn maps_status_latency_and_group() {
        let (config, heartbeat) = load();
        let monitors = map_monitors(&config, &heartbeat);
        let by_id = |id: i64| {
            monitors
                .iter()
                .find(|m| m.id == id)
                .expect("monitor present")
        };

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
