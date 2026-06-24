# uptime-kuma-rs

A lightweight Rust scraper for Uptime Kuma that exposes a clean REST API — self-host your monitoring data without the socket.io mess.

uptime-kuma-rs is a standalone Rust service that polls your Uptime Kuma instance and exposes structured monitoring data via a simple REST API. No database required, no config bloat — just point it at your Uptime Kuma instance and start consuming /monitors, /uptime, and /incidents from any service or dashboard.
