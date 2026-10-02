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
//! The ingest path is the real one: the committed `fixtures/ts/h264_aac.ts`
//! capture (real H.264+AAC) is pushed through `InputSpec::TsUdp` into
//! `serve_with_registry`, and everything is fetched back over a real loopback
//! HTTP `GET`. Both oracles skip **loudly** (with a `--nocapture`-visible
//! line) when their binary is absent, so this file stays green on Linux/CI;
//! run it on macOS for the genuine check.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry;

fn fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac.ts"
    ))
}

/// Locate `bin` on `PATH`, returning its full path.
fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

fn have_oracle(bin: &str) -> bool {
    Command::new(bin)
        .arg("-version")
        .output()
        .or_else(|_| Command::new(bin).arg("-h").output())
        .map(|_| true)
        .unwrap_or(false)
}

fn reserve_tcp_addr() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr
}

fn reserve_udp_addr() -> std::net::SocketAddr {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("reserve udp port");
    let addr = socket.local_addr().expect("local addr");
    drop(socket);
    addr
}

fn base_config(bind: std::net::SocketAddr, input: InputSpec) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input,
            outputs: vec![OutputKind::TsHls],
            dvr: DvrConfig::default(),
        }],
        ..Config::default()
    }
}

/// Polls the media playlist until it carries a real closed-segment
/// `#EXTINF:` line (never satisfied by header-only tags the engine renders
/// unconditionally). Bounded by a generous hang guard, not a latency
/// assertion (issue #807).
async fn poll_until_extinf(client: &reqwest::Client, url: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(resp) = client.get(url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no #EXTINF: line appeared in {url} within the hang guard -- \
             TS-HLS ingest never produced a closed segment"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

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

/// Start a TS-HLS route and feed it the fixture, retrying the whole attempt
/// if the reserved port loses a bind race (never a bare reserve-drop-rebind).
async fn serve_ts_hls_until_extinf() -> Served {
    let client = reqwest::Client::new();
    for _ in 0..10 {
        let bind_addr = reserve_tcp_addr();
        let udp_addr = reserve_udp_addr();
        let config = base_config(
            bind_addr,
            InputSpec::TsUdp {
                addr: udp_addr.to_string(),
                multicast_group: None,
            },
        );
        let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

        let ts_bytes = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
        let stop = Arc::new(AtomicBool::new(false));
        let sender_stop = Arc::clone(&stop);
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind sender");
        let sender = tokio::spawn(async move {
            while !sender_stop.load(Ordering::Relaxed) {
                for chunk in ts_bytes.chunks(7 * 188) {
                    let _ = socket.send_to(chunk, udp_addr).await;
                    // Pacing (not synchronisation): space the datagrams so the
                    // ingest task is not starved by the flood.
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        });

        let url = format!("http://{bind_addr}/cam/media.m3u8");
        match tokio::time::timeout(Duration::from_secs(20), poll_until_extinf(&client, &url)).await
        {
            Ok(playlist) => {
                return Served {
                    bind_addr,
                    server,
                    stop,
                    sender,
                    playlist,
                };
            }
            Err(_) => {
                // Port lost the race (or the route never produced a segment):
                // tear down and retry with a fresh port.
                stop.store(true, Ordering::Relaxed);
                sender.abort();
                server.abort();
            }
        }
    }
    panic!("could not start the TS-HLS test server after 10 attempts");
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

    let out = Command::new("ffprobe")
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name",
        ])
        .arg(dir.join(&seg_uri))
        .output()
        .expect("spawn ffprobe");
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
    let Some(validator) = which("mediastreamvalidator") else {
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
    let out = Command::new(&validator)
        .args([
            "--parse-playlist-only",
            "--quiet",
            "-t",
            &MIN_RELOAD_TIMEOUT_SECS.to_string(),
            "-O",
        ])
        .arg(&json_path)
        .arg("media.m3u8")
        .current_dir(&dir)
        .output()
        .expect("spawn mediastreamvalidator");

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
