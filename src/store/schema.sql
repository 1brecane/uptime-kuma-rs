CREATE TABLE IF NOT EXISTS heartbeats (
    monitor_id INTEGER NOT NULL,
    time       TEXT    NOT NULL,   -- fixed-width UTC "%Y-%m-%dT%H:%M:%S%.3fZ"
    status     INTEGER NOT NULL,   -- 0=down, 1=up, 2=pending, 3=maintenance
    ping_ms    INTEGER,
    PRIMARY KEY (monitor_id, time)
);
CREATE INDEX IF NOT EXISTS idx_heartbeats_monitor_time ON heartbeats (monitor_id, time);
