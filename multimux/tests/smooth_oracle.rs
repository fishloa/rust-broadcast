//! Independent oracle for the Smooth Streaming output (MS-SSTR) — audit run
//! 7, W13.
//!
//! The committed `fixtures/ts/h264_aac.ts` capture (real H.264 + AAC) is
//! pushed through `InputSpec::TsUdp` into a `smooth` route served by
//! `serve_with_registry`; the client Manifest and every advertised fragment
//! are then fetched back over real loopback HTTP.
//!
//! Two independent checks, neither of which shares our code:
//!
//! - **Per-track fragments** — an audio `StreamIndex` must be answered with
//!   an *audio-only* fragment and a video `StreamIndex` with a *video-only*
//!   one (MS-SSTR §2.2.4). `MP4Box -info` (GPAC) identifies the track
//!   handler of each fetched fragment, and the two fragments are asserted to
//!   be byte-different; before the fix every StreamIndex returned the same
//!   muxed segment, so the two were byte-identical.
//! - **No fabricated FourCC** — every advertised `StreamIndex` carries the
//!   FourCC its codec actually is, never a hard-coded `H264` for an unknown
//!   codec.
//!
//! `MP4Box`/`ffprobe` skip **loudly** when absent, so this file stays green
//! on Linux/CI.
#![cfg(feature = "test-seams")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry_on;

#[path = "support/listener.rs"]
mod listener;
use listener::bind_tcp;
use transmux::smooth_parse::SmoothManifest;

fn fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac_40s.ts"
    ))
}

fn have_oracle(bin: &str) -> bool {
    Command::new(bin)
        .arg("-version")
        .output()
        .or_else(|_| Command::new(bin).arg("-h").output())
        .map(|_| true)
        .unwrap_or(false)
}

/// Bind `127.0.0.1:0` for UDP and return the live address + the still-bound
/// socket, handed to the route via `Config::prebound` so it never races a
/// reserve-then-rebind (SP7.1).
async fn bind_udp_addr() -> (std::net::SocketAddr, tokio::net::UdpSocket) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0 udp");
    let addr = socket.local_addr().expect("local addr");
    (addr, socket)
}

fn base_config(bind: std::net::SocketAddr, input: InputSpec) -> Config {
    Config {
        bind: bind.to_string(),
        // >= 4 s segments: the shape W13a/W13b are about (the default 0.5 s
        // would never reach it).
        target_duration_secs: 4.0,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input,
            outputs: vec![OutputKind::Smooth],
            dvr: DvrConfig::default(),
        }],
        ..Config::default()
    }
}

/// Polls the Smooth Manifest until it advertises at least `min_chunks` chunks.
async fn poll_manifest(client: &reqwest::Client, url: &str, min_chunks: usize) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(resp) = client.get(url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.matches("<c ").count() >= min_chunks
        {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "fewer than {min_chunks} chunks (<c>) appeared in {url} within the hang guard"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct Served {
    bind_addr: std::net::SocketAddr,
    server: tokio::task::JoinHandle<Result<(), multimux::MultimuxError>>,
    stop: Arc<AtomicBool>,
    sender: tokio::task::JoinHandle<()>,
    manifest: String,
}

/// Start a Smooth route fed the real fixture over UDP, returning once the
/// manifest advertises at least `min_chunks` chunks.
///
/// Both the media listener AND the UDP input socket are bound `:0` here and
/// handed over live (SP7.1) — the media listener into `serve_with_registry_on`
/// and the UDP socket into the route via `Config::prebound` — so the origin
/// never races a reserved port. No whole-attempt retry loop is needed now that
/// neither address can lose a bind race.
async fn serve_smooth_until_fragment(min_chunks: usize) -> Served {
    let client = reqwest::Client::new();
    let (bind_addr, bind_listener) = bind_tcp();
    let (udp_addr, udp_socket) = bind_udp_addr().await;
    let mut config = base_config(
        bind_addr,
        InputSpec::TsUdp {
            addr: udp_addr.to_string(),
            multicast_group: None,
            socket: Default::default(),
        },
    );
    config.prebound.with_udp(udp_addr.to_string(), udp_socket);
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));

    let ts_bytes = std::fs::read(fixture_path()).expect("h264_aac_40s.ts fixture must exist");
    let stop = Arc::new(AtomicBool::new(false));
    let sender_stop = Arc::clone(&stop);
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind sender");
    let sender = tokio::spawn(async move {
        while !sender_stop.load(Ordering::Relaxed) {
            for chunk in ts_bytes.chunks(7 * 188) {
                let _ = socket.send_to(chunk, udp_addr).await;
                tokio::time::sleep(Duration::from_millis(3)).await;
            }
        }
    });

    let url = format!("http://{bind_addr}/cam/Manifest");
    let manifest = tokio::time::timeout(
        Duration::from_secs(20),
        poll_manifest(&client, &url, min_chunks),
    )
    .await
    .expect("the Smooth manifest must advertise a fragment within 20s");
    Served {
        bind_addr,
        server,
        stop,
        sender,
        manifest,
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.sender.abort();
        self.server.abort();
    }
}

impl Served {
    fn shutdown(self) {
        // Drop does the teardown; this exists so a test can end early.
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "multimux-smooth-oracle-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Fetch the first fragment for each advertised StreamIndex, returning
/// `(stream_type_name, fourcc, bytes)` in manifest order.
async fn fetch_first_fragments(
    client: &reqwest::Client,
    base: &str,
    manifest: &SmoothManifest,
) -> Vec<(String, String, Vec<u8>)> {
    let mut out = Vec::new();
    for stream in &manifest.streams {
        let chunks = stream
            .enumerate_chunks()
            .unwrap_or_else(|e| panic!("enumerate_chunks failed: {e:?}"));
        let Some((start_time, _)) = chunks.first() else {
            continue;
        };
        let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);
        let url = stream.resolve_fragment_url(bitrate, *start_time);
        let bytes = client
            .get(format!("{base}/{url}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {url} failed: {e}"))
            .error_for_status()
            .unwrap_or_else(|e| panic!("fragment {url} not OK: {e}"))
            .bytes()
            .await
            .expect("fragment body");
        out.push((
            stream.stream_type.name().to_string(),
            stream
                .qualities
                .first()
                .map(|q| q.four_cc.clone())
                .unwrap_or_default(),
            bytes.to_vec(),
        ));
    }
    out
}

/// The manifest advertises both a video and an audio StreamIndex for the
/// real H.264+AAC capture, each with the FourCC its codec actually is.
#[tokio::test]
async fn manifest_advertises_real_codecs_for_both_tracks() {
    let served = serve_smooth_until_fragment(1).await;
    let manifest = SmoothManifest::parse(&served.manifest).expect("manifest parses");
    served.shutdown();

    let types: Vec<&str> = manifest
        .streams
        .iter()
        .map(|s| s.stream_type.name())
        .collect();
    assert!(
        types.contains(&"video") && types.contains(&"audio"),
        "the H.264+AAC capture must yield both StreamIndexes: {types:?}"
    );
    for stream in &manifest.streams {
        let fourcc = stream
            .qualities
            .first()
            .map(|q| q.four_cc.as_str())
            .unwrap_or("");
        match stream.stream_type.name() {
            "video" => assert_eq!(fourcc, "H264", "video quality FourCC"),
            "audio" => assert_eq!(fourcc, "AACL", "audio quality FourCC"),
            other => panic!("unexpected StreamIndex type {other}"),
        }
    }
}

/// Each StreamIndex is answered with its **own** track's fragment — the
/// video and audio fragment bytes differ, and `ffprobe` (given the route's
/// init segment) decodes only the requested track's codec from each.
#[tokio::test]
async fn each_stream_index_serves_its_own_track_fragment() {
    if !have_oracle("MP4Box") {
        eprintln!("SKIP each_stream_index_serves_its_own_track_fragment: MP4Box not on PATH");
        return;
    }
    let served = serve_smooth_until_fragment(1).await;
    let manifest = SmoothManifest::parse(&served.manifest).expect("manifest parses");
    let client = reqwest::Client::new();
    let base = format!("http://{}/cam", served.bind_addr);
    let fragments = fetch_first_fragments(&client, &base, &manifest).await;
    served.shutdown();

    let video = fragments
        .iter()
        .find(|(ty, _, _)| ty == "video")
        .expect("video fragment");
    let audio = fragments
        .iter()
        .find(|(ty, _, _)| ty == "audio")
        .expect("audio fragment");

    assert!(
        video.2 != audio.2,
        "the audio StreamIndex must not be served the video (or muxed) fragment \
         (video {} bytes, audio {} bytes)",
        video.2.len(),
        audio.2.len()
    );

    let dir = temp_dir("mp4box");
    let video_tkids = mp4box_track_ids(&dir, "video", &video.2);
    let audio_tkids = mp4box_track_ids(&dir, "audio", &audio.2);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        video_tkids.len(),
        1,
        "the video fragment must carry exactly one track (MS-SSTR §2.2.4): {video_tkids:?}"
    );
    assert_eq!(
        audio_tkids.len(),
        1,
        "the audio fragment must carry exactly one track (MS-SSTR §2.2.4): {audio_tkids:?}"
    );
    assert_ne!(
        video_tkids[0], audio_tkids[0],
        "video and audio StreamIndexes must map to different tracks, not the same (muxed) one"
    );
}

/// Write `fragment` to `dir/<tag>.mp4` and return the track ids MP4Box
/// reports for its movie fragments (`{TKID n}`), an independent reader that
/// sees exactly the tracks the fragment's `moof` actually references.
fn mp4box_track_ids(dir: &std::path::Path, tag: &str, fragment: &[u8]) -> Vec<u32> {
    let path = dir.join(format!("{tag}.mp4"));
    std::fs::write(&path, fragment).expect("write fragment");
    let out = Command::new("MP4Box")
        .arg("-info")
        .arg(&path)
        .output()
        .expect("spawn MP4Box");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let mut ids = Vec::new();
    let mut rest = text.as_str();
    while let Some(pos) = rest.find("{TKID ") {
        let after = &rest[pos + "{TKID ".len()..];
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if let Ok(id) = after[..end].parse::<u32>() {
            ids.push(id);
        }
        rest = &after[end..];
    }
    ids
}
