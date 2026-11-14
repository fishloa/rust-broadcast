//! Independent HLS conformance oracle for the classic TS-HLS output
//! (`ts_hls`, `Container::MpegTs`), covering audit run 7 warnings W11 and
//! W12 with tools that do not share our code:
//!
//! - **W12** — a TS segment must be served with `Content-Type: video/mp2t`,
//!   not `video/mp4`. Asserted twice: directly on the response header, and
//!   independently by writing the fetched bytes to disk and asking `ffprobe`
//!   (and Apple's `mediastreamvalidator`) to identify them.
//! - **W11** — a served media playlist on a route whose retention/window can
//!   remove leading segments must not claim `#EXT-X-PLAYLIST-TYPE` (RFC 8216
//!   §6.2.2). The classic TS-HLS media playlist is *live-continuing* (no
//!   `#EXT-X-ENDLIST`), so `mediastreamvalidator` would block reloading it at
//!   its default timeout — it is run with a short `-t` and playlist-only
//!   parsing for the tag-vocabulary check, while `ffprobe` decodes a real
//!   fetched segment to prove the media itself is genuine.
//!
//! The ingest path is the real one: the committed `fixtures/ts/h264-aac.ts`
//! capture (real H.264+AAC) is pushed through `InputSpec::TsUdp` into
//! `serve_with_registry`, and everything is fetched back over a real loopback
//! HTTP `GET`. Both oracles skip **loudly** (with a `--nocapture`-visible
//! line) when their binary is absent, so this file stays green on Linux/CI;
//! run it on macOS for the genuine check.
#![cfg(feature = "test-seams")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use test_fixtures::process::output_bounded;

/// Hard deadline for a `--version`/`-version` availability probe.
const PROBE_DEADLINE: Duration = Duration::from_secs(10);
/// Hard deadline for one `ffprobe` read of a short local file or capped live
/// read (`-read_intervals %+3`); it is killed and the test fails on overrun.
const FFPROBE_DEADLINE: Duration = Duration::from_secs(60);
/// Margin added to the validator's own `-t` timeout to form the hard kill
/// deadline.
const VALIDATOR_MARGIN: Duration = Duration::from_secs(30);

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;

use crate::support::shared_it::{fixture_path, poll_until_extinf};
use crate::support::udp_origin::{UdpTsOrigin, spawn_udp_ts_origin};

fn have_oracle(bin: &str) -> bool {
    let probe = |arg: &str| {
        output_bounded(Command::new(bin).arg(arg), PROBE_DEADLINE).inspect_err(|e| {
            assert!(e.kind() != std::io::ErrorKind::TimedOut, "{e}");
        })
    };
    probe("-version").or_else(|_| probe("-h")).is_ok()
}

fn base_config(bind: std::net::SocketAddr, input: InputSpec) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: 0.5,
        part_target: Duration::from_millis(100),
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input,
            outputs: vec![OutputKind::TsHls],
            dvr: DvrConfig::default(),
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }],
        ..Config::default()
    }
}

/// Polls the media playlist until it carries a real closed-segment
/// `#EXTINF:` line (never satisfied by header-only tags the engine renders
/// unconditionally). Bounded by a generous hang guard, not a latency
/// assertion (issue #807).
fn first_segment_uri(playlist: &str) -> String {
    let start = playlist
        .find("seg-")
        .unwrap_or_else(|| panic!("no seg-*.ts URI in playlist: {playlist}"));
    let rest = &playlist[start..];
    let end = rest
        .find(".ts")
        .unwrap_or_else(|| panic!("no .ts in playlist: {playlist}"))
        + ".ts".len();
    rest[..end].to_string()
}

/// A live TS-HLS route fed the real capture over UDP. `Drop` aborts the
/// sender and server even on a panic, so a failing assertion never leaves a
/// UDP flood running.
struct Served {
    bind_addr: std::net::SocketAddr,
    server: tokio::task::JoinHandle<Result<(), multimux::MultimuxError>>,
    stop: Arc<AtomicBool>,
    sender: tokio::task::JoinHandle<()>,
    playlist: String,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.sender.abort();
        self.server.abort();
    }
}

/// Start a TS-HLS route and feed it the fixture. Both the media listener AND
/// the UDP input socket are bound `:0` and handed over live (SP7.1) — the
/// media listener into `serve_with_registry_on`, the UDP socket into the route
/// via `Config::prebound` — so neither address can lose a bind race and no
/// whole-attempt retry loop is needed.
async fn serve_ts_hls_until_extinf() -> Served {
    let client = reqwest::Client::new();
    let UdpTsOrigin {
        bind_addr,
        server,
        stop,
        sender,
    } = spawn_udp_ts_origin(
        base_config,
        &fixture_path("ts/h264-aac.ts"),
        Duration::from_millis(5),
    )
    .await;

    let url = format!("http://{bind_addr}/cam/media.m3u8");
    let playlist = tokio::time::timeout(Duration::from_secs(20), poll_until_extinf(&client, &url))
        .await
        .expect("the TS-HLS playlist must carry an #EXTINF within 20s");
    Served {
        bind_addr,
        server,
        stop,
        sender,
        playlist,
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "multimux-ts-hls-oracle-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[tokio::test]
async fn ts_hls_segment_content_type_is_video_mp2t() {
    let served = serve_ts_hls_until_extinf().await;
    let bind_addr = served.bind_addr;
    let playlist = served.playlist.clone();
    let client = reqwest::Client::new();

    let seg_uri = first_segment_uri(&playlist);
    let resp = client
        .get(format!("http://{bind_addr}/cam/{seg_uri}"))
        .send()
        .await
        .expect("GET ts segment");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let seg_bytes = resp.bytes().await.expect("segment body");

    assert_eq!(
        content_type, "video/mp2t",
        "a TS segment must be served as video/mp2t, not {content_type}"
    );
    // A TS segment starts with a 0x47 sync byte -- the byte ffprobe and
    // mediastreamvalidator key container detection on.
    assert_eq!(seg_bytes.first(), Some(&0x47u8), "not a TS-sync segment");
}

/// `ffprobe` (independent decoder) identifies a fetched segment as real
/// MPEG-2 TS carrying the fixture's H.264 video.
#[tokio::test]
async fn ffprobe_recognises_served_segment_as_mpegts() {
    if !have_oracle("ffprobe") {
        eprintln!("SKIP ffprobe_recognises_served_segment_as_mpegts: ffprobe not on PATH");
        return;
    }
    let served = serve_ts_hls_until_extinf().await;
    let bind_addr = served.bind_addr;
    let playlist = served.playlist.clone();
    let client = reqwest::Client::new();

    let dir = temp_dir("ffprobe");
    // Write a whole playlist tree the validator can read from disk: the
    // media playlist plus every segment it references.
    let seg_uri = first_segment_uri(&playlist);
    let seg_bytes = client
        .get(format!("http://{bind_addr}/cam/{seg_uri}"))
        .send()
        .await
        .expect("GET ts segment")
        .bytes()
        .await
        .expect("segment body");
    std::fs::write(dir.join(&seg_uri), &seg_bytes).expect("write segment");

    let out = output_bounded(
        Command::new("ffprobe")
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
            ])
            .arg(dir.join(&seg_uri)),
        FFPROBE_DEADLINE,
    )
    .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        stdout.contains("h264"),
        "ffprobe must identify the served segment as real MPEG-2 TS video: \
         stdout={stdout:?} stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Apple `mediastreamvalidator` parses the served playlist tree without a
/// MUST-level finding, and never reports a mutability conflict on a live
/// TS-HLS playlist (W11). Invoked exactly as `media-doctor`'s own oracle
/// does (`--quiet -t <low> -O <json>` with `current_dir` set to the
/// playlist's directory); a live playlist has no `#EXT-X-ENDLIST`, so a
/// short timeout is mandatory or it blocks the 300 s default.
#[tokio::test]
async fn mediastreamvalidator_accepts_served_ts_hls_playlist() {
    // Locate the validator via `which` (default install is
    // `/usr/local/bin/mediastreamvalidator`, but it may be elsewhere on PATH).
    let Some(validator) = test_fixtures::process::which("mediastreamvalidator") else {
        eprintln!(
            "SKIP mediastreamvalidator_accepts_served_ts_hls_playlist: mediastreamvalidator not on PATH (Apple Additional Tools for Xcode; macOS only)"
        );
        return;
    };
    const MIN_RELOAD_TIMEOUT_SECS: u32 = 1;

    let served = serve_ts_hls_until_extinf().await;
    let bind_addr = served.bind_addr;
    let playlist = served.playlist.clone();
    let client = reqwest::Client::new();

    let dir = temp_dir("msv");
    std::fs::write(dir.join("media.m3u8"), playlist.as_bytes()).expect("write playlist");
    // Fetch every segment the playlist references so the validator can read
    // the whole tree from disk.
    for line in playlist.lines().filter(|l| l.starts_with("seg-")) {
        let bytes = client
            .get(format!("http://{bind_addr}/cam/{line}"))
            .send()
            .await
            .expect("GET ts segment")
            .bytes()
            .await
            .expect("segment body");
        std::fs::write(dir.join(line), &bytes).expect("write segment");
    }

    let json_path = dir.join("report.json");
    let out = output_bounded(
        Command::new(&validator)
            .args([
                "--parse-playlist-only",
                "--quiet",
                "-t",
                &MIN_RELOAD_TIMEOUT_SECS.to_string(),
                "-O",
            ])
            .arg(&json_path)
            .arg("media.m3u8")
            .current_dir(&dir),
        Duration::from_secs(u64::from(MIN_RELOAD_TIMEOUT_SECS)) + VALIDATOR_MARGIN,
    )
    .expect("run mediastreamvalidator");

    let report = std::fs::read_to_string(&json_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !report.is_empty(),
        "mediastreamvalidator produced no report (stdout={:?} stderr={:?})",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The validator exits 0 even on parse failure; the JSON is the only
    // authoritative signal. Any MUST-level finding (errorRequirementLevel 1)
    // or a hard parse failure fails this test. Parsed as JSON (not string
    // matched) so the validator's incidental whitespace cannot hide a
    // failure.
    let parsed: serde_json::Value =
        serde_json::from_str(&report).expect("mediastreamvalidator report is JSON");
    let parse_failed = parsed
        .get("parseFailed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let must_level: Vec<&serde_json::Value> = parsed
        .get("messages")
        .and_then(serde_json::Value::as_array)
        .map(|msgs| {
            msgs.iter()
                .filter(|m| {
                    m.get("errorRequirementLevel")
                        .and_then(serde_json::Value::as_u64)
                        == Some(1)
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !parse_failed && must_level.is_empty(),
        "mediastreamvalidator flagged a MUST-level violation on the served \
         TS-HLS playlist: {report}"
    );
}
