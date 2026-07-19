//! multimux configuration: routes + segmentation/window/bind parameters.
//!
//! CLI-first with an optional JSON config file. A route maps one input
//! ([`InputSpec`] — RTSP pull, raw RTP/UDP, MPEG-TS/UDP, MPEG-TS/HTTP, or
//! HLS-pull) to a served stream name.

use crate::error::{MultimuxError, Result};
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

/// One route's ingest transport (issue #663 P3a/P3c): tagged so a JSON
/// config can name which transport a route uses (`"type": "rtsp" | "rtp" |
/// "ts_udp" | "ts_http" | "hls_pull"`).
///
/// - [`InputSpec::Rtsp`] pulls a live RTSP source (DESCRIBE/SETUP/PLAY,
///   interleaved TCP) — see [`crate::source::rtsp`].
/// - [`InputSpec::Rtp`] receives raw RTP over UDP (uni/multicast), depayloaded
///   using an out-of-band SDP (inline text, or `@path` to a file) that
///   supplies the codec/fmtp a DESCRIBE would otherwise provide — see
///   [`crate::source::rtp_udp`].
/// - [`InputSpec::TsUdp`] receives an MPEG-2 Transport Stream over UDP
///   (uni/multicast); the track set comes from the stream's own in-band PMT,
///   so no SDP is needed — see [`crate::source::ts_udp`].
/// - [`InputSpec::TsHttp`] receives an MPEG-2 Transport Stream over a
///   streaming HTTP GET (chunked/progressive) — see
///   [`crate::source::ts_http`].
/// - [`InputSpec::HlsPull`] pulls a remote (LL-)HLS Media Playlist — see
///   [`crate::source::hls_pull`].
///
/// [`InputSpec::TsHttp`]/[`InputSpec::HlsPull`] both may carry `user:pass@`
/// URL userinfo (Basic/Digest — see [`crate::source::http_auth`]), redacted
/// the same way [`InputSpec::Rtsp`]'s URL is.
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputSpec {
    /// Pull a live RTSP source.
    Rtsp {
        /// RTSP source URL to pull. May carry `user:pass@` userinfo — see
        /// [`InputSpec`]'s `Debug` impl, which redacts it.
        url: String,
    },
    /// Receive raw RTP over UDP (uni/multicast), depayloaded per an
    /// out-of-band SDP.
    Rtp {
        /// `host:port` to bind the UDP socket to.
        addr: String,
        /// The SDP describing the stream's codec/fmtp: either inline SDP
        /// text, or `@path` to a file containing one (read fresh on every
        /// connect/reconnect).
        sdp: String,
        /// Multicast group to join, if the stream is multicast rather than
        /// unicast (must be a multicast IP address of `addr`'s family).
        #[serde(default)]
        multicast_group: Option<String>,
    },
    /// Receive an MPEG-2 Transport Stream over UDP (uni/multicast).
    TsUdp {
        /// `host:port` to bind the UDP socket to.
        addr: String,
        /// Multicast group to join, if the stream is multicast rather than
        /// unicast (must be a multicast IP address of `addr`'s family).
        #[serde(default)]
        multicast_group: Option<String>,
    },
    /// Receive an MPEG-2 Transport Stream over a streaming HTTP GET
    /// (chunked/progressive).
    TsHttp {
        /// `http://` or `https://` URL to GET. May carry `user:pass@`
        /// userinfo — see [`InputSpec`]'s `Debug` impl, which redacts it.
        url: String,
    },
    /// Pull a remote (LL-)HLS Media Playlist.
    HlsPull {
        /// `http://` or `https://` Media Playlist URL to pull. May carry
        /// `user:pass@` userinfo — see [`InputSpec`]'s `Debug` impl, which
        /// redacts it.
        url: String,
    },
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): [`InputSpec::Rtsp`]'s
/// `url` may carry a live camera's `user:pass@` userinfo, so it must never
/// render verbatim; the UDP variants carry no secret but get a tidy summary
/// (the SDP body's length rather than its full text, which can be sizeable).
impl std::fmt::Debug for InputSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputSpec::Rtsp { url } => f
                .debug_struct("Rtsp")
                .field("url", &crate::redact::redact_url(url))
                .finish(),
            InputSpec::Rtp {
                addr,
                sdp,
                multicast_group,
            } => f
                .debug_struct("Rtp")
                .field("addr", addr)
                .field("sdp_len", &sdp.len())
                .field("multicast_group", multicast_group)
                .finish(),
            InputSpec::TsUdp {
                addr,
                multicast_group,
            } => f
                .debug_struct("TsUdp")
                .field("addr", addr)
                .field("multicast_group", multicast_group)
                .finish(),
            InputSpec::TsHttp { url } => f
                .debug_struct("TsHttp")
                .field("url", &crate::redact::redact_url(url))
                .finish(),
            InputSpec::HlsPull { url } => f
                .debug_struct("HlsPull")
                .field("url", &crate::redact::redact_url(url))
                .finish(),
        }
    }
}

impl InputSpec {
    /// Validates this input's fields in isolation (no I/O — reachability is
    /// checked at connect time, not here): an RTSP URL must parse with an
    /// `rtsp`/`rtsps` scheme; a UDP `addr` must parse as a socket address; a
    /// `multicast_group`, if present, must be a multicast IP; an [`Rtp`]
    /// input's `sdp` must be non-empty, and — unless it's an `@path`
    /// reference (existence checked at connect time) — parseable SDP.
    ///
    /// [`Rtp`]: InputSpec::Rtp
    fn validate(&self) -> Result<()> {
        match self {
            InputSpec::Rtsp { url } => validate_rtsp_url(url),
            InputSpec::Rtp {
                addr,
                sdp,
                multicast_group,
            } => {
                validate_udp_addr(addr)?;
                validate_sdp(sdp)?;
                if let Some(group) = multicast_group {
                    validate_multicast_group(group)?;
                }
                Ok(())
            }
            InputSpec::TsUdp {
                addr,
                multicast_group,
            } => {
                validate_udp_addr(addr)?;
                if let Some(group) = multicast_group {
                    validate_multicast_group(group)?;
                }
                Ok(())
            }
            InputSpec::TsHttp { url } => validate_http_url(url),
            InputSpec::HlsPull { url } => validate_http_url(url),
        }
    }
}

/// An RTSP URL must parse and use the `rtsp`/`rtsps` scheme (RFC 2326 §1 /
/// IANA).
fn validate_rtsp_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|e| MultimuxError::ConfigInvalid {
        field: "routes.input.url",
        reason: format!("bad rtsp(s) URL {url:?}: {e}"),
    })?;
    match parsed.scheme() {
        "rtsp" | "rtsps" => Ok(()),
        other => Err(MultimuxError::ConfigInvalid {
            field: "routes.input.url",
            reason: format!("scheme must be rtsp or rtsps, got {other:?}"),
        }),
    }
}

/// A TS-over-HTTP/HLS-pull URL must parse and use the `http`/`https` scheme.
fn validate_http_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|e| MultimuxError::ConfigInvalid {
        field: "routes.input.url",
        reason: format!("bad http(s) URL {url:?}: {e}"),
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => Err(MultimuxError::ConfigInvalid {
            field: "routes.input.url",
            reason: format!("scheme must be http or https, got {other:?}"),
        }),
    }
}

/// A UDP bind address must parse as `host:port`.
fn validate_udp_addr(addr: &str) -> Result<()> {
    addr.parse::<SocketAddr>()
        .map(|_| ())
        .map_err(|e| MultimuxError::ConfigInvalid {
            field: "routes.input.addr",
            reason: format!("bad UDP address {addr:?}: {e}"),
        })
}

/// A multicast group must parse as an IP address and actually be multicast
/// (RFC 1112 §4 IPv4 224.0.0.0/4; RFC 4291 §2.7 IPv6 `ff00::/8`) — a unicast
/// address here would silently fail (or worse, do nothing useful) at the
/// OS-level `IP_ADD_MEMBERSHIP`/`IPV6_JOIN_GROUP` join, so it's rejected at
/// config time instead.
fn validate_multicast_group(group: &str) -> Result<()> {
    let ip: IpAddr = group.parse().map_err(|e| MultimuxError::ConfigInvalid {
        field: "routes.input.multicast_group",
        reason: format!("bad multicast group {group:?}: {e}"),
    })?;
    if !ip.is_multicast() {
        return Err(MultimuxError::ConfigInvalid {
            field: "routes.input.multicast_group",
            reason: format!("{group} is not a multicast address"),
        });
    }
    Ok(())
}

/// An [`InputSpec::Rtp`] SDP must be non-empty; an inline body (not an
/// `@path` file reference) must also parse as SDP (RFC 4566) — this is the
/// full codec/fmtp source for that route, so a config with unparseable SDP
/// would never usefully connect. A `@path` reference is only checked for
/// non-emptiness of the path itself: the file may not exist yet at
/// config-validation time (mirrors how an RTSP URL's reachability is never
/// checked here either), and is read + parsed fresh at connect time by
/// [`crate::source::sdp::load_sdp`]/`parse_sdp_tracks`.
fn validate_sdp(sdp: &str) -> Result<()> {
    if sdp.is_empty() {
        return Err(MultimuxError::ConfigInvalid {
            field: "routes.input.sdp",
            reason: "must not be empty".into(),
        });
    }
    let Some(path) = sdp.strip_prefix('@') else {
        return sdp_types::Session::parse(sdp.as_bytes())
            .map(|_| ())
            .map_err(|e| MultimuxError::ConfigInvalid {
                field: "routes.input.sdp",
                reason: format!("unparsable inline SDP: {e}"),
            });
    };
    if path.is_empty() {
        return Err(MultimuxError::ConfigInvalid {
            field: "routes.input.sdp",
            reason: "@ file reference must name a path".into(),
        });
    }
    Ok(())
}

/// One input→output route: an [`InputSpec`] served under `name`.
#[derive(Clone, Deserialize)]
pub struct Route {
    /// Served stream name (URL path segment).
    pub name: String,
    /// The ingest transport this route pulls from.
    pub input: InputSpec,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): [`InputSpec`] already
/// redacts what needs redacting; this just forwards to it so `Route` values
/// embedded in `Config`'s (derived) `Debug` and ad-hoc `{:?}` logging never
/// leak a credential either.
impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("name", &self.name)
            .field("input", &self.input)
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

    /// Reject empty route sets, duplicate stream names, nonsensical timing,
    /// and any route whose [`InputSpec`] fails its own field validation.
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
            r.input.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_config_with_rtsp_routes() {
        let json = r#"{
            "bind": "127.0.0.1:9000",
            "target_duration_secs": 2.0,
            "part_target_ms": 250,
            "window_segments": 6,
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } },
                { "name": "cam2", "input": { "type": "rtsp", "url": "rtsp://host/stream2" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.bind, "127.0.0.1:9000");
        assert_eq!(cfg.part_target_ms, 250);
        assert_eq!(cfg.routes.len(), 2);
        assert_eq!(cfg.routes[1].name, "cam2");
        match &cfg.routes[1].input {
            InputSpec::Rtsp { url } => assert_eq!(url, "rtsp://host/stream2"),
            other => panic!("expected InputSpec::Rtsp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_rtp_input() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
                   m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n\
                   a=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\n";
        let json = serde_json::json!({
            "bind": "127.0.0.1:9000",
            "target_duration_secs": 2.0,
            "part_target_ms": 250,
            "window_segments": 6,
            "routes": [
                {
                    "name": "cam-rtp",
                    "input": {
                        "type": "rtp",
                        "addr": "0.0.0.0:5004",
                        "sdp": sdp,
                        "multicast_group": "239.1.1.1"
                    }
                }
            ]
        });
        let cfg: Config = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::Rtp {
                addr,
                sdp: parsed_sdp,
                multicast_group,
            } => {
                assert_eq!(addr, "0.0.0.0:5004");
                assert_eq!(parsed_sdp, sdp);
                assert_eq!(multicast_group.as_deref(), Some("239.1.1.1"));
            }
            other => panic!("expected InputSpec::Rtp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_ts_udp_input() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts",
                    "input": { "type": "ts_udp", "addr": "0.0.0.0:5005" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::TsUdp {
                addr,
                multicast_group,
            } => {
                assert_eq!(addr, "0.0.0.0:5005");
                assert_eq!(*multicast_group, None);
            }
            other => panic!("expected InputSpec::TsUdp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_ts_udp_multicast_group() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts-mc",
                    "input": {
                        "type": "ts_udp",
                        "addr": "0.0.0.0:5006",
                        "multicast_group": "239.2.2.2"
                    }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::TsUdp {
                multicast_group, ..
            } => assert_eq!(multicast_group.as_deref(), Some("239.2.2.2")),
            other => panic!("expected InputSpec::TsUdp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_ts_http_input() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts-http",
                    "input": { "type": "ts_http", "url": "http://host/stream.ts" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::TsHttp { url } => assert_eq!(url, "http://host/stream.ts"),
            other => panic!("expected InputSpec::TsHttp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_hls_pull_input() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-hls-pull",
                    "input": { "type": "hls_pull", "url": "https://origin/live/media.m3u8" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::HlsPull { url } => assert_eq!(url, "https://origin/live/media.m3u8"),
            other => panic!("expected InputSpec::HlsPull, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_bad_ts_http_scheme() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsHttp {
                    url: "rtsp://host/stream.ts".into(),
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_hls_pull_scheme() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::HlsPull {
                    url: "ftp://host/media.m3u8".into(),
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unparsable_ts_http_url() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsHttp {
                    url: "not a url".into(),
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    /// Biting test: an `InputSpec::TsHttp`/`HlsPull`'s credential must never
    /// appear in `Debug` output, mirroring `route_debug_redacts_rtsp_credentials`.
    #[test]
    fn route_debug_redacts_ts_http_and_hls_pull_credentials() {
        let ts_http = Route {
            name: "cam-ts-http".into(),
            input: InputSpec::TsHttp {
                url: "http://user:secretpass@host/stream.ts".into(),
            },
        };
        let debug = format!("{ts_http:?}");
        assert!(!debug.contains("user"), "debug leaked username: {debug}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@host"), "debug: {debug}");

        let hls_pull = Route {
            name: "cam-hls-pull".into(),
            input: InputSpec::HlsPull {
                url: "https://user:secretpass@origin/media.m3u8".into(),
            },
        };
        let debug = format!("{hls_pull:?}");
        assert!(!debug.contains("user"), "debug leaked username: {debug}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@origin"), "debug: {debug}");
    }

    #[test]
    fn validate_rejects_bad_rtsp_scheme() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtsp {
                    url: "http://host/stream".into(),
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unparsable_udp_addr() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsUdp {
                    addr: "not-an-addr".into(),
                    multicast_group: None,
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_non_multicast_group() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsUdp {
                    addr: "0.0.0.0:5005".into(),
                    // A unicast address, not a valid multicast group.
                    multicast_group: Some("10.0.0.1".into()),
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_rtp_sdp() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtp {
                    addr: "0.0.0.0:5004".into(),
                    sdp: String::new(),
                    multicast_group: None,
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unparsable_inline_rtp_sdp() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtp {
                    addr: "0.0.0.0:5004".into(),
                    sdp: "not an sdp body".into(),
                    multicast_group: None,
                },
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_at_path_rtp_sdp_reference_without_reading_it() {
        // The referenced file need not exist yet at validate() time — only
        // connect() reads/parses it (via `crate::source::sdp::load_sdp`).
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtp {
                    addr: "0.0.0.0:5004".into(),
                    sdp: "@/no/such/file/does-not-exist.sdp".into(),
                    multicast_group: None,
                },
            }],
            ..Config::default()
        };
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_duplicate_stream_names() {
        let cfg = Config {
            routes: vec![
                Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://a".into(),
                    },
                },
                Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://b".into(),
                    },
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
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let result: std::result::Result<Config, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "unknown key must be rejected, not silently ignored"
        );
    }

    #[test]
    fn rejects_unknown_input_type() {
        // A typo'd/unsupported `type` discriminator (e.g. "rtmp") must be
        // rejected by serde's internally-tagged enum, not silently coerced
        // into one of the known variants.
        let json = r#"{
            "routes": [
                { "name": "cam1", "input": { "type": "rtmp", "url": "rtmp://host/stream1" } }
            ]
        }"#;
        let result: std::result::Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_err(), "unknown input type must be rejected");
    }

    /// Biting test: an `InputSpec::Rtsp`'s credential must never appear in
    /// its (and therefore `Route`'s) `Debug` output. Fails immediately if
    /// the manual `Debug` impl is reverted to `#[derive(Debug)]` (which
    /// would render `url` verbatim, userinfo included).
    #[test]
    fn route_debug_redacts_rtsp_credentials() {
        let route = Route {
            name: "cam1".into(),
            input: InputSpec::Rtsp {
                url: "rtsp://user:secretpass@host/s".into(),
            },
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
                input: InputSpec::Rtsp {
                    url: "rtsp://user:secretpass@host/s".into(),
                },
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

    /// A raw-RTP route's `Debug` must not dump the full SDP body verbatim
    /// (just its length) — keeps a route's `Debug`/log line short even when
    /// the SDP is large, mirroring the RTSP variant's "no giant blobs in
    /// logs" spirit even though the SDP itself carries no secret.
    #[test]
    fn route_debug_shows_sdp_length_not_full_body() {
        let long_sdp = "v=0\r\n".repeat(50);
        let route = Route {
            name: "cam-rtp".into(),
            input: InputSpec::Rtp {
                addr: "0.0.0.0:5004".into(),
                sdp: long_sdp.clone(),
                multicast_group: None,
            },
        };
        let debug = format!("{route:?}");
        assert!(!debug.contains(&long_sdp), "debug: {debug}");
        assert!(
            debug.contains(&long_sdp.len().to_string()),
            "debug: {debug}"
        );
    }
}
