//! multimux configuration: routes + segmentation/window/bind parameters.
//!
//! CLI-first with an optional JSON config file. A route maps one RTSP input
//! URL to a served stream name.

use crate::error::{MultimuxError, Result};
use serde::Deserialize;
use std::path::Path;

/// One input→output route: an RTSP source URL served under `name`.
#[derive(Clone, Deserialize)]
pub struct Route {
    /// Served stream name (URL path segment).
    pub name: String,
    /// RTSP source URL to pull. May carry `user:pass@` userinfo — see
    /// [`Route`]'s `Debug` impl, which redacts it.
    pub rtsp_url: String,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `rtsp_url` may carry a
/// live camera's `user:pass@` userinfo, and `Route` values end up in
/// `Config`'s (derived) `Debug` and in ad-hoc `{:?}` logging — so the
/// credential must never appear verbatim.
impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("name", &self.name)
            .field("rtsp_url", &crate::redact::redact_url(&self.rtsp_url))
            .finish()
    }
}

/// multimux runtime configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
        let bytes = std::fs::read(path).map_err(|source| MultimuxError::ConfigRead {
            path: path.to_path_buf(),
            source,
        })?;
        let cfg: Config =
            serde_json::from_slice(&bytes).map_err(|e| MultimuxError::ConfigParse {
                path: path.to_path_buf(),
                reason: e.to_string(),
            })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject empty route sets, duplicate stream names, and nonsensical timing.
    pub fn validate(&self) -> Result<()> {
        if self.routes.is_empty() {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes",
                reason: "no routes configured".into(),
            });
        }
        if self.target_duration_secs <= 0.0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "target_duration_secs",
                reason: "must be positive".into(),
            });
        }
        if self.part_target_ms == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "part_target_ms",
                reason: "must be positive".into(),
            });
        }
        if self.window_segments == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "window_segments",
                reason: "must be positive".into(),
            });
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.routes {
            if !seen.insert(r.name.as_str()) {
                return Err(MultimuxError::ConfigInvalid {
                    field: "routes",
                    reason: format!("duplicate stream name {:?}", r.name),
                });
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

    #[test]
    fn rejects_unknown_config_key() {
        // A typo'd key (e.g. "window_segment" instead of "window_segments")
        // must error rather than silently fall back to the default —
        // `#[serde(deny_unknown_fields)]` on `Config` enforces this.
        let json = r#"{
            "bind": "127.0.0.1:9000",
            "window_segment": 6,
            "routes": [
                { "name": "cam1", "rtsp_url": "rtsp://host/stream1" }
            ]
        }"#;
        let result: std::result::Result<Config, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "unknown key must be rejected, not silently ignored"
        );
    }

    /// Biting test: a `Route`'s credential must never appear in its `Debug`
    /// output. This fails immediately if `Route`'s manual `Debug` impl is
    /// reverted to `#[derive(Debug)]` (which would render `rtsp_url`
    /// verbatim, userinfo included).
    #[test]
    fn route_debug_redacts_credentials() {
        let route = Route {
            name: "cam1".into(),
            rtsp_url: "rtsp://user:secretpass@host/s".into(),
        };
        let debug = format!("{route:?}");
        assert!(!debug.contains("user"), "debug leaked username: {debug}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@host"), "debug: {debug}");
    }

    /// Same biting property, but through `Config`'s *derived* `Debug` — this
    /// proves the redaction is wired end-to-end (a route embedded in a
    /// config, as it always is at runtime) and not just on a bare `Route`.
    #[test]
    fn config_debug_redacts_route_credentials() {
        let cfg = Config {
            routes: vec![Route {
                name: "cam1".into(),
                rtsp_url: "rtsp://user:secretpass@host/s".into(),
            }],
            ..Config::default()
        };
        let debug = format!("{cfg:?}");
        assert!(!debug.contains("user"), "config debug leaked username");
        assert!(
            !debug.contains("secretpass"),
            "config debug leaked password"
        );
        assert!(debug.contains("***@host"));
    }
}
