//! multimux configuration: routes + segmentation/window/bind parameters.
//!
//! CLI-first with an optional JSON config file. A route maps one RTSP input
//! URL to a served stream name.

use crate::error::{MultimuxError, Result};
use serde::Deserialize;
use std::path::Path;

/// One input→output route: an RTSP source URL served under `name`.
#[derive(Debug, Clone, Deserialize)]
pub struct Route {
    /// Served stream name (URL path segment).
    pub name: String,
    /// RTSP source URL to pull.
    pub rtsp_url: String,
}

/// multimux runtime configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `host:port` the HTTP origin binds.
    pub bind: String,
    /// Target full-segment duration (seconds).
    pub target_duration_secs: f64,
    /// LL-HLS part target (milliseconds).
    pub part_target_ms: u32,
    /// Rolling window depth (full segments retained in RAM).
    pub window_segments: usize,
    /// Input→output routes.
    pub routes: Vec<Route>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "0.0.0.0:8080".to_string(),
            target_duration_secs: 4.0,
            part_target_ms: 500,
            window_segments: 8,
            routes: Vec::new(),
        }
    }
}

impl Config {
    /// Load a JSON config file.
    pub fn from_json_file(path: &Path) -> Result<Config> {
        let bytes = std::fs::read(path)?;
        let cfg: Config = serde_json::from_slice(&bytes)
            .map_err(|e| MultimuxError::Config(format!("{path:?}: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject empty route sets, duplicate stream names, and nonsensical timing.
    pub fn validate(&self) -> Result<()> {
        if self.routes.is_empty() {
            return Err(MultimuxError::Config("no routes configured".into()));
        }
        if self.target_duration_secs <= 0.0 || self.part_target_ms == 0 || self.window_segments == 0
        {
            return Err(MultimuxError::Config(
                "timing/window must be positive".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.routes {
            if !seen.insert(r.name.as_str()) {
                return Err(MultimuxError::Config(format!(
                    "duplicate stream name {:?}",
                    r.name
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_config_with_routes() {
        let json = r#"{
            "bind": "127.0.0.1:9000",
            "target_duration_secs": 2.0,
            "part_target_ms": 250,
            "window_segments": 6,
            "routes": [
                { "name": "cam1", "rtsp_url": "rtsp://host/stream1" },
                { "name": "cam2", "rtsp_url": "rtsp://host/stream2" }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.bind, "127.0.0.1:9000");
        assert_eq!(cfg.part_target_ms, 250);
        assert_eq!(cfg.routes.len(), 2);
        assert_eq!(cfg.routes[1].name, "cam2");
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_duplicate_stream_names() {
        let cfg = Config {
            routes: vec![
                Route {
                    name: "x".into(),
                    rtsp_url: "rtsp://a".into(),
                },
                Route {
                    name: "x".into(),
                    rtsp_url: "rtsp://b".into(),
                },
            ],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_no_routes() {
        assert!(Config::default().validate().is_err());
    }
}
